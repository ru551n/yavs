//! Which `Inst::Stmt` markers of a function must trim the temporary area (`stmt_trims`).
//!
//! A marker frees what the statement before it made. Compiled code runs it as a test of the
//! recorded roots, a load of the length and the base, and a store. This pass proves, from the
//! instructions between markers, where that work can go:
//!
//! * `Skip`: nothing to free (no instruction since the last trim leaves temporaries), or, in a
//!   process outside every loop that does not suspend, only a small bounded amount, which the
//!   next suspension frees with the rest (the machine empties a process's temporaries there).
//! * `Trim { roots_free }`: trim. `roots_free`: no instruction since the last trim records a
//!   root and the function is a process (no caller holds roots), so the root test can go.
//!
//! The interpreter keeps every marker; only a tier that asks for this gets the shorter code.

use crate::opt::{is_dead, is_wait, leaves_temps};
use crate::{Func, FuncFlags, FuncKind, Inst, Program, Role};

/// What compiled code does at a marker.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StmtTrim {
    /// Nothing (also the value for pcs that are not markers).
    Skip,
    /// Truncate the temporary area to the frame's base; `roots_free`: the root test is not needed.
    Trim { roots_free: bool },
}

/// Bytes of small temporaries a process may carry across markers before one trims, and the
/// largest constant `Alloc` that counts as small. A statement's own temporaries come on top: the
/// two stay under the capacity a resident process keeps (`RESIDENT_TEMP` in `yavs-interp`, 1 KiB;
/// raising it charges every process against the resident budget and costs scratch swaps).
const LIMIT: u32 = 256;
const SMALL_ALLOC: i64 = 128;
/// An upper bound for the layout words one `LayoutOf` writes (a few words per dimension).
const LAYOUT_BYTES: u32 = 64;
/// More than `LIMIT`: the state of an instruction with no static bound.
const CAP: u32 = LIMIT + 1;

#[derive(Clone, Copy, PartialEq, Eq)]
struct State {
    dirt: u32,
    roots: bool,
}

const CLEAN: State = State {
    dirt: 0,
    roots: false,
};

impl State {
    fn join(&mut self, o: State) -> bool {
        let n = State {
            dirt: self.dirt.max(o.dirt),
            roots: self.roots | o.roots,
        };
        std::mem::replace(self, n) != n
    }
}

/// The marker decisions of `f`, one per instruction (`Skip` where there is no marker).
pub fn stmt_trims(prog: &Program, f: &Func) -> Vec<StmtTrim> {
    let n = f.code.len();
    let mut out = vec![StmtTrim::Skip; n];
    if !f.code.iter().any(|i| matches!(i, Inst::Stmt)) {
        return out;
    }
    let process = f.kind == FuncKind::Process;
    // A subprogram that waits keeps its temporaries across the wait: trim as before.
    if !process && f.code.iter().any(is_wait) {
        for (pc, i) in f.code.iter().enumerate() {
            if matches!(i, Inst::Stmt) {
                out[pc] = StmtTrim::Trim { roots_free: false };
            }
        }
        return out;
    }
    let clean_callee = |c| {
        prog.funcs
            .get(c)
            .is_some_and(|g| g.defined && g.flags.contains(FuncFlags::NO_TEMPS))
    };
    let leaves = |i: &Inst| leaves_temps(i, &f.reg_ty, &prog.natives, &clean_callee);
    // Registers defined once, by a constant.
    let nr = f.reg_ty.len();
    let mut defs = vec![0u32; nr];
    let mut konst: Vec<Option<i64>> = vec![None; nr];
    for i in &f.code {
        i.visit(&mut |r, role, _| {
            if role == Role::Def
                && let Some(d) = defs.get_mut(r.0 as usize)
            {
                *d += 1;
            }
        });
        if let Inst::Const { dst, v } = *i
            && let Some(k) = konst.get_mut(dst.0 as usize)
        {
            *k = Some(v);
        }
    }
    let size_of = |r: crate::Reg| {
        let k = r.0 as usize;
        (defs.get(k) == Some(&1)).then(|| konst[k]).flatten()
    };

    // Blocks and their successors.
    let nb = f.blocks.len();
    let range = |b: usize| {
        let bl = &f.blocks.values()[b];
        bl.start as usize..bl.end as usize
    };
    let succ = |b: usize| -> Vec<usize> {
        let r = range(b);
        let Some(last) = r.end.checked_sub(1).filter(|_| r.end > r.start) else {
            return Vec::new();
        };
        match f.code[last] {
            Inst::Case { table, .. } => prog
                .cases
                .get(table)
                .map(|t| {
                    std::iter::once(t.default.0 as usize)
                        .chain(t.arms.iter().map(|a| a.2.0 as usize))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            ref i => i.targets().iter().map(|t| t.0 as usize).collect(),
        }
        .into_iter()
        .filter(|&t| t < nb)
        .collect()
    };
    let is_susp = |i: &Inst| process && matches!(i, Inst::Suspend { .. }) && !is_dead(i);

    // The loops that do not pass a suspension (a process's outer loop does): per marker.
    let cyclic = if process {
        cyclic_segments(f, nb, &range, &succ, &is_susp)
    } else {
        Vec::new()
    };

    // The state at the start of each block, to a fixed point.
    let mut din = vec![CLEAN; nb];
    let mut seen = vec![false; nb];
    let mut work = vec![0usize];
    if nb > 0 {
        seen[0] = true;
    }
    let walk = |b: usize, mut s: State, rec: &mut Option<&mut Vec<StmtTrim>>| -> State {
        let mut seg = 0usize;
        for pc in range(b) {
            let i = &f.code[pc];
            if is_dead(i) {
                continue;
            }
            if matches!(i, Inst::Stmt) {
                let loop_free = process && !cyclic[b][seg];
                let skip = if process {
                    loop_free && s.dirt <= LIMIT && !s.roots
                } else {
                    s.dirt == 0 && !s.roots
                };
                if let Some(o) = rec {
                    o[pc] = if skip {
                        StmtTrim::Skip
                    } else {
                        StmtTrim::Trim {
                            roots_free: process && !s.roots,
                        }
                    };
                }
                if !skip {
                    s = CLEAN;
                }
            } else if is_susp(i) {
                s = CLEAN;
                seg += 1;
            } else if is_wait(i) {
            } else if let Inst::Alloc { size, .. } = *i {
                match size_of(size) {
                    Some(k) if (0..=SMALL_ALLOC).contains(&k) && process => {
                        s.dirt = s.dirt.saturating_add(k as u32 + 8).min(CAP);
                    }
                    _ => s.dirt = CAP,
                }
            } else if matches!(i, Inst::LayoutOf { .. }) && process {
                s.dirt = s.dirt.saturating_add(LAYOUT_BYTES).min(CAP);
            } else if leaves(i) {
                s.dirt = CAP;
                // These record no root (only `TempRoot` does); everything else is assumed to.
                s.roots |= !matches!(
                    i,
                    Inst::Alloc { .. }
                        | Inst::LayoutOf { .. }
                        | Inst::NewFrame { .. }
                        | Inst::SigParts { .. }
                        | Inst::SigSlice { .. }
                );
            }
        }
        s
    };
    while let Some(b) = work.pop() {
        let s = walk(b, din[b], &mut None);
        for t in succ(b) {
            if din[t].join(s) || !seen[t] {
                seen[t] = true;
                work.push(t);
            }
        }
    }
    for b in 0..nb {
        if seen[b] {
            walk(b, din[b], &mut Some(&mut out));
        } else {
            for pc in range(b) {
                if matches!(f.code[pc], Inst::Stmt) {
                    out[pc] = StmtTrim::Trim { roots_free: false };
                }
            }
        }
    }
    out
}

/// For each block and each of its segments (split after a suspension), whether it lies on a
/// cycle of the segment graph. An edge joins a block's last segment to its successors' first;
/// a suspension cuts the edge to the segment after it.
fn cyclic_segments(
    f: &Func,
    nb: usize,
    range: &dyn Fn(usize) -> std::ops::Range<usize>,
    succ: &dyn Fn(usize) -> Vec<usize>,
    is_susp: &dyn Fn(&Inst) -> bool,
) -> Vec<Vec<bool>> {
    // Node ids: first[b] + segment.
    let mut first = Vec::with_capacity(nb + 1);
    let mut total = 0usize;
    for b in 0..nb {
        first.push(total);
        total += 1 + range(b).filter(|&pc| is_susp(&f.code[pc])).count();
    }
    first.push(total);
    // A segment reaches successors only from the last one of its block.
    let edges = |v: usize| -> Vec<usize> {
        let b = first.partition_point(|&x| x <= v) - 1;
        if v + 1 == first[b + 1] {
            succ(b).into_iter().map(|t| first[t]).collect()
        } else {
            Vec::new()
        }
    };
    // Iterative Tarjan.
    let (mut idx, mut low) = (vec![usize::MAX; total], vec![0usize; total]);
    let (mut on, mut comp) = (vec![false; total], vec![usize::MAX; total]);
    let mut stack = Vec::new();
    let (mut counter, mut ncomp) = (0usize, 0usize);
    let mut size: Vec<usize> = Vec::new();
    let mut selfloop = vec![false; total];
    for root in 0..total {
        if idx[root] != usize::MAX {
            continue;
        }
        let mut call: Vec<(usize, Vec<usize>, usize)> = vec![(root, edges(root), 0)];
        idx[root] = counter;
        low[root] = counter;
        counter += 1;
        stack.push(root);
        on[root] = true;
        while let Some((v, es, k)) = call.last_mut() {
            let v = *v;
            if *k < es.len() {
                let w = es[*k];
                *k += 1;
                if w == v {
                    selfloop[v] = true;
                }
                if idx[w] == usize::MAX {
                    idx[w] = counter;
                    low[w] = counter;
                    counter += 1;
                    stack.push(w);
                    on[w] = true;
                    call.push((w, edges(w), 0));
                } else if on[w] {
                    low[v] = low[v].min(idx[w]);
                }
            } else {
                call.pop();
                if let Some((p, _, _)) = call.last() {
                    low[*p] = low[*p].min(low[v]);
                }
                if low[v] == idx[v] {
                    let mut sz = 0;
                    while let Some(w) = stack.pop() {
                        on[w] = false;
                        comp[w] = ncomp;
                        sz += 1;
                        if w == v {
                            break;
                        }
                    }
                    size.push(sz);
                    ncomp += 1;
                }
            }
        }
    }
    (0..nb)
        .map(|b| {
            (first[b]..first[b + 1])
                .map(|v| size[comp[v]] > 1 || selfloop[v])
                .collect()
        })
        .collect()
}
