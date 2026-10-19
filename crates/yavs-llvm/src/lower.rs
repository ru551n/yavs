//! `yavs-ir` to LLVM: the compiled tier (ir.md 5, `docs/decisions.md` "LLVM tier: side exits").
//!
//! One LLVM function per IR function, `fn(env, regs, pc) -> u32`, working on the registers of
//! the interpreter frame. It enters at `pc` (a switch over the block starts and the
//! instructions after an exit), runs register arithmetic, compares, checks and branches
//! itself, executes every other instruction that always continues with the next one through
//! the `yavs_rt_step` callback (the interpreter's own code), and returns the index of the
//! instruction the interpreter executes next: a call, return, wait, report, the slow or error
//! path of an inline instruction (re-executed by the interpreter, so messages are identical),
//! or a callback fault. Nothing beyond i64/f64 arithmetic is re-implemented (rule 5).

use std::any::Any;
use std::ffi::{CString, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Mutex;

use llvm_sys::analysis::{LLVMVerifierFailureAction, LLVMVerifyFunction};
use llvm_sys::core::*;
use llvm_sys::prelude::*;
use llvm_sys::{LLVMIntPredicate as IP, LLVMRealPredicate as RP};
use yavs_interp::Tier;
use yavs_ir::{
    BoolOp, CheckKind, CmpOp, FOp, FUnOp, Func, FuncId, FuncKind, Inst, IntOp, IntUnOp, Program,
    Reg,
};

use crate::{Error, Jit, Module, OptLevel};

const STEP: &str = "yavs_rt_step";

type RunFn = unsafe extern "C" fn(*mut c_void, *mut u64, u32) -> u32;

/// What a running compiled function reaches through `env`.
struct Env<'a> {
    regs: *mut Vec<u64>,
    step: &'a mut dyn FnMut(&mut Vec<u64>, u32) -> bool,
    panic: Option<Box<dyn Any + Send>>,
}

/// Executes one instruction in the interpreter: 1 to go on, 0 after a fault.
extern "C" fn yavs_rt_step(env: *mut c_void, _regs: *mut u64, pc: u32) -> u32 {
    // SAFETY: `env` is the `Env` that `LlvmTier::run` passed to the compiled function, which
    // is blocked in this call; `regs` aliases `*env.regs` but the compiled code does not touch
    // it until we return (it passes it only so LLVM reloads registers after the call).
    let env = unsafe { &mut *(env as *mut Env<'_>) };
    let regs = unsafe { &mut *env.regs };
    // No unwinding through JIT frames: a panic is carried over the call and resumed after.
    match catch_unwind(AssertUnwindSafe(|| (env.step)(regs, pc))) {
        Ok(ok) => ok.into(),
        Err(p) => {
            env.panic = Some(p);
            0
        }
    }
}

/// Compiled code for the functions of one program.
pub struct LlvmTier {
    /// Keeps the code alive; never used after construction.
    _jit: Mutex<Jit>,
    funcs: Vec<Option<(RunFn, usize)>>,
    /// Functions that stay interpreted, with the reason.
    pub notes: Vec<(FuncId, String)>,
    /// Instructions of the compiled functions the interpreter executes (stepped or at an
    /// exit), per instruction kind, most frequent first; and the number compiled inline.
    pub interpreted: Vec<(String, usize)>,
    pub inline: usize,
}

impl Tier for LlvmTier {
    fn run(
        &self,
        func: FuncId,
        pc: u32,
        regs: &mut Vec<u64>,
        step: &mut dyn FnMut(&mut Vec<u64>, u32) -> bool,
    ) -> Option<u32> {
        let &(f, nregs) = self.funcs.get(func.0 as usize)?.as_ref()?;
        if regs.len() != nregs {
            return None;
        }
        let rp = regs.as_mut_ptr();
        let mut env = Env {
            regs,
            step,
            panic: None,
        };
        // SAFETY: `f` was generated for this function with `nregs` registers and only
        // indexes `regs` below that (checked at compile time); `env` outlives the call.
        let next = unsafe { f(&mut env as *mut Env<'_> as *mut c_void, rp, pc) };
        if let Some(p) = env.panic {
            resume_unwind(p);
        }
        Some(next)
    }
}

/// Compile every function of `prog` that the tier can take; the others stay interpreted and
/// are listed in [`LlvmTier::notes`].
pub fn compile(prog: &Program, opt: OptLevel) -> Result<LlvmTier, Error> {
    let mut m = Module::new("yavs");
    let mut notes = Vec::new();
    let mut names = Vec::new();
    {
        let g = Gen::new(&m);
        for (id, f) in prog.funcs.iter() {
            // Elaboration code runs before the simulation, in the interpreter.
            if f.kind == FuncKind::Elab {
                continue;
            }
            if let Some(r) = bad_reg(f) {
                notes.push((id, format!("register r{} beyond the frame", r.0)));
                continue;
            }
            let name = format!("f{}", id.0);
            // SAFETY: generation of well-formed IR in our own module; a function that
            // fails the verifier is deleted again.
            let ok = unsafe { g.func(prog, f, &name) };
            match ok {
                Ok(()) => names.push((id, name, f.reg_ty.len())),
                Err(why) => notes.push((id, why)),
            }
        }
    }
    m.verify()?;
    m.optimize(opt)?;
    let mut jit = Jit::new(opt, &[(STEP, yavs_rt_step as *const () as usize)])?;
    jit.add(m)?;
    let mut funcs = vec![None; prog.funcs.len()];
    for (id, name, nregs) in names {
        let addr = jit.lookup(&name)?;
        // SAFETY: `name` was generated by `Gen::func` with the `RunFn` signature.
        let f: RunFn = unsafe { std::mem::transmute::<usize, RunFn>(addr) };
        funcs[id.0 as usize] = Some((f, nregs));
    }
    let mut kinds = std::collections::HashMap::new();
    let mut inline = 0;
    for (id, f) in prog.funcs.iter() {
        if funcs[id.0 as usize].is_none() {
            continue;
        }
        for i in &f.code {
            if is_inline(i) {
                inline += 1;
            } else {
                let d = format!("{i:?}");
                let k = d.split([' ', '{', '(']).next().unwrap_or("").to_string();
                *kinds.entry(k).or_insert(0) += 1;
            }
        }
    }
    let mut interpreted: Vec<(String, usize)> = kinds.into_iter().collect();
    interpreted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Ok(LlvmTier {
        _jit: Mutex::new(jit),
        funcs,
        notes,
        interpreted,
        inline,
    })
}

/// A register operand outside the function's register file, if any.
fn bad_reg(f: &Func) -> Option<Reg> {
    let n = f.reg_ty.len() as u32;
    f.code.iter().flat_map(inline_regs).find(|r| r.0 >= n)
}

/// The registers an inline instruction touches (the compiled code indexes `regs` with them).
fn inline_regs(i: &Inst) -> Vec<Reg> {
    match *i {
        Inst::Const { dst, .. } | Inst::ConstF { dst, .. } => vec![dst],
        Inst::Move { dst, src } | Inst::IToF { dst, src } => vec![dst, src],
        Inst::Bin { dst, a, b, .. }
        | Inst::FBin { dst, a, b, .. }
        | Inst::Cmp { dst, a, b, .. }
        | Inst::FCmp { dst, a, b, .. }
        | Inst::Bool { dst, a, b, .. } => vec![dst, a, b],
        Inst::Un { dst, a, .. } | Inst::FUn { dst, a, .. } => vec![dst, a],
        Inst::Check { v, lo, hi, .. } | Inst::FCheck { v, lo, hi, .. } => vec![v, lo, hi],
        Inst::BinRange {
            dst, a, b, lo, hi, ..
        } => vec![dst, a, b, lo, hi],
        Inst::Index { dst, base, pos, .. } => vec![dst, base, pos],
        Inst::CondBr { c, .. } => vec![c],
        Inst::Case { v, .. } => vec![v],
        _ => vec![],
    }
}

/// Instructions the compiled code executes itself on its fast path (keep in step with
/// `Gen::func`).
fn is_inline(i: &Inst) -> bool {
    match i {
        Inst::Bin { op, .. } | Inst::BinRange { op, .. } => *op != IntOp::Pow,
        Inst::FBin { op, .. } => !matches!(op, FOp::Pow | FOp::PowI),
        _ => !inline_regs(i).is_empty() || matches!(i, Inst::Br { .. }),
    }
}

/// Instructions the interpreter executes itself: they change the frame stack, suspend, or
/// may end the process.
fn is_exit(i: &Inst) -> bool {
    matches!(
        i,
        Inst::Ret { .. }
            | Inst::Call { .. }
            | Inst::CallIndirect { .. }
            | Inst::CallProt { .. }
            | Inst::CallNative { .. }
            | Inst::Report { .. }
            | Inst::Suspend { .. }
            | Inst::Unreachable { .. }
            | Inst::Elab { .. }
    )
}

struct Gen {
    ctx: LLVMContextRef,
    module: LLVMModuleRef,
    b: LLVMBuilderRef,
    i32: LLVMTypeRef,
    i64: LLVMTypeRef,
    f64: LLVMTypeRef,
    fn_ty: LLVMTypeRef,
    step: LLVMValueRef,
    sadd: (LLVMTypeRef, LLVMValueRef),
    ssub: (LLVMTypeRef, LLVMValueRef),
    smul: (LLVMTypeRef, LLVMValueRef),
    umul: (LLVMTypeRef, LLVMValueRef),
    uadd: (LLVMTypeRef, LLVMValueRef),
}

/// The function being generated.
struct Cur {
    f: LLVMValueRef,
    env: LLVMValueRef,
    regs: LLVMValueRef,
}

impl Gen {
    fn new(m: &Module) -> Gen {
        let (ctx, module) = (m.context(), m.raw());
        // SAFETY: ctx and module live as long as `m`; the builder is disposed in Drop.
        unsafe {
            let i64 = LLVMInt64TypeInContext(ctx);
            let i32 = LLVMInt32TypeInContext(ctx);
            let ptr = LLVMPointerTypeInContext(ctx, 0);
            let intrinsic = |name: &std::ffi::CStr| {
                let id = LLVMLookupIntrinsicID(name.as_ptr(), name.count_bytes());
                assert_ne!(id, 0, "unknown intrinsic {name:?}");
                let mut tys = [i64];
                (
                    LLVMIntrinsicGetType(ctx, id, tys.as_mut_ptr(), 1),
                    LLVMGetIntrinsicDeclaration(module, id, tys.as_mut_ptr(), 1),
                )
            };
            let mut params = [ptr, ptr, i32];
            let fn_ty = LLVMFunctionType(i32, params.as_mut_ptr(), 3, 0);
            let step_name = CString::new(STEP).expect("no NUL");
            Gen {
                ctx,
                module,
                b: LLVMCreateBuilderInContext(ctx),
                i32,
                i64,
                f64: LLVMDoubleTypeInContext(ctx),
                fn_ty,
                step: LLVMAddFunction(module, step_name.as_ptr(), fn_ty),
                sadd: intrinsic(c"llvm.sadd.with.overflow"),
                ssub: intrinsic(c"llvm.ssub.with.overflow"),
                smul: intrinsic(c"llvm.smul.with.overflow"),
                umul: intrinsic(c"llvm.umul.with.overflow"),
                uadd: intrinsic(c"llvm.uadd.with.overflow"),
            }
        }
    }

    fn c64(&self, v: i64) -> LLVMValueRef {
        // SAFETY: constant creation.
        unsafe { LLVMConstInt(self.i64, v as u64, 1) }
    }

    fn c32(&self, v: u32) -> LLVMValueRef {
        // SAFETY: constant creation.
        unsafe { LLVMConstInt(self.i32, u64::from(v), 0) }
    }

    unsafe fn block(&self, c: &Cur) -> LLVMBasicBlockRef {
        // SAFETY: c.f is a function of our context.
        unsafe { LLVMAppendBasicBlockInContext(self.ctx, c.f, c"".as_ptr()) }
    }

    unsafe fn slot(&self, c: &Cur, r: Reg) -> LLVMValueRef {
        // SAFETY: r is below the register count (`bad_reg`).
        unsafe {
            let mut idx = [self.c64(i64::from(r.0))];
            LLVMBuildGEP2(self.b, self.i64, c.regs, idx.as_mut_ptr(), 1, c"".as_ptr())
        }
    }

    unsafe fn ld(&self, c: &Cur, r: Reg) -> LLVMValueRef {
        // SAFETY: see `slot`.
        unsafe { LLVMBuildLoad2(self.b, self.i64, self.slot(c, r), c"".as_ptr()) }
    }

    unsafe fn ldf(&self, c: &Cur, r: Reg) -> LLVMValueRef {
        // SAFETY: see `slot`.
        unsafe { LLVMBuildBitCast(self.b, self.ld(c, r), self.f64, c"".as_ptr()) }
    }

    unsafe fn st(&self, c: &Cur, r: Reg, v: LLVMValueRef) {
        // SAFETY: see `slot`; `v` is an i64.
        unsafe {
            LLVMBuildStore(self.b, v, self.slot(c, r));
        }
    }

    unsafe fn stf(&self, c: &Cur, r: Reg, v: LLVMValueRef) {
        // SAFETY: see `slot`; `v` is a double.
        unsafe { self.st(c, r, LLVMBuildBitCast(self.b, v, self.i64, c"".as_ptr())) }
    }

    unsafe fn zext(&self, v: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: v is an i1.
        unsafe { LLVMBuildZExt(self.b, v, self.i64, c"".as_ptr()) }
    }

    unsafe fn icmp(&self, p: IP, a: LLVMValueRef, b: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: a and b have one integer type.
        unsafe { LLVMBuildICmp(self.b, p, a, b, c"".as_ptr()) }
    }

    unsafe fn or(&self, a: LLVMValueRef, b: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: two i1.
        unsafe { LLVMBuildOr(self.b, a, b, c"".as_ptr()) }
    }

    /// Leave to the interpreter at `pc` when `bad` holds; the builder continues after it.
    unsafe fn exit_if(&self, c: &Cur, bad: LLVMValueRef, pc: u32) {
        // SAFETY: well-formed blocks of c.f.
        unsafe {
            let out = self.block(c);
            let cont = self.block(c);
            LLVMBuildCondBr(self.b, bad, out, cont);
            LLVMPositionBuilderAtEnd(self.b, out);
            LLVMBuildRet(self.b, self.c32(pc));
            LLVMPositionBuilderAtEnd(self.b, cont);
        }
    }

    /// `intrinsic(a, b)` with its overflow flag: (value, overflowed).
    unsafe fn ovf(
        &self,
        (ty, f): (LLVMTypeRef, LLVMValueRef),
        a: LLVMValueRef,
        b: LLVMValueRef,
    ) -> (LLVMValueRef, LLVMValueRef) {
        // SAFETY: the intrinsic takes two i64.
        unsafe {
            let mut args = [a, b];
            let r = LLVMBuildCall2(self.b, ty, f, args.as_mut_ptr(), 2, c"".as_ptr());
            (
                LLVMBuildExtractValue(self.b, r, 0, c"".as_ptr()),
                LLVMBuildExtractValue(self.b, r, 1, c"".as_ptr()),
            )
        }
    }

    /// `x op y` as `yavs_std::arith`, leaving at `pc` wherever it reports an error.
    /// `None` for `**`, which goes through the interpreter.
    unsafe fn int_op(
        &self,
        c: &Cur,
        op: IntOp,
        x: LLVMValueRef,
        y: LLVMValueRef,
        pc: u32,
    ) -> Option<LLVMValueRef> {
        // SAFETY: well-formed IR in the current block.
        unsafe {
            let checked = |i| {
                let (v, o) = self.ovf(i, x, y);
                self.exit_if(c, o, pc);
                v
            };
            Some(match op {
                IntOp::Add => checked(self.sadd),
                IntOp::Sub => checked(self.ssub),
                IntOp::Mul => checked(self.smul),
                IntOp::Div | IntOp::Rem | IntOp::Mod => {
                    let zero = self.icmp(IP::LLVMIntEQ, y, self.c64(0));
                    let m1 = self.icmp(IP::LLVMIntEQ, y, self.c64(-1));
                    if op == IntOp::Div {
                        let min = self.icmp(IP::LLVMIntEQ, x, self.c64(i64::MIN));
                        let o = LLVMBuildAnd(self.b, min, m1, c"".as_ptr());
                        self.exit_if(c, self.or(zero, o), pc);
                        return Some(LLVMBuildSDiv(self.b, x, y, c"".as_ptr()));
                    }
                    self.exit_if(c, zero, pc);
                    // x rem -1 is 0; srem would trap on MIN rem -1.
                    let ys = LLVMBuildSelect(self.b, m1, self.c64(1), y, c"".as_ptr());
                    let r = LLVMBuildSRem(self.b, x, ys, c"".as_ptr());
                    if op == IntOp::Rem {
                        return Some(r);
                    }
                    let nz = self.icmp(IP::LLVMIntNE, r, self.c64(0));
                    let rn = self.icmp(IP::LLVMIntSLT, r, self.c64(0));
                    let yn = self.icmp(IP::LLVMIntSLT, y, self.c64(0));
                    let diff = LLVMBuildXor(self.b, rn, yn, c"".as_ptr());
                    let fix = LLVMBuildAnd(self.b, nz, diff, c"".as_ptr());
                    let sum = LLVMBuildAdd(self.b, r, y, c"".as_ptr());
                    LLVMBuildSelect(self.b, fix, sum, r, c"".as_ptr())
                }
                IntOp::Pow => return None,
            })
        }
    }

    /// Generate `name` for `f`; on a verifier failure the function is deleted again.
    unsafe fn func(&self, prog: &Program, f: &Func, name: &str) -> Result<(), String> {
        let cname = CString::new(name).expect("no NUL");
        // SAFETY: all values are built in this function with matching types.
        unsafe {
            let fv = LLVMAddFunction(self.module, cname.as_ptr(), self.fn_ty);
            let c = Cur {
                f: fv,
                env: LLVMGetParam(fv, 0),
                regs: LLVMGetParam(fv, 1),
            };
            let pc_in = LLVMGetParam(fv, 2);
            let n = f.code.len();
            // Entry points: 0, block starts, and the instruction after each exit.
            let mut leader = vec![false; n + 1];
            leader[0] = true;
            for (_, b) in f.blocks.iter() {
                if let Some(l) = leader.get_mut(b.start as usize) {
                    *l = true;
                }
            }
            for (i, inst) in f.code.iter().enumerate() {
                if is_exit(inst) {
                    leader[i + 1] = true;
                }
            }
            let entry = self.block(&c);
            let other = self.block(&c);
            let mut bbs = vec![std::ptr::null_mut(); n];
            for (i, bb) in bbs.iter_mut().enumerate() {
                if leader[i] {
                    *bb = self.block(&c);
                }
            }
            LLVMPositionBuilderAtEnd(self.b, other);
            LLVMBuildRet(self.b, pc_in);
            LLVMPositionBuilderAtEnd(self.b, entry);
            let nl = leader[..n].iter().filter(|&&l| l).count();
            let sw = LLVMBuildSwitch(self.b, pc_in, other, nl as u32);
            for (i, &bb) in bbs.iter().enumerate() {
                if !bb.is_null() {
                    LLVMAddCase(sw, self.c32(i as u32), bb);
                }
            }
            let block_bb = |b: yavs_ir::BlockId| -> Result<LLVMBasicBlockRef, String> {
                f.blocks
                    .get(b)
                    .and_then(|b| bbs.get(b.start as usize).copied())
                    .filter(|bb| !bb.is_null())
                    .ok_or_else(|| format!("branch to missing block {}", b.0))
            };
            let mut open = false;
            for (i, &inst) in f.code.iter().enumerate() {
                let pc = i as u32;
                if leader[i] {
                    if open {
                        LLVMBuildBr(self.b, bbs[i]);
                    }
                    LLVMPositionBuilderAtEnd(self.b, bbs[i]);
                    open = true;
                } else if !open {
                    // Dead code after a terminator: give it a block, LLVM drops it.
                    let bb = self.block(&c);
                    LLVMPositionBuilderAtEnd(self.b, bb);
                    open = true;
                }
                if is_exit(&inst) {
                    LLVMBuildRet(self.b, self.c32(pc));
                    open = false;
                    continue;
                }
                match inst {
                    Inst::Const { dst, v } => self.st(&c, dst, self.c64(v)),
                    Inst::ConstF { dst, v } => self.st(&c, dst, self.c64(v.to_bits() as i64)),
                    Inst::Move { dst, src } => self.st(&c, dst, self.ld(&c, src)),
                    Inst::Bin { op, dst, a, b } => {
                        let (x, y) = (self.ld(&c, a), self.ld(&c, b));
                        match self.int_op(&c, op, x, y, pc) {
                            Some(v) => self.st(&c, dst, v),
                            None => self.step(&c, pc),
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
                        let (x, y) = (self.ld(&c, a), self.ld(&c, b));
                        match self.int_op(&c, op, x, y, pc) {
                            Some(v) => {
                                self.range_exit(&c, v, self.ld(&c, lo), self.ld(&c, hi), pc);
                                self.st(&c, dst, v);
                            }
                            None => self.step(&c, pc),
                        }
                    }
                    Inst::Un { op, dst, a } => {
                        let x = self.ld(&c, a);
                        let min = self.icmp(IP::LLVMIntEQ, x, self.c64(i64::MIN));
                        self.exit_if(&c, min, pc);
                        let neg = LLVMBuildNeg(self.b, x, c"".as_ptr());
                        let v = match op {
                            IntUnOp::Neg => neg,
                            IntUnOp::Abs => {
                                let lt = self.icmp(IP::LLVMIntSLT, x, self.c64(0));
                                LLVMBuildSelect(self.b, lt, neg, x, c"".as_ptr())
                            }
                        };
                        self.st(&c, dst, v);
                    }
                    Inst::FBin { op, dst, a, b } => {
                        let (x, y) = (self.ldf(&c, a), self.ldf(&c, b));
                        let v = match op {
                            FOp::Add => LLVMBuildFAdd(self.b, x, y, c"".as_ptr()),
                            FOp::Sub => LLVMBuildFSub(self.b, x, y, c"".as_ptr()),
                            FOp::Mul => LLVMBuildFMul(self.b, x, y, c"".as_ptr()),
                            FOp::Div => {
                                let z = LLVMBuildFCmp(
                                    self.b,
                                    RP::LLVMRealOEQ,
                                    y,
                                    LLVMConstReal(self.f64, 0.0),
                                    c"".as_ptr(),
                                );
                                self.exit_if(&c, z, pc);
                                LLVMBuildFDiv(self.b, x, y, c"".as_ptr())
                            }
                            FOp::PowI | FOp::Pow => {
                                self.step(&c, pc);
                                continue;
                            }
                        };
                        // Not finite: |v| is not below infinity (NaN compares false).
                        let fin = LLVMBuildFCmp(
                            self.b,
                            RP::LLVMRealUGE,
                            self.fabs(v),
                            LLVMConstReal(self.f64, f64::INFINITY),
                            c"".as_ptr(),
                        );
                        self.exit_if(&c, fin, pc);
                        self.stf(&c, dst, v);
                    }
                    Inst::FUn { op, dst, a } => {
                        let x = self.ldf(&c, a);
                        let v = match op {
                            FUnOp::Neg => LLVMBuildFNeg(self.b, x, c"".as_ptr()),
                            FUnOp::Abs => self.fabs(x),
                        };
                        self.stf(&c, dst, v);
                    }
                    Inst::IToF { dst, src } => {
                        let v = LLVMBuildSIToFP(self.b, self.ld(&c, src), self.f64, c"".as_ptr());
                        self.stf(&c, dst, v);
                    }
                    Inst::Cmp { op, dst, a, b } => {
                        let p = match op {
                            CmpOp::Eq => IP::LLVMIntEQ,
                            CmpOp::Ne => IP::LLVMIntNE,
                            CmpOp::Lt => IP::LLVMIntSLT,
                            CmpOp::Le => IP::LLVMIntSLE,
                            CmpOp::Gt => IP::LLVMIntSGT,
                            CmpOp::Ge => IP::LLVMIntSGE,
                        };
                        let v = self.icmp(p, self.ld(&c, a), self.ld(&c, b));
                        self.st(&c, dst, self.zext(v));
                    }
                    Inst::FCmp { op, dst, a, b } => {
                        // Ordered: every comparison with a NaN is false, as `partial_cmp`.
                        let p = match op {
                            CmpOp::Eq => RP::LLVMRealOEQ,
                            CmpOp::Ne => RP::LLVMRealONE,
                            CmpOp::Lt => RP::LLVMRealOLT,
                            CmpOp::Le => RP::LLVMRealOLE,
                            CmpOp::Gt => RP::LLVMRealOGT,
                            CmpOp::Ge => RP::LLVMRealOGE,
                        };
                        let (x, y) = (self.ldf(&c, a), self.ldf(&c, b));
                        let v = LLVMBuildFCmp(self.b, p, x, y, c"".as_ptr());
                        self.st(&c, dst, self.zext(v));
                    }
                    Inst::Bool { op, dst, a, b } => {
                        let x = self.icmp(IP::LLVMIntNE, self.ld(&c, a), self.c64(0));
                        let v = if op == BoolOp::Not {
                            LLVMBuildNot(self.b, x, c"".as_ptr())
                        } else {
                            let y = self.icmp(IP::LLVMIntNE, self.ld(&c, b), self.c64(0));
                            match op {
                                BoolOp::And => LLVMBuildAnd(self.b, x, y, c"".as_ptr()),
                                BoolOp::Or => self.or(x, y),
                                _ => LLVMBuildXor(self.b, x, y, c"".as_ptr()),
                            }
                        };
                        self.st(&c, dst, self.zext(v));
                    }
                    Inst::Check {
                        kind, v, lo, hi, ..
                    } => {
                        let x = self.ld(&c, v);
                        match kind {
                            CheckKind::Range | CheckKind::Index => {
                                self.range_exit(&c, x, self.ld(&c, lo), self.ld(&c, hi), pc)
                            }
                            CheckKind::Length => {
                                let ne = self.icmp(IP::LLVMIntNE, x, self.ld(&c, lo));
                                self.exit_if(&c, ne, pc);
                            }
                            CheckKind::Null => {
                                let z = self.icmp(IP::LLVMIntEQ, x, self.c64(0));
                                self.exit_if(&c, z, pc);
                            }
                        }
                    }
                    Inst::FCheck { v, lo, hi, .. } => {
                        let (x, l, h) = (self.ldf(&c, v), self.ldf(&c, lo), self.ldf(&c, hi));
                        // !(l <= x && x <= h), NaN included.
                        let a = LLVMBuildFCmp(self.b, RP::LLVMRealUGT, l, x, c"".as_ptr());
                        let b = LLVMBuildFCmp(self.b, RP::LLVMRealUGT, x, h, c"".as_ptr());
                        self.exit_if(&c, self.or(a, b), pc);
                    }
                    Inst::Index {
                        dst,
                        base,
                        pos,
                        stride,
                    } => {
                        // Address offset in the low 32 bits; anything over 4 GiB is the
                        // interpreter's error.
                        let a = self.ld(&c, base);
                        let (off, o1) =
                            self.ovf(self.umul, self.ld(&c, pos), self.c64(i64::from(stride)));
                        let lo = LLVMBuildAnd(self.b, a, self.c64(0xffff_ffff), c"".as_ptr());
                        let (sum, o2) = self.ovf(self.uadd, lo, off);
                        let big = self.icmp(IP::LLVMIntUGT, sum, self.c64(0xffff_ffff));
                        self.exit_if(&c, self.or(self.or(o1, o2), big), pc);
                        let hi = LLVMBuildAnd(self.b, a, self.c64(!0xffff_ffff), c"".as_ptr());
                        self.st(&c, dst, LLVMBuildOr(self.b, hi, sum, c"".as_ptr()));
                    }
                    Inst::Br { to } => {
                        LLVMBuildBr(self.b, block_bb(to)?);
                        open = false;
                    }
                    Inst::CondBr { c: r, t, f: e } => {
                        let x = self.icmp(IP::LLVMIntNE, self.ld(&c, r), self.c64(0));
                        LLVMBuildCondBr(self.b, x, block_bb(t)?, block_bb(e)?);
                        open = false;
                    }
                    Inst::Case { v, table } => {
                        let t = prog
                            .cases
                            .get(table)
                            .ok_or_else(|| format!("no case table {}", table.0))?;
                        let x = self.ld(&c, v);
                        // ponytail: a compare chain per arm; a switch for dense tables if
                        // case-heavy code ever shows in profiles.
                        for &(lo, hi, to) in &t.arms {
                            let ge = self.icmp(IP::LLVMIntSGE, x, self.c64(lo));
                            let le = self.icmp(IP::LLVMIntSLE, x, self.c64(hi));
                            let hit = LLVMBuildAnd(self.b, ge, le, c"".as_ptr());
                            let next = self.block(&c);
                            LLVMBuildCondBr(self.b, hit, block_bb(to)?, next);
                            LLVMPositionBuilderAtEnd(self.b, next);
                        }
                        LLVMBuildBr(self.b, block_bb(t.default)?);
                        open = false;
                    }
                    _ => self.step(&c, pc),
                }
            }
            if open {
                // Falling off the end: the interpreter reports it.
                LLVMBuildRet(self.b, self.c32(n as u32));
            }
            if LLVMVerifyFunction(fv, LLVMVerifierFailureAction::LLVMReturnStatusAction) != 0 {
                LLVMDeleteFunction(fv);
                return Err("the generated LLVM function does not verify (a bug in yavs)".into());
            }
            Ok(())
        }
    }

    /// Execute the instruction at `pc` in the interpreter; leave at `pc` on a fault.
    unsafe fn step(&self, c: &Cur, pc: u32) {
        // SAFETY: `step` has the run-function signature.
        unsafe {
            let mut args = [c.env, c.regs, self.c32(pc)];
            let ok = LLVMBuildCall2(
                self.b,
                self.fn_ty,
                self.step,
                args.as_mut_ptr(),
                3,
                c"".as_ptr(),
            );
            let bad = self.icmp(IP::LLVMIntEQ, ok, self.c32(0));
            self.exit_if(c, bad, pc);
        }
    }

    unsafe fn range_exit(
        &self,
        c: &Cur,
        x: LLVMValueRef,
        lo: LLVMValueRef,
        hi: LLVMValueRef,
        pc: u32,
    ) {
        // SAFETY: i64 operands.
        unsafe {
            let a = self.icmp(IP::LLVMIntSLT, x, lo);
            let b = self.icmp(IP::LLVMIntSGT, x, hi);
            self.exit_if(c, self.or(a, b), pc);
        }
    }

    unsafe fn fabs(&self, x: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: the intrinsic takes one double.
        unsafe {
            let id = LLVMLookupIntrinsicID(c"llvm.fabs".as_ptr(), 9);
            let mut tys = [self.f64];
            let ty = LLVMIntrinsicGetType(self.ctx, id, tys.as_mut_ptr(), 1);
            let f = LLVMGetIntrinsicDeclaration(self.module, id, tys.as_mut_ptr(), 1);
            let mut args = [x];
            LLVMBuildCall2(self.b, ty, f, args.as_mut_ptr(), 1, c"".as_ptr())
        }
    }
}

impl Drop for Gen {
    fn drop(&mut self) {
        // SAFETY: we own the builder.
        unsafe { LLVMDisposeBuilder(self.b) }
    }
}
