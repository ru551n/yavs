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
}

/// Instructions the compiled code executes itself on its fast path (keep in step with
/// `Gen::func`).
pub fn is_inline(prog: &Program, f: &Func, i: &Inst) -> bool {
    match i {
        Inst::CallNative { native, args, dst } => matches!(
            native_kind(prog, f, *native, *args, *dst),
            Some(NativeKind::Table(..))
        ),
        Inst::LayoutOf { ty, cache, .. } => {
            static_layout(prog, *ty).is_some()
                || cache.is_some() && layout_shape(prog, *ty).is_some()
        }
        Inst::Bin { op, .. } | Inst::BinRange { op, .. } => *op != IntOp::Pow,
        Inst::FBin { op, .. } => !matches!(op, FOp::Pow | FOp::PowI),
        Inst::AddrOf { space, .. } => {
            addr_const(*space, 0).is_some() || matches!(space, AddrSpace::Up(1..=2))
        }
        _ => !inline_regs(f, i).is_empty() || matches!(i, Inst::Br { .. } | Inst::Stmt),
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
