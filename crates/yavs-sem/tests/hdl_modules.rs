//! hdl-modules v6.2.1 files that use user-defined attributes, analyzed from the corpus
//! fetched by `tools/fetch-corpus.sh` (`YAVS_CORPUS`, default `~/.cache/rsim-corpus`).
//! Built only with `--features corpus`: `cargo test -p yavs-sem --features corpus`.
//! `tests/user_attributes.rs` covers the same patterns with sources written for yavs.
//!
//! A test marked `ignore` is blocked by something other than attributes; the reason says what.

use std::path::PathBuf;

use yavs_diag::{MessageFormat, render};
use yavs_sem::{Db, LangRev, Options, default_stdlib};

const ATTRIBUTE_PKG: (&str, &str) = ("common", "common/src/attribute_pkg.vhd");
const COMMON_PKG: (&str, &str) = ("common", "common/src/common_pkg.vhd");
const TYPES_PKG: (&str, &str) = ("common", "common/src/types_pkg.vhd");
const MATH_PKG: (&str, &str) = ("math", "math/src/math_pkg.vhd");

/// Analyze `(library, path under modules/)` in order; every file must be clean.
fn analyze(files: &[(&str, &str)]) {
    let root = std::env::var_os("YAVS_CORPUS").map_or_else(
        || {
            std::env::home_dir()
                .expect("home directory")
                .join(".cache/rsim-corpus")
        },
        PathBuf::from,
    );
    let modules = root.join("hdl-modules/modules");
    assert!(
        modules.is_dir(),
        "{} not found: run tools/fetch-corpus.sh (and set YAVS_CORPUS to its directory)",
        modules.display()
    );
    let mut db = Db::new(Options::new(LangRev::Vhdl2008, default_stdlib()));
    for (lib, path) in files {
        let r = db.analyze_file(lib, &modules.join(path), LangRev::Vhdl2008);
        let text: String = db
            .diagnostics()
            .iter()
            .map(|d| render(&db.sources, d, MessageFormat::Human, false))
            .collect();
        assert!(r.is_ok() && db.diagnostics().is_empty(), "{path}:\n{text}");
    }
}

/// The attribute declarations themselves, with `to_attribute` in the body.
#[test]
fn attribute_pkg() {
    analyze(&[ATTRIBUTE_PKG]);
}

/// `use common.attribute_pkg.all` + `attribute dont_touch of data_in_int : signal is "true";`.
#[test]
fn resync_level_dont_touch() {
    analyze(&[ATTRIBUTE_PKG, ("resync", "resync/src/resync_level.vhd")]);
}

/// `attribute iob of x : signal is if_then_else(enable_iob, "true", "false");`.
#[test]
fn debounce_iob_async_reg() {
    analyze(&[
        ATTRIBUTE_PKG,
        COMMON_PKG,
        ("resync", "resync/src/resync_level.vhd"),
        ("common", "common/src/debounce.vhd"),
    ]);
}

/// `attribute ram_style of mem : signal is to_attribute(ram_type);`.
#[test]
fn fifo_ram_style() {
    analyze(&[
        ATTRIBUTE_PKG,
        TYPES_PKG,
        MATH_PKG,
        ("common", "common/src/handshake_pipeline.vhd"),
        ("fifo", "fifo/src/fifo.vhd"),
    ]);
}

/// `attribute latency of sine_lookup : entity is 3;` and `sine_lookup'latency` in its own
/// architecture (a constant, then `1 to our_latency - 1` ranges).
#[test]
fn sine_lookup_latency() {
    analyze(&[
        TYPES_PKG,
        ("sine_generator", "sine_generator/src/sine_lookup.vhd"),
    ]);
}

/// `work.sine_lookup'latency` and `work.taylor_expansion_core'latency` read in
/// sine_calculator, where `latency` is not visible (constants, `1 to lookup_latency - 1`),
/// and `use_dsp` from attribute_pkg.
#[test]
#[ignore = "math/saturate_signed.vhd:104: E1325 for `result <= (others => ..)` (stmt, not attributes)"]
fn sine_calculator_latency() {
    analyze(&[
        ATTRIBUTE_PKG,
        TYPES_PKG,
        MATH_PKG,
        ("math", "math/src/saturate_signed.vhd"),
        (
            "sine_generator",
            "sine_generator/src/sine_generator_pkg.vhd",
        ),
        ("sine_generator", "sine_generator/src/sine_lookup.vhd"),
        (
            "sine_generator",
            "sine_generator/src/taylor_expansion_core.vhd",
        ),
        ("sine_generator", "sine_generator/src/sine_calculator.vhd"),
    ]);
}
