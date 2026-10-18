//! Staticness classification and the static folder (sem.md section 4).

use crate::types::Direction;

/// 9.4. Ordered: `None < Globally < Locally`, so `min` combines operands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Staticness {
    None,
    Globally,
    Locally,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConstValue {
    Int(i64),
    Real(f64),
    /// Enumeration position.
    Enum(u32),
    /// In base units.
    Phys(i64),
    Array(ArrayConst),
    Record(Box<[ConstValue]>),
    Null,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArrayConst {
    /// One per dimension.
    pub bounds: Box<[(i64, Direction, i64)]>,
    pub elems: Elems,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Elems {
    /// 1-D arrays of enums with at most 256 literals: strings, bit and std_ulogic vectors.
    Bytes(Box<[u8]>),
    /// Run-length (rule 12): `(others => (others => '0'))` of 64K entries is one run.
    Runs(Box<[(u64, ConstValue)]>),
}

impl ConstValue {
    pub fn as_int(&self) -> Option<i64> {
        match *self {
            ConstValue::Int(v) | ConstValue::Phys(v) => Some(v),
            ConstValue::Enum(p) => Some(i64::from(p)),
            _ => None,
        }
    }
}
