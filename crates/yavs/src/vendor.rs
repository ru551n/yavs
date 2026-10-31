//! `yavs vendor vivado`: the vendor simulation libraries analyzed from the user's own install
//! (docs/cli-contract.md section 3, #119). The sources stay in the install and are never copied.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args as ClapArgs, Subcommand};
use yavs_diag::{Diagnostic, MessageFormat, SourceMap};
use yavs_syntax::Encoding;

use crate::{StdArg, analyze};

#[cfg_attr(test, derive(Debug))]
#[derive(Subcommand)]
pub enum Command {
    /// Build `unisim`, `unimacro` and `unifast` from a Vivado install into `DIR/<library>`.
    Vivado(Args),
}

#[cfg_attr(test, derive(Debug))]
#[derive(ClapArgs)]
pub struct Args {
    /// The directory of the install that holds `data/vhdl/src` (default: `$XILINX_VIVADO`).
    #[arg(long, value_name = "DIR")]
    vivado: Option<PathBuf>,
    /// The directory the libraries are built in.
    #[arg(long, value_name = "DIR", required = true)]
    out: PathBuf,
    /// Language standard the sources are analyzed with.
    #[arg(long, value_enum, default_value = "2008")]
    std: StdArg,
    /// Directory of the std/ieee units shared by all libraries, or `off`, as for `yavs analyze`.
    #[arg(long, value_name = "DIR|off")]
    std_cache: Option<std::ffi::OsString>,
}

/// The file that says what `--out` was built from: from anything else it is built afresh, so no
/// library keeps a unit of another install, and no library of another yavs is met.
const MARKER: &str = "yavs-vendor.txt";

fn missing(what: String, note: String) -> Box<Diagnostic> {
    Box::new(Diagnostic::error("E0129", what).with_note(note).with_help(
        "pass `--vivado=DIR` with the directory of your Vivado install that holds \
             `data/vhdl/src`, or source the `settings64.sh` of the install",
    ))
}

/// `data/vhdl/src` of the install named by `--vivado` or `XILINX_VIVADO`.
fn find_sources(arg: Option<PathBuf>) -> Result<PathBuf, Box<Diagnostic>> {
    let (dir, from) = match arg {
        Some(d) => (d, "--vivado"),
        None => match std::env::var_os("XILINX_VIVADO").filter(|v| !v.is_empty()) {
            Some(d) => (PathBuf::from(d), "XILINX_VIVADO"),
            None => {
                return Err(missing(
                    "no Vivado install to build the vendor libraries from".into(),
                    "`--vivado` is not given and `XILINX_VIVADO` is not set".into(),
                ));
            }
        },
    };
    let src = dir.join("data/vhdl/src");
    if !src.join("unisims").is_dir() {
        return Err(missing(
            format!("`{}` is not a Vivado install", dir.display()),
            format!(
                "`{}` (from `{from}`) has no directory `{}`",
                dir.display(),
                src.join("unisims").display()
            ),
        ));
    }
    // The libraries name their sources by this path: it must not depend on the working directory.
    std::path::absolute(&src)
        .map_err(|e| missing(format!("`{}` cannot be used", src.display()), e.to_string()))
}

/// The files of directory `dir` in the order of its own list.
fn listed(src: &Path, dir: &str) -> Result<Vec<PathBuf>, Box<Diagnostic>> {
    let d = src.join(dir);
    let list = d.join("vhdl_analyze_order");
    let text = std::fs::read_to_string(&list).map_err(|e| {
        missing(
            format!("the Vivado install has no list of `{dir}`"),
            format!("cannot read `{}`: {e}", list.display()),
        )
    })?;
    Ok(text.split_whitespace().map(|f| d.join(f)).collect())
}

/// The libraries in build order. `unisims/secureip` is left out: its wrappers need models that
/// the install holds only in encrypted Verilog.
fn libraries(src: &Path) -> Result<Vec<(&'static str, Vec<PathBuf>)>, Box<Diagnostic>> {
    let mut unisim = vec![
        src.join("unisims/unisim_VPKG.vhd"),
        src.join("unisims/unisim_retarget_VCOMP.vhd"),
    ];
    unisim.extend(listed(src, "unisims/primitive")?);
    unisim.extend(listed(src, "unisims/retarget")?);
    Ok(vec![
        ("unisim", unisim),
        ("unimacro", listed(src, "unimacro")?),
        ("unifast", listed(src, "unifast/primitive")?),
    ])
}

pub fn run(c: Command, format: MessageFormat, color: bool, encoding: Encoding) -> ExitCode {
    let Command::Vivado(a) = c;
    let libs = find_sources(a.vivado).and_then(|src| Ok((libraries(&src)?, src)));
    let (libs, src) = match libs {
        Ok(l) => l,
        Err(d) => {
            eprint!(
                "{}",
                yavs_diag::render(&SourceMap::default(), &d, format, color)
            );
            return ExitCode::from(2);
        }
    };
    let std = match a.std {
        StdArg::V1993 => "93",
        StdArg::V2008 => "2008",
        StdArg::V2019 => "2019",
    };
    let marker = format!("{}\n{std}\n{}\n", env!("CARGO_PKG_VERSION"), src.display());
    let marker_file = a.out.join(MARKER);
    let built = std::fs::read_to_string(&marker_file).is_ok_and(|m| m == marker);
    let write = (|| {
        if !built {
            for (name, _) in &libs {
                let dir = a.out.join(name);
                if dir.exists() {
                    std::fs::remove_dir_all(dir)?;
                }
            }
        }
        std::fs::create_dir_all(&a.out)?;
        std::fs::write(&marker_file, &marker)
    })();
    if let Err(e) = write {
        let d = Diagnostic::error(
            "L0010",
            format!("cannot prepare `{}`: {e}", a.out.display()),
        );
        eprint!(
            "{}",
            yavs_diag::render(&SourceMap::default(), &d, format, color)
        );
        return ExitCode::from(1);
    }
    let spec = |name: &str| format!("{name}:{}", a.out.join(name).display());
    for (i, (name, files)) in libs.iter().enumerate() {
        // Every library after `unisim` uses it.
        let maps = if i == 0 { vec![] } else { vec![spec("unisim")] };
        let args =
            analyze::Args::batch(a.std, spec(name), maps, a.std_cache.clone(), files.clone());
        let code = analyze::run(args, format, color, encoding);
        if code != ExitCode::SUCCESS {
            return code;
        }
        println!("{name}: {} files", files.len());
    }
    let maps: Vec<String> = libs
        .iter()
        .map(|(n, _)| format!("--map={}", spec(n)))
        .collect();
    println!("Use them with: {}", maps.join(" "));
    ExitCode::SUCCESS
}
