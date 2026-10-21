//! `yavs run --load`: VHPIDIRECT foreign subprograms calling a C library compiled at test
//! time with the compiler `cc` picks for the build target (#95).

mod common;
use common::tempdir;

use std::path::Path;
use std::process::Command;

const C: &str = r#"
#include <stdint.h>
#ifdef _WIN32
#define EXPORT __declspec(dllexport)
#else
#define EXPORT
#endif
EXPORT int32_t add1(int32_t x) { return x + 1; }
EXPORT int8_t twice(int8_t x) { return (int8_t)(x * 2); }
EXPORT double half(double x) { return x / 2; }
EXPORT uint8_t is_odd(int32_t x) { return (uint8_t)(x & 1); }
EXPORT void set42(int32_t *x) { *x = 42; }
EXPORT int64_t sum(const int32_t *p, int64_t n) {
    int64_t s = 0;
    for (int64_t i = 0; i < n; i++) s += p[i];
    return s;
}
/* std_logic codes: '0' is 2, '1' is 3. */
EXPORT void invert(uint8_t *p, int64_t n) {
    for (int64_t i = 0; i < n; i++) p[i] = p[i] == 3 ? 2 : 3;
}
EXPORT uint8_t bad_bool(void) { return 7; }
"#;

const VHDL: &str = r#"
library ieee; use ieee.std_logic_1164.all;
package c is
  function add1(x : integer) return integer;
  attribute foreign of add1 : function is "VHPIDIRECT add1";
  function twice(x : integer range 0 to 50) return integer;
  attribute foreign of twice : function is "VHPIDIRECT libc_test twice";
  function half(x : real) return real;
  attribute foreign of half : function is "VHPIDIRECT half";
  function is_odd(x : integer) return boolean;
  attribute foreign of is_odd : function is "VHPIDIRECT is_odd";
  procedure set42(x : out integer);
  attribute foreign of set42 : procedure is "VHPIDIRECT set42";
  function sum(v : integer_vector) return integer;
  attribute foreign of sum : function is "VHPIDIRECT sum";
  procedure invert(v : inout std_logic_vector);
  attribute foreign of invert : procedure is "VHPIDIRECT invert";
  function bad_bool return boolean;
  attribute foreign of bad_bool : function is "VHPIDIRECT bad_bool";
  function missing return integer;
  attribute foreign of missing : function is "VHPIDIRECT no_such_symbol";
end;
package body c is
  -- Placeholder bodies, as other simulators require; the foreign attribute wins.
  function add1(x : integer) return integer is begin return 0; end;
  function twice(x : integer range 0 to 50) return integer is begin return 0; end;
  function half(x : real) return real is begin return 0.0; end;
  function is_odd(x : integer) return boolean is begin return false; end;
  procedure set42(x : out integer) is begin x := 0; end;
  function sum(v : integer_vector) return integer is begin return 0; end;
  procedure invert(v : inout std_logic_vector) is begin end;
end;

library ieee; use ieee.std_logic_1164.all; use work.c.all;
entity tb is end;
architecture a of tb is begin
  process
    variable x : integer;
    variable v : std_logic_vector(3 downto 0) := "1100";
  begin
    assert add1(41) = 42;
    assert twice(21) = 42;
    assert half(5.0) = 2.5;
    assert is_odd(3) and not is_odd(4);
    set42(x);
    assert x = 42;
    assert sum((1, 2, 3, 4)) = 10;
    invert(v);
    assert v = "0011";
    report "foreign ok";
    wait;
  end process;
end;

use work.c.all;
entity bad is end;
architecture a of bad is begin
  process begin report boolean'image(bad_bool); wait; end process;
end;

use work.c.all;
entity lazy is end;
architecture a of lazy is begin
  process
    variable n : integer := 0;
  begin
    if n > 0 then report integer'image(missing); end if;
    report "lazy ok";
    wait;
  end process;
end;

use work.c.all;
entity miss is end;
architecture a of miss is begin
  process begin report integer'image(missing); wait; end process;
end;
"#;

/// Builds `src` into a shared library at `out`.
fn shared_lib(src: &Path, out: &Path) {
    let tool = cc::Build::new()
        .target(env!("YAVS_TARGET"))
        .host(env!("YAVS_HOST"))
        .opt_level(0)
        .debug(false)
        .cargo_metadata(false)
        .cargo_warnings(false)
        .get_compiler();
    let mut cmd = tool.to_command();
    cmd.current_dir(out.parent().expect("dir"));
    if tool.is_like_msvc() {
        cmd.args(["/nologo", "/W3", "/LD"])
            .arg(src)
            .arg(format!("/Fe{}", out.display()));
    } else {
        cmd.args(["-Wall", "-Werror", "-shared"])
            .arg(src)
            .arg("-o")
            .arg(out);
        if !cfg!(windows) {
            cmd.arg("-fPIC");
        }
    }
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("cannot run {cmd:?}: {e}"));
    assert!(status.success(), "{cmd:?} failed: {status}");
}

fn yavs(args: &[&str]) -> (i32, String, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["--color", "never"])
        .args(args)
        .output()
        .expect("run");
    (
        o.status.code().expect("exit code"),
        String::from_utf8_lossy(&o.stderr).into_owned(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
    )
}

#[test]
fn load_and_call_c() {
    let tmp = tempdir("foreign");
    let c = tmp.join("c_test.c");
    std::fs::write(&c, C).expect("write");
    let so = tmp.join(if cfg!(windows) {
        "c_test.dll"
    } else {
        "libc_test.so"
    });
    shared_lib(&c, &so);
    let src = tmp.join("tb.vhd");
    std::fs::write(&src, VHDL).expect("write");
    let lib = format!("--work=work:{}", tmp.join("lib").display());
    let load = format!("--load={}", so.display());
    let (code, err, _) = yavs(&["analyze", &lib, &src.display().to_string()]);
    assert_eq!(code, 0, "{err}");

    let (code, err, out) = yavs(&["run", &lib, &load, "tb"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("foreign ok"), "{out}");

    // A result outside its VHDL type is a runtime error, not a wrong value.
    let (code, err, _) = yavs(&["run", &lib, &load, "bad"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("error[E3"), "{err}");

    // Warn early, fail at the call: an unbound symbol on a branch never taken is a warning
    // naming what was searched, and the run completes.
    let (code, err, out) = yavs(&["run", &lib, &load, "lazy"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("lazy ok"), "{out}");
    assert!(
        err.contains("warning[W0108]: symbol `no_such_symbol` of foreign subprogram `missing`"),
        "{err}"
    );
    assert!(err.contains("searched") && err.contains("c_test"), "{err}");

    // `-Werror` turns the warning into an error before the simulation.
    let (code, err, out) = yavs(&["run", &lib, &load, "-Werror", "lazy"]);
    assert_eq!(code, 2, "{err}");
    assert!(!out.contains("lazy ok"), "{out}");

    // Calling it is E0107, a runtime error at the call with the same note.
    let (code, err, _) = yavs(&["run", &lib, &load, "miss"]);
    assert_eq!(code, 1, "{err}");
    assert!(
        err.contains("error[E0107]: symbol `no_such_symbol` of foreign subprogram `missing`"),
        "{err}"
    );
    assert!(err.contains("searched") && err.contains("c_test"), "{err}");

    // Without `--load` the C functions are not found either: a warning each, an error at the
    // first call.
    let (code, err, _) = yavs(&["run", &lib, "tb"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("warning[W0108]"), "{err}");
    assert!(err.contains("error[E0107]"), "{err}");

    let missing = format!("--load={}", tmp.join("nope.so").display());
    let (code, err, _) = yavs(&["run", &lib, &missing, "tb"]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("error[E0106]: cannot load library"), "{err}");
    let _ = std::fs::remove_dir_all(&tmp);
}
