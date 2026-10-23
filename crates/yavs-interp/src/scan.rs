//! Search loops: the loop shape the interpreter runs as one op (`pre::SCAN`, `Machine::scan_loop`)
//! and compiled code runs through a helper, with the one function that does the work
//! ([`scan_bytes`]) and the finder of the shape in the IR ([`search_loops`]) (#578).

use yavs_ir::{BlockId, Func, Inst, IntOp, Reg, Width};

/// What a search loop does each iteration: the element at the position of the index in the
/// descriptor is compared with `x` (`m`, the element on the left), then the index with `e`
/// (`m2`, as the IR compare has its operands), then the index steps by `delta`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanShape {
    pub stride: u32,
    pub w: Width,
    pub m: u8,
    pub m2: u8,
    /// The second compare has the index as its left operand.
    pub idx_left: bool,
    pub delta: i64,
}

/// Where a search loop starts: the index, the values the two compares use (`x` against the
/// element, `e` against the index) and the most iterations to do.
#[derive(Clone, Copy, Debug)]
pub struct ScanFrom {
    pub i0: i64,
    pub x: i64,
    pub e: i64,
    pub limit: u32,
}

/// Where a search loop stopped: the index and the element last read, which exit (1: the
/// element compare, 2: the index compare, 0: neither, the generic code goes on) and the
/// iterations that read an element.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scan {
    pub i: i64,
    pub ch: u64,
    pub hit: u8,
    pub iters: u32,
}

/// The iterations of a search loop while neither exit is taken: `dim` is the descriptor's
/// `(left, right, direction)`, `mem` the bytes of the object with the array at `soff`, `i0` the
/// index, `x` and `e` the values the two compares use. Stops at the first thing the generic code
/// should handle (an index outside the bounds, bytes outside the object, an overflow); the result
/// says how far it got. The position is kept as a number of steps from the end of the range the
/// loop starts at, so the bounds test is one unsigned compare and the offset a multiply-add.
#[inline(never)]
pub fn scan_bytes(
    p: &ScanShape,
    dim: (i64, i64, u64),
    mem: &[u8],
    soff: u64,
    from: ScanFrom,
) -> Scan {
    let ScanFrom { i0, x, e, limit } = from;
    let none = Scan {
        i: i0,
        ..Scan::default()
    };
    let (l, r, dir) = dim;
    let (lo, hi, up) = match dir {
        0 => (l, r, true),
        1 => (r, l, false),
        _ => return none,
    };
    let n = p.w.bytes().min(8) as usize;
    // A null range or a zero stride is the generic code's; so is an object too short for an element.
    if lo > hi || p.stride == 0 || mem.len() < n {
        return none;
    }
    let maxoff = ((mem.len() - n) as u64).min(u64::from(u32::MAX));
    if soff > maxoff {
        return none;
    }
    let span = (hi as u64).wrapping_sub(lo as u64);
    let ctx = Ctx {
        p,
        // The greatest position whose element lies in the object (and below 4 GiB).
        posmax: span.min((maxoff - soff) / u64::from(p.stride)),
        soff,
        start: if up {
            i0.wrapping_sub(lo)
        } else {
            hi.wrapping_sub(i0)
        } as u64,
        dpos: if up { p.delta } else { p.delta.wrapping_neg() },
        i0,
        x,
        e,
        limit,
    };
    // The compares that nearly every loop has (equal, not equal) get loops of their own.
    let kind = |m: u8| match m {
        0b010 => 0,
        0b101 => 1,
        _ => 2,
    };
    match (p.w, kind(p.m), kind(p.m2)) {
        (Width::B1, 0, 0) => run::<1, 0, 0>(&ctx, mem),
        (Width::B1, 0, _) => run::<1, 0, 2>(&ctx, mem),
        (Width::B1, 1, 0) => run::<1, 1, 0>(&ctx, mem),
        (Width::B1, 1, _) => run::<1, 1, 2>(&ctx, mem),
        (Width::B1, _, 0) => run::<1, 2, 0>(&ctx, mem),
        (Width::B1, _, _) => run::<1, 2, 2>(&ctx, mem),
        (Width::B4, 0, 0) => run::<4, 0, 0>(&ctx, mem),
        (Width::B4, _, _) => run::<4, 2, 2>(&ctx, mem),
        (_, 0, 0) => run::<8, 0, 0>(&ctx, mem),
        _ => run::<8, 2, 2>(&ctx, mem),
    }
}

/// `cmp_holds` with the mask known for `K` 0 (equal) and 1 (not equal).
#[inline(always)]
fn holds<const K: u8>(m: u8, a: i64, b: i64) -> bool {
    match K {
        0 => a == b,
        1 => a != b,
        _ => crate::pre::cmp_holds(m, a, b),
    }
}

struct Ctx<'a> {
    p: &'a ScanShape,
    posmax: u64,
    soff: u64,
    start: u64,
    dpos: i64,
    i0: i64,
    x: i64,
    e: i64,
    limit: u32,
}

/// The loop of [`scan_bytes`] for elements of `N` bytes.
fn run<const N: usize, const K1: u8, const K2: u8>(c: &Ctx<'_>, mem: &[u8]) -> Scan {
    let p = c.p;
    let (stride, m, m2) = (u64::from(p.stride), p.m, p.m2);
    // Bytes at stride 1 found by equality, the index compared for equality, a step of one: the
    // window the loop can cover is a slice, and the first match in it a search of the bytes.
    if N == 1 && K1 == 0 && K2 == 0 && p.stride == 1 && p.delta.unsigned_abs() == 1 {
        return window(c, mem);
    }
    let (mut i, mut pos) = (c.i0, c.start);
    let (mut last, mut ch, mut iters, mut hit) = (c.i0, 0u64, 0u32, 0u8);
    while iters < c.limit {
        // Outside the range or the object (the multiplication cannot wrap below `posmax`).
        if pos > c.posmax {
            break;
        }
        let off = (c.soff + pos * stride) as usize;
        let Some(b) = mem.get(off..off + N) else {
            break;
        };
        ch = match N {
            1 => u64::from(b[0]),
            4 => i64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]])) as u64,
            _ => u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
        };
        last = i;
        iters += 1;
        if holds::<K1>(m, ch as i64, c.x) {
            hit = 1;
            break;
        }
        let (a2, b2) = if p.idx_left { (i, c.e) } else { (c.e, i) };
        if holds::<K2>(m2, a2, b2) {
            hit = 2;
            break;
        }
        match i.checked_add(p.delta) {
            Some(v) => i = v,
            None => break,
        }
        last = i;
        pos = pos.wrapping_add(c.dpos as u64);
    }
    Scan {
        i: last,
        ch,
        hit,
        iters,
    }
}

/// [`run`] for single bytes at stride 1, `==` against `x` and `==` for the index, steps of one:
/// the iterations that read an element are a window of the object (as far as the range, the object,
/// the limit and the index `e` allow), searched for the byte `x` in one go. The result is what the
/// iteration by iteration loop gives.
fn window(c: &Ctx<'_>, mem: &[u8]) -> Scan {
    let p = c.p;
    let none = Scan {
        i: c.i0,
        ..Scan::default()
    };
    if c.start > c.posmax || c.limit == 0 {
        return none;
    }
    let forward = c.dpos == 1;
    // Positions left in the direction of the steps, the iterations until the index meets `e`
    // (that one included), and the limit.
    let room = if forward { c.posmax - c.start } else { c.start } + 1;
    let to_e = (i128::from(c.e) - i128::from(c.i0)) * i128::from(p.delta);
    let e_at = u64::try_from(to_e).map_or(u64::MAX, |v| v.saturating_add(1));
    let cnt = room.min(e_at).min(u64::from(c.limit)) as usize;
    let off0 = (c.soff + c.start) as usize;
    let (win, back) = if forward {
        (&mem[off0..off0 + cnt], false)
    } else {
        (&mem[off0 + 1 - cnt..=off0], true)
    };
    // The byte `x` can only match as a byte.
    let found = u8::try_from(c.x).ok().and_then(|x| {
        if back {
            win.iter().rposition(|&b| b == x).map(|k| win.len() - 1 - k)
        } else {
            win.iter().position(|&b| b == x)
        }
    });
    let at = |k: usize| i128::from(c.i0) + k as i128 * i128::from(p.delta);
    let byte = |k: usize| u64::from(if back { win[win.len() - 1 - k] } else { win[k] });
    let (iters, hit) = match found {
        Some(k) => (k + 1, 1),
        None if cnt as u64 == e_at => (cnt, 2),
        None => (cnt, 0),
    };
    let last = at(iters - 1);
    // After an iteration without an exit the index steps (an overflow ends the loop first).
    let i = if hit == 0 {
        i64::try_from(at(iters)).unwrap_or(last as i64)
    } else {
        last as i64
    };
    Scan {
        i,
        ch: byte(iters - 1),
        hit,
        iters: iters as u32,
    }
}

/// A search loop in the IR: the head block is `Pos`, `Index`, `Load`, `Cmp`, `CondBr` (the
/// element against `x`: taken to `found`), the next block the compare of the index with `e`
/// (taken to `end`) and the third `i += delta` and the jump back to the head. Nothing else is in
/// the loop, so the registers the iterations define besides `ch` and `idx` are dead outside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchLoop {
    /// The pc of the head block's `Pos`.
    pub head: u32,
    pub found: BlockId,
    pub end: BlockId,
    pub shape: ScanShape,
    pub dim: u8,
    pub dope: Reg,
    pub base: Reg,
    pub idx: Reg,
    /// The element read last.
    pub ch: Reg,
    pub x: Reg,
    pub e: Reg,
}

/// The search loops of `f`.
pub fn search_loops(f: &Func) -> Vec<SearchLoop> {
    let n = f.reg_ty.len();
    let mut defs = vec![0u32; n + 1];
    let mut konst: Vec<Option<i64>> = vec![None; n + 1];
    for i in &f.code {
        for r in crate::inst_regs(f, i).1 {
            if let Some(d) = defs.get_mut(r.0 as usize) {
                *d += 1;
            }
        }
        if let Inst::Const { dst, v } = *i
            && let Some(k) = konst.get_mut(dst.0 as usize)
        {
            *k = Some(v);
        }
    }
    // A block that only jumps on is the block it jumps to (the interpreter skips such a `Br`).
    let skip = |mut b: BlockId| {
        for _ in 0..4 {
            match f
                .blocks
                .get(b)
                .and_then(|k| f.code.get(k.start as usize..k.end as usize))
            {
                Some(&[Inst::Br { to }]) => b = to,
                _ => break,
            }
        }
        b
    };
    let block = |b: BlockId| {
        f.blocks
            .get(b)
            .and_then(|b| f.code.get(b.start as usize..b.end as usize))
    };
    let mut out = Vec::new();
    for (hb, h) in f.blocks.iter() {
        let Some(
            &[
                Inst::Pos {
                    dst: p,
                    dope,
                    dim,
                    idx,
                    ..
                },
                Inst::Index {
                    dst: a,
                    base,
                    pos,
                    stride,
                },
                Inst::Load { dst: ch, addr, w },
                Inst::Cmp {
                    op,
                    dst: c,
                    a: ca,
                    b: cb,
                },
                Inst::CondBr {
                    c: cc,
                    t: found,
                    f: latch,
                },
            ],
        ) = block(hb)
            .and_then(|b| <&[Inst; 5]>::try_from(b).ok())
            .map(|b| b.as_slice())
        else {
            continue;
        };
        let Some(
            &[
                Inst::Cmp {
                    op: op2,
                    dst: c2,
                    a: ya,
                    b: yb,
                },
                Inst::CondBr {
                    c: cc2,
                    t: end,
                    f: step_blk,
                },
            ],
        ) = block(skip(latch))
            .and_then(|b| <&[Inst; 2]>::try_from(b).ok())
            .map(|b| b.as_slice())
        else {
            continue;
        };
        let Some(
            &[
                Inst::Bin {
                    op: bop @ (IntOp::Add | IntOp::Sub),
                    dst: sd,
                    a: src,
                    b: step,
                },
                Inst::Br { to: back },
            ],
        ) = block(skip(step_blk))
            .and_then(|b| <&[Inst; 2]>::try_from(b).ok())
            .map(|b| b.as_slice())
        else {
            continue;
        };
        let (x, m) = if ca == ch {
            (cb, crate::pre::cmp_mask(op))
        } else {
            let m = crate::pre::cmp_mask(op);
            (ca, (m & 0b010) | (m & 1) << 2 | (m >> 2 & 1))
        };
        let (e, idx_left) = if ya == idx { (yb, true) } else { (ya, false) };
        let Some(v) = konst
            .get(step.0 as usize)
            .copied()
            .flatten()
            .filter(|_| defs[step.0 as usize] == 1)
        else {
            continue;
        };
        let Some(delta) = (if bop == IntOp::Add {
            Some(v)
        } else {
            v.checked_neg()
        }) else {
            continue;
        };
        let w_ok = matches!(w, Width::B1 | Width::B4 | Width::B8);
        let inner = [p, a, ch, c, c2, idx];
        let regs = [dope, base, x, e, step];
        let distinct = (0..inner.len()).all(|i| (i + 1..inner.len()).all(|j| inner[i] != inner[j]));
        if !(w_ok
            && back == hb
            && pos == p
            && addr == a
            && cc == c
            && cc2 == c2
            && sd == idx
            && src == idx
            && (ca == ch) != (cb == ch)
            && (ya == idx) != (yb == idx)
            && x != idx
            && e != ch
            && distinct
            && regs.iter().all(|r| !inner.contains(r)))
        {
            continue;
        }
        // The intermediates are not read after the loop (the helper does not write them).
        let dead = |to: BlockId| {
            let Some(b) = f.blocks.get(to) else {
                return false;
            };
            b.start > 0
                && [p, a, c, c2]
                    .iter()
                    .all(|&r| !crate::pre::live_after(f, b.start as usize - 1, r))
        };
        if !(dead(found) && dead(end)) {
            continue;
        }
        out.push(SearchLoop {
            head: h.start,
            found,
            end,
            shape: ScanShape {
                stride,
                w,
                m,
                m2: crate::pre::cmp_mask(op2),
                idx_left,
                delta,
            },
            dim,
            dope,
            base,
            idx,
            ch,
            x,
            e,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The loop as the interpreter's generic ops run it, one iteration at a time.
    fn naive(p: &ScanShape, dim: (i64, i64, u64), mem: &[u8], soff: u64, from: ScanFrom) -> Scan {
        let ScanFrom { i0, x, e, limit } = from;
        let mut sc = Scan {
            i: i0,
            ..Scan::default()
        };
        let (l, r, dir) = dim;
        let n = p.w.bytes().min(8) as usize;
        let mut i = i0;
        while sc.iters < limit {
            let pos = match dir {
                0 if l <= i && i <= r => i.wrapping_sub(l) as u64,
                1 if r <= i && i <= l => l.wrapping_sub(i) as u64,
                _ => break,
            };
            let Some(off) = pos
                .checked_mul(u64::from(p.stride))
                .and_then(|d| d.checked_add(soff))
                .filter(|&o| o <= u64::from(u32::MAX))
            else {
                break;
            };
            let Some(b) = mem.get(off as usize..off as usize + n) else {
                break;
            };
            sc.ch = crate::exec::cell_w(b, p.w);
            sc.i = i;
            sc.iters += 1;
            if crate::pre::cmp_holds(p.m, sc.ch as i64, x) {
                sc.hit = 1;
                break;
            }
            let (a2, b2) = if p.idx_left { (i, e) } else { (e, i) };
            if crate::pre::cmp_holds(p.m2, a2, b2) {
                sc.hit = 2;
                break;
            }
            match i.checked_add(p.delta) {
                Some(v) => i = v,
                None => break,
            }
            sc.i = i;
        }
        sc
    }

    /// The fast loop gives the generic one's result on random shapes, ranges and bytes, edge
    /// values included (an index at the ends of the range, bytes past the object, steps that
    /// leave the range, all three widths and both directions).
    #[test]
    fn scan_bytes_matches_the_generic_loop() {
        let mut s = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let edge = [
            0i64,
            1,
            2,
            5,
            15,
            16,
            17,
            -1,
            i64::MAX,
            i64::MAX - 1,
            i64::MIN,
            i64::MIN + 1,
        ];
        let mem: Vec<u8> = (0..64).map(|k| (k * 7 % 5) as u8).collect();
        for round in 0..400_000 {
            // Every other case is the shape of the byte search (`window`).
            let byte_search = round % 2 == 0;
            let pick = |n: &mut dyn FnMut() -> u64| {
                let r = n();
                if r.is_multiple_of(3) {
                    edge[(r >> 8) as usize % edge.len()]
                } else {
                    (r >> 8) as i64 % 40 - 8
                }
            };
            let w = [Width::B1, Width::B4, Width::B8][(next() % 3) as usize];
            let mut p = ScanShape {
                stride: [0u32, 1, 4, 8, 3][(next() % 5) as usize],
                w,
                m: [0b001, 0b010, 0b011, 0b100, 0b101, 0b110][(next() % 6) as usize],
                m2: [0b001, 0b010, 0b011, 0b100, 0b101, 0b110][(next() % 6) as usize],
                idx_left: next() % 2 == 0,
                delta: [1, -1, 2, 0, i64::MAX, i64::MIN][(next() % 6) as usize],
            };
            if byte_search {
                p = ScanShape {
                    stride: 1,
                    w: Width::B1,
                    m: 0b010,
                    m2: 0b010,
                    delta: [1, -1][(next() % 2) as usize],
                    ..p
                };
            }
            let dim = (pick(&mut next), pick(&mut next), next() % 3);
            let (i0, x, e) = (pick(&mut next), pick(&mut next) % 6, pick(&mut next));
            let soff = [0u64, 3, 8, 60, 70, u64::from(u32::MAX)][(next() % 6) as usize];
            let limit = [1u32, 3, 1 << 20][(next() % 3) as usize];
            let from = ScanFrom { i0, x, e, limit };
            let a = scan_bytes(&p, dim, &mem, soff, from);
            let b = naive(&p, dim, &mem, soff, from);
            // A zero stride and a null range are left to the generic code: nothing done.
            if p.stride == 0 && b.iters != 0 {
                assert_eq!(a.iters, 0);
                continue;
            }
            assert_eq!(
                a, b,
                "{p:?} {dim:?} soff {soff} i0 {i0} x {x} e {e} limit {limit}"
            );
        }
    }
}
