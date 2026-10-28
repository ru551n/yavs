//! Packages and package bodies declared in declarative parts (LRM 4.7, 4.8, 2008), generic
//! ones included: `DeclKind::Package`. The package's region is a `ScopeKind::Package` child of
//! the declaring region, its body a `ScopeKind::PackageBody` child of the package's region.

use yavs_diag::Diagnostic;
use yavs_syntax::ast;

use crate::db::{Db, Reported};
use crate::decl::{DeclKind, Designator};
use crate::diag;
use crate::ids::{DeclId, InstId, ScopeId};
use crate::scope::ScopeKind;

impl Db {
    /// `package p is ... end;` in a declarative part.
    pub(crate) fn nested_package(
        &mut self,
        scope: ScopeId,
        p: &ast::PackageDecl,
    ) -> Result<(), Reported> {
        self.not_nested_93(p.name.span, "package declaration")?;
        if p.generic_map.is_some() {
            let d = self.new_decl(scope, Designator::Ident(p.name), DeclKind::Error, p.span);
            let s = self.add_scope(ScopeKind::Instance, Some(scope), Some(d));
            let inst = self.mapped_package(scope, s, d, p)?;
            return self.declared_instance(scope, d, s, inst, p.name.span);
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

    /// `E1007`: in a VHDL-1993 file a package or package body is a library unit only (1993 1.1.2,
    /// 2.2, 2.5, 2.6, 4: no declarative part lists it; 2008 4.7 added nested packages, #814).
    fn not_nested_93(&mut self, span: yavs_diag::Span, what: &str) -> Result<(), Reported> {
        if self.file_rev() != crate::LangRev::Vhdl1993 {
            return Ok(());
        }
        Err(self.error(
            Diagnostic::error(
                diag::NOT_IN_REVISION,
                format!("a {what} inside a declarative part is not allowed in VHDL-1993"),
            )
            .with_primary(span, "nested here")
            .with_note("LRM 1993 1.1.2, 2.2, 2.5, 2.6: a package is a library unit only; packages in declarative parts are VHDL-2008 (LRM 2008 4.7)"),
        ))
    }

    /// `package p is generic (...); generic map (...); ... end;` (4.7, 4.9): the declaration is
    /// analysed as an invisible generic package in a region of its own and instantiated into
    /// `inst_scope`, the region `site` stands for (the unit's or the declared package's).
    pub(crate) fn mapped_package(
        &mut self,
        scope: ScopeId,
        inst_scope: ScopeId,
        site: DeclId,
        p: &ast::PackageDecl,
    ) -> Result<InstId, Reported> {
        let d = self.new_decl(scope, Designator::Ident(p.name), DeclKind::Error, p.span);
        let s = self.add_scope(ScopeKind::Package, Some(scope), Some(d));
        let generics = p.generics.as_ref().map(|g| self.generic_list(s, g));
        let failed = matches!(generics, Some(Err(_)));
        self.decls[d].kind = DeclKind::Package {
            scope: s,
            generics: generics.and_then(Result::ok),
            body: None,
        };
        self.decls_in(s, &p.decls);
        if failed {
            return Err(Reported);
        }
        let assocs = p.generic_map.as_deref().unwrap_or(&[]);
        self.instantiate_package(scope, inst_scope, site, d, assocs, p.name.span)
    }

    /// `package body p is ... end;` in a declarative part: in the region declaring `p`, or, for
    /// a package declared in a package declaration, in that package's body (12.1).
    pub(crate) fn nested_package_body(
        &mut self,
        scope: ScopeId,
        b: &ast::PackageBody,
    ) -> Result<(), Reported> {
        self.not_nested_93(b.name.span, "package body")?;
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
            // The declaration is a mapped package, not an instance of a generic package.
            if regions.iter().any(|&r| {
                self.scopes[r].names.get(&b.name.key).is_some_and(|ds| {
                    ds.iter().any(|&d| {
                        matches!(self.decls[d].kind, DeclKind::PackageInst(i)
                            if self.decls[self.insts[i].generic].name == self.decls[d].name
                                && self.decls[self.insts[i].generic].scope == self.decls[d].scope)
                    })
                })
            }) {
                return Err(self.unsupported(
                    b.name.span,
                    "a package body of a package with a generic map",
                    "S4",
                ));
            }
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
        // Instances in a process or subprogram made before this body existed (4.9).
        let (mine, keep) = std::mem::take(&mut self.pending_inst_checks)
            .into_iter()
            .partition(|&(i, ..)| self.insts[i].generic == d);
        self.pending_inst_checks = keep;
        let mut failed = false;
        for (_, sc, sp) in mine {
            let objs = self.decls_with_instances(s);
            failed |= self.check_instance_objects(objs, sc, sp).is_err();
        }
        if failed { Err(Reported) } else { Ok(()) }
    }

    /// The package region of package body region `body`: its parent for a nested body, else
    /// the parent of its context region (unit.rs).
    pub fn body_package(&self, body: ScopeId) -> Option<ScopeId> {
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
    pub(crate) fn sequential_owner(&self, mut scope: ScopeId) -> Option<ScopeId> {
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
