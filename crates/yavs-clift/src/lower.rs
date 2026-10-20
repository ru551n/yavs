//! `yavs-ir` to Cranelift: the same compiled-code contract as the LLVM tier (`yavs-llvm`
//! `rt.rs`): `fn(env, regs, mem, pc) -> u32` working on the registers of the interpreter frame,
//! entering at `pc`, executing a subset of the instructions itself and every other instruction
//! in place through the `yavs_rt_step` callback. Compiled code never reports an error itself: a
//! failing inline instruction returns its pc and the interpreter re-executes it.

use std::collections::HashMap;

use cranelift_codegen::Context;
use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{
    AbiParam, AliasRegionData, Block, BlockArg, FuncRef, GlobalValue, InstBuilder, MemFlagsData,
    Signature, Type, UserFuncName, Value, types,
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
    ctx: Context,
    fctx: FunctionBuilderContext,
    step: ClifId,
    host: ClifId,
    signal: ClifId,
    lcopy: ClifId,
    zero: ClifId,
    mov: ClifId,
    tables: HashMap<Vec<u8>, DataId>,
    sig: Signature,
}

// SAFETY: the module owns its code memory and is used by one thread at a time (the compile
// thread, or the thread that built it); compiled code is called through plain pointers.
unsafe impl Send for Jit {}

/// Measurement knobs of the spike: `YAVS_CLIFT_OPT` (`none`, `speed`), `YAVS_CLIFT_VERIFY`
/// (`1` runs the Cranelift verifier), `YAVS_CLIFT_RA` (`single_pass`, `backtracking`).
fn flag(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

impl Jit {
    pub fn new() -> Result<Jit, Error> {
        let opt = flag("YAVS_CLIFT_OPT", "none");
        let verify = flag("YAVS_CLIFT_VERIFY", "1");
        let ra = flag("YAVS_CLIFT_RA", "backtracking");
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
            ("clift_zero", clift_zero as *const u8),
            ("clift_move", clift_move as *const u8),
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
        let mut sig = module.make_signature();
        sig.params.extend([AbiParam::new(I64); 3]);
        sig.params.push(AbiParam::new(I32));
        sig.returns.push(AbiParam::new(I32));
        Ok(Jit {
            ctx: module.make_context(),
            module,
            fctx: FunctionBuilderContext::new(),
            step,
            host,
            signal,
            lcopy,
            zero,
            mov,
            tables: HashMap::new(),
            sig,
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
        let fid = self
            .module
            .declare_function(&format!("f{}", id.0), Linkage::Export, &self.sig)
            .map_err(|e| e.to_string())?;
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
                let gv = self.module.declare_data_in_func(did, &mut self.ctx.func);
                tabs.insert(t, gv);
            }
        }
        self.ctx.func.signature = self.sig.clone();
        self.ctx.func.name = UserFuncName::user(0, fid.as_u32());
        let imports = Imports {
            step: self
                .module
                .declare_func_in_func(self.step, &mut self.ctx.func),
            host: self
                .module
                .declare_func_in_func(self.host, &mut self.ctx.func),
            signal: self
                .module
                .declare_func_in_func(self.signal, &mut self.ctx.func),
            lcopy: self
                .module
                .declare_func_in_func(self.lcopy, &mut self.ctx.func),
            zero: self
                .module
                .declare_func_in_func(self.zero, &mut self.ctx.func),
            mov: self
                .module
                .declare_func_in_func(self.mov, &mut self.ctx.func),
            tables: tabs,
        };
        let cfg = self.module.target_config();
        let built = gen_func(&mut self.ctx, &mut self.fctx, cfg, prog, f, imports);
        let r = built.and_then(|stepped| {
            self.module
                .define_function(fid, &mut self.ctx)
                .map_err(|e| format!("Cranelift: {e:?} (a bug in yavs)"))?;
            Ok(stepped)
        });
        self.module.clear_context(&mut self.ctx);
        r.map(|stepped| (fid, stepped))
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
    tables: HashMap<Vec<u8>, GlobalValue>,
}

/// A slow path left to emit after the function body: step `inst` at `pc` and go on at `join`.
struct Slow {
    block: Block,
    join: Block,
    pc: u32,
    keep: bool,
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
) -> Result<Vec<String>, String> {
    let mut b = FunctionBuilder::new(&mut ctx.func, fctx);
    let entry = b.create_block();
    b.append_block_params_for_function_params(entry);
    let exit = b.create_block();
    let exit_pc = b.append_block_param(exit, I32);
    b.switch_to_block(entry);
    let p = b.block_params(entry).to_vec();
    let (env, regs, mem, pc_in) = (p[0], p[1], p[2], p[3]);
    let n = f.code.len();
    let mut leader = vec![false; n + 1];
    leader[0] = true;
    for (_, bl) in f.blocks.iter() {
        if let Some(l) = leader.get_mut(bl.start as usize) {
            *l = true;
        }
    }
    for (i, inst) in f.code.iter().enumerate() {
        if resumes(inst) {
            leader[i + 1] = true;
        }
    }
    let bbs: Vec<Option<Block>> = (0..n)
        .map(|i| leader[i].then(|| b.create_block()))
        .collect();
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
    let region = |b: &mut FunctionBuilder<'_>, id: u32, name: &'static str| {
        b.func.dfg.alias_regions.insert(AliasRegionData {
            user_id: id,
            description: name.into(),
        })
    };
    let (rr, gr, mr) = (
        region(&mut b, 1, "regs"),
        region(&mut b, 2, "guest"),
        region(&mut b, 3, "mem"),
    );
    let mut g = Gen {
        known: vec![None; f.reg_ty.len()],
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
    };
    let block_bb = |bl: yavs_ir::BlockId| -> Result<Block, String> {
        f.blocks
            .get(bl)
            .and_then(|bl| bbs.get(bl.start as usize).copied().flatten())
            .ok_or_else(|| format!("branch to missing block {}", bl.0))
    };
    let mut open = false;
    let mut prev = None;
    for (i, &inst) in f.code.iter().enumerate() {
        let pc = i as u32;
        if let Some(p) = prev.replace(inst) {
            track(&mut g.known, &p);
        }
        if leader[i] {
            g.known.fill(None);
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
        if let Inst::Ret { .. } = inst {
            let v = g.c32(pc);
            g.leave(v);
            open = false;
            continue;
        }
        match inst {
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
                    let hi = g.mem_ld(MEM_FRAME);
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
                let roots = g.mem_ld(MEM_TEMP_ROOTS);
                let z = g.b.ins().icmp_imm_s(IntCC::Equal, roots, 0);
                g.guard(z, slow);
                let len = g.mem_ld(MEM_TEMP_LEN);
                let base = g.mem_ld(MEM_TEMP_BASE);
                let v = g.b.ins().umin(base, len);
                g.mem_st(MEM_TEMP_LEN, v);
                g.join(join);
            }
            Inst::Alloc { dst, size } => {
                let (slow, join) = g.slow_path(pc, &inst);
                let sz = g.ld(size);
                let (a, _) = g.temp_alloc(sz, slow);
                g.st(dst, a);
                g.join(join);
            }
            Inst::LayoutOf { dst, ty, desc } => match static_layout(prog, ty) {
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
            Inst::SigReadScalar { .. }
            | Inst::SigRead { .. }
            | Inst::WaitOn { .. }
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
                    Some(NativeKind::Edge(rising)) => g.signal(pc, if rising { 1 } else { 2 }),
                    None => g.step(pc, &inst),
                }
            }
            _ => g.step(pc, &inst),
        }
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
        let at = if s.keep { s.pc | KEEP } else { s.pc };
        let pcv = g.c32(at);
        g.call_go(g.imp.step, &[pcv], false);
        g.b.ins().jump(s.join, &[]);
    }
    // The exit block.
    g.b.switch_to_block(g.exit);
    g.b.ins().return_(&[exit_pc]);
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
        let d = format!("{i:?}");
        self.stepped
            .push(d.split([' ', '{', '(']).next().unwrap_or("").to_string());
        let at = if keeps_objects(i) { pc | KEEP } else { pc };
        let pcv = self.c32(at);
        self.call_go(self.imp.step, &[pcv], true);
    }

    /// The signal instruction at `pc` through `yavs_rt_signal`.
    fn signal(&mut self, pc: u32, edge: u32) {
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
            let len = self.mem_ld(MEM_SLOTS_LEN);
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
