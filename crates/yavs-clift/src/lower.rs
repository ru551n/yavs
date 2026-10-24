//! `yavs-ir` to Cranelift: the same compiled-code contract as the LLVM tier (`yavs-llvm`
//! `rt.rs`): `fn(env, regs, mem, pc) -> u32` working on the registers of the interpreter frame,
//! entering at `pc`, executing a subset of the instructions itself and every other instruction
//! in place through the `yavs_rt_step` callback. Compiled code never reports an error itself: a
//! failing inline instruction returns its pc and the interpreter re-executes it.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use cranelift_codegen::Context;
use cranelift_codegen::inline::{Inline, InlineCommand};
use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{
    self, AbiParam, AliasRegionData, Block, BlockArg, FuncRef, GlobalValue, InstBuilder,
    MemFlagsData, Signature, StackSlotData, StackSlotKind, Type, UserFuncName, Value, types,
};

use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Switch};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{DataDescription, DataId, FuncId as ClifId, Linkage, Module};
use yavs_interp::space;
use yavs_ir::{
    AddrSpace, BoolOp, CheckKind, CmpOp, FOp, FUnOp, Func, FuncId, FuncKind, Inst, IntOp, IntUnOp,
    Program, Reg, Width,
};
use yavs_llvm::rt::*;

const I64: Type = types::I64;
const I32: Type = types::I32;
const I8: Type = types::I8;
const F64: Type = types::F64;

/// Zero `n` bytes (`Inst::Alloc`).
extern "C" fn clift_zero(p: *mut u8, n: u64) {
    // SAFETY: compiled code passes a range inside the temporary area's capacity.
    unsafe { std::ptr::write_bytes(p, 0, n as usize) }
}

/// `yavs_rt_deopt`, printing the frame when `YAVS_CLIFT_TRACE` is set (a measurement knob: how
/// often native frames are materialised).
extern "C" fn clift_deopt(
    env: *mut std::ffi::c_void,
    mem: *mut Mem,
    func: u32,
    pc: u32,
    temp_base: u64,
    nregs: u64,
    slen: u64,
    regs: *const u64,
) -> *mut u64 {
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *TRACE.get_or_init(|| std::env::var_os("YAVS_CLIFT_TRACE").is_some()) {
        eprintln!("materialise f{func} at pc {pc}");
    }
    // SAFETY: compiled code passes the `Env` and `Mem` it was given.
    unsafe { yavs_rt_deopt(env, mem, func, pc, temp_base, nregs, slen, u64::MAX, regs) }
}

/// `memmove` (`Inst::Copy`; ranges may overlap).
extern "C" fn clift_move(d: *mut u8, s: *const u8, n: u64) {
    // SAFETY: compiled code passes ranges `Mem::find` bounded.
    unsafe { std::ptr::copy(s, d, n as usize) }
}

/// Cranelift error text.
pub type Error = String;

/// A Cranelift JIT module with the runtime callbacks compiled code calls.
pub struct Jit {
    module: JITModule,
    step: ClifId,
    host: ClifId,
    signal: ClifId,
    lcopy: ClifId,
    zero: ClifId,
    mov: ClifId,
    deopt: ClifId,
    deopt_slots: ClifId,
    deopt_end: ClifId,
    /// The kernel-facing callbacks of `rt.rs`, in the order of `KERNEL`.
    kernel: [ClifId; 12],
    tables: HashMap<Vec<u8>, DataId>,
    sig: Signature,
    /// Calls between compiled functions without a frame in the interpreter (`NatInfo`) and
    /// whether Cranelift inlines them (`YAVS_CLIFT_NATIVE`, `YAVS_CLIFT_INLINE`).
    native_on: bool,
    /// Debugging: the most native variants to make (`YAVS_CLIFT_NATIVE_MAX`).
    native_limit: usize,
    inline_max: usize,
    /// Per function: its native-frame variant, or `None` when it has none.
    natives: HashMap<FuncId, Option<Rc<NatInfo>>>,
    building: HashSet<FuncId>,
}

/// The native-frame variant `n<id>` of a function: its registers and slots live on the native
/// stack, the interpreter has no frame for it, and anything the compiled code cannot finish
/// materialises the frame (`yavs_rt_deopt`).
struct NatInfo {
    id: ClifId,
    /// The Cranelift body, for the inliner.
    body: ir::Function,
    nparams: usize,
    /// Whether it returns a value.
    ret: bool,
}

/// Most IR instructions and frame bytes of a function with a native-frame variant.
const NAT_MAX_IR: usize = 400;
const NAT_MAX_FRAME: u32 = 1024;
const NAT_MAX_REGS: usize = 128;

// SAFETY: the module owns its code memory and is used by one thread at a time (the compile
// thread, or the thread that built it); compiled code is called through plain pointers.
unsafe impl Send for Jit {}

/// Measurement knobs: `YAVS_CLIFT_OPT` (`speed` by default, or `none`), `YAVS_CLIFT_VERIFY` (`1`
/// runs the Cranelift verifier; on in debug builds only), `YAVS_CLIFT_RA` (`backtracking` by
/// default, or `single_pass`).
fn flag(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

impl Jit {
    pub fn new() -> Result<Jit, Error> {
        let opt = flag("YAVS_CLIFT_OPT", "speed");
        let verify = flag(
            "YAVS_CLIFT_VERIFY",
            if cfg!(debug_assertions) { "1" } else { "0" },
        );
        let ra = flag("YAVS_CLIFT_RA", "backtracking");
        let native_on = flag("YAVS_CLIFT_NATIVE", "1") != "0";
        let native_limit = flag("YAVS_CLIFT_NATIVE_MAX", "")
            .parse()
            .unwrap_or(usize::MAX);
        let inline_max = flag("YAVS_CLIFT_INLINE", "150").parse().unwrap_or(0);
        let flags = [
            ("opt_level", opt.as_str()),
            (
                "enable_verifier",
                if verify == "1" { "true" } else { "false" },
            ),
            ("regalloc_algorithm", ra.as_str()),
        ];
        let mut b = JITBuilder::with_flags(&flags, cranelift_module::default_libcall_names())
            .map_err(|e| e.to_string())?;
        for (n, p) in [
            (STEP, yavs_rt_step as *const u8),
            (HOST, yavs_rt_host as *const u8),
            (SIGNAL, yavs_rt_signal as *const u8),
            (LCOPY, yavs_rt_lcopy as *const u8),
            (DEOPT, clift_deopt as *const u8),
            (DEOPT_SLOTS, yavs_rt_deopt_slots as *const u8),
            (DEOPT_END, yavs_rt_deopt_end as *const u8),
            ("clift_zero", clift_zero as *const u8),
            ("clift_move", clift_move as *const u8),
            (WAIT, yavs_rt_wait as *const u8),
            (WAIT_ON, yavs_rt_wait_on as *const u8),
            (WAIT_SAME, yavs_rt_wait_same as *const u8),
            (DRIVE, yavs_rt_drive as *const u8),
            (DRIVE_BIT, yavs_rt_drive_bit as *const u8),
            (CODES, yavs_rt_codes as *const u8),
            (PLANES, yavs_rt_planes as *const u8),
            (NATIVE, yavs_rt_native as *const u8),
            (FAST, yavs_rt_fast as *const u8),
            (TROOT, yavs_rt_troot as *const u8),
            (ENDSTMT, yavs_rt_endstmt as *const u8),
            (XSTEP, yavs_rt_xstep as *const u8),
        ] {
            b.symbol(n, p);
        }
        let mut module = JITModule::new(b);
        let mut imp = |name: &str, params: &[Type], ret: Option<Type>| {
            let mut s = module.make_signature();
            s.params.extend(params.iter().map(|&t| AbiParam::new(t)));
            s.returns.extend(ret.map(AbiParam::new));
            module
                .declare_function(name, Linkage::Import, &s)
                .map_err(|e| e.to_string())
        };
        let step = imp(STEP, &[I64, I64, I64, I32], Some(I32))?;
        let host = imp(HOST, &[I64, I64, I64, I64, I64, I32], Some(I64))?;
        let signal = imp(SIGNAL, &[I64, I64, I64, I32, I32], Some(I32))?;
        let lcopy = imp(LCOPY, &[I64; 7], None)?;
        let zero = imp("clift_zero", &[I64, I64], None)?;
        let mov = imp("clift_move", &[I64, I64, I64], None)?;
        let deopt = imp(DEOPT, &[I64, I64, I32, I32, I64, I64, I64, I64], Some(I64))?;
        let deopt_slots = imp(DEOPT_SLOTS, &[I64], Some(I64))?;
        let deopt_end = imp(DEOPT_END, &[I64, I64, I32], Some(I32))?;
        let wait_sig = [I64, I64, I64, I32, I64, I64];
        let kernel = [
            imp(WAIT, &wait_sig, Some(I32))?,
            imp(WAIT_ON, &wait_sig, Some(I32))?,
            imp(WAIT_SAME, &wait_sig, Some(I32))?,
            imp(
                DRIVE,
                &[I64, I64, I64, I32, I32, I64, I64, I64, I32],
                Some(I32),
            )?,
            imp(
                DRIVE_BIT,
                &[I64, I64, I64, I32, I32, I64, I64, I64, I64],
                Some(I32),
            )?,
            imp(CODES, &[I64; 4], None)?,
            imp(PLANES, &[I64; 5], None)?,
            imp(NATIVE, &[I64, I64, I32, I64, I32, I64, I64], Some(I32))?,
            imp(FAST, &[I64, I64, I64, I32, I32, I64, I64], Some(I32))?,
            imp(TROOT, &[I64, I64, I64, I32, I64, I64], Some(I32))?,
            imp(ENDSTMT, &[I64, I64, I64], None)?,
            imp(
                XSTEP,
                &[I64, I64, I32, I32, I64, I64, I64, I32, I64, I64, I32, I64],
                Some(I32),
            )?,
        ];
        let mut sig = module.make_signature();
        sig.params.extend([AbiParam::new(I64); 3]);
        sig.params.push(AbiParam::new(I32));
        sig.returns.push(AbiParam::new(I32));
        Ok(Jit {
            module,
            step,
            host,
            signal,
            lcopy,
            zero,
            mov,
            deopt,
            deopt_slots,
            deopt_end,
            kernel,
            tables: HashMap::new(),
            sig,
            native_on,
            native_limit,
            inline_max,
            natives: HashMap::new(),
            building: HashSet::new(),
        })
    }

    /// Translate and define `f` as `f<id>`; call `finalize` before running it.
    pub fn define(
        &mut self,
        prog: &Program,
        id: FuncId,
        f: &Func,
    ) -> Result<(ClifId, Vec<String>), Error> {
        if let Some(r) = bad_reg(f) {
            return Err(format!("register r{} beyond the frame", r.0));
        }
        if std::env::var_os("YAVS_CLIFT_DUMP").is_some() {
            eprintln!("{}", prog.dump_func(id));
        }
        self.build(prog, id, f, false).map(|(c, s, _)| (c, s))
    }

    /// The native-frame variant of `id`, made on first use.
    fn native_of(&mut self, prog: &Program, id: FuncId) -> Option<Rc<NatInfo>> {
        if !self.native_on {
            return None;
        }
        if let Some(r) = self.natives.get(&id) {
            return r.clone();
        }
        // A function calling itself, directly or not, has no native variant.
        if !self.building.insert(id) {
            return None;
        }
        let r = self.try_native(prog, id);
        self.building.remove(&id);
        self.natives.insert(id, r.clone());
        r
    }

    fn try_native(&mut self, prog: &Program, id: FuncId) -> Option<Rc<NatInfo>> {
        let f = prog.func(id)?;
        let dump = std::env::var_os("YAVS_CLIFT_DUMP").is_some();
        if !matches!(f.kind, FuncKind::Function | FuncKind::Procedure)
            || f.parent.is_some()
            || f.code.is_empty()
            || f.code.len() > NAT_MAX_IR
            || f.frame > NAT_MAX_FRAME
            || f.reg_ty.len() > NAT_MAX_REGS
            || bad_reg(f).is_some()
        {
            if dump {
                eprintln!(
                    "no native variant of {id} {}: kind {:?}, parent {:?}, {} IR, frame {}, {} regs",
                    f.name,
                    f.kind,
                    f.parent,
                    f.code.len(),
                    f.frame,
                    f.reg_ty.len()
                );
            }
            return None;
        }
        // Value parameters of one word only; every return agrees on a value.
        if !f.params.iter().all(|p| {
            p.class == yavs_ir::ParamClass::Value
                && p.width != Width::Sig
                && u64::from(p.slot) + u64::from(p.width.bytes()) <= u64::from(f.frame)
        }) {
            if dump {
                eprintln!(
                    "no native variant of {id} {}: params {:?}",
                    f.name,
                    f.params
                        .iter()
                        .map(|p| (p.class, p.width))
                        .collect::<Vec<_>>()
                );
            }
            return None;
        }
        let mut rets = f.code.iter().filter_map(|i| match i {
            Inst::Ret { v } => Some(v.is_some()),
            _ => None,
        });
        let ret = rets.next()?;
        if rets.any(|r| r != ret) {
            return None;
        }
        if self.natives.values().flatten().count() >= self.native_limit {
            return None;
        }
        let (cid, _, body) = match self.build(prog, id, f, true) {
            Ok(r) => r,
            Err(why) => {
                if std::env::var_os("YAVS_CLIFT_DUMP").is_some() {
                    eprintln!("no native variant of {id} {}: {why}", f.name);
                }
                return None;
            }
        };
        if std::env::var_os("YAVS_CLIFT_DUMP").is_some() {
            eprintln!(
                "native variant #{} of {} {}",
                self.natives.values().flatten().count() + 1,
                id,
                f.name
            );
        }
        Some(Rc::new(NatInfo {
            id: cid,
            body: body?,
            nparams: f.params.len(),
            ret,
        }))
    }

    /// Translate `f` into a function defined in the module: the frame-mode `f<id>`, or (`native`)
    /// the native-frame variant `n<id>`, whose Cranelift body is returned too.
    fn build(
        &mut self,
        prog: &Program,
        id: FuncId,
        f: &Func,
        native: bool,
    ) -> Result<(ClifId, Vec<String>, Option<ir::Function>), Error> {
        // The callees that have a native variant.
        let mut nats: HashMap<FuncId, Rc<NatInfo>> = HashMap::new();
        for i in &f.code {
            if let Inst::Call { func, .. } | Inst::CallProt { func, .. } = *i
                && !nats.contains_key(&func)
                && let Some(n) = self.native_of(prog, func)
            {
                nats.insert(func, n);
            }
        }
        let mut ctx = self.module.make_context();
        let mut fctx = FunctionBuilderContext::new();
        // Data for the table natives this function uses.
        let mut tabs = HashMap::new();
        for i in &f.code {
            if let Inst::CallNative { native, args, dst } = *i
                && let Some(NativeKind::Table(t, ..)) = native_kind(prog, f, native, args, dst)
                && !tabs.contains_key(&t)
            {
                let did = match self.tables.get(&t) {
                    Some(&d) => d,
                    None => {
                        let name = format!("t{}", self.tables.len());
                        let d = self
                            .module
                            .declare_data(&name, Linkage::Local, false, false)
                            .map_err(|e| e.to_string())?;
                        let mut dd = DataDescription::new();
                        dd.define(t.clone().into_boxed_slice());
                        self.module.define_data(d, &dd).map_err(|e| e.to_string())?;
                        self.tables.insert(t.clone(), d);
                        d
                    }
                };
                let gv = self.module.declare_data_in_func(did, &mut ctx.func);
                tabs.insert(t, gv);
            }
        }
        ctx.func.signature = if native {
            native_sig(&self.module, f.params.len())
        } else {
            self.sig.clone()
        };
        ctx.func.name = UserFuncName::user(u32::from(native), id.0);
        let mut natives = HashMap::new();
        let mut bodies: HashMap<FuncRef, Rc<NatInfo>> = HashMap::new();
        for (&g, n) in &nats {
            let fr = self.module.declare_func_in_func(n.id, &mut ctx.func);
            natives.insert(
                g,
                NatRef {
                    fr,
                    nparams: n.nparams,
                    ret: n.ret,
                },
            );
            bodies.insert(fr, n.clone());
        }
        let imports = Imports {
            step: self.module.declare_func_in_func(self.step, &mut ctx.func),
            host: self.module.declare_func_in_func(self.host, &mut ctx.func),
            signal: self.module.declare_func_in_func(self.signal, &mut ctx.func),
            lcopy: self.module.declare_func_in_func(self.lcopy, &mut ctx.func),
            zero: self.module.declare_func_in_func(self.zero, &mut ctx.func),
            mov: self.module.declare_func_in_func(self.mov, &mut ctx.func),
            deopt: self.module.declare_func_in_func(self.deopt, &mut ctx.func),
            deopt_slots: self
                .module
                .declare_func_in_func(self.deopt_slots, &mut ctx.func),
            deopt_end: self
                .module
                .declare_func_in_func(self.deopt_end, &mut ctx.func),
            wait: self
                .module
                .declare_func_in_func(self.kernel[0], &mut ctx.func),
            wait_on: self
                .module
                .declare_func_in_func(self.kernel[1], &mut ctx.func),
            wait_same: self
                .module
                .declare_func_in_func(self.kernel[2], &mut ctx.func),
            drive: self
                .module
                .declare_func_in_func(self.kernel[3], &mut ctx.func),
            drive_bit: self
                .module
                .declare_func_in_func(self.kernel[4], &mut ctx.func),
            codes: self
                .module
                .declare_func_in_func(self.kernel[5], &mut ctx.func),
            planes: self
                .module
                .declare_func_in_func(self.kernel[6], &mut ctx.func),
            native: self
                .module
                .declare_func_in_func(self.kernel[7], &mut ctx.func),
            fast: self
                .module
                .declare_func_in_func(self.kernel[8], &mut ctx.func),
            troot: self
                .module
                .declare_func_in_func(self.kernel[9], &mut ctx.func),
            endstmt: self
                .module
                .declare_func_in_func(self.kernel[10], &mut ctx.func),
            xstep: self
                .module
                .declare_func_in_func(self.kernel[11], &mut ctx.func),
            tables: tabs,
            natives,
        };
        let cfg = self.module.target_config();
        let stepped = gen_func(
            &mut ctx,
            &mut fctx,
            cfg,
            prog,
            f,
            imports,
            native.then_some(id.0),
        )?;
        if self.inline_max > 0 && !bodies.is_empty() {
            let inl = Inl {
                bodies: &bodies,
                max: self.inline_max,
                budget: 4 * self.inline_max.max(500),
            };
            ctx.inline(inl)
                .map_err(|e| format!("Cranelift inline: {e:?}"))?;
        }
        let fid = self
            .module
            .declare_function(
                &format!("{}{}", if native { "n" } else { "f" }, id.0),
                Linkage::Export,
                &ctx.func.signature,
            )
            .map_err(|e| e.to_string())?;
        let body = native.then(|| ctx.func.clone());
        self.module
            .define_function(fid, &mut ctx)
            .map_err(|e| format!("Cranelift: {e:?} (a bug in yavs)"))?;
        self.module.clear_context(&mut ctx);
        Ok((fid, stepped, body))
    }

    pub fn finalize(&mut self) -> Result<(), Error> {
        self.module
            .finalize_definitions()
            .map_err(|e| e.to_string())
    }

    /// The entry of a defined function after `finalize`.
    pub fn entry(&self, id: ClifId) -> RunFn {
        let p = self.module.get_finalized_function(id);
        // SAFETY: `define` generated the function with the `RunFn` signature.
        unsafe { std::mem::transmute::<*const u8, RunFn>(p) }
    }
}

struct Imports {
    step: FuncRef,
    host: FuncRef,
    signal: FuncRef,
    lcopy: FuncRef,
    zero: FuncRef,
    mov: FuncRef,
    deopt: FuncRef,
    deopt_slots: FuncRef,
    deopt_end: FuncRef,
    wait: FuncRef,
    wait_on: FuncRef,
    wait_same: FuncRef,
    drive: FuncRef,
    drive_bit: FuncRef,
    codes: FuncRef,
    planes: FuncRef,
    native: FuncRef,
    fast: FuncRef,
    troot: FuncRef,
    endstmt: FuncRef,
    xstep: FuncRef,
    tables: HashMap<Vec<u8>, GlobalValue>,
    natives: HashMap<FuncId, NatRef>,
}

/// A call to a native-frame variant from a function being built.
#[derive(Clone, Copy)]
struct NatRef {
    fr: FuncRef,
    nparams: usize,
    ret: bool,
}

/// The signature of a native-frame variant: `(env, mem, depth, args...) -> (status, value)`;
/// status 0 is a return (with the value), anything else a recorded frame (`yavs_rt_deopt`).
fn native_sig(module: &JITModule, nparams: usize) -> Signature {
    let mut s = module.make_signature();
    s.params.extend([AbiParam::new(I64); 3]);
    s.params.extend((0..nparams).map(|_| AbiParam::new(I64)));
    s.returns.push(AbiParam::new(I32));
    s.returns.push(AbiParam::new(I64));
    s
}

/// Inlines the native-frame variants that are small, up to a budget of Cranelift instructions
/// per caller (`YAVS_CLIFT_INLINE`: the largest callee, 0 for none).
struct Inl<'a> {
    bodies: &'a HashMap<FuncRef, Rc<NatInfo>>,
    max: usize,
    budget: usize,
}

impl Inline for Inl<'_> {
    fn inline(
        &mut self,
        _caller: &ir::Function,
        _call: ir::Inst,
        _opcode: ir::Opcode,
        callee: FuncRef,
        _args: &[Value],
    ) -> InlineCommand<'_> {
        match self.bodies.get(&callee) {
            Some(n) => {
                let size = n.body.dfg.num_insts();
                if size <= self.max && size <= self.budget {
                    self.budget -= size;
                    InlineCommand::Inline {
                        callee: Cow::Borrowed(&n.body),
                        visit_callee: false,
                    }
                } else {
                    InlineCommand::KeepCall
                }
            }
            None => InlineCommand::KeepCall,
        }
    }
}

/// Instructions that only change the temporary area or a register: the interpreter's own function
/// for them (`TierHost::signal`) runs them without its generic dispatch.
fn via_signal(i: &Inst) -> bool {
    matches!(
        i,
        Inst::TempRoot { .. }
            | Inst::NewFrame { .. }
            | Inst::Stmt
            | Inst::Deref { .. }
            | Inst::MemEq { .. }
            | Inst::Cat { .. }
    )
}

/// A slow path left to emit after the function body: step `inst` at `pc` and go on at `join`.
struct Slow {
    block: Block,
    join: Block,
    pc: u32,
    keep: bool,
    /// Through `yavs_rt_signal` with this edge argument instead of `yavs_rt_step`.
    edge: Option<u32>,
    /// In a native frame: `yavs_rt_endstmt` (the end of a statement that recorded temporaries)
    /// instead of leaving.
    endstmt: bool,
}

struct Gen<'a> {
    b: FunctionBuilder<'a>,
    imp: Imports,
    env: Value,
    regs: Value,
    mem: Value,
    exit: Block,
    /// Values of registers loaded or stored since the block began (invalidated at block
    /// starts, joins and after every callback).
    cache: Vec<Option<Value>>,
    slows: Vec<Slow>,
    /// Kinds of the instructions left to the interpreter whole (`step`).
    stepped: Vec<String>,
    /// What is known about each register at the current instruction (`track`).
    known: Vec<Option<Known>>,
    /// The frame's size in bytes (`Func::frame`): the slots of every frame of the function.
    frame_len: u64,
    /// The pointer to the frame's slots, loaded once per run of straight code.
    slots: Option<Value>,
    /// Memory flags per alias region: the register array, guest memory (slots, temporaries,
    /// objects), the `Mem` view. Cranelift forwards and drops accesses within a region.
    rf: MemFlagsData,
    gf: MemFlagsData,
    mf: MemFlagsData,
    /// Set for a native-frame variant.
    nat: Option<Nat>,
    /// Why a native-frame variant cannot be made.
    bad: Option<String>,
}

/// What a native-frame variant has instead of the interpreter's frame.
struct Nat {
    func: u32,
    nregs: u64,
    /// The frame index of this activation.
    depth: Value,
    /// The frame's slots on the native stack.
    slots: Value,
    /// The temporary area's length at entry: where `Stmt` trims to.
    /// The cell (a stack address) holding the temporary area's length the frame's statements trim
    /// to: the length at entry, raised by a `NewFrame` the frame's own statements must not free.
    temp_base: Value,
}

/// The instructions a native frame has the interpreter execute on a frame of its own: those of the
/// LLVM tier (`xstep_ok`) and the ones it lowers inline that this tier does not (`TierHost::native_step`
/// steps any instruction that needs only registers and memory the host reaches).
fn xstep_wide(f: &Func, i: &Inst) -> bool {
    xstep_ok(f, i)
        || matches!(
            i,
            Inst::NewFrame { .. }
                | Inst::Fill { .. }
                | Inst::Cat { .. }
                | Inst::SlicePos { .. }
                | Inst::MemEq { .. }
        ) && {
            let n = f.reg_ty.len() as u32;
            let (uses, defs) = yavs_interp::inst_regs(f, i);
            uses.iter().chain(&defs).all(|r| r.0 < n)
        }
}

/// A size or offset operand of `host`: known at compile time or in a value.
#[derive(Clone, Copy)]
enum Op {
    C(u64),
    V(Value),
}

fn ok<T>(r: Option<T>, what: &str) -> Result<T, String> {
    r.ok_or_else(|| what.to_string())
}

fn gen_func(
    ctx: &mut Context,
    fctx: &mut FunctionBuilderContext,
    cfg: cranelift_codegen::isa::TargetFrontendConfig,
    prog: &Program,
    f: &Func,
    imp: Imports,
    nat_id: Option<u32>,
) -> Result<Vec<String>, String> {
    let mut b = FunctionBuilder::new(&mut ctx.func, fctx);
    let entry = b.create_block();
    b.append_block_params_for_function_params(entry);
    let exit = b.create_block();
    let exit_pc = b.append_block_param(exit, I32);
    b.switch_to_block(entry);
    let p = b.block_params(entry).to_vec();
    let native = nat_id.is_some();
    let (env, mem) = (p[0], if native { p[1] } else { p[2] });
    let pc_in = if native { p[2] } else { p[3] };
    let nregs = f.reg_ty.len() as u64;
    // A native-frame variant keeps its registers and slots on the native stack.
    let mut nat = None;
    let regs = if native {
        let rs = b.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            (nregs * 8) as u32,
            3,
        ));
        let ss = b.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            f.frame.max(1),
            3,
        ));
        let regs = b.ins().stack_addr(I64, rs, 0);
        let slots = b.ins().stack_addr(I64, ss, 0);
        let len0 = b
            .ins()
            .load(I64, MemFlagsData::trusted(), mem, MEM_TEMP_LEN as i32);
        let tcell =
            b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
        let temp_base = b.ins().stack_addr(I64, tcell, 0);
        b.ins().store(MemFlagsData::trusted(), len0, temp_base, 0);
        nat = Some(Nat {
            func: nat_id.unwrap_or(0),
            nregs,
            depth: pc_in,
            slots,
            temp_base,
        });
        regs
    } else {
        p[1]
    };
    let n = f.code.len();
    let mut leader = vec![false; n + 1];
    leader[0] = true;
    for (_, bl) in f.blocks.iter() {
        if let Some(l) = leader.get_mut(bl.start as usize) {
            *l = true;
        }
    }
    for (i, inst) in f.code.iter().enumerate() {
        if resumes(prog, f, inst) {
            leader[i + 1] = true;
        }
    }
    // The interpreter enters a function after the constants and addresses it preloads into the
    // frame's registers (`Pre::next`): the pcs after such definitions at an entry are entries too.
    for i in 1..n {
        if native {
            break;
        }
        let preload = match f.code[i - 1] {
            Inst::Const { .. } | Inst::ConstF { .. } => true,
            Inst::AddrOf { space, .. } => !matches!(space, yavs_ir::AddrSpace::Up(_)),
            _ => false,
        };
        leader[i] |= leader[i - 1] && preload;
    }
    let bbs: Vec<Option<Block>> = (0..n)
        .map(|i| leader[i].then(|| b.create_block()))
        .collect();
    if !native {
        let other = b.create_block();
        let mut sw = Switch::new();
        for (i, bb) in bbs.iter().enumerate() {
            if let Some(bb) = bb {
                sw.set_entry(i as u128, *bb);
            }
        }
        sw.emit(&mut b, pc_in, other);
        b.switch_to_block(other);
        b.ins().jump(exit, &[BlockArg::Value(pc_in)]);
    }
    let region = |b: &mut FunctionBuilder<'_>, id: u32, name: &'static str| {
        b.func.dfg.alias_regions.insert(AliasRegionData {
            user_id: id,
            description: name.into(),
        })
    };
    let (rr, gr, mr) = (
        region(&mut b, if native { 4 } else { 1 }, "regs"),
        region(&mut b, 2, "guest"),
        region(&mut b, 3, "mem"),
    );
    // The registers the interpreter starts a frame with hold the same address everywhere; the
    // register itself may be unset (`Pre::lean`: no address for a parameter's slot), so a use
    // goes by what is known, as in the LLVM tier.
    let mut base_known: Vec<Option<Known>> = vec![None; f.reg_ty.len()];
    if !native {
        for (r, addr, _) in preloads(f) {
            base_known[r.0 as usize] = match addr {
                Some((AddrSpace::Frame, off)) => Some(Known::Frame(off.into())),
                Some((AddrSpace::Const(k), off)) => Some(Known::Const(k, off.into())),
                Some((AddrSpace::Inst, off)) => Some(Known::Inst(off.into())),
                _ => None,
            };
        }
    }
    let mut g = Gen {
        known: base_known.clone(),
        frame_len: u64::from(f.frame),
        slots: None,
        rf: MemFlagsData::trusted().with_alias_region(Some(rr)),
        gf: MemFlagsData::new()
            .with_notrap()
            .with_alias_region(Some(gr)),
        mf: MemFlagsData::trusted().with_alias_region(Some(mr)),
        b,
        imp,
        env,
        regs,
        mem,
        exit,
        cache: vec![None; f.reg_ty.len()],
        slows: Vec::new(),
        stepped: Vec::new(),
        nat,
        bad: None,
    };
    if g.nat.is_some() {
        g.native_entry(f, &p[3..])?;
        match bbs.first().copied().flatten() {
            Some(first) => {
                g.b.ins().jump(first, &[]);
            }
            None => return Err("no code".into()),
        }
    }
    let block_bb = |bl: yavs_ir::BlockId| -> Result<Block, String> {
        f.blocks
            .get(bl)
            .and_then(|bl| bbs.get(bl.start as usize).copied().flatten())
            .ok_or_else(|| format!("branch to missing block {}", bl.0))
    };
    // The scalar logic assignments the interpreter runs as one op (their waveform is not made here
    // either) and the waits that name the same signals every time (`yavs_rt_wait_same`).
    let kernel_facing = !native && f.kind == FuncKind::Process;
    let static_wait = if kernel_facing {
        yavs_interp::static_wait(prog, f, 1)
    } else {
        Vec::new()
    };
    let scalar_drives = if kernel_facing {
        yavs_interp::scalar_drives(prog, f)
    } else {
        Vec::new()
    };
    let skipped: HashSet<u32> = scalar_drives
        .iter()
        .flat_map(|d| [d.alloc, d.store, d.index, d.lset])
        .collect();
    let mut open = false;
    let mut prev = None;
    for (i, &inst) in f.code.iter().enumerate() {
        let pc = i as u32;
        if let Some(p) = prev.replace(inst) {
            track(&mut g.known, &[], &p);
        }
        if leader[i] {
            g.known.clone_from(&base_known);
        }
        if let Some(bb) = bbs[i] {
            if open {
                g.b.ins().jump(bb, &[]);
            }
            g.b.switch_to_block(bb);
            g.clear();
            open = true;
        } else if !open {
            // Dead code after a terminator: a block of its own, never entered.
            let bb = g.b.create_block();
            g.b.switch_to_block(bb);
            g.clear();
            open = true;
        }
        if let Inst::Ret { v } = inst {
            if g.nat.is_some() {
                let val = match v {
                    Some(r) => g.ld(r),
                    None => g.c64(0),
                };
                let ok = g.c32(0);
                g.b.ins().return_(&[ok, val]);
            } else {
                let v = g.c32(pc);
                g.leave(v);
            }
            open = false;
            continue;
        }
        if g.nat.is_some() && matches!(inst, Inst::Unreachable { .. }) {
            // The interpreter reports it, with the frame materialised.
            let v = g.c32(pc);
            g.leave(v);
            open = false;
            continue;
        }
        match inst {
            _ if skipped.contains(&pc) => {}
            Inst::Drive { .. } if let Some(d) = scalar_drives.iter().find(|d| d.drive == pc) => {
                g.drive_bit(pc, d)
            }
            Inst::Const { dst, v } => {
                let x = g.c64(v);
                g.st(dst, x)
            }
            Inst::ConstF { dst, v } => {
                let x = g.c64(v.to_bits() as i64);
                g.st(dst, x)
            }
            Inst::Move { dst, src } => {
                let x = g.ld(src);
                g.st(dst, x)
            }
            Inst::Bin { op, dst, a, b } => {
                let (x, y) = (g.ld(a), g.ld(b));
                match g.int_op(op, x, y, pc) {
                    Some(v) => g.st(dst, v),
                    None => g.step(pc, &inst),
                }
            }
            Inst::BinRange {
                op,
                dst,
                a,
                b,
                lo,
                hi,
                ..
            } => {
                let (x, y) = (g.ld(a), g.ld(b));
                match g.int_op(op, x, y, pc) {
                    Some(v) => {
                        let (l, h) = (g.ld(lo), g.ld(hi));
                        g.range_exit(v, l, h, pc);
                        g.st(dst, v);
                    }
                    None => g.step(pc, &inst),
                }
            }
            Inst::Select { dst, c, a, b } => {
                let (cv, x, y) = (g.ld(c), g.ld(a), g.ld(b));
                let nz = g.b.ins().icmp_imm_s(IntCC::NotEqual, cv, 0);
                let v = g.b.ins().select(nz, x, y);
                g.st(dst, v);
            }
            Inst::Un { op, dst, a } => {
                let x = g.ld(a);
                let min = g.b.ins().icmp_imm_s(IntCC::Equal, x, i64::MIN);
                g.exit_if(min, pc);
                let neg = g.b.ins().ineg(x);
                let v = match op {
                    IntUnOp::Neg => neg,
                    IntUnOp::Abs => {
                        let lt = g.b.ins().icmp_imm_s(IntCC::SignedLessThan, x, 0);
                        g.b.ins().select(lt, neg, x)
                    }
                };
                g.st(dst, v);
            }
            Inst::FBin { op, dst, a, b } => {
                let (x, y) = (g.ldf(a), g.ldf(b));
                let v = match op {
                    FOp::Add => g.b.ins().fadd(x, y),
                    FOp::Sub => g.b.ins().fsub(x, y),
                    FOp::Mul => g.b.ins().fmul(x, y),
                    FOp::Div => {
                        let zero = g.b.ins().f64const(0.0);
                        let z = g.b.ins().fcmp(FloatCC::Equal, y, zero);
                        g.exit_if(z, pc);
                        g.b.ins().fdiv(x, y)
                    }
                    FOp::PowI | FOp::Pow => {
                        g.step(pc, &inst);
                        continue;
                    }
                };
                // Not finite: |v| is not below infinity (NaN compares false).
                let abs = g.b.ins().fabs(v);
                let inf = g.b.ins().f64const(f64::INFINITY);
                let fin =
                    g.b.ins()
                        .fcmp(FloatCC::UnorderedOrGreaterThanOrEqual, abs, inf);
                g.exit_if(fin, pc);
                g.stf(dst, v);
            }
            Inst::FUn { op, dst, a } => {
                let x = g.ldf(a);
                let v = match op {
                    FUnOp::Neg => g.b.ins().fneg(x),
                    FUnOp::Abs => g.b.ins().fabs(x),
                };
                g.stf(dst, v);
            }
            Inst::IToF { dst, src } => {
                let x = g.ld(src);
                let v = g.b.ins().fcvt_from_sint(F64, x);
                g.stf(dst, v);
            }
            Inst::Cmp { op, dst, a, b } => {
                let cc = match op {
                    CmpOp::Eq => IntCC::Equal,
                    CmpOp::Ne => IntCC::NotEqual,
                    CmpOp::Lt => IntCC::SignedLessThan,
                    CmpOp::Le => IntCC::SignedLessThanOrEqual,
                    CmpOp::Gt => IntCC::SignedGreaterThan,
                    CmpOp::Ge => IntCC::SignedGreaterThanOrEqual,
                };
                let (x, y) = (g.ld(a), g.ld(b));
                let v = g.b.ins().icmp(cc, x, y);
                let v = g.b.ins().uextend(I64, v);
                g.st(dst, v);
            }
            Inst::FCmp { op, dst, a, b } => {
                // Ordered: every comparison with a NaN is false, as `partial_cmp`.
                let cc = match op {
                    CmpOp::Eq => FloatCC::Equal,
                    CmpOp::Ne => FloatCC::OrderedNotEqual,
                    CmpOp::Lt => FloatCC::LessThan,
                    CmpOp::Le => FloatCC::LessThanOrEqual,
                    CmpOp::Gt => FloatCC::GreaterThan,
                    CmpOp::Ge => FloatCC::GreaterThanOrEqual,
                };
                let (x, y) = (g.ldf(a), g.ldf(b));
                let v = g.b.ins().fcmp(cc, x, y);
                let v = g.b.ins().uextend(I64, v);
                g.st(dst, v);
            }
            Inst::Bool { op, dst, a, b } => {
                let x = g.ld(a);
                let x = g.b.ins().icmp_imm_s(IntCC::NotEqual, x, 0);
                let v = if op == BoolOp::Not {
                    g.b.ins().bxor_imm_s(x, 1)
                } else {
                    let y = g.ld(b);
                    let y = g.b.ins().icmp_imm_s(IntCC::NotEqual, y, 0);
                    match op {
                        BoolOp::And => g.b.ins().band(x, y),
                        BoolOp::Or => g.b.ins().bor(x, y),
                        _ => g.b.ins().bxor(x, y),
                    }
                };
                let v = g.b.ins().uextend(I64, v);
                g.st(dst, v);
            }
            Inst::Check {
                kind, v, lo, hi, ..
            } => {
                let x = g.ld(v);
                match kind {
                    CheckKind::Range | CheckKind::Index => {
                        let (l, h) = (g.ld(lo), g.ld(hi));
                        g.range_exit(x, l, h, pc)
                    }
                    CheckKind::Length => {
                        let l = g.ld(lo);
                        let ne = g.b.ins().icmp(IntCC::NotEqual, x, l);
                        g.exit_if(ne, pc);
                    }
                    CheckKind::Null => {
                        let z = g.b.ins().icmp_imm_s(IntCC::Equal, x, 0);
                        g.exit_if(z, pc);
                    }
                }
            }
            Inst::FCheck { v, lo, hi, .. } => {
                let (x, l, h) = (g.ldf(v), g.ldf(lo), g.ldf(hi));
                // !(l <= x && x <= h), NaN included.
                let a = g.b.ins().fcmp(FloatCC::UnorderedOrGreaterThan, l, x);
                let c = g.b.ins().fcmp(FloatCC::UnorderedOrGreaterThan, x, h);
                let bad = g.b.ins().bor(a, c);
                g.exit_if(bad, pc);
            }
            Inst::Index {
                dst,
                base,
                pos,
                stride,
            } => {
                // Address offset in the low 32 bits; anything over 4 GiB is the interpreter's
                // error.
                let a = g.ld(base);
                let p = g.ld(pos);
                let s = g.c64(i64::from(stride));
                let (off, o1) = g.b.ins().umul_overflow(p, s);
                let lo = g.b.ins().band_imm_s(a, 0xffff_ffff);
                let (sum, o2) = g.b.ins().uadd_overflow(lo, off);
                let big =
                    g.b.ins()
                        .icmp_imm_s(IntCC::UnsignedGreaterThan, sum, 0xffff_ffff);
                let bad = g.b.ins().bor(o1, o2);
                let bad = g.b.ins().bor(bad, big);
                g.exit_if(bad, pc);
                let hi = g.b.ins().band_imm_s(a, !0xffff_ffff_i64);
                let v = g.b.ins().bor(hi, sum);
                g.st(dst, v);
            }
            Inst::AddrOf { dst, space, off } => match addr_const(space, off) {
                Some(a) if space == AddrSpace::Frame => {
                    let hi = match &g.nat {
                        Some(n) => {
                            let d = n.depth;
                            g.b.ins().bor_imm_s(d, i64::from(space::FRAME) << 29)
                        }
                        None => g.mem_ld(MEM_FRAME),
                    };
                    let hi = g.b.ins().ishl_imm_s(hi, 32);
                    let v = g.b.ins().bor_imm_s(hi, a as i64);
                    g.st(dst, v);
                }
                Some(a) => {
                    let v = g.c64(a as i64);
                    g.st(dst, v)
                }
                None => match space {
                    AddrSpace::Up(d @ 1..=2) if g.nat.is_none() => {
                        let (slow, join) = g.slow_path(pc, &inst);
                        let hi = g.mem_ld(MEM_UP + 8 * u64::from(d - 1));
                        let okc = g.b.ins().icmp_imm_s(IntCC::NotEqual, hi, -1);
                        g.guard(okc, slow);
                        let hi = g.b.ins().ishl_imm_s(hi, 32);
                        let v = g.b.ins().bor_imm_s(hi, i64::from(off));
                        g.st(dst, v);
                        g.join(join);
                    }
                    _ => g.step(pc, &inst),
                },
            },
            Inst::Stmt => {
                let (slow, join) = g.slow_path(pc, &inst);
                // In a native frame the host's own function drops the recorded temporaries.
                if g.nat.is_some()
                    && let Some(sl) = g.slows.last_mut()
                {
                    sl.endstmt = true;
                }
                let roots = g.mem_ld(MEM_TEMP_ROOTS);
                let z = g.b.ins().icmp_imm_s(IntCC::Equal, roots, 0);
                g.guard(z, slow);
                let len = g.mem_ld(MEM_TEMP_LEN);
                let base = match &g.nat {
                    Some(_) => g.nat_temp_base(),
                    None => g.mem_ld(MEM_TEMP_BASE),
                };
                let v = g.b.ins().umin(base, len);
                g.mem_st(MEM_TEMP_LEN, v);
                g.join(join);
                if let Some(&(_, at)) = static_wait.iter().find(|w| w.0 == i) {
                    g.wait_same(at as u32);
                }
            }
            Inst::Deref { dst, handle, .. } => g.deref(pc, &inst, dst, handle),
            Inst::TempRoot { addr, desc, ty } => g.temp_root(prog, pc, &inst, addr, desc, ty)?,
            Inst::Alloc { dst, size } => {
                let (slow, join) = g.slow_path(pc, &inst);
                let sz = g.ld(size);
                let (a, _) = g.temp_alloc(sz, slow);
                g.st(dst, a);
                g.join(join);
            }
            Inst::LayoutOf { dst, ty, desc, .. } => match static_layout(prog, ty) {
                Some(sl) => {
                    let (slow, join) = g.slow_path(pc, &inst);
                    let words = g.layout_words(&sl, desc, slow);
                    let n = g.c64(words.len() as i64 * 8);
                    let (a, p) = g.temp_alloc(n, slow);
                    for (k, w) in words.into_iter().enumerate() {
                        g.b.ins().store(g.gf, w, p, k as i32 * 8);
                    }
                    g.st(dst, a);
                    g.join(join);
                }
                None => g.step(pc, &inst),
            },
            Inst::Load { dst, addr, w } => {
                let (slow, join) = g.slow_path(pc, &inst);
                let a = g.ld(addr);
                let p = g.host(
                    a,
                    g.fk(addr),
                    Op::C(0),
                    Op::C(u64::from(w.bytes())),
                    false,
                    slow,
                );
                let nf = g.gf;
                match w {
                    Width::Sig => {
                        let lo = g.b.ins().load(I64, nf, p, 0);
                        let hi = g.b.ins().load(I64, nf, p, 8);
                        g.st(dst, lo);
                        g.st(Reg(dst.0 + 1), hi);
                    }
                    Width::B1 => {
                        let v = g.b.ins().uload8(I64, nf, p, 0);
                        g.st(dst, v);
                    }
                    Width::B4 => {
                        let v = g.b.ins().sload32(nf, p, 0);
                        g.st(dst, v);
                    }
                    Width::B8 => {
                        let v = g.b.ins().load(I64, nf, p, 0);
                        g.st(dst, v);
                    }
                }
                g.join(join);
            }
            Inst::Store { addr, src, w } => {
                let (slow, join) = g.slow_path(pc, &inst);
                let a = g.ld(addr);
                let p = g.host(
                    a,
                    g.fk(addr),
                    Op::C(0),
                    Op::C(u64::from(w.bytes())),
                    true,
                    slow,
                );
                let nf = g.gf;
                match w {
                    Width::Sig => {
                        let (lo, hi) = (g.ld(src), g.ld(Reg(src.0 + 1)));
                        g.b.ins().store(nf, lo, p, 0);
                        g.b.ins().store(nf, hi, p, 8);
                    }
                    Width::B1 => {
                        let v = g.ld(src);
                        g.b.ins().istore8(nf, v, p, 0);
                    }
                    Width::B4 => {
                        let v = g.ld(src);
                        g.b.ins().istore32(nf, v, p, 0);
                    }
                    Width::B8 => {
                        let v = g.ld(src);
                        g.b.ins().store(nf, v, p, 0);
                    }
                }
                g.join(join);
            }
            Inst::Copy { dst, src, bytes } => {
                let (slow, join) = g.slow_path(pc, &inst);
                let n = g.ld(bytes);
                let (sa, da) = (g.ld(src), g.ld(dst));
                let ps = g.host(sa, g.fk(src), Op::C(0), Op::V(n), false, slow);
                let pd = g.host(da, g.fk(dst), Op::C(0), Op::V(n), true, slow);
                g.b.ins().call(g.imp.mov, &[pd, ps, n]);
                g.join(join);
            }
            Inst::DopeLen { dst, dope, dim } => {
                let (slow, join) = g.slow_path(pc, &inst);
                let d = g.ld(dope);
                let (n, fits) = g.dim_len(d, u64::from(dim) * 24, slow);
                // Lengths above i64::MAX are the interpreter's error.
                let nn = g.b.ins().icmp_imm_s(IntCC::SignedGreaterThanOrEqual, n, 0);
                let okv = g.b.ins().band(fits, nn);
                g.guard(okv, slow);
                g.st(dst, n);
                g.join(join);
            }
            Inst::SubtypeCheck {
                dope, want, dims, ..
            } => {
                let (slow, join) = g.slow_path(pc, &inst);
                let (d, w) = (g.ld(dope), g.ld(want));
                for k in 0..u64::from(dims) {
                    let (a, oka) = g.dim_len(d, k * 24, slow);
                    let (b, okb) = g.dim_len(w, k * 24, slow);
                    let eq = g.b.ins().icmp(IntCC::Equal, a, b);
                    let okv = g.b.ins().band(oka, okb);
                    let okv = g.b.ins().band(okv, eq);
                    g.guard(okv, slow);
                }
                g.join(join);
            }
            Inst::Pos {
                dst,
                dope,
                dim,
                idx,
                ..
            } => {
                let (slow, join) = g.slow_path(pc, &inst);
                let a = g.ld(dope);
                let p = g.host(
                    a,
                    g.fk(dope),
                    Op::C(u64::from(dim) * 24),
                    Op::C(24),
                    false,
                    slow,
                );
                let (l, r, dir) = g.dope_words(p);
                let i = g.ld(idx);
                let le = |g: &mut Gen, a, b| g.b.ins().icmp(IntCC::SignedLessThanOrEqual, a, b);
                let (d0, d1) = (
                    g.b.ins().icmp_imm_s(IntCC::Equal, dir, 0),
                    g.b.ins().icmp_imm_s(IntCC::Equal, dir, 1),
                );
                let (x, y, u, v) = (
                    le(&mut g, l, i),
                    le(&mut g, i, r),
                    le(&mut g, r, i),
                    le(&mut g, i, l),
                );
                let (xy, uv) = (g.b.ins().band(x, y), g.b.ins().band(u, v));
                let up = g.b.ins().band(d0, xy);
                let dn = g.b.ins().band(d1, uv);
                let any = g.b.ins().bor(up, dn);
                g.guard(any, slow);
                let (a, b) = (g.b.ins().isub(i, l), g.b.ins().isub(l, i));
                let v = g.b.ins().select(up, a, b);
                g.st(dst, v);
                g.join(join);
            }
            Inst::Field {
                dst,
                base,
                layout,
                n,
            } => {
                let (slow, join) = g.slow_path(pc, &inst);
                let a = g.ld(layout);
                let p = g.host(
                    a,
                    g.fk(layout),
                    Op::C(16 + 8 * u64::from(n)),
                    Op::C(8),
                    false,
                    slow,
                );
                let off = g.b.ins().load(I64, g.gf, p, 0);
                let a = g.ld(base);
                let lo = g.b.ins().band_imm_s(a, 0xffff_ffff);
                let (sum, o) = g.b.ins().uadd_overflow(lo, off);
                let big =
                    g.b.ins()
                        .icmp_imm_s(IntCC::UnsignedGreaterThan, sum, 0xffff_ffff);
                let bad = g.b.ins().bor(o, big);
                let good = g.b.ins().bxor_imm_s(bad, 1);
                g.guard(good, slow);
                let hi = g.b.ins().band_imm_s(a, !0xffff_ffff_i64);
                let v = g.b.ins().bor(hi, sum);
                g.st(dst, v);
                g.join(join);
            }
            Inst::LGet { dst, base, idx } => {
                let (slow, join) = g.slow_path(pc, &inst);
                let (p, sh) = g.logic_word(base, idx, false, slow);
                let mut code = g.c64(0);
                for k in 0..4 {
                    let w = g.b.ins().load(I64, g.gf, p, k * 8);
                    let bit = g.b.ins().ushr(w, sh);
                    let bit = g.b.ins().band_imm_s(bit, 1);
                    let bit = g.b.ins().ishl_imm_s(bit, i64::from(k));
                    code = g.b.ins().bor(code, bit);
                }
                g.st(dst, code);
                g.join(join);
            }
            Inst::LSet { base, idx, v } => {
                let (slow, join) = g.slow_path(pc, &inst);
                let code = g.ld(v);
                // A code beyond 'U' to '-' is the interpreter's error.
                let okc =
                    g.b.ins()
                        .icmp_imm_s(IntCC::UnsignedLessThanOrEqual, code, 8);
                g.guard(okc, slow);
                let (p, sh) = g.logic_word(base, idx, true, slow);
                let one = g.c64(1);
                let one = g.b.ins().ishl(one, sh);
                let keep = g.b.ins().bnot(one);
                for k in 0..4 {
                    let w = g.b.ins().load(I64, g.gf, p, k * 8);
                    let w = g.b.ins().band(w, keep);
                    let bit = g.b.ins().ushr_imm_s(code, i64::from(k));
                    let bit = g.b.ins().band_imm_s(bit, 1);
                    let bit = g.b.ins().ishl(bit, sh);
                    let w = g.b.ins().bor(w, bit);
                    g.b.ins().store(g.gf, w, p, k * 8);
                }
                g.join(join);
            }
            Inst::SigVal {
                dst,
                sig,
                offset,
                len,
            } => {
                // `Machine::sig_val`: element range within u32, an instance signal.
                let (slow, join) = g.slow_path(pc, &inst);
                let (o, l) = (g.ld(offset), g.ld(len));
                let max = i64::from(u32::MAX);
                let n = g.mem_ld(MEM_NSIGS);
                let a = g.b.ins().icmp_imm_s(IntCC::UnsignedLessThanOrEqual, o, max);
                let b = g.b.ins().icmp_imm_s(IntCC::UnsignedLessThanOrEqual, l, max);
                let s = g.c64(i64::from(sig));
                let has = g.b.ins().icmp(IntCC::UnsignedLessThan, s, n);
                let okv = g.b.ins().band(a, b);
                let okv = g.b.ins().band(okv, has);
                g.guard(okv, slow);
                let v = g.b.ins().bor_imm_s(o, i64::from(sig) << 32);
                g.st(dst, v);
                g.st(Reg(dst.0 + 1), l);
                g.join(join);
            }
            Inst::MakeDope { at, parts } => {
                let rs = f.regs(parts).ok_or("operand list out of range")?;
                let (slow, join) = g.slow_path(pc, &inst);
                let a = g.ld(at);
                let p = g.host(
                    a,
                    g.fk(at),
                    Op::C(0),
                    Op::C(rs.len() as u64 * 8),
                    true,
                    slow,
                );
                for (k, &r) in rs.iter().enumerate() {
                    let v = g.ld(r);
                    g.b.ins().store(g.gf, v, p, k as i32 * 8);
                }
                g.join(join);
            }
            Inst::LCopy {
                dst,
                dst_off,
                src,
                src_off,
                n,
            } => {
                let (slow, join) = g.slow_path(pc, &inst);
                let (k, so, d) = (g.ld(n), g.ld(src_off), g.ld(dst_off));
                // Nothing to copy: no memory is touched, as in the interpreter.
                let go = g.b.create_block();
                let z = g.b.ins().icmp_imm_s(IntCC::Equal, k, 0);
                g.b.ins().brif(z, join, &[], go, &[]);
                g.b.switch_to_block(go);
                let small =
                    |g: &mut Gen, v| g.b.ins().icmp_imm_s(IntCC::UnsignedLessThan, v, 1 << 32);
                let (sk, ss, sd) = (small(&mut g, k), small(&mut g, so), small(&mut g, d));
                let okv = g.b.ins().band(sk, ss);
                let okv = g.b.ins().band(okv, sd);
                g.guard(okv, slow);
                // First block, block count, first element in it.
                let blocks = |g: &mut Gen, off| {
                    let b0 = g.b.ins().ushr_imm_s(off, 6);
                    let last = g.b.ins().iadd(off, k);
                    let last = g.b.ins().iadd_imm_s(last, -1);
                    let b1 = g.b.ins().ushr_imm_s(last, 6);
                    let nb = g.b.ins().isub(b1, b0);
                    let nb = g.b.ins().iadd_imm_s(nb, 1);
                    let first = g.b.ins().ishl_imm_s(b0, 6);
                    (b0, nb, g.b.ins().isub(off, first))
                };
                let (sb0, ns, s0) = blocks(&mut g, so);
                let (db0, nd, d0) = blocks(&mut g, d);
                let a =
                    g.b.ins()
                        .icmp_imm_s(IntCC::UnsignedLessThanOrEqual, ns, LCOPY_BLOCKS as i64);
                let b =
                    g.b.ins()
                        .icmp_imm_s(IntCC::UnsignedLessThanOrEqual, nd, LCOPY_BLOCKS as i64);
                let fit = g.b.ins().band(a, b);
                g.guard(fit, slow);
                let (sb, sn) = (g.b.ins().ishl_imm_s(sb0, 5), g.b.ins().ishl_imm_s(ns, 5));
                let (db, dn) = (g.b.ins().ishl_imm_s(db0, 5), g.b.ins().ishl_imm_s(nd, 5));
                let (sa, da) = (g.ld(src), g.ld(dst));
                let ps = g.host(sa, g.fk(src), Op::V(sb), Op::V(sn), false, slow);
                let pd = g.host(da, g.fk(dst), Op::V(db), Op::V(dn), true, slow);
                g.b.ins().call(g.imp.lcopy, &[pd, nd, d0, ps, ns, s0, k]);
                g.join(join);
            }
            Inst::Call { func, args, dst } => {
                let nr = g.imp.natives.get(&func).copied();
                let rs = f.regs(args);
                match (nr, rs) {
                    (Some(nr), Some(rs)) if nr.nparams == rs.len() && nr.ret == dst.is_some() => {
                        g.native_call(pc, &inst, nr, rs, dst, None)
                    }
                    _ => g.step(pc, &inst),
                }
            }
            Inst::CallProt {
                func,
                obj,
                args,
                dst,
            } => {
                let nr = g.imp.natives.get(&func).copied();
                let rs = f.regs(args);
                match (nr, rs) {
                    (Some(nr), Some(rs))
                        if nr.nparams == rs.len() + 1
                            && nr.ret == dst.is_some()
                            && obj.0 < f.reg_ty.len() as u32 =>
                    {
                        g.native_call(pc, &inst, nr, rs, dst, Some(obj))
                    }
                    _ => g.step(pc, &inst),
                }
            }
            Inst::CondBr { c: r, t, f: e } => {
                let x = g.ld(r);
                let (tb, eb) = (block_bb(t)?, block_bb(e)?);
                g.b.ins().brif(x, tb, &[], eb, &[]);
                open = false;
            }
            Inst::Br { to } => {
                let tb = block_bb(to)?;
                g.b.ins().jump(tb, &[]);
                open = false;
            }
            Inst::Case { v, table } => {
                let t = prog
                    .cases
                    .get(table)
                    .ok_or_else(|| format!("no case table {}", table.0))?;
                let x = g.ld(v);
                // ponytail: a compare chain per arm; a Switch for dense tables if case-heavy
                // code ever shows in profiles.
                for &(lo, hi, to) in &t.arms {
                    let ge = g.b.ins().icmp_imm_s(IntCC::SignedGreaterThanOrEqual, x, lo);
                    let le = g.b.ins().icmp_imm_s(IntCC::SignedLessThanOrEqual, x, hi);
                    let hit = g.b.ins().band(ge, le);
                    let next = g.b.create_block();
                    let tb = block_bb(to)?;
                    g.b.ins().brif(hit, tb, &[], next, &[]);
                    g.b.switch_to_block(next);
                }
                let d = block_bb(t.default)?;
                g.b.ins().jump(d, &[]);
                open = false;
            }
            Inst::SigReadScalar { dst, sig } => g.sig_read_scalar(pc, dst, sig),
            Inst::Edge { dst, kind, sig } => g.edge(pc, kind, sig, dst, 0),
            Inst::SigAttr {
                dst,
                kind: yavs_ir::AttrKind::Event,
                sig,
            } if g.nat.is_none() => g.sig_event(pc, dst, sig),
            Inst::Drive {
                driver,
                target: Some(t),
                wave,
                n,
                mode,
                reject: None,
            } if g.nat.is_none() => g.drive_block(pc, driver, t, wave, n, mode),
            Inst::WaitOn { sig }
                if g.nat.is_none() && matches!(f.code.get(i + 1), Some(Inst::Suspend { .. })) =>
            {
                g.wait_suspend(pc, sig)
            }
            Inst::WaitOn { sig } if g.nat.is_none() => g.wait_on(pc, sig),
            Inst::SigRead { dst, sig } => g.sig_read(pc, dst, sig),
            Inst::WaitOn { .. }
            | Inst::WaitFor { .. }
            | Inst::DriveSlot { .. }
            | Inst::Drive { .. }
            | Inst::Suspend { .. } => g.signal(pc, 0),
            Inst::CallNative { native, args, dst } => {
                match native_kind(prog, f, native, args, dst) {
                    Some(NativeKind::Table(t, rs, d)) => {
                        let (slow, join) = g.slow_path(pc, &inst);
                        let mut idx = g.c64(0);
                        for r in rs {
                            // A code beyond the nine is the native's error.
                            let x = g.ld(r);
                            let okc = g.b.ins().icmp_imm_s(IntCC::UnsignedLessThanOrEqual, x, 8);
                            g.guard(okc, slow);
                            let i9 = g.b.ins().imul_imm_s(idx, 9);
                            idx = g.b.ins().iadd(i9, x);
                        }
                        let gv = *ok(g.imp.tables.get(&t), "table native without data")?;
                        let base = g.b.ins().symbol_value(I64, gv);
                        let p = g.b.ins().iadd(base, idx);
                        let v = g.b.ins().uload8(I64, g.gf, p, 0);
                        g.st(d, v);
                        g.join(join);
                    }
                    Some(NativeKind::Edge(rising)) => {
                        let e = if rising { 1 } else { 2 };
                        match (f.regs(args), dst) {
                            (Some(&[s]), Some(d)) => {
                                let kind = if rising {
                                    yavs_ir::EdgeKind::Rising
                                } else {
                                    yavs_ir::EdgeKind::Falling
                                };
                                g.edge(pc, kind, s, d, e)
                            }
                            _ => g.signal(pc, e),
                        }
                    }
                    None if frame_direct(prog, f, &inst) => {
                        g.native_direct(pc, &inst, f, native, args, dst)?
                    }
                    // Cold natives leave a native frame for the interpreter: what they do.
                    None if g.nat.is_some()
                        && prog
                            .natives
                            .get(native)
                            .is_some_and(|n| yavs_interp::cold_native(&n.symbol)) =>
                    {
                        g.deopt_here(pc)
                    }
                    None => g.step(pc, &inst),
                }
            }
            _ if g.nat.is_some() && xstep_wide(f, &inst) => g.native_step(pc, &inst, f),
            _ => g.step(pc, &inst),
        }
    }
    if let Some(why) = g.bad.take() {
        return Err(format!("no native frame: {why}"));
    }
    if open {
        // Falling off the end: the interpreter reports it.
        let v = g.c32(n as u32);
        g.leave(v);
    }
    // The slow paths, after the body.
    let slows = std::mem::take(&mut g.slows);
    for s in slows {
        g.b.set_cold_block(s.block);
        g.b.switch_to_block(s.block);
        g.clear();
        if s.endstmt && g.nat.is_some() {
            let base = g.nat_temp_base();
            g.b.ins().call(g.imp.endstmt, &[g.env, g.mem, base]);
            g.b.ins().jump(s.join, &[]);
            continue;
        }
        if g.nat.is_some() {
            // The interpreter executes the instruction in the materialised frame.
            let v = g.c32(s.pc);
            g.leave(v);
            continue;
        }
        if let Some(e) = s.edge {
            let (p, ev) = (g.c32(s.pc), g.c32(e));
            g.call_go(g.imp.signal, &[p, ev], false);
            g.b.ins().jump(s.join, &[]);
            continue;
        }
        let at = if s.keep { s.pc | KEEP } else { s.pc };
        let pcv = g.c32(at);
        g.call_go(g.imp.step, &[pcv], false);
        g.b.ins().jump(s.join, &[]);
    }
    // The exit block.
    g.b.switch_to_block(g.exit);
    if g.nat.is_some() {
        g.record_frame(f, exit_pc);
        let (one, zero) = (g.c32(1), g.c64(0));
        g.b.ins().return_(&[one, zero]);
    } else {
        g.b.ins().return_(&[exit_pc]);
    }
    g.b.seal_all_blocks();
    g.b.finalize(cfg);
    Ok(g.stepped)
}

impl Gen<'_> {
    fn clear(&mut self) {
        self.cache.fill(None);
        self.slots = None;
    }

    /// The frame slot offset `r` holds the address of, if known.
    fn fk(&self, r: Reg) -> Option<u64> {
        match self.known.get(r.0 as usize) {
            Some(Some(Known::Frame(o))) => Some(*o),
            _ => None,
        }
    }

    fn op(&mut self, o: Op) -> Value {
        match o {
            Op::C(v) => self.c64(v as i64),
            Op::V(v) => v,
        }
    }

    /// The pointer to the frame's slots.
    fn slots(&mut self) -> Value {
        if let Some(n) = &self.nat {
            return n.slots;
        }
        if let Some(v) = self.slots {
            return v;
        }
        let v = self.mem_ld(MEM_SLOTS);
        self.slots = Some(v);
        v
    }

    fn c64(&mut self, v: i64) -> Value {
        self.b.ins().iconst(I64, v)
    }

    fn c32(&mut self, v: u32) -> Value {
        self.b.ins().iconst(I32, i64::from(v))
    }

    fn ld(&mut self, r: Reg) -> Value {
        if let Some(Some(Known::Int(v))) = self.known.get(r.0 as usize) {
            let v = *v;
            return self.c64(v);
        }
        if let Some(Some(v)) = self.cache.get(r.0 as usize) {
            return *v;
        }
        let v = self.b.ins().load(I64, self.rf, self.regs, r.0 as i32 * 8);
        self.cache[r.0 as usize] = Some(v);
        v
    }

    fn st(&mut self, r: Reg, v: Value) {
        self.b.ins().store(self.rf, v, self.regs, r.0 as i32 * 8);
        // A signal reference's second word may be written by the next instruction only.
        self.cache[r.0 as usize] = Some(v);
    }

    fn ldf(&mut self, r: Reg) -> Value {
        let v = self.ld(r);
        self.b.ins().bitcast(F64, MemFlagsData::new(), v)
    }

    fn stf(&mut self, r: Reg, v: Value) {
        let v = self.b.ins().bitcast(I64, MemFlagsData::new(), v);
        self.st(r, v)
    }

    fn mem_ld(&mut self, field: u64) -> Value {
        self.b.ins().load(I64, self.mf, self.mem, field as i32)
    }

    fn mem_st(&mut self, field: u64, v: Value) {
        self.b.ins().store(self.mf, v, self.mem, field as i32);
    }

    /// The prologue of a native-frame variant: zeroed registers and slots (as a frame the
    /// interpreter pushes), the arguments stored into their parameter slots.
    fn native_entry(&mut self, f: &Func, args: &[Value]) -> Result<(), String> {
        let (regs, slots) = match &self.nat {
            Some(n) => (n.nregs * 8, n.slots),
            None => return Ok(()),
        };
        // Callees reach these slots through their frame index (`Mem::nslots`).
        let depth = self.nat.as_ref().map_or(self.regs, |n| n.depth);
        let tab = self.mem_ld(MEM_NSLOTS);
        let eo = self.b.ins().imul_imm_s(depth, 16);
        let ent = self.b.ins().iadd(tab, eo);
        self.b.ins().store(MemFlagsData::trusted(), slots, ent, 0);
        let fl = self.c64(i64::from(f.frame));
        self.b.ins().store(MemFlagsData::trusted(), fl, ent, 8);
        let regs_ptr = self.regs;
        for (ptr, bytes, fl) in [
            (regs_ptr, regs, self.rf),
            (slots, u64::from(f.frame), self.gf),
        ] {
            if bytes <= 128 {
                let z = self.c64(0);
                for k in 0..bytes.div_ceil(8) {
                    self.b.ins().store(fl, z, ptr, (k * 8) as i32);
                }
            } else {
                let n = self.c64(bytes as i64);
                self.b.ins().call(self.imp.zero, &[ptr, n]);
            }
        }
        // The interpreter preloads the registers a constant or an address defines once.
        let mut defs = vec![0u8; f.reg_ty.len()];
        for inst in &f.code {
            inst.visit(&mut |r, role, _| {
                if role == yavs_ir::Role::Def
                    && let Some(d) = defs.get_mut(r.0 as usize)
                {
                    *d = d.saturating_add(1);
                }
            });
        }
        for inst in &f.code {
            match *inst {
                Inst::Const { dst, v } if defs[dst.0 as usize] == 1 => {
                    let x = self.c64(v);
                    self.st(dst, x);
                }
                Inst::ConstF { dst, v } if defs[dst.0 as usize] == 1 => {
                    let x = self.c64(v.to_bits() as i64);
                    self.st(dst, x);
                }
                Inst::AddrOf { dst, space, off } if defs[dst.0 as usize] == 1 => {
                    if let Some(a) = addr_const(space, off) {
                        let v = if space == AddrSpace::Frame {
                            let d = self.nat.as_ref().map_or(self.regs, |n| n.depth);
                            let hi = self.b.ins().bor_imm_s(d, i64::from(space::FRAME) << 29);
                            let hi = self.b.ins().ishl_imm_s(hi, 32);
                            self.b.ins().bor_imm_s(hi, a as i64)
                        } else {
                            self.c64(a as i64)
                        };
                        self.st(dst, v);
                    }
                }
                _ => {}
            }
        }
        for (p, &v) in f.params.iter().zip(args) {
            let off = p.slot as i32;
            match p.width {
                Width::B1 => {
                    self.b.ins().istore8(self.gf, v, slots, off);
                }
                Width::B4 => {
                    self.b.ins().istore32(self.gf, v, slots, off);
                }
                Width::B8 => {
                    self.b.ins().store(self.gf, v, slots, off);
                }
                Width::Sig => return Err("signal parameter".into()),
            }
        }
        Ok(())
    }

    /// Record the native frame at `pc` for the interpreter (`yavs_rt_deopt`): the callback copies
    /// its registers from the register array, its slots are copied to the buffer it hands out.
    fn record_frame(&mut self, f: &Func, pc: Value) {
        let Some(n) = &self.nat else { return };
        let (func, nregs, slots) = (n.func, n.nregs, n.slots);
        let tb = self.nat_temp_base();
        let fu = self.c32(func);
        let nr = self.c64(nregs as i64);
        let sl = self.c64(i64::from(f.frame));
        let regs = self.regs;
        self.b.ins().call(
            self.imp.deopt,
            &[self.env, self.mem, fu, pc, tb, nr, sl, regs],
        );
        if f.frame > 0 {
            let c = self.b.ins().call(self.imp.deopt_slots, &[self.env]);
            let sb = self.b.inst_results(c)[0];
            let fb = self.c64(i64::from(f.frame));
            self.b.ins().call(self.imp.mov, &[sb, slots, fb]);
        }
    }

    /// `Inst::Call` of a function that has a native-frame variant: a direct call one frame
    /// deeper, with no frame in the interpreter; the interpreter makes the call when the depth
    /// limit is near, and the frames are materialised when the callee cannot finish.
    fn native_call(
        &mut self,
        pc: u32,
        inst: &Inst,
        nr: NatRef,
        args: &[Reg],
        dst: Option<Reg>,
        obj: Option<Reg>,
    ) {
        let (slow, join) = self.slow_path(pc, inst);
        // A protected call: the process must be allowed the shared state (else the interpreter's
        // error), and the callee allocates in the shared heap when the object is there
        // (`Machine::call_prot`).
        let flags = self.mem_ld(MEM_FLAGS);
        if let Some(o) = obj {
            let serial = self
                .b
                .ins()
                .band_imm_s(flags, yavs_interp::TIER_SERIAL as i64);
            let allowed = self.b.ins().icmp_imm_s(IntCC::NotEqual, serial, 0);
            self.guard(allowed, slow);
            let ov = self.ld(o);
            let tag = self.b.ins().ushr_imm_s(ov, 56);
            let shared =
                self.b
                    .ins()
                    .icmp_imm_s(IntCC::Equal, tag, i64::from(yavs_interp::SHARED_TAG));
            let shared = self.b.ins().uextend(I64, shared);
            let inner = self.b.ins().bor(flags, shared);
            self.mem_st(MEM_FLAGS, inner);
        }
        let depth = match &self.nat {
            Some(n) => n.depth,
            None => self.mem_ld(MEM_FRAME_IDX),
        };
        let d1 = self.b.ins().iadd_imm_s(depth, 1);
        let max = self.mem_ld(MEM_MAX_DEPTH);
        let a = self.b.ins().icmp(IntCC::UnsignedLessThan, d1, max);
        let b2 = self
            .b
            .ins()
            .icmp_imm_s(IntCC::UnsignedLessThan, d1, 1 << 29);
        let okc = self.b.ins().band(a, b2);
        // Native stack is left (`NEST_STACK`).
        let sp = self.b.ins().get_stack_pointer(I64);
        let lo = self.mem_ld(MEM_STACK_LO);
        let room = self.b.ins().icmp(IntCC::UnsignedGreaterThan, sp, lo);
        let okc = self.b.ins().band(okc, room);
        self.guard(okc, slow);
        let mut cargs = vec![self.env, self.mem, d1];
        if let Some(o) = obj {
            cargs.push(self.ld(o));
        }
        for &r in args {
            cargs.push(self.ld(r));
        }
        let call = self.b.ins().call(nr.fr, &cargs);
        let (status, val) = {
            let rs = self.b.inst_results(call);
            (rs[0], rs[1])
        };
        if obj.is_some() {
            self.mem_st(MEM_FLAGS, flags);
        }
        let (out, cont) = (self.b.create_block(), self.b.create_block());
        self.b.set_cold_block(out);
        self.b.ins().brif(status, out, &[], cont, &[]);
        self.b.switch_to_block(out);
        let pcv = self.c32(pc);
        if self.nat.is_some() {
            // This frame is recorded too, at its call.
            self.leave(pcv);
        } else {
            let c = self
                .b
                .ins()
                .call(self.imp.deopt_end, &[self.env, self.mem, pcv]);
            let r = self.b.inst_results(c)[0];
            self.leave(r);
        }
        self.b.switch_to_block(cont);
        if let (Some(d), true) = (dst, nr.ret) {
            self.st(d, val);
        }
        self.join(join);
    }

    /// Return `v` (an i32): a jump to the function's exit block.
    fn leave(&mut self, v: Value) {
        self.b.ins().jump(self.exit, &[BlockArg::Value(v)]);
    }

    /// Leave to the interpreter at `pc` when `bad` holds; code goes on after it.
    fn exit_if(&mut self, bad: Value, pc: u32) {
        let out = self.b.create_block();
        let cont = self.b.create_block();
        self.b.set_cold_block(out);
        self.b.ins().brif(bad, out, &[], cont, &[]);
        self.b.switch_to_block(out);
        let v = self.c32(pc);
        self.leave(v);
        self.b.switch_to_block(cont);
    }

    fn range_exit(&mut self, x: Value, lo: Value, hi: Value, pc: u32) {
        let a = self.b.ins().icmp(IntCC::SignedLessThan, x, lo);
        let b = self.b.ins().icmp(IntCC::SignedGreaterThan, x, hi);
        let bad = self.b.ins().bor(a, b);
        self.exit_if(bad, pc);
    }

    /// `x op y` as `yavs_std::arith`, leaving at `pc` wherever it reports an error. `None` for
    /// `**`, which goes through the interpreter.
    fn int_op(&mut self, op: IntOp, x: Value, y: Value, pc: u32) -> Option<Value> {
        Some(match op {
            IntOp::Add => {
                let (v, o) = self.b.ins().sadd_overflow(x, y);
                self.exit_if(o, pc);
                v
            }
            IntOp::Sub => {
                let (v, o) = self.b.ins().ssub_overflow(x, y);
                self.exit_if(o, pc);
                v
            }
            IntOp::Mul => {
                let (v, o) = self.b.ins().smul_overflow(x, y);
                self.exit_if(o, pc);
                v
            }
            IntOp::Div | IntOp::Rem | IntOp::Mod => {
                let zero = self.b.ins().icmp_imm_s(IntCC::Equal, y, 0);
                let m1 = self.b.ins().icmp_imm_s(IntCC::Equal, y, -1);
                if op == IntOp::Div {
                    let min = self.b.ins().icmp_imm_s(IntCC::Equal, x, i64::MIN);
                    let o = self.b.ins().band(min, m1);
                    let bad = self.b.ins().bor(zero, o);
                    self.exit_if(bad, pc);
                    return Some(self.b.ins().sdiv(x, y));
                }
                self.exit_if(zero, pc);
                // x rem -1 is 0; srem could trap on MIN rem -1.
                let one = self.c64(1);
                let ys = self.b.ins().select(m1, one, y);
                let r = self.b.ins().srem(x, ys);
                if op == IntOp::Rem {
                    return Some(r);
                }
                let nz = self.b.ins().icmp_imm_s(IntCC::NotEqual, r, 0);
                let rn = self.b.ins().icmp_imm_s(IntCC::SignedLessThan, r, 0);
                let yn = self.b.ins().icmp_imm_s(IntCC::SignedLessThan, y, 0);
                let diff = self.b.ins().bxor(rn, yn);
                let fix = self.b.ins().band(nz, diff);
                let sum = self.b.ins().iadd(r, y);
                self.b.ins().select(fix, sum, r)
            }
            IntOp::Pow => return None,
        })
    }

    /// Call a callback (the arguments after env, regs, mem) that returns `GO` or what the
    /// compiled function returns at once; code goes on when it returned `GO`.
    fn call_go(&mut self, f: FuncRef, rest: &[Value], cont_here: bool) {
        let mut args = vec![self.env, self.regs, self.mem];
        args.extend_from_slice(rest);
        let call = self.b.ins().call(f, &args);
        let r = self.b.inst_results(call)[0];
        self.clear();
        let cont = self.b.create_block();
        let go = self
            .b
            .ins()
            .icmp_imm_s(IntCC::Equal, r, i64::from(GO as i32));
        self.b
            .ins()
            .brif(go, cont, &[], self.exit, &[BlockArg::Value(r)]);
        self.b.switch_to_block(cont);
        let _ = cont_here;
    }

    /// Execute the instruction `i` at `pc` in the interpreter.
    fn step(&mut self, pc: u32, i: &Inst) {
        if self.nat.is_some() {
            self.bad = Some(format!("{i:?}"));
            return;
        }
        let d = format!("{i:?}");
        self.stepped
            .push(d.split([' ', '{', '(']).next().unwrap_or("").to_string());
        if via_signal(i) {
            let (p, e) = (self.c32(pc), self.c32(0));
            self.call_go(self.imp.signal, &[p, e], true);
            return;
        }
        let at = if keeps_objects(i) { pc | KEEP } else { pc };
        let pcv = self.c32(at);
        self.call_go(self.imp.step, &[pcv], true);
    }

    /// The signal instruction at `pc` through `yavs_rt_signal`.
    fn signal(&mut self, pc: u32, edge: u32) {
        if self.nat.is_some() {
            self.bad = Some("signal operation".into());
            return;
        }
        let (p, e) = (self.c32(pc), self.c32(edge));
        self.call_go(self.imp.signal, &[p, e], true);
    }

    /// A block that steps the instruction at `pc` in the interpreter and goes on at the
    /// returned join block (emitted after the body).
    fn slow_path(&mut self, pc: u32, i: &Inst) -> (Block, Block) {
        let (block, join) = (self.b.create_block(), self.b.create_block());
        self.slows.push(Slow {
            block,
            join,
            pc,
            keep: keeps_objects(i),
            edge: via_signal(i).then_some(0),
            endstmt: false,
        });
        (block, join)
    }

    /// Go on in a new block when `okc`, else take `slow`.
    fn guard(&mut self, okc: Value, slow: Block) {
        let go = self.b.create_block();
        self.b.ins().brif(okc, go, &[], slow, &[]);
        self.b.switch_to_block(go);
    }

    /// End the fast path at the slow path's join block.
    fn join(&mut self, join: Block) {
        self.b.ins().jump(join, &[]);
        self.b.switch_to_block(join);
        self.clear();
    }

    /// The host pointer for `n` bytes at address `a` plus `extra`: the frame's slots, the
    /// temporary area and (reading) the instance data inline as `Mem::find`, other memory
    /// through `yavs_rt_host`; takes `slow` when the access is the interpreter's.
    fn host(
        &mut self,
        a: Value,
        frame: Option<u64>,
        extra: Op,
        n: Op,
        write: bool,
        slow: Block,
    ) -> Value {
        if let Some(o) = frame {
            // Within the frame whatever the frame: no check.
            if let (Op::C(e), Op::C(k)) = (extra, n)
                && o.checked_add(e)
                    .and_then(|x| x.checked_add(k))
                    .is_some_and(|end| end <= self.frame_len)
            {
                let slots = self.slots();
                return self.b.ins().iadd_imm_s(slots, (o + e) as i64);
            }
            let (ev, nv) = (self.op(extra), self.op(n));
            // `extra` is below 2^63, the sum cannot wrap.
            let off = self.b.ins().iadd_imm_s(ev, o as i64);
            let len = if self.nat.is_some() {
                self.c64(self.frame_len as i64)
            } else {
                self.mem_ld(MEM_SLOTS_LEN)
            };
            let fits = self.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, nv, len);
            let rest = self.b.ins().isub(len, nv);
            let inside = self.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, off, rest);
            let okc = self.b.ins().band(fits, inside);
            self.guard(okc, slow);
            let slots = self.slots();
            return self.b.ins().iadd(slots, off);
        }
        let (extra, n) = (self.op(extra), self.op(n));
        let hi = self.b.ins().ushr_imm_s(a, 32);
        let lo = self.b.ins().band_imm_s(a, 0xffff_ffff);
        let off = self.b.ins().iadd(lo, extra);
        let frame_hi = self.mem_ld(MEM_FRAME);
        let is_f = self.b.ins().icmp(IntCC::Equal, hi, frame_hi);
        let is_t = self
            .b
            .ins()
            .icmp_imm_s(IntCC::Equal, hi, i64::from(space::TEMP) << 29);
        let (slots, slots_len) = (self.mem_ld(MEM_SLOTS), self.mem_ld(MEM_SLOTS_LEN));
        let (temp, temp_len) = (self.mem_ld(MEM_TEMP), self.mem_ld(MEM_TEMP_LEN));
        let (base, len, known) = if write {
            let base = self.b.ins().select(is_f, slots, temp);
            let len = self.b.ins().select(is_f, slots_len, temp_len);
            (base, len, self.b.ins().bor(is_f, is_t))
        } else {
            let is_i = self
                .b
                .ins()
                .icmp_imm_s(IntCC::Equal, hi, i64::from(space::INST) << 29);
            let (inst, inst_len) = (self.mem_ld(MEM_INST), self.mem_ld(MEM_INST_LEN));
            let b = self.b.ins().select(is_t, temp, inst);
            let l = self.b.ins().select(is_t, temp_len, inst_len);
            let base = self.b.ins().select(is_f, slots, b);
            let len = self.b.ins().select(is_f, slots_len, l);
            let k = self.b.ins().bor(is_f, is_t);
            (base, len, self.b.ins().bor(k, is_i))
        };
        let fits = self.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, n, len);
        let rest = self.b.ins().isub(len, n);
        let inside = self.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, off, rest);
        let k2 = self.b.ins().band(fits, inside);
        let known = self.b.ins().band(known, k2);
        let (fast, call, done) = (
            self.b.create_block(),
            self.b.create_block(),
            self.b.create_block(),
        );
        let p = self.b.append_block_param(done, I64);
        self.b.ins().brif(known, fast, &[], call, &[]);
        self.b.switch_to_block(fast);
        let p1 = self.b.ins().iadd(base, off);
        self.b.ins().jump(done, &[BlockArg::Value(p1)]);
        self.b.switch_to_block(call);
        if self.nat.is_some() {
            // A native frame's own slots, and the slots of the native frames below it, registered
            // at their entry in `Mem::nslots` (a frame above the interpreter's top frame and
            // below this one).
            let (depth, slots) = self.nat.as_ref().map(|n| (n.depth, n.slots)).expect("nat");
            let flen = self.c64(self.frame_len as i64);
            let (own, other, up, rest) = (
                self.b.create_block(),
                self.b.create_block(),
                self.b.create_block(),
                self.b.create_block(),
            );
            let is_own = self.b.ins().icmp(IntCC::Equal, hi, depth);
            self.b.ins().brif(is_own, own, &[], other, &[]);
            self.b.switch_to_block(own);
            let fits = self.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, n, flen);
            let orest = self.b.ins().isub(flen, n);
            let inside = self
                .b
                .ins()
                .icmp(IntCC::UnsignedLessThanOrEqual, off, orest);
            let okc = self.b.ins().band(fits, inside);
            self.guard(okc, slow);
            let pown = self.b.ins().iadd(slots, off);
            self.b.ins().jump(done, &[BlockArg::Value(pown)]);
            self.b.switch_to_block(other);
            let top = self.mem_ld(MEM_FRAME_IDX);
            let above = self.b.ins().icmp(IntCC::UnsignedGreaterThan, hi, top);
            let below = self.b.ins().icmp(IntCC::UnsignedLessThan, hi, depth);
            let between = self.b.ins().band(above, below);
            self.b.ins().brif(between, up, &[], rest, &[]);
            self.b.switch_to_block(up);
            let tab = self.mem_ld(MEM_NSLOTS);
            let eo = self.b.ins().imul_imm_s(hi, 16);
            let ent = self.b.ins().iadd(tab, eo);
            let sp = self.b.ins().load(I64, MemFlagsData::trusted(), ent, 0);
            let len = self.b.ins().load(I64, MemFlagsData::trusted(), ent, 8);
            let fits = self.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, n, len);
            let urest = self.b.ins().isub(len, n);
            let inside = self
                .b
                .ins()
                .icmp(IntCC::UnsignedLessThanOrEqual, off, urest);
            let okc = self.b.ins().band(fits, inside);
            self.guard(okc, slow);
            let pup = self.b.ins().iadd(sp, off);
            self.b.ins().jump(done, &[BlockArg::Value(pup)]);
            self.b.switch_to_block(rest);
        }
        // The object's entry of `Mem::objs` (the objects found before: heap objects a `Deref`
        // found, a process's heap variables): one load and compare, as the LLVM generator does;
        // else `yavs_rt_host` (`Mem::find`, then the interpreter is asked for the object).
        let flag = self.b.ins().ushr_imm_s(a, 32 + i64::from(KEY_SHIFT_ADDR));
        let flag = self.b.ins().band_imm_s(flag, KEY_FLAG as i64);
        let key = self.b.ins().bxor(hi, flag);
        let key = self.b.ins().band_imm_s(key, OBJS as i64 - 1);
        let eo = self.b.ins().imul_imm_s(key, OBJ_SIZE as i64);
        let ent = self.b.ins().iadd(self.mem, eo);
        let ehi = self.b.ins().load(I64, self.mf, ent, MEM_OBJS as i32);
        let mut hit = self.b.ins().icmp(IntCC::Equal, ehi, hi);
        if write {
            let w = self
                .b
                .ins()
                .load(I64, self.mf, ent, (MEM_OBJS + OBJ_W) as i32);
            let w = self.b.ins().icmp_imm_s(IntCC::NotEqual, w, 0);
            hit = self.b.ins().band(hit, w);
        }
        let optr = self
            .b
            .ins()
            .load(I64, self.mf, ent, (MEM_OBJS + OBJ_PTR) as i32);
        let olen = self
            .b
            .ins()
            .load(I64, self.mf, ent, (MEM_OBJS + OBJ_LEN) as i32);
        let ofits = self.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, n, olen);
        let orest = self.b.ins().isub(olen, n);
        let oinside = self
            .b
            .ins()
            .icmp(IntCC::UnsignedLessThanOrEqual, off, orest);
        let ok = self.b.ins().band(ofits, oinside);
        let ok = self.b.ins().band(hit, ok);
        let (obj, miss) = (self.b.create_block(), self.b.create_block());
        self.b.set_cold_block(miss);
        self.b.ins().brif(ok, obj, &[], miss, &[]);
        self.b.switch_to_block(obj);
        let p3 = self.b.ins().iadd(optr, off);
        self.b.ins().jump(done, &[BlockArg::Value(p3)]);
        self.b.switch_to_block(miss);
        let w = self.c32(u32::from(write));
        let c = self
            .b
            .ins()
            .call(self.imp.host, &[self.env, self.mem, a, extra, n, w]);
        let p2 = self.b.inst_results(c)[0];
        self.b.ins().jump(done, &[BlockArg::Value(p2)]);
        self.b.switch_to_block(done);
        let nn = self.b.ins().icmp_imm_s(IntCC::NotEqual, p, 0);
        self.guard(nn, slow);
        p
    }

    /// `left, right, dir` of the descriptor dimension at `p`.
    fn dope_words(&mut self, p: Value) -> (Value, Value, Value) {
        let nf = self.gf;
        (
            self.b.ins().load(I64, nf, p, 0),
            self.b.ins().load(I64, nf, p, 8),
            self.b.ins().load(I64, nf, p, 16),
        )
    }

    /// The length of the descriptor dimension at `dope + off` as `yavs_ir::dim_len`, and
    /// whether it fits a u64 (`dim_len` is not `None`).
    fn dim_len(&mut self, dope: Value, off: u64, slow: Block) -> (Value, Value) {
        let p = self.host(dope, None, Op::C(off), Op::C(24), false, slow);
        let (l, r, dir) = self.dope_words(p);
        self.len_of(l, r, dir)
    }

    /// `yavs_ir::dim_len(l, r, dir == 1)` and whether it is not `None`.
    fn len_of(&mut self, l: Value, r: Value, dir: Value) -> (Value, Value) {
        let down = self.b.ins().icmp_imm_s(IntCC::Equal, dir, 1);
        let lo = self.b.ins().select(down, r, l);
        let hi = self.b.ins().select(down, l, r);
        let empty = self.b.ins().icmp(IntCC::SignedLessThan, hi, lo);
        // hi - lo is exact as a u64 when hi >= lo; + 1 wraps only for the full i64 range.
        let d = self.b.ins().isub(hi, lo);
        let n = self.b.ins().iadd_imm_s(d, 1);
        let nz = self.b.ins().icmp_imm_s(IntCC::NotEqual, n, 0);
        let fits = self.b.ins().bor(empty, nz);
        let zero = self.c64(0);
        (self.b.ins().select(empty, zero, n), fits)
    }

    /// The 32-byte bit-plane block holding element `idx` of the logic array at `base`, and the
    /// element's bit position in each plane word.
    fn logic_word(&mut self, base: Reg, idx: Reg, write: bool, slow: Block) -> (Value, Value) {
        let i = self.ld(idx);
        let blk = self.b.ins().ushr_imm_s(i, 6);
        let off = self.b.ins().ishl_imm_s(blk, 5);
        let a = self.ld(base);
        let p = self.host(a, self.fk(base), Op::V(off), Op::C(32), write, slow);
        (p, self.b.ins().band_imm_s(i, 63))
    }

    /// `n` zeroed bytes of temporary memory as `Machine::temp_alloc` makes them: the address,
    /// taking `slow` when the area must grow or exceeds 4 GiB.
    fn temp_alloc(&mut self, n: Value, slow: Block) -> (Value, Value) {
        let len = self.mem_ld(MEM_TEMP_LEN);
        let off = self.b.ins().iadd_imm_s(len, 7);
        let off = self.b.ins().band_imm_s(off, !7_i64);
        let (end, o) = self.b.ins().uadd_overflow(off, n);
        let cap = self.mem_ld(MEM_TEMP_CAP);
        let b1 = self
            .b
            .ins()
            .icmp_imm_s(IntCC::UnsignedGreaterThan, end, 0xffff_ffff);
        let b2 = self.b.ins().icmp(IntCC::UnsignedGreaterThan, end, cap);
        let bad = self.b.ins().bor(b1, b2);
        let bad = self.b.ins().bor(bad, o);
        let good = self.b.ins().bxor_imm_s(bad, 1);
        self.guard(good, slow);
        let temp = self.mem_ld(MEM_TEMP);
        let at = self.b.ins().iadd(temp, len);
        let fill = self.b.ins().isub(end, len);
        self.b.ins().call(self.imp.zero, &[at, fill]);
        self.mem_st(MEM_TEMP_LEN, end);
        let tag = (u64::from(space::TEMP) << 61) as i64;
        let addr = self.b.ins().bor_imm_s(off, tag);
        let ptr = self.b.ins().iadd(temp, off);
        (addr, ptr)
    }

    /// The words `LayoutOf` writes for a layout known up to the descriptor's lengths, taking
    /// `slow` wherever `Ty::layout` would fail.
    fn layout_words(&mut self, sl: &StaticLayout, desc: Reg, slow: Block) -> Vec<Value> {
        let dims = match sl {
            StaticLayout::Words(_) => 0,
            StaticLayout::Array { dims, .. } => *dims,
        };
        // The interpreter reads the descriptor even when it has no bytes.
        let d = self.ld(desc);
        let p = self.host(
            d,
            self.fk(desc),
            Op::C(0),
            Op::C(u64::from(dims) * 24),
            false,
            slow,
        );
        let (el, logic) = match sl {
            StaticLayout::Words(w) => return w.iter().map(|&x| self.c64(x as i64)).collect(),
            StaticLayout::Array { elem, logic, .. } => (elem, *logic),
        };
        let mut count = self.c64(1);
        let mut okv = self.b.ins().iconst(I8, 1);
        for k in 0..i64::from(dims) {
            let q = self.b.ins().iadd_imm_s(p, k * 24);
            let (l, r, dir) = self.dope_words(q);
            let dir_ok = self
                .b
                .ins()
                .icmp_imm_s(IntCC::UnsignedLessThanOrEqual, dir, 1);
            let (len, fits) = self.len_of(l, r, dir);
            let (m, o) = self.b.ins().umul_overflow(count, len);
            count = m;
            let no = self.b.ins().bxor_imm_s(o, 1);
            let t = self.b.ins().band(dir_ok, fits);
            let t = self.b.ins().band(t, no);
            okv = self.b.ins().band(okv, t);
        }
        let (count, size, align, stride) = if logic {
            let c = self.c64(el.count as i64);
            let (n, o) = self.b.ins().umul_overflow(count, c);
            let no = self.b.ins().bxor_imm_s(o, 1);
            okv = self.b.ins().band(okv, no);
            // div_ceil(n, 64) * 32 cannot overflow.
            let blocks = self.b.ins().ushr_imm_s(n, 6);
            let rest = self.b.ins().band_imm_s(n, 63);
            let part = self.b.ins().icmp_imm_s(IntCC::NotEqual, rest, 0);
            let part = self.b.ins().uextend(I64, part);
            let blocks = self.b.ins().iadd(blocks, part);
            let size = self.b.ins().ishl_imm_s(blocks, 5);
            (n, size, self.c64(8), self.c64(0))
        } else {
            let stride = el.size.next_multiple_of(el.align.max(1));
            let sc = self.c64(stride as i64);
            let (size, o) = self.b.ins().umul_overflow(sc, count);
            let no = self.b.ins().bxor_imm_s(o, 1);
            okv = self.b.ins().band(okv, no);
            (count, size, self.c64(el.align as i64), sc)
        };
        self.guard(okv, slow);
        vec![size, align, stride, count]
    }
}

/// The kernel-facing instructions: signal reads, clock edges, drives and waits, inline or through
/// the small callbacks of `rt.rs`, as the LLVM tier's generator does (`TierSig` table entries,
/// `yavs_rt_drive*`, `yavs_rt_wait*`), instead of `yavs_rt_signal` for each.
impl Gen<'_> {
    /// A field of a `TierSig` entry (kernel memory: no alias region).
    fn ent_ld(&mut self, ent: Value, field: i64) -> Value {
        self.b
            .ins()
            .load(I64, MemFlagsData::trusted(), ent, field as i32)
    }

    /// The value register `r` holds when the generator knows it.
    fn kint(&self, r: Reg) -> Option<i64> {
        match self.known.get(r.0 as usize) {
            Some(Some(Known::Int(v))) => Some(*v),
            _ => None,
        }
    }

    /// Signal operations have no native-frame variant.
    fn no_native(&mut self) -> bool {
        if self.nat.is_some() {
            self.bad = Some("signal operation".into());
        }
        self.nat.is_some()
    }

    /// `slow_path` for a signal operation: the slow block calls `yavs_rt_signal` (`edge`).
    fn slow_signal(&mut self, pc: u32, edge: u32) -> (Block, Block) {
        let (block, join) = (self.b.create_block(), self.b.create_block());
        self.slows.push(Slow {
            block,
            join,
            pc,
            keep: false,
            edge: Some(edge),
            endstmt: false,
        });
        (block, join)
    }

    /// Start a block of a multi-block lowering.
    fn start(&mut self, bb: Block) {
        self.b.switch_to_block(bb);
        self.clear();
    }

    /// `cond ? t : f` with block arguments `v`: a branch on `cond`.
    fn branch(&mut self, cond: Value, t: Block, f: Block) {
        self.b.ins().brif(cond, t, &[], f, &[]);
    }

    /// The `TierSig` entry of the instance signal that reference registers `sig`, `sig + 1` name,
    /// and the element offset in it; takes `slow` unless the reference is a plain one of `len`
    /// elements (any number but 0 for `None`) to an instance signal the table has.
    fn sig_entry(&mut self, sig: Reg, len: Option<i64>, slow: Block) -> (Value, Value) {
        let (v, l) = (self.ld(sig), self.ld(Reg(sig.0 + 1)));
        let base = self.b.ins().ushr_imm_s(v, 32);
        let off = self.b.ins().band_imm_s(v, 0xffff_ffff);
        let n = self.mem_ld(MEM_SIGTAB_LEN);
        let has = self.b.ins().icmp(IntCC::UnsignedLessThan, base, n);
        let want = match len {
            Some(k) => self.b.ins().icmp_imm_s(IntCC::Equal, l, k),
            None => self.b.ins().icmp_imm_s(IntCC::NotEqual, l, 0),
        };
        let okc = self.b.ins().band(has, want);
        self.guard(okc, slow);
        let tab = self.mem_ld(MEM_SIGTAB);
        let at = self.b.ins().imul_imm_s(base, SIG_ENT);
        let ent = self.b.ins().iadd(tab, at);
        (ent, off)
    }

    /// `Inst::SigReadScalar` of a logic signal: the byte of its codes, else the four plane bits of
    /// the kernel's blocks; the bytes of an enumeration, integer, boolean or real signal; anything
    /// else through `TierHost::signal`.
    fn sig_read_scalar(&mut self, pc: u32, dst: Reg, sig: Reg) {
        if self.no_native() {
            return;
        }
        let (slow, join) = self.slow_signal(pc, 0);
        let (ent, off) = self.sig_entry(sig, None, slow);
        let len = self.ent_ld(ent, SIG_LEN);
        let (bytes, planes) = (self.b.create_block(), self.b.create_block());
        let in_codes = self.b.ins().icmp(IntCC::UnsignedLessThan, off, len);
        self.branch(in_codes, bytes, planes);
        self.start(bytes);
        let codes = self.ent_ld(ent, SIG_CODES);
        let p = self.b.ins().iadd(codes, off);
        let byte = self.b.ins().uload8(I64, MemFlagsData::trusted(), p, 0);
        self.st(dst, byte);
        self.b.ins().jump(join, &[]);
        self.start(planes);
        let plen = self.ent_ld(ent, SIG_PLEN);
        let inside = self.b.ins().icmp(IntCC::UnsignedLessThan, off, plen);
        self.guard(inside, slow);
        let blocks = self.ent_ld(ent, SIG_PLANES);
        let (logic, other) = (self.b.create_block(), self.b.create_block());
        let has = self.b.ins().icmp_imm_s(IntCC::NotEqual, blocks, 0);
        self.branch(has, logic, other);
        // Elements of one, four or eight bytes, a four-byte one sign-extended.
        self.start(other);
        let data = self.ent_ld(ent, SIG_BYTES);
        let has = self.b.ins().icmp_imm_s(IntCC::NotEqual, data, 0);
        self.guard(has, slow);
        let es = self.ent_ld(ent, SIG_ES);
        let first = self.ent_ld(ent, SIG_OFF);
        let e = self.b.ins().iadd(first, off);
        let at = self.b.ins().imul(e, es);
        let p = self.b.ins().iadd(data, at);
        let done = self.b.create_block();
        let v = self.b.append_block_param(done, I64);
        let (one, four, not1, not4) = (
            self.b.create_block(),
            self.b.create_block(),
            self.b.create_block(),
            self.b.create_block(),
        );
        let is1 = self.b.ins().icmp_imm_s(IntCC::Equal, es, 1);
        self.branch(is1, one, not1);
        self.start(not1);
        let is4 = self.b.ins().icmp_imm_s(IntCC::Equal, es, 4);
        self.branch(is4, four, not4);
        self.start(not4);
        let is8 = self.b.ins().icmp_imm_s(IntCC::Equal, es, 8);
        self.guard(is8, slow);
        let v8 = self.b.ins().load(I64, MemFlagsData::trusted(), p, 0);
        self.b.ins().jump(done, &[BlockArg::Value(v8)]);
        self.start(one);
        let v1 = self.b.ins().uload8(I64, MemFlagsData::trusted(), p, 0);
        self.b.ins().jump(done, &[BlockArg::Value(v1)]);
        self.start(four);
        let v4 = self.b.ins().sload32(MemFlagsData::trusted(), p, 0);
        self.b.ins().jump(done, &[BlockArg::Value(v4)]);
        self.start(done);
        self.st(dst, v);
        self.b.ins().jump(join, &[]);
        // A logic signal wider than its codes: the four plane bits.
        self.start(logic);
        let first = self.ent_ld(ent, SIG_OFF);
        let e = self.b.ins().iadd(first, off);
        let blk = self.b.ins().ushr_imm_s(e, 6);
        let at = self.b.ins().ishl_imm_s(blk, 5);
        let pb = self.b.ins().iadd(blocks, at);
        let sh = self.b.ins().band_imm_s(e, 63);
        let mut code = self.c64(0);
        for k in 0..4 {
            let w = self.b.ins().load(I64, MemFlagsData::trusted(), pb, 8 * k);
            let bit = self.b.ins().ushr(w, sh);
            let bit = self.b.ins().band_imm_s(bit, 1);
            let bit = self.b.ins().ishl_imm_s(bit, i64::from(k));
            code = self.b.ins().bor(code, bit);
        }
        self.st(dst, code);
        self.join(join);
    }

    /// The clock-edge test `kind` of one element (`Inst::Edge`, or the `rising_edge` /
    /// `falling_edge` native, whose `TierHost::signal` entry is `edge` 1 or 2): true in place when
    /// the kernel resumed the process for that edge (`TierFrame::known`), else from the event
    /// stamp, the codes and, for the IEEE function, the `'last_value` planes; anything else
    /// through `TierHost::signal`.
    fn edge(&mut self, pc: u32, kind: yavs_ir::EdgeKind, sig: Reg, dst: Reg, edge: u32) {
        if self.no_native() {
            return;
        }
        let (slow, join) = self.slow_signal(pc, edge);
        let (ent, off) = self.sig_entry(sig, Some(1), slow);
        let signal = self.ent_ld(ent, SIG_SIGNAL);
        let first = self.ent_ld(ent, SIG_OFF);
        let elem = self.b.ins().iadd(first, off);
        // `yavs_kernel::edge_key`.
        let hi = self.b.ins().ishl_imm_s(signal, 32);
        let mid = self.b.ins().ishl_imm_s(elem, 2);
        let k = self.b.ins().bor(hi, mid);
        let key = self.b.ins().bor_imm_s(k, i64::from(kind.code()));
        let known = self.mem_ld(MEM_KNOWN);
        let held = self.b.ins().icmp(IntCC::Equal, key, known);
        let (fast, rest) = (self.b.create_block(), self.b.create_block());
        self.branch(held, fast, rest);
        self.start(fast);
        let one = self.c64(1);
        self.st(dst, one);
        self.b.ins().jump(join, &[]);
        self.start(rest);
        let stamps = self.ent_ld(ent, SIG_STAMPS);
        let has = self.b.ins().icmp_imm_s(IntCC::NotEqual, stamps, 0);
        self.guard(has, slow);
        let last = self.ent_ld(ent, SIG_LAST);
        if !kind.to_level() {
            let has = self.b.ins().icmp_imm_s(IntCC::NotEqual, last, 0);
            self.guard(has, slow);
        }
        let len = self.ent_ld(ent, SIG_LEN);
        let inside = self.b.ins().icmp(IntCC::UnsignedLessThan, off, len);
        self.guard(inside, slow);
        let at = self.b.ins().ishl_imm_s(elem, 3);
        let sp = self.b.ins().iadd(stamps, at);
        let stamp = self.b.ins().load(I64, MemFlagsData::trusted(), sp, 0);
        let cycle = self.mem_ld(MEM_CYCLE);
        let (ev, none, merge) = (
            self.b.create_block(),
            self.b.create_block(),
            self.b.create_block(),
        );
        let res = self.b.append_block_param(merge, I64);
        let is_ev = self.b.ins().icmp(IntCC::Equal, stamp, cycle);
        self.branch(is_ev, ev, none);
        self.start(none);
        let zero = self.c64(0);
        self.b.ins().jump(merge, &[BlockArg::Value(zero)]);
        self.start(ev);
        let codes = self.ent_ld(ent, SIG_CODES);
        let cp = self.b.ins().iadd(codes, off);
        let cur = self.b.ins().uload8(I64, MemFlagsData::trusted(), cp, 0);
        let r = if kind.to_level() {
            // An event ending on the level: the code is exactly '1' (3) or '0' (2).
            let e = self
                .b
                .ins()
                .icmp_imm_s(IntCC::Equal, cur, if kind.rising() { 3 } else { 2 });
            self.b.ins().uextend(I64, e)
        } else {
            let mut before = self.c64(0);
            for k in 0..2 {
                let w = self.b.ins().load(I64, MemFlagsData::trusted(), last, 8 * k);
                let bit = self.b.ins().ushr(w, elem);
                let bit = self.b.ins().band_imm_s(bit, 1);
                let bit = self.b.ins().ishl_imm_s(bit, i64::from(k));
                before = self.b.ins().bor(before, bit);
            }
            // `To_X01` of a code is '1' or '0' when its low two bits are 1 x (3 and 7 are '1' and
            // 'H', 2 and 6 '0' and 'L'); the level is bit 0.
            let (now_want, before_want) = if kind.rising() { (3, 2) } else { (2, 3) };
            let c3 = self.b.ins().band_imm_s(cur, 3);
            let a = self.b.ins().icmp_imm_s(IntCC::Equal, c3, now_want);
            let b3 = self.b.ins().band_imm_s(before, 3);
            let b2 = self.b.ins().icmp_imm_s(IntCC::Equal, b3, before_want);
            let both = self.b.ins().band(a, b2);
            self.b.ins().uextend(I64, both)
        };
        self.b.ins().jump(merge, &[BlockArg::Value(r)]);
        self.start(merge);
        self.st(dst, res);
        self.join(join);
    }

    /// `Inst::SigAttr` `'event` of one element: its last event was in this cycle, from the
    /// kernel's event stamp; anything else through `TierHost::signal`.
    fn sig_event(&mut self, pc: u32, dst: Reg, sig: Reg) {
        if self.no_native() {
            return;
        }
        let (slow, join) = self.slow_signal(pc, 0);
        let (ent, off) = self.sig_entry(sig, Some(1), slow);
        let stamps = self.ent_ld(ent, SIG_STAMPS);
        let has = self.b.ins().icmp_imm_s(IntCC::NotEqual, stamps, 0);
        self.guard(has, slow);
        let first = self.ent_ld(ent, SIG_OFF);
        let elem = self.b.ins().iadd(first, off);
        let at = self.b.ins().ishl_imm_s(elem, 3);
        let sp = self.b.ins().iadd(stamps, at);
        let stamp = self.b.ins().load(I64, MemFlagsData::trusted(), sp, 0);
        let cycle = self.mem_ld(MEM_CYCLE);
        let e = self.b.ins().icmp(IntCC::Equal, stamp, cycle);
        let v = self.b.ins().uextend(I64, e);
        self.st(dst, v);
        self.join(join);
    }

    /// `Inst::SigRead` of `l` elements from the kernel's plane blocks of the signal whose entry is
    /// `ent` (`off` the first element in it), through `yavs_rt_planes`; `slow` when it offers
    /// none or has fewer elements. The builder ends in a block that jumps to `join`.
    fn sig_read_planes(
        &mut self,
        ent: Value,
        off: Value,
        l: Value,
        dst: Reg,
        slow: Block,
        join: Block,
    ) {
        let blocks = self.ent_ld(ent, SIG_PLANES);
        let has = self.b.ins().icmp_imm_s(IntCC::NotEqual, blocks, 0);
        self.guard(has, slow);
        // A count and an offset are below 2^32 where the reference was made; checked, so the sum
        // below cannot wrap.
        let small = self
            .b
            .ins()
            .icmp_imm_s(IntCC::UnsignedLessThanOrEqual, l, i64::from(u32::MAX));
        self.guard(small, slow);
        let plen = self.ent_ld(ent, SIG_PLEN);
        let used = self.b.ins().iadd(off, l);
        let fits = self
            .b
            .ins()
            .icmp(IntCC::UnsignedLessThanOrEqual, used, plen);
        self.guard(fits, slow);
        let first = self.ent_ld(ent, SIG_OFF);
        let bit0 = self.b.ins().iadd(first, off);
        let nelem = self.b.ins().iadd(first, plen);
        let nelem = self.b.ins().iadd_imm_s(nelem, 63);
        let nblk = self.b.ins().ushr_imm_s(nelem, 6);
        let lb = self.b.ins().iadd_imm_s(l, 63);
        let lb = self.b.ins().ushr_imm_s(lb, 6);
        let bytes = self.b.ins().ishl_imm_s(lb, 5);
        let a = self.ld(dst);
        let pd = self.host(a, self.fk(dst), Op::C(0), Op::V(bytes), true, slow);
        self.b
            .ins()
            .call(self.imp.planes, &[blocks, nblk, bit0, l, pd]);
        self.b.ins().jump(join, &[]);
    }

    /// `Inst::SigRead`: up to 64 elements from the codes in place (a count known at compile time
    /// builds the planes eight codes at a time, else `yavs_rt_codes`), more from the kernel's
    /// plane blocks (`yavs_rt_planes`); anything else through `TierHost::signal`.
    fn sig_read(&mut self, pc: u32, dst: Reg, sig: Reg) {
        if self.no_native() {
            return;
        }
        let known = self.kint(Reg(sig.0 + 1));
        if let Some(l) = known
            && !(1..=i64::from(u32::MAX)).contains(&l)
        {
            self.signal(pc, 0);
            return;
        }
        let (slow, join) = self.slow_signal(pc, 0);
        let (ent, off) = self.sig_entry(sig, known, slow);
        let lv = self.ld(Reg(sig.0 + 1));
        if known.is_some_and(|l| l > 64) {
            self.sig_read_planes(ent, off, lv, dst, slow, join);
            self.start(join);
            return;
        }
        // Up to 64 elements from the codes, else (or without the codes) from the planes.
        let alt = self.b.create_block();
        let l1 = self.b.ins().iadd_imm_s(lv, -1);
        let small = self.b.ins().icmp_imm_s(IntCC::UnsignedLessThan, l1, 64);
        self.guard(small, alt);
        let tl = self.ent_ld(ent, SIG_LEN);
        let used = self.b.ins().iadd(off, lv);
        let fits = self.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, used, tl);
        self.guard(fits, alt);
        let groups = self.b.ins().iadd_imm_s(lv, 7);
        let groups = self.b.ins().ushr_imm_s(groups, 3);
        let gb = self.b.ins().ishl_imm_s(groups, 3);
        let end = self.b.ins().iadd(off, gb);
        let cap = self.ent_ld(ent, SIG_CAP);
        let in_cap = self.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, end, cap);
        self.guard(in_cap, alt);
        let codes = self.ent_ld(ent, SIG_CODES);
        let base = self.b.ins().iadd(codes, off);
        // (1 << l) - 1, all ones for 64.
        let ones = self.c64(-1);
        let c64v = self.c64(64);
        let sh = self.b.ins().isub(c64v, lv);
        let mask = self.b.ins().ushr(ones, sh);
        let a = self.ld(dst);
        let pd = self.host(a, self.fk(dst), Op::C(0), Op::C(32), true, slow);
        self.b.ins().call(self.imp.codes, &[base, groups, mask, pd]);
        self.b.ins().jump(join, &[]);
        self.start(alt);
        self.sig_read_planes(ent, off, lv, dst, slow, join);
        self.start(join);
    }

    /// A `Drive` of one waveform element to a single-range target through `yavs_rt_drive`
    /// (`TierHost::drive_block`: at most 64 logic elements assigned from the element's first
    /// plane block, a wider one a block of planes per 64, or one cell of an integer, enumeration
    /// or real signal); the instruction through `yavs_rt_signal` unless `n` is 1 and the bytes of
    /// the waveform are in memory compiled code reaches.
    fn drive_block(
        &mut self,
        pc: u32,
        driver: u32,
        target: Reg,
        wave: Reg,
        n: Reg,
        mode: yavs_ir::DriveMode,
    ) {
        if self.no_native() {
            return;
        }
        let (slow, join) = self.slow_signal(pc, 0);
        let nv = self.ld(n);
        let one = self.b.ins().icmp_imm_s(IntCC::Equal, nv, 1);
        self.guard(one, slow);
        // A waveform element of 16 bytes (time and cell) for a signal of integer, enumeration or
        // real elements, 40 (time and the first plane block) for a logic one, 8 and a block of
        // planes per 64 elements beyond 64.
        let (ent, _) = self.sig_entry(target, None, slow);
        let es = self.ent_ld(ent, SIG_ES);
        let bytes = self.b.ins().icmp_imm_s(IntCC::NotEqual, es, 0);
        let len = self.ld(Reg(target.0 + 1));
        let wide = self.b.ins().icmp_imm_s(IntCC::UnsignedGreaterThan, len, 64);
        let small =
            self.b
                .ins()
                .icmp_imm_s(IntCC::UnsignedLessThanOrEqual, len, i64::from(u32::MAX));
        self.guard(small, slow);
        let blocks = self.b.ins().iadd_imm_s(len, 63);
        let blocks = self.b.ins().ushr_imm_s(blocks, 6);
        let wb = self.b.ins().ishl_imm_s(blocks, 5);
        let wide_bytes = self.b.ins().iadd_imm_s(wb, 8);
        let c40 = self.c64(40);
        let logic = self.b.ins().select(wide, wide_bytes, c40);
        let c16 = self.c64(16);
        let want = self.b.ins().select(bytes, c16, logic);
        let a = self.ld(wave);
        let w = self.host(a, self.fk(wave), Op::C(0), Op::V(want), false, slow);
        let (c2, c0, c4, cm) = (
            self.c32(2),
            self.c32(0),
            self.c32(4),
            self.c32(u32::from(mode == yavs_ir::DriveMode::Inertial)),
        );
        let b2 = self.b.ins().select(bytes, c2, c0);
        let nb = self.b.ins().bxor_imm_s(bytes, 1);
        let wnb = self.b.ins().band(wide, nb);
        let w4 = self.b.ins().select(wnb, c4, c0);
        let fl = self.b.ins().bor(b2, w4);
        let flags = self.b.ins().bor(fl, cm);
        let (pcv, drv) = (self.c32(pc), self.c32(driver));
        let (lo, len) = (self.ld(target), self.ld(Reg(target.0 + 1)));
        self.call_go(self.imp.drive, &[pcv, drv, lo, len, w, flags], false);
        self.join(join);
    }

    /// The `Drive` of a scalar logic assignment the interpreter fuses
    /// (`yavs_interp::ScalarDrive`): the waveform is never made, the delay and the code go to the
    /// kernel in registers.
    fn drive_bit(&mut self, pc: u32, s: &yavs_interp::ScalarDrive) {
        if self.no_native() {
            return;
        }
        let v = self.ld(s.value);
        let code = if s.inertial {
            self.b.ins().bor_imm_s(v, 0x100)
        } else {
            v
        };
        let (pcv, drv) = (self.c32(pc), self.c32(s.driver));
        let (lo, len, time) = (
            self.ld(s.target),
            self.ld(Reg(s.target.0 + 1)),
            self.ld(s.time),
        );
        self.call_go(self.imp.drive_bit, &[pcv, drv, lo, len, time, code], true);
    }

    /// The temporary area's length the native frame's statements trim to.
    fn nat_temp_base(&mut self) -> Value {
        let cell = self.nat.as_ref().map_or(self.mem, |n| n.temp_base);
        self.b.ins().load(I64, MemFlagsData::trusted(), cell, 0)
    }

    /// Materialise a native frame at `pc` (its exit block); the builder goes on in a new block,
    /// which nothing reaches.
    fn deopt_here(&mut self, pc: u32) {
        let v = self.c32(pc);
        self.leave(v);
        let dead = self.b.create_block();
        self.start(dead);
    }

    /// `Inst::TempRoot`: a fat pair is recorded only when a word points into a heap
    /// (`Machine::fat_root`), checked in place; a typed temporary (native frames) and the pair
    /// that does point into a heap go to `yavs_rt_troot`, the interpreter's `TierHost::temp_root`,
    /// which says 0 where the interpreter must execute the instruction.
    fn temp_root(
        &mut self,
        prog: &Program,
        pc: u32,
        i: &Inst,
        addr: Reg,
        desc: Option<Reg>,
        ty: Option<yavs_ir::TyId>,
    ) -> Result<(), String> {
        if ty.is_some() && self.nat.is_none() {
            // A frame-mode function steps the typed temporary.
            self.step(pc, i);
            return Ok(());
        }
        let (slow, join) = self.slow_path(pc, i);
        match ty {
            None => {
                let rec = self.b.create_block();
                self.b.set_cold_block(rec);
                let a = self.ld(addr);
                let p = self.host(a, self.fk(addr), Op::C(0), Op::C(16), false, rec);
                let nf = self.gf;
                let (w0, w1) = (
                    self.b.ins().load(I64, nf, p, 0),
                    self.b.ins().load(I64, nf, p, 8),
                );
                let mut heap = None;
                for w in [w0, w1] {
                    let sp = self.b.ins().ushr_imm_s(w, 61);
                    let h = self
                        .b
                        .ins()
                        .icmp_imm_s(IntCC::Equal, sp, i64::from(space::HEAP));
                    let sh =
                        self.b
                            .ins()
                            .icmp_imm_s(IntCC::Equal, sp, i64::from(space::SHARED_HEAP));
                    let any = self.b.ins().bor(h, sh);
                    heap = Some(match heap {
                        Some(x) => self.b.ins().bor(x, any),
                        None => any,
                    });
                }
                let none = self.b.ins().bxor_imm_s(heap.expect("two words"), 1);
                self.b.ins().brif(none, join, &[], rec, &[]);
                self.start(rec);
                let (ty, z, n) = (self.c32(u32::MAX), self.c64(0), self.c64(0));
                let call = self
                    .b
                    .ins()
                    .call(self.imp.troot, &[self.env, self.mem, a, ty, z, n]);
                let ok = self.b.inst_results(call)[0];
                self.b.ins().brif(ok, join, &[], slow, &[]);
                self.start(join);
            }
            Some(t) if self.nat.is_some() => {
                let n = ok(yavs_ir::Ty::desc_size(&prog.types, t), "no descriptor size")?;
                let dp = match (n, desc) {
                    (0, _) => self.c64(0),
                    (_, Some(d)) => {
                        let dv = self.ld(d);
                        self.host(dv, self.fk(d), Op::C(0), Op::C(n), false, slow)
                    }
                    _ => return Err("recorded temporary without its descriptor".into()),
                };
                let a = self.ld(addr);
                let (tv, nv) = (self.c32(t.0), self.c64(n as i64));
                let call = self
                    .b
                    .ins()
                    .call(self.imp.troot, &[self.env, self.mem, a, tv, dp, nv]);
                let okv = self.b.inst_results(call)[0];
                self.clear();
                self.guard(okv, slow);
                self.join(join);
            }
            Some(_) => unreachable!("handled above"),
        }
        Ok(())
    }

    /// An instruction of a native frame the interpreter executes on a frame of its own
    /// (`TierHost::native_step`, `xstep_ok`); the registers it reads go in, those it writes come
    /// back.
    fn native_step(&mut self, pc: u32, i: &Inst, f: &Func) {
        let Some(n) = &self.nat else { return };
        let (func, depth) = (n.func, n.depth);
        let temp_base = self.nat_temp_base();
        let (uses, defs) = yavs_interp::inst_regs(f, i);
        let (slow, join) = self.slow_path(pc, i);
        let slot = |b: &mut FunctionBuilder<'_>, n: usize| {
            let ss = b.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                (n.max(1) * 8) as u32,
                3,
            ));
            b.ins().stack_addr(I64, ss, 0)
        };
        let (ins, outs) = (slot(&mut self.b, uses.len()), slot(&mut self.b, defs.len()));
        // The register numbers, as `u32`s in a slot of their own.
        let (iu, od) = (
            slot(&mut self.b, uses.len().div_ceil(2)),
            slot(&mut self.b, defs.len().div_ceil(2)),
        );
        for (k, &r) in uses.iter().enumerate() {
            let v = self.ld(r);
            self.b
                .ins()
                .store(MemFlagsData::trusted(), v, ins, (k * 8) as i32);
            let idx = self.b.ins().iconst(I32, i64::from(r.0));
            self.b
                .ins()
                .store(MemFlagsData::trusted(), idx, iu, (k * 4) as i32);
        }
        for (k, &r) in defs.iter().enumerate() {
            let idx = self.b.ins().iconst(I32, i64::from(r.0));
            self.b
                .ins()
                .store(MemFlagsData::trusted(), idx, od, (k * 4) as i32);
        }
        let (fv, pv, nin, nout) = (
            self.c32(func),
            self.c32(pc),
            self.c32(uses.len() as u32),
            self.c32(defs.len() as u32),
        );
        let call = self.b.ins().call(
            self.imp.xstep,
            &[
                self.env, self.mem, fv, pv, depth, iu, ins, nin, od, outs, nout, temp_base,
            ],
        );
        let st = self.b.inst_results(call)[0];
        self.clear();
        let okv = self.b.ins().icmp_imm_s(IntCC::NotEqual, st, 0);
        self.guard(okv, slow);
        for (k, &r) in defs.iter().enumerate() {
            let v = self
                .b
                .ins()
                .load(I64, MemFlagsData::trusted(), outs, (k * 8) as i32);
            self.st(r, v);
        }
        // The frame's own statements must not free what a `NewFrame` made.
        if matches!(i, Inst::NewFrame { .. }) {
            let len = self.mem_ld(MEM_TEMP_LEN);
            let cell = self.nat.as_ref().map(|n| n.temp_base);
            if let Some(cell) = cell {
                self.b.ins().store(MemFlagsData::trusted(), len, cell, 0);
            }
        }
        // The instruction ended the process: the frame is materialised after it.
        let (out, cont) = (self.b.create_block(), self.b.create_block());
        let stop = self.b.ins().icmp_imm_s(IntCC::Equal, st, 2);
        self.b.set_cold_block(out);
        self.branch(stop, out, cont);
        self.start(out);
        let v = self.c32(pc + 1);
        self.leave(v);
        self.start(cont);
        self.join(join);
    }

    /// `Inst::CallNative` of a native `yavs_interp::direct_native` allows, in a frame-mode
    /// function: the argument cells go to `TierHost::native_frame` (`yavs_rt_native`), the
    /// interpreter steps the call where that does not run it.
    fn native_direct(
        &mut self,
        pc: u32,
        i: &Inst,
        f: &Func,
        native: yavs_ir::NativeId,
        args: yavs_ir::Regs,
        dst: Option<Reg>,
    ) -> Result<(), String> {
        let rs = ok(f.regs(args), "operand list out of range")?;
        let mut cells = Vec::new();
        for &r in rs {
            cells.push(r);
            if f.reg_ty.get(r.0 as usize) == Some(&yavs_ir::RegTy::Sig) {
                cells.push(Reg(r.0 + 1));
            }
        }
        let (slow, join) = self.slow_path(pc, i);
        let slot = |b: &mut FunctionBuilder<'_>, n: usize| {
            let ss = b.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                (n.max(1) * 8) as u32,
                3,
            ));
            b.ins().stack_addr(I64, ss, 0)
        };
        let buf = slot(&mut self.b, cells.len());
        let scratch = slot(&mut self.b, 1);
        for (k, &r) in cells.iter().enumerate() {
            let v = self.ld(r);
            self.b
                .ins()
                .store(MemFlagsData::trusted(), v, buf, (k * 8) as i32);
        }
        let depth = match &self.nat {
            Some(n) => n.depth,
            None => self.c64((NATIVE_FRAME_MODE | u64::from(pc)) as i64),
        };
        let (nv, nn) = (self.c32(native.0), self.c32(cells.len() as u32));
        let call = self.b.ins().call(
            self.imp.native,
            &[self.env, self.mem, nv, buf, nn, depth, scratch],
        );
        let st = self.b.inst_results(call)[0];
        self.clear();
        let kind = self.b.ins().band_imm_s(st, 0xff);
        let mut okc = self.b.ins().icmp_imm_s(IntCC::NotEqual, kind, 0);
        if dst.is_some() {
            let has = self.b.ins().band_imm_s(st, 0x100);
            let has = self.b.ins().icmp_imm_s(IntCC::NotEqual, has, 0);
            okc = self.b.ins().band(okc, has);
        }
        self.guard(okc, slow);
        if let Some(d) = dst {
            let v = self.b.ins().load(I64, MemFlagsData::trusted(), scratch, 0);
            self.st(d, v);
        }
        // The native ended the process: a native frame is materialised after the call, a
        // frame-mode function suspends with its registers in place.
        let (out, cont) = (self.b.create_block(), self.b.create_block());
        let stop = self.b.ins().icmp_imm_s(IntCC::Equal, kind, 2);
        self.b.set_cold_block(out);
        self.branch(stop, out, cont);
        self.start(out);
        let v = if self.nat.is_some() {
            self.c32(pc + 1)
        } else {
            self.c32(yavs_interp::TIER_SUSPEND)
        };
        self.leave(v);
        self.start(cont);
        self.join(join);
        Ok(())
    }

    /// `Inst::Deref` of an access value a `Deref` found before: its object's entry in `Mem::objs`
    /// holds the value (bits 32 and up of it are the object's index) and the address is the
    /// entry's, inline; else `yavs_rt_fast` finds the object and enters it with the value, and
    /// where that fails the interpreter executes the instruction (its error).
    fn deref(&mut self, pc: u32, i: &Inst, dst: Reg, handle: Reg) {
        let (slow, join) = self.slow_path(pc, i);
        let h = self.ld(handle);
        let hi = self.b.ins().ushr_imm_s(h, 32);
        let flag = self.b.ins().ushr_imm_s(hi, i64::from(KEY_SHIFT_HANDLE));
        let flag = self.b.ins().band_imm_s(flag, KEY_FLAG as i64);
        let key = self.b.ins().bxor(hi, flag);
        let key = self.b.ins().band_imm_s(key, OBJS as i64 - 1);
        let eo = self.b.ins().imul_imm_s(key, OBJ_SIZE as i64);
        let ent = self.b.ins().iadd(self.mem, eo);
        // An entry with an access value is writable, and a null handle is no entry's value.
        let eh = self
            .b
            .ins()
            .load(I64, self.mf, ent, (MEM_OBJS + OBJ_H) as i32);
        let hit = self.b.ins().icmp(IntCC::Equal, eh, h);
        let (fast, miss) = (self.b.create_block(), self.b.create_block());
        self.b.set_cold_block(miss);
        self.branch(hit, fast, miss);
        self.start(fast);
        let ehi = self.b.ins().load(I64, self.mf, ent, MEM_OBJS as i32);
        let v = self.b.ins().ishl_imm_s(ehi, 32);
        self.st(dst, v);
        self.b.ins().jump(join, &[]);
        self.start(miss);
        let (op, d, z) = (self.c32(FAST_DEREF), self.c32(dst.0), self.c64(0));
        let call = self
            .b
            .ins()
            .call(self.imp.fast, &[self.env, self.regs, self.mem, op, d, h, z]);
        let st = self.b.inst_results(call)[0];
        let failed = self.b.ins().icmp_imm_s(IntCC::Equal, st, 0);
        self.b.ins().brif(failed, slow, &[], join, &[]);
        self.start(join);
    }

    /// A `WaitOn` and the `Suspend` after it through `yavs_rt_wait` (`TierHost::wait_suspend`).
    fn wait_suspend(&mut self, pc: u32, sig: Reg) {
        if self.no_native() {
            return;
        }
        let (pcv, lo, len) = (self.c32(pc), self.ld(sig), self.ld(Reg(sig.0 + 1)));
        self.call_go(self.imp.wait, &[pcv, lo, len], false);
    }

    /// A `WaitOn` that is not the last of its wait: the kernel's registration of the waiter it
    /// had at the last wait, else the instruction (`yavs_rt_wait_on`).
    fn wait_on(&mut self, pc: u32, sig: Reg) {
        if self.no_native() {
            return;
        }
        let (pcv, lo, len) = (self.c32(pc), self.ld(sig), self.ld(Reg(sig.0 + 1)));
        self.call_go(self.imp.wait_on, &[pcv, lo, len], true);
    }

    /// The wait that names the same signals every time, at its first statement: the process
    /// suspends at the `Suspend` at `at` when the kernel still holds the last wait
    /// (`yavs_rt_wait_same`).
    fn wait_same(&mut self, at: u32) {
        if self.no_native() {
            return;
        }
        let (pcv, z) = (self.c32(at), self.c64(0));
        self.call_go(self.imp.wait_same, &[pcv, z, z], true);
    }
}

/// Compile every function of `prog` the tier can take; the others stay interpreted and are
/// listed in [`ClifTier::notes`].
pub fn compile(prog: &Program, max_size: usize) -> Result<crate::ClifTier, Error> {
    let mut jit = Jit::new()?;
    let mut notes = Vec::new();
    let mut ids = Vec::new();
    let mut kinds = HashMap::new();
    let mut inline = 0;
    let mut instrs = 0;
    let mut times = Vec::new();
    let t0 = std::time::Instant::now();
    for (id, _) in prog.funcs.iter() {
        let Some(f) = prog.func(id) else { continue };
        if f.kind == FuncKind::Elab {
            continue;
        }
        if f.code.len() > max_size {
            notes.push((
                id,
                format!(
                    "{} IR instructions, over the size cap of {max_size} (--tier-max-size)",
                    f.code.len()
                ),
            ));
            continue;
        }
        let t = std::time::Instant::now();
        let r = jit.define(prog, id, f);
        times.push((f.code.len(), t.elapsed()));
        match r {
            Ok((cid, stepped)) => {
                instrs += f.code.len();
                inline += f.code.len().saturating_sub(stepped.len());
                for k in stepped {
                    *kinds.entry(k).or_insert(0) += 1;
                }
                ids.push((id, cid, f.reg_ty.len()))
            }
            Err(why) => notes.push((id, why)),
        }
    }
    jit.finalize()?;
    let compile_time = t0.elapsed();
    let mut funcs = vec![None; prog.funcs.len()];
    for (id, cid, nregs) in ids {
        funcs[id.0 as usize] = Some((jit.entry(cid), nregs));
    }
    let mut interpreted: Vec<(String, usize)> = kinds.into_iter().collect();
    interpreted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Ok(crate::ClifTier {
        _jit: std::sync::Mutex::new(jit),
        funcs,
        notes,
        compile_time,
        interpreted,
        inline,
        instrs,
        times,
    })
}
