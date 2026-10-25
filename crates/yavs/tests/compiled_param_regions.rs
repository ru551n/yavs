//! Address parameters of compiled native frames (`--tier=llvm`, `--tier=auto` compiling everything
//! at once; a build with the `llvm` feature): the region a by-reference or array parameter points
//! into is resolved once at the function's entry (`yavs_rt_region`) and held for the activation.
//! The arguments live in a native caller's frame, in a process variable, in the temporary area and
//! in a heap object; the callee either does nothing that could move them (a stable activation) or
//! calls a function that allocates between its accesses. Every tier gives the same output, and
//! compiled code resolves regions (`cb_region`, counted in debug builds).
#![cfg(feature = "llvm")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

const SRC: &str = r#"entity regions is
  generic (bad : integer := 0);
end;
architecture a of regions is
  type ints_t is array (natural range <>) of integer;
  type ints_ptr_t is access ints_t;

  -- Nothing here allocates, calls or ends a statement: the regions its parameters point
  -- into stay put for the whole activation.
  procedure fill_stable (code : string; variable idx : inout integer;
                         variable res : out ints_t; variable sum : inout integer) is
  begin
    for i in res'range loop
      res(i) := character'pos(code(idx)) + i;
      idx := idx + 1;
      if idx > code'right then idx := code'left; end if;
      sum := sum + res(i);
    end loop;
  end procedure;

  function pad (s : string) return string is
  begin
    return s & "!";
  end function;

  -- A call that allocates between the accesses: only the native callers' frames are held.
  procedure fill_calls (code : string; variable idx : inout integer;
                        variable res : out ints_t; variable sum : inout integer) is
  begin
    for i in res'range loop
      res(i) := character'pos(code(idx)) + i + pad("ab")'length;
      idx := idx + 1;
      if idx > code'right then idx := code'left; end if;
      sum := sum + res(i);
    end loop;
  end procedure;

  -- The arguments are this subprogram's own locals: a native caller's frame.
  procedure from_frame (variable total : inout integer) is
    variable idx : integer := 1;
    variable res : ints_t(0 to 6);
    variable s   : string(1 to 5) := "hello";
    variable sum : integer := 0;
  begin
    fill_stable(s, idx, res, sum);
    fill_calls(s, idx, res, sum);
    total := total + sum + res(3) + idx;
  end procedure;
begin
  process
    variable idx   : integer := 2;
    variable res   : ints_t(1 to 9);
    variable sum   : integer := 0;
    variable total : integer := 0;
    variable str   : string(1 to 6) := "abcdef";
    variable heap  : ints_ptr_t := new ints_t(0 to 4);
    variable hidx  : integer := 1;
  begin
    for round in 1 to 30 loop
      -- the caller's frame
      from_frame(total);
      -- process variables
      fill_stable(str, idx, res, sum);
      fill_calls(str, idx, res, sum);
      -- the temporary area (a concatenation) and a heap object
      fill_stable("xy" & str, hidx, heap.all, sum);
      fill_calls(str & "z", hidx, heap.all, sum);
      total := total + sum + res(5) + heap(2) + idx + hidx;
    end loop;
    report "total " & integer'image(total) & " sum " & integer'image(sum);
    if bad = 1 then fill_stable(str, idx, res(1 to 12), sum); end if;
    if bad = 2 then fill_calls("", idx, res, sum); end if;
    report "end";
    wait;
  end process;
end;
"#;

fn yavs(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(args)
        .env("YAVS_COUNTERS", "1")
        .current_dir(dir)
        .output()
        .expect("run yavs")
}

/// Standard output, standard error without the counters, the exit status, and `cb_region`.
fn run(dir: &Path, flags: &[&str], bad: u32) -> (String, String, Option<i32>, u64) {
    let g = format!("-gbad={bad}");
    let mut args = vec!["run", "--std=2008", "--work=work:work"];
    args.extend_from_slice(flags);
    args.extend(["regions", &g]);
    let o = yavs(dir, &args);
    let err = String::from_utf8_lossy(&o.stderr).into_owned();
    let region = err
        .lines()
        .find_map(|l| l.strip_prefix("counters: cb_region="))
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or(0);
    let rest = err
        .lines()
        .filter(|l| !l.starts_with("counters: "))
        .collect::<Vec<_>>()
        .join("\n");
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        rest,
        o.status.code(),
        region,
    )
}

#[test]
fn address_parameters_match_the_interpreter() {
    let dir = tempdir("compiled_param_regions");
    std::fs::write(dir.join("regions.vhd"), SRC).expect("write");
    let a = yavs(
        &dir,
        &["analyze", "--std=2008", "--work=work:work", "regions.vhd"],
    );
    assert!(a.status.success(), "{a:?}");
    for bad in 0..=2 {
        let want = run(&dir, &["--tier=interp"], bad);
        assert!(want.0.contains("total "), "{want:?}");
        assert_eq!(want.2 == Some(0), bad == 0, "bad {bad}: {want:?}");
        for flags in [
            vec!["--tier=llvm"],
            vec!["--tier=auto", "--tier-heat=1,0,0"],
            vec!["--tier=auto", "--tier-heat=1,0,0", "--threads=4"],
        ] {
            let got = run(&dir, &flags, bad);
            assert_eq!(
                (&got.0, &got.1, got.2),
                (&want.0, &want.1, want.2),
                "bad {bad} {flags:?}"
            );
            if bad == 0 && flags == ["--tier=llvm"] && cfg!(debug_assertions) {
                assert!(
                    got.3 > 0,
                    "compiled code resolved no parameter region: {got:?}"
                );
            }
        }
    }
}
