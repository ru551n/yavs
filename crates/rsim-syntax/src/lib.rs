//! VHDL-2008 and VHDL-2019 lexer and parser.
//!
//! [`parse_file`] lexes a file, evaluates conditional analysis directives
//! (`` `if ``, see [`directive`]), and parses the tokens into an owned
//! [`ast::DesignFile`]. The parser recovers from errors so that one run
//! reports every independent syntax error of a file, but it only hands out
//! the tree when there were none: later stages never see a guessed AST.
//!
//! # Shape of the tree
//!
//! The AST is purely syntactic and is meant to be consumed by semantic
//! analysis. VHDL cannot be fully disambiguated without declarations, so the
//! tree keeps ambiguous forms ambiguous instead of guessing:
//!
//! - A function call, an indexed name, a slice and a type conversion are all
//!   [`ast::NameKind::Apply`]: a prefix name plus an association list. Only
//!   semantic analysis knows whether `f(x)` calls `f` or indexes it.
//! - A bare name used as a discrete range is [`ast::Range::Name`], which may
//!   be a type mark (`for i in t loop`) or a range attribute (`a'range`).
//! - Choices and generic actuals that look like expressions may name subtypes.
//! - `label: p;` in a concurrent region is a procedure call; it may be a
//!   component instantiation without maps.
//!
//! Identifiers keep their original spelling next to a case-folded lookup key
//! (see [`ast::Ident`]). PSL is not supported: it is reported and skipped.

pub mod ast;
pub mod directive;
mod lexer;
mod parser;
mod symbol;
pub mod token;

pub use symbol::Symbol;

use rsim_diag::{Diagnostic, FileId, Severity};

/// The language standard a file is analyzed under.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Std {
    #[default]
    Vhdl2008,
    Vhdl2019,
}

impl Std {
    /// The value of `VHDL_VERSION`.
    pub fn as_str(self) -> &'static str {
        match self {
            Std::Vhdl2008 => "2008",
            Std::Vhdl2019 => "2019",
        }
    }
}

/// Stable diagnostic codes of this crate.
pub mod codes {
    /// Invalid character, unterminated literal or comment, bad number.
    pub const LEX: &str = "E0001";
    /// A token the grammar does not allow here.
    pub const SYNTAX: &str = "E0002";
    /// An end label that does not repeat the construct's name.
    pub const END_NAME: &str = "E0003";
    /// A VHDL-2019 construct used in VHDL-2008 mode.
    pub const NEEDS_2019: &str = "E0004";
    /// PSL, which rsim does not support.
    pub const PSL: &str = "E0005";
    /// Nesting beyond the parser's limit.
    pub const TOO_DEEP: &str = "E0006";
    /// Malformed conditional analysis directive.
    pub const DIRECTIVE: &str = "E0007";
    /// A `` `error `` directive.
    pub const USER_ERROR: &str = "E0008";
    /// Internal compiler error.
    pub const INTERNAL: &str = "E9999";
    /// A `` `warning `` directive.
    pub const USER_WARNING: &str = "W0001";
    /// A tool directive rsim does not know, ignored.
    pub const UNKNOWN_DIRECTIVE: &str = "W0002";
    /// A number directly followed by an identifier, as in `10ns`.
    pub const NO_SEPARATOR: &str = "W0003";
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub std: Std,
    /// User conditional analysis identifiers, `(name, value)`.
    pub defines: Vec<(String, String)>,
}

pub struct Parsed {
    /// The tree, present only when there were no errors.
    pub design: Option<ast::DesignFile>,
    pub diags: Vec<Diagnostic>,
}

/// Lex, preprocess and parse one source file.
pub fn parse_file(file: FileId, src: &str, opts: &Options) -> Parsed {
    let mut diags = Vec::new();
    let toks = lexer::lex(file, src, opts.std, &mut diags);
    let toks = directive::preprocess(file, src, toks, opts.std, &opts.defines, &mut diags);
    let design = parser::parse(file, src, toks, opts.std, &mut diags);
    let ok = !diags.iter().any(|d| d.severity == Severity::Error);
    Parsed {
        design: ok.then_some(design),
        diags,
    }
}

/// Decode a source file. VHDL source is ISO-8859-1, but UTF-8 is common in
/// comments and strings, so valid UTF-8 is taken as such and anything else is
/// decoded as Latin-1, which never fails.
pub fn decode_source(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => e.into_bytes().iter().map(|&b| b as char).collect(),
    }
}

#[cfg(test)]
mod tests;
