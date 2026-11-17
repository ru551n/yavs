//! Interpreter arms: calls the interpreter takes inline while compiled code keeps the call.
//!
//! The compiled tiers compile the optimised copy of a function ([`crate::Program::func`]); a
//! callee inlined there is compiled again at every call site, so the inliner's limits keep
//! accessors of 50 to 100 instructions (a protected method of a pointer table) calls. The
//! interpreter pays about as much for the call as for such a body. So the interpreter may run a
//! function that it called often as [`crate::Program::arm`]: the same copy, every instruction at
//! the same pc, plus blocks after the last one that no instruction of the copy jumps to. For
//! each call that has an arm the interpreter jumps to the arm's first block instead of calling
//! (`Program::arm_sites`).
//!
//! An arm is the callee's inlinable body with larger limits (`ARM_MULTI`, `ARM_BLOCKS`, its
//! registers new ones, its parameters the call's arguments), followed by a copy of the rest of
//! the call's block: up to the next call with an arm (a jump to that arm) or to the block's
//! terminator. Every jump of an arm leads into an arm or to a successor of the call's block, as
//! the code after the call does, so what holds on the code after the call (definitions,
//! liveness, the roots of a collection) holds on the arm the same way. Constants and addresses
//! defined once are not copied: frames start with them (`Pre`). A callee is taken only once its
//! optimised copy exists: making an arm never fixes another function's version.
//!
//! An inlined instruction keeps the callee's statement and `via`, as in `inline.rs`, so a check
//! that fails in an arm or a trace through a call made there reads as the call would. Only
//! subprograms that cannot suspend get arms. `YAVS_NO_ARMS=1` turns them off.

use super::*;

/// Are interpreter arms made? `YAVS_NO_ARMS=1` keeps every call a call.
pub(crate) fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("YAVS_NO_ARMS").map_or(true, |v| v.is_empty() || v == "0"))
}

/// `f` (the optimised copy of `id`) with its arms, `None` when no call gets one.
pub(crate) fn make_armed(
    prog: &crate::Program,
    id: FuncId,
    f: &crate::Func,
) -> Option<crate::program::Armed> {
    if !enabled()
        || !super::enabled()
        || super::super::keep_variables_on()
        || !matches!(f.kind, FuncKind::Function | FuncKind::Procedure)
        || f.flags.contains(crate::FuncFlags::MAY_SUSPEND)
    {
        return None;
    }
    let ty = |r: Reg| f.reg_ty.get(r.0 as usize);
    let mut sites: std::collections::BTreeMap<usize, Site> = std::collections::BTreeMap::new();
    let mut added = 0usize;
    for (pc, i) in f.code.iter().enumerate() {
        let (func, args, obj, dst) = match *i {
            Inst::Call { func, args, dst } => (func, args, None, dst),
            Inst::CallProt {
                func,
                obj,
                args,
                dst,
            } => (func, args, Some(obj), dst),
            _ => continue,
        };
        if func == id
            || tabbed(prog, func)
            || prog
                .funcs
                .get(func)
                .is_some_and(|c| c.flags.contains(crate::FuncFlags::PURE))
                && prog.memo_sig(func).is_some()
        {
            continue;
        }
        let Some(fw) = forwarder(prog, func, 1, false, true) else {
            continue;
        };
        // (A cold block calls the callee with `Call`: a protected method stays a call.)
        if obj.is_some() && !fw.cold.is_empty() {
            continue;
        }
        let Some(callee) = prog.funcs.get(func) else {
            continue;
        };
        let Some(rest) = f
            .operands
            .get(args.start as usize..(args.start + args.len) as usize)
        else {
            continue;
        };
        let sargs: Vec<Reg> = obj.into_iter().chain(rest.iter().copied()).collect();
        let typed = sargs.len() == callee.params.len()
            && fw.params.iter().enumerate().all(|(k, regs)| {
                regs.iter().all(|r| {
                    fw.cold.contains(r)
                        || ty(sargs[k]).is_some() && ty(sargs[k]) == fw.reg_ty.get(r.0 as usize)
                })
            });
        let returns = fw.blocks.iter().all(|b| match b.last() {
            Some((Inst::Ret { v }, ..)) => match (dst, v) {
                (Some(d), Some(r)) => ty(d).is_some() && ty(d) == fw.reg_ty.get(r.0 as usize),
                (None, _) => true,
                (Some(_), None) => false,
            },
            _ => true,
        });
        let cost = fw.len() + 2;
        if typed && returns && added + cost <= GROWTH + 4 * f.code.len() {
            added += cost;
            sites.insert(
                pc,
                Site {
                    fw,
                    func,
                    args: sargs,
                    dst,
                    dup: false,
                },
            );
        }
    }
    if sites.is_empty() {
        return None;
    }
    // Registers a frame starts with (`Pre`): their one definition is not copied.
    let mut defs = vec![0u8; f.reg_ty.len()];
    for i in &f.code {
        i.visit(&mut |r, role, _| {
            if role == Role::Def
                && let Some(d) = defs.get_mut(r.0 as usize)
            {
                *d = d.saturating_add(1);
            }
        });
    }
    let preloaded = |i: &Inst| match *i {
        Inst::Const { dst, .. } | Inst::ConstF { dst, .. } => defs[dst.0 as usize] == 1,
        Inst::AddrOf { dst, space, .. } => {
            !matches!(space, AddrSpace::Up(_)) && defs[dst.0 as usize] == 1
        }
        _ => false,
    };
    let via = |pc: usize| f.via.get(pc).copied().unwrap_or(0);
    let mut g = f.clone();
    if g.via.is_empty() {
        g.via = vec![0; g.code.len()];
    }
    // Each block of the arms, and how many of its first instructions are the callee's (the tier
    // books them to it, `Program::arm_owner`).
    let mut blocks: Vec<Vec<Out>> = Vec::new();
    let mut body: Vec<(usize, u32)> = Vec::new();
    let first = f.blocks.len();
    let mut armed: Vec<(u32, BlockId)> = Vec::new();
    for (_, blk) in f.blocks.iter() {
        let (start, end) = (blk.start as usize, blk.end as usize);
        let pcs: Vec<usize> = sites.range(start..end).map(|(&pc, _)| pc).collect();
        // (A call is never a terminator: the last instruction stays where it is.)
        if pcs.is_empty() || !f.code[end - 1].is_terminator() {
            continue;
        }
        for (k, &pc) in pcs.iter().enumerate() {
            let site = &sites[&pc];
            let fw = &site.fw;
            let entry = first + blocks.len();
            armed.push((pc as u32, BlockId(entry as u32)));
            let ex = Expand::new(f, site, pc, Round::All, &mut g.reg_ty, &mut g.via_nodes);
            let (loc, sv) = (ex.loc, ex.sv);
            // The block after the callee's body, the rest of the call's block (the last of the
            // arm, whatever the body's shape).
            let cont = if fw.straight() {
                entry
            } else {
                entry + fw.blocks.len()
            };
            let next = pcs.get(k + 1).copied();
            let mut tail: Vec<Out> = (pc + 1..next.unwrap_or(end - 1))
                .filter(|&p| !preloaded(&f.code[p]))
                .map(|p| (f.code[p], f.loc[p], via(p)))
                .collect();
            tail.push(match next {
                Some(n) => (
                    Inst::Br {
                        to: BlockId(cont as u32 + 1),
                    },
                    f.loc[n],
                    via(n),
                ),
                None => (f.code[end - 1], f.loc[end - 1], via(end - 1)),
            });
            if fw.straight() {
                let mut cur = Vec::new();
                ex.splice(&mut cur, &mut g.operands);
                body.push((cur.len(), site.func.0));
                cur.extend(tail);
                blocks.push(cur);
                continue;
            }
            for (i, cb) in fw.blocks.iter().enumerate() {
                let mut ents: Vec<Out> = Vec::with_capacity(cb.len() + 2);
                if i == 0 {
                    ex.param_moves(&mut ents);
                }
                for &(inst, l, v) in cb {
                    let (l, v) = ex.own(l, v);
                    match inst {
                        Inst::Ret { .. } => {
                            if let Some(m) = ex.ret_move(&inst) {
                                ents.push((m, loc, sv));
                            }
                            ents.push((
                                Inst::Br {
                                    to: BlockId(cont as u32),
                                },
                                loc,
                                sv,
                            ));
                        }
                        Inst::Br { to } => ents.push((
                            Inst::Br {
                                to: BlockId((entry as u32) + to.0),
                            },
                            l,
                            v,
                        )),
                        Inst::CondBr { c, t, f: e } => ents.push((
                            Inst::CondBr {
                                c: ex.map(c),
                                t: BlockId(entry as u32 + t.0),
                                f: BlockId(entry as u32 + e.0),
                            },
                            l,
                            v,
                        )),
                        other => ents.push((ex.place_inst(other, &mut g.operands), l, v)),
                    }
                }
                body.push((ents.len(), site.func.0));
                blocks.push(ents);
            }
            body.push((0, site.func.0));
            blocks.push(tail);
        }
    }
    if armed.is_empty() {
        return None;
    }
    // (The interpreter tells the two forms of a frame by their registers: the armed one has more.)
    if g.reg_ty.len() == f.reg_ty.len() {
        g.reg_ty.push(RegTy::I64);
    }
    let base = g.code.len() as u32;
    let mut owner = Vec::new();
    for (ents, &(n, callee)) in blocks.into_iter().zip(&body) {
        let start = g.code.len() as u32;
        owner.extend((0..ents.len()).map(|k| if k < n { callee } else { u32::MAX }));
        for (i, l, v) in ents {
            g.code.push(i);
            g.loc.push(l);
            g.via.push(v);
        }
        g.blocks.push(crate::Block {
            start,
            end: g.code.len() as u32,
        });
    }
    if crate::build::eager_verify() {
        verify(prog, id, &g, &armed);
    }
    Some(crate::program::Armed {
        func: g,
        sites: armed,
        base,
        owner,
    })
}

/// Debug builds and `YAVS_VERIFY_IR`: `g` verified as code, and its arms as the interpreter
/// reaches them: in a view where each call with an arm ends its block with a jump to the arm
/// (the rest of the block, which the arm copies, is a block nothing reaches).
fn verify(prog: &crate::Program, id: FuncId, g: &crate::Func, armed: &[(u32, BlockId)]) {
    let mut errs = Vec::new();
    crate::verify::verify_func(prog, id, g, &mut errs);
    assert!(errs.is_empty(), "arms broke {}: {errs:?}", g.name);
    // The view: blocks split after each call with an arm, ids in code order.
    let mut cuts: Vec<(u32, BlockId)> = armed.to_vec();
    cuts.sort_unstable();
    let mut view = g.clone();
    let mut starts: Vec<u32> = Vec::new();
    // Old block -> new id of its first part.
    let mut renum: Vec<u32> = Vec::with_capacity(g.blocks.len());
    for (_, blk) in g.blocks.iter() {
        renum.push(starts.len() as u32);
        starts.push(blk.start);
        for &(pc, _) in cuts.iter().filter(|c| c.0 >= blk.start && c.0 < blk.end) {
            starts.push(pc + 1);
        }
    }
    let map = |b: BlockId| BlockId(renum[b.0 as usize]);
    for i in &mut view.code {
        match i {
            Inst::Br { to } => *to = map(*to),
            Inst::CondBr { t, f, .. } => {
                *t = map(*t);
                *f = map(*f);
            }
            _ => {}
        }
    }
    for &(pc, to) in &cuts {
        view.code[pc as usize] = Inst::Br { to: map(to) };
    }
    view.blocks = crate::IdVec::new();
    for (k, &s) in starts.iter().enumerate() {
        let end = starts.get(k + 1).copied().unwrap_or(g.code.len() as u32);
        view.blocks.push(crate::Block { start: s, end });
    }
    // (`Case` tables name blocks of the program: a function with a `Case` is checked as code only.)
    if view.code.iter().any(|i| matches!(i, Inst::Case { .. })) {
        return;
    }
    let bad = crate::verify::undefined_uses(prog, &view);
    assert!(
        bad.is_empty(),
        "arms of {} read before writing: {bad:?}",
        g.name
    );
}
