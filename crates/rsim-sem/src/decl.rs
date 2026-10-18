//! Declarations: the `Decl` entity and its kinds (sem.md 1.3, 2.10, 6.2), and
//! the analysis of declarative items.

use rsim_diag::Span;
use rsim_syntax::Symbol;
use rsim_syntax::ast::{Ident, SignalKind};

use crate::LangRev;
use crate::hir::Mode;
use crate::ids::{
    DeclId, ExprId, InstId, LibId, ScopeId, StmtId, SubtypeId, TypeId, UnitId, ViewId,
};
use crate::implicit::ImplicitOp;
use crate::types::BoundsFrom;

/// The name of a declaration, original spelling kept (rule 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    Symbol::intern(&format!("\"{}\"", op.to_ascii_lowercase()))
}

/// Lookup key of character literal `c`.
pub fn char_key(c: char) -> Symbol {
    Symbol::intern(&format!("'{c}'"))
}

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
    /// A generic type (its `TypeKind::Generic` carries the class).
    GenericType(TypeId),
    /// Formal generic subprogram (`Subprogram.body == Body::Interface`).
    GenericSubprogram(Subprogram),
    /// Formal generic package.
    GenericPackage(InstId),
    Group {
        template: DeclId,
    },
    GroupTemplate,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjClass {
    Constant,
    Signal,
    Variable,
    SharedVariable,
    File,
}

/// What kind of declaration made the object.
#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubprogramKind {
    Function { pure: bool },
    Procedure,
}

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
}

pub struct Param {
    pub name: Ident,
    pub class: ObjClass,
    pub mode: Mode,
    pub subtype: SubtypeId,
    pub default: Option<ExprId>,
    /// The parameter's object declaration; `None` for implicit operations.
    pub decl: Option<DeclId>,
}

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

pub enum Alias {
    Object {
        /// A static name.
        target: ExprId,
        subtype: Option<SubtypeId>,
    },
    /// Type, subtype, subprogram (with signature), enum literal, unit, ...
    NonObject { target: DeclId },
}

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

use rsim_syntax::ast;

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
            K::Package | K::Protected | K::ProtectedBody | K::Instance => {}
            K::PackageBody => {
                // A package's subprograms get their bodies in the package body (4.8).
                // Its parent is the body's context region, whose parent is the package.
                let mut up = self.scopes[scope].parent;
                while let Some(u) = up
                    && self.scopes[u].kind != K::Package
                {
                    up = self.scopes[u].parent;
                }
                if let Some(pkg) = up {
                    self.check_subprogram_bodies(pkg, end);
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

    /// `E1230`: a subprogram declared in this region has no body in it (4.2, 4.3).
    fn check_subprogram_bodies(&mut self, scope: ScopeId, end: Span) {
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
                rsim_diag::Diagnostic::error(
                    crate::diag::MISSING_BODY,
                    format!("{what} `{name}` has no body"),
                )
                .with_primary(self.decls[d].name_span(), "declared here")
                .with_secondary(end, "the region ends here without its body"),
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
                rsim_diag::Diagnostic::error(
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
            D::Component(c) => self.component_decl(scope, c),
            D::SubprogramInstance(x) => self.subprogram_instance(scope, x),
            D::Package(x) => Err(self.unsupported(x.span, "nested packages", "S4")),
            D::PackageBody(x) => Err(self.unsupported(x.span, "nested package bodies", "S4")),
            D::PackageInstance(x) => self.package_instance_decl(scope, x),
            D::Group(x) => Err(self.unsupported(x.span, "groups", "S6")),
            D::GroupTemplate(x) => Err(self.unsupported(x.span, "group templates", "S6")),
            D::Disconnect(x) => Err(self.unsupported(x.span, "disconnection specifications", "S5")),
            D::ConfigSpec(x) => self.config_spec(scope, x),
            D::View(x) => Err(self.unsupported(x.span, "mode views", "S7")),
        }
    }

    /// `constant`, `signal`, `variable`, `shared variable` declarations (6.4.2).
    fn object_decl(&mut self, scope: ScopeId, o: &ast::ObjectDecl) -> Result<(), crate::Reported> {
        if o.private {
            return Err(self.unsupported(o.span, "private variables", "S7"));
        }
        let subtype = self.subtype_indication(scope, &o.subtype)?;
        let class = match o.class {
            ast::ObjectClass::Constant => ObjClass::Constant,
            ast::ObjectClass::Signal => ObjClass::Signal,
            ast::ObjectClass::Variable => ObjClass::Variable,
            ast::ObjectClass::SharedVariable => ObjClass::SharedVariable,
        };
        let default = o
            .default
            .as_ref()
            .map(|e| self.expr_in_subtype(scope, e, subtype));
        let full = self.constrainedness(subtype) == crate::types::Constrainedness::Full;
        let bounds = if full {
            BoundsFrom::Subtype
        } else {
            BoundsFrom::Initializer
        };
        if !full && class != ObjClass::Constant {
            let what = if class == ObjClass::Signal {
                "signal"
            } else {
                "variable"
            };
            let name = o.names.first().map_or(o.subtype.span, |n| n.span);
            self.report(
                rsim_diag::Diagnostic::error(
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
        let res = match &name.kind {
            ast::NameKind::Simple(_)
            | ast::NameKind::Selected { .. }
            | ast::NameKind::Operator(_)
            | ast::NameKind::Character(_) => self.classify(scope, name),
            ast::NameKind::External(_) => {
                return Err(self.unsupported(name.span, "aliases of external names", "S6"));
            }
            _ => {
                return Err(self.unsupported(
                    name.span,
                    "aliases of indexed, sliced or selected object names",
                    "S3",
                ));
            }
        };
        let cands = match res {
            NameRes::One(d) if self.object_type(d).is_some() => {
                let target = self.object_ref(d, name.span);
                let alias = self.object_alias(scope, a, target, subtype)?;
                self.declare(scope, alias);
                return Ok(());
            }
            NameRes::One(d) => vec![d],
            NameRes::Overloads(ds) => ds,
            other => return Err(self.report_name(scope, other)),
        };
        let sig = match sig {
            Some(s) => Some(self.signature_of(scope, s)?),
            None => None,
        };
        let decls = self.nonobject_alias(scope, a, &cands, sig.as_ref())?;
        for d in decls {
            self.declare(scope, d);
        }
        Ok(())
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
                    rsim_diag::Diagnostic::error(
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
                rsim_diag::Diagnostic::error(
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
        let new = self.subprogram_spec(scope, &b.spec)?;
        let key = self.decls[new].name.key();
        let mut regions = vec![scope];
        match self.scopes[scope].kind {
            crate::scope::ScopeKind::PackageBody => {
                if let Some(c) = self.scopes[scope].parent
                    && let Some(p) = self.scopes[c].parent
                {
                    regions.push(p);
                }
            }
            crate::scope::ScopeKind::ProtectedBody => regions.extend(self.scopes[scope].parent),
            _ => {}
        }
        let profile = self.profile(new);
        let spec = regions.iter().find_map(|&r| {
            self.scopes[r].names.get(&key)?.iter().copied().find(|&d| {
                matches!(self.decls[d].kind.subprogram(), Some(s) if matches!(s.body, Body::Pending)) && self.profile(d) == profile
            })
        });
        let pscope = match &self.decls[new].kind {
            DeclKind::Subprogram(s) => s.scope.expect("specs have a parameter region"),
            _ => return Err(crate::Reported),
        };
        let target = match spec {
            Some(d) => {
                // The body's parameter region (its spelling and defaults) becomes the body region.
                self.scopes[pscope].owner = Some(d);
                d
            }
            None => {
                self.declare(scope, new);
                new
            }
        };
        self.decls_in(pscope, &b.decls);
        let stmts = self.seq_stmts(pscope, &b.stmts);
        // The body's own declaration (not declared when it completes a spec) gets the
        // body too, so no subprogram is left pending (validate, 7.3).
        for d in [target, new] {
            if let DeclKind::Subprogram(s) = &mut self.decls[d].kind {
                s.body = Body::Hir {
                    scope: pscope,
                    stmts: stmts.clone(),
                };
            }
        }
        Ok(())
    }

    /// A subprogram declaration (4.2): parameters in their own region, result type.
    /// The caller declares the returned entity.
    pub(crate) fn subprogram_spec(
        &mut self,
        scope: ScopeId,
        s: &ast::SubprogramSpec,
    ) -> Result<DeclId, crate::Reported> {
        if s.generic_map.is_some() {
            return Err(self.unsupported(s.span, "subprograms with a generic map", "S4"));
        }
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
        let mut params = Vec::new();
        for p in s.params.iter().flatten() {
            let ast::InterfaceDecl::Object(o) = p else {
                return Err(self.unsupported(s.span, "non-object parameters", "S4"));
            };
            params.extend(self.param_decls(pscope, o, kind)?);
        }
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
                rsim_diag::Diagnostic::error(
                    crate::diag::CLASS_MISUSE,
                    format!("function `{name}` has a parameter that is not of mode `in`"),
                )
                .with_primary(name_span, "function parameters must have mode `in`"),
            ));
        }
        if let Designator::Operator { key, .. } = name {
            self.check_operator_arity(key, params.len(), name_span)?;
        }
        let ret = match &s.return_type {
            // The generics of a generic subprogram are visible in its result type.
            Some(n) => Some(self.type_mark(if generics.is_some() { pscope } else { scope }, n)?),
            None => None,
        };
        if s.return_ident.is_some() {
            return Err(self.unsupported(s.span, "named return values", "S7"));
        }
        let mangled = rsim_syntax::Symbol::intern(&crate::mangle::subprogram(
            self,
            scope,
            &name,
            &params.iter().map(|p| p.subtype).collect::<Vec<_>>(),
            ret,
        ));
        // std packages have no bodies: their bodiless subprograms are natives (NATIVE.md section 1).
        let body = match self.std_package_of(scope) {
            Some((pkg, pt)) => Body::Native(rsim_syntax::Symbol::intern(
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
        });
        Ok(decl)
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
        // Declared even when an interface failed, so instances do not cascade into `E1100`.
        self.decls[decl].kind = DeclKind::Component(Component {
            scope: cscope,
            generics: generics.clone().unwrap_or_default(),
            ports: ports.clone().unwrap_or_default(),
        });
        self.declare(scope, decl);
        generics?;
        ports?;
        Ok(())
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
            let ast::InterfaceDecl::Object(o) = i else {
                let sp = match i {
                    ast::InterfaceDecl::Type(t) => t.span,
                    ast::InterfaceDecl::Subprogram(s) => s.spec.span,
                    ast::InterfaceDecl::Package(p) => p.span,
                    ast::InterfaceDecl::Object(o) => o.span,
                };
                failed = Some(self.unsupported(
                    sp,
                    "generic types, subprograms and packages of components and blocks",
                    "S4",
                ));
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
            Some(ast::Mode::View { .. }) => {
                return Err(self.unsupported(o.span, "mode views", "S7"));
            }
        };
        let (class, want) = match role {
            ObjRole::Generic => (ObjClass::Constant, Some(ast::InterfaceClass::Constant)),
            _ => (ObjClass::Signal, Some(ast::InterfaceClass::Signal)),
        };
        let what = if role == ObjRole::Generic {
            "generic"
        } else {
            "port"
        };
        if o.class.is_some() && o.class != want || (role == ObjRole::Generic && mode != Mode::In) {
            return Err(self.error(
                rsim_diag::Diagnostic::error(
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
            _ => return Err(self.unsupported(o.span, "anonymous interface types", "S7")),
        };
        if role == ObjRole::Port {
            for n in &o.names {
                self.check_not_protected("port", subtype, n.span)?;
            }
        }
        let default = o
            .default
            .as_ref()
            .map(|e| self.expr_in_subtype(scope, e, subtype));
        let bounds = if self.constrainedness(subtype) == crate::types::Constrainedness::Full {
            BoundsFrom::Subtype
        } else {
            BoundsFrom::Actual
        };
        let mut out = Vec::new();
        for &name in &o.names {
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
        Ok(out)
    }

    /// `E1303`: an operator symbol that is not a VHDL operator, or the wrong number
    /// of parameters for it (4.5.2, 9.2).
    fn check_operator_arity(
        &mut self,
        key: rsim_syntax::Symbol,
        n: usize,
        span: Span,
    ) -> Result<(), crate::Reported> {
        let op = key.as_str();
        let op = op.trim_matches('"');
        let ok: &[usize] = match op {
            "+" | "-" => &[1, 2],
            // 2008 9.2.2: logical operators are also unary reductions.
            "and" | "or" | "nand" | "nor" | "xor" | "xnor" => &[1, 2],
            "not" | "abs" | "??" => &[1],
            "=" | "/=" | "<" | "<=" | ">" | ">=" | "?=" | "?/=" | "?<" | "?<=" | "?>" | "?>="
            | "*" | "/" | "mod" | "rem" | "**" | "&" | "sll" | "srl" | "sla" | "sra" | "rol"
            | "ror" => &[2],
            _ => {
                return Err(self.error(
                    rsim_diag::Diagnostic::error(
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
            rsim_diag::Diagnostic::error(
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
        let mode = match &o.mode {
            None | Some(ast::Mode::In) => Mode::In,
            Some(ast::Mode::Out) => Mode::Out,
            Some(ast::Mode::Inout) => Mode::Inout,
            Some(ast::Mode::Buffer) => Mode::Buffer,
            Some(ast::Mode::Linkage) => Mode::Linkage,
            Some(ast::Mode::View { .. }) => {
                return Err(self.unsupported(o.span, "mode views", "S7"));
            }
        };
        let class = match o.class {
            Some(ast::InterfaceClass::Constant) => ObjClass::Constant,
            Some(ast::InterfaceClass::Signal) => ObjClass::Signal,
            Some(ast::InterfaceClass::Variable) => ObjClass::Variable,
            Some(ast::InterfaceClass::File) => ObjClass::File,
            None if mode == Mode::In => ObjClass::Constant,
            None => ObjClass::Variable,
        };
        let _ = kind;
        let subtype = match &o.subtype {
            Some(ast::IncompleteSubtype::Subtype(si)) => self.subtype_indication(pscope, si)?,
            _ => return Err(self.unsupported(o.span, "anonymous interface types", "S7")),
        };
        let base = self.base(subtype);
        let default = o
            .default
            .as_ref()
            .map(|e| self.expr(pscope, e, crate::expr::Expected::Type(base)));
        let bounds = if self.constrainedness(subtype) == crate::types::Constrainedness::Full {
            BoundsFrom::Subtype
        } else {
            BoundsFrom::Actual
        };
        let mut out = Vec::new();
        for &name in &o.names {
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
            out.push(Param {
                name,
                class,
                mode,
                subtype,
                default,
                decl: Some(decl),
            });
        }
        Ok(out)
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
