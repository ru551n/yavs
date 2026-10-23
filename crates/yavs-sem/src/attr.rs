//! Predefined attributes and user-defined attributes (sem.md 6.3, LRM 16, 6.7, 7.2).
//!
//! - [`PredefAttr`] and the table [`TABLE`]: for each predefined attribute the
//!   prefixes it applies to (with the revision that allows each form), its
//!   parameter, result and staticness rule. Name interpretation (`names.rs`)
//!   describes the prefix as a [`Prefix`] and calls [`select`].
//! - [`result_subtype`], [`staticness`]: the typing of the chosen form.
//! - [`implicit_signal`]: `'delayed`, `'stable`, `'quiet`, `'transaction`.
//! - [`declare_attribute`], [`specify`], [`user_value`]: user-defined
//!   attributes (`attribute a : t;`, `attribute a of x : signal is v;`).
//!
//! Revision rules follow docs/design/lrm-checks.md L12 to L14: there is no
//! `'RANGE_RECORD`; 2019 has `R'RECORD` and `R'VALUE` for a range attribute
//! name `R`, `O'IMAGE` on scalar objects and `A'INDEX(n)`.

use yavs_diag::{Diagnostic, Span};
use yavs_syntax::Symbol;
use yavs_syntax::ast::{self, Ident};
use yavs_syntax::token::Kw;

use crate::LangRev;
use crate::db::{Db, Reported};
use crate::decl::{
    Alias, Body, Decl, DeclKind, Designator, ObjClass, ObjRole, Object, SubprogramKind,
};
use crate::diag;
use crate::hir::{AttrPrefix, Expr, ExprKind};
use crate::ids::{AttrValueId, DeclId, ExprId, ScopeId, SubtypeId, TypeId};
use crate::loader::UnitKind;
use crate::scope::ScopeKind;
use crate::stat::Staticness;
use crate::types::{BoundsFrom, TypeKind};

/// Predefined attributes (H5: an enum on a resolved prefix).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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
    /// `R'RECORD`, `R` a range attribute name: the `<T>_RANGE_RECORD` type.
    Record,
    Reflect,
    DesignatedSubtype,
    Index,
    Converse,
}

impl PredefAttr {
    /// Whether the attribute reads the value of its signal prefix (LRM 1993 4.3.2: an `out` port
    /// is not a legal prefix of these). The others (`'range`, `'length`, `'driving_value`,
    /// `'simple_name`, ...) do not.
    pub fn reads_value(self) -> bool {
        use PredefAttr::*;
        matches!(
            self,
            Event
                | Active
                | LastEvent
                | LastActive
                | LastValue
                | Delayed
                | Stable
                | Quiet
                | Transaction
        )
    }

    /// Look up by folded name.
    pub fn from_key(key: Symbol) -> Option<Self> {
        TABLE
            .iter()
            .find(|i| i.name == key.as_str())
            .map(|i| i.attr)
    }

    pub fn info(self) -> &'static AttrInfo {
        TABLE
            .iter()
            .find(|i| i.attr == self)
            .expect("every predefined attribute has a table entry")
    }

    pub fn name(self) -> &'static str {
        self.info().name
    }
}

// ---------------------------------------------------------------------------
// The table

/// What the prefix of an attribute name denotes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefixForm {
    /// A type mark (a type or subtype name, `T'base`, `x'subtype`, ...).
    TypeMark,
    /// An object name; `signal` for signals (ports and implicit signals included).
    Object { signal: bool },
    /// A function call or other value that is not an object name.
    Value,
    /// A range attribute name (`T'range`, `a'reverse_range(2)`), 2019 `'RECORD`/`'VALUE`.
    Range,
    /// A named entity that is none of the above: label, design unit,
    /// subprogram, literal, unit, component, group.
    Entity,
    /// A mode view (2019).
    View,
}

/// The prefix as name interpretation found it.
#[derive(Clone, Copy, Debug)]
pub struct Prefix {
    pub form: PrefixForm,
    /// The prefix subtype for type marks, objects, values and ranges.
    pub subtype: Option<SubtypeId>,
    /// The prefix subtype is fully constrained (5.1).
    pub fully_constrained: bool,
}

/// The type a prefix form must have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Any,
    Scalar,
    Discrete,
    DiscreteOrPhysical,
    Array,
    /// A fully constrained array (type mark prefixes of `'range`, `'length`, ...).
    ConstrainedArray,
    /// An access or file type (`'designated_subtype`).
    AccessOrFile,
    /// A type whose values `'image`/`'value` can show (2019: scalars and
    /// composites of them, LCS-2016-018).
    Representable,
    /// Fully constrained (`T'reflect`).
    Constrained,
}

/// One accepted prefix of an attribute.
#[derive(Clone, Copy, Debug)]
pub struct PrefixRule {
    pub form: Form,
    pub class: Class,
    pub since: LangRev,
}

/// [`PrefixForm`] without the signal detail, plus `Signal` for signals only
/// and `NamedEntity` for anything that has a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    TypeMark,
    Object,
    Signal,
    Range,
    View,
    NamedEntity,
}

/// The parameter an attribute takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Param {
    None,
    /// Optional locally static dimension, of any integer type, default 1.
    Dim,
    /// A value of the prefix's base type (`T'image(x)`, `T'succ(x)`).
    PrefixValue,
    /// A STRING (`T'value(s)`).
    String,
    /// A value of any integer type (`T'val(n)`).
    AnyInteger,
    /// Optional globally static TIME, default `0 ns` (`'delayed`, `'stable`, `'quiet`).
    Time,
}

/// The result of an attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Result_ {
    /// A value of the prefix's base type (`T'left`, `T'succ(x)`).
    PrefixBase,
    /// A value of the prefix's subtype (`s'last_value`).
    PrefixSubtype,
    /// A value of the index subtype of dimension `Dim` (`a'left(2)`).
    IndexValue,
    UniversalInteger,
    Boolean,
    String,
    Time,
    Bit,
    /// A range (`'range`, `'reverse_range`): not a value.
    Range,
    /// A subtype (`'base`, `'subtype`, `'element`, `'index`, ...): only in a type mark.
    Subtype,
    /// The `<T>_RANGE_RECORD` value (2019 `R'value`).
    RangeRecord,
    /// `std.reflection.VALUE_MIRROR` or `SUBTYPE_MIRROR` (2019).
    Mirror,
    /// A mode view (2019 `'converse`).
    View,
}

/// What kind of entity the attribute name denotes (LRM 16.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Value,
    Function,
    /// An implicit signal (`'delayed`, `'stable`, `'quiet`, `'transaction`).
    Signal,
    Range,
    Type,
    View,
}

/// How the staticness of the attribute name follows from its parts (9.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stat {
    /// That of the prefix subtype (9.4.2 e, 9.4.3).
    OfPrefix,
    /// The prefix subtype and the actual parameter.
    OfPrefixAndArgs,
    Locally,
    Globally,
    None,
}

#[derive(Debug)]
pub struct AttrInfo {
    pub attr: PredefAttr,
    /// Folded name.
    pub name: &'static str,
    pub kind: Kind,
    pub prefixes: &'static [PrefixRule],
    pub param: Param,
    pub result: Result_,
    pub stat: Stat,
    /// The attribute is not available before this revision.
    pub since: LangRev,
}

const V93: LangRev = LangRev::Vhdl1993;
const V08: LangRev = LangRev::Vhdl2008;
const V19: LangRev = LangRev::Vhdl2019;

const fn p(form: Form, class: Class, since: LangRev) -> PrefixRule {
    PrefixRule { form, class, since }
}

use Class as C;
use Form as F;

/// `'left` and friends: scalar types; arrays (with a dimension); 2019 scalar
/// objects and range attribute names (LRM 2019 16.2.2, 16.2.6).
const BOUNDS: &[PrefixRule] = &[
    p(F::TypeMark, C::Scalar, V93),
    p(F::TypeMark, C::ConstrainedArray, V93),
    p(F::Object, C::Array, V93),
    p(F::Object, C::Scalar, V19),
    p(F::Range, C::Scalar, V19),
];
/// `'length`: arrays; 2019 discrete types (lrm-checks, sim1 2019).
const LENGTH: &[PrefixRule] = &[
    p(F::TypeMark, C::ConstrainedArray, V93),
    p(F::Object, C::Array, V93),
    p(F::TypeMark, C::Discrete, V19),
];
/// `'range`, `'reverse_range`: arrays; scalar type marks are accepted in
/// both revisions, as sim1 does (the 2008 text is silent, user policy).
const RANGES: &[PrefixRule] = &[
    p(F::TypeMark, C::ConstrainedArray, V93),
    p(F::Object, C::Array, V93),
    p(F::TypeMark, C::Scalar, V93),
];
const IMAGE: &[PrefixRule] = &[
    p(F::TypeMark, C::Scalar, V93),
    p(F::TypeMark, C::Representable, V19),
    p(F::Object, C::Representable, V19),
];
const VALUE: &[PrefixRule] = &[
    p(F::TypeMark, C::Scalar, V93),
    p(F::TypeMark, C::Representable, V19),
    p(F::Range, C::Scalar, V19),
];
/// `'pos` and friends: discrete or physical types; 2019 objects (LCS-2016-008).
const POS: &[PrefixRule] = &[
    p(F::TypeMark, C::DiscreteOrPhysical, V93),
    p(F::Object, C::DiscreteOrPhysical, V19),
];
const ANY_TYPE: &[PrefixRule] = &[p(F::TypeMark, C::Any, V93)];
const ANY_OBJECT: &[PrefixRule] = &[p(F::Object, C::Any, V93)];
const ARRAY_TM_OR_OBJ: &[PrefixRule] =
    &[p(F::TypeMark, C::Array, V93), p(F::Object, C::Array, V93)];
const SIGNAL: &[PrefixRule] = &[p(F::Signal, C::Any, V93)];
const NAMED: &[PrefixRule] = &[p(F::NamedEntity, C::Any, V93)];
const DESIGNATED: &[PrefixRule] = &[
    p(F::TypeMark, C::AccessOrFile, V19),
    p(F::Object, C::AccessOrFile, V19),
];
const INDEX: &[PrefixRule] = &[p(F::TypeMark, C::Array, V19), p(F::Object, C::Array, V19)];
const RECORD: &[PrefixRule] = &[p(F::Range, C::Scalar, V19)];
const REFLECT: &[PrefixRule] = &[
    p(F::TypeMark, C::Constrained, V19),
    p(F::Object, C::Any, V19),
];
const VIEW: &[PrefixRule] = &[p(F::View, C::Any, V19)];

#[allow(clippy::too_many_arguments)]
const fn row(
    attr: PredefAttr,
    name: &'static str,
    kind: Kind,
    prefixes: &'static [PrefixRule],
    param: Param,
    result: Result_,
    stat: Stat,
    since: LangRev,
) -> AttrInfo {
    AttrInfo {
        attr,
        name,
        kind,
        prefixes,
        param,
        result,
        stat,
        since,
    }
}

use PredefAttr as A;
use Result_ as R;

/// Every predefined attribute (LRM 2008 16.2, 2019 16.2).
pub static TABLE: &[AttrInfo] = &[
    // 16.2.2 types and subtypes, 16.2.3 arrays.
    row(
        A::Base,
        "base",
        Kind::Type,
        ANY_TYPE,
        Param::None,
        R::Subtype,
        Stat::OfPrefix,
        V93,
    ),
    row(
        A::Left,
        "left",
        Kind::Value,
        BOUNDS,
        Param::Dim,
        R::IndexValue,
        Stat::OfPrefix,
        V93,
    ),
    row(
        A::Right,
        "right",
        Kind::Value,
        BOUNDS,
        Param::Dim,
        R::IndexValue,
        Stat::OfPrefix,
        V93,
    ),
    row(
        A::High,
        "high",
        Kind::Value,
        BOUNDS,
        Param::Dim,
        R::IndexValue,
        Stat::OfPrefix,
        V93,
    ),
    row(
        A::Low,
        "low",
        Kind::Value,
        BOUNDS,
        Param::Dim,
        R::IndexValue,
        Stat::OfPrefix,
        V93,
    ),
    row(
        A::Ascending,
        "ascending",
        Kind::Value,
        BOUNDS,
        Param::Dim,
        R::Boolean,
        Stat::OfPrefix,
        V93,
    ),
    row(
        A::Length,
        "length",
        Kind::Value,
        LENGTH,
        Param::Dim,
        R::UniversalInteger,
        Stat::OfPrefix,
        V93,
    ),
    row(
        A::Range,
        "range",
        Kind::Range,
        RANGES,
        Param::Dim,
        R::Range,
        Stat::OfPrefix,
        V93,
    ),
    row(
        A::ReverseRange,
        "reverse_range",
        Kind::Range,
        RANGES,
        Param::Dim,
        R::Range,
        Stat::OfPrefix,
        V93,
    ),
    row(
        A::Image,
        "image",
        Kind::Function,
        IMAGE,
        Param::PrefixValue,
        R::String,
        Stat::OfPrefixAndArgs,
        V93,
    ),
    row(
        A::Value,
        "value",
        Kind::Function,
        VALUE,
        Param::String,
        R::PrefixBase,
        Stat::OfPrefixAndArgs,
        V93,
    ),
    row(
        A::Pos,
        "pos",
        Kind::Function,
        POS,
        Param::PrefixValue,
        R::UniversalInteger,
        Stat::OfPrefixAndArgs,
        V93,
    ),
    row(
        A::Val,
        "val",
        Kind::Function,
        POS,
        Param::AnyInteger,
        R::PrefixBase,
        Stat::OfPrefixAndArgs,
        V93,
    ),
    row(
        A::Succ,
        "succ",
        Kind::Function,
        POS,
        Param::PrefixValue,
        R::PrefixBase,
        Stat::OfPrefixAndArgs,
        V93,
    ),
    row(
        A::Pred,
        "pred",
        Kind::Function,
        POS,
        Param::PrefixValue,
        R::PrefixBase,
        Stat::OfPrefixAndArgs,
        V93,
    ),
    row(
        A::Leftof,
        "leftof",
        Kind::Function,
        POS,
        Param::PrefixValue,
        R::PrefixBase,
        Stat::OfPrefixAndArgs,
        V93,
    ),
    row(
        A::Rightof,
        "rightof",
        Kind::Function,
        POS,
        Param::PrefixValue,
        R::PrefixBase,
        Stat::OfPrefixAndArgs,
        V93,
    ),
    row(
        A::Subtype,
        "subtype",
        Kind::Type,
        ANY_OBJECT,
        Param::None,
        R::Subtype,
        Stat::OfPrefix,
        V08,
    ),
    row(
        A::Element,
        "element",
        Kind::Type,
        ARRAY_TM_OR_OBJ,
        Param::None,
        R::Subtype,
        Stat::OfPrefix,
        V08,
    ),
    // 16.2.4 signals.
    row(
        A::Delayed,
        "delayed",
        Kind::Signal,
        SIGNAL,
        Param::Time,
        R::PrefixSubtype,
        Stat::None,
        V93,
    ),
    row(
        A::Stable,
        "stable",
        Kind::Signal,
        SIGNAL,
        Param::Time,
        R::Boolean,
        Stat::None,
        V93,
    ),
    row(
        A::Quiet,
        "quiet",
        Kind::Signal,
        SIGNAL,
        Param::Time,
        R::Boolean,
        Stat::None,
        V93,
    ),
    row(
        A::Transaction,
        "transaction",
        Kind::Signal,
        SIGNAL,
        Param::None,
        R::Bit,
        Stat::None,
        V93,
    ),
    row(
        A::Event,
        "event",
        Kind::Function,
        SIGNAL,
        Param::None,
        R::Boolean,
        Stat::None,
        V93,
    ),
    row(
        A::Active,
        "active",
        Kind::Function,
        SIGNAL,
        Param::None,
        R::Boolean,
        Stat::None,
        V93,
    ),
    row(
        A::LastEvent,
        "last_event",
        Kind::Function,
        SIGNAL,
        Param::None,
        R::Time,
        Stat::None,
        V93,
    ),
    row(
        A::LastActive,
        "last_active",
        Kind::Function,
        SIGNAL,
        Param::None,
        R::Time,
        Stat::None,
        V93,
    ),
    row(
        A::LastValue,
        "last_value",
        Kind::Function,
        SIGNAL,
        Param::None,
        R::PrefixSubtype,
        Stat::None,
        V93,
    ),
    row(
        A::Driving,
        "driving",
        Kind::Function,
        SIGNAL,
        Param::None,
        R::Boolean,
        Stat::None,
        V93,
    ),
    row(
        A::DrivingValue,
        "driving_value",
        Kind::Function,
        SIGNAL,
        Param::None,
        R::PrefixSubtype,
        Stat::None,
        V93,
    ),
    // 16.2.5 named entities.
    row(
        A::SimpleName,
        "simple_name",
        Kind::Value,
        NAMED,
        Param::None,
        R::String,
        Stat::Locally,
        V93,
    ),
    row(
        A::InstanceName,
        "instance_name",
        Kind::Value,
        NAMED,
        Param::None,
        R::String,
        Stat::Globally,
        V93,
    ),
    row(
        A::PathName,
        "path_name",
        Kind::Value,
        NAMED,
        Param::None,
        R::String,
        Stat::Globally,
        V93,
    ),
    // 2019.
    row(
        A::Record,
        "record",
        Kind::Type,
        RECORD,
        Param::None,
        R::Subtype,
        Stat::OfPrefix,
        V19,
    ),
    row(
        A::DesignatedSubtype,
        "designated_subtype",
        Kind::Type,
        DESIGNATED,
        Param::None,
        R::Subtype,
        Stat::OfPrefix,
        V19,
    ),
    row(
        A::Index,
        "index",
        Kind::Type,
        INDEX,
        Param::Dim,
        R::Subtype,
        Stat::OfPrefix,
        V19,
    ),
    row(
        A::Reflect,
        "reflect",
        Kind::Function,
        REFLECT,
        Param::None,
        R::Mirror,
        Stat::None,
        V19,
    ),
    row(
        A::Converse,
        "converse",
        Kind::View,
        VIEW,
        Param::None,
        R::View,
        Stat::Locally,
        V19,
    ),
];

// ---------------------------------------------------------------------------
// Selecting a form

fn class_holds(db: &Db, class: Class, prefix: &Prefix) -> bool {
    let Some(st) = prefix.subtype else {
        return class == Class::Any;
    };
    let kind = &db.types[db.base(st)].kind;
    if matches!(kind, TypeKind::Error) {
        return true;
    }
    // A generic type has the attributes of its class (2019 6.5.3, generic.rs).
    if let TypeKind::Generic { class: g } = kind {
        use crate::generic::GenericClass as G;
        let scalar = !matches!(g, G::Private | G::Access { .. } | G::File { .. });
        let discrete = matches!(g, G::Discrete | G::Integer);
        return match class {
            Class::Any => true,
            Class::Scalar | Class::Representable => scalar,
            Class::Discrete => discrete,
            Class::DiscreteOrPhysical => discrete || matches!(g, G::Physical),
            Class::Array | Class::ConstrainedArray => false,
            Class::AccessOrFile => matches!(g, G::Access { .. } | G::File { .. }),
            Class::Constrained => prefix.fully_constrained,
        };
    }
    match class {
        Class::Any => true,
        Class::Scalar => kind.is_scalar(),
        Class::Discrete => kind.is_discrete(),
        Class::DiscreteOrPhysical => {
            kind.is_discrete() || matches!(kind, TypeKind::Physical { .. })
        }
        Class::Array => matches!(kind, TypeKind::Array { .. }),
        Class::ConstrainedArray => {
            matches!(kind, TypeKind::Array { .. }) && prefix.fully_constrained
        }
        Class::AccessOrFile => matches!(kind, TypeKind::Access { .. } | TypeKind::File { .. }),
        Class::Representable => representable(db, db.base(st), 0),
        Class::Constrained => prefix.fully_constrained,
    }
}

/// Scalars and arrays/records of representable types (2019 16.2.2).
fn representable(db: &Db, ty: TypeId, depth: u32) -> bool {
    if depth > 32 {
        return false;
    }
    match &db.types[ty].kind {
        k if k.is_scalar() => true,
        TypeKind::Array { elem, .. } => representable(db, db.base(*elem), depth + 1),
        TypeKind::Record { elems } => elems
            .iter()
            .all(|e| representable(db, db.base(e.subtype), depth + 1)),
        TypeKind::Error => true,
        _ => false,
    }
}

fn form_holds(form: Form, prefix: PrefixForm) -> bool {
    match form {
        Form::TypeMark => prefix == PrefixForm::TypeMark,
        Form::Object => matches!(prefix, PrefixForm::Object { .. }),
        Form::Signal => prefix == PrefixForm::Object { signal: true },
        Form::Range => prefix == PrefixForm::Range,
        Form::View => prefix == PrefixForm::View,
        Form::NamedEntity => prefix != PrefixForm::Value && prefix != PrefixForm::Range,
    }
}

fn describe(db: &Db, prefix: &Prefix) -> String {
    let ty = prefix
        .subtype
        .map(|st| format!("`{}`", db.subtype_name(st)));
    let form = match prefix.form {
        PrefixForm::TypeMark => "a type mark",
        PrefixForm::Object { signal: true } => "a signal",
        PrefixForm::Object { signal: false } => "an object",
        PrefixForm::Value => "a value",
        PrefixForm::Range => "a range",
        PrefixForm::Entity => "a named entity",
        PrefixForm::View => "a mode view",
    };
    match ty {
        Some(t) => format!("{form} of type {t}"),
        None => form.to_string(),
    }
}

/// Choose the form of predefined attribute `key` for `prefix`. `Ok(None)`:
/// `key` is not a predefined attribute (try a user-defined one). The file's
/// revision gates 2019 forms (`E1007`).
pub fn select(
    db: &mut Db,
    attr: Ident,
    prefix: &Prefix,
    span: Span,
) -> Result<Option<&'static AttrInfo>, Reported> {
    let Some(a) = PredefAttr::from_key(attr.key) else {
        return Ok(None);
    };
    let info = a.info();
    let rev = db.file_rev();
    let fits: Vec<&PrefixRule> = info
        .prefixes
        .iter()
        .filter(|r| form_holds(r.form, prefix.form) && class_holds(db, r.class, prefix))
        .collect();
    if fits.iter().any(|r| r.since <= rev) {
        return Ok(Some(info));
    }
    let what = describe(db, prefix);
    if !fits.is_empty() {
        return Err(db.error(
            Diagnostic::error(
                diag::NOT_IN_REVISION,
                format!(
                    "`'{}` of {what} is not available in {}",
                    attr.text,
                    Db::rev_name(rev)
                ),
            )
            .with_primary(
                span,
                if rev < LangRev::Vhdl2008 {
                    "needs a later VHDL standard"
                } else {
                    "needs VHDL-2019"
                },
            )
            .with_note(format!("this file is analyzed as {}", Db::rev_name(rev))),
        ));
    }
    let mut d = Diagnostic::error(
        diag::UNKNOWN_ATTRIBUTE,
        format!("attribute `'{}` does not apply to {what}", attr.text),
    )
    .with_primary(span, "this prefix");
    if a == PredefAttr::Reflect {
        d = d.with_note(
            "the prefix of `'reflect` must be an object or a fully constrained subtype (LRM 2019 16.2)",
        );
    }
    Err(db.error(d))
}

/// Whether `info` applies to `prefix` in this file's revision, without
/// reporting: the quiet form of [`select`] for the bottom-up pass.
pub fn applies(db: &Db, info: &AttrInfo, prefix: &Prefix) -> bool {
    let rev = db.file_rev();
    info.prefixes.iter().any(|r| {
        r.since <= rev && form_holds(r.form, prefix.form) && class_holds(db, r.class, prefix)
    })
}

/// The subtype of a value-, function- or signal-kind attribute. `None` for
/// ranges, subtypes, mirrors and views, which the caller builds itself, and
/// when `std.standard` is not loaded yet.
pub fn result_subtype(db: &Db, info: &AttrInfo, prefix: &Prefix, dim: u32) -> Option<SubtypeId> {
    let w = &db.well_known;
    let first = |t: Option<TypeId>| t.map(|t| db.types[t].first_subtype);
    let pst = prefix.subtype;
    match info.result {
        R::PrefixBase => pst.map(|st| db.types[db.base(st)].base_subtype),
        R::PrefixSubtype => pst,
        R::IndexValue => {
            let st = pst?;
            match &db.types[db.base(st)].kind {
                TypeKind::Array { indexes, .. } => indexes.get(dim as usize).copied(),
                _ => Some(db.types[db.base(st)].base_subtype),
            }
        }
        R::UniversalInteger => first(w.universal_integer),
        R::Boolean => first(w.boolean),
        R::String => first(w.string),
        R::Time => first(w.time),
        R::Bit => first(w.bit),
        R::Range | R::Subtype | R::RangeRecord | R::Mirror | R::View => None,
    }
}

/// Staticness of an attribute name (9.4.2, 9.4.3), given the staticness of
/// the prefix subtype and of the actual parameter (if any).
pub fn staticness(info: &AttrInfo, prefix: Staticness, args: Option<Staticness>) -> Staticness {
    match info.stat {
        Stat::OfPrefix => prefix,
        Stat::OfPrefixAndArgs => prefix.min(args.unwrap_or(Staticness::Locally)),
        Stat::Locally => Staticness::Locally,
        Stat::Globally => Staticness::Globally,
        Stat::None => Staticness::None,
    }
}

/// The type the actual of `info` must have, given the prefix subtype.
/// `None`: no parameter, or any integer type (`'val`, `'left(n)`).
pub fn param_type(db: &Db, info: &AttrInfo, prefix: &Prefix) -> Option<TypeId> {
    match info.param {
        Param::PrefixValue => prefix.subtype.map(|st| db.base(st)),
        Param::String => db.well_known.string,
        Param::Time => db.well_known.time,
        Param::None | Param::Dim | Param::AnyInteger => None,
    }
}

// ---------------------------------------------------------------------------
// Implicit signals (16.2.4)

/// Create the implicit signal of `prefix'attr(param)`: an object of class
/// signal and role `ImplicitSignal` in `scope`, whose `default` is the
/// defining `ExprKind::Attr` (prefix and parameter), so M2 can build its
/// process. The attribute name then reads it as `ExprKind::Obj`.
// ponytail: one signal per occurrence; sharing per (prefix LSP, attribute,
// parameter) is an optimisation with the same behaviour.
#[allow(clippy::too_many_arguments)]
pub fn implicit_signal(
    db: &mut Db,
    attr: PredefAttr,
    in_subprogram: bool,
    scope: ScopeId,
    prefix: ExprId,
    param: Option<ExprId>,
    subtype: SubtypeId,
    span: Span,
) -> Result<DeclId, Reported> {
    debug_assert_eq!(attr.info().kind, Kind::Signal);
    if in_subprogram {
        return Err(db.error(
            Diagnostic::error(
                diag::SIGNAL_ATTR_IN_SUBPROGRAM,
                format!("`'{}` is not allowed in a subprogram", attr.name()),
            )
            .with_primary(span, "implicit signal attribute")
            .with_note(
                "it declares an implicit signal, which a subprogram cannot have (LRM 16.2.4)",
            ),
        ));
    }
    if let Some(p) = param
        && db.exprs[p].stat < Staticness::Globally
    {
        let sp = db.exprs[p].span;
        return Err(db.error(
            Diagnostic::error(
                diag::NOT_STATIC,
                format!(
                    "the parameter of `'{}` must be globally static",
                    attr.name()
                ),
            )
            .with_primary(sp, "not static"),
        ));
    }
    let def = db.add_expr(Expr {
        kind: ExprKind::Attr {
            attr,
            prefix: AttrPrefix::Expr(prefix),
            args: param.into_iter().collect(),
        },
        subtype,
        span,
        stat: Staticness::None,
        value: None,
    });
    let name = Ident {
        text: Symbol::intern(attr.name()),
        key: Symbol::intern(attr.name()),
        span,
    };
    let d = db.decls.push(Decl {
        name: Designator::Ident(name),
        kind: DeclKind::Object(Object {
            class: ObjClass::Signal,
            role: ObjRole::ImplicitSignal,
            mode: None,
            subtype,
            default: Some(def),
            bounds: BoundsFrom::Subtype,
            signal_kind: None,
            file_open: None,
        }),
        scope,
        span,
        implicit: true,
        since: LangRev::Vhdl1993,
    });
    // In the region's order (for M2), never in its names.
    db.scopes[scope].order.push(d);
    Ok(d)
}

/// Take back the implicit signal `d` just made by [`implicit_signal`]: it is no longer a name of
/// its region, so the elaborator never creates it (`not s'stable` becomes `s'event`).
pub fn discard_implicit_signal(db: &mut Db, d: DeclId) {
    let order = &mut db.scopes[db.decls[d].scope].order;
    if let Some(i) = order.iter().rposition(|&x| x == d) {
        order.remove(i);
    }
}

// ---------------------------------------------------------------------------
// User-defined attributes (6.7, 7.2)

/// A user-defined attribute attached to one named entity.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct AttrValue {
    pub attr: DeclId,
    pub entity: DeclId,
    pub value: ExprId,
}

/// Check and create `attribute name : ty;` (6.7) in `scope`.
pub fn declare_attribute(
    db: &mut Db,
    scope: ScopeId,
    name: Ident,
    ty: SubtypeId,
    span: Span,
) -> Result<DeclId, Reported> {
    if let Some(bad) = forbidden_attr_type(db, db.base(ty), 0) {
        return Err(db.error(
            Diagnostic::error(
                diag::ATTR_TYPE,
                format!("attribute `{}` cannot be of type {bad}", name.text),
            )
            .with_primary(span, "attribute declaration")
            .with_note("the type of an attribute may not be an access, file or protected type, or contain one (LRM 6.7)"),
        ));
    }
    let d = db.decls.push(Decl {
        name: Designator::Ident(name),
        kind: DeclKind::Attribute { ty },
        scope,
        span,
        implicit: false,
        since: LangRev::Vhdl1993,
    });
    db.declare(scope, d);
    Ok(d)
}

fn forbidden_attr_type(db: &Db, ty: TypeId, depth: u32) -> Option<String> {
    if depth > 32 {
        return None;
    }
    match &db.types[ty].kind {
        TypeKind::Access { .. } | TypeKind::File { .. } | TypeKind::Protected { .. } => {
            Some(crate::expr::aggregate::type_name(db, ty))
        }
        TypeKind::Array { elem, .. } => forbidden_attr_type(db, db.base(*elem), depth + 1),
        TypeKind::Record { elems } => elems
            .iter()
            .find_map(|e| forbidden_attr_type(db, db.base(e.subtype), depth + 1)),
        _ => None,
    }
}

/// The subtype the value of an attribute specification is resolved with.
pub fn spec_subtype(db: &Db, attr: DeclId) -> Option<SubtypeId> {
    match db.decls[db.entity(attr)].kind {
        DeclKind::Attribute { ty } => Some(ty),
        _ => None,
    }
}

/// The entity class keyword of a declaration (7.2), `None` when it has none.
pub fn entity_class(db: &Db, d: DeclId) -> Option<Kw> {
    Some(match &db.decls[d].kind {
        DeclKind::Unit(u) => match db.units[*u].kind {
            UnitKind::Entity => Kw::Entity,
            UnitKind::Architecture => Kw::Architecture,
            UnitKind::Configuration => Kw::Configuration,
            UnitKind::Package | UnitKind::PackageInst => Kw::Package,
            UnitKind::PackageBody | UnitKind::Context => return None,
        },
        DeclKind::PackageInst(_) | DeclKind::Package { .. } => Kw::Package,
        DeclKind::Subprogram(s) | DeclKind::GenericSubprogram(s) => match s.kind {
            SubprogramKind::Function { .. } => Kw::Function,
            SubprogramKind::Procedure => Kw::Procedure,
        },
        DeclKind::Type(_) | DeclKind::GenericType(_) => Kw::Type,
        DeclKind::Subtype(_) => Kw::Subtype,
        DeclKind::Object(o) => match o.class {
            ObjClass::Constant => Kw::Constant,
            ObjClass::Signal => Kw::Signal,
            ObjClass::Variable | ObjClass::SharedVariable => Kw::Variable,
            ObjClass::File => Kw::File,
        },
        DeclKind::Component(_) => Kw::Component,
        DeclKind::Label(_) => Kw::Label,
        DeclKind::EnumLit { .. } => Kw::Literal,
        DeclKind::PhysUnit { .. } => Kw::Units,
        DeclKind::Group { .. } => Kw::Group,
        DeclKind::View(_) => Kw::View,
        DeclKind::Alias(Alias::NonObject { target }) => return entity_class(db, *target),
        _ => return None,
    })
}

/// Does `d` belong to entity class `class`? An object alias is accepted for
/// any object class (the LRM gives an alias the class of what it denotes;
/// sem does not track that for object aliases).
pub(crate) fn in_class(db: &Db, d: DeclId, class: Kw) -> bool {
    match &db.decls[d].kind {
        DeclKind::Alias(Alias::Object { .. }) => {
            matches!(class, Kw::Constant | Kw::Signal | Kw::Variable | Kw::File)
        }
        _ => entity_class(db, d) == Some(class),
    }
}

/// A user-defined attribute value of `entity` by attribute name, if any.
pub fn user_value(db: &Db, entity: DeclId, attr_key: Symbol) -> Option<AttrValueId> {
    db.attr_index.get(&(entity, attr_key)).copied()
}

/// Picks one of `candidates` by a signature (6.4), from the resolver.
pub type SignatureFn<'a> =
    dyn FnMut(&mut Db, &[DeclId], &ast::Signature) -> Result<DeclId, Reported> + 'a;

/// Apply `attribute a of <names> : <class> is <value>;` (7.2) in `scope`.
/// `attr` is the attribute declaration, `value` the specification's value
/// resolved with [`spec_subtype`]. `signature` picks one overload for a name
/// written with a signature (6.4); a name without one denotes every
/// declaration with that designator in the region (7.2).
pub fn specify(
    db: &mut Db,
    scope: ScopeId,
    spec: &ast::AttributeSpec,
    attr: DeclId,
    value: ExprId,
    signature: &mut SignatureFn<'_>,
) -> Result<Vec<AttrValueId>, Reported> {
    let attr = db.entity(attr);
    let key = db.decls[attr].name.key();
    let class = spec.class;
    if db.attr_closed.contains(&(scope, key, class)) {
        return Err(db.error(
            Diagnostic::error(
                diag::ATTR_TWICE,
                format!(
                    "attribute `{}` was already given to every remaining {} with `others` or `all`",
                    spec.attr.text,
                    class.as_str()
                ),
            )
            .with_primary(spec.span, "too late for this specification")
            .with_note("a specification with `others` or `all` must be the last one for its attribute and class (LRM 7.2)"),
        ));
    }
    let mut targets: Vec<(DeclId, Span)> = Vec::new();
    let mut failed = None;
    match &spec.entities {
        ast::EntityNameList::Names(names) => {
            for (des, sig) in names {
                let (dkey, sp) = designator_key(des);
                let mut found: Vec<DeclId> = db.scopes[scope]
                    .names
                    .get(&dkey)
                    .cloned()
                    .unwrap_or_default();
                // The design unit, subprogram or block a region belongs to
                // may be named in its own declarative part.
                if let Some(owner) = db.scopes[scope].owner
                    && db.decls[owner].name.key() == dkey
                    && !found.contains(&owner)
                {
                    found.push(owner);
                }
                // A port or generic in the architecture: the entity and its
                // architectures are one declarative region (12.1); sim1, sim2
                // and sim3 accept it.
                if found.is_empty()
                    && let Some(ent) = arch_entity(db, scope)
                {
                    found.extend(db.scopes[ent].names.get(&dkey).into_iter().flatten().filter(
                        |&&d| matches!(&db.decls[d].kind, DeclKind::Object(o) if matches!(o.role, ObjRole::Port | ObjRole::Generic)),
                    ));
                }
                // A label of a statement of this region's statement part, which
                // is analyzed after the declarative part: wait for it.
                if found.is_empty()
                    && class == Kw::Label
                    && sig.is_none()
                    && has_conc_stmts(db, scope)
                {
                    db.pending_label_attrs.push(PendingLabel {
                        scope,
                        key: dkey,
                        name: des_text(des),
                        span: sp,
                        attr,
                        value,
                    });
                    continue;
                }
                if found.is_empty() {
                    failed = Some(db.error(
                        Diagnostic::error(
                            diag::NOT_DECLARED,
                            format!(
                                "`{}` is not declared in this declarative part",
                                des_text(des)
                            ),
                        )
                        .with_primary(sp, "not found in this region")
                        .with_note("an attribute specification names entities declared in the same declarative part (LRM 7.2)"),
                    ));
                    continue;
                }
                // A declaration that failed is silent (8.2).
                if found
                    .iter()
                    .any(|&d| matches!(db.decls[d].kind, DeclKind::Error))
                {
                    failed = Some(Reported);
                    continue;
                }
                let first = found[0];
                let found: Vec<DeclId> = found
                    .into_iter()
                    .filter(|&d| in_class(db, d, class))
                    .collect();
                if found.is_empty() {
                    let (msg, label) = match entity_class(db, first) {
                        Some(k) => (
                            format!(
                                "`{}` is a {}, not a {}",
                                des_text(des),
                                k.as_str(),
                                class.as_str()
                            ),
                            format!("`{}` has entity class `{}`", des_text(des), k.as_str()),
                        ),
                        None => (
                            format!(
                                "`{}` cannot be decorated as a {}",
                                des_text(des),
                                class.as_str()
                            ),
                            "this entity class does not match".to_string(),
                        ),
                    };
                    failed = Some(
                        db.error(
                            Diagnostic::error(diag::ENTITY_CLASS, msg)
                                .with_primary(spec.class_span, label)
                                .with_secondary(sp, "named here"),
                        ),
                    );
                    continue;
                }
                match sig {
                    Some(sig) => match signature(db, &found, sig) {
                        Ok(d) => targets.push((d, sp)),
                        Err(e) => failed = Some(e),
                    },
                    None => targets.extend(found.into_iter().map(|d| (d, sp))),
                }
            }
        }
        ast::EntityNameList::Others | ast::EntityNameList::All => {
            let others = matches!(spec.entities, ast::EntityNameList::Others);
            let order = db.scopes[scope].order.clone();
            for d in order {
                if db.decls[d].implicit || !in_class(db, d, class) {
                    continue;
                }
                if others && user_value(db, d, key).is_some() {
                    continue;
                }
                targets.push((d, spec.span));
            }
            db.attr_closed.insert((scope, key, class));
        }
    }
    let mut out = Vec::new();
    for (d, sp) in targets {
        match attach(db, attr, d, value, sp) {
            Ok(id) => out.push(id),
            Err(e) => failed = Some(e),
        }
    }
    match failed {
        Some(e) => Err(e),
        None => Ok(out),
    }
}

/// Give `d` the value of attribute `attr` (E1622 when it already has one).
fn attach(
    db: &mut Db,
    attr: DeclId,
    d: DeclId,
    value: ExprId,
    sp: Span,
) -> Result<AttrValueId, Reported> {
    let key = db.decls[attr].name.key();
    let entity = db.entity(d);
    if let Some(old) = user_value(db, entity, key) {
        let old_span = db.exprs[db.attr_values[old].value].span;
        return Err(db.error(
            Diagnostic::error(
                diag::ATTR_TWICE,
                format!(
                    "attribute `{}` is given to `{}` twice",
                    db.decls[attr].name, db.decls[entity].name
                ),
            )
            .with_primary(sp, "given again here")
            .with_secondary(old_span, "first given here"),
        ));
    }
    let id = db.attr_values.push(AttrValue {
        attr,
        entity,
        value,
    });
    db.attr_index.insert((entity, key), id);
    if key.as_str() == "foreign"
        && let DeclKind::Subprogram(s) = &mut db.decls[entity].kind
    {
        s.body = Body::Foreign(value);
    }
    Ok(id)
}

/// A label specification waiting for its statement (7.2: it is written in
/// the declarative part of the construct whose statement part holds it).
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct PendingLabel {
    scope: ScopeId,
    key: Symbol,
    name: String,
    span: Span,
    attr: DeclId,
    value: ExprId,
}

/// The entity region of an architecture region (its parent is the context
/// region, whose parent is the entity's).
fn arch_entity(db: &Db, scope: ScopeId) -> Option<ScopeId> {
    if db.scopes[scope].kind != ScopeKind::Architecture {
        return None;
    }
    db.scopes[scope]
        .parent
        .and_then(|c| db.scopes[c].parent)
        .filter(|&e| db.scopes[e].kind == ScopeKind::Entity)
}

/// Regions whose statement part (concurrent, or sequential for a process or subprogram body)
/// follows the declarative part and ends with [`unresolved_labels`].
pub(crate) fn has_conc_stmts(db: &Db, scope: ScopeId) -> bool {
    matches!(
        db.scopes[scope].kind,
        ScopeKind::Entity
            | ScopeKind::Architecture
            | ScopeKind::Block
            | ScopeKind::Generate
            | ScopeKind::Process
            | ScopeKind::Subprogram
    )
}

/// Label `d` was declared in `scope`: apply the specifications waiting for it.
pub(crate) fn label_declared(db: &mut Db, scope: ScopeId, d: DeclId) -> Result<(), Reported> {
    db.label_for_groups(scope, d);
    let key = db.decls[d].name.key();
    let mut failed = None;
    let mut i = 0;
    while i < db.pending_label_attrs.len() {
        let p = &db.pending_label_attrs[i];
        if p.scope != scope || p.key != key {
            i += 1;
            continue;
        }
        let p = db.pending_label_attrs.remove(i);
        if let Err(e) = attach(db, p.attr, d, p.value, p.span) {
            failed = Some(e);
        }
    }
    failed.map_or(Ok(()), Err)
}

/// The end of `scope`'s statement part: a label specification still waiting
/// names no label of it (E1100).
pub(crate) fn unresolved_labels(db: &mut Db, scope: ScopeId) -> Result<(), Reported> {
    let groups = db.unresolved_group_labels(scope);
    let (left, keep) = std::mem::take(&mut db.pending_label_attrs)
        .into_iter()
        .partition(|p| p.scope == scope);
    db.pending_label_attrs = keep;
    let mut failed = groups.err();
    for p in left.iter() {
        failed = Some(db.error(
            Diagnostic::error(diag::NOT_DECLARED, format!("`{}` is not declared", p.name))
                .with_primary(p.span, "no statement of this region has this label")
                .with_note("a label specification names a statement of the statement part of the construct whose declarative part holds it (LRM 7.2)"),
        ));
    }
    failed.map_or(Ok(()), Err)
}

fn designator_key(d: &ast::Designator) -> (Symbol, Span) {
    match d {
        ast::Designator::Ident(i) => (i.key, i.span),
        ast::Designator::Operator(s, sp) => (crate::decl::operator_key(s.as_str()), *sp),
        ast::Designator::Char(c) => (crate::decl::char_key(c.value), c.span),
    }
}

fn des_text(d: &ast::Designator) -> String {
    match d {
        ast::Designator::Ident(i) => i.text.to_string(),
        ast::Designator::Operator(s, _) => format!("\"{s}\""),
        ast::Designator::Char(c) => format!("'{}'", c.value),
    }
}

#[cfg(test)]
#[path = "attr_tests.rs"]
mod tests;

impl Db {
    /// 2019 `A'INDEX` or `A'INDEX(n)` as a type mark (16.2): the index subtype of
    /// dimension `n`, narrowed to `A'RANGE(n)` when `A` is an object or a fully
    /// constrained array subtype (L14).
    pub(crate) fn index_type_mark(
        &mut self,
        scope: ScopeId,
        prefix: &ast::Name,
        attr: Ident,
        dim: Option<&ast::Expr>,
        span: Span,
    ) -> Result<SubtypeId, Reported> {
        let (range, _) = self.range_attr_inner(scope, prefix, attr, false, dim, span)?;
        let crate::types::RangeC::Attr {
            prefix: aprefix,
            dim,
            ..
        } = &range
        else {
            return Err(self.ice(span, "a range attribute without an attribute range"));
        };
        let (st, narrow) = match *aprefix {
            AttrPrefix::Subtype(st) => (
                st,
                self.constrainedness(st) == crate::types::Constrainedness::Full,
            ),
            AttrPrefix::Expr(e) => (self.exprs[e].subtype, true),
            AttrPrefix::Entity(_) => return Err(self.ice(span, "`'index` of a named entity")),
        };
        let index = match &self.types[self.base(st)].kind {
            TypeKind::Array { indexes, .. } => indexes[*dim as usize - 1],
            TypeKind::Error => return Err(Reported),
            _ => {
                return Err(self.error(
                    Diagnostic::error(diag::NOT_A_TYPE, "`'index` needs an array prefix")
                        .with_primary(prefix.span, "not an array"),
                ));
            }
        };
        if !narrow {
            return Ok(index);
        }
        let base = self.base(index);
        Ok(self.subtypes.push(crate::types::SubtypeDef {
            base,
            name: None,
            parent: Some(index),
            constraint: crate::types::Constraint::Range(range),
            resolution: None,
            span,
        }))
    }

    /// 2019 `R'RECORD` (16.2.6), `R` a range attribute name: the range record type of
    /// the range's scalar type, with elements `LEFT`, `RIGHT` and `DIRECTION` (L12).
    /// One anonymous record type per scalar base type, made on first use.
    pub(crate) fn range_record_type_mark(
        &mut self,
        scope: ScopeId,
        prefix: &ast::Name,
        attr: Ident,
    ) -> Result<SubtypeId, Reported> {
        if self.file_rev() < yavs_syntax::Std::Vhdl2019 {
            return Err(self.error(
                Diagnostic::error(
                    diag::NOT_IN_REVISION,
                    "`'record` of a range attribute is not available in VHDL-2008",
                )
                .with_primary(attr.span, "needs VHDL-2019"),
            ));
        }
        let Some(r) = self.range_attr(scope, prefix) else {
            return Err(self.error(
                Diagnostic::error(
                    diag::NOT_A_TYPE,
                    "the prefix of `'record` must be a range attribute name",
                )
                .with_primary(prefix.span, "not `X'range` or `X'reverse_range`"),
            ));
        };
        let (_, t) = r?;
        self.range_record_of(t, attr.span)
    }

    /// The anonymous range record subtype of scalar base type `t` (made on first use).
    pub(crate) fn range_record_of(&mut self, t: TypeId, span: Span) -> Result<SubtypeId, Reported> {
        let attr = Ident {
            text: Symbol::intern("record"),
            key: Symbol::intern("record"),
            span,
        };
        if let Some(&st) = self.range_records.get(&t) {
            return Ok(st);
        }
        let Some(dir) = self.well_known.range_direction else {
            return Err(self.ice(attr.span, "`'record` without std.standard.RANGE_DIRECTION"));
        };
        let elem = self.types[t].first_subtype;
        let dir = self.types[dir].first_subtype;
        let name = Ident {
            text: Symbol::intern(&format!("{}_RANGE_RECORD", self.types[t].name.text)),
            key: Symbol::intern(&format!("{}_range_record", self.types[t].name.key)),
            span: attr.span,
        };
        let el = |n: &str, subtype| crate::types::RecordElem {
            name: Ident {
                text: Symbol::intern(n),
                key: Symbol::intern(&n.to_ascii_lowercase()),
                span: attr.span,
            },
            subtype,
        };
        let elems = Box::new([el("LEFT", elem), el("RIGHT", elem), el("DIRECTION", dir)]);
        let ty = self.types.next_id();
        let st = self.subtypes.push(crate::types::SubtypeDef {
            base: ty,
            name: Some(name),
            parent: None,
            constraint: crate::types::Constraint::None,
            resolution: None,
            span: attr.span,
        });
        let decl = self.types[t].decl;
        self.types.push(crate::types::TypeDef {
            decl,
            name,
            anonymous: true,
            kind: TypeKind::Record { elems },
            first_subtype: st,
            base_subtype: st,
        });
        self.range_records.insert(t, st);
        Ok(st)
    }
}
