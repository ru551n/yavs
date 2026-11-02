//! Composite variables of a process in its frame (`docs/decisions.md` "Frame objects in processes").
//!
//! The declarative part of a process runs once and the root frame persists, so a variable of a
//! fixed size and without handles needs no heap object: its bytes go after the lowered frame.
//! The slot keeps its shape `[data, desc, handle]`, the handle staying null. Loads of the data and
//! descriptor words become `AddrOf`, so a field is a constant frame offset.
//!
//! Runs on the lowered code, before the walk: the choice and the offsets depend on nothing an
//! instance can change, so a specialised copy, whose declarative part never runs, finds the bytes
//! where the shared code put them (`Machine::specialise` keeps them). A variable whose size only
//! the instance fixes moves from the heap into the copy's frame at the switch ([`for_copy`]).

use super::*;
use crate::ConstId;

/// The most bytes of one object, descriptor included: memories stay on the heap. (The address
/// holds the length in 12 bits of 8 bytes, and the start in 16: `FRAME_OBJECT`.)
const MAX_OBJECT: u32 = 16 * 1024;
const MAX_START: u32 = 512 * 1024;
/// The most bytes of objects one process takes.
const MAX_BYTES: u32 = 64 * 1024;

/// The type of the [`crate::Local`] that marks a frame object: no debugger or wave reader looks
/// at it (the pass is off for them), and the walk takes it as one object of unknown type.
pub(crate) const OBJECT_TY: TyId = TyId(u32::MAX);
/// The same for an object a specialised copy takes over at the switch ([`for_copy`]).
pub(crate) const MOVED_TY: TyId = TyId(u32::MAX - 1);

/// Are composite variables of processes kept in their frames? Off with `YAVS_NO_PROC_OBJECTS` set
/// to anything but `0` or the empty string (differential testing).
pub(super) fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("YAVS_NO_PROC_OBJECTS")
            .map(|v| v.is_empty() || v == "0")
            .unwrap_or(true)
    })
}

/// A value the lowered code computes from constants alone (registers defined once).
#[derive(Clone, Copy)]
enum V {
    K(i64),
    /// A frame address.
    Fr(u32),
    /// An address in the constant pool.
    Pool(ConstId, u32),
    /// The layout of a type under a descriptor of the pool (`None`: the type has none).
    Layout(TyId, Option<(ConstId, u32)>),
}

/// One object to place.
struct Cand {
    pc: usize,
    handle: Reg,
    deref: usize,
    p: Reg,
    slot: u32,
    size: u32,
    /// Descriptor bytes in front of the value (`NewFill`; a `New` copies both from one block).
    dsize: u32,
}

fn plain(types: &crate::IdVec<TyId, Ty>, ty: TyId, depth: u32) -> bool {
    depth < 32
        && match types.get(ty) {
            Some(Ty::Int { .. } | Ty::Physical { .. } | Ty::Enum { .. } | Ty::Real) => true,
            Some(Ty::Array { elem, .. }) => plain(types, *elem, depth + 1),
            Some(Ty::Record { fields }) => fields.iter().all(|f| plain(types, f.ty, depth + 1)),
            _ => false,
        }
}

/// The word `0` of the layout: the object's value size.
fn layout_size(prog: &Program, ty: TyId, desc: Option<(ConstId, u32)>) -> Option<i64> {
    let ds = Ty::desc_size(&prog.types, ty)? as usize;
    let mut b = vec![0u8; ds];
    match desc {
        Some((c, o)) => {
            if !prog.consts.get(c)?.read(o as usize, &mut b) {
                return None;
            }
        }
        None if ds != 0 => return None,
        None => {}
    }
    let (l, _) = Ty::layout(&prog.types, &prog.consts, ty, &b).ok()?;
    i64::try_from(l.size).ok()
}

/// The values of the registers defined once, in code order.
fn values(prog: &Program, f: &crate::Func, defs: &[u32]) -> Vec<Option<V>> {
    let mut v: Vec<Option<V>> = vec![None; f.reg_ty.len()];
    let once = |r: Reg| defs.get(r.0 as usize) == Some(&1);
    for i in &f.code {
        let new = match *i {
            Inst::Const { dst, v: k } => (dst, Some(V::K(k))),
            Inst::AddrOf {
                dst,
                space: AddrSpace::Frame,
                off,
            } => (dst, Some(V::Fr(off))),
            Inst::AddrOf {
                dst,
                space: AddrSpace::Const(c),
                off,
            } => (dst, Some(V::Pool(c, off))),
            Inst::Index {
                dst,
                base,
                pos,
                stride,
            } => {
                let at = match (v[base.0 as usize], v[pos.0 as usize]) {
                    (Some(V::Fr(o)), Some(V::K(k))) => i64::from(stride)
                        .checked_mul(k)
                        .and_then(|d| i64::from(o).checked_add(d))
                        .and_then(|e| u32::try_from(e).ok())
                        .map(V::Fr),
                    _ => None,
                };
                (dst, at)
            }
            Inst::LayoutOf { dst, ty, desc, .. } => {
                let at = match v[desc.0 as usize] {
                    Some(V::Pool(c, o)) => Some(V::Layout(ty, Some((c, o)))),
                    _ if Ty::desc_size(&prog.types, ty) == Some(0) => Some(V::Layout(ty, None)),
                    _ => None,
                };
                (dst, at)
            }
            Inst::Load {
                dst,
                addr,
                w: Width::B8,
            } => {
                let k = match v[addr.0 as usize] {
                    Some(V::Layout(ty, d)) => layout_size(prog, ty, d).map(V::K),
                    _ => None,
                };
                (dst, k)
            }
            Inst::Bin {
                op: IntOp::Add,
                dst,
                a,
                b,
            } => {
                let k = match (v[a.0 as usize], v[b.0 as usize]) {
                    (Some(V::K(x)), Some(V::K(y))) => x.checked_add(y).map(V::K),
                    _ => None,
                };
                (dst, k)
            }
            _ => continue,
        };
        if once(new.0) {
            v[new.0.0 as usize] = new.1;
        }
    }
    v
}

/// Is block `b` on every path from the entry to block `to`, with no way back to it?
fn once_before(f: &crate::Func, b: usize, to: usize) -> bool {
    let succs = |x: usize, out: &mut Vec<usize>| {
        let Some(l) = f
            .blocks
            .values()
            .get(x)
            .and_then(|bl| bl.end.checked_sub(1))
        else {
            return;
        };
        match f.code.get(l as usize) {
            Some(&Inst::Br { to }) => out.push(to.0 as usize),
            Some(&Inst::CondBr { t, f: e, .. }) => out.extend([t.0 as usize, e.0 as usize]),
            Some(Inst::Unreachable { .. }) => {}
            // (A case or another way out: taken as reaching everything.)
            _ => out.extend(0..f.blocks.len()),
        }
    };
    let reach = |from: usize, skip: Option<usize>| {
        let mut seen = vec![false; f.blocks.len()];
        let mut work = vec![from];
        let mut out = Vec::new();
        while let Some(x) = work.pop() {
            out.clear();
            succs(x, &mut out);
            for &s in &out {
                if Some(s) != skip && s < seen.len() && !std::mem::replace(&mut seen[s], true) {
                    work.push(s);
                }
            }
        }
        seen
    };
    // Not reached with `b` taken out, and `b` not reached from itself.
    b < to && (b == 0 || !reach(0, Some(b))[to]) && !reach(b, Some(to))[b]
}

/// The copy of the lowered process `raw` with its fixed composite variables in the frame, if it
/// has any.
pub(super) fn run(prog: &Program, raw: &crate::Func) -> Option<Box<crate::Func>> {
    let body = raw.body?.0 as usize;
    if raw.kind != FuncKind::Process
        || !enabled()
        || !super::enabled()
        || KEEP_VARS.load(std::sync::atomic::Ordering::Relaxed)
        || super::external()
        || !raw
            .code
            .iter()
            .any(|i| matches!(i, Inst::New { .. } | Inst::NewFill { .. }))
    {
        return None;
    }
    let mut defs = vec![0u32; raw.reg_ty.len()];
    for i in &raw.code {
        for_defs_fast(i, |d| {
            if let Some(n) = defs.get_mut(d.0 as usize) {
                *n += 1;
            }
        });
    }
    let vals = values(prog, raw, &defs);
    let val = |r: Reg| vals.get(r.0 as usize).copied().flatten();
    let at = |r: Reg| match val(r) {
        Some(V::Fr(o)) => Some(o),
        _ => None,
    };
    let block_of = |pc: usize| {
        raw.blocks
            .values()
            .iter()
            .position(|b| (b.start as usize) <= pc && pc < b.end as usize)
    };
    // What each register holding a handle or an address of interest is used for.
    let mut cands: Vec<Cand> = Vec::new();
    for b in 0..body.min(raw.blocks.len()) {
        let Some(bl) = raw.blocks.values().get(b) else {
            continue;
        };
        let mut checked = None;
        for pc in bl.start as usize..bl.end as usize {
            let (h, ty, size, dsize, shared) = match raw.code[pc] {
                Inst::New {
                    dst,
                    ty,
                    size,
                    shared,
                    ..
                } => (dst, ty, size, 0, shared),
                Inst::NewFill {
                    dst,
                    ty,
                    size,
                    dsize,
                    shared,
                    ..
                } => (dst, ty, size, dsize, shared),
                _ => continue,
            };
            let Some(V::K(n)) = val(size) else { continue };
            let Some(n) = u32::try_from(n).ok().filter(|&n| {
                n.checked_add(dsize)
                    .is_some_and(|t| t > 0 && t <= MAX_OBJECT)
            }) else {
                continue;
            };
            if shared || !plain(&prog.types, ty, 0) || defs.get(h.0 as usize) != Some(&1) {
                continue;
            }
            if !*checked.get_or_insert_with(|| once_before(raw, b, body)) {
                continue;
            }
            cands.push(Cand {
                pc,
                handle: h,
                deref: usize::MAX,
                p: h,
                slot: u32::MAX,
                size: n,
                dsize,
            });
        }
    }
    if cands.is_empty() {
        return None;
    }
    // The handle: one `Deref` and one store into a variable's handle slot, in the block of the
    // `New`, and nothing else.
    let by_handle: FxMap<u32, usize> = cands
        .iter()
        .enumerate()
        .map(|(k, c)| (c.handle.0, k))
        .collect();
    let mut bad = vec![false; cands.len()];
    for (pc, i) in raw.code.iter().enumerate() {
        match *i {
            Inst::Deref { dst, handle, .. } if by_handle.contains_key(&handle.0) => {
                let k = by_handle[&handle.0];
                let c = &mut cands[k];
                if c.deref != usize::MAX
                    || defs.get(dst.0 as usize) != Some(&1)
                    || block_of(pc) != block_of(c.pc)
                {
                    bad[k] = true;
                }
                c.deref = pc;
                c.p = dst;
                continue;
            }
            Inst::Store {
                addr,
                src,
                w: Width::B8,
            } if by_handle.contains_key(&src.0) => {
                let k = by_handle[&src.0];
                let c = &mut cands[k];
                match at(addr).and_then(|o| o.checked_sub(16)) {
                    Some(s) if c.slot == u32::MAX && block_of(pc) == block_of(c.pc) => c.slot = s,
                    _ => bad[k] = true,
                }
                continue;
            }
            Inst::New { .. } | Inst::NewFill { .. } => {}
            _ => {}
        }
        let mut hit = |r: Reg| {
            if let Some(&k) = by_handle.get(&r.0) {
                bad[k] = true;
            }
        };
        i.visit(&mut |r, role, _| {
            if role == Role::Use {
                hit(r);
            }
        });
        if let Some(l) = i.list() {
            for r in raw.regs(l).unwrap_or(&[]) {
                hit(*r);
            }
        }
    }
    for (k, c) in cands.iter().enumerate() {
        bad[k] |= c.deref == usize::MAX || c.slot == u32::MAX;
    }
    // A slot's three words: written where it is declared, read as words, used in no other way.
    // (Nested subprograms read them through their static link: the language gives an object one
    // constraint, so no code writes them after the declaration.)
    let slot_of = |o: u32, cs: &[Cand], bad: &[bool]| {
        cs.iter()
            .enumerate()
            .find(|(k, c)| !bad[*k] && c.slot <= o && o < c.slot + 24)
            .map(|(k, _)| k)
    };
    // Addresses of slot words used otherwise, and the stores of the words.
    let mut words: FxMap<u32, Vec<Reg>> = FxMap::default();
    for i in &raw.code {
        match *i {
            Inst::Load { addr, w, .. } if at(addr).is_some() => {
                let o = at(addr).unwrap_or(0);
                if let Some(k) = slot_of(o, &cands, &bad)
                    && (w != Width::B8
                        || o - cands[k].slot >= 16
                        || !(o - cands[k].slot).is_multiple_of(8))
                {
                    bad[k] = true;
                }
                continue;
            }
            Inst::Store { addr, src, w } if at(addr).is_some() => {
                let o = at(addr).unwrap_or(0);
                if let Some(k) = slot_of(o, &cands, &bad) {
                    if w != Width::B8 || !(o - cands[k].slot).is_multiple_of(8) {
                        bad[k] = true;
                    } else {
                        words.entry(o).or_default().push(src);
                    }
                }
                if let Some(s) = at(src)
                    && let Some(k) = slot_of(s, &cands, &bad)
                {
                    bad[k] = true;
                }
                continue;
            }
            Inst::Index { base, pos, .. } if at(base).is_some() && at(pos).is_none() => {
                // (A constant index of a slot's address is another tracked address.)
                if matches!(val(pos), Some(V::K(_))) {
                    continue;
                }
            }
            _ => {}
        }
        let mut hit = |r: Reg| {
            if let Some(o) = at(r)
                && let Some(k) = slot_of(o, &cands, &bad)
            {
                bad[k] = true;
            }
        };
        let mut used = Vec::new();
        i.visit(&mut |r, role, _| {
            if role == Role::Use {
                used.push(r);
            }
        });
        for r in used {
            hit(r);
        }
        if let Some(l) = i.list() {
            for r in raw.regs(l).unwrap_or(&[]) {
                hit(*r);
            }
        }
    }
    // Place them, in code order.
    let mut f = Box::new(raw.clone());
    let base = f.frame.next_multiple_of(8);
    let mut next = base;
    let mut placed: Vec<(usize, u32)> = Vec::new();
    let mut targets: FxMap<u32, u32> = FxMap::default();
    for (k, c) in cands.iter().enumerate() {
        if bad[k] {
            continue;
        }
        let total = c.size + c.dsize;
        if next - base + total > MAX_BYTES || next + total > MAX_START {
            continue;
        }
        // The data word holds the object's value, the descriptor word its descriptor (in the
        // object, or in the slot of an array).
        let Some(data) = words.get(&c.slot).filter(|w| w.len() == 1).map(|w| w[0]) else {
            continue;
        };
        let Some(desc) = words
            .get(&(c.slot + 8))
            .filter(|w| w.len() == 1)
            .map(|w| w[0])
        else {
            continue;
        };
        let off_of = |r: Reg| {
            if r == c.p { Some(next) } else { at(r) }
        };
        // Data past the descriptor (`Index` of `p`) is rare: such an object stays on the heap.
        let (Some(dv), Some(de)) = (off_of(data), off_of(desc)) else {
            continue;
        };
        targets.insert(c.slot, dv);
        targets.insert(c.slot + 8, de);
        f.locals.push(crate::Local {
            name: String::new(),
            slot: next,
            size: total,
            ty: OBJECT_TY,
        });
        placed.push((k, next));
        next = (next + total).next_multiple_of(8);
    }
    if placed.is_empty() {
        return None;
    }
    f.frame = next;
    // The code: the `New` is a null handle, the `Deref` the frame address with the bytes after it.
    let mut inserts: Vec<(usize, Vec<Inst>)> = Vec::new();
    let fresh = |f: &mut crate::Func, ty: RegTy| {
        let r = Reg(f.reg_ty.len() as u32);
        f.reg_ty.push(ty);
        r
    };
    for &(k, off) in &placed {
        let c = &cands[k];
        let mut after = Vec::new();
        match raw.code[c.pc] {
            Inst::New { init, size, .. } => after.push(Inst::Copy {
                dst: c.p,
                src: init,
                bytes: size,
            }),
            Inst::NewFill {
                desc,
                dsize,
                data,
                size,
                fill,
                ..
            } => {
                let mut q = c.p;
                if dsize != 0 {
                    let n = fresh(&mut f, RegTy::I64);
                    after.push(Inst::Const {
                        dst: n,
                        v: i64::from(dsize),
                    });
                    after.push(Inst::Copy {
                        dst: c.p,
                        src: desc,
                        bytes: n,
                    });
                    q = fresh(&mut f, RegTy::Ptr);
                    after.push(Inst::Index {
                        dst: q,
                        base: c.p,
                        pos: n,
                        stride: 1,
                    });
                }
                if fill == 0 {
                    after.push(Inst::Copy {
                        dst: q,
                        src: data,
                        bytes: size,
                    });
                } else if c.size != 0 {
                    let n = fresh(&mut f, RegTy::I64);
                    after.push(Inst::Const {
                        dst: n,
                        v: i64::from(c.size / fill),
                    });
                    after.push(Inst::Fill {
                        dst: q,
                        elem: data,
                        n,
                        size: fill,
                    });
                }
            }
            _ => {}
        }
        f.code[c.pc] = Inst::Const {
            dst: c.handle,
            v: 0,
        };
        f.code[c.deref] = Inst::AddrOf {
            dst: c.p,
            space: AddrSpace::Frame,
            off,
        };
        inserts.push((c.deref + 1, after));
    }
    // The loads of the words.
    for pc in 0..f.code.len() {
        if let Inst::Load {
            dst,
            addr,
            w: Width::B8,
        } = f.code[pc]
            && let Some(&off) = at(addr).and_then(|o| targets.get(&o))
        {
            f.code[pc] = Inst::AddrOf {
                dst,
                space: AddrSpace::Frame,
                off,
            };
        }
    }
    splice(&mut f, inserts);
    crate::build::number_func(&mut f, |c| {
        prog.funcs
            .get(c)
            .is_some_and(|c| c.flags.contains(FuncFlags::MAY_SUSPEND))
    });
    Some(f)
}

/// In a specialised copy, whose declarative part never runs: the variables the shared code left
/// on the heap because the instance sets their size (a generic), when the machine of `decl.rs`
/// knows that size. `Machine::specialise` moves their bytes to the frame offset the copy expects.
pub(super) fn for_copy(
    prog: &Program,
    id: FuncId,
    raw: &crate::Func,
    inst: Option<InstView<'_>>,
    layouts: &mut FxMap<TyId, Option<Vec<u64>>>,
) -> Option<Box<crate::Func>> {
    let body = raw.body?;
    if raw.kind != FuncKind::Process
        || !enabled()
        || !raw
            .code
            .iter()
            .any(|i| matches!(i, Inst::New { .. } | Inst::NewFill { .. }))
    {
        return None;
    }
    let mut o = Opt::new(
        prog,
        raw,
        raw.code.clone(),
        raw.operands.clone(),
        raw.nested,
        layouts,
    );
    o.inst = inst;
    o.fid = Some(id);
    o.drop_statements();
    o.analyse();
    let facts = o.decl_facts()?;
    drop(o);
    if facts.objects.is_empty() {
        return None;
    }
    let mut defs = vec![0u32; raw.reg_ty.len()];
    for i in &raw.code {
        for_defs_fast(i, |d| {
            if let Some(n) = defs.get_mut(d.0 as usize) {
                *n += 1;
            }
        });
    }
    let vals = values(prog, raw, &defs);
    let at = |r: Reg| match vals.get(r.0 as usize).copied().flatten() {
        Some(V::Fr(o)) => Some(o),
        _ => None,
    };
    let body_pc = raw.blocks.get(body)?.start as usize;
    let mut f = Box::new(raw.clone());
    let base = f.frame.next_multiple_of(8);
    let mut next = base;
    let mut targets: FxMap<u32, u32> = FxMap::default();
    for &(slot, total, ty, data) in &facts.objects {
        if total == 0
            || total > MAX_OBJECT
            || next - base + total > MAX_BYTES
            || next + total > MAX_START
            || !plain(&prog.types, ty, 0)
            || !only_in_its_slot(raw, &at, slot, body_pc)
        {
            continue;
        }
        targets.insert(slot, next + data);
        f.moved.push((slot, next, total));
        f.locals.push(crate::Local {
            name: String::new(),
            slot: next,
            size: total,
            ty: MOVED_TY,
        });
        next = (next + total).next_multiple_of(8);
    }
    if targets.is_empty() {
        return None;
    }
    f.frame = next;
    // (The word of the descriptor stays: its bytes are facts of the walk, wherever they are.)
    for pc in body_pc..f.code.len() {
        if let Inst::Load {
            dst,
            addr,
            w: Width::B8,
        } = f.code[pc]
            && let Some(&off) = at(addr).and_then(|o| targets.get(&o))
        {
            f.code[pc] = Inst::AddrOf {
                dst,
                space: AddrSpace::Frame,
                off,
            };
        }
    }
    Some(f)
}

/// Do the words of the slot at `slot` have no reader in the declarative part and no use but
/// direct loads and stores anywhere? (An alias of the variable would keep the object's address.)
fn only_in_its_slot(
    raw: &crate::Func,
    at: &impl Fn(Reg) -> Option<u32>,
    slot: u32,
    body_pc: usize,
) -> bool {
    let inside = |r: Reg| at(r).is_some_and(|o| slot <= o && o < slot + 24);
    for (pc, i) in raw.code.iter().enumerate() {
        match *i {
            Inst::Load { addr, w, .. } if inside(addr) => {
                let o = at(addr).unwrap_or(0) - slot;
                if pc < body_pc || w != Width::B8 || o >= 16 {
                    return false;
                }
            }
            Inst::Store { addr, src, .. } if inside(addr) => {
                if pc >= body_pc || inside(src) {
                    return false;
                }
            }
            Inst::Index { base, .. } if inside(base) => {}
            _ => {
                let mut bad = false;
                i.visit(&mut |r, role, _| bad |= role == Role::Use && inside(r));
                if let Some(l) = i.list() {
                    bad |= raw.regs(l).unwrap_or(&[]).iter().any(|&r| inside(r));
                }
                if bad {
                    return false;
                }
            }
        }
    }
    true
}

/// Insert `at` lists before the instructions they name; the insertions keep the statement of the
/// instruction before them.
fn splice(f: &mut crate::Func, mut ins: Vec<(usize, Vec<Inst>)>) {
    ins.sort_by_key(|x| x.0);
    let total: usize = ins.iter().map(|x| x.1.len()).sum();
    let mut code = Vec::with_capacity(f.code.len() + total);
    let mut loc = Vec::with_capacity(f.code.len() + total);
    // The new pc of each old pc, for the blocks.
    let mut map = Vec::with_capacity(f.code.len() + 1);
    let mut it = ins.into_iter().peekable();
    for pc in 0..=f.code.len() {
        while let Some((_, l)) = it.next_if(|x| x.0 == pc) {
            for i in l {
                code.push(i);
                loc.push(f.loc[pc.saturating_sub(1)]);
            }
        }
        map.push(code.len() as u32);
        if pc < f.code.len() {
            code.push(f.code[pc]);
            loc.push(f.loc[pc]);
        }
    }
    f.code = code;
    f.loc = loc;
    for b in f.blocks.values_mut() {
        // (An insertion at a block's start belongs to the block before it: the `Deref` it follows
        // never ends a block.)
        b.start = map[b.start as usize];
        b.end = map[b.end as usize];
    }
}
