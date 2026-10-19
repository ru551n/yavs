//! `yavs run --gui`: start `yavs-gui` on the same run arguments (`docs/cli-contract.md` 6). A
//! process start only: the GUI is EUPL-1.2 and never linked (docs/design/gui.md section 7).

use std::ffi::OsString;
use std::io::{ErrorKind, Write};
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use yavs_diag::{Diagnostic, MessageFormat, SourceMap};

/// `args` is the expanded command line (program name first). The GUI starts this executable as
/// `yavs run <args> --dap=stdio`, so the run arguments go through unchanged without `run` and
/// `--gui`.
pub fn launch(args: &[String], format: MessageFormat, color: bool) -> ExitCode {
    let name = format!("yavs-gui{}", std::env::consts::EXE_SUFFIX);
    let exe = std::env::current_exe().ok();
    let gui = exe
        .as_ref()
        .and_then(|e| Some(e.parent()?.join(&name)))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(&name));
    let mut cmd = Command::new(&gui);
    if let Some(exe) = &exe {
        cmd.arg("--yavs").arg(exe);
    }
    cmd.arg("--").args(run_args(args));
    match cmd.status() {
        // A signal (no code) is a failure.
        Ok(s) => ExitCode::from(s.code().map_or(1, |c| u8::try_from(c).unwrap_or(1))),
        Err(e) => {
            let d = if e.kind() == ErrorKind::NotFound {
                Diagnostic::error("E0122", "`yavs-gui` was not found")
                    .with_note("looked next to the yavs executable and on PATH")
                    .with_help("install the release archive, which ships `yavs-gui` next to `yavs`, or put `yavs-gui` on PATH")
            } else {
                Diagnostic::error("E0122", format!("cannot start `{}`: {e}", gui.display()))
            };
            let _ = std::io::stderr()
                .lock()
                .write_all(yavs_diag::render(&SourceMap::default(), &d, format, color).as_bytes());
            ExitCode::from(2)
        }
    }
}

/// The arguments after the program name without the first `run` (the subcommand: global options
/// before it take only fixed values) and every `--gui`.
fn run_args(args: &[String]) -> Vec<OsString> {
    let mut seen_run = false;
    args.iter()
        .skip(1)
        .filter(|a| match a.as_str() {
            "run" if !seen_run => {
                seen_run = true;
                false
            }
            "--gui" => false,
            _ => true,
        })
        .map(OsString::from)
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn run_args() {
        let a: Vec<String> = ["yavs", "--color=never", "run", "--gui", "--work=w:d", "run"]
            .map(String::from)
            .into();
        assert_eq!(
            super::run_args(&a),
            ["--color=never", "--work=w:d", "run"].map(std::ffi::OsString::from)
        );
    }
}
