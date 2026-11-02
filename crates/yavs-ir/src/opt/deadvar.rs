//! Bytes of a process's frame objects that nothing reads before they are written (`docs/decisions.md`
//! "Dead bytes of frame objects"): backward liveness per byte of the composite variables in the frame.
//!
//! A pointer into an object is followed through `Index`, `Move`, `Select` and `Field` (`Ptr`);
//! anything it cannot follow makes the object escaped, and an escaped object is always live.
//! A `Suspend` reads every byte by default (`Persist::Keep`): the frame stays what the other
//! versions of the process (shared code, its specialised copy) expect at a switch.

use super::*;
use crate::{ConstId, Regs, gather};

/// A set of objects (the first 64 of a function).
type Set = u64;

/// One frame object: its bytes in the frame and where it starts in the bit sets.
#[derive(Clone, Copy)]
struct Obj {
    start: u32,
    size: u32,
    bit: u32,
}

/// Is a variable's value at a suspension read later? `Dead` is the census's question only.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Persist {
    Keep,
    Dead,
}

/// What a register may hold, flow-insensitive.
#[derive(Clone, Copy, Default)]
struct Ptr {
    /// Objects it may point into.
    obj: Set,
    /// Objects that a pointer in the memory it points to may point into (a fat pair a call returned).
    inner: Set,
}

/// Where a register defined once points.
#[derive(Clone, Copy, PartialEq)]
enum Place {
    At(u32),
    /// Somewhere in these frame bytes.
    In(u32, u32),
}

/// What one instruction does to the object bytes: overwrites a span whole, reads spans, or may
/// suspend (then, under `Persist::Keep`, every byte is read).
pub(super) struct Eff {
    kill: Option<Span>,
    reads: Vec<Span>,
    susp: bool,
}

/// A logic leaf of a table an object is gathered from or scattered to.
#[derive(Clone, Copy)]
struct LogicLeaf {
    len: u32,
}

pub(super) struct Live<'a> {
    prog: &'a Program,
    f: &'a crate::Func,
    objs: Vec<Obj>,
    bits: usize,
    defs: Vec<u32>,
    konst: FxMap<u32, i64>,
    ptr: Vec<Ptr>,
    pub(super) escaped: Set,
    place: FxMap<u32, Place>,
    /// The frame offsets of the registers defined once by `AddrOf`.
    addrs: FxMap<u32, u32>,
    /// What the words of the frame outside the objects hold: the data word of a variable's slot.
    held: FxMap<u32, Ptr>,
    /// Sorted disjoint frame ranges an address stays in once it has an unknown index.
    regions: Vec<(u32, u32)>,
    logic: FxMap<u32, LogicLeaf>,
    /// The value sizes of the plans of `(table, descriptor register)`.
    sizes: FxMap<(u32, u32), Option<u32>>,
    /// The pool addresses of the registers defined once by `AddrOf` of a constant.
    pool: FxMap<u32, (ConstId, u32)>,
    /// Per block, the instructions that touch object bytes, last first, with what they do.
    effs: Vec<Vec<(usize, Eff)>>,
}

/// A range in the bit set: `(first, count)`.
type Span = (usize, usize);

/// The words of a span with the mask of its bits in each.
fn words((a, n): Span) -> impl Iterator<Item = (usize, u64)> {
    let e = a + n;
    (a / 64..e.div_ceil(64)).map(move |w| {
        let lo = (w * 64).max(a) - w * 64;
        let hi = (e.min(w * 64 + 64)) - w * 64;
        let m = if hi - lo == 64 {
            u64::MAX
        } else {
            ((1u64 << (hi - lo)) - 1) << lo
        };
        (w, m)
    })
}

fn any(s: &[u64], sp: Span) -> bool {
    words(sp).any(|(w, m)| s[w] & m != 0)
}

fn set(s: &mut [u64], sp: Span) {
    for (w, m) in words(sp) {
        s[w] |= m;
    }
}

fn clear(s: &mut [u64], sp: Span) {
    for (w, m) in words(sp) {
        s[w] &= !m;
    }
}

impl<'a> Live<'a> {
    pub(super) fn new(prog: &'a Program, f: &'a crate::Func) -> Option<Live<'a>> {
        let mut objs = Vec::new();
        let mut bits = 0u32;
        for l in &f.locals {
            if (l.ty == proc_objects::OBJECT_TY || l.ty == proc_objects::MOVED_TY)
                && objs.len() < 64
            {
                objs.push(Obj {
                    start: l.slot,
                    size: l.size,
                    bit: bits,
                });
                bits += l.size;
            }
        }
        if objs.is_empty() {
            return None;
        }
        let mut defs = vec![0u32; f.reg_ty.len()];
        let mut konst = FxMap::default();
        for i in &f.code {
            for_defs_fast(i, |d| {
                if let Some(n) = defs.get_mut(d.0 as usize) {
                    *n += 1;
                }
            });
            if let Inst::Const { dst, v } = *i {
                konst.insert(dst.0, v);
            }
        }
        konst.retain(|r, _| defs.get(*r as usize) == Some(&1));
        let mut s = Live {
            prog,
            f,
            objs,
            bits: bits as usize,
            defs,
            konst,
            ptr: vec![Ptr::default(); f.reg_ty.len()],
            escaped: 0,
            place: FxMap::default(),
            addrs: FxMap::default(),
            held: FxMap::default(),
            regions: Vec::new(),
            logic: FxMap::default(),
            sizes: FxMap::default(),
            pool: FxMap::default(),
            effs: Vec::new(),
        };
        for i in &f.code {
            if let Inst::AddrOf {
                dst,
                space: AddrSpace::Frame,
                off,
            } = *i
                && s.once(dst)
            {
                s.addrs.insert(dst.0, off);
            }
            if let Inst::AddrOf {
                dst,
                space: AddrSpace::Const(c),
                off,
            } = *i
                && s.once(dst)
            {
                s.pool.insert(dst.0, (c, off));
            }
        }
        s.pointers();
        for i in &f.code {
            if let Inst::SigGather { desc, tab, .. } | Inst::SigScatter { desc, tab, .. } = *i {
                let n = s.plan(tab, desc).and_then(|p| u32::try_from(p.size).ok());
                s.sizes.insert((tab.0, desc.0), n);
            }
        }
        s.leaves();
        s.places();
        s.effs = f
            .blocks
            .values()
            .iter()
            .map(|bl| {
                (bl.start as usize..bl.end as usize)
                    .rev()
                    .filter_map(|pc| Some((pc, s.effect(&f.code[pc])?)))
                    .collect()
            })
            .collect();
        Some(s)
    }

    fn obj_at(&self, off: u32) -> Option<usize> {
        self.objs
            .iter()
            .position(|o| o.start <= off && off < o.start + o.size)
    }

    fn once(&self, r: Reg) -> bool {
        self.defs.get(r.0 as usize) == Some(&1)
    }

    fn k(&self, r: Reg) -> Option<i64> {
        self.konst.get(&r.0).copied()
    }

    fn get(&self, r: Reg) -> Ptr {
        self.ptr.get(r.0 as usize).copied().unwrap_or_default()
    }

    fn put(&mut self, r: Reg, p: Ptr) -> bool {
        let Some(c) = self.ptr.get_mut(r.0 as usize) else {
            return false;
        };
        let (o, n) = (c.obj | p.obj, c.inner | p.inner);
        let moved = (o, n) != (c.obj, c.inner);
        (c.obj, c.inner) = (o, n);
        moved
    }

    fn escape(&mut self, p: Ptr) -> bool {
        let e = self.escaped | p.obj | p.inner;
        let moved = e != self.escaped;
        self.escaped = e;
        moved
    }

    fn list_ptrs(&self, l: Regs) -> Ptr {
        let mut p = Ptr::default();
        for &r in self.f.regs(l).unwrap_or(&[]) {
            let q = self.get(r);
            p.obj |= q.obj;
            p.inner |= q.inner;
        }
        p
    }

    /// May-point-to sets of the registers, to a fixed point.
    fn pointers(&mut self) {
        let f = self.f;
        if f.nested {
            // ponytail: a nested subprogram can reach the frame by a static link: every object stays
            // live. Scan them as `Opt::nested_writes` does if such processes turn out to matter.
            self.escaped = Set::MAX;
            return;
        }
        for i in &f.code {
            if let Inst::AddrOf {
                dst,
                space: AddrSpace::Frame,
                off,
            } = *i
                && let Some(k) = self.obj_at(off)
            {
                self.ptr[dst.0 as usize].obj |= 1 << k;
            }
        }
        loop {
            let mut moved = false;
            for i in &f.code {
                moved |= self.flow(i);
            }
            if !moved {
                break;
            }
        }
        // A word that holds a pointer may be read through its address only by the `Load`s above.
        for i in &f.code {
            let mut hit = Ptr::default();
            let mut see = |s: &Self, r: Reg| {
                if let Some(q) = s.addrs.get(&r.0).and_then(|o| s.held.get(o)) {
                    hit.obj |= q.obj;
                    hit.inner |= q.inner;
                }
            };
            match *i {
                Inst::Load { .. } => {}
                Inst::Store { src, .. } => see(self, src),
                _ => {
                    i.visit(&mut |r, role, _| {
                        if role == Role::Use {
                            see(self, r);
                        }
                    });
                    if let Some(l) = i.list() {
                        for &r in f.regs(l).unwrap_or(&[]) {
                            see(self, r);
                        }
                    }
                }
            }
            self.escape(hit);
        }
    }

    /// The pointers one instruction passes on or lets go.
    fn flow(&mut self, i: &Inst) -> bool {
        match *i {
            Inst::Move { dst, src } => self.put(dst, self.get(src)),
            Inst::Select { dst, a, b, .. } => {
                let p = self.get(a);
                let q = self.get(b);
                self.put(dst, p) | self.put(dst, q)
            }
            Inst::Index { dst, base, .. } | Inst::Field { dst, base, .. } => {
                self.put(dst, self.get(base))
            }
            // The objects hold plain values: memory gives back a pointer only if it may hold one.
            Inst::Load { dst, addr, .. } => {
                if let Some(q) = self
                    .addrs
                    .get(&addr.0)
                    .and_then(|o| self.held.get(o))
                    .copied()
                {
                    return self.put(dst, q);
                }
                let p = self.get(addr).inner;
                self.put(dst, Ptr { obj: p, inner: p })
            }
            Inst::Store { addr, src, .. } => {
                let p = self.get(src);
                if p.obj | p.inner == 0 {
                    return false;
                }
                match self.addrs.get(&addr.0).copied() {
                    Some(o) if self.obj_at(o).is_none() => {
                        let c = self.held.entry(o).or_default();
                        let moved = (c.obj | p.obj, c.inner | p.inner) != (c.obj, c.inner);
                        c.obj |= p.obj;
                        c.inner |= p.inner;
                        moved
                    }
                    _ => self.escape(p),
                }
            }
            Inst::Copy { src, .. } => {
                let p = self.get(src).inner;
                self.escape(Ptr { obj: p, inner: p })
            }
            Inst::MakeDope { parts, .. } => self.escape(self.list_ptrs(parts)),
            Inst::CallForeign { args, .. } => self.escape(self.list_ptrs(args)),
            Inst::Call { args, dst, .. }
            | Inst::CallIndirect { args, dst, .. }
            | Inst::CallProt { args, dst, .. }
            | Inst::CallNative { args, dst, .. } => {
                let p = self.list_ptrs(args);
                let b = p.obj | p.inner;
                dst.is_some_and(|d| self.put(d, Ptr { obj: b, inner: b }))
            }
            // A number, whatever it was computed from.
            Inst::Cmp { .. }
            | Inst::FCmp { .. }
            | Inst::Bool { .. }
            | Inst::LEq { .. }
            | Inst::LKey { .. }
            | Inst::MemEq { .. }
            | Inst::LGet { .. }
            | Inst::DopeLen { .. }
            | Inst::Pos { .. }
            | Inst::SlicePos { .. }
            | Inst::LayoutOf { .. }
            | Inst::Bin { .. }
            | Inst::BinRange { .. }
            | Inst::AddrOf { .. }
            | Inst::Const { .. } => false,
            _ => {
                // Anything else that takes a pointer and defines a register passes it on.
                let mut p = Ptr::default();
                let mut d = Vec::new();
                i.visit(&mut |r, role, _| match role {
                    Role::Use => {
                        let q = self.ptr.get(r.0 as usize).copied().unwrap_or_default();
                        p.obj |= q.obj;
                        p.inner |= q.inner;
                    }
                    Role::Def => d.push(r),
                });
                if let Some(l) = i.list() {
                    let q = self.list_ptrs(l);
                    p.obj |= q.obj;
                    p.inner |= q.inner;
                }
                let mut moved = false;
                if p.obj | p.inner != 0 {
                    for r in d {
                        moved |= self.put(r, p);
                    }
                }
                moved
            }
        }
    }

    /// The plan of a table under a descriptor of the pool.
    pub(super) fn plan(&self, tab: ConstId, desc: Reg) -> Option<gather::Plan> {
        let prog = self.prog;
        let Some(crate::ConstData::Bytes(t)) = prog.consts.get(tab) else {
            return None;
        };
        let (ty, _) = gather::entries(t)?;
        let ds = Ty::desc_size(&prog.types, ty)? as usize;
        let mut b = vec![0u8; ds];
        if ds != 0 {
            let (c, o) = *self.pool.get(&desc.0)?;
            if !prog.consts.get(c)?.read(o as usize, &mut b) {
                return None;
            }
        }
        gather::plan(&prog.types, &prog.consts, t, &b).ok()
    }

    fn size(&self, tab: ConstId, desc: Reg) -> Option<u32> {
        self.sizes.get(&(tab.0, desc.0)).copied().flatten()
    }

    /// The leaves of the values the objects are gathered from and scattered to.
    fn leaves(&mut self) {
        let addrs = self.addrs.clone();
        let at = |r: Reg, _: &Self| addrs.get(&r.0).copied();
        let mut base: Vec<(u32, gather::Plan)> = Vec::new();
        let mut gathered: FxMap<u32, gather::Plan> = FxMap::default();
        for i in &self.f.code {
            match *i {
                Inst::SigScatter { src, desc, tab, .. } => {
                    if let (Some(o), Some(p)) = (at(src, self), self.plan(tab, desc)) {
                        base.push((o, p));
                    }
                }
                Inst::SigGather { dst, desc, tab, .. } => {
                    if let Some(p) = self.plan(tab, desc) {
                        match at(dst, self) {
                            Some(o) => base.push((o, p)),
                            None => {
                                gathered.insert(dst.0, p);
                            }
                        }
                    }
                }
                Inst::Copy { dst, src, .. } => {
                    if let (Some(o), Some(p)) = (at(dst, self), gathered.get(&src.0)) {
                        base.push((o, p.clone()));
                    }
                }
                _ => {}
            }
        }
        let mut rg: Vec<(u32, u32)> = Vec::new();
        for (o, p) in &base {
            if self.obj_at(*o).is_none() {
                continue;
            }
            for l in &p.leaves {
                if l.insts == 0 {
                    continue;
                }
                let first = l.off(0) as u32;
                let last = l.off(l.insts as usize - 1) as u32;
                rg.push((o + first, o + last + l.vsize));
                if l.kind == gather::LeafKind::LogicVec && l.insts == 1 {
                    self.logic.insert(o + first, LogicLeaf { len: l.len });
                }
            }
        }
        rg.sort_unstable();
        let mut out: Vec<(u32, u32)> = Vec::new();
        for (a, e) in rg {
            match out.last_mut() {
                // (An exact repeat of a leaf from another table, or instances that interleave.)
                Some(l) if a < l.1 => l.1 = l.1.max(e),
                _ => out.push((a, e)),
            }
        }
        self.regions = out;
    }

    /// The region (else the object) of frame byte `off`.
    fn region(&self, off: u32) -> (u32, u32) {
        let k = self.regions.partition_point(|r| r.0 <= off);
        if k > 0 && off < self.regions[k - 1].1 {
            return self.regions[k - 1];
        }
        match self.obj_at(off) {
            Some(k) => (self.objs[k].start, self.objs[k].start + self.objs[k].size),
            None => (off, off),
        }
    }

    /// Where the registers defined once point.
    fn places(&mut self) {
        for (&r, &o) in &self.addrs {
            self.place.insert(r, Place::At(o));
        }
        // (Blocks are not in order of execution: twice takes the chains of practice, a register
        // left out is only less exact.)
        for _ in 0..2 {
            for i in &self.f.code {
                match *i {
                    Inst::Move { dst, src } if self.once(dst) => {
                        if let Some(&p) = self.place.get(&src.0) {
                            self.place.insert(dst.0, p);
                        }
                    }
                    Inst::Index {
                        dst,
                        base,
                        pos,
                        stride,
                    } if self.once(dst) => {
                        let Some(&p) = self.place.get(&base.0) else {
                            continue;
                        };
                        let c = self.k(pos).and_then(|c| c.checked_mul(i64::from(stride)));
                        let np = match (p, c) {
                            (Place::At(o), Some(c)) => match u32::try_from(i64::from(o) + c).ok() {
                                Some(n) => Place::At(n),
                                None => continue,
                            },
                            (Place::In(a, e), Some(_)) => Place::In(a, e),
                            // A byte stride may be the offset of a field: not bounded by a leaf.
                            (_, None) if stride <= 1 => continue,
                            (Place::At(o), None) => {
                                let (a, e) = self.region(o);
                                Place::In(a, e)
                            }
                            (Place::In(a, e), None) => Place::In(a, e),
                        };
                        self.place.insert(dst.0, np);
                    }
                    _ => {}
                }
            }
        }
    }

    /// The span of frame bytes `off .. off + n` when they lie in one object.
    fn span(&self, off: u32, n: u32) -> Option<(usize, Span)> {
        let k = self.obj_at(off)?;
        let o = self.objs[k];
        let at = off - o.start;
        let n = n.min(o.size - at);
        Some((k, ((o.bit + at) as usize, n as usize)))
    }

    /// Every byte `r` may reach, when it is read through an unknown length.
    fn read_reg(&self, r: Reg, out: &mut Vec<Span>) {
        let p = self.get(r);
        let m = p.obj | p.inner;
        if m == 0 {
            return;
        }
        if p.inner == 0
            && let Some(&Place::In(a, e)) = self.place.get(&r.0)
            && let Some((_, s)) = self.span(a, e - a)
        {
            out.push(s);
            return;
        }
        self.read_objs(m, out);
    }

    /// A logic array or plain array at `r` read: an address at a leaf stays in the leaf.
    fn read_arr(&self, r: Reg, out: &mut Vec<Span>) {
        if self.get(r).inner == 0
            && let Some(&Place::At(o)) = self.place.get(&r.0)
        {
            let (a, e) = self.region(o);
            if let Some((_, s)) = self.span(a, e - a) {
                out.push(s);
            }
            return;
        }
        self.read_reg(r, out);
    }

    fn read_objs(&self, m: Set, out: &mut Vec<Span>) {
        for (k, o) in self.objs.iter().enumerate() {
            if m >> k & 1 != 0 {
                out.push((o.bit as usize, o.size as usize));
            }
        }
    }

    /// `n` bytes at `r` read.
    fn read_n(&self, r: Reg, n: Option<u32>, out: &mut Vec<Span>) {
        match (self.place.get(&r.0), n) {
            (Some(&Place::At(o)), Some(n))
                if self.get(r).inner == 0
                    && self
                        .obj_at(o)
                        .is_some_and(|k| o + n <= self.objs[k].start + self.objs[k].size) =>
            {
                if let Some((_, s)) = self.span(o, n) {
                    out.push(s);
                }
            }
            _ => self.read_reg(r, out),
        }
    }

    /// The plane blocks elements `off .. off + n` of the logic array at frame byte `o` lie in.
    fn blocks(o: u32, off: i64, n: i64) -> Option<(u32, u32)> {
        if off < 0 || n <= 0 {
            return None;
        }
        let first = off / 64;
        let last = (off + n - 1) / 64;
        let a = i64::from(o) + first * 32;
        let len = (last - first + 1) * 32;
        Some((u32::try_from(a).ok()?, u32::try_from(len).ok()?))
    }

    /// The bytes a write of elements `off .. off + n` of the logic array at `base` overwrites whole.
    fn logic_kill(&self, base: Reg, off: Reg, n: Reg) -> Option<(u32, u32)> {
        let Some(&Place::At(o)) = self.place.get(&base.0) else {
            return None;
        };
        let (off, n) = (self.k(off)?, self.k(n)?);
        if off % 64 != 0 {
            return None;
        }
        let whole = n % 64 == 0
            || self
                .logic
                .get(&o)
                .is_some_and(|l| i64::from(l.len) == off + n);
        if !whole {
            return None;
        }
        Self::blocks(o, off, n)
    }

    /// What a write kills: bytes of the frame overwritten whole.
    pub(super) fn kill(&self, i: &Inst) -> Option<(u32, u32)> {
        let at = |r: Reg| match self.place.get(&r.0) {
            Some(&Place::At(o)) => Some(o),
            _ => None,
        };
        match *i {
            Inst::Store { addr, w, .. } if w != Width::Sig => Some((at(addr)?, w.bytes())),
            Inst::Copy { dst, bytes, .. } => Some((at(dst)?, u32::try_from(self.k(bytes)?).ok()?)),
            Inst::Zero { addr, bytes } => Some((at(addr)?, bytes)),
            Inst::LCopy {
                dst, dst_off, n, ..
            } => self.logic_kill(dst, dst_off, n),
            Inst::LFill { base, idx, n, .. } => self.logic_kill(base, idx, n),
            Inst::MakeDope { at: a, parts } => Some((at(a)?, parts.len * 8)),
            Inst::SigGather { dst, desc, tab, .. } => Some((at(dst)?, self.size(tab, desc)?)),
            _ => None,
        }
    }

    /// What instruction `i` does to the object bytes, once per instruction.
    fn effect(&self, i: &Inst) -> Option<Eff> {
        let kill = self
            .kill(i)
            .and_then(|(o, n)| self.span(o, n))
            .filter(|(k, _)| self.escaped >> k & 1 == 0)
            .map(|(_, s)| s);
        let mut reads = Vec::new();
        self.reads(i, &mut reads);
        let susp = match *i {
            Inst::Suspend { .. } => true,
            Inst::Call { func, .. } => self
                .prog
                .funcs
                .get(func)
                .is_none_or(|c| c.flags.contains(FuncFlags::MAY_SUSPEND)),
            _ => false,
        };
        (kill.is_some() || susp || !reads.is_empty()).then_some(Eff { kill, reads, susp })
    }

    /// One instruction, backward: `live` before it from `live` after it.
    fn transfer(&self, e: &Eff, persist: Persist, live: &mut [u64]) {
        if let Some(s) = e.kill {
            clear(live, s);
        }
        for &s in &e.reads {
            set(live, s);
        }
        if e.susp && persist == Persist::Keep {
            set(live, (0, self.bits));
        }
    }

    /// The bytes instruction `i` may read.
    fn reads(&self, i: &Inst, out: &mut Vec<Span>) {
        match *i {
            Inst::Suspend { .. } => {}
            // Addresses computed, not read.
            Inst::Index { .. }
            | Inst::Field { .. }
            | Inst::Move { .. }
            | Inst::Select { .. }
            | Inst::AddrOf { .. }
            | Inst::Const { .. }
            | Inst::ConstF { .. }
            | Inst::Stmt => {}
            Inst::Load { addr, w, .. } => self.read_n(addr, Some(w.bytes()), out),
            // A write through a pointer: no read, no kill.
            Inst::Store { src, .. } => self.read_reg(src, out),
            Inst::Zero { .. } | Inst::MakeDope { .. } | Inst::LSet { .. } | Inst::LFill { .. } => {}
            Inst::Copy { src, bytes, .. } => {
                let n = self.k(bytes).and_then(|n| u32::try_from(n).ok());
                self.read_n(src, n, out);
            }
            Inst::LCopy {
                src, src_off, n, ..
            } => match (self.place.get(&src.0), self.k(src_off), self.k(n)) {
                (Some(&Place::At(o)), Some(a), Some(c)) if self.get(src).inner == 0 => {
                    match Self::blocks(o, a, c) {
                        Some((b, l)) => {
                            if let Some((_, s)) = self.span(b, l) {
                                out.push(s);
                            }
                        }
                        None => self.read_reg(src, out),
                    }
                }
                _ => self.read_reg(src, out),
            },
            Inst::SigScatter {
                src,
                desc,
                tab,
                ops,
                ..
            } => {
                self.read_n(src, self.size(tab, desc), out);
                self.read_reg(desc, out);
                for &r in self.f.regs(ops).unwrap_or(&[]) {
                    self.read_reg(r, out);
                }
            }
            Inst::SigGather { desc, .. } => self.read_reg(desc, out),
            Inst::LogicOp { a, b, .. }
            | Inst::LNum { a, b, .. }
            | Inst::LEq { a, b, .. }
            | Inst::MemEq { a, b, .. } => {
                self.read_arr(a, out);
                self.read_arr(b, out);
            }
            Inst::LKey { a, .. } => self.read_arr(a, out),
            Inst::LGet { base, .. } => self.read_arr(base, out),
            Inst::Cat { parts, .. } => {
                for &r in self.f.regs(parts).unwrap_or(&[]) {
                    self.read_arr(r, out);
                }
            }
            Inst::Call { args, .. } => {
                for &r in self.f.regs(args).unwrap_or(&[]) {
                    self.read_reg(r, out);
                }
            }
            Inst::CallNative { native, args, .. } => {
                let arrays = self
                    .prog
                    .natives
                    .get(native)
                    .is_some_and(|n| n.symbol.starts_with("yavs.ieee."));
                for &r in self.f.regs(args).unwrap_or(&[]) {
                    if arrays {
                        self.read_arr(r, out);
                    } else {
                        self.read_reg(r, out);
                    }
                }
            }
            _ => {
                i.visit(&mut |r, role, _| {
                    if role == Role::Use {
                        self.read_reg(r, out);
                    }
                });
                if let Some(l) = i.list() {
                    for &r in self.f.regs(l).unwrap_or(&[]) {
                        self.read_reg(r, out);
                    }
                }
            }
        }
    }

    fn succs(&self, b: usize, out: &mut Vec<usize>) {
        out.clear();
        let Some(bl) = self.f.blocks.values().get(b) else {
            return;
        };
        if bl.end <= bl.start {
            return;
        }
        match bl
            .end
            .checked_sub(1)
            .and_then(|l| self.f.code.get(l as usize))
        {
            Some(&Inst::Br { to }) => out.push(to.0 as usize),
            Some(&Inst::CondBr { t, f, .. }) => out.extend([t.0 as usize, f.0 as usize]),
            Some(&Inst::Case { table, .. }) => {
                if let Some(t) = self.prog.cases.get(table) {
                    out.push(t.default.0 as usize);
                    out.extend(t.arms.iter().map(|a| a.2.0 as usize));
                }
            }
            _ => {}
        }
    }

    /// Does block `b` end in a way `succs` knows (else everything is live at its end)?
    fn known_end(&self, b: usize) -> bool {
        let bl = self.f.blocks.values()[b];
        bl.end > bl.start
            && matches!(
                self.f.code.get(bl.end as usize - 1),
                Some(
                    Inst::Br { .. }
                        | Inst::CondBr { .. }
                        | Inst::Case { .. }
                        | Inst::Ret { .. }
                        | Inst::Unreachable { .. }
                )
            )
    }

    /// The bytes live at the end of each block.
    pub(super) fn solve(&self, persist: Persist) -> Vec<Vec<u64>> {
        let w = self.bits.div_ceil(64);
        let nb = self.f.blocks.len();
        let mut base = vec![0u64; w];
        let mut esc = Vec::new();
        self.read_objs(self.escaped, &mut esc);
        for s in esc {
            set(&mut base, s);
        }
        let mut all = vec![0u64; w];
        set(&mut all, (0, self.bits));
        let ends: Vec<bool> = (0..nb).map(|b| self.known_end(b)).collect();
        let mut out: Vec<Vec<u64>> = ends
            .iter()
            .map(|&k| if k { base.clone() } else { all.clone() })
            .collect();
        let mut inn = vec![base; nb];
        let mut ss = Vec::new();
        loop {
            let mut moved = false;
            for b in (0..nb).rev() {
                self.succs(b, &mut ss);
                for &s in &ss {
                    if s < nb {
                        for (a, c) in out[b].iter_mut().zip(&inn[s]) {
                            *a |= *c;
                        }
                    }
                }
                let mut l = out[b].clone();
                for (_, e) in &self.effs[b] {
                    self.transfer(e, persist, &mut l);
                }
                if l != inn[b] {
                    inn[b] = l;
                    moved = true;
                }
            }
            if !moved {
                return out;
            }
        }
    }

    /// One line per process for `YAVS_DEADVAR_CENSUS`.
    pub(super) fn census(&self, id: FuncId) {
        let f = self.f;
        let obj_bytes: u64 = self.objs.iter().map(|o| u64::from(o.size)).sum();
        let esc_bytes: u64 = self
            .objs
            .iter()
            .enumerate()
            .filter(|(k, _)| self.escaped >> k & 1 != 0)
            .map(|(_, o)| u64::from(o.size))
            .sum();
        // Bytes live at a suspension, with nothing assumed about the next activation.
        let dead = self.solve(Persist::Dead);
        let keep = self.solve(Persist::Keep);
        let w = self.bits.div_ceil(64);
        let mut at_susp = vec![0u64; w];
        let body = f.body.map_or(0, |b| b.0 as usize);
        #[derive(Default)]
        struct C {
            copies: u64,
            copy_bytes: u64,
            gcopy_bytes: u64,
            gcopies: u64,
            /// Bytes of the `Copy`s dead (nothing persists, a `Suspend` keeps all).
            cdead_p: u64,
            cdead_k: u64,
            gdead_p: u64,
            gdead_k: u64,
            gleaves: u64,
            gleaves_p: u64,
            gleaves_k: u64,
            writes: u64,
            wdead_p: u64,
            wdead_k: u64,
        }
        let mut c = C::default();
        for b in 0..f.blocks.len() {
            let bl = f.blocks.values()[b];
            let mut lp = dead[b].clone();
            let mut lk = keep[b].clone();
            for pc in (bl.start as usize..bl.end as usize).rev() {
                let i = &f.code[pc];
                if matches!(i, Inst::Suspend { .. }) {
                    for (a, x) in at_susp.iter_mut().zip(&lp) {
                        *a |= *x;
                    }
                }
                if b >= body
                    && let Some((o, n)) = self.kill(i)
                    && let Some((k, s)) = self.span(o, n)
                    && self.escaped >> k & 1 == 0
                {
                    let dp = (s.0..s.0 + s.1).filter(|&x| !any(&lp, (x, 1))).count() as u64;
                    let dk = (s.0..s.0 + s.1).filter(|&x| !any(&lk, (x, 1))).count() as u64;
                    let whole = matches!(i, Inst::Copy { .. }) && n >= 32;
                    if whole && verbose() {
                        let src = match *i {
                            Inst::Copy { src, .. } => src,
                            _ => Reg(0),
                        };
                        let def = f.code.iter().find(|x| {
                            let mut d = false;
                            for_defs_fast(x, |r| d |= r == src);
                            d
                        });
                        eprintln!(
                            "deadvar-copy {} pc={pc} blk={b} off={o} n={n} dead={dp} src={def:?}",
                            f.name
                        );
                    }
                    if whole {
                        c.copies += 1;
                        c.copy_bytes += u64::from(n);
                        c.cdead_p += dp;
                        c.cdead_k += dk;
                        let src = match *i {
                            Inst::Copy { src, .. } => Some(src),
                            _ => None,
                        };
                        // A value a gather just made.
                        let g = (bl.start as usize..pc).rev().find_map(|q| match f.code[q] {
                            Inst::SigGather { dst, desc, tab, .. } if Some(dst) == src => {
                                Some((desc, tab))
                            }
                            _ => None,
                        });
                        if let Some((desc, tab)) = g {
                            c.gcopies += 1;
                            c.gcopy_bytes += u64::from(n);
                            c.gdead_p += dp;
                            c.gdead_k += dk;
                            if let Some(p) = self.plan(tab, desc) {
                                for l in &p.leaves {
                                    c.gleaves += 1;
                                    let all = |live: &[u64]| {
                                        (0..l.insts as usize).all(|j| {
                                            self.span(o + l.off(j) as u32, l.vsize)
                                                .is_some_and(|(_, t)| !any(live, t))
                                        })
                                    };
                                    c.gleaves_p += u64::from(all(&lp));
                                    c.gleaves_k += u64::from(all(&lk));
                                }
                            }
                        }
                    } else {
                        c.writes += 1;
                        c.wdead_p += u64::from(dp == s.1 as u64);
                        c.wdead_k += u64::from(dk == s.1 as u64);
                    }
                }
                if let Some(e) = self.effect(i) {
                    self.transfer(&e, Persist::Dead, &mut lp);
                    self.transfer(&e, Persist::Keep, &mut lk);
                }
            }
        }
        let live_susp: u64 = at_susp.iter().map(|x| u64::from(x.count_ones())).sum();
        // Accesses of object bytes (loads and stores at known places) and those of bytes no
        // suspension keeps (the other lens's question).
        let (mut acc, mut acc_dead) = (0u64, 0u64);
        for i in &f.code {
            let at = match *i {
                Inst::Load { addr, w, .. } | Inst::Store { addr, w, .. } if w != Width::Sig => {
                    match self.place.get(&addr.0) {
                        Some(&Place::At(o)) => Some((o, w.bytes())),
                        _ => None,
                    }
                }
                _ => None,
            };
            if let Some((o, n)) = at
                && let Some((k, s)) = self.span(o, n)
                && self.escaped >> k & 1 == 0
            {
                acc += 1;
                acc_dead += u64::from(!any(&at_susp, s));
            }
        }
        eprintln!(
            "deadvar {} id={} acc={acc} acc_dead={acc_dead} code={} nested={} obj={} esc={} live_at_suspend={} copies={}/{}B dead_p={} dead_k={} gather_copies={}/{}B gdead_p={} gdead_k={} leaves={} lp={} lk={} writes={} wdead_p={} wdead_k={}",
            f.name,
            id.0,
            f.code.len(),
            f.nested,
            obj_bytes,
            esc_bytes,
            live_susp,
            c.copies,
            c.copy_bytes,
            c.cdead_p,
            c.cdead_k,
            c.gcopies,
            c.gcopy_bytes,
            c.gdead_p,
            c.gdead_k,
            c.gleaves,
            c.gleaves_p,
            c.gleaves_k,
            c.writes,
            c.wdead_p,
            c.wdead_k,
        );
    }
}

fn verbose() -> bool {
    std::env::var("YAVS_DEADVAR_CENSUS").is_ok_and(|v| v == "2")
}

/// `YAVS_DEADVAR_CENSUS=1`: what the analysis says about a finished process.
pub(super) fn census(prog: &Program, id: FuncId, f: &crate::Func) {
    static ON: OnceLock<bool> = OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("YAVS_DEADVAR_CENSUS").is_some())
        && f.kind == FuncKind::Process
        && let Some(l) = Live::new(prog, f)
    {
        l.census(id);
    }
}

/// Is the pass on? Off with `YAVS_NO_DEADVAR` set to anything but `0` or the empty string.
pub(super) fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("YAVS_NO_DEADVAR")
            .map(|v| v.is_empty() || v == "0")
            .unwrap_or(true)
    })
}

/// What to do with a write whose bytes are partly or wholly dead.
enum Edit {
    Drop(usize),
    /// A `Copy` cut down to these `(offset, length)` pieces of its bytes.
    Trim(usize, Vec<(u32, u32)>),
}

/// The uses of every register (fixed operands and lists).
fn use_counts(f: &crate::Func) -> Vec<u32> {
    let mut u = vec![0u32; f.reg_ty.len()];
    for i in &f.code {
        i.visit(&mut |r, role, _| {
            if role == Role::Use
                && let Some(c) = u.get_mut(r.0 as usize)
            {
                *c += 1;
            }
        });
        if let Some(l) = i.list() {
            for r in f.regs(l).unwrap_or(&[]) {
                if let Some(c) = u.get_mut(r.0 as usize) {
                    *c += 1;
                }
            }
        }
    }
    u
}

/// `SigGather` into a temporary and a `Copy` of all of it into an object: the gather goes into
/// the object (`v := r`). Returns the edited code positions, or nothing.
fn gather_direct(l: &Live<'_>, code: &mut [Inst]) -> bool {
    let f = l.f;
    let uses = use_counts(f);
    let mut done = false;
    for bl in f.blocks.values() {
        for g in bl.start as usize..bl.end as usize {
            let Inst::SigGather {
                dst: t,
                desc,
                tab,
                part: None,
            } = code[g]
            else {
                continue;
            };
            if !l.once(t)
                || uses.get(t.0 as usize) != Some(&2)
                || !f
                    .code
                    .iter()
                    .any(|i| matches!(*i, Inst::Alloc { dst, .. } if dst == t))
            {
                continue;
            }
            let Some(size) = l.plan(tab, desc).map(|p| p.size) else {
                continue;
            };
            for c in g + 1..bl.end as usize {
                match code[c] {
                    Inst::Const { .. } | Inst::AddrOf { .. } => continue,
                    Inst::Copy { dst, src, bytes }
                        if src == t
                            && l.k(bytes) == i64::try_from(size).ok()
                            && matches!(l.place.get(&dst.0), Some(&Place::At(o)) if l.obj_at(o).is_some()) =>
                    {
                        code[c] = Inst::SigGather {
                            dst,
                            desc,
                            tab,
                            part: None,
                        };
                        code[g] = dead();
                        done = true;
                    }
                    _ => {}
                }
                break;
            }
        }
    }
    done
}

/// The dead writes of the body under `Persist::Keep`.
fn edits(l: &Live<'_>) -> Vec<Edit> {
    let f = l.f;
    let out = l.solve(Persist::Keep);
    let body = f.body.map_or(0, |b| b.0 as usize);
    let mut ed = Vec::new();
    for (b, o) in out.iter().enumerate().skip(body) {
        let mut live = o.clone();
        for (pc, e) in &l.effs[b] {
            let pc = *pc;
            let i = &f.code[pc];
            let removable = matches!(
                i,
                Inst::Store { .. }
                    | Inst::Copy { .. }
                    | Inst::Zero { .. }
                    | Inst::LCopy { .. }
                    | Inst::LFill { .. }
            );
            if removable
                && let Some((o, n)) = l.kill(i)
                && let Some((k, s)) = l.span(o, n)
                && l.escaped >> k & 1 == 0
                && s.1 == n as usize
            {
                let alive: Vec<bool> = (s.0..s.0 + s.1).map(|x| any(&live, (x, 1))).collect();
                if !alive.contains(&true) {
                    // (Its reads go with it: nothing of what it read is kept for it.)
                    ed.push(Edit::Drop(pc));
                    continue;
                }
                if matches!(i, Inst::Copy { .. }) {
                    let mut pieces: Vec<(u32, u32)> = Vec::new();
                    for (x, &a) in alive.iter().enumerate() {
                        if !a {
                            continue;
                        }
                        let x = x as u32;
                        match pieces.last_mut() {
                            // (A short dead gap costs less than another copy.)
                            Some(p) if x - (p.0 + p.1) < 32 => p.1 = x + 1 - p.0,
                            _ => pieces.push((x, 1)),
                        }
                    }
                    let kept: u32 = pieces.iter().map(|p| p.1).sum();
                    if pieces.len() <= 2 && n - kept >= 64 {
                        ed.push(Edit::Trim(pc, pieces));
                    }
                }
            }
            l.transfer(e, Persist::Keep, &mut live);
        }
    }
    ed
}

/// Drop the writes of a process's frame objects that nothing reads, and gather `v := r` into the
/// variable itself (`docs/decisions.md` "Dead bytes of frame objects").
pub(super) fn run(
    prog: &Program,
    f: &mut crate::Func,
    layouts: &mut FxMap<TyId, Option<Vec<u64>>>,
) -> bool {
    if !enabled()
        || f.kind != FuncKind::Process
        || KEEP_VARS.load(std::sync::atomic::Ordering::Relaxed)
        || super::external()
        || f.code.iter().any(|i| {
            matches!(
                i,
                Inst::Cover { .. } | Inst::CoverIn { .. } | Inst::CoverCond { .. }
            )
        })
    {
        return false;
    }
    // ponytail: one round; a write that only a dropped write read stays. Iterate if that shows.
    {
        let done = {
            let Some(l) = Live::new(prog, f) else {
                return false;
            };
            let mut code = f.code.clone();
            // (The edits are of the code before the gathers moved: a `Copy` a gather replaced is
            // dropped only when all of it is dead, as the gather then is.)
            let direct = gather_direct(&l, &mut code);
            let ed = edits(&l);
            if !direct && ed.is_empty() {
                return false;
            }
            let mut o = Opt::new(prog, f, code, f.operands.clone(), f.nested, layouts);
            o.analyse();
            for e in ed {
                match e {
                    Edit::Drop(pc) => o.code[pc] = dead(),
                    Edit::Trim(pc, pieces) => {
                        let Inst::Copy { dst, src, .. } = o.code[pc] else {
                            continue;
                        };
                        let Some(&Place::At(at)) = l.place.get(&dst.0) else {
                            continue;
                        };
                        let pool = f.code.iter().find_map(|i| match *i {
                            Inst::AddrOf { dst, space, off } if dst == src && l.once(src) => {
                                Some((space, off))
                            }
                            _ => None,
                        });
                        o.code[pc] = dead();
                        for (off, len) in pieces {
                            let d = o.fresh(RegTy::Ptr, 1);
                            o.inserts.push((
                                pc,
                                Inst::AddrOf {
                                    dst: d,
                                    space: AddrSpace::Frame,
                                    off: at + off,
                                },
                            ));
                            let s = o.fresh(RegTy::Ptr, 1);
                            match pool {
                                Some((space, so)) => o.inserts.push((
                                    pc,
                                    Inst::AddrOf {
                                        dst: s,
                                        space,
                                        off: so + off,
                                    },
                                )),
                                None => {
                                    let k = o.fresh(RegTy::I64, 1);
                                    o.inserts.push((
                                        pc,
                                        Inst::Const {
                                            dst: k,
                                            v: i64::from(off),
                                        },
                                    ));
                                    o.inserts.push((
                                        pc,
                                        Inst::Index {
                                            dst: s,
                                            base: src,
                                            pos: k,
                                            stride: 1,
                                        },
                                    ));
                                }
                            }
                            let n = o.fresh(RegTy::I64, 1);
                            o.inserts.push((
                                pc,
                                Inst::Const {
                                    dst: n,
                                    v: i64::from(len),
                                },
                            ));
                            o.inserts.push((
                                pc,
                                Inst::Copy {
                                    dst: d,
                                    src: s,
                                    bytes: n,
                                },
                            ));
                        }
                    }
                }
            }
            o.dce();
            let mut loc = f.loc.clone();
            let mut via = f.via.clone();
            let blocks = o.compact(&mut loc, (!via.is_empty()).then_some(&mut via));
            (o.code, o.operands, o.new_regs, loc, via, blocks)
        };
        let (code, operands, new_regs, loc, via, blocks) = done;
        for (i, nb) in blocks.iter().enumerate() {
            if let Some(b) = f.blocks.get_mut(BlockId(i as u32)) {
                b.start = nb.0;
                b.end = nb.1;
            }
        }
        f.code = code;
        f.loc = loc;
        f.via = via;
        f.operands = operands;
        f.reg_ty.extend(new_regs);
    }
    true
}
