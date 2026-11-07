//! `yavs analyze` of Verilog files: which files are Verilog, the options of the front end, and the
//! hand-over of its units to the library (the front end is `yavs-verilog`).

use std::path::{Path, PathBuf};

use clap::ValueEnum;
use yavs_diag::{Diagnostic, FileId, Severity, SourceMap, Span};
use yavs_lib::{Libraries, UnitKind, UnitRecord, VERILOG_STD, hash_content};
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
            result,
            at,
            sources,
            ids,
        }
    }

    fn render(&self, d: &yavs_verilog::Diag) -> Diagnostic {
        render(d, &self.ids)
    }

    /// The diagnostics of file `i` of the call: those its analysis produced, including a problem
    /// in a file it includes (the span names that file).
    pub(crate) fn diagnostics(&self, i: usize) -> Vec<Diagnostic> {
        let Some(j) = self.at[i] else {
            return Vec::new();
        };
        self.result
            .diagnostics
            .iter()
            .zip(&self.result.diag_file)
            .filter(|&(_, &f)| f == j)
            .map(|(d, _)| self.render(d))
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
            r.setup = Some(u.setup.clone());
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

/// A front-end diagnostic with its spans in the files `ids` (the front end's file numbers).
fn render(d: &yavs_verilog::Diag, ids: &[FileId]) -> Diagnostic {
    let severity = match d.severity {
        yavs_verilog::Severity::Error => Severity::Error,
        yavs_verilog::Severity::Warning => Severity::Warning,
        yavs_verilog::Severity::Note => Severity::Note,
    };
    let mut out = Diagnostic::new(severity, d.code, d.message.clone());
    if let Some(s) = d.span {
        out = out.with_primary(
            Span::new(ids[s.file], s.start, s.end),
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

/// The files of `paths` added to `sources`, as `Batch::of` adds them.
fn add_files(sources: &mut SourceMap, paths: &[PathBuf]) -> Vec<FileId> {
    paths
        .iter()
        .map(|p| {
            let name = p.display().to_string();
            match std::fs::read(p).map(String::from_utf8) {
                Ok(Ok(text)) => sources.add(name, text),
                Ok(Err(e)) => sources.add_latin1(name, e.into_bytes()),
                Err(_) => sources.add(name, String::new()),
            }
        })
        .collect()
}

/// The libraries searched for the Verilog module of a component with no VHDL binding (D6): the
/// work library, then each `-L` library in order. A `-L` library that is not mapped is `E0142`.
pub(crate) fn search_libraries(
    libs: &Libraries,
    work: &str,
    extra: &[String],
) -> Result<Vec<yavs_elab::VerilogLib>, Box<Diagnostic>> {
    let mut out: Vec<yavs_elab::VerilogLib> = Vec::new();
    for name in std::iter::once(work).chain(extra.iter().map(String::as_str)) {
        let Some(lib) = libs.get(name) else {
            let mut d = Diagnostic::error(
                "E0142",
                format!("`-L {name}`: no library `{name}` is mapped"),
            );
            let names: Vec<&str> = libs.names().collect();
            d = match yavs_sem::suggest::suggest(
                &yavs_lib::fold(name),
                "library",
                names.iter().map(|n| (*n, *n)),
            ) {
                Some(h) => d.with_help(h),
                None => d.with_help("map it with `--map NAME:PATH`"),
            };
            return Err(Box::new(d));
        };
        if out.iter().any(|l| l.name == lib.name) {
            continue;
        }
        out.push(yavs_elab::VerilogLib {
            name: lib.name.clone(),
            manifest: lib.read()?,
        });
    }
    Ok(out)
}

/// The Verilog side of an elaborated design (#776 step C): each module the component instances
/// bound to is read again as it was analysed, elaborated with the instance's generics and ports,
/// and the one domain of the design is handed to `e` for the build. Diagnostics name files that
/// are added to `sources`.
pub(crate) fn elaborate(
    e: &mut yavs_elab::Elaborated,
    libs: &Libraries,
    sources: &mut SourceMap,
) -> Result<(), Vec<Diagnostic>> {
    let fail = |d: Diagnostic| vec![d];
    let mut design = yavs_verilog::Design::default();
    let mut sigs: Vec<yavs_kernel::SigRef> = Vec::new();
    let mut errors = Vec::new();
    for vi in &e.verilog {
        // The component scope is left out of hierarchical paths; here it is the instance.
        let sc = e.hier.scope(vi.scope);
        let path = match sc.parent {
            Some(p) => format!(
                "{}.{}",
                e.hier.dotted_in(p, yavs_elab::Case::Lrm),
                sc.name.text
            ),
            None => sc.name.text.to_string(),
        };
        let at = |d: Diagnostic| d.with_primary(vi.span, "instantiated here");
        let Some(lib) = libs.get(&vi.library) else {
            return Err(fail(Diagnostic::error(
                "E9999",
                "a Verilog library vanished; this is a bug in yavs",
            )));
        };
        let (rec, hash) = match lib.record_of(UnitKind::Module, &vi.module) {
            Ok(Some(r)) => r,
            Ok(None) => match lib.record_of(UnitKind::Primitive, &vi.module) {
                Ok(Some(_)) => {
                    return Err(fail(at(Diagnostic::error(
                        "E4501",
                        format!(
                            "instantiating the user-defined primitive `{}` from VHDL is not implemented in Verilog elaboration",
                            vi.module
                        ),
                    ))));
                }
                Ok(None) => {
                    return Err(fail(Diagnostic::error(
                        "E9999",
                        format!(
                            "the Verilog module `{}` has no record; this is a bug in yavs",
                            vi.module
                        ),
                    )));
                }
                Err(d) => return Err(fail(*d)),
            },
            Err(d) => return Err(fail(*d)),
        };
        if rec.encrypted {
            return Err(fail(at(Diagnostic::error(
                "E4001",
                format!("the Verilog module `{}` bound to instance {} is encrypted (IEEE 1735) and cannot be simulated", vi.module, yavs_diag::dotted(&path)),
            )
            .with_help("yavs never decrypts protected IP; use the vendor's unencrypted simulation model, or simulate this block with a tool that holds the vendor's key"))));
        }
        let stale = |what: String| {
            at(Diagnostic::error(
                "E4508",
                format!(
                    "{what}, so the Verilog module `{}` cannot be read as it was analyzed",
                    vi.module
                ),
            )
            .with_help("analyze the Verilog files again"))
        };
        match std::fs::read(&vi.file) {
            Ok(b) if hash_content(&b) == hash => {}
            Ok(_) => {
                return Err(fail(stale(format!(
                    "`{}` changed since it was analyzed",
                    vi.file.display()
                ))));
            }
            Err(err) => {
                return Err(fail(stale(format!(
                    "`{}` cannot be read: {err}",
                    vi.file.display()
                ))));
            }
        }
        for (p, h) in &rec.includes {
            if std::fs::read(p).map(|b| hash_content(&b)).ok().as_ref() != Some(h) {
                return Err(fail(stale(format!(
                    "the included file `{p}` changed since the analysis"
                ))));
            }
        }
        let Some(setup) = &rec.setup else {
            return Err(fail(stale(format!(
                "library `{}` was written by an older yavs",
                vi.library
            ))));
        };
        let Some(src) = yavs_verilog::read_unit(&vi.file, setup) else {
            return Err(fail(stale(format!(
                "the analysis record of `{}` cannot be read",
                vi.file.display()
            ))));
        };
        let ids = add_files(sources, &src.files);
        let mut bad = false;
        for d in &src.diagnostics {
            if d.severity == yavs_verilog::Severity::Error {
                bad = true;
                errors.push(render(d, &ids));
            }
        }
        if bad {
            continue;
        }
        let Some(module) = src.module(&vi.module) else {
            return Err(fail(stale(format!(
                "`{}` no longer declares `{}`",
                vi.file.display(),
                vi.module
            ))));
        };
        let generics: Vec<yavs_verilog::Generic> = vi
            .generics
            .iter()
            .map(|(n, v)| yavs_verilog::Generic {
                name: n.clone(),
                value: match v {
                    yavs_elab::VerilogValue::Int(i) => yavs_verilog::ParamValue::Int(*i),
                    yavs_elab::VerilogValue::Real(r) => yavs_verilog::ParamValue::Real(*r),
                    yavs_elab::VerilogValue::Str(s) => yavs_verilog::ParamValue::Str(s.clone()),
                    yavs_elab::VerilogValue::Logic(l) => yavs_verilog::ParamValue::Logic(l.clone()),
                },
            })
            .collect();
        let mut ports = Vec::new();
        for p in &vi.ports {
            let o = e.nets.obj(p.obj);
            let leaf = o.leaves.0;
            let len = e.nets.leaf(leaf).ty.len;
            let r = o.part.unwrap_or(yavs_kernel::Range::new(0, len));
            ports.push(yavs_verilog::PortSpec {
                name: p.name.clone(),
                input: p.input,
                width: p.width,
                bit: p.bit,
                key: sigs.len() as u32,
            });
            sigs.push(e.nets.kref(leaf, r));
        }
        let inst = yavs_verilog::Instance {
            path: &path,
            source: &src,
            module,
            generics: &generics,
            ports: &ports,
        };
        if let Err(ds) = design.add(&inst) {
            for d in &ds {
                let d =
                    render(d, &ids).with_note(format!("in instance {}", yavs_diag::dotted(&path)));
                errors.push(d);
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let drivers: Vec<_> = design.outputs().iter().map(|&k| sigs[k as usize]).collect();
    // The program of the Verilog processes: an error is a bug in yavs.
    let runtime = design.runtime().map_err(|m| {
        vec![Diagnostic::error(
            "E9999",
            format!("internal compiler error: {m}"),
        )]
    })?;
    let design = std::sync::Arc::new(design);
    let inputs = std::sync::Arc::new(sigs);
    e.verilog_domain = Some(yavs_elab::VerilogDomain {
        name: "verilog".into(),
        drivers,
        make: std::sync::Arc::new(move |ds| {
            let inputs = inputs.clone();
            Box::new(design.domain(&move |k| inputs[k as usize], ds))
        }),
        runtime: Some(runtime),
    });
    Ok(())
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
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
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
            timescale: yavs_verilog::Timescale::DEFAULT,
            celldefine: false,
            default_nettype: yavs_verilog::Nettype::Wire,
            setup: String::new(),
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
                diag("E4005", Some(2)),
                diag("E4004", None),
                diag("E4003", Some(1)),
            ],
            diag_file: vec![0, 0, 1],
            ..Default::default()
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
        // No place, or a place in an included file: the file whose analysis found it.
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
                ..Default::default()
            },
            &other,
        );
        assert_ne!(r[0].context, batch2.records(0).unwrap()[0].context);
    }
}
