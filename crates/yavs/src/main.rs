//! The `yavs` command line.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

mod analyze;
mod cache;
mod counters;
mod cov;
mod elab;
mod explain;
mod gui;
mod jobserver;
mod profile;
mod stdout;
mod stuck;
mod timing;

use clap::{Parser, Subcommand, ValueEnum};
use yavs_diag::{ColorChoice, Diagnostic, MessageFormat, Severity, SourceMap};
use yavs_syntax::{Encoding, Options, Std};

/// What `yavs --version` prints after the name: the first line of its output is
/// `yavs MAJOR.MINOR.PATCH (details)` (docs/cli-contract.md section 1); the details say whether
/// the compiled tier is built in (release archives with and without it, #780).
const VERSION: &str = if cfg!(all(feature = "llvm", feature = "clift")) {
    concat!(
        env!("CARGO_PKG_VERSION"),
        " (compiled tier: llvm, cranelift)"
    )
} else if cfg!(feature = "llvm") {
    concat!(env!("CARGO_PKG_VERSION"), " (compiled tier: llvm)")
} else if cfg!(feature = "clift") {
    concat!(env!("CARGO_PKG_VERSION"), " (compiled tier: cranelift)")
} else {
    concat!(
        env!("CARGO_PKG_VERSION"),
        " (interpreter only, no compiled tier)"
    )
};

#[cfg_attr(test, derive(Debug))]
#[derive(Parser)]
#[command(
    name = "yavs",
    version = VERSION,
    about = "A VHDL simulator",
    args_override_self = true
)]
struct Cli {
    #[command(flatten)]
    top: Top,
    #[command(subcommand)]
    command: Option<Command>,
}

/// The options before the subcommand (`color`, `message_format` and `encoding` also after it).
#[cfg_attr(test, derive(Debug))]
#[derive(clap::Args)]
struct Top {
    /// When to colour diagnostics.
    #[arg(long, global = true, value_enum, default_value = "auto")]
    color: Color,
    /// How to print diagnostics.
    #[arg(long, global = true, value_enum, default_value = "human")]
    message_format: Format,
    /// How to read source files: UTF-8 when valid and ISO-8859-1 otherwise
    /// (auto), always ISO-8859-1 bytes (latin1), or UTF-8 only (utf8).
    #[arg(long, global = true, value_enum, default_value = "auto")]
    encoding: EncodingArg,
    /// Print the long explanation of a diagnostic code, such as `E0002`, and exit.
    #[arg(long, value_name = "CODE")]
    explain: Option<String>,
}

/// The command line of `analyze`, `elab` and `run`, which test runners call by the hundred, parsed
/// from a command that holds only the subcommand called: building all 70 arguments of all
/// subcommands cost 4% of a tiny run (#426). `None` for anything else and for every error or
/// request for help, which then go through the whole command (`Cli::parse_from`), so their text
/// is that of the whole command's.
fn parse_hot(args: &[String]) -> Option<Cli> {
    use clap::{Args, FromArgMatches};
    let mut i = 1;
    let name = loop {
        match args.get(i)?.as_str() {
            "--color" | "--message-format" | "--encoding" => i += 2,
            a if a.starts_with("--color=")
                || a.starts_with("--message-format=")
                || a.starts_with("--encoding=") =>
            {
                i += 1
            }
            a @ ("analyze" | "elab" | "run") => break a,
            _ => return None,
        }
    };
    if args
        .iter()
        .any(|a| matches!(a.as_str(), "-h" | "--help" | "-V" | "--version"))
    {
        return None;
    }
    // ponytail: the attributes of `Cli` and of the variants of `Command` are repeated here; the
    // test `hot_parse_equals_the_whole_command` compares the two.
    let sub = match name {
        "analyze" => analyze::Args::augment_args(
            clap::Command::new("analyze").about("Analyze one VHDL file into a library"),
        ),
        "elab" => elab::Common::augment_args(
            clap::Command::new("elab").about("Elaborate a design (no simulation)"),
        ),
        _ => elab::RunArgs::augment_args(
            clap::Command::new("run").about("Elaborate and simulate a design"),
        ),
    };
    let top = clap::Command::new("yavs")
        .version(VERSION)
        .about("A VHDL simulator")
        .args_override_self(true);
    let mut matches = Top::augment_args(top)
        .subcommand(sub)
        .try_get_matches_from(args)
        .ok()?;
    Cli::from_arg_matches_mut(&mut matches).ok()
}

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
// Parsed once per process, boxing RunArgs buys nothing.
#[allow(clippy::large_enum_variant)]
enum Command {
    /// Check that VHDL files parse.
    Parse {
        /// Language standard.
        #[arg(long, value_enum, default_value = "2008")]
        std: StdArg,
        /// Define a conditional analysis identifier, `NAME=VALUE`.
        #[arg(long = "define", short = 'D', value_name = "NAME=VALUE", value_parser = parse_define)]
        defines: Vec<(String, String)>,
        /// Read `-- psl` comments as PSL pragmas.
        #[arg(long)]
        psl: bool,
        /// Print the syntax tree (for debugging yavs).
        #[arg(long, hide = true)]
        dump_ast: bool,
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Analyze one VHDL file into a library.
    Analyze(analyze::Args),
    /// Elaborate a design (no simulation).
    Elab(elab::Common),
    /// Elaborate and simulate a design.
    Run(elab::RunArgs),
    /// Merge coverage databases and write reports.
    #[command(subcommand)]
    Cov(cov::Command),
    /// Manage the compilation cache kept in library directories.
    #[command(subcommand)]
    Cache(cache::Command),
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum StdArg {
    #[value(name = "93", alias = "1993")]
    V1993,
    #[value(name = "2008")]
    V2008,
    #[value(name = "2019")]
    V2019,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Copy, ValueEnum)]
enum EncodingArg {
    Auto,
    Latin1,
    Utf8,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum Color {
    Auto,
    Always,
    Never,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Human,
    Short,
    Json,
}

pub(crate) fn parse_define(s: &str) -> Result<(String, String), String> {
    match s.split_once('=') {
        Some((k, v)) if !k.is_empty() => Ok((k.to_string(), v.to_string())),
        _ => Err(format!("expected NAME=VALUE, found `{s}`")),
    }
}

/// Run `f`; a panic becomes an `E9999` internal-compiler-error diagnostic and exit status 3.
fn guard(format: MessageFormat, color: bool, f: impl FnOnce() -> ExitCode) -> ExitCode {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|p| {
        let msg = p
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| p.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        let d = Diagnostic::error("E9999", "internal compiler error: this is a bug in yavs")
            .with_note(format!("panicked: {msg}"));
        let _ = std::io::stderr()
            .lock()
            .write_all(yavs_diag::render(&SourceMap::default(), &d, format, color).as_bytes());
        ExitCode::from(3)
    })
}

/// Code for a file that cannot be read.
const READ_ERROR: &str = "E0100";

fn main() -> ExitCode {
    yavs_cache::huge_heap();
    yavs_cache::set_sources(env!("YAVS_SRC_HASH"));
    yavs_vhpi::keep_exports();
    let args = match analyze::expand_response_files(
        std::env::args_os().map(|a| a.to_string_lossy().into_owned()),
    ) {
        Ok(a) => a,
        Err(d) => {
            eprint!(
                "{}",
                yavs_diag::render(
                    &SourceMap::default(),
                    &d,
                    MessageFormat::Human,
                    ColorChoice::Auto.use_color()
                )
            );
            return ExitCode::from(2);
        }
    };
    let cli = parse_hot(&args).unwrap_or_else(|| Cli::parse_from(&args));
    if let Some(code) = &cli.top.explain {
        return match explain::explain(&code.to_ascii_uppercase()) {
            Some(text) => {
                let code = code.to_ascii_uppercase();
                println!("{code}: {text}");
                if code == "E0120" {
                    println!("This yavs has {}.", elab::tiers_built());
                }
                ExitCode::SUCCESS
            }
            None => {
                eprintln!("error: no such diagnostic code `{code}`");
                ExitCode::from(2)
            }
        };
    }
    let Some(command) = cli.command else {
        use clap::CommandFactory;
        Cli::command()
            .error(
                clap::error::ErrorKind::MissingSubcommand,
                "a subcommand or --explain CODE is required",
            )
            .exit()
    };
    let color = match cli.top.color {
        Color::Auto => ColorChoice::Auto,
        Color::Always => ColorChoice::Always,
        Color::Never => ColorChoice::Never,
    }
    .use_color();
    let format = match cli.top.message_format {
        Format::Human => MessageFormat::Human,
        Format::Short => MessageFormat::Short,
        Format::Json => MessageFormat::Json,
    };
    let encoding = match cli.top.encoding {
        EncodingArg::Auto => Encoding::Auto,
        EncodingArg::Latin1 => Encoding::Latin1,
        EncodingArg::Utf8 => Encoding::Utf8,
    };
    // A panic is an internal compiler error (E9999, exit 3), never a bare 101. `analyze` also
    // catches its own to name the file.
    std::panic::set_hook(Box::new(|_| {}));
    let (std, defines, psl, dump_ast, files) = match command {
        Command::Parse {
            std,
            defines,
            psl,
            dump_ast,
            files,
        } => (std, defines, psl, dump_ast, files),
        Command::Analyze(a) => return analyze::run(a, format, color, encoding),
        Command::Elab(a) => return guard(format, color, || elab::elab(a, format, color, encoding)),
        Command::Run(a) if a.gui => return gui::launch(&args, format, color),
        Command::Run(a) => return guard(format, color, || elab::run(a, format, color, encoding)),
        Command::Cov(c) => return guard(format, color, || cov::run(c, format, color)),
        Command::Cache(c) => return guard(format, color, || cache::run(c, format, color)),
    };
    let opts = Options {
        std: match std {
            StdArg::V1993 => Std::Vhdl1993,
            StdArg::V2008 => Std::Vhdl2008,
            StdArg::V2019 => Std::Vhdl2019,
        },
        defines,
        psl,
    };

    let mut sources = SourceMap::default();
    let mut errors = 0usize;
    let stderr = std::io::stderr();
    let mut emit = |sources: &SourceMap, d: &Diagnostic| {
        if d.severity == Severity::Error {
            errors += 1;
        }
        let _ = stderr
            .lock()
            .write_all(yavs_diag::render(sources, d, format, color).as_bytes());
    };
    for path in files {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                let d =
                    Diagnostic::error(READ_ERROR, format!("cannot read `{}`: {e}", path.display()));
                emit(&sources, &d);
                continue;
            }
        };
        let (file, bad) =
            yavs_syntax::load_source(&mut sources, path.display().to_string(), bytes, encoding);
        if let Some(d) = bad {
            emit(&sources, &d);
            continue;
        }
        let parsed = yavs_syntax::parse_file(file, sources.get(file), &opts);
        for d in &parsed.diags {
            emit(&sources, d);
        }
        if dump_ast && let Some(design) = parsed.design {
            let _ = writeln!(std::io::stdout().lock(), "{design:#?}");
        }
    }
    if errors > 0 {
        if format == MessageFormat::Human {
            let s = if errors == 1 { "" } else { "s" };
            let d = Diagnostic::error("", format!("aborting due to {errors} previous error{s}"));
            let _ = stderr
                .lock()
                .write_all(yavs_diag::render(&sources, &d, format, color).as_bytes());
        }
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_becomes_exit_3() {
        let r = guard(MessageFormat::Short, false, || panic!("boom"));
        assert_eq!(format!("{r:?}"), format!("{:?}", ExitCode::from(3)));
    }

    /// The command line of the hot subcommands parses to what the whole command gives, and
    /// whatever is not plain (help, errors, other subcommands) is left to the whole command.
    #[test]
    fn hot_parse_equals_the_whole_command() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let same = [
            &["yavs", "run", "--work=w:d", "top"][..],
            &[
                "yavs",
                "--color",
                "never",
                "run",
                "--work=w:d",
                "top",
                "-gN=3",
                "--stop-time=10ns",
                "--threads",
                "4",
                "+arg=1",
            ],
            &[
                "yavs",
                "run",
                "--color=always",
                "--message-format",
                "json",
                "--work=w:d",
                "t",
            ],
            &[
                "yavs",
                "--encoding=latin1",
                "analyze",
                "--work=w:d",
                "--std=2019",
                "a.vhd",
                "b.vhd",
            ],
            &[
                "yavs",
                "analyze",
                "--work=w:d",
                "-Werror",
                "-D",
                "A=1",
                "--relaxed",
                "--psl",
                "a.vhd",
            ],
            &[
                "yavs",
                "elab",
                "--std=2019",
                "--work=w:d",
                "--map=x:y",
                "top",
            ],
            &[
                "yavs",
                "--message-format",
                "short",
                "elab",
                "--work=w:d",
                "top",
                "-gN=3",
            ],
        ];
        for a in same {
            let a = args(a);
            let hot = parse_hot(&a).unwrap_or_else(|| panic!("not hot: {a:?}"));
            let full = Cli::try_parse_from(&a).unwrap();
            assert_eq!(format!("{hot:?}"), format!("{full:?}"), "{a:?}");
        }
        // Repeats, global options after the subcommand and plusargs parse alike.
        for a in [
            &["yavs", "run", "--work=w:d", "--std=2008", "--std=2019", "t"][..],
            &["yavs", "run", "--work=w:d", "t", "+x=1", "--color", "never"],
            &[
                "yavs",
                "analyze",
                "--work=w:d",
                "--message-format",
                "json",
                "a.vhd",
            ],
        ] {
            let a = args(a);
            let hot = parse_hot(&a).unwrap_or_else(|| panic!("not hot: {a:?}"));
            let full = Cli::try_parse_from(&a).unwrap();
            assert_eq!(format!("{hot:?}"), format!("{full:?}"), "{a:?}");
        }
        // What the whole command rejects the pruned one rejects too (and the whole command says why).
        for a in [
            &["yavs", "run", "--work=w:d"][..],
            &["yavs", "run", "t"],
            &["yavs", "run", "--work=w:d", "--tier=bogus", "t"],
            &["yavs", "run", "--color=purple", "--work=w:d", "t"],
            &["yavs", "elab", "--work=w:d", "--tier=interp", "t"],
            &["yavs", "analyze", "--work=w:d"],
            &["yavs", "analyze", "--work=w:d", "--threads=2", "a.vhd"],
        ] {
            let a = args(a);
            assert!(Cli::try_parse_from(&a).is_err(), "{a:?}");
            assert!(parse_hot(&a).is_none(), "{a:?}");
        }
        let whole = [
            &["yavs"][..],
            &["yavs", "--help"],
            &["yavs", "--version"],
            &["yavs", "--explain", "E0002"],
            &["yavs", "run", "--help"],
            &["yavs", "run", "-h"],
            &["yavs", "parse", "x.vhd"],
            &["yavs", "cov", "merge"],
            &["yavs", "run"],
            &["yavs", "run", "--bogus", "t"],
            &["yavs", "--bogus", "run", "t"],
            &["yavs", "ru", "t"],
        ];
        for a in whole {
            assert!(parse_hot(&args(a)).is_none(), "{a:?}");
        }
    }

    /// `yavs --version` keeps the shape of docs/cli-contract.md and says whether the compiled
    /// tier is built in (#780).
    #[test]
    fn version_names_the_compiled_tier() {
        use clap::CommandFactory;
        let v = Cli::command().render_version();
        let head = format!("yavs {} (", env!("CARGO_PKG_VERSION"));
        assert!(v.starts_with(&head), "{v}");
        assert_eq!(
            v.contains("compiled tier: llvm"),
            cfg!(feature = "llvm"),
            "{v}"
        );
        assert_eq!(v.contains("cranelift"), cfg!(feature = "clift"), "{v}");
    }

    /// The default tier (#125): `auto` with a compiled tier (`llvm` or `clift`), also under
    /// `--vhpi`; `interp` under `--dap` and without one; an explicit `--tier=` wins.
    #[test]
    fn default_tier() {
        let tier = |extra: &[&str]| {
            let cli = Cli::try_parse_from(["yavs", "run", "--work=w:d", "top"].iter().chain(extra))
                .unwrap();
            match cli.command {
                Some(Command::Run(a)) => a.tier(),
                _ => unreachable!(),
            }
        };
        let auto = if cfg!(all(feature = "clift", not(feature = "llvm"))) {
            elab::TierArg::CliftAuto
        } else if cfg!(feature = "llvm") {
            elab::TierArg::Auto
        } else {
            elab::TierArg::Interp
        };
        assert!(tier(&[]) == auto);
        assert!(tier(&["--vhpi=plugin.so"]) == auto);
        assert!(tier(&["--dap=stdio"]) == elab::TierArg::Interp);
        if cfg!(all(feature = "clift", not(feature = "llvm"))) {
            assert!(tier(&["--tier=auto"]) == elab::TierArg::CliftAuto);
        }
        assert!(tier(&["--vhpi=plugin.so", "--tier=interp"]) == elab::TierArg::Interp);
    }
}
