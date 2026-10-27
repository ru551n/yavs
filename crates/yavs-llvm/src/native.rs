//! What the compiled tiers share about which code a function gets: the parts of a function
//! (`Part`), what a compile made (`Compiled`), and the rules for the functions that run on
//! native frames (`native_set`), written without LLVM so that every code generator uses them.
//! Their instruction predicates (`is_inline`, `nat_inline`) describe what a generator lowers
//! without a step; keep them in step with the generators.

use yavs_ir::{AddrSpace, FOp, Func, FuncId, FuncKind, Inst, IntOp, Program, Reg, Width};

use crate::rt::*;

/// Why `f` stays interpreted before any code is generated: over `max_size` IR instructions
/// (codegen time grows faster than linearly; #126).
pub fn too_big(f: &Func, max_size: usize) -> Option<String> {
    (f.code.len() > max_size).then(|| {
        format!(
            "{} IR instructions, over the size cap of {max_size} (--tier-max-size)",
            f.code.len()
        )
    })
}

/// Functions of at least this many IR instructions are `big`: optimised LLVM code generation of
/// such a function takes more than linear time (the register allocator's live-variable analysis
/// over thousands of blocks), 100 to 500 us per LLVM instruction where small functions take 30; the
/// unoptimised code generator takes 5 to 10 us (#432, #501).
const FAST_MIN: usize = 300;

/// A function of this many IR instructions or more is `big` even when its loops are compute
/// (`auto::compute`): a process of 30 000 instructions with two short `for` loops is no compute
/// loop, and its optimised code generation took 35 s for 670 000 LLVM instructions where `Fast`
/// takes 8 (the register allocator is more than linear in the function's size).
const COMPUTE_MAX: usize = 1500;

/// Whether `f` is large: LLVM compiles it at `OptLevel::Fast` (`lower::level_for`), and
/// `--tier=auto` prices it as `Kind::Big` or `Kind::BigLoop`, with a first-level generator alone too.
pub fn big(prog: &Program, f: &Func) -> bool {
    f.code.len() >= FAST_MIN && (f.code.len() >= COMPUTE_MAX || !crate::auto::compute(prog, f))
}

/// Which code of a function `compile_one` makes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    /// Frame-mode code, and native-frame code where the function can have it.
    Both,
    /// Native-frame code only (a callee compiled for its callers).
    Native,
    /// Frame-mode code only (the native code exists already).
    Frame,
}

/// What `compile_one` made of a function.
pub struct Compiled {
    /// Its entry for the interpreter (none for `Part::Native`) and register count.
    pub run: Option<RunFn>,
    pub nregs: usize,
    /// The address of its native-frame code and of its `CallFn` (0 for none).
    pub native: usize,
    pub call: usize,
    /// Native-frame code and `CallFn` of other functions this compile made on the way (a
    /// generator that compiles the callees of a function into its module): function, addresses.
    pub extra: Vec<(FuncId, usize, usize)>,
}

/// The seed and result address registers of a `math_real.uniform` call that `Gen::uniform_fast`
/// computes in place: three scalar registers of the frame, no result.
pub fn uniform_regs(
    prog: &Program,
    f: &Func,
    native: yavs_ir::NativeId,
    args: yavs_ir::Regs,
    dst: Option<Reg>,
) -> Option<[Reg; 3]> {
    if dst.is_some() {
        return None;
    }
    let sym = &prog.natives.get(native)?.symbol;
    // (The native itself is the slow path: the same instruction `native_direct` runs.)
    if !yavs_interp::direct_native(sym) {
        return None;
    }
    let key = yavs_ir::ieee::symbol_key(sym)?;
    let op = yavs_ir::ieee::parse(key.split_once('@').map_or(key, |k| k.0))?;
    let n = f.reg_ty.len() as u32;
    match (op, f.regs(args)?) {
        (yavs_ir::ieee::Op::Uniform, &[a, b, x])
            if [a, b, x]
                .iter()
                .all(|r| r.0 < n && f.reg_ty.get(r.0 as usize) != Some(&yavs_ir::RegTy::Sig)) =>
        {
            Some([a, b, x])
        }
        _ => None,
    }
}

/// Instructions the compiled code executes itself on its fast path (keep in step with
/// `Gen::func`).
pub fn is_inline(prog: &Program, f: &Func, i: &Inst) -> bool {
    match i {
        Inst::CallNative { native, args, dst } => {
            matches!(
                native_kind(prog, f, *native, *args, *dst),
                Some(NativeKind::Table(..))
            ) || uniform_regs(prog, f, *native, *args, *dst).is_some()
        }
        Inst::LayoutOf { ty, cache, .. } => {
            static_layout(prog, *ty).is_some()
                || cache.is_some() && layout_shape(prog, *ty).is_some()
        }
        Inst::Bin { op, .. } | Inst::BinRange { op, .. } => *op != IntOp::Pow,
        Inst::FBin { op, .. } => !matches!(op, FOp::Pow | FOp::PowI),
        Inst::AddrOf { space, .. } => {
            addr_const(*space, 0).is_some() || matches!(space, AddrSpace::Up(1..=2))
        }
        _ => {
            !inline_regs(f, i).is_empty()
                || matches!(i, Inst::Br { .. } | Inst::Stmt | Inst::Cover { .. })
        }
    }
}

/// Whether compiled frame-mode code of `f` leaves for a callback at instruction `pc` on its usual
/// path, as the policy of `--tier=auto` counts it (`auto::callbacks`): what `Gen::func` does not
/// answer in place. Not only `is_inline`: also the signal reads, `'event` and the clock-edge natives
/// answered from the kernel's views, the `WaitOn` that ends an activation, and the calls of
/// functions that run on native frames (`nc`, `native_set`). Keep in step with `Gen::func`; the
/// `cb_*` counters (`YAVS_COUNTERS=1`) show what really leaves.
pub fn leaves(prog: &Program, f: &Func, pc: usize, nc: &std::collections::HashSet<FuncId>) -> bool {
    let Some(i) = f.code.get(pc) else {
        return false;
    };
    match i {
        _ if is_inline(prog, f, i) => false,
        Inst::SigReadScalar { .. }
        | Inst::SigRead { .. }
        | Inst::Edge { .. }
        | Inst::SigAttr {
            kind: yavs_ir::AttrKind::Event,
            ..
        } => false,
        // Computed in place on its usual operands (`Gen::uniform_fast`).
        Inst::CallNative { native, args, dst }
            if frame_direct(prog, f, i)
                && uniform_regs(prog, f, *native, *args, *dst).is_some() =>
        {
            false
        }
        // Computed in place on its usual operands (each generator's `ieee_fast`).
        Inst::CallNative { native, args, dst }
            if frame_direct(prog, f, i)
                && yavs_interp::light_native(prog, f, *native, *args)
                && ieee_fast(prog, f, *native, *args, *dst).is_some() =>
        {
            false
        }
        Inst::CallNative { native, args, dst } => {
            !matches!(
                native_kind(prog, f, *native, *args, *dst),
                Some(NativeKind::Edge(_))
            ) || !matches!((f.regs(*args), dst), (Some(&[_]), Some(_)))
        }
        // The wait that ends the activation is its way out, not a callback in the middle of it.
        Inst::WaitOn { .. } => !matches!(f.code.get(pc + 1), Some(Inst::Suspend { .. })),
        Inst::Suspend { .. } => false,
        Inst::Call { func, .. } | Inst::CallProt { func, .. } => !nc.contains(func),
        _ => true,
    }
}

/// The most registers, slot bytes and IR instructions of a native-callable function: its
/// registers and slots are `alloca`s, and a materialisation stores every one of them.
pub const NC_REGS: usize = 512;

pub const NC_SLOTS: u32 = 4096;

pub const NC_CODE: usize = 2000;

/// Instructions a native frame lowers without a step (`Gen::func`); a frame-mode function lowers
/// the `Cat` of non-logic elements and the `CallNative` of a `direct_native` too
/// (`frame_direct`) and steps the others. `NewFrame`, `Fill`, a `Cat` of non-logic elements, a
/// typed `TempRoot`, a `CallNative` of a native `yavs_interp::direct_native` allows, and
/// `CallProt` (its callee is checked by `native_set`).
pub fn nat_inline(prog: &Program, f: &Func, i: &Inst) -> bool {
    let n = f.reg_ty.len() as u32;
    let fits = |rs: &[Reg]| rs.iter().all(|r| r.0 < n);
    match *i {
        Inst::NewFrame {
            dst,
            desc,
            data,
            size,
            ..
        } => fits(&[dst, desc, data, size]),
        Inst::Fill { dst, elem, n, .. } => fits(&[dst, elem, n]),
        Inst::Cat {
            dst,
            n: cnt,
            m,
            parts,
            logic: false,
            w,
            ..
        } => {
            w != Width::Sig
                && fits(&[dst, cnt, m])
                && f.regs(parts)
                    .is_some_and(|rs| rs.len() % 2 == 0 && rs.len() <= CAT_PARTS && fits(rs))
        }
        Inst::TempRoot { addr, desc, ty } => {
            fits(&[addr])
                && desc.is_none_or(|d| fits(&[d]))
                && ty.is_none_or(|t| yavs_ir::Ty::desc_size(&prog.types, t).is_some())
        }
        Inst::CallNative { native, args, dst } => {
            let Some(sym) = prog.natives.get(native).map(|n| &n.symbol) else {
                return false;
            };
            f.regs(args)
                .is_some_and(|rs| fits(rs) && dst.is_none_or(|d| fits(&[d])))
                && (yavs_interp::direct_native(sym) || yavs_interp::cold_native(sym))
        }
        Inst::CallProt { obj, .. } => fits(&[obj]),
        // Read from the view's signal table; where that has no answer the frame is materialised
        // (`Gen::signal`).
        Inst::SigReadScalar { dst, sig } => fits(&[dst, sig, Reg(sig.0.saturating_add(1))]),
        // The planes from the code bytes in place, else the kernel's blocks; where neither
        // answers the frame is materialised (`Gen::sig_read_block`).
        Inst::SigRead { dst, sig } => fits(&[dst, sig, Reg(sig.0.saturating_add(1))]),
        // The kernel's answer for one waveform element; where it has none the frame is
        // materialised (`Gen::drive_block`).
        Inst::Drive {
            target: Some(t),
            wave,
            n,
            reject: None,
            ..
        } => fits(&[t, Reg(t.0.saturating_add(1)), wave, n]),
        Inst::DriveSlot {
            slot,
            target,
            wave,
            n,
            reject: None,
            ..
        } => fits(&[slot, target, Reg(target.0.saturating_add(1)), wave, n]),
        // Cold: leaving to the interpreter is what they do.
        Inst::Unreachable { .. } => true,
        _ => xstep_ok(f, i),
    }
}

/// The most pieces (operand pairs) of a `Cat` a native frame lowers.
pub const CAT_PARTS: usize = 64;

/// Whether `f` itself may run on a native frame (`docs/decisions.md` "Compiled frames"): a
/// function or procedure without a static link, small, whose instructions are all lowered
/// without a step on their fast path; its calls are checked by `native_set`.
pub fn native_local(prog: &Program, f: &Func) -> bool {
    matches!(f.kind, FuncKind::Function | FuncKind::Procedure)
        && f.reg_ty.len() <= NC_REGS
        && f.frame <= NC_SLOTS
        && f.code.len() <= NC_CODE
        && bad_reg(f).is_none()
        && f.code.iter().all(|i| match i {
            Inst::Ret { .. } | Inst::Call { .. } => true,
            // The frame of the lexically enclosing subprogram, one or two levels up.
            Inst::AddrOf {
                space: AddrSpace::Up(d),
                ..
            } => f.parent.is_some() && (1..=2).contains(d),
            _ => is_inline(prog, f, i) || nat_inline(prog, f, i),
        })
}

/// The function a call instruction calls, its argument registers, and whether the first
/// parameter is a protected object (`CallProt`).
pub fn call_of(i: &Inst) -> Option<(FuncId, yavs_ir::Regs, bool)> {
    match *i {
        Inst::Call { func, args, .. } => Some((func, args, false)),
        Inst::CallProt { func, args, .. } => Some((func, args, true)),
        _ => None,
    }
}

/// Whether the call of `callee` with `args` from `f` (function `caller`) passes one argument per
/// parameter, in registers of `f`, and can give a nested callee its static link; for a protected
/// call (`prot`) the object is the first parameter.
pub fn call_fits(f: &Func, caller: FuncId, callee: &Func, args: yavs_ir::Regs, prot: bool) -> bool {
    let n = f.reg_ty.len() as u32;
    let skip = usize::from(prot);
    // A nested callee gets its static link from the caller: the caller itself or its own
    // enclosing subprogram.
    if callee
        .parent
        .is_some_and(|p| p != caller && f.parent != Some(p))
    {
        return false;
    }
    if prot && callee.params.first().is_none_or(|p| p.width == Width::Sig) {
        return false;
    }
    f.regs(args).is_some_and(|rs| {
        rs.len() + skip == callee.params.len()
            && rs
                .iter()
                .zip(callee.params.iter().skip(skip))
                .all(|(r, p)| {
                    r.0.checked_add(u32::from(p.width == Width::Sig))
                        .is_some_and(|x| x < n)
                })
    })
}

/// The functions reachable through calls from `roots` that can run on native frames: each
/// `native_local`, and calling only such functions (the greatest such set).
pub fn native_set(
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
        todo.extend(f.code.iter().filter_map(call_of).map(|c| c.0));
    }
    loop {
        let out: Vec<FuncId> = set
            .iter()
            .copied()
            .filter(|&id| {
                prog.func(id).is_none_or(|f| {
                    f.code.iter().filter_map(call_of).any(|(func, args, prot)| {
                        !set.contains(&func)
                            || prog
                                .func(func)
                                .is_none_or(|g| !call_fits(f, id, g, args, prot))
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

/// The byte ranges of the slots of `f` (`id`) when no address of its frame can leave its own
/// code, so nothing else can ask for them by frame index (`Mem::nslots`): no function is nested
/// in it, every address of the frame is made once (`AddrOf`, a preload) and used only as the
/// address of a `Load` or a `Store`, never as data, an operand of a call or of any other
/// instruction. The ranges are the parameters and what those loads and stores touch, merged;
/// the rest of the slots stay zero.
pub fn closed_frame(prog: &Program, f: &Func, id: FuncId) -> Option<Vec<(u64, u64)>> {
    let base = prog.base_of(id);
    if prog.funcs.iter().any(|(_, g)| g.parent == Some(base)) {
        return None;
    }
    let n = f.reg_ty.len();
    let mut at: Vec<Option<u64>> = vec![None; n + 1];
    let mut defs = vec![0u32; n + 1];
    for i in &f.code {
        // Operand lists (calls, natives, concatenations) count: `inst_regs`, not `Inst::visit`.
        for r in yavs_interp::inst_regs(f, i).1 {
            if let Some(d) = defs.get_mut(r.0 as usize) {
                *d += 1;
            }
        }
        if let Inst::AddrOf {
            dst,
            space: AddrSpace::Frame,
            off,
        } = *i
            && let Some(a) = at.get_mut(dst.0 as usize)
        {
            *a = Some(off.into());
        }
    }
    if at.iter().zip(&defs).any(|(a, &d)| a.is_some() && d != 1) {
        return None;
    }
    let mut ranges: Vec<(u64, u64)> = f
        .params
        .iter()
        .map(|p| {
            (
                u64::from(p.slot),
                u64::from(p.slot) + u64::from(p.width.bytes()),
            )
        })
        .collect();
    for i in &f.code {
        let uses = yavs_interp::inst_regs(f, i)
            .0
            .iter()
            .filter(|r| at.get(r.0 as usize).is_some_and(|a| a.is_some()))
            .count();
        let allowed = match *i {
            Inst::Load { addr: a, w, .. } | Inst::Store { addr: a, w, .. } => {
                match at.get(a.0 as usize).copied().flatten() {
                    Some(o) => {
                        ranges.push((o, o + u64::from(w.bytes())));
                        1
                    }
                    None => 0,
                }
            }
            _ => 0,
        };
        if uses != allowed {
            return None;
        }
    }
    ranges.sort_unstable();
    let mut merged: Vec<(u64, u64)> = Vec::new();
    for (lo, hi) in ranges {
        match merged.last_mut() {
            Some(last) if lo <= last.1 => last.1 = last.1.max(hi),
            _ => merged.push((lo, hi)),
        }
    }
    // A range beyond the frame is the function's own bug: leave it to the registered form.
    merged
        .iter()
        .all(|&(_, hi)| hi <= u64::from(f.frame))
        .then_some(merged)
}

/// How a light `numeric_std` native takes its arguments: an array (four registers) or an integer.
#[derive(Clone, Copy, PartialEq)]
pub enum Arg {
    Arr,
    Int,
}

/// The light `numeric_std` natives compiled code computes itself on operands of at most 64
/// elements without a metavalue (the native's own fast path, `ieee::small`; `Gen::ieee_fast`),
/// and their arguments; any other operand, and every case that warns or fails, runs the callback.
pub fn ieee_layout(op: yavs_ir::ieee::Op) -> Option<&'static [Arg]> {
    use yavs_ir::ieee::{Form, Op};
    Some(match op {
        Op::ToInteger { .. } | Op::NumNot | Op::VecNot => &[Arg::Arr],
        Op::Add { form, .. } | Op::Compare { form, .. } => match form {
            Form::Vv => &[Arg::Arr, Arg::Arr],
            Form::Vi => &[Arg::Arr, Arg::Int],
            Form::Iv => &[Arg::Int, Arg::Arr],
        },
        Op::ToUnsigned | Op::ToSigned => &[Arg::Int, Arg::Int],
        Op::Resize { .. } | Op::Shift(_) | Op::ShiftFn(_) => &[Arg::Arr, Arg::Int],
        Op::NumGate(_) | Op::VecGate(_) => &[Arg::Arr, Arg::Arr],
        _ => return None,
    })
}

/// The native a `CallNative` is, when `Gen::ieee_fast` computes it, with its register count
/// checked.
pub fn ieee_fast(
    prog: &Program,
    f: &Func,
    native: yavs_ir::NativeId,
    args: yavs_ir::Regs,
    dst: Option<Reg>,
) -> Option<yavs_ir::ieee::Op> {
    dst?;
    let sym = &prog.natives.get(native)?.symbol;
    let key = yavs_ir::ieee::symbol_key(sym)?;
    let op = yavs_ir::ieee::parse(key.split_once('@').map_or(key, |k| k.0))?;
    let want: usize = ieee_layout(op)?
        .iter()
        .map(|a| match a {
            Arg::Arr => yavs_ir::ieee::ARRAY_REGS,
            Arg::Int => 1,
        })
        .sum();
    (f.regs(args)?.len() == want).then_some(op)
}
