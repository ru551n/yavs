//! Types, subtypes and constraints (sem.md section 2, plan rule 1).
//!
//! There is exactly one subtype representation and every constraint in it is
//! data: range bounds are HIR expressions, never numbers. A folded value on an
//! expression is a cache, not a second representation.

use rsim_syntax::ast::Ident;

use crate::generic::GenericClass;
use crate::hir::AttrPrefix;
use crate::ids::{DeclId, ExprId, ScopeId, SubtypeId, TypeId};
use rsim_diag::Span;

pub use rsim_syntax::ast::Direction;

pub struct TypeDef {
    pub decl: DeclId,
    /// For an anonymous base type: the declared name, with `anonymous = true`.
    pub name: Ident,
    /// Diagnostics print "the base type of `T`".
    pub anonymous: bool,
    pub kind: TypeKind,
    /// `T` in `type T is ...`.
    pub first_subtype: SubtypeId,
}

pub enum TypeKind {
    UniversalInteger,
    UniversalReal,
    /// `EnumLit` decls in position order.
    Enum {
        lits: Box<[DeclId]>,
    },
    /// Ranges live in subtypes.
    Integer,
    Floating,
    Physical {
        units: Box<[PhysUnit]>,
    },
    /// Index subtypes (`natural range <>`) and element subtype.
    Array {
        indexes: Box<[SubtypeId]>,
        elem: SubtypeId,
    },
    Record {
        elems: Box<[RecordElem]>,
    },
    Access {
        designated: SubtypeId,
    },
    File {
        elem: SubtypeId,
    },
    Protected {
        decl_scope: ScopeId,
        body: Option<DeclId>,
        generics: Option<Box<[DeclId]>>,
    },
    Incomplete {
        completion: Option<TypeId>,
    },
    /// Opaque inside a generic unit (2.9).
    Generic {
        class: GenericClass,
    },
    /// Poison (sem.md 8.2): compatible with everything, never diagnosed again.
    Error,
}

/// A unit of a physical type; `scale` is in base units.
#[derive(Clone, Copy, Debug)]
pub struct PhysUnit {
    pub decl: DeclId,
    pub scale: i64,
}

pub struct RecordElem {
    pub name: Ident,
    pub subtype: SubtypeId,
}

pub struct SubtypeDef {
    pub base: TypeId,
    /// `None`: an anonymous subtype indication.
    pub name: Option<Ident>,
    /// The type mark it was derived from (diagnostics, `'base`). Consumers never
    /// walk it to find constraints: `constraint` is already merged.
    pub parent: Option<SubtypeId>,
    /// The effective constraint, the parent's merged in.
    pub constraint: Constraint,
    pub resolution: Option<Resolution>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Constraint {
    /// Scalars: the base range; composites: nothing added at this level.
    None,
    Range(RangeC),
    /// `dims: None` is `open` or `<>`.
    Array {
        dims: Option<Box<[RangeC]>>,
        elem: Box<Constraint>,
    },
    /// Only the constrained elements, by element index.
    Record {
        elems: Box<[(u32, Constraint)]>,
    },
    /// `x'subtype`, `x'element`: taken from the object at runtime.
    FromObject {
        expr: ExprId,
        elem_depth: u8,
    },
}

#[derive(Clone, Debug)]
pub enum RangeC {
    Explicit {
        left: ExprId,
        dir: Direction,
        right: ExprId,
    },
    /// `a'range`, `t'reverse_range(2)`.
    Attr {
        prefix: AttrPrefix,
        reverse: bool,
        dim: u32,
    },
}

/// Resolution indication (6.3).
#[derive(Clone, Debug)]
pub enum Resolution {
    /// `resolved std_ulogic`
    Function(DeclId),
    /// `(resolved) std_ulogic_vector` (2008)
    Element(Box<Resolution>),
    /// `(a resolved_a, b resolved_b) rec_t`
    Record(Box<[(u32, Resolution)]>),
}

/// Where an object's bounds come from (2.2), stored on the object so M2 never guesses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundsFrom {
    /// Fully constrained subtype indication.
    Subtype,
    /// Constants (6.4.2.2) and allocators `new T'(x)`.
    Initializer,
    /// Interface objects with not fully constrained subtypes.
    Actual,
    /// Object alias without subtype indication.
    Aliased,
}

/// Computed, never stored (5.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Constrainedness {
    Full,
    Partial,
    Unconstrained,
}

/// Coarse classification used by implicit operations, conversions and generic classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeClass {
    Enum,
    Integer,
    Floating,
    Physical,
    Array,
    Record,
    Access,
    File,
    Protected,
    Incomplete,
    Generic,
    Error,
}

impl TypeKind {
    pub fn class(&self) -> TypeClass {
        match self {
            TypeKind::UniversalInteger | TypeKind::Integer => TypeClass::Integer,
            TypeKind::UniversalReal | TypeKind::Floating => TypeClass::Floating,
            TypeKind::Enum { .. } => TypeClass::Enum,
            TypeKind::Physical { .. } => TypeClass::Physical,
            TypeKind::Array { .. } => TypeClass::Array,
            TypeKind::Record { .. } => TypeClass::Record,
            TypeKind::Access { .. } => TypeClass::Access,
            TypeKind::File { .. } => TypeClass::File,
            TypeKind::Protected { .. } => TypeClass::Protected,
            TypeKind::Incomplete { .. } => TypeClass::Incomplete,
            TypeKind::Generic { .. } => TypeClass::Generic,
            TypeKind::Error => TypeClass::Error,
        }
    }

    pub fn is_scalar(&self) -> bool {
        matches!(
            self.class(),
            TypeClass::Enum | TypeClass::Integer | TypeClass::Floating | TypeClass::Physical
        )
    }

    pub fn is_discrete(&self) -> bool {
        matches!(self.class(), TypeClass::Enum | TypeClass::Integer)
    }
}
