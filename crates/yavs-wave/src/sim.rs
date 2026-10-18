//! The kernel side (M7, #129): the elaborated design as a [`Hierarchy`] and a [`Recorder`]
//! that subscribes to the kernel's value-change bus.
//!
//! Every recorded value is one sub-element of one kernel leaf (rule 11): a scalar, or a
//! one-dimensional array of `std_ulogic`/`bit` written as one vector. Records are scopes, arrays
//! of anything else are array scopes with one entry per element. The recorder marks the
//! entries a change touches and writes their final value once per time step (FST has no
//! deltas), in entry order, so the file does not depend on `--threads`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use yavs_elab::{Elaborated, ObjKind, ScopeId, Shape};
use yavs_kernel::{Elem, Hook, Observer, Phase, PortMode, SigRef, SimControl, SimError, Value};
use yavs_sem::Db;
use yavs_sem::decl::Designator;
use yavs_sem::hir::Direction;
use yavs_sem::types::TypeKind;

use crate::model::*;
use crate::{Error, Selection, WaveConfig, WaveWriter};

/// How one entry's kernel value becomes the byte layout of its [`SigType`] (crate docs).
#[derive(Clone, Copy)]
enum Conv {
    /// Logic elements, left first, as positions: the code minus `base` (2 for BIT, whose
    /// '0'/'1' are std_ulogic codes 2/3 in the kernel).
    Logic { base: u8 },
    /// BOOLEAN (a logic leaf) as a `u32` enum position.
    Bool,
    /// A one-byte enum as a `u32` position.
    Enum8,
    /// Already in the layout: 4-byte enums, integers, physicals, reals.
    Raw,
}

struct Entry {
    leaf: u32,
    /// Kernel offset and element count.
    offset: u32,
    len: u32,
    conv: Conv,
}

struct State {
    writer: Option<WaveWriter>,
    entries: Vec<Entry>,
    /// Per kernel leaf: its entries, by offset (disjoint).
    by_leaf: Vec<Vec<u32>>,
    dirty: Vec<bool>,
    pending: Vec<u32>,
    /// The time of the pending changes, fs.
    at: u64,
    error: Option<Error>,
}

/// The waveform subscriber. Clone it into [`SimControl::subscribe`] and keep one handle for
/// [`Recorder::finish`].
#[derive(Clone)]
pub struct Recorder(Arc<Mutex<State>>);

/// Build the hierarchy of `e` under `sel`, create the FST at `path` and return the recorder
/// and the arrays left out by the size cap.
pub fn record(
    db: &Db,
    e: &Elaborated,
    path: &Path,
    sel: Selection,
    cfg: WaveConfig,
) -> Result<(Recorder, Vec<Skipped>), Error> {
    let mut d = Design {
        db,
        e,
        b: HierarchyBuilder::new(sel),
        entries: Vec::new(),
        enums: HashMap::new(),
    };
    for (i, s) in e.hier.scopes.iter().enumerate() {
        if s.parent.is_none() {
            d.scope(ScopeId(i as u32));
        }
    }
    let Design { b, entries, .. } = d;
    let h = b.finish();
    let writer = WaveWriter::create(path, &h, cfg)?;
    let mut by_leaf = vec![Vec::new(); e.nets.leaves.len()];
    for (i, en) in entries.iter().enumerate() {
        by_leaf[en.leaf as usize].push(i as u32);
    }
    for v in &mut by_leaf {
        v.sort_by_key(|&i| entries[i as usize].offset);
    }
    let n = entries.len();
    let st = State {
        writer: Some(writer),
        entries,
        by_leaf,
        dirty: vec![false; n],
        pending: Vec::new(),
        at: 0,
        error: None,
    };
    Ok((Recorder(Arc::new(Mutex::new(st))), h.skipped))
}

impl Recorder {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Write what is still pending (a run that paused or failed inside a time step), close the
    /// file and return the first error of the run, if any.
    pub fn finish(&self, sim: &SimControl) -> Result<(), Error> {
        let mut st = self.lock();
        st.flush(|r| sim.value(r));
        if let Some(e) = st.error.take() {
            st.writer = None; // drop finishes what can be finished
            return Err(e);
        }
        match st.writer.take() {
            Some(w) => w.finish(),
            None => Ok(()),
        }
    }
}

impl State {
    fn mark(&mut self, i: u32) {
        if !self.dirty[i as usize] {
            self.dirty[i as usize] = true;
            self.pending.push(i);
        }
    }

    fn flush(&mut self, read: impl Fn(SigRef) -> Result<Value, SimError>) {
        if self.pending.is_empty() {
            return;
        }
        let mut pending = std::mem::take(&mut self.pending);
        pending.sort_unstable();
        let mut bytes = Vec::new();
        for &i in &pending {
            self.dirty[i as usize] = false;
            if self.error.is_some() {
                continue;
            }
            let en = &self.entries[i as usize];
            let r = yavs_kernel::SignalId(en.leaf).slice(en.offset, en.len);
            let v = match read(r) {
                Ok(v) => v,
                Err(e) => {
                    self.error = Some(Error::Value(format!("reading a signal: {e:?}")));
                    continue;
                }
            };
            bytes.clear();
            convert(en, &v, &mut bytes);
            let w = self.writer.as_mut().expect("writer open while recording");
            if let Err(e) = w.change(self.at, SignalId(i), &bytes) {
                self.error = Some(e);
            }
        }
        pending.clear();
        self.pending = pending;
    }
}

fn convert(en: &Entry, v: &Value, out: &mut Vec<u8>) {
    match (en.conv, v) {
        // Kernel offset 0 is the rightmost element (yavs-elab `NetGraph::select`).
        (Conv::Logic { base }, Value::Logic(l)) => out.extend(
            (0..l.len())
                .rev()
                .map(|i| (l.get(i) as u8).wrapping_sub(base)),
        ),
        (Conv::Bool, Value::Logic(l)) => {
            out.extend_from_slice(&u32::from(l.get(0) as u8).wrapping_sub(2).to_le_bytes())
        }
        (Conv::Enum8, Value::Bytes(b)) => {
            out.extend_from_slice(&u32::from(b[0]).to_le_bytes());
        }
        (Conv::Raw, Value::Bytes(b)) => out.extend_from_slice(b),
        // A mismatch reaches the writer as a wrong length, which it reports.
        _ => {}
    }
}

impl Observer for Recorder {
    fn phase(&mut self, hook: &mut Hook<'_>, phase: Phase) {
        let mut st = self.lock();
        match phase {
            Phase::StartOfSim => {
                st.at = hook.now().as_fs();
                for i in 0..st.entries.len() as u32 {
                    st.mark(i);
                }
            }
            Phase::NextTimeStep | Phase::EndOfTimeStep | Phase::EndOfSim => {
                st.flush(|r| hook.value(r))
            }
            Phase::LastKnownDelta => {}
        }
        if st.error.is_some() {
            hook.stop();
        }
    }

    fn value_change(
        &mut self,
        hook: &mut Hook<'_>,
        signal: yavs_kernel::SignalId,
        range: yavs_kernel::Range,
    ) {
        let mut st = self.lock();
        st.at = hook.now().as_fs();
        let Some(ids) = st.by_leaf.get(signal.0 as usize) else {
            return;
        };
        let (lo, hi) = (range.offset, range.offset + range.len);
        let first = ids.partition_point(|&i| {
            let en = &st.entries[i as usize];
            en.offset + en.len <= lo
        });
        let hit: Vec<u32> = ids[first..]
            .iter()
            .copied()
            .take_while(|&i| st.entries[i as usize].offset < hi)
            .collect();
        for i in hit {
            st.mark(i);
        }
    }
}

/// Walks the elaborated hierarchy into a [`HierarchyBuilder`].
struct Design<'a> {
    db: &'a Db,
    e: &'a Elaborated,
    b: HierarchyBuilder,
    entries: Vec<Entry>,
    enums: HashMap<yavs_sem::TypeId, Arc<EnumType>>,
}

impl Design<'_> {
    fn scope(&mut self, id: ScopeId) {
        let sc = self.e.hier.scope(id);
        // A component scope holds the component ports; its bound instance carries the label.
        let transparent = matches!(sc.kind, yavs_elab::ScopeKind::Component { .. });
        if !transparent {
            let (kind, comp) = match &sc.kind {
                yavs_elab::ScopeKind::Instance { entity, arch } => (
                    ScopeKind::Instance,
                    format!(
                        "{}({})",
                        self.db.units[*entity].name.text, self.db.units[*arch].name.text
                    ),
                ),
                yavs_elab::ScopeKind::Block => (ScopeKind::Block, String::new()),
                yavs_elab::ScopeKind::ForGenerate { .. } => (ScopeKind::ForGenerate, String::new()),
                yavs_elab::ScopeKind::Generate => (ScopeKind::Generate, String::new()),
                yavs_elab::ScopeKind::Process { .. } => (ScopeKind::Process, String::new()),
                // ponytail: FST has no library scope; a block keeps `lib.pkg.sig` paths.
                yavs_elab::ScopeKind::Library => (ScopeKind::Block, String::new()),
                yavs_elab::ScopeKind::Package { unit } => (
                    ScopeKind::Package,
                    self.db.units[*unit].name.text.to_string(),
                ),
                yavs_elab::ScopeKind::Component { .. } => unreachable!(),
            };
            let mut name = sc.name.text.to_string();
            if let Some(i) = &sc.index {
                name = format!("{name}({i})");
            }
            self.b.scope(&name, kind, &comp);
            for &o in &sc.objs {
                self.object(o);
            }
        }
        for &c in &sc.children {
            self.scope(c);
        }
        if !transparent {
            self.b.up();
        }
    }

    fn object(&mut self, o: yavs_elab::ObjId) {
        let obj = self.e.nets.obj(o);
        let port = match obj.kind {
            ObjKind::Signal => PortDir::None,
            ObjKind::Port(m) => match m {
                PortMode::In => PortDir::In,
                PortMode::Out => PortDir::Out,
                PortMode::Inout => PortDir::InOut,
                PortMode::Buffer => PortDir::Buffer,
            },
            // Component ports are the hidden middle of an instance; implicit signals are not
            // declared objects.
            ObjKind::ComponentPort(_) | ObjKind::Implicit => return,
        };
        self.node(&obj.shape, obj.name.text.as_str(), obj.leaves.0.0, 0, port);
    }

    /// Instance `k` (row major among all instances of `shape` in its leaves) of `shape`, whose
    /// first leaf is `leaf`.
    fn node(&mut self, shape: &Shape, name: &str, leaf: u32, k: u64, port: PortDir) {
        match shape {
            Shape::Scalar { ty, elem, res, .. } => {
                let (sty, conv) = self.scalar(*ty, *elem, res.is_some());
                let len = 1;
                let id = self.entry(leaf, k, len, conv);
                self.b.signal(name, id, sty, port, ObjClass::Signal);
            }
            Shape::Array {
                ty,
                dims,
                elem,
                res: whole,
            } => {
                if let (
                    [d],
                    Shape::Scalar {
                        ty: et,
                        elem: Elem::Logic,
                        res,
                        ..
                    },
                ) = (dims.as_slice(), &**elem)
                    && Some(*et) != self.db.well_known.boolean
                {
                    if d.is_empty() {
                        return;
                    }
                    let (SigType::Scalar(ScalarType::Enum(e)), conv) =
                        self.scalar(*et, Elem::Logic, res.is_some() || whole.is_some())
                    else {
                        unreachable!("logic scalars are enums")
                    };
                    let id = self.entry(leaf, k, d.len(), conv);
                    let sty = SigType::Vector {
                        elem: e,
                        bounds: bounds(*d),
                        type_name: Some(self.vector_name(*ty, res.is_some() || whole.is_some())),
                    };
                    self.b.signal(name, id, sty, port, ObjClass::Signal);
                    return;
                }
                let n = shape.len();
                if n == 0 {
                    return;
                }
                if self.b.array(name, n) {
                    for j in 0..n {
                        let child = format!("{name}({})", index_image(dims, j));
                        self.node(elem, &child, leaf, k * n + j, port);
                    }
                }
                self.b.up();
            }
            Shape::Record { fields, .. } => {
                self.b.scope(name, ScopeKind::Record, "");
                let mut l = leaf;
                for (fname, f) in fields {
                    self.node(f, fname.text.as_str(), l, k, port);
                    l += f.leaves().len() as u32;
                }
                self.b.up();
            }
        }
    }

    /// Row-major elements `k*len .. (k+1)*len` of `leaf`.
    fn entry(&mut self, leaf: u32, k: u64, len: u64, conv: Conv) -> SignalId {
        let n = self.e.nets.leaves[leaf as usize].ty.len as u64;
        let first = k * len;
        let offset = match conv {
            Conv::Logic { .. } | Conv::Bool => n - first - len,
            _ => first,
        };
        let id = SignalId(self.entries.len() as u32);
        self.entries.push(Entry {
            leaf,
            offset: offset as u32,
            len: len as u32,
            conv,
        });
        id
    }

    fn scalar(&mut self, t: yavs_sem::TypeId, elem: Elem, resolved: bool) -> (SigType, Conv) {
        let db = self.db;
        let td = &db.types[t];
        let tname = td.name.text.to_string();
        let sc = match &td.kind {
            TypeKind::Enum { lits } => {
                let e = self
                    .enums
                    .entry(t)
                    .or_insert_with(|| {
                        let lits: Vec<String> = lits
                            .iter()
                            .map(|&l| match db.decls[l].name {
                                Designator::Char(c) => format!("'{c}'"),
                                Designator::Ident(i) => i.text.to_string(),
                                Designator::Operator { text, .. } => text.to_string(),
                            })
                            .collect();
                        Arc::new(EnumType {
                            name: tname.clone(),
                            literals: lits,
                        })
                    })
                    .clone();
                let wk = &db.well_known;
                let conv = match elem {
                    Elem::Logic if Some(t) == wk.boolean => Conv::Bool,
                    Elem::Logic if Some(t) == wk.bit => Conv::Logic { base: 2 },
                    Elem::Logic => Conv::Logic { base: 0 },
                    Elem::Bytes(1) => Conv::Enum8,
                    Elem::Bytes(_) => Conv::Raw,
                };
                // A resolved std_ulogic is std_logic.
                let e = if resolved && matches!(conv, Conv::Logic { base: 0 }) {
                    Arc::new(EnumType {
                        name: "std_logic".into(),
                        literals: e.literals.clone(),
                    })
                } else {
                    e
                };
                return (SigType::Scalar(ScalarType::Enum(e)), conv);
            }
            TypeKind::Floating | TypeKind::UniversalReal => ScalarType::Real,
            TypeKind::Physical { .. } if Some(t) == db.well_known.time => ScalarType::Time,
            // ponytail: 64 bits for every integer and physical; the subtype's range is not in
            // the shape. Narrow it when viewers need the declared width.
            _ => ScalarType::Integer {
                name: tname,
                bits: 64,
            },
        };
        (SigType::Scalar(sc), Conv::Raw)
    }

    /// The type name of a logic vector as users write it: `std_logic_vector`, `unsigned`.
    fn vector_name(&self, t: yavs_sem::TypeId, resolved: bool) -> String {
        let n = self.db.types[t].name.text.to_string();
        let l = n.to_ascii_lowercase();
        match (resolved, l.as_str()) {
            (true, "std_ulogic_vector") => "std_logic_vector".into(),
            (true, _) if l.starts_with("unresolved_") => n["unresolved_".len()..].to_string(),
            _ => n,
        }
    }
}

fn bounds(d: yavs_elab::Bounds) -> Bounds {
    Bounds {
        left: d.left,
        right: d.right,
        dir: match d.dir {
            Direction::To => Dir::To,
            Direction::Downto => Dir::Downto,
        },
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
