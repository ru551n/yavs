//! Expression typing and overload resolution (sem.md section 3).
//!
//! Two passes over each complete context (LRM 12.5): bottom-up `cands` (the
//! possible result types of an AST expression) and top-down `resolve` (pick
//! exactly one interpretation that fits the expected type, build HIR).

pub mod aggregate;
pub mod cands;
pub mod literal;
pub mod names;
pub mod resolve;

use crate::ids::{DeclId, TypeId};

/// What the context requires of an expression (3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expected {
    /// Assignment target, formal, index type, ...
    Type(TypeId),
    /// BOOLEAN, else the implicit `??` (3.6).
    Condition,
    /// Must resolve without context (case expression, conversion operand, ...).
    Any,
    /// Range bounds without a type mark: both universal gives INTEGER.
    Discrete,
    /// The value assigned to an aggregate target: any composite type (10.5.2.1, 10.6.2.1).
    Composite,
}

/// Literal forms typed only by context.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ClassMask(pub u8);

impl ClassMask {
    pub const ANY_INTEGER: ClassMask = ClassMask(1);
    pub const ANY_REAL: ClassMask = ClassMask(2);
    /// `null`, allocators.
    pub const ANY_ACCESS: ClassMask = ClassMask(4);
    /// Aggregates.
    pub const ANY_COMPOSITE: ClassMask = ClassMask(8);
    /// 1-D array of a character-literal enum: string and bit string literals.
    pub const STRING_LIKE: ClassMask = ClassMask(16);
    pub const ANY_ARRAY: ClassMask = ClassMask(32);
    pub const ANY_RECORD: ClassMask = ClassMask(64);

    pub fn contains(self, other: ClassMask) -> bool {
        self.0 & other.0 == other.0
    }
}

/// How an interpretation produces its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    Object,
    Call(DeclId),
    /// Method call `x.m` of a protected object (or an access to one) of type `.1`.
    Method(DeclId, TypeId),
    EnumLit(DeclId),
    /// Attribute of a type mark.
    Attr,
    /// Attribute of an object or value whose type is the payload.
    AttrOf(TypeId),
    /// Indexed name of a prefix of this array type.
    Index(TypeId),
    /// Slice of a prefix of this array type.
    Slice(TypeId),
    /// Element `.1` of a prefix of record type `.0`.
    Field(TypeId, u32),
    /// `.all` of a prefix of this access type.
    Deref(TypeId),
    Literal,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interp {
    pub ty: TypeId,
    pub via: Via,
    /// A convertible universal operand (9.3.6): a numeric literal, an attribute
    /// of universal type, or the division of two values of one physical type.
    pub convertible: bool,
    /// Implicit conversions this interpretation needs in its subtree (Q1:
    /// interpretations without conversions win).
    pub convs: u32,
}

impl Interp {
    pub fn new(ty: TypeId, via: Via) -> Self {
        Interp {
            ty,
            via,
            convertible: false,
            convs: 0,
        }
    }
}

/// Interpretations of one expression: mostly one or two, so no heap.
pub type Interps = smallvec::SmallVec<[Interp; 2]>;

/// The bottom-up result of `cands` (3.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CandSet {
    Types(Interps),
    Class(ClassMask),
    /// An allocator: any access type designating this (base) type (9.3.7).
    Alloc(TypeId),
}
