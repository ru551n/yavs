//! Generics: classes, instances and instantiation by HIR cloning (sem.md section 5).
//!
//! A generic unit (package, subprogram) is analyzed once with opaque formals
//! (2.9). Instantiation clones the analyzed HIR and remaps ids; it never
//! re-analyzes the AST (5.1, Q7). The visible part of a package instance is
//! cloned eagerly, bodies lazily through [`Db::instance_body`].
//!
//! What the clone does (5.1):
//! - every declaration, type, scope and statement of the generic region gets a
//!   fresh id, so two instances never share a type (rule 4);
//! - references to formals become the actuals: a formal constant's `Obj` is the
//!   actual expression, a formal type is the actual subtype, a formal subprogram
//!   the actual subprogram, a formal package's declarations the actual
//!   instance's clones;
//! - the implicit operations of a formal type become the same family's
//!   predefined operation of the actual type (Q9, U-6: also for `=` on a 2008
//!   `type T`, with `W1506` when NVC would pick a visible user `"="`);
//! - subtypes and expressions that mention none of the above keep their ids;
//!   changed expressions lose their folded value and are folded again.

use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};

use yavs_diag::{Diagnostic, Span};
use yavs_syntax::Symbol;
use yavs_syntax::ast;

use crate::db::{Db, Reported};
use crate::decl::{
    Alias, Body, Component, Decl, DeclKind, Designator, ObjClass, ObjRole, Object, Param,
    Subprogram, SubprogramKind,
};
use crate::diag;
use crate::hir::{
    Actual, AggAssoc, AggBounds, Arg, AttrPrefix, Call, Choice, ChoiceSet, Delay, Expr, ExprKind,
    External, GenAlt, InstTarget, LoopScheme, Lsp, LspStep, Mode, NewInit, ObjRoot, ProcessInfo,
    Rhs, Sensitivity, Stmt, StmtKind, Target, UnitHir, WaveElem, Waveform,
};
use crate::ids::{DeclId, ExprId, InstId, ScopeId, StmtId, SubtypeId, TypeId, UnitId, ViewId};
use crate::implicit::ImplicitOp;
use crate::loader::{UnitKey, UnitKind};
use crate::scope::{Scope, ScopeKind, UseItem, Visible};
use crate::types::{
    BoundsFrom, Constrainedness, Constraint, RangeC, RecordElem, Resolution, SubtypeDef, TypeDef,
    TypeKind,
};

mod entity;
pub use entity::GenericActual;

/// The class of a generic type (2.9; M0 `GenericTypeDef`).
#[derive(Clone, Debug)]
pub enum GenericClass {
    /// `type T` (2008) and `type T is private` (2019).
    Private,
    /// `is <>`
    Scalar,
    /// `is (<>)`
    Discrete,
    /// `is range <>`
    Integer,
    /// `is units <>`
    Physical,
    /// `is range <> . <>`
    Floating,
    /// Index subtypes and element subtype; either may be an anonymous generic type.
    Array {
        indexes: Box<[SubtypeId]>,
        elem: SubtypeId,
    },
    Access {
        designated: SubtypeId,
    },
    File {
        elem: SubtypeId,
    },
}

pub struct Instance {
    /// The uninstantiated package or subprogram.
    pub generic: DeclId,
    /// The instantiation declaration.
    pub site: DeclId,
    pub map: GenericMap,
    /// Cloned visible part (eager). For a formal generic package (`generic map
    /// (<>)`) this is the generic package's own region: its formals stay opaque.
    pub scope: ScopeId,
    /// Cloned body (lazy, when M2 asks: [`Db::instance_body`]).
    pub body: OnceCell<ScopeId>,
    pub mangled: Symbol,
}

#[derive(Default)]
pub struct GenericMap {
    /// Formal generic type to actual subtype.
    pub types: HashMap<TypeId, SubtypeId>,
    /// Formal constant to actual expression.
    pub consts: HashMap<DeclId, ExprId>,
    /// Formal subprogram (and implicit operation of a formal type) to actual subprogram.
    pub subprograms: HashMap<DeclId, DeclId>,
    /// Formal package to actual instance.
    pub packages: HashMap<DeclId, InstId>,
}

/// Default of a formal generic subprogram (6.5.6.2).
#[derive(Clone, Debug)]
pub enum SubpDefault {
    /// `is <>`: looked up at the instantiation.
    Box,
    /// `is name`: resolved where the formal is declared (L11), checked at the instantiation.
    Name { cands: Vec<DeclId>, span: Span },
}

/// Generic bookkeeping kept on `Db` (sem.md 5).
#[derive(Default)]
pub struct GenericState {
    pub(crate) defaults: HashMap<DeclId, SubpDefault>,
    /// The generic list analyzed into a region (a package scope or a subprogram's
    /// parameter region, also the parameter region of a generic subprogram body).
    pub(crate) lists: HashMap<ScopeId, Box<[DeclId]>>,
    /// The id remapping of each instance (for its body and for formal packages).
    pub(crate) maps: HashMap<InstId, Remap>,
    /// The generic spec the subprogram body being analyzed completes (`prepare_generic_body`).
    pub(crate) reuse: Option<DeclId>,
    /// Uninstantiated subprograms of headers with a generic map (`header_instance`).
    pub(crate) header: HashSet<DeclId>,
    /// Generic constants, types and packages of packages and subprograms not yet
    /// looked up (`W1501`; `Db::lookup` removes them).
    pub(crate) unused: HashSet<DeclId>,
    /// The actuals a formal package with a generic map requires of its actual instance.
    pub(crate) wants: HashMap<InstId, Vec<(DeclId, Want)>>,
    /// Design entities cloned per map of generic type, subprogram and package actuals
    /// (`instantiate_entity`): (entity, architecture, actuals) to the clones.
    #[allow(clippy::type_complexity)]
    pub(crate) entities: HashMap<(UnitId, UnitId, Box<[Option<GenericActual>]>), (UnitId, UnitId)>,
    /// Mangled names of those clones (5.3).
    pub(crate) unit_names: HashMap<UnitId, Symbol>,
    /// Components cloned per map of those actuals (`instantiate_component`).
    #[allow(clippy::type_complexity)]
    pub(crate) components: HashMap<(DeclId, Box<[Option<GenericActual>]>), DeclId>,
    /// Blocks with generic types, subprograms or packages of the architectures being
    /// analyzed, innermost first (`instantiate_blocks`).
    pub(crate) blocks: Vec<(UnitId, StmtId)>,
}

/// One association of a formal package's generic map (6.5.6.3).
#[derive(Clone, Debug)]
pub(crate) enum Want {
    Const(ExprId),
    Type(SubtypeId),
    Subprogram(Vec<DeclId>),
}

/// Original to clone ids of one instance (5.1).
#[derive(Default, Clone)]
pub(crate) struct Remap {
    /// Regions whose content is cloned.
    roots: Vec<ScopeId>,
    decls: HashMap<DeclId, DeclId>,
    types: HashMap<TypeId, TypeId>,
    subtypes: HashMap<SubtypeId, SubtypeId>,
    scopes: HashMap<ScopeId, ScopeId>,
    exprs: HashMap<ExprId, ExprId>,
    stmts: HashMap<StmtId, StmtId>,
    /// Formal constants to actual expressions.
    consts: HashMap<DeclId, ExprId>,
    /// First subtypes of formal types (a `subtype s is T` inherits the actual's constraint).
    formal_firsts: HashSet<SubtypeId>,
    /// Clone subprogram bodies (the body pass) or leave them pending (visible part).
    bodies: bool,
    changed: bool,
    failed: bool,
}

impl Remap {
    fn new(root: ScopeId, to: ScopeId) -> Self {
        let mut r = Remap {
            roots: vec![root],
            ..Default::default()
        };
        r.scopes.insert(root, to);
        r
    }

    /// Take over the mapping of an actual instance (for a formal package).
    fn merge(&mut self, o: &Remap) {
        self.decls.extend(o.decls.iter().map(|(a, b)| (*a, *b)));
        self.types.extend(o.types.iter().map(|(a, b)| (*a, *b)));
        self.subtypes
            .extend(o.subtypes.iter().map(|(a, b)| (*a, *b)));
        self.scopes.extend(o.scopes.iter().map(|(a, b)| (*a, *b)));
        self.exprs.extend(o.exprs.iter().map(|(a, b)| (*a, *b)));
        self.stmts.extend(o.stmts.iter().map(|(a, b)| (*a, *b)));
        self.consts.extend(o.consts.iter().map(|(a, b)| (*a, *b)));
        self.formal_firsts.extend(o.formal_firsts.iter().copied());
    }
}

/// What a name of a package-like declaration gives access to.
pub(crate) enum PkgRegion {
    Scope(ScopeId),
    /// An uninstantiated generic package (`E1505`).
    Generic,
    No,
}

/// The last simple name of a (selected) name, for diagnostics.
pub(crate) fn last_span(n: &ast::Name) -> Span {
    match &n.kind {
        ast::NameKind::Selected { suffix, .. } => crate::unit::suffix_span(suffix, n.span),
        _ => n.span,
    }
}

fn formal_key(n: &ast::Name) -> Option<(Symbol, Span)> {
    match &n.kind {
        ast::NameKind::Simple(i) => Some((i.key, i.span)),
        ast::NameKind::Operator(op) => Some((crate::decl::operator_key(op.as_str()), n.span)),
        _ => None,
    }
}

fn actual_span(a: &ast::Actual) -> Span {
    match a {
        ast::Actual::Expr(e) | ast::Actual::Inertial(e) => e.span,
        ast::Actual::Range(ast::Range::Simple { left, .. }) => left.span,
        ast::Actual::Range(ast::Range::Name(n)) => n.span,
        ast::Actual::Subtype(si) => si.span,
        ast::Actual::Open(s) => *s,
    }
}

impl Db {
    // -- regions ------------------------------------------------------------------

    /// The region a package, package instance or formal package name denotes.
    pub(crate) fn package_region(&self, d: DeclId) -> PkgRegion {
        match self.decls[self.entity(d)].kind {
            DeclKind::Unit(u) => match self.units[u].kind {
                UnitKind::Package
                    if matches!(self.units[u].hir, UnitHir::Package { generics: Some(_) }) =>
                {
                    PkgRegion::Generic
                }
                UnitKind::Package | UnitKind::PackageInst => PkgRegion::Scope(self.units[u].scope),
                _ => PkgRegion::No,
            },
            DeclKind::PackageInst(i) | DeclKind::GenericPackage(i) => {
                PkgRegion::Scope(self.insts[i].scope)
            }
            _ => PkgRegion::No,
        }
    }

    /// `E1505` for a use of an uninstantiated generic package or subprogram.
    pub(crate) fn uninstantiated(&mut self, d: DeclId, span: Span) -> Reported {
        let name = self.decls[d].name;
        let what = if self.decls[d].kind.subprogram().is_some() {
            "subprogram"
        } else {
            "package"
        };
        self.error(
            Diagnostic::error(
                diag::UNINSTANTIATED_CALL,
                format!("`{name}` is a generic {what} and must be instantiated before use"),
            )
            .with_primary(span, format!("uninstantiated generic {what}"))
            .with_secondary(self.decls[d].span, "declared here")
            .with_help(format!(
                "declare an instance: `{what} i is new {name} generic map (...);`"
            )),
        )
    }

    /// The generic subprogram a call names when every candidate is uninstantiated (`E1505`).
    pub(crate) fn uninstantiated_of(&self, r: &crate::expr::names::NameRes) -> Option<DeclId> {
        use crate::expr::names::NameRes;
        let ds = match r {
            NameRes::One(d) => std::slice::from_ref(d),
            NameRes::Overloads(ds) => ds.as_slice(),
            _ => return None,
        };
        let generic = |d: &DeclId| {
            self.decls[self.entity(*d)]
                .kind
                .subprogram()
                .is_some_and(|s| s.generics.is_some())
        };
        (!ds.is_empty() && ds.iter().all(generic)).then(|| self.entity(ds[0]))
    }

    /// `W1501` for the generics of `list` that nothing referred to. Called once
    /// the whole unit is seen: the package body (or the package declaration
    /// when it has none) or the subprogram body.
    pub(crate) fn report_unused_generics(&mut self, list: &[DeclId]) {
        for &d in list {
            if self.generic.unused.remove(&d) {
                let name = self.decls[d].name;
                self.report(
                    Diagnostic::warning(
                        diag::W_UNUSED_GENERIC,
                        format!("generic `{name}` is never used"),
                    )
                    .with_primary(self.decls[d].span, "declared here, never referred to"),
                );
            }
        }
    }

    /// The body of a generic protected type sees the generics of its declaration.
    pub(crate) fn protected_body_generics(&mut self, t: TypeId, bscope: ScopeId) {
        let TypeKind::Protected {
            generics: Some(g), ..
        } = &self.types[t].kind
        else {
            return;
        };
        let pscope = self.decls[g[0]].scope;
        let decls: Vec<DeclId> = self.scopes[pscope]
            .order
            .iter()
            .copied()
            .take_while(|d| {
                // The generics and their class operations come before the methods.
                g.contains(d) || self.decls[*d].implicit
            })
            .collect();
        for d in decls {
            let k = self.decls[d].name.key();
            self.scopes[bscope].names.entry(k).or_default().push(d);
            self.scopes[bscope].order.push(d);
        }
    }

    /// `E1505` for an object of a generic protected type: 2019 instantiates it
    /// with a generic map on the variable declaration, which yavs does not parse yet.
    pub(crate) fn check_generic_protected(
        &mut self,
        st: SubtypeId,
        span: Span,
    ) -> Result<(), Reported> {
        let t = self.base(st);
        if !matches!(
            self.types[t].kind,
            TypeKind::Protected {
                generics: Some(_),
                ..
            }
        ) {
            return Ok(());
        }
        let d = self.types[t].decl;
        let name = self.decls[d].name;
        Err(self.error(
            Diagnostic::error(
                diag::UNINSTANTIATED_CALL,
                format!("`{name}` is a generic protected type and must be instantiated before use"),
            )
            .with_primary(span, "uninstantiated generic protected type")
            .with_secondary(self.decls[d].span, "declared here")
            .with_note("VHDL-2019 instantiates it with a generic map on the variable declaration, which yavs does not accept yet"),
        ))
    }

    // -- generic lists ------------------------------------------------------------

    /// Analyze a generic clause into `scope` (6.5.6): constants, types (with
    /// their class operations), subprograms, packages, in order.
    pub(crate) fn generic_list(
        &mut self,
        scope: ScopeId,
        list: &[ast::InterfaceDecl],
    ) -> Result<Box<[DeclId]>, Reported> {
        if let Some(spec) = self.generic.reuse.take() {
            return Ok(self.reuse_generics(scope, spec));
        }
        let mut out = Vec::new();
        let mut failed = false;
        for g in list {
            match self.generic_item(scope, g) {
                Ok(ds) => {
                    // Before the next item: a later generic may refer to this one.
                    for &d in &ds {
                        if matches!(
                            self.decls[d].kind,
                            DeclKind::Object(_)
                                | DeclKind::GenericType(_)
                                | DeclKind::GenericPackage(_)
                        ) {
                            self.generic.unused.insert(d);
                        }
                    }
                    out.extend(ds);
                }
                Err(_) => failed = true,
            }
        }
        let out: Box<[DeclId]> = out.into();
        self.generic.lists.insert(scope, out.clone());
        if failed { Err(Reported) } else { Ok(out) }
    }

    pub(crate) fn generic_item(
        &mut self,
        scope: ScopeId,
        g: &ast::InterfaceDecl,
    ) -> Result<Vec<DeclId>, Reported> {
        match g {
            ast::InterfaceDecl::Object(o) => self.generic_object(scope, o),
            ast::InterfaceDecl::Type(t) => {
                let class = self.generic_class(scope, t.def.as_ref(), t.span)?;
                let decl = self.new_decl(scope, Designator::Ident(t.name), DeclKind::Error, t.span);
                let constraint = match class {
                    GenericClass::Array { .. } => Constraint::Array {
                        dims: None,
                        elem: Box::new(Constraint::None),
                    },
                    _ => Constraint::None,
                };
                let ty = self.new_type(
                    decl,
                    t.name,
                    false,
                    TypeKind::Generic { class },
                    constraint,
                    t.span,
                );
                self.decls[decl].kind = DeclKind::GenericType(ty);
                self.declare(scope, decl);
                // 2008 6.5.3: `=` and `/=` (and the class operations, 2019) follow the type.
                self.declare_implicit_ops(ty, scope);
                Ok(vec![decl])
            }
            ast::InterfaceDecl::Subprogram(s) => {
                if s.spec.generic_map.is_some() {
                    return Err(self.unsupported(
                        s.spec.span,
                        "generic subprograms with a generic map as a generic",
                        "S4",
                    ));
                }
                let d = self.subprogram_spec(scope, &s.spec)?;
                if let DeclKind::Subprogram(mut sp) =
                    std::mem::replace(&mut self.decls[d].kind, DeclKind::Error)
                {
                    sp.body = Body::Interface;
                    self.decls[d].kind = DeclKind::GenericSubprogram(sp);
                }
                match &s.default {
                    Some(ast::SubprogramDefault::Box) => {
                        self.generic.defaults.insert(d, SubpDefault::Box);
                    }
                    Some(ast::SubprogramDefault::Name(n)) => {
                        use crate::expr::names::NameRes;
                        let cands = match self.classify(scope, n) {
                            NameRes::One(x) => vec![x],
                            NameRes::Overloads(xs) => xs,
                            other => {
                                return Err(self.report_name(
                                    scope,
                                    other,
                                    n.span,
                                    "a subprogram name",
                                ));
                            }
                        };
                        self.generic.defaults.insert(
                            d,
                            SubpDefault::Name {
                                cands,
                                span: n.span,
                            },
                        );
                    }
                    None => {}
                }
                self.declare(scope, d);
                Ok(vec![d])
            }
            ast::InterfaceDecl::Package(p) => {
                let g = self.generic_package_of(scope, &p.package)?;
                let DeclKind::Unit(u) = self.decls[g].kind else {
                    return Err(self.ice(p.span, "generic package is not a unit"));
                };
                let gscope = self.units[u].scope;
                let wants = match &p.generic_map {
                    ast::InterfacePackageMap::Assoc(assocs) => {
                        let Some(formals) = self.generic.lists.get(&gscope).cloned() else {
                            return Err(self.ice(p.span, "generic package without generics"));
                        };
                        let what = format!("package `{}`", self.decls[g].name);
                        let (actuals, ok) = self.associate(&what, &formals, assocs);
                        let wants = self.formal_package_wants(scope, &formals, &actuals);
                        if !ok {
                            return Err(Reported);
                        }
                        wants?
                    }
                    _ => Vec::new(),
                };
                let decl = self.new_decl(scope, Designator::Ident(p.name), DeclKind::Error, p.span);
                let mangled = Symbol::intern(&crate::mangle::region(self, gscope));
                let inst = self.insts.push(Instance {
                    generic: g,
                    site: decl,
                    map: GenericMap::default(),
                    scope: gscope,
                    body: OnceCell::new(),
                    mangled,
                });
                self.generic.maps.insert(inst, Remap::default());
                if !wants.is_empty() {
                    self.generic.wants.insert(inst, wants);
                }
                self.decls[decl].kind = DeclKind::GenericPackage(inst);
                self.declare(scope, decl);
                Ok(vec![decl])
            }
        }
    }

    /// Analyze the actuals of a formal package's generic map where it is declared.
    /// ponytail: inside the generic the formal package stays opaque (its generics
    /// do not take these values); they are only checked against the actual instance.
    fn formal_package_wants(
        &mut self,
        scope: ScopeId,
        formals: &[DeclId],
        actuals: &[Option<&ast::AssocElem>],
    ) -> Result<Vec<(DeclId, Want)>, Reported> {
        use crate::expr::names::NameRes;
        let mut out = Vec::new();
        for (&f, a) in formals.iter().zip(actuals) {
            let Some(a) = a else { continue };
            let want = match (&self.decls[f].kind, &a.actual) {
                (_, ast::Actual::Open(_)) => continue,
                (DeclKind::Object(o), ast::Actual::Expr(e)) => {
                    let st = o.subtype;
                    if matches!(self.types[self.base(st)].kind, TypeKind::Generic { .. }) {
                        // The type is another generic of the same package: not checked.
                        continue;
                    }
                    Want::Const(self.expr_in_subtype(scope, e, st))
                }
                (DeclKind::GenericType(_), ast::Actual::Subtype(si)) => {
                    Want::Type(self.subtype_indication(scope, si)?)
                }
                (
                    DeclKind::GenericType(_),
                    ast::Actual::Expr(ast::Expr {
                        kind: ast::ExprKind::Name(n),
                        ..
                    }),
                ) => Want::Type(self.type_mark(scope, n)?),
                (
                    DeclKind::GenericSubprogram(_),
                    ast::Actual::Expr(ast::Expr {
                        kind: ast::ExprKind::Name(n),
                        ..
                    }),
                ) => match self.classify(scope, n) {
                    NameRes::One(d) => Want::Subprogram(vec![self.entity(d)]),
                    NameRes::Overloads(ds) => {
                        Want::Subprogram(ds.into_iter().map(|d| self.entity(d)).collect())
                    }
                    other => {
                        return Err(self.report_name(scope, other, n.span, "a subprogram name"));
                    }
                },
                (_, other) => {
                    return Err(self.unsupported(
                        actual_span(other),
                        "this actual in the generic map of a formal package",
                        "S4",
                    ));
                }
            };
            out.push((f, want));
        }
        Ok(out)
    }

    /// `E1503` when actual instance `i` was not instantiated with what formal
    /// package `fi` requires (6.5.7.2). Non-static values are not compared.
    fn check_wants(
        &mut self,
        r: &mut Remap,
        fi: InstId,
        i: InstId,
        span: Span,
    ) -> Result<(), Reported> {
        let Some(wants) = self.generic.wants.get(&fi).cloned() else {
            return Ok(());
        };
        let iname = self.decls[self.insts[i].site].name;
        for (f, want) in wants {
            let fname = self.decls[f].name;
            let bad = match want {
                Want::Const(e) => {
                    let e = self.r_expr(r, e);
                    let Some(&got) = self.insts[i].map.consts.get(&f) else {
                        continue;
                    };
                    matches!((self.value(e), self.value(got)), (Some(a), Some(b)) if a != b)
                }
                Want::Type(st) => {
                    let st = self.r_subtype(r, st);
                    let t = match self.decls[f].kind {
                        DeclKind::GenericType(t) => t,
                        _ => continue,
                    };
                    let Some(&got) = self.insts[i].map.types.get(&t) else {
                        continue;
                    };
                    self.base(st) != self.base(got)
                }
                Want::Subprogram(cands) => {
                    let Some(&got) = self.insts[i].map.subprograms.get(&f) else {
                        continue;
                    };
                    let cands: Vec<DeclId> = cands.into_iter().map(|d| self.r_decl(r, d)).collect();
                    !cands.contains(&self.entity(got))
                }
            };
            if bad {
                return Err(self.error(
                    Diagnostic::error(
                        diag::GENERIC_CLASS,
                        format!("`{iname}` was not instantiated with the `{fname}` the formal package requires"),
                    )
                    .with_primary(span, format!("`{fname}` differs"))
                    .with_secondary(self.decls[self.insts[fi].site].span, "formal package declared here")
                    .with_note("LRM 6.5.7.2: the actual of a formal package with a generic map must match its actuals"),
                ));
            }
        }
        Ok(())
    }

    /// Generic constants (6.5.6.2: class constant, mode `in`).
    fn generic_object(
        &mut self,
        scope: ScopeId,
        o: &ast::InterfaceObject,
    ) -> Result<Vec<DeclId>, Reported> {
        if !matches!(o.class, None | Some(ast::InterfaceClass::Constant))
            || !matches!(o.mode, None | Some(ast::Mode::In))
        {
            return Err(self.error(
                Diagnostic::error(
                    diag::CLASS_MISUSE,
                    "a generic must be a constant of mode `in`",
                )
                .with_primary(o.span, "not a generic constant")
                .with_note(
                    "LRM 6.5.6.2: only constants, types, subprograms and packages can be generics",
                ),
            ));
        }
        let st = match &o.subtype {
            Some(ast::IncompleteSubtype::Subtype(si)) => self.subtype_indication(scope, si)?,
            _ => return Err(self.unsupported(o.span, "anonymous generic types", "S7")),
        };
        let default = o
            .default
            .as_ref()
            .map(|e| self.expr_in_subtype(scope, e, st));
        let bounds = if self.constrainedness(st) == Constrainedness::Full {
            BoundsFrom::Subtype
        } else {
            BoundsFrom::Actual
        };
        let mut out = Vec::new();
        for &name in &o.names {
            let d = self.new_decl(
                scope,
                Designator::Ident(name),
                DeclKind::Object(Object {
                    class: ObjClass::Constant,
                    role: ObjRole::Generic,
                    mode: Some(Mode::In),
                    subtype: st,
                    default,
                    bounds,
                    signal_kind: None,
                    file_open: None,
                }),
                name.span,
            );
            self.declare(scope, d);
            out.push(d);
        }
        Ok(out)
    }

    fn generic_class(
        &mut self,
        scope: ScopeId,
        def: Option<&ast::GenericTypeDef>,
        span: Span,
    ) -> Result<GenericClass, Reported> {
        use ast::GenericTypeDef as G;
        let elem = |db: &mut Db, s: &ast::IncompleteSubtype| match s {
            ast::IncompleteSubtype::Subtype(si) => db.subtype_indication(scope, si),
            ast::IncompleteSubtype::Anonymous(_, sp) => {
                Err(db.unsupported(*sp, "anonymous generic types", "S7"))
            }
        };
        Ok(match def {
            None | Some(G::Private) => GenericClass::Private,
            Some(G::Scalar) => GenericClass::Scalar,
            Some(G::Discrete) => GenericClass::Discrete,
            Some(G::Integer) => GenericClass::Integer,
            Some(G::Physical) => GenericClass::Physical,
            Some(G::Floating) => GenericClass::Floating,
            Some(G::Array { indexes, element }) => {
                let ast::ArrayIndexes::Unconstrained(marks) = indexes else {
                    return Err(self.unsupported(
                        span,
                        "generic array types with constrained or anonymous index types",
                        "S7",
                    ));
                };
                let mut idx = Vec::new();
                for m in marks {
                    idx.push(self.type_mark(scope, m)?);
                }
                GenericClass::Array {
                    indexes: idx.into(),
                    elem: elem(self, element)?,
                }
            }
            Some(G::Access(s)) => GenericClass::Access {
                designated: elem(self, s)?,
            },
            Some(G::File(s)) => GenericClass::File {
                elem: elem(self, s)?,
            },
        })
    }

    /// The generic package a name denotes (`E1501` otherwise).
    fn generic_package_of(&mut self, scope: ScopeId, n: &ast::Name) -> Result<DeclId, Reported> {
        let d = self.expanded_name_decl(scope, n)?;
        let d = self.entity(d);
        match self.package_region(d) {
            PkgRegion::Generic => Ok(d),
            _ if matches!(self.decls[d].kind, DeclKind::Error) => Err(Reported),
            _ => {
                let name = self.decls[d].name;
                Err(self.error(
                    Diagnostic::error(
                        diag::NOT_GENERIC,
                        format!("`{name}` is not a generic package"),
                    )
                    .with_primary(last_span(n), "expected an uninstantiated generic package")
                    .with_secondary(self.decls[d].span, "declared here"),
                ))
            }
        }
    }

    // -- instantiation --------------------------------------------------------------

    /// `package i is new g generic map (...);` as a library unit, into the unit's region.
    pub(crate) fn package_instance_unit(
        &mut self,
        scope: ScopeId,
        site: DeclId,
        p: &ast::PackageInstance,
    ) -> Result<InstId, Reported> {
        self.package_instance(scope, scope, site, p)
    }

    /// A package instantiation declaration in a declarative part (4.9).
    pub(crate) fn package_instance_decl(
        &mut self,
        scope: ScopeId,
        p: &ast::PackageInstance,
    ) -> Result<(), Reported> {
        let d = self.new_decl(scope, Designator::Ident(p.name), DeclKind::Error, p.span);
        let s = self.add_scope(ScopeKind::Instance, Some(scope), Some(d));
        let inst = self.package_instance(scope, s, d, p)?;
        self.decls[d].kind = DeclKind::PackageInst(inst);
        self.declare(scope, d);
        Ok(())
    }

    fn package_instance(
        &mut self,
        scope: ScopeId,
        inst_scope: ScopeId,
        site: DeclId,
        p: &ast::PackageInstance,
    ) -> Result<InstId, Reported> {
        let g = self.generic_package_of(scope, &p.package)?;
        let DeclKind::Unit(u) = self.decls[g].kind else {
            return Err(self.ice(p.span, "generic package is not a unit"));
        };
        let gscope = self.units[u].scope;
        let UnitHir::Package {
            generics: Some(formals),
        } = &self.units[u].hir
        else {
            return Err(self.ice(p.span, "generic package without generics"));
        };
        let formals = formals.clone();
        let what = format!("package `{}`", self.decls[g].name);
        let assocs = p.generic_map.as_deref().unwrap_or(&[]);
        let (actuals, assoc_ok) = self.associate(&what, &formals, assocs);
        let mut r = Remap::new(gscope, inst_scope);
        let mut map = GenericMap::default();
        let overflow = self.overflow.len();
        let bound = self.bind_formals(
            scope,
            inst_scope,
            p.name.span,
            gscope,
            &formals,
            &actuals,
            &mut r,
            &mut map,
        );
        if bound.is_ok() {
            self.fill_scope(&mut r, gscope, inst_scope);
        }
        self.overflow.truncate(overflow);
        bound?;
        if r.failed || !assoc_ok {
            return Err(Reported);
        }
        let mangled = Symbol::intern(&crate::mangle::region(self, inst_scope));
        let inst = self.insts.push(Instance {
            generic: g,
            site,
            map,
            scope: inst_scope,
            body: OnceCell::new(),
            mangled,
        });
        self.generic.maps.insert(inst, r);
        Ok(inst)
    }

    /// `function f is new g [signature] generic map (...);` (4.4).
    pub(crate) fn subprogram_instance(
        &mut self,
        scope: ScopeId,
        x: &ast::SubprogramInstance,
    ) -> Result<(), Reported> {
        use crate::expr::names::NameRes;
        let (name, sig) = match &x.subprogram.kind {
            ast::NameKind::Signature { prefix, signature } => (prefix.as_ref(), Some(signature)),
            _ => (&x.subprogram, None),
        };
        let sig = match sig {
            Some(s) => Some(self.signature_of(scope, s)?),
            None => None,
        };
        let cands = match self.classify(scope, name) {
            NameRes::One(d) => vec![d],
            NameRes::Overloads(ds) => ds,
            other => return Err(self.report_name(scope, other, name.span, "a subprogram name")),
        };
        let want_fn = matches!(x.kind, ast::SubprogramKind::Function(_));
        let cands: Vec<DeclId> = cands
            .into_iter()
            .map(|d| self.entity(d))
            .filter(|&d| match self.decls[d].kind.subprogram() {
                Some(s) => {
                    s.generics.is_some()
                        && matches!(s.kind, SubprogramKind::Function { .. }) == want_fn
                }
                None => false,
            })
            .filter(|&d| sig.as_ref().is_none_or(|s| self.signature_matches(d, s)))
            .collect();
        let g = match cands.as_slice() {
            [g] => *g,
            [] => {
                return Err(self.error(
                    Diagnostic::error(
                        diag::NOT_GENERIC,
                        format!(
                            "no generic {} matches `{}`",
                            if want_fn { "function" } else { "procedure" },
                            last_name(name)
                        ),
                    )
                    .with_primary(x.subprogram.span, "expected an uninstantiated subprogram"),
                ));
            }
            many => {
                let mut d = Diagnostic::error(
                    diag::AMBIGUOUS,
                    "several generic subprograms match the instantiated name",
                )
                .with_primary(x.subprogram.span, "ambiguous")
                .with_help("add a signature: `is new g [t return t]`");
                for &c in many {
                    d = d.with_secondary(self.decls[c].span, "candidate");
                }
                return Err(self.error(d));
            }
        };
        let assocs = x.generic_map.as_deref().unwrap_or(&[]);
        let dname = crate::alias::designator(&x.designator);
        let d = self.instantiate_subprogram(scope, g, assocs, dname, x.span)?;
        self.declare(scope, d);
        Ok(())
    }

    /// A subprogram declaration with a generic list and a generic map in its header
    /// (4.2.1): `g` is the uninstantiated subprogram `subprogram_spec` built from it,
    /// never declared; the instance is returned for the caller to declare.
    pub(crate) fn header_instance(
        &mut self,
        scope: ScopeId,
        g: DeclId,
        s: &ast::SubprogramSpec,
    ) -> Result<DeclId, Reported> {
        let Some(assocs) = &s.generic_map else {
            return Ok(g);
        };
        self.generic.header.insert(g);
        let name = self.decls[g].name;
        self.instantiate_subprogram(scope, g, assocs, name, s.span)
    }

    /// The uninstantiated subprogram behind `d` when `d` is the instance a header
    /// generic map declared (`header_instance`).
    pub(crate) fn header_generic(&self, d: DeclId) -> Option<DeclId> {
        let Body::Instance(i) = self.decls[d].kind.subprogram()?.body else {
            return None;
        };
        let g = self.insts[i].generic;
        self.generic.header.contains(&g).then_some(g)
    }

    /// Instance `dname` of generic subprogram `g` with generic map `assocs` (4.4):
    /// generics bound, profile cloned, body cloned on demand (`instance_body`).
    fn instantiate_subprogram(
        &mut self,
        scope: ScopeId,
        g: DeclId,
        assocs: &[ast::AssocElem],
        dname: Designator,
        span: Span,
    ) -> Result<DeclId, Reported> {
        let s = self.decls[g]
            .kind
            .subprogram()
            .expect("a generic subprogram");
        let formals = s.generics.clone().expect("a generic subprogram");
        let gscope = s.scope.expect("specs have a parameter region");
        let kind = s.kind;
        let params: Vec<(Param, Option<DeclId>)> = s
            .params
            .iter()
            .map(|p| {
                (
                    Param {
                        name: p.name,
                        class: p.class,
                        mode: p.mode,
                        subtype: p.subtype,
                        default: p.default,
                        decl: p.decl,
                    },
                    p.decl,
                )
            })
            .collect();
        let ret = s.ret;
        let what = format!("subprogram `{}`", self.decls[g].name);
        let (actuals, assoc_ok) = self.associate(&what, &formals, assocs);
        let d = self.new_decl(scope, dname, DeclKind::Error, span);
        let new_scope = self.add_scope(ScopeKind::Subprogram, Some(scope), Some(d));
        let mut r = Remap::new(gscope, new_scope);
        let mut map = GenericMap::default();
        let overflow = self.overflow.len();
        let bound = self.bind_formals(
            scope, new_scope, span, gscope, &formals, &actuals, &mut r, &mut map,
        );
        if bound.is_ok() {
            self.fill_scope(&mut r, gscope, new_scope);
        }
        bound?;
        if !assoc_ok {
            return Err(Reported);
        }
        let params: Box<[Param]> = params
            .into_iter()
            .map(|(p, decl)| Param {
                subtype: self.r_subtype(&mut r, p.subtype),
                default: p.default.map(|e| self.r_expr(&mut r, e)),
                decl: decl.map(|x| self.r_decl(&mut r, x)),
                ..p
            })
            .collect();
        let ret = ret.map(|t| self.r_subtype(&mut r, t));
        self.overflow.truncate(overflow);
        if r.failed {
            return Err(Reported);
        }
        let types: Vec<SubtypeId> = params.iter().map(|p| p.subtype).collect();
        let mangled = Symbol::intern(&crate::mangle::subprogram(self, scope, &dname, &types, ret));
        let inst = self.insts.push(Instance {
            generic: g,
            site: d,
            map,
            scope: new_scope,
            body: OnceCell::new(),
            mangled,
        });
        self.generic.maps.insert(inst, r);
        self.decls[d].kind = DeclKind::Subprogram(Subprogram {
            kind,
            params,
            ret,
            generics: None,
            scope: Some(new_scope),
            body: Body::Instance(inst),
            mangled,
        });
        Ok(d)
    }

    /// Match a generic map against the formals (6.5.7.2): positional first,
    /// then named; `E1310` twice, `E1100` unknown. One entry per formal; the
    /// flag is false after an error (binding still runs, so a missing actual is
    /// reported too).
    fn associate<'a>(
        &mut self,
        what: &str,
        formals: &[DeclId],
        assocs: &'a [ast::AssocElem],
    ) -> (Vec<Option<&'a ast::AssocElem>>, bool) {
        let mut out: Vec<Option<&ast::AssocElem>> = vec![None; formals.len()];
        let mut failed = false;
        let mut pos = 0;
        for a in assocs {
            let i = match &a.formal {
                None => {
                    pos += 1;
                    if pos > formals.len() {
                        failed = true;
                        self.report(
                            Diagnostic::error(
                                diag::FORMAL_TWICE,
                                format!("{what} has only {} generics", formals.len()),
                            )
                            .with_primary(a.span, "no generic left for this actual"),
                        );
                        continue;
                    }
                    pos - 1
                }
                Some(f) => {
                    let Some((key, span)) = formal_key(f) else {
                        failed = true;
                        self.unsupported(f.span, "this form of generic formal", "S4");
                        continue;
                    };
                    // Overloaded formal subprograms: the first one not yet associated.
                    let found = formals
                        .iter()
                        .enumerate()
                        .filter(|(_, d)| self.decls[**d].name.key() == key)
                        .map(|(i, _)| i)
                        .find(|&i| out[i].is_none())
                        .or_else(|| {
                            formals
                                .iter()
                                .position(|d| self.decls[*d].name.key() == key)
                        });
                    let Some(i) = found else {
                        failed = true;
                        let mut d = Diagnostic::error(
                            diag::NOT_DECLARED,
                            format!("{what} has no generic `{key}`"),
                        )
                        .with_primary(span, "no such generic");
                        let keys: Vec<Symbol> =
                            formals.iter().map(|d| self.decls[*d].name.key()).collect();
                        if let Some(best) =
                            crate::suggest::closest(key.as_str(), keys.iter().map(|k| k.as_str()))
                        {
                            d = d.with_help(format!("did you mean `{best}`?"));
                        }
                        self.report(d);
                        continue;
                    };
                    i
                }
            };
            if out[i].is_some() {
                failed = true;
                let name = self.decls[formals[i]].name;
                let span = a.formal.as_ref().map_or(a.span, |f| f.span);
                self.report(
                    Diagnostic::error(
                        diag::FORMAL_TWICE,
                        format!("generic `{name}` is associated twice"),
                    )
                    .with_primary(span, "associated again here")
                    .with_secondary(out[i].map_or(a.span, |p| p.span), "first associated here"),
                );
                continue;
            }
            out[i] = Some(a);
        }
        (out, !failed)
    }

    /// Bind each formal to its actual (or default), in order, seeding `r` (5.2).
    #[allow(clippy::too_many_arguments)]
    fn bind_formals(
        &mut self,
        scope: ScopeId,
        inst_scope: ScopeId,
        site_span: Span,
        gscope: ScopeId,
        formals: &[DeclId],
        actuals: &[Option<&ast::AssocElem>],
        r: &mut Remap,
        map: &mut GenericMap,
    ) -> Result<(), Reported> {
        let mut failed = false;
        // Actual spans of formal types, to place `E1504` at the actual that lacks the operation.
        let mut type_spans: HashMap<TypeId, Span> = HashMap::new();
        for (i, &f) in formals.iter().enumerate() {
            let a = actuals[i];
            let ok = match &self.decls[f].kind {
                DeclKind::Object(_) => self.bind_const(scope, inst_scope, site_span, f, a, r, map),
                DeclKind::GenericType(t) => {
                    let t = *t;
                    let res = self.bind_type(scope, inst_scope, site_span, gscope, f, t, a, r, map);
                    if let Some(a) = a {
                        type_spans.insert(t, actual_span(&a.actual));
                    }
                    res
                }
                DeclKind::GenericSubprogram(_) => {
                    self.bind_subprogram(scope, site_span, f, a, &type_spans, r, map)
                }
                DeclKind::GenericPackage(_) => {
                    self.bind_package(scope, inst_scope, site_span, f, a, r, map)
                }
                DeclKind::Error => Err(Reported),
                _ => Err(self.ice(self.decls[f].span, "unexpected generic declaration")),
            };
            failed |= ok.is_err();
        }
        if failed { Err(Reported) } else { Ok(()) }
    }

    fn missing_generic(&mut self, f: DeclId, site: Span, what: &str) -> Reported {
        let name = self.decls[f].name;
        self.error(
            Diagnostic::error(
                diag::MISSING_GENERIC,
                format!(
                    "generic {what}`{name}` has no actual{}",
                    if what.is_empty() {
                        " and no default"
                    } else {
                        ""
                    }
                ),
            )
            .with_primary(site, "in this instantiation")
            .with_secondary(self.decls[f].span, "generic declared here")
            .with_help(format!("add `{name} => ...` to the generic map")),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn bind_const(
        &mut self,
        scope: ScopeId,
        inst_scope: ScopeId,
        site: Span,
        f: DeclId,
        a: Option<&ast::AssocElem>,
        r: &mut Remap,
        map: &mut GenericMap,
    ) -> Result<(), Reported> {
        let DeclKind::Object(o) = &self.decls[f].kind else {
            return Err(Reported);
        };
        let (fst, fdefault, bounds) = (o.subtype, o.default, o.bounds);
        let st = self.r_subtype(r, fst);
        let actual = match a.map(|a| &a.actual) {
            Some(ast::Actual::Expr(e)) => self.expr_in_subtype(scope, e, st),
            Some(ast::Actual::Open(_)) | None => match fdefault {
                Some(d) => self.r_expr(r, d),
                None => return Err(self.missing_generic(f, site, "")),
            },
            Some(other) => {
                return Err(self.error(
                    Diagnostic::error(
                        diag::TYPE_MISMATCH,
                        "the actual of a generic constant must be an expression",
                    )
                    .with_primary(actual_span(other), "not an expression"),
                ));
            }
        };
        if matches!(self.exprs[actual].kind, ExprKind::Error) {
            return Err(Reported);
        }
        let name = self.decls[f].name;
        let span = self.decls[f].span;
        let new = self.new_decl(
            inst_scope,
            name,
            DeclKind::Object(Object {
                class: ObjClass::Constant,
                role: ObjRole::Generic,
                mode: Some(Mode::In),
                subtype: st,
                default: Some(actual),
                bounds,
                signal_kind: None,
                file_open: None,
            }),
            span,
        );
        r.decls.insert(f, new);
        r.consts.insert(f, actual);
        map.consts.insert(f, actual);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn bind_type(
        &mut self,
        scope: ScopeId,
        inst_scope: ScopeId,
        site: Span,
        gscope: ScopeId,
        f: DeclId,
        t: TypeId,
        a: Option<&ast::AssocElem>,
        r: &mut Remap,
        map: &mut GenericMap,
    ) -> Result<(), Reported> {
        let Some(a) = a else {
            return Err(self.missing_generic(f, site, "type "));
        };
        let span = actual_span(&a.actual);
        let actual = match &a.actual {
            ast::Actual::Subtype(si) => self.subtype_indication(scope, si)?,
            ast::Actual::Expr(ast::Expr {
                kind: ast::ExprKind::Name(n),
                ..
            }) => match self.index_constrained_name(scope, n) {
                // `t_element => std_logic_vector(W - 1 downto 0)` parses as a name (UVVM).
                Some(st) => st?,
                None => self.type_mark(scope, n)?,
            },
            ast::Actual::Expr(e) => {
                return Err(self.error(
                    Diagnostic::error(
                        diag::NOT_A_TYPE,
                        "the actual of a generic type must be a subtype",
                    )
                    .with_primary(e.span, "not a type mark"),
                ));
            }
            other => {
                return Err(self.error(
                    Diagnostic::error(
                        diag::NOT_A_TYPE,
                        "the actual of a generic type must be a subtype",
                    )
                    .with_primary(actual_span(other), "not a subtype indication"),
                ));
            }
        };
        let TypeKind::Generic { class } = &self.types[t].kind else {
            return Err(Reported);
        };
        let class = class.clone();
        self.check_class(&class, actual, span, r)?;
        let warn = matches!(class, GenericClass::Private).then_some(scope);
        self.seed_type(warn, inst_scope, gscope, f, t, actual, span, r, map)
    }

    /// Map formal type `t` (declared by `f` in `gscope`) to `actual` in `r`: the formal's
    /// subtypes, a subtype declaration in `inst_scope`, and the formal's class operations
    /// as the actual's predefined ones (Q9). `warn`: the instantiating region, for `W1506`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn seed_type(
        &mut self,
        warn: Option<ScopeId>,
        inst_scope: ScopeId,
        gscope: ScopeId,
        f: DeclId,
        t: TypeId,
        actual: SubtypeId,
        span: Span,
        r: &mut Remap,
        map: &mut GenericMap,
    ) -> Result<(), Reported> {
        let base = self.base(actual);
        let first = self.types[t].first_subtype;
        let tbase = self.types[t].base_subtype;
        r.types.insert(t, base);
        r.subtypes.insert(first, actual);
        r.subtypes.insert(tbase, actual);
        r.formal_firsts.insert(first);
        map.types.insert(t, actual);
        let name = self.decls[f].name;
        let new = self.new_decl(
            inst_scope,
            name,
            DeclKind::Subtype(actual),
            self.decls[f].span,
        );
        r.decls.insert(f, new);
        // The class operations of the formal denote the actual's predefined ones (Q9).
        let ops: Vec<(DeclId, ImplicitOp)> = self.scopes[gscope]
            .order
            .iter()
            .filter_map(
                |&d| match self.decls[d].kind.subprogram().map(|s| &s.body) {
                    Some(Body::Implicit(op)) if op.ty == t => Some((d, *op)),
                    _ => None,
                },
            )
            .collect();
        let mut failed = false;
        for (d, op) in ops {
            match self.actual_op(base, op, self.decls[d].name.key()) {
                Some(x) => {
                    r.decls.insert(d, x);
                    map.subprograms.insert(d, x);
                    if op.family == crate::implicit::Family::Eq
                        && let Some(scope) = warn
                    {
                        self.warn_user_eq(scope, x, base, span);
                    }
                }
                None if self.is_error_type(base) => {
                    r.decls.insert(d, self.poison.decl);
                }
                None => {
                    failed = true;
                    let (n, an) = (self.decls[d].name, self.type_name(base));
                    self.report(
                        Diagnostic::error(
                            diag::GENERIC_CLASS,
                            format!("`{an}` has no predefined {n}, which generic type `{name}` needs"),
                        )
                        .with_primary(span, format!("no predefined {n}"))
                        .with_note("LRM 6.5.3: the operations of an interface type are the predefined operations of the actual"),
                    );
                }
            }
        }
        if failed { Err(Reported) } else { Ok(()) }
    }

    fn is_error_type(&self, t: TypeId) -> bool {
        matches!(self.types[t].kind, TypeKind::Error)
    }

    /// The predefined operation of `ty` of the same family and form as `op`.
    fn actual_op(&self, ty: TypeId, op: ImplicitOp, key: Symbol) -> Option<DeclId> {
        let scope = self.decls[self.types[ty].decl].scope;
        let find = |s: ScopeId| {
            self.scopes[s].order.iter().copied().find(|&d| {
                self.decls[d].name.key() == key
                    && matches!(self.decls[d].kind.subprogram().map(|s| &s.body),
                        Some(Body::Implicit(o)) if o.ty == ty && o.family == op.family && o.form == op.form)
            })
        };
        find(scope).or_else(|| {
            // The universal types' operations live in std.standard.
            self.well_known.standard.and_then(find)
        })
    }

    /// `W1506` (U-6): the instance uses the actual's predefined `"="` while a
    /// different user `"="` for the actual type is visible at the instantiation.
    fn warn_user_eq(&mut self, scope: ScopeId, predefined: DeclId, ty: TypeId, span: Span) {
        let key = crate::decl::operator_key("=");
        let ds = match self.lookup(scope, key) {
            Visible::Overloads(ds) => ds,
            Visible::One(d) => vec![d],
            _ => return,
        };
        let boolean = self.well_known.boolean;
        let user = ds.into_iter().map(|d| self.entity(d)).find(|&d| {
            d != predefined
                && !self.decls[d].implicit
                && self
                    .profile(d)
                    .is_some_and(|p| p.params == [ty, ty] && p.ret.is_some() && p.ret == boolean)
        });
        if let Some(u) = user {
            let an = self.type_name(ty);
            self.report(
                Diagnostic::warning(
                    diag::W_GENERIC_EQ,
                    format!("this instance compares `{an}` values with the predefined `\"=\"`, not the visible user `\"=\"`"),
                )
                .with_primary(span, format!("the generic type's `=` is the predefined `=` of `{an}`"))
                .with_secondary(self.decls[u].span, "this `\"=\"` is visible here but not used")
                .with_note("LRM 6.5.3: the `=` of an interface type is the actual's predefined operator; NVC uses the visible user one instead (yavs decision U-6)")
                .with_help("to use the user `\"=\"`, declare a formal `function \"=\" (l, r : t) return boolean is <>` in the generic"),
            );
        }
    }

    /// `E1503`: the actual belongs to the class of the formal (2.9; 2019 6.5.3).
    fn check_class(
        &mut self,
        class: &GenericClass,
        actual: SubtypeId,
        span: Span,
        r: &mut Remap,
    ) -> Result<(), Reported> {
        let base = self.base(actual);
        let kind = &self.types[base].kind;
        if matches!(kind, TypeKind::Error) {
            return Err(Reported);
        }
        let gen_class = match kind {
            TypeKind::Generic { class } => Some(class.clone()),
            _ => None,
        };
        let is = |want: fn(&TypeKind) -> bool, gc: &[fn(&GenericClass) -> bool]| {
            want(kind) || gen_class.as_ref().is_some_and(|c| gc.iter().any(|f| f(c)))
        };
        let an = self.subtype_name(actual);
        let fail = |db: &mut Db, msg: String| {
            Err(db.error(
                Diagnostic::error(diag::GENERIC_CLASS, msg)
                    .with_primary(span, "actual outside the class of the generic type")
                    .with_note(
                        "LRM 6.5.3: the actual of an interface type must belong to its class",
                    ),
            ))
        };
        let scalar: fn(&GenericClass) -> bool = |c| {
            matches!(
                c,
                GenericClass::Scalar
                    | GenericClass::Discrete
                    | GenericClass::Integer
                    | GenericClass::Floating
                    | GenericClass::Physical
            )
        };
        let discrete: fn(&GenericClass) -> bool =
            |c| matches!(c, GenericClass::Discrete | GenericClass::Integer);
        match class {
            GenericClass::Private => Ok(()),
            GenericClass::Scalar if !is(TypeKind::is_scalar, &[scalar]) => {
                fail(self, format!("`{an}` is not a scalar type"))
            }
            GenericClass::Discrete if !is(TypeKind::is_discrete, &[discrete]) => {
                fail(self, format!("`{an}` is not a discrete type"))
            }
            GenericClass::Integer
                if !is(
                    |k| matches!(k, TypeKind::Integer | TypeKind::UniversalInteger),
                    &[|c| matches!(c, GenericClass::Integer)],
                ) =>
            {
                fail(self, format!("`{an}` is not an integer type"))
            }
            GenericClass::Floating
                if !is(
                    |k| matches!(k, TypeKind::Floating | TypeKind::UniversalReal),
                    &[|c| matches!(c, GenericClass::Floating)],
                ) =>
            {
                fail(self, format!("`{an}` is not a floating-point type"))
            }
            GenericClass::Physical
                if !is(
                    |k| matches!(k, TypeKind::Physical { .. }),
                    &[|c| matches!(c, GenericClass::Physical)],
                ) =>
            {
                fail(self, format!("`{an}` is not a physical type"))
            }
            GenericClass::Array { indexes, elem } => {
                let (aidx, aelem) = match (kind, &gen_class) {
                    (TypeKind::Array { indexes, elem }, _)
                    | (_, Some(GenericClass::Array { indexes, elem })) => (indexes.clone(), *elem),
                    _ => return fail(self, format!("`{an}` is not an array type")),
                };
                if aidx.len() != indexes.len() {
                    return fail(
                        self,
                        format!(
                            "`{an}` has {} dimensions, the generic array type has {}",
                            aidx.len(),
                            indexes.len()
                        ),
                    );
                }
                for (&fi, &ai) in indexes.iter().zip(aidx.iter()) {
                    let want = self.r_subtype(r, fi);
                    let (w, g) = (self.base(want), self.base(ai));
                    if w != g && !self.is_error_type(w) {
                        let (wn, gn) = (self.type_name(w), self.type_name(g));
                        return fail(
                            self,
                            format!("index type of `{an}` is `{gn}`, expected `{wn}`"),
                        );
                    }
                }
                self.same_elem(r, *elem, aelem, &an, "element", span)
            }
            GenericClass::Access { designated } => {
                let ad = match (kind, &gen_class) {
                    (TypeKind::Access { designated }, _)
                    | (_, Some(GenericClass::Access { designated })) => *designated,
                    _ => return fail(self, format!("`{an}` is not an access type")),
                };
                self.same_elem(r, *designated, ad, &an, "designated", span)
            }
            GenericClass::File { elem } => {
                let ae = match (kind, &gen_class) {
                    (TypeKind::File { elem }, _) | (_, Some(GenericClass::File { elem })) => *elem,
                    _ => return fail(self, format!("`{an}` is not a file type")),
                };
                self.same_elem(r, *elem, ae, &an, "element", span)
            }
            _ => Ok(()),
        }
    }

    fn same_elem(
        &mut self,
        r: &mut Remap,
        formal: SubtypeId,
        actual: SubtypeId,
        an: &str,
        what: &str,
        span: Span,
    ) -> Result<(), Reported> {
        let want = self.r_subtype(r, formal);
        let (w, g) = (self.base(want), self.base(actual));
        if w == g || self.is_error_type(w) || self.is_error_type(g) {
            return Ok(());
        }
        let (wn, gn) = (self.type_name(w), self.type_name(g));
        Err(self.error(
            Diagnostic::error(
                diag::GENERIC_CLASS,
                format!("{what} type of `{an}` is `{gn}`, expected `{wn}`"),
            )
            .with_primary(span, "actual outside the class of the generic type")
            .with_note("LRM 6.5.3: the actual of an interface type must belong to its class"),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn bind_subprogram(
        &mut self,
        scope: ScopeId,
        site: Span,
        f: DeclId,
        a: Option<&ast::AssocElem>,
        type_spans: &HashMap<TypeId, Span>,
        r: &mut Remap,
        map: &mut GenericMap,
    ) -> Result<(), Reported> {
        use crate::expr::names::NameRes;
        let s = self.decls[f].kind.subprogram().expect("formal subprogram");
        let (fparams, fret, fkind) = (
            s.params.iter().map(|p| p.subtype).collect::<Vec<_>>(),
            s.ret,
            s.kind,
        );
        let want = crate::scope::Profile {
            params: fparams
                .iter()
                .map(|&p| {
                    let st = self.r_subtype(r, p);
                    self.base(st)
                })
                .collect(),
            ret: fret.map(|t| {
                let st = self.r_subtype(r, t);
                self.base(st)
            }),
        };
        // A failed generic type actual (already reported, or a cascade) leaves the profile
        // unknowable: no second error for the subprogram (8.2).
        if want.params.iter().chain(want.ret.iter()).any(|&t| {
            t == self.poison.ty || matches!(self.types[t].kind, crate::types::TypeKind::Error)
        }) {
            return Err(Reported);
        }
        let fits = |db: &Db, d: DeclId| {
            let e = db.entity(d);
            let kind_ok = match db.decls[e].kind.subprogram() {
                Some(s) => {
                    s.generics.is_none()
                        && matches!(s.kind, SubprogramKind::Function { .. })
                            == matches!(fkind, SubprogramKind::Function { .. })
                }
                None => matches!(db.decls[e].kind, DeclKind::EnumLit { .. }) && fret.is_some(),
            };
            kind_ok && db.profile(e).as_ref() == Some(&want)
        };
        let fname = self.decls[f].name;
        let actual = match a.map(|a| &a.actual) {
            Some(ast::Actual::Expr(e)) => {
                // `Match => "="`: an operator symbol parses as a string literal (6.5.7.2).
                let op_name;
                let n = match &e.kind {
                    ast::ExprKind::Name(n) => Some(n),
                    ast::ExprKind::Literal(ast::Literal {
                        kind: ast::LiteralKind::String(sym),
                        ..
                    }) if crate::decl::is_operator_symbol(sym.as_str()) => {
                        op_name = ast::Name {
                            kind: ast::NameKind::Operator(*sym),
                            span: e.span,
                        };
                        Some(&op_name)
                    }
                    _ => None,
                };
                let Some(n) = n else {
                    return Err(self.error(
                        Diagnostic::error(
                            diag::TYPE_MISMATCH,
                            "the actual of a generic subprogram must be a subprogram name",
                        )
                        .with_primary(e.span, "not a subprogram name"),
                    ));
                };
                let (n, sig) = match &n.kind {
                    ast::NameKind::Signature { prefix, signature } => {
                        (prefix.as_ref(), Some(self.signature_of(scope, signature)?))
                    }
                    _ => (n, None),
                };
                let cands = match self.classify(scope, n) {
                    NameRes::One(d) => vec![d],
                    NameRes::Overloads(ds) => ds,
                    other => {
                        return Err(self.report_name(scope, other, n.span, "a subprogram name"));
                    }
                };
                let ok: Vec<DeclId> = cands
                    .iter()
                    .copied()
                    .filter(|&d| fits(self, d))
                    .filter(|&d| sig.as_ref().is_none_or(|s| self.signature_matches(d, s)))
                    .collect();
                match ok.as_slice() {
                    [d] => self.entity(*d),
                    // A missing unit made a type actual fail silently: a cascade (8.2).
                    [] if self.cur().is_some_and(|c| self.tainted.contains(&c.unit)) => {
                        return Err(Reported);
                    }
                    [] => {
                        let mut d = Diagnostic::error(
                            diag::TYPE_MISMATCH,
                            format!(
                                "profile of `{}` does not match generic subprogram `{fname}`",
                                last_name(n)
                            ),
                        )
                        .with_primary(e.span, "profile does not conform")
                        .with_secondary(self.decls[f].span, "generic subprogram declared here");
                        if let Some(&c) = cands.first()
                            && self.profile(c).is_some()
                        {
                            d = d.with_note(format!("found {}", self.signature(c)));
                        }
                        return Err(self.error(d));
                    }
                    many => {
                        let mut d = Diagnostic::error(
                            diag::AMBIGUOUS,
                            format!("several subprograms match generic subprogram `{fname}`"),
                        )
                        .with_primary(e.span, "ambiguous")
                        .with_help("add a signature to the actual");
                        for &c in many {
                            d = d.with_secondary(self.decls[c].span, "candidate");
                        }
                        return Err(self.error(d));
                    }
                }
            }
            Some(ast::Actual::Open(_)) | None => {
                let (cands, from_box) = match self.generic.defaults.get(&f).cloned() {
                    Some(SubpDefault::Box) => {
                        let key = fname.key();
                        let cands = match self.lookup(scope, key) {
                            Visible::One(d) => vec![d],
                            Visible::Overloads(ds) => ds,
                            _ => Vec::new(),
                        };
                        (cands, true)
                    }
                    Some(SubpDefault::Name { cands, .. }) => (cands, false),
                    None => return Err(self.missing_generic(f, site, "")),
                };
                let mut ok: Vec<DeclId> = Vec::new();
                for d in cands.into_iter().filter(|&d| fits(self, d)) {
                    let e = self.entity(d);
                    if !ok.contains(&e) {
                        ok.push(e);
                    }
                }
                match ok.as_slice() {
                    [d] => *d,
                    found => {
                        // Blame the actual of the type the operation is missing for.
                        let span = want
                            .params
                            .iter()
                            .chain(want.ret.iter())
                            .find_map(|t| {
                                map.types
                                    .iter()
                                    .find(|(_, st)| self.base(**st) == *t)
                                    .and_then(|(ft, _)| type_spans.get(ft).copied())
                            })
                            .unwrap_or(site);
                        let msg = if found.is_empty() {
                            format!(
                                "no visible `{fname}` for the `is <>` default of `{}`",
                                fname_plain(&fname)
                            )
                        } else {
                            format!(
                                "several visible `{fname}` fit the default of generic subprogram `{}`",
                                fname_plain(&fname)
                            )
                        };
                        let what = if from_box { "`is <>`" } else { "`is name`" };
                        let mut d = Diagnostic::error(diag::GENERIC_DEFAULT, msg)
                            .with_primary(
                                span,
                                format!("the {what} default needs a matching subprogram"),
                            )
                            .with_secondary(self.decls[f].span, "generic subprogram declared here")
                            .with_help(format!("associate it explicitly: `{fname} => ...`"));
                        for &c in found {
                            d = d.with_secondary(self.decls[c].span, "candidate");
                        }
                        return Err(self.error(d));
                    }
                }
            }
            Some(other) => {
                return Err(self.error(
                    Diagnostic::error(
                        diag::TYPE_MISMATCH,
                        "the actual of a generic subprogram must be a subprogram name",
                    )
                    .with_primary(actual_span(other), "not a subprogram name"),
                ));
            }
        };
        r.decls.insert(f, actual);
        map.subprograms.insert(f, actual);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn bind_package(
        &mut self,
        scope: ScopeId,
        inst_scope: ScopeId,
        site: Span,
        f: DeclId,
        a: Option<&ast::AssocElem>,
        r: &mut Remap,
        map: &mut GenericMap,
    ) -> Result<(), Reported> {
        let DeclKind::GenericPackage(fi) = self.decls[f].kind else {
            return Err(Reported);
        };
        let i = self.package_actual(scope, f, a.map(|a| &a.actual), site, r)?;
        if let Some(m) = self.generic.maps.get(&i) {
            r.merge(&m.clone());
        }
        // The formal's region (the generic package itself) denotes the actual instance.
        r.scopes.insert(self.insts[fi].scope, self.insts[i].scope);
        let name = self.decls[f].name;
        let new = self.new_decl(
            inst_scope,
            name,
            DeclKind::PackageInst(i),
            self.decls[f].span,
        );
        r.decls.insert(f, new);
        map.packages.insert(f, i);
        Ok(())
    }

    /// The package instance an actual of formal package `f` names, checked against the
    /// formal's generic map (6.5.7.2). `r` maps the formal's wanted actuals.
    pub(crate) fn package_actual(
        &mut self,
        scope: ScopeId,
        f: DeclId,
        a: Option<&ast::Actual>,
        site: Span,
        r: &mut Remap,
    ) -> Result<InstId, Reported> {
        let DeclKind::GenericPackage(fi) = self.decls[f].kind else {
            return Err(self.ice(site, "formal package without an instance"));
        };
        let n = match a {
            Some(ast::Actual::Expr(ast::Expr {
                kind: ast::ExprKind::Name(n),
                ..
            })) => n,
            Some(other) => {
                return Err(self.error(
                    Diagnostic::error(
                        diag::GENERIC_CLASS,
                        "the actual of a generic package must name a package instance",
                    )
                    .with_primary(actual_span(other), "not a package instance"),
                ));
            }
            None => return Err(self.missing_generic(f, site, "package ")),
        };
        let d = self.expanded_name_decl(scope, n)?;
        let d = self.entity(d);
        let inst = match self.decls[d].kind {
            DeclKind::PackageInst(i) | DeclKind::GenericPackage(i) => Some(i),
            DeclKind::Unit(u) => match self.units[u].hir {
                UnitHir::PackageInst(i) => Some(i),
                _ => None,
            },
            DeclKind::Error => return Err(Reported),
            _ => None,
        };
        let want = self.insts[fi].generic;
        let Some(i) = inst.filter(|&i| self.insts[i].generic == want) else {
            let (an, gn) = (self.decls[d].name, self.decls[want].name);
            return Err(self.error(
                Diagnostic::error(
                    diag::GENERIC_CLASS,
                    format!("`{an}` is not an instance of `{gn}`"),
                )
                .with_primary(n.span, format!("expected an instance of `{gn}`"))
                .with_secondary(self.decls[f].span, "generic package declared here"),
            ));
        };
        self.check_wants(r, fi, i, n.span)?;
        Ok(i)
    }

    /// The actual of formal package `f` in a design entity's generic map: its
    /// instance, with the generic package's types mapped to the instance's in
    /// `inst_types` for the ports (6.5.6.3).
    pub(crate) fn entity_package_actual(
        &mut self,
        scope: ScopeId,
        f: DeclId,
        a: &ast::Actual,
        site: Span,
    ) -> Result<InstId, Reported> {
        let i = self.package_actual(scope, f, Some(a), site, &mut Remap::default())?;
        if let Some(m) = self.generic.maps.get(&i) {
            let pairs: Vec<(TypeId, TypeId)> = m.types.iter().map(|(&o, &c)| (o, c)).collect();
            for (o, c) in pairs {
                let st = self.types[c].first_subtype;
                self.inst_types.push((o, st));
            }
        }
        Ok(i)
    }

    // -- bodies ------------------------------------------------------------------------

    /// The cloned body of an instance (lazy, 5.1): the package body region of a
    /// package instance (`None` when the generic package has no body), or the
    /// body region of an instantiated subprogram. Subprograms of the instance get
    /// their cloned `Body::Hir`.
    pub fn instance_body(&mut self, inst: InstId) -> Result<Option<ScopeId>, Reported> {
        if let Some(&s) = self.insts[inst].body.get() {
            return Ok(Some(s));
        }
        let Some(mut r) = self.generic.maps.remove(&inst) else {
            return Ok(None);
        };
        let res = self.instance_body_with(inst, &mut r);
        self.generic.maps.insert(inst, r);
        let s = res?;
        if let Some(s) = s {
            let _ = self.insts[inst].body.set(s);
        }
        Ok(s)
    }

    fn instance_body_with(
        &mut self,
        inst: InstId,
        r: &mut Remap,
    ) -> Result<Option<ScopeId>, Reported> {
        r.bodies = true;
        let g = self.insts[inst].generic;
        let site = self.insts[inst].site;
        let iscope = self.insts[inst].scope;
        let span = self.decls[site].span;
        let overflow = self.overflow.len();
        let out = match &self.decls[g].kind {
            DeclKind::Unit(u) => {
                let u = *u;
                let lib = self.units[u].lib;
                let key = UnitKey::body(self.units[u].name.key);
                if self.locate(lib, key).is_none() && !self.libs[lib].units.contains_key(&key) {
                    return Ok(None);
                }
                let b = self.load_unit(lib, key, Some(span))?;
                let bscope = self.units[b].scope;
                let bdecl = self.units[b].decl;
                if let Some(ctx) = self.scopes[bscope].parent {
                    r.scopes.insert(ctx, iscope);
                }
                r.decls.insert(bdecl, site);
                let s = self.r_scope(r, bscope);
                self.fix_bodies(r);
                Some(s)
            }
            DeclKind::Subprogram(sp) => {
                let Body::Hir { scope, stmts } = &sp.body else {
                    return Err(self.error(
                        Diagnostic::error(
                            diag::MISSING_BODY,
                            format!("generic subprogram `{}` has no body", self.decls[g].name),
                        )
                        .with_primary(span, "instantiated here")
                        .with_secondary(self.decls[g].span, "declared here"),
                    ));
                };
                let (bscope, stmts) = (*scope, stmts.clone());
                // The body shares the spec's generics (`prepare_generic_body`).
                r.roots.push(bscope);
                let s = self.r_scope(r, bscope);
                let stmts: Box<[StmtId]> = stmts.iter().map(|&x| self.r_stmt(r, x)).collect();
                if let DeclKind::Subprogram(x) = &mut self.decls[site].kind {
                    x.body = Body::Hir { scope: s, stmts };
                }
                Some(s)
            }
            _ => None,
        };
        self.overflow.truncate(overflow);
        if r.failed { Err(Reported) } else { Ok(out) }
    }

    /// After cloning a package body: give cloned specs their cloned bodies and
    /// protected types their cloned bodies.
    fn fix_bodies(&mut self, r: &mut Remap) {
        let pairs: Vec<(DeclId, DeclId)> = r.decls.iter().map(|(a, b)| (*a, *b)).collect();
        for (orig, new) in pairs {
            if orig == new {
                continue;
            }
            let Some(Body::Hir { scope, stmts }) =
                self.decls[orig].kind.subprogram().map(|s| &s.body)
            else {
                continue;
            };
            let pending = matches!(
                self.decls[new].kind.subprogram().map(|s| &s.body),
                Some(Body::Pending)
            );
            if !pending {
                continue;
            }
            let (scope, stmts) = (*scope, stmts.clone());
            let s = self.r_scope(r, scope);
            let stmts: Box<[StmtId]> = stmts.iter().map(|&x| self.r_stmt(r, x)).collect();
            if let DeclKind::Subprogram(x) = &mut self.decls[new].kind {
                x.body = Body::Hir { scope: s, stmts };
            }
        }
        let tpairs: Vec<(TypeId, TypeId)> = r.types.iter().map(|(a, b)| (*a, *b)).collect();
        for (orig, new) in tpairs {
            if orig == new {
                continue;
            }
            if let TypeKind::Protected { body: Some(b), .. } = self.types[orig].kind {
                let nb = self.r_scope(r, b);
                if let TypeKind::Protected { body, .. } = &mut self.types[new].kind
                    && body.is_none()
                {
                    *body = Some(nb);
                }
            }
        }
    }

    // -- cloning (5.1) -------------------------------------------------------------------

    fn r_inside(&self, r: &Remap, mut s: ScopeId) -> bool {
        loop {
            if r.roots.contains(&s) {
                return true;
            }
            match self.scopes[s].parent {
                Some(p) => s = p,
                None => return false,
            }
        }
    }

    /// Copy the names, order and use clauses of `from` into the existing region `to`.
    fn fill_scope(&mut self, r: &mut Remap, from: ScopeId, to: ScopeId) {
        let names: Vec<(Symbol, Vec<DeclId>)> = self.scopes[from]
            .names
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        let order = self.scopes[from].order.clone();
        let uses = self.scopes[from].uses.clone();
        let mut new_order = Vec::with_capacity(order.len());
        for d in order {
            let n = self.r_decl(r, d);
            if !new_order.contains(&n) {
                new_order.push(n);
            }
        }
        let mut new_names: HashMap<Symbol, Vec<DeclId>> = HashMap::new();
        for (k, ds) in names {
            let v = new_names.entry(k).or_default();
            for d in ds {
                let n = self.r_decl(r, d);
                if !v.contains(&n) {
                    v.push(n);
                }
            }
        }
        let uses: Vec<UseItem> = uses.into_iter().map(|u| self.r_use(r, u)).collect();
        let private_from = self.scopes[from].private_from;
        let s = &mut self.scopes[to];
        s.order.extend(new_order);
        for (k, v) in new_names {
            s.names.entry(k).or_default().extend(v);
        }
        s.uses.extend(uses);
        s.private_from = private_from;
    }

    fn r_use(&mut self, r: &mut Remap, u: UseItem) -> UseItem {
        match u {
            UseItem::All(s) => UseItem::All(self.r_scope(r, s)),
            UseItem::One(ds) => UseItem::One(ds.into_iter().map(|d| self.r_decl(r, d)).collect()),
            UseItem::Library(l) => UseItem::Library(l),
        }
    }

    fn r_scope(&mut self, r: &mut Remap, s: ScopeId) -> ScopeId {
        if let Some(&n) = r.scopes.get(&s) {
            if n != s {
                r.changed = true;
            }
            return n;
        }
        if !self.r_inside(r, s) {
            return s;
        }
        r.changed = true;
        let kind = self.scopes[s].kind;
        let n = self.scopes.push(Scope::new(kind, None, None));
        r.scopes.insert(s, n);
        let parent = self.scopes[s].parent.map(|p| self.r_scope(r, p));
        let owner = self.scopes[s].owner.map(|o| self.r_decl(r, o));
        self.scopes[n].parent = parent;
        self.scopes[n].owner = owner;
        self.fill_scope(r, s, n);
        n
    }

    pub(crate) fn r_decl(&mut self, r: &mut Remap, d: DeclId) -> DeclId {
        if let Some(&n) = r.decls.get(&d) {
            if n != d {
                r.changed = true;
            }
            return n;
        }
        let old_scope = self.decls[d].scope;
        if !self.r_inside(r, old_scope) {
            return d;
        }
        r.changed = true;
        let old = &self.decls[d];
        let n = self.decls.push(Decl {
            name: old.name,
            kind: DeclKind::Error,
            scope: old_scope,
            span: old.span,
            implicit: old.implicit,
            since: old.since,
        });
        r.decls.insert(d, n);
        let scope = self.r_scope(r, old_scope);
        self.decls[n].scope = scope;
        let kind = std::mem::replace(&mut self.decls[d].kind, DeclKind::Error);
        let new_kind = self.r_decl_kind(r, n, &kind);
        self.decls[d].kind = kind;
        self.decls[n].kind = new_kind;
        if let Some(&pv) = self.forwarding.get(&d) {
            let pv = self.r_decl(r, pv);
            self.forwarding.insert(n, pv);
        }
        n
    }

    fn r_decl_kind(&mut self, r: &mut Remap, new: DeclId, k: &DeclKind) -> DeclKind {
        match k {
            DeclKind::Library(l) => DeclKind::Library(*l),
            DeclKind::Unit(u) => DeclKind::Unit(*u),
            DeclKind::Type(t) => DeclKind::Type(self.r_ty(r, *t)),
            DeclKind::Subtype(s) => DeclKind::Subtype(self.r_subtype(r, *s)),
            DeclKind::Object(o) => DeclKind::Object(self.r_object(r, new, o)),
            DeclKind::Subprogram(s) => DeclKind::Subprogram(self.r_subprogram(r, new, s)),
            DeclKind::GenericSubprogram(s) => {
                DeclKind::GenericSubprogram(self.r_subprogram(r, new, s))
            }
            DeclKind::EnumLit { ty, pos } => DeclKind::EnumLit {
                ty: self.r_ty(r, *ty),
                pos: *pos,
            },
            DeclKind::PhysUnit { ty, index } => DeclKind::PhysUnit {
                ty: self.r_ty(r, *ty),
                index: *index,
            },
            DeclKind::Alias(Alias::Object { target, subtype }) => DeclKind::Alias(Alias::Object {
                target: self.r_expr(r, *target),
                subtype: subtype.map(|s| self.r_subtype(r, s)),
            }),
            DeclKind::Alias(Alias::NonObject { target }) => DeclKind::Alias(Alias::NonObject {
                target: self.r_decl(r, *target),
            }),
            DeclKind::Attribute { ty } => DeclKind::Attribute {
                ty: self.r_subtype(r, *ty),
            },
            DeclKind::Component(c) => DeclKind::Component(Component {
                scope: self.r_scope(r, c.scope),
                generics: c.generics.iter().map(|&d| self.r_decl(r, d)).collect(),
                ports: c.ports.iter().map(|&d| self.r_decl(r, d)).collect(),
            }),
            DeclKind::GenericType(t) => DeclKind::GenericType(self.r_ty(r, *t)),
            DeclKind::GenericPackage(i) => DeclKind::GenericPackage(*i),
            DeclKind::Group { template } => DeclKind::Group {
                template: self.r_decl(r, *template),
            },
            DeclKind::GroupTemplate => DeclKind::GroupTemplate,
            DeclKind::Label(s) => DeclKind::Label(self.r_stmt(r, *s)),
            DeclKind::Element { record, index } => DeclKind::Element {
                record: self.r_ty(r, *record),
                index: *index,
            },
            DeclKind::PackageInst(i) => DeclKind::PackageInst(self.r_instance(r, new, *i)),
            DeclKind::View(v) => {
                let record = self.r_subtype(r, self.views[*v].record);
                let elems = self.views[*v].elems.clone();
                let elems = elems.iter().map(|&e| self.r_elem_mode(r, e)).collect();
                DeclKind::View(self.views.push(crate::view::ViewDef {
                    decl: new,
                    record,
                    elems,
                    converse: None,
                }))
            }
            DeclKind::Error => DeclKind::Error,
        }
    }

    /// A mode view of the generic unit is cloned with its declaration (2019 6.5.2); the
    /// converse of a clone is materialized again on use.
    fn r_view(&mut self, r: &mut Remap, v: ViewId) -> ViewId {
        let d = self.views[v].decl;
        let n = self.r_decl(r, d);
        match self.decls[n].kind {
            DeclKind::View(n) => n,
            _ => v,
        }
    }

    fn r_elem_mode(&mut self, r: &mut Remap, e: crate::view::ElemMode) -> crate::view::ElemMode {
        match e {
            crate::view::ElemMode::View {
                view,
                converse,
                array,
            } => crate::view::ElemMode::View {
                view: self.r_view(r, view),
                converse,
                array,
            },
            simple => simple,
        }
    }

    fn r_mode(&mut self, r: &mut Remap, m: Mode) -> Mode {
        match m {
            Mode::View { view, array } => Mode::View {
                view: self.r_view(r, view),
                array,
            },
            other => other,
        }
    }

    /// A package instance declared inside the generic unit: its region is cloned
    /// with the rest, and its body remap is the inner remap followed by `r`.
    fn r_instance(&mut self, r: &mut Remap, site: DeclId, i: InstId) -> InstId {
        let (generic, scope) = (self.insts[i].generic, self.insts[i].scope);
        if !self.r_inside(r, scope) {
            return i;
        }
        let scope = self.r_scope(r, scope);
        let old = &self.insts[i].map;
        let (types, consts, subprograms, packages) = (
            old.types.clone(),
            old.consts.clone(),
            old.subprograms.clone(),
            old.packages.clone(),
        );
        let map = GenericMap {
            types: types
                .into_iter()
                .map(|(t, st)| (t, self.r_subtype(r, st)))
                .collect(),
            consts: consts
                .into_iter()
                .map(|(d, e)| (d, self.r_expr(r, e)))
                .collect(),
            subprograms: subprograms
                .into_iter()
                .map(|(f, a)| (f, self.r_decl(r, a)))
                .collect(),
            packages,
        };
        let inner = self.generic.maps.get(&i).cloned().unwrap_or_default();
        let composed = self.r_compose(r, &inner);
        let mangled = Symbol::intern(&crate::mangle::region(self, scope));
        let n = self.insts.push(Instance {
            generic,
            site,
            map,
            scope,
            body: OnceCell::new(),
            mangled,
        });
        self.generic.maps.insert(n, composed);
        n
    }

    /// `inner` followed by `r`: what `inner` maps to, as `r` clones it.
    fn r_compose(&mut self, r: &mut Remap, inner: &Remap) -> Remap {
        let mut c = Remap {
            roots: inner.roots.clone(),
            formal_firsts: inner.formal_firsts.clone(),
            ..Default::default()
        };
        for (&a, &b) in &inner.decls {
            c.decls.insert(a, self.r_decl(r, b));
        }
        for (&a, &b) in &inner.types {
            c.types.insert(a, self.r_ty(r, b));
        }
        for (&a, &b) in &inner.subtypes {
            c.subtypes.insert(a, self.r_subtype(r, b));
        }
        for (&a, &b) in &inner.scopes {
            c.scopes.insert(a, self.r_scope(r, b));
        }
        for (&a, &b) in &inner.exprs {
            c.exprs.insert(a, self.r_expr(r, b));
        }
        for (&a, &b) in &inner.consts {
            c.consts.insert(a, self.r_expr(r, b));
        }
        // Statements of an inner body already cloned are cloned again with that body.
        c
    }

    fn r_object(&mut self, r: &mut Remap, new: DeclId, o: &Object) -> Object {
        let subtype = self.r_subtype(r, o.subtype);
        Object {
            class: o.class,
            role: o.role.clone(),
            mode: o.mode.map(|m| self.r_mode(r, m)),
            subtype,
            default: o.default.map(|e| self.r_expr(r, e)),
            bounds: self.r_bounds(r, new, o, subtype),
            signal_kind: o.signal_kind,
            file_open: o
                .file_open
                .map(|(k, n)| (k.map(|k| self.r_expr(r, k)), self.r_expr(r, n))),
        }
    }

    /// Where a cloned object's bounds come from, from its substituted subtype: a
    /// generic type counts as fully constrained in the generic unit, the actual may
    /// not be (rule 1: decided again here, never inherited). A signal or variable
    /// whose subtype is no longer fully constrained is `E1212`, as in NVC and Questa.
    fn r_bounds(&mut self, r: &mut Remap, new: DeclId, o: &Object, st: SubtypeId) -> BoundsFrom {
        if o.bounds != BoundsFrom::Subtype || self.constrainedness(st) == Constrainedness::Full {
            return o.bounds;
        }
        match (&o.role, o.class) {
            (ObjRole::Param | ObjRole::Port | ObjRole::Generic, _) => BoundsFrom::Actual,
            (ObjRole::Deferred, _) => BoundsFrom::Initializer,
            (_, ObjClass::Constant) if o.default.is_some() => BoundsFrom::Initializer,
            _ => {
                let name = self.decls[new].name;
                let what = match o.class {
                    ObjClass::Signal => "signal",
                    ObjClass::Constant => "constant",
                    _ => "variable",
                };
                self.report(
                    Diagnostic::error(
                        diag::NOT_FULLY_CONSTRAINED,
                        format!("{what} `{name}` needs a fully constrained subtype"),
                    )
                    .with_primary(self.decls[new].name_span(), "declared here")
                    .with_note(format!(
                        "in this instance its subtype is `{}`, which is not fully constrained",
                        self.subtype_name(st)
                    )),
                );
                r.failed = true;
                o.bounds
            }
        }
    }

    fn r_subprogram(&mut self, r: &mut Remap, new: DeclId, s: &Subprogram) -> Subprogram {
        let params: Box<[Param]> = s
            .params
            .iter()
            .map(|p| Param {
                name: p.name,
                class: p.class,
                mode: self.r_mode(r, p.mode),
                subtype: self.r_subtype(r, p.subtype),
                default: p.default.map(|e| self.r_expr(r, e)),
                decl: p.decl.map(|d| self.r_decl(r, d)),
            })
            .collect();
        let ret = s.ret.map(|t| self.r_subtype(r, t));
        let generics = s
            .generics
            .as_ref()
            .map(|g| g.iter().map(|&d| self.r_decl(r, d)).collect());
        let scope = s.scope.map(|x| self.r_scope(r, x));
        let body = match &s.body {
            Body::Pending => Body::Pending,
            Body::Hir { scope, stmts } if r.bodies => Body::Hir {
                scope: self.r_scope(r, *scope),
                stmts: stmts.iter().map(|&x| self.r_stmt(r, x)).collect(),
            },
            Body::Hir { .. } => Body::Pending,
            Body::Implicit(op) => Body::Implicit(ImplicitOp {
                ty: self.r_ty(r, op.ty),
                ..*op
            }),
            Body::Native(n) => Body::Native(*n),
            Body::Foreign(e) => Body::Foreign(self.r_expr(r, *e)),
            Body::Instance(i) => Body::Instance(self.r_instance(r, new, *i)),
            Body::Interface => Body::Interface,
        };
        let types: Vec<SubtypeId> = params.iter().map(|p| p.subtype).collect();
        let (dscope, name) = (self.decls[new].scope, self.decls[new].name);
        let mangled = Symbol::intern(&crate::mangle::subprogram(self, dscope, &name, &types, ret));
        Subprogram {
            kind: s.kind,
            params,
            ret,
            generics,
            scope,
            body,
            mangled,
        }
    }

    fn r_ty(&mut self, r: &mut Remap, t: TypeId) -> TypeId {
        if let Some(&n) = r.types.get(&t) {
            if n != t {
                r.changed = true;
            }
            return n;
        }
        let decl = self.types[t].decl;
        if !self.r_inside(r, self.decls[decl].scope) {
            return t;
        }
        r.changed = true;
        let old = &self.types[t];
        let n = self.types.push(TypeDef {
            decl,
            name: old.name,
            anonymous: old.anonymous,
            kind: TypeKind::Error,
            first_subtype: old.first_subtype,
            base_subtype: old.base_subtype,
        });
        r.types.insert(t, n);
        let ndecl = self.r_decl(r, decl);
        let kind = std::mem::replace(&mut self.types[t].kind, TypeKind::Error);
        let new_kind = self.r_type_kind(r, &kind);
        self.types[t].kind = kind;
        let (first, base) = (self.types[t].first_subtype, self.types[t].base_subtype);
        let first = self.r_subtype(r, first);
        let base = self.r_subtype(r, base);
        let td = &mut self.types[n];
        td.decl = ndecl;
        td.kind = new_kind;
        td.first_subtype = first;
        td.base_subtype = base;
        n
    }

    fn r_type_kind(&mut self, r: &mut Remap, k: &TypeKind) -> TypeKind {
        match k {
            TypeKind::UniversalInteger => TypeKind::UniversalInteger,
            TypeKind::UniversalReal => TypeKind::UniversalReal,
            TypeKind::Enum { lits } => TypeKind::Enum {
                lits: lits.iter().map(|&d| self.r_decl(r, d)).collect(),
            },
            TypeKind::Integer => TypeKind::Integer,
            TypeKind::Floating => TypeKind::Floating,
            TypeKind::Physical { units } => TypeKind::Physical {
                units: units
                    .iter()
                    .map(|u| crate::types::PhysUnit {
                        decl: self.r_decl(r, u.decl),
                        scale: u.scale,
                    })
                    .collect(),
            },
            TypeKind::Array { indexes, elem } => TypeKind::Array {
                indexes: indexes.iter().map(|&s| self.r_subtype(r, s)).collect(),
                elem: self.r_subtype(r, *elem),
            },
            TypeKind::Record { elems } => TypeKind::Record {
                elems: elems
                    .iter()
                    .map(|e| RecordElem {
                        name: e.name,
                        subtype: self.r_subtype(r, e.subtype),
                    })
                    .collect(),
            },
            TypeKind::Access { designated } => TypeKind::Access {
                designated: self.r_subtype(r, *designated),
            },
            TypeKind::File { elem } => TypeKind::File {
                elem: self.r_subtype(r, *elem),
            },
            TypeKind::Protected {
                decl_scope,
                body,
                generics,
            } => TypeKind::Protected {
                decl_scope: self.r_scope(r, *decl_scope),
                // Bodies are cloned with the package body (`fix_bodies`).
                body: body.filter(|_| r.bodies).map(|b| self.r_scope(r, b)),
                generics: generics
                    .as_ref()
                    .map(|g| g.iter().map(|&d| self.r_decl(r, d)).collect()),
            },
            TypeKind::Incomplete { completion } => TypeKind::Incomplete {
                completion: completion.map(|c| self.r_ty(r, c)),
            },
            TypeKind::Generic { class } => TypeKind::Generic {
                class: self.r_class(r, class),
            },
            TypeKind::Error => TypeKind::Error,
        }
    }

    fn r_class(&mut self, r: &mut Remap, c: &GenericClass) -> GenericClass {
        match c {
            GenericClass::Array { indexes, elem } => GenericClass::Array {
                indexes: indexes.iter().map(|&s| self.r_subtype(r, s)).collect(),
                elem: self.r_subtype(r, *elem),
            },
            GenericClass::Access { designated } => GenericClass::Access {
                designated: self.r_subtype(r, *designated),
            },
            GenericClass::File { elem } => GenericClass::File {
                elem: self.r_subtype(r, *elem),
            },
            other => other.clone(),
        }
    }

    pub(crate) fn r_subtype(&mut self, r: &mut Remap, st: SubtypeId) -> SubtypeId {
        if let Some(&n) = r.subtypes.get(&st) {
            if n != st {
                r.changed = true;
            }
            return n;
        }
        let saved = std::mem::take(&mut r.changed);
        // Provisional: a range bound's subtype is often the subtype itself.
        r.subtypes.insert(st, st);
        let s = &self.subtypes[st];
        let (base, name, parent, constraint, resolution, span) = (
            s.base,
            s.name,
            s.parent,
            s.constraint.clone(),
            s.resolution.clone(),
            s.span,
        );
        let nbase = self.r_ty(r, base);
        let nparent = parent.map(|p| self.r_subtype(r, p));
        let mut nconstraint = self.r_constraint(r, &constraint);
        let mut nresolution = resolution.as_ref().map(|x| self.r_resolution(r, x));
        // `subtype s is T`: the actual's constraint and resolution come with it.
        if let (Some(p), Some(np)) = (parent, nparent)
            && r.formal_firsts.contains(&p)
        {
            if matches!(nconstraint, Constraint::None) {
                nconstraint = self.subtypes[np].constraint.clone();
            }
            if nresolution.is_none() {
                nresolution = self.subtypes[np].resolution.clone();
            }
        }
        let changed = r.changed;
        r.changed = saved || changed;
        match r.subtypes.get(&st) {
            Some(&n) if n != st => return n,
            _ if !changed => return st,
            _ => {}
        }
        let n = self.subtypes.push(SubtypeDef {
            base: nbase,
            name,
            parent: nparent,
            constraint: nconstraint,
            resolution: nresolution,
            span,
        });
        r.subtypes.insert(st, n);
        n
    }

    fn r_constraint(&mut self, r: &mut Remap, c: &Constraint) -> Constraint {
        match c {
            Constraint::None => Constraint::None,
            Constraint::Range(rc) => Constraint::Range(self.r_range(r, rc)),
            Constraint::Array { dims, elem } => Constraint::Array {
                dims: dims
                    .as_ref()
                    .map(|ds| ds.iter().map(|d| self.r_range(r, d)).collect()),
                elem: Box::new(self.r_constraint(r, elem)),
            },
            Constraint::Record { elems } => Constraint::Record {
                elems: elems
                    .iter()
                    .map(|(i, c)| (*i, self.r_constraint(r, c)))
                    .collect(),
            },
            Constraint::FromObject { expr, elem_depth } => Constraint::FromObject {
                expr: self.r_expr(r, *expr),
                elem_depth: *elem_depth,
            },
        }
    }

    fn r_resolution(&mut self, r: &mut Remap, x: &Resolution) -> Resolution {
        match x {
            Resolution::Function(d) => Resolution::Function(self.r_decl(r, *d)),
            Resolution::Element(e) => Resolution::Element(Box::new(self.r_resolution(r, e))),
            Resolution::Record(es) => Resolution::Record(
                es.iter()
                    .map(|(i, e)| (*i, self.r_resolution(r, e)))
                    .collect(),
            ),
        }
    }

    fn r_range(&mut self, r: &mut Remap, rc: &RangeC) -> RangeC {
        match rc {
            RangeC::Explicit { left, dir, right } => RangeC::Explicit {
                left: self.r_expr(r, *left),
                dir: *dir,
                right: self.r_expr(r, *right),
            },
            RangeC::Attr {
                prefix,
                reverse,
                dim,
            } => RangeC::Attr {
                prefix: self.r_attr_prefix(r, *prefix),
                reverse: *reverse,
                dim: *dim,
            },
        }
    }

    fn r_attr_prefix(&mut self, r: &mut Remap, p: AttrPrefix) -> AttrPrefix {
        match p {
            AttrPrefix::Subtype(s) => AttrPrefix::Subtype(self.r_subtype(r, s)),
            AttrPrefix::Expr(e) => AttrPrefix::Expr(self.r_expr(r, e)),
            AttrPrefix::Entity(d) => AttrPrefix::Entity(self.r_decl(r, d)),
        }
    }

    pub(crate) fn r_expr(&mut self, r: &mut Remap, e: ExprId) -> ExprId {
        if let Some(&n) = r.exprs.get(&e) {
            if n != e {
                r.changed = true;
            }
            return n;
        }
        if let ExprKind::Obj(d) = self.exprs[e].kind
            && let Some(&a) = r.consts.get(&d)
        {
            r.changed = true;
            r.exprs.insert(e, a);
            return a;
        }
        let saved = std::mem::take(&mut r.changed);
        r.exprs.insert(e, e);
        let kind = std::mem::replace(&mut self.exprs[e].kind, ExprKind::Error);
        let new_kind = self.r_expr_kind(r, &kind);
        self.exprs[e].kind = kind;
        let st = self.r_subtype(r, self.exprs[e].subtype);
        let changed = r.changed;
        r.changed = saved || changed;
        if !changed {
            return e;
        }
        let old = &self.exprs[e];
        // ponytail: staticness is kept from the generic; an actual that makes an
        // expression locally static only shows in the refolded value.
        let n = self.exprs.push(Expr {
            kind: new_kind,
            subtype: st,
            span: old.span,
            stat: old.stat,
            value: None,
        });
        self.fold_into(n);
        // A constant of the instance whose initial value folded again (`hi.hm`).
        if let ExprKind::Obj(d) = self.exprs[n].kind
            && let DeclKind::Object(o) = &self.decls[d].kind
            && o.class == ObjClass::Constant
            && o.role == ObjRole::Plain
            && let Some(init) = o.default
        {
            self.exprs[n].value = self.exprs[init].value;
        }
        r.exprs.insert(e, n);
        n
    }

    fn r_args(&mut self, r: &mut Remap, args: &[Arg]) -> Box<[Arg]> {
        args.iter()
            .map(|a| Arg {
                formal: a.formal,
                inertial: a.inertial,
                actual: match &a.actual {
                    Actual::Expr(e) => Actual::Expr(self.r_expr(r, *e)),
                    Actual::Default => Actual::Default,
                    Actual::Open => Actual::Open,
                    Actual::Individual(parts) => Actual::Individual(
                        parts
                            .iter()
                            .map(|(path, e, f)| {
                                (self.r_path(r, path), self.r_expr(r, *e), self.r_expr(r, *f))
                            })
                            .collect(),
                    ),
                    Actual::Converted {
                        formal_conv,
                        actual_conv,
                        actual,
                        copy_in,
                        copy_out,
                    } => Actual::Converted {
                        formal_conv: formal_conv.map(|d| self.r_decl(r, d)),
                        actual_conv: actual_conv.map(|d| self.r_decl(r, d)),
                        actual: self.r_expr(r, *actual),
                        copy_in: copy_in.map(|e| self.r_expr(r, e)),
                        copy_out: copy_out.map(|e| self.r_expr(r, e)),
                    },
                    Actual::Subtype(st) => Actual::Subtype(self.r_subtype(r, *st)),
                    Actual::Subprogram(d) => Actual::Subprogram(self.r_decl(r, *d)),
                    // ponytail: the instance is kept; a formal package actual of an instance
                    // inside a generic would need the instance remapped (no Remap of InstIds).
                    Actual::Package(i) => Actual::Package(*i),
                },
            })
            .collect()
    }

    fn r_path(&mut self, r: &mut Remap, path: &[LspStep]) -> Box<[LspStep]> {
        path.iter()
            .map(|s| match s {
                LspStep::Field(i) => LspStep::Field(*i),
                LspStep::Index(es) => {
                    LspStep::Index(es.iter().map(|&e| self.r_expr(r, e)).collect())
                }
                LspStep::Slice(rc) => LspStep::Slice(self.r_range(r, rc)),
                LspStep::Rebase(st) => LspStep::Rebase(self.r_subtype(r, *st)),
            })
            .collect()
    }

    fn r_choice(&mut self, r: &mut Remap, c: &Choice) -> Choice {
        match c {
            Choice::Expr(e) => Choice::Expr(self.r_expr(r, *e)),
            Choice::Range(rc) => Choice::Range(self.r_range(r, rc)),
            Choice::Element(i) => Choice::Element(*i),
            Choice::Others => Choice::Others,
        }
    }

    fn r_expr_kind(&mut self, r: &mut Remap, k: &ExprKind) -> ExprKind {
        match k {
            ExprKind::Const(c) => ExprKind::Const(*c),
            ExprKind::Obj(d) => ExprKind::Obj(self.r_decl(r, *d)),
            ExprKind::External(x) => {
                let old = &self.externals[*x];
                let (class, path_kind, path, st, span) = (
                    old.class,
                    old.path_kind,
                    old.path.iter().map(|(i, e)| (*i, *e)).collect::<Vec<_>>(),
                    old.subtype,
                    old.span,
                );
                let saved = std::mem::take(&mut r.changed);
                let path: Box<[_]> = path
                    .into_iter()
                    .map(|(i, e)| (i, e.map(|e| self.r_expr(r, e))))
                    .collect();
                let subtype = self.r_subtype(r, st);
                let changed = r.changed;
                r.changed = saved || changed;
                if !changed {
                    ExprKind::External(*x)
                } else {
                    ExprKind::External(self.externals.push(External {
                        class,
                        path_kind,
                        path,
                        subtype,
                        span,
                    }))
                }
            }
            ExprKind::Call {
                callee,
                args,
                method_of,
                implicit_cond,
            } => ExprKind::Call {
                callee: self.r_decl(r, *callee),
                args: self.r_args(r, args),
                method_of: method_of.map(|e| self.r_expr(r, e)),
                implicit_cond: *implicit_cond,
            },
            ExprKind::Index { prefix, indices } => ExprKind::Index {
                prefix: self.r_expr(r, *prefix),
                indices: indices.iter().map(|&e| self.r_expr(r, e)).collect(),
            },
            ExprKind::Slice { prefix, range } => ExprKind::Slice {
                prefix: self.r_expr(r, *prefix),
                range: self.r_range(r, range),
            },
            ExprKind::Field { prefix, elem } => ExprKind::Field {
                prefix: self.r_expr(r, *prefix),
                elem: *elem,
            },
            ExprKind::Deref { prefix } => ExprKind::Deref {
                prefix: self.r_expr(r, *prefix),
            },
            ExprKind::PrivateVar { object, var } => ExprKind::PrivateVar {
                object: self.r_expr(r, *object),
                var: self.r_decl(r, *var),
            },
            ExprKind::Attr { attr, prefix, args } => ExprKind::Attr {
                attr: *attr,
                prefix: self.r_attr_prefix(r, *prefix),
                args: args.iter().map(|&e| self.r_expr(r, e)).collect(),
            },
            // ponytail: user attribute values stay shared with the generic; clone the
            // AttrValue when an attribute specification inside a generic needs remapping.
            ExprKind::AttrValue { spec } => ExprKind::AttrValue { spec: *spec },
            ExprKind::Aggregate { assocs, bounds } => ExprKind::Aggregate {
                assocs: assocs
                    .iter()
                    .map(|a| AggAssoc {
                        choices: a.choices.iter().map(|c| self.r_choice(r, c)).collect(),
                        value: self.r_expr(r, a.value),
                        slice_piece: a.slice_piece,
                    })
                    .collect(),
                bounds: match bounds {
                    AggBounds::Context => AggBounds::Context,
                    AggBounds::Positional { count } => AggBounds::Positional {
                        count: self.r_expr(r, *count),
                    },
                    AggBounds::Named { low, high } => AggBounds::Named {
                        low: self.r_expr(r, *low),
                        high: self.r_expr(r, *high),
                    },
                    AggBounds::Concat => AggBounds::Concat,
                    AggBounds::Record => AggBounds::Record,
                },
            },
            ExprKind::Qualified { to, operand } => ExprKind::Qualified {
                to: self.r_subtype(r, *to),
                operand: self.r_expr(r, *operand),
            },
            ExprKind::Conv { to, operand } => ExprKind::Conv {
                to: self.r_subtype(r, *to),
                operand: self.r_expr(r, *operand),
            },
            ExprKind::ImplicitConv { operand } => ExprKind::ImplicitConv {
                operand: self.r_expr(r, *operand),
            },
            ExprKind::New { init } => ExprKind::New {
                init: match init {
                    NewInit::Subtype(s) => NewInit::Subtype(self.r_subtype(r, *s)),
                    NewInit::Value(e) => NewInit::Value(self.r_expr(r, *e)),
                },
            },
            ExprKind::Null => ExprKind::Null,
            ExprKind::Open => ExprKind::Open,
            ExprKind::Conditional { alts } => ExprKind::Conditional {
                alts: alts
                    .iter()
                    .map(|(v, c)| (self.r_expr(r, *v), c.map(|c| self.r_expr(r, c))))
                    .collect(),
            },
            ExprKind::Error => ExprKind::Error,
        }
    }

    // -- statements (bodies) ---------------------------------------------------------------

    fn r_stmt(&mut self, r: &mut Remap, s: StmtId) -> StmtId {
        if let Some(&n) = r.stmts.get(&s) {
            return n;
        }
        let (label, span) = (self.stmts[s].label, self.stmts[s].span);
        let n = self.stmts.next_id();
        self.stmts.push(Stmt {
            id: n,
            label,
            span,
            kind: StmtKind::Null,
        });
        r.stmts.insert(s, n);
        let kind = std::mem::replace(&mut self.stmts[s].kind, StmtKind::Null);
        let new_kind = self.r_stmt_kind(r, &kind);
        self.stmts[s].kind = kind;
        self.stmts[n].kind = new_kind;
        n
    }

    fn r_stmts(&mut self, r: &mut Remap, ss: &[StmtId]) -> Box<[StmtId]> {
        ss.iter().map(|&s| self.r_stmt(r, s)).collect()
    }

    fn r_lsp(&mut self, r: &mut Remap, l: &Lsp) -> Lsp {
        Lsp {
            root: match l.root {
                ObjRoot::Decl(d) => ObjRoot::Decl(self.r_decl(r, d)),
                ObjRoot::External(x) => ObjRoot::External(x),
                ObjRoot::ImplicitSignal(d) => ObjRoot::ImplicitSignal(self.r_decl(r, d)),
            },
            path: self.r_path(r, &l.path),
            stat: l.stat,
            span: l.span,
        }
    }

    fn r_target(&mut self, r: &mut Remap, t: &Target) -> Target {
        Target {
            expr: self.r_expr(r, t.expr),
            lsps: t.lsps.iter().map(|l| self.r_lsp(r, l)).collect(),
        }
    }

    fn r_choices(&mut self, r: &mut Remap, c: &ChoiceSet) -> ChoiceSet {
        ChoiceSet {
            choices: c.choices.iter().map(|x| self.r_choice(r, x)).collect(),
            intervals: c.intervals.clone(),
        }
    }

    fn r_rhs<T>(
        &mut self,
        r: &mut Remap,
        rhs: &Rhs<T>,
        f: &mut dyn FnMut(&mut Db, &mut Remap, &T) -> T,
    ) -> Rhs<T> {
        match rhs {
            Rhs::Simple(t) => Rhs::Simple(f(self, r, t)),
            Rhs::Conditional(alts) => Rhs::Conditional(
                alts.iter()
                    .map(|(t, c)| (f(self, r, t), c.map(|c| self.r_expr(r, c))))
                    .collect(),
            ),
            Rhs::Selected {
                expr,
                matching,
                alts,
            } => Rhs::Selected {
                expr: self.r_expr(r, *expr),
                matching: *matching,
                alts: alts
                    .iter()
                    .map(|(t, c)| (f(self, r, t), self.r_choices(r, c)))
                    .collect(),
            },
        }
    }

    fn r_waveform(&mut self, r: &mut Remap, w: &Waveform) -> Waveform {
        match w {
            Waveform::Unaffected => Waveform::Unaffected,
            Waveform::Elements(es) => Waveform::Elements(
                es.iter()
                    .map(|e| WaveElem {
                        value: e.value.map(|v| self.r_expr(r, v)),
                        after: e.after.map(|a| self.r_expr(r, a)),
                    })
                    .collect(),
            ),
        }
    }

    fn r_delay(&mut self, r: &mut Remap, d: &Delay) -> Delay {
        match d {
            Delay::Default => Delay::Default,
            Delay::Transport => Delay::Transport,
            Delay::Inertial { reject } => Delay::Inertial {
                reject: reject.map(|e| self.r_expr(r, e)),
            },
        }
    }

    fn r_call(&mut self, r: &mut Remap, c: &Call) -> Call {
        Call {
            callee: self.r_decl(r, c.callee),
            args: self.r_args(r, &c.args),
            method_of: c.method_of.map(|e| self.r_expr(r, e)),
            span: c.span,
        }
    }

    fn r_stmt_kind(&mut self, r: &mut Remap, k: &StmtKind) -> StmtKind {
        use StmtKind as S;
        let oe = |db: &mut Db, r: &mut Remap, e: &Option<ExprId>| e.map(|e| db.r_expr(r, e));
        match k {
            S::Wait { on, until, timeout } => S::Wait {
                on: on.iter().map(|l| self.r_lsp(r, l)).collect(),
                until: oe(self, r, until),
                timeout: oe(self, r, timeout),
            },
            S::Assert {
                cond,
                report,
                severity,
            } => S::Assert {
                cond: self.r_expr(r, *cond),
                report: oe(self, r, report),
                severity: oe(self, r, severity),
            },
            S::Report { report, severity } => S::Report {
                report: self.r_expr(r, *report),
                severity: oe(self, r, severity),
            },
            S::SignalAssign { target, delay, rhs } => S::SignalAssign {
                target: self.r_target(r, target),
                delay: self.r_delay(r, delay),
                rhs: self.r_rhs(r, rhs, &mut |db, r, w| db.r_waveform(r, w)),
            },
            S::Force { target, mode, rhs } => S::Force {
                target: self.r_target(r, target),
                mode: *mode,
                rhs: self.r_rhs(r, rhs, &mut |db, r, e| db.r_expr(r, *e)),
            },
            S::Release { target, mode } => S::Release {
                target: self.r_target(r, target),
                mode: *mode,
            },
            S::VarAssign { target, rhs } => S::VarAssign {
                target: self.r_target(r, target),
                rhs: self.r_rhs(r, rhs, &mut |db, r, e| db.r_expr(r, *e)),
            },
            S::ProcCall(c) => S::ProcCall(self.r_call(r, c)),
            S::If { branches, else_ } => S::If {
                branches: branches
                    .iter()
                    .map(|(c, b)| (self.r_expr(r, *c), self.r_stmts(r, b)))
                    .collect(),
                else_: else_.as_ref().map(|b| self.r_stmts(r, b)),
            },
            S::Case {
                expr,
                matching,
                alts,
            } => S::Case {
                expr: self.r_expr(r, *expr),
                matching: *matching,
                alts: alts
                    .iter()
                    .map(|(c, b)| (self.r_choices(r, c), self.r_stmts(r, b)))
                    .collect(),
            },
            S::Loop {
                scheme,
                scope,
                body,
            } => S::Loop {
                scheme: match scheme {
                    LoopScheme::Forever => LoopScheme::Forever,
                    LoopScheme::While(e) => LoopScheme::While(self.r_expr(r, *e)),
                    LoopScheme::For { param, range } => LoopScheme::For {
                        param: self.r_decl(r, *param),
                        range: self.r_range(r, range),
                    },
                },
                scope: self.r_scope(r, *scope),
                body: self.r_stmts(r, body),
            },
            S::Next { loop_, when } => S::Next {
                loop_: self.r_stmt(r, *loop_),
                when: oe(self, r, when),
            },
            S::Exit { loop_, when } => S::Exit {
                loop_: self.r_stmt(r, *loop_),
                when: oe(self, r, when),
            },
            S::Return(e) => S::Return(oe(self, r, e)),
            S::Null => S::Null,
            S::SeqBlock { scope, body } => S::SeqBlock {
                scope: self.r_scope(r, *scope),
                body: self.r_stmts(r, body),
            },
            // Concurrent statements: design entities cloned per generic map (`entity.rs`).
            S::Process {
                postponed,
                sensitivity,
                info,
                scope,
                body,
            } => S::Process {
                postponed: *postponed,
                sensitivity: match sensitivity {
                    Sensitivity::None => Sensitivity::None,
                    Sensitivity::All => Sensitivity::All,
                    Sensitivity::List(l) => {
                        Sensitivity::List(l.iter().map(|l| self.r_lsp(r, l)).collect())
                    }
                },
                info: self.r_info(r, info),
                scope: self.r_scope(r, *scope),
                body: self.r_stmts(r, body),
            },
            S::Block {
                scope,
                guard,
                generics,
                generic_map,
                ports,
                port_map,
                body,
            } => S::Block {
                scope: self.r_scope(r, *scope),
                guard: guard.map(|g| self.r_decl(r, g)),
                generics: generics.iter().map(|&d| self.r_decl(r, d)).collect(),
                generic_map: self.r_args(r, generic_map),
                ports: ports.iter().map(|&d| self.r_decl(r, d)).collect(),
                port_map: self.r_args(r, port_map),
                body: self.r_stmts(r, body),
            },
            S::ConcSignalAssign {
                postponed,
                guarded,
                target,
                delay,
                rhs,
                info,
            } => S::ConcSignalAssign {
                postponed: *postponed,
                guarded: *guarded,
                target: self.r_target(r, target),
                delay: self.r_delay(r, delay),
                rhs: self.r_rhs(r, rhs, &mut |db, r, w| db.r_waveform(r, w)),
                info: self.r_info(r, info),
            },
            S::ConcProcCall {
                postponed,
                call,
                info,
            } => S::ConcProcCall {
                postponed: *postponed,
                call: self.r_call(r, call),
                info: self.r_info(r, info),
            },
            S::ConcAssert {
                postponed,
                cond,
                report,
                severity,
                info,
            } => S::ConcAssert {
                postponed: *postponed,
                cond: self.r_expr(r, *cond),
                report: oe(self, r, report),
                severity: oe(self, r, severity),
                info: self.r_info(r, info),
            },
            S::Instance {
                target,
                scope,
                generic_map,
                port_map,
            } => {
                let target = match target {
                    InstTarget::Component(c) => InstTarget::Component(self.r_decl(r, *c)),
                    InstTarget::Entity { unit, arch } => InstTarget::Entity {
                        unit: *unit,
                        arch: *arch,
                    },
                    InstTarget::Configuration(u) => InstTarget::Configuration(*u),
                };
                S::Instance {
                    target,
                    scope: self.r_scope(r, *scope),
                    generic_map: self.r_args(r, generic_map),
                    port_map: self.r_args(r, port_map),
                }
            }
            S::ForGenerate {
                param,
                range,
                scope,
                body,
            } => S::ForGenerate {
                param: self.r_decl(r, *param),
                range: self.r_range(r, range),
                scope: self.r_scope(r, *scope),
                body: self.r_stmts(r, body),
            },
            S::IfGenerate { alts } => S::IfGenerate {
                alts: alts.iter().map(|a| self.r_gen_alt(r, a)).collect(),
            },
            S::CaseGenerate { expr, alts } => S::CaseGenerate {
                expr: self.r_expr(r, *expr),
                alts: alts
                    .iter()
                    .map(|(c, a)| (self.r_choices(r, c), self.r_gen_alt(r, a)))
                    .collect(),
            },
        }
    }

    fn r_info(&mut self, r: &mut Remap, i: &ProcessInfo) -> ProcessInfo {
        ProcessInfo {
            drivers: i.drivers.iter().map(|l| self.r_lsp(r, l)).collect(),
            reads_all: i
                .reads_all
                .as_ref()
                .map(|v| v.iter().map(|l| self.r_lsp(r, l)).collect()),
        }
    }

    fn r_gen_alt(&mut self, r: &mut Remap, a: &GenAlt) -> GenAlt {
        GenAlt {
            label: a.label,
            cond: a.cond.map(|e| self.r_expr(r, e)),
            scope: self.r_scope(r, a.scope),
            body: self.r_stmts(r, &a.body),
        }
    }

    // -- generic subprogram bodies -------------------------------------------------------------

    /// Before the spec of a generic subprogram body is analyzed: if it completes
    /// a pending generic spec, the body reuses the spec's generics (4.10: the
    /// generic lists conform) instead of declaring new opaque types.
    pub(crate) fn prepare_generic_body(&mut self, scope: ScopeId, spec: &ast::SubprogramSpec) {
        let Some(list) = &spec.generics else {
            return;
        };
        let key = crate::alias::designator(&spec.designator).key();
        let mut regions = vec![scope];
        match self.scopes[scope].kind {
            ScopeKind::PackageBody => {
                if let Some(c) = self.scopes[scope].parent
                    && let Some(p) = self.scopes[c].parent
                {
                    regions.push(p);
                }
            }
            ScopeKind::ProtectedBody => regions.extend(self.scopes[scope].parent),
            _ => {}
        }
        let names: Vec<Symbol> = list.iter().flat_map(interface_keys).collect();
        let mapped = spec.generic_map.is_some();
        let found = regions.iter().find_map(|&r| {
            self.scopes[r]
                .names
                .get(&key)?
                .iter()
                .copied()
                .find_map(|d| {
                    // A body with a header generic map completes the generic of a spec with one.
                    let d = if mapped { self.header_generic(d)? } else { d };
                    matches!(self.decls[d].kind.subprogram(), Some(s)
                    if matches!(s.body, Body::Pending)
                        && s.generics.as_ref().is_some_and(|g| {
                            g.iter().map(|&x| self.decls[x].name.key()).collect::<Vec<_>>() == names
                        }))
                    .then_some(d)
                })
        });
        self.generic.reuse = found;
    }

    /// The generic list of a body that completes generic spec `spec`: the spec's
    /// formals and their class operations, declared again in the body's region.
    fn reuse_generics(&mut self, scope: ScopeId, spec: DeclId) -> Box<[DeclId]> {
        let s = self.decls[spec].kind.subprogram().expect("generic spec");
        let list = s.generics.clone().unwrap_or_default();
        let pscope = s.scope.expect("specs have a parameter region");
        let params: HashSet<DeclId> = s.params.iter().filter_map(|p| p.decl).collect();
        let decls: Vec<DeclId> = self.scopes[pscope]
            .order
            .iter()
            .copied()
            .filter(|d| !params.contains(d))
            .collect();
        for d in decls {
            let k = self.decls[d].name.key();
            let s = &mut self.scopes[scope];
            s.names.entry(k).or_default().push(d);
            s.order.push(d);
        }
        self.generic.lists.insert(scope, list.clone());
        list
    }
}

/// The keys an interface declaration declares.
fn interface_keys(g: &ast::InterfaceDecl) -> Vec<Symbol> {
    match g {
        ast::InterfaceDecl::Object(o) => o.names.iter().map(|n| n.key).collect(),
        ast::InterfaceDecl::Type(t) => vec![t.name.key],
        ast::InterfaceDecl::Subprogram(s) => {
            vec![crate::alias::designator(&s.spec.designator).key()]
        }
        ast::InterfaceDecl::Package(p) => vec![p.name.key],
    }
}

fn last_name(n: &ast::Name) -> String {
    match &n.kind {
        ast::NameKind::Simple(i) => i.text.to_string(),
        ast::NameKind::Selected { suffix, .. } => match suffix {
            ast::Suffix::Ident(i) => i.text.to_string(),
            ast::Suffix::Char(c) => format!("'{}'", c.value),
            ast::Suffix::Operator(op, _) => format!("\"{op}\""),
            ast::Suffix::All(_) => "all".into(),
        },
        ast::NameKind::Operator(op) => format!("\"{op}\""),
        _ => "name".into(),
    }
}

/// `+` for `"+"`, the name as the user wrote it without quotes.
fn fname_plain(d: &Designator) -> String {
    match d {
        Designator::Operator { text, .. } => text.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
#[path = "generic_tests.rs"]
mod tests;
