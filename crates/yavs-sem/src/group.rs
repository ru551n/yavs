//! Group template declarations, group declarations (LRM 6.9, 6.10) and disconnection
//! specifications (LRM 7.4).

use yavs_diag::{Diagnostic, Span};
use yavs_syntax::ast;

use crate::db::{Db, Reported};
use crate::decl::{DeclKind, Designator, ObjClass, ObjRole, char_key};
use crate::diag;
use crate::ids::{DeclId, ScopeId};
use crate::scope::Visible;
use crate::stat::Staticness;

/// A group constituent naming a label not declared yet.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct PendingGroup {
    scope: ScopeId,
    group: DeclId,
    index: usize,
    id: yavs_syntax::ast::Ident,
}

impl Db {
    /// `group t is (class [<>], ...);` The `<>` entry stands for any number of constituents and
    /// is only allowed last (6.9).
    pub(crate) fn group_template(
        &mut self,
        scope: ScopeId,
        g: &ast::GroupTemplateDecl,
    ) -> Result<(), Reported> {
        let last = g.classes.len() - 1;
        if let Some(i) = g.classes.iter().position(|c| c.1)
            && i != last
        {
            return Err(self.error(
                Diagnostic::error(
                    diag::GROUP_TEMPLATE,
                    format!(
                        "`<>` is only allowed on the last entity class of group template `{}`",
                        g.name.text
                    ),
                )
                .with_primary(g.span, "an entry before the last ends in `<>`")
                .with_note("LRM 6.9"),
            ));
        }
        let d = self.new_decl(
            scope,
            Designator::Ident(g.name),
            DeclKind::GroupTemplate(g.classes.clone().into()),
            g.span,
        );
        self.declare(scope, d);
        Ok(())
    }

    /// `group g : t (c1, c2, ...);` Each constituent names an entity of the template's class
    /// at its position (6.10).
    pub(crate) fn group_decl(
        &mut self,
        scope: ScopeId,
        g: &ast::GroupDecl,
    ) -> Result<(), Reported> {
        let t = self.expanded_name_decl(scope, &g.template)?;
        let classes = match &self.decls[self.entity(t)].kind {
            DeclKind::GroupTemplate(c) => c.clone(),
            DeclKind::Error => return Err(Reported),
            _ => {
                return Err(self.error(
                    Diagnostic::error(
                        diag::GROUP_TEMPLATE,
                        format!("`{}` is not a group template", self.decls[t].name),
                    )
                    .with_primary(g.template.span, "expected a group template")
                    .with_note("LRM 6.10"),
                ));
            }
        };
        let (n, open) = (classes.len(), classes[classes.len() - 1].1);
        let c = g.constituents.len();
        if (!open && c != n) || (open && c + 1 < n) {
            let want = if open {
                format!("at least {}", n - 1)
            } else {
                n.to_string()
            };
            return Err(self.error(
                Diagnostic::error(
                    diag::GROUP_CONSTITUENT,
                    format!(
                        "group template `{}` takes {want} constituents, found {c}",
                        self.decls[t].name
                    ),
                )
                .with_primary(g.span, "constituent list does not fit the template")
                .with_secondary(self.decls[t].span, "template declared here")
                .with_note("LRM 6.10"),
            ));
        }
        let mut out = Vec::new();
        let mut failed = false;
        let mut waiting = Vec::new();
        for (i, name) in g.constituents.iter().enumerate() {
            let class = classes[i.min(n - 1)].0;
            // A label of the region's statement part is declared after this declarative part
            // (6.10): wait for it, `t` holds the place meanwhile.
            if class == yavs_syntax::token::Kw::Label
                && let ast::NameKind::Simple(id) = &name.kind
                && matches!(self.lookup(scope, id.key), Visible::None)
                && crate::attr::has_conc_stmts(self, scope)
            {
                waiting.push((i, *id));
                out.push(t);
                continue;
            }
            match self.constituent(scope, name, class) {
                Ok(d) => out.push(d),
                Err(_) => failed = true,
            }
        }
        if failed {
            return Err(Reported);
        }
        let d = self.new_decl(
            scope,
            Designator::Ident(g.name),
            DeclKind::Group {
                template: t,
                constituents: out.into(),
            },
            g.span,
        );
        self.declare(scope, d);
        for (index, id) in waiting {
            self.pending_groups.push(PendingGroup {
                scope,
                group: d,
                index,
                id,
            });
        }
        Ok(())
    }

    /// Label `l` was declared in `scope`: fill the group constituents waiting for it.
    pub(crate) fn label_for_groups(&mut self, scope: ScopeId, l: DeclId) {
        let key = self.decls[l].name.key();
        let waiting = std::mem::take(&mut self.pending_groups);
        for p in waiting {
            if p.scope == scope && p.id.key == key {
                if let DeclKind::Group { constituents, .. } = &mut self.decls[p.group].kind {
                    constituents[p.index] = l;
                }
            } else {
                self.pending_groups.push(p);
            }
        }
    }

    /// The end of `scope`'s statement part: a group constituent still waiting names no label.
    pub(crate) fn unresolved_group_labels(&mut self, scope: ScopeId) -> Result<(), Reported> {
        let (left, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_groups)
            .into_iter()
            .partition(|p| p.scope == scope);
        self.pending_groups = keep;
        let mut failed = false;
        for p in left {
            failed = true;
            self.error(
                Diagnostic::error(diag::NOT_DECLARED, format!("`{}` is not declared", p.id.text))
                    .with_primary(p.id.span, "no statement of this region has this label")
                    .with_note("a label constituent names a statement of the statement part of the construct whose declarative part holds the group (LRM 6.10)"),
            );
        }
        if failed { Err(Reported) } else { Ok(()) }
    }

    /// The entity a group constituent names; `class` is the template's entry.
    fn constituent(
        &mut self,
        scope: ScopeId,
        n: &ast::Name,
        class: yavs_syntax::token::Kw,
    ) -> Result<DeclId, Reported> {
        let d = match &n.kind {
            ast::NameKind::Simple(i) => match self.lookup(scope, i.key) {
                Visible::One(d) => d,
                Visible::Overloads(ds) => ds
                    .iter()
                    .copied()
                    .find(|&d| crate::attr::in_class(self, d, class))
                    .unwrap_or(ds[0]),
                Visible::None => return Err(self.undeclared(scope, *i)),
                Visible::Hidden(c) => return Err(self.conflict(*i, &c)),
            },
            ast::NameKind::Character(c) => match self.lookup(scope, char_key(*c)) {
                Visible::One(d) => d,
                Visible::Overloads(ds) => ds[0],
                _ => {
                    return Err(self.error(
                        Diagnostic::error(
                            diag::NOT_DECLARED,
                            format!("character literal '{c}' is not declared"),
                        )
                        .with_primary(n.span, "not visible here"),
                    ));
                }
            },
            ast::NameKind::Selected { .. } => self.expanded_name_decl(scope, n)?,
            _ => {
                return Err(self.error(
                    Diagnostic::error(
                        diag::GROUP_CONSTITUENT,
                        "a group constituent is the name of a named entity",
                    )
                    .with_primary(n.span, "not a simple or selected name")
                    .with_note("LRM 6.10"),
                ));
            }
        };
        if matches!(self.decls[d].kind, DeclKind::Error) {
            return Err(Reported);
        }
        if !crate::attr::in_class(self, d, class) {
            let found = match crate::attr::entity_class(self, d) {
                Some(k) => format!("a {}", k.as_str()),
                None => "not a named entity of a class".to_string(),
            };
            return Err(self.error(
                Diagnostic::error(
                    diag::GROUP_CONSTITUENT,
                    format!(
                        "`{}` is {found}, the template asks for a {} here",
                        self.decls[d].name,
                        class.as_str()
                    ),
                )
                .with_primary(n.span, format!("expected a {}", class.as_str()))
                .with_note("LRM 6.10"),
            ));
        }
        Ok(d)
    }

    /// `disconnect s1, s2 : t after 5 ns;` (7.4). The time goes to `Db::disconnect_times`;
    /// elaboration gives it to the guarded signal.
    pub(crate) fn disconnect_spec(
        &mut self,
        scope: ScopeId,
        x: &ast::DisconnectSpec,
    ) -> Result<(), Reported> {
        let ty = self.type_mark(scope, &x.type_mark)?;
        let Some(time) = self.well_known.time else {
            return Err(self.ice(x.span, "std.standard.time is not loaded"));
        };
        let time_st = self.types[time].first_subtype;
        let after = self.expr_in_subtype(scope, &x.after, time_st);
        if self.is_poison_expr(after) {
            return Err(Reported);
        }
        if self.exprs[after].stat < Staticness::Globally {
            return Err(self.error(
                Diagnostic::error(
                    diag::NOT_STATIC,
                    "the time of a disconnection specification must be static",
                )
                .with_primary(self.exprs[after].span, "not static")
                .with_note("LRM 7.4"),
            ));
        }
        let base = self.base(ty);
        let mut targets: Vec<(DeclId, Span)> = Vec::new();
        let mut failed = false;
        match &x.signals {
            ast::SignalList::Names(names) => {
                for n in names {
                    match self.guarded_signal(scope, n, base) {
                        Ok(d) => targets.push((d, n.span)),
                        Err(_) => failed = true,
                    }
                }
            }
            ast::SignalList::Others | ast::SignalList::All => {
                let others = matches!(x.signals, ast::SignalList::Others);
                for d in self.scopes[scope].order.clone() {
                    if self.decls[d].implicit
                        || (others && self.disconnect_times.contains_key(&d))
                        || !self.is_guarded_of(d, base)
                    {
                        continue;
                    }
                    targets.push((d, x.span));
                }
            }
        }
        for (d, sp) in targets {
            // A second specification keeps the first one's time.
            if let Some(first) = self.disconnect_times.insert(d, after) {
                self.disconnect_times.insert(d, first);
                self.report(
                    Diagnostic::error(
                        diag::DISCONNECT_TWICE,
                        format!(
                            "guarded signal `{}` already has a disconnection specification",
                            self.decls[d].name
                        ),
                    )
                    .with_primary(sp, "second specification")
                    .with_note("LRM 7.4"),
                );
                failed = true;
            }
        }
        if failed { Err(Reported) } else { Ok(()) }
    }

    /// A guarded signal of the declarative part `scope`, of base type `base`.
    fn is_guarded_of(&self, d: DeclId, base: crate::ids::TypeId) -> bool {
        matches!(&self.decls[d].kind, DeclKind::Object(o)
            if o.class == ObjClass::Signal
                && matches!(o.role, ObjRole::Plain | ObjRole::Port)
                && o.signal_kind.is_some()
                && self.base(o.subtype) == base)
    }

    /// The guarded signal a name of a signal list denotes.
    fn guarded_signal(
        &mut self,
        scope: ScopeId,
        n: &ast::Name,
        base: crate::ids::TypeId,
    ) -> Result<DeclId, Reported> {
        let ast::NameKind::Simple(i) = &n.kind else {
            return Err(self.error(
                Diagnostic::error(
                    diag::DISCONNECT_SPEC,
                    "a disconnection specification names guarded signals by simple name",
                )
                .with_primary(n.span, "not a simple name")
                .with_note("LRM 7.4"),
            ));
        };
        let d = match self.scopes[scope].names.get(&i.key).and_then(|v| v.first()) {
            Some(&d) => d,
            None => {
                return Err(match self.lookup(scope, i.key) {
                    Visible::None => self.undeclared(scope, *i),
                    _ => self.error(
                        Diagnostic::error(
                            diag::DISCONNECT_SPEC,
                            format!("`{}` is not declared in this declarative part", i.text),
                        )
                        .with_primary(i.span, "declared elsewhere")
                        .with_note("a disconnection specification names signals of its own declarative part (LRM 7.4)"),
                    ),
                });
            }
        };
        let (msg, ok) = match &self.decls[d].kind {
            DeclKind::Error => return Err(Reported),
            // A port of the region is a signal of its declarative part (the entity's or the
            // block's) and may be disconnected too (#644).
            DeclKind::Object(o)
                if o.class == ObjClass::Signal
                    && matches!(o.role, ObjRole::Plain | ObjRole::Port) =>
            {
                if o.signal_kind.is_none() {
                    ("is not a guarded signal", false)
                } else if self.base(o.subtype) != base {
                    ("has another type than the type mark", false)
                } else {
                    ("", true)
                }
            }
            _ => ("is not a signal declared in this declarative part", false),
        };
        if ok {
            return Ok(d);
        }
        Err(self.error(
            Diagnostic::error(diag::DISCONNECT_SPEC, format!("`{}` {msg}", i.text))
                .with_primary(i.span, "named here")
                .with_secondary(self.decls[d].span, "declared here")
                .with_note("LRM 7.4"),
        ))
    }
}
