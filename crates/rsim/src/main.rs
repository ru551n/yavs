//! The `rsim` command line.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

mod analyze;

use clap::{Parser, Subcommand, ValueEnum};
use rsim_diag::{ColorChoice, Diagnostic, MessageFormat, Severity, SourceMap};
use rsim_syntax::{Encoding, Options, Std};

#[derive(Parser)]
#[command(name = "rsim", version, about = "A VHDL simulator")]
struct Cli {
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
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check that VHDL files parse.
    Parse {
        /// Language standard.
        #[arg(long, value_enum, default_value = "2008")]
        std: StdArg,
        /// Define a conditional analysis identifier, `NAME=VALUE`.
        #[arg(long = "define", short = 'D', value_name = "NAME=VALUE", value_parser = parse_define)]
        defines: Vec<(String, String)>,
        /// Print the syntax tree (for debugging rsim).
        #[arg(long, hide = true)]
        dump_ast: bool,
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Analyze one VHDL file into a library.
    Analyze(analyze::Args),
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum StdArg {
    #[value(name = "2008")]
    V2008,
    #[value(name = "2019")]
    V2019,
}

#[derive(Clone, Copy, ValueEnum)]
enum EncodingArg {
    Auto,
    Latin1,
    Utf8,
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum Color {
    Auto,
    Always,
    Never,
}

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

/// Code for a file that cannot be read.
const READ_ERROR: &str = "E0100";

fn main() -> ExitCode {
    let args = match analyze::expand_response_files(
        std::env::args_os().map(|a| a.to_string_lossy().into_owned()),
    ) {
        Ok(a) => a,
        Err(d) => {
            eprint!(
                "{}",
                rsim_diag::render(
                    &SourceMap::default(),
                    &d,
                    MessageFormat::Human,
                    ColorChoice::Auto.use_color()
                )
            );
            return ExitCode::from(2);
        }
    };
    let cli = Cli::parse_from(args);
    let color = match cli.color {
        Color::Auto => ColorChoice::Auto,
        Color::Always => ColorChoice::Always,
        Color::Never => ColorChoice::Never,
    }
    .use_color();
    let format = match cli.message_format {
        Format::Human => MessageFormat::Human,
        Format::Short => MessageFormat::Short,
        Format::Json => MessageFormat::Json,
    };
    let encoding = match cli.encoding {
        EncodingArg::Auto => Encoding::Auto,
        EncodingArg::Latin1 => Encoding::Latin1,
        EncodingArg::Utf8 => Encoding::Utf8,
    };
    let (std, defines, dump_ast, files) = match cli.command {
        Command::Parse {
            std,
            defines,
            dump_ast,
            files,
        } => (std, defines, dump_ast, files),
        Command::Analyze(a) => return analyze::run(a, format, color, encoding),
    };
    let opts = Options {
        std: match std {
            StdArg::V2008 => Std::Vhdl2008,
            StdArg::V2019 => Std::Vhdl2019,
        },
        defines,
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
            .write_all(rsim_diag::render(sources, d, format, color).as_bytes());
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
            rsim_syntax::load_source(&mut sources, path.display().to_string(), bytes, encoding);
        if let Some(d) = bad {
            emit(&sources, &d);
            continue;
        }
        let parsed = rsim_syntax::parse_file(file, sources.get(file), &opts);
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
                .write_all(rsim_diag::render(&sources, &d, format, color).as_bytes());
        }
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
