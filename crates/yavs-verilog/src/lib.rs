//! Verilog (IEEE 1364-2005) front end: preprocessor, lexer, parser and module analysis.
//!
//! The one entry point is [`analyze`]. Diagnostics are plain data ([`Diag`]); the caller renders them
//! (`yavs-diag`) and registers the codes of [`CODES`].

mod analysis;
pub mod ast;
mod diag;
mod lex;
mod parse;
mod pre;
mod stmt;

use std::path::{Path, PathBuf};

pub use pre::Nettype;

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
pub const CODES: &[CodeInfo] = diag::TABLE;

/// `` `timescale ``: unit and precision in femtoseconds (a power of ten from 1 fs to 100 s).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timescale {
    pub unit_fs: u64,
    pub precision_fs: u64,
}

impl Timescale {
    /// The default of the plan (D10): 1ns/1ps.
    pub const DEFAULT: Timescale = Timescale {
        unit_fs: 1_000_000,
        precision_fs: 1_000,
    };
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
    /// The `` `timescale `` in force at the module, or the default when none is.
    pub timescale: Timescale,
    /// Inside `` `celldefine ``.
    pub celldefine: bool,
    pub default_nettype: Nettype,
}

#[derive(Debug, Default)]
pub struct AnalysisResult {
    /// The files read: the arguments first, in order, then the includes.
    pub files: Vec<PathBuf>,
    pub units: Vec<Unit>,
    pub diagnostics: Vec<Diag>,
    /// Index into the argument files of the first file with an error: the files after it were not read.
    pub stopped_at: Option<usize>,
}

/// Analyze the files in order. Macros, `` `timescale `` and `` `default_nettype `` persist from one
/// file to the next (one compilation unit, clause 19). The first file with an error stops the batch.
pub fn analyze(files: &[PathBuf], opts: &Options) -> AnalysisResult {
    use std::collections::HashMap;
    let mut res = AnalysisResult {
        files: files.to_vec(),
        ..Default::default()
    };
    let mut st = pre::State::new(opts);
    let mut names: HashMap<String, usize> = HashMap::new();
    for (fi, path) in files.iter().enumerate() {
        let before = res
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count();
        one_file(&mut res, &mut st, &mut names, opts, fi, path);
        if res
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count()
            > before
        {
            res.stopped_at = Some(fi);
            break;
        }
    }
    let _ = &names;
    res
}

fn one_file(
    res: &mut AnalysisResult,
    st: &mut pre::State,
    names: &mut std::collections::HashMap<String, usize>,
    opts: &Options,
    fi: usize,
    path: &Path,
) {
    let here = Span {
        file: fi,
        start: 0,
        end: 0,
    };
    let sv = match opts.lang_override {
        Some(Lang::SystemVerilog) => true,
        Some(Lang::Verilog2005) => false,
        None => is_systemverilog_path(path),
    };
    if sv {
        res.diagnostics.push(
            Diag::error(
                "E4002",
                format!("`{}` is a SystemVerilog file", path.display()),
                here,
            )
            .note("yavs reads Verilog (IEEE 1364-2005) only")
            .help("if the file is plain Verilog, pass --lang=verilog"),
        );
        return;
    }
    if opts.lang_override.is_none() && !is_verilog_path(path) {
        res.diagnostics.push(
            Diag::error(
                "E4012",
                format!("`{}` has no Verilog extension", path.display()),
                here,
            )
            .help("name it .v or .vh, or pass --lang=verilog"),
        );
        return;
    }
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            res.diagnostics.push(
                Diag::error("E4012", format!("cannot read `{}`", path.display()), here)
                    .note(e.to_string()),
            );
            return;
        }
    };
    let incoming = st.clone();
    let digest = st.digest();
    let pre = pre::preprocess(opts, &mut res.files, st, fi, &bytes);
    let mut diags: Vec<Diag> = pre.diags.clone();
    let (toks, lexd) = lex::lex(&pre);
    diags.extend(lexd);
    let has_error = |d: &[Diag]| d.iter().any(|x| x.severity == Severity::Error);
    let mut decls = Vec::new();
    if !has_error(&diags) {
        let mut p = parse::Parser::new(&pre, &toks);
        decls = p.file(&|off| pre.state_at(off, &incoming));
        diags.append(&mut p.diags);
    }
    // The envelope: one diagnostic per file.
    if let Some(env) = pre.envelopes.first() {
        let headerless = decls.iter().any(|d| matches!(d, ast::Decl::Envelope(_)));
        let mut d = Diag::new(
            "E4001",
            if headerless {
                Severity::Error
            } else {
                Severity::Warning
            },
            "encrypted (IEEE 1735 protected) source",
            Some(env.span),
        )
        .label(format!(
            "envelope starts here; key blocks for {} tool vendor(s)",
            env.keys
        ));
        if !env.terminated {
            d = d.note("the envelope has no end_protected");
        }
        for dec in &decls {
            if let ast::Decl::Module(m) = dec
                && m.encrypted.is_some()
            {
                d = d.note(format!(
                    "module `{}` cannot be simulated; it is recorded as an encrypted unit",
                    m.name.name
                ));
            }
        }
        d = d.help(
            "yavs never decrypts protected IP; use the vendor's unencrypted simulation model, or simulate this block with a tool that holds the vendor's key",
        );
        diags.push(d);
    }
    let failed = has_error(&diags);
    let mut warned_ts = false;
    if !failed {
        let mut checks = false;
        for decl in &decls {
            let (kind, name, ports, encrypted, ts, cd, nt) = match decl {
                ast::Decl::Module(m) => {
                    let mut cx = analysis::Ctx {
                        pre: &pre,
                        file: fi,
                        diags: &mut diags,
                    };
                    let _ = cx.file;
                    let ports = analysis::module(&mut cx, m);
                    checks |= analysis::has_timing_check(m);
                    (
                        UnitKind::Module,
                        &m.name,
                        ports,
                        m.encrypted.is_some(),
                        m.timescale,
                        m.celldefine,
                        m.nettype,
                    )
                }
                ast::Decl::Udp(u) => (
                    UnitKind::Primitive,
                    &u.name,
                    analysis::udp_ports(u),
                    false,
                    None,
                    false,
                    Nettype::Wire,
                ),
                ast::Decl::Envelope(_) => continue,
            };
            let span = pre.span(name.pos, name.pos + name.name.len().max(1) as u32);
            if ts.is_none() && kind == UnitKind::Module && !warned_ts {
                warned_ts = true;
                diags.push(
                    Diag::warning(
                        "W4006",
                        format!("no `timescale in force for module `{}`", name.name),
                        span,
                    )
                    .note(format!(
                        "using {}",
                        if opts.default_timescale == Timescale::DEFAULT {
                            "the default 1ns/1ps".to_string()
                        } else {
                            "the --timescale value".to_string()
                        }
                    )),
                );
            }
            if let Some(&prev) = names.get(&name.name) {
                diags.push(
                    Diag::error("E4200", format!("`{}` is declared twice", name.name), span)
                        .label("declared again here")
                        .note(format!("first declared in `{}`", res.files[prev].display())),
                );
                continue;
            }
            names.insert(name.name.clone(), fi);
            res.units.push(Unit {
                kind,
                name: name.name.clone(),
                file: fi,
                span,
                ports,
                encrypted,
                context_digest: digest.clone(),
                included_files: pre.includes.clone(),
                timescale: ts.unwrap_or(opts.default_timescale),
                celldefine: cd,
                default_nettype: nt,
            });
        }
        if checks && st.macros.contains_key("XIL_TIMING") {
            diags.push(Diag::new(
                "W4015",
                Severity::Warning,
                "timing checks are parsed but not evaluated",
                Some(Span {
                    file: fi,
                    start: 0,
                    end: 0,
                }),
            ));
        }
    }
    res.diagnostics.extend(diags);
}

/// Parse one source text (no file on disk) to its syntax tree, with the options' macros and include
/// directories. The error is the diagnostics, with spans into `<string>` (file 0).
pub fn parse_str(src: &str, opts: &Options) -> Result<Vec<ast::Decl>, Vec<Diag>> {
    let mut files = vec![PathBuf::from("<string>")];
    let mut st = pre::State::new(opts);
    let incoming = st.clone();
    let pre = pre::preprocess(opts, &mut files, &mut st, 0, src.as_bytes());
    let mut diags = pre.diags.clone();
    let (toks, lexd) = lex::lex(&pre);
    diags.extend(lexd);
    let mut decls = Vec::new();
    if diags.iter().all(|d| d.severity != Severity::Error) {
        let mut p = parse::Parser::new(&pre, &toks);
        decls = p.file(&|off| pre.state_at(off, &incoming));
        diags.append(&mut p.diags);
    }
    if diags.iter().any(|d| d.severity == Severity::Error) {
        return Err(diags);
    }
    Ok(decls)
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
