//! The slots of a procedure that can wait (docs/decisions.md "Intervals and register promotion in
//! processes"): a variable that no wait leaves live becomes a register, one that is live across a
//! wait or a call that waits stays in its frame slot (the suspension invariant). Run results do
//! not show a register left live across a wait (the interpreter keeps registers), so this reads
//! the optimised code `YAVS_DUMPIR` prints for `tests/corpus/process/wait_proc_locals.vhd`.

mod common;
use common::tempdir as tmp;

use std::process::Command;

const SRC: &str = include_str!("../../../tests/corpus/process/wait_proc_locals.vhd");

/// The dumped code of procedure `name`: from its header to the next function's.
fn body<'a>(dump: &'a str, name: &str) -> &'a str {
    let head = format!("Procedure `{name}`");
    let at = dump
        .find(&head)
        .unwrap_or_else(|| panic!("no {head}\n{dump}"));
    let rest = &dump[at + head.len()..];
    let end = rest
        .lines()
        .scan(0, |n, l| {
            let start = *n;
            *n += l.len() + 1;
            Some((start, l))
        })
        .find(|(_, l)| l.starts_with('f') && l.contains(" Procedure `") || l.starts_with("ops of"))
        .map_or(rest.len(), |(i, _)| i);
    &rest[..end]
}

#[test]
fn slots_live_across_a_wait_stay_in_the_frame() {
    let d = tmp("wait_proc_ir");
    std::fs::write(d.join("t.vhd"), SRC).expect("write");
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let work = format!("work:{}", d.join("lib").display());
    let a = Command::new(yavs)
        .current_dir(&d)
        .args(["analyze", "--std=2008", "--work", &work, "t.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let o = Command::new(yavs)
        .current_dir(&d)
        .env("YAVS_DUMPIR", "1")
        .args([
            "--color",
            "never",
            "run",
            "--no-cache",
            "--std=2008",
            "--work",
            &work,
        ])
        .args(["--tier=interp", "wait_proc_locals"])
        .output()
        .expect("run");
    let dump = String::from_utf8_lossy(&o.stderr);
    assert!(
        String::from_utf8_lossy(&o.stdout).contains("outer r=90011")
            || dump.contains("outer r=90011"),
        "wrong result\n{dump}"
    );
    // `across`: `a` (slot 16) is read after the wait, `b` (slot 24) is assigned again first.
    let across = body(&dump, "across");
    let (before, after) = across.split_once("Suspend").expect("a wait in `across`");
    assert!(
        after.contains("space: Frame, off: 16 }"),
        "`a` not reloaded after the wait\n{across}"
    );
    assert!(
        !across.contains("space: Frame, off: 24 }"),
        "`b` not promoted\n{across}"
    );
    assert!(
        before.contains("space: Frame, off: 16 }"),
        "`a` not stored before the wait\n{across}"
    );
    // `cycles`: `s` and `k` are live around the wait in the loop.
    let cycles = body(&dump, "cycles");
    let (_, after) = cycles.split_once("Suspend").expect("a wait in `cycles`");
    for off in [16, 24] {
        assert!(
            after.contains(&format!("space: Frame, off: {off} }}")),
            "slot {off} not in the frame after the wait\n{cycles}"
        );
    }
    // `outer`: `c` (slot 16) is live across the call of `across`, which waits.
    let outer = body(&dump, "outer");
    let (_, after) = outer.split_once("Call {").expect("the call in `outer`");
    assert!(
        after.contains("space: Frame, off: 16 }"),
        "`c` not in the frame after the call\n{outer}"
    );
}
