//! A configuration declaration and its entity: one library before VHDL-2019 (LRM 3.4.1),
//! any library from 2019 on (LCS-2016-023).

use std::path::PathBuf;

use yavs_sem::{Db, LangRev, Options, default_stdlib};

const ENTITY: &str =
    "entity e is end; architecture a of e is begin end; architecture b of e is begin end;";
const CONFIG: &str =
    "library other; configuration c of other.e is for b end for; end configuration;";

/// Analyze the entity into `other`, then the configuration into `work`; the diagnostics
/// codes of the second file.
fn config_codes(rev: LangRev, test: &str) -> Vec<String> {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(test);
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let mut db = Db::new(Options::new(rev, default_stdlib()));
    for (i, (lib, src)) in [("other", ENTITY), ("work", CONFIG)].iter().enumerate() {
        let path = dir.join(format!("f{i}.vhd"));
        std::fs::write(&path, src).expect("write source");
        let before = db.diagnostics().len();
        let _ = db.analyze_file(lib, &path, rev);
        if i == 1 {
            return db.diagnostics()[before..]
                .iter()
                .map(|d| d.code.to_string())
                .collect();
        }
        assert!(db.diagnostics().is_empty(), "entity file must be clean");
    }
    unreachable!()
}

#[test]
fn configuration_in_another_library_needs_2019() {
    assert_eq!(config_codes(LangRev::Vhdl2008, "cfg_lib_2008"), ["E1007"]);
    assert!(config_codes(LangRev::Vhdl2019, "cfg_lib_2019").is_empty());
}
