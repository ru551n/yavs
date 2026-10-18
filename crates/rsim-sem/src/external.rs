//! External names (sem.md 6.1, LRM 8.7). Sem resolves the subtype and the
//! class; the path is kept as data ([`crate::hir::External`]) and bound by
//! elab (rule 4, GHDL #548), in elaboration order. Nothing here looks the path
//! up, not even a package path: one code path for every kind.
//!
//! The `External` record is the elaboration-time binding record: class, path
//! kind (`@lib.pkg.obj`, `.top.x`, `^.^.x`), the path elements with their
//! resolved generate indexes, the subtype the target must have, and the span
//! for elab's "not found" diagnostic. Relative paths start at the innermost
//! enclosing concurrent region of the instance elab is elaborating; `work` in
//! a package path is the library of the unit containing the name.

use rsim_diag::{Diagnostic, Span};
use rsim_syntax::ast::{self, ExternalClass, PathKind};

use crate::db::{Db, Reported};
use crate::diag;
use crate::hir::{Expr, ExprKind, External};
use crate::ids::{ExprId, ExternalId, SubtypeId};
use crate::stat::Staticness;

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
