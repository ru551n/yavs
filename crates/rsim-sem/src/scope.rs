//! Declarative regions, visibility and lookup (sem.md 1.3, 1.4; LRM 12).
//!
//! Analysis is sequential in textual order, so inserting declarations as they
//! are analyzed gives the LRM scope rule (12.2) without position bookkeeping.

use std::collections::HashMap;

use rsim_syntax::Symbol;

use crate::ids::{DeclId, LibId, ScopeId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeKind {
    /// The session root: owns poison and universal entities, never searched by lookup.
    Root,
    /// The context clause region of a design unit (library, use and context items).
    Context,
    Entity,
    Architecture,
    Package,
    PackageBody,
    Configuration,
    Subprogram,
    Process,
    Block,
    Generate,
    Loop,
    Record,
    Protected,
    ProtectedBody,
    Instance,
    Component,
    SeqBlock,
}

pub struct Scope {
    pub kind: ScopeKind,
    /// Lexical parent; a package body's parent is its package, an architecture's its entity.
    pub parent: Option<ScopeId>,
    /// The declaration this region belongs to, for expanded names (`proc_label.v`, `pkg.x`).
    pub owner: Option<DeclId>,
    /// Folded key to declarations, in declaration order.
    pub names: HashMap<Symbol, Vec<DeclId>>,
    /// Declaration order, for cloning and for HIR consumers.
    pub order: Vec<DeclId>,
    /// Use clauses in this region, in order.
    pub uses: Vec<UseItem>,
    /// 2019: index into `order` where `private variable`s start (5.6.2).
    pub private_from: Option<usize>,
}

impl Scope {
    pub fn new(kind: ScopeKind, parent: Option<ScopeId>, owner: Option<DeclId>) -> Self {
        Scope {
            kind,
            parent,
            owner,
            names: HashMap::new(),
            order: Vec::new(),
            uses: Vec::new(),
            private_from: None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum UseItem {
    /// `use p.all`: every declaration of the region.
    All(ScopeId),
    /// `use p.x`: these declarations (all overloads of `x`).
    One(Vec<DeclId>),
    /// `use l.all`: the primary units of a library.
    Library(LibId),
}

/// The result of a lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Visible {
    None,
    One(DeclId),
    /// Subprograms and enumeration literals (and aliases of them).
    Overloads(Vec<DeclId>),
    /// Potentially visible non-overloadable declarations that cancel each other (12.4 c).
    Hidden(Conflict),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub decls: Vec<DeclId>,
}

// ---------------------------------------------------------------------------
// Declaring and looking up.

use rsim_diag::{Diagnostic, Span};
use rsim_syntax::ast::Ident;

use crate::db::Db;
use crate::decl::{Alias, DeclKind};
use crate::diag;
use crate::ids::TypeId;

/// Parameter and result type profile (4.5.3): base types only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub params: Vec<TypeId>,
    pub ret: Option<TypeId>,
}

impl Db {
    /// The entity a declaration denotes: non-object aliases are followed.
    pub fn entity(&self, mut d: DeclId) -> DeclId {
        for _ in 0..64 {
            match &self.decls[d].kind {
                DeclKind::Alias(Alias::NonObject { target }) => d = *target,
                _ => return d,
            }
        }
        d
    }

    /// Subprograms and enumeration literals, also through aliases.
    pub fn is_overloadable(&self, d: DeclId) -> bool {
        self.decls[self.entity(d)].kind.is_overloadable()
    }

    /// The profile of an overloadable entity (an enum literal is a parameterless function).
    pub fn profile(&self, d: DeclId) -> Option<Profile> {
        match &self.decls[self.entity(d)].kind {
            DeclKind::Subprogram(s) | DeclKind::GenericSubprogram(s) => Some(Profile {
                params: s.params.iter().map(|p| self.base(p.subtype)).collect(),
                ret: s.ret.map(|r| self.base(r)),
            }),
            DeclKind::EnumLit { ty, .. } => Some(Profile {
                params: Vec::new(),
                ret: Some(*ty),
            }),
            _ => None,
        }
    }

    /// LRM 12.3: same designator (the caller checks), and at most one
    /// overloadable, or both overloadable with the same profile.
    pub(crate) fn homographs(&self, a: DeclId, b: DeclId) -> bool {
        if self.entity(a) == self.entity(b) {
            return false;
        }
        match (self.is_overloadable(a), self.is_overloadable(b)) {
            (true, true) => self.profile(a) == self.profile(b),
            _ => true,
        }
    }

    /// Insert a declaration into a region. A homograph in the same region is
    /// `E1101`, except that an explicit declaration hides an implicit one
    /// (12.3). Completions (bodies, deferred constants, ...) must reuse the
    /// earlier `DeclId` instead of calling this.
    pub(crate) fn declare(&mut self, scope: ScopeId, d: DeclId) {
        let key = self.decls[d].name.key();
        let existing = self.scopes[scope]
            .names
            .get(&key)
            .cloned()
            .unwrap_or_default();
        for old in existing {
            if !self.homographs(old, d) {
                continue;
            }
            let (old_implicit, new_implicit) = (self.decls[old].implicit, self.decls[d].implicit);
            if old_implicit && !new_implicit {
                // Explicit hides implicit: the implicit one leaves the region's names.
                if let Some(v) = self.scopes[scope].names.get_mut(&key) {
                    v.retain(|x| *x != old);
                }
                continue;
            }
            if new_implicit && !old_implicit {
                // An implicit op created after an explicit homograph stays hidden.
                self.scopes[scope].order.push(d);
                return;
            }
            let name = self.decls[d].name;
            let (span, old_span) = (self.decls[d].name_span(), self.decls[old].name_span());
            self.report(
                Diagnostic::error(
                    diag::REDECLARED,
                    format!("`{name}` is declared twice in this region"),
                )
                .with_primary(span, "redeclared here")
                .with_secondary(old_span, "first declared here"),
            );
            return;
        }
        if self.entity_homograph(scope, d) {
            return;
        }
        let s = &mut self.scopes[scope];
        s.names.entry(key).or_default().push(d);
        s.order.push(d);
    }

    /// An entity and its architecture are one declarative region (12.1):
    /// a declaration in the architecture may not be a homograph of one in the
    /// entity (`E1101`). Reports and returns whether it did.
    fn entity_homograph(&mut self, scope: ScopeId, d: DeclId) -> bool {
        if self.scopes[scope].kind != ScopeKind::Architecture || self.decls[d].implicit {
            return false;
        }
        let Some(entity) = self.scopes[scope]
            .parent
            .and_then(|c| self.scopes[c].parent)
            .filter(|&e| self.scopes[e].kind == ScopeKind::Entity)
        else {
            return false;
        };
        let key = self.decls[d].name.key();
        let Some(old) = self.scopes[entity]
            .names
            .get(&key)
            .and_then(|v| v.iter().copied().find(|&o| self.homographs(o, d)))
        else {
            return false;
        };
        let what = match &self.decls[old].kind {
            DeclKind::Object(o) if o.role == crate::decl::ObjRole::Port => "a port",
            DeclKind::Object(o) if o.role == crate::decl::ObjRole::Generic => "a generic",
            _ => "a declaration",
        };
        let ename = self.scopes[entity]
            .owner
            .map(|o| self.decls[o].name.to_string())
            .unwrap_or_default();
        let name = self.decls[d].name;
        let (span, old_span) = (self.decls[d].span, self.decls[old].span);
        self.report(
            Diagnostic::error(
                diag::REDECLARED,
                format!("`{name}` is already declared as {what} of `{ename}`"),
            )
            .with_primary(span, "redeclared here")
            .with_secondary(old_span, "declared in the entity here")
            .with_note("an entity and its architectures form one declarative region (LRM 12.1)"),
        );
        true
    }

    /// Visibility (sem.md 1.4, LRM 12.3 and 12.4).
    pub fn lookup(&mut self, scope: ScopeId, key: Symbol) -> Visible {
        // 1. Direct visibility, innermost region first.
        let mut over: Vec<DeclId> = Vec::new();
        let mut single: Option<DeclId> = None;
        let mut regions = Vec::new();
        let mut cur = Some(scope);
        'walk: while let Some(s) = cur {
            regions.push(s);
            if let Some(ds) = self.scopes[s].names.get(&key) {
                for &d in ds {
                    if self.is_overloadable(d) {
                        if !over
                            .iter()
                            .any(|&o| self.homographs(o, d) || self.entity(o) == self.entity(d))
                        {
                            over.push(d);
                        }
                    } else if over.is_empty() {
                        single = Some(d);
                        break 'walk;
                    } else {
                        // Hidden by the inner overloadables; outer regions are hidden too.
                        break 'walk;
                    }
                }
                if !over.is_empty() && ds.iter().any(|&d| !self.is_overloadable(d)) {
                    break;
                }
            }
            cur = self.scopes[s].parent;
        }
        if let Some(d) = single {
            return Visible::One(d);
        }
        // Keep walking parents for use clauses even when overloadables were found.
        while let Some(s) = cur {
            if !regions.contains(&s) {
                regions.push(s);
            }
            cur = self.scopes[s].parent;
        }

        // 2. Potential visibility through use clauses of every enclosing region.
        let mut pot: Vec<DeclId> = Vec::new();
        for &s in &regions {
            let uses = self.scopes[s].uses.clone();
            for u in uses {
                let found: Vec<DeclId> = match u {
                    UseItem::All(p) => self.scopes[p].names.get(&key).cloned().unwrap_or_default(),
                    UseItem::One(ds) => ds
                        .into_iter()
                        .filter(|&d| self.decls[d].name.key() == key)
                        .collect(),
                    UseItem::Library(lib) => self.library_unit_decl(lib, key).into_iter().collect(),
                };
                for d in found {
                    if !pot.iter().any(|&p| self.entity(p) == self.entity(d)) {
                        pot.push(d);
                    }
                }
            }
        }
        // (a) a directly visible homograph hides the potentially visible declaration.
        pot.retain(|&p| {
            !over
                .iter()
                .any(|&o| self.homographs(o, p) || self.entity(o) == self.entity(p))
        });
        // (b) of two potentially visible homographs, explicit beats implicit.
        let snapshot = pot.clone();
        pot.retain(|&p| {
            !(self.decls[p].implicit
                && snapshot
                    .iter()
                    .any(|&q| q != p && !self.decls[q].implicit && self.homographs(p, q)))
        });
        let (pot_over, pot_single): (Vec<DeclId>, Vec<DeclId>) =
            pot.into_iter().partition(|&d| self.is_overloadable(d));
        // (c) non-overloadable conflicts make all of them invisible.
        if !pot_single.is_empty() {
            if pot_single.len() == 1 && pot_over.is_empty() && over.is_empty() {
                return Visible::One(pot_single[0]);
            }
            let mut decls = pot_single;
            decls.extend(pot_over);
            return Visible::Hidden(Conflict { decls });
        }
        over.extend(pot_over);
        match over.len() {
            0 => Visible::None,
            _ => Visible::Overloads(over),
        }
    }

    /// The `DeclKind::Unit` of primary unit `key` in `lib`, loading it (for `use lib.all`).
    fn library_unit_decl(&mut self, lib: LibId, key: Symbol) -> Option<DeclId> {
        let u = self
            .load_unit(lib, crate::loader::UnitKey::Primary(key), None)
            .ok()?;
        Some(self.units[u].decl)
    }

    /// Report an undeclared name once per (unit, key), with suggestions (8.3).
    pub(crate) fn undeclared(&mut self, scope: ScopeId, name: Ident) -> crate::Reported {
        let unit = self.cur().map(|c| c.unit);
        if unit.is_some_and(|u| self.tainted.contains(&u))
            || !self.undeclared.insert((unit, name.key))
        {
            return crate::Reported;
        }
        let mut d = Diagnostic::error(
            diag::NOT_DECLARED,
            format!("`{}` is not declared", name.text),
        )
        .with_primary(name.span, "not found in this scope");
        let keys = self.visible_keys(scope);
        if let Some(best) =
            crate::suggest::closest(name.key.as_str(), keys.iter().map(|k| k.as_str()))
        {
            d = d.with_help(format!("did you mean `{best}`?"));
        }
        self.error(d)
    }

    /// Report a use-clause conflict (12.4 c).
    pub(crate) fn conflict(&mut self, name: Ident, c: &Conflict) -> crate::Reported {
        let mut d = Diagnostic::error(
            diag::USE_CONFLICT,
            format!(
                "`{}` is ambiguous: several use clauses make it visible",
                name.text
            ),
        )
        .with_primary(name.span, "used here");
        for &x in &c.decls {
            let scope = self.decls[x].scope;
            let owner = self.scopes[scope]
                .owner
                .map(|o| self.decls[o].name.to_string())
                .unwrap_or_default();
            d = d.with_secondary(self.decls[x].span, format!("declared in `{owner}`"));
        }
        self.error(d.with_help("use a selected name such as `pkg.name`"))
    }

    /// Every key directly or potentially visible from `scope` (for suggestions).
    fn visible_keys(&self, scope: ScopeId) -> Vec<Symbol> {
        let mut out = Vec::new();
        let mut cur = Some(scope);
        while let Some(s) = cur {
            out.extend(self.scopes[s].names.keys().copied());
            for u in &self.scopes[s].uses {
                match u {
                    UseItem::All(p) => out.extend(self.scopes[*p].names.keys().copied()),
                    UseItem::One(ds) => out.extend(ds.iter().map(|&d| self.decls[d].name.key())),
                    UseItem::Library(_) => {}
                }
            }
            cur = self.scopes[s].parent;
        }
        out.sort();
        out.dedup();
        out
    }

    /// Span helper for diagnostics about a declaration.
    pub fn decl_span(&self, d: DeclId) -> Span {
        self.decls[d].span
    }
}
