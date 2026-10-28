//! What the declarative part of a process leaves in its frame (#678).
//!
//! A specialised copy of a process (`copies.rs`) takes over between two activations: the frame
//! already holds what the declarative part made, and the declarative part never runs in the copy.
//! The copy is for one instance's data, so that part is a function of the data: this module
//! runs it on the abstract machine below (registers, temporary blocks, heap objects and the
//! frame, as known bytes and addresses) and reports the frame bytes, the words that hold an
//! address and the words that point to the descriptor of a heap object (a record variable's
//! `[data, desc, handle]` slot) that the body only reads. The optimiser answers the body's loads
//! of those from the facts, so the layout and the bounds of a process variable fold like those
//! of an object in the constant pool.
//!
//! Anything the machine does not model gives an unknown value; a branch on an unknown value or
//! a run of more than `MAX_STEPS` instructions gives no facts at all. A check that fails would
//! have stopped the run in initialisation, so after the switch every check passed. The
//! descriptor of a heap object is never written after it is made (the constraint of an object
//! does not change), which is the one fact taken on the language's word: a pointer to it may go
//! anywhere.

use std::rc::Rc;

use super::*;

/// Most instructions of the declarative part run; a block of bytes tracked at most `MAX_BYTES`.
const MAX_STEPS: usize = 50_000;
const MAX_BYTES: i64 = 1 << 18;

/// Frame ranges: offset and length.
type Ranges = Vec<(i64, i64)>;

/// What the body can rely on, by frame offset.
#[derive(Default)]
pub(super) struct Facts {
    /// Bytes of the frame that are known and written nowhere else.
    pub bytes: Vec<Option<u8>>,
    /// 8-byte words that hold an address.
    pub addrs: FxMap<u32, (AddrSpace, u32)>,
    /// 8-byte words that point to a heap object's descriptor: its bytes and the offset the
    /// pointer has in them.
    pub descs: FxMap<u32, (Rc<[u8]>, u32)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Av {
    Unk,
    K(i64),
    /// An address in a space: the frame, the instance data, the constant pool.
    A(AddrSpace, i64),
    /// An address in a tracked block.
    B(u32, i64),
    /// The handle of a heap object (its block).
    H(u32),
}

/// Bytes that are known or not, and bytes that something unseen may write (`poisoned`): they
/// stay unknown, whatever is stored through the pointers the machine knows.
struct Mem {
    v: Vec<Option<u8>>,
    poisoned: Vec<bool>,
}

impl Mem {
    fn new(v: Vec<Option<u8>>) -> Mem {
        let poisoned = vec![false; v.len()];
        Mem { v, poisoned }
    }

    fn get(&self, off: i64, n: usize) -> Vec<Option<u8>> {
        usize::try_from(off)
            .ok()
            .and_then(|s| self.v.get(s..s.checked_add(n)?))
            .map_or_else(|| vec![None; n], <[_]>::to_vec)
    }

    /// Store `data` at `off`; out of range is ignored (the original would have failed).
    fn put(&mut self, off: i64, data: &[Option<u8>]) {
        if let Ok(s) = usize::try_from(off) {
            for (i, &d) in data.iter().enumerate() {
                if let Some(x) = self.v.get_mut(s + i)
                    && !self.poisoned[s + i]
                {
                    *x = d;
                }
            }
        }
    }

    fn poisoned(&self, off: i64, n: usize) -> bool {
        usize::try_from(off).ok().is_none_or(|s| {
            self.poisoned
                .get(s..s.saturating_add(n))
                .is_none_or(|p| p.iter().any(|&x| x))
        })
    }

    /// Something may write `[off, off + n)`: unknown from now on.
    fn poison(&mut self, off: i64, n: i64) {
        let lo = usize::try_from(off).unwrap_or(0).min(self.v.len());
        let hi = usize::try_from(off.saturating_add(n))
            .unwrap_or(0)
            .clamp(lo, self.v.len());
        self.v[lo..hi].fill(None);
        self.poisoned[lo..hi].fill(true);
    }
}

struct Blk {
    m: Mem,
    /// The type of a heap object.
    heap: Option<TyId>,
    /// A pointer was stored in it.
    holds_ptr: bool,
}

struct Ev<'o, 'a> {
    o: &'o Opt<'a>,
    regs: Vec<Av>,
    fb: Mem,
    fw: FxMap<u32, Av>,
    blks: Vec<Blk>,
    /// Blocks whose address was stored in memory a call can reach.
    leaked: Vec<u32>,
    /// Bytes of all blocks made (the machine's own memory is bounded).
    tracked: usize,
    /// A pointer was stored in the frame other than as an aligned word.
    frame_ptr: bool,
}

fn bytes_of(v: i64, n: usize) -> Vec<Option<u8>> {
    v.to_le_bytes()[..n].iter().map(|&b| Some(b)).collect()
}

impl Ev<'_, '_> {
    fn reg(&self, r: Reg) -> Av {
        self.regs.get(r.0 as usize).copied().unwrap_or(Av::Unk)
    }

    fn set(&mut self, r: Reg, v: Av) {
        if let Some(x) = self.regs.get_mut(r.0 as usize) {
            *x = v;
        }
    }

    fn kreg(&self, r: Reg) -> Option<i64> {
        match self.reg(r) {
            Av::K(v) => Some(v),
            _ => None,
        }
    }

    fn pointer(a: Av) -> bool {
        matches!(a, Av::A(..) | Av::B(..))
    }

    /// The `n` bytes at `a`, each known or not.
    fn read(&self, a: Av, n: usize) -> Vec<Option<u8>> {
        let unk = vec![None; n];
        match a {
            Av::B(b, off) => self.blks.get(b as usize).map_or(unk, |k| k.m.get(off, n)),
            Av::A(AddrSpace::Frame, off) => self.fb.get(off, n),
            Av::A(sp @ (AddrSpace::Inst | AddrSpace::Const(_) | AddrSpace::Pkg(_)), off)
                if n as i64 <= MAX_BYTES =>
            {
                let mut buf = vec![0u8; n];
                match u32::try_from(off) {
                    Ok(off) if self.o.const_read(Opn::A(sp, off), &mut buf) => {
                        buf.into_iter().map(Some).collect()
                    }
                    _ => unk,
                }
            }
            _ => unk,
        }
    }

    fn write(&mut self, a: Av, data: &[Option<u8>]) {
        match a {
            Av::B(b, off) => {
                if let Some(k) = self.blks.get_mut(b as usize) {
                    k.m.put(off, data);
                }
            }
            Av::A(AddrSpace::Frame, off) => {
                self.fb.put(off, data);
                let (lo, hi) = (off, off.saturating_add(data.len() as i64));
                self.fw.retain(|&w, _| {
                    let w = i64::from(w);
                    w + 8 <= lo || hi <= w
                });
            }
            _ => {}
        }
    }

    /// Something may write through `a`: what is behind it stays unknown. (A write through a
    /// pointer reaches forward from it.)
    fn clobber(&mut self, a: Av) {
        match a {
            Av::B(b, off) => {
                if let Some(k) = self.blks.get_mut(b as usize) {
                    k.m.poison(off, i64::MAX);
                }
            }
            Av::A(AddrSpace::Frame, off) => {
                let (lo, n) = self.o.frame_range(off, None);
                self.poison_frame(lo, n);
            }
            _ => {}
        }
    }

    fn poison_frame(&mut self, lo: i64, n: i64) {
        self.fb.poison(lo, n);
        let hi = lo.saturating_add(n);
        self.fw.retain(|&w, _| {
            let w = i64::from(w);
            w + 8 <= lo || hi <= w
        });
    }

    /// Does memory at `a` hold a pointer the machine does not follow?
    fn holds(&self, a: Av) -> bool {
        match a {
            Av::B(k, _) => self.blks.get(k as usize).is_some_and(|k| k.holds_ptr),
            Av::A(AddrSpace::Frame, _) => self.frame_ptr,
            _ => false,
        }
    }

    fn mark_holds(&mut self, a: Av) {
        match a {
            Av::B(k, _) => {
                if let Some(k) = self.blks.get_mut(k as usize) {
                    k.holds_ptr = true;
                }
            }
            Av::A(AddrSpace::Frame, _) => self.frame_ptr = true,
            _ => {}
        }
    }

    /// Memory at `ops` is copied or written through together: pointers in one may be in all.
    fn spread(&mut self, ops: &[Av]) {
        if ops.iter().any(|&a| self.holds(a)) {
            for &a in ops {
                self.mark_holds(a);
            }
        }
    }

    /// A call: what its arguments and the memory it can reach from them hold stays unknown.
    fn call_effects(&mut self, args: &[Reg]) {
        let ops: Vec<Av> = args.iter().map(|&r| self.reg(r)).collect();
        self.spread(&ops);
        for a in ops {
            self.clobber(a);
        }
        for b in std::mem::take(&mut self.leaked) {
            self.clobber(Av::B(b, 0));
        }
    }

    fn new_block(&mut self, bytes: Vec<Option<u8>>, heap: Option<TyId>) -> Option<u32> {
        self.tracked += bytes.len();
        if self.blks.len() >= 4096 || self.tracked > 4 * MAX_BYTES as usize {
            return None;
        }
        self.blks.push(Blk {
            m: Mem::new(bytes),
            heap,
            holds_ptr: false,
        });
        Some(self.blks.len() as u32 - 1)
    }

    fn store(&mut self, addr: Av, src: Av, w: Width) {
        let n = w.bytes() as usize;
        match src {
            Av::K(v) if w != Width::Sig => self.write(addr, &bytes_of(v, n)),
            s if Self::pointer(s) || matches!(s, Av::H(_)) => match addr {
                // A pointer word of the frame is read back exactly (unless something unseen may
                // write it).
                Av::A(AddrSpace::Frame, off) if off % 8 == 0 && w == Width::B8 => {
                    self.write(addr, &[None; 8]);
                    match u32::try_from(off) {
                        Ok(o) if !self.fb.poisoned(off, 8) => {
                            self.fw.insert(o, s);
                        }
                        _ => self.frame_ptr = true,
                    }
                }
                // Other places the machine keeps bytes of: a load of an unknown word out of them
                // stops it.
                Av::A(AddrSpace::Frame, _) => {
                    self.write(addr, &vec![None; n]);
                    self.frame_ptr = true;
                }
                Av::B(b, _) => {
                    if let Av::B(p, _) = s {
                        self.leaked.push(p);
                    }
                    if let Some(k) = self.blks.get_mut(b as usize) {
                        k.holds_ptr = true;
                    }
                    self.write(addr, &vec![None; n]);
                }
                // Memory it cannot see: the pointer may come back as an unknown value.
                _ => self.clobber(s),
            },
            _ => self.write(addr, &vec![None; n]),
        }
    }

    /// The value of a load of `w` at `a`; `Err` when the machine must stop (an unknown word
    /// out of a block that holds pointers).
    fn load(&self, a: Av, w: Width) -> Result<Av, ()> {
        if w == Width::Sig {
            return Ok(Av::Unk);
        }
        if let Av::A(AddrSpace::Frame, off) = a
            && w == Width::B8
            && let Ok(o) = u32::try_from(off)
            && let Some(&v) = self.fw.get(&o)
        {
            return Ok(v);
        }
        let n = w.bytes() as usize;
        let b = self.read(a, n);
        if b.iter().any(Option::is_none) {
            // (An unknown word may be a pointer the machine cannot follow.)
            if self.holds(a) {
                return Err(());
            }
            return Ok(Av::Unk);
        }
        let mut x = [0u8; 8];
        for (d, s) in x.iter_mut().zip(&b) {
            *d = s.unwrap_or(0);
        }
        Ok(Av::K(if n == 4 {
            i64::from(i32::from_le_bytes([x[0], x[1], x[2], x[3]]))
        } else {
            u64::from_le_bytes(x) as i64
        }))
    }

    fn shift(a: Av, d: i64) -> Av {
        match a {
            Av::A(s, o) => o.checked_add(d).map_or(Av::Unk, |o| Av::A(s, o)),
            Av::B(b, o) => o.checked_add(d).map_or(Av::Unk, |o| Av::B(b, o)),
            _ => Av::Unk,
        }
    }

    /// `(left, right, direction)` of dimension `dim` of the descriptor in `dope`, if known.
    fn dim(&self, dope: Reg, dim: u8) -> Option<(i64, i64, i64)> {
        let b = self.read(Self::shift(self.reg(dope), 24 * i64::from(dim)), 24);
        let w = |k: usize| -> Option<i64> {
            let mut x = [0u8; 8];
            for (d, s) in x.iter_mut().zip(&b[k * 8..k * 8 + 8]) {
                *d = (*s)?;
            }
            Some(i64::from_le_bytes(x))
        };
        Some((w(0)?, w(1)?, w(2)?))
    }

    /// The known bytes of the descriptor of `ty` at `desc`.
    fn desc_bytes(&self, ty: TyId, desc: Av) -> Option<Vec<u8>> {
        let ds = Ty::desc_size(&self.o.prog.types, ty)?;
        if ds as i64 > MAX_BYTES {
            return None;
        }
        self.read(desc, ds as usize).into_iter().collect()
    }

    /// Run instruction `i`. `Ok(Some(block))` is a jump, `Ok(None)` falls through to the next
    /// instruction; `Err` stops the machine.
    fn step(&mut self, i: &Inst) -> Result<Option<usize>, ()> {
        let o = self.o;
        let (f, types) = (o.f, &o.prog.types);
        match *i {
            Inst::Const { dst, v } => self.set(dst, Av::K(v)),
            Inst::Move { dst, src } => {
                let v = self.reg(src);
                self.set(dst, v);
            }
            Inst::AddrOf { dst, space, off } => self.set(dst, Av::A(space, i64::from(off))),
            Inst::Select { dst, c, a, b } => {
                let v = match self.kreg(c) {
                    Some(c) => self.reg(if c != 0 { a } else { b }),
                    None => {
                        for r in [a, b] {
                            let x = self.reg(r);
                            self.clobber(x);
                        }
                        Av::Unk
                    }
                };
                self.set(dst, v);
            }
            Inst::Alloc { dst, size } => {
                let v = match self.kreg(size) {
                    Some(n) if (0..=MAX_BYTES).contains(&n) => self
                        .new_block(vec![Some(0); n as usize], None)
                        .map_or(Av::Unk, |b| Av::B(b, 0)),
                    _ => Av::Unk,
                };
                self.set(dst, v);
            }
            Inst::Index {
                dst,
                base,
                pos,
                stride,
            } => {
                let b = self.reg(base);
                let v = match (self.kreg(pos), Self::pointer(b)) {
                    (Some(p), true) => {
                        Self::shift(b, p.checked_mul(i64::from(stride)).unwrap_or(i64::MAX))
                    }
                    _ => {
                        self.clobber(b);
                        Av::Unk
                    }
                };
                self.set(dst, v);
            }
            Inst::Load { dst, addr, w } => {
                let a = self.reg(addr);
                let v = self.load(a, w)?;
                self.set(dst, v);
            }
            Inst::Store { addr, src, w } => {
                let (a, s) = (self.reg(addr), self.reg(src));
                self.store(a, s, w);
            }
            Inst::Copy { dst, src, bytes } => {
                let (d, s) = (self.reg(dst), self.reg(src));
                self.spread(&[d, s]);
                match self.kreg(bytes) {
                    Some(n) if (0..=MAX_BYTES).contains(&n) => {
                        let data = self.read(s, n as usize);
                        self.write(d, &data);
                    }
                    _ => self.clobber(d),
                }
            }
            Inst::MakeDope { at, parts } => {
                let a = self.reg(at);
                let rs = f.regs(parts).unwrap_or(&[]);
                let mut data = Vec::with_capacity(rs.len() * 8);
                for &r in rs {
                    match self.kreg(r) {
                        Some(v) => data.extend(bytes_of(v, 8)),
                        None => data.extend([None; 8]),
                    }
                }
                self.write(a, &data);
            }
            Inst::DopeLen { dst, dope, dim } => {
                let v = match self.dim(dope, dim) {
                    Some((l, r, d)) => dim_len(l, r, d == 1)
                        .filter(|&n| n <= i64::MAX as u64)
                        .map_or(Av::Unk, |n| Av::K(n as i64)),
                    None => Av::Unk,
                };
                self.set(dst, v);
            }
            Inst::Pos {
                dst,
                dope,
                dim,
                idx,
                ..
            } => {
                let v = match (self.dim(dope, dim), self.kreg(idx)) {
                    (Some((l, r, d)), Some(i)) => match d {
                        0 if l <= i && i <= r => Av::K(i.wrapping_sub(l)),
                        1 if r <= i && i <= l => Av::K(l.wrapping_sub(i)),
                        _ => Av::Unk,
                    },
                    _ => Av::Unk,
                };
                self.set(dst, v);
            }
            Inst::LayoutOf { dst, ty, desc, .. } => {
                let d = self.reg(desc);
                let words = self
                    .desc_bytes(ty, d)
                    .and_then(|b| Ty::layout(types, &o.prog.consts, ty, &b).ok())
                    .map(|(l, _)| layout_words(&l));
                let v = match words {
                    Some(w) => {
                        let bytes = w.iter().flat_map(|&x| bytes_of(x as i64, 8)).collect();
                        self.new_block(bytes, None).map_or(Av::Unk, |b| Av::B(b, 0))
                    }
                    None => Av::Unk,
                };
                self.set(dst, v);
            }
            Inst::Field {
                dst,
                base,
                layout,
                n,
            } => {
                let off = self.read(Self::shift(self.reg(layout), 16 + 8 * i64::from(n)), 8);
                let v = match off.into_iter().collect::<Option<Vec<u8>>>() {
                    Some(b) => {
                        let o = u64::from_le_bytes(b.try_into().unwrap_or([0; 8]));
                        Self::shift(self.reg(base), o as i64)
                    }
                    None => {
                        let b = self.reg(base);
                        self.clobber(b);
                        Av::Unk
                    }
                };
                self.set(dst, v);
            }
            Inst::Bin { op, dst, a, b } => {
                let v = match (self.kreg(a), self.kreg(b)) {
                    (Some(x), Some(y)) => fold_int(op, x, y).map_or(Av::Unk, Av::K),
                    _ => Av::Unk,
                };
                self.taint(i, v, dst);
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
                let v = match (self.kreg(a), self.kreg(b), self.kreg(lo), self.kreg(hi)) {
                    (Some(x), Some(y), Some(l), Some(h)) => fold_int(op, x, y)
                        .filter(|v| l <= *v && *v <= h)
                        .map_or(Av::Unk, Av::K),
                    _ => Av::Unk,
                };
                self.taint(i, v, dst);
            }
            Inst::Un { op, dst, a } => {
                let v = match self.kreg(a) {
                    Some(x) => match op {
                        IntUnOp::Neg => x.checked_neg(),
                        IntUnOp::Abs => x.checked_abs(),
                    }
                    .map_or(Av::Unk, Av::K),
                    None => Av::Unk,
                };
                self.taint(i, v, dst);
            }
            Inst::Cmp { op, dst, a, b } => {
                let v = match (self.kreg(a), self.kreg(b)) {
                    (Some(x), Some(y)) => Av::K(i64::from(cmp(op, x.cmp(&y)))),
                    _ => Av::Unk,
                };
                self.taint(i, v, dst);
            }
            Inst::Bool { op, dst, a, b } => {
                let (x, y) = (self.kreg(a), self.kreg(b));
                let v = match (op, x, y) {
                    (BoolOp::Not, Some(x), _) => Some(x == 0),
                    (BoolOp::And, Some(x), Some(y)) => Some(x != 0 && y != 0),
                    (BoolOp::Or, Some(x), Some(y)) => Some(x != 0 || y != 0),
                    (BoolOp::Xor, Some(x), Some(y)) => Some((x != 0) != (y != 0)),
                    _ => None,
                };
                self.taint(i, v.map_or(Av::Unk, |v| Av::K(i64::from(v))), dst);
            }
            // After a failed check the process would not be running: they all passed.
            Inst::Check { .. }
            | Inst::FCheck { .. }
            | Inst::SubtypeCheck { .. }
            | Inst::Stmt
            | Inst::TempRoot { .. } => {}
            Inst::Unreachable { what } if what == DEAD_MSG => {}
            Inst::New {
                dst,
                ty,
                init,
                size,
                shared: false,
            } => {
                let (a, n) = (self.reg(init), self.kreg(size));
                let v = match n {
                    Some(n) if (0..=MAX_BYTES).contains(&n) => {
                        let data = self.read(a, n as usize);
                        self.new_block(data, Some(ty)).map_or(Av::Unk, Av::H)
                    }
                    _ => Av::Unk,
                };
                self.set(dst, v);
            }
            Inst::NewFill {
                dst,
                ty,
                desc,
                dsize,
                size,
                shared: false,
                ..
            } => {
                let (d, n) = (self.reg(desc), self.kreg(size));
                let v = match n {
                    Some(n) if (0..=MAX_BYTES).contains(&n) => {
                        let mut data = self.read(d, dsize as usize);
                        data.resize(dsize as usize + n as usize, None);
                        self.new_block(data, Some(ty)).map_or(Av::Unk, Av::H)
                    }
                    _ => Av::Unk,
                };
                self.set(dst, v);
            }
            Inst::Deref { dst, handle, .. } => {
                let v = match self.reg(handle) {
                    Av::H(b) => Av::B(b, 0),
                    _ => Av::Unk,
                };
                self.set(dst, v);
            }
            Inst::Br { to } => return Ok(Some(to.0 as usize)),
            Inst::CondBr { c, t, f } => {
                let c = self.kreg(c).ok_or(())?;
                return Ok(Some(if c != 0 { t } else { f }.0 as usize));
            }
            Inst::Case { v, table } => {
                let x = self.kreg(v).ok_or(())?;
                let t = o.prog.cases.get(table).ok_or(())?;
                let to = t
                    .arms
                    .iter()
                    .find(|a| a.0 <= x && x <= a.1)
                    .map_or(t.default, |a| a.2);
                return Ok(Some(to.0 as usize));
            }
            Inst::Ret { .. } | Inst::Unreachable { .. } | Inst::Suspend { .. } => return Err(()),
            Inst::Call { args, dst, .. }
            | Inst::CallNative { args, dst, .. }
            | Inst::CallForeign { args, dst, .. } => self.call(args, dst, None),
            Inst::CallIndirect { func, args, dst } => self.call(args, dst, Some(func)),
            Inst::CallProt { obj, args, dst, .. } => self.call(args, dst, Some(obj)),
            _ => self.opaque(i),
        }
        Ok(None)
    }

    fn call(&mut self, args: crate::Regs, dst: Option<Reg>, extra: Option<Reg>) {
        let mut rs: Vec<Reg> = self.o.f.regs(args).unwrap_or(&[]).to_vec();
        rs.extend(extra);
        self.call_effects(&rs);
        if let Some(d) = dst {
            self.set(d, Av::Unk);
        }
    }

    /// An instruction without a model: results unknown, what its pointer operands reach too.
    fn opaque(&mut self, i: &Inst) {
        let mut uses = Vec::new();
        let mut defs = Vec::new();
        i.visit(&mut |r, role, _| match role {
            Role::Use => uses.push(r),
            Role::Def => defs.push(r),
        });
        if let Some(l) = i.list() {
            uses.extend_from_slice(self.o.f.regs(l).unwrap_or(&[]));
        }
        let ops: Vec<Av> = uses.iter().map(|&r| self.reg(r)).collect();
        self.spread(&ops);
        for a in ops {
            self.clobber(a);
        }
        for r in defs {
            self.set(r, Av::Unk);
        }
    }

    /// An arithmetic result `v` for `dst`; an unknown one may be pointer arithmetic.
    fn taint(&mut self, i: &Inst, v: Av, dst: Reg) {
        if v == Av::Unk {
            self.opaque(i);
        }
        self.set(dst, v);
    }
}

/// The words `LayoutOf` writes for the layout `l`: `[size, align, offsets...]`, or without
/// offsets `[size, align, stride, count]` (as the interpreter's `layout_build`).
pub(super) fn layout_words(l: &crate::Layout) -> Vec<u64> {
    let mut w = vec![l.size, l.align];
    if l.offsets.is_empty() {
        w.extend([l.stride, l.count]);
    } else {
        w.extend(&l.offsets);
    }
    w
}

impl Opt<'_> {
    /// The frame object containing offset `o`: its range, or `[o, o + n)` for `n` bytes.
    pub(super) fn frame_range(&self, o: i64, n: Option<i64>) -> (i64, i64) {
        let frame = i64::from(self.f.frame);
        let objs = |o: i64| {
            self.f
                .locals
                .iter()
                .map(|l| (i64::from(l.slot), i64::from(l.size), l.ty))
                .chain(self.f.params.iter().map(|p| {
                    (
                        i64::from(p.slot),
                        i64::from(p.width.bytes()),
                        TyId(u32::MAX),
                    )
                }))
                .filter(move |&(s, z, _)| s <= o && o < s + z)
        };
        match n {
            Some(n) => (o, n),
            None => match objs(o).next() {
                Some((s, z, t)) => match self.prog.types.get(t) {
                    Some(Ty::Array { dims, .. }) if o == s + 24 => (o, 24 * i64::from(*dims)),
                    _ => (s, z),
                },
                None => {
                    let end = self
                        .f
                        .locals
                        .iter()
                        .map(|l| i64::from(l.slot))
                        .chain(self.f.params.iter().map(|p| i64::from(p.slot)))
                        .filter(|&s| s > o)
                        .min()
                        .unwrap_or(frame);
                    (o, (end - o).max(1))
                }
            },
        }
    }

    /// Is `o` inside the descriptor of an array variable, `[data, dope, handle, descriptor]`'s
    /// last `24 * dims` bytes? Nothing writes it after the declaration. (An array of composites
    /// has a `[data, desc, handle]` slot of 24 bytes and no descriptor in it.)
    fn in_descriptor(&self, o: i64) -> bool {
        self.f.locals.iter().any(|l| {
            matches!(self.prog.types.get(l.ty), Some(Ty::Array { dims, .. })
                if o >= i64::from(l.slot) + 24
                    && o < i64::from(l.slot) + 24 + 24 * i64::from(*dims)
                    && o < i64::from(l.slot) + i64::from(l.size))
        })
    }

    /// Frame ranges the body writes, and those whose address is used other than directly by a
    /// load or store anywhere (a pointer may reach them), by the walk's view of the addresses.
    fn frame_writes(&self) -> (Ranges, Ranges) {
        let body = self.f.body.map_or(0, |b| b.0 as usize);
        let (mut wr, mut esc) = (Vec::new(), Vec::new());
        let fa = |r: Reg| match self.opn(r) {
            Opn::A(AddrSpace::Frame, o) => Some(i64::from(o)),
            _ => None,
        };
        let k = |r: Reg| match self.opn(r) {
            Opn::K(n) => Some(n),
            _ => None,
        };
        for b in 0..self.blocks.len() {
            for pc in self.range(b) {
                self.classify(
                    self.f,
                    &self.code[pc],
                    &fa,
                    &k,
                    b >= body,
                    &mut wr,
                    &mut esc,
                );
            }
        }
        (wr, esc)
    }

    /// What instruction `i` of `g` does to the frame of this process: `fa` gives the frame
    /// offset a register holds the address of, `k` the constant it holds. A write goes to `wr`
    /// (only when `writes`: the declarative part's own are no fact against the frame it made),
    /// any other use of an address to `esc`.
    #[allow(clippy::too_many_arguments)]
    fn classify(
        &self,
        g: &crate::Func,
        i: &Inst,
        fa: &dyn Fn(Reg) -> Option<i64>,
        k: &dyn Fn(Reg) -> Option<i64>,
        writes: bool,
        wr: &mut Ranges,
        esc: &mut Ranges,
    ) {
        let mut escape = |o: i64| {
            if !self.in_descriptor(o) {
                esc.push(self.frame_range(o, None));
            }
        };
        match *i {
            Inst::Load { .. } => {}
            Inst::Store { addr, src, w } => {
                if let Some(o) = fa(addr)
                    && writes
                {
                    wr.push((o, i64::from(w.bytes())));
                }
                if let Some(o) = fa(src) {
                    escape(o);
                }
            }
            Inst::Copy { dst, bytes, .. } => {
                if let Some(o) = fa(dst)
                    && writes
                {
                    wr.push(match k(bytes) {
                        Some(n) => (o, n),
                        None => self.frame_range(o, None),
                    });
                }
            }
            Inst::MakeDope { at, parts } => {
                if let Some(o) = fa(at)
                    && writes
                {
                    wr.push((o, 8 * i64::from(parts.len)));
                }
                for &r in g.regs(parts).unwrap_or(&[]) {
                    if let Some(o) = fa(r) {
                        escape(o);
                    }
                }
            }
            _ => {
                i.visit(&mut |r, role, _| {
                    if role == Role::Use
                        && let Some(o) = fa(r)
                    {
                        escape(o);
                    }
                });
                if let Some(l) = i.list() {
                    for &r in g.regs(l).unwrap_or(&[]) {
                        if let Some(o) = fa(r) {
                            escape(o);
                        }
                    }
                }
            }
        }
    }

    /// What the subprograms nested in process `id` do to its frame, added to `wr` and `esc`:
    /// they reach it by `AddrOf { space: Up(depth) }`, so their writes and their uses of such an
    /// address (a derived address too: the whole object it is in) are the same as the process's
    /// own. An address register defined twice gives up on the whole frame.
    fn nested_writes(&self, id: FuncId, wr: &mut Ranges, esc: &mut Ranges) {
        for (_, g) in self.prog.funcs.iter() {
            let (mut depth, mut p) = (0u8, g.parent);
            while let Some(q) = p
                && q != id
                && depth < 32
            {
                depth += 1;
                p = self.prog.funcs.get(q).and_then(|h| h.parent);
            }
            if p != Some(id) {
                continue;
            }
            depth += 1;
            let g = g.full();
            let (mut up, mut defs) = (FxMap::<u32, i64>::default(), vec![0u8; g.reg_ty.len()]);
            for i in &g.code {
                for_defs_fast(i, |d| {
                    if let Some(n) = defs.get_mut(d.0 as usize) {
                        *n = n.saturating_add(1);
                    }
                });
                if let Inst::AddrOf {
                    dst,
                    space: AddrSpace::Up(d),
                    off,
                } = *i
                    && d == depth
                {
                    up.insert(dst.0, i64::from(off));
                }
            }
            if up.keys().any(|&r| defs.get(r as usize) != Some(&1)) {
                esc.push((0, i64::MAX));
                continue;
            }
            let fa = |r: Reg| up.get(&r.0).copied();
            let kc = |r: Reg| {
                g.code.iter().find_map(|i| match *i {
                    Inst::Const { dst, v } if dst == r && defs[r.0 as usize] == 1 => Some(v),
                    _ => None,
                })
            };
            for i in &g.code {
                self.classify(g, i, &fa, &kc, true, wr, esc);
            }
        }
    }

    /// The facts of a process copy's declarative part (see the module), or `None`.
    pub(super) fn decl_facts(&self) -> Option<Facts> {
        let f = self.f;
        let body = f.body?.0 as usize;
        if body == 0 || body >= self.blocks.len() || f.frame > MAX_BYTES as u32 {
            return None;
        }
        let (mut wr, mut esc) = self.frame_writes();
        if self.nested {
            self.nested_writes(self.fid?, &mut wr, &mut esc);
        }
        let mut ev = Ev {
            o: self,
            regs: vec![Av::Unk; f.reg_ty.len()],
            fb: Mem::new(vec![None; f.frame as usize]),
            fw: FxMap::default(),
            blks: Vec::new(),
            leaked: Vec::new(),
            tracked: 0,
            frame_ptr: false,
        };
        // A pointer the machine does not see may write these at any time.
        for &(lo, n) in &esc {
            ev.poison_frame(lo, n);
        }
        let (mut b, mut steps) = (0usize, 0usize);
        while b != body {
            let r = f.blocks.get(BlockId(b as u32))?;
            let mut next = None;
            for pc in r.start as usize..r.end as usize {
                steps += 1;
                if steps > MAX_STEPS {
                    return None;
                }
                if let Some(t) = ev.step(&f.code[pc]).ok()? {
                    next = Some(t);
                    break;
                }
            }
            b = next.filter(|&t| t < self.blocks.len())?;
        }
        // The words and bytes the body does not write and no pointer reaches.
        let gone = |o: i64, n: i64| !clear_of(&wr, o, n) || !clear_of(&esc, o, n);
        let mut facts = Facts::default();
        for (o, x) in ev.fb.v.iter().enumerate() {
            facts.bytes.push(x.filter(|_| !gone(o as i64, 1)));
        }
        for (&w, &v) in &ev.fw {
            if gone(i64::from(w), 8) {
                continue;
            }
            match v {
                Av::A(sp, off) => {
                    if let Ok(off) = u32::try_from(off) {
                        facts.addrs.insert(w, (sp, off));
                    }
                }
                // The pointer word of `[data, desc, handle]`: the data pointer before it is
                // the descriptor's end in the same object.
                Av::B(b, 0) => {
                    let Some(k) = ev.blks.get(b as usize) else {
                        continue;
                    };
                    let Some(ty) = k.heap else { continue };
                    let ds = Ty::desc_size(&self.prog.types, ty).unwrap_or(0) as usize;
                    let data = w.checked_sub(8).and_then(|d| ev.fw.get(&d));
                    if ds > 0
                        && data == Some(&Av::B(b, ds as i64))
                        && let Some(bytes) = k.m.get(0, ds).into_iter().collect::<Option<Vec<u8>>>()
                    {
                        facts.descs.insert(w, (bytes.into(), 0));
                    }
                }
                _ => {}
            }
        }
        Some(facts)
    }
}
