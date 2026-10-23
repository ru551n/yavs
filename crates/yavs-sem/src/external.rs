//! External names (sem.md 6.1, LRM 8.7). Sem resolves the subtype and the
//! class; the path is kept as data ([`crate::hir::External`]) and bound by
//! elab (rule 4, sim2 #548), in elaboration order. Nothing here looks the path
//! up, not even a package path: one code path for every kind.
//!
//! The `External` record is the elaboration-time binding record: class, path
//! kind (`@lib.pkg.obj`, `.top.x`, `^.^.x`), the path elements with their
//! resolved generate indexes, the subtype the target must have, and the span
//! for elab's "not found" diagnostic. Relative paths start at the innermost
//! enclosing concurrent region of the instance elab is elaborating; `work` in
//! a package path is the library of the unit containing the name.

use yavs_diag::{Diagnostic, Span};
use yavs_syntax::ast::{self, ExternalClass, PathKind};

use crate::db::{Db, Reported};
use crate::decl::{Alias, Body, DeclKind, Designator, ObjClass, ObjRole};
use crate::diag;
use crate::hir::{Expr, ExprKind, External, Lsp, ObjRoot, Sensitivity, StmtKind};
use crate::ids::{DeclId, ExprId, ExternalId, ScopeId, StmtId, SubtypeId};
use crate::stat::Staticness;
use crate::types::TypeKind;

/// Where an external name appears, for the checks that depend on it.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExternalCx {
    /// Inside a concurrent region (entity, architecture, block, generate),
    /// directly or through processes and subprograms: relative paths need one.
    pub in_concurrent_region: bool,
    /// Inside a pure function: only constant external names are allowed.
    pub in_pure_function: bool,
}

/// How a statement uses a name as a target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetUse {
    SignalAssign,
    VarAssign,
    /// `force` / `release` (signals only).
    Force,
}

fn class_str(c: ExternalClass) -> &'static str {
    match c {
        ExternalClass::Constant => "constant",
        ExternalClass::Signal => "signal",
        ExternalClass::Variable => "variable",
    }
}

impl Db {
    /// Build the HIR of an external name. `subtype` is its resolved subtype
    /// indication; `indexes[i]` the resolved generate index of path element
    /// `i`, `None` where none is written.
    pub fn external_name(
        &mut self,
        ast: &ast::ExternalName,
        span: Span,
        subtype: SubtypeId,
        indexes: &[Option<ExprId>],
        cx: ExternalCx,
    ) -> Result<ExprId, Reported> {
        let elems = &ast.path.elements;
        if indexes.len() != elems.len() {
            return Err(self.ice(span, "external name index list does not match its path"));
        }
        let path_err = |db: &mut Db, at: Span, msg: &str, label: &str| {
            Err(db.error(
                Diagnostic::error(diag::EXTERNAL_PATH, msg.to_string())
                    .with_primary(at, label.to_string())
                    .with_note("LRM 8.7"),
            ))
        };
        match ast.path.kind {
            PathKind::Package if elems.len() < 3 => {
                return path_err(
                    self,
                    ast.path.span,
                    "a package path names a library, a package and an object",
                    "expected `@lib.pkg.obj`",
                );
            }
            PathKind::Absolute if elems.len() < 2 => {
                return path_err(
                    self,
                    ast.path.span,
                    "an absolute path starts with the name of the top entity",
                    "expected `.top.obj`",
                );
            }
            PathKind::Relative(_) if !cx.in_concurrent_region => {
                return path_err(
                    self,
                    ast.path.span,
                    "a relative path needs an enclosing concurrent region",
                    "relative to nothing here",
                );
            }
            _ => {}
        }
        for (i, (el, ix)) in elems.iter().zip(indexes.iter()).enumerate() {
            let Some(ix) = *ix else { continue };
            let at = self.exprs[ix].span;
            if ast.path.kind == PathKind::Package || i + 1 == elems.len() {
                return path_err(
                    self,
                    at,
                    &format!(
                        "`{}` cannot take an index: only generate labels do",
                        el.name.text
                    ),
                    "index not allowed here",
                );
            }
            if self.exprs[ix].stat < Staticness::Globally
                && !matches!(self.exprs[ix].kind, ExprKind::Error)
            {
                return path_err(
                    self,
                    at,
                    "a generate index in an external name must be globally static",
                    "not globally static",
                );
            }
        }
        if cx.in_pure_function && ast.class != ExternalClass::Constant {
            return Err(self.error(
                Diagnostic::error(
                    diag::PURE_SIDE_EFFECT,
                    format!(
                        "a pure function cannot refer to an external {}",
                        class_str(ast.class)
                    ),
                )
                .with_primary(span, "external name in a pure function")
                .with_help("declare the function `impure`"),
            ));
        }
        if ast.class != ExternalClass::Variable {
            self.check_not_protected(class_str(ast.class), subtype, span)?;
        }
        let path = elems
            .iter()
            .zip(indexes.iter())
            .map(|(e, &ix)| (e.name, ix))
            .collect();
        let ext = self.externals.push(External {
            class: ast.class,
            path_kind: ast.path.kind,
            path,
            subtype,
            span,
        });
        // 9.4.3: an external constant name is globally static; the others are not static.
        let stat = match ast.class {
            ExternalClass::Constant => Staticness::Globally,
            _ => Staticness::None,
        };
        Ok(self.add_expr(Expr {
            kind: ExprKind::External(ext),
            subtype,
            span,
            stat,
            value: None,
        }))
    }

    pub fn external_class(&self, e: ExternalId) -> ExternalClass {
        self.externals[e].class
    }

    /// An external name used as an assignment target must have the class the
    /// statement assigns (`E1601`).
    pub fn check_external_target(
        &mut self,
        e: ExternalId,
        use_: TargetUse,
        span: Span,
    ) -> Result<(), Reported> {
        let class = self.externals[e].class;
        let ok = match use_ {
            TargetUse::SignalAssign | TargetUse::Force => class == ExternalClass::Signal,
            TargetUse::VarAssign => class == ExternalClass::Variable,
        };
        if ok {
            return Ok(());
        }
        let what = match use_ {
            TargetUse::SignalAssign => "signal assignment",
            TargetUse::VarAssign => "variable assignment",
            TargetUse::Force => "force or release",
        };
        Err(self.error(
            Diagnostic::error(
                diag::EXTERNAL_CLASS,
                format!(
                    "an external {} name is not a valid target of a {what}",
                    class_str(class)
                ),
            )
            .with_primary(span, format!("target of {what}"))
            .with_secondary(
                self.externals[e].span,
                format!("declared with class `{}`", class_str(class)),
            ),
        ))
    }
}

/// What owns the implicit aliases of the external names it uses directly (#93).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ExtOwner {
    /// A process or concurrent statement: its statements, the initial values of its
    /// declarations, its sensitivity, wait and driver names.
    Stmt(StmtId),
    /// An object alias of a part of an external name (`alias b is <<signal ...>>(3)`).
    Alias(DeclId),
    /// An expression evaluated by its own equivalent process: an expression or `inertial`
    /// port actual.
    Expr(ExprId),
    /// A subprogram declared in a package, an architecture, a block, a generate or a process
    /// (or a method of a protected body declared there): its body, with the subprograms and
    /// protected bodies nested in it. Its aliases belong to that region, bound once per
    /// elaborated region; declared aliases of whole external names in it are among them
    /// (#255).
    Sub(DeclId),
}

impl Db {
    /// The implicit aliases [`Db::alias_external_uses`] made for `owner`.
    pub fn external_uses(&self, owner: ExtOwner) -> &[DeclId] {
        self.ext_uses.get(&owner).map_or(&[], |v| v)
    }

    /// Give each external name `owner` uses directly an implicit alias declared in `scope`
    /// (`Alias::Object` of the external name, `implicit`), and make the uses name the alias:
    /// elab binds the alias like a declared one, M2 lowers it like one (#93). Names that only
    /// appear as the root of a sensitivity, wait or driver LSP get an alias too. Idempotent:
    /// a second call returns the same aliases.
    pub fn alias_external_uses(&mut self, owner: ExtOwner, scope: ScopeId) -> &[DeclId] {
        if !self.ext_uses.contains_key(&owner) {
            let mut exprs = Vec::new();
            let mut roots = Vec::new();
            // Declared aliases of whole external names (`ExtOwner::Sub`).
            let mut own = Vec::new();
            // Without an external name in the design there is nothing to find, and looking
            // decodes every expression of the statements of a run restored from an image (#426).
            if !self.externals.is_empty() {
                match owner {
                    ExtOwner::Stmt(s) => self.stmt_externals(s, &mut exprs, &mut roots),
                    ExtOwner::Sub(d) => self.sub_externals(d, &mut exprs, &mut roots, &mut own),
                    ExtOwner::Expr(e) => self.walk_expr(e, &mut |x| exprs.push(x)),
                    ExtOwner::Alias(d) => {
                        if let DeclKind::Alias(Alias::Object { target, .. }) = self.decls[d].kind {
                            self.walk_expr(target, &mut |e| {
                                if e != target {
                                    exprs.push(e);
                                }
                            });
                        }
                    }
                }
            }
            let mut out: Vec<(ExternalId, DeclId)> = Vec::new();
            for e in exprs {
                let (x, made) = match self.exprs[e].kind {
                    ExprKind::External(x) => (x, None),
                    // An alias made for another owner sharing this expression.
                    ExprKind::Obj(d) => match self.implicit_external(d) {
                        Some(x) => (x, Some(d)),
                        None => continue,
                    },
                    _ => continue,
                };
                let d = match out.iter().find(|(y, _)| *y == x) {
                    Some(&(_, d)) => d,
                    None => {
                        let d = made.unwrap_or_else(|| self.implicit_alias(x, scope));
                        out.push((x, d));
                        d
                    }
                };
                self.exprs[e].kind = ExprKind::Obj(d);
            }
            for x in roots {
                if !out.iter().any(|(y, _)| *y == x) {
                    let d = self.implicit_alias(x, scope);
                    out.push((x, d));
                }
            }
            own.extend(out.into_iter().map(|(_, d)| d));
            self.ext_uses.insert(owner, own.into());
        }
        &self.ext_uses[&owner]
    }

    /// Whether expression `e` contains an external name: a constant initialised from one is
    /// evaluated by elab, when its declaration is elaborated (LRM 8.7, 14.4.1).
    pub fn uses_external(&self, e: ExprId) -> bool {
        let mut found = false;
        self.walk_expr(e, &mut |x| {
            found |= matches!(self.exprs[x].kind, ExprKind::External(_));
        });
        found
    }

    /// The external name an implicit alias of [`Db::alias_external_uses`] stands for.
    pub fn implicit_external(&self, d: DeclId) -> Option<ExternalId> {
        let decl = &self.decls[d];
        match &decl.kind {
            DeclKind::Alias(Alias::Object { target, .. }) if decl.implicit => {
                match self.exprs[*target].kind {
                    ExprKind::External(x) => Some(x),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The external name a whole-name object alias (declared or implicit) aliases, of any
    /// class: `alias c is <<constant .tb.dut.c : integer>>;`.
    pub fn external_alias(&self, d: DeclId) -> Option<ExternalId> {
        match &self.decls[d].kind {
            DeclKind::Alias(Alias::Object { target, .. }) => match self.exprs[*target].kind {
                ExprKind::External(x) => Some(x),
                _ => None,
            },
            _ => None,
        }
    }

    fn implicit_alias(&mut self, x: ExternalId, scope: ScopeId) -> DeclId {
        let ext = &self.externals[x];
        let (st, span) = (ext.subtype, ext.span);
        let name = ext.path.last().expect("an external path is never empty").0;
        let stat = match ext.class {
            ExternalClass::Constant => Staticness::Globally,
            _ => Staticness::None,
        };
        let target = self.add_expr(Expr {
            kind: ExprKind::External(x),
            subtype: st,
            span,
            stat,
            value: None,
        });
        let kind = DeclKind::Alias(Alias::Object {
            target,
            subtype: None,
        });
        let d = self.new_decl(scope, Designator::Ident(name), kind, span);
        self.decls[d].implicit = true;
        d
    }

    /// The subprograms with VHDL bodies declaration `d` brings: itself, or the methods of a
    /// protected type's body (#255).
    pub fn decl_subprograms(&self, d: DeclId) -> Vec<DeclId> {
        match &self.decls[d].kind {
            DeclKind::Subprogram(sp) if matches!(sp.body, Body::Hir { .. }) => vec![d],
            // Methods are declared in the protected type declaration and completed in the body;
            // the body's own subprograms are declared there.
            DeclKind::Type(t) => match self.types[*t].kind {
                TypeKind::Protected {
                    decl_scope,
                    body: Some(b),
                    ..
                } => {
                    let mut out: Vec<DeclId> = Vec::new();
                    for &m in self.scopes[decl_scope]
                        .order
                        .iter()
                        .chain(&self.scopes[b].order)
                    {
                        if !out.contains(&m)
                            && matches!(&self.decls[m].kind,
                                DeclKind::Subprogram(sp) if matches!(sp.body, Body::Hir { .. }))
                        {
                            out.push(m);
                        }
                    }
                    out
                }
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// The scope of subprogram `d`'s body.
    pub fn sub_scope(&self, d: DeclId) -> Option<ScopeId> {
        match &self.decls[d].kind.subprogram()?.body {
            Body::Hir { scope, .. } => Some(*scope),
            _ => None,
        }
    }

    /// External names in subprogram `d`'s body and the subprograms nested in it.
    fn sub_externals(
        &self,
        d: DeclId,
        exprs: &mut Vec<ExprId>,
        roots: &mut Vec<ExternalId>,
        own: &mut Vec<DeclId>,
    ) {
        let Some(sp) = self.decls[d].kind.subprogram() else {
            return;
        };
        let Body::Hir { scope, stmts } = &sp.body else {
            return;
        };
        let push = |db: &Db, e: ExprId, exprs: &mut Vec<ExprId>| {
            db.walk_expr(e, &mut |e| {
                if matches!(db.exprs[e].kind, ExprKind::External(_) | ExprKind::Obj(_)) {
                    exprs.push(e);
                }
            })
        };
        for &x in &self.scopes[*scope].order {
            match &self.decls[x].kind {
                DeclKind::Object(o) if o.role != ObjRole::Param => {
                    if let Some(e) = o.default {
                        push(self, e, exprs);
                    }
                }
                DeclKind::Alias(Alias::Object { target, .. }) => {
                    if self.external_alias(x).is_some() {
                        own.push(x);
                    } else {
                        push(self, *target, exprs);
                    }
                }
                _ => {
                    for m in self.decl_subprograms(x) {
                        self.sub_externals(m, exprs, roots, own);
                    }
                }
            }
        }
        for &s in stmts.iter() {
            self.stmt_externals(s, exprs, roots);
        }
    }

    /// Resolve every external name with a package path (`@lib.pkg.obj`) that names an
    /// analyzed package object of its class and type: the name becomes a name of that object,
    /// like any other use of it (LRM 8.7: it denotes the object itself), so it is never
    /// bound per region. Names that do not resolve are left to elab, which reports them where
    /// they are bound. Idempotent (#255).
    pub fn bind_package_paths(&mut self) {
        // The list comes from the settled analysis, not from a walk over every expression.
        let uses = self.facts().package_paths.clone();
        for (e, x) in uses {
            if !matches!(self.exprs[e].kind, ExprKind::External(_)) {
                continue;
            }
            if let Ok(d) = self.package_path_target(x) {
                self.pkg_targets.insert(x, d);
                self.exprs[e].kind = ExprKind::Obj(d);
            }
        }
    }

    /// The package object a package-path external name resolved to ([`Db::bind_package_paths`]).
    pub fn package_target(&self, x: ExternalId) -> Option<DeclId> {
        self.pkg_targets.get(&x).copied()
    }

    /// The object package-path external name `x` (`@lib.pkg.obj`) names, or why there is
    /// none.
    pub fn package_path_target(&self, x: ExternalId) -> Result<DeclId, String> {
        let ext = &self.externals[x];
        let [(l, _), (p, _), (last, _)] = &ext.path[..] else {
            return Err("nested package paths are not supported yet".into());
        };
        let unit = self
            .libs
            .iter()
            .find(|(_, lib)| lib.name.key == l.key)
            .and_then(
                |(_, lib)| match lib.units.get(&crate::loader::UnitKey::Primary(p.key)) {
                    Some(crate::loader::UnitSlot::Done(u)) => Some(*u),
                    _ => None,
                },
            );
        let Some(u) = unit else {
            return Err(format!("package `{}.{}` is not analyzed", l.text, p.text));
        };
        let Some(d) = self.scopes[self.units[u].scope]
            .names
            .get(&last.key)
            .and_then(|v| v.first().copied())
        else {
            return Err(format!("package `{}` declares no `{}`", p.text, last.text));
        };
        let what = class_str(ext.class);
        match &self.decls[d].kind {
            DeclKind::Object(o)
                if matches!(
                    (ext.class, o.class),
                    (ExternalClass::Constant, ObjClass::Constant)
                        | (ExternalClass::Signal, ObjClass::Signal)
                        | (ExternalClass::Variable, ObjClass::SharedVariable)
                ) =>
            {
                if self.base(o.subtype) != self.base(ext.subtype) {
                    return Err(format!(
                        "`{}` is not of the external name's type",
                        last.text
                    ));
                }
                Ok(d)
            }
            _ => Err(format!("`{}` is not a {what}", last.text)),
        }
    }

    /// External names (and implicit aliases of them) in statement `s` and below; `roots`
    /// gets the external names at the root of its LSPs.
    fn stmt_externals(&self, s: StmtId, exprs: &mut Vec<ExprId>, roots: &mut Vec<ExternalId>) {
        let mut lsps = |ls: &[Lsp]| {
            roots.extend(ls.iter().filter_map(|l| match l.root {
                ObjRoot::External(x) => Some(x),
                _ => None,
            }))
        };
        let mut push = |db: &Db, e: ExprId| {
            db.walk_expr(e, &mut |e| {
                if matches!(db.exprs[e].kind, ExprKind::External(_) | ExprKind::Obj(_)) {
                    exprs.push(e);
                }
            })
        };
        match &self.stmts[s].kind {
            StmtKind::Process {
                sensitivity,
                info,
                scope,
                ..
            } => {
                if let Sensitivity::List(ls) = sensitivity {
                    lsps(ls);
                }
                lsps(&info.drivers);
                lsps(info.reads_all.as_deref().unwrap_or(&[]));
                for &d in &self.scopes[*scope].order {
                    if let DeclKind::Object(o) = &self.decls[d].kind
                        && let Some(e) = o.default
                    {
                        push(self, e);
                    }
                }
            }
            StmtKind::ConcSignalAssign { info, .. }
            | StmtKind::ConcAssert { info, .. }
            | StmtKind::ConcProcCall { info, .. } => {
                lsps(&info.drivers);
                lsps(info.reads_all.as_deref().unwrap_or(&[]));
            }
            StmtKind::Wait { on, .. } => lsps(on),
            _ => {}
        }
        self.stmt_exprs(s, &mut |e, _| push(self, e));
        for c in self.stmt_children(s) {
            self.stmt_externals(c, exprs, roots);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alias::fixture::*;
    use crate::scope::ScopeKind;

    fn ext(class: ExternalClass, kind: PathKind, names: &[&str]) -> ast::ExternalName {
        ast::ExternalName {
            class,
            path: ast::ExternalPath {
                kind,
                elements: names
                    .iter()
                    .map(|n| ast::PathElement {
                        name: ident(n),
                        index: None,
                    })
                    .collect(),
                span: sp(10),
            },
            subtype: ast::SubtypeIndication {
                resolution: None,
                type_mark: ast::Name {
                    kind: ast::NameKind::Simple(ident("t")),
                    span: sp(11),
                },
                constraint: None,
                span: sp(11),
            },
        }
    }

    const CONC: ExternalCx = ExternalCx {
        in_concurrent_region: true,
        in_pure_function: false,
    };

    #[test]
    fn builds_binding_record() {
        let mut db = db();
        let s = region(&mut db, ScopeKind::Package);
        let (_, int) = int_type(&mut db, s, "integer");
        let gix = expr(&mut db, ExprKind::Null, int, Staticness::Locally);
        let a = ext(ExternalClass::Signal, PathKind::Absolute, &["tb", "g", "s"]);
        let e = db
            .external_name(&a, sp(1), int, &[None, Some(gix), None], CONC)
            .unwrap();
        let ExprKind::External(x) = db.exprs[e].kind else {
            panic!()
        };
        assert_eq!(db.exprs[e].stat, Staticness::None);
        assert_eq!(db.externals[x].path.len(), 3);
        assert_eq!(db.externals[x].path[1].1, Some(gix));
        let c = ext(
            ExternalClass::Constant,
            PathKind::Package,
            &["work", "p", "c"],
        );
        let ce = db
            .external_name(&c, sp(2), int, &[None; 3], ExternalCx::default())
            .unwrap();
        assert_eq!(db.exprs[ce].stat, Staticness::Globally);
        assert!(db.diagnostics().is_empty());
    }

    #[test]
    fn path_shape_errors() {
        let mut db = db();
        let s = region(&mut db, ScopeKind::Package);
        let (_, int) = int_type(&mut db, s, "integer");
        let (_, v) = object(&mut db, s, "v", crate::decl::ObjClass::Variable, int);
        let st = expr(&mut db, ExprKind::Null, int, Staticness::Locally);
        let pkg2 = ext(ExternalClass::Constant, PathKind::Package, &["work", "c"]);
        assert!(
            db.external_name(&pkg2, sp(1), int, &[None; 2], CONC)
                .is_err()
        );
        let abs1 = ext(ExternalClass::Signal, PathKind::Absolute, &["s"]);
        assert!(db.external_name(&abs1, sp(1), int, &[None], CONC).is_err());
        let rel = ext(ExternalClass::Signal, PathKind::Relative(1), &["s"]);
        assert!(
            db.external_name(&rel, sp(1), int, &[None], ExternalCx::default())
                .is_err()
        );
        let last = ext(ExternalClass::Signal, PathKind::Absolute, &["tb", "s"]);
        assert!(
            db.external_name(&last, sp(1), int, &[None, Some(st)], CONC)
                .is_err()
        );
        let dynix = ext(ExternalClass::Signal, PathKind::Absolute, &["tb", "g", "s"]);
        assert!(
            db.external_name(&dynix, sp(1), int, &[None, Some(v), None], CONC)
                .is_err()
        );
        assert_eq!(codes(&db), [diag::EXTERNAL_PATH; 5]);
    }

    #[test]
    fn class_checks() {
        let mut db = db();
        let s = region(&mut db, ScopeKind::Package);
        let (_, int) = int_type(&mut db, s, "integer");
        let (_, pt, _) = protected_type(&mut db, s, "pt");
        let pure = ExternalCx {
            in_concurrent_region: true,
            in_pure_function: true,
        };
        let sig = ext(ExternalClass::Signal, PathKind::Relative(0), &["s"]);
        assert!(db.external_name(&sig, sp(1), int, &[None], pure).is_err());
        let cst = ext(ExternalClass::Constant, PathKind::Relative(0), &["c"]);
        let c = db.external_name(&cst, sp(1), int, &[None], pure).unwrap();
        // A signal of a protected type.
        assert!(db.external_name(&sig, sp(1), pt, &[None], CONC).is_err());
        let var = ext(ExternalClass::Variable, PathKind::Relative(0), &["sv"]);
        let v = db.external_name(&var, sp(1), pt, &[None], CONC).unwrap();
        let s2 = db.external_name(&sig, sp(1), int, &[None], CONC).unwrap();
        let id = |db: &Db, e: ExprId| match db.exprs[e].kind {
            ExprKind::External(x) => x,
            _ => panic!(),
        };
        let (c, v, s2) = (id(&db, c), id(&db, v), id(&db, s2));
        assert!(
            db.check_external_target(c, TargetUse::VarAssign, sp(2))
                .is_err()
        );
        assert!(
            db.check_external_target(v, TargetUse::VarAssign, sp(2))
                .is_ok()
        );
        assert!(
            db.check_external_target(v, TargetUse::SignalAssign, sp(2))
                .is_err()
        );
        assert!(
            db.check_external_target(s2, TargetUse::Force, sp(2))
                .is_ok()
        );
        assert_eq!(
            codes(&db),
            [
                diag::PURE_SIDE_EFFECT,
                diag::PROTECTED_MISUSE,
                diag::EXTERNAL_CLASS,
                diag::EXTERNAL_CLASS
            ]
        );
    }
}
