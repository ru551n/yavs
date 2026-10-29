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
    assert_eq!(m.len(), 2);
    let a_file = m.file(&a).unwrap();
    assert_eq!(m.records(&a_file).unwrap()[0].name, "A");
    // a.vhd no longer defines B, and moves to the end of the compile order.
    analyze(&work, t.path(), "a.vhd", "a2", vec![pkg("A")]);
    let m = work.read().unwrap();
    assert!(m.find(UnitKind::Package, "b").is_none());
    assert_eq!(m.files().last().unwrap().path, a);
    assert_eq!(m.files().last().unwrap().hash, hash_content(b"a2"));
    assert_eq!(m.len(), 2);
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
    // The body follows its package directly (it needs nothing else), before the entity that
    // uses the package.
    assert_eq!(paths, [base, body, ent]);
    assert_eq!(c[0].library, "util");
}

/// A package body in a file of its own follows its package's file when everything the body needs
/// is placed already, so that every design that shares the package's files shares the body's
/// place too; a body that needs a unit placed later is pulled in after it, as before.
#[test]
fn package_body_follows_its_package_when_it_can() {
    let (t, libs, work, util) = setup();
    let base = analyze(&util, t.path(), "base.vhd", "base", vec![pkg("Base")]);
    let helper = analyze(&work, t.path(), "helper.vhd", "helper", vec![pkg("Helper")]);
    let body = analyze(
        &util,
        t.path(),
        "base_body.vhd",
        "body",
        vec![UnitRecord::package_body("Base", STD).depends("work", UnitKind::Package, "helper")],
    );
    let other = analyze(&util, t.path(), "other.vhd", "other", vec![pkg("Other")]);
    let mid = analyze(
        &util,
        t.path(),
        "other_body.vhd",
        "obody",
        vec![UnitRecord::package_body("Other", STD)],
    );
    let top = analyze(
        &work,
        t.path(),
        "top.vhd",
        "top",
        vec![
            UnitRecord::new(UnitKind::Entity, "Top", STD)
                .depends("util", UnitKind::Package, "BASE")
                .depends("util", UnitKind::Package, "OTHER")
                .depends("work", UnitKind::Package, "HELPER"),
            UnitRecord::architecture("Top", "rtl", STD),
        ],
    );
    let c = closure(&libs, &UnitRef::new("work", UnitKind::Entity, "top")).unwrap();
    let paths: Vec<_> = c.iter().map(|f| f.path.clone()).collect();
    // `base`'s body needs `helper`, which the entity names after `base` and `other`: the body of
    // `other` follows `other`, the body of `base` waits for `helper`.
    let at = |p: &PathBuf| paths.iter().position(|q| q == p).unwrap();
    assert_eq!(at(&mid), at(&other) + 1, "{paths:?}");
    assert!(
        at(&body) > at(&helper) && at(&body) > at(&base),
        "{paths:?}"
    );
    assert!(at(&top) > at(&base) && at(&top) > at(&other), "{paths:?}");
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
        "error[L0031]: unknown package `counter_pck` in library `work`\n  |\n  = help: a package with a similar name exists: `Counter_Pkg`\n"
    );
    assert_eq!(
        e(UnitRef::new("work", UnitKind::Package, "zzzzzzzz")),
        "error[L0031]: unknown package `zzzzzzzz` in library `work`\n  |\n  = help: library `work` has no package `zzzzzzzz`; analyze the file that defines it\n"
    );
    // A directory without an index is an empty library: the unit is not found in it, and the
    // help names the directory (cli-contract section 3).
    let mut libs = Libraries::new();
    libs.add("work", t.path().join("none"));
    assert_eq!(
        snap(
            &closure(&libs, &UnitRef::new("work", UnitKind::Entity, "top")).unwrap_err(),
            t.path()
        ),
        "error[L0031]: unknown entity `top` in library `work`\n  |\n  = help: library `work` holds no design units (its directory `<tmp>/none` does not exist); analyze the files into it first or fix its `NAME:PATH` mapping\n"
    );
    std::fs::create_dir(t.path().join("none")).unwrap();
    assert!(
        snap(
            &closure(&libs, &UnitRef::new("work", UnitKind::Entity, "top")).unwrap_err(),
            t.path()
        )
        .contains("(its directory `<tmp>/none` has no analyzed files)")
    );
    std::fs::remove_dir(t.path().join("none")).unwrap();
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
        "error[L0031]: unknown package `gone` in library `work`\n  |\n  = note: needed by package `work.A`\n  = help: library `work` has no package `gone`; analyze the file that defines it\n"
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
        "error[L0034]: `<tmp>/a.vhd` changed on disk since it was analyzed into library `work`\n  |\n  = help: re-analyze it: `yavs analyze --std=2008 --work=work:<tmp>/work <tmp>/a.vhd`\n"
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
    assert_eq!(m.len(), 64, "lost updates");
    assert_eq!(m.units().count(), 64);
    assert!(
        !std::fs::read_dir(t.path().join("lib")).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp"))
    );
    // Every recorded hash matches its file.
    for f in m.files() {
        assert_eq!(f.hash, hash_content(&std::fs::read(f.path).unwrap()));
        assert_eq!(m.records(&f).unwrap().len(), 1);
    }
    // One record per file and no leftovers of replaced ones.
    assert_eq!(
        std::fs::read_dir(t.path().join("lib").join(yavs_lib::RECORD_DIR))
            .unwrap()
            .count(),
        64
    );
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
    // A library of format 2: the whole manifest in one file, no records.
    let old = "format = 2\nname = \"work\"\n\n[[file]]\npath = \"/x/a.vhd\"\nhash = \"sha256:ab\"\nstd = \"2008\"\n\n[[file.unit]]\nkind = \"package\"\nname = \"A\"\nkey = \"a\"\nstd = \"2008\"\ndepends_on = []\n";
    std::fs::write(work.dir.join("yavs-lib.toml"), old).unwrap();
    assert_eq!(
        snap(&work.read().unwrap_err(), t.path()),
        "error[L0004]: library manifest `<tmp>/work/yavs-lib.toml` has format version 2, this yavs reads version 3\n  |\n  = help: delete the library directory `<tmp>/work` and analyze its files again (a library of another format version is not converted)\n"
    );
    // Closures fail the same way, and analyzing into it fails early rather than overwriting.
    let mut libs = Libraries::new();
    libs.add("work", &work.dir);
    let e = closure(&libs, &UnitRef::new("work", UnitKind::Package, "a")).unwrap_err();
    assert!(render(&e, false).contains("L0004"));
    assert!(
        work.update_file("2008", &t.path().join("x.vhd"), "h", vec![])
            .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(work.dir.join("yavs-lib.toml")).unwrap(),
        old
    );
    assert!(!work.dir.join(yavs_lib::RECORD_DIR).exists());
    // Older still.
    std::fs::write(
        work.dir.join("yavs-lib.toml"),
        "format = 1\nname = \"work\"\nstandard = \"2008\"\n",
    )
    .unwrap();
    assert!(render(&work.read().unwrap_err(), false).contains("format version 1"));
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

#[test]
fn session_records_every_file_as_it_goes() {
    let (t, _, work, _) = setup();
    let session = work.session().unwrap();
    for (i, name) in ["a", "b", "c"].into_iter().enumerate() {
        let p = t.path().join(format!("{name}.vhd"));
        std::fs::write(&p, name).unwrap();
        session
            .record(STD, &p, &hash_content(name.as_bytes()), vec![pkg(name)])
            .unwrap();
        // Each record is published at once, before the session ends.
        assert_eq!(work.read().unwrap().len(), i + 1);
    }
    // Re-recording a file moves it to the end, from the manifest the session kept.
    let a = t.path().join("a.vhd");
    session
        .record(STD, &a, &hash_content(b"a2"), vec![pkg("a")])
        .unwrap();
    drop(session);
    let m = work.read().unwrap();
    assert_eq!(m.len(), 3);
    assert_eq!(m.files().last().unwrap().hash, hash_content(b"a2"));
}

/// A deferred record writes the file's record at once and the index at the flush only (#747): a
/// batch writes the index once, a file recorded twice and then as it first was keeps its record,
/// and the record a file replaced goes with the flush.
#[test]
fn deferred_records_are_published_by_the_flush() {
    let (t, _, work, _) = setup();
    let session = work.session().unwrap();
    let rec = |name: &str, text: &str, units: Vec<UnitRecord>| {
        let p = t.path().join(name);
        std::fs::write(&p, text).unwrap();
        session
            .record_deferred(STD, &p, &hash_content(text.as_bytes()), None, units)
            .unwrap();
        p
    };
    session.flush().unwrap(); // nothing recorded: no index
    assert!(work.read().is_err());
    let a = rec("a.vhd", "a1", vec![pkg("A")]);
    rec("b.vhd", "b1", vec![pkg("B")]);
    assert!(work.read().is_err(), "the index is written by the flush");
    session.flush().unwrap();
    assert_eq!(work.read().unwrap().len(), 2);
    // a.vhd: other units, then the first ones again; the record of the first is live at the flush.
    rec("a.vhd", "a2", vec![pkg("A2")]);
    rec("a.vhd", "a3", vec![pkg("A")]);
    rec("c.vhd", "c1", vec![pkg("C")]);
    assert_eq!(work.read().unwrap().len(), 2);
    session.flush().unwrap();
    let m = work.read().unwrap();
    assert_eq!(m.len(), 3);
    let f = m.file(&a).unwrap();
    assert_eq!(m.records(&f).unwrap()[0].name, "A");
    let records = std::fs::read_dir(work.dir.join("yavs-lib.d"))
        .unwrap()
        .count();
    assert_eq!(records, 3, "one record per file, none left over");
    drop(session);
}

/// A read under the session's own lock is parsed once and shared with the session, and it never
/// shows the manifest from before a record (the reader that parsed it and the writer are one
/// process).
#[test]
fn read_in_a_session_sees_every_record() {
    let (t, _, work, _) = setup();
    let session = work.session().unwrap();
    for i in 0..4 {
        let (name, text) = (format!("g{i}"), format!("text {i}"));
        let p = t.path().join(format!("{name}.vhd"));
        std::fs::write(&p, &text).unwrap();
        // Read first (the analyzer's lookups), then record: the record starts from that parse.
        let before = work.read().map(|m| m.len()).unwrap_or(0);
        assert_eq!(before, i);
        session
            .record(STD, &p, &hash_content(text.as_bytes()), vec![pkg(&name)])
            .unwrap();
        let again = work.read().unwrap();
        assert_eq!(again.len(), i + 1);
        assert_eq!(again.files().last().unwrap().path, p);
        let paths =
            |m: &yavs_lib::Manifest| m.files().map(|f| f.path.to_path_buf()).collect::<Vec<_>>();
        assert_eq!(paths(&work.read().unwrap()), paths(&again));
    }
    // The same directory under another name (another `Library` value) sees every record too.
    let alias = Library::new("alias", &work.dir);
    assert_eq!(alias.read().unwrap().len(), 4);
    let (name, text) = ("g4", "text 4");
    let p = t.path().join("g4.vhd");
    std::fs::write(&p, text).unwrap();
    session
        .record(STD, &p, &hash_content(text.as_bytes()), vec![pkg(name)])
        .unwrap();
    assert_eq!(alias.read().unwrap().len(), 5);
    drop(session);
    // Another process-like reader after the session sees the file as written.
    assert_eq!(Library::new("work", &work.dir).read().unwrap().len(), 5);
}

/// A reader polling the library while a session records files sees a whole index every time (the
/// old one or the new one, never a torn one), the count never goes back (#561), and every record
/// the index names is there (#599).
#[test]
fn manifest_is_never_torn() {
    let (t, _, work, _) = setup();
    let session = work.session().unwrap();
    let first = t.path().join("f0.vhd");
    std::fs::write(&first, "f0").unwrap();
    session
        .record(STD, &first, &hash_content(b"f0"), vec![pkg("f0")])
        .unwrap();
    let stop = std::sync::atomic::AtomicBool::new(false);
    let seen = std::thread::scope(|s| {
        let reader = s.spawn(|| {
            // A library value of its own, so that nothing is shared with the session's reads.
            let lib = Library::new("work", &work.dir);
            let (mut last, mut reads) = (1, 0);
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let m = lib.read().expect("manifest never torn");
                assert!(
                    m.len() >= last,
                    "manifest went back from {last} to {} files",
                    m.len()
                );
                last = m.len();
                for f in m.files() {
                    m.records(&f).expect("a record the index names is there");
                }
                reads += 1;
            }
            // The writer is done: what the last read says is the index on disk (#635).
            let m = lib.read().expect("manifest never torn");
            assert_eq!(m.len(), 300, "a read cached the index from before a write");
            reads
        });
        for i in 1..300 {
            let (name, text) = (format!("f{i}"), format!("text {i}"));
            let p = t.path().join(format!("{name}.vhd"));
            std::fs::write(&p, &text).unwrap();
            session
                .record(STD, &p, &hash_content(text.as_bytes()), vec![pkg(&name)])
                .unwrap();
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        reader.join().unwrap()
    });
    assert!(seen > 0);
    drop(session);
    assert_eq!(work.read().unwrap().len(), 300);
}

/// Recording what the library already says does not rewrite the index or the record (#561, #599).
#[cfg(unix)]
#[test]
fn unchanged_record_skips_the_rewrite() {
    use std::os::unix::fs::MetadataExt;
    let (t, _, work, _) = setup();
    let p = t.path().join("a.vhd");
    std::fs::write(&p, "a").unwrap();
    let h = hash_content(b"a");
    let session = work.session().unwrap();
    session.record(STD, &p, &h, vec![pkg("a")]).unwrap();
    let inos = || {
        let index = std::fs::metadata(work.dir.join(yavs_lib::MANIFEST_FILE)).unwrap();
        let records: Vec<_> = record_files(&work)
            .iter()
            .map(|r| std::fs::metadata(r).unwrap().ino())
            .collect();
        (index.ino(), records)
    };
    let before = inos();
    assert_eq!(before.1.len(), 1);
    session.record(STD, &p, &h, vec![pkg("a")]).unwrap();
    assert_eq!(inos(), before, "an identical library was written again");
    // A changed hash replaces the index and keeps the record (its text is the same).
    session
        .record(STD, &p, &hash_content(b"b"), vec![pkg("a")])
        .unwrap();
    let after = inos();
    assert_ne!(after.0, before.0);
    assert_eq!(after.1, before.1);
    // Other units make another record and the earlier one is deleted.
    session
        .record(STD, &p, &hash_content(b"b"), vec![pkg("a"), pkg("a2")])
        .unwrap();
    let after = inos();
    assert_eq!(after.1.len(), 1);
    assert_ne!(after.1, before.1);
}

fn record_files(lib: &Library) -> Vec<PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(lib.dir.join(yavs_lib::RECORD_DIR))
        .map(|d| d.map(|e| e.unwrap().path()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// A file that lost units to a later file is `partial` (#577): the index lists the rest, a file
/// that lost all of them is gone with its record, and a dependency on a unit it kept still resolves.
#[test]
fn partial_files() {
    let (t, libs, work, _) = setup();
    let a = analyze(&work, t.path(), "a.vhd", "a", vec![pkg("A"), pkg("B")]);
    let top = analyze(
        &work,
        t.path(),
        "top.vhd",
        "top",
        vec![pkg("Top").depends("work", UnitKind::Package, "a")],
    );
    assert_eq!(record_files(&work).len(), 2);
    let b = analyze(&work, t.path(), "b.vhd", "b", vec![pkg("B")]);
    let m = work.read().unwrap();
    let fa = m.file(&a).unwrap();
    assert!(fa.partial);
    assert_eq!(
        fa.units().map(|u| u.key.to_string()).collect::<Vec<_>>(),
        ["a"]
    );
    assert!(!m.file(&b).unwrap().partial && !m.file(&top).unwrap().partial);
    assert_eq!(m.find(UnitKind::Package, "b").unwrap().file, b);
    let c = closure(&libs, &UnitRef::new("work", UnitKind::Package, "top")).unwrap();
    assert_eq!(
        c.iter().map(|f| f.path.clone()).collect::<Vec<_>>(),
        [a.clone(), top]
    );
    // The partial file analyzed again is whole.
    work.update_file(STD, &a, &hash_content(b"a"), vec![pkg("A"), pkg("B")])
        .unwrap();
    let m = work.read().unwrap();
    assert!(!m.file(&a).unwrap().partial);
    assert!(m.file(&b).is_none(), "b lost its only unit");
    assert_eq!(record_files(&work).len(), 2, "the record of b is deleted");
    assert!(m.file(&a).unwrap().units().count() == 2);
}

/// The default architecture of an entity is the last one analyzed (LRM 7.3.3), and a file
/// analyzed again moves to the end of the analysis order.
#[test]
fn default_architecture_is_the_last_analyzed() {
    let (t, libs, work, _) = setup();
    let ent = |arch: &str| {
        vec![
            UnitRecord::new(UnitKind::Entity, "E", STD),
            UnitRecord::architecture("E", arch, STD),
        ]
    };
    let f1 = analyze(&work, t.path(), "f1.vhd", "f1", ent("one"));
    let f2 = analyze(
        &work,
        t.path(),
        "f2.vhd",
        "f2",
        vec![UnitRecord::architecture("E", "two", STD)],
    );
    let top = UnitRef::new("work", UnitKind::Entity, "e");
    let last = |work: &Library| {
        work.read()
            .unwrap()
            .units()
            .filter(|u| u.kind == UnitKind::Architecture && u.primary == Some("e"))
            .last()
            .map(|u| u.key.to_string())
    };
    assert_eq!(last(&work).as_deref(), Some("e/two"));
    let paths = |libs: &Libraries| -> Vec<PathBuf> {
        closure(libs, &top)
            .unwrap()
            .into_iter()
            .map(|f| f.path)
            .collect()
    };
    assert_eq!(paths(&libs), [f1.clone(), f2]);
    // `f1` analyzed again: its architecture is the newest.
    work.update_file(STD, &f1, &hash_content(b"f1"), ent("one"))
        .unwrap();
    assert_eq!(last(&work).as_deref(), Some("e/one"));
    assert_eq!(paths(&libs), [f1]);
}

/// Names that need an escape in the text survive.
#[test]
fn odd_strings_round_trip() {
    let (t, libs, work, _) = setup();
    let mut ids = Vec::new();
    let names: &[&str] = if cfg!(unix) {
        &[
            "q\"uote.vhd",
            "back\\slash.vhd",
            "\u{e9}t\u{e9}.vhd",
            "tab\there.vhd",
            "nl\nx.vhd",
            "a #1 [b] = c.vhd",
        ]
    } else {
        &["\u{e9}t\u{e9}.vhd", "a #1 [b] = c.vhd"]
    };
    for (i, name) in names.iter().copied().enumerate() {
        let unit = format!("\\ext {i}\\");
        let u = UnitRecord::new(UnitKind::Package, &unit, STD).depends(
            "work",
            UnitKind::Package,
            "\\d\"q\\",
        );
        ids.push((analyze_odd(&work, t.path(), name, vec![u]), unit));
    }
    let m = work.read().unwrap();
    for (p, unit) in &ids {
        let f = m.file(p).unwrap();
        assert_eq!(f.path, p);
        let r = m.records(&f).unwrap();
        assert_eq!(r[0].name, *unit);
        assert_eq!(r[0].depends_on[0].key, "\\d\"q\\");
        assert_eq!(
            m.find(UnitKind::Package, &unit.to_lowercase())
                .map(|u| u.file),
            Some(p.as_path())
        );
    }
    // A closure over them (the dependency is missing, which names the escaped unit).
    let e = closure(&libs, &UnitRef::new("work", UnitKind::Package, &ids[0].1)).unwrap_err();
    assert!(render(&e, false).contains("L0031"));
}

fn analyze_odd(lib: &Library, dir: &Path, name: &str, units: Vec<UnitRecord>) -> PathBuf {
    let p = dir.join(name);
    // File names with a newline or tab are not valid everywhere; the index is the subject here.
    lib.update_file(STD, &p, "sha256:00", units).unwrap();
    p
}

/// Damage to a record is an error that names it, never a different library.
#[test]
fn damaged_and_missing_records() {
    let (t, libs, work, _) = setup();
    let a = analyze(
        &work,
        t.path(),
        "a.vhd",
        "a",
        vec![pkg("A").depends("work", UnitKind::Package, "b")],
    );
    analyze(&work, t.path(), "b.vhd", "b", vec![pkg("B")]);
    let top = UnitRef::new("work", UnitKind::Package, "a");
    assert!(closure(&libs, &top).is_ok());
    let m = work.read().unwrap();
    let rec = work.dir.join(yavs_lib::RECORD_DIR).join(
        record_files(&work)
            .iter()
            .map(|p| p.file_name().unwrap().to_owned())
            .next()
            .unwrap(),
    );
    let text = std::fs::read_to_string(&rec).unwrap();
    // Changed text, same length: the digest differs.
    std::fs::write(&rec, text.replacen("depends_on", "dependz_on", 1)).unwrap();
    let broken = |f: &str| {
        let m = Library::new("work", &work.dir).read().unwrap();
        let fr = m.file(&t.path().join(f)).unwrap();
        m.records(&fr)
    };
    let errors: Vec<_> = ["a.vhd", "b.vhd"]
        .iter()
        .filter_map(|f| broken(f).err())
        .collect();
    assert_eq!(errors.len(), 1);
    let s = snap(&errors[0], t.path());
    assert!(
        s.starts_with("error[L0003]: library record `<tmp>/work/yavs-lib.d/"),
        "{s}"
    );
    assert!(
        s.contains("is damaged (its digest differs)")
            && s.contains("delete the library directory `<tmp>/work`"),
        "{s}"
    );
    std::fs::remove_file(&rec).unwrap();
    let e = closure(&libs, &top).unwrap_err();
    let s = snap(&e, t.path());
    assert!(
        s.contains("error[L0003]") && s.contains("is missing"),
        "{s}"
    );
    // The index alone is still readable, and so are the units it knows.
    assert!(m.find(UnitKind::Package, "a").is_some());
    assert_eq!(m.file(&a).unwrap().units().count(), 1);
}

/// A write interrupted after a record and before the index leaves the old library: the index
/// names the records it knew, and the record nobody names is ignored.
#[test]
fn interrupted_write_leaves_the_old_library() {
    let (t, libs, work, _) = setup();
    let a = analyze(&work, t.path(), "a.vhd", "a1", vec![pkg("A")]);
    let copy = tempfile::tempdir().unwrap();
    let cp = |from: &Path, to: &Path| {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            if e.path().is_file() {
                std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
            }
        }
    };
    cp(&work.dir, &copy.path().join("lib"));
    cp(
        &work.dir.join(yavs_lib::RECORD_DIR),
        &copy.path().join("lib").join(yavs_lib::RECORD_DIR),
    );
    // a.vhd is analyzed again with another set of units.
    std::fs::write(&a, "a2").unwrap();
    work.update_file(STD, &a, &hash_content(b"a2"), vec![pkg("A"), pkg("A2")])
        .unwrap();
    // The crash state: the old index, the old record, and the new record.
    for r in record_files(&work) {
        let to = copy
            .path()
            .join("lib")
            .join(yavs_lib::RECORD_DIR)
            .join(r.file_name().unwrap());
        if !to.exists() {
            std::fs::copy(&r, &to).unwrap();
        }
    }
    let mut crashed = Libraries::new();
    crashed.add("work", copy.path().join("lib"));
    let c = closure(&crashed, &UnitRef::new("work", UnitKind::Package, "a")).unwrap_err();
    // (the file on disk has the new text, which the old index did not hash)
    assert!(render(&c, false).contains("L0034"), "{}", render(&c, false));
    let m = Library::new("work", copy.path().join("lib"))
        .read()
        .unwrap();
    assert!(m.find(UnitKind::Package, "a2").is_none());
    assert_eq!(m.records(&m.file(&a).unwrap()).unwrap().len(), 1);
    // The next analyze in the crashed library repairs it and removes what nothing names.
    let lib = Library::new("work", copy.path().join("lib"));
    lib.update_file(STD, &a, &hash_content(b"a2"), vec![pkg("A"), pkg("A2")])
        .unwrap();
    assert_eq!(record_files(&lib).len(), 1);
    assert!(closure(&crashed, &UnitRef::new("work", UnitKind::Package, "a2")).is_ok());
    drop(libs);
}

/// Child entry point for `readers_and_writers_in_two_processes`.
#[test]
fn rewriter_entry() {
    let Ok(dir) = std::env::var("YAVS_LIB_TEST_REWRITER") else {
        return;
    };
    let lib = Library::new("work", Path::new(&dir).join("lib"));
    let top = Path::new(&dir).join("top.vhd");
    for i in 0..150 {
        let extra = format!("extra{i}");
        lib.update_file(
            STD,
            &top,
            &hash_content(b"top"),
            vec![
                pkg("Top").depends("work", UnitKind::Package, "base"),
                pkg(&extra),
            ],
        )
        .unwrap();
    }
}

/// A closure never sees a record replaced under it: another process analyzes the same file again
/// and again while this one resolves it (the shared lock is held for the whole closure).
#[test]
fn readers_and_writers_in_two_processes() {
    let t = tempfile::tempdir().unwrap();
    let lib = Library::new("work", t.path().join("lib"));
    let mut libs = Libraries::new();
    libs.add("work", &lib.dir);
    analyze(&lib, t.path(), "base.vhd", "base", vec![pkg("Base")]);
    std::fs::write(t.path().join("top.vhd"), "top").unwrap();
    lib.update_file(
        STD,
        &t.path().join("top.vhd"),
        &hash_content(b"top"),
        vec![pkg("Top").depends("work", UnitKind::Package, "base")],
    )
    .unwrap();
    let mut kid = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "rewriter_entry", "--nocapture"])
        .env("YAVS_LIB_TEST_REWRITER", t.path())
        .spawn()
        .unwrap();
    let top = UnitRef::new("work", UnitKind::Package, "top");
    let mut n = 0;
    while kid.try_wait().unwrap().is_none() {
        let c = closure(&libs, &top).unwrap_or_else(|e| panic!("{}", render(&e, false)));
        assert_eq!(c.len(), 2);
        n += 1;
    }
    assert!(n > 0);
    assert_eq!(record_files(&lib).len(), 2);
}
