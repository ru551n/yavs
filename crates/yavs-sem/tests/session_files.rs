//! One session over several files gives the per-file result of `yavs analyze`: a file that
//! failed is not recorded, so its units, even the ones that analyzed, are not visible to later
//! files, and a file that needs them fails (silently, 8.2) instead of passing.

use yavs_sem::{Db, LangRev, Options, default_stdlib};

#[test]
fn failed_file_hides_its_units() {
    let tmp = tempfile::tempdir().expect("scratch dir");
    let dir = tmp.path();
    let file = |name: &str, text: &str| {
        let p = dir.join(name);
        std::fs::write(&p, text).expect("write");
        p
    };
    // `good` analyzes, `bad` in the same file does not.
    let a = file(
        "a.vhd",
        "package good is\nend package;\npackage bad is\n  constant c : integer := undeclared;\nend package;\n",
    );
    let b = file(
        "b.vhd",
        "use work.good.all;\npackage user is\nend package;\n",
    );
    let c = file("c.vhd", "use work.bad.all;\nentity e is\nend entity;\n");
    let mut db = Db::new(Options::new(LangRev::Vhdl2008, default_stdlib()));
    assert!(db.analyze_file("work", &a, LangRev::Vhdl2008).is_err());
    let errors = db.error_count();
    assert!(db.analyze_file("work", &b, LangRev::Vhdl2008).is_err());
    assert!(db.analyze_file("work", &c, LangRev::Vhdl2008).is_err());
    let msgs: Vec<String> = db
        .diagnostics()
        .iter()
        .map(|d| format!("{} {}", d.code, d.message))
        .collect();
    assert_eq!(db.error_count(), errors, "cascades stay silent: {msgs:?}");
}
