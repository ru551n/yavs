//! Functions compiled by LLVM and by Cranelift call each other on native frames (`YAVS_MIXED=1`:
//! LLVM compiles the functions of even id, Cranelift the others, every call goes through the
//! native-frame table): the output is the interpreter's, also when a callee fails deep in a chain
//! of calls (the frames are materialised across the two generators), in a nested function and in a
//! protected object.
#![cfg(all(feature = "llvm", feature = "clift"))]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

/// A chain of eight functions (neighbours have ids of different parity), a nested function reading
/// its parent's variable, a protected counter, and a chain that fails at depth 5 when `fail` is set.
const SRC: &str = r#"
entity mixed is generic (fail : boolean := false); end;
architecture a of mixed is
  type counter_t is protected
    procedure bump (by : integer);
    impure function get return integer;
  end protected;
  type counter_t is protected body
    variable n : integer := 0;
    procedure bump (by : integer) is begin n := n + by; end;
    impure function get return integer is begin return n; end;
  end protected body;
  shared variable c : counter_t;

  function f8 (x : integer) return integer is
    variable r : integer range 0 to 100000;
  begin
    r := x * 2;
    return r;
  end;
  function f7 (x : integer) return integer is begin return f8(x + 1) + 1; end;
  function f6 (x : integer) return integer is begin return f7(x + 1) + 1; end;
  function f5 (x : integer) return integer is begin return f6(x + 1) + 1; end;
  function f4 (x : integer) return integer is begin return f5(x + 1) + 1; end;
  function f3 (x : integer) return integer is begin return f4(x + 1) + 1; end;
  function f2 (x : integer) return integer is begin return f3(x + 1) + 1; end;
  function f1 (x : integer) return integer is begin return f2(x + 1) + 1; end;

  impure function nested (k : integer) return integer is
    variable base : integer := k * 3;
    impure function inner (j : integer) return integer is begin return base + j; end;
    impure function inner2 (j : integer) return integer is begin return inner(j) + inner(j + 1); end;
  begin
    return inner2(k) + inner2(base);
  end;

  procedure tick (by : integer) is begin c.bump(by); end;
  procedure tick2 (by : integer) is begin tick(by); tick(by + 1); end;
begin
  process
    variable acc : integer := 0;
  begin
    for i in 1 to 3000 loop
      acc := acc + f1(i mod 100);
      acc := acc + nested(i mod 50);
      tick2(i mod 7);
    end loop;
    report "acc=" & integer'image(acc) & " c=" & integer'image(c.get);
    if fail then
      acc := f1(60000);
      report "not reached " & integer'image(acc);
    end if;
    wait;
  end process;
end;
"#;

fn run(dir: &Path, flags: &[&str], fail: bool, mixed: bool) -> Output {
    let mut args = vec!["run", "--work=work:work"];
    args.extend_from_slice(flags);
    if fail {
        args.push("-gfail=true");
    }
    args.push("mixed");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_yavs"));
    cmd.args(&args).current_dir(dir);
    if mixed {
        cmd.env("YAVS_MIXED", "1").env("YAVS_CLIFT_NTAB", "1");
    }
    cmd.output().expect("run yavs")
}

#[test]
fn llvm_and_cranelift_functions_call_each_other() {
    let dir = tempdir("compiled_mixed");
    std::fs::write(dir.join("mixed.vhd"), SRC).expect("write");
    let a = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["analyze", "--std=2008", "--work=work:work", "mixed.vhd"])
        .current_dir(&dir)
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{a:?}");
    for fail in [false, true] {
        let want = run(&dir, &["--tier=interp"], fail, false);
        let text = String::from_utf8_lossy(&want.stdout).into_owned()
            + &String::from_utf8_lossy(&want.stderr);
        assert!(text.contains("acc="), "{text}");
        assert_eq!(!want.status.success(), fail, "{text}");
        for flags in [
            vec!["--tier=auto3", "--tier-heat=0,0,0"],
            vec!["--tier=auto3", "--tier-heat=0,0,0", "--threads=4"],
            vec!["--tier=auto3", "--tier-heat=1,0,0"],
        ] {
            let got = run(&dir, &flags, fail, true);
            assert_eq!(got.status.code(), want.status.code(), "{flags:?} {got:?}");
            assert_eq!(got.stdout, want.stdout, "{flags:?}");
            assert_eq!(got.stderr, want.stderr, "{flags:?}");
        }
        // Cranelift first, LLVM again for what stays hot (the variants of the second replace those of
        // the first while the first is asked for again: once a duplicate LLVM module and a crash).
        for _ in 0..5 {
            let got = run(
                &dir,
                &["--tier=auto3", "--tier-heat=0,0,0", "--threads=4"],
                fail,
                false,
            );
            assert_eq!(got.status.code(), want.status.code(), "{got:?}");
            assert_eq!(got.stdout, want.stdout);
            assert_eq!(got.stderr, want.stderr);
        }
    }
}
