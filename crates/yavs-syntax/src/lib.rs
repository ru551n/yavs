//! VHDL-2008 and VHDL-2019 lexer and parser.
//!
//! [`load_source`] reads a file's bytes per [`Encoding`] into a `SourceMap`;
//! [`parse_file`] lexes it, evaluates conditional analysis directives
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
//! (see [`ast::Ident`]). PSL runs as checker processes for a
//! simple subset; the rest is skipped with a warning (`W0004`).

pub mod ast;
pub mod directive;
mod lexer;
mod parser;
pub mod protect;
mod source;
mod symbol;
pub mod token;

pub use symbol::{DecodeTable, Symbol, SymbolTable, encode_table, raw_symbols};

use yavs_diag::{Diagnostic, FileId, Severity, SourceFile, SourceMap, Span};

/// The language standard a file is analyzed under.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum Std {
    Vhdl1993,
    #[default]
    Vhdl2008,
    Vhdl2019,
}

impl Std {
    /// The value of `VHDL_VERSION`.
    pub fn as_str(self) -> &'static str {
        match self {
            Std::Vhdl1993 => "1993",
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
    // E0005 (PSL is not supported) is retired: PSL is skipped with W0004. Never reuse it.
    /// Nesting beyond the parser's limit.
    pub const TOO_DEEP: &str = "E0006";
    /// Malformed conditional analysis directive.
    pub const DIRECTIVE: &str = "E0007";
    /// A `` `error `` directive.
    pub const USER_ERROR: &str = "E0008";
    /// A character outside ISO-8859-1 in a literal or identifier of a UTF-8 file.
    pub const NOT_LATIN1: &str = "E0009";
    /// A file that is not valid UTF-8 under `--encoding=utf8`.
    pub const INVALID_UTF8: &str = "E0010";
    /// A VHDL-2008 construct used in VHDL-1993 mode.
    pub const NEEDS_2008: &str = "E0011";
    /// An encrypted (IEEE 1735 protected) envelope: yavs cannot read the file.
    pub const ENCRYPTED: &str = "E0012";
    /// Internal compiler error.
    pub const INTERNAL: &str = "E9999";
    /// A `` `warning `` directive.
    pub const USER_WARNING: &str = "W0001";
    /// A tool directive yavs does not know, ignored.
    pub const UNKNOWN_DIRECTIVE: &str = "W0002";
    /// A number directly followed by an identifier, as in `10ns`.
    pub const NO_SEPARATOR: &str = "W0003";
    /// PSL declarations and directives outside the subset in a design unit, skipped.
    pub const PSL_SKIPPED: &str = "W0004";
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub std: Std,
    /// User conditional analysis identifiers, `(name, value)`.
    pub defines: Vec<(String, String)>,
    /// Read `-- psl` comments as PSL pragmas (`--psl`); native PSL statements are always read.
    pub psl: bool,
}

pub struct Parsed {
    /// The tree, present only when there were no errors.
    pub design: Option<ast::DesignFile>,
    pub diags: Vec<Diagnostic>,
}

/// How to read the bytes of a source file.
///
/// VHDL source is ISO-8859-1 (LRM 15.2), but files are often saved as UTF-8.
/// Either way every character must be in ISO-8859-1, so `"é"'length` is 1
/// whether `é` is one byte or two.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Encoding {
    /// UTF-8 if the file is valid UTF-8, otherwise ISO-8859-1 bytes.
    #[default]
    Auto,
    /// Each byte is one ISO-8859-1 character.
    Latin1,
    /// UTF-8; invalid UTF-8 is an error.
    Utf8,
}

/// Add a file's bytes to `sources`, decoded per `encoding`. Spans into the
/// file are byte offsets into `bytes`. The diagnostic is an error for invalid
/// UTF-8 under [`Encoding::Utf8`]; the file is then loaded as Latin-1 so the
/// diagnostic can show it, and must not be parsed.
pub fn load_source(
    sources: &mut SourceMap,
    path: impl Into<String>,
    bytes: Vec<u8>,
    encoding: Encoding,
) -> (FileId, Option<Diagnostic>) {
    let path = path.into();
    if encoding == Encoding::Latin1 {
        return (sources.add_latin1(path, bytes), None);
    }
    match String::from_utf8(bytes) {
        Ok(text) => (sources.add(path, text), None),
        Err(e) if encoding == Encoding::Auto => (sources.add_latin1(path, e.into_bytes()), None),
        Err(e) => {
            let at = e.utf8_error().valid_up_to() as u32;
            let file = sources.add_latin1(path, e.into_bytes());
            let d = Diagnostic::error(codes::INVALID_UTF8, "file is not valid UTF-8")
                .with_primary(Span::new(file, at, at + 1), "invalid byte")
                .with_help(
                    "VHDL source is ISO-8859-1: use `--encoding=latin1` or `--encoding=auto`",
                );
            (file, Some(d))
        }
    }
}

/// Lex, preprocess and parse a file loaded with [`load_source`].
pub fn parse_file(file: FileId, source: &SourceFile, opts: &Options) -> Parsed {
    let src = source::Src {
        bytes: source.bytes(),
        latin1: source.is_latin1(),
    };
    let mut diags = Vec::new();
    let toks = lexer::lex(file, src, opts.std, opts.psl, &mut diags);
    // An encrypted envelope ends the file: nothing after it is source, so no further stage runs.
    if diags.iter().any(|d| d.code == codes::ENCRYPTED) {
        return Parsed {
            design: None,
            diags,
        };
    }
    let toks = directive::preprocess(file, src, toks, opts.std, &opts.defines, &mut diags);
    let design = parser::parse(file, src, toks, opts.std, &mut diags);
    let ok = !diags.iter().any(|d| d.severity == Severity::Error);
    Parsed {
        design: ok.then_some(design),
        diags,
    }
}

/// The lexical elements of `text` (LRM 15.3) as kind and text, comments and separators
/// dropped and nothing reported: what the conformance rules compare (LRM 4.10).
pub fn lexical_elements(text: &str, std: Std) -> Vec<(token::Tok, &str)> {
    let src = source::Src {
        bytes: text.as_bytes(),
        latin1: false,
    };
    lexer::lex(FileId(0), src, std, false, &mut Vec::new())
        .into_iter()
        .filter(|t| t.kind != token::Tok::Eof)
        .map(|t| (t.kind, &text[t.start as usize..t.end as usize]))
        .collect()
}

/// Parse UTF-8 text that is not in a [`SourceMap`] (tests and tools).
pub fn parse_str(file: FileId, text: &str, opts: &Options) -> Parsed {
    let mut sources = SourceMap::default();
    let id = sources.add(String::new(), text);
    parse_file(file, sources.get(id), opts)
}

#[cfg(test)]
mod tests;
