//! The design hierarchy as VHPI sees it (#269): an owned, read-only snapshot of the elaborated
//! design, built once before plugins run. It holds no simulation state; the layers that read
//! and write values (#270, #271) find the kernel objects through [`Origin`].
//!
//! Nodes are regions (root and component instances, blocks, generate iterations, processes,
//! package instances), declarations (generics, ports, signals, constants, variables) and the
//! design units instances name. Sub-elements of composite objects (record fields, array
//! elements) are not nodes: they are paths into a declaration's [`Layout`], made on demand,
//! so a large memory costs nothing until a plugin walks it.
//!
//! Names keep their original spelling (design rule 8): [`Node::name`] is `vhpiCaseNameP`,
//! [`vhpi_name`] derives `vhpiNameP` (basic identifiers upper case, extended ones as written).

// VHPI names are matched as written in the C standard.
#![allow(non_upper_case_globals)]

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use yavs_diag::Span;
use yavs_elab::{Bounds, Elaborated, ObjId, ScopeId, ScopeKind, Shape};
use yavs_sem::decl::Designator;
use yavs_sem::decl::{DeclKind, ObjClass, ObjRole};
use yavs_sem::hir::{Direction, Mode, StmtKind, UnitHir};
use yavs_sem::loader::UnitKind;
use yavs_sem::stat::{ConstValue, Elems};
use yavs_sem::types::{Constraint, RangeC, TypeKind};
use yavs_sem::{AttrValueId, Db, DeclId, SubtypeId, TypeId, UnitId};

use crate::sys::*;
use crate::types::{Bound, Def, Range, Ty, TyId, Types};
use crate::value::{Lit, Scalar};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct NodeId(pub(crate) u32);

/// The root instance is always the first node.
pub(crate) const ROOT: NodeId = NodeId(0);

/// What a node is in the elaborated design.
#[derive(Clone, Debug)]
#[expect(
    dead_code,
    reason = "the elaborated ids are for value puts and value-change callbacks"
)]
pub(crate) enum Origin {
    /// A region: an instance, block, generate iteration or process.
    Scope(ScopeId),
    /// A signal or port: its elaborated object and its first kernel leaf (its leaves are
    /// consecutive, one per scalar of the layout's record structure).
    /// `parts`: a port collapsed into parts of its actual's leaves: per leaf, the kernel
    /// elements it is (yavs-elab `SigObj::parts`; empty for whole leaves).
    Signal {
        obj: ObjId,
        leaf: u32,
        parts: Vec<yavs_kernel::Range>,
    },
    /// A generic, constant or variable, declared in elaborated region `scope` (a package's
    /// objects have none), with its value when elaboration knows it (generics, constants).
    Decl {
        scope: Option<ScopeId>,
        decl: DeclId,
        value: Option<ConstValue>,
    },
    /// A variable (#309): of the process that is kernel process `proc`, or (`None`) a shared
    /// variable declared in elaborated region `scope` (a package's has none).
    Var {
        proc: Option<u32>,
        scope: Option<ScopeId>,
        decl: DeclId,
    },
    /// A package instance or a design unit.
    Unit(UnitId),
    /// The root of a design given by name only (`set_design_root`).
    Named,
}

pub(crate) struct Node {
    /// A `vhpi*K` class kind.
    pub(crate) kind: vhpiClassKindT,
    /// Original spelling (`vhpiCaseNameP`); `label(index)` for a generate iteration,
    /// `lib:pkg` for a package instance.
    pub(crate) name: String,
    /// `vhpiFullCaseNameP`.
    pub(crate) full: String,
    /// The enclosing region (`vhpiUpperRegion`, `vhpiImmRegion`).
    pub(crate) parent: Option<NodeId>,
    pub(crate) file: Option<String>,
    pub(crate) line: Option<i32>,
    /// Internal regions, in elaboration order.
    pub(crate) regions: Vec<NodeId>,
    /// Declarations, in declaration order.
    pub(crate) decls: Vec<NodeId>,
    /// `vhpiDesignUnit` of an instance or package, `vhpiPrimaryUnit` of an architecture.
    pub(crate) unit: Option<NodeId>,
    /// `vhpiDefNameP` of an instance: `LIB.ENTITY(ARCH)`.
    pub(crate) def_name: Option<String>,
    pub(crate) mode: Option<vhpiModeT>,
    /// The sub-element structure of an object; `None` for regions and design units.
    pub(crate) layout: Option<Layout>,
    /// An object's type (`vhpiType`), in `Design::types`.
    pub(crate) ty: Option<TyId>,
    pub(crate) origin: Origin,
}

/// The sub-element structure of an object (records and arrays are native, rule 9).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Layout {
    /// A scalar of this type.
    Scalar(Scalar),
    Array {
        dims: Vec<Dim>,
        elem: Box<Layout>,
    },
    Record(Vec<(String, Layout)>),
    /// A composite whose bounds elaboration does not know: a variable sized when its process
    /// starts, an object of a partly constrained subtype.
    Open,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Dim {
    pub(crate) bounds: Bounds,
    /// The literals of an enumeration index type, by position, for element names.
    pub(crate) lits: Option<Vec<String>>,
}

impl Layout {
    /// The number of scalar sub-elements (`vhpiSizeP`), if known.
    pub(crate) fn scalars(&self) -> Option<u64> {
        match self {
            Layout::Scalar(_) => Some(1),
            Layout::Array { dims, elem } => elem
                .scalars()?
                .checked_mul(dims.iter().map(|d| d.bounds.len()).product()),
            Layout::Record(fields) => fields.iter().map(|(_, f)| f.scalars()).sum(),
            Layout::Open => None,
        }
    }

    /// Direct sub-elements: record fields or array elements (all dimensions, row major).
    pub(crate) fn count(&self) -> u64 {
        match self {
            Layout::Array { dims, .. } => dims.iter().map(|d| d.bounds.len()).product(),
            Layout::Record(fields) => fields.len() as u64,
            Layout::Scalar(_) | Layout::Open => 0,
        }
    }

    /// Sub-element `i`: its layout, the suffix naming it (`.F`, `(3)`, `(0,1)`) in original
    /// spelling, and its class kind.
    pub(crate) fn step(&self, i: u64) -> Option<(&Layout, String, vhpiClassKindT)> {
        match self {
            Layout::Record(fields) => {
                let (name, f) = fields.get(usize::try_from(i).ok()?)?;
                Some((f, format!(".{name}"), vhpiSelectedNameK))
            }
            Layout::Array { dims, elem } if i < self.count() => {
                let mut rest = i;
                let mut images = Vec::with_capacity(dims.len());
                for d in dims.iter().rev() {
                    let n = d.bounds.len();
                    images.push(d.image(index_at(d.bounds, rest % n)));
                    rest /= n;
                }
                images.reverse();
                Some((elem, format!("({})", images.join(",")), vhpiIndexedNameK))
            }
            _ => None,
        }
    }

    /// The sub-element named by `sel` (a field name or `(i,j)` index text, upper case).
    fn find(&self, sel: &Sel) -> Option<u64> {
        match (self, sel) {
            (Layout::Record(fields), Sel::Field(f)) => fields
                .iter()
                .position(|(n, _)| vhpi_name(n) == *f)
                .map(|p| p as u64),
            (Layout::Array { dims, .. }, Sel::Index(ix)) if ix.len() == dims.len() => {
                let mut lin = 0u64;
                for (text, d) in ix.iter().zip(dims) {
                    let v = d.parse(text)?;
                    lin = lin * d.bounds.len() + d.bounds.pos(v)?;
                }
                Some(lin)
            }
            _ => None,
        }
    }
}

impl Dim {
    fn image(&self, v: i64) -> String {
        match &self.lits {
            Some(lits) => usize::try_from(v)
                .ok()
                .and_then(|p| lits.get(p))
                .cloned()
                .unwrap_or_else(|| v.to_string()),
            None => v.to_string(),
        }
    }

    fn parse(&self, text: &str) -> Option<i64> {
        match &self.lits {
            Some(lits) => lits
                .iter()
                .position(|l| vhpi_name(l) == text)
                .map(|p| p as i64),
            None => text.parse().ok(),
        }
    }
}

fn range_kind(sc: &Scalar) -> vhpiClassKindT {
    match sc {
        Scalar::Enum { .. } => vhpiEnumRangeK,
        Scalar::Int { .. } => vhpiIntRangeK,
        Scalar::Real => vhpiFloatRangeK,
        Scalar::Phys { .. } => vhpiPhysRangeK,
    }
}

fn index_at(b: Bounds, p: u64) -> i64 {
    match b.dir {
        Direction::To => b.left + p as i64,
        Direction::Downto => b.left - p as i64,
    }
}

enum Sel {
    Field(String),
    Index(Vec<String>),
}

/// `vhpiNameP` of a name in original spelling: basic identifiers upper case, extended
/// identifiers (`\Foo\`) and character literals as written.
pub(crate) fn vhpi_name(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut extended = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                // `\\` inside an extended identifier is one backslash, not its end.
                if extended && chars.peek() == Some(&'\\') {
                    out.push(c);
                    out.push(chars.next().expect("peeked"));
                    continue;
                }
                extended = !extended;
                out.push(c);
            }
            '\'' if !extended => {
                out.push(c);
                out.extend(chars.by_ref().take(2));
            }
            _ if extended => out.push(c),
            _ => out.push(c.to_ascii_uppercase()),
        }
    }
    out
}

/// What only some nodes have: their relations and properties by the VHPI constant (#831).
#[derive(Default)]
pub(crate) struct Extra {
    pub(crate) one: HashMap<(NodeId, vhpiOneToOneT), NodeId>,
    pub(crate) many: HashMap<(NodeId, vhpiOneToManyT), Vec<NodeId>>,
    pub(crate) ints: HashMap<(NodeId, vhpiIntPropertyT), vhpiIntT>,
    pub(crate) strs: HashMap<(NodeId, vhpiStrPropertyT), String>,
}

pub(crate) struct Design {
    pub(crate) extra: Extra,
    /// The libraries of the design (`vhpiLibraryDecls`).
    pub(crate) libraries: Vec<NodeId>,
    pub(crate) net: crate::connect::Net,
    /// The variables of a protected type (`vhpi_protected_call`).
    pub(crate) protected: std::collections::HashSet<NodeId>,
    pub(crate) nodes: Vec<Node>,
    pub(crate) packages: Vec<NodeId>,
    pub(crate) types: Types,
    /// `vhpiFullNameP` of every region and declaration.
    by_name: HashMap<String, NodeId>,
    /// Where each kernel leaf is stored (`yavs_elab::NetGraph::loc`).
    pub(crate) locs: Vec<yavs_elab::LeafLoc>,
    /// `vhpiAttrDecl` of an attribute specification node (`vhpiAttrDeclK` nodes, no parent).
    pub(crate) attr_decls: HashMap<NodeId, NodeId>,
}

impl Default for Design {
    fn default() -> Design {
        Design::named("")
    }
}

impl Design {
    /// The region node of elaborated scope `s`.
    pub(crate) fn scope_node(&self, s: ScopeId) -> Option<NodeId> {
        let i = self
            .nodes
            .iter()
            .position(|n| matches!(n.origin, Origin::Scope(x) if x == s))?;
        Some(NodeId(i as u32))
    }

    pub(crate) fn node(&self, n: NodeId) -> &Node {
        &self.nodes[n.0 as usize]
    }

    /// A design of only a root instance called `name`: the spike host has no elaborated
    /// design.
    pub(crate) fn named(name: &str) -> Design {
        let mut d = Design::empty();
        d.push(Node {
            kind: vhpiRootInstK,
            name: name.to_owned(),
            full: format!(":{name}"),
            parent: None,
            file: None,
            line: None,
            regions: Vec::new(),
            decls: Vec::new(),
            unit: None,
            def_name: None,
            mode: None,
            layout: None,
            ty: None,
            origin: Origin::Named,
        });
        d
    }

    fn empty() -> Design {
        Design {
            extra: Extra::default(),
            libraries: Vec::new(),
            net: Default::default(),
            protected: Default::default(),
            nodes: Vec::new(),
            packages: Vec::new(),
            types: Types::default(),
            by_name: HashMap::new(),
            locs: Vec::new(),
            attr_decls: HashMap::new(),
        }
    }

    /// The snapshot of an elaborated design.
    pub(crate) fn build(db: &Db, e: &Elaborated) -> Design {
        let mut b = Builder {
            db,
            e,
            d: Design::empty(),
            units: HashMap::new(),
            scalars: RefCell::default(),
            types: RefCell::default(),
            base_tys: RefCell::default(),
            attrs: HashMap::new(),
            attr_nodes: HashMap::new(),
            subps: HashMap::new(),
            decl_nodes: HashMap::new(),
            sens_links: Vec::new(),
            procs_at: (e.procs.iter().enumerate())
                .map(|(i, p)| (p.scope, i))
                .rev()
                .collect(),
            labels: RefCell::default(),
            stmts: HashMap::new(),
            comp_links: Vec::new(),
        };
        for (id, a) in db.attr_values.iter() {
            b.attrs.entry(a.entity).or_default().push(id);
        }
        b.region(ScopeId(0), None);
        b.packages();
        b.units_and_libraries();
        b.links();
        b.net();
        b.d.types = b.types.into_inner();
        b.d.locs = e.nets.loc.clone();
        b.d
    }

    /// Adds a node; a region or declaration also goes under its parent and into the name
    /// index (design units are not found by name).
    fn push(&mut self, n: Node) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        let unit = matches!(
            n.kind,
            vhpiArchBodyK
                | vhpiEntityDeclK
                | vhpiPackDeclK
                | vhpiPackBodyK
                | vhpiConfigDeclK
                | vhpiAttrDeclK
                | vhpiLibraryDeclK
                | vhpiUseClauseK
                | vhpiCompConfigK
                | vhpiSubpBodyK
        ) || n.parent.is_none() && matches!(n.kind, vhpiFuncDeclK | vhpiProcDeclK);
        if !unit {
            self.by_name.entry(vhpi_name(&n.full)).or_insert(id);
        }
        if let Some(p) = n.parent {
            let parent = &mut self.nodes[p.0 as usize];
            let declared = matches!(
                n.kind,
                vhpiFileDeclK
                    | vhpiAliasDeclK
                    | vhpiCompDeclK
                    | vhpiFuncDeclK
                    | vhpiProcDeclK
                    | vhpiConstParamDeclK
                    | vhpiVarParamDeclK
                    | vhpiSigParamDeclK
                    | vhpiFileParamDeclK
            );
            if n.layout.is_some() || declared {
                parent.decls.push(id);
            } else {
                parent.regions.push(id);
            }
        }
        self.nodes.push(n);
        id
    }

    /// The layout of sub-element `path` of declaration `base`, with its names and kind.
    pub(crate) fn elem(&self, base: NodeId, path: &[u64]) -> Option<ElemInfo<'_>> {
        let n = self.node(base);
        let mut l = n.layout.as_ref()?;
        let (mut name, mut full, mut kind) = (n.name.clone(), n.full.clone(), n.kind);
        for &i in path {
            let (sub, suffix, k) = l.step(i)?;
            name.push_str(&suffix);
            full.push_str(&suffix);
            (l, kind) = (sub, k);
        }
        Some(ElemInfo {
            layout: l,
            name,
            full,
            kind,
        })
    }

    /// Looks up a VHPI name: absolute (`:top:u1:s`, `@lib:pkg:c`) or relative to `scope`
    /// (the root when `None`), case-insensitive, with `.field` and `(index)` selections.
    pub(crate) fn lookup(&self, name: &str, scope: Option<NodeId>) -> Option<(NodeId, Vec<u64>)> {
        // Spaces are dropped (`m(1, 0)`), except inside extended identifiers.
        let mut extended = false;
        let name: String = name
            .chars()
            .filter(|&c| {
                extended ^= c == '\\';
                extended || !c.is_whitespace()
            })
            .collect();
        let abs = if name.starts_with(':') || name.starts_with('@') {
            name
        } else if let Some(s) = scope {
            format!("{}:{name}", self.node(s).full)
        } else {
            format!(":{name}")
        };
        let abs = self.dots_to_colons(vhpi_name(&abs));
        if let Some(&n) = self.by_name.get(&abs) {
            return Some((n, Vec::new()));
        }
        // The longest prefix that names a declaration, then selections into its layout.
        let cuts = abs
            .char_indices()
            .filter(|&(_, c)| c == '.' || c == '(')
            .map(|(i, _)| i);
        let cuts: Vec<usize> = cuts.collect();
        for &cut in cuts.iter().rev() {
            let Some(&base) = self.by_name.get(&abs[..cut]) else {
                continue;
            };
            let mut l = self.node(base).layout.as_ref()?;
            let mut path = Vec::new();
            for sel in selections(&abs[cut..])? {
                let i = l.find(&sel)?;
                l = l.step(i)?.0;
                path.push(i);
            }
            return Some((base, path));
        }
        None
    }
}

impl Design {
    /// `.` also separates the levels of the hierarchy (`:TOP.CHILD.SIG`): a `.` that follows a
    /// region (no layout) is a `:`. A `.` after a declaration stays a field selection.
    fn dots_to_colons(&self, mut abs: String) -> String {
        let mut from = 0;
        while let Some(i) = abs[from..].find('.').map(|i| i + from) {
            match self.by_name.get(&abs[..i]) {
                Some(&n) if self.node(n).layout.is_none() => abs.replace_range(i..=i, ":"),
                _ => break,
            }
            from = i + 1;
        }
        abs
    }
}

pub(crate) struct ElemInfo<'a> {
    pub(crate) layout: &'a Layout,
    pub(crate) name: String,
    pub(crate) full: String,
    pub(crate) kind: vhpiClassKindT,
}

/// `.a(1,2).b` as selections; `None` when malformed.
fn selections(mut s: &str) -> Option<Vec<Sel>> {
    let mut out = Vec::new();
    while !s.is_empty() {
        if let Some(rest) = s.strip_prefix('.') {
            let end = rest.find(['.', '(']).unwrap_or(rest.len());
            out.push(Sel::Field(rest[..end].to_owned()));
            s = &rest[end..];
        } else {
            let rest = s.strip_prefix('(')?;
            let end = rest.find(')')?;
            out.push(Sel::Index(
                rest[..end].split(',').map(str::to_owned).collect(),
            ));
            s = &rest[end + 1..];
        }
    }
    Some(out)
}

struct Builder<'a> {
    db: &'a Db,
    e: &'a Elaborated,
    d: Design,
    units: HashMap<UnitId, NodeId>,
    /// Scalar types by base type: enumeration literals are shared.
    scalars: RefCell<HashMap<TypeId, Scalar>>,
    types: RefCell<Types>,
    /// Type declarations by base type.
    base_tys: RefCell<HashMap<TypeId, TyId>>,
    /// User attribute specifications by the declaration they decorate.
    attrs: HashMap<DeclId, Vec<AttrValueId>>,
    /// The `vhpiAttrDeclK` node of each attribute declaration (made once).
    attr_nodes: HashMap<DeclId, NodeId>,
    /// The `vhpiFuncDeclK` or `vhpiProcDeclK` node of each subprogram (made once).
    subps: HashMap<DeclId, NodeId>,
    /// The node of each listed declaration, by its elaborated region (`None`: a package's).
    decl_nodes: HashMap<(Option<ScopeId>, DeclId), NodeId>,
    /// `vhpiCompDecl` to fill in once every declaration has its node: the referring node, its
    /// elaborated region and the component.
    comp_links: Vec<(NodeId, ScopeId, DeclId)>,
    /// The signals a statement region is sensitive to: filled in as `comp_links` are.
    sens_links: Vec<(NodeId, ScopeId, Vec<DeclId>)>,
    /// The first kernel process of each process scope.
    procs_at: HashMap<ScopeId, usize>,
    /// The statement labels of each declarative region, by lower-case name (made once).
    labels: RefCell<HashMap<yavs_sem::ScopeId, HashMap<String, DeclId>>>,
    /// The node of each sequential statement (made once).
    stmts: HashMap<yavs_sem::StmtId, NodeId>,
}

impl Builder<'_> {
    fn place(&self, span: Span) -> (Option<String>, Option<i32>) {
        let f = self.db.sources.get(span.file);
        let line = i32::try_from(f.line_col(span.start).0).ok();
        (Some(f.path.clone()), line)
    }

    fn lib(&self, u: UnitId) -> String {
        self.db.libs[self.db.units[u].lib].name.text.to_string()
    }

    /// The node of design unit `u` (made once).
    fn unit(&mut self, u: UnitId) -> NodeId {
        if let Some(&n) = self.units.get(&u) {
            return n;
        }
        let unit = &self.db.units[u];
        let (kind, primary) = match &unit.hir {
            UnitHir::Architecture { entity, .. } => (vhpiArchBodyK, Some(*entity)),
            UnitHir::Entity { .. } => (vhpiEntityDeclK, None),
            UnitHir::PackageBody { package } => (vhpiPackBodyK, Some(*package)),
            UnitHir::Configuration(_) => (vhpiConfigDeclK, None),
            _ => (vhpiPackDeclK, None),
        };
        let name = unit.name.text.to_string();
        let full = match primary {
            Some(p) => format!("@{}:{}({name})", self.lib(u), self.db.units[p].name.text),
            None => format!("@{}:{name}", self.lib(u)),
        };
        let primary = primary.map(|p| self.unit(p));
        let (file, line) = self.place(unit.name.span);
        let n = self.d.push(Node {
            kind,
            name,
            full,
            parent: None,
            file,
            line,
            regions: Vec::new(),
            decls: Vec::new(),
            unit: primary,
            def_name: None,
            mode: None,
            layout: None,
            ty: None,
            origin: Origin::Unit(u),
        });
        self.units.insert(u, n);
        self.end_line(n, unit.span);
        self.attr_specs(n, None, unit.decl);
        n
    }

    /// `vhpiEndLineNoP` of node `n`: the last line of `span`.
    fn end_line(&mut self, n: NodeId, span: Span) {
        let f = self.db.sources.get(span.file);
        if let Ok(line) = i32::try_from(f.line_col(span.end).0) {
            self.d.extra.ints.insert((n, vhpiEndLineNoP), line);
        }
    }

    fn link_many(&mut self, n: NodeId, rel: vhpiOneToManyT, x: NodeId) {
        self.d.extra.many.entry((n, rel)).or_default().push(x);
    }

    /// A node that only relations reach: no parent, no name in the index.
    fn bare(&mut self, kind: vhpiClassKindT, name: String, span: Span) -> NodeId {
        let (file, line) = self.place(span);
        self.d.push(Node {
            kind,
            full: name.clone(),
            name,
            parent: None,
            file,
            line,
            regions: Vec::new(),
            decls: Vec::new(),
            unit: None,
            def_name: None,
            mode: None,
            layout: None,
            ty: None,
            origin: Origin::Named,
        })
    }

    /// The label of statement region `s`: its declaration in the enclosing region.
    fn label_of(&self, s: ScopeId) -> Option<DeclId> {
        let (db, e) = (self.db, self.e);
        let sc = e.hier.scope(s);
        let mut p = sc.parent;
        while let Some(x) = p
            && matches!(e.hier.scope(x).kind, ScopeKind::Component { .. })
        {
            p = e.hier.scope(x).parent;
        }
        let regions = e.regions.get(&p?)?;
        let wanted = sc.name.text.to_string().to_ascii_lowercase();
        let mut labels = self.labels.borrow_mut();
        regions.iter().find_map(|&r| {
            let of = labels.entry(r).or_insert_with(|| {
                (db.scopes[r].order.iter().copied())
                    .filter(|&d| matches!(db.decls[d].kind, DeclKind::Label(_)))
                    .map(|d| (db.decls[d].name.to_string().to_ascii_lowercase(), d))
                    .collect()
            });
            of.get(&wanted).copied()
        })
    }

    /// The node of declaration `d` seen from elaborated region `s`.
    fn find_decl(&self, mut s: Option<ScopeId>, d: DeclId) -> Option<NodeId> {
        while let Some(x) = s {
            if let Some(&n) = self.decl_nodes.get(&(Some(x), d)) {
                return Some(n);
            }
            s = self.e.hier.scope(x).parent;
        }
        self.decl_nodes.get(&(None, d)).copied()
    }

    /// What a process or an equivalent process says about itself: its flags, its sensitivity
    /// and its sequential statements (#829).
    fn statement(&mut self, n: NodeId, s: ScopeId, p: usize) {
        let (db, e) = (self.db, self.e);
        let ScopeKind::Process { stmt } = e.hier.scope(s).kind else {
            return;
        };
        let st = &db.stmts[stmt];
        let mut reads = Vec::new();
        let mut guarded = false;
        let mut body: &[yavs_sem::StmtId] = &[];
        let postponed = match &st.kind {
            StmtKind::Process {
                postponed,
                sensitivity,
                info,
                body: b,
                ..
            } => {
                body = b;
                match sensitivity {
                    yavs_sem::hir::Sensitivity::List(l) => reads.extend(l.iter().cloned()),
                    yavs_sem::hir::Sensitivity::All => {
                        reads.extend(info.reads_all.iter().flatten().cloned())
                    }
                    yavs_sem::hir::Sensitivity::None => {}
                }
                *postponed
            }
            StmtKind::ConcSignalAssign {
                postponed,
                guarded: g,
                ..
            } => {
                guarded = *g;
                db.stmt_reads(stmt, &mut reads);
                *postponed
            }
            StmtKind::ConcProcCall { postponed, .. } | StmtKind::ConcAssert { postponed, .. } => {
                db.stmt_reads(stmt, &mut reads);
                *postponed
            }
            _ => e.procs[p].postponed,
        };
        let ints = &mut self.d.extra.ints;
        ints.insert((n, vhpiIsPostponedP), vhpiIntT::from(postponed));
        ints.insert((n, vhpiIsGuardedP), vhpiIntT::from(guarded));
        // LRM 11.3: a process with no signal assignment is passive.
        let passive = e.procs[p].drivers.is_empty();
        ints.insert((n, vhpiIsPassiveP), vhpiIntT::from(passive));
        ints.insert((n, vhpiIsSeqStmtP), 0);
        let mut roots: Vec<DeclId> = (reads.iter())
            .filter_map(|l| match l.root {
                yavs_sem::hir::ObjRoot::Decl(d) => Some(d),
                _ => None,
            })
            .collect();
        let mut seen = std::collections::HashSet::new();
        roots.retain(|d| seen.insert(*d));
        self.sens_links.push((n, s, roots));
        for &x in body {
            let c = self.seq_stmt(x);
            self.link_many(n, vhpiSeqStmts, c);
        }
    }

    /// The node of sequential statement `x` (one per statement of the source) and those inside.
    fn seq_stmt(&mut self, x: yavs_sem::StmtId) -> NodeId {
        use yavs_sem::hir::LoopScheme;
        if let Some(&n) = self.stmts.get(&x) {
            return n;
        }
        let st = &self.db.stmts[x];
        let kind = match &st.kind {
            StmtKind::Wait { .. } => vhpiWaitStmtK,
            StmtKind::Assert { .. } => vhpiSeqAssertStmtK,
            StmtKind::Report { .. } => vhpiReportStmtK,
            StmtKind::SignalAssign { .. } | StmtKind::Force { .. } | StmtKind::Release { .. } => {
                vhpiSeqSigAssignStmtK
            }
            StmtKind::VarAssign { .. } => vhpiVarAssignStmtK,
            StmtKind::ProcCall(_) => vhpiSeqProcCallStmtK,
            StmtKind::If { .. } => vhpiIfStmtK,
            StmtKind::Case { .. } => vhpiCaseStmtK,
            StmtKind::Loop { scheme, .. } => match scheme {
                LoopScheme::Forever => vhpiForeverLoopK,
                LoopScheme::While(_) => vhpiWhileLoopK,
                LoopScheme::For { .. } => vhpiForLoopK,
            },
            StmtKind::Next { .. } => vhpiNextStmtK,
            StmtKind::Exit { .. } => vhpiExitStmtK,
            StmtKind::Return(_) => vhpiReturnStmtK,
            _ => vhpiNullStmtK,
        };
        let label = st.label.as_ref().map(|l| l.text.to_string());
        let n = self.bare(kind, label.clone().unwrap_or_default(), st.span);
        self.stmts.insert(x, n);
        self.end_line(n, st.span);
        self.d.extra.ints.insert((n, vhpiIsSeqStmtP), 1);
        if let Some(l) = label {
            self.d.extra.strs.insert((n, vhpiLabelNameP), l);
        }
        let mut inner: Vec<yavs_sem::StmtId> = Vec::new();
        match &st.kind {
            StmtKind::If { branches, else_ } => {
                inner.extend(branches.iter().flat_map(|(_, b)| b.iter().copied()));
                inner.extend(else_.iter().flat_map(|b| b.iter().copied()));
            }
            StmtKind::Case { alts, .. } => {
                inner.extend(alts.iter().flat_map(|(_, b)| b.iter().copied()));
            }
            StmtKind::Loop { body, .. } | StmtKind::SeqBlock { body, .. } => {
                inner.extend(body.iter().copied());
            }
            _ => {}
        }
        for i in inner {
            let c = self.seq_stmt(i);
            self.link_many(n, vhpiSeqStmts, c);
        }
        n
    }

    /// Fills in the relations that need every declaration's node.
    fn links(&mut self) {
        for (n, s, roots) in std::mem::take(&mut self.sens_links) {
            for d in roots {
                if let Some(x) = self.find_decl(Some(s), d) {
                    self.link_many(n, vhpiSensitivities, x);
                }
            }
        }
        for (n, s, comp) in std::mem::take(&mut self.comp_links) {
            if let Some(c) = self.find_decl(Some(s), comp) {
                self.d.extra.one.insert((n, vhpiCompDecl), c);
            }
        }
    }

    /// Libraries, the design units of each and the use clauses of each unit (#831).
    fn units_and_libraries(&mut self) {
        let db = self.db;
        let mut libs = HashMap::new();
        for (l, lib) in db.libs.iter() {
            let name = lib.name.text.to_string();
            let n = self.bare(vhpiLibraryDeclK, name.clone(), lib.name.span);
            self.d.extra.strs.insert((n, vhpiLibLogicalNameP), name);
            let path = match &lib.source {
                yavs_sem::loader::LibSource::Manifest(m) => Some(m.dir().to_path_buf()),
                yavs_sem::loader::LibSource::StdDir(p) => Some(p.clone()),
                yavs_sem::loader::LibSource::Files => None,
            };
            if let Some(p) = path {
                let p = p.display().to_string();
                self.d.extra.strs.insert((n, vhpiLibPhysicalNameP), p);
            }
            self.d.libraries.push(n);
            libs.insert(l, n);
        }
        for (u, unit) in db.units.iter() {
            if matches!(unit.hir, UnitHir::Pending | UnitHir::Context) {
                continue;
            }
            let n = self.unit(u);
            let lib = libs[&unit.lib];
            self.d.extra.one.insert((n, vhpiLibraryDecl), lib);
            self.link_many(lib, vhpiDesignUnits, n);
            for r in [unit.context, unit.scope] {
                for item in &db.scopes[r].uses {
                    let name = self.use_name(item);
                    let c = self.bare(vhpiUseClauseK, name, unit.span);
                    self.link_many(n, vhpiUseClauses, c);
                }
                for &d in &db.scopes[r].order {
                    if let DeclKind::Library(l) = db.decls[d].kind {
                        self.link_many(n, vhpiLibraryDecls, libs[&l]);
                    }
                }
            }
        }
    }

    /// What a use clause names, as written with `.all` when it is a whole region.
    fn use_name(&self, item: &yavs_sem::scope::UseItem) -> String {
        use yavs_sem::scope::UseItem;
        let db = self.db;
        match item {
            UseItem::All(s) => match db.scopes[*s].owner {
                Some(d) => format!("{}.all", db.decls[d].name),
                None => "all".to_owned(),
            },
            UseItem::One(ds) => ds
                .first()
                .map_or_else(String::new, |&d| db.decls[d].name.to_string()),
            UseItem::Library(l) => format!("{}.all", db.libs[*l].name.text),
        }
    }

    /// The region of elaborated scope `s` and everything below it.
    fn region(&mut self, s: ScopeId, parent: Option<NodeId>) {
        let (db, e) = (self.db, self.e);
        let sc = e.hier.scope(s);
        let mut sem_regions = e.regions.get(&s).cloned().unwrap_or_default();
        let (kind, unit) = match &sc.kind {
            // A component scope holds the component ports; its bound instance carries the label.
            ScopeKind::Component { .. } => {
                for &c in &sc.children {
                    self.region(c, parent);
                }
                return;
            }
            ScopeKind::Instance { arch, .. } if parent.is_none() => (vhpiRootInstK, Some(*arch)),
            ScopeKind::Instance { arch, .. } => (vhpiCompInstStmtK, Some(*arch)),
            ScopeKind::Block => (vhpiBlockStmtK, None),
            ScopeKind::ForGenerate { .. } => (vhpiForGenerateK, None),
            // VHPI has one class for the chosen alternative of `if` and `case` generates.
            ScopeKind::Generate { .. } => (vhpiIfGenerateK, None),
            ScopeKind::Process { stmt } => match &db.stmts[*stmt].kind {
                StmtKind::Process { scope, .. } => {
                    sem_regions.push(*scope);
                    (vhpiProcessStmtK, None)
                }
                // Equivalent processes of concurrent statements (#829).
                StmtKind::ConcSignalAssign { rhs, .. } => (
                    match rhs {
                        yavs_sem::hir::Rhs::Simple(_) => vhpiSimpleSigAssignStmtK,
                        yavs_sem::hir::Rhs::Conditional(_) => vhpiCondSigAssignStmtK,
                        yavs_sem::hir::Rhs::Selected { .. } => vhpiSelectSigAssignStmtK,
                    },
                    None,
                ),
                StmtKind::ConcAssert { .. } => (vhpiConcAssertStmtK, None),
                StmtKind::ConcProcCall { .. } => (vhpiConcProcCallStmtK, None),
                // The processes of a port's expression actual and of a block's guard are
                // the tool's, not statements of the design.
                _ => return,
            },
            // Packages are listed by `packages`.
            ScopeKind::Library | ScopeKind::Package { .. } => return,
        };
        let mut name = sc.name.text.to_string();
        if let Some(i) = &sc.index {
            name = format!("{name}({i})");
        }
        let full = match parent {
            Some(p) => format!("{}:{name}", self.d.node(p).full),
            None => format!(":{name}"),
        };
        let def_name = match sc.kind {
            ScopeKind::Instance { entity, arch } => Some(vhpi_name(&format!(
                "{}.{}({})",
                self.lib(entity),
                db.units[entity].name.text,
                db.units[arch].name.text
            ))),
            _ => None,
        };
        let (file, line) = self.place(sc.name.span);
        let n = self.d.push(Node {
            kind,
            name,
            full,
            parent,
            file,
            line,
            regions: Vec::new(),
            decls: Vec::new(),
            unit: None,
            def_name,
            mode: None,
            layout: None,
            ty: None,
            origin: Origin::Scope(s),
        });
        // A statement's label: its attributes, and where it ends (#831).
        if let Some(l) = self.label_of(s) {
            self.attr_specs(n, Some(s), l);
            if let DeclKind::Label(stmt) = db.decls[l].kind {
                self.end_line(n, db.stmts[stmt].span);
            }
        }
        if kind == vhpiCompInstStmtK || kind == vhpiRootInstK {
            let comp = sc.parent.and_then(|p| match e.hier.scope(p).kind {
                ScopeKind::Component { comp } => Some(comp),
                _ => None,
            });
            let inst = if comp.is_some() { vhpiComp } else { vhpiDirect };
            self.d.extra.ints.insert((n, vhpiCompInstKindP), inst);
            let label = sc.name.text.to_string();
            self.d.extra.strs.insert((n, vhpiCompInstNameP), label);
            if let Some(c) = comp {
                let name = db.decls[c].name.to_string();
                self.d.extra.strs.insert((n, vhpiCompNameP), name);
                self.comp_links.push((n, s, c));
            }
        }
        if matches!(sc.kind, ScopeKind::Process { .. })
            && let Some(&p) = self.procs_at.get(&s)
        {
            self.d.net.proc_node.insert(p as u32, n);
            self.d.net.node_proc.insert(n, p as u32);
            self.statement(n, s, p);
        }
        // After the region itself: the root instance is the first node.
        if let Some(u) = unit {
            let u = self.unit(u);
            self.d.nodes[n.0 as usize].unit = Some(u);
        }
        // A for generate's parameter is a constant of its iteration.
        let param = match sc.kind {
            ScopeKind::ForGenerate { param, .. } => Some(param),
            _ => None,
        };
        if let Some(p) = param {
            self.decl(n, Some(s), p);
        }
        for r in sem_regions {
            for &d in &db.scopes[r].order {
                if Some(d) != param {
                    self.decl(n, Some(s), d);
                }
            }
            for c in db.config_specs.get(&r).into_iter().flatten() {
                let name = db.decls[c.component].name.to_string();
                let x = self.bare(vhpiCompConfigK, name, c.span);
                self.link_many(n, vhpiConfigSpecs, x);
                self.comp_links.push((x, s, c.component));
            }
        }
        for &c in &sc.children {
            self.region(c, Some(n));
        }
    }

    /// Declaration `d` of region `n` (elaborated scope `s`), if VHPI lists it.
    fn decl(&mut self, n: NodeId, s: Option<ScopeId>, d: DeclId) {
        let (db, e) = (self.db, self.e);
        let decl = &db.decls[d];
        match &decl.kind {
            DeclKind::Alias(a) => return self.alias_decl(n, s, d, a),
            DeclKind::Component(_) => {
                let full = format!("{}:{}", self.d.node(n).full, decl.name);
                let (file, line) = self.place(decl.name_span());
                let c = self.d.push(Node {
                    kind: vhpiCompDeclK,
                    name: decl.name.to_string(),
                    full,
                    parent: Some(n),
                    file,
                    line,
                    regions: Vec::new(),
                    decls: Vec::new(),
                    unit: None,
                    def_name: None,
                    mode: None,
                    layout: None,
                    ty: None,
                    origin: Origin::Named,
                });
                self.end_line(c, decl.span);
                self.decl_nodes.insert((s, d), c);
                return;
            }
            DeclKind::Subprogram(_) if !decl.implicit => {
                self.subp(d, Some(n));
                return;
            }
            DeclKind::Attribute { .. } => {
                let a = self.attr_node(d);
                self.link_many(n, vhpiAttrDecls, a);
                return;
            }
            _ => {}
        }
        let DeclKind::Object(o) = &decl.kind else {
            return;
        };
        if o.class == ObjClass::File && o.role != ObjRole::Param {
            let ty = self.object_ty(o.subtype, &Layout::Open, s);
            let full = format!("{}:{}", self.d.node(n).full, decl.name);
            let (file, line) = self.place(decl.name_span());
            let f = self.d.push(Node {
                kind: vhpiFileDeclK,
                name: decl.name.to_string(),
                full,
                parent: Some(n),
                file,
                line,
                regions: Vec::new(),
                decls: Vec::new(),
                unit: None,
                def_name: None,
                mode: None,
                layout: None,
                ty: Some(ty),
                origin: Origin::Named,
            });
            self.end_line(f, decl.span);
            self.decl_nodes.insert((s, d), f);
            return;
        }
        let kind = match (&o.role, o.class) {
            (ObjRole::Param, ObjClass::Constant) => vhpiConstParamDeclK,
            (ObjRole::Param, ObjClass::Variable | ObjClass::SharedVariable) => vhpiVarParamDeclK,
            (ObjRole::Param, ObjClass::Signal) => vhpiSigParamDeclK,
            (ObjRole::Param, ObjClass::File) => vhpiFileParamDeclK,
            (ObjRole::Generic, ObjClass::Constant) => vhpiGenericDeclK,
            (ObjRole::Port, ObjClass::Signal | ObjClass::Variable) => vhpiPortDeclK,
            (ObjRole::Plain | ObjRole::Deferred | ObjRole::Guard, ObjClass::Signal) => vhpiSigDeclK,
            (ObjRole::Plain | ObjRole::Deferred | ObjRole::GenerateParam, ObjClass::Constant) => {
                vhpiConstDeclK
            }
            (ObjRole::Plain, ObjClass::Variable | ObjClass::SharedVariable) => vhpiVarDeclK,
            // Implicit signals are listed under their prefix (`vhpiSigAttrs`, `Builder::net`).
            _ => return,
        };
        let name = decl.name.to_string();
        let signal = match s {
            Some(s) => e.sigs.get(&(s, d)).copied(),
            // A package signal lives in its package's elaborated scope.
            None if o.class == ObjClass::Signal && o.role != ObjRole::Param => {
                e.sigs.iter().find(|((_, x), _)| *x == d).map(|(_, &o)| o)
            }
            None => None,
        };
        let (layout, origin) = match signal {
            Some(obj) => (
                self.shape_layout(&e.nets.obj(obj).shape),
                Origin::Signal {
                    obj,
                    leaf: e.nets.obj(obj).leaves.0.0,
                    parts: e.nets.obj(obj).parts.clone(),
                },
            ),
            None if kind == vhpiVarDeclK => {
                // Its value is the running process's (`Origin::Var`), not its initial value.
                let proc = s.and_then(|s| {
                    e.procs
                        .iter()
                        .position(|p| p.scope == s && p.actual.is_none())
                });
                let v = o.default.and_then(|x| db.value(x));
                (
                    self.layout(o.subtype, v, s),
                    Origin::Var {
                        proc: proc.map(|p| p as u32),
                        scope: s,
                        decl: d,
                    },
                )
            }
            None => {
                let v = s.and_then(|s| e.value(s, d));
                let v = v.or_else(|| o.default.and_then(|x| db.value(x)));
                (
                    self.layout(o.subtype, v, s),
                    Origin::Decl {
                        scope: s,
                        decl: d,
                        value: v.cloned(),
                    },
                )
            }
        };
        let ty = self.object_ty(o.subtype, &layout, s);
        let mode = match (kind, o.mode.as_ref()) {
            (vhpiPortDeclK, Some(m)) => Some(match m {
                Mode::In => vhpiInMode,
                Mode::Out => vhpiOutMode,
                Mode::Buffer => vhpiBufferMode,
                Mode::Linkage => vhpiLinkageMode,
                // A mode view has no VHPI mode: its elements go both ways.
                Mode::Inout | Mode::View { .. } => vhpiInoutMode,
            }),
            _ => None,
        };
        let full = format!("{}:{name}", self.d.node(n).full);
        let (file, line) = self.place(decl.name_span());
        let node = self.d.push(Node {
            kind,
            name,
            full,
            parent: Some(n),
            file,
            line,
            regions: Vec::new(),
            decls: Vec::new(),
            unit: None,
            def_name: None,
            mode,
            layout: Some(layout),
            ty: Some(ty),
            origin,
        });
        self.decl_nodes.insert((s, d), node);
        self.end_line(node, decl.span);
        if o.signal_kind.is_some() {
            self.d.extra.ints.insert((node, vhpiIsGuardedP), 1);
        }
        if let Some(obj) = signal {
            self.d.net.obj_nodes.insert(obj, node);
        }
        if matches!(
            db.types[db.base(o.subtype)].kind,
            TypeKind::Protected { .. }
        ) {
            self.d.protected.insert(node);
        }
        self.attr_specs(node, s, d);
    }

    /// The user attribute specifications of declaration `d` (node `n`), as `vhpiAttrSpecK`
    /// children of its node (`vhpiAttrSpecs`), named `decl'attr`, valued like constants.
    fn attr_specs(&mut self, n: NodeId, s: Option<ScopeId>, d: DeclId) {
        let db = self.db;
        for a in self.attrs.get(&d).cloned().unwrap_or_default() {
            let av = &db.attr_values[a];
            let adecl = &db.decls[av.attr];
            let DeclKind::Attribute { ty: st } = adecl.kind else {
                continue;
            };
            // A value that depends on a generic folds with the region's values (#831).
            let v = self.fold(av.value, s);
            let v = v.as_ref();
            let layout = self.layout(st, v, s);
            let ty = self.object_ty(st, &layout, s);
            let name = adecl.name.to_string();
            let node = self.d.node(n);
            let full = format!("{}'{name}", node.full);
            let (file, line) = self.place(db.exprs[av.value].span);
            let spec = self.d.push(Node {
                kind: vhpiAttrSpecK,
                name: name.clone(),
                full,
                parent: Some(n),
                file,
                line,
                regions: Vec::new(),
                decls: Vec::new(),
                unit: None,
                def_name: None,
                mode: None,
                layout: Some(layout),
                ty: Some(ty),
                origin: Origin::Decl {
                    scope: s,
                    decl: av.attr,
                    value: v.cloned(),
                },
            });
            let decl_node = self.attr_node(av.attr);
            self.d.attr_decls.insert(spec, decl_node);
        }
    }

    /// The `vhpiAttrDeclK` node of attribute declaration `d` (made once).
    fn attr_node(&mut self, d: DeclId) -> NodeId {
        if let Some(&x) = self.attr_nodes.get(&d) {
            return x;
        }
        let decl = &self.db.decls[d];
        let x = self.bare(vhpiAttrDeclK, decl.name.to_string(), decl.name_span());
        self.attr_nodes.insert(d, x);
        x
    }

    /// An alias (#831): of a whole object it reads and writes like the object; of a part it has
    /// the object as its `vhpiAliasedName` and no value.
    // ponytail: no value for an alias of a field, element or slice; a path into the layout when
    // a plugin needs one.
    fn alias_decl(&mut self, n: NodeId, s: Option<ScopeId>, d: DeclId, a: &yavs_sem::decl::Alias) {
        let db = self.db;
        let decl = &db.decls[d];
        let full = format!("{}:{}", self.d.node(n).full, decl.name);
        let (file, line) = self.place(decl.name_span());
        let (target, whole) = match a {
            yavs_sem::decl::Alias::Object { target, .. } => match db.lsp(*target) {
                Some(l) => match l.root {
                    yavs_sem::hir::ObjRoot::Decl(t) => (self.find_decl(s, t), l.path.is_empty()),
                    _ => (None, false),
                },
                None => (None, false),
            },
            yavs_sem::decl::Alias::NonObject { target } => (self.find_decl(s, *target), false),
        };
        let like = target.filter(|_| whole).map(|t| {
            let t = self.d.node(t);
            (t.layout.clone(), t.ty, t.origin.clone())
        });
        let (layout, ty, origin) = like.unwrap_or((None, None, Origin::Named));
        let al = self.d.push(Node {
            kind: vhpiAliasDeclK,
            name: decl.name.to_string(),
            full,
            parent: Some(n),
            file,
            line,
            regions: Vec::new(),
            decls: Vec::new(),
            unit: None,
            def_name: None,
            mode: None,
            layout,
            ty,
            origin,
        });
        self.end_line(al, decl.span);
        if let Some(t) = target {
            self.d.extra.one.insert((al, vhpiAliasedName), t);
        }
        self.decl_nodes.insert((s, d), al);
    }

    /// The net graph of `Design::net` (`connect.rs`), once every signal has its node.
    fn net(&mut self) {
        use crate::connect::{Conn, LeafInfo, NetDriver};
        use yavs_elab::{LeafRes, ObjKind};
        use yavs_kernel::{GuardKind, PortMode, Range};
        let e = self.e;
        let nets = &e.nets;
        // The element count of a leaf, and an object's kernel elements.
        let cover = |o: ObjId| -> Vec<(yavs_kernel::SignalId, Range)> {
            let obj = nets.obj(o);
            (obj.leaves.0.0..obj.leaves.1.0)
                .map(|l| {
                    let part = obj.part((l - obj.leaves.0.0) as usize);
                    let l = yavs_elab::LeafId(l);
                    nets.kpart(l, part.unwrap_or(Range::new(0, nets.leaf(l).ty.len)))
                })
                .collect()
        };
        let mut on: HashMap<yavs_kernel::SignalId, Vec<(ObjId, Range)>> = HashMap::new();
        let mut on_scope: HashMap<(yavs_kernel::SignalId, ScopeId), Vec<(ObjId, Range)>> =
            HashMap::new();
        for i in 0..nets.objs.len() {
            let o = ObjId(i as u32);
            for (s, r) in cover(o) {
                on.entry(s).or_default().push((o, r));
                on_scope
                    .entry((s, nets.obj(o).scope))
                    .or_default()
                    .push((o, r));
            }
        }
        let depth = |mut s: ScopeId| {
            let mut d = 0u32;
            while let Some(p) = e.hier.scope(s).parent {
                (s, d) = (p, d + 1);
            }
            d
        };
        // A collapsed port shares its actual's elements, so the object a driver belongs to is
        // the innermost one on its elements whose scope holds the process.
        for (pi, p) in e.procs.iter().enumerate() {
            for &di in &p.drivers {
                let d = &nets.drivers[di];
                let (sig, range) = nets.kpart(d.leaf, d.range);
                // Up the scopes from the process: the first object on its elements.
                let mut obj = None;
                let mut at = Some(p.scope);
                while let (None, Some(x)) = (obj, at) {
                    obj = (on_scope.get(&(sig, x)).into_iter().flatten())
                        .find(|&&(_, r)| r.overlaps(range))
                        .map(|&(o, _)| o);
                    at = e.hier.scope(x).parent;
                }
                // Driven from outside its scope (an external name): the outermost object.
                let obj = obj.or_else(|| {
                    (on.get(&sig).into_iter().flatten())
                        .filter(|&&(_, r)| r.overlaps(range))
                        .min_by_key(|&&(o, _)| depth(nets.obj(o).scope))
                        .map(|&(o, _)| o)
                });
                let Some(obj) = obj else {
                    continue;
                };
                let ty = self.leaf_scalar(d.leaf);
                let i = self.d.net.drivers.len() as u32;
                self.d.net.of_obj.entry(obj).or_default().push(i);
                self.d.net.of_proc.entry(pi as u32).or_default().push(i);
                self.d.net.drivers.push(NetDriver {
                    proc: pi as u32,
                    sig,
                    range,
                    owner: self.d.net.obj_nodes.get(&obj).copied(),
                    ty,
                });
            }
        }
        // Port associations, collapsed ones from the actual they share elements with; the
        // ports of components, which have no node, are joined out of the chains.
        let mode_of = |m: PortMode| match m {
            PortMode::In => vhpiInMode,
            PortMode::Inout => vhpiInoutMode,
            PortMode::Buffer => vhpiBufferMode,
            _ => vhpiOutMode,
        };
        let mut conns: Vec<Conn> = Vec::new();
        for edge in &nets.ports {
            let (fs, fr) = nets.kpart(edge.formal.0, edge.formal.1);
            let (ad, ar) = nets.kpart(edge.actual.0, edge.actual.1);
            conns.push(Conn {
                formal: (nets.leaf(edge.formal.0).obj, fs, fr),
                actual: (nets.leaf(edge.actual.0).obj, ad, ar),
                mode: mode_of(edge.mode),
            });
        }
        for edge in &nets.conv_ports {
            for &(fl, fr) in &edge.formal {
                for &(al, ar) in &edge.actual {
                    let (fs, fr) = nets.kpart(fl, fr);
                    let (ad, ar) = nets.kpart(al, ar);
                    conns.push(Conn {
                        formal: (nets.leaf(fl).obj, fs, fr),
                        actual: (nets.leaf(al).obj, ad, ar),
                        mode: mode_of(edge.mode),
                    });
                }
            }
        }
        let own: HashSet<(ObjId, yavs_kernel::SignalId)> =
            conns.iter().map(|c| (c.formal.0, c.formal.1)).collect();
        for i in 0..nets.objs.len() {
            let o = ObjId(i as u32);
            let (ObjKind::Port(m) | ObjKind::ComponentPort(m)) = nets.obj(o).kind else {
                continue;
            };
            for (s, r) in cover(o) {
                // Its own elements have an edge when it is not collapsed.
                if own.contains(&(o, s)) {
                    continue;
                }
                let mut at = e.hier.scope(nets.obj(o).scope).parent;
                while let Some(x) = at {
                    let up = (on_scope.get(&(s, x)).into_iter().flatten())
                        .find(|&&(y, q)| y != o && q.overlaps(r));
                    if let Some(&(y, q)) = up {
                        conns.push(Conn {
                            formal: (o, s, r),
                            actual: (y, s, q),
                            mode: mode_of(m),
                        });
                        break;
                    }
                    at = e.hier.scope(x).parent;
                }
            }
        }
        let hidden = |o: ObjId| matches!(nets.obj(o).kind, ObjKind::ComponentPort(_));
        // Each association up to a component port continues to what that port is associated to.
        let mut first_of: HashMap<ObjId, usize> = HashMap::new();
        for (i, c) in conns.iter().enumerate() {
            first_of.entry(c.formal.0).or_insert(i);
        }
        let mut joined: Vec<Conn> = Vec::new();
        for c in conns.iter().filter(|c| !hidden(c.formal.0)) {
            let (mut a, mut guard) = (c.actual, 0);
            while hidden(a.0) && guard < 16 {
                let Some(&next) = first_of.get(&a.0) else {
                    break;
                };
                a = conns[next].actual;
                guard += 1;
            }
            if !hidden(a.0) {
                joined.push(Conn {
                    formal: c.formal,
                    actual: a,
                    mode: c.mode,
                });
            }
        }
        for (i, c) in joined.iter().enumerate() {
            let by = &mut self.d.net.conns_of;
            by.entry(c.formal.0).or_default().push(i as u32);
            if c.actual.0 != c.formal.0 {
                by.entry(c.actual.0).or_default().push(i as u32);
            }
        }
        self.d.net.conns = joined;
        // Implicit signals that exist, under their prefix (#831).
        for (i, obj) in nets.objs.iter().enumerate() {
            if obj.kind != ObjKind::Implicit {
                continue;
            }
            let leaf = nets.leaf(obj.leaves.0);
            let Some((ik, src, _)) = leaf.implicit else {
                continue;
            };
            let Some(&prefix) = self.d.net.obj_nodes.get(&nets.leaf(src).obj) else {
                continue;
            };
            let (attr, pa, t) = match ik {
                yavs_elab::ImplicitKind::Stable(t) => ("stable", vhpiStablePA, Some(t)),
                yavs_elab::ImplicitKind::Quiet(t) => ("quiet", vhpiQuietPA, Some(t)),
                yavs_elab::ImplicitKind::Delayed(t) => ("delayed", vhpiDelayedPA, Some(t)),
                yavs_elab::ImplicitKind::Transaction => ("transaction", vhpiTransactionPA, None),
            };
            let at = t.map_or_else(String::new, |t| format!("({} fs)", t.as_fs()));
            let p = self.d.node(prefix);
            let name = format!("{}'{attr}{at}", p.name);
            let full = format!("{}'{attr}{at}", p.full);
            let ty = match &self.db.decls[obj.decl].kind {
                DeclKind::Object(o) => {
                    let l = self.shape_layout(&obj.shape);
                    Some(self.object_ty(o.subtype, &l, None))
                }
                _ => None,
            };
            let layout = Some(self.shape_layout(&obj.shape));
            let (file, line) = self.place(obj.name.span);
            let n = self.d.push(Node {
                kind: vhpiSimpAttrNameK,
                name,
                full,
                parent: None,
                file,
                line,
                regions: Vec::new(),
                decls: Vec::new(),
                unit: None,
                def_name: None,
                mode: None,
                layout,
                ty,
                origin: Origin::Signal {
                    obj: ObjId(i as u32),
                    leaf: obj.leaves.0.0,
                    parts: obj.parts.clone(),
                },
            });
            self.d.net.obj_nodes.insert(ObjId(i as u32), n);
            self.link_many(prefix, vhpiSigAttrs, n);
            self.d.extra.one.insert((n, vhpiPrefix), prefix);
            self.d.extra.ints.insert((n, vhpiPredefAttrP), pa);
            self.d.extra.ints.insert((n, vhpiAttrKindP), vhpiSignalAK);
        }
        // Per leaf: resolution and kind.
        for l in &nets.leaves {
            let (mut resolved, mut whole) = (l.resolution.is_some(), false);
            let func = match &l.resolution {
                Some(LeafRes::Func(d)) => Some(*d),
                Some(LeafRes::Whole(d)) => {
                    whole = true;
                    Some(*d)
                }
                Some(LeafRes::StdLogic) => self.std_logic_resolved(nets.obj(l.obj).decl),
                None => None,
            };
            if func.is_some() {
                resolved = true;
            }
            let resol_func = func.map(|d| self.subp(d, None));
            self.d.net.leaves.push(LeafInfo {
                resolved,
                whole,
                sig_kind: match l.guard {
                    Some(GuardKind::Register) => vhpiRegister,
                    Some(GuardKind::Bus) => vhpiBus,
                    None => vhpiNormal,
                },
                resol_func,
            });
        }
    }

    /// The resolution function of the subtype of object `d` (`resolved` for `std_logic`).
    fn std_logic_resolved(&self, d: DeclId) -> Option<DeclId> {
        let db = self.db;
        let DeclKind::Object(o) = &db.decls[d].kind else {
            return None;
        };
        let mut st = Some(o.subtype);
        while let Some(x) = st {
            let mut r = db.subtypes[x].resolution.as_ref();
            while let Some(res) = r {
                match res {
                    yavs_sem::types::Resolution::Function(f) => return Some(*f),
                    yavs_sem::types::Resolution::Element(e) => r = Some(e),
                    yavs_sem::types::Resolution::Record(fs) => r = fs.first().map(|f| &f.1),
                }
            }
            st = db.subtypes[x].parent;
        }
        None
    }

    /// The value type of leaf `l`: the scalar at the end of its path in the object's shape.
    fn leaf_scalar(&self, l: yavs_elab::LeafId) -> Scalar {
        let leaf = self.e.nets.leaf(l);
        let mut layout = self.shape_layout(&self.e.nets.obj(leaf.obj).shape);
        let mut path = leaf.path.iter();
        loop {
            layout = match layout {
                Layout::Scalar(s) => return s,
                Layout::Array { elem, .. } => *elem,
                Layout::Record(mut f) => {
                    let &i = path.next().expect("a leaf's path names its record fields");
                    f.swap_remove(i as usize).1
                }
                Layout::Open => unreachable!("a signal's shape is elaborated"),
            };
        }
    }

    /// The node of subprogram `d` (made once): under region `parent`, or with none for a
    /// resolution function that no listed region declares, with its parameters as declarations
    /// and, for a VHDL body, a `vhpiSubpBodyK` node (#831).
    fn subp(&mut self, d: DeclId, parent: Option<NodeId>) -> NodeId {
        if let Some(&n) = self.subps.get(&d) {
            return n;
        }
        let db = self.db;
        let decl = &db.decls[d];
        let sp = match &decl.kind {
            DeclKind::Subprogram(sp) => Some(sp),
            _ => None,
        };
        let function = sp.is_none_or(|s| s.ret.is_some());
        let ty = (sp.and_then(|s| s.ret)).map(|r| {
            let l = self.layout(r, None, None);
            self.object_ty(r, &l, None)
        });
        let name = decl.name.to_string();
        let full = match parent {
            Some(p) => format!("{}:{name}", self.d.node(p).full),
            None => name.clone(),
        };
        let (file, line) = self.place(decl.name_span());
        let n = self.d.push(Node {
            kind: if function {
                vhpiFuncDeclK
            } else {
                vhpiProcDeclK
            },
            name: name.clone(),
            full,
            parent,
            file,
            line,
            regions: Vec::new(),
            decls: Vec::new(),
            unit: None,
            def_name: None,
            mode: None,
            layout: None,
            ty,
            origin: Origin::Named,
        });
        self.subps.insert(d, n);
        self.end_line(n, decl.span);
        let Some(sp) = sp else {
            return n;
        };
        let count = vhpiIntT::try_from(sp.params.len()).unwrap_or(vhpiIntT::MAX);
        self.d.extra.ints.insert((n, vhpiNumParamsP), count);
        if let yavs_sem::decl::SubprogramKind::Function { pure } = sp.kind {
            self.d
                .extra
                .ints
                .insert((n, vhpiIsPureP), vhpiIntT::from(pure));
        }
        for p in &sp.params {
            if let Some(pd) = p.decl {
                self.decl(n, None, pd);
            }
        }
        if let yavs_sem::decl::Body::Hir { stmts, .. } = &sp.body {
            let span = stmts.first().map_or(decl.span, |&x| db.stmts[x].span);
            let b = self.bare(vhpiSubpBodyK, name, span);
            self.d.extra.one.insert((n, vhpiSubpBody), b);
            self.d.extra.one.insert((b, vhpiSubpDecl), n);
            for &x in stmts.iter() {
                let c = self.seq_stmt(x);
                self.link_many(b, vhpiSeqStmts, c);
            }
        }
        n
    }

    /// Package instances: every analyzed non-generic package and package instantiation the
    /// design loaded, named `lib:pkg`, full name `@lib:pkg`.
    fn packages(&mut self) {
        let db = self.db;
        for (u, unit) in db.units.iter() {
            let region = match (&unit.kind, &unit.hir) {
                (UnitKind::Package, UnitHir::Package { generics: None }) => unit.scope,
                (UnitKind::PackageInst, UnitHir::PackageInst(i)) => db.insts[*i].scope,
                _ => continue,
            };
            let name = format!("{}:{}", self.lib(u), unit.name.text);
            let decl_unit = self.unit(u);
            let (file, line) = self.place(unit.name.span);
            let n = self.d.push(Node {
                kind: vhpiPackInstK,
                full: format!("@{name}"),
                name,
                parent: None,
                file,
                line,
                regions: Vec::new(),
                decls: Vec::new(),
                unit: Some(decl_unit),
                def_name: None,
                mode: None,
                layout: None,
                ty: None,
                origin: Origin::Unit(u),
            });
            self.d.packages.push(n);
            for &d in &db.scopes[region].order {
                self.decl(n, None, d);
            }
        }
    }

    fn enum_lits(&self, st: SubtypeId) -> Option<Vec<String>> {
        let TypeKind::Enum { lits } = &self.db.types[self.db.base(st)].kind else {
            return None;
        };
        Some(
            lits.iter()
                .map(|&l| self.db.decls[l].name.to_string())
                .collect(),
        )
    }

    /// The value type of scalars of base type `t`.
    fn scalar(&self, t: TypeId) -> Scalar {
        let db = self.db;
        let wk = &db.well_known;
        let mut cache = self.scalars.borrow_mut();
        cache
            .entry(t)
            .or_insert_with(|| match &db.types[t].kind {
                TypeKind::Enum { lits } => {
                    let character = Some(t) == wk.character;
                    let bit01 = Some(t) == wk.bit || Some(t) == wk.boolean;
                    let lits = lits
                        .iter()
                        .enumerate()
                        .map(|(p, &l)| {
                            let name = &db.decls[l].name;
                            let ch = match name {
                                _ if character => u8::try_from(p).ok(),
                                Designator::Char(c) => u8::try_from(u32::from(*c)).ok(),
                                _ => None,
                            };
                            Lit {
                                name: name.to_string(),
                                ch,
                            }
                        })
                        .collect();
                    Scalar::Enum {
                        lits,
                        // std_ulogic is the logic enumeration that is neither BIT nor BOOLEAN.
                        logic: Some(t) == wk.bit
                            || (!bit01 && yavs_elab::elem_of(db, t) == yavs_kernel::Elem::Logic),
                        character,
                        bit01,
                    }
                }
                TypeKind::Floating | TypeKind::UniversalReal => Scalar::Real,
                TypeKind::Physical { .. } => Scalar::Phys {
                    time: Some(t) == wk.time,
                },
                // The kernel holds an integer type within 32 bits in 4 bytes (`Ty::elem_size`).
                _ => Scalar::Int {
                    narrow: yavs_elab::elem_of(db, t) == yavs_kernel::Elem::Bytes(4),
                },
            })
            .clone()
    }

    fn shape_layout(&self, shape: &Shape) -> Layout {
        let db = self.db;
        match shape {
            Shape::Scalar { ty, .. } => Layout::Scalar(self.scalar(*ty)),
            Shape::Array { ty, dims, elem, .. } => {
                let indexes = match &db.types[*ty].kind {
                    TypeKind::Array { indexes, .. } => &indexes[..],
                    _ => &[],
                };
                Layout::Array {
                    dims: dims
                        .iter()
                        .enumerate()
                        .map(|(i, &bounds)| Dim {
                            bounds,
                            lits: indexes.get(i).and_then(|&ix| self.enum_lits(ix)),
                        })
                        .collect(),
                    elem: Box::new(self.shape_layout(elem)),
                }
            }
            Shape::Record { fields, .. } => Layout::Record(
                fields
                    .iter()
                    .map(|(n, f)| (n.text.to_string(), self.shape_layout(f)))
                    .collect(),
            ),
        }
    }

    /// The layout of an object of subtype `st` with value `v` (generics and constants):
    /// array bounds from the value, else from the subtype's constraint when elaboration's
    /// values of region `s` fold it.
    fn layout(&self, st: SubtypeId, v: Option<&ConstValue>, s: Option<ScopeId>) -> Layout {
        let db = self.db;
        match &db.types[db.base(st)].kind {
            TypeKind::Array { indexes, elem } => {
                let arr = match v {
                    Some(ConstValue::Array(a)) => Some(a),
                    _ => None,
                };
                // A constrained subtype's bounds win over the value's (LRM 5.3.2.2: a
                // variable's `x"00"` has the bounds of its `(7 downto 0)`).
                let constrained = match &db.subtypes[st].constraint {
                    Constraint::Array { dims: Some(ds), .. } => {
                        ds.iter().map(|r| self.range(r, s)).collect()
                    }
                    _ => None,
                };
                let bounds: Option<Vec<Bounds>> = constrained.or_else(|| {
                    arr.map(|a| {
                        a.bounds
                            .iter()
                            .map(|&(left, dir, right)| Bounds { left, dir, right })
                            .collect()
                    })
                });
                let Some(bounds) = bounds else {
                    return Layout::Open;
                };
                let first = arr.and_then(|a| match &a.elems {
                    Elems::Runs(r) => r.first().map(|(_, v)| v),
                    Elems::Bytes(_) => None,
                });
                Layout::Array {
                    dims: bounds
                        .into_iter()
                        .enumerate()
                        .map(|(i, bounds)| Dim {
                            bounds,
                            lits: indexes.get(i).and_then(|&ix| self.enum_lits(ix)),
                        })
                        .collect(),
                    elem: Box::new(self.layout(*elem, first, s)),
                }
            }
            TypeKind::Record { elems } => Layout::Record(
                elems
                    .iter()
                    .enumerate()
                    .map(|(i, el)| {
                        let fv = match v {
                            Some(ConstValue::Record(r)) => r.get(i),
                            _ => None,
                        };
                        (el.name.text.to_string(), self.layout(el.subtype, fv, s))
                    })
                    .collect(),
            ),
            _ => Layout::Scalar(self.scalar(db.base(st))),
        }
    }

    /// An explicit range whose bounds are static or fold with the generics and constants of
    /// region `s`.
    fn range(&self, r: &RangeC, s: Option<ScopeId>) -> Option<Bounds> {
        let RangeC::Explicit { left, dir, right } = r else {
            return None;
        };
        Some(Bounds {
            left: self.fold(*left, s)?.as_int()?,
            dir: *dir,
            right: self.fold(*right, s)?.as_int()?,
        })
    }

    /// A static expression, or one that folds with the generics and constants of region `s`.
    fn fold(&self, x: yavs_sem::ExprId, s: Option<ScopeId>) -> Option<ConstValue> {
        let (db, e) = (self.db, self.e);
        let mut operand = |x: yavs_sem::ExprId| -> Option<ConstValue> {
            if let Some(v) = db.value(x) {
                return Some(v.clone());
            }
            match &db.exprs[x].kind {
                yavs_sem::hir::ExprKind::Obj(d) => s.and_then(|s| e.value(s, *d)).cloned(),
                _ => None,
            }
        };
        operand(x).or_else(|| db.fold_with(x, &mut operand).ok().flatten())
    }

    /// The type of an object of subtype `st` laid out as `layout` (elaborated bounds) in
    /// region `s`: its type declaration or named subtype when the bounds are their own, else
    /// an anonymous subtype.
    fn object_ty(&self, st: SubtypeId, layout: &Layout, s: Option<ScopeId>) -> TyId {
        let db = self.db;
        let t = db.base(st);
        // Open bounds (an element of a base type): the subtype's own.
        let own_layout = self.layout(st, None, None);
        let layout = if *layout == Layout::Open {
            &own_layout
        } else {
            layout
        };
        let own = own_layout == *layout;
        if own && st == db.types[t].first_subtype {
            return self.base_ty(t);
        }
        let base = self.base_ty(t);
        let mut ty = self.ty_of(st, layout, s);
        ty.base = Some(base);
        ty.kind = vhpiSubtypeDeclK;
        if let (true, Some(n)) = (own, db.subtypes[st].name) {
            ty.name = n.text.to_string();
        } else {
            ty.anonymous = true;
        }
        self.types.borrow_mut().add(ty)
    }

    /// The declaration of type `t`, with the bounds of its first subtype.
    fn base_ty(&self, t: TypeId) -> TyId {
        if let Some(&id) = self.base_tys.borrow().get(&t) {
            return id;
        }
        let fs = self.db.types[t].first_subtype;
        let ty = self.ty_of(fs, &self.layout(fs, None, None), None);
        let id = self.types.borrow_mut().add(ty);
        self.base_tys.borrow_mut().insert(t, id);
        id
    }

    /// Kind, name, structure and ranges of subtype `st` laid out as `layout`, as a type
    /// declaration (the caller makes a subtype of it).
    fn ty_of(&self, st: SubtypeId, layout: &Layout, s: Option<ScopeId>) -> Ty {
        let db = self.db;
        let t = db.base(st);
        let td = &db.types[t];
        let (kind, def, ranges) = match (&td.kind, layout) {
            (TypeKind::Array { indexes, elem }, _) => {
                let (dims, el) = match layout {
                    Layout::Array { dims, elem } => (Some(dims), &**elem),
                    _ => (None, &Layout::Open),
                };
                let ranges = dims.map(|ds| {
                    ds.iter()
                        .zip(indexes.iter())
                        .map(|(d, &ix)| {
                            let int = |v| Bound::Int(v);
                            Range {
                                kind: range_kind(&self.scalar(db.base(ix))),
                                left: int(d.bounds.left),
                                right: int(d.bounds.right),
                                up: d.bounds.dir == Direction::To,
                            }
                        })
                        .collect()
                });
                let def = Def::Array {
                    elem: self.object_ty(*elem, el, s),
                    dims: indexes.len() as u32,
                };
                (vhpiArrayTypeDeclK, def, ranges)
            }
            (TypeKind::Record { elems }, _) => {
                let fields = elems
                    .iter()
                    .enumerate()
                    .map(|(i, el)| {
                        let fl = match layout {
                            Layout::Record(fs) => fs.get(i).map_or(&Layout::Open, |f| &f.1),
                            _ => &Layout::Open,
                        };
                        (el.name.text.to_string(), self.object_ty(el.subtype, fl, s))
                    })
                    .collect();
                (vhpiRecordTypeDeclK, Def::Record(fields), None)
            }
            (k, _) => {
                let sc = self.scalar(t);
                let kind = match k {
                    TypeKind::Enum { .. } => vhpiEnumTypeDeclK,
                    TypeKind::Floating | TypeKind::UniversalReal => vhpiFloatTypeDeclK,
                    TypeKind::Physical { .. } => vhpiPhysTypeDeclK,
                    TypeKind::Access { .. } => vhpiAccessTypeDeclK,
                    TypeKind::File { .. } => vhpiFileTypeDeclK,
                    TypeKind::Protected { .. } => vhpiProtectedTypeDeclK,
                    _ => vhpiIntTypeDeclK,
                };
                let units = match k {
                    TypeKind::Physical { units } => units
                        .iter()
                        .map(|u| (db.decls[u.decl].name.to_string(), u.scale))
                        .collect(),
                    _ => Vec::new(),
                };
                let ranges = self.scalar_range(st, &sc, s).map(|r| vec![r]);
                (kind, Def::Scalar(sc, units), ranges)
            }
        };
        Ty {
            kind,
            name: td.name.text.to_string(),
            anonymous: false,
            base: None,
            def,
            ranges,
        }
    }

    /// The range of scalar subtype `st`: its constraint, else its type's.
    fn scalar_range(&self, st: SubtypeId, sc: &Scalar, s: Option<ScopeId>) -> Option<Range> {
        let db = self.db;
        let td = &db.types[db.base(st)];
        let kind = range_kind(sc);
        match &db.subtypes[st].constraint {
            Constraint::Range(RangeC::Explicit { left, dir, right }) => {
                let bound = |v: ConstValue| match v {
                    ConstValue::Real(r) => Some(Bound::Real(r)),
                    v => v.as_int().map(Bound::Int),
                };
                Some(Range {
                    kind,
                    left: bound(self.fold(*left, s)?)?,
                    right: bound(self.fold(*right, s)?)?,
                    up: *dir == Direction::To,
                })
            }
            _ if st != td.first_subtype => self.scalar_range(td.first_subtype, sc, None),
            _ => match sc {
                Scalar::Enum { lits, .. } => Some(Range {
                    kind,
                    left: Bound::Int(0),
                    right: Bound::Int(lits.len() as i64 - 1),
                    up: true,
                }),
                // ponytail: an integer, floating or physical type without a folded range
                // has no constraint object; the base range of Q2 when a plugin needs it.
                _ => None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn design(src: &str) -> Design {
        let tmp = tempfile::tempdir().expect("scratch dir");
        let p = tmp.path().join("t.vhd");
        std::fs::write(&p, src).expect("write");
        let rev = yavs_sem::LangRev::Vhdl2008;
        let mut db = Db::new(yavs_sem::Options::new(rev, yavs_sem::default_stdlib()));
        assert!(db.analyze_file("work", &p, rev).is_ok());
        assert!(db.diagnostics().is_empty());
        let top = yavs_elab::find_top(&mut db, "work", "top").expect("top");
        let e = yavs_elab::elaborate(&mut db, top, &[], &Default::default()).expect("elab");
        Design::build(&db, &e)
    }

    #[test]
    fn builds_regions_declarations_and_finds_them_by_name() {
        let d = design(
            "entity Sub is generic (G : integer := 3); port (P : out bit_vector(G - 1 downto 0)); end;
             architecture a of Sub is begin end;
             entity Top is end;
             architecture a of Top is
               signal S : bit_vector(2 downto 0);
             begin
               Gen : for k in 1 to 2 generate
                 U : entity work.Sub port map (P => open);
               end generate;
             end;",
        );
        let root = d.node(ROOT);
        assert_eq!((root.kind, root.name.as_str()), (vhpiRootInstK, "Top"));
        let names =
            |ns: &[NodeId]| -> Vec<String> { ns.iter().map(|&n| d.node(n).full.clone()).collect() };
        assert_eq!(names(&root.regions), [":Top:Gen(1)", ":Top:Gen(2)"]);
        let (u, path) = d.lookup("top:gen(2):u", None).expect("instance");
        assert!(path.is_empty());
        let u = d.node(u);
        assert_eq!(u.kind, vhpiCompInstStmtK);
        assert_eq!(names(&u.decls), [":Top:Gen(2):U:G", ":Top:Gen(2):U:P"]);
        // Bounds from the generic's value.
        let (p, path) = d.lookup(":TOP:GEN(2):U:P(0)", None).expect("element");
        assert_eq!(path, [2]);
        assert_eq!(d.elem(p, &path).expect("elem").full, ":Top:Gen(2):U:P(0)");
        assert_eq!(d.lookup(":top:gen(3)", None), None);
        assert_eq!(d.lookup("s(3)", Some(ROOT)), None);
    }

    #[test]
    fn names_upper_case_basic_identifiers_only() {
        assert_eq!(vhpi_name(":Top:g(0):Sig"), ":TOP:G(0):SIG");
        assert_eq!(vhpi_name(r":top:\Ext\Name.f"), r":TOP:\Ext\NAME.F");
        assert_eq!(vhpi_name(r"\a\\b\x"), r"\a\\b\X");
        assert_eq!(vhpi_name("m('a')"), "M('a')");
    }

    #[test]
    fn layout_steps_name_elements_row_major() {
        let dim = |left, right| Dim {
            bounds: Bounds {
                left,
                dir: Direction::Downto,
                right,
            },
            lits: None,
        };
        let l = Layout::Array {
            dims: vec![dim(1, 0), dim(3, 2)],
            elem: Box::new(Layout::Record(vec![
                ("a".into(), Layout::Scalar(Scalar::Int { narrow: false })),
                ("B".into(), Layout::Open),
            ])),
        };
        assert_eq!(l.count(), 4);
        assert_eq!(l.scalars(), None);
        let (e, s, k) = l.step(1).unwrap();
        assert_eq!((s.as_str(), k), ("(1,2)", vhpiIndexedNameK));
        assert_eq!(l.step(4), None);
        assert_eq!(
            e.step(1).map(|x| (x.1, x.2)),
            Some((".B".into(), vhpiSelectedNameK))
        );
        assert_eq!(l.find(&Sel::Index(vec!["0".into(), "3".into()])), Some(2));
        assert_eq!(l.find(&Sel::Index(vec!["2".into(), "3".into()])), None);
        assert_eq!(e.find(&Sel::Field("B".into())), Some(1));
    }
}
