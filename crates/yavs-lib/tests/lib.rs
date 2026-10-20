use std::path::{Path, PathBuf};
use std::process::Command;
use yavs_lib::{Libraries, Library, UnitKind, UnitRecord, UnitRef, closure, hash_content, render};

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

fn snap(d: &yavs_diag::Diagnostic, tmp: &Path) -> String {
    format!(
        "{}\n",
        render(d, false)
            .trim_end()
            .replace(tmp.to_str().unwrap(), "<tmp>")
            .replace('\\', "/")
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
        "error[L0001]: library `work` not found\n  |\n  = help: yavs looked for `<tmp>/none/yavs-lib.toml`; analyze a file into this library first or fix the NAME:PATH mapping\n"
    );
    let mut l = Libraries::new();
    assert_eq!(
        snap(&l.add_spec("oops").unwrap_err(), t.path()),
        "error[L0020]: invalid library mapping `oops`\n  |\n  = help: expected NAME:PATH, e.g. `ieee:/opt/yavs/lib/ieee`\n"
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
        "error[L0034]: `<tmp>/a.vhd` changed on disk since it was analyzed into library `work`\n  |\n  = help: re-analyze it: `yavs analyze --work work <tmp>/a.vhd`\n"
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
        .open(work.dir.join("_YAVS_LIB"))
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
    set(&work.dir.join("_YAVS_LIB"), 0o444);
    set(&work.dir.join("yavs-lib.toml"), 0o444);
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
        set(&work.dir.join("_YAVS_LIB"), 0o000);
        assert!(work.read().is_ok());
        set(&work.dir.join("_YAVS_LIB"), 0o644);
        std::fs::remove_file(work.dir.join("_YAVS_LIB")).unwrap();
        assert!(work.read().is_ok());
    }
}

/// Child entry point for `concurrent_analyze`: re-run by the parent with YAVS_LIB_TEST_CHILD set.
#[test]
fn child_entry() {
    let Ok(spec) = std::env::var("YAVS_LIB_TEST_CHILD") else {
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
                    "YAVS_LIB_TEST_CHILD",
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

#[test]
fn mixed_standards_per_file() {
    let (t, libs, work, _) = setup();
    let a = t.path().join("a.vhd");
    std::fs::write(&a, "a").unwrap();
    let h = hash_content(b"a");
    work.update_file("2008", &a, &h, vec![pkg("A")]).unwrap();
    let b = t.path().join("b.vhd");
    std::fs::write(&b, "b").unwrap();
    work.update_file(
        "2019",
        &b,
        &hash_content(b"b"),
        vec![pkg("B").depends("work", UnitKind::Package, "a")],
    )
    .unwrap();
    let c = closure(&libs, &UnitRef::new("work", UnitKind::Package, "b")).unwrap();
    assert_eq!(
        c.iter().map(|f| f.std.as_str()).collect::<Vec<_>>(),
        ["2008", "2019"]
    );
    // Re-analyzing with another standard replaces the entry.
    work.update_file("2019", &a, &h, vec![pkg("A")]).unwrap();
    assert_eq!(work.read().unwrap().file(&a).unwrap().std, "2019");
}

#[test]
fn old_name_manifest() {
    let (t, _, work, _) = setup();
    let old = "rsim-lib.toml"; // rename: keep
    std::fs::create_dir_all(&work.dir).unwrap();
    std::fs::write(work.dir.join(old), "format = 2\nname = \"work\"\n").unwrap();
    let want = format!(
        "error[L0004]: library manifest `<tmp>/work/{old}` was written before the rename to yavs\n  |\n  = help: delete the library directory and re-analyze its files\n"
    );
    assert!(work.has_manifest());
    assert_eq!(snap(&work.read().unwrap_err(), t.path()), want);
    let e = work
        .update_file("2008", &t.path().join("x.vhd"), "h", vec![])
        .unwrap_err();
    assert_eq!(snap(&e, t.path()), want);
    assert!(!work.dir.join(yavs_lib::MANIFEST_FILE).exists());
}

#[test]
fn old_format_manifest() {
    let (t, _, work, _) = setup();
    std::fs::create_dir_all(&work.dir).unwrap();
    std::fs::write(
        work.dir.join("yavs-lib.toml"),
        "format = 1\nname = \"work\"\nstandard = \"2008\"\n",
    )
    .unwrap();
    assert_eq!(
        snap(&work.read().unwrap_err(), t.path()),
        "error[L0004]: library manifest `<tmp>/work/yavs-lib.toml` has format version 1, this yavs reads version 2\n  |\n  = help: re-analyze the library's files to rewrite the manifest\n"
    );
    // Analyzing into it also fails early rather than overwriting.
    assert!(
        work.update_file("2008", &t.path().join("x.vhd"), "h", vec![])
            .is_err()
    );
}

/// Set the modification time of `p` to an hour ago.
fn age(p: &Path) {
    let f = std::fs::File::options().write(true).open(p).unwrap();
    f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
        .unwrap();
}

#[test]
fn unchanged_files_are_not_hashed_again() {
    let (t, libs, work, _) = setup();
    let a = t.path().join("a.vhd");
    std::fs::write(&a, "aaaa").unwrap();
    age(&a);
    let h = hash_content(b"aaaa");
    let stamp = yavs_lib::Stamp::of(&a);
    assert!(stamp.is_some(), "an hour old file is stampable");
    work.session()
        .unwrap()
        .record_stamped(STD, &a, &h, stamp, vec![pkg("A")])
        .unwrap();
    let top = UnitRef::new("work", UnitKind::Package, "a");
    assert!(work.read().unwrap().file(&a).unwrap().unchanged());
    assert!(closure(&libs, &top).is_ok());
    // New content of the same size and a new modification time: the stamp no longer matches, the
    // hash is checked and differs.
    let recorded = std::fs::metadata(&a).unwrap().modified().unwrap();
    std::fs::write(&a, "bbbb").unwrap();
    assert!(!work.read().unwrap().file(&a).unwrap().unchanged());
    let e = closure(&libs, &top).unwrap_err();
    assert!(render(&e, false).contains("L0034"), "{}", render(&e, false));
    // The same size and the recorded time (a restored time stamp) is trusted.
    let f = std::fs::File::options().write(true).open(&a).unwrap();
    f.set_modified(recorded).unwrap();
    assert!(work.read().unwrap().file(&a).unwrap().unchanged());
    assert!(closure(&libs, &top).is_ok());
}

#[test]
fn fresh_files_are_always_hashed() {
    let (t, libs, work, _) = setup();
    let a = t.path().join("a.vhd");
    std::fs::write(&a, "aaaa").unwrap();
    assert_eq!(
        yavs_lib::Stamp::of(&a),
        None,
        "a file just written is not stampable"
    );
    work.session()
        .unwrap()
        .record_stamped(STD, &a, &hash_content(b"aaaa"), None, vec![pkg("A")])
        .unwrap();
    let top = UnitRef::new("work", UnitKind::Package, "a");
    assert!(!work.read().unwrap().file(&a).unwrap().unchanged());
    assert!(closure(&libs, &top).is_ok());
    std::fs::write(&a, "bbbb").unwrap();
    assert!(closure(&libs, &top).is_err());
}
