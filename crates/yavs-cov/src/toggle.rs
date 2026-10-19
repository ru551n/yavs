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
use yavs_kernel::{Elem, Hook, Observer, Phase, Value};
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
    /// Per kernel leaf (= kernel signal): the bit of its element at offset 0, if tracked.
    base: Vec<Option<u32>>,
    /// Per bit: 0, 1, or 2 for neither.
    prev: Vec<u8>,
    /// Per bit: falls (1->0), rises (0->1).
    counts: Vec<[u64; 2]>,
    names: Vec<Name>,
}

/// The toggle observer. Clone it into `SimControl::subscribe`, keep one for [`Toggles::add_to`].
#[derive(Clone)]
pub struct Toggles(Arc<Mutex<State>>);

fn level(code: u8) -> u8 {
    // std_ulogic codes: '0' 2, '1' 3, 'L' 6, 'H' 7 (BIT is stored as 2/3).
    match code {
        2 | 6 => 0,
        3 | 7 => 1,
        _ => 2,
    }
}

/// The observer over every signal and port of `e` whose scalars are `bit` or `std_ulogic`;
/// signals with more than `max` such scalars are left out and returned.
pub fn toggles(db: &Db, e: &Elaborated, max: u64) -> (Toggles, Vec<Skipped>) {
    let mut st = State {
        base: vec![None; e.nets.leaves.len()],
        prev: Vec::new(),
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
                let base = *st.base[leaf as usize].get_or_insert_with(|| {
                    let b = st.prev.len() as u32;
                    st.prev.resize(st.prev.len() + n as usize, 2);
                    st.counts.resize(st.prev.len(), [0; 2]);
                    b
                });
                st.names.push(Name {
                    path: path.to_string(),
                    sig,
                    file: f.path.clone(),
                    line,
                    // Kernel offset 0 is the rightmost element.
                    bit: base + (n as u64 - 1 - k) as u32,
                });
            }
        }
    }
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
    /// Compare elements `off .. off + len` of `leaf` with their last level.
    fn change(&mut self, hook: &Hook<'_>, leaf: u32, off: u32, len: u32, count: bool) {
        let Some(base) = self.base.get(leaf as usize).copied().flatten() else {
            return;
        };
        let r = yavs_kernel::SignalId(leaf).slice(off, len);
        let Ok(Value::Logic(v)) = hook.value(r) else {
            return;
        };
        for i in 0..len as usize {
            let b = (base + off) as usize + i;
            let now = level(v.get(i) as u8);
            let was = std::mem::replace(&mut self.prev[b], now);
            if count && was != now && now != 2 && was != 2 {
                let c = &mut self.counts[b][now as usize];
                *c = c.saturating_add(1);
            }
        }
    }
}

impl Observer for Toggles {
    fn phase(&mut self, hook: &mut Hook<'_>, phase: Phase) {
        if phase != Phase::StartOfSim {
            return;
        }
        let mut st = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        for leaf in 0..st.base.len() as u32 {
            if st.base[leaf as usize].is_some() {
                let len = match hook.value(yavs_kernel::SignalId(leaf)) {
                    Ok(Value::Logic(v)) => v.len() as u32,
                    _ => continue,
                };
                st.change(hook, leaf, 0, len, false);
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
        st.change(hook, signal.0, range.offset, range.len, true);
    }
}

impl Toggles {
    /// Add one item per scalar and direction, count 0 included, to `db`.
    pub fn add_to(&self, db: &mut Database) {
        let st = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        for n in &st.names {
            for rise in [false, true] {
                let e = db
                    .items
                    .entry(Key {
                        path: n.path.clone(),
                        kind: Kind::Toggle(rise),
                        hash: 0,
                        occ: 0,
                        sig: n.sig.clone(),
                    })
                    .or_insert_with(|| Entry {
                        file: n.file.clone(),
                        line: n.line,
                        count: 0,
                        text: String::new(),
                    });
                e.count = e
                    .count
                    .saturating_add(st.counts[n.bit as usize][usize::from(rise)]);
            }
        }
    }
}
