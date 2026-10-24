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
use cranelift_codegen::isa::CallConv;

use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Switch, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{DataDescription, DataId, FuncId as ClifId, Linkage, Module};
use yavs_interp::space;
use yavs_ir::{
    AddrSpace, BoolOp, CheckKind, CmpOp, FOp, FUnOp, Func, FuncId, FuncKind, Inst, IntOp, IntUnOp,
    Program, Reg, Width,
};
use yavs_llvm::native::{Arg, call_fits, closed_frame, ieee_fast, ieee_layout, native_set};
use yavs_llvm::regs::Live;
use yavs_llvm::rt::*;

const I64: Type = types::I64;
const I32: Type = types::I32;
const I8: Type = types::I8;
const F64: Type = types::F64;
const I128: Type = types::I128;

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
    link: u64,
    regs: *const u64,
) -> *mut u64 {
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *TRACE.get_or_init(|| std::env::var_os("YAVS_CLIFT_TRACE").is_some()) {
        eprintln!("materialise f{func} at pc {pc}");
    }
    // SAFETY: compiled code passes the `Env` and `Mem` it was given.
    unsafe { yavs_rt_deopt(env, mem, func, pc, temp_base, nregs, slen, link, regs) }
}

/// `n` copies of the `size` bytes at `src` from `dst` on (`Inst::Fill`, the pattern of a `NewFrame`);
/// `src` must not overlap the target.
extern "C" fn clift_fill(dst: *mut u8, src: *const u8, n: u64, size: u64) {
    // SAFETY: compiled code passes `n * size` writable bytes at `dst` and `size` readable at `src`.
    unsafe {
        if size == 1 {
            std::ptr::write_bytes(dst, *src, n as usize);
        } else {
            for k in 0..n as usize {
                std::ptr::copy_nonoverlapping(src, dst.add(k * size as usize), size as usize);
            }
        }
    }
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
    fill: ClifId,
    memeq: ClifId,
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
    /// `native_set` of a function alone, once asked.
    nsets: HashMap<FuncId, Rc<HashSet<FuncId>>>,
    /// Every call of a native-frame function goes through `Mem::ntab`, none is direct or inlined
    /// (`YAVS_CLIFT_NTAB=1`: tests of the mixed tiers, where the callee may be LLVM's).
    via_ntab: bool,
    /// `YAVS_CLIFT_PERFMAP=1`: the functions defined and not yet written to `/tmp/perf-<pid>.map`
    /// (name, id, code size), so that `perf report` names the generated code.
    perfmap: Option<Vec<(String, ClifId, usize)>>,
}

/// The native-frame variant `n<id>` of a function, in the calling convention of the LLVM tier
/// (`native_sig`): its registers and slots live on the native stack, the interpreter has no frame
/// for it, and anything the compiled code cannot finish materialises the frame
/// (`yavs_rt_deopt`).
struct NatInfo {
    id: ClifId,
    /// The `CallFn` adapter `c<id>` for the interpreter's calls (`Tier::call`); none for a nested
    /// function.
    call: Option<ClifId>,
    /// The Cranelift body, for the inliner.
    body: ir::Function,
}

/// What `Jit::native_of` found for a callee.
enum NatVar {
    /// Defined in this module: a direct call, which Cranelift may inline.
    Direct(Rc<NatInfo>),
    /// Being built (a cycle): called through `Mem::ntab` once published.
    Indirect,
    /// No variant can be made.
    No,
}

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
            ("clift_fill", clift_fill as *const u8),
            (MEMEQ, yavs_rt_memeq as *const u8),
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
        let fill = imp("clift_fill", &[I64, I64, I64, I64], None)?;
        let memeq = imp(MEMEQ, &[I64, I64, I64], Some(I64))?;
        let deopt = imp(
            DEOPT,
            &[I64, I64, I32, I32, I64, I64, I64, I64, I64],
            Some(I64),
        )?;
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
            fill,
            memeq,
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
            nsets: HashMap::new(),
            via_ntab: flag("YAVS_CLIFT_NTAB", "0") == "1",
            perfmap: (flag("YAVS_CLIFT_PERFMAP", "0") == "1").then(Vec::new),
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

    /// Make the native-frame variant of `id` if it can have one; whether it has one.
    pub fn make_native(&mut self, prog: &Program, id: FuncId) -> bool {
        matches!(
            self.native_of(prog, id),
            NatVar::Direct(_) | NatVar::Indirect
        )
    }

    /// `native_set` of `id` alone.
    fn nset(&mut self, prog: &Program, id: FuncId) -> Rc<HashSet<FuncId>> {
        self.nsets
            .entry(id)
            .or_insert_with(|| Rc::new(native_set(prog, [id])))
            .clone()
    }

    /// The native-frame variant of `id`, made on first use.
    fn native_of(&mut self, prog: &Program, id: FuncId) -> NatVar {
        if !self.native_on {
            return NatVar::No;
        }
        if let Some(r) = self.natives.get(&id) {
            return r.clone().map_or(NatVar::No, NatVar::Direct);
        }
        // A function calling itself, directly or not, calls through `Mem::ntab`.
        if !self.building.insert(id) {
            return NatVar::Indirect;
        }
        let r = self.try_native(prog, id);
        self.building.remove(&id);
        self.natives.insert(id, r.clone());
        r.map_or(NatVar::No, NatVar::Direct)
    }

    fn try_native(&mut self, prog: &Program, id: FuncId) -> Option<Rc<NatInfo>> {
        let f = prog.func(id)?;
        let dump = std::env::var_os("YAVS_CLIFT_DUMP").is_some();
        // The functions that run on native frames are the LLVM tier's rule (`native_set`).
        if f.code.is_empty() || !self.nset(prog, id).contains(&id) {
            if dump {
                eprintln!(
                    "no native variant of {id} {}: not in the native set",
                    f.name
                );
            }
            return None;
        }
        if self.natives.values().flatten().count() >= self.native_limit {
            return None;
        }
        let (cid, _, body) = match self.build(prog, id, f, true) {
            Ok(r) => r,
            Err(why) => {
                if dump {
                    eprintln!("no native variant of {id} {}: {why}", f.name);
                }
                return None;
            }
        };
        let call = if f.parent.is_none() {
            match self.define_adapter(f, cid) {
                Ok(c) => Some(c),
                Err(why) => {
                    if dump {
                        eprintln!("no adapter of {id} {}: {why}", f.name);
                    }
                    return None;
                }
            }
        } else {
            None
        };
        if dump {
            eprintln!(
                "native variant #{} of {} {}",
                self.natives.values().flatten().count() + 1,
                id,
                f.name
            );
        }
        Some(Rc::new(NatInfo {
            id: cid,
            call,
            body: body?,
        }))
    }

    /// `c<id>(env, mem, depth, cells) -> i64` (`CallFn`): the variant `nat` with its argument cells
    /// read from an array, for the interpreter's calls (`Tier::call`).
    fn define_adapter(&mut self, f: &Func, nat: ClifId) -> Result<ClifId, Error> {
        let mut ctx = self.module.make_context();
        let mut fctx = FunctionBuilderContext::new();
        ctx.func.signature.params.extend([AbiParam::new(I64); 4]);
        ctx.func.signature.returns.push(AbiParam::new(I64));
        ctx.func.signature.call_conv = self.sig.call_conv;
        let fr = self.module.declare_func_in_func(nat, &mut ctx.func);
        let ncells: usize = f
            .params
            .iter()
            .map(|p| 1 + usize::from(p.width == Width::Sig))
            .sum();
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
            let e = b.create_block();
            b.append_block_params_for_function_params(e);
            b.switch_to_block(e);
            let p = b.block_params(e).to_vec();
            let mut args = vec![p[0], p[1], p[2]];
            for k in 0..ncells {
                let v = b
                    .ins()
                    .load(I64, MemFlagsData::trusted(), p[3], (8 * k) as i32);
                args.push(v);
            }
            let call = b.ins().call(fr, &args);
            let r = b.inst_results(call)[0];
            b.ins().return_(&[r]);
            b.seal_all_blocks();
            b.finalize(self.module.target_config());
        }
        let fid = self
            .module
            .declare_function(
                &format!("c{}", nat.as_u32()),
                Linkage::Export,
                &ctx.func.signature,
            )
            .map_err(|e| e.to_string())?;
        self.module
            .define_function(fid, &mut ctx)
            .map_err(|e| format!("Cranelift: {e:?} (a bug in yavs)"))?;
        self.module.clear_context(&mut ctx);
        Ok(fid)
    }

    /// The native-frame code and `CallFn` adapter of each variant made so far (0: none), after
    /// `finalize`.
    pub fn variants(&self) -> Vec<(FuncId, usize, usize)> {
        let mut v: Vec<_> = self
            .natives
            .iter()
            .filter_map(|(&id, n)| n.as_ref().map(|n| (id, n)))
            .map(|(id, n)| {
                let at = |c: ClifId| self.module.get_finalized_function(c) as usize;
                (id, at(n.id), n.call.map_or(0, at))
            })
            .collect();
        v.sort_by_key(|x| x.0.0);
        v
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
        // The callees that run on native frames (the LLVM tier's rule): called directly when
        // they have a variant in this module, else through `Mem::ntab`. A native variant is made
        // only when every such callee can be called.
        let nc = if native {
            self.nset(prog, id)
        } else {
            Rc::new(native_set(
                prog,
                std::iter::once(id).chain(f.code.iter().filter_map(|i| match *i {
                    Inst::Call { func, .. } | Inst::CallProt { func, .. } => Some(func),
                    _ => None,
                })),
            ))
        };
        let mut nats: HashMap<FuncId, Option<Rc<NatInfo>>> = HashMap::new();
        for i in &f.code {
            let (Inst::Call { func, args, .. } | Inst::CallProt { func, args, .. }) = *i else {
                continue;
            };
            let prot = matches!(i, Inst::CallProt { .. });
            let Some(g) = prog.func(func) else { continue };
            if nats.contains_key(&func) || !nc.contains(&func) || !call_fits(f, id, g, args, prot) {
                continue;
            }
            if self.via_ntab {
                nats.insert(func, None);
                continue;
            }
            match self.native_of(prog, func) {
                NatVar::Direct(n) => nats.insert(func, Some(n)),
                NatVar::Indirect => nats.insert(func, None),
                NatVar::No if native => return Err(format!("{func} has no native variant")),
                NatVar::No => None,
            };
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
        // Copies of the constants this function takes the address of.
        let mut cons = HashMap::new();
        for i in &f.code {
            if let Inst::AddrOf {
                space: AddrSpace::Const(k),
                ..
            } = *i
                && !cons.contains_key(&k)
                && let Some(yavs_ir::ConstData::Bytes(bytes)) = prog.consts.get(k)
                && bytes.len() <= 4096
            {
                let did = match self.tables.get(bytes) {
                    Some(&d) => d,
                    None => {
                        let name = format!("t{}", self.tables.len());
                        let d = self
                            .module
                            .declare_data(&name, Linkage::Local, false, false)
                            .map_err(|e| e.to_string())?;
                        let mut dd = DataDescription::new();
                        dd.define(bytes.clone().into_boxed_slice());
                        self.module.define_data(d, &dd).map_err(|e| e.to_string())?;
                        self.tables.insert(bytes.clone(), d);
                        d
                    }
                };
                let gv = self.module.declare_data_in_func(did, &mut ctx.func);
                cons.insert(k, (gv, bytes.len() as u64));
            }
        }
        ctx.func.signature = if native {
            native_sig(self.sig.call_conv, f)
        } else {
            self.sig.clone()
        };
        ctx.func.name = UserFuncName::user(u32::from(native), id.0);
        let mut natives = HashMap::new();
        let mut bodies: HashMap<FuncRef, Rc<NatInfo>> = HashMap::new();
        for (&g, n) in &nats {
            let fr = n
                .as_ref()
                .map(|n| self.module.declare_func_in_func(n.id, &mut ctx.func));
            natives.insert(g, NatRef { fr });
            if let (Some(fr), Some(n)) = (fr, n) {
                bodies.insert(fr, n.clone());
            }
        }
        let imports = Imports {
            step: self.module.declare_func_in_func(self.step, &mut ctx.func),
            host: self.module.declare_func_in_func(self.host, &mut ctx.func),
            signal: self.module.declare_func_in_func(self.signal, &mut ctx.func),
            lcopy: self.module.declare_func_in_func(self.lcopy, &mut ctx.func),
            zero: self.module.declare_func_in_func(self.zero, &mut ctx.func),
            mov: self.module.declare_func_in_func(self.mov, &mut ctx.func),
            fill: self.module.declare_func_in_func(self.fill, &mut ctx.func),
            memeq: self.module.declare_func_in_func(self.memeq, &mut ctx.func),
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
            consts: cons,
            natives,
            cc: self.sig.call_conv,
        };
        let cfg = self.module.target_config();
        let stepped = gen_func(&mut ctx, &mut fctx, cfg, prog, f, imports, id, native)?;
        // `YAVS_CLIFT_CLIF=<function id>`: the Cranelift IR of that function (variant) on stderr.
        if std::env::var("YAVS_CLIFT_CLIF")
            .is_ok_and(|v| v == "all" || v.parse::<u32>().is_ok_and(|n| n == id.0))
        {
            eprintln!(
                "clif of {} f{}:\n{}",
                if native { "variant" } else { "frame code" },
                id.0,
                ctx.func.display()
            );
        }
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
        // `YAVS_CLIFT_ASM=<function id>`: the machine code of that function (variant) on stderr.
        let asm = std::env::var("YAVS_CLIFT_ASM")
            .is_ok_and(|v| v.parse::<u32>().is_ok_and(|n| n == id.0));
        ctx.set_disasm(asm);
        self.module
            .define_function(fid, &mut ctx)
            .map_err(|e| format!("Cranelift: {e:?} (a bug in yavs)"))?;
        if let Some(v) = ctx
            .compiled_code()
            .and_then(|c| c.vcode.as_ref())
            .filter(|_| asm)
        {
            eprintln!(
                "asm of {} f{}:\n{v}",
                if native { "variant" } else { "frame code" },
                id.0
            );
        }
        if let (Some(list), Some(c)) = (self.perfmap.as_mut(), ctx.compiled_code()) {
            let kind = if native { "n" } else { "f" };
            list.push((
                format!("{kind}{}_{}", id.0, f.name.replace(' ', "_")),
                fid,
                c.code_info().total_size as usize,
            ));
        }
        self.module.clear_context(&mut ctx);
        Ok((fid, stepped, body))
    }

    pub fn finalize(&mut self) -> Result<(), Error> {
        self.module
            .finalize_definitions()
            .map_err(|e| e.to_string())?;
        if let Some(list) = self.perfmap.take() {
            use std::io::Write;
            let path = format!("/tmp/perf-{}.map", std::process::id());
            if let Ok(mut out) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                // `YAVS_CLIFT_CODEDIR=<dir>`: the machine code of each function as `<name>.bin`,
                // for `objdump -D -b binary -mi386:x86-64` against `perf script` addresses.
                let dir = std::env::var_os("YAVS_CLIFT_CODEDIR");
                for (name, id, size) in &list {
                    let at = self.module.get_finalized_function(*id) as usize;
                    let _ = writeln!(out, "{at:x} {size:x} {name}");
                    if let Some(d) = &dir {
                        // SAFETY: the finalized function owns `size` bytes of code at `at`.
                        let code = unsafe { std::slice::from_raw_parts(at as *const u8, *size) };
                        let file: String = name
                            .chars()
                            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                            .collect();
                        let _ = std::fs::write(
                            std::path::Path::new(d).join(format!("{file}.bin")),
                            code,
                        );
                    }
                }
            }
            self.perfmap = Some(Vec::new());
        }
        Ok(())
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
    fill: FuncRef,
    memeq: FuncRef,
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
    /// Copies of the constants (up to 4 KiB of bytes) the function takes the address of, which
    /// never change: reads of them need no lookup.
    consts: HashMap<yavs_ir::ConstId, (GlobalValue, u64)>,
    natives: HashMap<FuncId, NatRef>,
    /// The calling convention of the variants (`native_sig`).
    cc: CallConv,
}

/// A call to a native-frame variant from a function being built: direct (`fr`) when the variant
/// is in this module, else through `Mem::ntab`.
#[derive(Clone, Copy)]
struct NatRef {
    fr: Option<FuncRef>,
}

/// The signature of a native-frame variant, the LLVM tier's: `(env, mem, depth, [up1, up2], cells...)
/// -> i64`, the static links of a nested function after `depth`, a cell per parameter and two for
/// a signal; the value (0 for none). A frame recorded for materialisation (`yavs_rt_deopt`) is
/// signalled by `Mem::ndeopt` after the call.
fn native_sig(cc: CallConv, f: &Func) -> Signature {
    let mut s = Signature::new(cc);
    s.params.extend([AbiParam::new(I64); 3]);
    if f.parent.is_some() {
        s.params.extend([AbiParam::new(I64); 2]);
    }
    for p in &f.params {
        s.params.push(AbiParam::new(I64));
        if p.width == Width::Sig {
            s.params.push(AbiParam::new(I64));
        }
    }
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
#[derive(Clone, Copy)]
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
    /// In a native frame: the interpreter executes the instruction on a frame of its own
    /// (`native_step`) and the code goes on, instead of leaving.
    xstep: bool,
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
    known: Vec<Option<Known<Value>>>,
    /// The object the instruction being generated found for its result register: entered in
    /// `known` once `track` has run for it (`ObjRef`).
    objnew: Option<(Reg, ObjRef<Value>)>,
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
    /// A variant keeps its registers in Cranelift variables (the register array is written only
    /// where the frame is recorded for the interpreter, `spill`); empty in frame mode.
    vars: Vec<Variable>,
    /// Registers live before each instruction (`spill` stores only those), when the function is
    /// small enough for the analysis.
    live: Option<Live>,
    /// The registers the interpreter starts a frame with (`preloaded`): never variables, their
    /// values are constants or frame addresses made where they are used (`ld`).
    pre: Vec<Option<(AddrSpace, u32)>>,
    /// The high half of the addresses of this variant's frame (`Nat`).
    frame_hi: Option<Value>,
    /// The registers defined once by a constant (`fixed_consts`).
    fixed: Vec<Option<i64>>,
}

/// What a native-frame variant has instead of the interpreter's frame.
struct Nat {
    func: u32,
    nregs: u64,
    /// The frame index of this activation, and the high halves of the addresses of the frames one
    /// and two static links up (-1: none).
    depth: Value,
    up: [Value; 2],
    /// The frame's slots on the native stack.
    slots: Value,
    /// The temporary area's length at entry: where `Stmt` trims to.
    /// The cell (a stack address) holding the temporary area's length the frame's statements trim
    /// to: the length at entry, raised by a `NewFrame` the frame's own statements must not free.
    temp_base: TempBase,
    /// The byte ranges of the slots in use when no address of the frame leaves the function
    /// (`closed_frame`): the frame is not registered in `Mem::nslots`, only these are zeroed at
    /// entry and handed to the interpreter when the frame is recorded.
    closed: Option<Vec<(u64, u64)>>,
    /// Slots kept as variables (closed frames): offset, width, variable.
    svars: Vec<(u32, Width, Variable)>,
}

/// Where a variant keeps the temporary area's length its statements trim to.
#[derive(Clone, Copy)]
enum TempBase {
    /// The length at entry (no `NewFrame` raises it).
    Len(Value),
    /// A cell (a stack address) holding it.
    Cell(Value),
}

/// The slots of a closed frame that can live in variables: one width, accessed only through
/// `Load` and `Store` of the address registers `pre` names, overlapping no access of another
/// offset or width. A signal reference (two cells) stays in memory.
fn slot_vars(f: &Func, pre: &[Option<(AddrSpace, u32)>]) -> Vec<(u32, Width)> {
    let mut acc: Vec<(u32, Width)> = f.params.iter().map(|p| (p.slot, p.width)).collect();
    for i in &f.code {
        if let Inst::Load { addr, w, .. } | Inst::Store { addr, w, .. } = *i
            && let Some(Some((AddrSpace::Frame, off))) = pre.get(addr.0 as usize)
        {
            acc.push((*off, w));
        }
    }
    let mut out: Vec<(u32, Width)> = Vec::new();
    for &(o, w) in &acc {
        let end = o + w.bytes();
        let clash = acc
            .iter()
            .any(|&(o2, w2)| (o2, w2) != (o, w) && o2 < end && o < o2 + w2.bytes());
        if !clash && w != Width::Sig && !out.contains(&(o, w)) {
            out.push((o, w));
        }
    }
    out
}

/// The parts of `ranges` that no slot variable covers.
fn gaps(ranges: &[(u64, u64)], svars: &[(u32, Width, Variable)]) -> Vec<(u64, u64)> {
    let mut vars: Vec<(u64, u64)> = svars
        .iter()
        .map(|&(o, w, _)| (u64::from(o), u64::from(o) + u64::from(w.bytes())))
        .collect();
    vars.sort_unstable();
    let mut out = Vec::new();
    for &(lo, hi) in ranges {
        let mut at = lo;
        for &(a, b) in vars.iter().filter(|&&(a, b)| a < hi && lo < b) {
            if a > at {
                out.push((at, a));
            }
            at = at.max(b);
        }
        if at < hi {
            out.push((at, hi));
        }
    }
    out
}

/// The value a load of `w` reads back from a slot that was stored `v`.
fn canon(b: &mut FunctionBuilder<'_>, w: Width, v: Value) -> Value {
    match w {
        Width::B1 => {
            let x = b.ins().ireduce(I8, v);
            b.ins().uextend(I64, x)
        }
        Width::B4 => {
            let x = b.ins().ireduce(I32, v);
            b.ins().sextend(I64, x)
        }
        _ => v,
    }
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

#[allow(clippy::too_many_arguments)]
fn gen_func(
    ctx: &mut Context,
    fctx: &mut FunctionBuilderContext,
    cfg: cranelift_codegen::isa::TargetFrontendConfig,
    prog: &Program,
    f: &Func,
    imp: Imports,
    id: FuncId,
    native: bool,
) -> Result<Vec<String>, String> {
    let mut b = FunctionBuilder::new(&mut ctx.func, fctx);
    let entry = b.create_block();
    b.append_block_params_for_function_params(entry);
    let exit = b.create_block();
    let exit_pc = b.append_block_param(exit, I32);
    b.switch_to_block(entry);
    let p = b.block_params(entry).to_vec();
    let (env, mem) = (p[0], if native { p[1] } else { p[2] });
    let pc_in = if native { p[2] } else { p[3] };
    let nregs = f.reg_ty.len() as u64;
    // A native-frame variant keeps its registers and slots on the native stack.
    let mut nat = None;
    let mut frame_hi = None;
    let regs = if native {
        let rs = b.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            (nregs * 8) as u32,
            3,
        ));
        let closed = closed_frame(prog, f, id);
        // A closed frame needs the bytes in use only.
        let size = closed
            .as_ref()
            .map_or(f.frame, |c| c.iter().map(|r| r.1 as u32).max().unwrap_or(0));
        let ss = b.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            size.max(1).next_multiple_of(8),
            3,
        ));
        let regs = b.ins().stack_addr(I64, rs, 0);
        let slots = b.ins().stack_addr(I64, ss, 0);
        let len0 = b
            .ins()
            .load(I64, MemFlagsData::trusted(), mem, MEM_TEMP_LEN as i32);
        // The length statements trim to is the one at entry, a cell only where a `NewFrame` raises it.
        let temp_base = if f.code.iter().any(|i| matches!(i, Inst::NewFrame { .. })) {
            let tcell =
                b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
            let cell = b.ins().stack_addr(I64, tcell, 0);
            b.ins().store(MemFlagsData::trusted(), len0, cell, 0);
            TempBase::Cell(cell)
        } else {
            TempBase::Len(len0)
        };
        // The static links of a nested function follow the frame index.
        let up = if f.parent.is_some() {
            [p[3], p[4]]
        } else {
            let none = b.ins().iconst(I64, -1);
            [none, none]
        };
        nat = Some(Nat {
            func: id.0,
            nregs,
            depth: pc_in,
            up,
            slots,
            temp_base,
            closed,
            svars: Vec::new(),
        });
        // The high half of the addresses of this frame: the frame's index with the space tag.
        let hi = b.ins().bor_imm_s(pc_in, i64::from(space::FRAME) << 29);
        frame_hi = Some(b.ins().ishl_imm_s(hi, 32));
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
    // Address spaces that hold at the start of an IR block whatever path reached it.
    let space_facts = yavs_llvm::regs::spaces(prog, f, &leader);
    for (i, inst) in f.code.iter().enumerate() {
        if resumes(prog, f, inst) {
            leader[i + 1] = true;
        }
    }
    // How many ways lead to each block start; a variant's block with a single way in takes what
    // its predecessor knew at the jump (its values dominate it).
    let targets = |inst: &Inst| -> Vec<usize> {
        let at = |b: yavs_ir::BlockId| f.blocks.get(b).map(|b| b.start as usize);
        match *inst {
            Inst::Br { to } => at(to).into_iter().collect(),
            Inst::CondBr { t, f: e, .. } => at(t).into_iter().chain(at(e)).collect(),
            Inst::Case { table, .. } => prog
                .cases
                .get(table)
                .map(|t| {
                    std::iter::once(t.default)
                        .chain(t.arms.iter().map(|a| a.2))
                        .filter_map(at)
                        .collect()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    };
    let is_jump = |i: &Inst| {
        matches!(
            i,
            Inst::Br { .. }
                | Inst::CondBr { .. }
                | Inst::Case { .. }
                | Inst::Ret { .. }
                | Inst::Unreachable { .. }
        )
    };
    let mut ways = vec![0u32; n + 1];
    ways[0] = 2;
    for (i, inst) in f.code.iter().enumerate() {
        for t in targets(inst) {
            ways[t] += 1;
        }
        if i + 1 < n && !is_jump(inst) {
            ways[i + 1] += 1;
        }
    }
    let mut inherit: Vec<Option<Vec<Option<Known<Value>>>>> = vec![None; n + 1];
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
    let mut base_known: Vec<Option<Known<Value>>> = vec![None; f.reg_ty.len()];
    let mut pre: Vec<Option<(AddrSpace, u32)>> = vec![None; f.reg_ty.len()];
    for (r, addr, _) in preloads(f) {
        base_known[r.0 as usize] = match addr {
            Some((AddrSpace::Frame, off)) => Some(Known::Frame(off.into())),
            Some((AddrSpace::Const(k), off)) => Some(Known::Const(k, off.into())),
            Some((AddrSpace::Inst, off)) => Some(Known::Inst(off.into())),
            _ => None,
        };
        if let Some((sp, off)) = addr
            && native
            && addr_const(sp, off).is_some()
        {
            pre[r.0 as usize] = addr;
        }
    }
    // The registers a `Const` defines once are preloaded, so they hold the value everywhere
    // (lengths and counts that `cat_block` and others need to know).
    let fixed = fixed_consts(f);
    for (r, v) in fixed.iter().enumerate() {
        if let (Some(v), None) = (v, &base_known[r]) {
            base_known[r] = Some(Known::Int(*v));
        }
    }
    let mut g = Gen {
        known: base_known.clone(),
        objnew: None,
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
        vars: Vec::new(),
        live: None,
        pre,
        frame_hi,
        fixed: fixed.clone(),
    };
    if g.nat.is_some() {
        g.vars = (0..f.reg_ty.len()).map(|_| g.b.declare_var(I64)).collect();
        g.live = Live::new(prog, f, None);
        if g.nat.as_ref().is_some_and(|n| n.closed.is_some()) {
            let sv: Vec<_> = slot_vars(f, &g.pre)
                .into_iter()
                .map(|(o, w)| (o, w, g.b.declare_var(I64)))
                .collect();
            if let Some(n) = g.nat.as_mut() {
                n.svars = sv;
            }
        }
    }
    if g.nat.is_some() {
        g.native_entry(f, &p[if f.parent.is_some() { 5 } else { 3 }..])?;
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
    let always: Vec<usize> = (0..base_known.len())
        .filter(|&r| base_known[r].is_some())
        .collect();
    let mut open = false;
    let mut prev = None;
    for (i, &inst) in f.code.iter().enumerate() {
        let pc = i as u32;
        if let Some(p) = prev.replace(inst) {
            track(&mut g.known, &fixed, &p);
            // `track` forgets the register after a definition too (a signal reference is two):
            // what holds everywhere stays known.
            for &r in &always {
                if g.known[r].is_none() {
                    g.known[r] = base_known[r];
                }
            }
            // The objects found stay valid up to an instruction that may free or move one (a jump
            // does neither).
            if !keeps_objects(&p)
                && !matches!(p, Inst::Br { .. } | Inst::CondBr { .. } | Inst::Case { .. })
            {
                g.objnew = None;
                for k in g.known.iter_mut().flatten() {
                    if let Known::Heap(o) = k {
                        *o = None;
                    }
                }
            }
            if let Some((r, o)) = g.objnew.take()
                && let Some(k) = g.known.get_mut(r.0 as usize)
            {
                *k = Some(Known::Heap(Some(o)));
            }
        }
        if leader[i] {
            // The single way in: the fall-through keeps what is known, a jump brings it (one that
            // comes from further down the code, not yet seen, brings nothing).
            let single = native && ways[i] == 1;
            let fall = i > 0 && !is_jump(&f.code[i - 1]);
            let brought = inherit[i].take().filter(|_| single);
            if !(single && fall) {
                match brought {
                    Some(k) => g.known = k,
                    None => {
                        g.known.clone_from(&base_known);
                        for &(r, sp) in space_facts.as_ref().map_or(&[][..], |v| &v[i][..]) {
                            if let Some(x) = g.known.get_mut(r as usize)
                                && x.is_none()
                            {
                                *x = Some(match sp {
                                    yavs_llvm::regs::Sp::Inst => Known::InstAny,
                                    yavs_llvm::regs::Sp::Temp => Known::Temp,
                                    yavs_llvm::regs::Sp::Heap => Known::Heap(None),
                                });
                            }
                        }
                    }
                }
            }
        }
        if native {
            for t in targets(&inst) {
                if ways[t] == 1 {
                    inherit[t] = Some(g.known.clone());
                }
            }
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
                g.b.ins().return_(&[val]);
            } else {
                g.leave_at(pc);
            }
            open = false;
            continue;
        }
        if g.nat.is_some() && matches!(inst, Inst::Unreachable { .. }) {
            // The interpreter reports it, with the frame materialised.
            g.leave_at(pc);
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
                // error. With the sum under 2^32 the high half is untouched: the result is the
                // base plus the offset.
                let a = g.ld(base);
                let (base_off, base_obj) = match g.known.get(base.0 as usize) {
                    Some(Some(Known::Frame(o) | Known::Const(_, o) | Known::Inst(o))) => {
                        (Some(*o), None)
                    }
                    Some(Some(Known::Heap(Some(o)))) if g.nat.is_some() => (None, Some(*o)),
                    _ => (None, None),
                };
                // The offset of the base in its 4 GiB window.
                let lo_of = |g: &mut Gen| match base_obj {
                    Some(o) => o.off,
                    None => g.lo32(a),
                };
                let mut sum_out = None;
                let v = match g.kint(pos) {
                    Some(p) => match (p as u64).checked_mul(u64::from(stride)) {
                        Some(off) if off <= 0xffff_ffff => {
                            if base_off.is_none_or(|o| o + off > 0xffff_ffff) {
                                let lo = lo_of(&mut g);
                                let sum = g.b.ins().iadd_imm_s(lo, off as i64);
                                let big = g.b.ins().ushr_imm_s(sum, 32);
                                g.exit_if(big, pc);
                                sum_out = Some(sum);
                            }
                            g.b.ins().iadd_imm_s(a, off as i64)
                        }
                        // Always the interpreter's error.
                        _ => {
                            g.leave_at(pc);
                            let dead = g.b.create_block();
                            g.start(dead);
                            a
                        }
                    },
                    None => {
                        let p = g.ld(pos);
                        if stride == 0 {
                            sum_out = base_obj.map(|o| o.off);
                            a
                        } else {
                            let off = if stride == 1 {
                                p
                            } else {
                                g.b.ins().imul_imm_s(p, i64::from(stride))
                            };
                            // Both under 2^32 (the position and the stride are): no wrap.
                            let lo = lo_of(&mut g);
                            let sum = g.b.ins().iadd(lo, off);
                            let wide = g.b.ins().bor(p, sum);
                            let big = g.b.ins().ushr_imm_s(wide, 32);
                            g.exit_if(big, pc);
                            sum_out = Some(sum);
                            g.b.ins().iadd(a, off)
                        }
                    }
                };
                if let (Some(o), Some(sum)) = (base_obj, sum_out) {
                    g.objnew = Some((dst, ObjRef { off: sum, ..o }));
                }
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
                    AddrSpace::Up(d @ 1..=2) => {
                        let (slow, join) = g.slow_path(pc, &inst);
                        let hi = match &g.nat {
                            Some(n) => n.up[usize::from(d - 1)],
                            None => g.mem_ld(MEM_UP + 8 * u64::from(d - 1)),
                        };
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
            Inst::Cat {
                dst,
                n,
                m,
                parts,
                scalars,
                bias,
                logic: true,
                ..
            } => {
                let rs = ok(f.regs(parts), "operand list out of range")?;
                if !g.cat_block(pc, &inst, dst, n, m, rs, scalars, bias)
                    && !g.cat_dyn(pc, &inst, f, dst, n, m, rs, scalars, bias)
                {
                    if g.nat.is_some() && xstep_wide(f, &inst) {
                        g.native_step(pc, &inst, f)
                    } else {
                        g.step(pc, &inst)
                    }
                }
            }
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
            Inst::Load { dst, addr, w } if let Some(var) = g.svar(addr, w) => {
                let v = g.b.use_var(var);
                g.st(dst, v);
            }
            Inst::Store { addr, src, w } if let Some(var) = g.svar(addr, w) => {
                let x = g.ld(src);
                let c = canon(&mut g.b, w, x);
                g.b.def_var(var, c);
            }
            Inst::Load { dst, addr, w } => {
                let (slow, join) = g.slow_path(pc, &inst);
                let a = g.ld(addr);
                let p = g.host(
                    a,
                    Some(addr),
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
                    Some(addr),
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
                let ps = g.host(sa, Some(src), Op::C(0), Op::V(n), false, slow);
                let pd = g.host(da, Some(dst), Op::C(0), Op::V(n), true, slow);
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
            Inst::SlicePos {
                dst,
                dope,
                left,
                right,
                dir,
                ..
            } => {
                let (slow, join) = g.slow_path(pc, &inst);
                let a = g.ld(dope);
                let p = g.host(a, Some(dope), Op::C(0), Op::C(24), false, slow);
                let (l, r, d) = g.dope_words(p);
                let (x, y, w) = (g.ld(left), g.ld(right), g.ld(dir));
                // A null slice names no element and checks nothing.
                let wd = g.b.ins().icmp_imm_s(IntCC::Equal, w, 1);
                let lt = g.b.ins().icmp(IntCC::SignedLessThan, x, y);
                let gt = g.b.ins().icmp(IntCC::SignedGreaterThan, x, y);
                let null = g.b.ins().select(wd, lt, gt);
                // Both ends inside the array's range, and the slice's direction its own.
                let down = g.b.ins().icmp_imm_s(IntCC::Equal, d, 1);
                let lo = g.b.ins().select(down, r, l);
                let hi = g.b.ins().select(down, l, r);
                let dir_ok = g.b.ins().icmp_imm_s(IntCC::UnsignedLessThanOrEqual, d, 1);
                let same = g.b.ins().icmp(IntCC::Equal, w, d);
                let nonempty = g.b.ins().icmp(IntCC::SignedLessThanOrEqual, lo, hi);
                let span = g.b.ins().isub(hi, lo);
                let (rx, ry) = (g.b.ins().isub(x, lo), g.b.ins().isub(y, lo));
                let ix = g.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, rx, span);
                let iy = g.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, ry, span);
                let okv = g.b.ins().band(dir_ok, same);
                let okv = g.b.ins().band(okv, nonempty);
                let okv = g.b.ins().band(okv, ix);
                let okv = g.b.ins().band(okv, iy);
                let okv = g.b.ins().bor(null, okv);
                g.guard(okv, slow);
                let up = g.b.ins().isub(x, l);
                let dn = g.b.ins().isub(l, x);
                let pos = g.b.ins().select(down, dn, up);
                let zero = g.c64(0);
                let v = g.b.ins().select(null, zero, pos);
                g.st(dst, v);
                g.join(join);
            }
            // A run of 1 to 64 elements inside one plane block set to one code; any other run is
            // the interpreter's.
            Inst::LFill { base, idx, n, v } => {
                let (slow, join) = g.slow_path_x(pc, &inst, f);
                let code = g.ld(v);
                let okc =
                    g.b.ins()
                        .icmp_imm_s(IntCC::UnsignedLessThanOrEqual, code, 8);
                g.guard(okc, slow);
                let cnt = g.ld(n);
                let n1 = g.b.ins().iadd_imm_s(cnt, -1);
                let okn = g.b.ins().icmp_imm_s(IntCC::UnsignedLessThan, n1, 64);
                g.guard(okn, slow);
                let (p, sh) = g.logic_word(base, idx, true, slow);
                let end = g.b.ins().iadd(sh, cnt);
                let oke =
                    g.b.ins()
                        .icmp_imm_s(IntCC::UnsignedLessThanOrEqual, end, 64);
                g.guard(oke, slow);
                let one = g.c64(1);
                let bit = g.b.ins().ishl(one, cnt);
                let low = g.b.ins().iadd_imm_s(bit, -1);
                let full = g.b.ins().icmp_imm_s(IntCC::Equal, cnt, 64);
                let ones = g.c64(-1);
                let low = g.b.ins().select(full, ones, low);
                let mask = g.b.ins().ishl(low, sh);
                let keep = g.b.ins().bnot(mask);
                for k in 0..4 {
                    let w = g.b.ins().load(I64, g.gf, p, k * 8);
                    let w = g.b.ins().band(w, keep);
                    let bit = g.b.ins().ushr_imm_s(code, i64::from(k));
                    let bit = g.b.ins().band_imm_s(bit, 1);
                    let neg = g.b.ins().ineg(bit);
                    let set = g.b.ins().band(mask, neg);
                    let w = g.b.ins().bor(w, set);
                    g.b.ins().store(g.gf, w, p, k * 8);
                }
                g.join(join);
            }
            // Equality of two logic arrays of 1 to 64 elements: the four planes under the mask of
            // the elements; any other count is the interpreter's.
            Inst::LEq { dst, a, b, n } => {
                let (slow, join) = g.slow_path_x(pc, &inst, f);
                let cnt = g.ld(n);
                let n1 = g.b.ins().iadd_imm_s(cnt, -1);
                let okn = g.b.ins().icmp_imm_s(IntCC::UnsignedLessThan, n1, 64);
                g.guard(okn, slow);
                let av = g.ld(a);
                let pa = g.host(av, Some(a), Op::C(0), Op::C(32), false, slow);
                let bv = g.ld(b);
                let pb = g.host(bv, Some(b), Op::C(0), Op::C(32), false, slow);
                let gf = g.gf;
                let mut diff = g.c64(0);
                for k in 0..4 {
                    let x = g.b.ins().load(I64, gf, pa, k * 8);
                    let y = g.b.ins().load(I64, gf, pb, k * 8);
                    let d = g.b.ins().bxor(x, y);
                    diff = g.b.ins().bor(diff, d);
                }
                let mask = g.low_mask(cnt);
                let masked = g.b.ins().band(diff, mask);
                let eq = g.b.ins().icmp_imm_s(IntCC::Equal, masked, 0);
                let v = g.b.ins().uextend(I64, eq);
                g.st(dst, v);
                g.join(join);
            }
            // The bytes of two ranges compared.
            Inst::MemEq { dst, a, b, n } => {
                let (slow, join) = g.slow_path_x(pc, &inst, f);
                let k = g.ld(n);
                let av = g.ld(a);
                let pa = g.host(av, Some(a), Op::C(0), Op::V(k), false, slow);
                let bv = g.ld(b);
                let pb = g.host(bv, Some(b), Op::C(0), Op::V(k), false, slow);
                let c = g.b.ins().call(g.imp.memeq, &[pa, pb, k]);
                let v = g.b.inst_results(c)[0];
                g.st(dst, v);
                g.join(join);
            }
            Inst::LogicOp { op, dst, a, b, n } if logic_inline(op) => {
                g.logic_op(pc, &inst, f, op, dst, a, b, n)
            }
            Inst::NewFrame {
                dst,
                desc,
                dsize,
                data,
                size,
                fill,
            } if g.nat.is_some() => g.new_frame(pc, &inst, dst, desc, dsize, data, size, fill),
            Inst::Fill { dst, elem, n, size } if g.nat.is_some() => {
                g.fill_inst(pc, &inst, dst, elem, n, size)
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
                    Some(dope),
                    Op::C(u64::from(dim) * 24),
                    Op::C(24),
                    false,
                    slow,
                );
                let (l, r, dir) = g.dope_words(p);
                let i = g.ld(idx);
                // Up (0): l <= i <= r, position i - l; down (1): r <= i <= l, position l - i; any
                // other direction word, and an empty range, are the interpreter's error.
                let down = g.b.ins().icmp_imm_s(IntCC::Equal, dir, 1);
                let lo = g.b.ins().select(down, r, l);
                let hi = g.b.ins().select(down, l, r);
                let dir_ok = g.b.ins().icmp_imm_s(IntCC::UnsignedLessThanOrEqual, dir, 1);
                let nonempty = g.b.ins().icmp(IntCC::SignedLessThanOrEqual, lo, hi);
                let rel = g.b.ins().isub(i, lo);
                let span = g.b.ins().isub(hi, lo);
                let inside = g.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, rel, span);
                let okv = g.b.ins().band(dir_ok, nonempty);
                let okv = g.b.ins().band(okv, inside);
                g.guard(okv, slow);
                let up = g.b.ins().isub(i, l);
                let dn = g.b.ins().isub(l, i);
                let v = g.b.ins().select(down, dn, up);
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
                    Some(layout),
                    Op::C(16 + 8 * u64::from(n)),
                    Op::C(8),
                    false,
                    slow,
                );
                let off = g.b.ins().load(I64, g.gf, p, 0);
                let a = g.ld(base);
                let lo = g.lo32(a);
                let sum = g.b.ins().iadd(lo, off);
                // Both under 2^32 (the offset is checked as well): the sum does not wrap.
                let wide = g.b.ins().bor(off, sum);
                let big = g.b.ins().ushr_imm_s(wide, 32);
                let good = g.b.ins().icmp_imm_s(IntCC::Equal, big, 0);
                g.guard(good, slow);
                let v = g.b.ins().iadd(a, off);
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
                    Some(at),
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
                let ps = g.host(sa, Some(src), Op::V(sb), Op::V(sn), false, slow);
                let pd = g.host(da, Some(dst), Op::V(db), Op::V(dn), true, slow);
                // Up to 64 elements: at most two blocks each side, shifted in registers. In a
                // process, whose copy count is a constant (a subprogram's body is not worth the
                // code it saves a call for, nor is it for a count known at run time only: the
                // 128-bit shifts cost more than the call).
                if f.kind == FuncKind::Process && g.kint(n).is_some_and(|v| v <= 64) {
                    let few = g.b.ins().icmp_imm_s(IntCC::UnsignedLessThanOrEqual, k, 64);
                    let (inl, call) = (g.b.create_block(), g.b.create_block());
                    g.b.ins().brif(few, inl, &[], call, &[]);
                    g.b.switch_to_block(inl);
                    g.lcopy_inline(pd, nd, d0, ps, ns, s0, k);
                    g.b.ins().jump(join, &[]);
                    g.b.switch_to_block(call);
                }
                g.b.ins().call(g.imp.lcopy, &[pd, nd, d0, ps, ns, s0, k]);
                g.join(join);
            }
            Inst::Call { func, args, dst } => {
                let nr = g.imp.natives.get(&func).copied();
                let callee = prog.func(func);
                match (nr, callee) {
                    (Some(nr), Some(callee)) if call_fits(f, id, callee, args, false) => {
                        g.native_call(pc, &inst, id, f, nr, func, callee, args, dst, None)
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
                let callee = prog.func(func);
                match (nr, callee) {
                    (Some(nr), Some(callee))
                        if call_fits(f, id, callee, args, true)
                            && obj.0 < f.reg_ty.len() as u32 =>
                    {
                        g.native_call(pc, &inst, id, f, nr, func, callee, args, dst, Some(obj))
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
                    None if frame_direct(prog, f, &inst)
                        && yavs_interp::light_native(prog, f, native, args)
                        && let Some(op) = ieee_fast(prog, f, native, args, dst) =>
                    {
                        g.ieee_native(pc, &inst, f, op, native, args, dst)?
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
        g.leave_at(n as u32);
    }
    // The slow paths, after the body (a slow path may add more).
    let mut next_slow = 0;
    while next_slow < g.slows.len() {
        let s = g.slows[next_slow];
        next_slow += 1;
        g.b.set_cold_block(s.block);
        g.b.switch_to_block(s.block);
        g.clear();
        if s.xstep && g.nat.is_some() {
            let inst = f.code[s.pc as usize];
            g.native_step(s.pc, &inst, f);
            g.b.ins().jump(s.join, &[]);
            continue;
        }
        if s.endstmt && g.nat.is_some() {
            let base = g.nat_temp_base();
            g.b.ins().call(g.imp.endstmt, &[g.env, g.mem, base]);
            g.b.ins().jump(s.join, &[]);
            continue;
        }
        if g.nat.is_some() {
            // The interpreter executes the instruction in the materialised frame.
            g.leave_at(s.pc);
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
        // (`yavs_rt_deopt` set `Mem::ndeopt`: the caller sees the frames recorded.)
        g.record_frame(f, exit_pc);
        let zero = g.c64(0);
        g.b.ins().return_(&[zero]);
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

    /// The variable of the closed frame's slot a load or store of `w` through `addr` reaches.
    fn svar(&self, addr: Reg, w: Width) -> Option<Variable> {
        let o = self.frame_of(addr)?;
        let n = self.nat.as_ref()?;
        n.svars
            .iter()
            .find(|s| u64::from(s.0) == o && s.1 == w)
            .map(|s| s.2)
    }

    /// The frame slot offset `r` holds the address of, if known.
    fn frame_of(&self, r: Reg) -> Option<u64> {
        if let Some(Some((AddrSpace::Frame, off))) = self.pre.get(r.0 as usize) {
            return Some(u64::from(*off));
        }
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

    /// The value of `v` when it is an integer constant.
    fn const_of(&self, v: Value) -> Option<i64> {
        let dfg = &self.b.func.dfg;
        match dfg.value_def(v) {
            ir::ValueDef::Result(inst, 0) => match dfg.insts[inst] {
                ir::InstructionData::UnaryImm {
                    opcode: ir::Opcode::Iconst,
                    imm,
                } => Some(imm.bits()),
                _ => None,
            },
            _ => None,
        }
    }

    /// The low 32 bits of `a`, as an i64.
    fn lo32(&mut self, a: Value) -> Value {
        let l = self.b.ins().ireduce(I32, a);
        self.b.ins().uextend(I64, l)
    }

    fn c32(&mut self, v: u32) -> Value {
        self.b.ins().iconst(I32, i64::from(v))
    }

    fn ld(&mut self, r: Reg) -> Value {
        if let Some(Some(Known::Int(v))) = self.known.get(r.0 as usize) {
            let v = *v;
            return self.c64(v);
        }
        if self.nat.is_some() {
            return self.var_ld(r);
        }
        if let Some(Some(v)) = self.cache.get(r.0 as usize) {
            return *v;
        }
        let v = self.b.ins().load(I64, self.rf, self.regs, r.0 as i32 * 8);
        self.cache[r.0 as usize] = Some(v);
        v
    }

    /// Register `r` of a variant: an address the interpreter preloads is made where it is used,
    /// the rest is a variable.
    fn var_ld(&mut self, r: Reg) -> Value {
        // A constant defined once is the interpreter's preload: its value wherever it is used,
        // also on a path that did not run the definition.
        if let Some(c) = self.fixed.get(r.0 as usize).copied().flatten() {
            return self.c64(c);
        }
        match self.pre.get(r.0 as usize).copied().flatten() {
            Some((AddrSpace::Frame, off)) => {
                let hi = self.frame_hi.expect("variant");
                self.b.ins().bor_imm_s(hi, i64::from(off))
            }
            Some((sp, off)) => {
                let a = addr_const(sp, off).expect("preload");
                self.c64(a as i64)
            }
            None => self.b.use_var(self.vars[r.0 as usize]),
        }
    }

    fn st(&mut self, r: Reg, v: Value) {
        if self.nat.is_some() {
            self.b.def_var(self.vars[r.0 as usize], v);
            return;
        }
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

    /// The prologue of a native-frame variant: zeroed slots (as a frame the interpreter pushes),
    /// the arguments stored into their parameter slots.
    fn native_entry(&mut self, f: &Func, args: &[Value]) -> Result<(), String> {
        let slots = match &self.nat {
            Some(n) => n.slots,
            None => return Ok(()),
        };
        // Callees reach these slots through their frame index (`Mem::nslots`).
        let depth = self.nat.as_ref().map_or(self.regs, |n| n.depth);
        let tab = self.mem_ld(MEM_NSLOTS);
        let eo = self.b.ins().imul_imm_s(depth, 16);
        let ent = self.b.ins().iadd(tab, eo);
        let closed = self.nat.as_ref().and_then(|n| n.closed.clone());
        let svars: Vec<(u32, Width, Variable)> = self
            .nat
            .as_ref()
            .map(|n| n.svars.clone())
            .unwrap_or_default();
        // The slots start zeroed, as in a pushed frame; the registers do not (every register is
        // written before it is read, as in the LLVM tier's native frames). A closed frame is not
        // registered (length 0, so a stale entry of an earlier frame at this depth is never read)
        // and only the bytes in use that are not variables are zeroed.
        let z = self.c64(0);
        match &closed {
            Some(ranges) => {
                self.b.ins().store(MemFlagsData::trusted(), z, ent, 8);
                for (lo, hi) in gaps(ranges, &svars) {
                    self.zero_range(slots, lo, hi);
                }
            }
            None => {
                self.b.ins().store(MemFlagsData::trusted(), slots, ent, 0);
                let fl = self.c64(i64::from(f.frame));
                self.b.ins().store(MemFlagsData::trusted(), fl, ent, 8);
                self.zero_range(slots, 0, u64::from(f.frame));
            }
        }
        for &(_, _, v) in &svars {
            self.b.def_var(v, z);
        }
        // The registers the interpreter preloads (`fixed_consts`, `Gen::pre`) are made where they
        // are used and stored where the frame is recorded.
        let mut args = args.iter();
        for p in &f.params {
            let off = p.slot as i32;
            let Some(&v) = args.next() else {
                return Err("missing argument cell".into());
            };
            if let Some(&(_, w, var)) = svars.iter().find(|s| s.0 == p.slot) {
                let c = canon(&mut self.b, w, v);
                self.b.def_var(var, c);
                continue;
            }
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
                // A signal reference takes two cells, stored as the interpreter's push does.
                Width::Sig => {
                    self.b.ins().store(self.gf, v, slots, off);
                    let Some(&w) = args.next() else {
                        return Err("missing argument cell".into());
                    };
                    self.b.ins().store(self.gf, w, slots, off + 8);
                }
            }
        }
        Ok(())
    }

    /// Zero the bytes `lo..hi` of the slots.
    fn zero_range(&mut self, slots: Value, lo: u64, hi: u64) {
        if hi <= lo {
            return;
        }
        let z = self.c64(0);
        if hi - lo <= 128 {
            let mut at = lo;
            while at < hi {
                let rest = hi - at;
                // Aligned words where the range allows, bytes at its edges.
                if at.is_multiple_of(8) && rest >= 8 {
                    self.b.ins().store(self.gf, z, slots, at as i32);
                    at += 8;
                } else if at.is_multiple_of(4) && rest >= 4 {
                    self.b.ins().istore32(self.gf, z, slots, at as i32);
                    at += 4;
                } else {
                    self.b.ins().istore8(self.gf, z, slots, at as i32);
                    at += 1;
                }
            }
        } else {
            let p = self.b.ins().iadd_imm_s(slots, lo as i64);
            let n = self.c64((hi - lo) as i64);
            self.b.ins().call(self.imp.zero, &[p, n]);
        }
    }

    /// Record the native frame at `pc` for the interpreter (`yavs_rt_deopt`): the callback copies
    /// its registers from the register array, its slots are copied to the buffer it hands out.
    fn record_frame(&mut self, f: &Func, pc: Value) {
        let Some(n) = &self.nat else { return };
        let (func, nregs, slots, link) = (n.func, n.nregs, n.slots, n.up[0]);
        // The registers the interpreter preloads: constants and frame addresses, the same at
        // every exit (`leave_at` stores the others).
        for r in 0..self.vars.len() {
            let v = match (self.fixed[r], self.pre[r]) {
                (Some(c), _) => self.c64(c),
                (None, Some(_)) => self.var_ld(Reg(r as u32)),
                _ => continue,
            };
            self.b.ins().store(self.rf, v, self.regs, r as i32 * 8);
        }
        // Slot variables go back to the slots (closed frames).
        let svars: Vec<(u32, Width, Variable)> = self
            .nat
            .as_ref()
            .map(|n| n.svars.clone())
            .unwrap_or_default();
        for (o, w, var) in svars {
            let v = self.b.use_var(var);
            match w {
                Width::B1 => self.b.ins().istore8(self.gf, v, slots, o as i32),
                Width::B4 => self.b.ins().istore32(self.gf, v, slots, o as i32),
                _ => self.b.ins().store(self.gf, v, slots, o as i32),
            };
        }
        let tb = self.nat_temp_base();
        let fu = self.c32(func);
        let nr = self.c64(nregs as i64);
        let sl = self.c64(i64::from(f.frame));
        let regs = self.regs;
        self.b.ins().call(
            self.imp.deopt,
            &[self.env, self.mem, fu, pc, tb, nr, sl, link, regs],
        );
        if f.frame > 0 {
            let c = self.b.ins().call(self.imp.deopt_slots, &[self.env]);
            let sb = self.b.inst_results(c)[0];
            // A closed frame hands over the bytes in use; the buffer is zero elsewhere.
            let closed = self.nat.as_ref().and_then(|n| n.closed.clone());
            match closed {
                Some(ranges) => {
                    for (lo, hi) in ranges {
                        let d = self.b.ins().iadd_imm_s(sb, lo as i64);
                        let sp = self.b.ins().iadd_imm_s(slots, lo as i64);
                        let n = self.c64((hi - lo) as i64);
                        self.b.ins().call(self.imp.mov, &[d, sp, n]);
                    }
                }
                None => {
                    let fb = self.c64(i64::from(f.frame));
                    self.b.ins().call(self.imp.mov, &[sb, slots, fb]);
                }
            }
        }
    }

    /// `Inst::Call` of a function that runs on native frames: a call one frame deeper (direct when
    /// the variant is in this module, else through `Mem::ntab`, which a generator publishes its
    /// variants in), with no frame in the interpreter; the frame is materialised (this one, if
    /// native) when the depth limit or the native stack is near, when the callee has no code yet
    /// and when it cannot finish (`Mem::ndeopt` after the call).
    #[allow(clippy::too_many_arguments)]
    fn native_call(
        &mut self,
        pc: u32,
        inst: &Inst,
        caller: FuncId,
        f: &Func,
        nr: NatRef,
        callee: FuncId,
        g: &Func,
        args: yavs_ir::Regs,
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
        let mut okc = self.b.ins().band(a, b2);
        // Native stack is left (`NEST_STACK`).
        let sp = self.b.ins().get_stack_pointer(I64);
        let lo = self.mem_ld(MEM_STACK_LO);
        let room = self.b.ins().icmp(IntCC::UnsignedGreaterThan, sp, lo);
        okc = self.b.ins().band(okc, room);
        // Through the table: nothing there yet (null) is a materialised frame.
        let target = match nr.fr {
            Some(_) => None,
            None => {
                let tab = self.mem_ld(MEM_NTAB);
                let at = self.b.ins().load(
                    I64,
                    MemFlagsData::trusted(),
                    tab,
                    (u64::from(callee.0) * 8) as i32,
                );
                let set = self.b.ins().icmp_imm_s(IntCC::NotEqual, at, 0);
                okc = self.b.ins().band(okc, set);
                Some(at)
            }
        };
        self.guard(okc, slow);
        let mut cargs = vec![self.env, self.mem, d1];
        if let Some(p) = g.parent {
            // The enclosing frames: the caller's own frame when it encloses the callee, else
            // those its own static links name.
            let up = |s: &mut Self, k: usize| match &s.nat {
                Some(n) => n.up[k],
                None => s.mem_ld(MEM_UP + 8 * k as u64),
            };
            let (u1, u2) = if p == caller {
                let own = match &self.nat {
                    Some(n) => self
                        .b
                        .ins()
                        .bor_imm_s(n.depth, i64::from(space::FRAME) << 29),
                    None => self.mem_ld(MEM_FRAME),
                };
                (own, up(self, 0))
            } else {
                (up(self, 0), up(self, 1))
            };
            debug_assert!(f.parent == Some(p) || p == caller);
            let known = self.b.ins().icmp_imm_s(IntCC::NotEqual, u1, -1);
            self.guard(known, slow);
            cargs.extend([u1, u2]);
        }
        if let Some(o) = obj {
            cargs.push(self.ld(o));
        }
        let rs = f.regs(args).unwrap_or(&[]).to_vec();
        for (r, p) in rs
            .iter()
            .zip(g.params.iter().skip(usize::from(obj.is_some())))
        {
            cargs.push(self.ld(*r));
            if p.width == Width::Sig {
                cargs.push(self.ld(Reg(r.0 + 1)));
            }
        }
        let call = match (nr.fr, target) {
            (Some(fr), _) => self.b.ins().call(fr, &cargs),
            (None, Some(at)) => {
                let sig = self.b.import_signature(native_sig(self.imp.cc, g));
                self.b.ins().call_indirect(sig, at, &cargs)
            }
            (None, None) => unreachable!("an indirect call has its address"),
        };
        let val = self.b.inst_results(call)[0];
        if obj.is_some() {
            self.mem_st(MEM_FLAGS, flags);
        }
        // The callee recorded its frames and unwound (`yavs_rt_deopt`).
        let nd = self.mem_ld(MEM_NDEOPT);
        let (out, cont) = (self.b.create_block(), self.b.create_block());
        self.b.set_cold_block(out);
        self.b.ins().brif(nd, out, &[], cont, &[]);
        self.b.switch_to_block(out);
        if self.nat.is_some() {
            // This frame is recorded too, at its call.
            self.leave_at(pc);
        } else {
            let pcv = self.c32(pc);
            let c = self
                .b
                .ins()
                .call(self.imp.deopt_end, &[self.env, self.mem, pcv]);
            let r = self.b.inst_results(c)[0];
            self.leave(r);
        }
        self.b.switch_to_block(cont);
        if let Some(d) = dst {
            self.st(d, val);
        }
        self.join(join);
    }

    /// Return `v` (an i32): a jump to the function's exit block.
    fn leave(&mut self, v: Value) {
        self.b.ins().jump(self.exit, &[BlockArg::Value(v)]);
    }

    /// Leave a variant for the interpreter at `pc`: the registers live there go to the register
    /// array the frame is recorded from (`record_frame`), the preloaded ones are stored by the
    /// exit block.
    fn leave_at(&mut self, pc: u32) {
        if self.nat.is_some() {
            let n = self.vars.len();
            let row: Option<Vec<u64>> = self.live.as_ref().map(|l| l.row(pc as usize).to_vec());
            for r in 0..n {
                let live = row
                    .as_ref()
                    .is_none_or(|w| w.get(r / 64).is_some_and(|x| x >> (r % 64) & 1 != 0));
                let pre = self.pre[r].is_some() || self.fk_int(r);
                if live && !pre {
                    let v = self.b.use_var(self.vars[r]);
                    self.b.ins().store(self.rf, v, self.regs, r as i32 * 8);
                }
            }
        }
        let v = self.c32(pc);
        self.leave(v);
    }

    /// Whether register `r` is a constant the interpreter preloads (`fixed_consts`).
    fn fk_int(&self, r: usize) -> bool {
        self.fixed.get(r).copied().flatten().is_some()
    }

    /// Leave to the interpreter at `pc` when `bad` holds; code goes on after it.
    fn exit_if(&mut self, bad: Value, pc: u32) {
        let out = self.b.create_block();
        let cont = self.b.create_block();
        self.b.set_cold_block(out);
        self.b.ins().brif(bad, out, &[], cont, &[]);
        self.b.switch_to_block(out);
        self.leave_at(pc);
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
            xstep: false,
        });
        (block, join)
    }

    /// `slow_path` for an instruction a native frame can have the interpreter execute on a frame
    /// of its own (`xstep_wide`): the code goes on after it instead of leaving.
    fn slow_path_x(&mut self, pc: u32, i: &Inst, f: &Func) -> (Block, Block) {
        let (block, join) = self.slow_path(pc, i);
        if self.nat.is_some()
            && xstep_wide(f, i)
            && let Some(sl) = self.slows.last_mut()
        {
            sl.xstep = true;
        }
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
        reg: Option<Reg>,
        extra: Op,
        n: Op,
        write: bool,
        slow: Block,
    ) -> Value {
        let frame = reg.and_then(|r| self.frame_of(r));
        let heap = reg.and_then(|r| match self.known.get(r.0 as usize) {
            Some(Some(Known::Heap(o))) => Some(*o),
            _ => None,
        });
        if let Some(Some(o)) = heap
            && self.nat.is_some()
        {
            // An object in hand: only the bounds of the access.
            let (ev, nv) = (self.op(extra), self.op(n));
            let off = self.b.ins().iadd(o.off, ev);
            let fits = self.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, nv, o.len);
            let rest = self.b.ins().isub(o.len, nv);
            let inside = self.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, off, rest);
            let okc = self.b.ins().band(fits, inside);
            self.guard(okc, slow);
            return self.b.ins().iadd(o.ptr, off);
        }
        if heap.is_some() && frame.is_none() {
            // A heap object, in no frame, temporary area, instance data or constant: its entry.
            let (ev, nv) = (self.op(extra), self.op(n));
            let hi = self.b.ins().ushr_imm_s(a, 32);
            let lo = self.lo32(a);
            let off = self.b.ins().iadd(lo, ev);
            let done = self.b.create_block();
            let p = self.b.append_block_param(done, I64);
            self.obj_lookup(a, hi, off, ev, nv, write, done);
            self.b.switch_to_block(done);
            let nn = self.b.ins().icmp_imm_s(IntCC::NotEqual, p, 0);
            self.guard(nn, slow);
            return p;
        }
        // A constant never changes: a read is from a copy of its own.
        if !write
            && let Some(r) = reg
            && let Some(Some(Known::Const(k, o))) = self.known.get(r.0 as usize).copied()
            && let Some(&(gv, len)) = self.imp.consts.get(&k)
        {
            let (ev, nv) = (self.op(extra), self.op(n));
            let off = self.b.ins().iadd_imm_s(ev, o as i64);
            let lenv = self.c64(len as i64);
            let fits = self.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, nv, lenv);
            let rest = self.b.ins().isub(lenv, nv);
            let inside = self.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, off, rest);
            let okc = self.b.ins().band(fits, inside);
            self.guard(okc, slow);
            let base = self.b.ins().symbol_value(I64, gv);
            return self.b.ins().iadd(base, off);
        }
        // The temporary area, the instance data (reading) and the frames of enclosing
        // subprograms (frame-mode code), whose space is known: the view's bytes, no lookup.
        let area = reg.and_then(|r| match self.known.get(r.0 as usize) {
            Some(Some(Known::Temp)) => Some((MEM_TEMP, MEM_TEMP_LEN)),
            Some(Some(Known::Inst(_) | Known::InstAny)) if !write => Some((MEM_INST, MEM_INST_LEN)),
            Some(Some(Known::Up(d))) if self.nat.is_none() => Some((
                MEM_UP_SLOTS + 8 * u64::from(*d - 1),
                MEM_UP_LEN + 8 * u64::from(*d - 1),
            )),
            _ => None,
        });
        if let Some((base_f, len_f)) = area {
            let (ev, nv) = (self.op(extra), self.op(n));
            let lo = self.lo32(a);
            let off = self.b.ins().iadd(lo, ev);
            let len = self.mem_ld(len_f);
            let fits = self.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, nv, len);
            let rest = self.b.ins().isub(len, nv);
            let inside = self.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, off, rest);
            let okc = self.b.ins().band(fits, inside);
            self.guard(okc, slow);
            let base = self.mem_ld(base_f);
            return self.b.ins().iadd(base, off);
        }
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
        let lo = self.lo32(a);
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
            let closed = self.nat.as_ref().is_some_and(|n| n.closed.is_some());
            let (other, up, rest) = (
                self.b.create_block(),
                self.b.create_block(),
                self.b.create_block(),
            );
            // No address of a closed frame is of unknown space.
            if closed {
                self.b.ins().jump(other, &[]);
            } else {
                let own = self.b.create_block();
                let flen = self.c64(self.frame_len as i64);
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
            }
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
        self.obj_lookup(a, hi, off, extra, n, write, done);
        self.b.switch_to_block(done);
        let nn = self.b.ins().icmp_imm_s(IntCC::NotEqual, p, 0);
        self.guard(nn, slow);
        p
    }

    /// The object table's entry for the access (the objects found before: heap objects a `Deref`
    /// found, a process's heap variables): one load and compare, as the LLVM generator does;
    /// else `yavs_rt_host` (`Mem::find`, then the interpreter is asked for the object). Jumps
    /// to `done` with the host pointer (0: not available).
    #[allow(clippy::too_many_arguments)]
    fn obj_lookup(
        &mut self,
        a: Value,
        hi: Value,
        off: Value,
        extra: Value,
        n: Value,
        write: bool,
        done: Block,
    ) {
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
        let p = self.host(a, Some(base), Op::V(off), Op::C(32), write, slow);
        (p, self.b.ins().band_imm_s(i, 63))
    }

    /// `Inst::LCopy` of `k` (1 to 64) elements from bit `s0` of the one or two plane blocks at
    /// `ps` (`ns` of them) to bit `d0` of those at `pd` (`nd`), as `copy_bits` does: the source
    /// planes are read first (the ranges may overlap), the destination bits outside the copied
    /// range stay.
    #[allow(clippy::too_many_arguments)]
    fn lcopy_inline(
        &mut self,
        pd: Value,
        nd: Value,
        d0: Value,
        ps: Value,
        ns: Value,
        s0: Value,
        k: Value,
    ) {
        let gf = self.gf;
        let full = self
            .b
            .ins()
            .icmp_imm_s(IntCC::UnsignedGreaterThanOrEqual, k, 64);
        let one = self.c64(1);
        let sh = self.b.ins().ishl(one, k);
        let low = self.b.ins().iadd_imm_s(sh, -1);
        let ones = self.c64(-1);
        let mask = self.b.ins().select(full, ones, low);
        let two_s = self.b.ins().icmp_imm_s(IntCC::Equal, ns, 2);
        let two_d = self.b.ins().icmp_imm_s(IntCC::Equal, nd, 2);
        let zero = self.c64(0);
        let wide = |g: &mut Self, p: Value, plane: i32, two: Value| -> Value {
            let l = g.b.ins().load(I64, gf, p, plane * 8);
            // Block 1 is only read when there is one.
            let b1 = g.b.ins().iadd_imm_s(p, 32);
            let hp = g.b.ins().select(two, b1, p);
            let h = g.b.ins().load(I64, gf, hp, plane * 8);
            let h = g.b.ins().select(two, h, zero);
            let (l, h) = (g.b.ins().uextend(I128, l), g.b.ins().uextend(I128, h));
            let hs = g.b.ins().ishl_imm_s(h, 64);
            g.b.ins().bor(l, hs)
        };
        let s128 = self.b.ins().uextend(I128, s0);
        let d128 = self.b.ins().uextend(I128, d0);
        let mut vals = [zero; 4];
        for (plane, v) in vals.iter_mut().enumerate() {
            let w = wide(self, ps, plane as i32, two_s);
            let sh = self.b.ins().ushr(w, s128);
            let low = self.b.ins().ireduce(I64, sh);
            *v = self.b.ins().band(low, mask);
        }
        let m128 = self.b.ins().uextend(I128, mask);
        let hole = self.b.ins().ishl(m128, d128);
        let keep = self.b.ins().bnot(hole);
        for (plane, v) in vals.iter().enumerate() {
            let w = wide(self, pd, plane as i32, two_d);
            let v128 = self.b.ins().uextend(I128, *v);
            let put = self.b.ins().ishl(v128, d128);
            let kept = self.b.ins().band(w, keep);
            let out = self.b.ins().bor(kept, put);
            let lo = self.b.ins().ireduce(I64, out);
            self.b.ins().store(gf, lo, pd, plane as i32 * 8);
            // The second block: the high half, when the destination has one.
            let (hi_bb, done) = (self.b.create_block(), self.b.create_block());
            self.b.ins().brif(two_d, hi_bb, &[], done, &[]);
            self.b.switch_to_block(hi_bb);
            let top = self.b.ins().ushr_imm_s(out, 64);
            let hi = self.b.ins().ireduce(I64, top);
            self.b.ins().store(gf, hi, pd, 32 + plane as i32 * 8);
            self.b.ins().jump(done, &[]);
            self.b.switch_to_block(done);
        }
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
        match self.const_of(n) {
            // A known size at an aligned end is zeroed by stores, not a call.
            Some(k) if (1..=128).contains(&k) => {
                let (even, odd, merge) = (
                    self.b.create_block(),
                    self.b.create_block(),
                    self.b.create_block(),
                );
                let aligned = self.b.ins().icmp(IntCC::Equal, off, len);
                self.b.ins().brif(aligned, even, &[], odd, &[]);
                self.b.switch_to_block(even);
                self.zero_range(at, 0, k as u64);
                self.b.ins().jump(merge, &[]);
                self.b.switch_to_block(odd);
                self.b.ins().call(self.imp.zero, &[at, fill]);
                self.b.ins().jump(merge, &[]);
                self.b.switch_to_block(merge);
            }
            _ => {
                self.b.ins().call(self.imp.zero, &[at, fill]);
            }
        }
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
            Some(desc),
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

/// A light `numeric_std` native computed in place (`yavs_llvm::native::ieee_fast`): the
/// native's own fast path (`ieee::small`) on operands of 1 to 64 elements without a metavalue,
/// as the LLVM generator computes it (`Gen::ieee_fast`); any other operand, and every case that
/// warns or fails, runs the native.
impl Gen<'_> {
    /// An array operand (`data, left, right, dir` as values) of 1 to 64 elements that holds only
    /// `0 1 L H` (and, with `pure`, no `L` or `H`): `(n, bits, mask)`, taking `fb` for any other.
    fn small_operand(&mut self, r: &[Reg], pure: bool, fb: Block) -> (Value, Value, Value) {
        let (l, rt, d) = (self.ld(r[1]), self.ld(r[2]), self.ld(r[3]));
        let down = self.b.ins().icmp_imm_s(IntCC::Equal, d, 1);
        let hi = self.b.ins().select(down, l, rt);
        let lo = self.b.ins().select(down, rt, l);
        let diff = self.b.ins().isub(hi, lo);
        let ge = self.b.ins().icmp(IntCC::SignedGreaterThanOrEqual, hi, lo);
        let small = self.b.ins().icmp_imm_s(IntCC::UnsignedLessThan, diff, 64);
        let okn = self.b.ins().band(ge, small);
        self.guard(okn, fb);
        let n = self.b.ins().iadd_imm_s(diff, 1);
        let two = self.c64(2);
        let sh = self.b.ins().ishl(two, diff);
        let mask = self.b.ins().iadd_imm_s(sh, -1);
        let a = self.ld(r[0]);
        let p = self.host(a, Some(r[0]), Op::C(0), Op::C(32), false, fb);
        let gf = self.gf;
        let w: Vec<Value> = (0..4)
            .map(|k| self.b.ins().load(I64, gf, p, 8 * k))
            .collect();
        let live = self.b.ins().band_not(w[1], w[3]);
        // `0 L`: plane 1 only; `1 H`: planes 0 and 1.
        let zero = self.b.ins().band_not(live, w[0]);
        let one = self.b.ins().band(live, w[0]);
        let both = self.b.ins().bor(zero, one);
        let all = self.b.ins().band(both, mask);
        let oka = self.b.ins().icmp(IntCC::Equal, all, mask);
        self.guard(oka, fb);
        if pure {
            // Plane 2 is the weak bit of `L` and `H`.
            let weak = self.b.ins().band(w[2], mask);
            let okw = self.b.ins().icmp_imm_s(IntCC::Equal, weak, 0);
            self.guard(okw, fb);
        }
        let bits = self.b.ins().band(one, mask);
        (n, bits, mask)
    }

    /// The low `n` bits (0 to 64) as a mask.
    fn low_mask(&mut self, n: Value) -> Value {
        let full = self.b.ins().icmp_imm_s(IntCC::Equal, n, 64);
        let one = self.c64(1);
        let sh = self.b.ins().ishl(one, n);
        let low = self.b.ins().iadd_imm_s(sh, -1);
        let ones = self.c64(-1);
        self.b.ins().select(full, ones, low)
    }

    /// The value of `bits` of `n` elements as a signed integer.
    fn sext_bits(&mut self, x: Value, n: Value) -> Value {
        let sixty4 = self.c64(64);
        let k = self.b.ins().isub(sixty4, n);
        let up = self.b.ins().ishl(x, k);
        self.b.ins().sshr(up, k)
    }

    #[allow(clippy::too_many_arguments)]
    fn ieee_native(
        &mut self,
        pc: u32,
        inst: &Inst,
        f: &Func,
        op: yavs_ir::ieee::Op,
        native: yavs_ir::NativeId,
        args: yavs_ir::Regs,
        dst: Option<Reg>,
    ) -> Result<(), String> {
        use yavs_ir::ieee::{ARRAY_REGS, Cmp, Form, Gate, Op, Shift};
        let rs = ok(f.regs(args), "operand list out of range")?.to_vec();
        let d = ok(dst, "a native without a result")?;
        let layout = ok(ieee_layout(op), "not a native computed in place")?;
        let (fb, done) = (self.b.create_block(), self.b.create_block());
        let pure = matches!(op, Op::Resize { .. } | Op::Shift(_) | Op::ShiftFn(_));
        let (mut arrs, mut ints) = (Vec::new(), Vec::new());
        let mut at = 0;
        for a in layout {
            match a {
                Arg::Arr => {
                    arrs.push(self.small_operand(&rs[at..at + ARRAY_REGS], pure, fb));
                    at += ARRAY_REGS;
                }
                Arg::Int => {
                    ints.push(self.ld(rs[at]));
                    at += 1;
                }
            }
        }
        let c0 = self.c64(0);
        let c1 = self.c64(1);
        // `natural`: a negative integer is the native's error.
        let nonneg =
            |g: &mut Self, x: Value| g.b.ins().icmp(IntCC::SignedGreaterThanOrEqual, x, c0);
        // Unsigned or signed `size` low bits hold `v` (the native warns when they do not).
        let fits = |g: &mut Self, v: Value, n: Value, mask: Value, signed: bool| {
            if signed {
                let m = g.b.ins().band(v, mask);
                let e = g.sext_bits(m, n);
                g.b.ins().icmp(IntCC::Equal, e, v)
            } else {
                let nn = nonneg(g, v);
                let hi = g.b.ins().band_not(v, mask);
                let z = g.b.ins().icmp_imm_s(IntCC::Equal, hi, 0);
                g.b.ins().band(nn, z)
            }
        };
        enum Out {
            Scalar(Value),
            // Count, bits, and whether the bounds are `1 to n` (else `n - 1 downto 0`).
            Vector(Value, Value, bool),
        }
        let out = match op {
            Op::ToInteger { signed } => {
                let (n, bits, mask) = arrs[0];
                let nm1 = self.b.ins().isub(n, c1);
                let sh = self.b.ins().ushr(bits, nm1);
                let top = self.b.ins().band(sh, c1);
                let neg = if signed {
                    self.b.ins().icmp_imm_s(IntCC::NotEqual, top, 0)
                } else {
                    self.b.ins().icmp(IntCC::NotEqual, c0, c0)
                };
                if signed {
                    // `to_signed (1, 1)` truncates: the native words that warning.
                    let one_el = self.b.ins().icmp_imm_s(IntCC::Equal, n, 1);
                    let bad = self.b.ins().band(one_el, neg);
                    let okb = self.b.ins().bxor_imm_s(bad, 1);
                    self.guard(okb, fb);
                }
                let nb = self.b.ins().band_not(mask, bits);
                let mag = self.b.ins().select(neg, nb, bits);
                // Beyond `integer'high` of the shortest revision the native decides.
                let okm = self
                    .b
                    .ins()
                    .icmp_imm_s(IntCC::UnsignedLessThanOrEqual, mag, 0x7fff_ffff);
                self.guard(okm, fb);
                let nm = self.b.ins().bnot(mag);
                Out::Scalar(self.b.ins().select(neg, nm, mag))
            }
            Op::Add { sub, signed, form } => {
                let (n, x, y) = match form {
                    Form::Vv => {
                        let ((n1, b1, _), (n2, b2, _)) = (arrs[0], arrs[1]);
                        let n = self.b.ins().umax(n1, n2);
                        let mask = self.low_mask(n);
                        let ext = |g: &mut Self, bits: Value, k: Value| {
                            if signed {
                                let e = g.sext_bits(bits, k);
                                g.b.ins().band(e, mask)
                            } else {
                                bits
                            }
                        };
                        (n, ext(self, b1, n1), ext(self, b2, n2))
                    }
                    _ => {
                        let (n, bits, mask) = arrs[0];
                        let v = ints[0];
                        let okf = fits(self, v, n, mask, signed);
                        self.guard(okf, fb);
                        let iv = self.b.ins().band(v, mask);
                        if form == Form::Vi {
                            (n, bits, iv)
                        } else {
                            (n, iv, bits)
                        }
                    }
                };
                let r = if sub {
                    self.b.ins().isub(x, y)
                } else {
                    self.b.ins().iadd(x, y)
                };
                Out::Vector(n, r, false)
            }
            Op::Compare {
                cmp: k,
                signed,
                form,
            } => {
                let (x, y) = match form {
                    Form::Vv => {
                        let ((n1, b1, _), (n2, b2, _)) = (arrs[0], arrs[1]);
                        if signed {
                            (self.sext_bits(b1, n1), self.sext_bits(b2, n2))
                        } else {
                            (b1, b2)
                        }
                    }
                    _ => {
                        let (n, bits, _) = arrs[0];
                        let v = ints[0];
                        if !signed {
                            let nn = nonneg(self, v);
                            self.guard(nn, fb);
                        }
                        let e = if signed {
                            self.sext_bits(bits, n)
                        } else {
                            bits
                        };
                        if form == Form::Vi { (e, v) } else { (v, e) }
                    }
                };
                let cc = match (k, signed) {
                    (Cmp::Gt, false) => IntCC::UnsignedGreaterThan,
                    (Cmp::Lt, false) => IntCC::UnsignedLessThan,
                    (Cmp::Le, false) => IntCC::UnsignedLessThanOrEqual,
                    (Cmp::Ge, false) => IntCC::UnsignedGreaterThanOrEqual,
                    (Cmp::Gt, true) => IntCC::SignedGreaterThan,
                    (Cmp::Lt, true) => IntCC::SignedLessThan,
                    (Cmp::Le, true) => IntCC::SignedLessThanOrEqual,
                    (Cmp::Ge, true) => IntCC::SignedGreaterThanOrEqual,
                    (Cmp::Eq, _) => IntCC::Equal,
                    (Cmp::Ne, _) => IntCC::NotEqual,
                };
                let r = self.b.ins().icmp(cc, x, y);
                Out::Scalar(self.b.ins().uextend(I64, r))
            }
            Op::ToUnsigned | Op::ToSigned => {
                let signed = op == Op::ToSigned;
                let (v, size) = (ints[0], ints[1]);
                let ge = self.b.ins().icmp(IntCC::SignedGreaterThanOrEqual, size, c1);
                let le = self
                    .b
                    .ins()
                    .icmp_imm_s(IntCC::SignedLessThanOrEqual, size, 64);
                let okr = self.b.ins().band(ge, le);
                self.guard(okr, fb);
                let mask = self.low_mask(size);
                let okf = fits(self, v, size, mask, signed);
                self.guard(okf, fb);
                let bits = self.b.ins().band(v, mask);
                Out::Vector(size, bits, false)
            }
            Op::Resize { signed } => {
                let (n, bits, _) = arrs[0];
                let new = ints[0];
                let ge = self.b.ins().icmp(IntCC::SignedGreaterThanOrEqual, new, c1);
                let le = self
                    .b
                    .ins()
                    .icmp_imm_s(IntCC::SignedLessThanOrEqual, new, 64);
                let okr = self.b.ins().band(ge, le);
                self.guard(okr, fb);
                let res = if signed {
                    let m = self.b.ins().umin(n, new);
                    let keep = self.b.ins().isub(m, c1);
                    let nm1 = self.b.ins().isub(n, c1);
                    let sh = self.b.ins().ushr(bits, nm1);
                    let sb = self.b.ins().band(sh, c1);
                    let sign = self.b.ins().icmp_imm_s(IntCC::NotEqual, sb, 0);
                    let lk = self.low_mask(keep);
                    let kept = self.b.ins().band(bits, lk);
                    let ln = self.low_mask(new);
                    let ext = self.b.ins().band_not(ln, lk);
                    let pickext = self.b.ins().select(sign, ext, c0);
                    self.b.ins().bor(kept, pickext)
                } else {
                    let m = self.b.ins().umin(n, new);
                    let lm = self.low_mask(m);
                    self.b.ins().band(bits, lm)
                };
                Out::Vector(new, res, false)
            }
            Op::Shift(k) | Op::ShiftFn(k) => {
                let (n, bits, mask) = arrs[0];
                let count = ints[0];
                let nn = nonneg(self, count);
                self.guard(nn, fb);
                let big = self
                    .b
                    .ins()
                    .icmp(IntCC::UnsignedGreaterThanOrEqual, count, n);
                let cnt = self.b.ins().select(big, c0, count);
                let res = match k {
                    Shift::Sll => {
                        let r = self.b.ins().ishl(bits, cnt);
                        self.b.ins().select(big, c0, r)
                    }
                    Shift::Srl => {
                        let r = self.b.ins().ushr(bits, cnt);
                        self.b.ins().select(big, c0, r)
                    }
                    Shift::Sra => {
                        // For 0 or 1 elements and a zero count the native keeps the argument's
                        // own bounds.
                        let gt = self.b.ins().icmp(IntCC::UnsignedGreaterThan, n, c1);
                        let nz = self.b.ins().icmp_imm_s(IntCC::NotEqual, count, 0);
                        let okb = self.b.ins().band(gt, nz);
                        self.guard(okb, fb);
                        let nm1 = self.b.ins().isub(n, c1);
                        let kk = self.b.ins().umin(count, nm1);
                        let se = self.sext_bits(bits, n);
                        let r = self.b.ins().sshr(se, kk);
                        self.b.ins().band(r, mask)
                    }
                    // A rotation by `count mod n`; by zero both shifts are by zero.
                    Shift::Rol | Shift::Ror => {
                        let kk = self.b.ins().urem(count, n);
                        let zero = self.b.ins().icmp_imm_s(IntCC::Equal, kk, 0);
                        let nmk = self.b.ins().isub(n, kk);
                        let inv = self.b.ins().select(zero, c0, nmk);
                        let (first, second) = if k == Shift::Rol {
                            (kk, inv)
                        } else {
                            (inv, kk)
                        };
                        let l = self.b.ins().ishl(bits, first);
                        let r = self.b.ins().ushr(bits, second);
                        self.b.ins().bor(l, r)
                    }
                };
                Out::Vector(n, res, false)
            }
            Op::NumGate(g) | Op::VecGate(g) => {
                let ((n1, b1, _), (n2, b2, _)) = (arrs[0], arrs[1]);
                let same = self.b.ins().icmp(IntCC::Equal, n1, n2);
                self.guard(same, fb);
                let res = match g {
                    Gate::And => self.b.ins().band(b1, b2),
                    Gate::Nand => {
                        let a = self.b.ins().band(b1, b2);
                        self.b.ins().bnot(a)
                    }
                    Gate::Or => self.b.ins().bor(b1, b2),
                    Gate::Nor => {
                        let a = self.b.ins().bor(b1, b2);
                        self.b.ins().bnot(a)
                    }
                    Gate::Xor => self.b.ins().bxor(b1, b2),
                    Gate::Xnor => {
                        let a = self.b.ins().bxor(b1, b2);
                        self.b.ins().bnot(a)
                    }
                };
                Out::Vector(n1, res, matches!(op, Op::VecGate(_)))
            }
            Op::NumNot | Op::VecNot => {
                let (n, bits, _) = arrs[0];
                let r = self.b.ins().bnot(bits);
                Out::Vector(n, r, op == Op::VecNot)
            }
            _ => return Err("not a native computed in place".into()),
        };
        match out {
            Out::Scalar(v) => self.st(d, v),
            Out::Vector(n, bits, up) => {
                let mask = self.low_mask(n);
                let sz = self.c64(i64::from(PUT_SMALL_BYTES));
                let (at, p) = self.temp_alloc(sz, fb);
                let (left, right, down) = if up {
                    (c1, n, c0)
                } else {
                    (self.b.ins().iadd_imm_s(n, -1), c0, c1)
                };
                let data = self
                    .b
                    .ins()
                    .iadd_imm_s(at, (16 + yavs_ir::DOPE_DIM_BYTES) as i64);
                let dope = self.b.ins().iadd_imm_s(at, 16);
                let masked = self.b.ins().band(bits, mask);
                let gf = self.gf;
                for (k, w) in [data, dope, left, right, down, masked, mask, c0, c0]
                    .into_iter()
                    .enumerate()
                {
                    self.b.ins().store(gf, w, p, 8 * k as i32);
                }
                self.st(d, at);
            }
        }
        self.b.ins().jump(done, &[]);
        self.b.switch_to_block(fb);
        self.clear();
        self.native_direct(pc, inst, f, native, args, dst)?;
        self.b.ins().jump(done, &[]);
        self.start(done);
        Ok(())
    }
}

/// Whether `LogicOp` of `op` is lowered here (the others are the interpreter's).
fn logic_inline(op: yavs_ir::LOp) -> bool {
    use yavs_ir::LOp::*;
    matches!(
        op,
        And | Or | Nand | Nor | Xor | Xnor | Not | ToX01 | ToUx01
    )
}

/// Bit-parallel std_ulogic classes of a block of 64 elements (`yavs-logic` planes: code bits 0 to
/// 3 in planes 0 to 3).
impl Gen<'_> {
    fn not64(&mut self, x: Value) -> Value {
        self.b.ins().bnot(x)
    }
    fn is_u(&mut self, x: [Value; 4]) -> Value {
        let o = self.b.ins().bor(x[0], x[1]);
        let o = self.b.ins().bor(o, x[2]);
        let o = self.b.ins().bor(o, x[3]);
        self.not64(o)
    }
    /// `0` or `L`: not bit 3, bit 1 and not bit 0.
    fn is0(&mut self, x: [Value; 4]) -> Value {
        let nd = self.b.ins().band_not(x[1], x[3]);
        self.b.ins().band_not(nd, x[0])
    }
    /// `1` or `H`.
    fn is1(&mut self, x: [Value; 4]) -> Value {
        let nd = self.b.ins().band_not(x[1], x[3]);
        self.b.ins().band(nd, x[0])
    }

    /// One block of `op`: planes 0 and 1 of the result (2 and 3 are zero).
    fn logic_block(&mut self, op: yavs_ir::LOp, x: [Value; 4], y: [Value; 4]) -> [Value; 2] {
        use yavs_ir::LOp::*;
        let ins = |g: &mut Self| {
            let (ux, uy) = (g.is_u(x), g.is_u(y));
            g.b.ins().bor(ux, uy)
        };
        let and = |g: &mut Self| {
            let (zx, zy) = (g.is0(x), g.is0(y));
            let z = g.b.ins().bor(zx, zy);
            let u = ins(g);
            let (ox, oy) = (g.is1(x), g.is1(y));
            let one = g.b.ins().band(ox, oy);
            let nu = g.b.ins().bnot(u);
            let nz = g.b.ins().band_not(nu, z);
            [nz, g.b.ins().bor(z, one)]
        };
        let or = |g: &mut Self| {
            let (ox, oy) = (g.is1(x), g.is1(y));
            let o = g.b.ins().bor(ox, oy);
            let u = ins(g);
            let (zx, zy) = (g.is0(x), g.is0(y));
            let zz = g.b.ins().band(zx, zy);
            let nu = g.b.ins().bnot(u);
            let a = g.b.ins().band_not(nu, zz);
            let b = g.b.ins().band(nu, zz);
            [g.b.ins().bor(o, a), g.b.ins().bor(o, b)]
        };
        let xor = |g: &mut Self| {
            let u = ins(g);
            let (zx, ox) = (g.is0(x), g.is1(x));
            let (zy, oy) = (g.is0(y), g.is1(y));
            let kx = g.b.ins().bor(zx, ox);
            let ky = g.b.ins().bor(zy, oy);
            let k = g.b.ins().band(kx, ky);
            let v = g.b.ins().bxor(ox, oy);
            let nk = g.b.ins().bnot(k);
            let nu = g.b.ins().bnot(u);
            let t = g.b.ins().bor(nk, v);
            [g.b.ins().band(nu, t), g.b.ins().band(nu, k)]
        };
        let not = |g: &mut Self, r: [Value; 2]| {
            // Not of the planes 0 and 1 (planes 2 and 3 are zero): `[!u & !one, zero | one]`.
            let x2 = [r[0], r[1], g.c64(0), g.c64(0)];
            let (u, o, z) = (g.is_u(x2), g.is1(x2), g.is0(x2));
            let a = g.b.ins().bor(u, o);
            [g.b.ins().bnot(a), g.b.ins().bor(z, o)]
        };
        match op {
            And => and(self),
            Or => or(self),
            Xor => xor(self),
            Nand => {
                let r = and(self);
                not(self, r)
            }
            Nor => {
                let r = or(self);
                not(self, r)
            }
            Xnor => {
                let r = xor(self);
                not(self, r)
            }
            Not => {
                let (u, o, z) = (self.is_u(x), self.is1(x), self.is0(x));
                let a = self.b.ins().bor(u, o);
                [self.b.ins().bnot(a), self.b.ins().bor(z, o)]
            }
            ToX01 => {
                let (o, z) = (self.is1(x), self.is0(x));
                [self.b.ins().bnot(z), self.b.ins().bor(z, o)]
            }
            ToUx01 => {
                let (o, z, u) = (self.is1(x), self.is0(x), self.is_u(x));
                let a = self.b.ins().bor(z, u);
                [self.b.ins().bnot(a), self.b.ins().bor(z, o)]
            }
            _ => unreachable!("not lowered inline"),
        }
    }

    /// `Inst::LogicOp` of a logic array pair, block by block (64 elements, four planes of 8
    /// bytes); the planes beyond the length are zero.
    #[allow(clippy::too_many_arguments)]
    fn logic_op(
        &mut self,
        pc: u32,
        inst: &Inst,
        f: &Func,
        op: yavs_ir::LOp,
        dst: Reg,
        a: Reg,
        b: Reg,
        n: Reg,
    ) {
        let (slow, join) = self.slow_path_x(pc, inst, f);
        let unary = matches!(
            op,
            yavs_ir::LOp::Not | yavs_ir::LOp::ToX01 | yavs_ir::LOp::ToUx01
        );
        let cnt = self.ld(n);
        let small = self
            .b
            .ins()
            .icmp_imm_s(IntCC::UnsignedLessThan, cnt, 1 << 32);
        self.guard(small, slow);
        let c63 = self.b.ins().iadd_imm_s(cnt, 63);
        let nblk = self.b.ins().ushr_imm_s(c63, 6);
        let bytes = self.b.ins().ishl_imm_s(nblk, 5);
        let av = self.ld(a);
        let pa = self.host(av, Some(a), Op::C(0), Op::V(bytes), false, slow);
        let pb = if unary {
            pa
        } else {
            let bv = self.ld(b);
            self.host(bv, Some(b), Op::C(0), Op::V(bytes), false, slow)
        };
        let dv = self.ld(dst);
        let pd = self.host(dv, Some(dst), Op::C(0), Op::V(bytes), true, slow);
        let (head, body, done) = (
            self.b.create_block(),
            self.b.create_block(),
            self.b.create_block(),
        );
        let i = self.b.append_block_param(head, I64);
        let zero = self.c64(0);
        self.b.ins().jump(head, &[BlockArg::Value(zero)]);
        self.b.switch_to_block(head);
        let more = self.b.ins().icmp(IntCC::UnsignedLessThan, i, nblk);
        self.b.ins().brif(more, body, &[], done, &[]);
        self.b.switch_to_block(body);
        let off = self.b.ins().ishl_imm_s(i, 5);
        let (qa, qb, qd) = (
            self.b.ins().iadd(pa, off),
            self.b.ins().iadd(pb, off),
            self.b.ins().iadd(pd, off),
        );
        let gf = self.gf;
        let mut x = [zero; 4];
        let mut y = [zero; 4];
        for k in 0..4 {
            x[k] = self.b.ins().load(I64, gf, qa, k as i32 * 8);
            y[k] = if unary {
                x[k]
            } else {
                self.b.ins().load(I64, gf, qb, k as i32 * 8)
            };
        }
        let r = self.logic_block(op, x, y);
        // The last block keeps its elements only.
        let base = self.b.ins().ishl_imm_s(i, 6);
        let rem = self.b.ins().isub(cnt, base);
        let one = self.c64(1);
        let bit = self.b.ins().ishl(one, rem);
        let low = self.b.ins().iadd_imm_s(bit, -1);
        let fullb = self
            .b
            .ins()
            .icmp_imm_s(IntCC::UnsignedGreaterThanOrEqual, rem, 64);
        let ones = self.c64(-1);
        let mask = self.b.ins().select(fullb, ones, low);
        let (r0, r1) = (self.b.ins().band(r[0], mask), self.b.ins().band(r[1], mask));
        self.b.ins().store(gf, r0, qd, 0);
        self.b.ins().store(gf, r1, qd, 8);
        self.b.ins().store(gf, zero, qd, 16);
        self.b.ins().store(gf, zero, qd, 24);
        let next = self.b.ins().iadd_imm_s(i, 1);
        self.b.ins().jump(head, &[BlockArg::Value(next)]);
        self.b.switch_to_block(done);
        self.join(join);
    }

    /// `Inst::NewFrame` on a native frame: a frame-pinned temporary of `dsize` descriptor bytes and
    /// `size` data bytes (copied from `data`, or the `fill` bytes at `data` repeated), which the
    /// frame's statements then keep (`temp_base`).
    #[allow(clippy::too_many_arguments)]
    fn new_frame(
        &mut self,
        pc: u32,
        inst: &Inst,
        dst: Reg,
        desc: Reg,
        dsize: u32,
        data: Reg,
        size: Reg,
        fill: u32,
    ) {
        let (slow, join) = self.slow_path(pc, inst);
        let sz = self.ld(size);
        // Over 4 GiB is the interpreter's error; a size that is no multiple of the element is its
        // internal error.
        let mut okc = self
            .b
            .ins()
            .icmp_imm_s(IntCC::UnsignedLessThanOrEqual, sz, 1 << 32);
        if fill != 0 {
            let r = self.b.ins().urem_imm_u(sz, i64::from(fill));
            let rz = self.b.ins().icmp_imm_s(IntCC::Equal, r, 0);
            okc = self.b.ins().band(okc, rz);
        }
        self.guard(okc, slow);
        // The sources before the allocation: a failure leaves nothing changed.
        let pdesc = (dsize != 0).then(|| {
            let dv = self.ld(desc);
            self.host(
                dv,
                Some(desc),
                Op::C(0),
                Op::C(u64::from(dsize)),
                false,
                slow,
            )
        });
        let n_src = if fill == 0 {
            Op::V(sz)
        } else {
            Op::C(u64::from(fill))
        };
        let dv = self.ld(data);
        let psrc = self.host(dv, Some(data), Op::C(0), n_src, false, slow);
        let total = self.b.ins().iadd_imm_s(sz, i64::from(dsize));
        let (at, p) = self.temp_alloc(total, slow);
        if let Some(pd) = pdesc {
            let ds = self.c64(i64::from(dsize));
            self.b.ins().call(self.imp.mov, &[p, pd, ds]);
        }
        let val = self.b.ins().iadd_imm_s(p, i64::from(dsize));
        if fill == 0 {
            self.b.ins().call(self.imp.mov, &[val, psrc, sz]);
        } else {
            let count = self.b.ins().udiv_imm_u(sz, i64::from(fill));
            let fv = self.c64(i64::from(fill));
            self.b.ins().call(self.imp.fill, &[val, psrc, count, fv]);
        }
        // The frame's own statements must not free the object.
        if let Some(TempBase::Cell(cell)) = self.nat.as_ref().map(|n| n.temp_base) {
            let end = self.mem_ld(MEM_TEMP_LEN);
            self.b.ins().store(MemFlagsData::trusted(), end, cell, 0);
        }
        self.st(dst, at);
        self.join(join);
    }

    /// `Inst::Fill` on a native frame: `n` copies of the `size` bytes at `elem` from `dst` on.
    fn fill_inst(&mut self, pc: u32, inst: &Inst, dst: Reg, elem: Reg, n: Reg, size: u32) {
        let (slow, join) = self.slow_path(pc, inst);
        let cnt = self.ld(n);
        let sc = self.c64(i64::from(size));
        let (total, o) = self.b.ins().umul_overflow(cnt, sc);
        let no = self.b.ins().bxor_imm_s(o, 1);
        self.guard(no, slow);
        let ev = self.ld(elem);
        let pe = self.host(
            ev,
            Some(elem),
            Op::C(0),
            Op::C(u64::from(size)),
            false,
            slow,
        );
        let dv = self.ld(dst);
        let pd = self.host(dv, Some(dst), Op::C(0), Op::V(total), true, slow);
        // The element is read before anything is written (it may lie in the target).
        let src = if size == 1 {
            pe
        } else {
            let ss = self.b.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                size,
                3,
            ));
            let stage = self.b.ins().stack_addr(I64, ss, 0);
            self.b.ins().call(self.imp.mov, &[stage, pe, sc]);
            stage
        };
        self.b.ins().call(self.imp.fill, &[pd, src, cnt, sc]);
        self.join(join);
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
            xstep: false,
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
        let off = self.lo32(v);
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
        let pd = self.host(a, Some(dst), Op::C(0), Op::V(bytes), true, slow);
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
        let pd = self.host(a, Some(dst), Op::C(0), Op::C(32), true, slow);
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
        let w = self.host(a, Some(wave), Op::C(0), Op::V(want), false, slow);
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
        match self.nat.as_ref().map(|n| n.temp_base) {
            Some(TempBase::Len(v)) => v,
            Some(TempBase::Cell(cell)) => self.b.ins().load(I64, MemFlagsData::trusted(), cell, 0),
            None => self.b.ins().load(I64, MemFlagsData::trusted(), self.mem, 0),
        }
    }

    /// Materialise a native frame at `pc` (its exit block); the builder goes on in a new block,
    /// which nothing reaches.
    fn deopt_here(&mut self, pc: u32) {
        self.leave_at(pc);
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
                let p = self.host(a, Some(addr), Op::C(0), Op::C(16), false, rec);
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
                        self.host(dv, Some(d), Op::C(0), Op::C(n), false, slow)
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
            if let Some(TempBase::Cell(cell)) = self.nat.as_ref().map(|n| n.temp_base) {
                self.b.ins().store(MemFlagsData::trusted(), len, cell, 0);
            }
        }
        // The instruction ended the process: the frame is materialised after it.
        let (out, cont) = (self.b.create_block(), self.b.create_block());
        let stop = self.b.ins().icmp_imm_s(IntCC::Equal, st, 2);
        self.b.set_cold_block(out);
        self.branch(stop, out, cont);
        self.start(out);
        self.leave_at(pc + 1);
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
        if self.nat.is_some() {
            self.leave_at(pc + 1);
        } else {
            let v = self.c32(yavs_interp::TIER_SUSPEND);
            self.leave(v);
        }
        self.start(cont);
        self.join(join);
        Ok(())
    }

    /// `Inst::Cat` of a logic array whose element size is known and whose lengths are known only at
    /// run time: the same one-block form as `cat_block` for a result of at most 64 elements, the
    /// instruction's own form otherwise. `false` (nothing generated) where the element size is not
    /// known.
    #[allow(clippy::too_many_arguments)]
    fn cat_dyn(
        &mut self,
        pc: u32,
        i: &Inst,
        f: &Func,
        dst: Reg,
        n: Reg,
        m: Reg,
        parts: &[Reg],
        scalars: u16,
        bias: u8,
    ) -> bool {
        let Some(mv) = self.kint(m).and_then(|v| u64::try_from(v).ok()) else {
            return false;
        };
        if !parts.len().is_multiple_of(2) || parts.len() > 16 || mv == 0 || mv > 64 {
            return false;
        }
        let (slow, join) = self.slow_path_x(pc, i, f);
        let nv = self.ld(n);
        let okn = self
            .b
            .ins()
            .icmp_imm_s(IntCC::UnsignedLessThanOrEqual, nv, 64);
        self.guard(okn, slow);
        let total = self.b.ins().imul_imm_s(nv, mv as i64);
        let okt = self
            .b
            .ins()
            .icmp_imm_s(IntCC::UnsignedLessThanOrEqual, total, 64);
        self.guard(okt, slow);
        let zero = self.c64(0);
        let ones = self.c64(-1);
        let mut words = [zero; 4];
        let mut mask = zero;
        let mut at_k = zero;
        for (j, pr) in parts.as_chunks::<2>().0.iter().enumerate() {
            let scalar = scalars >> j & 1 == 1;
            let units = if scalar {
                self.c64(1)
            } else {
                let l = self.ld(pr[1]);
                let okl = self
                    .b
                    .ins()
                    .icmp_imm_s(IntCC::UnsignedLessThanOrEqual, l, 64);
                self.guard(okl, slow);
                let u = self.b.ins().imul_imm_s(l, mv as i64);
                let oku = self
                    .b
                    .ins()
                    .icmp_imm_s(IntCC::UnsignedLessThanOrEqual, u, 64);
                self.guard(oku, slow);
                u
            };
            // The pieces fill the result from its top end.
            let end = self.b.ins().iadd(at_k, units);
            let fits = self
                .b
                .ins()
                .icmp(IntCC::UnsignedLessThanOrEqual, end, total);
            self.guard(fits, slow);
            let off0 = self.b.ins().isub(total, at_k);
            let off = self.b.ins().isub(off0, units);
            let full = self.b.ins().icmp_imm_s(IntCC::Equal, units, 64);
            let one = self.c64(1);
            let sh = self.b.ins().ishl(one, units);
            let low = self.b.ins().iadd_imm_s(sh, -1);
            let pm = self.b.ins().select(full, ones, low);
            let bits: [Value; 4] = if scalar {
                let v = self.ld(pr[0]);
                let code = self.b.ins().iadd_imm_s(v, i64::from(bias));
                // A code beyond 'U' to '-' is the interpreter's error.
                let okc = self
                    .b
                    .ins()
                    .icmp_imm_s(IntCC::UnsignedLessThanOrEqual, code, 8);
                self.guard(okc, slow);
                std::array::from_fn(|p| {
                    let b = self.b.ins().ushr_imm_s(code, p as i64);
                    self.b.ins().band_imm_s(b, 1)
                })
            } else {
                // An empty piece names no memory.
                let (load, after) = (self.b.create_block(), self.b.create_block());
                let ws: Vec<Value> = (0..4)
                    .map(|_| self.b.append_block_param(after, I64))
                    .collect();
                let nz = self.b.ins().icmp_imm_s(IntCC::NotEqual, units, 0);
                self.b
                    .ins()
                    .brif(nz, load, &[], after, &[BlockArg::Value(zero); 4]);
                self.b.switch_to_block(load);
                let a = self.ld(pr[0]);
                let ptr = self.host(a, Some(pr[0]), Op::C(0), Op::C(32), false, slow);
                let nf = self.gf;
                let vs: Vec<BlockArg> = (0..4)
                    .map(|p| {
                        let w = self.b.ins().load(I64, nf, ptr, 8 * p);
                        BlockArg::Value(self.b.ins().band(w, pm))
                    })
                    .collect();
                self.b.ins().jump(after, &vs);
                self.b.switch_to_block(after);
                // What the load block cached does not dominate the code after it.
                self.clear();
                [ws[0], ws[1], ws[2], ws[3]]
            };
            for p in 0..4 {
                let sh = self.b.ins().ishl(bits[p], off);
                words[p] = self.b.ins().bor(words[p], sh);
            }
            let pmo = self.b.ins().ishl(pm, off);
            mask = self.b.ins().bor(mask, pmo);
            at_k = end;
        }
        // Nothing covered: no memory is touched.
        let (put, done) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(mask, put, &[], done, &[]);
        self.b.switch_to_block(put);
        let d = self.ld(dst);
        let pd = self.host(d, Some(dst), Op::C(0), Op::C(32), true, slow);
        let nf = self.gf;
        let keepm = self.b.ins().bnot(mask);
        for (p, &word) in words.iter().enumerate() {
            let cur = self.b.ins().load(I64, nf, pd, 8 * p as i32);
            let keep = self.b.ins().band(cur, keepm);
            let v = self.b.ins().bor(keep, word);
            self.b.ins().store(nf, v, pd, 8 * p as i32);
        }
        self.b.ins().jump(done, &[]);
        self.b.switch_to_block(done);
        self.join(join);
        true
    }

    /// `Inst::Cat` of a logic array of at most 64 elements whose piece lengths are known here: the
    /// planes of all pieces are combined in registers and stored once, as `Machine::cat_block`
    /// does. `false` (nothing generated) where a length is not known or does not fit that pattern;
    /// a runtime case it does not take (a code beyond 'U' to '-', memory compiled code does not
    /// reach) steps the whole instruction before any store.
    #[allow(clippy::too_many_arguments)]
    fn cat_block(
        &mut self,
        pc: u32,
        i: &Inst,
        dst: Reg,
        n: Reg,
        m: Reg,
        parts: &[Reg],
        scalars: u16,
        bias: u8,
    ) -> bool {
        let (Some(n), Some(m)) = (self.kint(n), self.kint(m)) else {
            return false;
        };
        let Some(total) = u64::try_from(n)
            .ok()
            .zip(u64::try_from(m).ok())
            .and_then(|(n, m)| n.checked_mul(m))
            .filter(|&t| t <= 64)
        else {
            return false;
        };
        // (register, units, scalar) per piece, empty ones dropped, as `cat_block` skips them.
        let mut pieces = Vec::new();
        let mut k = 0u64;
        for (j, pr) in parts.as_chunks::<2>().0.iter().enumerate() {
            if scalars >> j & 1 == 1 {
                pieces.push((pr[0], 1u64, true));
                k += 1;
            } else {
                let Some(len) = self.kint(pr[1]).and_then(|l| u64::try_from(l).ok()) else {
                    return false;
                };
                let Some(units) = len.checked_mul(m as u64).filter(|&u| u <= 64) else {
                    return false;
                };
                if units == 0 {
                    continue;
                }
                pieces.push((pr[0], units, false));
                k += units;
            }
            if k > total {
                return false;
            }
        }
        let pmask = |units: u64| {
            if units == 64 {
                !0u64
            } else {
                (1u64 << units) - 1
            }
        };
        let (slow, join) = self.slow_path(pc, i);
        let mut words: [Value; 4] = std::array::from_fn(|_| self.b.ins().iconst(I64, 0));
        let mut mask = 0u64;
        let mut at_k = 0u64;
        for &(r, units, scalar) in &pieces {
            let bits: [Value; 4] = if scalar {
                let v = self.ld(r);
                let code = self.b.ins().iadd_imm_s(v, i64::from(bias));
                // A code beyond 'U' to '-' is the interpreter's error.
                let okc = self
                    .b
                    .ins()
                    .icmp_imm_s(IntCC::UnsignedLessThanOrEqual, code, 8);
                self.guard(okc, slow);
                std::array::from_fn(|p| {
                    let b = self.b.ins().ushr_imm_s(code, p as i64);
                    self.b.ins().band_imm_s(b, 1)
                })
            } else {
                let a = self.ld(r);
                let ptr = self.host(a, Some(r), Op::C(0), Op::C(32), false, slow);
                let nf = self.gf;
                std::array::from_fn(|p| {
                    let w = self.b.ins().load(I64, nf, ptr, 8 * p as i32);
                    self.b.ins().band_imm_s(w, pmask(units) as i64)
                })
            };
            let off = total - at_k - units;
            for p in 0..4 {
                let sh = self.b.ins().ishl_imm_s(bits[p], off as i64);
                words[p] = self.b.ins().bor(words[p], sh);
            }
            mask |= pmask(units) << off;
            at_k += units;
        }
        if mask == 0 {
            self.join(join);
            return true;
        }
        let d = self.ld(dst);
        let pd = self.host(d, Some(dst), Op::C(0), Op::C(32), true, slow);
        let nf = self.gf;
        for (p, &word) in words.iter().enumerate() {
            let cur = self.b.ins().load(I64, nf, pd, 8 * p as i32);
            let keep = self.b.ins().band_imm_s(cur, !mask as i64);
            let v = self.b.ins().bor(keep, word);
            self.b.ins().store(nf, v, pd, 8 * p as i32);
        }
        self.join(join);
        true
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
        // A variant has no register array to write the address into: a cell of its own.
        let scratch = self.nat.is_some().then(|| {
            let ss = self.b.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                8,
                3,
            ));
            self.b.ins().stack_addr(I64, ss, 0)
        });
        let (op, z) = (self.c32(FAST_DEREF), self.c64(0));
        let (regs, d) = match scratch {
            Some(sc) => (sc, self.c32(0)),
            None => (self.regs, self.c32(dst.0)),
        };
        let call = self
            .b
            .ins()
            .call(self.imp.fast, &[self.env, regs, self.mem, op, d, h, z]);
        let st = self.b.inst_results(call)[0];
        let failed = self.b.ins().icmp_imm_s(IntCC::Equal, st, 0);
        if let Some(sc) = scratch {
            let ok = self.b.create_block();
            self.b.ins().brif(failed, slow, &[], ok, &[]);
            self.b.switch_to_block(ok);
            let v = self.b.ins().load(I64, MemFlagsData::trusted(), sc, 0);
            // The callback may have found the address and failed to enter the object: the entry
            // must hold what the result points into.
            let at = self.b.ins().ushr_imm_s(v, 32);
            let found = self.b.ins().load(I64, self.mf, ent, MEM_OBJS as i32);
            let same = self.b.ins().icmp(IntCC::Equal, found, at);
            self.guard(same, slow);
            self.st(dst, v);
            self.b.ins().jump(join, &[]);
        } else {
            self.b.ins().brif(failed, slow, &[], join, &[]);
        }
        self.start(join);
        if scratch.is_some() {
            // The object stays in hand for the instructions that follow (`ObjRef`).
            let hi = self.ld(dst);
            let ptr = self
                .b
                .ins()
                .load(I64, self.mf, ent, (MEM_OBJS + OBJ_PTR) as i32);
            let len = self
                .b
                .ins()
                .load(I64, self.mf, ent, (MEM_OBJS + OBJ_LEN) as i32);
            let off = self.c64(0);
            self.objnew = Some((dst, ObjRef { ptr, len, hi, off }));
        }
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
    // Every function that runs on native frames has its variant, for the interpreter's calls and
    // for callers in other modules.
    for (id, f) in prog
        .funcs
        .iter()
        .filter_map(|(id, _)| prog.func(id).map(|f| (id, f)))
    {
        if f.kind != FuncKind::Elab && f.code.len() <= max_size {
            jit.make_native(prog, id);
        }
    }
    jit.finalize()?;
    let compile_time = t0.elapsed();
    let mut natives = vec![0usize; prog.funcs.len()];
    let mut calls = vec![0usize; prog.funcs.len()];
    for (id, n, c) in jit.variants() {
        natives[id.0 as usize] = n;
        calls[id.0 as usize] = c;
    }
    let mut funcs = vec![None; prog.funcs.len()];
    for (id, cid, nregs) in ids {
        funcs[id.0 as usize] = Some((jit.entry(cid), nregs));
    }
    let mut interpreted: Vec<(String, usize)> = kinds.into_iter().collect();
    interpreted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Ok(crate::ClifTier {
        _jit: std::sync::Mutex::new(jit),
        funcs,
        natives,
        calls,
        notes,
        compile_time,
        interpreted,
        inline,
        instrs,
        times,
    })
}
