//! Structured conversion errors.
//!
//! Every fallible function in this crate returns a [`ConvError`]: what went
//! wrong ([`ErrorKind`]), the offending input text, the byte span inside it
//! and the form that was expected. `yavs-diag` renders it; this crate never
//! panics and never clamps or defaults a value.

use std::fmt;
use std::ops::Range;

/// What kind of value was being converted. Used in messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum What {
    Integer,
    Real,
    Physical,
    Enumeration,
    BitVector,
    String,
    Format,
    Unit,
    Aggregate,
}

impl What {
    pub fn noun(self) -> &'static str {
        match self {
            What::Integer => "integer",
            What::Real => "real",
            What::Physical => "physical",
            What::Enumeration => "enumeration",
            What::BitVector => "bit_vector",
            What::String => "string",
            What::Format => "format",
            What::Unit => "unit",
            What::Aggregate => "aggregate",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// The input is empty or only whitespace.
    Empty,
    /// The text does not have the expected lexical form.
    Syntax { expected: &'static str },
    /// A valid value is followed by more, non-whitespace, text.
    TrailingText,
    /// The literal is too large for the implementation (64-bit or f64).
    Overflow,
    /// The value is outside the bounds of the target subtype.
    OutOfRange { low: String, high: String },
    /// A physical literal names a unit the type does not have.
    UnknownUnit,
    /// The text names no literal of the enumeration type.
    UnknownLiteral,
    /// The input ended before the expected number of characters or digits.
    TooShort { expected: usize, found: usize },
    /// A value that does not fit the target width (hread/oread high bits).
    DoesNotFit,
    /// An array value with another number of elements than its type.
    Length { expected: u64, found: u64 },
    /// An aggregate that gives no value for an element, or two.
    Element { what: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvError {
    pub what: What,
    pub kind: ErrorKind,
    /// The whole input the conversion was given (Latin-1).
    pub input: Vec<u8>,
    /// Byte range of the offending part of `input`.
    pub span: Range<usize>,
}

impl ConvError {
    pub fn new(what: What, kind: ErrorKind, input: &[u8], span: Range<usize>) -> Self {
        let span = span.start.min(input.len())..span.end.min(input.len());
        ConvError {
            what,
            kind,
            input: input.to_vec(),
            span,
        }
    }

    /// The offending text, as Latin-1 decoded to a Rust string.
    pub fn offending(&self) -> String {
        latin1(&self.input[self.span.clone()])
    }
}

/// Decode Latin-1 bytes to a `String` (every byte is one char).
pub fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

impl fmt::Display for ConvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let w = self.what.noun();
        let input = latin1(&self.input);
        match &self.kind {
            ErrorKind::Empty => write!(f, "empty {w} value \"{input}\""),
            ErrorKind::Syntax { expected } => write!(
                f,
                "invalid {w} value \"{input}\" at \"{}\": expected {expected}",
                self.offending()
            ),
            ErrorKind::TrailingText => write!(
                f,
                "unexpected text \"{}\" after {w} value in \"{input}\"",
                self.offending()
            ),
            ErrorKind::Overflow => write!(
                f,
                "{w} value \"{}\" is too large to represent",
                self.offending()
            ),
            ErrorKind::OutOfRange { low, high } => write!(
                f,
                "{w} value \"{}\" is outside the range {low} to {high}",
                self.offending()
            ),
            ErrorKind::UnknownUnit => {
                write!(f, "\"{}\" is not a unit of the type", self.offending())
            }
            ErrorKind::UnknownLiteral => write!(
                f,
                "\"{}\" is not a literal of the enumeration type",
                self.offending()
            ),
            ErrorKind::TooShort { expected, found } => write!(
                f,
                "{w} value \"{input}\" is too short: expected {expected}, found {found}"
            ),
            ErrorKind::Length { expected, found } => write!(
                f,
                "{w} value {input} has {found} elements, expected {expected}"
            ),
            ErrorKind::Element { what } => write!(f, "{w} \"{input}\": {what}"),
            ErrorKind::DoesNotFit => write!(
                f,
                "{w} value \"{}\" does not fit the target width",
                self.offending()
            ),
        }
    }
}

impl std::error::Error for ConvError {}
