//! Generics: classes, instances and instantiation by HIR cloning (sem.md section 5).
//!
//! Instantiation clones the analyzed HIR and remaps ids; it never re-analyzes
//! the AST (5.1).

use std::cell::OnceCell;
use std::collections::HashMap;

use rsim_syntax::Symbol;

use crate::ids::{DeclId, ExprId, InstId, ScopeId, SubtypeId, TypeId};

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
    /// Cloned visible part (eager).
    pub scope: ScopeId,
    /// Cloned body (lazy, when M2 asks).
    pub body: OnceCell<ScopeId>,
    pub mangled: Symbol,
}

#[derive(Default)]
pub struct GenericMap {
    /// Formal generic type to actual subtype.
    pub types: HashMap<TypeId, SubtypeId>,
    /// Formal constant to actual expression.
    pub consts: HashMap<DeclId, ExprId>,
    /// Formal subprogram to actual subprogram.
    pub subprograms: HashMap<DeclId, DeclId>,
    /// Formal package to actual instance.
    pub packages: HashMap<DeclId, InstId>,
}
