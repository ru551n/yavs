//! The nine-valued region (no `--two-state`, A66): every logic value of a member is the kernel's
//! code of each element (`U X 0 1 Z W L H -` = 0 to 8) in four planes, plane `p` bit `p` of the
//! code. A value of `full` elements keeps plane `p` at word `p * words(full)` of its words (a
//! signal's from element `64 * words(full) * p`), so plane 0 is where the two-state region keeps
//! its bit, and that bit is the value of `0`, `1`, `L` and `H`. [`expand`] turns the operations
//! the lowering made on logic values into operations on planes; integers keep one word.

use crate::codegen::{mask, words};
use crate::lower::{Arith, Cmp, Gate, NineF, Op, P4F, Ref, ShiftK};

/// The words of the slots holding nine-valued values: first word, words of one plane, elements;
/// ordered by first word.
pub(crate) type Allocs = Vec<(u32, u32, u32)>;

/// What a member's operations read and write is turned into plane operations by.
struct Nine<'a> {
    /// The lowering's, then those made here for a signal an operation writes whole.
    allocs: Allocs,
    /// A kernel signal of logic elements: its elements.
    sig: &'a dyn Fn(u32) -> Option<u32>,
    locs: u32,
    out: Vec<Op>,
}

const NOT_NINE: &str = "an operation on `std_ulogic` values of this form without `--two-state`";

/// The operations `ops` with nine-valued logic: `allocs` the nine-valued slots, `sig` the logic
/// kernel signals, `locs` the slots used (more are taken for intermediate planes). An operation
/// that mixes a nine-valued value with a one-word one is an error: the process stays out.
pub(crate) fn expand(
    ops: Vec<Op>,
    allocs: &Allocs,
    sig: &dyn Fn(u32) -> Option<u32>,
    locs: &mut u32,
) -> Result<Vec<Op>, String> {
    let mut n = Nine {
        allocs: allocs.clone(),
        sig,
        locs: *locs,
        out: Vec::new(),
    };
    for op in ops {
        n.op(op)?;
    }
    *locs = n.locs;
    Ok(n.out)
}

type P4 = [Ref; 4];

/// The first plane of `p` and the words between its planes.
fn bs(p: P4) -> (Ref, u32) {
    match (p[0], p[1]) {
        (Ref::Loc(a), Ref::Loc(b)) => (p[0], b - a),
        _ => (p[0], 0),
    }
}

impl Nine<'_> {
    /// The planes of slot `r`, or `None` for a one-word slot.
    fn loc_planes(&self, r: Ref) -> Option<P4> {
        let Ref::Loc(s) = r else { return None };
        let i = self.allocs.partition_point(|a| a.0 <= s).checked_sub(1)?;
        let (start, w, _) = self.allocs[i];
        let d = s - start;
        (d < w).then(|| [0, 1, 2, 3].map(|p| Ref::Loc(start + p * w + d)))
    }

    /// The elements of the nine-valued slot `r` starts (`None`: not one).
    fn loc_len(&self, r: Ref) -> Option<u32> {
        let Ref::Loc(s) = r else { return None };
        let i = self.allocs.partition_point(|a| a.0 <= s).checked_sub(1)?;
        let (start, _, len) = self.allocs[i];
        (start == s).then_some(len)
    }

    fn is_nine(&self, r: Ref) -> bool {
        match r {
            Ref::Sig(k) => (self.sig)(k).is_some(),
            r => self.loc_planes(r).is_some(),
        }
    }

    /// A new nine-valued value of `len` elements.
    fn t4(&mut self, len: u32) -> P4 {
        let w = words(len);
        let s = self.locs;
        self.locs += 4 * w;
        [0, 1, 2, 3].map(|p| Ref::Loc(s + p * w))
    }

    /// A new one-word value of `len` elements (a boolean, a plane).
    fn t1(&mut self, len: u32) -> Ref {
        let s = self.locs;
        self.locs += words(len);
        Ref::Loc(s)
    }

    fn push(&mut self, op: Op) {
        self.out.push(op);
    }

    fn lit(&mut self, len: u32, v: u64) -> Ref {
        let t = self.t1(len);
        let n = words(len);
        let val: Vec<u64> = (0..n).map(|k| v & mask((len - 64 * k).min(64))).collect();
        self.push(Op::Lit {
            dst: t,
            val: val.into(),
        });
        t
    }

    fn ones(&mut self, len: u32) -> Ref {
        self.lit(len, !0)
    }

    fn g(&mut self, g: Gate, a: Ref, b: Ref, len: u32) -> Ref {
        let t = self.t1(len);
        self.push(Op::Gate {
            g,
            dst: t,
            a,
            b,
            len,
        });
        t
    }

    fn not(&mut self, a: Ref, len: u32) -> Ref {
        let t = self.t1(len);
        self.push(Op::Not { dst: t, a, len });
        t
    }

    fn and(&mut self, a: Ref, b: Ref, len: u32) -> Ref {
        self.g(Gate::And, a, b, len)
    }

    fn or(&mut self, a: Ref, b: Ref, len: u32) -> Ref {
        self.g(Gate::Or, a, b, len)
    }

    /// The planes of `r` as read (a signal's, all its elements, copied out first).
    fn rd(&mut self, r: Ref, _len: u32) -> Result<Option<P4>, String> {
        match r {
            Ref::Sig(k) => {
                let Some(full) = (self.sig)(k) else {
                    return Ok(None);
                };
                let len = full;
                let t = self.t4(len);
                let w = 64 * words(full);
                for (p, &d) in t.iter().enumerate() {
                    self.push(Op::Extract {
                        dst: d,
                        src: r,
                        lo: p as u32 * w,
                        len,
                        full: 4 * w,
                    });
                }
                Ok(Some(t))
            }
            r => Ok(self.loc_planes(r)),
        }
    }

    /// Where an operation writes `dst` (`len` elements): its planes, and for a signal the
    /// temporary to insert afterwards ([`Nine::flush`]).
    fn wr(&mut self, dst: Ref, len: u32) -> Result<Option<(P4, Option<Ref>)>, String> {
        match dst {
            Ref::Sig(k) => {
                if (self.sig)(k).is_none() {
                    return Ok(None);
                }
                Ok(Some((self.t4(len), Some(dst))))
            }
            r => Ok(self.loc_planes(r).map(|p| (p, None))),
        }
    }

    fn flush(&mut self, w: (P4, Option<Ref>), len: u32) {
        let (t, Some(Ref::Sig(k))) = w else {
            return;
        };
        let full = (self.sig)(k).expect("a logic signal");
        let pw = 64 * words(full);
        for (p, &s) in t.iter().enumerate() {
            self.push(Op::Insert {
                dst: Ref::Sig(k),
                src: s,
                lo: p as u32 * pw,
                len,
            });
        }
    }

    /// Plane values computed into `vals` to the planes of `dst`.
    fn put(&mut self, dst: Ref, vals: P4, len: u32) -> Result<(), String> {
        let Some(w) = self.wr(dst, len)? else {
            return Err(NOT_NINE.into());
        };
        for (&d, &v) in w.0.iter().zip(&vals) {
            self.push(Op::Copy {
                dst: d,
                src: v,
                len,
            });
        }
        self.flush(w, len);
        Ok(())
    }

    /// One fused operation (`Op::P4`) into the planes of `dst`.
    fn p4(&mut self, f: P4F, dst: Ref, x: P4, y: P4, len: u32) -> Result<(), String> {
        let dlen = if matches!(f, P4F::Eq { .. }) { 1 } else { len };
        let Some(w) = self.wr(dst, dlen)? else {
            return Err(NOT_NINE.into());
        };
        let ((d, dw), (a, aw), (b, bw)) = (bs(w.0), bs(x), bs(y));
        self.push(Op::P4 {
            f,
            dst: d,
            dw,
            a,
            aw,
            b,
            bw,
            len,
        });
        self.flush(w, dlen);
        Ok(())
    }

    /// Codes `[v0, v1, 0, 0]` from two planes.
    fn put2(&mut self, dst: Ref, p0: Ref, p1: Ref, len: u32) -> Result<(), String> {
        let z = self.lit(len, 0);
        self.put(dst, [p0, p1, z, z], len)
    }

    /// A boolean `b` (one word, 0 or 1) into `dst`: as it is into a one-word slot, as the codes of
    /// `'0'` and `'1'` into a nine-valued one.
    fn put_bool(&mut self, dst: Ref, b: Ref) -> Result<(), String> {
        if self.is_nine(dst) {
            return self.p4(P4F::Codes, dst, [b; 4], [b; 4], 1);
        }
        self.push(Op::Copy {
            dst,
            src: b,
            len: 1,
        });
        Ok(())
    }

    // Classes of elements, as `yavs_logic::planes`.
    fn is_u(&mut self, x: P4, n: u32) -> Ref {
        let a = self.or(x[0], x[1], n);
        let b = self.or(x[2], x[3], n);
        let o = self.or(a, b, n);
        self.not(o, n)
    }
    /// `0` or `L`.
    fn is0(&mut self, x: P4, n: u32) -> Ref {
        let nd = self.not(x[3], n);
        let na = self.not(x[0], n);
        let t = self.and(nd, x[1], n);
        self.and(t, na, n)
    }
    /// `1` or `H`.
    fn is1(&mut self, x: P4, n: u32) -> Ref {
        let nd = self.not(x[3], n);
        let t = self.and(nd, x[1], n);
        self.and(t, x[0], n)
    }

    /// Every element `0`, `1`, `L` or `H` (plane 1 set): a boolean.
    fn clean(&mut self, x: P4, n: u32) -> Ref {
        let t = self.t1(1);
        self.push(Op::Reduce {
            g: Gate::And,
            dst: t,
            a: x[1],
            len: n,
        });
        t
    }

    fn any(&mut self, a: Ref, n: u32) -> Ref {
        let t = self.t1(1);
        self.push(Op::Reduce {
            g: Gate::Or,
            dst: t,
            a,
            len: n,
        });
        t
    }

    /// IEEE 1164 `not`: planes 0 and 1.
    fn not9(&mut self, x: P4, n: u32) -> (Ref, Ref) {
        let u = self.is_u(x, n);
        let one = self.is1(x, n);
        let zero = self.is0(x, n);
        let nu = self.not(u, n);
        let n1 = self.not(one, n);
        (self.and(nu, n1, n), self.or(zero, one, n))
    }

    /// The planes 0 and 1 of the scalar result of IEEE 1164 `and_reduce` (and friends) of `x`.
    fn reduce9(&mut self, g: Gate, x: P4, n: u32) -> (Ref, Ref) {
        let u = self.is_u(x, n);
        let zero = self.is0(x, n);
        let one = self.is1(x, n);
        let k = self.or(zero, one, n);
        let ku = self.or(k, u, n);
        let meta = self.not(ku, n);
        let any_u = self.any(u, n);
        let any_m = self.any(meta, n);
        let nu = self.not(any_u, 1);
        let nm = self.not(any_m, 1);
        match g {
            Gate::And | Gate::Nand => {
                let any0 = self.any(zero, n);
                let n0 = self.not(any0, 1);
                let p0 = self.and(n0, nu, 1);
                let a = self.and(nu, nm, 1);
                (p0, self.or(any0, a, 1))
            }
            Gate::Or | Gate::Nor => {
                let any1 = self.any(one, n);
                let a = self.and(nu, any_m, 1);
                let b = self.and(nu, nm, 1);
                (self.or(any1, a, 1), self.or(any1, b, 1))
            }
            Gate::Xor | Gate::Xnor => {
                let par = self.t1(1);
                self.push(Op::Reduce {
                    g: Gate::Xor,
                    dst: par,
                    a: x[0],
                    len: n,
                });
                let a = self.or(any_m, par, 1);
                (self.and(nu, a, 1), self.and(nu, nm, 1))
            }
        }
    }

    /// Both planes of a scalar code inverted by IEEE 1164 `not` (for `nand`, `nor`, `xnor`).
    fn inv(&mut self, (p0, p1): (Ref, Ref), n: u32) -> (Ref, Ref) {
        let z = self.lit(n, 0);
        self.not9([p0, p1, z, z], n)
    }

    /// Operands of a nine-valued operation: both nine-valued, or the operation is not one.
    fn two_rd(&mut self, a: Ref, b: Ref, n: u32) -> Result<Option<(P4, P4)>, String> {
        match (self.rd(a, n)?, self.rd(b, n)?) {
            (Some(x), Some(y)) => Ok(Some((x, y))),
            (None, None) => Ok(None),
            _ => Err(NOT_NINE.into()),
        }
    }

    /// The operation as it is: none of what it reads or writes is nine-valued.
    fn plain(&mut self, op: Op, refs: &[Ref]) -> Result<(), String> {
        if refs.iter().any(|&r| self.is_nine(r)) {
            return Err(NOT_NINE.into());
        }
        self.push(op);
        Ok(())
    }

    fn op(&mut self, mut op: Op) -> Result<(), String> {
        // A signal written whole (the lowering retargets a temporary's operation to it): into a
        // value of its own first, then its planes inserted.
        if let Some(d) = op.dst_mut()
            && let Ref::Sig(k) = *d
            && let Some(full) = (self.sig)(k)
        {
            let w = words(full);
            let s = self.locs;
            self.locs += 4 * w;
            self.allocs.push((s, w, full));
            *d = Ref::Loc(s);
            self.op(op)?;
            let t = [0, 1, 2, 3].map(|p| Ref::Loc(s + p * w));
            self.flush((t, Some(Ref::Sig(k))), full);
            return Ok(());
        }
        match op {
            Op::If { cond, then, else_ } => {
                let mut t = Vec::new();
                std::mem::swap(&mut t, &mut self.out);
                for o in then {
                    self.op(o)?;
                }
                let then = std::mem::take(&mut self.out);
                for o in else_ {
                    self.op(o)?;
                }
                let else_ = std::mem::replace(&mut self.out, t);
                // A boolean's plane 0 is its value.
                self.push(Op::If { cond, then, else_ });
                Ok(())
            }
            Op::Copy { dst, src, len } => {
                let x = self.rd(src, len)?;
                let Some(x) = x else {
                    return self.plain(Op::Copy { dst, src, len }, &[dst]);
                };
                self.put(dst, x, len)
            }
            Op::Lit { dst, val } => {
                let Some(d) = self.loc_planes(dst) else {
                    return self.plain(Op::Lit { dst, val }, &[dst]);
                };
                let w = val.len() / 4;
                if self
                    .loc_len(dst)
                    .is_some_and(|n| val.len() == 4 * words(n) as usize)
                {
                    // Planes given (a literal with a metavalue).
                    for (p, &r) in d.iter().enumerate() {
                        self.push(Op::Lit {
                            dst: r,
                            val: val[p * w..(p + 1) * w].into(),
                        });
                    }
                    return Ok(());
                }
                // Bits: `'0'` and `'1'`.
                let Some(n) = self
                    .loc_len(dst)
                    .filter(|&n| words(n) as usize == val.len())
                else {
                    return Err(NOT_NINE.into());
                };
                let p1: Vec<u64> = (0..words(n)).map(|k| mask((n - 64 * k).min(64))).collect();
                let z = vec![0u64; val.len()];
                for (r, v) in d
                    .into_iter()
                    .zip([val, p1.into(), z.clone().into(), z.into()])
                {
                    self.push(Op::Lit { dst: r, val: v });
                }
                Ok(())
            }
            Op::Blend { dst, src, mask: m } => {
                match (dst, self.is_nine(dst), self.rd(src, 64 * m.len() as u32)?) {
                    (_, false, None) => self.push(Op::Blend { dst, src, mask: m }),
                    (Ref::Sig(k), true, Some(s)) => {
                        // Each run of the mask, inserted into each plane.
                        let full = (self.sig)(k).expect("logic");
                        let pw = 64 * words(full);
                        let mut runs = Vec::new();
                        let mut i = 0u32;
                        while i < 64 * m.len() as u32 {
                            if m[i as usize / 64] >> (i % 64) & 1 == 0 {
                                i += 1;
                                continue;
                            }
                            let lo = i;
                            while i < 64 * m.len() as u32 && m[i as usize / 64] >> (i % 64) & 1 == 1
                            {
                                i += 1;
                            }
                            runs.push((lo, i - lo));
                        }
                        for (p, &sp) in s.iter().enumerate() {
                            for &(lo, len) in &runs {
                                let t = self.t1(len);
                                self.push(Op::Extract {
                                    dst: t,
                                    src: sp,
                                    lo,
                                    len,
                                    full: 64 * m.len() as u32,
                                });
                                self.push(Op::Insert {
                                    dst,
                                    src: t,
                                    lo: p as u32 * pw + lo,
                                    len,
                                });
                            }
                        }
                    }
                    (_, true, Some(s)) => {
                        let d = self.loc_planes(dst).ok_or(NOT_NINE)?;
                        for p in 0..4 {
                            self.push(Op::Blend {
                                dst: d[p],
                                src: s[p],
                                mask: m.clone(),
                            });
                        }
                    }
                    _ => return Err(NOT_NINE.into()),
                }
                Ok(())
            }
            Op::Gate { g, dst, a, b, len } => {
                let Some((x, y)) = self.two_rd(a, b, len)? else {
                    return self.plain(Op::Gate { g, dst, a, b, len }, &[dst]);
                };
                self.p4(P4F::Gate(g), dst, x, y, len)
            }
            Op::Not { dst, a, len } => {
                let Some(x) = self.rd(a, len)? else {
                    return self.plain(Op::Not { dst, a, len }, &[dst]);
                };
                self.p4(P4F::Not, dst, x, x, len)
            }
            Op::Reduce { g, dst, a, len } => {
                let Some(x) = self.rd(a, len)? else {
                    return self.plain(Op::Reduce { g, dst, a, len }, &[dst]);
                };
                if !self.is_nine(dst) {
                    return Err(NOT_NINE.into());
                }
                let mut r = self.reduce9(g, x, len);
                if matches!(g, Gate::Nand | Gate::Nor | Gate::Xnor) {
                    r = self.inv(r, 1);
                }
                self.put2(dst, r.0, r.1, 1)
            }
            Op::Extract {
                dst,
                src,
                lo,
                len,
                full,
            } => {
                let d = self.wr(dst, len)?;
                let s: Option<[(Ref, u32, u32); 4]> = match src {
                    Ref::Sig(k) => (self.sig)(k).map(|f| {
                        let pw = 64 * words(f);
                        [0, 1, 2, 3].map(|p| (src, lo + p * pw, 4 * pw))
                    }),
                    r => self.loc_planes(r).map(|ps| ps.map(|p| (p, lo, full))),
                };
                match (d, s) {
                    (None, None) => self.push(Op::Extract {
                        dst,
                        src,
                        lo,
                        len,
                        full,
                    }),
                    (Some(d), Some(s)) => {
                        for (&dp, &(src, lo, full)) in d.0.iter().zip(&s) {
                            self.push(Op::Extract {
                                dst: dp,
                                src,
                                lo,
                                len,
                                full,
                            });
                        }
                        self.flush(d, len);
                    }
                    _ => return Err(NOT_NINE.into()),
                }
                Ok(())
            }
            Op::Insert { dst, src, lo, len } => {
                let s = self.rd(src, len)?;
                let d: Option<[(Ref, u32); 4]> = match dst {
                    Ref::Sig(k) => (self.sig)(k).map(|f| {
                        let pw = 64 * words(f);
                        [0, 1, 2, 3].map(|p| (dst, lo + p * pw))
                    }),
                    r => self.loc_planes(r).map(|ps| ps.map(|p| (p, lo))),
                };
                match (d, s) {
                    (None, None) => self.push(Op::Insert { dst, src, lo, len }),
                    (Some(d), Some(s)) => {
                        for p in 0..4 {
                            self.push(Op::Insert {
                                dst: d[p].0,
                                src: s[p],
                                lo: d[p].1,
                                len,
                            });
                        }
                    }
                    _ => return Err(NOT_NINE.into()),
                }
                Ok(())
            }
            Op::Cat {
                dst,
                hi,
                lo,
                lo_len,
                len,
            } => {
                let Some((x, y)) = (match (self.rd(hi, len - lo_len)?, self.rd(lo, lo_len)?) {
                    (Some(x), Some(y)) => Some((x, y)),
                    (None, None) => None,
                    _ => return Err(NOT_NINE.into()),
                }) else {
                    return self.plain(
                        Op::Cat {
                            dst,
                            hi,
                            lo,
                            lo_len,
                            len,
                        },
                        &[dst],
                    );
                };
                let w = self.wr(dst, len)?.ok_or(NOT_NINE)?;
                let t = w.0;
                for p in 0..4 {
                    self.push(Op::Cat {
                        dst: t[p],
                        hi: x[p],
                        lo: y[p],
                        lo_len,
                        len,
                    });
                }
                self.flush(w, len);
                Ok(())
            }
            Op::Select { dst, c, a, b, len } => {
                let Some((x, y)) = self.two_rd(a, b, len)? else {
                    return self.plain(Op::Select { dst, c, a, b, len }, &[dst]);
                };
                let w = self.wr(dst, len)?.ok_or(NOT_NINE)?;
                let t = w.0;
                for p in 0..4 {
                    self.push(Op::Select {
                        dst: t[p],
                        c,
                        a: x[p],
                        b: y[p],
                        len,
                    });
                }
                self.flush(w, len);
                Ok(())
            }
            Op::Splat { dst, a, len } => {
                let Some(x) = self.rd(a, 1)? else {
                    return self.plain(Op::Splat { dst, a, len }, &[dst]);
                };
                let w = self.wr(dst, len)?.ok_or(NOT_NINE)?;
                let t = w.0;
                for p in 0..4 {
                    self.push(Op::Splat {
                        dst: t[p],
                        a: x[p],
                        len,
                    });
                }
                self.flush(w, len);
                Ok(())
            }
            Op::Ext {
                dst,
                src,
                from,
                to,
                signed,
            } => {
                let Some(x) = self.rd(src, from)? else {
                    return self.plain(
                        Op::Ext {
                            dst,
                            src,
                            from,
                            to,
                            signed,
                        },
                        &[dst],
                    );
                };
                let w = self.wr(dst, to)?.ok_or(NOT_NINE)?;
                let t = w.0;
                for p in 0..4 {
                    self.push(Op::Ext {
                        dst: t[p],
                        src: x[p],
                        from,
                        to,
                        signed,
                    });
                }
                if !signed && to > from {
                    // The new elements are `'0'`.
                    let m = self.t1(to);
                    let val: Vec<u64> = (0..words(to))
                        .map(|k| {
                            let lo = 64 * k;
                            let all = mask((to - lo).min(64));
                            let below = if from <= lo {
                                0
                            } else {
                                mask((from - lo).min(64))
                            };
                            all & !below
                        })
                        .collect();
                    self.push(Op::Lit {
                        dst: m,
                        val: val.into(),
                    });
                    self.push(Op::Gate {
                        g: Gate::Or,
                        dst: t[1],
                        a: t[1],
                        b: m,
                        len: to,
                    });
                }
                self.flush(w, to);
                Ok(())
            }
            Op::Shift { k, dst, a, len, n } => {
                let Some(x) = self.rd(a, len)? else {
                    return self.plain(Op::Shift { k, dst, a, len, n }, &[dst]);
                };
                let w = self.wr(dst, len)?.ok_or(NOT_NINE)?;
                let t = w.0;
                for p in 0..4 {
                    self.push(Op::Shift {
                        k,
                        dst: t[p],
                        a: x[p],
                        len,
                        n,
                    });
                }
                if matches!(k, ShiftK::Sll | ShiftK::Srl) {
                    // The elements shifted in are `'0'`.
                    let ones = self.ones(len);
                    let s = self.t1(len);
                    self.push(Op::Shift {
                        k,
                        dst: s,
                        a: ones,
                        len,
                        n,
                    });
                    let fill = self.not(s, len);
                    self.push(Op::Gate {
                        g: Gate::Or,
                        dst: t[1],
                        a: t[1],
                        b: fill,
                        len,
                    });
                }
                self.flush(w, len);
                Ok(())
            }
            Op::IndexRead {
                dst,
                src,
                lo,
                n,
                stride,
                w,
                idx,
                left,
                down,
                site,
            } => {
                let d = self.wr(dst, w)?;
                let s: Option<[(Ref, u32); 4]> = match src {
                    Ref::Sig(k) => (self.sig)(k).map(|f| {
                        let pw = 64 * words(f);
                        [0, 1, 2, 3].map(|p| (src, lo + p * pw))
                    }),
                    r => self.loc_planes(r).map(|ps| ps.map(|p| (p, lo))),
                };
                let one = |dst, src, lo, site| Op::IndexRead {
                    dst,
                    src,
                    lo,
                    n,
                    stride,
                    w,
                    idx,
                    left,
                    down,
                    site,
                };
                match (d, s) {
                    (None, None) => self.push(one(dst, src, lo, site)),
                    (Some(d), Some(s)) => {
                        for (&dp, &(sp, lo)) in d.0.iter().zip(&s) {
                            self.push(one(dp, sp, lo, site));
                        }
                        self.flush(d, w);
                    }
                    _ => return Err(NOT_NINE.into()),
                }
                Ok(())
            }
            Op::IndexWrite {
                dst,
                src,
                lo,
                n,
                stride,
                w,
                idx,
                left,
                down,
                site,
            } => {
                let s = self.rd(src, w)?;
                let d: Option<[(Ref, u32); 4]> = match dst {
                    Ref::Sig(k) => (self.sig)(k).map(|f| {
                        let pw = 64 * words(f);
                        [0, 1, 2, 3].map(|p| (dst, lo + p * pw))
                    }),
                    r => self.loc_planes(r).map(|ps| ps.map(|p| (p, lo))),
                };
                let one = |dst, src, lo| Op::IndexWrite {
                    dst,
                    src,
                    lo,
                    n,
                    stride,
                    w,
                    idx,
                    left,
                    down,
                    site,
                };
                match (d, s) {
                    (None, None) => self.push(one(dst, src, lo)),
                    (Some(d), Some(s)) => {
                        for p in 0..4 {
                            self.push(one(d[p].0, s[p], d[p].1));
                        }
                    }
                    _ => return Err(NOT_NINE.into()),
                }
                Ok(())
            }
            Op::VecEq { dst, a, b, len, ne } => {
                let Some((x, y)) = self.two_rd(a, b, len)? else {
                    return self.plain_bool(Op::VecEq { dst, a, b, len, ne }, dst);
                };
                // The predefined equality: the same codes.
                if self.is_nine(dst) {
                    return self.p4(P4F::Eq { ne }, dst, x, y, len);
                }
                let (a, aw) = bs(x);
                let (b, bw) = bs(y);
                self.push(Op::P4 {
                    f: P4F::Eq { ne },
                    dst,
                    dw: 0,
                    a,
                    aw,
                    b,
                    bw,
                    len,
                });
                Ok(())
            }
            Op::VCmp {
                cmp,
                signed,
                dst,
                a,
                b,
                len,
            } => {
                let Some((x, y)) = self.two_rd(a, b, len)? else {
                    return self.plain_bool(
                        Op::VCmp {
                            cmp,
                            signed,
                            dst,
                            a,
                            b,
                            len,
                        },
                        dst,
                    );
                };
                // `numeric_std`: a metavalue gives false (`/=` true).
                let r = self.t1(1);
                self.push(Op::VCmp {
                    cmp,
                    signed,
                    dst: r,
                    a: x[0],
                    b: y[0],
                    len,
                });
                let (cx, cy) = (self.clean(x, len), self.clean(y, len));
                let c = self.and(cx, cy, 1);
                let fb = self.lit(1, u64::from(cmp == Cmp::Ne));
                let t = self.t1(1);
                self.push(Op::Select {
                    dst: t,
                    c,
                    a: r,
                    b: fb,
                    len: 1,
                });
                self.put_bool(dst, t)
            }
            Op::Add {
                dst,
                a,
                b,
                len,
                sub,
            } => self.arith(dst, a, b, len, |d, x, y| Op::Add {
                dst: d,
                a: x,
                b: y,
                len,
                sub,
            }),
            Op::Mul { dst, a, b, len } => self.arith(dst, a, b, len, |d, x, y| Op::Mul {
                dst: d,
                a: x,
                b: y,
                len,
            }),
            Op::VDiv {
                op,
                signed,
                dst,
                a,
                b,
                len,
                site,
            } => self.div(op, signed, dst, a, b, len, site),
            Op::ToInt {
                dst,
                a,
                len,
                signed,
            } => {
                let Some(x) = self.rd(a, len)? else {
                    return self.plain(
                        Op::ToInt {
                            dst,
                            a,
                            len,
                            signed,
                        },
                        &[dst],
                    );
                };
                // `to_integer` of a metavalue is 0.
                let r = self.t1(64);
                self.push(Op::ToInt {
                    dst: r,
                    a: x[0],
                    len,
                    signed,
                });
                let c = self.clean(x, len);
                let z = self.t1(64);
                self.push(Op::IntLit { dst: z, v: 0 });
                self.plain(
                    Op::Select {
                        dst,
                        c,
                        a: r,
                        b: z,
                        len: 64,
                    },
                    &[dst],
                )
            }
            Op::FromInt { dst, a, len } => {
                if !self.is_nine(dst) {
                    return self.plain(Op::FromInt { dst, a, len }, &[a]);
                }
                let r = self.t1(len);
                self.push(Op::FromInt { dst: r, a, len });
                let one = self.ones(len);
                self.put2(dst, r, one, len)
            }
            Op::IntCmp { op, dst, a, b } => {
                if self.is_nine(a) || self.is_nine(b) {
                    return Err(NOT_NINE.into());
                }
                let r = self.t1(1);
                self.push(Op::IntCmp { op, dst: r, a, b });
                self.put_bool(dst, r)
            }
            Op::Nine { f, dst, a, b, len } => self.nine(f, dst, a, b, len),
            op @ (Op::IntLit { .. } | Op::Int { .. } | Op::Range { .. }) => {
                let refs = match &op {
                    Op::IntLit { dst, .. } => vec![*dst],
                    Op::Int { dst, a, b, .. } => vec![*dst, *a, *b],
                    Op::Range { a, .. } => vec![*a],
                    _ => Vec::new(),
                };
                self.plain(op, &refs)
            }
            // `P4` is made here only, `Track` after the expansion (`lib.rs`): passed on.
            op @ (Op::P4 { .. } | Op::Track { .. }) => {
                self.push(op);
                Ok(())
            }
            Op::Note { site, args } => {
                // A logic arg's four planes are stored (`Note::text_nine` reads them).
                let args = args
                    .into_iter()
                    .map(|(r, n)| (r, if self.is_nine(r) { 4 * n } else { n }))
                    .collect();
                self.push(Op::Note { site, args });
                Ok(())
            }
        }
    }

    /// A boolean result of an operation on one-word values, into `dst`.
    fn plain_bool(&mut self, op: Op, dst: Ref) -> Result<(), String> {
        if !self.is_nine(dst) {
            self.push(op);
            return Ok(());
        }
        let r = self.t1(1);
        let mut op = op;
        if let Some(d) = op_dst(&mut op) {
            *d = r;
        }
        self.push(op);
        self.put_bool(dst, r)
    }

    /// `numeric_std` arithmetic: all `'X'` when an operand holds a metavalue.
    fn arith(
        &mut self,
        dst: Ref,
        a: Ref,
        b: Ref,
        len: u32,
        f: impl Fn(Ref, Ref, Ref) -> Op,
    ) -> Result<(), String> {
        let Some((x, y)) = self.two_rd(a, b, len)? else {
            return self.plain(f(dst, a, b), &[dst]);
        };
        let r = self.t1(len);
        self.push(f(r, x[0], y[0]));
        let (cx, cy) = (self.clean(x, len), self.clean(y, len));
        let c = self.and(cx, cy, 1);
        self.metaval(dst, c, r, len)
    }

    /// `r` (clean bits) where `c`, else all `'X'`, into `dst`.
    fn metaval(&mut self, dst: Ref, c: Ref, r: Ref, len: u32) -> Result<(), String> {
        let ones = self.ones(len);
        let zero = self.lit(len, 0);
        let (p0, p1) = (self.t1(len), self.t1(len));
        self.push(Op::Select {
            dst: p0,
            c,
            a: r,
            b: ones,
            len,
        });
        self.push(Op::Select {
            dst: p1,
            c,
            a: ones,
            b: zero,
            len,
        });
        self.put2(dst, p0, p1, len)
    }

    #[allow(clippy::too_many_arguments)]
    fn div(
        &mut self,
        op: Arith,
        signed: bool,
        dst: Ref,
        a: Ref,
        b: Ref,
        len: u32,
        site: u32,
    ) -> Result<(), String> {
        let mk = |d, x, y| Op::VDiv {
            op,
            signed,
            dst: d,
            a: x,
            b: y,
            len,
            site,
        };
        let Some((x, y)) = self.two_rd(a, b, len)? else {
            return self.plain(mk(dst, a, b), &[dst]);
        };
        let (cx, cy) = (self.clean(x, len), self.clean(y, len));
        let c = self.and(cx, cy, 1);
        // The zero divisor's check only on clean operands (`numeric_std` tests metavalues first).
        let r = self.t1(len);
        self.push(Op::If {
            cond: c,
            then: vec![mk(r, x[0], y[0])],
            else_: Vec::new(),
        });
        self.metaval(dst, c, r, len)
    }

    fn nine(&mut self, f: NineF, dst: Ref, a: Ref, b: Ref, n: u32) -> Result<(), String> {
        let x = self.rd(a, n)?.ok_or(NOT_NINE)?;
        match f {
            // `??`: `'1'` or `'H'`.
            NineF::Truthy => {
                let r = self.is1(x, 1);
                self.put_bool(dst, r)
            }
            NineF::IsX => {
                let k = self.not(x[1], n);
                let r = self.any(k, n);
                self.put_bool(dst, r)
            }
            NineF::ToX01 | NineF::ToUX01 | NineF::ToX01Z => {
                let zero = self.is0(x, n);
                let one = self.is1(x, n);
                let nz = self.not(zero, n);
                let p1 = self.or(zero, one, n);
                match f {
                    NineF::ToX01 => self.put2(dst, nz, p1, n),
                    NineF::ToUX01 => {
                        let u = self.is_u(x, n);
                        let nu = self.not(u, n);
                        let p0 = self.and(nz, nu, n);
                        self.put2(dst, p0, p1, n)
                    }
                    _ => {
                        // `'Z'`: code 4, plane 2 alone.
                        let o = self.or(x[0], x[1], n);
                        let o = self.or(o, x[3], n);
                        let no = self.not(o, n);
                        let z = self.and(no, x[2], n);
                        let nzz = self.not(z, n);
                        let p0 = self.and(nz, nzz, n);
                        let zero4 = self.lit(n, 0);
                        self.put(dst, [p0, p1, z, zero4], n)
                    }
                }
            }
            // `to_bit`, `to_bitvector`: `'1'` and `'H'` give 1, everything else the `xmap` 0.
            NineF::ToBit => {
                let one = self.is1(x, n);
                let m = self.ones(n);
                self.put2(dst, one, m, n)
            }
            // `to_01`: the value, or all `'0'` when an element is a metavalue.
            NineF::To01 => {
                let c = self.clean(x, n);
                let s = self.t1(n);
                self.push(Op::Splat {
                    dst: s,
                    a: c,
                    len: n,
                });
                let p0 = self.and(x[0], s, n);
                let m = self.ones(n);
                self.put2(dst, p0, m, n)
            }
            NineF::Match | NineF::MatchEq { .. } => {
                let y = self.rd(b, n)?.ok_or(NOT_NINE)?;
                // `'-'`: code 8, plane 3.
                let dc = self.or(x[3], y[3], n);
                let (zx, zy) = (self.is0(x, n), self.is0(y, n));
                let (ox, oy) = (self.is1(x, n), self.is1(y, n));
                let a0 = self.and(zx, zy, n);
                let a1 = self.and(ox, oy, n);
                let same = self.or(a0, a1, n);
                let one = self.or(dc, same, n);
                if f == NineF::Match {
                    let none = self.not(one, n);
                    let bad = self.any(none, n);
                    let r = self.not(bad, 1);
                    return self.put_bool(dst, r);
                }
                // `?=`: per element `'1'`, `'0'`, `'U'` or `'X'`, joined by `and`.
                let b0 = self.and(zx, oy, n);
                let b1 = self.and(ox, zy, n);
                let diff = self.or(b0, b1, n);
                let ndc = self.not(dc, n);
                let zero = self.and(ndc, diff, n);
                let (ux, uy) = (self.is_u(x, n), self.is_u(y, n));
                let u = self.or(ux, uy, n);
                let u = self.and(ndc, u, n);
                let any0 = self.any(zero, n);
                let any_u = self.any(u, n);
                let n1 = self.not(one, n);
                let any_n1 = self.any(n1, n);
                // and-reduce: `'0'` if any; else `'U'` if any; else `'X'` if any not `'1'`.
                let n0 = self.not(any0, 1);
                let nu = self.not(any_u, 1);
                let p0 = self.and(n0, nu, 1);
                let nn1 = self.not(any_n1, 1);
                let ok = self.and(nu, nn1, 1);
                let p1 = self.or(any0, ok, 1);
                let mut r = (p0, p1);
                if let NineF::MatchEq { ne: true } = f {
                    r = self.inv(r, 1);
                }
                self.put2(dst, r.0, r.1, 1)
            }
            NineF::XIf => {
                let y = self.rd(b, n)?.ok_or(NOT_NINE)?;
                let c = self.clean(y, n);
                let ones = self.ones(n);
                let zero = self.lit(n, 0);
                let xs = [ones, zero, zero, zero];
                let t = self.t4(n);
                for p in 0..4 {
                    self.push(Op::Select {
                        dst: t[p],
                        c,
                        a: x[p],
                        b: xs[p],
                        len: n,
                    });
                }
                self.put(dst, t, n)
            }
            // A `numeric_std` comparison `b` of `a` and an integer: false (`/=`: true) when `a`
            // holds a metavalue.
            NineF::NumGuard { ne } => {
                let c = self.clean(x, n);
                let r = self.t1(1);
                let rb = match self.rd(b, 1)? {
                    Some(p) => p[0],
                    None => b,
                };
                let fb = self.lit(1, u64::from(ne));
                self.push(Op::Select {
                    dst: r,
                    c,
                    a: rb,
                    b: fb,
                    len: 1,
                });
                self.put_bool(dst, r)
            }
        }
    }
}

fn op_dst(op: &mut Op) -> Option<&mut Ref> {
    match op {
        Op::VecEq { dst, .. } | Op::VCmp { dst, .. } => Some(dst),
        _ => None,
    }
}
