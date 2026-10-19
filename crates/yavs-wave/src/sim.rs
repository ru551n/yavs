//! The kernel side (M7, #129): the elaborated design as a [`Hierarchy`] and a [`Recorder`]
//! that subscribes to the kernel's value-change bus.
//!
//! Every recorded value is one sub-element of one kernel leaf (rule 11): a scalar, or a
//! one-dimensional array of `std_ulogic`/`bit` written as one vector. Records are scopes, arrays
//! of anything else are array scopes with one entry per element. The recorder marks the
//! entries a change touches and writes their final value once per time step (FST has no
//! deltas), in entry order, so the file does not depend on `--threads`.
//!
//! Variables (`--wave-variables`: process variables and non-protected shared variables) are
//! not on the bus: each is read once per time step, at its end, when every process is
//! suspended (`Hook::with_process`, `yavs_interp::shared_var`), and a sub-element is written
//! when it differs from the last written value. Their shapes are the elaborated ones
//! (`Elaborated::var_shapes`), so records and arrays are scopes as for signals.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use yavs_elab::{Built, Elaborated, ObjKind, ScopeId, Shape, Val};
use yavs_interp::InterpProcess;
use yavs_kernel::{
    Elem, Hook, Observer, Phase, PortMode, ProcId, SigRef, SimControl, SimError, Value,
};
use yavs_sem::Db;
use yavs_sem::decl::Designator;
use yavs_sem::decl::{DeclKind, ObjClass as SemClass};
use yavs_sem::hir::{Direction, StmtKind};
use yavs_sem::stat::Elems;
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

enum Src {
    /// Kernel leaf, offset and element count.
    Sig { leaf: u32, offset: u32, len: u32 },
    /// Sub-element `path` of a variable (`Var::entries`): per step an array element (row
    /// major) or a record field; a logic vector is the whole array at the end of the path.
    Var { path: Box<[u64]> },
}

/// Where a variable is read.
enum VarAt {
    /// Variable `name` of kernel process `proc`.
    Proc { proc: ProcId, name: String },
    /// The slot and IR type of a shared variable.
    Shared { slot: u64, ty: yavs_ir::TyId },
}

/// A recorded variable, the run of entries of its sub-elements and the value last written
/// to each (empty: none yet).
struct Var {
    at: VarAt,
    entries: std::ops::Range<u32>,
    last: Vec<Vec<u8>>,
}

struct Entry {
    src: Src,
    conv: Conv,
}

struct State {
    writer: Option<Box<dyn Sink>>,
    entries: Vec<Entry>,
    /// Per kernel leaf: its entries, by offset (disjoint).
    by_leaf: Vec<Vec<u32>>,
    dirty: Vec<bool>,
    pending: Vec<u32>,
    /// The time of the pending changes, fs.
    at: u64,
    vars: Vec<Var>,
    /// The shared state, for shared variables.
    built: Option<Built>,
    /// The time step in progress, fs (variables are written at its end).
    step: u64,
    error: Option<Error>,
}

/// The waveform subscriber. Clone it into [`SimControl::subscribe`] and keep one handle for
/// [`Recorder::finish`].
#[derive(Clone)]
pub struct Recorder(Arc<Mutex<State>>);

/// Where a [`Recorder`] puts the values: the FST [`WaveWriter`], or another consumer (the DAP
/// server's live stream). Values follow the byte layouts of the crate docs.
pub trait Sink: Send {
    /// Times do not decrease.
    fn change(&mut self, time: u64, id: SignalId, value: &[u8]) -> Result<(), Error>;
    /// Every change of the time step at `time` has been given.
    fn step(&mut self, _time: u64) {}
    fn finish(self: Box<Self>) -> Result<(), Error>;
}

impl Sink for WaveWriter {
    fn change(&mut self, time: u64, id: SignalId, value: &[u8]) -> Result<(), Error> {
        WaveWriter::change(self, time, id, value)
    }

    fn finish(self: Box<Self>) -> Result<(), Error> {
        WaveWriter::finish(*self)
    }
}

/// Build the hierarchy of `e` under `sel`, create the FST at `path` and return the recorder
/// and the arrays left out by the size cap.
pub fn record(
    db: &Db,
    e: &Elaborated,
    path: &Path,
    sel: Selection,
    cfg: WaveConfig,
    built: Option<&Built>,
) -> Result<(Recorder, Vec<Skipped>), Error> {
    record_to(db, e, sel, built, |h| {
        Ok(Box::new(WaveWriter::create(path, h, cfg)?))
    })
    .map(|(r, h)| (r, h.skipped))
}

/// [`record`] into the sink `sink` makes for the hierarchy; returns the recorder and the
/// hierarchy.
pub fn record_to(
    db: &Db,
    e: &Elaborated,
    sel: Selection,
    built: Option<&Built>,
    sink: impl FnOnce(&Hierarchy) -> Result<Box<dyn Sink>, Error>,
) -> Result<(Recorder, Hierarchy), Error> {
    let mut d = Design {
        db,
        e,
        built,
        b: HierarchyBuilder::new(sel),
        entries: Vec::new(),
        enums: HashMap::new(),
        vars: Vec::new(),
        pkg_vars: BTreeMap::new(),
    };
    d.package_vars();
    for (i, s) in e.hier.scopes.iter().enumerate() {
        if s.parent.is_none() {
            d.scope(ScopeId(i as u32));
        }
    }
    // Packages whose library has no scope in the design.
    let libs: Vec<_> = d.pkg_vars.keys().map(|&u| db.units[u].lib).collect();
    for lib in libs {
        if d.pkg_vars.keys().any(|&u| db.units[u].lib == lib) {
            d.b.scope(db.libs[lib].name.text.as_str(), ScopeKind::Block, "");
            d.packages_of(lib);
            d.b.up();
        }
    }
    let Design {
        b, entries, vars, ..
    } = d;
    let h = b.finish();
    let writer = sink(&h)?;
    let mut by_leaf = vec![Vec::new(); e.nets.leaves.len()];
    for (i, en) in entries.iter().enumerate() {
        if let Src::Sig { leaf, .. } = &en.src {
            by_leaf[*leaf as usize].push(i as u32);
        }
    }
    for v in &mut by_leaf {
        v.sort_by_key(|&i| entries[i as usize].span().0);
    }
    let n = entries.len();
    let st = State {
        writer: Some(writer),
        entries,
        by_leaf,
        dirty: vec![false; n],
        pending: Vec::new(),
        at: 0,
        vars,
        built: built.cloned(),
        step: 0,
        error: None,
    };
    Ok((Recorder(Arc::new(Mutex::new(st))), h))
}

impl Recorder {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Record every signal's value from `now` (fs) on, for a recorder subscribed after the
    /// start of the simulation (`Phase::StartOfSim` does this otherwise).
    pub fn start(&self, now: u64) {
        self.lock().start(now);
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

impl Entry {
    /// Kernel offset and length of a signal entry.
    fn span(&self) -> (u32, u32) {
        match self.src {
            Src::Sig { offset, len, .. } => (offset, len),
            Src::Var { .. } => (0, 0),
        }
    }
}

impl State {
    fn start(&mut self, now: u64) {
        self.at = now;
        self.step = now;
        for i in 0..self.entries.len() as u32 {
            if matches!(self.entries[i as usize].src, Src::Sig { .. }) {
                self.mark(i);
            }
        }
    }

    fn step_done(&mut self) {
        if let (None, Some(w)) = (&self.error, &mut self.writer) {
            w.step(self.step);
        }
    }

    /// Write the variable sub-elements that changed since the last sample, at the time of the
    /// step that just ended (every process is suspended).
    fn sample_vars(&mut self, hook: &mut Hook<'_>) {
        let mut bytes = Vec::new();
        for k in 0..self.vars.len() {
            if self.error.is_some() {
                return;
            }
            let v = match self.read_var(hook, k) {
                Ok(Some(v)) => v,
                // The process has not started yet, or has ended: its last values stay.
                Ok(None) => continue,
                Err(m) => {
                    self.error = Some(Error::Value(m));
                    return;
                }
            };
            let var = &mut self.vars[k];
            for (i, last) in var.entries.clone().zip(&mut var.last) {
                let en = &self.entries[i as usize];
                let x = match &en.src {
                    Src::Var { path } => sub(&v, path),
                    Src::Sig { .. } => None,
                };
                let Some(x) = x else {
                    let m = "this is a bug in yavs: a recorded variable's value has another shape";
                    self.error = Some(Error::Value(m.into()));
                    return;
                };
                bytes.clear();
                convert_val(en.conv, &x, &mut bytes);
                if *last == bytes {
                    continue;
                }
                last.clone_from(&bytes);
                let w = self.writer.as_mut().expect("writer open while recording");
                if let Err(e) = w.change(self.step, SignalId(i), &bytes) {
                    self.error = Some(e);
                    return;
                }
            }
        }
    }

    /// The value of variable `k`; `None` while its process has no frame.
    fn read_var(&self, hook: &mut Hook<'_>, k: usize) -> Result<Option<Val>, String> {
        let bug =
            |what: &str| format!("this is a bug in yavs: reading a recorded variable: {what}");
        match &self.vars[k].at {
            VarAt::Proc { proc, name } => hook
                .with_process(*proc, |p| {
                    let p = p
                        .as_any()
                        .and_then(|a| a.downcast_mut::<InterpProcess>())
                        .ok_or_else(|| bug("not an interpreter process"))?;
                    if !p.has_vars() {
                        return Ok(None);
                    }
                    let (ty, cv) = p.var(name).map_err(|m| bug(&m))?;
                    yavs_elab::ir_decode(p.program(), ty, &cv)
                        .map(Some)
                        .ok_or_else(|| bug("value not decoded"))
                })
                .unwrap_or_else(|| Err(bug("no process"))),
            VarAt::Shared { slot, ty } => {
                let b = self.built.as_ref().ok_or_else(|| bug("no shared state"))?;
                let cv =
                    yavs_interp::shared_var(&b.rt, &b.shared, *slot, *ty).map_err(|m| bug(&m))?;
                yavs_elab::ir_decode(&b.rt.prog, *ty, &cv)
                    .map(Some)
                    .ok_or_else(|| bug("value not decoded"))
            }
        }
    }

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
            let Src::Sig { leaf, offset, len } = en.src else {
                continue;
            };
            let r = yavs_kernel::SignalId(leaf).slice(offset, len);
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

/// A variable's value in the byte layout of its [`SigType`]: a logic enumeration as one byte
/// (`Conv::Logic`, also per element of a vector), other enumerations as `u32`, integers and
/// physicals as `i64`, reals as `f64`.
fn convert_val(conv: Conv, v: &Val, out: &mut Vec<u8>) {
    match (conv, v) {
        (Conv::Logic { .. }, Val::Enum(p)) => out.push(*p as u8),
        (Conv::Logic { .. }, Val::Array(a)) => match &a.elems {
            Elems::Bytes(b) => out.extend_from_slice(b),
            Elems::Runs(rs) => {
                for (n, x) in rs.iter() {
                    if let Val::Enum(p) = x {
                        out.extend(std::iter::repeat_n(*p as u8, *n as usize));
                    }
                }
            }
        },
        (_, Val::Enum(p)) => out.extend_from_slice(&p.to_le_bytes()),
        (_, Val::Int(x) | Val::Phys(x)) => out.extend_from_slice(&x.to_le_bytes()),
        (_, Val::Real(x)) => out.extend_from_slice(&x.to_le_bytes()),
        // A mismatch reaches the writer as a wrong length, which it reports.
        _ => {}
    }
}

/// Sub-element `path` of `v` (see `Src::Var`).
fn sub<'v>(v: &'v Val, path: &[u64]) -> Option<Cow<'v, Val>> {
    let mut cur = Cow::Borrowed(v);
    for &k in path {
        cur = match cur {
            Cow::Borrowed(Val::Record(fs)) => Cow::Borrowed(fs.get(k as usize)?),
            Cow::Borrowed(Val::Array(a)) => match &a.elems {
                Elems::Bytes(b) => Cow::Owned(Val::Enum(u32::from(*b.get(k as usize)?))),
                Elems::Runs(rs) => {
                    let mut at = k;
                    let (_, x) = rs.iter().find(|(n, _)| {
                        let hit = at < *n;
                        if !hit {
                            at -= n;
                        }
                        hit
                    })?;
                    Cow::Borrowed(x)
                }
            },
            // Elements of an element are borrowed from `v`; an owned one is a scalar.
            _ => return None,
        };
    }
    Some(cur)
}

impl Observer for Recorder {
    fn phase(&mut self, hook: &mut Hook<'_>, phase: Phase) {
        let mut st = self.lock();
        match phase {
            Phase::StartOfSim => st.start(hook.now().as_fs()),
            Phase::NextTimeStep => {
                // The step in progress is complete: its signals, then its variables.
                st.flush(|r| hook.value(r));
                st.sample_vars(hook);
                st.step_done();
                st.step = hook.now().as_fs();
            }
            Phase::EndOfTimeStep => st.flush(|r| hook.value(r)),
            Phase::EndOfSim => {
                st.flush(|r| hook.value(r));
                st.sample_vars(hook);
                st.step_done();
            }
            Phase::EndOfInit
            | Phase::NextCycle
            | Phase::StartOfProcesses
            | Phase::EndOfProcesses
            | Phase::LastKnownDelta
            | Phase::StartOfPostponed => {}
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
            en.span().0 + en.span().1 <= lo
        });
        let hit: Vec<u32> = ids[first..]
            .iter()
            .copied()
            .take_while(|&i| st.entries[i as usize].span().0 < hi)
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
    /// `Some` with `--wave-variables`.
    built: Option<&'a Built>,
    vars: Vec<Var>,
    /// The recorded shared variables of each package not yet declared, with their slots.
    pkg_vars: BTreeMap<yavs_sem::UnitId, Vec<(yavs_sem::DeclId, u64, yavs_ir::TyId)>>,
}

/// Where a node's values come from.
#[derive(Clone)]
enum At {
    /// Row-major instance `k` of the node in the leaves from `leaf` on.
    Sig { leaf: u32, k: u64 },
    /// Sub-element `path` of a variable (`Src::Var`).
    Var { path: Vec<u64> },
}

impl At {
    /// Element `j` of an array node of `n` elements.
    fn elem(&self, n: u64, j: u64) -> At {
        match self {
            At::Sig { leaf, k } => At::Sig {
                leaf: *leaf,
                k: k * n + j,
            },
            At::Var { path } => At::Var {
                path: [path.as_slice(), &[j]].concat(),
            },
        }
    }

    /// Field `i` of a record node, whose leaves start `skip` leaves after its own.
    fn field(&self, i: u64, skip: u32) -> At {
        match self {
            At::Sig { leaf, k } => At::Sig {
                leaf: leaf + skip,
                k: *k,
            },
            At::Var { path } => At::Var {
                path: [path.as_slice(), &[i]].concat(),
            },
        }
    }
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
                yavs_elab::ScopeKind::Generate { .. } => (ScopeKind::Generate, String::new()),
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
            // The label of the chosen alternative stays in the name: `g.alt1`.
            if let yavs_elab::ScopeKind::Generate { alt: Some(a) } = &sc.kind {
                name = format!("{name}.{a}");
            }
            self.b.scope(&name, kind, &comp);
            for &o in &sc.objs {
                self.object(o);
            }
            match &sc.kind {
                yavs_elab::ScopeKind::Process { stmt } => self.variables(id, *stmt),
                yavs_elab::ScopeKind::Package { unit } => {
                    for (d, slot, ty) in self.pkg_vars.remove(unit).unwrap_or_default() {
                        self.variable(d, VarAt::Shared { slot, ty }, None);
                    }
                }
                _ => {}
            }
            self.shared_vars(id);
        }
        for &c in &sc.children {
            self.scope(c);
        }
        if let yavs_elab::ScopeKind::Library = sc.kind {
            let lib = self
                .db
                .units
                .iter()
                .map(|(_, u)| u.lib)
                .find(|&l| self.db.libs[l].name.text == sc.name.text);
            if let Some(lib) = lib {
                self.packages_of(lib);
            }
        }
        if !transparent {
            self.b.up();
        }
    }

    /// The recorded shared variables of packages, by package, before the walk (none without
    /// `--wave-variables`).
    fn package_vars(&mut self) {
        let Some(b) = self.built else { return };
        let db = self.db;
        let mut unit_of = HashMap::new();
        for (u, _) in db.units.iter() {
            for r in db.package_regions(u).unwrap_or_default() {
                unit_of.insert(r, u);
            }
        }
        let mut vs: Vec<_> = b
            .vars
            .iter()
            .filter(|((r, _), _)| r.is_none())
            .map(|(&(_, d), &(slot, ty))| (d, slot, ty))
            .collect();
        vs.sort_by_key(|v| v.0);
        for v in vs {
            // ponytail: a variable of a package instance nested in a package is left out;
            // walk the instance regions too if one is ever wanted.
            if let Some(&u) = unit_of.get(&db.decls[v.0].scope) {
                self.pkg_vars.entry(u).or_default().push(v);
            }
        }
    }

    /// Scopes for the packages of library `lib` with recorded shared variables that the
    /// design has no scope for (no signal of theirs is elaborated).
    fn packages_of(&mut self, lib: yavs_sem::LibId) {
        let us: Vec<_> = self
            .pkg_vars
            .keys()
            .copied()
            .filter(|&u| self.db.units[u].lib == lib)
            .collect();
        for u in us {
            let name = self.db.units[u].name.text.to_string();
            self.b.scope(&name, ScopeKind::Package, &name);
            for (d, slot, ty) in self.pkg_vars.remove(&u).unwrap_or_default() {
                self.variable(d, VarAt::Shared { slot, ty }, None);
            }
            self.b.up();
        }
    }

    /// The recorded shared variables of region `id`, in declaration order.
    fn shared_vars(&mut self, id: ScopeId) {
        let Some(b) = self.built else { return };
        let mut vs: Vec<_> = b
            .vars
            .iter()
            .filter(|((r, _), _)| *r == Some(id))
            .map(|(&(_, d), &(slot, ty))| (d, slot, ty))
            .collect();
        vs.sort_by_key(|v| v.0);
        for (d, slot, ty) in vs {
            self.variable(d, VarAt::Shared { slot, ty }, Some(id));
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
        let at = At::Sig {
            leaf: obj.leaves.0.0,
            k: 0,
        };
        self.node(
            &obj.shape,
            obj.name.text.as_str(),
            at,
            port,
            ObjClass::Signal,
        );
    }

    /// `shape` named `name`, its values from `at`.
    fn node(&mut self, shape: &Shape, name: &str, at: At, port: PortDir, class: ObjClass) {
        match shape {
            Shape::Scalar {
                ty,
                sub,
                range,
                elem,
                res,
                ..
            } => {
                let (sty, conv) = self.scalar(*ty, *elem, res.is_some(), Some((*sub, *range)));
                self.signal(name, sty, port, class, at, 1, conv);
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
                        self.scalar(*et, Elem::Logic, res.is_some() || whole.is_some(), None)
                    else {
                        unreachable!("logic scalars are enums")
                    };
                    let sty = SigType::Vector {
                        elem: e,
                        bounds: bounds(*d),
                        type_name: Some(self.vector_name(*ty, res.is_some() || whole.is_some())),
                    };
                    self.signal(name, sty, port, class, at, d.len(), conv);
                    return;
                }
                let n = shape.len();
                if n == 0 {
                    return;
                }
                if self.b.array(name, n) {
                    for j in 0..n {
                        let child = format!("{name}({})", index_image(dims, j));
                        self.node(elem, &child, at.elem(n, j), port, class);
                    }
                }
                self.b.up();
            }
            Shape::Record { fields, .. } => {
                self.b.scope(name, ScopeKind::Record, "");
                let mut skip = 0;
                for (i, (fname, f)) in fields.iter().enumerate() {
                    self.node(
                        f,
                        fname.text.as_str(),
                        at.field(i as u64, skip),
                        port,
                        class,
                    );
                    skip += f.leaves().len() as u32;
                }
                self.b.up();
            }
        }
    }

    /// Declare signal `name`: for a signal over row-major elements `k*len .. (k+1)*len` of
    /// `leaf`. It gets an entry only if the selection keeps it, so unrecorded signals cost
    /// nothing at run time.
    #[allow(clippy::too_many_arguments)]
    fn signal(
        &mut self,
        name: &str,
        ty: SigType,
        port: PortDir,
        class: ObjClass,
        at: At,
        len: u64,
        conv: Conv,
    ) {
        let id = SignalId(self.entries.len() as u32);
        if !self.b.signal(name, id, ty, port, class) {
            return;
        }
        let src = match at {
            At::Sig { leaf, k } => {
                let n = self.e.nets.leaves[leaf as usize].ty.len as u64;
                let first = k * len;
                let offset = match conv {
                    Conv::Logic { .. } | Conv::Bool => n - first - len,
                    _ => first,
                };
                Src::Sig {
                    leaf,
                    offset: offset as u32,
                    len: len as u32,
                }
            }
            At::Var { path } => Src::Var { path: path.into() },
        };
        self.entries.push(Entry { src, conv });
    }

    /// The variables of process `scope` (statement `stmt`).
    fn variables(&mut self, scope: ScopeId, stmt: yavs_sem::StmtId) {
        let (db, e) = (self.db, self.e);
        if self.built.is_none() {
            return;
        }
        let StmtKind::Process { scope: region, .. } = &db.stmts[stmt].kind else {
            return;
        };
        let Some(proc) = e
            .procs
            .iter()
            .position(|p| p.scope == scope && p.actual.is_none())
        else {
            return;
        };
        let parent = e.hier.scope(scope).parent;
        for &d in &db.scopes[*region].order {
            if matches!(&db.decls[d].kind, DeclKind::Object(o) if o.class == SemClass::Variable) {
                let at = VarAt::Proc {
                    proc: ProcId(proc as u32),
                    name: db.decls[d].name.to_string(),
                };
                self.variable(d, at, parent);
            }
        }
    }

    /// Variable `d` of region `region` (`None`: a package's), read at `at`, with the
    /// sub-elements the selection keeps.
    fn variable(&mut self, d: yavs_sem::DeclId, at: VarAt, region: Option<ScopeId>) {
        let name = self.db.decls[d].name.to_string();
        let Some(shape) = self.e.var_shapes.get(&(region, d)) else {
            // Not a scalar or composite (an access, a protected object), or its bounds are
            // known only when the simulation runs.
            if let DeclKind::Object(o) = &self.db.decls[d].kind {
                let k = &self.db.types[self.db.base(o.subtype)].kind;
                if k.is_scalar() || matches!(k, TypeKind::Array { .. } | TypeKind::Record { .. }) {
                    self.b.unknown(&name);
                }
            }
            return;
        };
        let first = self.entries.len() as u32;
        let node = At::Var { path: Vec::new() };
        self.node(shape, &name, node, PortDir::None, ObjClass::Variable);
        let last = self.entries.len() as u32;
        if last > first {
            self.vars.push(Var {
                at,
                entries: first..last,
                last: vec![Vec::new(); (last - first) as usize],
            });
        }
    }

    /// `decl`: the declared subtype and its bounds, which name and size integers.
    fn scalar(
        &mut self,
        t: yavs_sem::TypeId,
        elem: Elem,
        resolved: bool,
        decl: Option<(yavs_sem::SubtypeId, Option<yavs_elab::Bounds>)>,
    ) -> (SigType, Conv) {
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
            // The declared subtype's name and the width its range needs; 64 bits and the base
            // type name when there is neither (an anonymous subtype keeps the base name).
            _ => ScalarType::Integer {
                name: decl
                    .and_then(|(st, _)| subtype_name(db, st))
                    .unwrap_or(tname),
                bits: decl.and_then(|(_, r)| r).map_or(64, int_bits),
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

/// The name of `st`, else of the nearest named subtype it was derived from (`natural range
/// 0 to 15` is a `natural`).
fn subtype_name(db: &Db, mut st: yavs_sem::SubtypeId) -> Option<String> {
    loop {
        if let Some(n) = db.subtypes[st].name {
            return Some(n.text.to_string());
        }
        st = db.subtypes[st].parent?;
    }
}

/// The width of the two's complement encoding that holds every value of `b`.
fn int_bits(b: yavs_elab::Bounds) -> u32 {
    let (lo, hi) = b.low_high();
    if hi < lo {
        return 1;
    }
    if lo >= 0 {
        return (64 - hi.leading_zeros()).max(1);
    }
    // `!lo` is -lo - 1, the largest magnitude a negative value needs.
    let mag = (!lo as u64).max(hi.max(0) as u64);
    65 - mag.leading_zeros()
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
