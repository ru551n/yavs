//! Developer tool: analyze Verilog files and print the diagnostics.
//!   vcheck [-I DIR] [-D NAME[=VAL]] [--each] [--quiet] FILE...
//! `--each` analyzes every file on its own (macros do not carry over) and prints counts per code.

use std::collections::BTreeMap;
use std::path::PathBuf;

use yavs_verilog::{Options, Severity, analyze};

fn main() {
    let mut opts = Options::default();
    let mut files = Vec::new();
    let (mut each, mut quiet) = (false, false);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-I" => opts.include_dirs.push(args.next().unwrap().into()),
            "-D" => {
                let d = args.next().unwrap();
                match d.split_once('=') {
                    Some((n, v)) => opts.defines.push((n.into(), Some(v.into()))),
                    None => opts.defines.push((d, None)),
                }
            }
            "--each" => each = true,
            "--quiet" => quiet = true,
            _ => files.push(PathBuf::from(a)),
        }
    }
    let groups: Vec<Vec<PathBuf>> = if each {
        files.iter().map(|f| vec![f.clone()]).collect()
    } else {
        vec![files]
    };
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let (mut units, mut failed) = (0, 0);
    for g in groups {
        let r = analyze(&g, &opts);
        units += r.units.len();
        let mut bad = false;
        for d in &r.diagnostics {
            *counts
                .entry(format!(
                    "{}{}",
                    d.code,
                    if d.severity == Severity::Error {
                        ""
                    } else {
                        " (warn)"
                    }
                ))
                .or_default() += 1;
            bad |= d.severity == Severity::Error;
            if quiet && d.severity != Severity::Error {
                continue;
            }
            let loc = d.span.map(|s| {
                let p = &r.files[s.file];
                let text = std::fs::read(p).unwrap_or_default();
                let line = text[..(s.start as usize).min(text.len())]
                    .iter()
                    .filter(|&&c| c == b'\n')
                    .count()
                    + 1;
                format!("{}:{}", p.display(), line)
            });
            println!(
                "{} {} {}: {}",
                loc.unwrap_or_default(),
                d.code,
                if d.severity == Severity::Error {
                    "error"
                } else {
                    "warning"
                },
                d.message
            );
            if !quiet {
                for n in &d.notes {
                    println!("    note: {n}");
                }
            }
        }
        failed += bad as usize;
    }
    println!("units: {units}, files with errors: {failed}");
    for (c, n) in counts {
        println!("  {c}: {n}");
    }
}
