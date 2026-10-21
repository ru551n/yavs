//! `yavs-ir` to LLVM: the compiled tier (ir.md 5, `docs/decisions.md` "LLVM tier").
//!
//! One LLVM function per IR function, `fn(env, regs, mem, pc) -> u32`, working on the
//! registers of the interpreter frame and, through [`Mem`], on the frame's slots, the
//! temporary area and the instance data (the interpreter's [`TierFrame`] view). It enters at
//! `pc` (a switch over the block starts and the instructions after a call or a suspension),
//! runs register arithmetic, compares, checks, branches, loads, stores, copies, temporary
//! allocation, descriptor and signal-reference arithmetic itself, and executes every other
//! instruction in place through the `yavs_rt_step` callback (the interpreter's own code):
//! calls, natives, reports, signal operations and the slow paths. A call whose callee has
//! compiled code runs that directly on the frame the interpreter pushed, and goes on when it
//! returns (#336). It returns at a return or a suspension, or when an inline instruction
//! fails (re-executed by the interpreter, so messages are identical): compiled code never
//! reports an error itself (rule 5).

use std::ffi::CString;
use std::sync::Mutex;

use llvm_sys::analysis::{LLVMVerifierFailureAction, LLVMVerifyFunction};
use llvm_sys::core::*;
use llvm_sys::prelude::*;
use llvm_sys::{LLVMIntPredicate as IP, LLVMRealPredicate as RP};
use yavs_interp::{Tier, TierHost, space};
use yavs_ir::{
    AddrSpace, BoolOp, CheckKind, CmpOp, FOp, FUnOp, Func, FuncId, FuncKind, Inst, IntOp, IntUnOp,
    Program, Reg, Width,
};

use crate::rt::*;
use crate::{Error, Jit, Module, OptLevel};

/// Compiled code for the functions of one program.
pub struct LlvmTier {
    /// Keeps the code alive; never used after construction.
    _jit: Mutex<Jit>,
    funcs: Vec<Option<(RunFn, usize)>>,
    /// Per function, its native-frame code's address or 0 (`Mem::ntab`).
    natives: Vec<usize>,
    /// Functions that stay interpreted, with the reason.
    pub notes: Vec<(FuncId, String)>,
    /// Instructions of the compiled functions the interpreter executes (stepped or at an
    /// exit), per instruction kind, most frequent first; and the number compiled inline.
    pub interpreted: Vec<(String, usize)>,
    pub inline: usize,
}

impl LlvmTier {
    /// The functions compiled code calls on native frames (`docs/decisions.md` "Compiled
    /// frames").
    pub fn native_frames(&self) -> usize {
        self.natives.iter().filter(|&&a| a != 0).count()
    }
}

impl Tier for LlvmTier {
    fn run(&self, func: FuncId, pc: u32, host: &mut dyn TierHost) -> Option<u32> {
        let &(f, nregs) = self.funcs.get(func.0 as usize)?.as_ref()?;
        let lookup = |id: FuncId| self.funcs.get(id.0 as usize).copied().flatten();
        enter(f, nregs, pc, host, &lookup, self.natives.as_ptr(), None)
    }

    fn run_cached(
        &self,
        func: FuncId,
        pc: u32,
        host: &mut dyn TierHost,
        state: &mut Option<Box<yavs_interp::TierState>>,
    ) -> Option<u32> {
        let &(f, nregs) = self.funcs.get(func.0 as usize)?.as_ref()?;
        let lookup = |id: FuncId| self.funcs.get(id.0 as usize).copied().flatten();
        enter(
            f,
            nregs,
            pc,
            host,
            &lookup,
            self.natives.as_ptr(),
            Some(state),
        )
    }

    fn ready(&self, func: FuncId) -> bool {
        self.funcs.get(func.0 as usize).is_some_and(Option::is_some)
    }
}

/// A JIT with the runtime callbacks compiled code calls.
pub(crate) fn new_jit(opt: OptLevel) -> Result<Jit, Error> {
    Jit::new(
        opt,
        &[
            (STEP, yavs_rt_step as *const () as usize),
            (HOST, yavs_rt_host as *const () as usize),
            (SIGNAL, yavs_rt_signal as *const () as usize),
            (DRIVE, yavs_rt_drive as *const () as usize),
            (WAIT, yavs_rt_wait as *const () as usize),
            (LCOPY, yavs_rt_lcopy as *const () as usize),
            (LAYOUT, yavs_rt_layout as *const () as usize),
            (FAST, yavs_rt_fast as *const () as usize),
            (DEOPT, yavs_rt_deopt as *const () as usize),
            (DEOPT_SLOTS, yavs_rt_deopt_slots as *const () as usize),
            (DEOPT_END, yavs_rt_deopt_end as *const () as usize),
            (MEMEQ, yavs_rt_memeq as *const () as usize),
        ],
    )
}

/// Why `f` stays interpreted before any code is generated: over `max_size` IR instructions
/// (codegen time grows faster than linearly; #126).
pub(crate) fn too_big(f: &Func, max_size: usize) -> Option<String> {
    (f.code.len() > max_size).then(|| {
        format!(
            "{} IR instructions, over the size cap of {max_size} (--tier-max-size)",
            f.code.len()
        )
    })
}

/// Compile `f` alone into `jit`: its entry and register count, or why it stays interpreted.
pub(crate) fn compile_one(
    jit: &mut Jit,
    prog: &Program,
    id: FuncId,
    f: &Func,
    opt: OptLevel,
) -> Result<(RunFn, usize, usize), String> {
    if let Some(r) = bad_reg(f) {
        return Err(format!("register r{} beyond the frame", r.0));
    }
    // Native callees are reached through `Mem::ntab`, so only the reachable set's shape
    // matters here, not what is compiled yet.
    let nc = native_set(prog, [id]);
    let name = format!("f{}", id.0);
    let nname = format!("f{}_n", id.0);
    let llvm = |e: Error| format!("LLVM: {e} (a bug in yavs)");
    let mut native = nc.contains(&id);
    let mut m = loop {
        let m = Module::new(&name);
        let g = Gen::new(&m);
        // SAFETY: generation of well-formed IR in our own module; a frame-mode function that
        // fails the verifier is deleted again, a native one makes us start again without it.
        if native && unsafe { g.func(prog, f, id, &nname, true, &nc) }.is_err() {
            native = false;
            continue;
        }
        unsafe { g.func(prog, f, id, &name, false, &nc) }?;
        drop(g);
        break m;
    };
    m.verify().map_err(llvm)?;
    m.optimize(opt).map_err(llvm)?;
    jit.add(m).map_err(llvm)?;
    let addr = jit.lookup(&name).map_err(llvm)?;
    let naddr = if native {
        jit.lookup(&nname).map_err(llvm)?
    } else {
        0
    };
    // SAFETY: `name` was generated by `Gen::func` with the `RunFn` signature.
    let run: RunFn = unsafe { std::mem::transmute::<usize, RunFn>(addr) };
    Ok((run, f.reg_ty.len(), naddr))
}

/// Compile every function of `prog` that the tier can take; the others (among them those over
/// `max_size` IR instructions) stay interpreted and are listed in [`LlvmTier::notes`].
pub fn compile(prog: &Program, opt: OptLevel, max_size: usize) -> Result<LlvmTier, Error> {
    crate::lazy::ensure()?;
    let mut notes = Vec::new();
    let mut todo = Vec::new();
    for (id, f) in prog
        .funcs
        .iter()
        .filter_map(|(id, _)| prog.func(id).map(|f| (id, f)))
    {
        // Elaboration code runs before the simulation, in the interpreter.
        if f.kind == FuncKind::Elab {
            continue;
        }
        if let Some(why) = too_big(f, max_size) {
            notes.push((id, why));
            continue;
        }
        if let Some(r) = bad_reg(f) {
            notes.push((id, format!("register r{} beyond the frame", r.0)));
            continue;
        }
        if let Some(dir) = std::env::var_os("YAVS_LLVM_DUMP") {
            // Debug aid next to the module dumps of `Module::optimize`.
            let _ = std::fs::write(
                std::path::Path::new(&dir).join(format!("f{}.ir", id.0)),
                prog.dump_func(id),
            );
        }
        todo.push((id, f));
    }
    let ids: std::collections::HashSet<FuncId> = todo.iter().map(|t| t.0).collect();
    let mut nc = native_set(prog, ids.iter().copied());
    nc.retain(|id| ids.contains(id));
    let gen_all = |nc: &std::collections::HashSet<FuncId>| {
        let m = Module::new("yavs");
        let mut g = Gen::new(&m);
        g.direct = true;
        let mut names = Vec::new();
        let mut notes = Vec::new();
        for &(id, f) in &todo {
            if nc.contains(&id) {
                // SAFETY: generation of well-formed IR in our own module.
                unsafe { g.func(prog, f, id, &format!("f{}_n", id.0), true, nc) }.ok()?;
            }
        }
        for &(id, f) in &todo {
            let name = format!("f{}", id.0);
            // SAFETY: generation of well-formed IR in our own module; a function that
            // fails the verifier is deleted again.
            match unsafe { g.func(prog, f, id, &name, false, nc) } {
                Ok(()) => names.push((id, name, f.reg_ty.len())),
                Err(why) => notes.push((id, why)),
            }
        }
        drop(g);
        Some((m, names, notes))
    };
    // A native function that does not verify: everything again without native frames.
    let (mut m, names, more) = match gen_all(&nc) {
        Some(r) => r,
        None => {
            nc.clear();
            gen_all(&nc).ok_or_else(|| Error("no module without native frames".into()))?
        }
    };
    notes.extend(more);
    m.verify()?;
    m.optimize(opt)?;
    let mut jit = new_jit(opt)?;
    jit.add(m)?;
    let mut funcs = vec![None; prog.funcs.len()];
    for (id, name, nregs) in names {
        let addr = jit.lookup(&name)?;
        // SAFETY: `name` was generated by `Gen::func` with the `RunFn` signature.
        let f: RunFn = unsafe { std::mem::transmute::<usize, RunFn>(addr) };
        funcs[id.0 as usize] = Some((f, nregs));
    }
    let mut natives = vec![0; prog.funcs.len()];
    for &id in &nc {
        natives[id.0 as usize] = jit.lookup(&format!("f{}_n", id.0))?;
    }
    let mut kinds = std::collections::HashMap::new();
    let mut inline = 0;
    for (id, f) in prog
        .funcs
        .iter()
        .filter_map(|(id, _)| prog.func(id).map(|f| (id, f)))
    {
        if funcs[id.0 as usize].is_none() {
            continue;
        }
        for i in &f.code {
            if is_inline(prog, f, i) {
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
        natives,
        notes,
        interpreted,
        inline,
    })
}

/// Instructions the compiled code executes itself on its fast path (keep in step with
/// `Gen::func`).
pub(crate) fn is_inline(prog: &Program, f: &Func, i: &Inst) -> bool {
    match i {
        Inst::CallNative { native, args, dst } => matches!(
            native_kind(prog, f, *native, *args, *dst),
            Some(NativeKind::Table(..))
        ),
        Inst::LayoutOf { ty, .. } => static_layout(prog, *ty).is_some(),
        Inst::Bin { op, .. } | Inst::BinRange { op, .. } => *op != IntOp::Pow,
        Inst::FBin { op, .. } => !matches!(op, FOp::Pow | FOp::PowI),
        Inst::AddrOf { space, .. } => {
            addr_const(*space, 0).is_some() || matches!(space, AddrSpace::Up(1..=2))
        }
        _ => !inline_regs(f, i).is_empty() || matches!(i, Inst::Br { .. } | Inst::Stmt),
    }
}

/// `ModRef` of an LLVM `memory(...)` attribute: reads, writes, both.
const REF: u64 = 1;
const MODS: u64 = 2;
const RW: u64 = 3;

/// The value of a `memory(...)` attribute (LLVM 21 `MemoryEffects`: two bits per location,
/// argument memory, inaccessible memory, errno, other memory).
fn mem_effects(arg: u64, inaccessible: u64, other: u64) -> u64 {
    arg | inaccessible << 2 | other << 6
}

/// The most registers, slot bytes and IR instructions of a native-callable function: its
/// registers and slots are `alloca`s, and a materialisation stores every one of them.
const NC_REGS: usize = 128;
const NC_SLOTS: u32 = 4096;
const NC_CODE: usize = 2000;

/// Whether `f` itself may run on a native frame (`docs/decisions.md` "Compiled frames"): a
/// function or procedure without a static link, small, whose instructions are all lowered
/// without a step on their fast path; its calls are checked by `native_set`.
fn native_local(prog: &Program, f: &Func) -> bool {
    matches!(f.kind, FuncKind::Function | FuncKind::Procedure)
        && f.parent.is_none()
        && f.reg_ty.len() <= NC_REGS
        && f.frame <= NC_SLOTS
        && f.code.len() <= NC_CODE
        && bad_reg(f).is_none()
        && f.code.iter().all(|i| match i {
            Inst::Ret { .. } | Inst::Call { .. } | Inst::TempRoot { ty: None, .. } => true,
            Inst::AddrOf {
                space: AddrSpace::Up(_),
                ..
            } => false,
            _ => is_inline(prog, f, i),
        })
}

/// Whether the call of `callee` with `args` from `f` passes one argument per parameter, in
/// registers of `f`.
fn call_fits(f: &Func, callee: &Func, args: yavs_ir::Regs) -> bool {
    let n = f.reg_ty.len() as u32;
    f.regs(args).is_some_and(|rs| {
        rs.len() == callee.params.len()
            && rs.iter().zip(&callee.params).all(|(r, p)| {
                r.0.checked_add(u32::from(p.width == Width::Sig))
                    .is_some_and(|x| x < n)
            })
    })
}

/// The functions reachable through calls from `roots` that can run on native frames: each
/// `native_local`, and calling only such functions (the greatest such set).
pub(crate) fn native_set(
    prog: &Program,
    roots: impl IntoIterator<Item = FuncId>,
) -> std::collections::HashSet<FuncId> {
    let mut set = std::collections::HashSet::new();
    let mut seen = std::collections::HashSet::new();
    let mut todo: Vec<FuncId> = roots.into_iter().collect();
    while let Some(id) = todo.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(f) = prog.func(id).filter(|f| native_local(prog, f)) else {
            continue;
        };
        set.insert(id);
        for i in &f.code {
            if let Inst::Call { func, .. } = i {
                todo.push(*func);
            }
        }
    }
    loop {
        let out: Vec<FuncId> = set
            .iter()
            .copied()
            .filter(|&id| {
                prog.func(id).is_none_or(|f| {
                    f.code.iter().any(|i| match *i {
                        Inst::Call { func, args, .. } => {
                            !set.contains(&func)
                                || prog.func(func).is_none_or(|g| !call_fits(f, g, args))
                        }
                        _ => false,
                    })
                })
            })
            .collect();
        if out.is_empty() {
            return set;
        }
        for id in out {
            set.remove(&id);
        }
    }
}

struct Gen {
    ctx: LLVMContextRef,
    module: LLVMModuleRef,
    b: LLVMBuilderRef,
    i8: LLVMTypeRef,
    i32: LLVMTypeRef,
    i64: LLVMTypeRef,
    f64: LLVMTypeRef,
    ptr: LLVMTypeRef,
    fn_ty: LLVMTypeRef,
    step: LLVMValueRef,
    /// `yavs_rt_host(env, mem, addr, extra, n, write) -> ptr`, null for the slow path.
    host_ty: LLVMTypeRef,
    host: LLVMValueRef,
    /// `yavs_rt_signal(env, regs, mem, pc, edge) -> u32`, as `step`.
    signal_ty: LLVMTypeRef,
    signal: LLVMValueRef,
    /// `yavs_rt_drive(env, regs, mem, pc, driver, lo, len, wave, inertial) -> u32`.
    drive_ty: LLVMTypeRef,
    drive: LLVMValueRef,
    /// `yavs_rt_wait(env, regs, mem, pc, lo, len) -> u32`.
    wait_ty: LLVMTypeRef,
    wait: LLVMValueRef,
    /// `yavs_rt_lcopy(dst, nd, d0, src, ns, s0, n)`.
    lcopy_ty: LLVMTypeRef,
    lcopy: LLVMValueRef,
    /// `yavs_rt_layout(env, regs, mem, desc, dst, dims, stride, align, el_count, logic) -> u32`.
    layout_ty: LLVMTypeRef,
    layout: LLVMValueRef,
    /// `yavs_rt_fast(env, regs, mem, op, dst, a, b) -> u32`.
    fast_ty: LLVMTypeRef,
    fast: LLVMValueRef,
    /// `yavs_rt_deopt(env, mem, func, pc, temp_base, nregs, slen) -> ptr`.
    deopt_ty: LLVMTypeRef,
    deopt: LLVMValueRef,
    /// `yavs_rt_deopt_slots(env) -> ptr`.
    deopt_slots_ty: LLVMTypeRef,
    deopt_slots: LLVMValueRef,
    /// `yavs_rt_deopt_end(env, mem, pc) -> u32`.
    deopt_end_ty: LLVMTypeRef,
    deopt_end: LLVMValueRef,
    /// `yavs_rt_memeq(a, b, n) -> i64`.
    memeq_ty: LLVMTypeRef,
    memeq: LLVMValueRef,
    /// `llvm.frameaddress.p0(i32) -> ptr`.
    frame_addr: (LLVMTypeRef, LLVMValueRef),
    /// Native callees are called by symbol (one module, `compile`), else through `Mem::ntab`.
    pub(crate) direct: bool,
    /// Which memory a pointer value points into (`AA_*`), for alias scope metadata.
    kinds: std::cell::RefCell<std::collections::HashMap<usize, usize>>,
    /// `alias.scope` and `noalias` list nodes per kind, and their metadata kind ids.
    aa: std::cell::RefCell<Option<AaNodes>>,
    sadd: (LLVMTypeRef, LLVMValueRef),
    ssub: (LLVMTypeRef, LLVMValueRef),
    smul: (LLVMTypeRef, LLVMValueRef),
    umul: (LLVMTypeRef, LLVMValueRef),
    uadd: (LLVMTypeRef, LLVMValueRef),
}

/// Memory kinds that never overlap: the frame's slots, the temporary area, the instance data,
/// constants, the signals' code bytes. A pointer of a known kind gets alias scope metadata, so
/// LLVM keeps values in registers across stores to the others.
const AA_SLOTS: usize = 0;
const AA_TEMP: usize = 1;
const AA_INST: usize = 2;
const AA_CONST: usize = 3;
const AA_CODES: usize = 4;
const AA_N: usize = 5;

struct AaNodes {
    scope: [LLVMValueRef; AA_N],
    others: [LLVMValueRef; AA_N],
    k_scope: u32,
    k_noalias: u32,
}

/// The function being generated.
struct Cur<'p> {
    prog: &'p Program,
    /// The private globals holding constants this function reads (`Gen::host`).
    consts: std::cell::RefCell<std::collections::HashMap<yavs_ir::ConstId, LLVMValueRef>>,
    f: LLVMValueRef,
    env: LLVMValueRef,
    regs: LLVMValueRef,
    mem: LLVMValueRef,
    /// What the generator knows about each register at the current instruction (`track`).
    known: std::cell::RefCell<Vec<Option<Known>>>,
    /// Registers the function defines once, by a `Const`: that value wherever they are read
    /// (the interpreter's loop skips such a definition and relies on the same).
    fixed: Vec<Option<i64>>,
    /// The one block that returns: `ret` of its phi (`leave`).
    exit: LLVMBasicBlockRef,
    exit_pc: LLVMValueRef,
    /// The frame's size in bytes (`Func::frame`): the slots of every frame of the function.
    frame_len: u64,
    /// The native frame, when this is a function's native-frame code (`f<id>_n`).
    nat: Option<Nat>,
    /// Frame-mode code that stores a register only where the interpreter can look at it.
    defer: Option<Defer>,
}

/// Deferred register stores (`regs.rs`): a definition of a pure instruction is kept as a value
/// in `pend` and stored by `Gen::flush` where it is live and compiled code leaves the frame
/// to the interpreter (a failing guard, a stepped instruction, a block edge); registers the
/// interpreter preloads are never stored.
struct Defer {
    live: crate::regs::Live,
    pre: Vec<bool>,
    pend: std::cell::RefCell<Vec<Option<LLVMValueRef>>>,
    /// Registers deferred by the instruction being generated.
    fresh: std::cell::RefCell<Vec<u32>>,
    /// The pc of the instruction being generated.
    pc: std::cell::Cell<u32>,
}

/// A native frame (`docs/decisions.md` "Compiled frames"): slots in an `alloca`, the high half
/// of its frame addresses, its frame index, the temporary area's length at entry, a word for
/// the helpers that write a register, and the function.
struct Nat {
    slots: LLVMValueRef,
    fhi: LLVMValueRef,
    depth: LLVMValueRef,
    temp_base: LLVMValueRef,
    scratch: LLVMValueRef,
    id: FuncId,
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
            let mut params = [ptr, ptr, ptr, i32];
            let fn_ty = LLVMFunctionType(i32, params.as_mut_ptr(), 4, 0);
            let step_name = CString::new(STEP).expect("no NUL");
            let host_name = CString::new(HOST).expect("no NUL");
            let mut hparams = [ptr, ptr, i64, i64, i64, i32];
            let host_ty = LLVMFunctionType(ptr, hparams.as_mut_ptr(), 6, 0);
            let signal_name = CString::new(SIGNAL).expect("no NUL");
            let mut sparams = [ptr, ptr, ptr, i32, i32];
            let signal_ty = LLVMFunctionType(i32, sparams.as_mut_ptr(), 5, 0);
            let drive_name = CString::new(DRIVE).expect("no NUL");
            let mut dparams = [ptr, ptr, ptr, i32, i32, i64, i64, ptr, i32];
            let drive_ty = LLVMFunctionType(i32, dparams.as_mut_ptr(), 9, 0);
            let wait_name = CString::new(WAIT).expect("no NUL");
            let mut wparams = [ptr, ptr, ptr, i32, i64, i64];
            let wait_ty = LLVMFunctionType(i32, wparams.as_mut_ptr(), 6, 0);
            let lcopy_name = CString::new(LCOPY).expect("no NUL");
            let mut lparams = [ptr, i64, i64, ptr, i64, i64, i64];
            let lcopy_ty = LLVMFunctionType(LLVMVoidTypeInContext(ctx), lparams.as_mut_ptr(), 7, 0);
            let fast_name = CString::new(FAST).expect("no NUL");
            let mut fparams = [ptr, ptr, ptr, i32, i32, i64, i64];
            let fast_ty = LLVMFunctionType(i32, fparams.as_mut_ptr(), 7, 0);
            let layout_name = CString::new(LAYOUT).expect("no NUL");
            let mut yparams = [ptr, ptr, ptr, i64, i32, i32, i64, i64, i64, i32];
            let layout_ty = LLVMFunctionType(i32, yparams.as_mut_ptr(), 10, 0);
            let mut dparams = [ptr, ptr, i32, i32, i64, i64, i64];
            let deopt_ty = LLVMFunctionType(ptr, dparams.as_mut_ptr(), 7, 0);
            let mut dsparams = [ptr];
            let deopt_slots_ty = LLVMFunctionType(ptr, dsparams.as_mut_ptr(), 1, 0);
            let mut deparams = [ptr, ptr, i32];
            let deopt_end_ty = LLVMFunctionType(i32, deparams.as_mut_ptr(), 3, 0);
            let mut mparams = [ptr, ptr, i64];
            let memeq_ty = LLVMFunctionType(i64, mparams.as_mut_ptr(), 3, 0);
            let add = |name: &str, ty| {
                let n = CString::new(name).expect("no NUL");
                LLVMAddFunction(module, n.as_ptr(), ty)
            };
            let frame_addr = {
                let id = LLVMLookupIntrinsicID(c"llvm.frameaddress".as_ptr(), 17);
                assert_ne!(id, 0, "unknown intrinsic llvm.frameaddress");
                let mut tys = [ptr];
                (
                    LLVMIntrinsicGetType(ctx, id, tys.as_mut_ptr(), 1),
                    LLVMGetIntrinsicDeclaration(module, id, tys.as_mut_ptr(), 1),
                )
            };
            let g = Gen {
                ctx,
                module,
                b: LLVMCreateBuilderInContext(ctx),
                i8: LLVMInt8TypeInContext(ctx),
                i32,
                i64,
                f64: LLVMDoubleTypeInContext(ctx),
                ptr,
                fn_ty,
                step: LLVMAddFunction(module, step_name.as_ptr(), fn_ty),
                host_ty,
                host: LLVMAddFunction(module, host_name.as_ptr(), host_ty),
                signal_ty,
                signal: LLVMAddFunction(module, signal_name.as_ptr(), signal_ty),
                drive_ty,
                drive: LLVMAddFunction(module, drive_name.as_ptr(), drive_ty),
                wait_ty,
                wait: LLVMAddFunction(module, wait_name.as_ptr(), wait_ty),
                lcopy_ty,
                lcopy: LLVMAddFunction(module, lcopy_name.as_ptr(), lcopy_ty),
                layout_ty,
                layout: LLVMAddFunction(module, layout_name.as_ptr(), layout_ty),
                fast_ty,
                fast: LLVMAddFunction(module, fast_name.as_ptr(), fast_ty),
                deopt_ty,
                deopt: add(DEOPT, deopt_ty),
                deopt_slots_ty,
                deopt_slots: add(DEOPT_SLOTS, deopt_slots_ty),
                deopt_end_ty,
                deopt_end: add(DEOPT_END, deopt_end_ty),
                memeq_ty,
                memeq: add(MEMEQ, memeq_ty),
                frame_addr,
                direct: false,
                kinds: Default::default(),
                aa: Default::default(),
                sadd: intrinsic(c"llvm.sadd.with.overflow"),
                ssub: intrinsic(c"llvm.ssub.with.overflow"),
                smul: intrinsic(c"llvm.smul.with.overflow"),
                umul: intrinsic(c"llvm.umul.with.overflow"),
                uadd: intrinsic(c"llvm.uadd.with.overflow"),
            };
            g.effects();
            g
        }
    }

    /// Attribute `name` (with `val` for an integer attribute) on function `f` or call `f` at
    /// `idx` (`LLVMAttributeFunctionIndex` for the function).
    unsafe fn attr(&self, f: LLVMValueRef, call: bool, name: &str, val: u64) {
        // SAFETY: a function or call of our context; the attribute exists in LLVM 21.
        unsafe {
            let kind = LLVMGetEnumAttributeKindForName(name.as_ptr().cast(), name.len());
            assert_ne!(kind, 0, "unknown attribute {name}");
            let a = LLVMCreateEnumAttribute(self.ctx, kind, val);
            let idx = llvm_sys::LLVMAttributeFunctionIndex;
            if call {
                LLVMAddCallSiteAttribute(f, idx, a);
            } else {
                LLVMAddAttributeAtIndex(f, idx, a);
            }
        }
    }

    /// What the runtime helpers do, so LLVM keeps values in registers across them and moves
    /// loads past them (`docs/decisions.md` "Compiled frames", effects): none unwinds (they
    /// catch panics); `yavs_rt_host` touches only `Mem`, `Env` and the interpreter's own
    /// bookkeeping, never object bytes, registers or slots; `yavs_rt_lcopy` only the blocks it
    /// is given. `yavs_rt_step` and `yavs_rt_signal` run interpreter code that may collect or
    /// change any memory: no memory attribute, so registers holding handles are reloaded after
    /// them and the precise roots (#422) see what compiled code stored.
    unsafe fn effects(&self) {
        // SAFETY: functions of our module.
        unsafe {
            for f in [
                self.step,
                self.host,
                self.signal,
                self.drive,
                self.wait,
                self.lcopy,
                self.layout,
                self.fast,
                self.deopt,
                self.deopt_slots,
                self.deopt_end,
            ] {
                self.attr(f, false, "nounwind", 0);
            }
            self.attr(self.memeq, false, "nounwind", 0);
            for f in [self.host, self.lcopy, self.deopt_slots, self.memeq] {
                self.attr(f, false, "willreturn", 0);
            }
            self.attr(self.memeq, false, "memory", mem_effects(REF, 0, 0));
            self.attr(self.memeq, false, "nofree", 0);
            self.attr(self.memeq, false, "nosync", 0);
            self.attr(self.host, false, "memory", mem_effects(RW, RW, 0));
            self.attr(self.lcopy, false, "memory", mem_effects(RW, 0, 0));
            self.attr(self.lcopy, false, "nofree", 0);
            self.attr(self.lcopy, false, "nosync", 0);
            self.attr(self.deopt_slots, false, "memory", mem_effects(REF, 0, 0));
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

    /// What the generator knows about address register `r` (not an integer).
    fn known_addr(&self, c: &Cur, r: Reg) -> Option<Known> {
        match c.known.borrow().get(r.0 as usize) {
            Some(&Some(k)) if !matches!(k, Known::Int(_)) => Some(k),
            _ => None,
        }
    }

    /// The value of register `r` when the generator knows it.
    fn kint(&self, c: &Cur, r: Reg) -> Option<i64> {
        match c.known.borrow().get(r.0 as usize) {
            Some(Some(Known::Int(v))) => Some(*v),
            _ => c.fixed.get(r.0 as usize).copied().flatten(),
        }
    }

    unsafe fn ld(&self, c: &Cur, r: Reg) -> LLVMValueRef {
        if let Some(v) = self.kint(c, r) {
            return self.c64(v);
        }
        if let Some(d) = &c.defer
            && let Some(Some(v)) = d.pend.borrow().get(r.0 as usize)
        {
            return *v;
        }
        // SAFETY: see `slot`.
        unsafe { LLVMBuildLoad2(self.b, self.i64, self.slot(c, r), c"".as_ptr()) }
    }

    /// Return `v` (an i32): a branch to the function's exit block.
    unsafe fn leave(&self, c: &Cur, v: LLVMValueRef) {
        // SAFETY: the builder is in a block of c.f; the phi is the exit block's.
        unsafe {
            let mut vals = [v];
            let mut from = [LLVMGetInsertBlock(self.b)];
            LLVMAddIncoming(c.exit_pc, vals.as_mut_ptr(), from.as_mut_ptr(), 1);
            LLVMBuildBr(self.b, c.exit);
        }
    }

    unsafe fn ldf(&self, c: &Cur, r: Reg) -> LLVMValueRef {
        // SAFETY: see `slot`.
        unsafe { LLVMBuildBitCast(self.b, self.ld(c, r), self.f64, c"".as_ptr()) }
    }

    /// Store `v` in register `r` at once.
    unsafe fn st(&self, c: &Cur, r: Reg, v: LLVMValueRef) {
        if let Some(d) = &c.defer
            && let Some(p) = d.pend.borrow_mut().get_mut(r.0 as usize)
        {
            *p = None;
        }
        // SAFETY: see `slot`; `v` is an i64.
        unsafe {
            LLVMBuildStore(self.b, v, self.slot(c, r));
        }
    }

    /// Define register `r` as `v` without a store: `flush` stores it where the frame is looked
    /// at. Only in straight-line code (not between the fast and slow path of an instruction).
    unsafe fn std(&self, c: &Cur, r: Reg, v: LLVMValueRef) {
        if let Some(d) = &c.defer
            && let Some(p) = d.pend.borrow_mut().get_mut(r.0 as usize)
        {
            // The interpreter starts every frame with a register defined once by a constant
            // or an address: nothing to store.
            *p = (!d.pre[r.0 as usize]).then_some(v);
            d.fresh.borrow_mut().push(r.0);
            return;
        }
        // SAFETY: as `st`.
        unsafe { self.st(c, r, v) }
    }

    unsafe fn stfd(&self, c: &Cur, r: Reg, v: LLVMValueRef) {
        // SAFETY: `v` is a double.
        unsafe { self.std(c, r, LLVMBuildBitCast(self.b, v, self.i64, c"".as_ptr())) }
    }

    /// Store the deferred registers that `live` (a row of `Live`) has live.
    unsafe fn flush_row(&self, c: &Cur, live: &[u64]) {
        let Some(d) = &c.defer else { return };
        for (r, v) in d.pend.borrow().iter().enumerate() {
            if let Some(v) = v
                && live.get(r / 64).is_some_and(|w| w >> (r % 64) & 1 != 0)
            {
                // SAFETY: r is a register of the function.
                unsafe { LLVMBuildStore(self.b, *v, self.slot(c, Reg(r as u32))) };
            }
        }
    }

    /// Store what the interpreter reads when it takes over at instruction `pc`.
    unsafe fn flush(&self, c: &Cur, pc: usize) {
        if let Some(d) = &c.defer {
            // SAFETY: as `flush_row`.
            unsafe { self.flush_row(c, d.live.row(pc)) };
        }
    }

    /// Store what the successors `pcs` read.
    unsafe fn flush_edge(&self, c: &Cur, pcs: &[usize]) {
        let Some(d) = &c.defer else { return };
        let mut row = d.live.row(pcs[0]).to_vec();
        for &p in &pcs[1..] {
            for (a, b) in row.iter_mut().zip(d.live.row(p)) {
                *a |= b;
            }
        }
        // SAFETY: as `flush_row`.
        unsafe { self.flush_row(c, &row) };
    }

    /// Forget the deferred registers (the frame has them all, or nothing reaches the code).
    fn drop_pend(&self, c: &Cur) {
        if let Some(d) = &c.defer {
            d.pend.borrow_mut().fill(None);
        }
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
            self.flush(c, pc as usize);
            self.leave(c, self.c32(pc));
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
    /// The type of `f`'s native-frame code: `(env, mem, depth, argument cells...) -> i64`.
    unsafe fn native_ty(&self, f: &Func) -> LLVMTypeRef {
        let cells: usize = f
            .params
            .iter()
            .map(|p| 1 + usize::from(p.width == Width::Sig))
            .sum();
        let mut params = vec![self.ptr, self.ptr, self.i64];
        params.extend(std::iter::repeat_n(self.i64, cells));
        // SAFETY: types of our context.
        unsafe { LLVMFunctionType(self.i64, params.as_mut_ptr(), params.len() as u32, 0) }
    }

    /// The function named `name` with type `ty`, declared if not yet in the module.
    unsafe fn declare(&self, name: &str, ty: LLVMTypeRef) -> LLVMValueRef {
        let cname = CString::new(name).expect("no NUL");
        // SAFETY: a function of our module.
        unsafe {
            let f = LLVMGetNamedFunction(self.module, cname.as_ptr());
            if f.is_null() {
                LLVMAddFunction(self.module, cname.as_ptr(), ty)
            } else {
                f
            }
        }
    }

    /// Generate `f` (`id`) as `name`: its frame-mode code, or with `native` its native-frame
    /// code (`docs/decisions.md` "Compiled frames"). Calls of functions in `nc` are native
    /// calls.
    #[allow(clippy::too_many_arguments)]
    unsafe fn func(
        &self,
        prog: &Program,
        f: &Func,
        id: FuncId,
        name: &str,
        native: bool,
        nc: &std::collections::HashSet<FuncId>,
    ) -> Result<(), String> {
        // SAFETY: all values are built in this function with matching types.
        unsafe {
            let fty = if native {
                self.native_ty(f)
            } else {
                self.fn_ty
            };
            let fv = self.declare(name, fty);
            // `regs` and `mem` overlap no other memory the function reaches (frame slots,
            // temporaries, objects), so LLVM may keep their values in registers between
            // calls; the callbacks get both pointers.
            let kind = LLVMGetEnumAttributeKindForName(c"noalias".as_ptr(), 7);
            let restrict: &[u32] = if native { &[2] } else { &[2, 3] };
            for &i in restrict {
                LLVMAddAttributeAtIndex(fv, i, LLVMCreateEnumAttribute(self.ctx, kind, 0));
            }
            let exit = LLVMAppendBasicBlockInContext(self.ctx, fv, c"".as_ptr());
            LLVMPositionBuilderAtEnd(self.b, exit);
            let exit_pc = LLVMBuildPhi(self.b, self.i32, c"".as_ptr());
            if !native {
                LLVMBuildRet(self.b, exit_pc);
            }
            let entry = LLVMAppendBasicBlockInContext(self.ctx, fv, c"".as_ptr());
            LLVMMoveBasicBlockBefore(entry, exit);
            LLVMPositionBuilderAtEnd(self.b, entry);
            let nregs = f.reg_ty.len();
            let (regs, mem, nat) = if native {
                let rty = LLVMArrayType2(self.i64, nregs.max(1) as u64);
                let regs = LLVMBuildAlloca(self.b, rty, c"".as_ptr());
                let sty = LLVMArrayType2(self.i64, (u64::from(f.frame)).div_ceil(8).max(1));
                let slots = LLVMBuildAlloca(self.b, sty, c"".as_ptr());
                let scratch = LLVMBuildAlloca(self.b, self.i64, c"".as_ptr());
                let depth = LLVMGetParam(fv, 2);
                let tag = self.c64((u64::from(space::FRAME) << 29) as i64);
                let fhi = LLVMBuildOr(self.b, tag, depth, c"".as_ptr());
                let nat = Nat {
                    slots,
                    fhi,
                    depth,
                    temp_base: std::ptr::null_mut(),
                    scratch,
                    id,
                };
                (regs, LLVMGetParam(fv, 1), Some(nat))
            } else {
                (LLVMGetParam(fv, 1), LLVMGetParam(fv, 2), None)
            };
            let mut c = Cur {
                prog,
                consts: Default::default(),
                f: fv,
                env: LLVMGetParam(fv, 0),
                regs,
                mem,
                known: std::cell::RefCell::new(vec![None; f.reg_ty.len()]),
                fixed: fixed_consts(f),
                exit,
                exit_pc,
                frame_len: u64::from(f.frame),
                nat,
                defer: None,
            };
            // Registers the interpreter starts the frame with hold the same address everywhere.
            let mut base_known: Vec<Option<Known>> = vec![None; nregs];
            if !native {
                let pre = crate::regs::preloaded(f);
                for i in &f.code {
                    if let Inst::AddrOf { dst, space, off } = *i
                        && pre.get(dst.0 as usize) == Some(&true)
                    {
                        base_known[dst.0 as usize] = match space {
                            AddrSpace::Frame => Some(Known::Frame(off.into())),
                            AddrSpace::Const(k) => Some(Known::Const(k, off.into())),
                            AddrSpace::Inst => Some(Known::Inst(off.into())),
                            _ => None,
                        };
                    }
                }
                *c.known.borrow_mut() = base_known.clone();
            }
            if !native && let Some(live) = crate::regs::Live::new(prog, f) {
                c.defer = Some(Defer {
                    live,
                    pre: crate::regs::preloaded(f),
                    pend: std::cell::RefCell::new(vec![None; nregs]),
                    fresh: Default::default(),
                    pc: Default::default(),
                });
            }
            let n = f.code.len();
            // Entry points: 0, block starts, and the instruction after each that may leave
            // frames to the interpreter or suspend.
            let mut leader = vec![false; n + 1];
            leader[0] = true;
            for (_, b) in f.blocks.iter() {
                if let Some(l) = leader.get_mut(b.start as usize) {
                    *l = true;
                }
            }
            for (i, inst) in f.code.iter().enumerate() {
                if resumes(inst) {
                    leader[i + 1] = true;
                }
            }
            // The interpreter enters a function after the constants and addresses it preloads
            // into the frame's registers (`Pre::next`): the pcs after such definitions at a
            // leader are entry points too (what is known there is what the registers hold).
            let mut enters = leader.clone();
            if !native {
                for i in 1..n {
                    let preload = match f.code[i - 1] {
                        Inst::Const { .. } | Inst::ConstF { .. } => true,
                        Inst::AddrOf { space, .. } => !matches!(space, AddrSpace::Up(_)),
                        _ => false,
                    };
                    enters[i] |= enters[i - 1] && preload;
                }
            }
            let mut bbs = vec![std::ptr::null_mut(); n];
            for (i, bb) in bbs.iter_mut().enumerate() {
                if enters[i] {
                    *bb = self.block(&c);
                }
            }
            if native && bbs.is_empty() {
                return Err("a function without instructions".into());
            }
            if native {
                // The frame as the interpreter's push makes it: zeroed slots, the arguments
                // in the parameter slots.
                let nat = c.nat.as_mut().expect("native");
                let slots = nat.slots;
                LLVMBuildMemSet(
                    self.b,
                    slots,
                    LLVMConstInt(self.i8, 0, 0),
                    self.c64(i64::from(f.frame)),
                    8,
                );
                let mut k = 3;
                for p in &f.params {
                    let at = self.gep(slots, self.c64(i64::from(p.slot)));
                    let v = LLVMGetParam(fv, k);
                    k += 1;
                    match p.width {
                        Width::Sig => {
                            self.store(v, at);
                            self.store(LLVMGetParam(fv, k), self.gep(at, self.c64(8)));
                            k += 1;
                        }
                        Width::B8 => self.store(v, at),
                        w => self.store(
                            LLVMBuildTrunc(self.b, v, self.width_ty(w), c"".as_ptr()),
                            at,
                        ),
                    }
                }
                nat.temp_base = LLVMBuildLoad2(
                    self.b,
                    self.i64,
                    self.gep(mem, self.c64(MEM_TEMP_LEN as i64)),
                    c"".as_ptr(),
                );
                LLVMBuildBr(self.b, bbs[0]);
            } else {
                let pc_in = LLVMGetParam(fv, 3);
                let other = self.block(&c);
                LLVMPositionBuilderAtEnd(self.b, other);
                LLVMBuildRet(self.b, pc_in);
                LLVMPositionBuilderAtEnd(self.b, entry);
                let nl = enters[..n].iter().filter(|&&l| l).count();
                let sw = LLVMBuildSwitch(self.b, pc_in, other, nl as u32);
                for (i, &bb) in bbs.iter().enumerate() {
                    if !bb.is_null() {
                        LLVMAddCase(sw, self.c32(i as u32), bb);
                    }
                }
            }
            let c = c;
            let block_bb = |b: yavs_ir::BlockId| -> Result<LLVMBasicBlockRef, String> {
                f.blocks
                    .get(b)
                    .and_then(|b| bbs.get(b.start as usize).copied())
                    .filter(|bb| !bb.is_null())
                    .ok_or_else(|| format!("branch to missing block {}", b.0))
            };
            let start_of = |b: yavs_ir::BlockId| -> Result<usize, String> {
                f.blocks
                    .get(b)
                    .map(|b| b.start as usize)
                    .ok_or_else(|| format!("branch to missing block {}", b.0))
            };
            let mut open = false;
            let mut prev = None;
            for (i, &inst) in f.code.iter().enumerate() {
                let pc = i as u32;
                if let Some(p) = prev.replace(inst) {
                    track(&mut c.known.borrow_mut(), &c.fixed, &p);
                    // What the previous instruction defined and did not defer is in the frame.
                    if let Some(d) = &c.defer {
                        let fresh = std::mem::take(&mut *d.fresh.borrow_mut());
                        let mut pend = d.pend.borrow_mut();
                        p.visit(&mut |r, role, _| {
                            if role == yavs_ir::Role::Def {
                                let sig = f.reg_ty.get(r.0 as usize) == Some(&yavs_ir::RegTy::Sig);
                                for k in [r.0, r.0 + 1].into_iter().take(1 + usize::from(sig)) {
                                    if !fresh.contains(&k)
                                        && let Some(x) = pend.get_mut(k as usize)
                                    {
                                        *x = None;
                                    }
                                }
                            }
                        });
                    }
                }
                if let Some(d) = &c.defer {
                    d.pc.set(pc);
                }
                if leader[i] {
                    c.known.borrow_mut().clone_from(&base_known);
                }
                // What is known about an address register (`Gen::host`).
                let fk = |r: Reg| self.known_addr(&c, r);
                if enters[i] {
                    if open {
                        self.flush(&c, i);
                        LLVMBuildBr(self.b, bbs[i]);
                    }
                    LLVMPositionBuilderAtEnd(self.b, bbs[i]);
                    self.drop_pend(&c);
                    open = true;
                } else if !open {
                    // Dead code after a terminator: give it a block, LLVM drops it.
                    let bb = self.block(&c);
                    LLVMPositionBuilderAtEnd(self.b, bb);
                    self.drop_pend(&c);
                    open = true;
                }
                // The interpreter returns (`TierHost::ret` for a frame compiled code pushed):
                // compiled code leaves the frame to it there.
                if let Inst::Ret { v } = inst {
                    match &c.nat {
                        // A native frame returns its value itself.
                        Some(_) => {
                            let v = v.map_or_else(|| self.c64(0), |r| self.ld(&c, r));
                            LLVMBuildRet(self.b, v);
                        }
                        None => {
                            self.flush(&c, i);
                            self.leave(&c, self.c32(pc))
                        }
                    }
                    open = false;
                    continue;
                }
                match inst {
                    Inst::Const { dst, v } => self.std(&c, dst, self.c64(v)),
                    Inst::ConstF { dst, v } => self.std(&c, dst, self.c64(v.to_bits() as i64)),
                    Inst::Move { dst, src } => self.std(&c, dst, self.ld(&c, src)),
                    Inst::Select { dst, c: cond, a, b } => {
                        let t = self.icmp(IP::LLVMIntNE, self.ld(&c, cond), self.c64(0));
                        let v = LLVMBuildSelect(
                            self.b,
                            t,
                            self.ld(&c, a),
                            self.ld(&c, b),
                            c"".as_ptr(),
                        );
                        self.std(&c, dst, v);
                    }
                    Inst::Bin { op, dst, a, b } => {
                        let (x, y) = (self.ld(&c, a), self.ld(&c, b));
                        match self.int_op(&c, op, x, y, pc) {
                            Some(v) => self.std(&c, dst, v),
                            None => self.step(&c, pc, &inst),
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
                                self.std(&c, dst, v);
                            }
                            None => self.step(&c, pc, &inst),
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
                        self.std(&c, dst, v);
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
                                self.step(&c, pc, &inst);
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
                        self.stfd(&c, dst, v);
                    }
                    Inst::FUn { op, dst, a } => {
                        let x = self.ldf(&c, a);
                        let v = match op {
                            FUnOp::Neg => LLVMBuildFNeg(self.b, x, c"".as_ptr()),
                            FUnOp::Abs => self.fabs(x),
                        };
                        self.stfd(&c, dst, v);
                    }
                    Inst::IToF { dst, src } => {
                        let v = LLVMBuildSIToFP(self.b, self.ld(&c, src), self.f64, c"".as_ptr());
                        self.stfd(&c, dst, v);
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
                        self.std(&c, dst, self.zext(v));
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
                        self.std(&c, dst, self.zext(v));
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
                        self.std(&c, dst, self.zext(v));
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
                    Inst::AddrOf { dst, space, off } => match addr_const(space, off) {
                        Some(a) if space == AddrSpace::Frame => {
                            let hi = self.own_hi(&c);
                            let hi = LLVMBuildShl(self.b, hi, self.c64(32), c"".as_ptr());
                            self.std(
                                &c,
                                dst,
                                LLVMBuildOr(self.b, hi, self.c64(a as i64), c"".as_ptr()),
                            );
                        }
                        Some(a) => self.std(&c, dst, self.c64(a as i64)),
                        // A frame one or two static links up, from the view.
                        None => match space {
                            AddrSpace::Up(d @ 1..=2) => {
                                let (slow, join) = self.slow_path(&c, pc, &inst);
                                let hi = self.mem_ld(&c, MEM_UP + 8 * u64::from(d - 1), self.i64);
                                let ok = self.icmp(IP::LLVMIntNE, hi, self.c64(-1));
                                self.guard(&c, ok, slow);
                                let hi = LLVMBuildShl(self.b, hi, self.c64(32), c"".as_ptr());
                                let a = LLVMBuildOr(self.b, hi, self.c64(off.into()), c"".as_ptr());
                                self.st(&c, dst, a);
                                self.join(join);
                            }
                            _ => self.step(&c, pc, &inst),
                        },
                    },
                    // A fat pair is recorded only when a word points into a heap
                    // (`Machine::fat_root`); the helper says when none does.
                    Inst::TempRoot { addr, ty: None, .. } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        self.fast_op(&c, FAST_FAT_ROOT, 0, self.ld(&c, addr), self.c64(0), slow);
                        self.join(join);
                    }
                    Inst::Stmt => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let roots = self.mem_ld(&c, MEM_TEMP_ROOTS, self.i64);
                        self.guard(&c, self.icmp(IP::LLVMIntEQ, roots, self.c64(0)), slow);
                        let len = self.mem_ld(&c, MEM_TEMP_LEN, self.i64);
                        let base = match &c.nat {
                            Some(n) => n.temp_base,
                            None => self.mem_ld(&c, MEM_TEMP_BASE, self.i64),
                        };
                        let lt = self.icmp(IP::LLVMIntULT, base, len);
                        let v = LLVMBuildSelect(self.b, lt, base, len, c"".as_ptr());
                        self.mem_st(&c, MEM_TEMP_LEN, v);
                        self.join(join);
                    }
                    Inst::Alloc { dst, size } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let (a, _) = self.temp_alloc(&c, self.ld(&c, size), slow);
                        self.st(&c, dst, a);
                        self.join(join);
                    }
                    Inst::LayoutOf { dst, ty, desc } => match static_layout(prog, ty) {
                        Some(StaticLayout::Array { dims, elem, logic }) => {
                            // `yavs_rt_layout` does what `Ty::layout` would; 0 is the interpreter's.
                            let (slow, join) = self.slow_path(&c, pc, &inst);
                            let stride = elem.size.next_multiple_of(elem.align.max(1));
                            let (rp, d) = self.reg_out(&c, dst);
                            let mut args = [
                                c.env,
                                rp,
                                c.mem,
                                self.ld(&c, desc),
                                self.c32(d),
                                self.c32(u32::from(dims)),
                                self.c64(stride as i64),
                                self.c64(elem.align as i64),
                                self.c64(elem.count as i64),
                                self.c32(u32::from(logic)),
                            ];
                            let ok = LLVMBuildCall2(
                                self.b,
                                self.layout_ty,
                                self.layout,
                                args.as_mut_ptr(),
                                10,
                                c"".as_ptr(),
                            );
                            self.guard(&c, self.icmp(IP::LLVMIntNE, ok, self.c32(0)), slow);
                            self.reg_in(&c, dst);
                            self.join(join);
                        }
                        Some(sl) => {
                            let (slow, join) = self.slow_path(&c, pc, &inst);
                            let words = self.layout_words(&c, &sl, desc, fk(desc), slow);
                            let n = self.c64(words.len() as i64 * 8);
                            let (a, p) = self.temp_alloc(&c, n, slow);
                            for (k, w) in words.into_iter().enumerate() {
                                self.store(w, self.gep(p, self.c64(k as i64 * 8)));
                            }
                            self.st(&c, dst, a);
                            self.join(join);
                        }
                        None => self.step(&c, pc, &inst),
                    },
                    // A variable of the frame: no check, no slow path, so the result can be
                    // deferred like any other (`Gen::host` takes the same case).
                    Inst::Load { dst, addr, w }
                        if w != Width::Sig
                            && matches!(fk(addr), Some(Known::Frame(o))
                                if o.checked_add(w.bytes() as u64).is_some_and(|e| e <= c.frame_len)) =>
                    {
                        let n = self.c64(i64::from(w.bytes()));
                        let p = self.host(
                            &c,
                            self.ld(&c, addr),
                            fk(addr),
                            self.c64(0),
                            n,
                            false,
                            std::ptr::null_mut(),
                        );
                        let v = self.load(self.width_ty(w), p);
                        let v = match w {
                            Width::B8 => v,
                            Width::B4 => LLVMBuildSExt(self.b, v, self.i64, c"".as_ptr()),
                            _ => LLVMBuildZExt(self.b, v, self.i64, c"".as_ptr()),
                        };
                        self.std(&c, dst, v);
                    }
                    Inst::Load { dst, addr, w } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n = self.c64(i64::from(w.bytes()));
                        let p =
                            self.host(&c, self.ld(&c, addr), fk(addr), self.c64(0), n, false, slow);
                        match w {
                            Width::Sig => {
                                self.st(&c, dst, self.load(self.i64, p));
                                let p8 = self.gep(p, self.c64(8));
                                self.st(&c, Reg(dst.0 + 1), self.load(self.i64, p8));
                            }
                            _ => {
                                let ty = self.width_ty(w);
                                let v = self.load(ty, p);
                                let v = match w {
                                    Width::B8 => v,
                                    Width::B4 => LLVMBuildSExt(self.b, v, self.i64, c"".as_ptr()),
                                    _ => LLVMBuildZExt(self.b, v, self.i64, c"".as_ptr()),
                                };
                                self.st(&c, dst, v);
                            }
                        }
                        self.join(join);
                    }
                    Inst::Store { addr, src, w } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n = self.c64(i64::from(w.bytes()));
                        let p =
                            self.host(&c, self.ld(&c, addr), fk(addr), self.c64(0), n, true, slow);
                        match w {
                            Width::Sig => {
                                self.store(self.ld(&c, src), p);
                                let p8 = self.gep(p, self.c64(8));
                                self.store(self.ld(&c, Reg(src.0 + 1)), p8);
                            }
                            _ => {
                                let v = self.ld(&c, src);
                                let v = if w == Width::B8 {
                                    v
                                } else {
                                    LLVMBuildTrunc(self.b, v, self.width_ty(w), c"".as_ptr())
                                };
                                self.store(v, p);
                            }
                        }
                        self.join(join);
                    }
                    Inst::Copy { dst, src, bytes } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n = self.ld(&c, bytes);
                        let ps =
                            self.host(&c, self.ld(&c, src), fk(src), self.c64(0), n, false, slow);
                        let pd =
                            self.host(&c, self.ld(&c, dst), fk(dst), self.c64(0), n, true, slow);
                        LLVMBuildMemMove(self.b, pd, 1, ps, 1, n);
                        self.join(join);
                    }
                    Inst::MakeDope { at, parts } => {
                        let rs = f.regs(parts).ok_or("operand list out of range")?;
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n = self.c64(rs.len() as i64 * 8);
                        let p = self.host(&c, self.ld(&c, at), fk(at), self.c64(0), n, true, slow);
                        for (k, &r) in rs.iter().enumerate() {
                            let pk = self.gep(p, self.c64(k as i64 * 8));
                            self.store(self.ld(&c, r), pk);
                        }
                        self.join(join);
                    }
                    Inst::DopeLen { dst, dope, dim } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n24 = self.c64(24);
                        let p = self.host(
                            &c,
                            self.ld(&c, dope),
                            fk(dope),
                            self.c64(i64::from(dim) * 24),
                            n24,
                            false,
                            slow,
                        );
                        let (l, r, dir) = self.dope_words(p);
                        let (n, fits) = self.len_of(l, r, dir);
                        // A length over i64::MAX is the interpreter's error.
                        let small = self.icmp(IP::LLVMIntSGE, n, self.c64(0));
                        self.guard(&c, LLVMBuildAnd(self.b, fits, small, c"".as_ptr()), slow);
                        self.st(&c, dst, n);
                        self.join(join);
                    }
                    Inst::SubtypeCheck {
                        dope, want, dims, ..
                    } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let (d, w) = (self.ld(&c, dope), self.ld(&c, want));
                        self.fast_op(&c, FAST_SUBTYPE, u32::from(dims), d, w, slow);
                        self.join(join);
                    }
                    Inst::Pos {
                        dst,
                        dope,
                        dim,
                        idx,
                        ..
                    } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n24 = self.c64(24);
                        let p = self.host(
                            &c,
                            self.ld(&c, dope),
                            fk(dope),
                            self.c64(i64::from(dim) * 24),
                            n24,
                            false,
                            slow,
                        );
                        let (l, r, dir) = self.dope_words(p);
                        let i = self.ld(&c, idx);
                        let le = |a, b| self.icmp(IP::LLVMIntSLE, a, b);
                        let and = |a, b| LLVMBuildAnd(self.b, a, b, c"".as_ptr());
                        let up = and(
                            self.icmp(IP::LLVMIntEQ, dir, self.c64(0)),
                            and(le(l, i), le(i, r)),
                        );
                        let dn = and(
                            self.icmp(IP::LLVMIntEQ, dir, self.c64(1)),
                            and(le(r, i), le(i, l)),
                        );
                        self.guard(&c, self.or(up, dn), slow);
                        let a = LLVMBuildSub(self.b, i, l, c"".as_ptr());
                        let b = LLVMBuildSub(self.b, l, i, c"".as_ptr());
                        self.st(&c, dst, LLVMBuildSelect(self.b, up, a, b, c"".as_ptr()));
                        self.join(join);
                    }
                    Inst::Field {
                        dst,
                        base,
                        layout,
                        n,
                    } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n8 = self.c64(8);
                        let p = self.host(
                            &c,
                            self.ld(&c, layout),
                            fk(layout),
                            self.c64(16 + 8 * i64::from(n)),
                            n8,
                            false,
                            slow,
                        );
                        let off = self.load(self.i64, p);
                        let a = self.ld(&c, base);
                        let lo = LLVMBuildAnd(self.b, a, self.c64(0xffff_ffff), c"".as_ptr());
                        let (sum, o) = self.ovf(self.uadd, lo, off);
                        let big = self.icmp(IP::LLVMIntUGT, sum, self.c64(0xffff_ffff));
                        self.guard(
                            &c,
                            LLVMBuildNot(self.b, self.or(o, big), c"".as_ptr()),
                            slow,
                        );
                        let hi = LLVMBuildAnd(self.b, a, self.c64(!0xffff_ffff), c"".as_ptr());
                        self.st(&c, dst, LLVMBuildOr(self.b, hi, sum, c"".as_ptr()));
                        self.join(join);
                    }
                    Inst::LGet { dst, base, idx } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let (p, sh) = self.logic_word(&c, base, fk(base), idx, false, slow);
                        let mut code = self.c64(0);
                        for k in 0..4 {
                            let w = self.load(self.i64, self.gep(p, self.c64(k * 8)));
                            let bit = LLVMBuildLShr(self.b, w, sh, c"".as_ptr());
                            let bit = LLVMBuildAnd(self.b, bit, self.c64(1), c"".as_ptr());
                            let bit = LLVMBuildShl(self.b, bit, self.c64(k), c"".as_ptr());
                            code = LLVMBuildOr(self.b, code, bit, c"".as_ptr());
                        }
                        self.st(&c, dst, code);
                        self.join(join);
                    }
                    Inst::LSet { base, idx, v } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let code = self.ld(&c, v);
                        // A code beyond 'U' to '-' is the interpreter's error.
                        self.guard(&c, self.icmp(IP::LLVMIntULE, code, self.c64(8)), slow);
                        let (p, sh) = self.logic_word(&c, base, fk(base), idx, true, slow);
                        let one = LLVMBuildShl(self.b, self.c64(1), sh, c"".as_ptr());
                        let keep = LLVMBuildNot(self.b, one, c"".as_ptr());
                        for k in 0..4 {
                            let pk = self.gep(p, self.c64(k * 8));
                            let w =
                                LLVMBuildAnd(self.b, self.load(self.i64, pk), keep, c"".as_ptr());
                            let bit = LLVMBuildLShr(self.b, code, self.c64(k), c"".as_ptr());
                            let bit = LLVMBuildAnd(self.b, bit, self.c64(1), c"".as_ptr());
                            let bit = LLVMBuildShl(self.b, bit, sh, c"".as_ptr());
                            self.store(LLVMBuildOr(self.b, w, bit, c"".as_ptr()), pk);
                        }
                        self.join(join);
                    }
                    // The heap's own lookup through a helper; a failure is the interpreter's.
                    Inst::Deref { dst, handle, .. } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let h = self.ld(&c, handle);
                        self.fast_reg(&c, FAST_DEREF, dst, h, self.c64(0), slow);
                        self.join(join);
                    }
                    Inst::MemEq { dst, a, b, n } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let k = self.ld(&c, n);
                        let pa = self.host(&c, self.ld(&c, a), fk(a), self.c64(0), k, false, slow);
                        let pb = self.host(&c, self.ld(&c, b), fk(b), self.c64(0), k, false, slow);
                        let mut args = [pa, pb, k];
                        let v = LLVMBuildCall2(
                            self.b,
                            self.memeq_ty,
                            self.memeq,
                            args.as_mut_ptr(),
                            3,
                            c"".as_ptr(),
                        );
                        self.st(&c, dst, v);
                        self.join(join);
                    }
                    Inst::SigVal {
                        dst,
                        sig,
                        offset,
                        len,
                    } => {
                        // `Machine::sig_val`: element range within u32, an instance signal.
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let (o, l) = (self.ld(&c, offset), self.ld(&c, len));
                        let max = self.c64(i64::from(u32::MAX));
                        let n = self.mem_ld(&c, MEM_NSIGS, self.i64);
                        let ok = LLVMBuildAnd(
                            self.b,
                            self.icmp(IP::LLVMIntULE, o, max),
                            self.icmp(IP::LLVMIntULE, l, max),
                            c"".as_ptr(),
                        );
                        let s = self.c64(i64::from(sig));
                        let has = self.icmp(IP::LLVMIntULT, s, n);
                        self.guard(&c, LLVMBuildAnd(self.b, ok, has, c"".as_ptr()), slow);
                        let hi = self.c64(i64::from(sig) << 32);
                        self.st(&c, dst, LLVMBuildOr(self.b, hi, o, c"".as_ptr()));
                        self.st(&c, Reg(dst.0 + 1), l);
                        self.join(join);
                    }
                    Inst::Br { to } => {
                        let bb = block_bb(to)?;
                        self.flush_edge(&c, &[start_of(to)?]);
                        LLVMBuildBr(self.b, bb);
                        open = false;
                    }
                    Inst::CondBr { c: r, t, f: e } => {
                        let x = self.icmp(IP::LLVMIntNE, self.ld(&c, r), self.c64(0));
                        let (bt, be) = (block_bb(t)?, block_bb(e)?);
                        self.flush_edge(&c, &[start_of(t)?, start_of(e)?]);
                        LLVMBuildCondBr(self.b, x, bt, be);
                        open = false;
                    }
                    Inst::Case { v, table } => {
                        let t = prog
                            .cases
                            .get(table)
                            .ok_or_else(|| format!("no case table {}", table.0))?;
                        let x = self.ld(&c, v);
                        let mut to_all = vec![start_of(t.default)?];
                        for &(_, _, to) in &t.arms {
                            to_all.push(start_of(to)?);
                        }
                        self.flush_edge(&c, &to_all);
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
                    Inst::LCopy {
                        dst,
                        dst_off,
                        src,
                        src_off,
                        n,
                    } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let (k, so, d) =
                            (self.ld(&c, n), self.ld(&c, src_off), self.ld(&c, dst_off));
                        // Nothing to copy: no memory is touched, as in the interpreter.
                        let go = self.block(&c);
                        let z = self.icmp(IP::LLVMIntEQ, k, self.c64(0));
                        LLVMBuildCondBr(self.b, z, join, go);
                        LLVMPositionBuilderAtEnd(self.b, go);
                        let small = |v| self.icmp(IP::LLVMIntULT, v, self.c64(1 << 32));
                        let ok = LLVMBuildAnd(self.b, small(k), small(so), c"".as_ptr());
                        self.guard(&c, LLVMBuildAnd(self.b, ok, small(d), c"".as_ptr()), slow);
                        // First block, block count, first element in it.
                        let blocks = |off| {
                            let b0 = LLVMBuildLShr(self.b, off, self.c64(6), c"".as_ptr());
                            let last = LLVMBuildAdd(self.b, off, k, c"".as_ptr());
                            let last = LLVMBuildSub(self.b, last, self.c64(1), c"".as_ptr());
                            let b1 = LLVMBuildLShr(self.b, last, self.c64(6), c"".as_ptr());
                            let nb = LLVMBuildSub(self.b, b1, b0, c"".as_ptr());
                            let nb = LLVMBuildAdd(self.b, nb, self.c64(1), c"".as_ptr());
                            let first = LLVMBuildShl(self.b, b0, self.c64(6), c"".as_ptr());
                            (b0, nb, LLVMBuildSub(self.b, off, first, c"".as_ptr()))
                        };
                        let (sb0, ns, s0) = blocks(so);
                        let (db0, nd, d0) = blocks(d);
                        let max = self.c64(LCOPY_BLOCKS as i64);
                        let fit = LLVMBuildAnd(
                            self.b,
                            self.icmp(IP::LLVMIntULE, ns, max),
                            self.icmp(IP::LLVMIntULE, nd, max),
                            c"".as_ptr(),
                        );
                        self.guard(&c, fit, slow);
                        let b32 = |v| LLVMBuildShl(self.b, v, self.c64(5), c"".as_ptr());
                        let ps = self.host(
                            &c,
                            self.ld(&c, src),
                            fk(src),
                            b32(sb0),
                            b32(ns),
                            false,
                            slow,
                        );
                        let pd =
                            self.host(&c, self.ld(&c, dst), fk(dst), b32(db0), b32(nd), true, slow);
                        // Up to 64 elements: at most two blocks each side, shifted in registers.
                        let few = self.icmp(IP::LLVMIntULE, k, self.c64(64));
                        let (inl, call) = (self.block(&c), self.block(&c));
                        LLVMBuildCondBr(self.b, few, inl, call);
                        LLVMPositionBuilderAtEnd(self.b, inl);
                        self.lcopy_inline(&c, pd, nd, d0, ps, ns, s0, k);
                        LLVMBuildBr(self.b, join);
                        LLVMPositionBuilderAtEnd(self.b, call);
                        let mut args = [pd, nd, d0, ps, ns, s0, k];
                        LLVMBuildCall2(
                            self.b,
                            self.lcopy_ty,
                            self.lcopy,
                            args.as_mut_ptr(),
                            7,
                            c"".as_ptr(),
                        );
                        self.join(join);
                    }
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
                        let rs = f.regs(parts).ok_or("operand list out of range")?;
                        if !self.cat_block(&c, pc, &inst, dst, n, m, rs, scalars, bias, fk(dst)) {
                            self.step(&c, pc, &inst);
                        }
                    }
                    Inst::SigReadScalar { dst, sig } => self.sig_read_scalar(&c, pc, dst, sig),
                    Inst::Drive {
                        driver,
                        target: Some(t),
                        wave,
                        n,
                        mode,
                        reject: None,
                    } if c.nat.is_none() => self.drive_block(&c, pc, driver, t, wave, n, mode),
                    Inst::WaitOn { sig }
                        if c.nat.is_none()
                            && matches!(f.code.get(i + 1), Some(Inst::Suspend { .. })) =>
                    {
                        self.wait_suspend(&c, pc, sig)
                    }
                    Inst::SigRead { dst, sig }
                        if c.nat.is_none() && self.sig_read_block(&c, pc, dst, sig) => {}
                    Inst::SigRead { .. }
                    | Inst::WaitOn { .. }
                    | Inst::WaitFor { .. }
                    | Inst::DriveSlot { .. }
                    | Inst::Drive { .. }
                    | Inst::Suspend { .. } => self.signal(&c, pc, 0),
                    Inst::CallNative { native, args, dst } => {
                        match native_kind(prog, f, native, args, dst) {
                            Some(NativeKind::Table(t, rs, d)) => {
                                let (slow, join) = self.slow_path(&c, pc, &inst);
                                let mut idx = self.c64(0);
                                for r in rs {
                                    // A code beyond the nine is the native's error.
                                    let x = self.ld(&c, r);
                                    self.guard(&c, self.icmp(IP::LLVMIntULE, x, self.c64(8)), slow);
                                    let i9 = LLVMBuildMul(self.b, idx, self.c64(9), c"".as_ptr());
                                    idx = LLVMBuildAdd(self.b, i9, x, c"".as_ptr());
                                }
                                let p = self.gep(self.table(&t), idx);
                                self.st(&c, d, self.zext8(self.load(self.i8, p)));
                                self.join(join);
                            }
                            Some(NativeKind::Edge(rising)) => match (f.regs(args), dst) {
                                (Some(&[s]), Some(d)) => self.edge(&c, pc, rising, s, d),
                                _ => self.signal(&c, pc, if rising { 1 } else { 2 }),
                            },
                            None => self.step(&c, pc, &inst),
                        }
                    }
                    Inst::Call { func, args, dst }
                        if nc.contains(&func)
                            && prog.func(func).is_some_and(|g| call_fits(f, g, args)) =>
                    {
                        let g = prog.func(func).ok_or("callee vanished")?;
                        let rs = f.regs(args).ok_or("operand list out of range")?;
                        self.native_call(&c, pc, &inst, func, g, rs, dst);
                    }
                    _ => self.step(&c, pc, &inst),
                }
            }
            if open {
                // Falling off the end: the interpreter reports it.
                self.leave(&c, self.c32(n as u32));
            }
            if let Some(nat) = &c.nat {
                // Materialisation: record the registers, slots, pc and temporary base, and
                // return with `Mem::ndeopt` set.
                LLVMPositionBuilderAtEnd(self.b, exit);
                c.known.borrow_mut().fill(None);
                let mut args = [
                    c.env,
                    c.mem,
                    self.c32(nat.id.0),
                    exit_pc,
                    nat.temp_base,
                    self.c64(nregs as i64),
                    self.c64(i64::from(f.frame)),
                ];
                let rp = LLVMBuildCall2(
                    self.b,
                    self.deopt_ty,
                    self.deopt,
                    args.as_mut_ptr(),
                    7,
                    c"".as_ptr(),
                );
                for r in 0..nregs {
                    let v = self.ld(&c, Reg(r as u32));
                    self.store(v, self.gep(rp, self.c64(r as i64 * 8)));
                }
                let mut env = [c.env];
                let sp = LLVMBuildCall2(
                    self.b,
                    self.deopt_slots_ty,
                    self.deopt_slots,
                    env.as_mut_ptr(),
                    1,
                    c"".as_ptr(),
                );
                LLVMBuildMemCpy(self.b, sp, 1, nat.slots, 8, self.c64(i64::from(f.frame)));
                LLVMBuildRet(self.b, self.c64(0));
            }
            if LLVMVerifyFunction(fv, LLVMVerifierFailureAction::LLVMReturnStatusAction) != 0 {
                // A native function may have callers in the module already: the whole module
                // is generated again without native frames (`compile`).
                if !native {
                    LLVMDeleteFunction(fv);
                }
                return Err("the generated LLVM function does not verify (a bug in yavs)".into());
            }
            Ok(())
        }
    }

    /// Like `slow_path`, but the interpreter runs the signal instruction at `pc` through
    /// `TierHost::signal` (`edge` as in `signal`).
    unsafe fn slow_signal(
        &self,
        c: &Cur,
        pc: u32,
        edge: u32,
    ) -> (LLVMBasicBlockRef, LLVMBasicBlockRef) {
        // SAFETY: well-formed blocks of c.f.
        unsafe {
            let here = LLVMGetInsertBlock(self.b);
            let (slow, join) = (self.block(c), self.block(c));
            LLVMPositionBuilderAtEnd(self.b, slow);
            self.signal(c, pc, edge);
            LLVMBuildBr(self.b, join);
            LLVMPositionBuilderAtEnd(self.b, here);
            (slow, join)
        }
    }

    /// The `TierSig` entry of the instance signal that reference registers `sig`, `sig + 1`
    /// name, and the element offset in it; taking `slow` unless the reference is a plain one
    /// of `len` elements to an instance signal the table has.
    unsafe fn sig_entry(
        &self,
        c: &Cur,
        sig: Reg,
        len: Option<i64>,
        slow: LLVMBasicBlockRef,
    ) -> (LLVMValueRef, LLVMValueRef) {
        // SAFETY: loads of `Mem` fields and of a `TierSig` the guard checked is in the table.
        unsafe {
            let (v, l) = (self.ld(c, sig), self.ld(c, Reg(sig.0 + 1)));
            let base = LLVMBuildLShr(self.b, v, self.c64(32), c"".as_ptr());
            let off = LLVMBuildAnd(self.b, v, self.c64(0xffff_ffff), c"".as_ptr());
            let n = self.mem_ld(c, MEM_SIGTAB_LEN, self.i64);
            let has = self.icmp(IP::LLVMIntULT, base, n);
            let want = match len {
                Some(k) => self.icmp(IP::LLVMIntEQ, l, self.c64(k)),
                None => self.icmp(IP::LLVMIntNE, l, self.c64(0)),
            };
            self.guard(c, LLVMBuildAnd(self.b, has, want, c"".as_ptr()), slow);
            let tab = self.mem_ld(c, MEM_SIGTAB, self.ptr);
            let at = LLVMBuildMul(self.b, base, self.c64(SIG_ENT), c"".as_ptr());
            (self.gep(tab, at), off)
        }
    }

    /// `Inst::SigReadScalar` of a logic signal that keeps its codes: the byte, in place;
    /// anything else through `TierHost::signal`.
    unsafe fn sig_read_scalar(&self, c: &Cur, pc: u32, dst: Reg, sig: Reg) {
        // SAFETY: see `sig_entry`; the guard checks the element is within the codes.
        unsafe {
            let (slow, join) = self.slow_signal(c, pc, 0);
            let (ent, off) = self.sig_entry(c, sig, None, slow);
            let len = self.load(self.i64, self.gep(ent, self.c64(SIG_LEN)));
            self.guard(c, self.icmp(IP::LLVMIntULT, off, len), slow);
            let codes = self.tag(
                self.load(self.ptr, self.gep(ent, self.c64(SIG_CODES))),
                AA_CODES,
            );
            let byte = self.load(self.i8, self.gep(codes, off));
            self.st(c, dst, self.zext8(byte));
            self.join(join);
        }
    }

    /// `Inst::SigRead` of a plain reference of 1 to 64 elements (the count known) to a logic
    /// signal that keeps its codes: the planes built from the code bytes in place, eight
    /// elements at a time; anything else through `TierHost::signal`. `false`: not that case,
    /// nothing generated.
    unsafe fn sig_read_block(&self, c: &Cur, pc: u32, dst: Reg, sig: Reg) -> bool {
        let Some(l) = self
            .kint(c, Reg(sig.0 + 1))
            .filter(|l| (1..=64).contains(l))
        else {
            return false;
        };
        let groups = (l as u64).div_ceil(8);
        // SAFETY: see `sig_entry`; the guards check the groups lie within the codes.
        unsafe {
            let (slow, join) = self.slow_signal(c, pc, 0);
            let (ent, off) = self.sig_entry(c, sig, Some(l), slow);
            let tl = self.load(self.i64, self.gep(ent, self.c64(SIG_LEN)));
            let end = LLVMBuildAdd(self.b, off, self.c64(8 * groups as i64), c"".as_ptr());
            self.guard(c, self.icmp(IP::LLVMIntULE, end, tl), slow);
            let codes = self.tag(
                self.load(self.ptr, self.gep(ent, self.c64(SIG_CODES))),
                AA_CODES,
            );
            let base = self.gep(codes, off);
            let mut planes = [self.c64(0); 4];
            let ones = self.c64(0x0101_0101_0101_0101);
            let spread = self.c64(0x0102_0408_1020_4080);
            for j in 0..groups {
                let w = self.load(self.i64, self.gep(base, self.c64(8 * j as i64)));
                for (p, plane) in planes.iter_mut().enumerate() {
                    let bit = LLVMBuildLShr(self.b, w, self.c64(p as i64), c"".as_ptr());
                    let bit = LLVMBuildAnd(self.b, bit, ones, c"".as_ptr());
                    let bit = LLVMBuildMul(self.b, bit, spread, c"".as_ptr());
                    let bit = LLVMBuildLShr(self.b, bit, self.c64(56), c"".as_ptr());
                    let bit = LLVMBuildShl(self.b, bit, self.c64(8 * j as i64), c"".as_ptr());
                    *plane = LLVMBuildOr(self.b, *plane, bit, c"".as_ptr());
                }
            }
            let mask = if l == 64 { !0 } else { (1u64 << l) - 1 };
            let pd = self.host(
                c,
                self.ld(c, dst),
                self.known_addr(c, dst),
                self.c64(0),
                self.c64(32),
                true,
                slow,
            );
            for (p, plane) in planes.iter().enumerate() {
                let w = LLVMBuildAnd(self.b, *plane, self.c64(mask as i64), c"".as_ptr());
                self.store(w, self.gep(pd, self.c64(8 * p as i64)));
            }
            self.join(join);
        }
        true
    }

    /// `rising_edge` / `falling_edge` of one element: true in place when the kernel resumed the
    /// process for that edge (`TierFrame::known_sig`), else through `TierHost::signal`.
    unsafe fn edge(&self, c: &Cur, pc: u32, rising: bool, sig: Reg, dst: Reg) {
        // SAFETY: see `sig_entry`.
        unsafe {
            let (slow, join) = self.slow_signal(c, pc, if rising { 1 } else { 2 });
            let (ent, off) = self.sig_entry(c, sig, Some(1), slow);
            let signal = self.load(self.i64, self.gep(ent, self.c64(SIG_SIGNAL)));
            let first = self.load(self.i64, self.gep(ent, self.c64(SIG_OFF)));
            let elem = LLVMBuildAdd(self.b, first, off, c"".as_ptr());
            let held = |a: LLVMValueRef, b: LLVMValueRef| self.icmp(IP::LLVMIntEQ, a, b);
            let ks = self.mem_ld(c, MEM_KNOWN_SIG, self.i64);
            let ke = self.mem_ld(c, MEM_KNOWN_ELEM, self.i64);
            let ku = self.mem_ld(c, MEM_KNOWN_UP, self.i64);
            let ok = LLVMBuildAnd(
                self.b,
                held(signal, ks),
                LLVMBuildAnd(
                    self.b,
                    held(elem, ke),
                    held(ku, self.c64(i64::from(rising))),
                    c"".as_ptr(),
                ),
                c"".as_ptr(),
            );
            self.guard(c, ok, slow);
            self.st(c, dst, self.c64(1));
            self.join(join);
        }
    }

    /// Execute the instruction `i` at `pc` in the interpreter; return what `yavs_rt_step`
    /// returns unless it is `GO`.
    unsafe fn step(&self, c: &Cur, pc: u32, i: &Inst) {
        if c.nat.is_some() {
            // SAFETY: as `deopt_here`.
            return unsafe { self.deopt_here(c, pc) };
        }
        // Instructions that only change the temporary area or a register go to the interpreter's own
        // function for them (`TierHost::signal`), without its generic dispatch.
        if matches!(
            i,
            Inst::TempRoot { .. }
                | Inst::NewFrame { .. }
                | Inst::Stmt
                | Inst::Deref { .. }
                | Inst::MemEq { .. }
                | Inst::Cat { .. }
        ) {
            // SAFETY: as `signal`.
            return unsafe { self.signal(c, pc, 0) };
        }
        let at = if keeps_objects(i) { pc | KEEP } else { pc };
        // SAFETY: `step` has the run-function signature.
        unsafe {
            self.call_go(
                c,
                self.fn_ty,
                self.step,
                &mut [c.env, c.regs, c.mem, self.c32(at)],
            )
        }
    }

    /// `Inst::LCopy` of `k` (1 to 64) elements from bit `s0` of the one or two plane blocks at
    /// `ps` (`ns` of them) to bit `d0` of those at `pd` (`nd`), as `copy_bits` does: the
    /// source planes are read first (the ranges may overlap), the destination bits outside
    /// the copied range stay.
    #[allow(clippy::too_many_arguments)]
    unsafe fn lcopy_inline(
        &self,
        c: &Cur,
        pd: LLVMValueRef,
        nd: LLVMValueRef,
        d0: LLVMValueRef,
        ps: LLVMValueRef,
        ns: LLVMValueRef,
        s0: LLVMValueRef,
        k: LLVMValueRef,
    ) {
        // SAFETY: the pointers cover `ns` and `nd` blocks (`host`, checked by the caller).
        unsafe {
            let i128t = LLVMIntTypeInContext(self.ctx, 128);
            let b = self.b;
            let z128 = |v| LLVMBuildZExt(b, v, i128t, c"".as_ptr());
            let c128 = |v: u64| LLVMConstInt(i128t, v, 0);
            // Low `k` bits.
            let mask = {
                let full = self.icmp(IP::LLVMIntUGE, k, self.c64(64));
                let sh = LLVMBuildShl(b, self.c64(1), k, c"".as_ptr());
                let low = LLVMBuildSub(b, sh, self.c64(1), c"".as_ptr());
                LLVMBuildSelect(b, full, self.c64(-1), low, c"".as_ptr())
            };
            let two_s = self.icmp(IP::LLVMIntEQ, ns, self.c64(2));
            let two_d = self.icmp(IP::LLVMIntEQ, nd, self.c64(2));
            // Block 1 is only read (written) when there is one.
            let word = |p: LLVMValueRef, plane: i64, blk1: LLVMValueRef| {
                let lo = self.gep(p, self.c64(plane * 8));
                let hi_at = LLVMBuildSelect(
                    b,
                    blk1,
                    self.gep(p, self.c64(32 + plane * 8)),
                    lo,
                    c"".as_ptr(),
                );
                (lo, hi_at)
            };
            let mut vals = [self.c64(0); 4];
            for (plane, v) in vals.iter_mut().enumerate() {
                let (lo, hi_at) = word(ps, plane as i64, two_s);
                let (l, h) = (self.load(self.i64, lo), self.load(self.i64, hi_at));
                let h = LLVMBuildSelect(b, two_s, h, self.c64(0), c"".as_ptr());
                let wide = LLVMBuildOr(
                    b,
                    z128(l),
                    LLVMBuildShl(b, z128(h), c128(64), c"".as_ptr()),
                    c"".as_ptr(),
                );
                let sh = LLVMBuildLShr(b, wide, z128(s0), c"".as_ptr());
                let low = LLVMBuildTrunc(b, sh, self.i64, c"".as_ptr());
                *v = LLVMBuildAnd(b, low, mask, c"".as_ptr());
            }
            let hole = LLVMBuildShl(b, z128(mask), z128(d0), c"".as_ptr());
            let keep = LLVMBuildNot(b, hole, c"".as_ptr());
            for (plane, v) in vals.iter().enumerate() {
                let (lo, hi_at) = word(pd, plane as i64, two_d);
                let (l, h) = (self.load(self.i64, lo), self.load(self.i64, hi_at));
                let h = LLVMBuildSelect(b, two_d, h, self.c64(0), c"".as_ptr());
                let wide = LLVMBuildOr(
                    b,
                    z128(l),
                    LLVMBuildShl(b, z128(h), c128(64), c"".as_ptr()),
                    c"".as_ptr(),
                );
                let put = LLVMBuildShl(b, z128(*v), z128(d0), c"".as_ptr());
                let out = LLVMBuildOr(
                    b,
                    LLVMBuildAnd(b, wide, keep, c"".as_ptr()),
                    put,
                    c"".as_ptr(),
                );
                self.store(LLVMBuildTrunc(b, out, self.i64, c"".as_ptr()), lo);
                // The second block: the high half, when the destination has one.
                let (hi_bb, done) = (self.block(c), self.block(c));
                LLVMBuildCondBr(b, two_d, hi_bb, done);
                LLVMPositionBuilderAtEnd(b, hi_bb);
                let top = LLVMBuildLShr(b, out, c128(64), c"".as_ptr());
                self.store(LLVMBuildTrunc(b, top, self.i64, c"".as_ptr()), hi_at);
                LLVMBuildBr(b, done);
                LLVMPositionBuilderAtEnd(b, done);
            }
        }
    }

    /// A `Drive` of one waveform element to a single-range target through `yavs_rt_drive`
    /// (`TierHost::drive_block`: at most 64 logic elements assigned from the element's first
    /// plane block); the instruction through `yavs_rt_signal` unless `n` is 1 and the 40 bytes
    /// of the waveform are in memory compiled code reaches.
    #[allow(clippy::too_many_arguments)]
    unsafe fn drive_block(
        &self,
        c: &Cur,
        pc: u32,
        driver: u32,
        target: Reg,
        wave: Reg,
        n: Reg,
        mode: yavs_ir::DriveMode,
    ) {
        // SAFETY: the arguments match `drive_ty`; well-formed blocks of c.f.
        unsafe {
            let (slow, join) = self.slow_signal(c, pc, 0);
            self.guard(
                c,
                self.icmp(IP::LLVMIntEQ, self.ld(c, n), self.c64(1)),
                slow,
            );
            let w = self.host(
                c,
                self.ld(c, wave),
                None,
                self.c64(0),
                self.c64(40),
                false,
                slow,
            );
            let inertial = u32::from(mode == yavs_ir::DriveMode::Inertial);
            let mut args = [
                c.env,
                c.regs,
                c.mem,
                self.c32(pc),
                self.c32(driver),
                self.ld(c, target),
                self.ld(c, Reg(target.0 + 1)),
                w,
                self.c32(inertial),
            ];
            self.call_go(c, self.drive_ty, self.drive, &mut args);
            self.join(join);
        }
    }

    /// `Inst::Cat` of a logic array of at most 64 elements whose piece lengths the generator
    /// knows: the planes of all pieces combined in registers and stored once, as
    /// `Machine::cat_block` does; `false` (nothing generated) where a length is not known or
    /// does not fit that pattern. A runtime case it does not take (a code beyond 'U' to '-',
    /// memory compiled code does not reach) steps the whole instruction before any store.
    #[allow(clippy::too_many_arguments)]
    unsafe fn cat_block(
        &self,
        c: &Cur,
        pc: u32,
        inst: &Inst,
        dst: Reg,
        n: Reg,
        m: Reg,
        parts: &[Reg],
        scalars: u16,
        bias: u8,
        at: Option<Known>,
    ) -> bool {
        let (Some(n), Some(m)) = (self.kint(c, n), self.kint(c, m)) else {
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
                let Some(len) = self.kint(c, pr[1]).and_then(|l| u64::try_from(l).ok()) else {
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
        // SAFETY: loads through `host`, integer arithmetic, well-formed blocks of c.f.
        unsafe {
            let (slow, join) = self.slow_path(c, pc, inst);
            let mut words = [self.c64(0); 4];
            let mut mask = 0u64;
            let mut at_k = 0u64;
            for &(r, units, scalar) in &pieces {
                let bits: [LLVMValueRef; 4] = if scalar {
                    let code = LLVMBuildAdd(
                        self.b,
                        self.ld(c, r),
                        self.c64(i64::from(bias)),
                        c"".as_ptr(),
                    );
                    // A code beyond 'U' to '-' is the interpreter's error.
                    self.guard(c, self.icmp(IP::LLVMIntULE, code, self.c64(8)), slow);
                    std::array::from_fn(|p| {
                        let b = LLVMBuildLShr(self.b, code, self.c64(p as i64), c"".as_ptr());
                        LLVMBuildAnd(self.b, b, self.c64(1), c"".as_ptr())
                    })
                } else {
                    let ptr = self.host(
                        c,
                        self.ld(c, r),
                        self.known_addr(c, r),
                        self.c64(0),
                        self.c64(32),
                        false,
                        slow,
                    );
                    std::array::from_fn(|p| {
                        let w = self.load(self.i64, self.gep(ptr, self.c64(p as i64 * 8)));
                        let pm = if units == 64 {
                            !0u64
                        } else {
                            (1u64 << units) - 1
                        };
                        LLVMBuildAnd(self.b, w, self.c64(pm as i64), c"".as_ptr())
                    })
                };
                let off = total - at_k - units;
                let pm = if units == 64 {
                    !0u64
                } else {
                    (1u64 << units) - 1
                };
                for p in 0..4 {
                    let sh = LLVMBuildShl(self.b, bits[p], self.c64(off as i64), c"".as_ptr());
                    words[p] = LLVMBuildOr(self.b, words[p], sh, c"".as_ptr());
                }
                mask |= pm << off;
                at_k += units;
            }
            if mask == 0 {
                self.join(join);
                return true;
            }
            let d = self.ld(c, dst);
            let pd = self.host(c, d, at, self.c64(0), self.c64(32), true, slow);
            // A block just allocated (zeroed) and covered by the pieces needs no read.
            let fresh = matches!(at, Some(Known::Temp)) && at_k == total;
            for (p, &word) in words.iter().enumerate() {
                let pp = self.gep(pd, self.c64(p as i64 * 8));
                let v = if fresh {
                    word
                } else {
                    let cur = self.load(self.i64, pp);
                    let keep = LLVMBuildAnd(self.b, cur, self.c64(!mask as i64), c"".as_ptr());
                    LLVMBuildOr(self.b, keep, word, c"".as_ptr())
                };
                self.store(v, pp);
            }
            self.join(join);
        }
        true
    }

    /// A `WaitOn` and the `Suspend` after it through `yavs_rt_wait` (`TierHost::wait_suspend`).
    unsafe fn wait_suspend(&self, c: &Cur, pc: u32, sig: Reg) {
        // SAFETY: the arguments match `wait_ty`.
        unsafe {
            let mut args = [
                c.env,
                c.regs,
                c.mem,
                self.c32(pc),
                self.ld(c, sig),
                self.ld(c, Reg(sig.0 + 1)),
            ];
            self.call_go(c, self.wait_ty, self.wait, &mut args);
        }
    }

    /// The signal instruction at `pc` through `yavs_rt_signal` (`edge`: `TierHost::signal`).
    unsafe fn signal(&self, c: &Cur, pc: u32, edge: u32) {
        if c.nat.is_some() {
            // SAFETY: as `deopt_here`.
            return unsafe { self.deopt_here(c, pc) };
        }
        let mut args = [c.env, c.regs, c.mem, self.c32(pc), self.c32(edge)];
        // SAFETY: the arguments match `signal_ty`.
        unsafe { self.call_go(c, self.signal_ty, self.signal, &mut args) }
    }

    /// `Inst::Call` of `callee` (`g`) on a native frame (`docs/decisions.md` "Compiled frames"):
    /// one frame index deeper, within the depth limit and the native stack allowance, when it
    /// has native code; the call is stepped (frame mode) or the frame materialised otherwise.
    /// When the callee unwinds for materialisation, a native caller records itself at the call,
    /// a frame-mode caller pushes the recorded frames and leaves them to the interpreter.
    #[allow(clippy::too_many_arguments)]
    unsafe fn native_call(
        &self,
        c: &Cur,
        pc: u32,
        i: &Inst,
        callee: FuncId,
        g: &Func,
        args: &[Reg],
        dst: Option<Reg>,
    ) {
        // SAFETY: `Mem` fields, the callee's native type, well-formed blocks of c.f.
        unsafe {
            let (slow, join) = self.slow_path(c, pc, i);
            let depth = match &c.nat {
                Some(n) => n.depth,
                None => self.mem_ld(c, MEM_FRAME_IDX, self.i64),
            };
            let d1 = LLVMBuildAdd(self.b, depth, self.c64(1), c"".as_ptr());
            let max = self.mem_ld(c, MEM_MAX_DEPTH, self.i64);
            let and = |a, b| LLVMBuildAnd(self.b, a, b, c"".as_ptr());
            let mut ok = and(
                self.icmp(IP::LLVMIntULT, d1, max),
                self.icmp(IP::LLVMIntULT, d1, self.c64(1 << 29)),
            );
            let mut zero = [self.c32(0)];
            let fa = LLVMBuildCall2(
                self.b,
                self.frame_addr.0,
                self.frame_addr.1,
                zero.as_mut_ptr(),
                1,
                c"".as_ptr(),
            );
            let fa = LLVMBuildPtrToInt(self.b, fa, self.i64, c"".as_ptr());
            let lo = self.mem_ld(c, MEM_STACK_LO, self.i64);
            ok = and(ok, self.icmp(IP::LLVMIntUGT, fa, lo));
            let ty = self.native_ty(g);
            let fp = if self.direct {
                self.declare(&format!("f{}_n", callee.0), ty)
            } else {
                let tab = self.mem_ld(c, MEM_NTAB, self.ptr);
                let at = self.gep(tab, self.c64(i64::from(callee.0) * 8));
                let fp = LLVMBuildLoad2(self.b, self.ptr, at, c"".as_ptr());
                LLVMSetOrdering(
                    fp,
                    llvm_sys::LLVMAtomicOrdering::LLVMAtomicOrderingMonotonic,
                );
                ok = and(ok, LLVMBuildIsNotNull(self.b, fp, c"".as_ptr()));
                fp
            };
            self.guard(c, ok, slow);
            // A collection in the callee finds this frame's live references in the frame.
            self.flush(c, pc as usize);
            let mut cells = vec![c.env, c.mem, d1];
            for (r, p) in args.iter().zip(&g.params) {
                cells.push(self.ld(c, *r));
                if p.width == Width::Sig {
                    cells.push(self.ld(c, Reg(r.0 + 1)));
                }
            }
            let v = LLVMBuildCall2(
                self.b,
                ty,
                fp,
                cells.as_mut_ptr(),
                cells.len() as u32,
                c"".as_ptr(),
            );
            let nd = self.mem_ld(c, MEM_NDEOPT, self.i64);
            let (out, cont) = (self.block(c), self.block(c));
            LLVMBuildCondBr(self.b, self.icmp(IP::LLVMIntNE, nd, self.c64(0)), out, cont);
            LLVMPositionBuilderAtEnd(self.b, out);
            match &c.nat {
                Some(_) => self.leave(c, self.c32(pc)),
                None => {
                    self.flush(c, pc as usize);
                    let mut a = [c.env, c.mem, self.c32(pc)];
                    let r = LLVMBuildCall2(
                        self.b,
                        self.deopt_end_ty,
                        self.deopt_end,
                        a.as_mut_ptr(),
                        3,
                        c"".as_ptr(),
                    );
                    self.leave(c, r);
                }
            }
            LLVMPositionBuilderAtEnd(self.b, cont);
            if let Some(d) = dst {
                self.st(c, d, v);
            }
            self.join(join);
        }
    }

    /// Materialise a native frame at `pc` (its exit block); the builder goes on in a new block,
    /// which nothing reaches.
    unsafe fn deopt_here(&self, c: &Cur, pc: u32) {
        // SAFETY: well-formed blocks of c.f.
        unsafe {
            self.leave(c, self.c32(pc));
            let dead = self.block(c);
            LLVMPositionBuilderAtEnd(self.b, dead);
        }
    }

    /// The high half of the addresses of the function's own frame.
    unsafe fn own_hi(&self, c: &Cur) -> LLVMValueRef {
        match &c.nat {
            Some(n) => n.fhi,
            // SAFETY: a `Mem` field.
            None => unsafe { self.mem_ld(c, MEM_FRAME, self.i64) },
        }
    }

    /// The function's own slots.
    unsafe fn own_slots(&self, c: &Cur) -> LLVMValueRef {
        match &c.nat {
            Some(n) => n.slots,
            // SAFETY: a `Mem` field.
            None => unsafe { self.mem_ld(c, MEM_SLOTS, self.ptr) },
        }
    }

    /// Where a helper writes register `dst` (the registers and its index; a native frame's
    /// scratch word and 0); `reg_in` moves it into the register.
    unsafe fn reg_out(&self, c: &Cur, dst: Reg) -> (LLVMValueRef, u32) {
        match &c.nat {
            Some(n) => (n.scratch, 0),
            None => {
                // The helper writes the frame's register.
                if let Some(d) = &c.defer
                    && let Some(p) = d.pend.borrow_mut().get_mut(dst.0 as usize)
                {
                    *p = None;
                }
                (c.regs, dst.0)
            }
        }
    }

    unsafe fn reg_in(&self, c: &Cur, dst: Reg) {
        if let Some(n) = &c.nat {
            // SAFETY: the scratch word is an i64 `alloca`.
            unsafe {
                let v = LLVMBuildLoad2(self.b, self.i64, n.scratch, c"".as_ptr());
                self.st(c, dst, v);
            }
        }
    }

    /// `fast_op` of an operation that writes register `dst`.
    unsafe fn fast_reg(
        &self,
        c: &Cur,
        op: u32,
        dst: Reg,
        a: LLVMValueRef,
        b: LLVMValueRef,
        slow: LLVMBasicBlockRef,
    ) {
        // SAFETY: as `fast_op`.
        unsafe {
            let (_, d) = self.reg_out(c, dst);
            self.fast_op(c, op, d, a, b, slow);
            self.reg_in(c, dst);
        }
    }

    /// `yavs_rt_fast` operation `op` on `a` and `b` (result register or count `dst`), taking
    /// `slow` when it returns 0: the interpreter then executes the instruction.
    unsafe fn fast_op(
        &self,
        c: &Cur,
        op: u32,
        dst: u32,
        a: LLVMValueRef,
        b: LLVMValueRef,
        slow: LLVMBasicBlockRef,
    ) {
        // SAFETY: the arguments match `fast_ty`; well-formed blocks of c.f.
        unsafe {
            // A native frame's registers are not the helpers' to see.
            let rp = c.nat.as_ref().map_or(c.regs, |n| n.scratch);
            let mut args = [c.env, rp, c.mem, self.c32(op), self.c32(dst), a, b];
            let ok = LLVMBuildCall2(
                self.b,
                self.fast_ty,
                self.fast,
                args.as_mut_ptr(),
                7,
                c"".as_ptr(),
            );
            // Alloc writes the temporary area, the others only read memory besides the
            // register and the object cache.
            let other = match op {
                FAST_ALLOC => MODS,
                FAST_DEREF => 0,
                _ => REF,
            };
            self.attr(ok, true, "memory", mem_effects(RW, RW, other));
            self.attr(ok, true, "willreturn", 0);
            self.guard(c, self.icmp(IP::LLVMIntNE, ok, self.c32(0)), slow);
        }
    }

    /// Call a callback that returns `GO` or what the compiled function returns at once.
    unsafe fn call_go(&self, c: &Cur, ty: LLVMTypeRef, f: LLVMValueRef, args: &mut [LLVMValueRef]) {
        // SAFETY: `args` match `ty`; well-formed blocks of c.f.
        unsafe {
            // The callback reads the registers of its instruction from the frame.
            if let Some(d) = &c.defer {
                self.flush(c, d.pc.get() as usize);
            }
            let r = LLVMBuildCall2(
                self.b,
                ty,
                f,
                args.as_mut_ptr(),
                args.len() as u32,
                c"".as_ptr(),
            );
            let cont = self.block(c);
            let mut vals = [r];
            let mut from = [LLVMGetInsertBlock(self.b)];
            LLVMAddIncoming(c.exit_pc, vals.as_mut_ptr(), from.as_mut_ptr(), 1);
            LLVMBuildCondBr(
                self.b,
                self.icmp(IP::LLVMIntEQ, r, self.c32(GO)),
                cont,
                c.exit,
            );
            LLVMPositionBuilderAtEnd(self.b, cont);
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

    /// A block that steps the instruction `i` at `pc` in the interpreter and goes on at the
    /// returned join block: the slow path of a native instruction. The builder stays.
    unsafe fn slow_path(
        &self,
        c: &Cur,
        pc: u32,
        i: &Inst,
    ) -> (LLVMBasicBlockRef, LLVMBasicBlockRef) {
        // SAFETY: well-formed blocks of c.f.
        unsafe {
            let here = LLVMGetInsertBlock(self.b);
            let (slow, join) = (self.block(c), self.block(c));
            LLVMPositionBuilderAtEnd(self.b, slow);
            self.step(c, pc, i);
            LLVMBuildBr(self.b, join);
            LLVMPositionBuilderAtEnd(self.b, here);
            (slow, join)
        }
    }

    /// Go on in a new block when `ok`, else take `slow`.
    unsafe fn guard(&self, c: &Cur, ok: LLVMValueRef, slow: LLVMBasicBlockRef) {
        // SAFETY: well-formed blocks of c.f.
        unsafe {
            let go = self.block(c);
            LLVMBuildCondBr(self.b, ok, go, slow);
            LLVMPositionBuilderAtEnd(self.b, go);
        }
    }

    /// End the fast path at the slow path's join block.
    unsafe fn join(&self, join: LLVMBasicBlockRef) {
        // SAFETY: the builder is in a block of the function of `join`.
        unsafe {
            LLVMBuildBr(self.b, join);
            LLVMPositionBuilderAtEnd(self.b, join);
        }
    }

    /// A private constant array of bytes.
    unsafe fn table(&self, t: &[u8]) -> LLVMValueRef {
        // SAFETY: a global of our module with a matching initialiser.
        unsafe {
            let mut vals: Vec<LLVMValueRef> = t
                .iter()
                .map(|&b| LLVMConstInt(self.i8, u64::from(b), 0))
                .collect();
            let ty = LLVMArrayType2(self.i8, t.len() as u64);
            let g = LLVMAddGlobal(self.module, ty, c"".as_ptr());
            LLVMSetInitializer(
                g,
                LLVMConstArray2(self.i8, vals.as_mut_ptr(), t.len() as u64),
            );
            LLVMSetGlobalConstant(g, 1);
            LLVMSetLinkage(g, llvm_sys::LLVMLinkage::LLVMPrivateLinkage);
            g
        }
    }

    /// A private global copy of constant `k` and its length, made once per function; `None`
    /// for a constant the interpreter expands (compact forms) or one over 4 KiB.
    unsafe fn const_global(&self, c: &Cur, k: yavs_ir::ConstId) -> Option<(LLVMValueRef, u64)> {
        let yavs_ir::ConstData::Bytes(b) = c.prog.consts.get(k)? else {
            return None;
        };
        if b.len() > 4096 {
            return None;
        }
        let mut m = c.consts.borrow_mut();
        // SAFETY: a global of our module (`table`).
        let g = *m.entry(k).or_insert_with(|| unsafe { self.table(b) });
        Some((g, b.len() as u64))
    }

    unsafe fn zext8(&self, v: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: v is an i8.
        unsafe { LLVMBuildZExt(self.b, v, self.i64, c"".as_ptr()) }
    }

    unsafe fn mem_ptr(&self, c: &Cur, field: u64) -> LLVMValueRef {
        // SAFETY: `field` is an offset in `Mem`.
        unsafe { self.gep(c.mem, self.c64(field as i64)) }
    }

    unsafe fn mem_ld(&self, c: &Cur, field: u64, ty: LLVMTypeRef) -> LLVMValueRef {
        // SAFETY: the field at `field` has type `ty`.
        unsafe { LLVMBuildLoad2(self.b, ty, self.mem_ptr(c, field), c"".as_ptr()) }
    }

    unsafe fn mem_st(&self, c: &Cur, field: u64, v: LLVMValueRef) {
        // SAFETY: the field at `field` has the type of `v`.
        unsafe {
            LLVMBuildStore(self.b, v, self.mem_ptr(c, field));
        }
    }

    unsafe fn gep(&self, p: LLVMValueRef, off: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: byte offset from a pointer.
        unsafe {
            let mut idx = [off];
            let g = LLVMBuildGEP2(self.b, self.i8, p, idx.as_mut_ptr(), 1, c"".as_ptr());
            let k = self.kinds.borrow().get(&(p as usize)).copied();
            if let Some(k) = k {
                self.tag(g, k);
            }
            g
        }
    }

    /// Remember that `p` points into memory of kind `k` (`AA_*`).
    fn tag(&self, p: LLVMValueRef, k: usize) -> LLVMValueRef {
        self.kinds.borrow_mut().insert(p as usize, k);
        p
    }

    /// Alias scope metadata on the access `inst` through `p`, if its kind is known.
    unsafe fn scope(&self, inst: LLVMValueRef, p: LLVMValueRef) {
        let Some(k) = self.kinds.borrow().get(&(p as usize)).copied() else {
            return;
        };
        // SAFETY: metadata nodes of our context attached to an instruction of our module.
        unsafe {
            let mut aa = self.aa.borrow_mut();
            let n = aa.get_or_insert_with(|| {
                let md = |ops: &mut [LLVMMetadataRef]| {
                    LLVMMDNodeInContext2(self.ctx, ops.as_mut_ptr(), ops.len())
                };
                let name = |s: &str| LLVMMDStringInContext2(self.ctx, s.as_ptr().cast(), s.len());
                let domain = md(&mut [name("yavs memory")]);
                let names = ["slots", "temp", "inst", "const", "codes"];
                let scopes: Vec<LLVMMetadataRef> =
                    names.iter().map(|n| md(&mut [name(n), domain])).collect();
                let v = |m| LLVMMetadataAsValue(self.ctx, m);
                let lists = |ix: &[usize]| {
                    let mut ops: Vec<LLVMMetadataRef> = ix.iter().map(|&i| scopes[i]).collect();
                    v(md(&mut ops))
                };
                AaNodes {
                    scope: std::array::from_fn(|i| lists(&[i])),
                    others: std::array::from_fn(|i| {
                        let ix: Vec<usize> = (0..AA_N).filter(|&j| j != i).collect();
                        lists(&ix)
                    }),
                    k_scope: LLVMGetMDKindIDInContext(self.ctx, c"alias.scope".as_ptr(), 11),
                    k_noalias: LLVMGetMDKindIDInContext(self.ctx, c"noalias".as_ptr(), 7),
                }
            });
            LLVMSetMetadata(inst, n.k_scope, n.scope[k]);
            LLVMSetMetadata(inst, n.k_noalias, n.others[k]);
        }
    }

    /// An unaligned load.
    unsafe fn load(&self, ty: LLVMTypeRef, p: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: `p` points to a value of `ty`.
        unsafe {
            let v = LLVMBuildLoad2(self.b, ty, p, c"".as_ptr());
            LLVMSetAlignment(v, 1);
            self.scope(v, p);
            v
        }
    }

    /// An unaligned store.
    unsafe fn store(&self, v: LLVMValueRef, p: LLVMValueRef) {
        // SAFETY: `p` points to memory for a value of `v`'s type.
        unsafe {
            let s = LLVMBuildStore(self.b, v, p);
            LLVMSetAlignment(s, 1);
            self.scope(s, p);
        }
    }

    fn width_ty(&self, w: Width) -> LLVMTypeRef {
        match w {
            Width::B1 => self.i8,
            Width::B4 => self.i32,
            _ => self.i64,
        }
    }

    /// The host pointer to `n` bytes at address `a` plus `extra`, taking `slow` unless they
    /// lie in the frame's slots (inline when the generator knows the address), a constant it
    /// has a copy of, or memory `yavs_rt_host` finds: what the interpreter's `rd`/`wr` would
    /// find there without an error. `at` is what the generator knows about `a`: an offset in the
    /// frame's slots, or in a constant, whose bytes compiled code then reads from a copy of
    /// its own (constants never change).
    #[allow(clippy::too_many_arguments)]
    unsafe fn host(
        &self,
        c: &Cur,
        a: LLVMValueRef,
        at: Option<Known>,
        extra: LLVMValueRef,
        n: LLVMValueRef,
        write: bool,
        slow: LLVMBasicBlockRef,
    ) -> LLVMValueRef {
        // SAFETY: loads of `Mem` fields, integer arithmetic, well-formed blocks of c.f.
        unsafe {
            if let Some(Known::Const(k, o)) = at {
                match self.const_global(c, k) {
                    // A write to a constant is the interpreter's error.
                    Some((g, len)) if !write => {
                        let o = LLVMBuildAdd(self.b, self.c64(o as i64), extra, c"".as_ptr());
                        let len = self.c64(len as i64);
                        let fits = self.icmp(IP::LLVMIntULE, n, len);
                        let rest = LLVMBuildSub(self.b, len, n, c"".as_ptr());
                        let inside = self.icmp(IP::LLVMIntULE, o, rest);
                        self.guard(c, LLVMBuildAnd(self.b, fits, inside, c"".as_ptr()), slow);
                        return self.gep(self.tag(g, AA_CONST), o);
                    }
                    _ => {}
                }
            }
            if let Some(Known::Frame(o)) = at {
                // Within the frame whatever the frame: no check.
                if !LLVMIsAConstantInt(extra).is_null() && !LLVMIsAConstantInt(n).is_null() {
                    let (e, k) = (LLVMConstIntGetZExtValue(extra), LLVMConstIntGetZExtValue(n));
                    if o.checked_add(e)
                        .and_then(|x| x.checked_add(k))
                        .is_some_and(|end| end <= c.frame_len)
                    {
                        let at = self.c64((o + e) as i64);
                        return self.gep(self.tag(self.own_slots(c), AA_SLOTS), at);
                    }
                }
                // `extra` is below 2^63, the sum cannot wrap.
                let o = LLVMBuildAdd(self.b, self.c64(o as i64), extra, c"".as_ptr());
                let len = match &c.nat {
                    Some(_) => self.c64(c.frame_len as i64),
                    None => self.mem_ld(c, MEM_SLOTS_LEN, self.i64),
                };
                let fits = self.icmp(IP::LLVMIntULE, n, len);
                let rest = LLVMBuildSub(self.b, len, n, c"".as_ptr());
                let inside = self.icmp(IP::LLVMIntULE, o, rest);
                self.guard(c, LLVMBuildAnd(self.b, fits, inside, c"".as_ptr()), slow);
                return self.gep(self.tag(self.own_slots(c), AA_SLOTS), o);
            }
            // The temporary area and (reading) the instance data, whose space the generator
            // knows: the bytes the view holds, bounds checked, no call.
            let area = match at {
                Some(Known::Temp) => Some((MEM_TEMP, MEM_TEMP_LEN)),
                Some(Known::Inst(_)) if !write => Some((MEM_INST, MEM_INST_LEN)),
                _ => None,
            };
            if let Some((base, len)) = area {
                let lo = LLVMBuildAnd(self.b, a, self.c64(0xffff_ffff), c"".as_ptr());
                let o = LLVMBuildAdd(self.b, lo, extra, c"".as_ptr());
                let len = self.mem_ld(c, len, self.i64);
                let fits = self.icmp(IP::LLVMIntULE, n, len);
                let rest = LLVMBuildSub(self.b, len, n, c"".as_ptr());
                let inside = self.icmp(IP::LLVMIntULE, o, rest);
                self.guard(c, LLVMBuildAnd(self.b, fits, inside, c"".as_ptr()), slow);
                let kind = if base == MEM_TEMP { AA_TEMP } else { AA_INST };
                return self.gep(self.tag(self.mem_ld(c, base, self.ptr), kind), o);
            }
            // A native frame's own slots are found here: the interpreter has no frame for it.
            let (own, other, merge) = match &c.nat {
                Some(nat) => {
                    let (own, other, merge) = (self.block(c), self.block(c), self.block(c));
                    let hi = LLVMBuildLShr(self.b, a, self.c64(32), c"".as_ptr());
                    LLVMBuildCondBr(self.b, self.icmp(IP::LLVMIntEQ, hi, nat.fhi), own, other);
                    LLVMPositionBuilderAtEnd(self.b, own);
                    let lo = LLVMBuildAnd(self.b, a, self.c64(0xffff_ffff), c"".as_ptr());
                    let o = LLVMBuildAdd(self.b, lo, extra, c"".as_ptr());
                    let len = self.c64(c.frame_len as i64);
                    let fits = self.icmp(IP::LLVMIntULE, n, len);
                    let rest = LLVMBuildSub(self.b, len, n, c"".as_ptr());
                    let inside = self.icmp(IP::LLVMIntULE, o, rest);
                    self.guard(c, LLVMBuildAnd(self.b, fits, inside, c"".as_ptr()), slow);
                    let p = self.gep(nat.slots, o);
                    let from = LLVMGetInsertBlock(self.b);
                    LLVMBuildBr(self.b, merge);
                    LLVMPositionBuilderAtEnd(self.b, other);
                    (Some((p, from)), other, merge)
                }
                None => (None, std::ptr::null_mut(), std::ptr::null_mut()),
            };
            // Other memory through `yavs_rt_host` (`Mem::find`, then the objects the interpreter
            // is asked for) unless the address is in the frame, the temporary area or the
            // instance data: those the view holds.
            let hi = LLVMBuildLShr(self.b, a, self.c64(32), c"".as_ptr());
            let fhi = self.mem_ld(c, MEM_FRAME, self.i64);
            let isf = self.icmp(IP::LLVMIntEQ, hi, fhi);
            let ist = self.icmp(
                IP::LLVMIntEQ,
                hi,
                self.c64((u64::from(space::TEMP) << 29) as i64),
            );
            let mut in_view = self.or(isf, ist);
            let isi = self.icmp(
                IP::LLVMIntEQ,
                hi,
                self.c64((u64::from(space::INST) << 29) as i64),
            );
            if !write {
                in_view = self.or(in_view, isi);
            }
            let sel = |f: LLVMValueRef, t: LLVMValueRef, i: LLVMValueRef| {
                let ti = LLVMBuildSelect(self.b, ist, t, i, c"".as_ptr());
                LLVMBuildSelect(self.b, isf, f, ti, c"".as_ptr())
            };
            let base = sel(
                self.mem_ld(c, MEM_SLOTS, self.ptr),
                self.mem_ld(c, MEM_TEMP, self.ptr),
                self.mem_ld(c, MEM_INST, self.ptr),
            );
            let len = sel(
                self.mem_ld(c, MEM_SLOTS_LEN, self.i64),
                self.mem_ld(c, MEM_TEMP_LEN, self.i64),
                self.mem_ld(c, MEM_INST_LEN, self.i64),
            );
            let lo = LLVMBuildAnd(self.b, a, self.c64(0xffff_ffff), c"".as_ptr());
            let o = LLVMBuildAdd(self.b, lo, extra, c"".as_ptr());
            let fits = self.icmp(IP::LLVMIntULE, n, len);
            let rest = LLVMBuildSub(self.b, len, n, c"".as_ptr());
            let inside = self.icmp(IP::LLVMIntULE, o, rest);
            let hit = LLVMBuildAnd(
                self.b,
                in_view,
                LLVMBuildAnd(self.b, fits, inside, c"".as_ptr()),
                c"".as_ptr(),
            );
            let (fast, call, done) = (self.block(c), self.block(c), self.block(c));
            LLVMBuildCondBr(self.b, hit, fast, call);
            LLVMPositionBuilderAtEnd(self.b, fast);
            let pf = self.gep(base, o);
            LLVMBuildBr(self.b, done);
            LLVMPositionBuilderAtEnd(self.b, call);
            let mut args = [c.env, c.mem, a, extra, n, self.c32(u32::from(write))];
            let pc = LLVMBuildCall2(
                self.b,
                self.host_ty,
                self.host,
                args.as_mut_ptr(),
                6,
                c"".as_ptr(),
            );
            let call_end = LLVMGetInsertBlock(self.b);
            LLVMBuildBr(self.b, done);
            LLVMPositionBuilderAtEnd(self.b, done);
            let phi = LLVMBuildPhi(self.b, self.ptr, c"".as_ptr());
            let mut vals = [pf, pc];
            let mut from = [fast, call_end];
            LLVMAddIncoming(phi, vals.as_mut_ptr(), from.as_mut_ptr(), 2);
            self.guard(c, LLVMBuildIsNotNull(self.b, phi, c"".as_ptr()), slow);
            let mut p = phi;
            if let Some((po, from)) = own {
                let here = LLVMGetInsertBlock(self.b);
                LLVMBuildBr(self.b, merge);
                LLVMPositionBuilderAtEnd(self.b, merge);
                let phi = LLVMBuildPhi(self.b, self.ptr, c"".as_ptr());
                let mut vals = [po, p];
                let mut bbs = [from, here];
                LLVMAddIncoming(phi, vals.as_mut_ptr(), bbs.as_mut_ptr(), 2);
                p = phi;
            }
            let _ = other;
            p
        }
    }

    /// `n` zeroed bytes of temporary memory as `Machine::temp_alloc` makes them: the address
    /// and the host pointer, taking `slow` when the area must grow or exceeds 4 GiB.
    unsafe fn temp_alloc(
        &self,
        c: &Cur,
        n: LLVMValueRef,
        slow: LLVMBasicBlockRef,
    ) -> (LLVMValueRef, LLVMValueRef) {
        // SAFETY: integer arithmetic on `Mem` fields; the zeroed bytes lie within the capacity.
        unsafe {
            let len = self.mem_ld(c, MEM_TEMP_LEN, self.i64);
            let off = LLVMBuildAdd(self.b, len, self.c64(7), c"".as_ptr());
            let off = LLVMBuildAnd(self.b, off, self.c64(!7), c"".as_ptr());
            let (end, o) = self.ovf(self.uadd, off, n);
            let cap = self.mem_ld(c, MEM_TEMP_CAP, self.i64);
            let big = self.or(
                self.icmp(IP::LLVMIntUGT, end, self.c64(0xffff_ffff)),
                self.icmp(IP::LLVMIntUGT, end, cap),
            );
            self.guard(c, LLVMBuildNot(self.b, self.or(o, big), c"".as_ptr()), slow);
            let temp = self.tag(self.mem_ld(c, MEM_TEMP, self.ptr), AA_TEMP);
            let at = self.gep(temp, len);
            let fill = LLVMBuildSub(self.b, end, len, c"".as_ptr());
            LLVMBuildMemSet(self.b, at, LLVMConstInt(self.i8, 0, 0), fill, 1);
            self.mem_st(c, MEM_TEMP_LEN, end);
            let tag = self.c64((u64::from(space::TEMP) << 61) as i64);
            (
                LLVMBuildOr(self.b, tag, off, c"".as_ptr()),
                self.gep(temp, off),
            )
        }
    }

    /// The words `LayoutOf` writes for a layout known up to the descriptor's lengths,
    /// taking `slow` wherever `Ty::layout` would fail.
    unsafe fn layout_words(
        &self,
        c: &Cur,
        sl: &StaticLayout,
        desc: Reg,
        at: Option<Known>,
        slow: LLVMBasicBlockRef,
    ) -> Vec<LLVMValueRef> {
        // SAFETY: integer arithmetic on descriptor words read through `host`.
        unsafe {
            let dims = match sl {
                StaticLayout::Words(_) => 0,
                StaticLayout::Array { dims, .. } => *dims,
            };
            // The interpreter reads the descriptor even when it has no bytes.
            let n = self.c64(i64::from(dims) * 24);
            let d = self.ld(c, desc);
            let p = self.host(c, d, at, self.c64(0), n, false, slow);
            let (el, logic) = match sl {
                StaticLayout::Words(w) => return w.iter().map(|&x| self.c64(x as i64)).collect(),
                StaticLayout::Array { elem, logic, .. } => (elem, *logic),
            };
            let not = |x| LLVMBuildNot(self.b, x, c"".as_ptr());
            let and = |a, b| LLVMBuildAnd(self.b, a, b, c"".as_ptr());
            let mut count = self.c64(1);
            let mut ok = LLVMConstInt(LLVMInt1TypeInContext(self.ctx), 1, 0);
            for k in 0..i64::from(dims) {
                let (l, r, dir) = self.dope_words(self.gep(p, self.c64(k * 24)));
                let dir_ok = self.icmp(IP::LLVMIntULE, dir, self.c64(1));
                let (len, fits) = self.len_of(l, r, dir);
                let (m, o) = self.ovf(self.umul, count, len);
                count = m;
                ok = and(ok, and(and(dir_ok, fits), not(o)));
            }
            let (count, size, align, stride) = if logic {
                let (n, o) = self.ovf(self.umul, count, self.c64(el.count as i64));
                ok = and(ok, not(o));
                // div_ceil(n, 64) * 32 cannot overflow.
                let blocks = LLVMBuildLShr(self.b, n, self.c64(6), c"".as_ptr());
                let rest = LLVMBuildAnd(self.b, n, self.c64(63), c"".as_ptr());
                let part = self.icmp(IP::LLVMIntNE, rest, self.c64(0));
                let blocks = LLVMBuildAdd(self.b, blocks, self.zext(part), c"".as_ptr());
                let size = LLVMBuildShl(self.b, blocks, self.c64(5), c"".as_ptr());
                (n, size, self.c64(8), self.c64(0))
            } else {
                let stride = el.size.next_multiple_of(el.align.max(1));
                let (size, o) = self.ovf(self.umul, self.c64(stride as i64), count);
                ok = and(ok, not(o));
                (
                    count,
                    size,
                    self.c64(el.align as i64),
                    self.c64(stride as i64),
                )
            };
            self.guard(c, ok, slow);
            vec![size, align, stride, count]
        }
    }

    /// `yavs_ir::dim_len(l, r, dir == 1)` and whether it is not `None`.
    unsafe fn len_of(
        &self,
        l: LLVMValueRef,
        r: LLVMValueRef,
        dir: LLVMValueRef,
    ) -> (LLVMValueRef, LLVMValueRef) {
        // SAFETY: integer arithmetic.
        unsafe {
            let down = self.icmp(IP::LLVMIntEQ, dir, self.c64(1));
            let lo = LLVMBuildSelect(self.b, down, r, l, c"".as_ptr());
            let hi = LLVMBuildSelect(self.b, down, l, r, c"".as_ptr());
            let empty = self.icmp(IP::LLVMIntSLT, hi, lo);
            // hi - lo is exact as a u64 when hi >= lo; + 1 wraps only for the full i64 range.
            let d = LLVMBuildSub(self.b, hi, lo, c"".as_ptr());
            let n = LLVMBuildAdd(self.b, d, self.c64(1), c"".as_ptr());
            let fits = self.or(empty, self.icmp(IP::LLVMIntNE, n, self.c64(0)));
            (
                LLVMBuildSelect(self.b, empty, self.c64(0), n, c"".as_ptr()),
                fits,
            )
        }
    }

    /// The 32-byte bit-plane block holding element `idx` of the logic array at `base`, and
    /// the element's bit position in each plane word.
    #[allow(clippy::too_many_arguments)]
    unsafe fn logic_word(
        &self,
        c: &Cur,
        base: Reg,
        at: Option<Known>,
        idx: Reg,
        write: bool,
        slow: LLVMBasicBlockRef,
    ) -> (LLVMValueRef, LLVMValueRef) {
        // SAFETY: integer arithmetic and `host`.
        unsafe {
            let i = self.ld(c, idx);
            let blk = LLVMBuildLShr(self.b, i, self.c64(6), c"".as_ptr());
            let off = LLVMBuildShl(self.b, blk, self.c64(5), c"".as_ptr());
            let p = self.host(c, self.ld(c, base), at, off, self.c64(32), write, slow);
            (p, LLVMBuildAnd(self.b, i, self.c64(63), c"".as_ptr()))
        }
    }

    /// `left, right, dir` of the descriptor dimension at `p`.
    unsafe fn dope_words(&self, p: LLVMValueRef) -> (LLVMValueRef, LLVMValueRef, LLVMValueRef) {
        // SAFETY: `p` points to 24 readable bytes.
        unsafe {
            (
                self.load(self.i64, p),
                self.load(self.i64, self.gep(p, self.c64(8))),
                self.load(self.i64, self.gep(p, self.c64(16))),
            )
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The helper declarations carry the effects `Gen::effects` means (the encoding of
    /// `memory(...)` is LLVM's own, checked here against its printer).
    #[test]
    fn helper_effects() {
        crate::lazy::ensure().unwrap();
        let m = Module::new("effects");
        let g = Gen::new(&m);
        // SAFETY: our module; the string is disposed after the copy.
        let ir = unsafe {
            let p = LLVMPrintModuleToString(m.raw());
            let s = std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned();
            LLVMDisposeMessage(p);
            s
        };
        drop(g);
        let decl = |name: &str| {
            ir.lines()
                .find(|l| l.starts_with("declare") && l.contains(name))
                .unwrap_or_else(|| panic!("no {name} in {ir}"))
                .to_string()
        };
        let group = |name: &str| {
            let d = decl(name);
            let id = d.rsplit('#').next().unwrap_or("").trim().to_string();
            ir.lines()
                .find(|l| l.starts_with(&format!("attributes #{id} ")))
                .unwrap_or_else(|| panic!("no attributes for {d}"))
                .to_string()
        };
        let host = group(HOST);
        assert!(
            host.contains("memory(argmem: readwrite, inaccessiblemem: readwrite)"),
            "{host}"
        );
        assert!(
            host.contains("nounwind") && host.contains("willreturn"),
            "{host}"
        );
        let lcopy = group(LCOPY);
        assert!(lcopy.contains("memory(argmem: readwrite)"), "{lcopy}");
        assert!(group(DEOPT_SLOTS).contains("memory(argmem: read)"));
        let step = group(STEP);
        assert!(
            step.contains("nounwind") && !step.contains("memory"),
            "{step}"
        );
    }
}
