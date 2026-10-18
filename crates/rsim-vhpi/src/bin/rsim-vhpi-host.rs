//! Spike host for the VHPI export mechanism: an executable that exports
//! `vhpi_*` the way `rsim` will, loads plugins and runs a simulation that
//! only fires the start and end of simulation callbacks. It stands in for
//! the `rsim` CLI until that exists.
//!
//! Usage: rsim-vhpi-host [--top NAME] [--vhpi LIB[:ENTRY]]...

use std::process::ExitCode;

use rsim_diag::{ColorChoice, MessageFormat, SourceMap, render};
use rsim_vhpi::sys::{vhpiCbEndOfSimulation, vhpiCbStartOfSimulation};
use rsim_vhpi::{Control, PluginSpec};

fn main() -> ExitCode {
    let mut top = "top".to_owned();
    let mut plugins = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match (arg.as_str(), args.next()) {
            ("--top", Some(v)) => top = v,
            ("--vhpi", Some(v)) => plugins.push(v.parse::<PluginSpec>().unwrap()),
            _ => {
                eprintln!("usage: rsim-vhpi-host [--top NAME] [--vhpi LIB[:ENTRY]]...");
                return ExitCode::from(2);
            }
        }
    }

    rsim_vhpi::set_design_root(&top);
    for p in &plugins {
        if let Err(e) = rsim_vhpi::load(p) {
            let color = ColorChoice::Auto.use_color();
            eprint!(
                "{}",
                render(
                    &SourceMap::default(),
                    &e.to_diagnostic(),
                    MessageFormat::Human,
                    color
                )
            );
            return ExitCode::from(2);
        }
    }
    rsim_vhpi::run_callbacks(vhpiCbStartOfSimulation);
    match rsim_vhpi::take_control() {
        Some(Control::Finish) => println!("host: finish requested"),
        Some(Control::Stop) => println!("host: stop requested"),
        None => println!("host: ran to completion"),
    }
    rsim_vhpi::run_callbacks(vhpiCbEndOfSimulation);
    ExitCode::SUCCESS
}
