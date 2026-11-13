//! Processes sharing one cache directory, as the test runs of a parallel suite do: writers and
//! readers at once (no entry is lost or read torn), and a writer killed at any point (what it
//! leaves is a miss or a whole entry, never a wrong one, and the next put heals it). The test
//! binary runs itself as the child processes (`child`, only with `CHILD_DIR` set).

use std::path::Path;
use std::process::{Child, Command, Stdio};

use yavs_cache::{Key, KeyBuilder, Store};

const CHILD_DIR: &str = "CACHE_TEST_CHILD_DIR";
const CHILD_ROUNDS: &str = "CACHE_TEST_CHILD_ROUNDS";
const ENTRIES: usize = 60;

fn key(i: usize) -> Key {
    let mut k = KeyBuilder::new("t");
    k.text(&i.to_string());
    k.finish()
}

/// The one payload of key `i` (an entry's bytes depend on its key alone); some of them big, so a
/// kill can land in the middle of a write.
fn payload(i: usize) -> Vec<u8> {
    let n = if i.is_multiple_of(10) {
        3 << 20
    } else {
        1000 + i * 37
    };
    (0..n).map(|j| (i * 31 + j / 7) as u8).collect()
}

/// The stores of a library directory, each with its own way of writing (rename, in place).
fn stores(lib: &Path) -> [Store; 3] {
    [
        Store::of_library(lib),
        Store::objects_of_library(lib),
        Store::units_of_library(lib),
    ]
}

/// Every entry that reads back is whole and right; returns how many did.
fn check_all(lib: &Path) -> usize {
    let s = stores(lib);
    (0..ENTRIES)
        .filter(|&i| {
            s[i % 3].get("t", &key(i)).is_some_and(|e| {
                assert!(*e == payload(i)[..], "entry {i} reads back wrong");
                true
            })
        })
        .count()
}

/// The child's part: read every entry, store the missing ones, for some rounds (forever with
/// `CHILD_ROUNDS=0`, until killed).
#[test]
fn child() {
    let Some(dir) = std::env::var_os(CHILD_DIR) else {
        return;
    };
    let rounds: usize = std::env::var(CHILD_ROUNDS).map_or(3, |r| r.parse().unwrap());
    let s = stores(Path::new(&dir));
    let mut round = 0;
    while rounds == 0 || round < rounds {
        for i in 0..ENTRIES {
            match s[i % 3].get("t", &key(i)) {
                Some(e) => assert!(*e == payload(i)[..], "entry {i} reads back wrong"),
                None => s[i % 3].put("t", &key(i), &payload(i)),
            }
        }
        round += 1;
    }
}

fn spawn(dir: &Path, rounds: usize) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child", "--test-threads=1", "--quiet"])
        .env(CHILD_DIR, dir)
        .env(CHILD_ROUNDS, rounds.to_string())
        .stdout(Stdio::null())
        .spawn()
        .unwrap()
}

/// Six processes write and read the same entries at once: each finds every entry it reads whole,
/// and afterwards every entry is there.
#[test]
fn processes_share_one_store() {
    if std::env::var_os(CHILD_DIR).is_some() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let kids: Vec<Child> = (0..6).map(|_| spawn(d.path(), 3)).collect();
    for mut k in kids {
        assert!(k.wait().unwrap().success(), "a child read a wrong entry");
    }
    assert_eq!(check_all(d.path()), ENTRIES, "an entry was lost");
}

/// Writers killed in the middle of their work: whatever they left reads back whole or not at all,
/// and the next writer stores every entry again.
#[test]
fn a_killed_writer_leaves_no_wrong_entry() {
    if std::env::var_os(CHILD_DIR).is_some() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    for attempt in 0..4 {
        let mut k = spawn(d.path(), 0);
        // Kill it once it has written something (and, from the second attempt, while it rewrites
        // entries the last one cut short): the point does not matter, every one must be safe.
        let files = || {
            stores(d.path())
                .iter()
                .map(|s| std::fs::read_dir(s.dir()).map_or(0, |r| r.count()))
                .sum::<usize>()
        };
        let before = files();
        while files() <= before + attempt && k.try_wait().unwrap().is_none() {
            std::thread::yield_now();
        }
        let _ = k.kill();
        let _ = k.wait();
        check_all(d.path());
    }
    let mut k = spawn(d.path(), 1);
    assert!(k.wait().unwrap().success());
    assert_eq!(
        check_all(d.path()),
        ENTRIES,
        "a damaged entry was not healed"
    );
}
