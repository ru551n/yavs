//! Toggle coverage (#343): an observer on the kernel's value-change bus that counts, per
//! scalar `bit`/`std_ulogic` subelement of every signal and port, its 0->1 and 1->0
//! transitions. 'L' reads as '0' and 'H' as '1'; every other value ('U', 'X', 'Z', 'W', '-')
//! is neither, so a change through it (0 -> 'Z' -> 1) counts no toggle. The observer exists
//! only with toggle coverage on, so a run without it pays nothing.
//!
//! Items are keyed by the path of the scope declaring the signal (`:top:u1`, `:top:g(3)`)
//! and the subelement's name (`data(3)`, `r.valid`, `mem(2)(7)`): no offsets, so databases
//! of a changed design still merge.

use std::sync::{Arc, Mutex, PoisonError};

use yavs_elab::{Case, Elaborated, ObjKind, Shape};
use yavs_kernel::{Elem, Hook, Observer, Phase, Range};
use yavs_sem::Db;
use yavs_sem::hir::Direction;

use crate::{Database, Entry, Key, Kind};

/// A signal left out because it has more scalar subelements than the cap.
pub struct Skipped {
    pub path: String,
    pub elems: u64,
}

/// One named scalar: its key parts, declaration and bit (index into the flat arrays).
struct Name {
    path: String,
    sig: String,
    file: String,
    line: u32,
    bit: u32,
}

struct State {
    /// Per kernel signal: the bit of its element at offset 0, if tracked.
    base: Vec<Option<u32>>,
    /// Bits in use (the tracked elements of all signals, one bit each).
    nbits: usize,
    /// Per bit, as bitsets: the last level was '0'/'L' (`prev0`) or '1'/'H' (`prev1`); neither
    /// is both clear.
    prev0: Vec<u64>,
    prev1: Vec<u64>,
    /// Per bit: falls (1->0), rises (0->1).
    counts: Vec<[u64; 2]>,
    names: Vec<Name>,
}

/// The toggle observer. Clone it into `SimControl::subscribe`, keep one for [`Toggles::add_to`].
#[derive(Clone)]
pub struct Toggles(Arc<Mutex<State>>);

/// `n` (at most 64) bits of `bits` from bit `g`, in bits 0 and up.
#[inline]
fn get(bits: &[u64], g: usize, n: usize) -> u64 {
    let (w, s) = (g / 64, g % 64);
    let mut v = bits[w] >> s;
    if s + n > 64 {
        v |= bits[w + 1] << (64 - s);
    }
    v & mask(n)
}

/// Store the low `n` (at most 64) bits of `v` at bit `g`.
#[inline]
fn put(bits: &mut [u64], g: usize, n: usize, v: u64) {
    let (w, s) = (g / 64, g % 64);
    let m = mask(n);
    bits[w] = bits[w] & !(m << s) | (v & m) << s;
    if s + n > 64 {
        bits[w + 1] = bits[w + 1] & !(m >> (64 - s)) | (v & m) >> (64 - s);
    }
}

#[inline]
fn mask(n: usize) -> u64 {
    if n >= 64 { !0 } else { (1 << n) - 1 }
}

/// The observer over every signal and port of `e` whose scalars are `bit` or `std_ulogic`;
/// signals with more than `max` such scalars are left out and returned.
pub fn toggles(db: &Db, e: &Elaborated, max: u64) -> (Toggles, Vec<Skipped>) {
    let mut st = State {
        base: vec![None; e.nets.signal_count()],
        nbits: 0,
        prev0: Vec::new(),
        prev1: Vec::new(),
        counts: Vec::new(),
        names: Vec::new(),
    };
    let mut skipped = Vec::new();
    for (si, sc) in e.hier.scopes.iter().enumerate() {
        // Package signals under `library.package`, as package statements.
        let p = match sc.kind {
            yavs_elab::ScopeKind::Package { unit } => {
                let u = &db.units[unit];
                format!("{}.{}", db.libs[u.lib].name.text, u.name.text).to_ascii_lowercase()
            }
            _ => e.hier.path_name(yavs_elab::ScopeId(si as u32), Case::Lrm),
        };
        let path = p.strip_suffix(':').filter(|p| !p.is_empty()).unwrap_or(&p);
        for &o in &sc.objs {
            let obj = e.nets.obj(o);
            if !matches!(obj.kind, ObjKind::Signal | ObjKind::Port(_)) {
                continue;
            }
            let mut out = Vec::new();
            walk(
                db,
                &obj.shape,
                obj.name.text.to_string(),
                obj.leaves.0.0,
                0,
                &mut out,
            );
            if out.len() as u64 > max {
                skipped.push(Skipped {
                    path: format!("{path}:{}", obj.name.text),
                    elems: out.len() as u64,
                });
                continue;
            }
            let span = db.decls[obj.decl].span;
            let f = db.sources.get(span.file);
            let line = f.line_col(span.start).0 as u32;
            for (sig, leaf, k) in out {
                let n = e.nets.leaves[leaf as usize].ty.len;
                // A port collapsed into part of its actual's leaf counts that part's bins.
                let (off, own) = obj.part.map_or((0, n), |r| (r.offset, r.len));
                let (ks, at) = e.nets.kelem(yavs_elab::LeafId(leaf), off);
                let base = *st.base[ks.0 as usize].get_or_insert_with(|| {
                    let b = st.nbits as u32;
                    st.nbits += e.nets.signal_len(ks) as usize;
                    st.counts.resize(st.nbits, [0; 2]);
                    b
                });
                st.names.push(Name {
                    path: path.to_string(),
                    sig,
                    file: f.path.clone(),
                    line,
                    // Kernel offset 0 is the rightmost element.
                    bit: base + at + (own as u64 - 1 - k) as u32,
                });
            }
        }
    }
    // A word more: `get` and `put` read the next word when a range crosses a boundary.
    st.prev0 = vec![0; st.nbits / 64 + 2];
    st.prev1 = vec![0; st.nbits / 64 + 2];
    (Toggles(Arc::new(Mutex::new(st))), skipped)
}

/// The `bit`/`std_ulogic` scalars of `shape` (named `name`, first leaf `leaf`, instance `k`
/// row major among the instances of `shape` in its leaves): (name, leaf, element).
fn walk(
    db: &Db,
    shape: &Shape,
    name: String,
    leaf: u32,
    k: u64,
    out: &mut Vec<(String, u32, u64)>,
) {
    match shape {
        Shape::Scalar { ty, elem, .. } => {
            if *elem == Elem::Logic && Some(*ty) != db.well_known.boolean {
                out.push((name, leaf, k));
            }
        }
        Shape::Array { dims, elem, .. } => {
            let n = shape.len();
            for j in 0..n {
                let child = format!("{name}({})", index_image(dims, j));
                walk(db, elem, child, leaf, k * n + j, out);
            }
        }
        Shape::Record { fields, .. } => {
            let mut l = leaf;
            for (f, s) in fields {
                walk(db, s, format!("{name}.{}", f.text), l, k, out);
                l += s.leaves().len() as u32;
            }
        }
    }
}

/// `3` or `1,2`: the index values of row-major element `j` of an array with `dims`.
fn index_image(dims: &[yavs_elab::Bounds], mut j: u64) -> String {
    let mut ix = vec![0i64; dims.len()];
    for (d, slot) in dims.iter().zip(ix.iter_mut()).rev() {
        let n = d.len();
        let p = (j % n) as i64;
        j /= n;
        *slot = match d.dir {
            Direction::To => d.left + p,
            Direction::Downto => d.left - p,
        };
    }
    ix.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
}

impl State {
    /// Compare the elements of `ranges` of kernel signal `sig` with their last level.
    fn change(&mut self, hook: &Hook<'_>, sig: u32, ranges: &[Range], count: bool) {
        let Some(base) = self.base.get(sig as usize).copied().flatten() else {
            return;
        };
        let Ok(v) = hook.view(yavs_kernel::SignalId(sig)) else {
            return;
        };
        // An element that did not change has its last level already: one range over the hull of
        // the changed ones counts the same as each of them, with one pass of the plane words
        // (unless the changes are far apart).
        let (mut lo, mut hi, mut total) = (u32::MAX, 0, 0);
        for r in ranges {
            lo = lo.min(r.offset);
            hi = hi.max(r.offset + r.len);
            total += r.len;
        }
        let hull = [Range::new(lo, hi.saturating_sub(lo))];
        let ranges = if ranges.len() > 1 && hi - lo <= total.saturating_mul(4) + 64 {
            &hull[..]
        } else {
            ranges
        };
        for r in ranges {
            let mut i = 0;
            while i < r.len as usize {
                let n = (r.len as usize - i).min(64);
                let at = r.offset as usize + i;
                // Byte storage holds no `std_ulogic` element (nothing is tracked there).
                let Some([p0, p1, _, p3]) = v.planes(at, n) else {
                    return;
                };
                // std_ulogic codes: '0' 2, '1' 3, 'L' 6, 'H' 7 (BIT is stored as 2/3): bit 1
                // set, bit 3 clear, bit 0 the level.
                let defined = p1 & !p3;
                let (one, zero) = (defined & p0, defined & !p0);
                let g = base as usize + at;
                if count {
                    let rise = one & get(&self.prev0, g, n);
                    let fall = zero & get(&self.prev1, g, n);
                    for (m, d) in [(fall, 0), (rise, 1)] {
                        let mut m = m;
                        while m != 0 {
                            let c = &mut self.counts[g + m.trailing_zeros() as usize][d];
                            *c = c.saturating_add(1);
                            m &= m - 1;
                        }
                    }
                }
                put(&mut self.prev0, g, n, zero);
                put(&mut self.prev1, g, n, one);
                i += n;
            }
        }
    }
}

impl Observer for Toggles {
    /// Only the start of the simulation: the kernel calls nothing else (every delta cycle
    /// otherwise pays six hook calls).
    fn wants_phase(&self, phase: Phase) -> bool {
        phase == Phase::StartOfSim
    }

    fn phase(&mut self, hook: &mut Hook<'_>, phase: Phase) {
        if phase != Phase::StartOfSim {
            return;
        }
        let mut st = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        for sig in 0..st.base.len() as u32 {
            if st.base[sig as usize].is_some() {
                let len = match hook.view(yavs_kernel::SignalId(sig)) {
                    Ok(v) => v.len() as u32,
                    _ => continue,
                };
                st.change(hook, sig, &[Range::new(0, len)], false);
            }
        }
    }

    fn value_change(
        &mut self,
        hook: &mut Hook<'_>,
        signal: yavs_kernel::SignalId,
        range: yavs_kernel::Range,
    ) {
        let mut st = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        st.change(hook, signal.0, &[range], true);
    }

    /// One lock for the whole cycle.
    fn value_changes(&mut self, hook: &mut Hook<'_>, signals: &[u32]) {
        let mut st = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        for &s in signals {
            st.change(hook, s, hook.changed(yavs_kernel::SignalId(s)), true);
        }
    }
}

impl Toggles {
    /// Add one item per scalar and direction, count 0 included, to `db`, which holds no toggle
    /// items yet.
    pub fn add_to(&self, db: &mut Database) {
        let st = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let mut items: Vec<(Key, Entry)> = Vec::with_capacity(st.names.len() * 2);
        for n in &st.names {
            for rise in [false, true] {
                items.push((
                    Key {
                        path: n.path.clone(),
                        kind: Kind::Toggle(rise),
                        hash: 0,
                        occ: 0,
                        sig: n.sig.clone(),
                    },
                    Entry {
                        file: n.file.clone(),
                        line: n.line,
                        count: st.counts[n.bit as usize][usize::from(rise)],
                        text: String::new(),
                        tests: Vec::new(),
                    },
                ));
            }
        }
        // A scalar named twice keeps its first entry and the sum of the counts. The sort is
        // stable and the names come in nearly sorted order; one bulk insert beats a search per item.
        items.sort_by(|a, b| a.0.cmp(&b.0));
        let mut merged: Vec<(Key, Entry)> = Vec::with_capacity(items.len());
        for (k, e) in items {
            match merged.last_mut() {
                Some(last) if last.0 == k => last.1.count = last.1.count.saturating_add(e.count),
                _ => merged.push((k, e)),
            }
        }
        db.items.append(&mut merged.into_iter().collect());
    }
}

#[cfg(test)]
mod tests {
    use super::{get, put};

    /// Bit ranges across a word boundary read back what was stored and leave their neighbours.
    #[test]
    fn bit_ranges_cross_words() {
        let mut bits = vec![0u64; 4];
        put(&mut bits, 60, 10, 0b11_0101_1011);
        assert_eq!(get(&bits, 60, 10), 0b11_0101_1011);
        assert_eq!(get(&bits, 56, 4), 0);
        assert_eq!(get(&bits, 70, 8), 0);
        put(&mut bits, 62, 4, 0);
        assert_eq!(get(&bits, 60, 10), 0b11_0100_0011);
        put(&mut bits, 0, 64, !0);
        assert_eq!(bits[0], !0);
        assert_eq!(get(&bits, 0, 64), !0);
        put(&mut bits, 64, 64, 0x0123_4567_89ab_cdef);
        assert_eq!(bits[1], 0x0123_4567_89ab_cdef);
    }
}
