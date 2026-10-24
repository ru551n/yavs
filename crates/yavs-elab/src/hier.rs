//! The elaborated hierarchy and the signal net graph (docs/design/ir.md section 4).
//!
//! Ids are assigned in elaboration order (deterministic depth-first), so they are stable for one
//! design, generics and configuration. A leaf is stored at [`LeafLoc`] (`NetGraph::loc`): a
//! kernel signal and the element its range starts at; kernel `ProcId`s equal process indices.
//!
//! **Element order inside a leaf.** Elements of a composite are flattened row major, leftmost
//! element first. A logic leaf (yavs-logic bit-planes) stores element 0 as the *rightmost*
//! element, so its kernel offsets are mirrored: row-major position `p` of a leaf of `n` elements
//! is kernel offset `n - 1 - p`. [`NetGraph::select`] is the one place that applies this; the IR
//! must agree with it when it computes offsets of non-static indices.

use std::collections::HashSet;

use yavs_kernel::{Elem, PortMode, Range, SignalType, Value};
use yavs_sem::hir::Direction;
use yavs_sem::{DeclId, StmtId, SubtypeId, TypeId, UnitId};
use yavs_syntax::Symbol;
use yavs_syntax::ast::Ident;

use crate::eval::Val;

macro_rules! id {
    ($($(#[$m:meta])* $name:ident),*) => {$(
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub u32);
        impl $name {
            pub fn index(self) -> usize {
                self.0 as usize
            }
        }
    )*};
}

id!(
    /// A scope of the elaborated hierarchy.
    ScopeId,
    /// A VHDL signal object (signal, port, component port, guard).
    ObjId,
    /// A scalar or an array of scalars of a signal object (a record field): stored at its
    /// [`LeafLoc`].
    LeafId,
    /// A process instance; equal to its `yavs_kernel::ProcId`.
    ProcIdx
);

#[derive(Clone, Debug, PartialEq)]
pub enum ScopeKind {
    /// An entity instance (the top level, or bound to an instance statement).
    Instance {
        entity: UnitId,
        arch: UnitId,
    },
    /// The component of a component instance: holds the component ports. Not part of
    /// `'path_name`/`'instance_name` (its bound entity instance carries the label).
    Component {
        comp: DeclId,
    },
    Block,
    /// One iteration of a `for` generate; `value` is the parameter.
    ForGenerate {
        param: DeclId,
        value: Val,
    },
    /// The chosen alternative of an `if` or `case` generate; `alt` is its label, if it has one.
    Generate {
        alt: Option<Symbol>,
    },
    Process {
        stmt: StmtId,
    },
    /// A library holding elaborated packages (a root scope, after the top level).
    Library,
    /// A package with signals (LRM 14.4.2): `:lib:pkg:s`.
    Package {
        unit: UnitId,
    },
}

pub struct Scope {
    pub kind: ScopeKind,
    /// The label, or the entity name of the top level; original spelling (rule 8).
    pub name: Ident,
    /// `0` in `g(0)`: the generate parameter's image.
    pub index: Option<String>,
    pub parent: Option<ScopeId>,
    pub children: Vec<ScopeId>,
    pub objs: Vec<ObjId>,
}

/// Scopes in elaboration order; [`ScopeId(0)`] is the top-level instance.
#[derive(Default)]
pub struct Hierarchy {
    pub scopes: Vec<Scope>,
}

/// Which spelling path strings use.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Case {
    /// LRM 16.2.5: basic identifiers in lower case (`'path_name`, `'instance_name`).
    Lrm,
    /// Original spelling (VHPI `vhpiCaseNameP`, waves, diagnostics).
    Original,
}

impl Hierarchy {
    pub fn scope(&self, s: ScopeId) -> &Scope {
        &self.scopes[s.index()]
    }

    pub(crate) fn add(&mut self, parent: Option<ScopeId>, kind: ScopeKind, name: Ident) -> ScopeId {
        let id = ScopeId(self.scopes.len() as u32);
        self.scopes.push(Scope {
            kind,
            name,
            index: None,
            parent,
            children: Vec::new(),
            objs: Vec::new(),
        });
        if let Some(p) = parent {
            self.scopes[p.index()].children.push(id);
        }
        id
    }

    fn segments(&self, s: ScopeId) -> Vec<ScopeId> {
        let mut v = Vec::new();
        let mut cur = Some(s);
        while let Some(c) = cur {
            if !matches!(self.scope(c).kind, ScopeKind::Component { .. }) {
                v.push(c);
            }
            cur = self.scope(c).parent;
        }
        v.reverse();
        v
    }

    fn label(&self, s: ScopeId, case: Case) -> String {
        let mut n = String::new();
        self.push_label(s, case, &mut n);
        n
    }

    /// The label of scope `s` appended to `out`: its name and `(index)`.
    fn push_label(&self, s: ScopeId, case: Case, out: &mut String) {
        let sc = self.scope(s);
        out.push_str(match case {
            Case::Lrm => sc.name.key.as_str(),
            Case::Original => sc.name.text.as_str(),
        });
        if let Some(i) = &sc.index {
            out.push('(');
            out.push_str(i);
            out.push(')');
        }
    }

    /// `label:` of each scope from the root to `s`, components left out, appended to `out`.
    fn push_path(&self, s: ScopeId, case: Case, out: &mut String) {
        let sc = self.scope(s);
        if let Some(p) = sc.parent {
            self.push_path(p, case, out);
        }
        if !matches!(sc.kind, ScopeKind::Component { .. }) {
            self.push_label(s, case, out);
            out.push(':');
        }
    }

    /// The instance scope a scope belongs to.
    pub fn instance_of(&self, mut s: ScopeId) -> ScopeId {
        loop {
            let sc = self.scope(s);
            match (&sc.kind, sc.parent) {
                (ScopeKind::Instance { .. }, _) | (_, None) => return s,
                (_, Some(p)) => s = p,
            }
        }
    }

    /// `'path_name` of a scope (LRM 16.2.5), ending in `:`: `:top:u1:g(0):`.
    pub fn path_name(&self, s: ScopeId, case: Case) -> String {
        let mut out = String::with_capacity(48);
        out.push(':');
        self.push_path(s, case, &mut out);
        out
    }

    /// `'instance_name` of a scope: `:top(arch):u1@leaf(a):g(0):`.
    pub fn instance_name(&self, s: ScopeId, db: &yavs_sem::Db, case: Case) -> String {
        let mut out = String::from(":");
        for (i, c) in self.segments(s).into_iter().enumerate() {
            match self.scope(c).kind {
                ScopeKind::Instance { entity, arch } => {
                    let (e, a) = (db.units[entity].name, db.units[arch].name);
                    if i > 0 {
                        out.push_str(&self.label(c, case));
                        out.push('@');
                    }
                    out.push_str(&format!("{}({})", spell(e, case), spell(a, case)));
                }
                _ => out.push_str(&self.label(c, case)),
            }
            out.push(':');
        }
        out
    }
}

pub(crate) fn spell(i: Ident, case: Case) -> String {
    match case {
        Case::Lrm => i.key.to_string(),
        Case::Original => i.text.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Shapes: the elaborated, fully constrained subtype of a signal.

/// Bounds of one index range, as positions (`i64`, enumeration positions for enum indexes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    pub left: i64,
    pub dir: Direction,
    pub right: i64,
}

impl Bounds {
    pub fn len(self) -> u64 {
        let (lo, hi) = self.low_high();
        if hi < lo {
            0
        } else {
            (hi as i128 - lo as i128 + 1) as u64
        }
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    pub fn low_high(self) -> (i64, i64) {
        match self.dir {
            Direction::To => (self.left, self.right),
            Direction::Downto => (self.right, self.left),
        }
    }

    /// Position of index `v` counted from the left, if it is in range.
    pub fn pos(self, v: i64) -> Option<u64> {
        let (lo, hi) = self.low_high();
        if v < lo || v > hi {
            return None;
        }
        Some(match self.dir {
            Direction::To => (v as i128 - self.left as i128) as u64,
            Direction::Downto => (self.left as i128 - v as i128) as u64,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// `left` is the subtype's `'left`, the default initial value; `res` its resolution
    /// function, if resolved. `sub` is the declared subtype and `range` its bounds (positions;
    /// `None` for reals and shapes only used for attribute prefixes).
    Scalar {
        ty: TypeId,
        sub: SubtypeId,
        range: Option<Bounds>,
        elem: Elem,
        left: Val,
        res: Option<DeclId>,
    },
    /// `res`: the resolution function of a composite subtype resolved as a whole (LRM 4.6).
    Array {
        ty: TypeId,
        dims: Vec<Bounds>,
        elem: Box<Shape>,
        res: Option<DeclId>,
    },
    Record {
        ty: TypeId,
        fields: Vec<(Ident, Shape)>,
        res: Option<DeclId>,
    },
}

impl Shape {
    /// Elements of all dimensions of an array, 1 otherwise.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> u64 {
        match self {
            Shape::Array { dims, .. } => dims.iter().map(|d| d.len()).product(),
            _ => 1,
        }
    }

    pub fn ty(&self) -> TypeId {
        match self {
            Shape::Scalar { ty, .. } | Shape::Array { ty, .. } | Shape::Record { ty, .. } => *ty,
        }
    }

    /// The leaves of this shape (ir.md 4): scalars and arrays of scalars are one leaf, a
    /// record one leaf per field, an array of records one leaf per field (struct of arrays).
    /// Each is `(field path, scalar shape, element count)`.
    pub fn leaves(&self) -> Vec<(Vec<u32>, &Shape, u64)> {
        let mut out = Vec::with_capacity(1);
        self.leaves_into(&mut Vec::new(), 1, &mut out);
        out
    }

    fn leaves_into<'a>(
        &'a self,
        path: &mut Vec<u32>,
        mult: u64,
        out: &mut Vec<(Vec<u32>, &'a Shape, u64)>,
    ) {
        match self {
            Shape::Scalar { .. } => out.push((path.clone(), self, mult)),
            Shape::Array { elem, .. } => elem.leaves_into(path, mult * self.len(), out),
            Shape::Record { fields, .. } => {
                for (i, (_, f)) in fields.iter().enumerate() {
                    path.push(i as u32);
                    f.leaves_into(path, mult, out);
                    path.pop();
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The net graph.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjKind {
    Signal,
    Port(PortMode),
    /// A port of a component (between the instance's actual and the entity's formal).
    ComponentPort(PortMode),
    /// `'stable`, `'quiet`, `'delayed`, `'transaction` of a signal (kernel-maintained, K2).
    Implicit,
}

/// The implicit signal attributes the kernel maintains (K2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImplicitKind {
    Stable(yavs_kernel::Time),
    Quiet(yavs_kernel::Time),
    Delayed(yavs_kernel::Time),
    Transaction,
}

pub struct SigObj {
    pub scope: ScopeId,
    pub decl: DeclId,
    pub name: Ident,
    pub kind: ObjKind,
    pub shape: Shape,
    /// Its leaves: `leaves.0 .. leaves.1`.
    pub leaves: (LeafId, LeafId),
    /// A port collapsed into part of its actual's leaf (`collapse.rs`): its one leaf is
    /// the actual's, and it is the elements of this kernel range of it. `None`: the whole
    /// leaves.
    pub part: Option<Range>,
}

#[derive(Clone, Debug)]
pub enum LeafRes {
    /// IEEE 1164 `resolved`.
    StdLogic,
    /// A user resolution function (needs the interpreter, M3).
    Func(DeclId),
    /// A leaf of a composite resolved as a whole by this function (LRM 4.6): the signal or a
    /// subelement of it whose subtype is resolved; its parts are resolved together (#193, a
    /// [`Whole`] of `NetGraph::wholes`).
    Whole(DeclId),
}

pub struct Leaf {
    pub obj: ObjId,
    /// Record field indices from the object's root to this leaf.
    pub path: Vec<u32>,
    pub ty: SignalType,
    pub resolution: Option<LeafRes>,
    /// `bus` or `register` (a guarded signal, LRM 6.4.2.3).
    pub guard: Option<yavs_kernel::GuardKind>,
    /// The disconnection time of a guarded signal (LRM 7.4), 0 without a specification.
    pub disconnect: yavs_kernel::Time,
    /// An implicit signal: its kind and the source elements.
    pub implicit: Option<(ImplicitKind, LeafId, Range)>,
    /// The other sources of `'stable`, `'quiet` or `'transaction` of a prefix over several
    /// leaves (a record).
    pub implicit_more: Vec<(LeafId, Range)>,
    pub init: Value,
}

/// An uncollapsed port association (rule 3): kernel `Builder::port`.
#[derive(Clone, Debug, PartialEq)]
pub struct PortEdge {
    pub formal: (LeafId, Range),
    pub actual: (LeafId, Range),
    pub mode: PortMode,
    /// Conversions (indices into `Elaborated::convs`): actual to formal, formal to actual.
    pub to_formal: Option<u32>,
    pub to_actual: Option<u32>,
}

/// A converted port association with several leaves on a side (a record port, an array of
/// composites): kernel `Builder::port_parts`, converted in the update phase like a
/// [`PortEdge`]. Parts are in leaf order, as `NetGraph::select` gives them.
#[derive(Clone, Debug, PartialEq)]
pub struct ConvPort {
    pub formal: Vec<(LeafId, Range)>,
    pub actual: Vec<(LeafId, Range)>,
    pub mode: PortMode,
    pub to_formal: Option<u32>,
    pub to_actual: Option<u32>,
}

/// A driver of a process: the elements of one leaf it drives.
#[derive(Clone, Debug, PartialEq)]
pub struct DriverDecl {
    pub proc: ProcIdx,
    pub leaf: LeafId,
    pub range: Range,
}

/// A composite resolved as a whole (LRM 4.6, 14.7.3.1: a basic signal): a signal whose
/// subtype has a resolution function, or a subelement of it that is the outermost resolved
/// composite on its path (one instance per element of the arrays above it).
pub struct Whole {
    pub func: DeclId,
    /// The resolved composite's subtype.
    pub shape: Shape,
    /// Per instance: its `(leaf, kernel range)` parts in leaf order (`NetGraph::select`).
    pub instances: Vec<Vec<(LeafId, Range)>>,
}

/// The most elements a pack of leaves holds: one plane block, so the kernel keeps its codes and
/// element stamps and reads, drives and finishes it with word operations (#686).
pub const PACK_MAX: u32 = 64;

/// Where a leaf is stored: elements `base .. base + len` of kernel signal `signal`
/// (docs/design/record-signal-layout.md, #686).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LeafLoc {
    pub signal: yavs_kernel::SignalId,
    pub base: u32,
}

#[derive(Default)]
pub struct NetGraph {
    pub objs: Vec<SigObj>,
    pub leaves: Vec<Leaf>,
    pub ports: Vec<PortEdge>,
    pub conv_ports: Vec<ConvPort>,
    pub drivers: Vec<DriverDecl>,
    pub wholes: Vec<Whole>,
    /// Per leaf, where it is stored (`NetGraph::place`).
    pub loc: Vec<LeafLoc>,
    /// Per kernel signal, its leaves `.0 .. .1` (consecutive, ascending bases).
    pub packs: Vec<(LeafId, LeafId)>,
}

/// One static selector of a name (sub-element map key).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sel {
    Field(u32),
    /// One value per dimension.
    Index(Vec<i64>),
    /// `left dir right` of a one-dimensional slice.
    Slice(Bounds),
}

impl NetGraph {
    pub fn obj(&self, o: ObjId) -> &SigObj {
        &self.objs[o.index()]
    }

    pub fn leaf(&self, l: LeafId) -> &Leaf {
        &self.leaves[l.index()]
    }

    /// Store the leaves in kernel signals (docs/design/record-signal-layout.md, #686): runs of
    /// consecutive leaves of one signal object that share every signal-wide kernel property
    /// form one kernel signal (a pack, at most [`PACK_MAX`] elements); every other leaf has a
    /// signal of its own. `forced`: the objects a force or release names (`None`: one names
    /// something untraceable, so no packs). Run once the leaves are final (after port
    /// collapsing).
    pub(crate) fn place(&mut self, packs: bool, forced: Option<&HashSet<ObjId>>) {
        let n = self.leaves.len();
        let ok = match (packs, forced) {
            (true, Some(f)) => self.packable(f),
            _ => vec![false; n],
        };
        self.loc = Vec::with_capacity(n);
        self.packs = Vec::new();
        let mut l = 0;
        while l < n {
            let start = l;
            let mut len = self.leaves[l].ty.len;
            l += 1;
            if ok[start] {
                while l < n
                    && ok[l]
                    && self.leaves[l].obj == self.leaves[start].obj
                    && len + self.leaves[l].ty.len <= PACK_MAX
                {
                    len += self.leaves[l].ty.len;
                    l += 1;
                }
            }
            let signal = yavs_kernel::SignalId(self.packs.len() as u32);
            let mut base = 0;
            for k in start..l {
                self.loc.push(LeafLoc { signal, base });
                base += self.leaves[k].ty.len;
            }
            self.packs.push((LeafId(start as u32), LeafId(l as u32)));
        }
    }

    /// Per leaf: whether it may share a kernel signal with its neighbours (section 3.2 of the
    /// design): logic elements, unresolved or resolved by `resolved` (one source each is the
    /// identity, and elaboration refused several on an unresolved leaf), at most `PACK_MAX`
    /// elements, and nothing that is signal-wide in the kernel: a guard, an implicit signal on
    /// it or being one, a force or release, a port, a composite resolved as a whole.
    fn packable(&self, forced: &HashSet<ObjId>) -> Vec<bool> {
        let mut ok: Vec<bool> = (self.leaves.iter())
            .map(|l| {
                l.ty.elem == Elem::Logic
                    && (1..=PACK_MAX).contains(&l.ty.len)
                    && matches!(l.resolution, None | Some(LeafRes::StdLogic))
                    && l.guard.is_none()
                    && l.implicit.is_none()
            })
            .collect();
        let mut no = |l: LeafId| ok[l.index()] = false;
        for l in &self.leaves {
            if let Some((_, src, _)) = l.implicit {
                no(src);
            }
            l.implicit_more.iter().for_each(|&(s, _)| no(s));
        }
        for e in &self.ports {
            no(e.formal.0);
            no(e.actual.0);
        }
        for p in &self.conv_ports {
            p.formal.iter().chain(&p.actual).for_each(|&(l, _)| no(l));
        }
        for w in &self.wholes {
            w.instances.iter().flatten().for_each(|&(l, _)| no(l));
        }
        for &o in forced {
            let (a, b) = self.obj(o).leaves;
            (a.0..b.0).for_each(|l| no(LeafId(l)));
        }
        ok
    }

    /// The kernel signal that stores leaf `l`.
    pub fn signal_of(&self, l: LeafId) -> yavs_kernel::SignalId {
        self.loc[l.index()].signal
    }

    /// The kernel reference of elements `r` of leaf `l` (leaf-relative offsets).
    pub fn kref(&self, l: LeafId, r: Range) -> yavs_kernel::SigRef {
        let (s, r) = self.kpart(l, r);
        s.slice(r.offset, r.len)
    }

    /// The kernel signal and range of elements `r` of leaf `l`.
    pub fn kpart(&self, l: LeafId, r: Range) -> (yavs_kernel::SignalId, Range) {
        let at = self.loc[l.index()];
        (at.signal, Range::new(at.base + r.offset, r.len))
    }

    /// The kernel signal and element of element `e` of leaf `l`.
    pub fn kelem(&self, l: LeafId, e: u32) -> (yavs_kernel::SignalId, u32) {
        let at = self.loc[l.index()];
        (at.signal, at.base + e)
    }

    /// The number of kernel signals.
    pub fn signal_count(&self) -> usize {
        self.packs.len()
    }

    /// The element count of kernel signal `s`.
    pub fn signal_len(&self, s: yavs_kernel::SignalId) -> u32 {
        let (a, b) = self.packs[s.0 as usize];
        (a.0..b.0).map(|l| self.leaves[l as usize].ty.len).sum()
    }

    /// The composite resolved as a whole that leaf `l` belongs to.
    pub fn whole_of(&self, l: LeafId) -> Option<&Whole> {
        // ponytail: linear in the composites resolved as a whole; index by leaf if many.
        (self.wholes.iter()).find(|w| {
            w.instances
                .first()
                .is_some_and(|i| i.iter().any(|p| p.0 == l))
        })
    }

    /// The sub-element map (rule 2): the `(leaf, kernel range)` pairs of `obj` selected by
    /// `sels`, in leaf order. Also returns the selected shape. Errors name the bad selector.
    pub fn select(
        &self,
        obj: ObjId,
        sels: &[Sel],
    ) -> Result<(Vec<(LeafId, Range)>, Shape), String> {
        let o = self.obj(obj);
        let mut shape = &o.shape;
        let mut path: Vec<u32> = Vec::new();
        // `count` consecutive instances of `shape`, starting at instance `offset`, among the
        // `outer` instances of `shape` in the object.
        let (mut offset, mut count, mut outer) = (0u64, 1u64, 1u64);
        // After a slice: the sliced array, its bounds and the offset of its first element.
        let mut window: Option<(TypeId, Bounds, u64)> = None;
        for s in sels {
            match (s, shape) {
                (Sel::Field(f), Shape::Record { fields, .. }) if window.is_none() => {
                    let (_, fs) = fields.get(*f as usize).ok_or("no such record element")?;
                    path.push(*f);
                    shape = fs;
                }
                (Sel::Index(ix), _) if window.is_some() => {
                    let (_, b, base) = window.take().expect("checked");
                    let [v] = ix.as_slice() else {
                        return Err("a slice has one dimension".into());
                    };
                    let p = b.pos(*v).ok_or_else(|| out_of(*v, b))?;
                    let (lo, hi) = (offset - base, offset - base + count);
                    if p < lo || p >= hi {
                        return Err(format!("index {v} is outside the slice"));
                    }
                    offset = base + p;
                    count = 1;
                }
                (Sel::Index(ix), Shape::Array { dims, elem, .. }) => {
                    if ix.len() != dims.len() {
                        return Err(format!(
                            "{} indices for {} dimensions",
                            ix.len(),
                            dims.len()
                        ));
                    }
                    let mut lin = 0u64;
                    for (v, d) in ix.iter().zip(dims) {
                        lin = lin * d.len() + d.pos(*v).ok_or_else(|| out_of(*v, *d))?;
                    }
                    let n = shape.len();
                    offset = offset * n + lin;
                    outer *= n;
                    shape = elem;
                }
                (Sel::Slice(r), _) if window.is_some() => {
                    let (_, b, base) = window.expect("checked");
                    let (lo, hi) = (offset - base, offset - base + count);
                    if r.is_empty() {
                        count = 0;
                        continue;
                    }
                    let (a, z) = slice_pos(*r, b)?;
                    if a < lo || z >= hi {
                        return Err("slice outside the sliced range".into());
                    }
                    offset = base + a;
                    count = z - a + 1;
                }
                (Sel::Slice(r), Shape::Array { ty, dims, elem, .. }) if dims.len() == 1 => {
                    let b = dims[0];
                    let n = b.len();
                    let base = offset * n;
                    if r.is_empty() {
                        (offset, count) = (base, 0);
                    } else {
                        let (a, z) = slice_pos(*r, b)?;
                        (offset, count) = (base + a, z - a + 1);
                    }
                    outer *= n;
                    window = Some((*ty, b, base));
                    shape = elem;
                }
                // That element of every element of an array of records: a driver of a mode
                // view signal parameter of an array (`view (v) of arr_t`, sem
                // `view_driven_paths`). The shape returned is the element's, not an array.
                (Sel::Field(f), _) if window.is_some() || matches!(shape, Shape::Array { .. }) => {
                    if window.take().is_none() {
                        while let Shape::Array { elem, .. } = shape {
                            let n = shape.len();
                            (offset, count, outer) = (offset * n, count * n, outer * n);
                            shape = elem;
                        }
                    }
                    let Shape::Record { fields, .. } = shape else {
                        return Err("selector does not apply to this subtype".into());
                    };
                    let (_, fs) = fields.get(*f as usize).ok_or("no such record element")?;
                    path.push(*f);
                    shape = fs;
                }
                _ => return Err("selector does not apply to this subtype".into()),
            }
        }
        let mut out = Vec::with_capacity(o.leaves.1.0 as usize - o.leaves.0.0 as usize);
        for l in o.leaves.0.0..o.leaves.1.0 {
            let leaf = &self.leaves[l as usize];
            if !leaf.path.starts_with(&path) {
                continue;
            }
            let n = u64::from(leaf.ty.len);
            let k = n / outer;
            let (mut a, len) = (offset * k, count * k);
            if leaf.ty.elem == Elem::Logic {
                a = n - a - len;
            }
            out.push((LeafId(l), Range::new(a as u32, len as u32)));
        }
        let shape = match window {
            // A slice keeps its array subtype, with the slice's bounds.
            Some((ty, b, base)) => {
                let left = index_at(b, offset - base);
                let right = match (count, b.dir) {
                    (0, Direction::To) => left - 1,
                    (0, Direction::Downto) => left + 1,
                    _ => index_at(b, offset - base + count - 1),
                };
                Shape::Array {
                    ty,
                    dims: vec![Bounds {
                        left,
                        dir: b.dir,
                        right,
                    }],
                    elem: Box::new(shape.clone()),
                    res: None,
                }
            }
            None => shape.clone(),
        };
        Ok((out, shape))
    }
}

fn out_of(v: i64, b: Bounds) -> String {
    let d = match b.dir {
        Direction::To => "to",
        Direction::Downto => "downto",
    };
    format!("index {v} is outside {} {d} {}", b.left, b.right)
}

/// Positions (from the left) of the first and last element of slice `r` of an array `b`.
fn slice_pos(r: Bounds, b: Bounds) -> Result<(u64, u64), String> {
    if r.dir != b.dir {
        return Err("slice direction differs from the array's".into());
    }
    let a = b.pos(r.left).ok_or_else(|| out_of(r.left, b))?;
    let z = b.pos(r.right).ok_or_else(|| out_of(r.right, b))?;
    Ok((a, z))
}

pub(crate) fn index_at(b: Bounds, p: u64) -> i64 {
    match b.dir {
        Direction::To => b.left + p as i64,
        Direction::Downto => b.left - p as i64,
    }
}
