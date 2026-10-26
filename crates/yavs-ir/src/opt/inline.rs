//! Inlining of small subprograms: a call of a function whose body reads its parameters,
//! computes, branches a little and calls one or two more (the package function in front of a
//! protected method, an accessor, the method itself) becomes that code in the caller.
//!
//! A call costs about 500 host instructions, as much as ten to twenty operations (frame,
//! registers, argument slots, return), and a library testbench is a chain of such calls. The pass runs last, on the finished code of a
//! subprogram (after the other passes and the renumbering), and takes the callee from its own
//! optimised code, made apart from the callee's published copy (`forwarder`), so the result
//! does not depend on which function was optimised first. (The reads of result tables are the
//! exception: `tab_calls` makes them before the first pass, #744, #769.)
//!
//! What stays as it was: the callee's checks and calls, in their order, and what a trace shows.
//! An inlined instruction keeps the statement of the callee and `Func::via` names the call it
//! came from, so a runtime error lists the callee as a frame of its own (`Machine::trace`).
//! Callees that allocate temporaries, have statements of their own to free (their `Stmt`
//! markers go: a new frame has no temporaries to free), touch their frame beyond reading the
//! parameters, suspend or are nested are not inlined. `YAVS_NO_INLINE=1` turns it off.
//!
//! A body of one path is spliced in where the call was. A body with branches (a protected
//! method: a case on the storage mode, one arm calling on) becomes blocks appended to the
//! caller after its last block, so existing block ids, which the program's case tables name,
//! never move: the call's block is cut at the call and jumps to the copy of the callee's first
//! block, every `Ret` becomes a move of the result and a jump to the block that holds the rest
//! of the cut block, and a `Case` of the callee a chain of compares (a table cannot be added to
//! the program here). The protected object of a `CallProt` is the callee's first argument.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use crate::{
    AddrSpace, BlockId, BoolOp, CmpOp, FuncId, FuncKind, Inst, Reg, RegTy, Role, StmtId, ViaNode,
    Width,
};

/// The most instructions of an inlined body of one path.
const MAX_BODY: usize = 24;
/// The most instructions and blocks of an inlined body that branches. (Bigger bodies, such as
/// the 61-instruction accessors of a pointer table, stay calls: inlined five times into one
/// function they make a unit the compiled tier needs ten times as long to compile and runs
/// 40 percent slower in `--tier=auto`, measured on the queue of a verification library.)
const MAX_MULTI: usize = 60;
const MAX_BLOCKS: usize = 8;
/// Bit of a forwarder's level in the cache: made for a call that passes a string it builds.
const WIDE: u8 = 0x80;
/// The most arms of a `Case` that becomes compares.
const MAX_ARMS: usize = 6;
/// The most instructions inlining adds to one function, beyond four times its own size.
const GROWTH: usize = 800;

/// An instruction, its statement and the call it was inlined through (0: none, else 1 and an
/// index in the `via_nodes` it belongs to).
type Ent = (Inst, StmtId, u32);

/// The part of a callee that is inlined: its code with the parameter loads taken out, as
/// blocks.
pub(crate) struct Forwarder {
    /// The blocks (callee order, the first is the entry), each ending in a `Br`, `CondBr`,
    /// `Ret` or `Unreachable`; no frame addresses, parameter loads, statement markers or
    /// `Case`.
    blocks: Vec<Vec<Ent>>,
    /// The callee's operand lists.
    operands: Vec<Reg>,
    reg_ty: Vec<RegTy>,
    /// The calls inlined in the body (a body with calls of its own that were inlined), by
    /// the `Ent` indices.
    via_nodes: Vec<ViaNode>,
    /// For each parameter the registers its loads defined.
    params: Vec<Vec<Reg>>,
    /// How many instructions of the body define each register.
    defs: Vec<u8>,
}

impl Forwarder {
    fn len(&self) -> usize {
        self.blocks.iter().map(Vec::len).sum()
    }

    /// One path ending in the `Ret`: spliced in without new blocks.
    fn straight(&self) -> bool {
        self.blocks.len() == 1 && matches!(self.blocks[0].last(), Some((Inst::Ret { .. }, ..)))
    }
}

/// Forwarders by callee, made once per program (a copy of the program starts without them).
#[derive(Default, Debug)]
pub struct FwdCache(Mutex<HashMap<(u32, u8), Fwd>>);

type Fwd = Option<Arc<Forwarder>>;

impl Clone for FwdCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl std::fmt::Debug for Forwarder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Forwarder({} instructions)", self.len())
    }
}

/// Is inlining on? `YAVS_NO_INLINE` set to anything but `0` or the empty string turns it off.
pub(super) fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("YAVS_NO_INLINE")
            .map(|v| v.is_empty() || v == "0")
            .unwrap_or(true)
    })
}

/// The inlinable body of `callee`, if it has one: at level 0 its own optimised code, at level 1
/// that with the calls inlined that level 0 makes inlinable. (A level is a function of the
/// program alone, so the result does not depend on the order functions are first used, and a
/// cycle of calls ends at level 0.)
fn forwarder(
    prog: &crate::Program,
    callee: FuncId,
    level: u8,
    wide: bool,
) -> Option<Arc<Forwarder>> {
    let key = (callee.0, level | if wide { WIDE } else { 0 });
    if let Some(r) = prog
        .fwd
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key)
    {
        return r.clone();
    }
    // Not under the lock: the optimiser takes the program's other locks.
    let made = make(prog, callee, level, wide).map(Arc::new);
    prog.fwd
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key, made.clone());
    made
}

fn make(prog: &crate::Program, callee: FuncId, level: u8, wide: bool) -> Option<Forwarder> {
    let raw = prog.funcs.get(callee)?;
    if let Some(t) = prog.tabs.get(&callee.0)
        && crate::casetab::enabled()
        && !raw.flags.contains(crate::FuncFlags::HAS_COVER)
    {
        return tab_forwarder(prog, callee, t);
    }
    // Cheap exits before optimising a function: not a small subprogram of the plain kind.
    if !matches!(raw.kind, FuncKind::Function | FuncKind::Procedure)
        || raw.nested
        || raw.parent.is_some()
        || raw.body.is_some()
        || raw.flags.contains(crate::FuncFlags::MAY_SUSPEND)
        || raw
            .params
            .iter()
            .any(|p| p.width != Width::B8 || p.width == Width::Sig)
        || size(&raw.full().code) > if wide { 4 } else { 2 } * MAX_MULTI
    {
        return None;
    }
    let mut c = super::optimize_body(prog, callee)?;
    // (A body without calls has nothing to inline: the common leaf costs no second walk.)
    if level > 0
        && c.code
            .iter()
            .any(|i| matches!(i, Inst::Call { .. } | Inst::CallProt { .. }))
    {
        run(prog, callee, &mut c, level - 1);
    }
    if c.reg_ty.contains(&RegTy::Sig)
        || size(&c.code) > 4 * MAX_MULTI
        || c.blocks.len() > 4 * MAX_BLOCKS
    {
        return None;
    }
    // The registers holding the address of a parameter's slot.
    let mut frame: HashMap<u32, u32> = HashMap::new();
    for i in &c.code {
        if let Inst::AddrOf {
            dst,
            space: AddrSpace::Frame,
            off,
        } = *i
        {
            frame.insert(dst.0, off);
        }
    }
    let slot = |off: u32| c.params.iter().position(|p| p.slot == off);
    let mut params: Vec<Vec<Reg>> = vec![Vec::new(); c.params.len()];
    let mut reg_ty = c.reg_ty.clone();
    let mut blocks: Vec<Vec<Ent>> = Vec::with_capacity(c.blocks.len());
    // Blocks the converted `Case`s add: (index, entries).
    let mut extra: Vec<Vec<Ent>> = Vec::new();
    let nb = c.blocks.len();
    for (b, blk) in c.blocks.iter() {
        let mut out: Vec<Ent> = Vec::new();
        for pc in blk.start as usize..blk.end as usize {
            let i = *c.code.get(pc)?;
            let loc = *c.loc.get(pc)?;
            let via = c.via.get(pc).copied().unwrap_or(0);
            match i {
                Inst::Stmt => continue,
                Inst::AddrOf {
                    space: AddrSpace::Frame,
                    ..
                } => continue,
                Inst::Load { dst, addr, w } if frame.contains_key(&addr.0) => {
                    // Parameters are read once, at the start of the entry block.
                    let p = slot(*frame.get(&addr.0)?)?;
                    if w != Width::B8 || b.0 != 0 {
                        return None;
                    }
                    params[p].push(dst);
                    continue;
                }
                Inst::Case { v, table } => {
                    let t = prog.cases.get(table)?;
                    if t.arms.len() > MAX_ARMS {
                        return None;
                    }
                    case_chain(v, t, (loc, via), &mut reg_ty, &mut out, &mut extra, nb)?;
                    continue;
                }
                _ => {}
            }
            if !allowed(prog, &i) {
                return None;
            }
            // The frame is touched by the parameter loads only.
            let mut touches = false;
            i.visit(&mut |r, role, _| touches |= role == Role::Use && frame.contains_key(&r.0));
            if let Some(l) = i.list() {
                touches |= c
                    .operands
                    .get(l.start as usize..(l.start + l.len) as usize)?
                    .iter()
                    .any(|r| frame.contains_key(&r.0));
            }
            if touches {
                return None;
            }
            out.push((i, loc, via));
        }
        blocks.push(out);
    }
    blocks.extend(extra);
    // Blocks nothing reaches (the arms of a `Case` that only fail) are not copied.
    let mut seen = vec![false; blocks.len()];
    let mut work = vec![0usize];
    while let Some(b) = work.pop() {
        if std::mem::replace(&mut seen[b], true) {
            continue;
        }
        let (last, ..) = blocks[b].last()?;
        for t in last.targets() {
            let t = t.0 as usize;
            if t >= blocks.len() {
                return None;
            }
            work.push(t);
        }
        if !last.is_terminator() {
            return None;
        }
    }
    let mut renum = vec![u32::MAX; blocks.len()];
    let mut kept: Vec<Vec<Ent>> = Vec::new();
    for (b, blk) in blocks.into_iter().enumerate() {
        if seen[b] {
            renum[b] = kept.len() as u32;
            kept.push(blk);
        }
    }
    for blk in &mut kept {
        if let Some((last, ..)) = blk.last_mut() {
            match last {
                Inst::Br { to } => *to = BlockId(renum[to.0 as usize]),
                Inst::CondBr { t, f, .. } => {
                    *t = BlockId(renum[t.0 as usize]);
                    *f = BlockId(renum[f.0 as usize]);
                }
                _ => {}
            }
        }
    }
    let total: usize = kept
        .iter()
        .map(|b| b.iter().filter(|e| !is_cov(&e.0)).count())
        .sum();
    let limit = if kept.len() == 1 { MAX_BODY } else { MAX_MULTI };
    if total > limit || kept.len() > MAX_BLOCKS || total == 0 {
        return None;
    }
    // Something must return.
    if !kept
        .iter()
        .any(|b| matches!(b.last(), Some((Inst::Ret { .. }, ..))))
    {
        return None;
    }
    let mut defs = vec![0u8; reg_ty.len()];
    for blk in &kept {
        for (i, ..) in blk {
            i.visit(&mut |r, role, _| {
                if role == Role::Def
                    && let Some(d) = defs.get_mut(r.0 as usize)
                {
                    *d = d.saturating_add(1);
                }
            });
        }
    }
    Some(Forwarder {
        blocks: kept,
        operands: c.operands.clone(),
        reg_ty,
        via_nodes: c.via_nodes.clone(),
        params,
        defs,
    })
}

/// The forwarder of a function with a result table (`casetab.rs`): when the argument is a
/// value the table holds, the result is read from it (an array result as the fat pair the
/// function builds, its data in the pool), else the function is called as before.
fn tab_forwarder(prog: &crate::Program, callee: FuncId, t: &crate::CaseTab) -> Option<Forwarder> {
    use crate::{IntOp, Regs};
    let loc = *prog.funcs.get(callee)?.full().loc.first()?;
    let mut reg_ty: Vec<RegTy> = vec![RegTy::I64];
    let mut fresh = |ty: RegTy| {
        reg_ty.push(ty);
        Reg(reg_ty.len() as u32 - 1)
    };
    let p = Reg(0);
    let (lo, hi) = (fresh(RegTy::I64), fresh(RegTy::I64));
    let (c1, c2, ok, k) = (
        fresh(RegTy::I64),
        fresh(RegTy::I64),
        fresh(RegTy::I64),
        fresh(RegTy::I64),
    );
    let hit_to = BlockId(2);
    let miss = BlockId(3);
    let b0 = if t.whole {
        vec![
            Inst::Const { dst: lo, v: t.lo },
            Inst::Br { to: BlockId(1) },
        ]
    } else {
        vec![
            Inst::Const { dst: lo, v: t.lo },
            Inst::Const {
                dst: hi,
                v: t.lo + i64::from(t.n) - 1,
            },
            Inst::Cmp {
                op: CmpOp::Ge,
                dst: c1,
                a: p,
                b: lo,
            },
            Inst::Cmp {
                op: CmpOp::Le,
                dst: c2,
                a: p,
                b: hi,
            },
            Inst::Bool {
                op: BoolOp::And,
                dst: ok,
                a: c1,
                b: c2,
            },
            Inst::CondBr {
                c: ok,
                t: BlockId(1),
                f: miss,
            },
        ]
    };
    let mut b1 = vec![Inst::Bin {
        op: IntOp::Sub,
        dst: k,
        a: p,
        b: lo,
    }];
    if let Some(v) = t.valid {
        let (vt, vp, vv, one) = (
            fresh(RegTy::Ptr),
            fresh(RegTy::Ptr),
            fresh(RegTy::I64),
            fresh(RegTy::I64),
        );
        let _ = one;
        b1.extend([
            Inst::AddrOf {
                dst: vt,
                space: AddrSpace::Const(v),
                off: 0,
            },
            Inst::Index {
                dst: vp,
                base: vt,
                pos: k,
                stride: 1,
            },
            Inst::Load {
                dst: vv,
                addr: vp,
                w: Width::B1,
            },
            Inst::CondBr {
                c: vv,
                t: hit_to,
                f: miss,
            },
        ]);
    } else {
        b1.push(Inst::Br { to: hit_to });
    }
    let (tb, tp) = (fresh(RegTy::Ptr), fresh(RegTy::Ptr));
    let mut b2 = vec![
        Inst::AddrOf {
            dst: tb,
            space: AddrSpace::Const(t.table),
            off: 0,
        },
        Inst::Index {
            dst: tp,
            base: tb,
            pos: k,
            stride: t.stride,
        },
    ];
    let res;
    if let Some(d) = t.desc {
        let (dd, c16, pr, c8, q) = (
            fresh(RegTy::Ptr),
            fresh(RegTy::I64),
            fresh(RegTy::Ptr),
            fresh(RegTy::I64),
            fresh(RegTy::Ptr),
        );
        b2.extend([
            Inst::AddrOf {
                dst: dd,
                space: AddrSpace::Const(d),
                off: 0,
            },
            Inst::Const { dst: c16, v: 16 },
            Inst::Alloc { dst: pr, size: c16 },
            Inst::Store {
                addr: pr,
                src: tp,
                w: Width::B8,
            },
            Inst::Const { dst: c8, v: 8 },
            Inst::Index {
                dst: q,
                base: pr,
                pos: c8,
                stride: 1,
            },
            Inst::Store {
                addr: q,
                src: dd,
                w: Width::B8,
            },
            Inst::TempRoot {
                addr: pr,
                desc: None,
                ty: None,
            },
        ]);
        res = pr;
    } else {
        res = fresh(RegTy::I64);
        b2.push(Inst::Load {
            dst: res,
            addr: tp,
            w: Width::B8,
        });
    }
    b2.push(Inst::Ret { v: Some(res) });
    let cres = fresh(if t.ptr { RegTy::Ptr } else { RegTy::I64 });
    let b3 = vec![
        Inst::Call {
            func: callee,
            args: Regs { start: 0, len: 1 },
            dst: Some(cres),
        },
        Inst::Ret { v: Some(cres) },
    ];
    // A table that holds every value of the type never calls: one path, spliced in place, so the
    // caller's loads of the result see the stores that built it (no join with a dead call path).
    let lists: Vec<Vec<Inst>> = if t.whole && t.valid.is_none() {
        let one = [b0, b1, b2]
            .into_iter()
            .flatten()
            .filter(|i| !matches!(i, Inst::Br { .. }))
            .collect();
        vec![one]
    } else {
        vec![b0, b1, b2, b3]
    };
    let blocks: Vec<Vec<Ent>> = lists
        .into_iter()
        .map(|b| b.into_iter().map(|i| (i, loc, 0)).collect())
        .collect();
    let mut defs = vec![0u8; reg_ty.len()];
    for blk in &blocks {
        for (i, ..) in blk {
            i.visit(&mut |r, role, _| {
                if role == Role::Def
                    && let Some(d) = defs.get_mut(r.0 as usize)
                {
                    *d = d.saturating_add(1);
                }
            });
        }
    }
    Some(Forwarder {
        blocks,
        operands: vec![p],
        reg_ty,
        via_nodes: Vec::new(),
        params: vec![vec![p]],
        defs,
    })
}

/// A `Case` as compares and branches: the entries that follow the block's own in `out` (the
/// test of the first arm), and new blocks for the tests of the others (`extra`; block `nb + i`
/// is `extra[i]`). The arms are sorted and disjoint, so the tests run in turn and the last
/// falls to the default.
fn case_chain(
    v: Reg,
    t: &crate::CaseTable,
    at: (StmtId, u32),
    reg_ty: &mut Vec<RegTy>,
    out: &mut Vec<Ent>,
    extra: &mut Vec<Vec<Ent>>,
    nb: usize,
) -> Option<()> {
    fn fresh(reg_ty: &mut Vec<RegTy>) -> Reg {
        let r = Reg(reg_ty.len() as u32);
        reg_ty.push(RegTy::I64);
        r
    }
    let (loc, via) = at;
    if t.arms.is_empty() {
        out.push((Inst::Br { to: t.default }, loc, via));
        return Some(());
    }
    let first_extra = nb + extra.len();
    let mut tests: Vec<Vec<Ent>> = Vec::with_capacity(t.arms.len());
    for (k, &(lo, hi, to)) in t.arms.iter().enumerate() {
        let mut cur: Vec<Ent> = if k == 0 {
            std::mem::take(out)
        } else {
            Vec::new()
        };
        let hit = if lo == hi {
            let (kr, c) = (fresh(reg_ty), fresh(reg_ty));
            cur.push((Inst::Const { dst: kr, v: lo }, loc, via));
            let cmp = Inst::Cmp {
                op: CmpOp::Eq,
                dst: c,
                a: v,
                b: kr,
            };
            cur.push((cmp, loc, via));
            c
        } else {
            let (k1, k2, c1, c2, c) = (
                fresh(reg_ty),
                fresh(reg_ty),
                fresh(reg_ty),
                fresh(reg_ty),
                fresh(reg_ty),
            );
            cur.push((Inst::Const { dst: k1, v: lo }, loc, via));
            cur.push((Inst::Const { dst: k2, v: hi }, loc, via));
            let ge = Inst::Cmp {
                op: CmpOp::Ge,
                dst: c1,
                a: v,
                b: k1,
            };
            let le = Inst::Cmp {
                op: CmpOp::Le,
                dst: c2,
                a: v,
                b: k2,
            };
            let both = Inst::Bool {
                op: BoolOp::And,
                dst: c,
                a: c1,
                b: c2,
            };
            cur.extend([(ge, loc, via), (le, loc, via), (both, loc, via)]);
            c
        };
        let next = if k + 1 == t.arms.len() {
            t.default
        } else {
            BlockId((first_extra + k) as u32)
        };
        cur.push((
            Inst::CondBr {
                c: hit,
                t: to,
                f: next,
            },
            loc,
            via,
        ));
        tests.push(cur);
    }
    let mut it = tests.into_iter();
    *out = it.next()?;
    extra.extend(it);
    Some(())
}

/// The instructions of a body that count against the size limits: not the coverage counters, so a
/// run with counters inlines what a run without them does.
fn size(code: &[Inst]) -> usize {
    code.iter().filter(|i| !is_cov(i)).count()
}

/// A coverage counter instruction.
fn is_cov(i: &Inst) -> bool {
    matches!(
        i,
        Inst::Cover { .. } | Inst::CoverIn { .. } | Inst::CoverCond { .. }
    )
}

/// Instructions an inlined body may have: reads, computations, checks, calls, branches.
fn allowed(prog: &crate::Program, i: &Inst) -> bool {
    match *i {
        Inst::Const { .. }
        | Inst::ConstF { .. }
        | Inst::Move { .. }
        | Inst::Select { .. }
        | Inst::Index { .. }
        | Inst::Field { .. }
        | Inst::Bin { .. }
        | Inst::BinRange { .. }
        | Inst::Un { .. }
        | Inst::Cmp { .. }
        | Inst::Bool { .. }
        | Inst::Check { .. }
        | Inst::Pos { .. }
        | Inst::SlicePos { .. }
        | Inst::DopeLen { .. }
        | Inst::Deref { .. }
        | Inst::LGet { .. }
        | Inst::LEq { .. }
        | Inst::MemEq { .. }
        | Inst::SubtypeCheck { .. }
        | Inst::FBin { .. }
        | Inst::FUn { .. }
        | Inst::FCmp { .. }
        | Inst::IToF { .. }
        | Inst::FToI { .. }
        | Inst::Br { .. }
        | Inst::CondBr { .. }
        | Inst::Unreachable { .. }
        | Inst::Ret { .. } => true,
        // (Counters add to the instance's row at run time, so inlined they count the same.)
        _ if is_cov(i) => true,
        Inst::AddrOf { space, .. } => !matches!(space, AddrSpace::Frame | AddrSpace::Up(_)),
        Inst::Load { w, .. } | Inst::Store { w, .. } => w != Width::Sig,
        // (A native runs the same in the caller's frame; the record of the inlined call keeps
        // its call path.)
        Inst::CallNative { .. } => true,
        Inst::Call { func, .. } | Inst::CallProt { func, .. } => prog
            .funcs
            .get(func)
            .is_some_and(|c| !c.flags.contains(crate::FuncFlags::MAY_SUSPEND)),
        _ => false,
    }
}

/// Does the call at `pc` take an argument that `Cat` filled earlier in the same block?
fn builds_string(f: &crate::Func, pc: usize, args: crate::Regs, obj: Option<Reg>) -> bool {
    let mut want: Vec<Reg> = obj.into_iter().collect();
    if let Some(a) = f
        .operands
        .get(args.start as usize..(args.start + args.len) as usize)
    {
        want.extend_from_slice(a);
    }
    for i in f.code[..pc].iter().rev().take(800) {
        if i.is_terminator() {
            return false;
        }
        if let Inst::Cat { dst, .. } = *i
            && want.contains(&dst)
        {
            return true;
        }
        i.visit(&mut |r, role, _| {
            if role == Role::Def {
                want.retain(|w| *w != r);
            }
        });
        if want.is_empty() {
            return false;
        }
    }
    false
}

/// A call to inline: the callee's body, the arguments in the order of its parameters (the
/// protected object of a `CallProt` first) and the register receiving the result.
struct Site {
    fw: Arc<Forwarder>,
    func: FuncId,
    args: Vec<Reg>,
    dst: Option<Reg>,
    /// A table read with branches that takes the rest of its block into the arm that reads the table.
    dup: bool,
}

/// Where each register of the body lives in the caller at one site: a parameter the body never
/// writes is the argument itself, any other register is a new one.
fn place(site: &Site, reg_ty: &mut Vec<RegTy>) -> HashMap<u32, Reg> {
    let fw = &site.fw;
    let mut at: HashMap<u32, Reg> = HashMap::new();
    for (k, regs) in fw.params.iter().enumerate() {
        for &r in regs {
            if fw.defs[r.0 as usize] == 0 {
                at.insert(r.0, site.args[k]);
            }
        }
    }
    let mut ret_into = None;
    // One path: the returned value is made in the result register when nothing else is.
    if fw.straight()
        && let (Some(d), Some((Inst::Ret { v: Some(r) }, ..))) = (site.dst, fw.blocks[0].last())
    {
        let is_arg = site.args.contains(&d);
        let is_param = fw.params.iter().any(|p| p.contains(r));
        if fw.defs[r.0 as usize] == 1 && !is_param && !is_arg {
            ret_into = Some((r.0, d));
        }
    }
    if let Some((r, d)) = ret_into {
        at.insert(r, d);
    }
    let mut used: Vec<u32> = Vec::new();
    for blk in &fw.blocks {
        for (inst, ..) in blk {
            inst.visit(&mut |r, _, _| used.push(r.0));
            if let Some(l) = inst.list() {
                used.extend(
                    fw.operands[l.start as usize..(l.start + l.len) as usize]
                        .iter()
                        .map(|r| r.0),
                );
            }
        }
    }
    // The registers the parameter loads defined and the body reads.
    used.sort_unstable();
    used.dedup();
    for r in used {
        if let std::collections::hash_map::Entry::Vacant(e) = at.entry(r) {
            e.insert(Reg(reg_ty.len() as u32));
            reg_ty.push(fw.reg_ty[r as usize]);
        }
    }
    at
}

type Out = (Inst, StmtId, u32);

/// Inline the calls of `f` (function `id`) whose callee has a forwarder. `true`: the code
/// changed.
pub(super) fn run(prog: &crate::Program, id: FuncId, f: &mut crate::Func, level: u8) -> bool {
    run_with(prog, id, f, level, Round::All)
}

/// `raw` with the calls of functions that have a result table (`casetab.rs`) replaced by the
/// table read, made before the first pass so that it sees the read where the registers are still
/// single-definition: the loads of an array result then find the stores that built it. `None`
/// when there is no such call.
pub(super) fn tab_calls(
    prog: &crate::Program,
    id: FuncId,
    raw: &crate::Func,
) -> Option<Box<crate::Func>> {
    if !enabled()
        || !crate::casetab::enabled()
        || super::keep_variables_on()
        || !raw.code.iter().any(|i| {
            matches!(i, Inst::Call { func, .. } if *func != id && prog.tabs.contains_key(&func.0))
        })
    {
        return None;
    }
    let mut f = Box::new(raw.clone());
    // (A table read cannot fail, so the trace needs no frame for it: no `via` entries.)
    if !run_with(prog, id, &mut f, 0, Round::Tabs) {
        return None;
    }
    f.via.clear();
    f.via_nodes.clear();
    Some(f)
}

/// Which calls a round of inlining takes.
#[derive(Clone, Copy, PartialEq)]
enum Round {
    All,
    /// Only the calls that hand over a string built just before them.
    Wide,
    /// Only the calls of functions with a result table whose read is one path.
    Tabs,
}

/// The second round over a function that inlined (`Round::Wide`): only the calls that hand over a
/// string built just before them, which the first round may have brought in with the body of a
/// wrapper (a package procedure in front of a protected method).
pub(super) fn run_wide(prog: &crate::Program, id: FuncId, f: &mut crate::Func) -> bool {
    f.code
        .iter()
        .any(|i| matches!(i, Inst::Cat { logic: false, .. }))
        && run_with(prog, id, f, 0, Round::Wide)
}

/// The most instructions of the rest of a block copied for the hit path of a table read.
const MAX_TAIL: usize = 64;

/// Is a call of `func` replaced by the read of its result table (before the first pass,
/// `tab_calls`) instead of being left to the inliner?
fn tabbed(prog: &crate::Program, func: FuncId) -> bool {
    crate::casetab::enabled()
        && prog.tabs.contains_key(&func.0)
        && prog
            .funcs
            .get(func)
            .is_some_and(|c| !c.flags.contains(crate::FuncFlags::HAS_COVER))
}

/// One site expanded: its registers placed in the caller, and what a trace shows.
struct Expand<'a> {
    site: &'a Site,
    at: HashMap<u32, Reg>,
    loc: StmtId,
    sv: u32,
    round: Round,
    outer: u32,
    remap: Vec<u32>,
}

impl<'a> Expand<'a> {
    fn new(
        f: &crate::Func,
        site: &'a Site,
        pc: usize,
        round: Round,
        reg_ty: &mut Vec<RegTy>,
        nodes: &mut Vec<ViaNode>,
    ) -> Self {
        let sv = f.via.get(pc).copied().unwrap_or(0);
        let loc = f.loc[pc];
        let at = place(site, reg_ty);
        // This call, and the calls the body's own inlined instructions came through, now
        // inlined through it as well.
        if round != Round::Tabs {
            nodes.push(ViaNode {
                site: loc,
                callee: site.func,
                up: sv,
            });
        }
        let outer = nodes.len() as u32;
        let mut remap: Vec<u32> = Vec::with_capacity(site.fw.via_nodes.len());
        for n in &site.fw.via_nodes {
            let up = match n.up {
                0 => outer,
                u => remap[u as usize - 1],
            };
            nodes.push(ViaNode { up, ..*n });
            remap.push(nodes.len() as u32);
        }
        Self {
            site,
            at,
            loc,
            sv,
            round,
            outer,
            remap,
        }
    }

    fn map(&self, r: Reg) -> Reg {
        self.at.get(&r.0).copied().unwrap_or(r)
    }

    fn chain(&self, v: u32) -> u32 {
        if v == 0 {
            self.outer
        } else {
            self.remap[v as usize - 1]
        }
    }

    /// (A table read has no frame of its own in a trace: it is the call's statement.)
    fn own(&self, l: StmtId, v: u32) -> (StmtId, u32) {
        if self.round == Round::Tabs {
            (self.loc, self.sv)
        } else {
            (l, self.chain(v))
        }
    }

    /// The other parameters arrive in the registers the loads defined.
    fn param_moves(&self, cur: &mut Vec<Out>) {
        for (k, regs) in self.site.fw.params.iter().enumerate() {
            for &r in regs {
                if self.at.get(&r.0) != Some(&self.site.args[k]) && self.at.contains_key(&r.0) {
                    let mv = Inst::Move {
                        dst: self.map(r),
                        src: self.site.args[k],
                    };
                    cur.push((mv, self.loc, self.sv));
                }
            }
        }
    }

    /// The body's instruction with its registers placed.
    fn place_inst(&self, inst: Inst, operands: &mut Vec<Reg>) -> Inst {
        let mut inst = inst;
        inst.visit_mut(&mut |r, _, _| *r = self.map(*r));
        match &mut inst {
            Inst::Call { args, .. }
            | Inst::CallProt { args, .. }
            | Inst::CallNative { args, .. } => {
                let start = operands.len() as u32;
                let from = args.start as usize..(args.start + args.len) as usize;
                operands.extend(self.site.fw.operands[from].iter().map(|&r| self.map(r)));
                args.start = start;
            }
            _ => {}
        }
        inst
    }

    fn ret_move(&self, inst: &Inst) -> Option<Inst> {
        match (*inst, self.site.dst) {
            (Inst::Ret { v: Some(r) }, Some(d)) if self.map(r) != d => Some(Inst::Move {
                dst: d,
                src: self.map(r),
            }),
            _ => None,
        }
    }

    /// A body of one path, spliced in where the call was.
    fn splice(&self, cur: &mut Vec<Out>, operands: &mut Vec<Reg>) {
        self.param_moves(cur);
        for &(inst, l, v) in &self.site.fw.blocks[0] {
            if let Inst::Ret { .. } = inst {
                if let Some(m) = self.ret_move(&inst) {
                    cur.push((m, self.loc, self.sv));
                }
                continue;
            }
            let (l, v) = self.own(l, v);
            cur.push((self.place_inst(inst, operands), l, v));
        }
    }
}

/// Uses of each register in `code` (operand lists included).
fn use_counts(f: &crate::Func) -> Vec<u32> {
    let mut n = vec![0u32; f.reg_ty.len()];
    for i in &f.code {
        count_uses(f, i, &mut n);
    }
    n
}

fn count_uses(f: &crate::Func, i: &Inst, n: &mut [u32]) {
    i.visit(&mut |r, role, _| {
        if role == Role::Use {
            n[r.0 as usize] += 1;
        }
    });
    if let Some(l) = i.list() {
        for r in f.regs(l).unwrap_or(&[]) {
            n[r.0 as usize] += 1;
        }
    }
}

/// `tail` (the rest of a block, its terminator last) with every register it defines renamed to
/// a new one, so that the copy leaves the original registers to the other copy: the registers
/// the code outside the tail reads (`live_out`), or the tail itself reads before it defines
/// them, are moved back before the terminator. `seed`: a register that already stands for
/// another here (the result of the call, which the table read made).
fn rename_tail(
    tail: &[Out],
    seed: Option<(Reg, Reg)>,
    live_out: &dyn Fn(Reg) -> bool,
    reg_ty: &mut Vec<RegTy>,
    operands: &mut Vec<Reg>,
) -> Vec<Out> {
    let mut map: std::collections::BTreeMap<u32, Reg> = std::collections::BTreeMap::new();
    map.extend(seed.map(|(a, b)| (a.0, b)));
    let mut exposed: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    let mut out: Vec<Out> = Vec::with_capacity(tail.len() + 2);
    for &(mut inst, l, v) in tail {
        let mut see = |r: &mut Reg| match map.get(&r.0) {
            Some(&n) => *r = n,
            None => {
                exposed.insert(r.0);
            }
        };
        inst.visit_mut(&mut |r, role, _| {
            if role == Role::Use {
                see(r);
            }
        });
        if let Some(list) = inst.list_mut() {
            let start = operands.len() as u32;
            for k in list.start as usize..(list.start + list.len) as usize {
                let mut r = operands[k];
                see(&mut r);
                operands.push(r);
            }
            list.start = start;
        }
        inst.visit_mut(&mut |r, role, _| {
            if role == Role::Def {
                let n = Reg(reg_ty.len() as u32);
                reg_ty.push(reg_ty[r.0 as usize]);
                map.insert(r.0, n);
                *r = n;
            }
        });
        out.push((inst, l, v));
    }
    if let Some(term) = out.pop() {
        let at = term.1;
        for (&r, &n) in &map {
            if live_out(Reg(r)) || exposed.contains(&r) {
                let mv = Inst::Move {
                    dst: Reg(r),
                    src: n,
                };
                out.push((mv, at, term.2));
            }
        }
        out.push(term);
    }
    out
}

fn run_with(
    prog: &crate::Program,
    id: FuncId,
    f: &mut crate::Func,
    level: u8,
    round: Round,
) -> bool {
    let wide_only = round == Round::Wide;
    if !enabled()
        || super::keep_variables_on()
        || !matches!(
            f.kind,
            FuncKind::Function | FuncKind::Procedure | FuncKind::Process
        )
    {
        return false;
    }
    // The calls to inline and their callees.
    let mut sites: HashMap<usize, Site> = HashMap::new();
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
        if func == id {
            continue;
        }
        // The reads of result tables are made by the round before the first pass, where the
        // loads of an array result find the stores that built it; a call left in the code
        // (the path for the values the table does not hold) stays one.
        if (round == Round::Tabs) != tabbed(prog, func) {
            continue;
        }
        // A call that hands over a string built just before it (the message of a check): a
        // callee of up to four times the usual raw size, because sinking can then skip
        // building it (`opt/sink.rs`).
        let wide = || {
            f.code
                .iter()
                .any(|i| matches!(i, Inst::Cat { logic: false, .. }))
                && builds_string(f, pc, args, obj)
        };
        let fw = if round == Round::Tabs {
            forwarder(prog, func, 0, false)
        } else if wide_only {
            wide().then(|| forwarder(prog, func, level, true)).flatten()
        } else {
            forwarder(prog, func, level, false)
                .or_else(|| wide().then(|| forwarder(prog, func, level, true)).flatten())
        };
        let Some(fw) = fw else {
            continue;
        };
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
        // One argument for each parameter, the result in a register of the type it is returned in.
        let ty = |r: Reg| f.reg_ty.get(r.0 as usize);
        let typed = sargs.len() == callee.params.len()
            && fw.params.iter().enumerate().all(|(k, regs)| {
                regs.iter()
                    .all(|r| ty(sargs[k]).is_some() && ty(sargs[k]) == fw.reg_ty.get(r.0 as usize))
            });
        let returns = fw.blocks.iter().all(|b| match b.last() {
            Some((Inst::Ret { v }, ..)) => match (dst, v) {
                (Some(d), Some(r)) => ty(d).is_some() && ty(d) == fw.reg_ty.get(r.0 as usize),
                (None, _) => true,
                (Some(_), None) => false,
            },
            _ => true,
        });
        let cost = if fw.straight() { 0 } else { fw.len() + 2 };
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
        return false;
    }
    // A table read that branches (a range test, a validity mask) takes the rest of its block
    // with it into the arm that reads the table, so that the loads of an array result there
    // find the stores that built it (the join of the two arms hides them); a long rest, or one
    // with another such read or a suspension, is joined instead.
    let mut dups: Vec<usize> = Vec::new();
    if round == Round::Tabs {
        let mut end_of = vec![0usize; f.code.len()];
        for blk in f.blocks.values() {
            end_of[blk.start as usize..blk.end as usize].fill(blk.end as usize);
        }
        let wide: Vec<usize> = sites
            .iter()
            .filter(|(_, s)| !s.fw.straight())
            .map(|(&pc, _)| pc)
            .collect();
        for &pc in &wide {
            let end = end_of[pc];
            let rest = &f.code[pc + 1..end];
            if sites[&pc].fw.blocks.len() == 4
                && rest.len() <= MAX_TAIL
                && !wide.iter().any(|&o| o > pc && o < end)
                && !rest.iter().any(|i| matches!(i, Inst::Suspend { .. }))
            {
                dups.push(pc);
            }
        }
        for pc in &dups {
            if let Some(s) = sites.get_mut(pc) {
                s.dup = true;
            }
        }
    }
    let uses = if dups.is_empty() {
        Vec::new()
    } else {
        use_counts(f)
    };
    let nb = f.blocks.len();
    let mut blocks: Vec<Option<Vec<Out>>> = vec![None; nb];
    let mut reg_ty = f.reg_ty.clone();
    let mut operands = f.operands.clone();
    let old_via = |pc: usize| f.via.get(pc).copied().unwrap_or(0);
    let mut nodes = f.via_nodes.clone();
    for b in 0..nb {
        let Some(blk) = f.blocks.get(BlockId(b as u32)) else {
            return false;
        };
        let mut cur: Vec<Out> = Vec::new();
        let mut cur_id = b;
        let mut closed = false;
        for pc in blk.start as usize..blk.end as usize {
            let Some(site) = sites.get(&pc) else {
                cur.push((f.code[pc], f.loc[pc], old_via(pc)));
                continue;
            };
            let ex = Expand::new(f, site, pc, round, &mut reg_ty, &mut nodes);
            let (loc, sv) = (ex.loc, ex.sv);
            let fw = &site.fw;
            if fw.straight() {
                ex.splice(&mut cur, &mut operands);
                continue;
            }
            ex.param_moves(&mut cur);
            let base = blocks.len();
            let n = fw.blocks.len();
            let dup = site.dup;
            // The rest of the block, for the arm that reads the table (renamed) and the one
            // that calls.
            let (mut hit_tail, mut miss_tail) = (Vec::new(), Vec::new());
            if dup {
                let end = blk.end as usize;
                for pc2 in pc + 1..end {
                    match sites.get(&pc2) {
                        Some(s2) => {
                            let e2 = Expand::new(f, s2, pc2, round, &mut reg_ty, &mut nodes);
                            e2.splice(&mut miss_tail, &mut operands);
                        }
                        None => miss_tail.push((f.code[pc2], f.loc[pc2], old_via(pc2))),
                    }
                }
                let mut inside = vec![0u32; uses.len()];
                for i in &f.code[pc + 1..end] {
                    count_uses(f, i, &mut inside);
                }
                let live_out = |r: Reg| {
                    let (u, i) = (uses.get(r.0 as usize), inside.get(r.0 as usize));
                    u.zip(i).is_some_and(|(u, i)| u > i)
                };
                // The table arm is block 2 of the forwarder: its `Ret` returns the pair.
                let seed = match (site.dst, fw.blocks[2].last()) {
                    (Some(d), Some((Inst::Ret { v: Some(r) }, ..))) => Some((d, ex.map(*r))),
                    _ => None,
                };
                hit_tail = rename_tail(&miss_tail, seed, &live_out, &mut reg_ty, &mut operands);
            }
            let cont = BlockId((base + n) as u32);
            blocks.resize(base + n + usize::from(!dup), None);
            cur.push((
                Inst::Br {
                    to: BlockId(base as u32),
                },
                loc,
                sv,
            ));
            blocks[cur_id] = Some(std::mem::take(&mut cur));
            for (i, cb) in fw.blocks.iter().enumerate() {
                let mut ents: Vec<Out> = Vec::with_capacity(cb.len() + 1);
                for &(inst, l, v) in cb {
                    let (l, v) = ex.own(l, v);
                    match inst {
                        Inst::Ret { .. } if dup && i == 2 => {
                            ents.extend(std::mem::take(&mut hit_tail));
                        }
                        Inst::Ret { .. } if dup => {
                            if let Some(m) = ex.ret_move(&inst) {
                                ents.push((m, loc, sv));
                            }
                            ents.extend(std::mem::take(&mut miss_tail));
                        }
                        Inst::Ret { .. } => {
                            if let Some(m) = ex.ret_move(&inst) {
                                ents.push((m, loc, sv));
                            }
                            ents.push((Inst::Br { to: cont }, loc, sv));
                        }
                        Inst::Br { to } => {
                            let to = BlockId(base as u32 + to.0);
                            ents.push((Inst::Br { to }, l, v));
                        }
                        Inst::CondBr { c, t, f: e } => {
                            let (t, e) = (BlockId(base as u32 + t.0), BlockId(base as u32 + e.0));
                            ents.push((
                                Inst::CondBr {
                                    c: ex.map(c),
                                    t,
                                    f: e,
                                },
                                l,
                                v,
                            ));
                        }
                        other => ents.push((ex.place_inst(other, &mut operands), l, v)),
                    }
                }
                blocks[base + i] = Some(ents);
            }
            if dup {
                closed = true;
                break;
            }
            cur_id = base + n;
        }
        if !closed {
            blocks[cur_id] = Some(cur);
        }
    }
    let mut code = Vec::new();
    let mut loc = Vec::new();
    let mut via = Vec::new();
    let mut ranges = Vec::with_capacity(blocks.len());
    for b in blocks {
        let Some(ents) = b else {
            return false;
        };
        let start = code.len() as u32;
        for (i, l, v) in ents {
            code.push(i);
            loc.push(l);
            via.push(v);
        }
        ranges.push((start, code.len() as u32));
    }
    for (k, &(s, e)) in ranges.iter().enumerate() {
        match f.blocks.get_mut(BlockId(k as u32)) {
            Some(b) => {
                b.start = s;
                b.end = e;
            }
            None => {
                f.blocks.push(crate::Block { start: s, end: e });
            }
        }
    }
    f.code = code;
    f.loc = loc;
    f.via = via;
    f.via_nodes = nodes;
    f.reg_ty = reg_ty;
    f.operands = operands;
    true
}
