//! Access types: allocators, dereference, incomplete type uses and
//! `'designated_subtype` (sem.md 2.6, 3.7, 7.1; LRM 5.4, 8.1, 9.3.7).
//!
//! An allocator fits the access types designating its type (`CandSet::Alloc`); this
//! module checks it against the designated subtype of the expected access type
//! and builds `ExprKind::New`. Dereference, explicit (`p.all`) or implicit (a
//! selected, indexed or sliced name or an attribute of an access value, 8.1
//! "appropriate for"), is `ExprKind::Deref` with the designated subtype.

use yavs_diag::{Diagnostic, Span};
use yavs_syntax::ast;

use crate::db::{Db, Reported};
use crate::diag;
use crate::expr::Expected;
use crate::hir::{Expr, ExprKind, NewInit};
use crate::ids::{ExprId, ScopeId, SubtypeId, TypeId};
use crate::scope::ScopeKind;
use crate::stat::Staticness;
use crate::types::{Constrainedness, TypeKind};

impl Db {
    /// The designated subtype of access type `t`.
    pub(crate) fn designated(&self, t: TypeId) -> Option<SubtypeId> {
        match &self.types[t].kind {
            TypeKind::Access { designated } => Some(*designated),
            TypeKind::Generic {
                class: crate::generic::GenericClass::Access { designated },
            } => Some(*designated),
            _ => None,
        }
    }

    /// `ts` plus, for each access type in it, its designated type: the types
    /// a prefix is appropriate for (8.1).
    pub(crate) fn with_designated(&self, ts: Vec<TypeId>) -> Vec<TypeId> {
        let mut out = ts.clone();
        for t in ts {
            if let Some(d) = self.designated(t) {
                let d = self.base(d);
                if !out.contains(&d) {
                    out.push(d);
                }
            }
        }
        out
    }

    /// `p.all`, or the implicit dereference of a prefix.
    pub(crate) fn deref(&mut self, p: ExprId, span: Span) -> ExprId {
        let t = self.base(self.exprs[p].subtype);
        if t == self.poison.ty || self.is_poison_expr(p) {
            return p;
        }
        let Some(subtype) = self.designated(t) else {
            self.ice(span, "dereference of a value that is not an access value");
            return self.poison.expr;
        };
        self.add_expr(Expr {
            kind: ExprKind::Deref { prefix: p },
            subtype,
            span,
            stat: Staticness::None,
            value: None,
        })
    }

    /// A prefix resolved for type `want` that is an access to `want`: its
    /// implicit dereference (8.1); else `p` itself.
    pub(crate) fn deref_to(&mut self, p: ExprId, want: TypeId) -> ExprId {
        let t = self.base(self.exprs[p].subtype);
        if t != want && self.designated(t).is_some_and(|d| self.base(d) == want) {
            let span = self.exprs[p].span;
            return self.deref(p, span);
        }
        p
    }

    /// `new S` or `new T'(e)` for access type `t` (9.3.7).
    pub(crate) fn allocator(
        &mut self,
        scope: ScopeId,
        a: &ast::Allocator,
        t: TypeId,
        span: Span,
    ) -> ExprId {
        let poison = self.poison.expr;
        let Some(designated) = self.designated(t) else {
            // Poison context (class_admits admits only access types and Error).
            return poison;
        };
        let want = self.base(designated);
        let init = match a {
            ast::Allocator::Subtype(si) => {
                let Ok(st) = self.subtype_indication(scope, si) else {
                    return poison;
                };
                if self.check_designated(t, st, si.span).is_err() {
                    return poison;
                }
                let full = self.constrainedness(st) == Constrainedness::Full;
                if !full && !matches!(self.types[self.base(st)].kind, TypeKind::Error) {
                    let name = self.subtype_name(st);
                    self.report(
                        Diagnostic::error(
                            diag::NOT_FULLY_CONSTRAINED,
                            format!(
                                "allocator needs a fully constrained subtype, `{name}` is unconstrained"
                            ),
                        )
                        .with_primary(si.span, "bounds unknown")
                        .with_help(format!(
                            "give the bounds, as in `new {name}(0 to 7)`, or an initial value, as in `new {name}'(...)`"
                        ))
                        .with_note("LRM 9.3.7: the subtype indication of an allocator must denote a fully constrained subtype"),
                    );
                    return poison;
                }
                NewInit::Subtype(st)
            }
            ast::Allocator::Qualified(e) => {
                let ast::ExprKind::Qualified { type_mark, .. } = &e.kind else {
                    self.ice(e.span, "allocator operand is not a qualified expression");
                    return poison;
                };
                let Ok(st) = self.type_mark(scope, type_mark) else {
                    return poison;
                };
                if self.check_designated(t, st, type_mark.span).is_err() {
                    return poison;
                }
                let v = self.resolve(scope, e, Expected::Type(want));
                if self.is_poison_expr(v) {
                    return poison;
                }
                NewInit::Value(v)
            }
        };
        self.add_expr(Expr {
            kind: ExprKind::New { init },
            subtype: self.types[t].first_subtype,
            span,
            stat: Staticness::None,
            value: None,
        })
    }

    /// `E1300`: the subtype of an allocator is not of the designated type.
    fn check_designated(&mut self, t: TypeId, st: SubtypeId, span: Span) -> Result<(), Reported> {
        let want = self.designated(t).map(|d| self.base(d));
        let got = self.base(st);
        if want == Some(got) || got == self.poison.ty || want == Some(self.poison.ty) {
            return Ok(());
        }
        let (gn, an) = (self.type_name(got), self.type_name(t));
        let dn = want.map_or_else(String::new, |w| self.type_name(w));
        Err(self.error(
            Diagnostic::error(
                diag::TYPE_MISMATCH,
                format!("expected an allocator of `{dn}`, found one of `{gn}`"),
            )
            .with_primary(span, format!("`{an}` designates `{dn}`"))
            .with_note("LRM 9.3.7: the type of the allocated object must be the designated type of the access type"),
        ))
    }

    /// `E1221`: a 2008 access type designating a file or protected type
    /// (5.4.1). 2019 allows both (sim1 and sim2 accept them; sim3 2026.1
    /// still rejects a file type).
    pub(crate) fn check_designated_type(
        &mut self,
        st: SubtypeId,
        span: Span,
    ) -> Result<(), Reported> {
        if self.file_rev() >= crate::LangRev::Vhdl2019 {
            return Ok(());
        }
        let t = self.base(st);
        let what = match self.types[t].kind {
            TypeKind::File { .. } => "file",
            TypeKind::Protected { .. } => "protected",
            _ => return Ok(()),
        };
        let name = self.type_name(t);
        Err(self.error(
            Diagnostic::error(
                diag::BAD_DESIGNATED,
                format!("an access type cannot designate {what} type `{name}` in VHDL-2008"),
            )
            .with_primary(span, format!("a {what} type"))
            .with_note("LRM 5.4.1 (2008): the designated type shall not be a file type or a protected type; VHDL-2019 allows both"),
        ))
    }

    /// `E1220`: a type mark naming incomplete type `t` before its full
    /// declaration. Allowed as the designated subtype of an access type
    /// (5.4.2) and, by ruling U-2, in the method profiles of a protected type
    /// declaration.
    pub(crate) fn check_incomplete_use(
        &mut self,
        scope: ScopeId,
        t: TypeId,
        span: Span,
    ) -> Result<(), Reported> {
        if !matches!(
            self.types[t].kind,
            TypeKind::Incomplete { completion: None }
        ) || self.incomplete_ok
        {
            return Ok(());
        }
        // ponytail: any type mark in a protected type declaration counts as a
        // method profile (a 2019 private variable of an incomplete type slips through).
        let sc = &self.scopes[scope];
        let in_profile = sc.kind == ScopeKind::Protected
            || (sc.kind == ScopeKind::Subprogram
                && sc
                    .parent
                    .is_some_and(|p| self.scopes[p].kind == ScopeKind::Protected));
        // Ruling U-2 (amended 2026-10-18): only units of library `std` may do this.
        let in_std = self
            .stack
            .last()
            .is_some_and(|cx| self.libs[cx.lib].name.key.as_str() == "std");
        if in_profile && in_std {
            return Ok(());
        }
        let name = self.types[t].name.text;
        let mut d = Diagnostic::error(
                diag::INCOMPLETE,
                format!("incomplete type `{name}` can only be used as an access designated type"),
            )
            .with_primary(span, "used before its full declaration")
            .with_secondary(self.decls[self.types[t].decl].span, "declared incomplete here")
            .with_note("LRM 5.4.2: before the full type declaration, an incomplete type may only be named in an access type definition");
        if in_profile {
            d = d
                .with_note("only units of library `std` (e.g. `std.reflection`) may use an incomplete type in a protected method profile; sim1, sim2 and sim3 reject it in user code")
                .with_help(format!("declare an access type to `{name}` and use that in the profile"));
        }
        Err(self.error(d))
    }

    /// `T'designated_subtype` (2019 16.2.2): the designated subtype of an
    /// access type, the element subtype of a file type.
    pub(crate) fn designated_subtype_attr(
        &mut self,
        st: SubtypeId,
        attr: ast::Ident,
    ) -> Result<SubtypeId, Reported> {
        let prefix = crate::attr::Prefix {
            form: crate::attr::PrefixForm::TypeMark,
            subtype: Some(st),
            fully_constrained: self.constrainedness(st) == Constrainedness::Full,
        };
        crate::attr::select(self, attr, &prefix, attr.span)?;
        let t = self.base(st);
        match &self.types[t].kind {
            TypeKind::File { elem } => Ok(*elem),
            TypeKind::Error => Err(Reported),
            _ => match self.designated(t) {
                Some(d) => Ok(d),
                None => Err(self.ice(attr.span, "`'designated_subtype` passed attr::select")),
            },
        }
    }
}
