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
