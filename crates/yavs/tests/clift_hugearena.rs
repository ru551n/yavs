//! Code in the huge-page arena or without it, the results are the same: on a host that gives no
//! huge pages (process-wide THP off), with a limit on the address space (no reservation), with
//! the knob. Needs the `clift` feature and Linux for the two host conditions.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::process::Command;

// Large enough for a chunk: a run past 256 KiB of code asks for a huge page.
fn source() -> String {
    let mut s = String::from(
        "entity huge is end;\narchitecture t of huge is\n  signal c : integer := 0;\nbegin\n",
    );
    for p in 0..400 {
        s += &format!(
            "  p{p}: process (c)\n    variable v : integer := {p};\n  begin\n    for i in 0 to 3 loop\n      if c mod 2 = i mod 2 then v := v + c * {p}; else v := v - i; end if;\n    end loop;\n    if c = 3 and {p} = 399 then report integer'image(v); end if;\n  end process;\n"
        );
    }
    s + "  c <= c + 1 after 1 ns when c < 3;\nend;\n"
}

fn output(mut c: Command) -> String {
    let o = c.output().expect("run");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr)
}

#[test]
fn same_result_with_and_without_huge_pages() {
    let d = tempdir("clift_hugearena");
    std::fs::write(d.join("h.vhd"), source()).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let a = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(&d)
        .args(["analyze", "--work", &work, "h.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));

    let args = [
        "--color",
        "never",
        "run",
        "--no-cache",
        "--tier=clift",
        "--work",
        &work,
        "huge",
    ];
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let plain = || {
        let mut c = Command::new(yavs);
        c.current_dir(&d).args(args);
        c
    };
    let want = output(plain());
    assert!(want.contains("Note") || want.contains("note"), "{want}");

    let mut off = plain();
    off.env("YAVS_CLIFT_HUGE", "0");
    assert_eq!(output(off), want);

    #[cfg(target_os = "linux")]
    {
        // In the child, before exec: every huge page request of the process is refused
        // (PR_SET_THP_DISABLE), or the address space is limited (RLIMIT_AS: no reservation).
        for setup in [
            "import ctypes; ctypes.CDLL(None).prctl(41, 1, 0, 0, 0)",
            "import resource; resource.setrlimit(resource.RLIMIT_AS, (8 << 30, 8 << 30))",
        ] {
            let mut c = Command::new("python3");
            c.current_dir(&d)
                .arg("-Ic")
                .arg(format!(
                    "{setup}; import os, sys; os.execv(sys.argv[1], sys.argv[1:])"
                ))
                .arg(yavs)
                .args(args);
            assert_eq!(output(c), want, "{setup}");
        }
    }
}
