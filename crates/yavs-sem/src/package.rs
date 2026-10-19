//! Packages and package bodies declared in declarative parts (LRM 4.7, 4.8, 2008), generic
//! ones included: `DeclKind::Package`. The package's region is a `ScopeKind::Package` child of
//! the declaring region, its body a `ScopeKind::PackageBody` child of the package's region.

use yavs_diag::Diagnostic;
use yavs_syntax::ast;

use crate::db::{Db, Reported};
use crate::decl::{DeclKind, Designator};
use crate::diag;
use crate::ids::{DeclId, ScopeId};
use crate::scope::ScopeKind;

impl Db {
    /// `package p is ... end;` in a declarative part.
    pub(crate) fn nested_package(
        &mut self,
        scope: ScopeId,
        p: &ast::PackageDecl,
    ) -> Result<(), Reported> {
        if p.generic_map.is_some() {
            return Err(self.unsupported(p.name.span, "a package with a generic map", "S4"));
        }
        let d = self.new_decl(scope, Designator::Ident(p.name), DeclKind::Error, p.span);
        let s = self.add_scope(ScopeKind::Package, Some(scope), Some(d));
        let generics = p.generics.as_ref().map(|g| self.generic_list(s, g));
        let failed = matches!(generics, Some(Err(_)));
        self.decls[d].kind = DeclKind::Package {
            scope: s,
            generics: generics.and_then(Result::ok),
            body: None,
        };
        self.declare(scope, d);
        self.decls_in(s, &p.decls);
        if failed { Err(Reported) } else { Ok(()) }
    }

    /// `package body p is ... end;` in a declarative part: in the region declaring `p`, or, for
    /// a package declared in a package declaration, in that package's body (12.1).
    pub(crate) fn nested_package_body(
        &mut self,
        scope: ScopeId,
        b: &ast::PackageBody,
    ) -> Result<(), Reported> {
        let mut regions = vec![scope];
        regions.extend(self.body_package(scope));
        let found = regions.iter().find_map(|&r| {
            self.scopes[r].names.get(&b.name.key).and_then(|ds| {
                ds.iter()
                    .copied()
                    .find(|&d| matches!(self.decls[d].kind, DeclKind::Package { .. }))
            })
        });
        let Some(d) = found else {
            return Err(self.error(
                Diagnostic::error(
                    diag::NO_PRIMARY,
                    format!("package `{}` is not declared in this region", b.name.text),
                )
                .with_primary(b.name.span, "no package with this name")
                .with_help("declare the package earlier in the same declarative part"),
            ));
        };
        let DeclKind::Package {
            scope: pkg,
            ref generics,
            body,
        } = self.decls[d].kind
        else {
            unreachable!("found above");
        };
        let generics = generics.clone();
        if body.is_some() {
            return Err(self.error(
                Diagnostic::error(
                    diag::REDECLARED,
                    format!("package `{}` already has a body", b.name.text),
                )
                .with_primary(b.name.span, "second body")
                .with_secondary(self.decls[d].span, "package declared here"),
            ));
        }
        let s = self.add_scope(ScopeKind::PackageBody, Some(pkg), Some(d));
        if self.decls[d].scope != scope {
            // Lookup continues in the body's own enclosing region after the package's.
            self.scopes[s].outer = Some(scope);
        }
        if let DeclKind::Package { body, .. } = &mut self.decls[d].kind {
            *body = Some(s);
        }
        self.decls_in(s, &b.decls);
        let _ = self.check_protected_bodies(pkg, b.span);
        if let Some(g) = generics {
            self.report_unused_generics(&g);
        }
        Ok(())
    }

    /// The package region of package body region `body`: its parent for a nested body, else
    /// the parent of its context region (unit.rs).
    pub(crate) fn body_package(&self, body: ScopeId) -> Option<ScopeId> {
        if self.scopes[body].kind != ScopeKind::PackageBody {
            return None;
        }
        let p = self.scopes[body].parent?;
        let p = if self.scopes[p].kind == ScopeKind::Package {
            p
        } else {
            self.scopes[p].parent?
        };
        (self.scopes[p].kind == ScopeKind::Package).then_some(p)
    }

    /// The process or subprogram `scope` is in, if any.
    fn sequential_owner(&self, mut scope: ScopeId) -> Option<ScopeId> {
        loop {
            if matches!(
                self.scopes[scope].kind,
                ScopeKind::Process | ScopeKind::Subprogram
            ) {
                return Some(scope);
            }
            scope = self.scopes[scope].parent?;
        }
    }

    /// `E1237`: a signal or shared variable declared at `span` in region `scope` (directly, in
    /// a nested package, or object `inst_obj` through a package instance) that is inside a
    /// process or subprogram.
    pub(crate) fn check_not_sequential(
        &mut self,
        scope: ScopeId,
        span: yavs_diag::Span,
        inst_obj: Option<DeclId>,
    ) -> Result<(), Reported> {
        let Some(s) = self.sequential_owner(scope) else {
            return Ok(());
        };
        let what = if self.scopes[s].kind == ScopeKind::Process {
            "process"
        } else {
            "subprogram"
        };
        let mut d = Diagnostic::error(
            diag::SEQUENTIAL_SIGNAL,
            format!("a {what} declares no signals or shared variables"),
        )
        .with_primary(span, format!("declared inside a {what}"))
        .with_help("declare it in the enclosing architecture, block or package");
        if let Some(o) = inst_obj {
            let name = self.decls[o].name;
            d = d.with_secondary(
                self.decls[o].span,
                format!("`{name}` is declared in the package"),
            );
        }
        Err(self.error(d))
    }

    /// The packages declared in `scope` that have no body (yet).
    pub(crate) fn bodiless_packages(&self, scope: ScopeId) -> Vec<(DeclId, ScopeId)> {
        self.scopes[scope]
            .order
            .iter()
            .filter_map(|&d| match self.decls[d].kind {
                DeclKind::Package {
                    scope, body: None, ..
                } => Some((d, scope)),
                _ => None,
            })
            .collect()
    }
}
