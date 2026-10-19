//! `yavs-gui`: the graphical debugger for yavs (docs/design/gui.md). It starts
//! `yavs run <args> --dap=stdio`, or attaches to a `yavs run --dap=PORT`, and speaks DAP only.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

mod app;
mod dap;
mod panels;
mod state;

use dap::Target;

const USAGE: &str = "usage: yavs-gui [--yavs PATH] -- <yavs run arguments>
       yavs-gui --attach HOST:PORT";

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Target, String> {
    let mut yavs = None;
    let mut attach = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--" if attach.is_none() => {
                let args: Vec<String> = args.collect();
                if args.is_empty() {
                    break;
                }
                return Ok(Target::Launch { yavs, args });
            }
            "--yavs" => yavs = Some(PathBuf::from(args.next().ok_or("--yavs needs a PATH")?)),
            "--attach" => attach = Some(args.next().ok_or("--attach needs HOST:PORT")?),
            _ => return Err(format!("unexpected argument `{a}`")),
        }
    }
    match (attach, yavs) {
        (Some(addr), None) => Ok(Target::Attach(addr)),
        (Some(_), _) => Err("--attach takes neither --yavs nor run arguments".into()),
        (None, _) => Err("no yavs run arguments given after `--`".into()),
    }
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1).peekable();
    match args.peek().map(String::as_str) {
        Some("--help" | "-h") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some("--version" | "-V") => {
            println!("yavs-gui {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        _ => {}
    }
    let target = match parse_args(args) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let r = eframe::run_native(
        "yavs-gui",
        eframe::NativeOptions::default(),
        Box::new(move |cc| {
            let ctx = cc.egui_ctx.clone();
            Ok(Box::new(app::App::new(
                &target,
                Arc::new(move || ctx.request_repaint()),
            )))
        }),
    );
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests;
