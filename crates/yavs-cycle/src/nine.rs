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

/// `YAVS_CYCLE_CLEAN` (A81): `off` (the default) keeps every member's nine-valued operations
/// alone; `on` gives each value a clean flag and members a plane-0 body for clean values; `check`
/// is `on` checking the flags after every run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Clean {
    Off,
    On,
    Check,
}

pub(crate) fn clean_knob() -> Result<Clean, String> {
    let v = std::env::var_os("YAVS_CYCLE_CLEAN").unwrap_or_default();
    match &*v.to_string_lossy() {
        "" | "off" => Ok(Clean::Off),
        "on" => Ok(Clean::On),
        "check" => Ok(Clean::Check),
        v => Err(format!(
            "YAVS_CYCLE_CLEAN takes `off`, `on` or `check`, not `{v}`{}",
            yavs_diag::suggest::tail(v, "value", ["off", "on", "check"].into_iter())
        )),
    }
}

/// The clean flags are kept: nine values and `YAVS_CYCLE_CLEAN` on or check.
pub(crate) fn clean() -> Clean {
    if !crate::region::exact_meta() {
        return Clean::Off;
    }
    clean_knob().unwrap_or(Clean::Off)
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

    /// `op` on the planes; with `YAVS_CYCLE_CLEAN` the dirty bits of the words of logic signals
    /// it writes made again after it (a memory's element writes its own).
    fn op(&mut self, op: Op) -> Result<(), String> {
        if clean() == Clean::Off || matches!(op, Op::If { .. }) {
            return self.op_planes(op);
        }
        let mut dirty = Vec::new();
        for r in refs(&op).1 {
            let Ref::Sig(k) = r else { continue };
            let Some(f) = (self.sig)(k) else { continue };
            let (a, b) = match op {
                Op::IndexWrite { .. } if big(self.sig, r).is_some() => continue,
                Op::Insert { lo, len, .. } => (lo, lo + len),
                Op::IndexWrite { lo, n, stride, .. } => (lo, lo + n * stride),
                _ => (0, f),
            };
            let (a, b) = (a / 64, b.div_ceil(64).min(words(f)));
            if !dirty
                .iter()
                .any(|d: &Op| matches!(d, Op::Dirty { dst: Ref::Sig(j), .. } if *j == k))
            {
                dirty.push(Op::Dirty {
                    dst: r,
                    pw: words(f),
                    lo: a,
                    n: b.max(a + 1) - a,
                    len: f,
                });
            }
        }
        self.op_planes(op)?;
        self.out.extend(dirty);
        Ok(())
    }

    fn op_planes(&mut self, mut op: Op) -> Result<(), String> {
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
                        // A81: a memory keeps the dirty bits of each element as it is written.
                        if let Some(f) = big(self.sig, dst) {
                            let x = self.not(s[1], w);
                            let x = self.or(x, s[2], w);
                            let x = self.or(x, s[3], w);
                            self.push(one(dst, x, lo + 4 * 64 * words(f)));
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
            op @ (Op::P4 { .. }
            | Op::Track { .. }
            | Op::Dirty { .. }
            | Op::Cold
            | Op::Tally { .. }
            | Op::Peek { .. }) => {
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

/// A81, plain `--cycle`: a member's operations (`ops`, a combinational member's or a clocked
/// member's edge body) behind a check that every logic word they read or write holds only `'0'`
/// and `'1'` (its dirty bits, plane 4, clear). Then each runs on plane 0 alone, where the
/// two-state value lies, unless what it reads may hold a metavalue (a literal's, or what such an
/// operation made): that one runs on the planes. Else the nine-valued operations. Signals and
/// variables an operation on planes writes take that form in every operation of the member.
/// `vars`: the variables that hold state (a clocked member's); `outer`: temporaries written
/// before `ops` (nine-valued). A memory is read on plane 0 with the element's dirty bits checked
/// where it is read: when they are not clear, the rest of the plane-0 body is skipped, the
/// variables are put back as they were and the nine-valued body runs (a member without notes).
/// `None`: no variant worth its check.
#[allow(clippy::too_many_arguments)]
pub(crate) fn expand_fast(
    check: Option<u32>,
    ops: Vec<Op>,
    allocs: &Allocs,
    sig: &dyn Fn(u32) -> Option<u32>,
    vars: &[(u32, u32)],
    outer: &BTreeSet<u32>,
    locs: &mut u32,
) -> Result<Option<Variant>, String> {
    let nine = expand(ops.clone(), allocs, sig, locs)?;
    // A very large member keeps its nine-valued body alone: Cranelift compiles a function in more
    // than linear time, and a second body would double the largest ones.
    if nine.iter().map(crate::codegen::weight_of).sum::<usize>() > VARIANT_MAX {
        return Ok(None);
    }
    let mut f = Fast {
        n: Nine {
            allocs: allocs.clone(),
            sig,
            locs: *locs,
            out: Vec::new(),
        },
        vars: vars.iter().map(|v| v.0).collect(),
        dsig: BTreeSet::new(),
        // A variable too large for dirty bits is always on the planes.
        dvar: (vars.iter())
            .filter_map(|v| allocs.iter().find(|a| a.0 == v.0))
            .filter(|a| a.1 > FLAG_MAX)
            .map(|a| a.0)
            .collect(),
        d: BTreeSet::new(),
        m: BTreeSet::new(),
        ranges: Vec::new(),
        writes: Vec::new(),
        planes: 0,
        fast: 0,
        mem: !has_note(&ops),
        ok: None,
    };
    loop {
        let before = (f.dsig.len(), f.dvar.len());
        // Each pass from the lowering's slots: a pass adds those of the signals written whole.
        f.n.locs = *locs;
        f.n.allocs = allocs.clone();
        f.n.out.clear();
        f.d = outer.clone();
        f.m = outer.clone();
        f.ranges.clear();
        f.writes.clear();
        f.planes = 0;
        f.fast = 0;
        f.ok = None;
        f.run(&ops)?;
        if (f.dsig.len(), f.dvar.len()) == before {
            break;
        }
    }
    if f.fast == 0 {
        return Ok(None);
    }
    // The dirty bits of what the plane-0 operations read or write (plane 4), each range once.
    let flags = merged(&f.ranges);
    let wt = |o: &[Op]| o.iter().map(crate::codegen::weight_of).sum::<usize>();
    let cw: usize = flags.iter().map(|r| words(r.2) as usize + 2).sum::<usize>() + 2;
    if wt(&f.n.out) + cw > wt(&nine) {
        return Ok(None);
    }
    let fast = std::mem::take(&mut f.n.out);
    let n = &mut f.n;
    let any = n.lit(1, 0);
    for &(src, lo, len, stride) in &flags {
        let t = n.t1(len);
        n.push(Op::Extract {
            dst: t,
            src,
            lo: 4 * stride + lo,
            len,
            full: 5 * stride,
        });
        let b = n.any(t, len);
        n.push(Op::Gate {
            g: Gate::Or,
            dst: any,
            a: any,
            b,
            len: 1,
        });
    }
    let cond = n.not(any, 1);
    let ok = f.ok;
    if let Some(ok) = ok {
        n.push(Op::Copy {
            dst: ok,
            src: cond,
            len: 1,
        });
    }
    let head = std::mem::take(&mut n.out);
    // The variables as they were before the plane-0 body, for the nine-valued one after a
    // memory element that holds a metavalue, or under `YAVS_CYCLE_CLEAN=check`.
    let mut fast = match ok {
        Some(ok) => guard(fast, ok),
        None => fast,
    };
    let mut restore = Vec::new();
    if check.is_some() || ok.is_some() {
        // Every word of every variable that holds state (integers too).
        let mut pre = Vec::new();
        for &(w, c) in vars {
            let t = n.t1(64 * c);
            pre.push(Op::Copy {
                dst: t,
                src: Ref::Loc(w),
                len: 64 * c,
            });
            restore.push(Op::Copy {
                dst: Ref::Loc(w),
                src: t,
                len: 64 * c,
            });
        }
        pre.extend(fast);
        fast = pre;
    }
    // `YAVS_CYCLE_CLEAN=check`: the nine-valued body after the plane-0 one, from the variables as
    // they were before it (put back), which must leave every word written as the plane-0 one did
    // (its notes left out: they were made).
    if let Some(site) = check {
        let mut chk = Vec::new();
        // What the plane-0 body wrote: a signal from where its writes go (a clocked member's
        // next-state words), a variable from its words.
        let outs: Vec<_> = (merged(&f.writes).into_iter())
            .filter(|r| big(sig, r.0).is_none())
            .collect();
        let mut saved = Vec::new();
        for &(r, lo, len, stride) in &outs {
            for p in 0..4 {
                let t = n.t1(len);
                chk.push(peek(t, r, lo + p * stride, len, 5 * stride));
                saved.push((t, r, lo + p * stride, len, stride));
            }
        }
        chk.extend(restore.iter().cloned());
        chk.extend(no_notes(&nine));
        let diff = n.t1(1);
        chk.push(Op::IntLit { dst: diff, v: 0 });
        for (t, r, lo, len, stride) in saved {
            let now = n.t1(len);
            let e = n.t1(1);
            chk.push(peek(now, r, lo, len, 5 * stride));
            chk.push(Op::VecEq {
                dst: e,
                a: t,
                b: now,
                len,
                ne: true,
            });
            chk.push(Op::Gate {
                g: Gate::Or,
                dst: diff,
                a: diff,
                b: e,
                len: 1,
            });
        }
        chk.push(Op::Range {
            a: diff,
            lo: 0,
            hi: 0,
            site,
        });
        match ok {
            Some(ok) => fast.push(Op::If {
                cond: ok,
                then: chk,
                else_: Vec::new(),
            }),
            None => fast.extend(chk),
        }
    }
    *locs = n.locs;
    Ok(Some(Variant {
        head,
        cond,
        fast,
        nine,
        ok,
        restore,
        dvar: std::mem::take(&mut f.dvar),
    }))
}

/// `ops` with what follows each write of `ok` (a plane-0 read of a memory element) run only
/// while it is 1, at every level of `If`s.
fn guard(ops: Vec<Op>, ok: Ref) -> Vec<Op> {
    let mut out = Vec::with_capacity(ops.len());
    let mut it = ops.into_iter();
    while let Some(op) = it.next() {
        let hit = writes_ref(std::slice::from_ref(&op), ok);
        out.push(match op {
            Op::If { cond, then, else_ } if hit => Op::If {
                cond,
                then: guard(then, ok),
                else_: guard(else_, ok),
            },
            o => o,
        });
        if hit {
            let rest: Vec<Op> = it.collect();
            if !rest.is_empty() {
                out.push(Op::If {
                    cond: ok,
                    then: guard(rest, ok),
                    else_: Vec::new(),
                });
            }
            break;
        }
    }
    out
}

/// Whether an operation of `ops` (nested ones too) writes `r`.
fn writes_ref(ops: &[Op], r: Ref) -> bool {
    ops.iter().any(|op| match op {
        Op::If { then, else_, .. } => writes_ref(then, r) || writes_ref(else_, r),
        op => refs(op).1.contains(&r),
    })
}

/// Whether `ops` (nested ones too) hold a note.
fn has_note(ops: &[Op]) -> bool {
    ops.iter().any(|op| match op {
        Op::If { then, else_, .. } => has_note(then) || has_note(else_),
        op => matches!(op, Op::Note { .. }),
    })
}

/// The most nine-valued operations of a member with a plane-0 body.
const VARIANT_MAX: usize = 4000;

/// The operations of a check from which a member stops checking after `MISSES` nine-valued runs
/// in a row, checking again once every `RETRY` runs (`Variant::assemble`).
const ADAPT: usize = 16;
const MISSES: u64 = 8;
const RETRY: u64 = 64;

/// The slots `Variant::assemble` uses from its `count` on.
pub(crate) const COUNT_SLOTS: u32 = 10;

/// A member's plane-0 body with its check (`expand_fast`).
pub(crate) struct Variant {
    /// Computes `cond`: every dirty bit the plane-0 body depends on is clear.
    head: Vec<Op>,
    cond: Ref,
    fast: Vec<Op>,
    nine: Vec<Op>,
    /// With plane-0 reads of memories: 0 when one of them found a metavalue (`head` sets it to
    /// `cond`); then the variables put back (`restore`) before the nine-valued body.
    ok: Option<Ref>,
    restore: Vec<Op>,
    /// The variables (first slots) the plane-0 body writes on the planes: it reads them there
    /// only, so their dirty bits are never read.
    pub(crate) dvar: BTreeSet<u32>,
}

impl Variant {
    /// The member's operations: `tail` (the dirty bits of its variables made again) ends the
    /// nine-valued body (a variable the plane-0 body writes on the planes it reads there only).
    /// The nine-valued body counts its runs in slot `count`, those in a row in `count + 2` (the
    /// others to `count + COUNT_SLOTS` scratch words).
    pub(crate) fn assemble(self, tail: &[Op], count: u32) -> Vec<Op> {
        let mut fast = Vec::new();
        let mut nine = vec![
            Op::Cold,
            Op::Tally {
                at: Ref::Loc(count),
                reset: false,
            },
        ];
        fast.extend(self.fast);
        nine.extend(self.nine);
        nine.extend(tail.iter().cloned());
        // A member whose check is long and whose last `MISSES` runs were nine-valued skips the
        // check (but once every `RETRY` runs): its values hold metavalues for a while.
        let miss = Ref::Loc(count + 2);
        let l = |s: u32, v: u64| Op::Lit {
            dst: Ref::Loc(count + s),
            val: Box::new([v]),
        };
        let mut out = Vec::new();
        if self
            .head
            .iter()
            .map(crate::codegen::weight_of)
            .sum::<usize>()
            >= ADAPT
        {
            fast.insert(
                0,
                Op::Tally {
                    at: miss,
                    reset: true,
                },
            );
            nine.push(Op::Tally {
                at: miss,
                reset: false,
            });
            let r = |s: u32| Ref::Loc(count + s);
            out.extend([
                l(3, MISSES),
                Op::VCmp {
                    cmp: Cmp::Lt,
                    signed: false,
                    dst: r(4),
                    a: miss,
                    b: r(3),
                    len: 64,
                },
                l(5, RETRY - 1),
                Op::Gate {
                    g: Gate::And,
                    dst: r(6),
                    a: miss,
                    b: r(5),
                    len: 64,
                },
                l(7, 0),
                Op::VecEq {
                    dst: r(8),
                    a: r(6),
                    b: r(7),
                    len: 64,
                    ne: false,
                },
                Op::Gate {
                    g: Gate::Or,
                    dst: r(9),
                    a: r(4),
                    b: r(8),
                    len: 1,
                },
            ]);
            let mut no = vec![Op::Lit {
                dst: self.cond,
                val: Box::new([0]),
            }];
            if let Some(ok) = self.ok {
                no.push(Op::Lit {
                    dst: ok,
                    val: Box::new([0]),
                });
            }
            out.push(Op::If {
                cond: r(9),
                then: self.head,
                else_: no,
            });
        } else {
            out = self.head;
        }
        match self.ok {
            None => out.push(Op::If {
                cond: self.cond,
                then: fast,
                else_: nine,
            }),
            Some(ok) => {
                out.push(Op::If {
                    cond: self.cond,
                    then: fast,
                    else_: Vec::new(),
                });
                nine.insert(
                    2,
                    Op::If {
                        cond: self.cond,
                        then: self.restore,
                        else_: Vec::new(),
                    },
                );
                out.push(Op::If {
                    cond: ok,
                    then: Vec::new(),
                    else_: nine,
                });
            }
        }
        out
    }
}

/// `Extract` of what a member wrote: a signal from where its writes go (`Op::Peek`).
fn peek(dst: Ref, src: Ref, lo: u32, len: u32, full: u32) -> Op {
    match src {
        Ref::Sig(_) => Op::Peek {
            dst,
            src,
            lo,
            len,
            full,
        },
        _ => Op::Extract {
            dst,
            src,
            lo,
            len,
            full,
        },
    }
}

/// `ranges` ordered, those of one value that overlap or touch joined.
fn merged(ranges: &[(Ref, u32, u32, u32)]) -> Vec<(Ref, u32, u32, u32)> {
    let mut r = ranges.to_vec();
    r.sort_unstable_by_key(|r| (matches!(r.0, Ref::Loc(_)), ref_id(r.0), r.1));
    let mut out: Vec<(Ref, u32, u32, u32)> = Vec::new();
    for r in r {
        match out.last_mut() {
            Some(m) if m.0 == r.0 && r.1 <= m.1 + m.2 => m.2 = m.2.max(r.1 + r.2 - m.1),
            _ => out.push(r),
        }
    }
    out
}

fn ref_id(r: Ref) -> u32 {
    match r {
        Ref::Sig(k) | Ref::Loc(k) => k,
        Ref::Edge => 0,
    }
}

/// [`expand_fast`]'s pass over a member's operations.
struct Fast<'a> {
    n: Nine<'a>,
    /// The first slots of its variables.
    vars: Vec<u32>,
    /// Signals and variables (first slots) an operation on planes writes.
    dsig: BTreeSet<u32>,
    dvar: BTreeSet<u32>,
    /// Temporaries (first slots) that may hold a metavalue; those whose planes hold the value.
    d: BTreeSet<u32>,
    m: BTreeSet<u32>,
    /// The ranges of logic words the plane-0 operations read or write (where, first element,
    /// elements, elements between planes).
    ranges: Vec<(Ref, u32, u32, u32)>,
    /// Of those, the ones they write.
    writes: Vec<(Ref, u32, u32, u32)>,
    planes: usize,
    fast: usize,
    /// Memories may be read on plane 0 (the member has no notes, which a rerun would repeat).
    mem: bool,
    /// The word a plane-0 read of a memory element that holds a metavalue clears: the rest of
    /// the plane-0 body is skipped and the nine-valued one runs.
    ok: Option<Ref>,
}

use std::collections::BTreeSet;

impl Fast<'_> {
    /// The nine-valued slot `r` lies in: its first slot, words of a plane, elements.
    fn alloc(&self, r: Ref) -> Option<(u32, u32, u32)> {
        let Ref::Loc(s) = r else { return None };
        let a = &self.n.allocs;
        let i = a.partition_point(|a| a.0 <= s).checked_sub(1)?;
        let (start, w, len) = a[i];
        (s - start < w).then_some((start, w, len))
    }

    fn run(&mut self, ops: &[Op]) -> Result<(), String> {
        for op in ops {
            self.op(op)?;
        }
        Ok(())
    }

    fn op(&mut self, op: &Op) -> Result<(), String> {
        if let Op::If { cond, then, else_ } = op {
            let saved = std::mem::take(&mut self.n.out);
            let (d0, m0) = (self.d.clone(), self.m.clone());
            self.run(then)?;
            let t = std::mem::take(&mut self.n.out);
            let (d1, m1) = (
                std::mem::replace(&mut self.d, d0),
                std::mem::replace(&mut self.m, m0),
            );
            self.run(else_)?;
            let mut e = std::mem::replace(&mut self.n.out, saved);
            let (d2, m2) = (std::mem::take(&mut self.d), std::mem::take(&mut self.m));
            // A temporary that may hold a metavalue after the `If` has its planes in both
            // branches: made at the end of one where it holds none.
            let mut t = t;
            let du: BTreeSet<u32> = d1.union(&d2).copied().collect();
            let (mut m1, mut m2) = (m1, m2);
            for (out, m) in [(&mut t, &mut m1), (&mut e, &mut m2)] {
                for &s in &du {
                    if m.insert(s) {
                        out.extend(self.make(s));
                    }
                }
            }
            self.d = du;
            self.m = m1.intersection(&m2).copied().collect();
            // The condition is a one-word value or a boolean's plane 0 in both forms.
            self.n.push(Op::If {
                cond: *cond,
                then: t,
                else_: e,
            });
            return Ok(());
        }
        let (reads, writes) = refs(op);
        let sig = self.n.sig;
        // A signal it reads holds what its dirty bits checked: what the member writes reaches no
        // read of its own (a clocked member's writes go to the next-state words, a member of a
        // loop group reads a copy).
        let dirty_src = reads.iter().any(|&r| match r {
            Ref::Sig(_) => false,
            r => self
                .alloc(r)
                .is_some_and(|a| self.d.contains(&a.0) || self.dvar.contains(&a.0)),
        });
        let dirty_dst = writes.iter().any(|&r| match r {
            Ref::Sig(k) => self.dsig.contains(&k),
            r => self.alloc(r).is_some_and(|a| self.dvar.contains(&a.0)),
        });
        let partial = matches!(
            op,
            Op::Insert { .. } | Op::Blend { .. } | Op::IndexWrite { .. }
        );
        let into_dirty = partial
            && writes
                .iter()
                .any(|&r| self.alloc(r).is_some_and(|a| self.d.contains(&a.0)));
        // A literal given as planes: its plane 0 when it holds `'0'` and `'1'` only.
        let lit = match op {
            Op::Lit {
                dst: Ref::Sig(k),
                val,
            } => sig(*k)
                .filter(|&f| val.len() == 4 * words(f) as usize)
                .map(|f| (f, val)),
            Op::Lit { dst, val } => self
                .alloc(*dst)
                .filter(|a| val.len() == 4 * a.1 as usize)
                .map(|a| (a.2, val)),
            _ => None,
        };
        let lit0 = lit.and_then(|(len, val)| {
            let w = words(len) as usize;
            (0..w)
                .all(|k| {
                    let m = mask((len - 64 * k as u32).min(64));
                    val[w + k] & m == m && (val[2 * w + k] | val[3 * w + k]) & m == 0
                })
                .then(|| val[..w].to_vec())
        });
        let makes = match op {
            Op::Lit { .. } => lit.is_some() && lit0.is_none(),
            Op::Nine { f, .. } => matches!(
                f,
                NineF::Match | NineF::MatchEq { .. } | NineF::XIf | NineF::NumGuard { .. }
            ),
            Op::Note { .. }
            | Op::IndexWrite {
                dst: Ref::Sig(_), ..
            } => true,
            // A read of a memory, whose flag says nothing (A81).
            Op::IndexRead { src, .. } => big(sig, *src).is_some(),
            _ => false,
        };
        if let Op::IndexRead {
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
        } = *op
            && let Some(f) = big(sig, src)
            && self.mem
            && !dirty_src
            && !dirty_dst
            && self.alloc(dst).is_some_and(|a| !self.vars.contains(&a.0))
        {
            // The element on plane 0, its dirty bits checked (the memory is in no range).
            self.fast += 1;
            self.n.push(op.clone());
            let dd = self.n.t1(w);
            self.n.push(Op::IndexRead {
                dst: dd,
                src,
                lo: lo + 4 * 64 * words(f),
                n,
                stride,
                w,
                idx,
                left,
                down,
                site,
            });
            let b = self.n.any(dd, w);
            let nb = self.n.not(b, 1);
            let ok = *self.ok.get_or_insert_with(|| self.n.t1(1));
            self.n.push(Op::Gate {
                g: Gate::And,
                dst: ok,
                a: ok,
                b: nb,
                len: 1,
            });
            if let Some((s, _, _)) = self.alloc(dst) {
                self.d.remove(&s);
                self.m.remove(&s);
            }
            return Ok(());
        }
        if makes || dirty_src || dirty_dst || into_dirty {
            return self.on_planes(op, &reads, &writes);
        }
        self.fast += 1;
        let mut op = op.clone();
        match &mut op {
            Op::Lit { val, .. } if lit0.is_some() => *val = lit0.unwrap_or_default().into(),
            Op::Extract {
                src: Ref::Sig(k),
                full,
                ..
            } => {
                if let Some(f) = sig(*k) {
                    *full = 64 * words(f);
                }
            }
            Op::Nine { f, dst, a, len, .. } => {
                // On `'0'` and `'1'`: `is_x` is false, the others give the value.
                op = match f {
                    NineF::IsX => Op::Lit {
                        dst: *dst,
                        val: vec![0u64].into(),
                    },
                    NineF::Truthy => Op::Copy {
                        dst: *dst,
                        src: *a,
                        len: 1,
                    },
                    _ => Op::Copy {
                        dst: *dst,
                        src: *a,
                        len: *len,
                    },
                };
            }
            _ => {}
        }
        // The words of logic signals it reads or writes (all a signal's but where an operation
        // says which), and its variables, for the check.
        let part = match &op {
            Op::Extract {
                src: Ref::Sig(k),
                lo,
                len,
                ..
            }
            | Op::Insert {
                dst: Ref::Sig(k),
                lo,
                len,
                ..
            } => Some((*k, *lo, *len)),
            Op::IndexRead {
                src: Ref::Sig(k),
                lo,
                n,
                stride,
                ..
            } => Some((*k, *lo, *n * *stride)),
            _ => None,
        };
        for (i, &r) in reads.iter().chain(&writes).enumerate() {
            let range = match r {
                Ref::Sig(k) => sig(k).map(|f| {
                    let (lo, len) = match part {
                        Some((pk, lo, len)) if pk == k => (lo, len),
                        _ => (0, f),
                    };
                    (r, lo, len, 64 * words(f))
                }),
                r => (self.alloc(r))
                    .filter(|a| self.vars.contains(&a.0))
                    .map(|(s, w, len)| (Ref::Loc(s), 0, len, 64 * w)),
            };
            if let Some(range) = range {
                self.ranges.push(range);
                if i >= reads.len() {
                    self.writes.push(range);
                }
            }
        }
        // A temporary it writes holds no metavalue, and its planes no longer its value.
        for &r in &writes {
            if let Some((s, _, _)) = self.alloc(r)
                && !self.vars.contains(&s)
            {
                if !partial {
                    self.d.remove(&s);
                }
                self.m.remove(&s);
            }
        }
        self.n.push(op);
        Ok(())
    }

    /// The planes 1 to 3 of the temporary at `s` holding no metavalue: `'0'` and `'1'`.
    fn make(&self, s: u32) -> Vec<Op> {
        let Some((_, w, len)) = self.alloc(Ref::Loc(s)) else {
            return Vec::new();
        };
        (1..4u32)
            .map(|p| Op::Lit {
                dst: Ref::Loc(s + p * w),
                val: (0..w)
                    .map(|k| {
                        if p == 1 {
                            mask((len - 64 * k).min(64))
                        } else {
                            0
                        }
                    })
                    .collect(),
            })
            .collect()
    }

    /// `op` on the planes: the planes of the temporaries it reads made first.
    fn on_planes(&mut self, op: &Op, reads: &[Ref], writes: &[Ref]) -> Result<(), String> {
        self.planes += 1;
        let partial = matches!(
            op,
            Op::Insert { .. } | Op::Blend { .. } | Op::IndexWrite { .. }
        );
        let mut need: Vec<Ref> = reads.to_vec();
        if partial {
            need.extend(writes);
        }
        for r in need {
            if let Some((s, _, _)) = self.alloc(r)
                && !self.vars.contains(&s)
                && !self.d.contains(&s)
                && self.m.insert(s)
            {
                for o in self.make(s) {
                    self.n.push(o);
                }
            }
        }
        self.n.op(op.clone())?;
        let clean = matches!(
            op,
            Op::VecEq { .. }
                | Op::VCmp { .. }
                | Op::Nine {
                    f: NineF::Match | NineF::NumGuard { .. },
                    ..
                }
        );
        for &r in writes {
            match r {
                Ref::Sig(k) => {
                    if let Some(f) = (self.n.sig)(k) {
                        self.dsig.insert(k);
                        if big(self.n.sig, r).is_none() {
                            self.writes.push((r, 0, f, 64 * words(f)));
                        }
                    }
                }
                r => {
                    if let Some((s, w, len)) = self.alloc(r) {
                        if self.vars.contains(&s) {
                            self.dvar.insert(s);
                            self.writes.push((Ref::Loc(s), 0, len, 64 * w));
                        } else {
                            if !clean {
                                self.d.insert(s);
                            }
                            if !partial {
                                self.m.insert(s);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// What `op` reads and what it writes (an operation that writes part of a value reads it too).
fn refs(op: &Op) -> (Vec<Ref>, Vec<Ref>) {
    let (r, w) = (
        std::cell::RefCell::new(Vec::new()),
        std::cell::RefCell::new(Vec::new()),
    );
    op.map(
        &|x| {
            r.borrow_mut().push(x);
            x
        },
        &|x| {
            w.borrow_mut().push(x);
            x
        },
        &|x| x,
    );
    let (mut r, w) = (r.into_inner(), w.into_inner());
    if matches!(
        op,
        Op::Insert { .. } | Op::Blend { .. } | Op::IndexWrite { .. }
    ) {
        r.extend(&w);
    }
    (r, w)
}

/// A81: the elements of `r` when it is a memory (a logic signal of more than 4096 elements,
/// `region::big_signal`) whose dirty bits are kept: each written element's with it.
fn big(sig: &dyn Fn(u32) -> Option<u32>, r: Ref) -> Option<u32> {
    let Ref::Sig(k) = r else { return None };
    sig(k).filter(|&f| f > 4096 && clean() != Clean::Off)
}

/// A81: the most words of a plane a variable's clean flag is kept for; a larger one's stays 1.
pub(crate) const FLAG_MAX: u32 = 64;

/// `ops` without their notes.
fn no_notes(ops: &[Op]) -> Vec<Op> {
    (ops.iter())
        .filter(|o| !matches!(o, Op::Note { .. }))
        .map(|o| match o {
            Op::If { cond, then, else_ } => Op::If {
                cond: *cond,
                then: no_notes(then),
                else_: no_notes(else_),
            },
            o => o.clone(),
        })
        .collect()
}
