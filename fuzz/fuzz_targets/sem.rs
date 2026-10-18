//! Parser and semantic analysis against yavs's std/ieee must never panic, hang or report an
//! internal error ("this is a bug in yavs"). A hang is caught by libFuzzer's `-timeout`; the
//! parser's own fuel guard ends stalls. Run with `-rss_limit_mb=2048 -timeout=20`.
#![no_main]

use libfuzzer_sys::fuzz_target;
use std::path::PathBuf;
use std::sync::OnceLock;
use yavs_sem::{Db, LangRev, Options, default_stdlib};

fn stdlib() -> &'static PathBuf {
    static P: OnceLock<PathBuf> = OnceLock::new();
    P.get_or_init(default_stdlib)
}

fuzz_target!(|data: &[u8]| {
    // The loader reads files, so each worker process has one scratch file (tmpfs).
    let path = PathBuf::from(format!("/dev/shm/yavs-sem-fuzz-{}.vhd", std::process::id()));
    if std::fs::write(&path, data).is_err() {
        return;
    }
    for rev in [LangRev::Vhdl2008, LangRev::Vhdl2019] {
        let mut db = Db::new(Options::new(rev, stdlib()));
        let _ = db.analyze_file("work", &path, rev);
        for d in db.diagnostics() {
            // Unimplemented slices are expected; an ICE is not.
            assert!(
                !d.notes.iter().any(|n| n.contains("this is a bug in yavs")),
                "ICE: {}",
                d.message
            );
            yavs_diag::render(&db.sources, d, yavs_diag::MessageFormat::Human, false);
        }
    }
});
