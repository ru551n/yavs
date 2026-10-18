use rsim_lib::{Libraries, Library, UnitKind, UnitRecord, UnitRef, closure, hash_content, render};
use std::path::{Path, PathBuf};
use std::process::Command;

const STD: &str = "2008";

/// Write `text` to `dir/name`, analyze it into `lib` with `units`, return the path.
fn analyze(lib: &Library, dir: &Path, name: &str, text: &str, units: Vec<UnitRecord>) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, text).unwrap();
    lib.update_file(STD, &p, &hash_content(text.as_bytes()), units)
        .unwrap();
    p
}

fn pkg(name: &str) -> UnitRecord {
    UnitRecord::new(UnitKind::Package, name, STD)
}

fn snap(d: &rsim_diag::Diagnostic, tmp: &Path) -> String {
    format!(
        "{}\n",
        render(d, false)
            .trim_end()
            .replace(tmp.to_str().unwrap(), "<tmp>")
    )
}

fn setup() -> (tempfile::TempDir, Libraries, Library, Library) {
    let t = tempfile::tempdir().unwrap();
    let work = Library::new("work", t.path().join("work"));
    let util = Library::new("util", t.path().join("util"));
    let mut libs = Libraries::new();
    libs.add("work", &work.dir);
    libs.add("util", &util.dir);
    (t, libs, work, util)
}

#[test]
fn reanalyze_replaces_entries() {
    let (t, _, work, _) = setup();
    let a = analyze(&work, t.path(), "a.vhd", "a1", vec![pkg("A"), pkg("B")]);
    analyze(&work, t.path(), "b.vhd", "b1", vec![pkg("C")]);
    let m = work.read().unwrap();
    assert_eq!(m.files.len(), 2);
    assert_eq!(m.find(UnitKind::Package, "a").unwrap().name, "A");
    // a.vhd no longer defines B, and moves to the end of the compile order.
    analyze(&work, t.path(), "a.vhd", "a2", vec![pkg("A")]);
    let m = work.read().unwrap();
    assert!(m.find(UnitKind::Package, "b").is_none());
    assert_eq!(m.files.last().unwrap().path, a);
    assert_eq!(m.files.last().unwrap().hash, hash_content(b"a2"));
    assert_eq!(m.files.len(), 2);
}

#[test]
fn closure_orders_across_libraries() {
    let (t, libs, work, util) = setup();
    let base = analyze(&util, t.path(), "base.vhd", "base", vec![pkg("Base")]);
    let body = analyze(
        &util,
        t.path(),
        "base_body.vhd",
        "body",
        vec![UnitRecord::package_body("Base", STD)],
    );
    let ent = analyze(
        &work,
        t.path(),
        "top.vhd",
        "top",
        vec![
            UnitRecord::new(UnitKind::Entity, "Top", STD).depends(
                "util",
                UnitKind::Package,
                "BASE",
            ),
            UnitRecord::architecture("Top", "rtl", STD),
        ],
    );
    analyze(&work, t.path(), "unused.vhd", "unused", vec![pkg("Unused")]);
    let c = closure(&libs, &UnitRef::new("WORK", UnitKind::Entity, "top")).unwrap();
    let paths: Vec<_> = c.iter().map(|f| f.path.clone()).collect();
    assert_eq!(paths, [base, ent, body]);
    assert_eq!(c[0].library, "util");
}

#[test]
fn errors() {
    let (t, libs, work, _) = setup();
    analyze(&work, t.path(), "a.vhd", "a", vec![pkg("Counter_Pkg")]);
    let e = |top: UnitRef| snap(&closure(&libs, &top).unwrap_err(), t.path());

    assert_eq!(
        e(UnitRef::new("nope", UnitKind::Entity, "x")),
        "error[L0030]: unknown library `nope`\n  |\n  = note: mapped libraries: work, util\n  = help: map it with `nope:PATH`\n"
    );
    assert_eq!(
        e(UnitRef::new("work", UnitKind::Package, "counter_pck")),
        "error[L0031]: unknown package `counter_pck` in library `work`\n  |\n  = help: did you mean package `Counter_Pkg`?\n"
    );
    assert_eq!(
        e(UnitRef::new("work", UnitKind::Package, "zzzzzzzz")),
        "error[L0031]: unknown package `zzzzzzzz` in library `work`\n  |\n  = help: library `work` has no package zzzzzzzz; analyze the file that defines it\n"
    );
    let missing = Library::new("work", t.path().join("none"));
    assert_eq!(
        snap(&missing.read().unwrap_err(), t.path()),
        "error[L0001]: library `work` not found\n  |\n  = help: rsim looked for `<tmp>/none/rsim-lib.toml`; analyze a file into this library first or fix the NAME:PATH mapping\n"
    );
    let mut l = Libraries::new();
    assert_eq!(
        snap(&l.add_spec("oops").unwrap_err(), t.path()),
        "error[L0020]: invalid library mapping `oops`\n  |\n  = help: expected NAME:PATH, e.g. `ieee:/opt/rsim/lib/ieee`\n"
    );
    assert_eq!(
        snap(
            &work
                .update_file("1993", &t.path().join("a.vhd"), "h", vec![])
                .unwrap_err(),
            t.path()
        ),
        "error[L0005]: library `work` holds VHDL-2008 units, cannot analyze `<tmp>/a.vhd` as VHDL-1993\n  |\n  = help: analyze with the library's standard, or use a separate library directory\n"
    );
}

#[test]
fn missing_dependency_names_the_user() {
    let (t, libs, work, _) = setup();
    analyze(
        &work,
        t.path(),
        "a.vhd",
        "a",
        vec![pkg("A").depends("work", UnitKind::Package, "gone")],
    );
    let s = snap(
        &closure(&libs, &UnitRef::new("work", UnitKind::Package, "a")).unwrap_err(),
        t.path(),
    );
    assert_eq!(
        s,
        "error[L0031]: unknown package `gone` in library `work`\n  |\n  = note: needed by package `work.A`\n  = help: library `work` has no package gone; analyze the file that defines it\n"
    );
}

#[test]
fn stale_hash() {
    let (t, libs, work, _) = setup();
    let a = analyze(&work, t.path(), "a.vhd", "v1", vec![pkg("A")]);
    std::fs::write(&a, "v2").unwrap();
    let s = snap(
        &closure(&libs, &UnitRef::new("work", UnitKind::Package, "a")).unwrap_err(),
        t.path(),
    );
    assert_eq!(
        s,
        "error[L0034]: `<tmp>/a.vhd` changed on disk since it was analyzed into library `work`\n  |\n  = help: re-analyze it: `rsim analyze --work work <tmp>/a.vhd`\n"
    );
    std::fs::remove_file(&a).unwrap();
    let s = snap(
        &closure(&libs, &UnitRef::new("work", UnitKind::Package, "a")).unwrap_err(),
        t.path(),
    );
    assert!(
        s.starts_with("error[L0033]: source file `<tmp>/a.vhd` of library `work` cannot be read"),
        "{s}"
    );
}

#[test]
fn cycle() {
    let (t, libs, work, _) = setup();
    analyze(
        &work,
        t.path(),
        "a.vhd",
        "a",
        vec![pkg("A").depends("work", UnitKind::Package, "b")],
    );
    analyze(
        &work,
        t.path(),
        "b.vhd",
        "b",
        vec![pkg("B").depends("work", UnitKind::Package, "c")],
    );
    analyze(
        &work,
        t.path(),
        "c.vhd",
        "c",
        vec![pkg("C").depends("work", UnitKind::Package, "a")],
    );
    let s = snap(
        &closure(&libs, &UnitRef::new("work", UnitKind::Package, "a")).unwrap_err(),
        t.path(),
    );
    assert_eq!(
        s,
        "error[L0032]: dependency cycle between design units\n  |\n  = note: package `work.A` in <tmp>/a.vhd needs package `work.B` in <tmp>/b.vhd\n  = note: package `work.B` in <tmp>/b.vhd needs package `work.C` in <tmp>/c.vhd\n  = note: package `work.C` in <tmp>/c.vhd needs package `work.A` in <tmp>/a.vhd\n  = help: break the cycle, for example by moving shared declarations into a separate package\n"
    );
}

#[test]
fn same_file_units_are_not_a_cycle() {
    let (t, libs, work, _) = setup();
    analyze(
        &work,
        t.path(),
        "e.vhd",
        "e",
        vec![
            UnitRecord::new(UnitKind::Entity, "E", STD),
            UnitRecord::architecture("E", "a", STD),
        ],
    );
    let c = closure(&libs, &UnitRef::new("work", UnitKind::Entity, "e")).unwrap();
    assert_eq!(c.len(), 1);
}

#[test]
fn readers_wait_for_writers() {
    let (t, _, work, _) = setup();
    analyze(&work, t.path(), "a.vhd", "a", vec![pkg("A")]);
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(work.dir.join("_RSIM_LIB"))
        .unwrap();
    lock.lock().unwrap();
    let w2 = work.clone();
    let h = std::thread::spawn(move || w2.read().unwrap());
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(!h.is_finished(), "reader must wait for the exclusive lock");
    lock.unlock().unwrap();
    h.join().unwrap();
}

#[cfg(unix)]
#[test]
fn read_only_library() {
    use std::os::unix::fs::PermissionsExt;
    let (t, libs, work, _) = setup();
    analyze(&work, t.path(), "a.vhd", "a", vec![pkg("A")]);
    let set = |p: &Path, mode| {
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap()
    };
    set(&work.dir.join("_RSIM_LIB"), 0o444);
    set(&work.dir.join("rsim-lib.toml"), 0o444);
    set(&work.dir, 0o555);
    let before = std::fs::read_dir(&work.dir).unwrap().count();

    // Reads work with a read-only lock file, and with no lock file at all.
    let c = closure(&libs, &UnitRef::new("work", UnitKind::Package, "a")).unwrap();
    assert_eq!(c.len(), 1);
    assert_eq!(
        std::fs::read_dir(&work.dir).unwrap().count(),
        before,
        "readers must not write"
    );
    let r = work.update_file(STD, &t.path().join("a.vhd"), "h", vec![]);
    let denied = r.is_err();
    if denied {
        let s = snap(&r.unwrap_err(), t.path());
        assert!(
            s.starts_with("error[L0010]: library `work` is not writable"),
            "{s}"
        );
    }
    set(&work.dir, 0o755);
    if denied {
        // Unreadable lock file: still readable without a lock (running as root skips this).
        set(&work.dir.join("_RSIM_LIB"), 0o000);
        assert!(work.read().is_ok());
        set(&work.dir.join("_RSIM_LIB"), 0o644);
        std::fs::remove_file(work.dir.join("_RSIM_LIB")).unwrap();
        assert!(work.read().is_ok());
    }
}

/// Child entry point for `concurrent_analyze`: re-run by the parent with RSIM_LIB_TEST_CHILD set.
#[test]
fn child_entry() {
    let Ok(spec) = std::env::var("RSIM_LIB_TEST_CHILD") else {
        return;
    };
    let (dir, id) = spec.rsplit_once(',').unwrap();
    let lib = Library::new("work", Path::new(dir).join("lib"));
    for i in 0..8 {
        let name = format!("p{id}_{i}");
        let text = format!("pkg {name}");
        let p = Path::new(dir).join(format!("{name}.vhd"));
        std::fs::write(&p, &text).unwrap();
        lib.update_file(STD, &p, &hash_content(text.as_bytes()), vec![pkg(&name)])
            .unwrap();
    }
}

#[test]
fn concurrent_analyze() {
    let t = tempfile::tempdir().unwrap();
    let exe = std::env::current_exe().unwrap();
    let kids: Vec<_> = (0..8)
        .map(|id| {
            Command::new(&exe)
                .args(["--exact", "child_entry", "--nocapture"])
                .env(
                    "RSIM_LIB_TEST_CHILD",
                    format!("{},{id}", t.path().display()),
                )
                .spawn()
                .unwrap()
        })
        .collect();
    for mut k in kids {
        assert!(k.wait().unwrap().success());
    }
    let m = Library::new("work", t.path().join("lib")).read().unwrap();
    assert_eq!(m.files.len(), 64, "lost updates");
    assert_eq!(m.units().count(), 64);
    assert!(
        !std::fs::read_dir(t.path().join("lib")).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp"))
    );
    // Every recorded hash matches its file.
    for f in &m.files {
        assert_eq!(f.hash, hash_content(&std::fs::read(&f.path).unwrap()));
    }
}
