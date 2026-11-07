//! `yavs analyze` of Verilog files: which files are Verilog, the options of the front end, and the
//! hand-over of its units to the library (the front end is `yavs-verilog`).

use std::path::{Path, PathBuf};

use clap::ValueEnum;
use yavs_diag::{Diagnostic, FileId, Severity, SourceMap, Span};
use yavs_lib::{UnitKind, UnitRecord, VERILOG_STD, hash_content};
use yavs_verilog::{AnalysisResult, Lang, Options, Timescale};

/// `--lang`.
#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Copy, PartialEq, ValueEnum)]
pub(crate) enum LangArg {
    Vhdl,
    Verilog,
}

/// What a file of the command line is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FileKind {
    Vhdl,
    Verilog,
    SystemVerilog,
}

impl FileKind {
    /// By `--lang` if given, else by extension: `.v`, `.vh`, `.vp` are Verilog, `.sv`, `.svh`,
    /// `.svp` SystemVerilog (refused), anything else VHDL.
    pub(crate) fn of(path: &Path, lang: Option<LangArg>) -> FileKind {
        match lang {
            Some(LangArg::Vhdl) => FileKind::Vhdl,
            Some(LangArg::Verilog) => FileKind::Verilog,
            None if yavs_verilog::is_verilog_path(path) => FileKind::Verilog,
            None if yavs_verilog::is_systemverilog_path(path) => FileKind::SystemVerilog,
            None => FileKind::Vhdl,
        }
    }
}

/// `-D NAME` or `-D NAME=VALUE`.
pub(crate) fn parse_define(s: &str) -> Result<(String, Option<String>), String> {
    match s.split_once('=') {
        Some(("", _)) => Err(format!("expected NAME or NAME=VALUE, found `{s}`")),
        Some((k, v)) => Ok((k.to_string(), Some(v.to_string()))),
        None if s.is_empty() => Err("expected NAME or NAME=VALUE, found an empty value".into()),
        None => Ok((s.to_string(), None)),
    }
}

/// One side of a time scale: 1, 10 or 100 of s, ms, us, ns, ps or fs, in femtoseconds.
fn scale_fs(s: &str) -> Result<u64, String> {
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    let magnitude: u64 = match &s[..digits] {
        "1" => 1,
        "10" => 10,
        "100" => 100,
        _ => return Err(format!("`{s}` does not start with 1, 10 or 100")),
    };
    let unit: u64 = match &s[digits..] {
        "s" => 1_000_000_000_000_000,
        "ms" => 1_000_000_000_000,
        "us" => 1_000_000_000,
        "ns" => 1_000_000,
        "ps" => 1_000,
        "fs" => 1,
        u => {
            return Err(format!(
                "`{u}` is not a time unit (s, ms, us, ns, ps or fs)"
            ));
        }
    };
    Ok(magnitude * unit)
}

/// `--timescale UNIT/PRECISION`, as in `1ns/1ps`.
pub(crate) fn parse_timescale(s: &str) -> Result<Timescale, String> {
    let (u, p) = s
        .split_once('/')
        .ok_or_else(|| format!("expected UNIT/PRECISION such as 1ns/1ps, found `{s}`"))?;
    let (unit_fs, precision_fs) = (scale_fs(u.trim())?, scale_fs(p.trim())?);
    if precision_fs > unit_fs {
        return Err(format!(
            "the precision `{p}` is coarser than the unit `{u}` (IEEE 1364-2005 19.8)"
        ));
    }
    Ok(Timescale {
        unit_fs,
        precision_fs,
    })
}

/// The Verilog files of one `yavs analyze` call, analysed together: macros, `` `timescale `` and
/// `` `default_nettype `` persist from one file to the next (IEEE 1364-2005 19.3).
pub(crate) struct Batch {
    result: AnalysisResult,
    /// Per file of the call: its index in `result.files`, for a Verilog file.
    at: Vec<Option<usize>>,
    /// The files of `result.files` and `FileId`s of the same order.
    pub(crate) sources: SourceMap,
    ids: Vec<FileId>,
    /// Digest part that is not the front end's: the `-I` list and the default time scale.
    options: String,
    /// The first Verilog file of the call: what no file owns is reported there.
    first: usize,
}

impl Batch {
    pub(crate) fn run(files: &[PathBuf], kinds: &[FileKind], opts: &Options) -> Batch {
        let verilog: Vec<usize> = (0..files.len())
            .filter(|&i| kinds[i] == FileKind::Verilog)
            .collect();
        let args: Vec<PathBuf> = verilog.iter().map(|&i| files[i].clone()).collect();
        Batch::of(files, kinds, yavs_verilog::analyze(&args, opts), opts)
    }

    /// The batch for the `result` of analysing the Verilog files among `files`.
    fn of(files: &[PathBuf], kinds: &[FileKind], result: AnalysisResult, opts: &Options) -> Batch {
        let verilog: Vec<usize> = (0..files.len())
            .filter(|&i| kinds[i] == FileKind::Verilog)
            .collect();
        let mut at = vec![None; files.len()];
        for (j, &i) in verilog.iter().enumerate() {
            at[i] = Some(j);
        }
        let mut sources = SourceMap::default();
        let ids = result
            .files
            .iter()
            .map(|p| {
                let name = p.display().to_string();
                match std::fs::read(p).map(String::from_utf8) {
                    Ok(Ok(text)) => sources.add(name, text),
                    Ok(Err(e)) => sources.add_latin1(name, e.into_bytes()),
                    // Unreadable: the front end said so; the span has no text to show.
                    Err(_) => sources.add(name, String::new()),
                }
            })
            .collect();
        let dirs: Vec<_> = opts
            .include_dirs
            .iter()
            .map(|d| d.display().to_string())
            .collect();
        Batch {
            options: format!(
                "-I {}\ntimescale {}/{}\n",
                dirs.join(":"),
                opts.default_timescale.unit_fs,
                opts.default_timescale.precision_fs
            ),
            first: verilog.first().copied().unwrap_or(0),
            result,
            at,
            sources,
            ids,
        }
    }

    fn render(&self, d: &yavs_verilog::Diag) -> Diagnostic {
        let severity = match d.severity {
            yavs_verilog::Severity::Error => Severity::Error,
            yavs_verilog::Severity::Warning => Severity::Warning,
            yavs_verilog::Severity::Note => Severity::Note,
        };
        let mut out = Diagnostic::new(severity, d.code, d.message.clone());
        if let Some(s) = d.span {
            out = out.with_primary(
                Span::new(self.ids[s.file], s.start, s.end),
                d.label.clone().unwrap_or_default(),
            );
        }
        for n in &d.notes {
            out = out.with_note(n.clone());
        }
        match &d.help {
            Some(h) => out.with_help(h.clone()),
            None => out,
        }
    }

    /// The diagnostics of file `i` of the call: those with a place in it, and, for the first
    /// Verilog file, those with no place or a place in an included file.
    // ponytail: a problem in an included file is reported at the first Verilog file of the call;
    // the front end names no including file. Upgrade when `Diag` carries one.
    pub(crate) fn diagnostics(&self, i: usize) -> Vec<Diagnostic> {
        let nargs = self.at.iter().flatten().count();
        self.result
            .diagnostics
            .iter()
            .filter(|d| match (d.span, self.at[i]) {
                (Some(s), Some(j)) if s.file < nargs => s.file == j,
                _ => i == self.first,
            })
            .map(|d| self.render(d))
            .collect()
    }

    /// The library records of the units of file `i`, with the state it was analysed under and the
    /// hashes of the files it included. An included file that cannot be read is the error.
    pub(crate) fn records(&self, i: usize) -> Result<Vec<UnitRecord>, Box<Diagnostic>> {
        let j = self.at[i].expect("a Verilog file");
        let mut out = Vec::new();
        for u in self.result.units.iter().filter(|u| u.file == j) {
            let kind = match u.kind {
                yavs_verilog::UnitKind::Module => UnitKind::Module,
                yavs_verilog::UnitKind::Primitive => UnitKind::Primitive,
            };
            let mut r = UnitRecord::verilog(kind, &u.name, VERILOG_STD);
            r.encrypted = u.encrypted;
            r.context = Some(hash_content(
                format!("{}\n{}", u.context_digest, self.options).as_bytes(),
            ));
            for p in &u.included_files {
                let bytes = std::fs::read(p).map_err(|e| {
                    Box::new(Diagnostic::error(
                        "E0100",
                        format!("cannot read `{}`: {e}", p.display()),
                    ))
                })?;
                r.includes
                    .push((p.display().to_string(), hash_content(&bytes)));
            }
            out.push(r);
        }
        Ok(out)
    }
}

/// The front end's options from the command line.
pub(crate) fn options(
    include_dirs: &[PathBuf],
    defines: &[(String, Option<String>)],
    timescale: Option<Timescale>,
    lang: Option<LangArg>,
) -> Options {
    Options {
        include_dirs: include_dirs.to_vec(),
        defines: defines.to_vec(),
        default_timescale: timescale.unwrap_or(Timescale::DEFAULT),
        lang_override: match lang {
            Some(LangArg::Verilog) => Some(Lang::Verilog2005),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_told_apart_by_extension_or_lang() {
        let k = |p, l| FileKind::of(Path::new(p), l);
        assert_eq!(k("a.vhd", None), FileKind::Vhdl);
        assert_eq!(k("a", None), FileKind::Vhdl);
        assert_eq!(k("a.v", None), FileKind::Verilog);
        assert_eq!(k("a.vh", None), FileKind::Verilog);
        assert_eq!(k("a.sv", None), FileKind::SystemVerilog);
        assert_eq!(k("a.svh", None), FileKind::SystemVerilog);
        assert_eq!(k("a.txt", Some(LangArg::Verilog)), FileKind::Verilog);
        assert_eq!(k("a.sv", Some(LangArg::Verilog)), FileKind::Verilog);
        assert_eq!(k("a.v", Some(LangArg::Vhdl)), FileKind::Vhdl);
    }

    #[test]
    fn defines_and_timescales_parse() {
        assert_eq!(parse_define("A"), Ok(("A".into(), None)));
        assert_eq!(parse_define("A="), Ok(("A".into(), Some(String::new()))));
        assert_eq!(parse_define("A=b=c"), Ok(("A".into(), Some("b=c".into()))));
        assert!(parse_define("=x").is_err() && parse_define("").is_err());
        let t = parse_timescale("1ns/1ps").unwrap();
        assert_eq!((t.unit_fs, t.precision_fs), (1_000_000, 1_000));
        assert_eq!(parse_timescale("100ps/10fs").unwrap().unit_fs, 100_000);
        for bad in ["1ns", "2ns/1ps", "1xs/1ps", "1ps/1ns", "ns/ps", "1ns/"] {
            assert!(parse_timescale(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn units_become_records_and_diagnostics_stay_with_their_file() {
        use yavs_verilog::{Diag, Unit};
        let dir = std::env::temp_dir().join(format!("yavs-verilog-batch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b, inc) = (dir.join("a.v"), dir.join("b.v"), dir.join("defs.vh"));
        std::fs::write(&a, "module A; endmodule\n").unwrap();
        std::fs::write(&b, "module b; endmodule\n").unwrap();
        std::fs::write(&inc, "`define W 8\n").unwrap();
        let unit = |name: &str, file: usize, encrypted| Unit {
            kind: yavs_verilog::UnitKind::Module,
            name: name.into(),
            file,
            span: yavs_verilog::Span::default(),
            ports: vec![],
            encrypted,
            context_digest: "ctx".into(),
            included_files: vec![inc.clone()],
        };
        let diag = |code: &'static str, file: Option<usize>| Diag {
            code,
            severity: yavs_verilog::Severity::Error,
            message: "m".into(),
            span: file.map(|f| yavs_verilog::Span {
                file: f,
                start: 0,
                end: 6,
            }),
            label: Some("here".into()),
            notes: vec![],
            help: None,
        };
        let result = AnalysisResult {
            files: vec![a.clone(), b.clone(), inc.clone()],
            units: vec![unit("A", 0, false), unit("b", 1, true)],
            diagnostics: vec![
                diag("E4003", Some(1)),
                diag("E4005", Some(2)),
                diag("E4004", None),
            ],
        };
        // The call: a VHDL file between the two Verilog files.
        let files = vec![a, "x.vhd".into(), b];
        let kinds = [FileKind::Verilog, FileKind::Vhdl, FileKind::Verilog];
        let batch = Batch::of(&files, &kinds, result, &Options::default());
        let codes = |i| {
            batch
                .diagnostics(i)
                .iter()
                .map(|d| d.code)
                .collect::<Vec<_>>()
        };
        assert_eq!(codes(2), ["E4003"]);
        // No place, or a place in an included file: the first Verilog file.
        assert_eq!(codes(0), ["E4005", "E4004"]);
        let r = batch.records(0).unwrap();
        assert_eq!(
            (r[0].name.as_str(), r[0].key.as_str(), r[0].encrypted),
            ("A", "A", false)
        );
        assert_eq!(
            r[0].includes,
            [(inc.display().to_string(), hash_content(b"`define W 8\n"))]
        );
        assert!(batch.records(2).unwrap()[0].encrypted);
        // The state includes the options the front end does not know of.
        let other = Options {
            include_dirs: vec!["/inc".into()],
            ..Options::default()
        };
        let batch2 = Batch::of(
            &files,
            &kinds,
            AnalysisResult {
                files: vec![files[0].clone(), files[2].clone(), inc.clone()],
                units: vec![unit("A", 0, false)],
                diagnostics: vec![],
            },
            &other,
        );
        assert_ne!(r[0].context, batch2.records(0).unwrap()[0].context);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
