//! Diagnostic snapshot tests, in the style of rustc's UI tests.
//!
//! Every `tests/diag/NAME.vhd` is parsed by the `yavs` binary with colour off,
//! and its stderr must equal `tests/diag/NAME.stderr`. A first line
//! `-- yavs: ARGS` adds command line arguments. `YAVS_BLESS=1` rewrites the
//! expected files from the current output.

use std::path::Path;
use std::process::Command;

#[test]
fn snapshots() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/diag");
    let bless = std::env::var_os("YAVS_BLESS").is_some_and(|v| v == "1");
    let mut inputs: Vec<_> = std::fs::read_dir(&dir)
        .expect("tests/diag")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "vhd"))
        .collect();
    inputs.sort();
    assert!(!inputs.is_empty());
    let mut failed = Vec::new();
    for input in &inputs {
        let name = input.file_name().and_then(|n| n.to_str()).expect("name");
        let src = std::fs::read(input).expect("read");
        let first = String::from_utf8_lossy(&src)
            .lines()
            .next()
            .unwrap_or_default()
            .to_string();
        let mut args = vec!["--color", "never", "parse"];
        if let Some(extra) = first.strip_prefix("-- yavs:") {
            args.extend(extra.split_whitespace());
        }
        let out = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(&args)
            .arg(name)
            .current_dir(&dir)
            .env_remove("NO_COLOR")
            .output()
            .expect("run yavs");
        let stderr = String::from_utf8(out.stderr).expect("utf-8 stderr");
        let has_error = stderr.contains("error[") || stderr.contains("\"severity\":\"error\"");
        assert_eq!(
            !out.status.success(),
            has_error,
            "{name}: exit status {:?} does not match the diagnostics",
            out.status
        );
        let expected_path = input.with_extension("stderr");
        if bless {
            std::fs::write(&expected_path, &stderr).expect("write");
            continue;
        }
        let expected = std::fs::read_to_string(&expected_path).unwrap_or_default();
        if expected != stderr {
            failed.push(format!(
                "--- {name}: expected\n{expected}\n--- got\n{stderr}"
            ));
        }
    }
    assert!(
        failed.is_empty(),
        "{}\nRerun with YAVS_BLESS=1 to accept the new output.",
        failed.join("\n")
    );
}
