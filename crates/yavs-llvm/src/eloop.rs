//! The runtime of element loops (#757): the iterations of a loop found by
//! `yavs_interp::elem_loops`, planned and done at once for compiled code on native frames.

use std::mem::MaybeUninit;
use yavs_interp::eloop::{
    ADD, ADDR, GET, IND, KONST, LD, LEN, MAX_READS, MAX_STEPS, MOV, MUL, POS, SET, SUB,
};

/// Memory for [`run`]: the object an address points into.
pub trait LoopMem {
    /// The start and length in bytes of the object holding address `a` (writable when `write`);
    /// `None` where it is not reachable. The bytes stay valid until `run` returns.
    fn region(&mut self, a: u64, write: bool) -> Option<(*mut u8, u64)>;
}

/// The elements `e0 + step * n` of a logic vector at byte `off` of the object at `ptr`.
#[derive(Clone, Copy)]
struct Stream {
    ptr: *mut u8,
    off: usize,
    e0: i64,
    step: i64,
}

/// Elements stay below 2^32, as an object does.
const ELEMS: i64 = 1 << 32;

/// The position of the affine index `x` in the dimension `(left, right, dir)`, affine too, when
/// the index stays in range up to iteration `last`.
#[inline(always)]
fn pos_of(d: (i64, i64, u64), x: (i64, i64), last: i64) -> Option<(i64, i64)> {
    let (l, r, dir) = d;
    let e1 = x.0.checked_add(x.1.checked_mul(last)?)?;
    let (lo, hi, c0, c1) = match dir {
        0 => (l, r, x.0.checked_sub(l)?, x.1),
        1 => (r, l, l.checked_sub(x.0)?, x.1.checked_neg()?),
        _ => return None,
    };
    (lo <= x.0 && x.0 <= hi && lo <= e1 && e1 <= hi).then_some((c0, c1))
}

/// The iterations of the loop (`prog`, `inputs` as `yavs_interp::ElemLoop` has them) but the
/// last, from loop variable `ind` to `bound` counting by `step` (1 or -1), at most `limit`:
/// returns how many were done, 0 where nothing was (anything not as the generic code would find
/// it: it then does every iteration and fails where and as it would). `prog` is what
/// `elem_loops` made, `inputs` has `MAX_INPUTS` words.
pub fn run(
    prog: &[i64],
    inputs: &[u64],
    ind: i64,
    bound: i64,
    step: i64,
    limit: i64,
    mem: &mut dyn LoopMem,
) -> i64 {
    if step != 1 && step != -1 {
        return 0;
    }
    let Some(dist) = bound.checked_sub(ind).and_then(|d| d.checked_mul(step)) else {
        return 0;
    };
    // `dist` iterations go round again, and the one at the bound is left to the generic code.
    let m = dist.min(limit);
    if m < 2 || prog.len() > MAX_STEPS * 4 || inputs.len() < yavs_interp::eloop::MAX_INPUTS {
        return 0;
    }
    match plan(prog, inputs, ind, step, m, mem) {
        Some((src, dst, code)) => exec(src, dst, code, m),
        None => 0,
    }
}

/// Which steps of `prog` have the same value in every iteration, when compiled code can plan the
/// loop itself (`Gen::eloop_plan`): every base and code is such a step, a product has one
/// factor that is, and the single `SET` ends the program. `None` leaves the loop to [`run`].
/// `ninputs` is how many input registers there are.
#[cfg(any(feature = "llvm", test))]
pub fn invariant_steps(prog: &[i64], ninputs: usize) -> Option<Vec<bool>> {
    if !prog.len().is_multiple_of(4) || prog.len() > MAX_STEPS * 4 {
        return None;
    }
    let mut inv: Vec<bool> = Vec::new();
    let mut gets = 0;
    let steps = prog.len() / 4;
    for (j, st) in prog.as_chunks::<4>().0.iter().enumerate() {
        let of = |v: i64, inv: &[bool]| -> Option<bool> {
            if v >= 0 {
                inv.get(v as usize).copied().filter(|_| (v as usize) < j)
            } else if v == IND {
                Some(false)
            } else {
                (((-2 - v) as usize) < ninputs).then_some(true)
            }
        };
        let (a, b, c) = (st[1], st[2], st[3]);
        let r = match st[0] {
            MOV => of(a, &inv)?,
            KONST => true,
            ADD | SUB => of(a, &inv)? & of(b, &inv)?,
            MUL => {
                let (x, y) = (of(a, &inv)?, of(b, &inv)?);
                if !x && !y {
                    return None;
                }
                x & y
            }
            POS => {
                if !of(a, &inv)? || !(0..1 << 20).contains(&b) {
                    return None;
                }
                of(c, &inv)?
            }
            LEN => {
                if !of(a, &inv)? || !(0..1 << 20).contains(&b) {
                    return None;
                }
                true
            }
            LD => {
                if !of(a, &inv)? || !(0..1 << 31).contains(&b) || !matches!(c, 1 | 4 | 8) {
                    return None;
                }
                true
            }
            ADDR => {
                if !of(a, &inv)? || !(0..1 << 32).contains(&b) {
                    return None;
                }
                true
            }
            GET => {
                if !of(a, &inv)? {
                    return None;
                }
                of(b, &inv)?;
                gets += 1;
                true
            }
            SET => {
                let ok = of(a, &inv)? && of(b, &inv).is_some() && j + 1 == steps;
                let ok = ok
                    && if c >= 0 {
                        gets == 1
                    } else {
                        of(c, &inv)? && gets == 0
                    };
                if !ok {
                    return None;
                }
                true
            }
            _ => return None,
        };
        inv.push(r);
    }
    matches!(prog.get(prog.len().wrapping_sub(4)), Some(&SET)).then_some(inv)
}

/// Evaluate the steps for iterations `0..m` (each value affine in the iteration number) and check
/// every position, descriptor and element range at both ends: the source stream (none for a fill),
/// the destination and the fill code.
#[inline(always)]
fn plan(
    prog: &[i64],
    inputs: &[u64],
    i0: i64,
    delta: i64,
    m: i64,
    mem: &mut dyn LoopMem,
) -> Option<(Option<Stream>, Stream, u8)> {
    let last = m - 1;
    // (value at iteration 0, change per iteration) of each step; written before it is read.
    let mut val = [MaybeUninit::<(i64, i64)>::uninit(); MAX_STEPS];
    // The memory read besides the vectors, as host address ranges.
    let mut reads = [MaybeUninit::<(usize, usize)>::uninit(); MAX_READS];
    let mut nreads = 0usize;
    let mut gets: Option<(u64, (i64, i64))> = None;
    let mut out = None;
    for (j, st) in prog.as_chunks::<4>().0.iter().enumerate() {
        let get = |v: i64| -> (i64, i64) {
            if v >= 0 {
                debug_assert!((v as usize) < j);
                // SAFETY: a step refers to the steps before it (`elem_loops`).
                unsafe { val.get_unchecked(v as usize).assume_init() }
            } else if v == IND {
                (i0, delta)
            } else {
                (
                    inputs.get((-2 - v) as usize).copied().unwrap_or(0) as i64,
                    0,
                )
            }
        };
        let (a, b, c) = (st[1], st[2], st[3]);
        let r = match st[0] {
            MOV => get(a),
            KONST => (a, 0),
            ADD | SUB => {
                let (x, y) = (get(a), get(b));
                let (c0, c1) = if st[0] == ADD {
                    (x.0.checked_add(y.0)?, x.1.checked_add(y.1)?)
                } else {
                    (x.0.checked_sub(y.0)?, x.1.checked_sub(y.1)?)
                };
                // The value at the last iteration fits too.
                c0.checked_add(c1.checked_mul(last)?)?;
                (c0, c1)
            }
            MUL => {
                // One of them does not change.
                let (x, y) = (get(a), get(b));
                let (k, v) = if x.1 == 0 {
                    (x.0, y)
                } else if y.1 == 0 {
                    (y.0, x)
                } else {
                    return None;
                };
                let (c0, c1) = (k.checked_mul(v.0)?, k.checked_mul(v.1)?);
                c0.checked_add(c1.checked_mul(last)?)?;
                (c0, c1)
            }
            POS | LEN => {
                let d = get(a).0 as u64;
                let (p, len) = mem.region(d, false)?;
                let at = (d & 0xffff_ffff).checked_add((b as u64).checked_mul(24)?)? as usize;
                if at.checked_add(24)? > len as usize || nreads >= MAX_READS {
                    return None;
                }
                reads[nreads].write((p as usize + at, p as usize + at + 24));
                nreads += 1;
                // SAFETY: the 24 bytes at `at` lie in the object `region` returned.
                let w =
                    |k: usize| unsafe { std::ptr::read_unaligned(p.add(at + k * 8).cast::<i64>()) };
                let dim = (w(0), w(1), w(2) as u64);
                if st[0] == POS {
                    pos_of(dim, get(c), last)?
                } else {
                    let n = yavs_ir::dim_len(dim.0, dim.1, dim.2 == 1)
                        .filter(|&n| n <= i64::MAX as u64)?;
                    (n as i64, 0)
                }
            }
            LD => {
                let base = get(a).0 as u64;
                let w = c as usize;
                let (p, len) = mem.region(base, false)?;
                let at = (base & 0xffff_ffff).checked_add(b as u64)? as usize;
                if at.checked_add(w)? > len as usize || nreads >= MAX_READS {
                    return None;
                }
                reads[nreads].write((p as usize + at, p as usize + at + w));
                nreads += 1;
                let mut buf = [0u8; 8];
                // SAFETY: the `w` bytes at `at` lie in the object `region` returned.
                unsafe { std::ptr::copy_nonoverlapping(p.add(at), buf.as_mut_ptr(), w.min(8)) };
                (u64::from_le_bytes(buf) as i64, 0)
            }
            ADDR => {
                let base = get(a).0 as u64;
                // Within the object's 4 GiB, as the interpreter's address arithmetic.
                let lo = (base & 0xffff_ffff).checked_add(b as u64)?;
                if lo > 0xffff_ffff {
                    return None;
                }
                ((base & !0xffff_ffff | lo) as i64, 0)
            }
            GET => {
                let (base, x) = (get(a), get(b));
                if base.1 != 0 {
                    return None;
                }
                gets = Some((base.0 as u64, x));
                (0, 0)
            }
            SET => {
                let (base, x) = (get(a), get(b));
                if base.1 != 0 {
                    return None;
                }
                let dst = stream(mem, base.0 as u64, x, last, true)?;
                // Nothing it read is among the elements it writes.
                let e1 = dst.e0 + dst.step * last;
                let lo = dst.ptr as usize + dst.off + (dst.e0.min(e1) as usize) / 64 * 32;
                let hi = dst.ptr as usize + dst.off + (dst.e0.max(e1) as usize) / 64 * 32 + 32;
                // SAFETY: the first `nreads` entries were written.
                if reads[..nreads].iter().any(|r| {
                    let (s, z) = unsafe { r.assume_init() };
                    s < hi && lo < z
                }) {
                    return None;
                }
                out = Some(if c >= 0 {
                    let (sb, sx) = gets?;
                    (Some(stream(mem, sb, sx, last, false)?), dst, 0)
                } else {
                    let code = get(c).0;
                    if !(0..=8).contains(&code) {
                        return None;
                    }
                    (None, dst, code as u8)
                });
                (0, 0)
            }
            _ => return None,
        };
        val[j].write(r);
    }
    out
}

/// The elements `e + step * n` for `n` up to `last` of the logic vector at `base`, all inside its
/// object.
#[inline(always)]
fn stream(
    mem: &mut dyn LoopMem,
    base: u64,
    e: (i64, i64),
    last: i64,
    write: bool,
) -> Option<Stream> {
    let e1 = e.0.checked_add(e.1.checked_mul(last)?)?;
    if !(0..ELEMS).contains(&e.0) || !(0..ELEMS).contains(&e1) {
        return None;
    }
    let (ptr, len) = mem.region(base, write)?;
    let off = (base & 0xffff_ffff) as usize;
    let end = off + e.0.max(e1) as usize / 64 * 32 + 32;
    (end <= len as usize).then_some(Stream {
        ptr,
        off,
        e0: e.0,
        step: e.1,
    })
}

/// The four plane words of the block at `at`.
///
/// # Safety
/// The 32 bytes at `at` are in the object.
#[inline(always)]
unsafe fn blk(m: *const u8, at: usize) -> [u64; 4] {
    // SAFETY: as stated.
    unsafe { std::ptr::read_unaligned(m.add(at).cast::<[u64; 4]>()) }
}

/// # Safety
/// The 32 bytes at `at` are in the object.
#[inline(always)]
unsafe fn put_blk(m: *mut u8, at: usize, b: [u64; 4]) {
    // SAFETY: as stated.
    unsafe { std::ptr::write_unaligned(m.add(at).cast::<[u64; 4]>(), b) }
}

#[inline(always)]
fn nmask(n: usize) -> u64 {
    if n >= 64 { !0 } else { (1u64 << n) - 1 }
}

/// Elements `e .. e + k` (`k <= 64`) of the vector at byte `off` as one block, element `e` in bit 0.
///
/// # Safety
/// The blocks of the elements are in the object (`stream`).
#[inline(always)]
pub(crate) unsafe fn get_run(m: *const u8, off: usize, e: usize, k: usize) -> [u64; 4] {
    let (b, s) = (e / 64, e % 64);
    let at = off + b * 32;
    let mask = nmask(k);
    // SAFETY: as stated.
    unsafe {
        let a = blk(m, at);
        if s + k <= 64 {
            return a.map(|p| p >> s & mask);
        }
        let c = blk(m, at + 32);
        std::array::from_fn(|p| (a[p] >> s | c[p] << (64 - s)) & mask)
    }
}

/// Set elements `e .. e + k` (`k <= 64`) of the vector at byte `off` to the block `v`.
///
/// # Safety
/// As `get_run`.
#[inline(always)]
pub(crate) unsafe fn put_run(m: *mut u8, off: usize, e: usize, k: usize, v: [u64; 4]) {
    let (b, s) = (e / 64, e % 64);
    let at = off + b * 32;
    let mask = nmask(k);
    // SAFETY: as stated.
    unsafe {
        let mut a = blk(m, at);
        for (w, x) in a.iter_mut().zip(v) {
            *w = *w & !(mask << s) | (x & mask) << s;
        }
        put_blk(m, at, a);
        if s + k > 64 {
            let mut c = blk(m, at + 32);
            for (w, x) in c.iter_mut().zip(v) {
                *w = *w & !(mask >> (64 - s)) | (x & mask) >> (64 - s);
            }
            put_blk(m, at + 32, c);
        }
    }
}

/// # Safety
/// As `get_run`.
#[inline(always)]
unsafe fn lget(m: *const u8, off: usize, e: usize) -> u8 {
    // SAFETY: as stated.
    let b = unsafe { blk(m, off + e / 64 * 32) };
    (0..4).fold(0u8, |c, p| c | ((b[p] >> (e % 64) & 1) as u8) << p)
}

/// # Safety
/// As `get_run`.
#[inline(always)]
unsafe fn lset(m: *mut u8, off: usize, e: usize, code: u8) {
    let at = off + e / 64 * 32;
    // SAFETY: as stated.
    unsafe {
        let mut b = blk(m, at);
        for (p, w) in b.iter_mut().enumerate() {
            *w = *w & !(1 << (e % 64)) | u64::from(code >> p & 1) << (e % 64);
        }
        put_blk(m, at, b);
    }
}

/// Run the `m` iterations of a plan; returns how many were done.
#[inline(always)]
fn exec(src: Option<Stream>, d: Stream, code: u8, m: i64) -> i64 {
    // SAFETY: `stream` checked that every element of both streams lies in its object, and the
    // objects stay as they are until this returns.
    unsafe {
        let Some(s) = src else {
            // A fill.
            let at = |n: i64| d.e0 + d.step * n;
            if d.step.abs() <= 1 {
                // One run of elements.
                let lo = at(0).min(at(m - 1)) as usize;
                let n = if d.step == 0 { 1 } else { m as usize };
                let v: [u64; 4] = std::array::from_fn(|p| if code >> p & 1 != 0 { !0 } else { 0 });
                let mut done = 0;
                while done < n {
                    let k = (n - done).min(64);
                    put_run(d.ptr, d.off, lo + done, k, v);
                    done += k;
                }
                return m;
            }
            for n in 0..m {
                lset(d.ptr, d.off, at(n) as usize, code);
            }
            return m;
        };
        // The bytes of the blocks each stream touches, as host addresses: the streams share
        // blocks exactly when these overlap, whatever objects they were found in.
        let span = |x: &Stream| {
            let (a, b) = (x.e0, x.e0 + x.step * (m - 1));
            let base = x.ptr as usize + x.off;
            (
                base + a.min(b) as usize / 64 * 32,
                base + a.max(b) as usize / 64 * 32 + 32,
            )
        };
        let (sr, dr) = (span(&s), span(&d));
        let apart = sr.1 <= dr.0 || dr.1 <= sr.0;
        if s.step == d.step && s.step.abs() == 1 && apart {
            // The same run of elements in the same order: a copy of planes.
            let (sa, da) = if s.step == 1 {
                (s.e0, d.e0)
            } else {
                (s.e0 - (m - 1), d.e0 - (m - 1))
            };
            let mut done = 0usize;
            while done < m as usize {
                let k = (m as usize - done).min(64);
                let v = get_run(s.ptr, s.off, sa as usize + done, k);
                put_run(d.ptr, d.off, da as usize + done, k, v);
                done += k;
            }
            return m;
        }
        // In order, element by element: what the loop does, whatever the vectors overlap.
        for n in 0..m {
            let c = lget(s.ptr, s.off, (s.e0 + s.step * n) as usize);
            if c > 8 {
                return n;
            }
            lset(d.ptr, d.off, (d.e0 + d.step * n) as usize, c);
        }
        m
    }
}

/// The iterations `0..m` of a plan compiled code made (`Gen::eloop_plan`): the elements
/// `s_e0 + s_step * n` of the vector at `src` (null: a fill with `code`) to the elements
/// `d_e0 + d_step * n` of the vector at `dst`, each pointer at its vector's first byte. Returns
/// how many iterations were done (fewer where a source element is not a code).
///
/// # Safety
/// Compiled code checked that every element lies in the memory its pointer points into.
#[allow(clippy::too_many_arguments)]
pub unsafe fn exec_planned(
    src: *mut u8,
    s_e0: i64,
    s_step: i64,
    dst: *mut u8,
    d_e0: i64,
    d_step: i64,
    code: u8,
    m: i64,
) -> i64 {
    let stream = |ptr, e0, step| Stream {
        ptr,
        off: 0,
        e0,
        step,
    };
    let s = (!src.is_null()).then(|| stream(src, s_e0, s_step));
    exec(s, stream(dst, d_e0, d_step), code, m)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Flat(Vec<u8>);
    impl LoopMem for Flat {
        fn region(&mut self, a: u64, _w: bool) -> Option<(*mut u8, u64)> {
            // One object per high half: index 1 is the vectors, 2 the descriptors.
            let _ = a;
            Some((self.0.as_mut_ptr(), self.0.len() as u64))
        }
    }

    fn pad(i: &[u64]) -> [u64; yavs_interp::eloop::MAX_INPUTS] {
        let mut p = [0; yavs_interp::eloop::MAX_INPUTS];
        p[..i.len()].copy_from_slice(i);
        p
    }

    fn put_code(m: &mut [u8], e: usize, c: u8) {
        // SAFETY: the tests' vectors are large enough for the element.
        unsafe { super::lset(m.as_mut_ptr(), 0, e, c) }
    }

    fn lget(m: &[u8], off: usize, e: usize) -> u8 {
        // SAFETY: as above.
        unsafe { super::lget(m.as_ptr(), off, e) }
    }

    fn lset(m: &mut [u8], off: usize, e: usize, c: u8) {
        // SAFETY: as above.
        unsafe { super::lset(m.as_mut_ptr(), off, e, c) }
    }

    /// A copy `dst(e0d + n * sd) := src(e0s + n * ss)`, `n` in `0 ..= m`, against the plain loop.
    #[test]
    fn copy_matches_the_plain_loop() {
        // Steps: input 0 base (the vectors), the index as loop variable scaled and shifted.
        // GET base=in0 idx=(ind*ss + es), SET base=in0 idx=(ind*sd + ed) v=get.
        for (ss, sd) in [(1, 1), (-1, -1), (1, -1), (2, 1), (1, 3)] {
            for (es, ed, m) in [(10, 200, 5), (70, 300, 40), (130, 20, 70), (0, 128, 3)] {
                let mut base = vec![0u8; 32 * 16];
                for e in 0..400 {
                    put_code(&mut base, e, (e % 9) as u8);
                }
                let want = {
                    let mut w = base.clone();
                    for n in 0..m {
                        let c = lget(&w, 0, (es + ss * n).max(0) as usize);
                        lset(&mut w, 0, (ed + sd * n).max(0) as usize, c);
                    }
                    w
                };
                // steps: 0: MUL ind, ss; 1: ADD s0, es; 2: MUL ind, sd; 3: ADD s2, ed; 4: GET; 5: SET
                // (inputs: 0 base, 1 ss, 2 es, 3 sd, 4 ed)
                let prog = [
                    MUL, IND, -3, 0, ADD, 0, -4, 0, MUL, IND, -5, 0, ADD, 2, -6, 0, GET, -2, 1, 0,
                    SET, -2, 3, 4,
                ];
                let inputs = [0u64, ss as u64, es as u64, sd as u64, ed as u64];
                let mut mem = Flat(base.clone());
                // Reference run is only valid when streams do not overlap in a way the plan
                // declines; compare when the plan ran.
                let done = run(&prog, &pad(&inputs), 0, m, 1, 1 << 20, &mut mem);
                if done > 0 {
                    assert_eq!(done, m, "all but the last");
                    let mut w = base.clone();
                    for n in 0..done {
                        let c = lget(&w, 0, (es + ss * n) as usize);
                        lset(&mut w, 0, (ed + sd * n) as usize, c);
                    }
                    assert_eq!(mem.0, w, "ss {ss} sd {sd} es {es} ed {ed} m {m}");
                }
                let _ = want;
            }
        }
    }

    #[test]
    fn fill_matches_the_plain_loop() {
        for sd in [1, -1, 2] {
            for (ed, m) in [(5, 9), (60, 70), (300, 3)] {
                let mut base = vec![0u8; 32 * 16];
                for e in 0..400 {
                    put_code(&mut base, e, (e % 9) as u8);
                }
                // steps: 0: MUL ind, sd; 1: ADD s0, ed; 2: SET base idx=s1 v=input 3
                let prog = [MUL, IND, -3, 0, ADD, 0, -4, 0, SET, -2, 1, -5];
                let inputs = [0u64, sd as u64, ed as u64, 3];
                let mut mem = Flat(base.clone());
                let done = run(&prog, &pad(&inputs), 0, m, 1, 1 << 20, &mut mem);
                if ed + sd * (m - 1) < 0 {
                    assert_eq!(done, 0, "out of the vector: left to the generic loop");
                    continue;
                }
                assert_eq!(done, m);
                let mut w = base.clone();
                for n in 0..done {
                    lset(&mut w, 0, (ed + sd * n) as usize, 3);
                }
                assert_eq!(mem.0, w);
            }
        }
    }

    /// Objects by the index in the high half of the address.
    struct Objs(Vec<Vec<u8>>);
    impl LoopMem for Objs {
        fn region(&mut self, a: u64, _w: bool) -> Option<(*mut u8, u64)> {
            let o = self.0.get_mut((a >> 32) as usize)?;
            Some((o.as_mut_ptr(), o.len() as u64))
        }
    }

    /// Which steps of the `get_row` loop do not change, and the programs `Gen::eloop_plan` leaves
    /// to `run`.
    #[test]
    fn invariant_steps_of_loops() {
        let get_row = [
            5, -2, 1, -1, 2, -3, 0, 0, 3, -4, 1, 0, 9, -5, 2, 0, 5, -6, 0, -1, 6, -6, 0, 0, 3, 5,
            -7, 0, 3, 6, 4, 0, 10, -8, 7, 3,
        ];
        let inv = invariant_steps(&get_row, 7).expect("the get_row loop");
        assert_eq!(
            inv,
            [false, false, false, true, false, true, true, false, true]
        );
        // Not enough inputs, a step that is not there, two factors that change, a base that does.
        assert_eq!(invariant_steps(&get_row, 6), None);
        assert_eq!(invariant_steps(&[MUL, IND, IND, 0], 1), None);
        assert_eq!(invariant_steps(&[ADD, 3, -2, 0], 1), None);
        assert_eq!(invariant_steps(&[MUL, IND, IND, 0, GET, 0, 0, 0], 1), None);
        // A fill with an unchanging code.
        let fill = [MUL, IND, -3, 0, ADD, 0, -4, 0, SET, -2, 1, -5];
        assert!(invariant_steps(&fill, 4).is_some());
        // The code of a fill must not be the loop variable.
        assert_eq!(invariant_steps(&[SET, -2, -1, IND], 1), None);
    }

    /// The `get_row` loop of a 256 x 8 matrix, many times: `perf stat -- cargo test --release
    /// -p yavs-llvm --features llvm eloop_cost -- --ignored` gives the instructions of a call.
    #[test]
    #[ignore]
    fn eloop_cost() {
        let prog = [
            5, -2, 1, -1, 2, -3, 0, 0, 3, -4, 1, 0, 9, -5, 2, 0, 5, -6, 0, -1, 6, -6, 0, 0, 3, 5,
            -7, 0, 3, 6, 4, 0, 10, -8, 7, 3,
        ];
        let mut dope = vec![0u8; 48];
        for (k, w) in [255i64, 0, 1, 7, 0, 1].iter().enumerate() {
            dope[k * 8..k * 8 + 8].copy_from_slice(&w.to_le_bytes());
        }
        let mut rdope = vec![0u8; 24];
        for (k, w) in [7i64, 0, 1].iter().enumerate() {
            rdope[k * 8..k * 8 + 8].copy_from_slice(&w.to_le_bytes());
        }
        let mut mem = Objs(vec![vec![], dope, vec![0u8; 32 * 40], rdope, vec![0u8; 32]]);
        let a = |o: u64| o << 32;
        // r3 dope, r26, r16, r1 base, r23 result dope, r19 = 1, r22 result base
        let inputs = [a(1), 8 * 5, 2047, a(2), a(3), 1, a(4)];
        let mut done = 0;
        for _ in 0..2_000_000 {
            done += run(&prog, &pad(&inputs), 7, 0, -1, 1 << 24, &mut mem);
        }
        assert_eq!(done, 2_000_000 * 7);
    }
}
