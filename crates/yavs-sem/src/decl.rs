//! Declarations: the `Decl` entity and its kinds (sem.md 1.3, 2.10, 6.2), and
//! the analysis of declarative items.

use yavs_diag::Span;
use yavs_syntax::Symbol;
use yavs_syntax::ast::{Ident, SignalKind};
use yavs_syntax::token::Kw;

use crate::LangRev;
use crate::hir::Mode;
use crate::ids::{
    DeclId, ExprId, InstId, LibId, ScopeId, StmtId, SubtypeId, TypeId, UnitId, ViewId,
};
use crate::implicit::ImplicitOp;
use crate::types::BoundsFrom;

/// The name of a declaration, original spelling kept (rule 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Designator {
    Ident(Ident),
    /// Operator symbol as written between the quotes (`"AND"`), plus its folded key.
    Operator {
        text: Symbol,
        key: Symbol,
    },
    Char(char),
}

impl Designator {
    /// The folded key the declaration is looked up by. Operators are keyed with
    /// their quotes (`"and"`), characters with theirs (`'a'`), so neither can
    /// collide with an identifier.
    pub fn key(&self) -> Symbol {
        match *self {
            Designator::Ident(i) => i.key,
            Designator::Operator { key, .. } => key,
            Designator::Char(c) => char_key(c),
        }
    }

    pub fn operator(text: &str) -> Self {
        Designator::Operator {
            text: Symbol::intern(text),
            key: operator_key(text),
        }
    }
}

impl std::fmt::Display for Designator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Designator::Ident(i) => write!(f, "{}", i.text),
            Designator::Operator { text, .. } => write!(f, "\"{text}\""),
            Designator::Char(c) => write!(f, "'{c}'"),
        }
    }
}

/// Lookup key of operator symbol `op` (without quotes).
pub fn operator_key(op: &str) -> Symbol {
    thread_local! {
        static KEYS: std::cell::RefCell<rustc_hash::FxHashMap<Box<str>, Symbol>> = Default::default();
    }
    if op.bytes().any(|b| b.is_ascii_uppercase()) {
        return operator_key(&op.to_ascii_lowercase());
    }
    KEYS.with_borrow_mut(|m| {
        if let Some(&k) = m.get(op) {
            return k;
        }
        let k = Symbol::intern(&format!("\"{op}\""));
        m.insert(op.into(), k);
        k
    })
}

/// Whether `text` (between the quotes) is a VHDL operator symbol (9.2).
pub fn is_operator_symbol(text: &str) -> bool {
    matches!(
        text.to_ascii_lowercase().as_str(),
        "and"
            | "or"
            | "nand"
            | "nor"
            | "xor"
            | "xnor"
            | "not"
            | "abs"
            | "??"
            | "="
            | "/="
            | "<"
            | "<="
            | ">"
            | ">="
            | "?="
            | "?/="
            | "?<"
            | "?<="
            | "?>"
            | "?>="
            | "+"
            | "-"
            | "*"
            | "/"
            | "mod"
            | "rem"
            | "**"
            | "&"
            | "sll"
            | "srl"
            | "sla"
            | "sra"
            | "rol"
            | "ror"
    )
}

/// Lookup key of character literal `c`.
pub fn char_key(c: char) -> Symbol {
    thread_local! {
        static LATIN1: std::cell::RefCell<[Option<Symbol>; 256]> = const { std::cell::RefCell::new([None; 256]) };
    }
    let Some(slot) = usize::try_from(u32::from(c)).ok().filter(|&i| i < 256) else {
        return Symbol::intern(&format!("'{c}'"));
    };
    LATIN1.with_borrow_mut(|t| *t[slot].get_or_insert_with(|| Symbol::intern(&format!("'{c}'"))))
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Decl {
    pub name: Designator,
    pub kind: DeclKind,
    pub scope: ScopeId,
    pub span: Span,
    /// Implicitly declared (predefined ops, range records, labels, ...).
    pub implicit: bool,
    /// First revision that has this declaration (sem.md 0.1); `V2008` unless std/ieee says otherwise.
    pub since: LangRev,
}

impl Decl {
    /// The identifier's span when the designator is one, else the whole declaration.
    pub fn name_span(&self) -> Span {
        match self.name {
            Designator::Ident(i) => i.span,
            _ => self.span,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum DeclKind {
    Library(LibId),
    Unit(UnitId),
    Type(TypeId),
    Subtype(SubtypeId),
    /// Constant, signal, variable, shared variable, file, interface objects.
    Object(Object),
    Subprogram(Subprogram),
    EnumLit {
        ty: TypeId,
        pos: u32,
    },
    PhysUnit {
        ty: TypeId,
        index: u32,
    },
    Alias(Alias),
    /// Attribute declaration.
    Attribute {
        ty: SubtypeId,
    },
    Component(Component),
    View(ViewId),
    PackageInst(InstId),
    /// A package declared in a declarative part (4.7, 2008).
    Package {
        scope: ScopeId,
        /// Its generic list: an uninstantiated generic package, objects only in its instances.
        generics: Option<Box<[DeclId]>>,
        /// Its body region once analyzed.
        body: Option<ScopeId>,
    },
    /// A generic type (its `TypeKind::Generic` carries the class; a generic array type is
    /// a `TypeKind::Array`).
    GenericType(TypeId),
    /// Formal generic subprogram (`Subprogram.body == Body::Interface`).
    GenericSubprogram(Subprogram),
    /// Formal generic package.
    GenericPackage(InstId),
    Group {
        template: DeclId,
        /// The named entities of the constituent list, in order (6.10).
        constituents: Box<[DeclId]>,
    },
    /// Entity class entries; `true` when followed by `<>` (6.9).
    GroupTemplate(Box<[(Kw, bool)]>),
    /// Statement labels are implicitly declared.
    Label(StmtId),
    /// Only reachable through selection, never in `Scope::names`.
    Element {
        record: TypeId,
        index: u32,
    },
    /// A declaration whose analysis failed; silent from then on (8.2).
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ObjClass {
    Constant,
    Signal,
    Variable,
    SharedVariable,
    File,
}

/// What kind of declaration made the object.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ObjRole {
    Plain,
    /// `constant c : t;` in a package, completed in the body.
    Deferred,
    Generic,
    Port,
    Param,
    LoopParam,
    GenerateParam,
    /// `s'delayed(t)`, `s'stable(t)`, ... (sem.md 6.3).
    ImplicitSignal,
    /// The implicit `GUARD` signal of a guarded block.
    Guard,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Object {
    pub class: ObjClass,
    pub role: ObjRole,
    pub mode: Option<Mode>,
    pub subtype: SubtypeId,
    pub default: Option<ExprId>,
    pub bounds: BoundsFrom,
    pub signal_kind: Option<SignalKind>,
    /// File declarations: `open kind is name`.
    pub file_open: Option<(Option<ExprId>, ExprId)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SubprogramKind {
    Function { pure: bool },
    Procedure,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Subprogram {
    pub kind: SubprogramKind,
    pub params: Box<[Param]>,
    pub ret: Option<SubtypeId>,
    /// Uninstantiated subprogram (2008): its generic declarations.
    pub generics: Option<Box<[DeclId]>>,
    /// The region holding generics and parameters (and, later, the body's declarations).
    pub scope: Option<ScopeId>,
    pub body: Body,
    /// Long readable mangled name (sem.md 5.3).
    pub mangled: Symbol,
    /// The result's type mark as written (`integer`, not `INTEGER`), for messages (8.3).
    pub ret_mark: Option<Span>,
    /// A pure function whose body reads or calls something outside it (E1830, a warning under
    /// `--relaxed`): its result is not a function of its arguments, so no process that calls it is
    /// skipped when the signals it reads have no event (`Db::pure_comb_reads`).
    pub impure_reads: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Param {
    pub name: Ident,
    pub class: ObjClass,
    pub mode: Mode,
    pub subtype: SubtypeId,
    pub default: Option<ExprId>,
    /// The parameter's object declaration; `None` for implicit operations.
    pub decl: Option<DeclId>,
    /// The type mark as written, for messages in the user's spelling (8.3); `None` for anything
    /// but a parameter of a subprogram the user wrote.
    pub mark: Option<Span>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum Body {
    /// Spec analyzed, body not yet seen.
    Pending,
    /// VHDL body: declarations in `scope`, statements in order.
    Hir {
        scope: ScopeId,
        stmts: Box<[StmtId]>,
    },
    /// Predefined operation.
    Implicit(ImplicitOp),
    /// Explicit declaration without body in std (NATIVE.md section 1): the native symbol.
    Native(Symbol),
    /// `'FOREIGN` attribute (VHPIDIRECT, M3): the attribute value.
    Foreign(ExprId),
    Instance(InstId),
    /// Formal generic subprogram.
    Interface,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum Alias {
    Object {
        /// A static name.
        target: ExprId,
        subtype: Option<SubtypeId>,
    },
    /// Type, subtype, subprogram (with signature), enum literal, unit, ...
    NonObject { target: DeclId },
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Component {
    pub scope: ScopeId,
    pub generics: Box<[DeclId]>,
    pub ports: Box<[DeclId]>,
}

impl DeclKind {
    /// Subprograms and enumeration literals are overloadable (sem.md 1.6);
    /// aliases of those are classified by the caller through their target.
    pub fn is_overloadable(&self) -> bool {
        matches!(
            self,
            DeclKind::Subprogram(_) | DeclKind::GenericSubprogram(_) | DeclKind::EnumLit { .. }
        )
    }

    pub fn subprogram(&self) -> Option<&Subprogram> {
        match self {
            DeclKind::Subprogram(s) | DeclKind::GenericSubprogram(s) => Some(s),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Analysis of declarative parts.

use yavs_syntax::ast;

use crate::db::Db;

impl Db {
    /// Analyze a declarative part in order. Each item's errors are reported
    /// independently; the enclosing unit fails if any were.
    pub(crate) fn decls_in(&mut self, scope: ScopeId, decls: &[ast::Decl]) {
        for d in decls {
            if self.decl_item(scope, d).is_err() {
                self.poison_names(scope, d);
            }
        }
        self.check_incomplete_completed(scope);
        let end = decls.last().map_or(Span::default(), decl_span);
        use crate::scope::ScopeKind as K;
        match self.scopes[scope].kind {
            // An entity's subprograms may get their bodies in an architecture (12.1).
            K::Package | K::Protected | K::ProtectedBody | K::Instance | K::Entity => {}
            K::Architecture => {
                if let Some(e) = self.entity_of_arch(scope) {
                    self.check_subprogram_bodies(e, end);
                }
                self.check_subprogram_bodies(scope, end);
            }
            K::PackageBody => {
                // A package's subprograms get their bodies in the package body (4.8).
                if let Some(pkg) = self.body_package(scope) {
                    self.check_subprogram_bodies(pkg, end);
                    self.check_deferred_completed(pkg, scope, end);
                }
                self.check_subprogram_bodies(scope, end);
            }
            _ => self.check_subprogram_bodies(scope, end),
        }
        // Protected types of a package get their bodies in the package body (unit.rs).
        if !matches!(
            self.scopes[scope].kind,
            crate::scope::ScopeKind::Package | crate::scope::ScopeKind::Protected
        ) {
            let end = decls.last().map_or(Span::default(), decl_span);
            let _ = self.check_protected_bodies(scope, end);
        }
    }

    /// The entity region of an architecture region (its parent is the context region,
    /// whose parent is the entity).
    pub(crate) fn entity_of_arch(&self, arch: ScopeId) -> Option<ScopeId> {
        self.scopes[arch]
            .parent
            .and_then(|c| self.scopes[c].parent)
            .filter(|&e| self.scopes[e].kind == crate::scope::ScopeKind::Entity)
    }

    /// `E1230`: a subprogram declared in this region has no body in it (4.2, 4.3), also in a
    /// package declared here without a body (which then also gets its `W1501`).
    fn check_subprogram_bodies(&mut self, scope: ScopeId, end: Span) {
        for (d, pkg) in self.bodiless_packages(scope) {
            self.check_subprogram_bodies(pkg, end);
            let _ = self.check_protected_bodies(pkg, end);
            if let DeclKind::Package {
                generics: Some(g), ..
            } = &self.decls[d].kind
            {
                let g = g.clone();
                self.report_unused_generics(&g);
            }
        }
        let missing: Vec<DeclId> = self.scopes[scope]
            .order
            .iter()
            .copied()
            .filter(|&d| !self.decls[d].implicit && matches!(&self.decls[d].kind, DeclKind::Subprogram(s) if matches!(s.body, Body::Pending)))
            .collect();
        for d in missing {
            let what = match &self.decls[d].kind {
                DeclKind::Subprogram(s) if matches!(s.kind, SubprogramKind::Procedure) => {
                    "procedure"
                }
                _ => "function",
            };
            let name = self.decls[d].name;
            self.report(
                yavs_diag::Diagnostic::error(
                    crate::diag::MISSING_BODY,
                    format!("{what} `{name}` has no body"),
                )
                .with_primary(self.decls[d].name_span(), "declared here")
                .with_secondary(end, "the region ends here without its body"),
            );
        }
    }

    /// `E1230`: a deferred constant of the package has no full declaration in its body (4.8).
    fn check_deferred_completed(&mut self, pkg: ScopeId, body: ScopeId, end: Span) {
        let missing: Vec<DeclId> = self.scopes[pkg]
            .order
            .iter()
            .copied()
            .filter(|&d| {
                let decl = &self.decls[d];
                !decl.implicit
                    && matches!(&decl.kind, DeclKind::Object(o) if o.role == ObjRole::Deferred)
                    && !self.scopes[body].names.get(&decl.name.key()).is_some_and(|v| {
                        v.iter().any(|&f| matches!(&self.decls[f].kind, DeclKind::Object(o) if o.class == ObjClass::Constant && o.default.is_some()))
                    })
            })
            .collect();
        for d in missing {
            let name = self.decls[d].name;
            self.report(
                yavs_diag::Diagnostic::error(
                    crate::diag::MISSING_BODY,
                    format!(
                        "deferred constant `{name}` has no full declaration in the package body"
                    ),
                )
                .with_primary(self.decls[d].name_span(), "declared here")
                .with_secondary(end, "the package body ends here without it"),
            );
        }
    }

    /// `E1226`: an initial value in a package declaration names one of its deferred constants,
    /// whose value is not known before the package body (6.4.2.2, #809). Uses in the body are
    /// `check_deferred_read`'s.
    fn check_deferred_uses(&mut self, scope: ScopeId, e: ExprId) {
        if self.scopes[scope].kind != crate::scope::ScopeKind::Package {
            return;
        }
        let mut early = Vec::new();
        self.walk_expr(e, &mut |x| {
            if let crate::hir::ExprKind::Obj(d) = self.exprs[x].kind
                && self.decls[d].scope == scope
                && matches!(&self.decls[d].kind, DeclKind::Object(o) if o.role == ObjRole::Deferred)
            {
                early.push((d, self.exprs[x].span));
            }
        });
        for (d, span) in early {
            self.report_deferred_early(d, span);
        }
    }

    /// `E1226`: a name in the body of `d`'s package, or in a subprogram there, denotes the
    /// deferred constant `d` before its full declaration (6.4.2.2): the subprogram may run
    /// before the value exists (#846). Interface defaults are exempt.
    pub(crate) fn check_deferred_read(&mut self, scope: ScopeId, d: DeclId, span: Span) {
        if self.interface_default
            || !matches!(&self.decls[d].kind, DeclKind::Object(o) if o.role == ObjRole::Deferred)
        {
            return;
        }
        let pkg = self.decls[d].scope;
        let mut cur = Some(scope);
        while let Some(sc) = cur {
            if self.body_package(sc) == Some(pkg) {
                let full = self.scopes[sc].names.get(&self.decls[d].name.key()).is_some_and(|v| {
                    v.iter().any(|&f| matches!(&self.decls[f].kind, DeclKind::Object(o) if o.class == ObjClass::Constant))
                });
                if !full {
                    self.report_deferred_early(d, span);
                }
                return;
            }
            cur = self.scopes[sc].parent;
        }
    }

    /// The default expression of an interface object (`Db::interface_default`).
    fn interface_default_expr(&mut self, scope: ScopeId, e: &ast::Expr, st: SubtypeId) -> ExprId {
        let outer = std::mem::replace(&mut self.interface_default, true);
        let x = self.expr_in_subtype(scope, e, st);
        self.interface_default = outer;
        x
    }

    fn report_deferred_early(&mut self, d: DeclId, span: Span) {
        let name = self.decls[d].name;
        self.report(
            yavs_diag::Diagnostic::error(
                crate::diag::DEFERRED_EARLY,
                format!("deferred constant `{name}` is used before its full declaration"),
            )
            .with_primary(span, "its value is not known here")
            .with_secondary(self.decls[d].name_span(), "deferred here")
            .with_note("LRM 6.4.2.2: before the end of the full declaration, a deferred constant may appear only in the default expression of a local generic, port or formal parameter"),
        );
    }

    /// `E1227`: the subtype indication of the full declaration of deferred constant `n` does
    /// not conform to the deferred one (6.4.2.2): another type mark, or other static bounds.
    fn check_full_conforms(&mut self, scope: ScopeId, n: &Ident, st: SubtypeId, span: Span) {
        let Some(pkg) = self.body_package(scope) else {
            return;
        };
        if self.scopes[scope].kind != crate::scope::ScopeKind::PackageBody {
            return;
        }
        let Some(d) = self.scopes[pkg].names.get(&n.key).and_then(|v| {
            v.iter().copied().find(
                |&d| matches!(&self.decls[d].kind, DeclKind::Object(o) if o.role == ObjRole::Deferred),
            )
        }) else {
            return;
        };
        let DeclKind::Object(o) = &self.decls[d].kind else {
            return;
        };
        let deferred = o.subtype;
        let mark = |db: &Self, s: SubtypeId| {
            if db.subtypes[s].name.is_some() {
                Some(s)
            } else {
                db.subtypes[s].parent
            }
        };
        let dims = match &self.types[self.base(st)].kind {
            crate::types::TypeKind::Array { indexes, .. } => indexes.len(),
            _ => 1,
        };
        let sr = crate::expr::aggregate::subtype_range;
        let same = mark(self, st) == mark(self, deferred)
            && (0..dims).all(|k| match (sr(self, st, k), sr(self, deferred, k)) {
                (Some(a), Some(b)) => (a.left, a.dir, a.right) == (b.left, b.dir, b.right),
                _ => true,
            });
        if !same {
            let (want, have) = (self.subtype_name(deferred), self.subtype_name(st));
            self.report(
                yavs_diag::Diagnostic::error(
                    crate::diag::DEFERRED_SUBTYPE,
                    format!(
                        "the full declaration of `{}` has subtype `{have}`, the deferred constant `{want}`",
                        n.text
                    ),
                )
                .with_primary(span, "does not conform")
                .with_secondary(self.decls[d].name_span(), "deferred here")
                .with_note("LRM 6.4.2.2: the subtype indications of the deferred and the full declaration must conform"),
            );
        }
    }

    /// `E1220`: an incomplete type declared in this region was never completed (5.4.2).
    fn check_incomplete_completed(&mut self, scope: ScopeId) {
        let open: Vec<DeclId> = self.scopes[scope]
            .order
            .iter()
            .copied()
            .filter(|&d| matches!(self.decls[d].kind, DeclKind::Type(t) if matches!(self.types[t].kind, crate::types::TypeKind::Incomplete { completion: None })))
            .collect();
        for d in open {
            let name = self.decls[d].name;
            self.report(
                yavs_diag::Diagnostic::error(
                    crate::diag::INCOMPLETE,
                    format!("incomplete type `{name}` is never completed"),
                )
                .with_primary(
                    self.decls[d].name_span(),
                    "declared here without a full declaration",
                )
                .with_help("declare the full type later in the same declarative part"),
            );
        }
    }

    /// After a failed declaration, declare its names as poison so later uses stay silent (8.2).
    fn poison_names(&mut self, scope: ScopeId, d: &ast::Decl) {
        use ast::Decl as D;
        let names: Vec<Designator> = match d {
            D::Type(t) => vec![Designator::Ident(t.name)],
            D::Subtype(s) => vec![Designator::Ident(s.name)],
            D::Object(o) => o.names.iter().map(|&n| Designator::Ident(n)).collect(),
            D::File(f) => f.names.iter().map(|&n| Designator::Ident(n)).collect(),
            D::Attribute(a) => vec![Designator::Ident(a.name)],
            D::Component(c) => vec![Designator::Ident(c.name)],
            D::Group(g) => vec![Designator::Ident(g.name)],
            D::GroupTemplate(g) => vec![Designator::Ident(g.name)],
            D::Alias(a) => match &a.designator {
                ast::Designator::Ident(i) => vec![Designator::Ident(*i)],
                _ => vec![],
            },
            D::Subprogram(s) | D::SubprogramBody(ast::SubprogramBody { spec: s, .. }) => {
                match &s.designator {
                    ast::Designator::Ident(i) => vec![Designator::Ident(*i)],
                    _ => vec![],
                }
            }
            _ => vec![],
        };
        for n in names {
            if self.scopes[scope]
                .names
                .get(&n.key())
                .is_some_and(|v| !v.is_empty())
            {
                continue;
            }
            let span = decl_span(d);
            let decl = self.new_decl(scope, n, DeclKind::Error, span);
            self.declare(scope, decl);
        }
    }

    fn decl_item(&mut self, scope: ScopeId, d: &ast::Decl) -> Result<(), crate::Reported> {
        use ast::Decl as D;
        match d {
            D::Type(t) => self.type_decl(scope, t),
            D::Subtype(s) => self.subtype_decl(scope, s),
            D::Object(o) => self.object_decl(scope, o),
            D::File(f) => self.file_decl(scope, f),
            D::Subprogram(s) => {
                let d = self.subprogram_spec(scope, s)?;
                let d = self.header_instance(scope, d, s)?;
                self.declare(scope, d);
                Ok(())
            }
            D::Attribute(a) => {
                let ty = self.type_mark(scope, &a.type_mark)?;
                crate::attr::declare_attribute(self, scope, a.name, ty, a.span).map(|_| ())
            }
            D::AttributeSpec(sp) => self.attribute_spec(scope, sp),
            D::Use(u) => {
                self.use_clause(scope, u);
                Ok(())
            }
            D::SubprogramBody(b) => self.subprogram_body(scope, b),
            D::Alias(a) => self.alias_decl(scope, a),
            D::Component(_) | D::Disconnect(_) | D::ConfigSpec(_)
                if let Some(s) = self.sequential_owner(scope) =>
            {
                // 4.3, 11.3: not process or subprogram declarative items (#808, #824).
                let what = match d {
                    D::Component(_) => "a component declaration",
                    D::Disconnect(_) => "a disconnection specification",
                    _ => "a configuration specification",
                };
                let owner = if self.scopes[s].kind == crate::scope::ScopeKind::Process {
                    "process"
                } else {
                    "subprogram"
                };
                Err(self.error(
                    yavs_diag::Diagnostic::error(
                        crate::diag::NOT_DECL_ITEM,
                        format!("{what} is not allowed in a {owner}"),
                    )
                    .with_primary(decl_span(d), format!("inside a {owner}"))
                    .with_help("move it to the enclosing architecture, block or package"),
                ))
            }
            D::Component(c) => self.component_decl(scope, c),
            D::SubprogramInstance(x) => self.subprogram_instance(scope, x),
            D::Package(x) => self.nested_package(scope, x),
            D::PackageBody(x) => self.nested_package_body(scope, x),
            D::PackageInstance(x) => self.package_instance_decl(scope, x),
            D::Group(x) => self.group_decl(scope, x),
            D::GroupTemplate(x) => self.group_template(scope, x),
            D::Disconnect(x) => self.disconnect_spec(scope, x),
            D::ConfigSpec(x) => self.config_spec(scope, x),
            D::View(x) => {
                let record = self.subtype_indication(scope, &x.subtype)?;
                let d = self.declare_view(scope, x, record, &mut |db, n| db.view_name(scope, n))?;
                self.declare(scope, d);
                Ok(())
            }
        }
    }

    /// `constant`, `signal`, `variable`, `shared variable` declarations (6.4.2).
    fn object_decl(&mut self, scope: ScopeId, o: &ast::ObjectDecl) -> Result<(), crate::Reported> {
        let subtype = self.subtype_indication(scope, &o.subtype)?;
        self.check_generic_protected(subtype, o.subtype.span)?;
        let class = match o.class {
            ast::ObjectClass::Constant => ObjClass::Constant,
            ast::ObjectClass::Signal => ObjClass::Signal,
            ast::ObjectClass::Variable => ObjClass::Variable,
            ast::ObjectClass::SharedVariable => ObjClass::SharedVariable,
        };
        let first = o.names.first().map_or(o.subtype.span, |n| n.span);
        if matches!(class, ObjClass::Signal | ObjClass::SharedVariable) {
            self.check_not_sequential(scope, first, None)?;
        }
        // 6.4.2.4: a variable outside a process, subprogram or protected type (2019 private
        // variables) is shared (#811).
        if class == ObjClass::Variable
            && self.sequential_owner(scope).is_none()
            && !matches!(
                self.scopes[scope].kind,
                crate::scope::ScopeKind::ProtectedBody | crate::scope::ScopeKind::Protected
            )
        {
            return Err(self.error(
                yavs_diag::Diagnostic::error(
                    crate::diag::NON_SHARED_VARIABLE,
                    "a variable declared here must be a shared variable",
                )
                .with_primary(first, "not in a process, subprogram or protected type body")
                .with_help("write `shared variable`, or declare the variable in a process"),
            ));
        }
        match class {
            ObjClass::Constant => self.check_not_access_or_file("constant", subtype, first)?,
            ObjClass::Signal => self.check_not_access_or_file("signal", subtype, first)?,
            _ => {}
        }
        // Reported, but the names are still declared so their uses do not cascade.
        let _ = match class {
            ObjClass::Constant => self.check_not_protected("constant", subtype, first),
            ObjClass::Signal => self.check_not_protected("signal", subtype, first),
            ObjClass::SharedVariable => self
                .check_shared_variable(subtype, first)
                .and_then(|()| self.check_shared_body_elaborated(scope, subtype, first)),
            _ => Ok(()),
        };
        let default = o
            .default
            .as_ref()
            .map(|e| self.expr_in_subtype(scope, e, subtype));
        if let Some(e) = default {
            self.check_deferred_uses(scope, e);
        }
        if class == ObjClass::Constant && default.is_some() {
            for n in &o.names {
                self.check_full_conforms(scope, n, subtype, o.subtype.span);
            }
        }
        let full = self.constrainedness(subtype) == crate::types::Constrainedness::Full;
        let bounds = if full {
            BoundsFrom::Subtype
        } else {
            BoundsFrom::Initializer
        };
        // 2019 signals and variables, like constants, take missing bounds from their
        // initial value (6.4.2.3, 6.4.2.4; LCS-2016-019).
        let from_init = default.is_some() && self.file_rev() >= LangRev::Vhdl2019;
        // 6.4.2.2: a constant without a value is deferred, and only a package declaration
        // may hold one; without a value an unconstrained one has no bounds either.
        if class == ObjClass::Constant
            && default.is_none()
            && self.scopes[scope].kind != crate::scope::ScopeKind::Package
        {
            let name = o.names.first().map_or(o.subtype.span, |n| n.span);
            self.report(
                yavs_diag::Diagnostic::error(
                    crate::diag::DEFERRED_OUTSIDE_PACKAGE,
                    match o.names.first() {
                        Some(n) => format!("constant `{}` needs an initial value", n.text),
                        None => "a constant needs an initial value".into(),
                    },
                )
                .with_primary(name, "declared here")
                .with_note("only a package declaration may hold a deferred constant (LRM 6.4.2.2)"),
            );
        }
        if !full && class != ObjClass::Constant && !from_init {
            let what = if class == ObjClass::Signal {
                "signal"
            } else {
                "variable"
            };
            let name = o.names.first().map_or(o.subtype.span, |n| n.span);
            self.report(
                yavs_diag::Diagnostic::error(
                    crate::diag::NOT_FULLY_CONSTRAINED,
                    match o.names.first() {
                        Some(n) => format!("{what} `{}` needs a fully constrained subtype", n.text),
                        None => format!("a {what} needs a fully constrained subtype"),
                    },
                )
                .with_primary(name, "declared here")
                .with_secondary(o.subtype.span, "not fully constrained")
                .with_help(format!(
                    "give the {what} a constrained subtype, e.g. `bit_vector(7 downto 0)`"
                )),
            );
        }
        let role = match (class, &default) {
            (ObjClass::Constant, None) => ObjRole::Deferred,
            _ => ObjRole::Plain,
        };
        for &name in &o.names {
            let decl = self.new_decl(
                scope,
                Designator::Ident(name),
                DeclKind::Object(Object {
                    class,
                    role: role.clone(),
                    mode: None,
                    subtype,
                    default,
                    bounds,
                    signal_kind: o.signal_kind,
                    file_open: None,
                }),
                name.span,
            );
            self.declare(scope, decl);
        }
        Ok(())
    }

    /// `alias a [: st] is name [signature];` (6.6), through `alias.rs`.
    fn alias_decl(&mut self, scope: ScopeId, a: &ast::AliasDecl) -> Result<(), crate::Reported> {
        use crate::expr::names::NameRes;
        let (name, sig) = match &a.name.kind {
            ast::NameKind::Signature { prefix, signature } => (prefix.as_ref(), Some(signature)),
            _ => (&a.name, None),
        };
        let subtype = match &a.subtype {
            Some(si) => Some(self.subtype_indication(scope, si)?),
            None => None,
        };
        // `alias v2 is v'converse` (2019): an alias of the converse mode view. The alias
        // denotes the converse view itself.
        // ponytail: declared as a view, not an Alias; an alias-of-view marker is needed only
        // if something must tell the two apart.
        if let ast::NameKind::Attribute { attr, .. } = &name.kind
            && attr.key.as_str() == "converse"
        {
            let r = self.view_name(scope, name)?;
            let v = self.effective_view(r);
            let ast::Designator::Ident(id) = a.designator else {
                return Err(self.error(
                    yavs_diag::Diagnostic::error(
                        crate::diag::BAD_ALIAS,
                        "an alias of a mode view must be named by an identifier",
                    )
                    .with_primary(a.span, "operator or character designator"),
                ));
            };
            let d = self.new_decl(scope, Designator::Ident(id), DeclKind::View(v), id.span);
            self.declare(scope, d);
            return Ok(());
        }
        let res = match &name.kind {
            ast::NameKind::Simple(_)
            | ast::NameKind::Selected { .. }
            | ast::NameKind::Operator(_)
            | ast::NameKind::Character(_) => self.classify(scope, name),
            // Indexed or sliced names denote parts of objects (6.6.2), external names
            // objects (8.7).
            _ => NameRes::Value,
        };
        // `alias m is obj.m [sig];` (2019 5.6.2, 6.6.3): a method of a protected object, such
        // as a private variable or a shared variable; calls through the alias run on `obj`.
        let mut via = None;
        let res = match (&res, &name.kind) {
            (
                NameRes::Value,
                ast::NameKind::Selected {
                    prefix: obj,
                    suffix,
                },
            ) => {
                let key = crate::unit::suffix_key(suffix);
                let ms: Vec<DeclId> = match (self.prefix_types(scope, obj), key) {
                    (Ok(ts), Some(k)) => ts
                        .iter()
                        .filter_map(|&t| self.method_target(t))
                        .flat_map(|(pt, _)| self.method_decls(pt, k))
                        .collect(),
                    _ => Vec::new(),
                };
                if ms.is_empty() {
                    res
                } else {
                    match self.classify(scope, obj) {
                        NameRes::One(v) if self.object_type(v).is_some() => via = Some(v),
                        _ => {
                            return Err(self.unsupported(
                                name.span,
                                "aliases of methods of objects not denoted by a simple or expanded name",
                                "S7",
                            ));
                        }
                    }
                    NameRes::Overloads(ms)
                }
            }
            _ => res,
        };
        if let NameRes::Value = res {
            let target = self.object_name_expr(scope, name)?;
            let alias = self.object_alias(scope, a, target, subtype)?;
            self.declare(scope, alias);
            return Ok(());
        }
        let cands = match res {
            NameRes::One(d) if self.object_type(d).is_some() => {
                let target = self.object_ref(d, name.span);
                let alias = self.object_alias(scope, a, target, subtype)?;
                self.declare(scope, alias);
                return Ok(());
            }
            NameRes::One(d) => vec![d],
            NameRes::Overloads(ds) => ds,
            // `alias a : t is obj.field;`: an object name that is not a plain object.
            NameRes::Value => {
                return Err(self.unsupported(name.span, "aliases of selected object names", "S3"));
            }
            other => return Err(self.report_name(scope, other, name.span, "an aliased name")),
        };
        let sig = match sig {
            Some(s) => Some(self.signature_of(scope, s)?),
            None => None,
        };
        let decls = self.nonobject_alias(scope, a, &cands, sig.as_ref())?;
        for d in decls {
            if let Some(pv) = via {
                self.forwarding.insert(d, pv);
            }
            self.declare(scope, d);
        }
        Ok(())
    }

    /// The external signal name `d` aliases when it is `alias a is <<signal ...>>` (a whole
    /// external name): elab binds it at the alias, lowering gives it a signal slot (#19).
    pub fn external_signal_alias(&self, d: DeclId) -> Option<crate::ids::ExternalId> {
        let DeclKind::Alias(Alias::Object { target, .. }) = &self.decls[d].kind else {
            return None;
        };
        match self.exprs[*target].kind {
            crate::hir::ExprKind::External(x)
                if self.externals[x].class == yavs_syntax::ast::ExternalClass::Signal =>
            {
                Some(x)
            }
            _ => None,
        }
    }

    /// A name that must denote (a part of) an object, such as the target of an
    /// object alias: `r.f`, `a(i)`, `v(0 to 3)`. Function calls do not qualify.
    fn object_name_expr(
        &mut self,
        scope: ScopeId,
        n: &ast::Name,
    ) -> Result<crate::ids::ExprId, crate::Reported> {
        let saved = std::mem::take(&mut self.memo);
        let r = match self.name_cands(scope, n) {
            crate::expr::CandSet::Types(is) => {
                let objs: Vec<_> = is
                    .iter()
                    .copied()
                    .filter(|i| !matches!(i.via, crate::expr::Via::Call(_)))
                    .collect();
                match objs.as_slice() {
                    [i] => {
                        let e = self.build_name(scope, n, *i, n.span);
                        if self.is_poison_expr(e) {
                            Err(crate::Reported)
                        } else {
                            Ok(e)
                        }
                    }
                    _ if is.iter().any(|i| i.ty == self.poison.ty) => Err(crate::Reported),
                    _ => Err(self.error(
                        yavs_diag::Diagnostic::error(
                            crate::diag::NOT_AN_OBJECT,
                            "the aliased name does not denote an object",
                        )
                        .with_primary(
                            n.span,
                            if objs.is_empty() {
                                "not an object"
                            } else {
                                "several objects fit"
                            },
                        )
                        .with_note("LRM 6.6.2"),
                    )),
                }
            }
            crate::expr::CandSet::Class(_) | crate::expr::CandSet::Alloc(_) => Err(self.error(
                yavs_diag::Diagnostic::error(
                    crate::diag::NOT_AN_OBJECT,
                    "the aliased name does not denote an object",
                )
                .with_primary(n.span, "not an object"),
            )),
        };
        self.memo = saved;
        r
    }

    /// Resolve the type marks of a signature (4.5.3).
    pub(crate) fn signature_of(
        &mut self,
        scope: ScopeId,
        s: &ast::Signature,
    ) -> Result<crate::alias::Signature, crate::Reported> {
        let mut params = Vec::new();
        for p in &s.params {
            params.push(self.type_mark(scope, p)?);
        }
        let ret = match &s.ret {
            Some(r) => Some(self.type_mark(scope, r)?),
            None => None,
        };
        Ok(crate::alias::Signature::new(self, &params, ret, s.span))
    }

    /// `attribute a of names : class is value;` (7.2), through `attr.rs`.
    fn attribute_spec(
        &mut self,
        scope: ScopeId,
        sp: &ast::AttributeSpec,
    ) -> Result<(), crate::Reported> {
        let attr = match self.lookup(scope, sp.attr.key) {
            crate::scope::Visible::One(d)
                if matches!(self.decls[self.entity(d)].kind, DeclKind::Attribute { .. }) =>
            {
                d
            }
            crate::scope::Visible::One(d) if matches!(self.decls[d].kind, DeclKind::Error) => {
                return Err(crate::Reported);
            }
            crate::scope::Visible::None => return Err(self.undeclared(scope, sp.attr)),
            crate::scope::Visible::Hidden(c) => return Err(self.conflict(sp.attr, &c)),
            _ => {
                return Err(self.error(
                    yavs_diag::Diagnostic::error(
                        crate::diag::UNKNOWN_ATTRIBUTE,
                        format!("`{}` is not an attribute", sp.attr.text),
                    )
                    .with_primary(
                        sp.attr.span,
                        "expected the name of an attribute declaration",
                    ),
                ));
            }
        };
        let Some(st) = crate::attr::spec_subtype(self, attr) else {
            return Err(crate::Reported);
        };
        let value = self.expr_in_subtype(scope, &sp.value, st);
        if self.is_poison_expr(value) {
            return Err(crate::Reported);
        }
        let mut sig = |db: &mut Db, cands: &[DeclId], s: &ast::Signature| {
            let s = db.signature_of(scope, s)?;
            db.match_signature(cands, &s)
        };
        crate::attr::specify(self, scope, sp, attr, value, &mut sig).map(|_| ())
    }

    /// `file f : t open k is "name";` (6.4.2.5).
    fn file_decl(&mut self, scope: ScopeId, f: &ast::FileDecl) -> Result<(), crate::Reported> {
        let subtype = self.subtype_indication(scope, &f.subtype)?;
        let base = self.base(subtype);
        if !matches!(
            self.types[base].kind,
            crate::types::TypeKind::File { .. } | crate::types::TypeKind::Error
        ) {
            return Err(self.error(
                yavs_diag::Diagnostic::error(
                    crate::diag::NOT_A_TYPE,
                    "a file declaration needs a file type",
                )
                .with_primary(f.subtype.span, "not a file type"),
            ));
        }
        let kind_t = self.well_known.file_open_kind.unwrap_or(self.poison.ty);
        let string = self.well_known.string.unwrap_or(self.poison.ty);
        let open_kind = f
            .open_kind
            .as_ref()
            .map(|e| self.expr(scope, e, crate::expr::Expected::Type(kind_t)));
        let file_open = f.file_name.as_ref().map(|e| {
            (
                open_kind,
                self.expr(scope, e, crate::expr::Expected::Type(string)),
            )
        });
        for &name in &f.names {
            let decl = self.new_decl(
                scope,
                Designator::Ident(name),
                DeclKind::Object(Object {
                    class: ObjClass::File,
                    role: ObjRole::Plain,
                    mode: None,
                    subtype,
                    default: None,
                    bounds: BoundsFrom::Subtype,
                    signal_kind: None,
                    file_open,
                }),
                name.span,
            );
            self.declare(scope, decl);
        }
        Ok(())
    }

    /// A subprogram body (4.3). It completes an earlier specification with the
    /// same designator and profile in the same region (for a package body: the
    /// package; for a protected type body: the protected type declaration),
    /// which keeps its `DeclId`; otherwise it is a new declaration.
    fn subprogram_body(
        &mut self,
        scope: ScopeId,
        b: &ast::SubprogramBody,
    ) -> Result<(), crate::Reported> {
        self.prepare_generic_body(scope, &b.spec);
        let mut regions = vec![scope];
        match self.scopes[scope].kind {
            crate::scope::ScopeKind::PackageBody => regions.extend(self.body_package(scope)),
            crate::scope::ScopeKind::ProtectedBody => regions.extend(self.scopes[scope].parent),
            crate::scope::ScopeKind::Architecture => regions.extend(self.entity_of_arch(scope)),
            _ => {}
        }
        // A body whose parameters have anonymous types completes the spec's types (#283).
        let anon_spec = self.anonymous_spec(&regions, &b.spec);
        if let Some(d) = anon_spec {
            self.anon_reuse = self.generic.implicit[&d]
                .iter()
                .map(|&t| self.types[t].first_subtype)
                .collect();
        }
        let new = self.subprogram_spec(scope, &b.spec);
        self.anon_reuse.clear();
        let new = new?;
        if let Some(d) = anon_spec {
            self.share_anonymous_ops(d, new);
        }
        let key = self.decls[new].name.key();
        // ponytail: a spec in the entity takes the body of the last architecture analyzed
        // in the session (one per elaboration in practice); per-architecture bodies would
        // need the body keyed by architecture.
        // A body with a header generic map (4.2.1) completes the generic of such a spec.
        let mapped = b.spec.generic_map.is_some();
        let spec = regions.iter().find_map(|&r| {
            let entity = self.scopes[r].kind == crate::scope::ScopeKind::Entity;
            self.scopes[r].names.get(&key)?.iter().copied().find_map(|d| {
                let d = if mapped { self.header_generic(d)? } else { d };
                (matches!(self.decls[d].kind.subprogram(), Some(s) if matches!(s.body, Body::Pending | Body::Foreign(_)) || entity && matches!(s.body, Body::Hir { .. }) && !self.decls[d].implicit) && self.same_profile(d, new)).then_some(d)
            })
        });
        let pscope = match &self.decls[new].kind {
            DeclKind::Subprogram(s) => s.scope.expect("specs have a parameter region"),
            _ => return Err(crate::Reported),
        };
        if let Some(d) = spec
            && !mapped
            && anon_spec.is_none()
        {
            self.check_conforms(d, b.spec.span);
        }
        let target = match spec {
            Some(d) => {
                // The body's parameter region (its spelling and defaults) becomes the body region.
                self.scopes[pscope].owner = Some(d);
                d
            }
            None => {
                // The instance is declared before the body, so the body sees its name (12.3).
                let d = self.header_instance(scope, new, &b.spec)?;
                self.declare(scope, d);
                new
            }
        };
        self.decls_in(pscope, &b.decls);
        let stmts = self.seq_stmts(pscope, &b.stmts);
        if let Some(g) = self.decls[target]
            .kind
            .subprogram()
            .and_then(|s| s.generics.clone())
        {
            self.report_unused_generics(&g);
        }
        // The body's own declaration (not declared when it completes a spec) gets the
        // body too, so no subprogram is left pending (validate, 7.3). A `'FOREIGN` spec keeps
        // its foreign body: the VHDL body is a placeholder, as in other simulators.
        for d in [target, new] {
            if let DeclKind::Subprogram(s) = &mut self.decls[d].kind
                && !matches!(s.body, Body::Foreign(_))
            {
                s.body = Body::Hir {
                    scope: pscope,
                    stmts: stmts.clone(),
                };
            }
        }
        // 10.2: a function calls no procedure that waits.
        if matches!(self.decls[target].kind.subprogram(), Some(s) if matches!(s.kind, SubprogramKind::Function { .. }))
        {
            let name = self.decls[target].name.to_string();
            self.check_calls_wait(&stmts, self.decls[new].name_span(), Some(&name))?;
        }
        Ok(())
    }

    /// `E1240`: the body's specification at `body` conforms to the declaration `d` (4.10, #808):
    /// the same lexical elements, apart from numeric literals of the same value and a simple
    /// name written as an expanded name. Not checked when a text cannot be read.
    fn check_conforms(&mut self, d: DeclId, body: Span) {
        use yavs_syntax::token::Tok;
        let decl = self.decls[d].span;
        let text = |s: Span| {
            let f = self.sources.get(s.file);
            let t = f.try_text().ok()?;
            Some(t[f.text_offset(s.start)..f.text_offset(s.end)].to_string())
        };
        let (Some(ta), Some(tb)) = (text(decl), text(body)) else {
            return;
        };
        let rev = self.file_rev();
        let (a, b) = (
            yavs_syntax::lexical_elements(&ta, rev),
            yavs_syntax::lexical_elements(&tb, rev),
        );
        // The value of an abstract literal, as bits of a REAL or an integer.
        let num = |t: &str| {
            use yavs_std::{error::What, value::lex_abstract};
            let what = if t.contains('.') {
                What::Real
            } else {
                What::Integer
            };
            let (v, end) = lex_abstract(t.as_bytes(), 0, what).ok()?;
            (end == t.len()).then(|| {
                if v.real {
                    Some((true, v.to_f64().to_bits() as i128))
                } else {
                    v.to_i128().map(|i| (false, i))
                }
            })?
        };
        let same = |x: &(Tok, &str), y: &(Tok, &str)| match (x.0, y.0) {
            (Tok::Abstract, Tok::Abstract) => {
                x.1.eq_ignore_ascii_case(y.1)
                    || matches!((num(x.1), num(y.1)), (Some(p), Some(q)) if p == q)
            }
            (Tok::Ident | Tok::Kw(_), Tok::Ident | Tok::Kw(_)) if !x.1.starts_with('\\') => {
                x.1.eq_ignore_ascii_case(y.1)
            }
            _ => x.0 == y.0 && x.1 == y.1,
        };
        // `p.q.n` for `n`: the prefix and its dot are skipped on either side.
        let prefix = |v: &[(Tok, &str)], i: usize| {
            matches!(v.get(i), Some((Tok::Ident, _))) && matches!(v.get(i + 1), Some((Tok::Dot, _)))
        };
        let (mut i, mut j) = (0, 0);
        let ok = loop {
            match (a.get(i), b.get(j)) {
                (None, None) => break true,
                (Some(x), Some(y)) if same(x, y) => (i, j) = (i + 1, j + 1),
                _ if prefix(&a, i) => i += 2,
                _ if prefix(&b, j) => j += 2,
                _ => break false,
            }
        };
        if !ok {
            let name = self.decls[d].name;
            self.report(
                yavs_diag::Diagnostic::error(
                    crate::diag::NOT_CONFORMING,
                    format!("the body of `{name}` does not conform to its declaration"),
                )
                .with_primary(body, "this specification")
                .with_secondary(decl, "differs from this one")
                .with_note("LRM 4.10: the two are the same sequence of lexical elements, apart from numeric literals of the same value and simple names written as expanded names"),
            );
        }
    }

    /// The pending spec in `regions` that a body with `spec`'s header completes, when its
    /// parameters have anonymous types: same designator, parameter and anonymous counts.
    fn anonymous_spec(&self, regions: &[ScopeId], spec: &ast::SubprogramSpec) -> Option<DeclId> {
        let objs = || {
            spec.params.iter().flatten().filter_map(|p| match p {
                ast::InterfaceDecl::Object(o) => Some(o),
                _ => None,
            })
        };
        let anon: usize = objs()
            .filter(|o| matches!(o.subtype, Some(ast::IncompleteSubtype::Anonymous(..))))
            .map(|o| o.names.len())
            .sum();
        if anon == 0 {
            return None;
        }
        let total: usize = objs().map(|o| o.names.len()).sum();
        let key = crate::alias::designator(&spec.designator).key();
        regions.iter().find_map(|&r| {
            self.scopes[r].names.get(&key)?.iter().copied().find(|d| {
                self.generic.implicit.get(d).is_some_and(|t| t.len() == anon)
                    && matches!(self.decls[*d].kind.subprogram(), Some(s)
                        if matches!(s.body, Body::Pending | Body::Foreign(_)) && s.params.len() == total)
            })
        })
    }

    /// The body's parameter region shares the spec's anonymous types, so it also holds their
    /// class operations (as `reuse_generics` does for generic types).
    fn share_anonymous_ops(&mut self, spec: DeclId, body: DeclId) {
        let (Some(sp), Some(bp)) = (
            self.decls[spec].kind.subprogram().and_then(|s| s.scope),
            self.decls[body].kind.subprogram().and_then(|s| s.scope),
        ) else {
            return;
        };
        let params: std::collections::HashSet<DeclId> = self.decls[spec]
            .kind
            .subprogram()
            .map(|s| s.params.iter().filter_map(|p| p.decl).collect())
            .unwrap_or_default();
        let ops: Vec<DeclId> = self.scopes[sp]
            .order
            .iter()
            .copied()
            .filter(|d| !params.contains(d))
            .collect();
        for d in ops {
            let k = self.decls[d].name.key();
            let s = &mut self.scopes[bp];
            s.names.entry(k).or_default().push(d);
            s.order.push(d);
        }
    }

    /// The type mark of a named return value denotes a scalar or composite type (lrm-checks
    /// L77), or a generic type that may be one.
    pub(crate) fn check_named_return_type(
        &mut self,
        span: Span,
        ret: crate::ids::SubtypeId,
    ) -> Result<(), crate::Reported> {
        use crate::generic::GenericClass as G;
        use crate::types::TypeKind as K;
        let base = self.base(ret);
        // A generic type is checked again in each instance (`fill_scope`).
        let kind = &self.types[base].kind;
        if kind.is_scalar()
            || matches!(
                kind,
                K::Array { .. }
                    | K::Record { .. }
                    | K::Error
                    | K::Generic {
                        class: G::Private
                            | G::Scalar
                            | G::Discrete
                            | G::Integer
                            | G::Physical
                            | G::Floating,
                    }
            )
        {
            return Ok(());
        }
        Err(self.error(
            yavs_diag::Diagnostic::error(
                crate::diag::NAMED_RETURN_TYPE,
                format!(
                    "a named return value needs a scalar, array or record type, not `{}`",
                    self.type_name(base)
                ),
            )
            .with_primary(span, "named here")
            .with_help("drop the return identifier: `return t`"),
        ))
    }

    /// `return r of t` (2019, 4.2.1, LCS-2016-072b): `r` denotes the result subtype, which
    /// the call takes from its context (the target's subtype). `r` is a subtype whose bounds
    /// come at run time from a hidden constant of type `t` that M2 passes with each call.
    fn named_return(
        &mut self,
        pscope: ScopeId,
        r: Ident,
        ret: crate::ids::SubtypeId,
    ) -> Result<(), crate::Reported> {
        let base = self.base(ret);
        self.check_named_return_type(r.span, ret)?;
        let hidden = self.new_decl(
            pscope,
            Designator::Ident(r),
            DeclKind::Object(Object {
                class: ObjClass::Constant,
                role: ObjRole::Plain,
                mode: Some(Mode::In),
                subtype: ret,
                default: None,
                bounds: crate::types::BoundsFrom::Actual,
                signal_kind: None,
                file_open: None,
            }),
            r.span,
        );
        let expr = self.add_expr(crate::hir::Expr {
            kind: crate::hir::ExprKind::Obj(hidden),
            subtype: ret,
            span: r.span,
            stat: crate::stat::Staticness::None,
            value: None,
        });
        let st = self.subtypes.push(crate::types::SubtypeDef {
            base,
            name: Some(r),
            parent: Some(ret),
            constraint: crate::types::Constraint::FromObject {
                expr,
                elem_depth: 0,
            },
            resolution: self.subtypes[ret].resolution.clone(),
            span: r.span,
        });
        let d = self.new_decl(pscope, Designator::Ident(r), DeclKind::Subtype(st), r.span);
        self.declare(pscope, d);
        self.ret_names.insert(pscope, (hidden, st));
        Ok(())
    }

    /// A subprogram declaration (4.2): parameters in their own region, result type.
    /// The caller declares the returned entity. A generic map in the header is the
    /// caller's (`header_instance`): this returns the uninstantiated subprogram.
    pub(crate) fn subprogram_spec(
        &mut self,
        scope: ScopeId,
        s: &ast::SubprogramSpec,
    ) -> Result<DeclId, crate::Reported> {
        let name = match &s.designator {
            ast::Designator::Ident(i) => Designator::Ident(*i),
            ast::Designator::Operator(op, _) => Designator::operator(op.as_str()),
            ast::Designator::Char(c) => Designator::Char(c.value),
        };
        let kind = match s.kind {
            ast::SubprogramKind::Procedure => SubprogramKind::Procedure,
            ast::SubprogramKind::Function(p) => SubprogramKind::Function {
                pure: p != Some(ast::Purity::Impure),
            },
        };
        let decl = self.new_decl(scope, name, DeclKind::Error, s.span);
        let pscope = self.add_scope(crate::scope::ScopeKind::Subprogram, Some(scope), Some(decl));
        let generics = match &s.generics {
            Some(g) => Some(self.generic_list(pscope, g)?),
            None => None,
        };
        // 5.6.2 (2008): no access or file type in the profile of a method; 2019 drops it
        // (LCS-2016-004; user, 2026-10-19, rule 15).
        let method_2008 = self.scopes[scope].kind == crate::scope::ScopeKind::Protected
            && self.file_rev() < LangRev::Vhdl2019
            && !self.in_std_library(scope);
        let mut params = Vec::new();
        self.param_anon.clear();
        for p in s.params.iter().flatten() {
            // 4.2.2.1: only constants, signals, variables and files are parameters.
            let o = match p {
                ast::InterfaceDecl::Object(o) => o,
                ast::InterfaceDecl::Type(t) => return Err(self.non_object_param("type", t.span)),
                ast::InterfaceDecl::Subprogram(x) => {
                    return Err(self.non_object_param("subprogram", x.spec.span));
                }
                ast::InterfaceDecl::Package(x) => {
                    return Err(self.non_object_param("package", x.span));
                }
            };
            let first = params.len();
            params.extend(self.param_decls(pscope, o, kind)?);
            if method_2008 {
                let at = o.names.first().map_or(o.span, |n| n.span);
                for p in &params[first..] {
                    self.check_not_access_or_file("protected method parameter", p.subtype, at)?;
                }
            }
        }
        let anon_params = std::mem::take(&mut self.param_anon);
        let name_span = match &s.designator {
            ast::Designator::Ident(i) => i.span,
            ast::Designator::Operator(_, sp) => *sp,
            ast::Designator::Char(c) => c.span,
        };
        if let SubprogramKind::Function { pure } = kind
            && params.iter().any(|p| p.mode != Mode::In)
            && (pure || self.file_rev() < LangRev::Vhdl2019)
        {
            // 4.2.2.1: `in` only; 2019 lets impure functions have `out` and `inout`.
            return Err(self.error(
                yavs_diag::Diagnostic::error(
                    crate::diag::CLASS_MISUSE,
                    format!("function `{name}` has a parameter that is not of mode `in`"),
                )
                .with_primary(name_span, "function parameters must have mode `in`"),
            ));
        }
        if let SubprogramKind::Function { pure } = kind
            && params.iter().any(|p| p.class == ObjClass::Variable)
            && (pure || self.file_rev() < LangRev::Vhdl2019)
        {
            // 4.2.2.1: constant, signal or file; 2019 impure functions take variables (#808).
            return Err(self.error(
                yavs_diag::Diagnostic::error(
                    crate::diag::CLASS_MISUSE,
                    format!("function `{name}` has a parameter of class `variable`"),
                )
                .with_primary(
                    name_span,
                    "function parameters are constants, signals or files",
                )
                .with_note("LRM 4.2.2.1; VHDL-2019 allows variable parameters of impure functions"),
            ));
        }
        if let Designator::Operator { key, .. } = name {
            self.check_operator_arity(key, params.len(), name_span)?;
        }
        let ret = match &s.return_type {
            // The generics of a generic subprogram are visible in its result type.
            // An anonymous parameter type is named by `p'subtype`, so the parameters are
            // visible here too (#283).
            Some(n) => Some(self.type_mark(
                if generics.is_some() || !anon_params.is_empty() {
                    pscope
                } else {
                    scope
                },
                n,
            )?),
            None => None,
        };
        if let (true, Some(r), Some(n)) = (method_2008, ret, &s.return_type) {
            self.check_not_access_or_file("protected method result", r, n.span)?;
        }
        // 4.2.1 (2008, 2019): the result of a pure function is not of an access type and
        // has no access subelement (clear; sim2 and sim4 reject, sim1 and sim3 accept).
        // `--relaxed` makes it warning W1830. Vendored VITAL packages are exempt, and a 1993
        // file is: the rule is not in the 1993 text (a vests 1993 test returns an access type).
        if let (SubprogramKind::Function { pure: true }, Some(r), Some(n)) =
            (kind, ret, &s.return_type)
            && self.file_rev() >= LangRev::Vhdl2008
            && !self.in_vital_library(scope)
            && let Some((_, bad)) = self.bad_elem(self.base(r), false)
            && matches!(self.types[bad].kind, crate::types::TypeKind::Access { .. })
        {
            let msg = format!(
                "the result of pure function `{name}` is or contains access type `{}`",
                self.type_name(bad)
            );
            let d = if self.relaxed() {
                yavs_diag::Diagnostic::warning(crate::diag::W_PURE_IMPURE, msg)
            } else {
                yavs_diag::Diagnostic::error(crate::diag::PURE_SIDE_EFFECT, msg)
                    .with_help("declare the function `impure`")
            };
            self.report(d.with_primary(n.span, "access type in the result of a pure function"));
        }
        if let (Some(r), Some(ret)) = (s.return_ident, ret) {
            self.named_return(pscope, r, ret)?;
        }
        let mangled = yavs_syntax::Symbol::intern(&crate::mangle::subprogram(
            self,
            scope,
            &name,
            &params.iter().map(|p| p.subtype).collect::<Vec<_>>(),
            ret,
        ));
        // std packages have no bodies: their bodiless subprograms are natives (NATIVE.md section 1).
        let body = match self.std_package_of(scope) {
            Some((pkg, pt)) => Body::Native(yavs_syntax::Symbol::intern(
                &crate::mangle::native_symbol(&pkg, pt.as_deref(), s),
            )),
            None => Body::Pending,
        };
        self.decls[decl].kind = DeclKind::Subprogram(Subprogram {
            kind,
            params: params.into(),
            ret,
            generics,
            scope: Some(pscope),
            body,
            mangled,
            ret_mark: s.return_type.as_ref().map(|n| n.span),
            impure_reads: false,
        });
        if !anon_params.is_empty() {
            self.generic.implicit.insert(decl, anon_params.into());
        }
        Ok(decl)
    }

    fn non_object_param(&mut self, what: &str, at: Span) -> crate::Reported {
        self.error(
            yavs_diag::Diagnostic::error(
                crate::diag::NON_OBJECT_PARAM,
                format!("a {what} interface declaration is not a parameter"),
            )
            .with_primary(at, "not a constant, signal, variable or file")
            .with_help(format!(
                "declare the {what} in the subprogram's generic list: `generic (...) parameter (...)`"
            )),
        )
    }

    /// `component c is generic (...); port (...); end component;` (6.8): generics
    /// and ports in the component's own region.
    fn component_decl(
        &mut self,
        scope: ScopeId,
        c: &ast::ComponentDecl,
    ) -> Result<(), crate::Reported> {
        let decl = self.new_decl(
            scope,
            Designator::Ident(c.name),
            DeclKind::Error,
            c.name.span,
        );
        let cscope = self.add_scope(crate::scope::ScopeKind::Component, Some(scope), Some(decl));
        self.hidden_types.clear();
        let generics = self.interface_list(
            cscope,
            c.generics.as_deref().unwrap_or_default(),
            ObjRole::Generic,
        );
        let ports = self.interface_list(
            cscope,
            c.ports.as_deref().unwrap_or_default(),
            ObjRole::Port,
        );
        // Anonymous interface types (#283) come after the declared generics.
        let mut all: Vec<DeclId> = generics.clone().unwrap_or_default().into_vec();
        all.extend(self.take_hidden_types());
        // Declared even when an interface failed, so instances do not cascade into `E1100`.
        self.decls[decl].kind = DeclKind::Component(Component {
            scope: cscope,
            generics: all.into(),
            ports: ports.clone().unwrap_or_default(),
        });
        self.declare(scope, decl);
        generics?;
        ports?;
        Ok(())
    }

    /// The anonymous interface types of the lists just analyzed, taken by their owner.
    pub(crate) fn take_hidden_types(&mut self) -> Vec<DeclId> {
        std::mem::take(&mut self.hidden_types)
    }

    /// A generic or port clause (6.5.6.2, 6.5.6.3) declared into `scope`, in order.
    /// Generics are constants of mode `in`; ports are signals, default mode `in`.
    pub(crate) fn interface_list(
        &mut self,
        scope: ScopeId,
        list: &[ast::InterfaceDecl],
        role: ObjRole,
    ) -> Result<Box<[DeclId]>, crate::Reported> {
        let mut out = Vec::new();
        let mut failed = None;
        for i in list {
            // Generic types, subprograms and packages (generic.rs).
            if role == ObjRole::Generic && !matches!(i, ast::InterfaceDecl::Object(_)) {
                match self.generic_item(scope, i) {
                    Ok(ds) => out.extend(ds),
                    Err(r) => failed = Some(r),
                }
                continue;
            }
            let ast::InterfaceDecl::Object(o) = i else {
                // Only generic clauses take these (taken above); 6.5.6.3: a port is a signal.
                let (what, sp) = match i {
                    ast::InterfaceDecl::Type(t) => ("type", t.span),
                    ast::InterfaceDecl::Subprogram(s) => ("subprogram", s.spec.span),
                    ast::InterfaceDecl::Package(p) => ("package", p.span),
                    ast::InterfaceDecl::Object(o) => ("object", o.span),
                };
                failed = Some(
                    self.error(
                        yavs_diag::Diagnostic::error(
                            crate::diag::NON_SIGNAL_PORT,
                            format!("a {what} interface declaration is not a port"),
                        )
                        .with_primary(sp, "not a signal")
                        .with_help(format!(
                            "declare the {what} in a generic list: `generic (...)`"
                        )),
                    ),
                );
                continue;
            };
            match self.interface_object(scope, o, role.clone()) {
                Ok(ds) => out.extend(ds),
                Err(r) => failed = Some(r),
            }
        }
        match failed {
            Some(r) => Err(r),
            None => Ok(out.into()),
        }
    }

    /// A mode view name: a view, an alias of one, or `v'converse` (2019 6.5.2, 16.2).
    pub(crate) fn view_name(
        &mut self,
        scope: ScopeId,
        n: &ast::Name,
    ) -> Result<crate::view::ViewRef, crate::Reported> {
        if let ast::NameKind::Attribute { prefix, attr } = &n.kind
            && attr.key.as_str() == "converse"
        {
            // 16.2: `'converse` is defined only for a mode view prefix.
            if !matches!(prefix.kind, ast::NameKind::Attribute { .. }) {
                let d = self.expanded_name_decl(scope, prefix)?;
                let d = self.entity(d);
                match self.decls[d].kind {
                    DeclKind::View(_) => {}
                    DeclKind::Error => return Err(crate::Reported),
                    _ => {
                        return Err(self.error(
                            yavs_diag::Diagnostic::error(
                                crate::diag::UNKNOWN_ATTRIBUTE,
                                "attribute `converse` is only defined for mode views",
                            )
                            .with_primary(attr.span, "needs a mode view prefix")
                            .with_secondary(self.decls[d].span, "not a mode view")
                            .with_note("LRM 2019 16.2"),
                        ));
                    }
                }
            }
            let r = self.view_name(scope, prefix)?;
            return Ok(crate::view::ViewRef {
                converse: !r.converse,
                ..r
            });
        }
        let d = self.expanded_name_decl(scope, n)?;
        self.view_ref(d, false, n.span)
    }

    fn interface_object(
        &mut self,
        scope: ScopeId,
        o: &ast::InterfaceObject,
        role: ObjRole,
    ) -> Result<Vec<DeclId>, crate::Reported> {
        let mode = match &o.mode {
            None | Some(ast::Mode::In) => Mode::In,
            Some(ast::Mode::Out) => Mode::Out,
            Some(ast::Mode::Inout) => Mode::Inout,
            Some(ast::Mode::Buffer) => Mode::Buffer,
            Some(ast::Mode::Linkage) => Mode::Linkage,
            Some(ast::Mode::View { name, array }) if role == ObjRole::Port => {
                // `p : view v [of t]`: the view gives the mode of each element (2019 6.5.2).
                let r = self.view_name(scope, name)?;
                let st = match &o.subtype {
                    Some(ast::IncompleteSubtype::Subtype(si)) => {
                        Some(self.subtype_indication(scope, si)?)
                    }
                    None => None,
                    _ => return Err(self.unsupported(o.span, "anonymous interface types", "S7")),
                };
                let (mode, subtype) = self.view_port(
                    o.class.or(Some(ast::InterfaceClass::Signal)),
                    r,
                    *array,
                    st,
                    o.span,
                )?;
                return Ok(self.declare_interface(
                    scope,
                    o,
                    role,
                    ObjClass::Signal,
                    mode,
                    &[subtype],
                ));
            }
            Some(ast::Mode::View { .. }) => {
                return Err(self.unsupported(o.span, "mode views outside port clauses", "S7"));
            }
        };
        // 2019 ports of class variable (6.5.2, LCS-2016-047): checked below to be
        // of a protected type with mode inout.
        let var_port = role == ObjRole::Port
            && o.class == Some(ast::InterfaceClass::Variable)
            && self.file_rev() >= LangRev::Vhdl2019;
        let (class, want) = match role {
            ObjRole::Generic => (ObjClass::Constant, Some(ast::InterfaceClass::Constant)),
            _ if var_port => (ObjClass::Variable, Some(ast::InterfaceClass::Variable)),
            _ => (ObjClass::Signal, Some(ast::InterfaceClass::Signal)),
        };
        let what = if role == ObjRole::Generic {
            "generic"
        } else {
            "port"
        };
        if o.class.is_some() && o.class != want || (role == ObjRole::Generic && mode != Mode::In) {
            return Err(self.error(
                yavs_diag::Diagnostic::error(
                    crate::diag::CLASS_MISUSE,
                    if role == ObjRole::Generic {
                        "a generic is a constant of mode `in`"
                    } else {
                        "a port is a signal"
                    },
                )
                .with_primary(o.span, format!("not allowed for a {what}")),
            ));
        }
        let subtype = match &o.subtype {
            Some(ast::IncompleteSubtype::Subtype(si)) => self.subtype_indication(scope, si)?,
            // `p : in type is (<>)`: one anonymous interface type per name, bound to the type
            // of its actual (2019 6.5.2, 6.5.3). The owner appends the types to its generics.
            Some(ast::IncompleteSubtype::Anonymous(def, sp)) if !var_port => {
                let mut sts = Vec::new();
                for &n in &o.names {
                    let st = self.anonymous_object_type(scope, n, def, *sp)?;
                    self.hidden_types.push(self.types[self.base(st)].decl);
                    sts.push(st);
                }
                return Ok(self.declare_interface(scope, o, role, class, mode, &sts));
            }
            _ => return Err(self.unsupported(o.span, "anonymous interface types", "S7")),
        };
        if var_port {
            let t = self.base(subtype);
            if mode == Mode::Inout
                && !matches!(self.types[t].kind, crate::types::TypeKind::Protected { .. })
                && self.all_protected(t)
            {
                return Err(self.error(
                    yavs_diag::Diagnostic::error(
                        crate::diag::PROTECTED_COMPOSITE_PORT,
                        "a variable port of a composite of protected types is not supported",
                    )
                    .with_primary(o.span, "variable port")
                    .with_help(
                        "most reference simulators reject this form; declare the port with a protected type, or one port per object",
                    ),
                ));
            }
            if !matches!(
                self.types[t].kind,
                crate::types::TypeKind::Protected { .. } | crate::types::TypeKind::Error
            ) || mode != Mode::Inout
            {
                return Err(self.error(
                    yavs_diag::Diagnostic::error(
                        crate::diag::CLASS_MISUSE,
                        "a variable port must be of a protected type and have mode `inout`",
                    )
                    .with_primary(o.span, "variable port"),
                ));
            }
        } else {
            let first = o.names.first().map_or(o.span, |n| n.span);
            self.check_not_access_or_file(what, subtype, first)?;
            if role == ObjRole::Port {
                for n in &o.names {
                    self.check_not_protected("port", subtype, n.span)?;
                }
            }
        }
        Ok(self.declare_interface(scope, o, role, class, mode, &[subtype]))
    }

    /// Declare the names of interface object `o`, with its default resolved in its subtype:
    /// `subtypes` is one subtype for all names or one per name.
    fn declare_interface(
        &mut self,
        scope: ScopeId,
        o: &ast::InterfaceObject,
        role: ObjRole,
        class: ObjClass,
        mode: Mode,
        subtypes: &[crate::ids::SubtypeId],
    ) -> Vec<DeclId> {
        let mut out = Vec::new();
        // One default for all names of one subtype (diagnostics once), else one per name.
        let shared = match (subtypes, &o.default) {
            ([st], Some(e)) => Some(self.interface_default_expr(scope, e, *st)),
            _ => None,
        };
        for (i, &name) in o.names.iter().enumerate() {
            let subtype = subtypes[i.min(subtypes.len() - 1)];
            let default = shared.or_else(|| {
                o.default
                    .as_ref()
                    .map(|e| self.interface_default_expr(scope, e, subtype))
            });
            let bounds = if self.constrainedness(subtype) == crate::types::Constrainedness::Full {
                BoundsFrom::Subtype
            } else {
                BoundsFrom::Actual
            };
            let decl = self.new_decl(
                scope,
                Designator::Ident(name),
                DeclKind::Object(Object {
                    class,
                    role: role.clone(),
                    mode: Some(mode),
                    subtype,
                    default,
                    bounds,
                    signal_kind: o.bus.then_some(ast::SignalKind::Bus),
                    file_open: None,
                }),
                name.span,
            );
            self.declare(scope, decl);
            out.push(decl);
        }
        out
    }

    /// `E1303`: an operator symbol that is not a VHDL operator, or the wrong number
    /// of parameters for it (4.5.2, 9.2).
    fn check_operator_arity(
        &mut self,
        key: yavs_syntax::Symbol,
        n: usize,
        span: Span,
    ) -> Result<(), crate::Reported> {
        let op = key.as_str();
        let op = op.trim_matches('"');
        let ok: &[usize] = match op {
            "+" | "-" => &[1, 2],
            // 2008 9.2.2: logical operators are also unary reductions; 1993 7.2 has binary
            // ones only (#808, as #814).
            "and" | "or" | "nand" | "nor" | "xor" | "xnor"
                if self.file_rev() == crate::LangRev::Vhdl1993 =>
            {
                &[2]
            }
            "and" | "or" | "nand" | "nor" | "xor" | "xnor" => &[1, 2],
            "not" | "abs" | "??" => &[1],
            "=" | "/=" | "<" | "<=" | ">" | ">=" | "?=" | "?/=" | "?<" | "?<=" | "?>" | "?>="
            | "*" | "/" | "mod" | "rem" | "**" | "&" | "sll" | "srl" | "sla" | "sra" | "rol"
            | "ror" => &[2],
            _ => {
                return Err(self.error(
                    yavs_diag::Diagnostic::error(
                        crate::diag::BAD_OPERATOR_DECL,
                        format!("`\"{op}\"` is not a VHDL operator"),
                    )
                    .with_primary(span, "not an operator symbol"),
                ));
            }
        };
        if ok.contains(&n) {
            return Ok(());
        }
        let want = if ok.len() == 2 {
            "1 or 2".to_string()
        } else {
            ok[0].to_string()
        };
        Err(self.error(
            yavs_diag::Diagnostic::error(
                crate::diag::BAD_OPERATOR_DECL,
                format!(
                    "operator `{op}` cannot take {n} parameter{}",
                    if n == 1 { "" } else { "s" }
                ),
            )
            .with_primary(span, format!("takes {want}")),
        ))
    }

    /// Interface objects of a parameter list (6.5.2): default class constant for `in`, else variable.
    fn param_decls(
        &mut self,
        pscope: ScopeId,
        o: &ast::InterfaceObject,
        kind: SubprogramKind,
    ) -> Result<Vec<Param>, crate::Reported> {
        // 4.2.2.3: a formal signal parameter is not declared `bus` (#808).
        if o.bus {
            return Err(self.error(
                yavs_diag::Diagnostic::error(
                    crate::diag::CLASS_MISUSE,
                    "a subprogram parameter cannot be declared `bus`",
                )
                .with_primary(o.span, "this parameter")
                .with_note("LRM 4.2.2.3: the reserved word `bus` may not appear in a formal signal parameter declaration"),
            ));
        }
        // `signal p : view v [of t]` (2019 4.2.2.3): the view gives the element modes.
        let mut view_subtype = None;
        let mode = match &o.mode {
            None | Some(ast::Mode::In) => Mode::In,
            Some(ast::Mode::Out) => Mode::Out,
            Some(ast::Mode::Inout) => Mode::Inout,
            // Subprogram parameters are `in`, `out` or `inout` (4.2.2.1); a mode view may
            // still give an element `buffer` or `linkage`.
            Some(m @ (ast::Mode::Buffer | ast::Mode::Linkage)) => {
                let m = if matches!(m, ast::Mode::Buffer) {
                    "buffer"
                } else {
                    "linkage"
                };
                return Err(self.error(
                    yavs_diag::Diagnostic::error(
                        crate::diag::PARAM_MODE,
                        format!("a subprogram parameter cannot have mode `{m}`"),
                    )
                    .with_primary(o.span, "this parameter")
                    .with_help("use `in`, `out` or `inout`"),
                ));
            }
            Some(ast::Mode::View { name, array }) => {
                let r = self.view_name(pscope, name)?;
                let st = match &o.subtype {
                    Some(ast::IncompleteSubtype::Subtype(si)) => {
                        Some(self.subtype_indication(pscope, si)?)
                    }
                    None => None,
                    _ => return Err(self.unsupported(o.span, "anonymous interface types", "S7")),
                };
                let (m, st) = self.view_port(o.class, r, *array, st, o.span)?;
                view_subtype = Some(st);
                m
            }
        };
        let class = match o.class {
            _ if view_subtype.is_some() => ObjClass::Signal,
            Some(ast::InterfaceClass::Constant) => ObjClass::Constant,
            Some(ast::InterfaceClass::Signal) => ObjClass::Signal,
            Some(ast::InterfaceClass::Variable) => ObjClass::Variable,
            Some(ast::InterfaceClass::File) => ObjClass::File,
            None if mode == Mode::In => ObjClass::Constant,
            None => ObjClass::Variable,
        };
        let _ = kind;
        // `x : in type is (<>)`: one anonymous interface type per name, bound by each call
        // to the type of its actual (2019 6.5.2, 6.5.3; `Db::implicit_instance`).
        let mut anon = Vec::new();
        let subtype = match (&o.subtype, view_subtype) {
            (_, Some(st)) => st,
            (Some(ast::IncompleteSubtype::Subtype(si)), None) => {
                self.subtype_indication(pscope, si)?
            }
            (Some(ast::IncompleteSubtype::Anonymous(def, sp)), None) => {
                for &n in &o.names {
                    let st = match self.anon_reuse.pop_front() {
                        Some(st) => st,
                        None => self.anonymous_object_type(pscope, n, def, *sp)?,
                    };
                    self.param_anon.push(self.base(st));
                    anon.push(st);
                }
                anon[0]
            }
            _ => return Err(self.unsupported(o.span, "anonymous interface types", "S7")),
        };
        // Only library `std` is exempt: its sources declare constant parameters of DIRECTORY
        // and CALL_PATH_VECTOR_PTR (#198). User code gets `E1223` in 2008 and 2019
        // (majority of the reference simulators, user 2026-10-19).
        let in_std = self.in_std_library(pscope);
        let first = o.names.first().map_or(o.span, |n| n.span);
        match class {
            _ if in_std => {}
            ObjClass::Constant => {
                self.check_not_access_or_file("constant parameter", subtype, first)?
            }
            ObjClass::Signal => {
                self.check_not_access_or_file("signal parameter", subtype, first)?
            }
            _ => {}
        }
        let shared = o
            .default
            .as_ref()
            .filter(|_| anon.is_empty())
            .map(|e| self.interface_default_expr(pscope, e, subtype));
        let mut out = Vec::new();
        for (i, &name) in o.names.iter().enumerate() {
            let subtype = anon.get(i).copied().unwrap_or(subtype);
            let default = match (&o.default, anon.is_empty()) {
                (Some(e), false) => Some(self.interface_default_expr(pscope, e, subtype)),
                _ => shared,
            };
            let bounds = if self.constrainedness(subtype) == crate::types::Constrainedness::Full {
                BoundsFrom::Subtype
            } else {
                BoundsFrom::Actual
            };
            let decl = self.new_decl(
                pscope,
                Designator::Ident(name),
                DeclKind::Object(Object {
                    class,
                    role: ObjRole::Param,
                    mode: Some(mode),
                    subtype,
                    default,
                    bounds,
                    signal_kind: None,
                    file_open: None,
                }),
                name.span,
            );
            self.declare(pscope, decl);
            let mark = match &o.subtype {
                Some(ast::IncompleteSubtype::Subtype(si)) => Some(si.type_mark.span),
                _ => None,
            };
            out.push(Param {
                name,
                class,
                mode,
                subtype,
                default,
                decl: Some(decl),
                mark,
            });
        }
        Ok(out)
    }

    /// Is `scope` inside a design unit of library `std`?
    pub(crate) fn in_std_library(&self, mut scope: ScopeId) -> bool {
        loop {
            if let Some(o) = self.scopes[scope].owner
                && let DeclKind::Unit(u) = self.decls[o].kind
            {
                return self.libs[self.units[u].lib].name.key.as_str() == "std";
            }
            match self.scopes[scope].parent {
                Some(p) => scope = p,
                None => return false,
            }
        }
    }

    /// Whether `scope` is in a vendored IEEE 1076.4 VITAL package (`ieee.vital_*`, body
    /// included). Their unmodified sources have pure functions calling `NOW`, which the
    /// LRM forbids but every VITAL-capable tool accepts (user, 2026-10-19).
    pub(crate) fn in_vital_library(&self, mut scope: ScopeId) -> bool {
        loop {
            if let Some(o) = self.scopes[scope].owner
                && let DeclKind::Unit(u) = self.decls[o].kind
            {
                let u = &self.units[u];
                return self.libs[u.lib].name.key.as_str() == "ieee"
                    && u.name.key.as_str().starts_with("vital_");
            }
            match self.scopes[scope].parent {
                Some(p) => scope = p,
                None => return false,
            }
        }
    }

    /// `(package, protected type)` when `scope` is the declaration of a package of
    /// library `std` or of a protected type in one.
    fn std_package_of(&self, scope: ScopeId) -> Option<(String, Option<String>)> {
        let (pkg_scope, pt) = match self.scopes[scope].kind {
            crate::scope::ScopeKind::Package => (scope, None),
            crate::scope::ScopeKind::Protected => {
                let owner = self.scopes[scope].owner?;
                (
                    self.scopes[scope].parent?,
                    Some(self.decls[owner].name.key().to_string()),
                )
            }
            _ => return None,
        };
        let DeclKind::Unit(u) = self.decls[self.scopes[pkg_scope].owner?].kind else {
            return None;
        };
        let unit = &self.units[u];
        (self.libs[unit.lib].name.key.as_str() == "std").then(|| (unit.name.key.to_string(), pt))
    }
}

pub(crate) fn decl_span(d: &ast::Decl) -> Span {
    use ast::Decl as D;
    match d {
        D::Type(x) => x.span,
        D::Subtype(x) => x.span,
        D::Object(x) => x.span,
        D::File(x) => x.span,
        D::Alias(x) => x.span,
        D::Component(x) => x.span,
        D::Attribute(x) => x.span,
        D::AttributeSpec(x) => x.span,
        D::Subprogram(x) => x.span,
        D::SubprogramBody(x) => x.span,
        D::SubprogramInstance(x) => x.span,
        D::Package(x) => x.span,
        D::PackageBody(x) => x.span,
        D::PackageInstance(x) => x.span,
        D::Use(x) => x.span,
        D::Group(x) => x.span,
        D::GroupTemplate(x) => x.span,
        D::Disconnect(x) => x.span,
        D::ConfigSpec(x) => x.span,
        D::View(x) => x.span,
    }
}
