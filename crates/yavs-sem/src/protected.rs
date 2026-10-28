//! Protected types, their bodies, methods with implicit `self`, and the 2019
//! private variables and method aliases (sem.md 6.6, LRM 5.6).
//!
//! The declarative items themselves are analyzed by `decl.rs` into the
//! regions of the protected type (`ScopeKind::Protected`) and its body
//! (`ScopeKind::ProtectedBody`, whose parent is the declaration's region, so
//! method specifications are visible in the body). This module checks which
//! items may appear there, pairs the body with its declaration, checks that
//! every method has a body, classifies members for selection, and holds the
//! object rules (no signal, constant or port of a protected type, shared
//! variables must be protected).
//!
//! Function methods follow the normal rule: pure unless declared `impure`
//! (LRM 4.2.1; sim1 agrees, and rejects a pure method reading the body's
//! variables). The purity check itself is the general one (`E1830`).

use yavs_diag::{Diagnostic, Span};
use yavs_syntax::ast::{self, Ident};

use crate::LangRev;
use crate::db::{Db, Reported};
use crate::decl::{Alias, Body, DeclKind, ObjClass};
use crate::diag;
use crate::ids::{DeclId, ExprId, ScopeId, SubtypeId, TypeId};
use crate::scope::ScopeKind;
use crate::types::TypeKind;

impl Db {
    /// Is `t` a protected type, or a composite with a protected subelement?
    pub fn type_contains_protected(&self, t: TypeId) -> bool {
        match &self.types[t].kind {
            TypeKind::Protected { .. } => true,
            TypeKind::Array { elem, .. } => self.type_contains_protected(self.base(*elem)),
            TypeKind::Record { elems } => elems
                .iter()
                .any(|e| self.type_contains_protected(self.base(e.subtype))),
            TypeKind::Incomplete {
                completion: Some(c),
            } => self.type_contains_protected(*c),
            _ => false,
        }
    }

    /// "protected type `t`", or for a composite "type `t`, which has protected subelements".
    pub(crate) fn protected_desc(&self, t: TypeId) -> String {
        if matches!(self.types[t].kind, TypeKind::Protected { .. }) {
            format!("protected type `{}`", self.type_name(t))
        } else {
            format!(
                "type `{}`, which has protected subelements",
                self.type_name(t)
            )
        }
    }

    /// Is `t` a protected type, or (2019, LCS-2016-014) a composite whose scalar
    /// subelements are all of protected types? Such a shared variable holds no
    /// unprotected state.
    pub(crate) fn all_protected(&self, t: TypeId) -> bool {
        match &self.types[t].kind {
            TypeKind::Protected { .. } => true,
            TypeKind::Array { elem, .. } => self.all_protected(self.base(*elem)),
            TypeKind::Record { elems } => elems
                .iter()
                .all(|e| self.all_protected(self.base(e.subtype))),
            _ => false,
        }
    }

    /// `E1905` when an object of class `what` (signal, constant, port, ...)
    /// would have a protected type or subelement (5.6.1, 6.4.2).
    pub fn check_not_protected(
        &mut self,
        what: &str,
        st: SubtypeId,
        span: Span,
    ) -> Result<(), Reported> {
        let t = self.base(st);
        if !self.type_contains_protected(t) {
            return Ok(());
        }
        Err(self.error(
            Diagnostic::error(
                diag::PROTECTED_MISUSE,
                format!("a {what} cannot be of {}", self.protected_desc(t)),
            )
            .with_primary(span, format!("{what} of a protected type"))
            .with_note("LRM 5.6.1: only variables, shared variables and variable parameters can hold a protected type"),
        ))
    }

    /// Parameters of a protected type must be of class variable (5.6.1).
    pub fn check_protected_param(
        &mut self,
        class: ObjClass,
        st: SubtypeId,
        span: Span,
    ) -> Result<(), Reported> {
        match class {
            ObjClass::Variable => Ok(()),
            ObjClass::Constant => self.check_not_protected("constant parameter", st, span),
            ObjClass::Signal => self.check_not_protected("signal parameter", st, span),
            ObjClass::File => self.check_not_protected("file parameter", st, span),
            ObjClass::SharedVariable => Ok(()),
        }
    }

    /// A variable of a protected type cannot be the target of an assignment
    /// (5.6.1: protected types have no predefined assignment): `E1905`.
    pub fn check_protected_assign(&mut self, target: ExprId) -> Result<(), Reported> {
        let t = self.base(self.exprs[target].subtype);
        if !self.type_contains_protected(t) {
            return Ok(());
        }
        Err(self.error(
            Diagnostic::error(
                diag::PROTECTED_MISUSE,
                format!("{} cannot be assigned", self.protected_desc(t)),
            )
            .with_primary(self.exprs[target].span, "target of a protected type")
            .with_note("LRM 5.6.1: a protected type has no predefined assignment")
            .with_help("change its state through one of its methods"),
        ))
    }

    /// A shared variable must be of a protected type (6.4.2.4), in 2019 also of a composite
    /// whose scalar subelements are all protected (LCS-2016-014): `E1904`, or
    /// warning `W1904` under `--relaxed` (R1, sem.md 8.4).
    pub fn check_shared_variable(&mut self, st: SubtypeId, span: Span) -> Result<(), Reported> {
        let t = self.base(st);
        if matches!(self.types[t].kind, TypeKind::Error)
            || matches!(self.types[t].kind, TypeKind::Protected { .. })
            // 1993 6.4.2.4: a shared variable can be of any type (protected types are new in 2000).
            || self.file_rev() < LangRev::Vhdl2008
            || (self.file_rev() >= LangRev::Vhdl2019 && self.all_protected(t))
        {
            return Ok(());
        }
        let msg = format!(
            "shared variable of type `{}`, which is not a protected type",
            self.type_name(t)
        );
        if self.relaxed() {
            self.report(
                Diagnostic::warning(diag::W_SHARED_NOT_PROTECTED, msg)
                    .with_primary(span, "accepted by `--relaxed`")
                    .with_note("processes using it run serially")
                    .with_help("--relaxed accepts a shared variable of a non-protected type (VHDL-93 style); a protected type is portable"),
            );
            return Ok(());
        }
        Err(self.error(
            Diagnostic::error(diag::SHARED_NOT_PROTECTED, msg)
                .with_primary(span, "not a protected type")
                .with_note("LRM 6.4.2.4: since VHDL-2000 a shared variable must be of a protected type")
                .with_help("wrap the value in a protected type, or pass `--relaxed` for VHDL-93 style code"),
        ))
    }

    /// A shared variable of a protected type declared in the same region as the
    /// type, before its body: the body is not elaborated yet (14.4.2, `E1230`).
    /// In a package declaration the body is in the package body, so it is always
    /// too early; a declaration in another unit is fine (the body is elaborated
    /// with its package first).
    pub fn check_shared_body_elaborated(
        &mut self,
        scope: ScopeId,
        st: SubtypeId,
        span: Span,
    ) -> Result<(), Reported> {
        let t = self.base(st);
        let TypeKind::Protected { body: None, .. } = self.types[t].kind else {
            return Ok(());
        };
        if self.generic.protected.contains(&t) {
            return Ok(());
        }
        let tdecl = self.types[t].decl;
        if self.decls[tdecl].scope != scope {
            return Ok(());
        }
        Err(self.error(
            Diagnostic::error(
                diag::MISSING_BODY,
                format!(
                    "shared variable of protected type `{}` declared before its body",
                    self.type_name(t)
                ),
            )
            .with_primary(span, "the protected body is not elaborated yet")
            .with_secondary(self.decls[tdecl].span, "protected type declared here")
            .with_note("LRM 14.4.2: elaborating the declaration of an object of a protected type needs the protected body to be elaborated first")
            .with_help("declare the shared variable after the protected body, in the package body or in another unit that uses the package"),
        ))
    }

    /// Which items a protected type declaration may hold (5.6.2): subprogram
    /// declarations and instances, use clauses, attribute specifications; 2019
    /// adds private variables and aliases.
    pub fn check_protected_decl_items(&mut self, pt: &ast::ProtectedDecl) -> Result<(), Reported> {
        let v2019 = self.file_rev() >= LangRev::Vhdl2019;
        let mut res = Ok(());
        for item in &pt.decls {
            let (ok, span, what) = match item {
                ast::Decl::Subprogram(s) => (true, s.span, ""),
                ast::Decl::SubprogramInstance(s) => (true, s.span, ""),
                ast::Decl::Use(u) => (true, u.span, ""),
                ast::Decl::AttributeSpec(a) => (true, a.span, ""),
                ast::Decl::Alias(a) if v2019 => (true, a.span, ""),
                ast::Decl::Alias(a) => {
                    res = Err(self.needs_2019(a.span, "an alias in a protected type declaration"));
                    continue;
                }
                ast::Decl::Object(o) if o.private && o.class == ast::ObjectClass::Variable => {
                    if !v2019 {
                        res = Err(self.needs_2019(o.span, "a private variable"));
                    }
                    continue;
                }
                ast::Decl::Object(o) if o.class == ast::ObjectClass::Variable => (
                    false,
                    o.span,
                    "variables of a protected type are declared in its body (or, in VHDL-2019, as `private variable`)",
                ),
                other => (
                    false,
                    decl_span(other),
                    "this declaration is not allowed here",
                ),
            };
            if !ok {
                res = Err(self.error(
                    Diagnostic::error(
                        diag::PROTECTED_ITEM,
                        "not allowed in a protected type declaration",
                    )
                    .with_primary(span, what)
                    .with_note("LRM 5.6.2: a protected type declaration holds subprogram declarations and instances, use clauses and attribute specifications; VHDL-2019 adds private variables and method aliases"),
                ));
            }
        }
        res
    }

    fn needs_2019(&mut self, span: Span, what: &str) -> Reported {
        self.error(
            Diagnostic::error(diag::NEEDS_2019, format!("{what} needs VHDL-2019"))
                .with_primary(span, "VHDL-2019 only")
                .with_note("this file is analyzed as VHDL-2008"),
        )
    }

    /// Which items a protected type body may hold (5.6.3): no signals, shared
    /// variables, components, disconnection or configuration specifications.
    pub fn check_protected_body_items(
        &mut self,
        body: &ast::ProtectedBody,
    ) -> Result<(), Reported> {
        let mut res = Ok(());
        for item in &body.decls {
            let bad = match item {
                ast::Decl::Object(o) => match o.class {
                    ast::ObjectClass::Signal => Some((o.span, "signals")),
                    ast::ObjectClass::SharedVariable => Some((o.span, "shared variables")),
                    _ => None,
                },
                ast::Decl::Component(c) => Some((c.span, "components")),
                ast::Decl::Disconnect(d) => Some((d.span, "disconnection specifications")),
                ast::Decl::ConfigSpec(c) => Some((c.span, "configuration specifications")),
                _ => None,
            };
            if let Some((span, what)) = bad {
                res = Err(self.error(
                    Diagnostic::error(
                        diag::PROTECTED_ITEM,
                        format!("{what} are not allowed in a protected type body"),
                    )
                    .with_primary(span, "not allowed here")
                    .with_note("LRM 5.6.3"),
                ));
            }
        }
        res
    }

    /// 2019 (5.6.2): an alias declared in a protected type declaration must
    /// denote a method of a protected type (typically of a private variable,
    /// forwarding it as a method of this type).
    pub fn check_protected_alias(&mut self, alias: DeclId) -> Result<(), Reported> {
        let DeclKind::Alias(a) = &self.decls[alias].kind else {
            return Err(self.ice(self.decls[alias].span, "not an alias"));
        };
        let ok = match a {
            Alias::NonObject { target } => {
                let t = self.entity(*target);
                self.is_poison(t) || self.method_of(t).is_some()
            }
            Alias::Object { .. } => false,
        };
        if ok {
            return Ok(());
        }
        Err(self.error(
            Diagnostic::error(
                diag::PROTECTED_ITEM,
                "an alias in a protected type declaration must denote a method of a protected type",
            )
            .with_primary(self.decls[alias].span, "not a method")
            .with_note("LRM 5.6.2 (VHDL-2019)"),
        ))
    }

    /// The protected type a subprogram is a method of.
    pub fn method_of(&self, d: DeclId) -> Option<TypeId> {
        let d = self.entity(d);
        self.decls[d].kind.subprogram()?;
        self.protected_of_region(self.decls[d].scope)
    }

    /// The protected type whose declaration region is `s`.
    pub fn protected_of_region(&self, s: ScopeId) -> Option<TypeId> {
        let sc = &self.scopes[s];
        if sc.kind != ScopeKind::Protected {
            return None;
        }
        match self.decls[sc.owner?].kind {
            DeclKind::Type(t) => Some(t),
            _ => None,
        }
    }

    /// A 2019 private variable: an object declared in a protected type
    /// declaration (only private variables can be).
    pub fn is_private_variable(&self, d: DeclId) -> bool {
        matches!(self.decls[d].kind, DeclKind::Object(_))
            && self.scopes[self.decls[d].scope].kind == ScopeKind::Protected
    }

    /// Is `inner` the region `outer` or nested in it?
    pub fn is_within(&self, inner: ScopeId, outer: ScopeId) -> bool {
        let mut s = Some(inner);
        while let Some(x) = s {
            if x == outer {
                return true;
            }
            s = self.scopes[x].parent;
        }
        false
    }

    /// Pair a protected type body with its declaration `ty_decl` (found by
    /// looking `name` up in `region`). The body must be in the same region,
    /// or in the package body of the package declaring the type. Sets the
    /// body region's parent to the declaration region.
    pub fn pair_protected_body(
        &mut self,
        region: ScopeId,
        ty_decl: DeclId,
        name: Ident,
        body: ScopeId,
    ) -> Result<TypeId, Reported> {
        if self.is_poison(ty_decl) {
            return Err(Reported);
        }
        let t = match self.decls[ty_decl].kind {
            DeclKind::Type(t) if matches!(self.types[t].kind, TypeKind::Protected { .. }) => t,
            _ => {
                return Err(self.error(
                    Diagnostic::error(
                        diag::NOT_A_TYPE,
                        format!("`{}` is not a protected type", name.text),
                    )
                    .with_primary(name.span, "protected body for this name")
                    .with_secondary(self.decls[ty_decl].span, "declared here"),
                ));
            }
        };
        let decl_region = self.decls[ty_decl].scope;
        let same = decl_region == region || self.body_package(region) == Some(decl_region);
        if !same {
            return Err(self.error(
                Diagnostic::error(
                    diag::PROTECTED_ITEM,
                    format!(
                        "the body of protected type `{}` must be in the region that declares it",
                        name.text
                    ),
                )
                .with_primary(name.span, "body here")
                .with_secondary(self.decls[ty_decl].span, "declared here")
                .with_note("LRM 5.6.3: in the same declarative region, or in the package body of its package"),
            ));
        }
        let TypeKind::Protected {
            decl_scope,
            body: slot,
            ..
        } = &mut self.types[t].kind
        else {
            unreachable!("checked above");
        };
        let decl_scope = *decl_scope;
        if slot.is_some() {
            return Err(self.error(
                Diagnostic::error(
                    diag::REDECLARED,
                    format!("protected type `{}` already has a body", name.text),
                )
                .with_primary(name.span, "second body"),
            ));
        }
        *slot = Some(body);
        if self.scopes[body].parent != Some(decl_scope) {
            self.scopes[body].outer = self.scopes[body].parent;
        }
        self.scopes[body].parent = Some(decl_scope);
        Ok(t)
    }

    /// At the end of a protected body: every method has a body (`E1230`).
    pub fn check_protected_complete(&mut self, t: TypeId, end: Span) -> Result<(), Reported> {
        let TypeKind::Protected { decl_scope, .. } = self.types[t].kind else {
            return Err(self.ice(end, "not a protected type"));
        };
        let missing: Vec<DeclId> = self.scopes[decl_scope]
            .order
            .iter()
            .copied()
            .filter(|&d| matches!(self.decls[d].kind.subprogram(), Some(s) if matches!(s.body, Body::Pending)))
            .collect();
        let mut res = Ok(());
        for d in missing {
            res = Err(self.error(
                Diagnostic::error(
                    diag::MISSING_BODY,
                    format!(
                        "method `{}` of protected type `{}` has no body",
                        self.decls[d].name,
                        self.type_name(t)
                    ),
                )
                .with_primary(self.decls[d].name_span(), "declared here")
                .with_secondary(end, "the protected body ends here"),
            ));
        }
        res
    }

    /// At the end of a region (or of a package body, for its package's
    /// region): every protected type declared in `scope` has a body (`E1230`).
    pub fn check_protected_bodies(&mut self, scope: ScopeId, end: Span) -> Result<(), Reported> {
        let missing: Vec<TypeId> = self.scopes[scope]
            .order
            .iter()
            .filter_map(|&d| match self.decls[d].kind {
                DeclKind::Type(t)
                    if matches!(self.types[t].kind, TypeKind::Protected { body: None, .. })
                        && !self.generic.protected.contains(&t) =>
                {
                    Some(t)
                }
                _ => None,
            })
            .collect();
        let mut res = Ok(());
        for t in missing {
            res = Err(self.error(
                Diagnostic::error(
                    diag::MISSING_BODY,
                    format!("protected type `{}` has no body", self.type_name(t)),
                )
                .with_primary(self.decls[self.types[t].decl].span, "declared here")
                .with_secondary(end, "the region ends here"),
            ));
        }
        res
    }

    /// Select `name` from an object of protected type `t` (1.5): its methods,
    /// or a private variable when `from` is inside the protected body.
    pub fn protected_member(
        &mut self,
        t: TypeId,
        name: Ident,
        from: ScopeId,
    ) -> Result<Vec<DeclId>, Reported> {
        let TypeKind::Protected {
            decl_scope, body, ..
        } = self.types[t].kind
        else {
            return Err(self.ice(name.span, "protected_member on a non-protected type"));
        };
        let found = self.scopes[decl_scope]
            .names
            .get(&name.key)
            .cloned()
            .unwrap_or_default();
        if found.is_empty()
            && let Some(b) = body
            && let Some(&d) = self.scopes[b].names.get(&name.key).and_then(|v| v.first())
            && !self.is_within(from, b)
        {
            return Err(self.error(
                Diagnostic::error(
                    diag::PRIVATE,
                    format!(
                        "`{}` is not in the visible part of protected type `{}`",
                        name.text,
                        self.type_name(t)
                    ),
                )
                .with_primary(name.span, "declared in the protected body")
                .with_secondary(self.decls[d].span, "declared here")
                .with_note("only the methods of the protected type declaration can be used from outside its body (LRM 5.6.2)"),
            ));
        }
        if found.is_empty() {
            return Err(self.error(
                Diagnostic::error(
                    diag::NOT_DECLARED,
                    format!(
                        "protected type `{}` has no method `{}`",
                        self.type_name(t),
                        name.text
                    ),
                )
                .with_primary(name.span, "not a method of this type")
                .with_secondary(self.decls[self.types[t].decl].span, "type declared here"),
            ));
        }
        let inside = body.is_some_and(|b| self.is_within(from, b));
        if !inside && let Some(&p) = found.iter().find(|&&d| self.is_private_variable(d)) {
            return Err(self.error(
                Diagnostic::error(
                    diag::PRIVATE,
                    format!("`{}` is a private variable of protected type `{}`", name.text, self.type_name(t)),
                )
                .with_primary(name.span, "private")
                .with_secondary(self.decls[p].span, "declared `private` here")
                .with_note("LRM 5.6.2 (VHDL-2019): private variables are visible only in the protected body"),
            ));
        }
        Ok(found)
    }

    /// The protected type whose methods `x.m` selects when `x` has type `t`:
    /// `t` itself, or the designated type of an access type (implicit
    /// dereference, 8.3; the 2019 mirrors are accessed this way). The flag says
    /// whether the prefix is dereferenced.
    pub(crate) fn method_target(&self, t: TypeId) -> Option<(TypeId, bool)> {
        let is_pt = |t: TypeId| matches!(self.types[t].kind, TypeKind::Protected { .. });
        match self.types[t].kind {
            TypeKind::Protected { .. } => Some((t, false)),
            TypeKind::Access { designated } if is_pt(self.base(designated)) => {
                Some((self.base(designated), true))
            }
            _ => None,
        }
    }

    /// The methods named `key` of the protected type a value of type `t`
    /// selects from, without reporting (the bottom-up pass; the top-down pass
    /// reports through [`Db::protected_member`]).
    pub(crate) fn method_decls(&self, t: TypeId, key: yavs_syntax::Symbol) -> Vec<DeclId> {
        let Some((pt, _)) = self.method_target(t) else {
            return Vec::new();
        };
        let TypeKind::Protected { decl_scope, .. } = self.types[pt].kind else {
            return Vec::new();
        };
        self.scopes[decl_scope]
            .names
            .get(&key)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&d| self.decls[self.entity(d)].kind.subprogram().is_some())
            .collect()
    }
}

fn decl_span(d: &ast::Decl) -> Span {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alias::fixture::*;
    use crate::decl::Designator;

    fn pkg() -> (Db, ScopeId, SubtypeId) {
        let mut db = db();
        let s = region(&mut db, ScopeKind::Package);
        let (_, int) = int_type(&mut db, s, "integer");
        (db, s, int)
    }

    #[test]
    fn protected_objects_and_shared_variables() {
        let (mut db, s, int) = pkg();
        let (_, pt, _) = protected_type(&mut db, s, "pt");
        let (_, rec) = record_type(&mut db, s, "r", &[("p", pt)]);
        assert!(db.check_not_protected("signal", int, sp(1)).is_ok());
        assert!(db.check_not_protected("signal", pt, sp(1)).is_err());
        assert!(db.check_not_protected("constant", rec, sp(1)).is_err());
        assert!(
            db.check_protected_param(ObjClass::Variable, pt, sp(1))
                .is_ok()
        );
        assert!(
            db.check_protected_param(ObjClass::Constant, pt, sp(1))
                .is_err()
        );
        assert!(db.check_shared_variable(pt, sp(1)).is_ok());
        assert!(db.check_shared_variable(int, sp(1)).is_err());
        assert_eq!(
            codes(&db),
            [
                diag::PROTECTED_MISUSE,
                diag::PROTECTED_MISUSE,
                diag::PROTECTED_MISUSE,
                diag::SHARED_NOT_PROTECTED
            ]
        );
        let mut opts = crate::Options::new(LangRev::Vhdl2008, crate::default_stdlib());
        opts.relaxed = true;
        let mut db = Db::new(opts);
        let s = region(&mut db, ScopeKind::Package);
        let (_, int) = int_type(&mut db, s, "integer");
        assert!(db.check_shared_variable(int, sp(1)).is_ok());
        assert_eq!(codes(&db), [diag::W_SHARED_NOT_PROTECTED]);
        assert_eq!(db.error_count(), 0);
    }

    fn first_protected(decls: &[ast::Decl]) -> (&ast::ProtectedDecl, Option<&ast::ProtectedBody>) {
        let mut d = None;
        let mut b = None;
        for x in decls {
            if let ast::Decl::Type(t) = x {
                match &t.def {
                    Some(ast::TypeDef::Protected(p)) if d.is_none() => d = Some(p),
                    Some(ast::TypeDef::ProtectedBody(p)) if b.is_none() => b = Some(p),
                    _ => {}
                }
            }
        }
        (d.unwrap(), b)
    }

    #[test]
    fn declaration_items() {
        let src = "package p is
            type pt is protected
              procedure inc;
              private variable c : integer;
              alias a is inc [];
              constant k : integer := 1;
            end protected;
          end package;";
        let decls = parse_decls(src, yavs_syntax::Std::Vhdl2019);
        let mut db = db_rev(LangRev::Vhdl2019);
        let (pt, _) = first_protected(&decls);
        assert!(db.check_protected_decl_items(pt).is_err());
        assert_eq!(codes(&db), [diag::PROTECTED_ITEM]);
        // The same in a 2008 design: alias and private variable need 2019.
        let mut db = crate::alias::fixture::db();
        let (pt, _) = first_protected(&decls);
        assert!(db.check_protected_decl_items(pt).is_err());
        assert_eq!(
            codes(&db),
            [diag::NEEDS_2019, diag::NEEDS_2019, diag::PROTECTED_ITEM]
        );
    }

    #[test]
    fn body_items() {
        let src = "package p is
            type pt is protected body
              variable n : integer;
              signal s : bit;
            end protected body;
          end package;";
        let decls = parse_decls(src, yavs_syntax::Std::Vhdl2008);
        let mut db = db();
        let b = decls
            .iter()
            .find_map(|x| match x {
                ast::Decl::Type(ast::TypeDecl {
                    def: Some(ast::TypeDef::ProtectedBody(b)),
                    ..
                }) => Some(b),
                _ => None,
            })
            .unwrap();
        assert!(db.check_protected_body_items(b).is_err());
        assert_eq!(codes(&db), [diag::PROTECTED_ITEM]);
    }

    #[test]
    fn pairing_completeness_and_members() {
        let (mut db, s, int) = pkg();
        let (t, _, ps) = protected_type(&mut db, s, "pt");
        let inc = function(&mut db, ps, "inc", &[], None);
        let get = function(&mut db, ps, "get", &[], Some(int));
        let (c, _) = object(&mut db, ps, "c", ObjClass::Variable, int);
        assert_eq!(db.method_of(inc), Some(t));
        assert!(db.is_private_variable(c));
        // Outside the body: methods selectable, the private variable is not.
        let outside = region(&mut db, ScopeKind::Process);
        assert_eq!(db.protected_member(t, ident("get"), outside), Ok(vec![get]));
        assert!(db.protected_member(t, ident("c"), outside).is_err());
        assert!(db.protected_member(t, ident("nope"), outside).is_err());
        // The region ends without a body.
        assert!(db.check_protected_bodies(s, sp(2)).is_err());
        let body = region(&mut db, ScopeKind::ProtectedBody);
        let tdecl = db.types[t].decl;
        assert_eq!(db.pair_protected_body(s, tdecl, ident("pt"), body), Ok(t));
        assert!(db.pair_protected_body(s, tdecl, ident("pt"), body).is_err());
        assert!(db.check_protected_bodies(s, sp(2)).is_ok());
        // Inside the body the private variable is selectable.
        let m = region(&mut db, ScopeKind::Subprogram);
        db.scopes[m].parent = Some(body);
        assert_eq!(db.protected_member(t, ident("c"), m), Ok(vec![c]));
        // `inc` and `get` have no body yet.
        assert!(db.check_protected_complete(t, sp(3)).is_err());
        if let DeclKind::Subprogram(sp_) = &mut db.decls[inc].kind {
            sp_.body = Body::Hir {
                scope: m,
                stmts: Box::new([]),
            };
        }
        if let DeclKind::Subprogram(sp_) = &mut db.decls[get].kind {
            sp_.body = Body::Hir {
                scope: m,
                stmts: Box::new([]),
            };
        }
        assert!(db.check_protected_complete(t, sp(3)).is_ok());
        // Not a protected type; and a body in another region.
        let idecl = db.scopes[s].names[&yavs_syntax::Symbol::intern("integer")][0];
        assert!(
            db.pair_protected_body(s, idecl, ident("integer"), body)
                .is_err()
        );
        let (t2, _, _) = protected_type(&mut db, s, "pt2");
        let other = region(&mut db, ScopeKind::Architecture);
        let t2d = db.types[t2].decl;
        assert!(
            db.pair_protected_body(other, t2d, ident("pt2"), body)
                .is_err()
        );
        assert_eq!(
            codes(&db),
            [
                diag::PRIVATE,
                diag::NOT_DECLARED,
                diag::MISSING_BODY,
                diag::REDECLARED,
                diag::MISSING_BODY,
                diag::MISSING_BODY,
                diag::NOT_A_TYPE,
                diag::PROTECTED_ITEM
            ]
        );
    }

    #[test]
    fn method_alias_rule() {
        let (mut db, s, int) = pkg();
        let (_, _, ps) = protected_type(&mut db, s, "inner");
        let inc = function(&mut db, ps, "inc", &[], None);
        let f = function(&mut db, s, "f", &[], Some(int));
        let (_, _, outer) = protected_type(&mut db, s, "outer");
        let a = declare(
            &mut db,
            outer,
            Designator::Ident(ident("inc")),
            DeclKind::Alias(Alias::NonObject { target: inc }),
        );
        let b = declare(
            &mut db,
            outer,
            Designator::Ident(ident("g")),
            DeclKind::Alias(Alias::NonObject { target: f }),
        );
        assert!(db.check_protected_alias(a).is_ok());
        assert!(db.check_protected_alias(b).is_err());
        assert_eq!(codes(&db), [diag::PROTECTED_ITEM]);
    }
}
