//! Sharing the result of a repeated `to_integer` (round 17 of #520).
//!
//! A check `check(x = to_integer(unsigned(a & b)), "..." & image(to_integer(unsigned(a & b))))`
//! converts the same vector twice: the lowering builds each argument as its own signal read
//! and concatenation, and passes it as data address and bounds. The second call is the same
//! function of the same value, so its result is the first one's, except that the second call warns again when the vector is null
//! or holds a metavalue. `to_integer` of an `unsigned` returns 0 exactly then (and for the
//! value 0), so a first result other than 0 proves the second call silent. The code becomes
//!
//! ```text
//!     r1 = call(x)                 ; the first call, as before
//!     ...                          ; the second argument (moves into R by the sink pass)
//!     CondBr r1, FAST, R           ; branches on r1 != 0
//! FAST: r2 = r1; Br J
//! R:    r2 = call(x')              ; the second call as it was: warns again, or returns 0
//!       Br J
//! J:    the rest
//! ```
//!
//! Which calls are the same is found by value numbering one block: scalars by their operation
//! and operands, a block `Alloc` made by what is written into it (stores, descriptors, signal
//! reads, concatenations), all compared by structure at the call. Anything that may suspend the
//! process, change a signal's value or write memory the numbering cannot place (statement
//! markers, calls other than `'image` and the light `ieee` natives, stores through unknown
//! pointers) forgets everything numbered so far, so the two reads are of the same signal values
//! and the replay finds the order of warnings as it was.

use super::*;

type V = u32;

/// What was written into a block of this run, in order.
#[derive(Clone, PartialEq, Eq, Hash)]
enum W {
    Store(V, Width, V),
    Dope(V, Vec<V>),
    SigRead(V, V),
    Cat {
        at: V,
        shape: (u16, u8, bool, Width),
        n: V,
        m: V,
        parts: Vec<V>,
    },
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum K {
    Const(i64),
    Addr(AddrSpace, u32),
    /// A register defined before the block, or not numbered.
    In(Reg),
    Uniq(u32),
    /// The block the `n`-th `Alloc` of the scan made.
    Blk(u32),
    Bin(IntOp, V, V),
    Cmp(CmpOp, V, V),
    /// Address `base + pos * stride` and the same as an offset into its block.
    Idx(V, V, u32),
    Off(V, V, u32),
    SigVal(u32, V, V),
    SigScalar(V),
    /// A block by what it holds.
    Canon(V, Vec<W>),
    /// An address inside a block by what the block holds.
    Derived(V, V, u32),
}

struct Blk {
    size: V,
    writes: Vec<W>,
    /// Something unplaceable may have written it since.
    dead: bool,
}

struct Vn<'a> {
    /// Registers the function defines once, by a constant.
    konst: &'a [Option<i64>],
    /// Registers the function defines once, by an address.
    addr: &'a [Option<(AddrSpace, u32)>],
    kinds: Vec<K>,
    ids: HashMap<K, V>,
    reg: HashMap<Reg, V>,
    /// Address value to (block, offset value).
    ptr: HashMap<V, (usize, V)>,
    blocks: Vec<Blk>,
    uniq: u32,
    /// Steps of `canon` left in this block.
    budget: u32,
}

impl Vn<'_> {
    fn intern(&mut self, k: K) -> V {
        if let Some(&v) = self.ids.get(&k) {
            return v;
        }
        let v = self.kinds.len() as V;
        self.kinds.push(k.clone());
        self.ids.insert(k, v);
        v
    }

    fn uniq(&mut self) -> V {
        self.uniq += 1;
        let u = self.uniq;
        self.intern(K::Uniq(u))
    }

    fn val(&mut self, r: Reg) -> V {
        match self.reg.get(&r) {
            Some(&v) => v,
            None => {
                let i = r.0 as usize;
                let v = match (
                    self.konst.get(i).copied().flatten(),
                    self.addr.get(i).copied().flatten(),
                ) {
                    (Some(c), _) => self.intern(K::Const(c)),
                    (_, Some((sp, off))) => self.intern(K::Addr(sp, off)),
                    _ => self.intern(K::In(r)),
                };
                self.reg.insert(r, v);
                v
            }
        }
    }

    /// Forget what the blocks hold (an unplaceable write may have changed them).
    fn reset(&mut self) {
        for b in &mut self.blocks {
            b.dead = true;
        }
    }

    fn new_block(&mut self, size: V) -> V {
        let b = self.blocks.len() as u32;
        let v = self.intern(K::Blk(b));
        let zero = self.intern(K::Const(0));
        self.blocks.push(Blk {
            size,
            writes: Vec::new(),
            dead: false,
        });
        self.ptr.insert(v, (b as usize, zero));
        v
    }

    /// The value with the blocks it points to replaced by what they hold: equal values are
    /// equal numbers. A block of unknown content, or too deep, is a number of its own.
    fn canon(&mut self, v: V, depth: u32) -> V {
        if self.budget == 0 {
            return self.uniq();
        }
        self.budget -= 1;
        match self.kinds[v as usize].clone() {
            K::Blk(b) => {
                let b = b as usize;
                if self.blocks[b].dead || depth > 6 {
                    return self.uniq();
                }
                let size = self.blocks[b].size;
                let mut out = Vec::new();
                for w in self.blocks[b].writes.clone() {
                    out.push(self.canon_w(w, depth + 1));
                }
                self.intern(K::Canon(size, out))
            }
            K::Idx(base, pos, stride) => {
                let base = self.canon(base, depth + 1);
                self.intern(K::Derived(base, pos, stride))
            }
            _ => v,
        }
    }

    fn canon_w(&mut self, w: W, depth: u32) -> W {
        let c = |s: &mut Self, v: V| s.canon(v, depth);
        match w {
            W::Store(o, w, v) => W::Store(o, w, c(self, v)),
            W::Dope(o, ps) => W::Dope(o, ps),
            W::SigRead(o, s) => W::SigRead(o, s),
            W::Cat {
                at,
                shape,
                n,
                m,
                parts,
            } => W::Cat {
                at,
                shape,
                n,
                m,
                parts: parts.into_iter().map(|p| c(self, p)).collect(),
            },
        }
    }

    /// Record a write through the address `addr`; `false` when it is not into a block of this
    /// scan.
    fn write(&mut self, addr: V, w: impl FnOnce(V) -> W) -> bool {
        match self.ptr.get(&addr) {
            Some(&(b, off)) => {
                self.blocks[b].writes.push(w(off));
                true
            }
            None => false,
        }
    }
}

impl Sink<'_> {
    /// `to_integer` of an `unsigned`: returns 0 for a null or metavalue argument (with a
    /// warning) and warns in no other case.
    fn shareable(&self, native: crate::NativeId) -> bool {
        let Some(n) = self.prog.natives.get(native) else {
            return false;
        };
        let Some(key) = n.symbol.strip_prefix("yavs.ieee.") else {
            return false;
        };
        matches!(
            crate::ieee::parse(key.split_once('@').map_or(key, |k| k.0)),
            Some(crate::ieee::Op::ToInteger { signed: false })
        )
    }

    /// Pairs of positions in block `b`: a `to_integer` call and a later one of the same value.
    fn find_pairs(&self, b: usize) -> Vec<(usize, usize)> {
        let code = &self.code[b];
        let mut vn = Vn {
            konst: &self.konst,
            addr: &self.addr,
            kinds: Vec::new(),
            ids: HashMap::new(),
            reg: HashMap::new(),
            ptr: HashMap::new(),
            blocks: Vec::new(),
            uniq: 0,
            budget: 4000,
        };
        let mut first: HashMap<(crate::NativeId, [V; 4]), usize> = HashMap::new();
        let mut pairs = Vec::new();
        for (p, i) in code.iter().enumerate().take(code.len().saturating_sub(1)) {
            match *i {
                Inst::Const { dst, v } => {
                    let x = vn.intern(K::Const(v));
                    vn.reg.insert(dst, x);
                }
                Inst::AddrOf { dst, space, off } => {
                    let x = vn.intern(K::Addr(space, off));
                    vn.reg.insert(dst, x);
                }
                Inst::Move { dst, src } => {
                    let x = vn.val(src);
                    vn.reg.insert(dst, x);
                }
                Inst::Bin { op, dst, a, b } => {
                    let k = K::Bin(op, vn.val(a), vn.val(b));
                    let x = vn.intern(k);
                    vn.reg.insert(dst, x);
                }
                Inst::Cmp { op, dst, a, b } => {
                    let k = K::Cmp(op, vn.val(a), vn.val(b));
                    let x = vn.intern(k);
                    vn.reg.insert(dst, x);
                }
                Inst::Alloc { dst, size } => {
                    let s = vn.val(size);
                    let x = vn.new_block(s);
                    vn.reg.insert(dst, x);
                }
                Inst::Index {
                    dst,
                    base,
                    pos,
                    stride,
                } => {
                    let (vb, vp) = (vn.val(base), vn.val(pos));
                    let x = vn.intern(K::Idx(vb, vp, stride));
                    if let Some(&(blk, off)) = vn.ptr.get(&vb) {
                        let o = vn.intern(K::Off(off, vp, stride));
                        vn.ptr.insert(x, (blk, o));
                    }
                    vn.reg.insert(dst, x);
                }
                Inst::Store { addr, src, w } => {
                    let (a, s) = (vn.val(addr), vn.val(src));
                    if !vn.write(a, |o| W::Store(o, w, s)) {
                        vn.reset();
                        first.clear();
                    }
                }
                Inst::MakeDope { at, parts } => {
                    let a = vn.val(at);
                    let ps: Vec<V> = self.list(i).iter().map(|&r| vn.val(r)).collect();
                    let _ = parts;
                    if !vn.write(a, |o| W::Dope(o, ps)) {
                        vn.reset();
                        first.clear();
                    }
                }
                Inst::SigVal {
                    dst,
                    sig,
                    offset,
                    len,
                } => {
                    let k = K::SigVal(sig, vn.val(offset), vn.val(len));
                    let x = vn.intern(k);
                    vn.reg.insert(dst, x);
                }
                Inst::SigReadScalar { dst, sig } => {
                    let k = K::SigScalar(vn.val(sig));
                    let x = vn.intern(k);
                    vn.reg.insert(dst, x);
                }
                Inst::SigRead { dst, sig } => {
                    let (a, s) = (vn.val(dst), vn.val(sig));
                    if !vn.write(a, |o| W::SigRead(o, s)) {
                        vn.reset();
                        first.clear();
                    }
                }
                Inst::Cat {
                    dst,
                    n,
                    m,
                    scalars,
                    bias,
                    logic,
                    w,
                    ..
                } => {
                    let a = vn.val(dst);
                    let (vn_n, vn_m) = (vn.val(n), vn.val(m));
                    let parts: Vec<V> = self.list(i).iter().map(|&r| vn.val(r)).collect();
                    if !vn.write(a, |at| W::Cat {
                        at,
                        shape: (scalars, bias, logic, w),
                        n: vn_n,
                        m: vn_m,
                        parts,
                    }) {
                        vn.reset();
                        first.clear();
                    }
                }
                Inst::CallNative { native, dst, .. }
                    if self.is_image(native, self.list(i).len()) || self.light_native(native) =>
                {
                    // `data, left, right, dir` (`ieee::ARRAY_REGS`): the same planes and bounds.
                    if self.shareable(native)
                        && let ([a, l, r, d], Some(_)) = (self.list(i), dst)
                    {
                        let av = vn.val(*a);
                        // Only a fresh block: the content of any other pointer is not known.
                        if matches!(vn.kinds[av as usize], K::Blk(_)) {
                            let c = vn.canon(av, 0);
                            if !matches!(vn.kinds[c as usize], K::Uniq(_)) {
                                let key = (native, [c, vn.val(*l), vn.val(*r), vn.val(*d)]);
                                match first.get(&key) {
                                    Some(&p1) => pairs.push((p1, p)),
                                    None => {
                                        first.insert(key, p);
                                    }
                                }
                            }
                        }
                    }
                    for d in Self::writes(i) {
                        let u = vn.uniq();
                        vn.reg.insert(d, u);
                    }
                }
                _ => {
                    if !writes_nothing(i) {
                        vn.reset();
                        first.clear();
                    }
                    for d in Self::writes(i) {
                        let u = vn.uniq();
                        vn.reg.insert(d, u);
                    }
                }
            }
        }
        pairs
    }

    /// Put the guard and the replay of every pair found; whether any was.
    pub(super) fn share_calls(&mut self) -> bool {
        let mut changed = false;
        for b in 0..self.code.len() {
            let calls = self.code[b]
                .iter()
                .filter(|i| matches!(i, Inst::CallNative { native, .. } if self.shareable(*native)))
                .count();
            if calls < 2 {
                continue;
            }
            let pairs = self.find_pairs(b);
            if !pairs.is_empty() {
                self.split_pairs(b, pairs);
                changed = true;
            }
        }
        changed
    }

    fn fresh(&mut self, ty: RegTy) -> Reg {
        self.reg_ty.push(ty);
        Reg(self.reg_ty.len() as u32 - 1)
    }

    fn split_pairs(&mut self, b: usize, mut pairs: Vec<(usize, usize)>) {
        // The first call's result, where the guards read it: its own register unless something
        // writes that before the last of its second calls, else a copy made right after it.
        let mut saved: HashMap<usize, Reg> = HashMap::new();
        let mut copies: Vec<(usize, Reg, Reg)> = Vec::new();
        for &(p1, _) in &pairs {
            if saved.contains_key(&p1) {
                continue;
            }
            let Inst::CallNative { dst: Some(d), .. } = self.code[b][p1] else {
                continue;
            };
            let last = pairs.iter().filter(|q| q.0 == p1).map(|q| q.1).max();
            let clobbered = last.is_some_and(|l| {
                self.code[b][p1 + 1..l]
                    .iter()
                    .any(|i| Self::writes(i).contains(&d))
            });
            if clobbered {
                let s = self.fresh(self.reg_ty[d.0 as usize]);
                copies.push((p1, d, s));
                saved.insert(p1, s);
            } else {
                saved.insert(p1, d);
            }
        }
        // Copies after the first calls, last position first so the others stay valid; the
        // second calls move down with them.
        copies.sort_by_key(|x| std::cmp::Reverse(x.0));
        for &(p1, d, s) in &copies {
            let loc = self.loc[b][p1];
            self.code[b].insert(p1 + 1, Inst::Move { dst: s, src: d });
            self.loc[b].insert(p1 + 1, loc);
            if !self.via[b].is_empty() {
                let v = self.via[b][p1];
                self.via[b].insert(p1 + 1, v);
            }
            for q in &mut pairs {
                if q.1 > p1 {
                    q.1 += 1;
                }
            }
        }
        // Split at the last second call first: what is before it stays in place.
        let mut seconds: Vec<(usize, Reg)> = pairs
            .iter()
            .filter_map(|&(p1, p2)| saved.get(&p1).map(|&s| (p2, s)))
            .collect();
        seconds.sort_by_key(|x| std::cmp::Reverse(x.0));
        for (p2, s) in seconds {
            self.split_at(b, p2, s);
        }
    }

    /// Block `b` up to the call at `p` ends in the guard; the call is in a new block `R`, the
    /// rest of the block in `J`.
    fn split_at(&mut self, b: usize, p: usize, saved: Reg) {
        let Inst::CallNative { dst, .. } = self.code[b][p] else {
            return;
        };
        let call = self.code[b][p];
        let (loc, via) = (
            self.loc[b][p],
            self.via[b].get(p).copied().unwrap_or_default(),
        );
        let has_via = !self.via[b].is_empty();
        let tail = self.code[b].split_off(p + 1);
        let tail_loc = self.loc[b].split_off(p + 1);
        let tail_via = if has_via {
            self.via[b].split_off(p + 1)
        } else {
            Vec::new()
        };
        self.code[b].pop();
        self.loc[b].pop();
        if has_via {
            self.via[b].pop();
        }
        let n = self.code.len() as u32;
        let (r, j) = (BlockId(n), BlockId(n + 1));
        let fast = match dst {
            Some(d) if d != saved => Some(BlockId(n + 2)),
            _ => None,
        };
        self.code[b].push(Inst::CondBr {
            c: saved,
            t: fast.unwrap_or(j),
            f: r,
        });
        self.loc[b].push(loc);
        if has_via {
            self.via[b].push(via);
        }
        let add = |s: &mut Self, code: Vec<Inst>, locs: Vec<StmtId>, vias: Vec<u32>| {
            s.code.push(code);
            s.loc.push(locs);
            s.via.push(vias);
        };
        let v1 = |x: u32| if has_via { vec![x; 2] } else { Vec::new() };
        add(
            self,
            vec![call, Inst::Br { to: j }],
            vec![loc, loc],
            v1(via),
        );
        add(self, tail, tail_loc, tail_via);
        if let (Some(f), Some(d)) = (fast, dst) {
            debug_assert_eq!(f.0 as usize, self.code.len());
            add(
                self,
                vec![Inst::Move { dst: d, src: saved }, Inst::Br { to: j }],
                vec![loc, loc],
                v1(via),
            );
        }
        self.replay.push(r.0 as usize);
    }
}
