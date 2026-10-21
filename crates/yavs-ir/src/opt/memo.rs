//! Instance-constant memoisation in processes (#443).
//!
//! One process function serves every instance of a design unit, so a generic, a generate
//! index or a constant is read from instance data every time: `q((i + 1) mod N)` repeats the
//! load, the addition, the `mod`, the range checks and the index arithmetic at each
//! activation, always with the same result. This pass, run last on a process, splits the
//! block of the process loop that holds such instructions at the first of them into
//!
//! - a head that tests a flag in the process frame,
//! - a slow copy of the rest of the block that is the original code (so a failing check
//!   fails with the same message at the same time) and, after the instruction that defines a
//!   value the rest of the code needs, stores it in a frame slot, and after the last
//!   instance-constant instruction sets the flag,
//! - and a fast copy without the instance-constant instructions, where those values are
//!   loaded from their slots.
//!
//! An instruction is instance-constant when it is pure, its operands are instance-constant
//! (constants, addresses, loads from instance data and the constant pool, which are frozen
//! before the simulation starts, and what is computed from them) and it can only fail in a
//! way that is the same at every execution (a check, a checked operation, a position). Moves
//! of such values, constants and addresses stay in both copies when something needs them.
//! The slow copy renames the registers it defines to fresh ones (register sharing gives them
//! cells again) so that the fast copy keeps the register counts the predecoder's fusions
//! count on.
//!
//! The flag and the slots are frame bytes behind the function's own frame, which start zero.
//! Debuggers keep the old shape ([`super::keep_variables`]).

// Loops index several parallel per-instruction vectors by position.
#![allow(clippy::needless_range_loop)]

use super::*;

/// Slots (and loads in the fast copy) a block may use.
const MAX_ROOTS: usize = 12;
/// Instructions the fast copy has to save, net of the flag test, to be worth a copy.
const MIN_GAIN: usize = 3;
/// Longest block the pass copies.
const MAX_BLOCK: usize = 800;

/// Instructions of a new block with their statements.
type Part = Vec<(Inst, crate::StmtId)>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum K {
    /// Not instance-constant.
    Other,
    /// A constant, an address or a move of an instance-constant value: kept where needed.
    Free,
    /// Instance-constant, computed once.
    Memo,
}

/// What to do with one block.
struct Plan {
    block: usize,
    first: usize,
    kind: Vec<K>,
    keep: Vec<bool>,
    root: Vec<bool>,
    last_memo: usize,
    rename: Vec<u32>,
}

fn operands(f: &crate::Func, i: &Inst, mut g: impl FnMut(Reg, Role)) {
    i.visit(&mut |r, role, _| g(r, role));
    if let Some(l) = i.list()
        && let Some(rs) = f.regs(l)
    {
        for &r in rs {
            g(r, Role::Use);
        }
    }
}

fn list_mut(i: &mut Inst) -> Option<&mut crate::Regs> {
    match i {
        Inst::MakeDope { parts, .. } | Inst::Cat { parts, .. } => Some(parts),
        Inst::Call { args, .. }
        | Inst::CallNative { args, .. }
        | Inst::CallForeign { args, .. }
        | Inst::CallIndirect { args, .. }
        | Inst::CallProt { args, .. }
        | Inst::Elab { args, .. } => Some(args),
        _ => None,
    }
}

fn successors(prog: &Program, f: &crate::Func) -> Vec<Vec<usize>> {
    f.blocks
        .values()
        .iter()
        .map(|b| {
            if b.end <= b.start {
                return Vec::new();
            }
            match f.code[b.end as usize - 1] {
                Inst::Case { table, .. } => prog.cases.get(table).map_or_else(Vec::new, |t| {
                    t.arms
                        .iter()
                        .map(|a| a.2.0 as usize)
                        .chain(std::iter::once(t.default.0 as usize))
                        .collect()
                }),
                ref i => i.targets().iter().map(|t| t.0 as usize).collect(),
            }
        })
        .collect()
}

/// Is block `b` on a cycle (the process loop, or a loop inside it)?
fn on_cycle(succ: &[Vec<usize>], b: usize) -> bool {
    let mut seen = vec![false; succ.len()];
    let mut stack: Vec<usize> = succ[b].clone();
    while let Some(x) = stack.pop() {
        if x == b {
            return true;
        }
        if x < seen.len() && !seen[x] {
            seen[x] = true;
            stack.extend(&succ[x]);
        }
    }
    false
}

/// Registers live out of each block (bit sets).
fn live_out(f: &crate::Func, succ: &[Vec<usize>]) -> Vec<Vec<u64>> {
    let n = f.reg_ty.len();
    let w = n.div_ceil(64).max(1);
    let nb = f.blocks.len();
    let mut gen_ = vec![vec![0u64; w]; nb];
    let mut kill = vec![vec![0u64; w]; nb];
    for (b, blk) in f.blocks.values().iter().enumerate() {
        for pc in blk.start as usize..blk.end as usize {
            let mut uses = Vec::new();
            let mut defs = Vec::new();
            operands(f, &f.code[pc], |r, role| match role {
                Role::Use => uses.push(r.0 as usize),
                Role::Def => defs.push(r.0 as usize),
            });
            for r in uses {
                if kill[b][r / 64] & (1 << (r % 64)) == 0 {
                    gen_[b][r / 64] |= 1 << (r % 64);
                }
            }
            for r in defs {
                kill[b][r / 64] |= 1 << (r % 64);
            }
        }
    }
    let mut live_in = vec![vec![0u64; w]; nb];
    let mut out = vec![vec![0u64; w]; nb];
    let mut changed = true;
    while changed {
        changed = false;
        for b in (0..nb).rev() {
            let mut o = vec![0u64; w];
            for &s in &succ[b] {
                if s < nb {
                    for k in 0..w {
                        o[k] |= live_in[s][k];
                    }
                }
            }
            let inn: Vec<u64> = (0..w).map(|k| gen_[b][k] | (o[k] & !kill[b][k])).collect();
            if inn != live_in[b] || o != out[b] {
                live_in[b] = inn;
                out[b] = o;
                changed = true;
            }
        }
    }
    out
}

fn bit(s: &[u64], r: u32) -> bool {
    s.get(r as usize / 64)
        .is_some_and(|w| w & (1 << (r % 64)) != 0)
}

/// Classify one instruction given which registers hold instance-constant values (`inv`) and
/// which of them point into frozen memory (`ro`). Returns the kind and, for the registers
/// it defines, whether they end up instance-constant and read-only addresses.
fn classify(f: &crate::Func, i: &Inst, inv: &[bool], ro: &[bool]) -> (K, bool) {
    let ok = |r: Reg| inv.get(r.0 as usize).copied().unwrap_or(false);
    let rof = |r: Reg| ro.get(r.0 as usize).copied().unwrap_or(false);
    let plain = |r: Reg| matches!(f.reg_ty.get(r.0 as usize), Some(RegTy::I64 | RegTy::Ptr));
    match *i {
        Inst::Const { .. } | Inst::ConstF { .. } => (K::Free, false),
        Inst::AddrOf { space, .. } => (
            K::Free,
            matches!(space, AddrSpace::Inst | AddrSpace::Const(_)),
        ),
        Inst::Move { src, .. } if ok(src) => (K::Free, rof(src)),
        Inst::Load { dst, addr, w } if w != Width::Sig && rof(addr) && plain(dst) => {
            (K::Memo, false)
        }
        Inst::Bin { dst, a, b, .. } if ok(a) && ok(b) && plain(dst) => (K::Memo, false),
        Inst::BinRange {
            dst, a, b, lo, hi, ..
        } if ok(a) && ok(b) && ok(lo) && ok(hi) && plain(dst) => (K::Memo, false),
        Inst::Un { dst, a, .. } if ok(a) && plain(dst) => (K::Memo, false),
        Inst::Cmp { dst, a, b, .. } | Inst::Bool { dst, a, b, .. }
            if ok(a) && ok(b) && plain(dst) =>
        {
            (K::Memo, false)
        }
        Inst::Select { dst, c, a, b } if ok(c) && ok(a) && ok(b) && plain(dst) => (K::Memo, false),
        Inst::Pos { dst, dope, idx, .. } if rof(dope) && ok(idx) && plain(dst) => (K::Memo, false),
        Inst::DopeLen { dst, dope, .. } if rof(dope) && plain(dst) => (K::Memo, false),
        Inst::Index { dst, base, pos, .. } if ok(base) && ok(pos) && plain(dst) => {
            (K::Memo, rof(base))
        }
        Inst::Check {
            v, lo, hi, kind, ..
        } if ok(v)
            && (kind == CheckKind::Null || ok(lo))
            && (!matches!(kind, CheckKind::Range | CheckKind::Index) || ok(hi)) =>
        {
            (K::Memo, false)
        }
        _ => (K::Other, false),
    }
}

fn plan_block(f: &crate::Func, b: usize, live_out: &[u64]) -> Option<Plan> {
    let blk = f.blocks.values()[b];
    let (s, e) = (blk.start as usize, blk.end as usize);
    let n = e.checked_sub(s)?;
    if !(3..=MAX_BLOCK).contains(&n) {
        return None;
    }
    let insts = &f.code[s..e];
    if insts.iter().any(|i| matches!(i, Inst::Cover { .. })) {
        return None;
    }
    let nregs = f.reg_ty.len();
    let mut inv = vec![false; nregs];
    let mut ro = vec![false; nregs];
    let mut kind = vec![K::Other; n];
    for (i, ins) in insts.iter().enumerate() {
        let (k, is_ro) = classify(f, ins, &inv, &ro);
        kind[i] = k;
        operands(f, ins, |r, role| {
            if role == Role::Def
                && let (Some(a), Some(c)) = (inv.get_mut(r.0 as usize), ro.get_mut(r.0 as usize))
            {
                *a = k != K::Other;
                *c = is_ro;
            }
        });
    }
    let first = kind.iter().position(|&k| k == K::Memo)?;
    // Backward over the rest of the block: which instructions the fast copy needs.
    let mut live: Vec<bool> = (0..nregs as u32).map(|r| bit(live_out, r)).collect();
    let mut keep = vec![true; n];
    let mut root = vec![false; n];
    let mut gain = 0usize;
    for i in (first..n).rev() {
        let ins = &insts[i];
        let mut defs = Vec::new();
        let mut uses = Vec::new();
        operands(f, ins, |r, role| match role {
            Role::Def => defs.push(r.0 as usize),
            Role::Use => uses.push(r.0 as usize),
        });
        let needed = defs.iter().any(|&d| live[d]);
        match kind[i] {
            K::Other => {}
            K::Free => keep[i] = needed,
            K::Memo => {
                keep[i] = false;
                if needed {
                    let d = defs[0];
                    if f.reg_ty[d] != RegTy::I64 {
                        return None;
                    }
                    root[i] = true;
                    keep[i] = true;
                } else {
                    gain += 1;
                }
            }
        }
        for &d in &defs {
            live[d] = false;
        }
        if keep[i] && !root[i] {
            for &u in &uses {
                live[u] = true;
            }
        }
    }
    let roots = root.iter().filter(|&&r| r).count();
    if roots > MAX_ROOTS || gain < roots + MIN_GAIN {
        return None;
    }
    let last_memo = kind.iter().rposition(|&k| k == K::Memo)?;
    // Registers the slow copy can rename: defined in the rest, neither read before that
    // definition (in the slow copy, which keeps every instruction) nor read after the block.
    let mut slow: Vec<bool> = (0..nregs as u32).map(|r| bit(live_out, r)).collect();
    let mut defined = vec![false; nregs];
    for ins in insts[first..].iter().rev() {
        let mut uses = Vec::new();
        operands(f, ins, |r, role| match role {
            Role::Def => {
                defined[r.0 as usize] = true;
                slow[r.0 as usize] = false;
            }
            Role::Use => uses.push(r.0 as usize),
        });
        for u in uses {
            slow[u] = true;
        }
    }
    let rename = (0..nregs as u32)
        .filter(|&r| defined[r as usize] && !bit(live_out, r) && !slow[r as usize])
        .collect();
    Some(Plan {
        block: b,
        first,
        kind,
        keep,
        root,
        last_memo,
        rename,
    })
}

/// Apply the pass to process `f`.
pub(super) fn run(prog: &Program, f: &mut crate::Func) {
    if f.kind != FuncKind::Process || f.code.len() > 6000 {
        return;
    }
    let succ = successors(prog, f);
    let nb = f.blocks.len();
    // Cheap test first: some cyclic block with a load from frozen memory.
    let candidate = |b: usize| {
        let blk = f.blocks.values()[b];
        f.code[blk.start as usize..blk.end as usize]
            .iter()
            .any(|i| {
                matches!(
                    i,
                    Inst::AddrOf {
                        space: AddrSpace::Inst | AddrSpace::Const(_),
                        ..
                    }
                )
            })
            && on_cycle(&succ, b)
    };
    let cands: Vec<usize> = (0..nb).filter(|&b| candidate(b)).collect();
    if cands.is_empty() {
        return;
    }
    let out = live_out(f, &succ);
    let plans: Vec<Plan> = cands
        .into_iter()
        .filter_map(|b| plan_block(f, b, &out[b]))
        .collect();
    if plans.is_empty() {
        return;
    }
    let base = f.frame.next_multiple_of(8);
    let mut slots = 0u32;
    let mut code: Vec<Inst> = Vec::with_capacity(f.code.len() + 64);
    let mut loc = Vec::with_capacity(code.capacity());
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    let mut extra: Vec<(u32, u32)> = Vec::new();
    let fresh = |f: &mut crate::Func, t: RegTy| -> Reg {
        let r = Reg(f.reg_ty.len() as u32);
        f.reg_ty.push(t);
        if t == RegTy::Sig {
            f.reg_ty.push(RegTy::I64);
        }
        r
    };
    let mut by_block: Vec<Option<&Plan>> = vec![None; nb];
    for p in &plans {
        by_block[p.block] = Some(p);
    }
    // New blocks are numbered from `nb`; every plan adds two (fast, slow).
    let mut next_block = nb as u32;
    let mut new_blocks: Vec<(Part, Part)> = Vec::new();
    let old_blocks: Vec<(u32, u32)> = f.blocks.values().iter().map(|b| (b.start, b.end)).collect();
    for (b, &(s, e)) in old_blocks.iter().enumerate() {
        let start = code.len() as u32;
        let Some(p) = by_block[b] else {
            code.extend_from_slice(&f.code[s as usize..e as usize]);
            loc.extend_from_slice(&f.loc[s as usize..e as usize]);
            ranges.push((start, code.len() as u32));
            continue;
        };
        let (s, e) = (s as usize, e as usize);
        let first_pc = s + p.first;
        code.extend_from_slice(&f.code[s..first_pc]);
        loc.extend_from_slice(&f.loc[s..first_pc]);
        let st = f.loc[first_pc];
        let (fast_id, slow_id) = (next_block, next_block + 1);
        next_block += 2;
        let flag_off = base + 8 * slots;
        slots += 1;
        let rf = fresh(f, RegTy::Ptr);
        let rv = fresh(f, RegTy::I64);
        for i in [
            Inst::AddrOf {
                dst: rf,
                space: AddrSpace::Frame,
                off: flag_off,
            },
            Inst::Load {
                dst: rv,
                addr: rf,
                w: Width::B8,
            },
            Inst::CondBr {
                c: rv,
                t: crate::BlockId(fast_id),
                f: crate::BlockId(slow_id),
            },
        ] {
            code.push(i);
            loc.push(st);
        }
        ranges.push((start, code.len() as u32));
        // Slots of the roots, in order.
        let mut root_off = vec![0u32; e - s];
        for i in p.first..e - s {
            if p.root[i] {
                root_off[i] = base + 8 * slots;
                slots += 1;
            }
        }
        // Fast copy.
        let mut fast = Vec::new();
        for i in p.first..e - s {
            let ins = f.code[s + i];
            let l = f.loc[s + i];
            if p.root[i] {
                let mut d = Reg(0);
                ins.visit(&mut |r, role, _| {
                    if role == Role::Def {
                        d = r;
                    }
                });
                let ra = fresh(f, RegTy::Ptr);
                fast.push((
                    Inst::AddrOf {
                        dst: ra,
                        space: AddrSpace::Frame,
                        off: root_off[i],
                    },
                    l,
                ));
                fast.push((
                    Inst::Load {
                        dst: d,
                        addr: ra,
                        w: Width::B8,
                    },
                    l,
                ));
            } else if p.keep[i] || p.kind[i] == K::Other {
                fast.push((ins, l));
            }
        }
        // Slow copy: the original, renamed, with the stores.
        let mut map: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        for &r in &p.rename {
            let t = f.reg_ty[r as usize];
            let nr = fresh(f, t);
            map.insert(r, nr.0);
        }
        let scratch = fresh(f, RegTy::Ptr);
        let one = fresh(f, RegTy::I64);
        let mut slow = Vec::new();
        for i in p.first..e - s {
            let mut ins = f.code[s + i];
            let l = f.loc[s + i];
            ins.visit_mut(&mut |r, _, _| {
                if let Some(&m) = map.get(&r.0) {
                    r.0 = m;
                }
            });
            if let Some(list) = ins.list()
                && let Some(rs) = f.regs(list).map(<[Reg]>::to_vec)
                && rs.iter().any(|r| map.contains_key(&r.0))
            {
                let start = f.operands.len() as u32;
                f.operands.extend(
                    rs.iter()
                        .map(|r| Reg(map.get(&r.0).copied().unwrap_or(r.0))),
                );
                if let Some(slot) = list_mut(&mut ins) {
                    *slot = crate::Regs {
                        start,
                        len: rs.len() as u32,
                    };
                }
            }
            slow.push((ins, l));
            if p.root[i] {
                let mut d = Reg(0);
                ins.visit(&mut |r, role, _| {
                    if role == Role::Def {
                        d = r;
                    }
                });
                slow.push((
                    Inst::AddrOf {
                        dst: scratch,
                        space: AddrSpace::Frame,
                        off: root_off[i],
                    },
                    l,
                ));
                slow.push((
                    Inst::Store {
                        addr: scratch,
                        src: d,
                        w: Width::B8,
                    },
                    l,
                ));
            }
            if i == p.last_memo {
                slow.push((Inst::Const { dst: one, v: 1 }, l));
                slow.push((
                    Inst::AddrOf {
                        dst: scratch,
                        space: AddrSpace::Frame,
                        off: flag_off,
                    },
                    l,
                ));
                slow.push((
                    Inst::Store {
                        addr: scratch,
                        src: one,
                        w: Width::B8,
                    },
                    l,
                ));
            }
        }
        new_blocks.push((fast, slow));
    }
    for (fast, slow) in new_blocks {
        for part in [fast, slow] {
            let start = code.len() as u32;
            for (i, l) in part {
                code.push(i);
                loc.push(l);
            }
            extra.push((start, code.len() as u32));
        }
    }
    for (i, &(s, e)) in ranges.iter().enumerate() {
        if let Some(b) = f.blocks.get_mut(crate::BlockId(i as u32)) {
            b.start = s;
            b.end = e;
        }
    }
    for (s, e) in extra {
        f.blocks.push(crate::Block { start: s, end: e });
    }
    f.code = code;
    f.loc = loc;
    f.frame = base + 8 * slots;
}
