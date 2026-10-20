//! `yavs-ir` to LLVM: the compiled tier (ir.md 5, `docs/decisions.md` "LLVM tier: side exits").
//!
//! One LLVM function per IR function, `fn(env, regs, mem, pc) -> u32`, working on the
//! registers of the interpreter frame and, through [`Mem`], on the frame's slots, the
//! temporary area and the instance data. It enters at `pc` (a switch over the block starts
//! and the instructions after an exit), runs register arithmetic, compares, checks,
//! branches, loads, stores, copies, temporary allocation and descriptor reads itself,
//! executes every other instruction that always continues with the next one through the
//! `yavs_rt_step` callback (the interpreter's own code), and returns the index of the
//! instruction the interpreter executes next: a call, return, wait, report, the slow or error
//! path of an inline instruction (re-executed by the interpreter, so messages are identical),
//! or a callback fault. A memory access outside the three areas or out of bounds is such a
//! slow path: compiled code never reports an error itself (rule 5).

use std::any::Any;
use std::ffi::{CString, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Mutex;

use llvm_sys::analysis::{LLVMVerifierFailureAction, LLVMVerifyFunction};
use llvm_sys::core::*;
use llvm_sys::prelude::*;
use llvm_sys::{LLVMIntPredicate as IP, LLVMRealPredicate as RP};
use yavs_interp::{Tier, TierFrame, TierHost, space};
use yavs_ir::{
    AddrSpace, BoolOp, CheckKind, CmpOp, FOp, FUnOp, Func, FuncId, FuncKind, Inst, IntOp, IntUnOp,
    Program, Reg, Width,
};

use crate::{Error, Jit, Module, OptLevel};

const STEP: &str = "yavs_rt_step";
const OBJ: &str = "yavs_rt_obj";

pub(crate) type RunFn = unsafe extern "C" fn(*mut c_void, *mut u64, *mut Mem, u32) -> u32;

/// The memory compiled code accesses itself, as host pointers into the [`TierFrame`]
/// (field offsets: `MEM_*`). The temporary area's length lives here while compiled code
/// runs; it is written back to the vector before every step and at the end.
#[repr(C)]
pub(crate) struct Mem {
    slots: *mut u8,
    slots_len: u64,
    /// The high half of the frame's addresses: `space::FRAME << 29 | frame`.
    frame: u64,
    temp: *mut u8,
    temp_len: u64,
    temp_cap: u64,
    temp_base: u64,
    inst: *const u8,
    inst_len: u64,
    /// The last objects `yavs_rt_obj` found, valid until a step; `obj_next` is the entry it
    /// replaces next.
    objs: [Obj; OBJS],
    obj_next: u64,
    /// `TierFrame::temp_roots`: while non-zero, `Inst::Stmt` steps (the interpreter drops
    /// the ending statement's recorded temporaries).
    temp_roots: u64,
}

/// An object in `Mem::objs`: the high half of its addresses (`u64::MAX`: none), start,
/// length and whether it was found for writing.
#[derive(Clone, Copy)]
#[repr(C)]
struct Obj {
    hi: u64,
    ptr: *mut u8,
    len: u64,
    w: u64,
}

const OBJS: usize = 4;

const MEM_SLOTS: u64 = 0;
const MEM_SLOTS_LEN: u64 = 8;
const MEM_FRAME: u64 = 16;
const MEM_TEMP: u64 = 24;
const MEM_TEMP_LEN: u64 = 32;
const MEM_TEMP_CAP: u64 = 40;
const MEM_TEMP_BASE: u64 = 48;
const MEM_INST: u64 = 56;
const MEM_INST_LEN: u64 = 64;
/// `Mem::objs[k]` is at `MEM_OBJS + 32 * k`, its fields at 0, 8, 16, 24.
const MEM_OBJS: u64 = 72;
const MEM_TEMP_ROOTS: u64 = MEM_OBJS + 32 * OBJS as u64 + 8;
const _: () = assert!(std::mem::offset_of!(Mem, temp_roots) as u64 == MEM_TEMP_ROOTS);

impl Mem {
    fn of(fr: &mut TierFrame) -> Mem {
        Mem {
            slots: fr.slots_mut().as_mut_ptr(),
            slots_len: fr.slots_len as u64,
            frame: u64::from(space::FRAME) << 29 | u64::from(fr.frame),
            temp: fr.temp.as_mut_ptr(),
            temp_len: fr.temp.len() as u64,
            temp_cap: fr.temp.capacity() as u64,
            temp_base: fr.temp_base as u64,
            inst: fr.inst as *const u8,
            inst_len: fr.inst.len() as u64,
            objs: [Obj {
                hi: u64::MAX,
                ptr: std::ptr::null_mut(),
                len: 0,
                w: 0,
            }; OBJS],
            obj_next: 0,
            temp_roots: fr.temp_roots as u64,
        }
    }

    /// Give the temporary area the length compiled code left it.
    fn sync(&self, fr: &mut TierFrame) {
        // SAFETY: compiled code only shrinks the length or grows it within the capacity
        // after zeroing the new bytes (`Inst::Alloc`), so they are initialised.
        unsafe { fr.temp.set_len(self.temp_len as usize) }
    }
}

/// What a running compiled function reaches through `env`.
struct Env<'a> {
    fr: *mut TierFrame,
    host: &'a mut dyn TierHost,
    panic: Option<Box<dyn Any + Send>>,
}

/// Executes one instruction in the interpreter: 1 to go on, 0 after a fault.
extern "C" fn yavs_rt_step(env: *mut c_void, _regs: *mut u64, mem: *mut Mem, pc: u32) -> u32 {
    // SAFETY: `env` and `mem` are what `LlvmTier::run` passed to the compiled function, which
    // is blocked in this call; `regs` is `env.fr.regs`'s buffer, which the compiled code does
    // not touch until we return (it passes it only so LLVM reloads registers after the call).
    let env = unsafe { &mut *(env as *mut Env<'_>) };
    let fr = unsafe { &mut *env.fr };
    let mem = unsafe { &mut *mem };
    mem.sync(fr);
    // No unwinding through JIT frames: a panic is carried over the call and resumed after.
    let r = match catch_unwind(AssertUnwindSafe(|| env.host.step(fr, pc))) {
        Ok(ok) => ok.into(),
        Err(p) => {
            env.panic = Some(p);
            0
        }
    };
    let objs = mem.objs;
    *mem = Mem::of(fr);
    // Constants never move or change: their entries stay valid.
    for o in objs {
        if o.hi >> 29 == u64::from(space::CONST) {
            mem.objs[mem.obj_next as usize % OBJS] = o;
            mem.obj_next += 1;
        }
    }
    r
}

/// Finds the object `addr` points into for `Mem::obj`: 1 when found, 0 when the access is
/// the interpreter's (stepped).
extern "C" fn yavs_rt_obj(env: *mut c_void, mem: *mut Mem, addr: u64, write: u32) -> u32 {
    // SAFETY: as in `yavs_rt_step`.
    let env = unsafe { &mut *(env as *mut Env<'_>) };
    let mem = unsafe { &mut *mem };
    match catch_unwind(AssertUnwindSafe(|| env.host.object(addr, write != 0))) {
        Ok(Some((p, len))) => {
            mem.objs[mem.obj_next as usize % OBJS] = Obj {
                hi: addr >> 32,
                ptr: p,
                len: len as u64,
                w: u64::from(write),
            };
            mem.obj_next = mem.obj_next.wrapping_add(1);
            1
        }
        Ok(None) => 0,
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
        fr: &mut TierFrame,
        host: &mut dyn TierHost,
    ) -> Option<u32> {
        let &(f, nregs) = self.funcs.get(func.0 as usize)?.as_ref()?;
        enter(f, nregs, pc, fr, host)
    }

    fn ready(&self, func: FuncId) -> bool {
        self.funcs.get(func.0 as usize).is_some_and(Option::is_some)
    }
}

/// Run compiled `f` (generated for `nregs` registers) from `pc` (`Tier::run`).
pub(crate) fn enter(
    f: RunFn,
    nregs: usize,
    pc: u32,
    fr: &mut TierFrame,
    host: &mut dyn TierHost,
) -> Option<u32> {
    {
        // A frame index beyond the address format is the interpreter's error.
        if fr.regs_mut().len() != nregs || fr.frame >= 1 << 29 {
            return None;
        }
        let rp = fr.regs_mut().as_mut_ptr();
        let mut mem = Mem::of(fr);
        let mut env = Env {
            fr,
            host,
            panic: None,
        };
        // SAFETY: `f` was generated for this function with `nregs` registers and only
        // indexes `regs` below that (checked at compile time); it accesses memory only
        // within the bounds in `mem`; `env` and `mem` outlive the call.
        let next = unsafe { f(&mut env as *mut Env<'_> as *mut c_void, rp, &mut mem, pc) };
        // SAFETY: `env.fr` is `fr`, borrowed for this call.
        mem.sync(unsafe { &mut *env.fr });
        if let Some(p) = env.panic {
            resume_unwind(p);
        }
        Some(next)
    }
}

/// A JIT with the runtime callbacks compiled code calls.
pub(crate) fn new_jit(opt: OptLevel) -> Result<Jit, Error> {
    Jit::new(
        opt,
        &[
            (STEP, yavs_rt_step as *const () as usize),
            (OBJ, yavs_rt_obj as *const () as usize),
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
) -> Result<(RunFn, usize), String> {
    if let Some(r) = bad_reg(f) {
        return Err(format!("register r{} beyond the frame", r.0));
    }
    let name = format!("f{}", id.0);
    let mut m = Module::new(&name);
    // SAFETY: generation of well-formed IR in our own module; a function that fails the
    // verifier is deleted again.
    unsafe { Gen::new(&m).func(prog, f, &name) }?;
    let llvm = |e: Error| format!("LLVM: {e} (a bug in yavs)");
    m.verify().map_err(llvm)?;
    m.optimize(opt).map_err(llvm)?;
    jit.add(m).map_err(llvm)?;
    let addr = jit.lookup(&name).map_err(llvm)?;
    // SAFETY: `name` was generated by `Gen::func` with the `RunFn` signature.
    let run: RunFn = unsafe { std::mem::transmute::<usize, RunFn>(addr) };
    Ok((run, f.reg_ty.len()))
}

/// Compile every function of `prog` that the tier can take; the others (among them those over
/// `max_size` IR instructions) stay interpreted and are listed in [`LlvmTier::notes`].
pub fn compile(prog: &Program, opt: OptLevel, max_size: usize) -> Result<LlvmTier, Error> {
    let mut m = Module::new("yavs");
    let mut notes = Vec::new();
    let mut names = Vec::new();
    {
        let g = Gen::new(&m);
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
    let mut jit = new_jit(opt)?;
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
        notes,
        interpreted,
        inline,
    })
}

/// A register operand outside the function's register file, if any.
fn bad_reg(f: &Func) -> Option<Reg> {
    let n = f.reg_ty.len() as u32;
    f.code
        .iter()
        .flat_map(|i| inline_regs(f, i))
        .find(|r| r.0 >= n)
}

/// The registers an inline instruction touches (the compiled code indexes `regs` with them);
/// `Reg(u32::MAX)` for an operand list outside the function.
fn inline_regs(f: &Func, i: &Inst) -> Vec<Reg> {
    let pair = |r: Reg, w: Width| {
        if w == Width::Sig {
            vec![r, Reg(r.0.saturating_add(1))]
        } else {
            vec![r]
        }
    };
    match *i {
        Inst::AddrOf { dst, .. } => vec![dst],
        Inst::Alloc { dst, size } => vec![dst, size],
        Inst::Load { dst, addr, w } => [vec![addr], pair(dst, w)].concat(),
        Inst::Store { addr, src, w } => [vec![addr], pair(src, w)].concat(),
        Inst::Copy { dst, src, bytes } => vec![dst, src, bytes],
        Inst::Pos { dst, dope, idx, .. } => vec![dst, dope, idx],
        Inst::Field {
            dst, base, layout, ..
        } => vec![dst, base, layout],
        Inst::DopeLen { dst, dope, .. } => vec![dst, dope],
        Inst::LGet { dst, base, idx } => vec![dst, base, idx],
        Inst::LSet { base, idx, v } => vec![base, idx, v],
        Inst::SubtypeCheck { dope, want, .. } => vec![dope, want],
        Inst::MakeDope { at, parts } => match f.regs(parts) {
            Some(rs) => [&[at], rs].concat(),
            None => vec![Reg(u32::MAX)],
        },
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
fn is_inline(prog: &Program, f: &Func, i: &Inst) -> bool {
    match i {
        Inst::LayoutOf { ty, .. } => static_layout(prog, *ty).is_some(),
        Inst::Bin { op, .. } | Inst::BinRange { op, .. } => *op != IntOp::Pow,
        Inst::FBin { op, .. } => !matches!(op, FOp::Pow | FOp::PowI),
        Inst::AddrOf { space, .. } => addr_const(*space, 0).is_some(),
        _ => !inline_regs(f, i).is_empty() || matches!(i, Inst::Br { .. } | Inst::Stmt),
    }
}

/// The address `AddrOf` yields in `space`, without the frame index for `Frame`; `None` where
/// the interpreter computes it (static links, an index beyond the address format).
fn addr_const(space: AddrSpace, off: u32) -> Option<u64> {
    let (sp, idx) = match space {
        AddrSpace::Frame => (space::FRAME, 0),
        AddrSpace::Up(_) => return None,
        AddrSpace::Inst => (space::INST, 0),
        AddrSpace::Pkg(p) => (space::PKG, p),
        AddrSpace::Const(c) => (space::CONST, c.0),
        AddrSpace::Shared => (space::SHARED, 0),
    };
    (idx < 1 << 29).then(|| u64::from(sp) << 61 | u64::from(idx) << 32 | u64::from(off))
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

/// A `LayoutOf` result the generator can build: fixed words, or an array whose element
/// layout is fixed, so only the descriptor's lengths are read at run time.
enum StaticLayout {
    Words(Vec<u64>),
    Array {
        dims: u8,
        elem: yavs_ir::Layout,
        logic: bool,
    },
}

/// The `LayoutOf` of `ty` as far as it does not depend on the descriptor, as `Ty::layout`
/// computes it; `None` where the interpreter computes it.
fn static_layout(prog: &Program, ty: yavs_ir::TyId) -> Option<StaticLayout> {
    use yavs_ir::Ty;
    let (types, consts) = (&prog.types, &prog.consts);
    let words = |l: yavs_ir::Layout| {
        let mut w = vec![l.size, l.align];
        if l.offsets.is_empty() {
            w.extend([l.stride, l.count]);
        } else {
            w.extend(&l.offsets);
        }
        StaticLayout::Words(w)
    };
    if Ty::desc_size(types, ty)? == 0 {
        return Ty::layout(types, consts, ty, &[])
            .ok()
            .map(|(l, _)| words(l));
    }
    let Some(Ty::Array {
        elem,
        elem_desc,
        dims,
    }) = types.get(ty)
    else {
        return None;
    };
    let el = match elem_desc {
        Some(c) => match consts.get(*c)? {
            yavs_ir::ConstData::Bytes(b) => Ty::layout(types, consts, *elem, b).ok()?.0,
            _ => return None,
        },
        None if Ty::desc_size(types, *elem)? == 0 => Ty::layout(types, consts, *elem, &[]).ok()?.0,
        None => return None,
    };
    // A scalar element has its element size (`Ty::elem_size`), as in `Ty::layout`.
    let el = match types.get(*elem).and_then(Ty::elem_size) {
        Some(n) => yavs_ir::Layout {
            size: n,
            align: n,
            ..el
        },
        None => el,
    };
    Some(StaticLayout::Array {
        dims: *dims,
        elem: el,
        logic: Ty::logic_leaf(types, ty).is_some(),
    })
}

/// What the generator knows about a register's value within a basic block.
#[derive(Clone, Copy)]
enum Known {
    Int(i64),
    /// The address of byte `.0` of the frame's slots.
    Frame(u64),
}

/// Update `known` for what `i` writes.
fn track(known: &mut [Option<Known>], i: &Inst) {
    let get = |r: Reg| known.get(r.0 as usize).copied().flatten();
    let new = match *i {
        Inst::Const { dst, v } => Some((dst, Known::Int(v))),
        Inst::AddrOf {
            dst,
            space: AddrSpace::Frame,
            off,
        } => Some((dst, Known::Frame(off.into()))),
        Inst::Move { dst, src } => get(src).map(|k| (dst, k)),
        // As the interpreter computes it when it is no error.
        Inst::Index {
            dst,
            base,
            pos,
            stride,
        } => match (get(base), get(pos)) {
            (Some(Known::Frame(o)), Some(Known::Int(p))) => (p as u64)
                .checked_mul(stride.into())
                .and_then(|d| o.checked_add(d))
                .filter(|&a| a <= u64::from(u32::MAX))
                .map(|a| (dst, Known::Frame(a))),
            _ => None,
        },
        _ => None,
    };
    i.visit(&mut |r, role, _| {
        if role == yavs_ir::Role::Def {
            for k in [r.0, r.0.saturating_add(1)] {
                if let Some(x) = known.get_mut(k as usize) {
                    *x = None;
                }
            }
        }
    });
    if let Some((r, k)) = new
        && let Some(x) = known.get_mut(r.0 as usize)
    {
        *x = Some(k);
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
    obj_ty: LLVMTypeRef,
    obj: LLVMValueRef,
    /// `host(env, mem, addr, extra, n) -> ptr`, null for the slow path: reading, writing.
    host_ty: LLVMTypeRef,
    host_fns: [LLVMValueRef; 2],
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
    mem: LLVMValueRef,
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
            let obj_name = CString::new(OBJ).expect("no NUL");
            let mut oparams = [ptr, ptr, i64, i32];
            let obj_ty = LLVMFunctionType(i32, oparams.as_mut_ptr(), 4, 0);
            let mut hparams = [ptr, ptr, i64, i64, i64];
            let host_ty = LLVMFunctionType(ptr, hparams.as_mut_ptr(), 5, 0);
            let mut g = Gen {
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
                obj_ty,
                obj: LLVMAddFunction(module, obj_name.as_ptr(), obj_ty),
                host_ty,
                host_fns: [std::ptr::null_mut(); 2],
                sadd: intrinsic(c"llvm.sadd.with.overflow"),
                ssub: intrinsic(c"llvm.ssub.with.overflow"),
                smul: intrinsic(c"llvm.smul.with.overflow"),
                umul: intrinsic(c"llvm.umul.with.overflow"),
                uadd: intrinsic(c"llvm.uadd.with.overflow"),
            };
            g.host_fns = [g.host_fn(false), g.host_fn(true)];
            g
        }
    }

    /// The out-of-line part of [`Gen::host`]: an internal function returning the host
    /// pointer, or null for the slow path.
    unsafe fn host_fn(&self, write: bool) -> LLVMValueRef {
        // SAFETY: a new function of our module, built with matching types.
        unsafe {
            let name = if write {
                c"yavs_host_w"
            } else {
                c"yavs_host_r"
            };
            let fv = LLVMAddFunction(self.module, name.as_ptr(), self.host_ty);
            LLVMSetLinkage(fv, llvm_sys::LLVMLinkage::LLVMInternalLinkage);
            let kind = LLVMGetEnumAttributeKindForName(c"noinline".as_ptr(), 8);
            LLVMAddAttributeAtIndex(fv, u32::MAX, LLVMCreateEnumAttribute(self.ctx, kind, 0));
            let c = Cur {
                f: fv,
                env: LLVMGetParam(fv, 0),
                regs: std::ptr::null_mut(),
                mem: LLVMGetParam(fv, 1),
            };
            let (a, extra, n) = (
                LLVMGetParam(fv, 2),
                LLVMGetParam(fv, 3),
                LLVMGetParam(fv, 4),
            );
            let entry = self.block(&c);
            LLVMPositionBuilderAtEnd(self.b, entry);
            let hi = LLVMBuildLShr(self.b, a, self.c64(32), c"".as_ptr());
            let off = LLVMBuildAnd(self.b, a, self.c64(0xffff_ffff), c"".as_ptr());
            let off = LLVMBuildAdd(self.b, off, extra, c"".as_ptr());
            let (ok, p) = self.lookup(&c, hi, off, n, write);
            let (hit, miss, slow) = (self.block(&c), self.block(&c), self.block(&c));
            LLVMBuildCondBr(self.b, ok, hit, miss);
            LLVMPositionBuilderAtEnd(self.b, hit);
            LLVMBuildRet(self.b, p);
            LLVMPositionBuilderAtEnd(self.b, slow);
            LLVMBuildRet(self.b, LLVMConstNull(self.ptr));
            LLVMPositionBuilderAtEnd(self.b, miss);
            let mut args = [c.env, c.mem, a, self.c32(u32::from(write))];
            let found = LLVMBuildCall2(
                self.b,
                self.obj_ty,
                self.obj,
                args.as_mut_ptr(),
                4,
                c"".as_ptr(),
            );
            self.guard(&c, self.icmp(IP::LLVMIntNE, found, self.c32(0)), slow);
            let (ok, p) = self.lookup(&c, hi, off, n, write);
            let hit2 = self.block(&c);
            LLVMBuildCondBr(self.b, ok, hit2, slow);
            LLVMPositionBuilderAtEnd(self.b, hit2);
            LLVMBuildRet(self.b, p);
            fv
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
            // `regs` and `mem` overlap no other memory the function reaches (frame slots,
            // temporaries, objects), so LLVM may keep their values in registers between
            // calls; the callbacks get both pointers.
            let kind = LLVMGetEnumAttributeKindForName(c"noalias".as_ptr(), 7);
            for i in [2, 3] {
                LLVMAddAttributeAtIndex(fv, i, LLVMCreateEnumAttribute(self.ctx, kind, 0));
            }
            let c = Cur {
                f: fv,
                env: LLVMGetParam(fv, 0),
                regs: LLVMGetParam(fv, 1),
                mem: LLVMGetParam(fv, 2),
            };
            let pc_in = LLVMGetParam(fv, 3);
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
            let mut known = vec![None; f.reg_ty.len()];
            let mut prev = None;
            for (i, &inst) in f.code.iter().enumerate() {
                let pc = i as u32;
                if let Some(p) = prev.replace(inst) {
                    track(&mut known, &p);
                }
                if leader[i] {
                    known.fill(None);
                }
                let fk = |r: Reg| match known.get(r.0 as usize) {
                    Some(Some(Known::Frame(o))) => Some(*o),
                    _ => None,
                };
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
                    Inst::AddrOf { dst, space, off } => match addr_const(space, off) {
                        Some(a) if space == AddrSpace::Frame => {
                            let hi = self.mem_ld(&c, MEM_FRAME, self.i64);
                            let hi = LLVMBuildShl(self.b, hi, self.c64(32), c"".as_ptr());
                            self.st(
                                &c,
                                dst,
                                LLVMBuildOr(self.b, hi, self.c64(a as i64), c"".as_ptr()),
                            );
                        }
                        Some(a) => self.st(&c, dst, self.c64(a as i64)),
                        None => self.step(&c, pc),
                    },
                    Inst::Stmt => {
                        let (slow, join) = self.slow_path(&c, pc);
                        let roots = self.mem_ld(&c, MEM_TEMP_ROOTS, self.i64);
                        self.guard(&c, self.icmp(IP::LLVMIntEQ, roots, self.c64(0)), slow);
                        let len = self.mem_ld(&c, MEM_TEMP_LEN, self.i64);
                        let base = self.mem_ld(&c, MEM_TEMP_BASE, self.i64);
                        let lt = self.icmp(IP::LLVMIntULT, base, len);
                        let v = LLVMBuildSelect(self.b, lt, base, len, c"".as_ptr());
                        self.mem_st(&c, MEM_TEMP_LEN, v);
                        self.join(join);
                    }
                    Inst::Alloc { dst, size } => {
                        let (slow, join) = self.slow_path(&c, pc);
                        let (a, _) = self.temp_alloc(&c, self.ld(&c, size), slow);
                        self.st(&c, dst, a);
                        self.join(join);
                    }
                    Inst::LayoutOf { dst, ty, desc } => match static_layout(prog, ty) {
                        Some(sl) => {
                            let (slow, join) = self.slow_path(&c, pc);
                            let words = self.layout_words(&c, &sl, desc, fk(desc), slow);
                            let n = self.c64(words.len() as i64 * 8);
                            let (a, p) = self.temp_alloc(&c, n, slow);
                            for (k, w) in words.into_iter().enumerate() {
                                self.store(w, self.gep(p, self.c64(k as i64 * 8)));
                            }
                            self.st(&c, dst, a);
                            self.join(join);
                        }
                        None => self.step(&c, pc),
                    },
                    Inst::Load { dst, addr, w } => {
                        let (slow, join) = self.slow_path(&c, pc);
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
                        let (slow, join) = self.slow_path(&c, pc);
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
                        let (slow, join) = self.slow_path(&c, pc);
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
                        let (slow, join) = self.slow_path(&c, pc);
                        let n = self.c64(rs.len() as i64 * 8);
                        let p = self.host(&c, self.ld(&c, at), fk(at), self.c64(0), n, true, slow);
                        for (k, &r) in rs.iter().enumerate() {
                            let pk = self.gep(p, self.c64(k as i64 * 8));
                            self.store(self.ld(&c, r), pk);
                        }
                        self.join(join);
                    }
                    Inst::DopeLen { dst, dope, dim } => {
                        let (slow, join) = self.slow_path(&c, pc);
                        let d = self.ld(&c, dope);
                        let (n, ok) = self.dim_len(&c, d, u64::from(dim) * 24, slow);
                        // Lengths above i64::MAX are the interpreter's error.
                        let ok = LLVMBuildAnd(
                            self.b,
                            ok,
                            self.icmp(IP::LLVMIntSGE, n, self.c64(0)),
                            c"".as_ptr(),
                        );
                        self.guard(&c, ok, slow);
                        self.st(&c, dst, n);
                        self.join(join);
                    }
                    Inst::SubtypeCheck {
                        dope, want, dims, ..
                    } => {
                        let (slow, join) = self.slow_path(&c, pc);
                        let (d, w) = (self.ld(&c, dope), self.ld(&c, want));
                        for k in 0..u64::from(dims) {
                            let (a, oka) = self.dim_len(&c, d, k * 24, slow);
                            let (b, okb) = self.dim_len(&c, w, k * 24, slow);
                            let eq = self.icmp(IP::LLVMIntEQ, a, b);
                            let ok = LLVMBuildAnd(self.b, oka, okb, c"".as_ptr());
                            self.guard(&c, LLVMBuildAnd(self.b, ok, eq, c"".as_ptr()), slow);
                        }
                        self.join(join);
                    }
                    Inst::Pos {
                        dst,
                        dope,
                        dim,
                        idx,
                        ..
                    } => {
                        let (slow, join) = self.slow_path(&c, pc);
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
                        let (slow, join) = self.slow_path(&c, pc);
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
                        let (slow, join) = self.slow_path(&c, pc);
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
                        let (slow, join) = self.slow_path(&c, pc);
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
            let mut args = [c.env, c.regs, c.mem, self.c32(pc)];
            let ok = LLVMBuildCall2(
                self.b,
                self.fn_ty,
                self.step,
                args.as_mut_ptr(),
                4,
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

    /// A block that steps the instruction at `pc` in the interpreter and goes on at the
    /// returned join block: the slow path of a native instruction. The builder stays.
    unsafe fn slow_path(&self, c: &Cur, pc: u32) -> (LLVMBasicBlockRef, LLVMBasicBlockRef) {
        // SAFETY: well-formed blocks of c.f.
        unsafe {
            let here = LLVMGetInsertBlock(self.b);
            let (slow, join) = (self.block(c), self.block(c));
            LLVMPositionBuilderAtEnd(self.b, slow);
            self.step(c, pc);
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
            LLVMBuildGEP2(self.b, self.i8, p, idx.as_mut_ptr(), 1, c"".as_ptr())
        }
    }

    /// An unaligned load.
    unsafe fn load(&self, ty: LLVMTypeRef, p: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: `p` points to a value of `ty`.
        unsafe {
            let v = LLVMBuildLoad2(self.b, ty, p, c"".as_ptr());
            LLVMSetAlignment(v, 1);
            v
        }
    }

    /// An unaligned store.
    unsafe fn store(&self, v: LLVMValueRef, p: LLVMValueRef) {
        // SAFETY: `p` points to memory for a value of `v`'s type.
        unsafe {
            let s = LLVMBuildStore(self.b, v, p);
            LLVMSetAlignment(s, 1);
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
    /// lie in the frame's slots, the temporary area, (reading) the instance data, or the
    /// object `yavs_rt_obj` finds: what the interpreter's `rd`/`wr` would find there
    /// without an error. `frame` is the offset of `a` in the frame's slots when the
    /// generator knows it.
    #[allow(clippy::too_many_arguments)]
    unsafe fn host(
        &self,
        c: &Cur,
        a: LLVMValueRef,
        frame: Option<u64>,
        extra: LLVMValueRef,
        n: LLVMValueRef,
        write: bool,
        slow: LLVMBasicBlockRef,
    ) -> LLVMValueRef {
        // SAFETY: loads of `Mem` fields, integer arithmetic, well-formed blocks of c.f.
        unsafe {
            if let Some(o) = frame {
                // `extra` is below 2^63, the sum cannot wrap.
                let o = LLVMBuildAdd(self.b, self.c64(o as i64), extra, c"".as_ptr());
                let len = self.mem_ld(c, MEM_SLOTS_LEN, self.i64);
                let fits = self.icmp(IP::LLVMIntULE, n, len);
                let rest = LLVMBuildSub(self.b, len, n, c"".as_ptr());
                let inside = self.icmp(IP::LLVMIntULE, o, rest);
                self.guard(c, LLVMBuildAnd(self.b, fits, inside, c"".as_ptr()), slow);
                return self.gep(self.mem_ld(c, MEM_SLOTS, self.ptr), o);
            }
            let mut args = [c.env, c.mem, a, extra, n];
            let p = LLVMBuildCall2(
                self.b,
                self.host_ty,
                self.host_fns[usize::from(write)],
                args.as_mut_ptr(),
                5,
                c"".as_ptr(),
            );
            self.guard(c, LLVMBuildIsNotNull(self.b, p, c"".as_ptr()), slow);
            p
        }
    }

    /// Whether `n` bytes at `off` of the object `hi` lie in memory `Mem` points to, and the
    /// pointer to them.
    unsafe fn lookup(
        &self,
        c: &Cur,
        hi: LLVMValueRef,
        off: LLVMValueRef,
        n: LLVMValueRef,
        write: bool,
    ) -> (LLVMValueRef, LLVMValueRef) {
        // SAFETY: loads of `Mem` fields and integer arithmetic.
        unsafe {
            let sel = |c, a, b| LLVMBuildSelect(self.b, c, a, b, c"".as_ptr());
            let and = |a, b| LLVMBuildAnd(self.b, a, b, c"".as_ptr());
            let tag = |s: u8| self.c64(i64::from(s) << 29);
            let mut any = LLVMConstInt(LLVMInt1TypeInContext(self.ctx), 0, 0);
            let mut base = LLVMConstNull(self.ptr);
            let mut len = self.c64(0);
            for k in 0..OBJS as u64 {
                let at = MEM_OBJS + 32 * k;
                let is_o = self.icmp(IP::LLVMIntEQ, hi, self.mem_ld(c, at, self.i64));
                let is_o = if write {
                    let w = self.mem_ld(c, at + 24, self.i64);
                    and(is_o, self.icmp(IP::LLVMIntNE, w, self.c64(0)))
                } else {
                    is_o
                };
                any = self.or(any, is_o);
                base = sel(is_o, self.mem_ld(c, at + 8, self.ptr), base);
                len = sel(is_o, self.mem_ld(c, at + 16, self.i64), len);
            }
            if !write {
                let is_i = self.icmp(IP::LLVMIntEQ, hi, tag(space::INST));
                any = self.or(any, is_i);
                base = sel(is_i, self.mem_ld(c, MEM_INST, self.ptr), base);
                len = sel(is_i, self.mem_ld(c, MEM_INST_LEN, self.i64), len);
            }
            let is_t = self.icmp(IP::LLVMIntEQ, hi, tag(space::TEMP));
            any = self.or(any, is_t);
            base = sel(is_t, self.mem_ld(c, MEM_TEMP, self.ptr), base);
            len = sel(is_t, self.mem_ld(c, MEM_TEMP_LEN, self.i64), len);
            let is_f = self.icmp(IP::LLVMIntEQ, hi, self.mem_ld(c, MEM_FRAME, self.i64));
            any = self.or(any, is_f);
            base = sel(is_f, self.mem_ld(c, MEM_SLOTS, self.ptr), base);
            len = sel(is_f, self.mem_ld(c, MEM_SLOTS_LEN, self.i64), len);
            // n <= len && off <= len - n
            let fits = self.icmp(IP::LLVMIntULE, n, len);
            let rest = LLVMBuildSub(self.b, len, n, c"".as_ptr());
            let inside = self.icmp(IP::LLVMIntULE, off, rest);
            (and(any, and(fits, inside)), self.gep(base, off))
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
            let temp = self.mem_ld(c, MEM_TEMP, self.ptr);
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
        frame: Option<u64>,
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
            let p = self.host(c, d, frame, self.c64(0), n, false, slow);
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
        frame: Option<u64>,
        idx: Reg,
        write: bool,
        slow: LLVMBasicBlockRef,
    ) -> (LLVMValueRef, LLVMValueRef) {
        // SAFETY: integer arithmetic and `host`.
        unsafe {
            let i = self.ld(c, idx);
            let blk = LLVMBuildLShr(self.b, i, self.c64(6), c"".as_ptr());
            let off = LLVMBuildShl(self.b, blk, self.c64(5), c"".as_ptr());
            let p = self.host(c, self.ld(c, base), frame, off, self.c64(32), write, slow);
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

    /// The length of the descriptor dimension at `dope + off` as `yavs_ir::dim_len`, and
    /// whether it fits a u64 (`dim_len` is not `None`).
    unsafe fn dim_len(
        &self,
        c: &Cur,
        dope: LLVMValueRef,
        off: u64,
        slow: LLVMBasicBlockRef,
    ) -> (LLVMValueRef, LLVMValueRef) {
        // SAFETY: integer arithmetic on loaded words.
        unsafe {
            let p = self.host(
                c,
                dope,
                None,
                self.c64(off as i64),
                self.c64(24),
                false,
                slow,
            );
            let (l, r, dir) = self.dope_words(p);
            self.len_of(l, r, dir)
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
