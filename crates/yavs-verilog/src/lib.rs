//! Verilog (IEEE 1364-2005) front end: preprocessor, lexer, parser and module analysis.
//!
//! The one entry point is [`analyze`]. Diagnostics are plain data ([`Diag`]); the caller renders them
//! (`yavs-diag`) and registers the codes of [`CODES`].

use std::path::{Path, PathBuf};

/// A byte range of an original source file (`AnalysisResult::files[file]`), never of preprocessed text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Span {
    pub file: usize,
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

/// One diagnostic: a registered code, a message, a primary span with an optional label, notes and a help line.
#[derive(Clone, Debug)]
pub struct Diag {
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub span: Option<Span>,
    pub label: Option<String>,
    pub notes: Vec<String>,
    pub help: Option<String>,
}

/// A code of this crate with its `--explain` text.
pub struct CodeInfo {
    pub code: &'static str,
    pub title: &'static str,
    pub explain: &'static str,
}

/// Every code this crate can emit.
pub const CODES: &[CodeInfo] = &[];

/// `` `timescale ``: unit and precision in femtoseconds (a power of ten from 1 fs to 100 s).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timescale {
    pub unit_fs: u64,
    pub precision_fs: u64,
}

impl Timescale {
    /// The default of the plan (D10): 1ns/1ps.
    pub const DEFAULT: Timescale = Timescale { unit_fs: 1_000_000, precision_fs: 1_000 };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Verilog2005,
    SystemVerilog,
}

#[derive(Clone, Debug)]
pub struct Options {
    /// `-I DIR`, searched in order after the directory of the including file.
    pub include_dirs: Vec<PathBuf>,
    /// `-D NAME[=VALUE]`, defined before the first file.
    pub defines: Vec<(String, Option<String>)>,
    /// In force until a file says otherwise (`--timescale`).
    pub default_timescale: Timescale,
    /// `--lang`: overrides the language chosen by the file extension.
    pub lang_override: Option<Lang>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            include_dirs: Vec::new(),
            defines: Vec::new(),
            default_timescale: Timescale::DEFAULT,
            lang_override: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitKind {
    Module,
    /// A user-defined primitive (UDP).
    Primitive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Input,
    Output,
    Inout,
}

#[derive(Clone, Debug)]
pub struct Port {
    pub name: String,
    pub dir: Dir,
    /// The packed range as source text, `(msb, lsb)`, when the port is a vector.
    pub range: Option<(String, String)>,
}

#[derive(Clone, Debug)]
pub struct Unit {
    pub kind: UnitKind,
    /// Case preserved (D5).
    pub name: String,
    /// Index into `AnalysisResult::files`.
    pub file: usize,
    /// The name in the declaration.
    pub span: Span,
    pub ports: Vec<Port>,
    /// The body is an IEEE 1735 envelope (D8): ports are those of the plaintext header.
    pub encrypted: bool,
    /// Digest of the state entering the file: macros, `` `timescale ``, `` `default_nettype ``.
    pub context_digest: String,
    /// Files pulled in by `` `include `` in the file of this unit (absolute, in first-use order).
    pub included_files: Vec<PathBuf>,
}

#[derive(Debug, Default)]
pub struct AnalysisResult {
    /// The files read: the arguments first, in order, then the includes.
    pub files: Vec<PathBuf>,
    pub units: Vec<Unit>,
    pub diagnostics: Vec<Diag>,
}

/// Analyze the files in order. Macros, `` `timescale `` and `` `default_nettype `` persist from one
/// file to the next (one compilation unit, clause 19).
pub fn analyze(files: &[PathBuf], _opts: &Options) -> AnalysisResult {
    AnalysisResult { files: files.to_vec(), ..Default::default() }
}

/// Whether the extension of `path` is a Verilog (1364) source: `.v`, `.vh`, `.vp`.
pub fn is_verilog_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("v" | "vh" | "vp")
    )
}

/// Whether the extension of `path` is a SystemVerilog source: `.sv`, `.svh`, `.svp`.
pub fn is_systemverilog_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("sv" | "svh" | "svp")
    )
}
