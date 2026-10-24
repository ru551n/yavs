//! Implicitly declared operations (sem.md 2.10, `lib/NATIVE.md` section 2).
//!
//! Every predefined operation is a real `DeclKind::Subprogram` entity, created
//! in the same region right after its type, with `implicit = true`, so
//! visibility, hiding, aliases, `use p."+"` and overload resolution need no
//! special cases.
//!
//! Entry point: [`Db::declare_implicit_ops`], called by type declaration
//! analysis once the type's declaration is in its region. Inside
//! `std.standard` an operation may name a type that is declared later
//! (`RANGE_DIRECTION`'s `"="` returns BOOLEAN, `TO_STRING` returns STRING): it waits
//! in `Db::pending_implicit` and is created as soon as every type it names
//! exists. Outside `std.standard` everything it names exists, so nothing waits.
//!
//! LRM references are to 1076-2008 unless marked 2019:
//! - 5.2.6 (and 9.2): scalar types: `=` `/=` `<` `<=` `>` `>=`, `MINIMUM`, `MAXIMUM`, `TO_STRING`.
//! - 9.2.5-9.2.8: arithmetic of integer, floating and physical types; `**` with an INTEGER right operand.
//! - 9.2.2: logical operators on BIT, BOOLEAN and their one-dimensional arrays (array-scalar and reduction forms new in 2008).
//! - 9.2.3: matching relational operators on BIT and STD_ULOGIC, `?=`/`?/=` on their one-dimensional arrays.
//! - 9.2.4: shift operators on one-dimensional arrays of BIT and BOOLEAN.
//! - 9.2.9: `??` on BIT (STD_ULOGIC's is explicit in `std_logic_1164`).
//! - 5.3.2.4: one-dimensional arrays: `&` (4 forms), `MINIMUM`/`MAXIMUM` (array and reduction forms),
//!   `TO_STRING` for arrays of character-literal-only enumerations; `TO_OSTRING`/`TO_HSTRING` on BIT_VECTOR.
//! - 9.2.3: relational operators on one-dimensional arrays of discrete elements; 2019 LCS-2016-059a: of scalar elements.
//! - 5.3.3: records: `=` `/=`. 5.4.3: access: `DEALLOCATE`. 5.5.2: file operations (2019 adds
//!   `FILE_OPEN` as a function, `FILE_REWIND`, `FILE_SEEK`, `FILE_TRUNCATE`, `FILE_STATE`, `FILE_MODE`,
//!   `FILE_POSITION`, `FILE_SIZE`, `FILE_CANSEEK`).
//! - 16.3: `RISING_EDGE`/`FALLING_EDGE` on BIT and BOOLEAN signals, and the `TO_STRING` overloads
//!   with `DIGITS`, `FORMAT` and `UNIT`, declared in `std.standard`.
//! - 2019 5.7: `TO_STRING` also on composite types whose elements have it (sim1 agrees).

use yavs_diag::Span;
use yavs_syntax::Symbol;
use yavs_syntax::ast::Ident;

use crate::LangRev;
use crate::db::Db;
use crate::decl::{
    Alias, Body, Decl, DeclKind, Designator, ObjClass, Param, Subprogram, SubprogramKind,
};
use crate::generic::GenericClass;
use crate::hir::{Expr, ExprKind, Mode};
use crate::ids::{ScopeId, SubtypeId, TypeId};
use crate::loader::UnitKey;
use crate::stat::{ConstValue, Staticness};
use crate::types::{Constraint, TypeKind};

#[cfg(test)]
mod tests;

/// The native link of an implicit op is `yavs.implicit.<family>.<type mangled>[#<form>]`
/// (`mangle::implicit_native`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImplicitOp {
    pub family: Family,
    /// The type the operation was declared for.
    pub ty: TypeId,
    /// Which overload of the family, when the type has several.
    pub form: Form,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Family {
    Eq,
    Neq,
    Lt,
    Le,
    Gt,
    Ge,
    Minimum,
    Maximum,
    ToString,
    Add,
    Sub,
    Mul,
    Div,
    Neg,
    Pos,
    Abs,
    Exp,
    Rem,
    Mod,
    And,
    Or,
    Nand,
    Nor,
    Xor,
    Xnor,
    Not,
    /// Matching relational operators `?=` etc. and the condition operator `??`.
    MEq,
    MNeq,
    MLt,
    MLe,
    MGt,
    MGe,
    Cond,
    Cat,
    Sll,
    Srl,
    Sla,
    Sra,
    Rol,
    Ror,
    ToOString,
    ToHString,
    Deallocate,
    FileOpen,
    FileClose,
    Read,
    Write,
    Endfile,
    Flush,
    FileState,
    FileMode,
    FilePosition,
    FileSize,
    FileSeek,
    FileTruncate,
    FileRewind,
    FileCanseek,
    RisingEdge,
    FallingEdge,
}

impl Family {
    /// The family part of the native symbol.
    pub fn as_str(self) -> &'static str {
        use Family::*;
        match self {
            Eq => "eq",
            Neq => "neq",
            Lt => "lt",
            Le => "le",
            Gt => "gt",
            Ge => "ge",
            Minimum => "minimum",
            Maximum => "maximum",
            ToString => "to_string",
            Add => "add",
            Sub => "sub",
            Mul => "mul",
            Div => "div",
            Neg => "neg",
            Pos => "pos",
            Abs => "abs",
            Exp => "exp",
            Rem => "rem",
            Mod => "mod",
            And => "and",
            Or => "or",
            Nand => "nand",
            Nor => "nor",
            Xor => "xor",
            Xnor => "xnor",
            Not => "not",
            MEq => "meq",
            MNeq => "mneq",
            MLt => "mlt",
            MLe => "mle",
            MGt => "mgt",
            MGe => "mge",
            Cond => "cond",
            Cat => "cat",
            Sll => "sll",
            Srl => "srl",
            Sla => "sla",
            Sra => "sra",
            Rol => "rol",
            Ror => "ror",
            ToOString => "to_ostring",
            ToHString => "to_hstring",
            Deallocate => "deallocate",
            FileOpen => "file_open",
            FileClose => "file_close",
            Read => "read",
            Write => "write",
            Endfile => "endfile",
            Flush => "flush",
            FileState => "file_state",
            FileMode => "file_mode",
            FilePosition => "file_position",
            FileSize => "file_size",
            FileSeek => "file_seek",
            FileTruncate => "file_truncate",
            FileRewind => "file_rewind",
            FileCanseek => "file_canseek",
            RisingEdge => "rising_edge",
            FallingEdge => "falling_edge",
        }
    }
}

/// Which overload of a family: `Plain` is the one named by the family alone
/// (`T op T`, or the only one); the others are suffixed to the native symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Form {
    Plain,
    /// Array op element (`&`, logical operators).
    ArrayScalar,
    /// Element op array.
    ScalarArray,
    /// Element `&` element, giving an array.
    ScalarScalar,
    /// Unary reduction of an array to its element (`and v`, `MINIMUM(v)`).
    Reduce,
    /// Physical by INTEGER, physical by REAL, INTEGER by physical, REAL by physical.
    PhysInt,
    PhysReal,
    IntPhys,
    RealPhys,
    /// Physical / physical, giving universal_integer.
    PhysPhys,
    /// universal_real by universal_integer and the other way round.
    RealInt,
    IntReal,
    /// `FILE_OPEN` with a `Status` out parameter.
    Status,
    /// 2019 `FILE_OPEN` as a function returning FILE_OPEN_STATUS.
    Function,
    /// `TO_STRING(VALUE: REAL; DIGITS: NATURAL)`, `(VALUE: REAL; FORMAT: STRING)`, `(VALUE: TIME; UNIT: TIME)`.
    Digits,
    Format,
    Unit,
}

impl Form {
    /// The suffix of the native symbol; `None` for `Plain`.
    pub fn suffix(self) -> Option<&'static str> {
        use Form::*;
        Some(match self {
            Plain => return None,
            ArrayScalar => "as",
            ScalarArray => "sa",
            ScalarScalar => "ss",
            Reduce => "reduce",
            PhysInt => "pi",
            PhysReal => "pr",
            IntPhys => "ip",
            RealPhys => "rp",
            PhysPhys => "pp",
            RealInt => "ri",
            IntReal => "ir",
            Status => "status",
            Function => "function",
            Digits => "digits",
            Format => "format",
            Unit => "unit",
        })
    }
}

/// A type an implicit operation names.
#[derive(Clone, Copy, Debug)]
enum T {
    /// The whole base type (`base_subtype`): operands and results of operators.
    Base(TypeId),
    /// A subtype as declared (the element subtype of a file type).
    Sub(SubtypeId),
    /// A named type or subtype of `std.standard` (`INTEGER`, `NATURAL`, `STRING`).
    Std(&'static str),
    /// universal_integer.
    UInt,
}

#[derive(Clone, Copy, Debug)]
struct P {
    name: &'static str,
    class: ObjClass,
    mode: Mode,
    ty: T,
    /// Default: the enumeration literal at this position of the parameter's type.
    default: Option<u32>,
}

/// One implicit operation to create.
#[derive(Clone, Debug)]
pub(crate) struct Spec {
    owner: TypeId,
    family: Family,
    form: Form,
    /// Operator symbol without quotes, lower case (`=`, `and`), or identifier in upper case (`MINIMUM`).
    name: &'static str,
    params: Vec<P>,
    /// `None`: a procedure.
    ret: Option<T>,
    since: LangRev,
    pure: bool,
    /// Implicit aliases of this operation (`TO_BSTRING` for BIT_VECTOR's `TO_STRING`).
    aliases: &'static [&'static str],
}

/// An operation waiting in `std.standard` for a type it names.
pub(crate) struct Pending {
    scope: ScopeId,
    span: Span,
    spec: Spec,
}

fn anon(ty: T) -> P {
    named("anonymous", ty)
}

fn named(name: &'static str, ty: T) -> P {
    P {
        name,
        class: ObjClass::Constant,
        mode: Mode::In,
        ty,
        default: None,
    }
}

fn file_param(ty: T) -> P {
    P {
        class: ObjClass::File,
        ..named("F", ty)
    }
}

fn out(name: &'static str, ty: T) -> P {
    P {
        class: ObjClass::Variable,
        mode: Mode::Out,
        ..named(name, ty)
    }
}

impl Spec {
    fn new(
        owner: TypeId,
        family: Family,
        name: &'static str,
        params: Vec<P>,
        ret: Option<T>,
    ) -> Self {
        Spec {
            owner,
            family,
            form: Form::Plain,
            name,
            params,
            ret,
            since: LangRev::Vhdl1993,
            pure: true,
            aliases: &[],
        }
    }

    fn form(mut self, form: Form) -> Self {
        self.form = form;
        self
    }

    fn since(mut self, rev: LangRev) -> Self {
        self.since = rev;
        self
    }
}

const LOGICAL: [(Family, &str); 6] = [
    (Family::And, "and"),
    (Family::Or, "or"),
    (Family::Nand, "nand"),
    (Family::Nor, "nor"),
    (Family::Xor, "xor"),
    (Family::Xnor, "xnor"),
];

const RELATIONAL: [(Family, &str); 4] = [
    (Family::Lt, "<"),
    (Family::Le, "<="),
    (Family::Gt, ">"),
    (Family::Ge, ">="),
];

const MATCHING: [(Family, &str); 6] = [
    (Family::MEq, "?="),
    (Family::MNeq, "?/="),
    (Family::MLt, "?<"),
    (Family::MLe, "?<="),
    (Family::MGt, "?>"),
    (Family::MGe, "?>="),
];

const SHIFTS: [(Family, &str); 6] = [
    (Family::Sll, "sll"),
    (Family::Srl, "srl"),
    (Family::Sla, "sla"),
    (Family::Sra, "sra"),
    (Family::Rol, "rol"),
    (Family::Ror, "ror"),
];

/// The class of a type as far as its implicit operations go.
enum Shape {
    Enum,
    Integer,
    Floating,
    Physical,
    /// A generic type of class `is <>` or `is (<>)`: scalar, no arithmetic.
    Scalar,
    Array {
        dims: usize,
        elem: SubtypeId,
    },
    Record,
    Access,
    File {
        elem: SubtypeId,
    },
    /// A generic type of class `private` (2008 `type T`).
    Private,
    /// Protected, incomplete, poison: no implicit operations.
    None,
}

impl Db {
    /// Declare the implicit operations of type `ty` in `scope` (sem.md 2.10).
    ///
    /// Call it right after the type's declaration is in `scope` (and, in
    /// `std.standard`, after the matching `well_known` field is set). For the
    /// universal types call it at the start of `std.standard`, in its scope. A
    /// completed incomplete type gets its operations at its full declaration.
    pub(crate) fn declare_implicit_ops(&mut self, ty: TypeId, scope: ScopeId) {
        let span = self.decls[self.types[ty].decl].span;
        let specs = self.implicit_specs(ty);
        let rev = self.rev;
        self.pending_implicit.extend(
            specs
                .into_iter()
                .filter(|s| s.since <= rev)
                .map(|spec| Pending { scope, span, spec }),
        );
        self.flush_pending_implicit();
    }

    /// Create every waiting operation whose types exist now; called after each
    /// `std.standard` type or subtype.
    pub(crate) fn flush_pending_implicit(&mut self) {
        let pending = std::mem::take(&mut self.pending_implicit);
        for p in pending {
            if self.emit_implicit(&p).is_some() {
                continue;
            }
            if Some(p.scope) == self.well_known.standard {
                self.pending_implicit.push(p);
            } else {
                // Outside std.standard every type an implicit op names exists.
                self.ice(
                    p.span,
                    format!(
                        "implicit operation `{}` names a std.standard type that does not exist",
                        p.spec.name
                    ),
                );
            }
        }
    }

    /// The subtype `t` denotes, or `None` while it is not declared yet.
    fn implicit_type(&self, t: T) -> Option<SubtypeId> {
        match t {
            T::Base(ty) => Some(self.types[ty].base_subtype),
            T::Sub(st) => Some(st),
            T::UInt => self
                .well_known
                .universal_integer
                .map(|u| self.types[u].base_subtype),
            T::Std(name) => self.std_subtype(name),
        }
    }

    /// A named type (its first subtype) or subtype of `std.standard`.
    fn std_subtype(&self, name: &str) -> Option<SubtypeId> {
        let scope = self.well_known.standard?;
        let decls = self.scopes[scope].names.get(&Symbol::intern(name))?;
        decls.iter().find_map(|&d| match self.decls[d].kind {
            DeclKind::Type(t) => Some(self.types[t].first_subtype),
            DeclKind::Subtype(st) => Some(st),
            _ => None,
        })
    }

    /// Create one operation; `None` (and nothing created) while a type it names is missing.
    fn emit_implicit(&mut self, p: &Pending) -> Option<()> {
        let spec = &p.spec;
        let mut params = Vec::with_capacity(spec.params.len());
        for q in &spec.params {
            params.push((q, self.implicit_type(q.ty)?));
        }
        let ret = match spec.ret {
            Some(t) => Some(self.implicit_type(t)?),
            None => None,
        };
        let since = spec
            .since
            .max(self.decls[self.types[spec.owner].decl].since);
        let params: Box<[Param]> = params
            .into_iter()
            .map(|(q, subtype)| Param {
                name: ident(q.name, p.span),
                class: q.class,
                mode: q.mode,
                subtype,
                default: q.default.map(|pos| self.enum_default(subtype, pos, p.span)),
                decl: None,
                mark: None,
            })
            .collect();
        // Identifiers are written in upper case, operator symbols (`and`, `=`) are not.
        let name = if spec.name.starts_with(|c: char| c.is_ascii_uppercase()) {
            Designator::Ident(ident(spec.name, p.span))
        } else {
            Designator::operator(spec.name)
        };
        let types: Vec<SubtypeId> = params.iter().map(|q| q.subtype).collect();
        let mangled = Symbol::intern(&crate::mangle::subprogram(
            self, p.scope, &name, &types, ret,
        ));
        let kind = match ret {
            Some(_) => SubprogramKind::Function { pure: spec.pure },
            None => SubprogramKind::Procedure,
        };
        let d = self.decls.push(Decl {
            name,
            kind: DeclKind::Subprogram(Subprogram {
                kind,
                params,
                ret,
                generics: None,
                scope: None,
                body: Body::Implicit(ImplicitOp {
                    family: spec.family,
                    ty: spec.owner,
                    form: spec.form,
                }),
                mangled,
                ret_mark: None,
                impure_reads: false,
            }),
            scope: p.scope,
            span: p.span,
            implicit: true,
            since,
        });
        self.declare(p.scope, d);
        for alias in spec.aliases {
            let a = self.decls.push(Decl {
                name: Designator::Ident(ident(alias, p.span)),
                kind: DeclKind::Alias(Alias::NonObject { target: d }),
                scope: p.scope,
                span: p.span,
                implicit: true,
                since,
            });
            self.declare(p.scope, a);
        }
        Some(())
    }

    /// A parameter default: the enumeration literal at `pos` (`READ_MODE`, `FILE_ORIGIN_BEGIN`).
    fn enum_default(&mut self, subtype: SubtypeId, pos: u32, span: Span) -> crate::ids::ExprId {
        let c = self.add_const(ConstValue::Enum(pos));
        self.add_expr(Expr {
            kind: ExprKind::Const(c),
            subtype,
            span,
            stat: Staticness::Locally,
            value: Some(c),
        })
    }

    fn shape(&self, ty: TypeId) -> Shape {
        match &self.types[ty].kind {
            // 2019 composites of protected types (LCS-2016-014) are not values: no `=`, `&`.
            TypeKind::Array { .. } | TypeKind::Record { .. }
                if self.type_contains_protected(ty) =>
            {
                Shape::None
            }
            TypeKind::Enum { .. } => Shape::Enum,
            TypeKind::UniversalInteger | TypeKind::Integer => Shape::Integer,
            TypeKind::UniversalReal | TypeKind::Floating => Shape::Floating,
            TypeKind::Physical { .. } => Shape::Physical,
            TypeKind::Array { indexes, elem } => Shape::Array {
                dims: indexes.len(),
                elem: *elem,
            },
            TypeKind::Record { .. } => Shape::Record,
            TypeKind::Access { .. } => Shape::Access,
            TypeKind::File { elem } => Shape::File { elem: *elem },
            TypeKind::Generic { class } => match class {
                GenericClass::Private => Shape::Private,
                GenericClass::Scalar | GenericClass::Discrete => Shape::Scalar,
                GenericClass::Integer => Shape::Integer,
                GenericClass::Floating => Shape::Floating,
                GenericClass::Physical => Shape::Physical,
                GenericClass::Access { .. } => Shape::Access,
                GenericClass::File { elem } => Shape::File { elem: *elem },
            },
            TypeKind::Protected { .. } | TypeKind::Incomplete { .. } | TypeKind::Error => {
                Shape::None
            }
        }
    }

    fn is_scalar_type(&self, ty: TypeId) -> bool {
        match &self.types[ty].kind {
            TypeKind::Generic { class } => !matches!(
                class,
                GenericClass::Private | GenericClass::Access { .. } | GenericClass::File { .. }
            ),
            k => k.is_scalar(),
        }
    }

    fn is_discrete_type(&self, ty: TypeId) -> bool {
        match &self.types[ty].kind {
            TypeKind::Generic { class } => {
                matches!(class, GenericClass::Discrete | GenericClass::Integer)
            }
            k => k.is_discrete(),
        }
    }

    /// An enumeration type whose literals are all character literals (5.3.2.4, `TO_STRING`).
    fn is_char_enum(&self, ty: TypeId) -> bool {
        match &self.types[ty].kind {
            TypeKind::Enum { lits } => {
                !lits.is_empty()
                    && lits
                        .iter()
                        .all(|&l| matches!(self.decls[l].name, Designator::Char(_)))
            }
            _ => false,
        }
    }

    /// 2019 5.7: scalar types, and composites all of whose elements have `TO_STRING`.
    fn is_representable_2019(&self, ty: TypeId) -> bool {
        if self.is_scalar_type(ty) {
            return true;
        }
        match &self.types[ty].kind {
            TypeKind::Array { elem, .. } => self.is_representable_2019(self.base(*elem)),
            TypeKind::Record { elems } => elems
                .iter()
                .all(|e| self.is_representable_2019(self.base(e.subtype))),
            _ => false,
        }
    }

    /// `ty` is the `std.standard` type `name`.
    fn is_std(&self, ty: TypeId, name: &str) -> bool {
        let t = &self.types[ty];
        t.name.key.as_str() == name && Some(self.decls[t.decl].scope) == self.well_known.standard
    }

    /// `ty` is `ieee.std_logic_1164.std_ulogic`.
    fn is_std_ulogic(&self, ty: TypeId) -> bool {
        let t = &self.types[ty];
        if t.name.key.as_str() != "std_ulogic" {
            return false;
        }
        let scope = self.decls[t.decl].scope;
        let Some(owner) = self.scopes[scope].owner else {
            return false;
        };
        let DeclKind::Unit(u) = self.decls[owner].kind else {
            return false;
        };
        let u = &self.units[u];
        self.libs[u.lib].name.key.as_str() == "ieee"
            && u.key == UnitKey::Primary(Symbol::intern("std_logic_1164"))
    }

    /// The subtype is an array whose index ranges are not given (5.5.2: `READ` with `LENGTH`).
    // ponytail: a partially constrained element (`array (0 to 3) of bit_vector`) counts as
    // constrained; check the 2019 5.5.2 wording if a file of such a type ever appears.
    fn is_unconstrained_array(&self, st: SubtypeId) -> bool {
        let base = self.base(st);
        let is_array = matches!(self.types[base].kind, TypeKind::Array { .. });
        is_array
            && matches!(
                self.subtypes[st].constraint,
                Constraint::None | Constraint::Array { dims: None, .. }
            )
    }

    /// The implicit operations of `ty`, in declaration order (the order of the
    /// `standard.vhdl` listing where it has one).
    pub(crate) fn implicit_specs(&self, ty: TypeId) -> Vec<Spec> {
        let me = T::Base(ty);
        let boolean = T::Std("boolean");
        let string = T::Std("string");
        let integer = T::Std("integer");
        let mut v = Vec::new();
        let bin = |f: Family, op: &'static str, l: T, r: T, ret: T| {
            Spec::new(ty, f, op, vec![anon(l), anon(r)], Some(ret))
        };
        let un = |f: Family, op: &'static str, x: T, ret: T| {
            Spec::new(ty, f, op, vec![anon(x)], Some(ret))
        };
        // `r: None` is the reduction form `MINIMUM(L: array) return element`.
        let minmax = |l: T, r: Option<T>, ret: T| {
            [(Family::Minimum, "MINIMUM"), (Family::Maximum, "MAXIMUM")].map(|(f, name)| match r {
                Some(r) => Spec::new(ty, f, name, vec![named("L", l), named("R", r)], Some(ret)),
                None => Spec::new(ty, f, name, vec![named("L", l)], Some(ret)).form(Form::Reduce),
            })
        };
        let v08 = LangRev::Vhdl2008;
        let to_string = |since: LangRev| {
            Spec::new(
                ty,
                Family::ToString,
                "TO_STRING",
                vec![named("VALUE", me)],
                Some(string),
            )
            .since(since)
        };
        let eq = |v: &mut Vec<Spec>| {
            v.push(bin(Family::Eq, "=", me, me, boolean));
            v.push(bin(Family::Neq, "/=", me, me, boolean));
        };
        let shape = self.shape(ty);
        match shape {
            Shape::None => {}
            Shape::Private => eq(&mut v),
            Shape::Enum | Shape::Integer | Shape::Floating | Shape::Physical | Shape::Scalar => {
                let (is_bool, is_bit) = (self.is_std(ty, "boolean"), self.is_std(ty, "bit"));
                if is_bool || is_bit {
                    for (f, op) in LOGICAL {
                        v.push(bin(f, op, me, me, me));
                    }
                    v.push(un(Family::Not, "not", me, me));
                }
                eq(&mut v);
                for (f, op) in RELATIONAL {
                    v.push(bin(f, op, me, me, boolean));
                }
                if is_bit || self.is_std_ulogic(ty) {
                    for (f, op) in MATCHING {
                        v.push(bin(f, op, me, me, me).since(v08));
                    }
                }
                if is_bit {
                    v.push(un(Family::Cond, "??", me, boolean).since(v08));
                }
                match shape {
                    Shape::Integer | Shape::Floating => {
                        v.push(un(Family::Pos, "+", me, me));
                        v.push(un(Family::Neg, "-", me, me));
                        v.push(un(Family::Abs, "abs", me, me));
                        v.push(bin(Family::Add, "+", me, me, me));
                        v.push(bin(Family::Sub, "-", me, me, me));
                        v.push(bin(Family::Mul, "*", me, me, me));
                        v.push(bin(Family::Div, "/", me, me, me));
                        if matches!(shape, Shape::Integer) {
                            v.push(bin(Family::Mod, "mod", me, me, me));
                            v.push(bin(Family::Rem, "rem", me, me, me));
                        }
                        if matches!(self.types[ty].kind, TypeKind::UniversalReal) {
                            v.push(bin(Family::Mul, "*", me, T::UInt, me).form(Form::RealInt));
                            v.push(bin(Family::Mul, "*", T::UInt, me, me).form(Form::IntReal));
                            v.push(bin(Family::Div, "/", me, T::UInt, me).form(Form::RealInt));
                        }
                        v.push(bin(Family::Exp, "**", me, integer, me));
                    }
                    Shape::Physical => {
                        let real = T::Std("real");
                        v.push(un(Family::Pos, "+", me, me));
                        v.push(un(Family::Neg, "-", me, me));
                        v.push(un(Family::Abs, "abs", me, me));
                        v.push(bin(Family::Add, "+", me, me, me));
                        v.push(bin(Family::Sub, "-", me, me, me));
                        v.push(bin(Family::Mul, "*", me, integer, me).form(Form::PhysInt));
                        v.push(bin(Family::Mul, "*", me, real, me).form(Form::PhysReal));
                        v.push(bin(Family::Mul, "*", integer, me, me).form(Form::IntPhys));
                        v.push(bin(Family::Mul, "*", real, me, me).form(Form::RealPhys));
                        v.push(bin(Family::Div, "/", me, integer, me).form(Form::PhysInt));
                        v.push(bin(Family::Div, "/", me, real, me).form(Form::PhysReal));
                        v.push(bin(Family::Div, "/", me, me, T::UInt).form(Form::PhysPhys));
                        v.push(bin(Family::Mod, "mod", me, me, me));
                        v.push(bin(Family::Rem, "rem", me, me, me));
                    }
                    _ => {}
                }
                v.extend(minmax(me, Some(me), me).map(|s| s.since(v08)));
                v.push(to_string(v08));
                if self.is_std(ty, "real") {
                    v.push(
                        Spec::new(
                            ty,
                            Family::ToString,
                            "TO_STRING",
                            vec![named("VALUE", me), named("DIGITS", T::Std("natural"))],
                            Some(string),
                        )
                        .form(Form::Digits)
                        .since(v08),
                    );
                    v.push(
                        Spec::new(
                            ty,
                            Family::ToString,
                            "TO_STRING",
                            vec![named("VALUE", me), named("FORMAT", string)],
                            Some(string),
                        )
                        .form(Form::Format)
                        .since(v08),
                    );
                }
                if self.is_std(ty, "time") {
                    v.push(
                        Spec::new(
                            ty,
                            Family::ToString,
                            "TO_STRING",
                            vec![named("VALUE", me), named("UNIT", me)],
                            Some(string),
                        )
                        .form(Form::Unit)
                        .since(v08),
                    );
                }
                if is_bool || is_bit {
                    for (f, name) in [
                        (Family::RisingEdge, "RISING_EDGE"),
                        (Family::FallingEdge, "FALLING_EDGE"),
                    ] {
                        let s = P {
                            class: ObjClass::Signal,
                            ..named("S", me)
                        };
                        v.push(Spec::new(ty, f, name, vec![s], Some(boolean)).since(v08));
                    }
                }
            }
            Shape::Array { dims, elem } => {
                eq(&mut v);
                let mut to_string_done = false;
                if dims == 1 {
                    let e = self.base(elem);
                    let et = T::Base(e);
                    // 9.2.3: discrete elements; 2019 LCS-2016-059a: any scalar element.
                    let ordered = if self.is_discrete_type(e) {
                        Some(LangRev::Vhdl1993)
                    } else if self.is_scalar_type(e) {
                        Some(LangRev::Vhdl2019)
                    } else {
                        None
                    };
                    if let Some(since) = ordered {
                        for (f, op) in RELATIONAL {
                            v.push(bin(f, op, me, me, boolean).since(since));
                        }
                    }
                    v.push(bin(Family::Cat, "&", me, me, me));
                    v.push(bin(Family::Cat, "&", me, et, me).form(Form::ArrayScalar));
                    v.push(bin(Family::Cat, "&", et, me, me).form(Form::ScalarArray));
                    v.push(bin(Family::Cat, "&", et, et, me).form(Form::ScalarScalar));
                    if let Some(since) = ordered {
                        v.extend(minmax(me, Some(me), me).map(|s| s.since(since.max(v08))));
                    }
                    if self.is_scalar_type(e) {
                        v.extend(minmax(me, None, et).map(|s| s.since(v08)));
                    }
                    if self.is_std(e, "boolean") || self.is_std(e, "bit") {
                        for (f, op) in LOGICAL {
                            v.push(bin(f, op, me, me, me));
                        }
                        v.push(un(Family::Not, "not", me, me));
                        for (f, op) in LOGICAL {
                            v.push(bin(f, op, me, et, me).form(Form::ArrayScalar).since(v08));
                            v.push(bin(f, op, et, me, me).form(Form::ScalarArray).since(v08));
                        }
                        for (f, op) in LOGICAL {
                            v.push(un(f, op, me, et).form(Form::Reduce).since(v08));
                        }
                        for (f, op) in SHIFTS {
                            v.push(bin(f, op, me, integer, me));
                        }
                    }
                    if self.is_std(e, "bit") || self.is_std_ulogic(e) {
                        v.push(bin(Family::MEq, "?=", me, me, et).since(v08));
                        v.push(bin(Family::MNeq, "?/=", me, me, et).since(v08));
                    }
                    if self.is_std(ty, "bit_vector") {
                        let value = || vec![named("VALUE", me)];
                        v.push(Spec {
                            aliases: &["TO_BSTRING", "TO_BINARY_STRING"],
                            ..to_string(v08)
                        });
                        v.push(Spec {
                            aliases: &["TO_OCTAL_STRING"],
                            ..Spec::new(ty, Family::ToOString, "TO_OSTRING", value(), Some(string))
                                .since(v08)
                        });
                        v.push(Spec {
                            aliases: &["TO_HEX_STRING"],
                            ..Spec::new(ty, Family::ToHString, "TO_HSTRING", value(), Some(string))
                                .since(v08)
                        });
                        to_string_done = true;
                    } else if self.is_char_enum(e) {
                        v.push(to_string(v08));
                        to_string_done = true;
                    }
                }
                if !to_string_done && self.is_representable_2019(ty) {
                    v.push(to_string(LangRev::Vhdl2019));
                }
            }
            Shape::Record => {
                eq(&mut v);
                if self.is_representable_2019(ty) {
                    v.push(to_string(LangRev::Vhdl2019));
                }
            }
            Shape::Access => {
                eq(&mut v);
                let p = P {
                    class: ObjClass::Variable,
                    mode: Mode::Inout,
                    ..named("P", me)
                };
                v.push(Spec::new(
                    ty,
                    Family::Deallocate,
                    "DEALLOCATE",
                    vec![p],
                    None,
                ));
            }
            Shape::File { elem } => v.extend(self.file_specs(ty, elem)),
        }
        v
    }

    /// 5.5.2 (2008 and 2019).
    fn file_specs(&self, ty: TypeId, elem: SubtypeId) -> Vec<Spec> {
        let f = || file_param(T::Base(ty));
        let tm = T::Sub(elem);
        let name = || named("External_Name", T::Std("string"));
        let kind = || P {
            default: Some(0), // READ_MODE
            ..named("Open_Kind", T::Std("file_open_kind"))
        };
        let origin = || P {
            default: Some(0), // FILE_ORIGIN_BEGIN
            ..named("Origin", T::Std("file_origin_kind"))
        };
        let integer = T::Std("integer");
        let v19 = LangRev::Vhdl2019;
        let proc = |fam, n, ps| Spec::new(ty, fam, n, ps, None);
        let func = |fam, n, ps, r| Spec::new(ty, fam, n, ps, Some(r));
        let mut read = vec![f(), out("VALUE", tm)];
        if self.is_unconstrained_array(elem) {
            read.push(out("LENGTH", T::Std("natural")));
        }
        vec![
            proc(Family::FileOpen, "FILE_OPEN", vec![f(), name(), kind()]),
            proc(
                Family::FileOpen,
                "FILE_OPEN",
                vec![
                    out("Status", T::Std("file_open_status")),
                    f(),
                    name(),
                    kind(),
                ],
            )
            .form(Form::Status),
            Spec {
                pure: false,
                ..func(
                    Family::FileOpen,
                    "FILE_OPEN",
                    vec![f(), name(), kind()],
                    T::Std("file_open_status"),
                )
                .form(Form::Function)
                .since(v19)
            },
            proc(Family::FileRewind, "FILE_REWIND", vec![f()]).since(v19),
            proc(
                Family::FileSeek,
                "FILE_SEEK",
                vec![f(), named("Offset", integer), origin()],
            )
            .since(v19),
            proc(
                Family::FileTruncate,
                "FILE_TRUNCATE",
                vec![f(), named("Size", integer), origin()],
            )
            .since(v19),
            func(
                Family::FileState,
                "FILE_STATE",
                vec![f()],
                T::Std("file_open_state"),
            )
            .since(v19),
            func(
                Family::FileMode,
                "FILE_MODE",
                vec![f()],
                T::Std("file_open_kind"),
            )
            .since(v19),
            func(
                Family::FilePosition,
                "FILE_POSITION",
                vec![f(), origin()],
                integer,
            )
            .since(v19),
            func(Family::FileSize, "FILE_SIZE", vec![f()], integer).since(v19),
            func(
                Family::FileCanseek,
                "FILE_CANSEEK",
                vec![f()],
                T::Std("boolean"),
            )
            .since(v19),
            proc(Family::FileClose, "FILE_CLOSE", vec![f()]),
            proc(Family::Read, "READ", read),
            proc(Family::Write, "WRITE", vec![f(), named("VALUE", tm)]),
            proc(Family::Flush, "FLUSH", vec![f()]).since(LangRev::Vhdl2008),
            func(Family::Endfile, "ENDFILE", vec![f()], T::Std("boolean")),
        ]
    }
}

/// An identifier with its folded key (rule 8). `anonymous` gets a key no
/// source name can have, so named association cannot reach it.
fn ident(text: &str, span: Span) -> Ident {
    let key = if text == "anonymous" {
        Symbol::intern("<anonymous>")
    } else {
        Symbol::intern(&text.to_ascii_lowercase())
    };
    Ident {
        text: Symbol::intern(text),
        key,
        span,
    }
}
