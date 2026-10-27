//! Literals (sem.md 3.5, decision Q11).
//!
//! Two layers. The pure layer ([`abstract_literal`], [`expand_bitstring`]) works on source
//! text only and is type independent. The typed layer ([`literal_expr`] and the functions it
//! calls) turns a literal and its resolved context type into a constant `Expr`, with the precise
//! diagnostics of sem.md 8.1: `E1330` (character not in the type), `E1331` (bit string would lose
//! digits), `E1332` (malformed), `E1402` (too large), `E1401` (index arithmetic overflow).
//!
//! Characters are Latin-1 values (decisions.md, source encoding): a character above U+00FF is
//! never a literal of any type and is reported as `E1330`.
//!
//! Decimal parsing reuses `yavs_std::value::lex_abstract`, the same lexer `T'VALUE` uses.

use yavs_diag::{Diagnostic, Span};
use yavs_std::error::{ErrorKind, What};
use yavs_std::value::{Abstract, lex_abstract};
use yavs_syntax::Symbol;
use yavs_syntax::ast::{Direction, Ident, Literal, LiteralKind};

use crate::db::{Db, Reported};
use crate::decl::Designator;
use crate::diag;
use crate::hir::{Expr, ExprKind};
use crate::ids::{ExprId, ScopeId, SubtypeId, TypeId};
use crate::scope::Visible;
use crate::stat::{ArrayConst, ConstValue, Elems, Staticness};
use crate::types::{Constraint, RangeC, TypeKind};

/// Longest expanded bit string accepted.
// ponytail (#42): guards `8_000_000_000X"1"` from allocating gigabytes; lift with run-length elements.
const MAX_BITSTRING_LEN: u64 = 1 << 26;

// ---------------------------------------------------------------------------
// Abstract literals

/// The value of a decimal or based literal, before the context type is known.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Abs {
    Int(i64),
    Real(f64),
}

/// Why an abstract literal has no value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AbsError {
    /// `E1332`
    Malformed(String),
    /// `E1402`
    TooLarge(&'static str),
}

fn lex(text: &str) -> Result<Abstract, AbsError> {
    let b = text.as_bytes();
    let what = if text.contains('.') {
        What::Real
    } else {
        What::Integer
    };
    match lex_abstract(b, 0, what) {
        Ok((a, end)) if end == b.len() => Ok(a),
        Ok(_) => Err(AbsError::Malformed(format!(
            "`{text}` is not a single abstract literal"
        ))),
        Err(e) => match e.kind {
            ErrorKind::Overflow => Err(AbsError::TooLarge("the exponent")),
            ErrorKind::Syntax { expected } => Err(AbsError::Malformed(format!(
                "`{text}`: expected {expected}"
            ))),
            _ => Err(AbsError::Malformed(format!("`{text}` is malformed"))),
        },
    }
}

/// Value of a decimal or based literal: integer (`i64`, checked) or real (nearest f64).
pub fn abstract_literal(text: &str) -> Result<Abs, AbsError> {
    let a = lex(text)?;
    if a.real {
        let v = a.to_f64(); // correctly rounded in every base
        if v.is_finite() {
            return Ok(Abs::Real(v));
        }
        return Err(AbsError::TooLarge("a real literal"));
    }
    if a.exp < 0 {
        return Err(AbsError::Malformed(format!(
            "`{text}`: an integer literal cannot have a negative exponent"
        )));
    }
    a.to_i128()
        .and_then(|v| i64::try_from(v).ok())
        .map(Abs::Int)
        .ok_or(AbsError::TooLarge("an integer literal"))
}

fn abs_error(db: &mut Db, e: AbsError, span: Span) -> Reported {
    match e {
        AbsError::Malformed(m) => db.error(
            Diagnostic::error(diag::BAD_LITERAL, "malformed literal")
                .with_primary(span, m)
                .with_note("based literals use bases 2 to 16 and digits of that base"),
        ),
        AbsError::TooLarge(what) => db.error(
            Diagnostic::error(diag::LITERAL_RANGE, format!("{what} is too large"))
                .with_primary(span, "does not fit")
                .with_note("integer literals are 64-bit signed; real literals must be finite in 64-bit floating point"),
        ),
    }
}

/// Value of an abstract literal, reporting `E1332`/`E1402` at `span`.
pub fn abstract_value(db: &mut Db, text: Symbol, span: Span) -> Result<Abs, Reported> {
    abstract_literal(text.as_str()).map_err(|e| abs_error(db, e, span))
}

// ---------------------------------------------------------------------------
// Physical literals

/// `value` in base units of `unit_scale`, rounded to the nearest base unit (ties away from zero).
pub fn physical_literal(value: &str, unit_scale: i64) -> Result<i64, AbsError> {
    let a = lex(value)?;
    a.scaled(unit_scale)
        .and_then(|v| i64::try_from(v).ok())
        .ok_or(AbsError::TooLarge("a physical literal"))
}

// ---------------------------------------------------------------------------
// Bit strings (LRM 15.8)

/// Why a bit string cannot be expanded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BitStringError {
    /// `E1332`: not of the form `[length] base "digits"`.
    Malformed(String),
    /// `E1332`: a character that is not a decimal digit in a `D` literal, at this byte offset.
    BadDecimalDigit { offset: usize, ch: char },
    /// `E1330`: a character above U+00FF, at this byte offset.
    NotLatin1 { offset: usize, ch: char },
    /// `E1402`: the length or the expansion is too large.
    TooLong,
    /// `E1331`: `dropped` characters were removed to reach the length; `expected` is what they
    /// had to be (`'0'` for unsigned, the leftmost kept character for signed).
    Lost {
        dropped: Vec<u8>,
        expected: u8,
        signed: bool,
    },
    /// `E1331`: a signed bit string with a length but no digits.
    EmptySigned,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sign {
    Unsigned,
    Signed,
}

/// Expand a bit string literal as written (`8x"F_F"`, `SB%101%`) to its characters (Latin-1 bytes)
/// by LRM 15.8: each digit of the base becomes 1, 3 or 4 binary digits, any other graphic
/// character is replicated that many times, `D` is converted to the shortest binary number
/// (zero is the empty string), then the length is adjusted: unsigned extends with `0` and may only
/// drop `0`; signed extends with the leftmost character and may only drop characters equal to the
/// leftmost one kept. Plain `B`/`O`/`X`/`D` are unsigned. This follows sim2; sim1 only requires the
/// dropped characters to equal each other.
pub fn expand_bitstring(text: &str) -> Result<Vec<u8>, BitStringError> {
    use BitStringError::*;
    let b = text.as_bytes();
    let bad = |m: &str| Malformed(format!("`{text}`: {m}"));
    let mut i = 0;
    let mut len: Option<u64> = None;
    while i < b.len() && (b[i].is_ascii_digit() || (b[i] == b'_' && i > 0)) {
        if b[i] != b'_' {
            let d = u64::from(b[i] - b'0');
            len = Some(
                len.unwrap_or(0)
                    .checked_mul(10)
                    .and_then(|v| v.checked_add(d))
                    .ok_or(TooLong)?,
            );
        }
        i += 1;
    }
    let spec_start = i;
    while i < b.len() && b[i].is_ascii_alphabetic() {
        i += 1;
    }
    let spec = text[spec_start..i].to_ascii_lowercase();
    let (sign, base) = match spec.as_str() {
        "b" | "o" | "x" | "d" => (Sign::Unsigned, spec.as_str()),
        "ub" | "uo" | "ux" => (Sign::Unsigned, &spec[1..]),
        "sb" | "so" | "sx" => (Sign::Signed, &spec[1..]),
        _ => return Err(bad("unknown base specifier")),
    };
    let q = *b.get(i).ok_or_else(|| bad("missing opening quote"))?;
    if !matches!(q, b'"' | b'%') || b.len() < i + 2 || b[b.len() - 1] != q {
        return Err(bad("expected a quoted digit string"));
    }
    let body = &text[i + 1..text.len() - 1];

    // Characters of the body, underscores dropped, doubled quotes undone.
    let mut chars: Vec<(usize, char)> = Vec::new();
    let mut it = body.char_indices().peekable();
    while let Some((o, c)) = it.next() {
        let off = i + 1 + o;
        if c == '_' {
            continue;
        }
        if c == q as char && it.peek().is_some_and(|&(_, n)| n == c) {
            it.next();
        }
        if u32::from(c) > 255 {
            return Err(NotLatin1 { offset: off, ch: c });
        }
        chars.push((off, c));
    }
    if chars.len() as u64 > MAX_BITSTRING_LEN {
        return Err(TooLong);
    }

    let mut out: Vec<u8> = Vec::new();
    if base == "d" {
        let mut digits = Vec::with_capacity(chars.len());
        for &(offset, ch) in &chars {
            match ch.to_digit(10) {
                Some(d) => digits.push(d as u8),
                None => return Err(BadDecimalDigit { offset, ch }),
            }
        }
        out = decimal_to_binary(digits);
    } else {
        let (n, radix) = match base {
            "b" => (1, 2),
            "o" => (3, 8),
            _ => (4, 16),
        };
        for &(_, c) in &chars {
            match c.to_digit(radix) {
                Some(d) => out.extend((0..n).rev().map(|k| b'0' + ((d >> k) & 1) as u8)),
                None => out.extend(std::iter::repeat_n(c as u8, n)),
            }
        }
    }

    let Some(len) = len else {
        return Ok(out);
    };
    if len > MAX_BITSTRING_LEN {
        return Err(TooLong);
    }
    let len = len as usize;
    if out.len() <= len {
        let fill = match (sign, out.first()) {
            (Sign::Unsigned, _) => b'0',
            (Sign::Signed, Some(&c)) => c,
            (Sign::Signed, None) if len == 0 => return Ok(out),
            (Sign::Signed, None) => return Err(EmptySigned),
        };
        let mut v = vec![fill; len - out.len()];
        v.extend_from_slice(&out);
        return Ok(v);
    }
    let cut = out.len() - len;
    let expected = match sign {
        Sign::Unsigned => b'0',
        // The kept part is empty only for length 0: nothing to compare, the sign is the first.
        Sign::Signed => out.get(cut).copied().unwrap_or(out[0]),
    };
    if out[..cut].iter().any(|&c| c != expected) {
        return Err(Lost {
            dropped: out[..cut].to_vec(),
            expected,
            signed: sign == Sign::Signed,
        });
    }
    Ok(out.split_off(cut))
}

/// Shortest binary digits (`0`/`1` bytes) of a decimal digit string; zero is empty.
fn decimal_to_binary(mut digits: Vec<u8>) -> Vec<u8> {
    let mut bits = Vec::new();
    loop {
        let start = digits.iter().position(|&d| d != 0).unwrap_or(digits.len());
        digits.drain(..start);
        if digits.is_empty() {
            break;
        }
        let mut rem = 0u8;
        for d in &mut digits {
            let v = rem * 10 + *d;
            *d = v / 2;
            rem = v % 2;
        }
        bits.push(b'0' + rem);
    }
    bits.reverse();
    bits
}

// ---------------------------------------------------------------------------
// Typed layer

fn type_name(db: &Db, ty: TypeId) -> String {
    db.types[ty].name.text.as_str().to_string()
}

fn mismatch(db: &mut Db, span: Span, what: &str, ty: TypeId, need: &str) -> Reported {
    let name = type_name(db, ty);
    db.error(
        Diagnostic::error(
            diag::TYPE_MISMATCH,
            format!("{what} cannot have type `{name}`"),
        )
        .with_primary(span, format!("{what} needs {need}")),
    )
}

/// The position of character literal `c` in enumeration type `ty`.
fn enum_pos(db: &Db, ty: TypeId, c: char) -> Option<u32> {
    let TypeKind::Enum { lits } = &db.types[ty].kind else {
        return None;
    };
    lits.iter()
        .position(|&l| db.decls[l].name == Designator::Char(c))
        .map(|p| p as u32)
}

fn bad_char(db: &mut Db, span: Span, c: char, ty: TypeId) -> Reported {
    let name = type_name(db, ty);
    let mut d = Diagnostic::error(
        diag::CHAR_NOT_IN_TYPE,
        format!("character '{c}' is not a literal of type `{name}`"),
    )
    .with_primary(span, format!("'{c}' is not in `{name}`"));
    if u32::from(c) > 255 {
        d = d.with_note("characters are Latin-1; this character is above U+00FF");
    }
    db.error(d)
}

/// A character literal `'c'` of enumeration type `ty`: `ConstValue::Enum(pos)`. `E1330` if `ty`
/// has no such literal.
pub fn char_literal(db: &mut Db, c: char, ty: TypeId, span: Span) -> Result<ConstValue, Reported> {
    if matches!(db.types[ty].kind, TypeKind::Error) {
        return Err(Reported);
    }
    if !matches!(db.types[ty].kind, TypeKind::Enum { .. }) {
        return Err(mismatch(
            db,
            span,
            "a character literal",
            ty,
            "an enumeration type",
        ));
    }
    match enum_pos(db, ty, c) {
        Some(p) => Ok(ConstValue::Enum(p)),
        None => Err(bad_char(db, span, c, ty)),
    }
}

/// `(left, direction, right)` of an index subtype, when its bounds are folded.
fn index_range(db: &Db, st: SubtypeId) -> Option<(i64, Direction, i64)> {
    match &db.subtypes[st].constraint {
        Constraint::Range(RangeC::Explicit { left, dir, right }) => Some((
            db.value(*left)?.as_int()?,
            *dir,
            db.value(*right)?.as_int()?,
        )),
        Constraint::None => match &db.types[db.subtypes[st].base].kind {
            TypeKind::Enum { lits } => Some((0, Direction::To, lits.len() as i64 - 1)),
            _ => None,
        },
        _ => None,
    }
}

/// Characters (Latin-1 bytes) as a string constant of array type `ty`. `col(i)` gives the
/// source offset of character `i` for the span of an `E1330`.
fn array_of_chars(
    db: &mut Db,
    chars: &[u8],
    ty: TypeId,
    span: Span,
    col: &dyn Fn(usize) -> u32,
) -> Result<ConstValue, Reported> {
    if matches!(db.types[ty].kind, TypeKind::Error) {
        return Err(Reported);
    }
    let (index, elem_ty) = match &db.types[ty].kind {
        TypeKind::Array { indexes, elem } if indexes.len() == 1 => {
            (indexes[0], db.subtypes[*elem].base)
        }
        _ => {
            return Err(mismatch(
                db,
                span,
                "a string literal",
                ty,
                "a one-dimensional array of a character type",
            ));
        }
    };
    let TypeKind::Enum { lits } = &db.types[elem_ty].kind else {
        let n = type_name(db, elem_ty);
        return Err(db.error(
            Diagnostic::error(
                diag::CHAR_NOT_IN_TYPE,
                format!("`{n}` has no character literals"),
            )
            .with_primary(
                span,
                format!("a string literal of type `{}`", type_name(db, ty)),
            )
            .with_note("a string literal needs an array type whose elements are characters"),
        ));
    };
    let mut table: [Option<u8>; 256] = [None; 256];
    for (pos, &l) in lits.iter().enumerate() {
        if let Designator::Char(c) = db.decls[l].name
            && (c as u32) < 256
            && let Ok(p) = u8::try_from(pos)
        {
            table[c as usize] = Some(p);
        }
    }
    let mut elems = Vec::with_capacity(chars.len());
    for (i, &c) in chars.iter().enumerate() {
        match table[c as usize] {
            Some(p) => elems.push(p),
            None => {
                let at = Span::new(span.file, col(i), col(i) + 1);
                return Err(bad_char(db, at, c as char, elem_ty));
            }
        }
    }
    let Some((left, dir, _)) = index_range(db, index) else {
        return Err(db.unsupported(
            span,
            "a string literal over an index subtype with unfolded bounds",
            "S2",
        ));
    };
    let n = elems.len() as i64;
    let overflow = |db: &mut Db| {
        db.error(
            Diagnostic::error(diag::OVERFLOW, "string literal bounds overflow 64 bits")
                .with_primary(span, "index arithmetic overflows"),
        )
    };
    // Null strings: right = left -/+ 1, an empty range whatever the index subtype.
    let step = |a: i64, k: i64| match dir {
        Direction::To => a.checked_add(k),
        Direction::Downto => a.checked_sub(k),
    };
    let right = step(left, n - 1).ok_or_else(|| overflow(db))?;
    if n > 0
        && let Some((il, idir, ir)) = index_range(db, index)
    {
        let (lo, hi) = if idir == Direction::To {
            (il, ir)
        } else {
            (ir, il)
        };
        if right < lo || right > hi || left < lo || left > hi {
            let name = type_name(db, ty);
            return Err(db.error(
                Diagnostic::error(
                    diag::OUT_OF_SUBTYPE,
                    format!("string literal of length {n} does not fit the index of `{name}`"),
                )
                .with_primary(span, format!("needs index {left} to {right}"))
                .with_note(format!("the index subtype runs {il} to {ir}")),
            ));
        }
    }
    Ok(ConstValue::Array(ArrayConst {
        bounds: Box::new([(left, dir, right)]),
        elems: Elems::Bytes(elems.into_boxed_slice()),
    }))
}

/// A string literal (contents without quotes, doubled quotes undone) of array type `ty`.
pub fn string_literal(
    db: &mut Db,
    text: Symbol,
    ty: TypeId,
    span: Span,
) -> Result<ConstValue, Reported> {
    let s = text.as_str();
    // Per character: its Latin-1 value (or a poisoned 0xFF-free marker below) and source column.
    let mut bytes = Vec::with_capacity(s.len());
    let mut cols = Vec::with_capacity(s.len());
    let mut at = span.start + 1;
    for c in s.chars() {
        cols.push(at);
        // A doubled quote is written as two characters.
        at += c.len_utf8() as u32 + u32::from(c == '"');
        match u8::try_from(u32::from(c)) {
            Ok(b) => bytes.push(b),
            Err(_) => {
                let here = Span::new(span.file, cols[cols.len() - 1], at);
                return Err(bad_char(db, here, c, ty));
            }
        }
    }
    array_of_chars(db, &bytes, ty, span, &|i| {
        cols[i].min(span.end.saturating_sub(1))
    })
}

/// A bit string literal as written, of array type `ty`: [`expand_bitstring`], then as a string.
pub fn bit_string_literal(
    db: &mut Db,
    text: Symbol,
    ty: TypeId,
    span: Span,
) -> Result<ConstValue, Reported> {
    match expand_bitstring(text.as_str()) {
        // The expansion has no source columns; point at the whole literal.
        Ok(chars) => array_of_chars(db, &chars, ty, span, &|_| span.start),
        Err(e) => Err(bit_string_error(db, e, span)),
    }
}

fn bit_string_error(db: &mut Db, e: BitStringError, span: Span) -> Reported {
    let at = |o: usize| Span::new(span.file, span.start + o as u32, span.start + o as u32 + 1);
    match e {
        BitStringError::Malformed(m) => db.error(
            Diagnostic::error(diag::BAD_LITERAL, "malformed bit string literal")
                .with_primary(span, m),
        ),
        BitStringError::BadDecimalDigit { offset, ch } => db.error(
            Diagnostic::error(
                diag::BAD_LITERAL,
                format!("invalid digit '{ch}' in a decimal bit string"),
            )
            .with_primary(at(offset), "not a decimal digit")
            .with_note("a `D` bit string holds decimal digits and underscores only"),
        ),
        BitStringError::NotLatin1 { offset, ch } => db.error(
            Diagnostic::error(
                diag::CHAR_NOT_IN_TYPE,
                format!("character '{ch}' is not in the Latin-1 character set"),
            )
            .with_primary(at(offset), "above U+00FF"),
        ),
        BitStringError::TooLong => db.error(
            Diagnostic::error(diag::LITERAL_RANGE, "bit string literal is too long")
                .with_primary(span, "length or expansion too large")
                .with_note(format!(
                    "at most {MAX_BITSTRING_LEN} characters are supported"
                )),
        ),
        BitStringError::Lost {
            dropped,
            expected,
            signed,
        } => {
            let d = String::from_utf8_lossy(&dropped).into_owned();
            let rule = if !signed {
                "an unsigned bit string may only lose leading `0` characters".to_string()
            } else {
                format!(
                    "a signed bit string may only lose characters equal to its sign character '{}'",
                    expected as char
                )
            };
            db.error(
                Diagnostic::error(
                    diag::BITSTRING_DIGITS,
                    "bit string literal would lose significant digits",
                )
                .with_primary(span, format!("the length drops \"{d}\""))
                .with_note(rule),
            )
        }
        BitStringError::EmptySigned => db.error(
            Diagnostic::error(
                diag::BITSTRING_DIGITS,
                "an empty signed bit string cannot be extended",
            )
            .with_primary(span, "no sign character to replicate"),
        ),
    }
}

/// A physical literal `value unit` of physical type `ty`: `ConstValue::Phys` in base units.
pub fn physical_value(
    db: &mut Db,
    value: Symbol,
    unit: Ident,
    ty: TypeId,
    span: Span,
) -> Result<ConstValue, Reported> {
    if matches!(db.types[ty].kind, TypeKind::Error) {
        return Err(Reported);
    }
    let scale = match &db.types[ty].kind {
        TypeKind::Physical { units } => units
            .iter()
            .find(|u| db.decls[u.decl].name.key() == unit.key)
            .map(|u| u.scale),
        _ => {
            return Err(mismatch(
                db,
                span,
                "a physical literal",
                ty,
                "a physical type",
            ));
        }
    };
    let Some(scale) = scale else {
        let name = type_name(db, ty);
        return Err(db.error(
            Diagnostic::error(
                diag::BAD_LITERAL,
                format!("`{}` is not a unit of `{name}`", unit.text),
            )
            .with_primary(unit.span, "no such unit"),
        ));
    };
    let zero = lex(value.as_str()).is_ok_and(|a| a.digits.iter().all(|&d| d == 0));
    db.note_time_unit(ty, unit.key, scale, zero, span);
    physical_literal(value.as_str(), scale)
        .map(ConstValue::Phys)
        .map_err(|e| abs_error(db, e, span))
}

/// The character literals a string or bit string value uses must be visible at the literal
/// (LRM 9.3.2: each character denotes a literal of the element type; 12.3).
fn chars_visible(
    db: &mut Db,
    scope: ScopeId,
    ty: TypeId,
    v: &ConstValue,
    span: Span,
) -> Result<(), Reported> {
    let (
        ConstValue::Array(ArrayConst {
            elems: Elems::Bytes(b),
            ..
        }),
        TypeKind::Array { elem, .. },
    ) = (v, &db.types[ty].kind)
    else {
        return Ok(());
    };
    let et = db.subtypes[*elem].base;
    let TypeKind::Enum { lits } = &db.types[et].kind else {
        return Ok(());
    };
    let (lits, mut seen) = (lits.clone(), [false; 256]);
    for &p in b.iter() {
        if std::mem::replace(&mut seen[p as usize], true) {
            continue;
        }
        let lit = lits[p as usize];
        let found = match db.lookup(scope, db.decls[lit].name.key()) {
            Visible::One(d) => db.entity(d) == lit,
            Visible::Overloads(ds) => ds.iter().any(|&d| db.entity(d) == lit),
            _ => false,
        };
        if !found {
            let name = type_name(db, et);
            let region = crate::mangle::region(db, db.decls[db.types[et].decl].scope);
            return Err(db.error(
                Diagnostic::error(
                    diag::NOT_DECLARED,
                    format!("the character literal {} of `{name}` is not visible here", db.decls[lit].name),
                )
                .with_primary(span, format!("this literal needs the characters of `{name}`"))
                .with_note(
                    "each character of a string or bit string literal is a literal of the element type, and must be visible where the literal is written (LRM 9.3.2, 12.3)",
                )
                .with_help(format!("add a use clause that makes `{name}` visible, for example `use {region}.all;`")),
            ));
        }
    }
    Ok(())
}

/// The folded value of any literal for context type `ty` (`ty` is `universal_*` for abstract
/// literals typed by themselves). `null` has no constant; use [`literal_expr`].
pub fn literal_value(
    db: &mut Db,
    scope: ScopeId,
    lit: &Literal,
    ty: TypeId,
) -> Result<Option<ConstValue>, Reported> {
    let span = lit.span;
    Ok(Some(match &lit.kind {
        LiteralKind::Abstract(t) => {
            let class = db.types[ty].kind.class();
            let v = abstract_value(db, *t, span)?;
            match (v, class) {
                (_, crate::types::TypeClass::Error) => return Err(Reported),
                (Abs::Int(i), crate::types::TypeClass::Integer) => ConstValue::Int(i),
                (Abs::Real(r), crate::types::TypeClass::Floating) => ConstValue::Real(r),
                (Abs::Int(_), _) => {
                    return Err(mismatch(
                        db,
                        span,
                        "an integer literal",
                        ty,
                        "an integer type",
                    ));
                }
                (Abs::Real(_), _) => {
                    return Err(mismatch(
                        db,
                        span,
                        "a real literal",
                        ty,
                        "a floating point type",
                    ));
                }
            }
        }
        LiteralKind::Char(c) => char_literal(db, *c, ty, span)?,
        LiteralKind::String(s) => {
            let v = string_literal(db, *s, ty, span)?;
            chars_visible(db, scope, ty, &v, span)?;
            v
        }
        LiteralKind::BitString(s) => {
            let v = bit_string_literal(db, *s, ty, span)?;
            chars_visible(db, scope, ty, &v, span)?;
            v
        }
        LiteralKind::Physical { value, unit } => physical_value(db, *value, *unit, ty, span)?,
        LiteralKind::Null => return Ok(None),
    }))
}

/// The value of `-lit` when the literal alone is one past `i64::MAX` (LRM 9.3, 5.2.3: the minus
/// applies to a `universal_integer` literal; only library `std` uses it, for `TIME`) and `ty` is an
/// integer or physical type; `None` for every other literal (the normal path handles and reports).
// ponytail: only the i64 minimum needs the wider intermediate; any other value folds via `-` as before.
pub fn negated_min(db: &Db, lit: &Literal, ty: TypeId) -> Option<ConstValue> {
    const MAG: i128 = 1 << 63;
    match (&lit.kind, &db.types[ty].kind) {
        (LiteralKind::Abstract(t), k) if k.class() == crate::types::TypeClass::Integer => {
            let a = lex(t.as_str()).ok()?;
            (a.to_i128() == Some(MAG)).then_some(ConstValue::Int(i64::MIN))
        }
        (LiteralKind::Physical { value, unit }, TypeKind::Physical { units }) => {
            let scale = units
                .iter()
                .find(|u| db.decls[u.decl].name.key() == unit.key)?
                .scale;
            let a = lex(value.as_str()).ok()?;
            (a.scaled(scale) == Some(MAG)).then_some(ConstValue::Phys(i64::MIN))
        }
        _ => None,
    }
}

/// Build the constant HIR expression of a literal of type `ty` (sem.md 3.5, "Resolved with type T").
pub fn literal_expr(
    db: &mut Db,
    scope: ScopeId,
    lit: &Literal,
    ty: TypeId,
) -> Result<ExprId, Reported> {
    let v = literal_value(db, scope, lit, ty)?;
    Ok(const_expr(db, lit.span, ty, v))
}

/// The expression of an already folded literal value (`None` is `null`).
pub fn const_expr(db: &mut Db, span: Span, ty: TypeId, v: Option<ConstValue>) -> ExprId {
    let subtype = db.types[ty].first_subtype;
    let (kind, value) = match v {
        Some(v) => {
            let c = db.add_const(v);
            (ExprKind::Const(c), Some(c))
        }
        None => (ExprKind::Null, None),
    };
    db.add_expr(Expr {
        kind,
        subtype,
        span,
        stat: Staticness::Locally,
        value,
    })
}

#[cfg(test)]
pub(crate) mod tests;
