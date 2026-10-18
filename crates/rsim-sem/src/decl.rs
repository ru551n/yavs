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
            D::Subprogram(s) => self.subprogram_spec(scope, s).map(|_| ()),
            D::Attribute(a) => {
                let ty = self.type_mark(scope, &a.type_mark)?;
                let decl = self.new_decl(
                    scope,
                    Designator::Ident(a.name),
                    DeclKind::Attribute { ty },
                    a.span,
                );
                self.declare(scope, decl);
                Ok(())
            }
            D::Use(u) => {
                self.use_clause(scope, u);
                Ok(())
            }
            D::SubprogramBody(b) => Err(self.unsupported(b.spec.span, "subprogram bodies", "S2")),
            D::Alias(a) => Err(self.unsupported(a.span, "alias declarations", "S2")),
            D::AttributeSpec(a) => Err(self.unsupported(a.span, "attribute specifications", "S6")),
            D::Component(c) => Err(self.unsupported(c.span, "component declarations", "S5")),
            D::SubprogramInstance(x) => {
                Err(self.unsupported(x.span, "subprogram instantiation", "S4"))
            }
            D::Package(x) => Err(self.unsupported(x.span, "nested packages", "S4")),
            D::PackageBody(x) => Err(self.unsupported(x.span, "nested package bodies", "S4")),
            D::PackageInstance(x) => Err(self.unsupported(x.span, "package instantiation", "S4")),
            D::Group(x) => Err(self.unsupported(x.span, "groups", "S6")),
            D::GroupTemplate(x) => Err(self.unsupported(x.span, "group templates", "S6")),
            D::Disconnect(x) => Err(self.unsupported(x.span, "disconnection specifications", "S5")),
            D::ConfigSpec(x) => Err(self.unsupported(x.span, "configuration specifications", "S5")),
            D::View(x) => Err(self.unsupported(x.span, "mode views", "S7")),
        }
    }

    /// `constant`, `signal`, `variable`, `shared variable` declarations (6.4.2).
    fn object_decl(&mut self, scope: ScopeId, o: &ast::ObjectDecl) -> Result<(), crate::Reported> {
        if o.private {
            return Err(self.unsupported(o.span, "private variables", "S7"));
        }
        let subtype = self.subtype_indication(scope, &o.subtype)?;
        let base = self.base(subtype);
        let class = match o.class {
            ast::ObjectClass::Constant => ObjClass::Constant,
            ast::ObjectClass::Signal => ObjClass::Signal,
            ast::ObjectClass::Variable => ObjClass::Variable,
            ast::ObjectClass::SharedVariable => ObjClass::SharedVariable,
        };
        let default = o
            .default
            .as_ref()
            .map(|e| self.expr(scope, e, crate::expr::Expected::Type(base)));
        let full = self.constrainedness(subtype) == crate::types::Constrainedness::Full;
        let bounds = if full {
            BoundsFrom::Subtype
        } else {
            BoundsFrom::Initializer
        };
        if !full && class != ObjClass::Constant {
            self.report(
                rsim_diag::Diagnostic::error(
                    crate::diag::NOT_FULLY_CONSTRAINED,
                    "a signal or variable needs a fully constrained subtype",
                )
                .with_primary(o.subtype.span, "not fully constrained")
                .with_help("give index ranges, such as `string(1 to 8)`"),
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

    /// A subprogram declaration (4.2): parameters in their own region, result type.
    pub(crate) fn subprogram_spec(
        &mut self,
        scope: ScopeId,
        s: &ast::SubprogramSpec,
    ) -> Result<DeclId, crate::Reported> {
        if s.generics.is_some() || s.generic_map.is_some() {
            return Err(self.unsupported(s.span, "generic subprograms", "S4"));
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
        let mut params = Vec::new();
        for p in s.params.iter().flatten() {
            let ast::InterfaceDecl::Object(o) = p else {
                return Err(self.unsupported(s.span, "non-object parameters", "S4"));
            };
            params.extend(self.param_decls(pscope, o, kind)?);
        }
        let ret = match &s.return_type {
            Some(n) => Some(self.type_mark(scope, n)?),
            None => None,
        };
        if s.return_ident.is_some() {
            return Err(self.unsupported(s.span, "named return values", "S7"));
        }
        let mangled =
            rsim_syntax::Symbol::intern(&self.mangle_subprogram(scope, &name, &params, ret));
        self.decls[decl].kind = DeclKind::Subprogram(Subprogram {
            kind,
            params: params.into(),
            ret,
            generics: None,
            scope: Some(pscope),
            body: Body::Pending,
            mangled,
        });
        self.declare(scope, decl);
        Ok(decl)
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

    /// `<region>.<name>(<param types>)<ret>` (sem.md 5.3), by mangled base type names.
    // ponytail: interim until mangle.rs (owner: implicit) provides the full scheme.
    fn mangle_subprogram(
        &self,
        scope: ScopeId,
        name: &Designator,
        params: &[Param],
        ret: Option<SubtypeId>,
    ) -> String {
        let region = self.region_path(scope);
        let tys: Vec<String> = params
            .iter()
            .map(|p| self.type_path(self.base(p.subtype)))
            .collect();
        let mut s = format!("{region}.{name}({})", tys.join(","));
        if let Some(r) = ret {
            s += &self.type_path(self.base(r));
        }
        s
    }

    /// `lib.unit[.nested]` of a region, by its owners.
    pub(crate) fn region_path(&self, scope: ScopeId) -> String {
        let mut parts = Vec::new();
        let mut cur = Some(scope);
        while let Some(s) = cur {
            if let Some(o) = self.scopes[s].owner {
                match self.decls[o].kind {
                    DeclKind::Unit(u) => {
                        let unit = &self.units[u];
                        parts.push(unit.name.key.to_string());
                        parts.push(self.libs[unit.lib].name.key.to_string());
                        break;
                    }
                    _ => parts.push(self.decls[o].name.key().to_string()),
                }
            }
            cur = self.scopes[s].parent;
        }
        parts.reverse();
        parts.join(".")
    }

    fn type_path(&self, t: TypeId) -> String {
        let td = &self.types[t];
        format!(
            "{}.{}",
            self.region_path(self.decls[td.decl].scope),
            td.name.key
        )
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
