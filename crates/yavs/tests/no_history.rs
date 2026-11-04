//! A wide bus that keeps no history, as `yavs run` runs it (no counters, so compared and copied
//! whole), against one that keeps it: the same reports, the same waves, the same hang report.

mod common;

use std::path::Path;
use std::process::Command;

use common::tempdir;

/// Status, standard output, standard error and the bytes of the wave file of a run of `top`.
/// Without a wave nothing wants the exact runs, so that is what the whole compare does.
fn run(dir: &Path, top: &str, wave: bool, off: bool) -> (i32, String, String, Vec<u8>) {
    let file = dir.join(format!("{top}-{off}.fst"));
    let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
    c.current_dir(dir)
        .env_remove("YAVS_COUNTERS")
        .args(["--color", "never", "run", "--work", "work:lib"])
        .args([
            "--threads",
            "1",
            "--no-cache",
            "--hang-report",
            "--tier=interp",
        ]);
    if wave {
        c.arg("--wave").arg(&file);
    }
    if off {
        c.env("YAVS_NO_HISTORY", "off");
    }
    let o = c.arg(top).output().expect("run yavs");
    let text = |b: &[u8]| String::from_utf8_lossy(b).replace(&*file.to_string_lossy(), "W");
    (
        o.status.code().unwrap_or(-1),
        text(&o.stdout),
        text(&o.stderr),
        if wave {
            std::fs::read(&file).expect("wave file")
        } else {
            Vec::new()
        },
    )
}

#[test]
fn a_bus_with_no_history_runs_as_one_that_keeps_it() {
    let d = tempdir("no_history");
    let designs = [
        ("bare_bus", include_str!("perf/bare_bus.vhd")),
        ("bare_stuck", include_str!("perf/bare_stuck.vhd")),
        (
            "wide_history",
            include_str!("../../../tests/corpus/sigassign/wide_history.vhd"),
        ),
        (
            "wide_level",
            include_str!("../../../tests/corpus/sigassign/wide_level.vhd"),
        ),
    ];
    for (top, src) in designs {
        let file = format!("{top}.vhd");
        std::fs::write(d.join(&file), src).expect("write");
        let a = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .current_dir(&*d)
            .args(["analyze", "--work", "work:lib", &file])
            .output()
            .expect("analyze");
        assert!(
            a.status.success(),
            "{top}: {}",
            String::from_utf8_lossy(&a.stderr)
        );
        for wave in [false, true] {
            let (on, off) = (run(&d, top, wave, false), run(&d, top, wave, true));
            assert_eq!(
                (on.0, &on.1, &on.2),
                (off.0, &off.1, &off.2),
                "{top} {wave}"
            );
            assert!(on.3 == off.3, "{top}: the wave files differ");
        }
    }
}
