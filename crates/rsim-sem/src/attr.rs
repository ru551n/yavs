//! Predefined attributes and user-defined attribute values (sem.md 6.3, LRM 16).

use rsim_syntax::Symbol;

use crate::ids::{DeclId, ExprId};

/// Predefined attributes (H5: an enum on a resolved prefix).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PredefAttr {
    // Scalar types and array objects/types.
    Left,
    Right,
    High,
    Low,
    Ascending,
    Length,
    Range,
    ReverseRange,
    Image,
    Value,
    Pos,
    Val,
    Succ,
    Pred,
    Leftof,
    Rightof,
    Base,
    Subtype,
    Element,
    // Signals.
    Event,
    Active,
    LastEvent,
    LastActive,
    LastValue,
    Driving,
    DrivingValue,
    Delayed,
    Stable,
    Quiet,
    Transaction,
    // Named entities.
    SimpleName,
    InstanceName,
    PathName,
    // 2019.
    RangeRecord,
    Reflect,
    DesignatedSubtype,
    Index,
    Converse,
}

impl PredefAttr {
    /// Look up by folded name.
    pub fn from_key(key: Symbol) -> Option<Self> {
        use PredefAttr::*;
        Some(match key.as_str() {
            "left" => Left,
            "right" => Right,
            "high" => High,
            "low" => Low,
            "ascending" => Ascending,
            "length" => Length,
            "range" => Range,
            "reverse_range" => ReverseRange,
            "image" => Image,
            "value" => Value,
            "pos" => Pos,
            "val" => Val,
            "succ" => Succ,
            "pred" => Pred,
            "leftof" => Leftof,
            "rightof" => Rightof,
            "base" => Base,
            "subtype" => Subtype,
            "element" => Element,
            "event" => Event,
            "active" => Active,
            "last_event" => LastEvent,
            "last_active" => LastActive,
            "last_value" => LastValue,
            "driving" => Driving,
            "driving_value" => DrivingValue,
            "delayed" => Delayed,
            "stable" => Stable,
            "quiet" => Quiet,
            "transaction" => Transaction,
            "simple_name" => SimpleName,
            "instance_name" => InstanceName,
            "path_name" => PathName,
            "range_record" => RangeRecord,
            "reflect" => Reflect,
            "designated_subtype" => DesignatedSubtype,
            "index" => Index,
            "converse" => Converse,
            _ => return None,
        })
    }
}

/// A user-defined attribute attached to one named entity.
pub struct AttrValue {
    pub attr: DeclId,
    pub entity: DeclId,
    pub value: ExprId,
}
