//! `yavs run --interactive` and `--do`: the console commands over `--do` and piped standard input,
//! restart, Ctrl-C, and the console hook of a test runner (an in-test runner over TCP).

mod common;

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use common::tempdir;
use serde_json::{Value, json};

fn yavs() -> Command {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
}

fn setup(name: &str) -> common::Tmp {
    let d = tempdir(&format!("console-{name}"));
    let cases = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/dap");
    std::fs::copy(cases.join("tb.vhd"), d.join("tb.vhd")).expect("copy");
    analyze(&d, "tb.vhd");
    d
}

fn tempdir_with(name: &str, src: &str) -> common::Tmp {
    let d = tempdir(&format!("console-{name}"));
    std::fs::write(d.join("t.vhd"), src).expect("write");
    analyze(&d, "t.vhd");
    d
}

fn analyze(d: &Path, file: &str) {
    let ok = yavs()
        .current_dir(d)
        .args(["analyze", "--work=work:lib", file])
        .status()
        .expect("yavs")
        .success();
    assert!(ok, "analyze failed");
}

fn run(d: &Path, top: &str, args: &[&str]) -> Output {
    yavs()
        .current_dir(d)
        .args(["run", "--work=work:lib", top])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("yavs")
}

fn script(d: &Path, top: &str, cmds: &str) -> Output {
    run(d, top, &[&format!("--do={cmds}")])
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// `--do` to the end prints and exits as a plain run.
#[test]
fn a_script_to_the_end_equals_a_batch_run() {
    let d = setup("batch");
    let plain = run(&d, "tb", &[]);
    let console = script(&d, "tb", "run; quit");
    assert_eq!(stdout(&console), stdout(&plain));
    assert_eq!(console.status.code(), plain.status.code());
    // Without `quit`, the end of the script ends the session.
    let console = script(&d, "tb", "run");
    assert_eq!(stdout(&console), stdout(&plain));
    assert_eq!(console.status.code(), Some(0));
}

/// A design with nothing to run still has a session: no process, signals only, generate blocks,
/// empty instances, and a package signal. `run` ends at time 0 as a plain run does (#995).
#[test]
fn a_design_with_nothing_to_run_has_a_session() {
    for (name, src, signal) in [
        (
            "empty",
            "entity t is end; architecture a of t is begin end;",
            None,
        ),
        (
            "signals",
            "entity t is end; architecture a of t is signal s : integer := 7; begin end;",
            Some("s"),
        ),
        (
            "generate",
            "entity t is end; architecture a of t is begin
               g : for i in 0 to 1 generate signal s : bit; begin end generate;
               h : if false generate end generate;
             end;",
            None,
        ),
        (
            "instances",
            "entity sub is end; architecture a of sub is begin end;
             entity t is end; architecture a of t is begin u : entity work.sub; end;",
            None,
        ),
        (
            "package",
            "package p is signal s : integer := 3; end package;
             use work.p.all;
             entity t is end; architecture a of t is begin end;",
            None,
        ),
    ] {
        let d = tempdir_with(&format!("nothing-{name}"), src);
        let plain = run(&d, "t", &[]);
        assert_eq!(plain.status.code(), Some(0), "{name}: {}", stderr(&plain));
        let o = script(&d, "t", "run; quit");
        assert_eq!(o.status.code(), Some(0), "{name}: {}", stderr(&o));
        assert_eq!(stdout(&o), stdout(&plain), "{name}");
        let o = run(&d, "t", &["--interactive"]);
        assert_eq!(o.status.code(), Some(0), "{name}: {}", stderr(&o));
        // The commands that look at a stop answer; the end of the script ends the session.
        let o = script(&d, "t", "where; waits; activity; restart; run");
        assert_eq!(o.status.code(), Some(0), "{name}: {}", stderr(&o));
        if let Some(s) = signal {
            let o = script(&d, "t", &format!("examine {s}; quit"));
            assert_eq!(o.status.code(), Some(0), "{name}: {}", stderr(&o));
            assert!(stdout(&o).contains('7'), "{name}: {}", stdout(&o));
        }
    }
}

#[test]
fn run_examine_where() {
    let d = setup("run");
    let o = script(
        &d,
        "tb",
        "examine cnt; where; run 10 ns; examine cnt; examine v(1 downto 0); print v(0); where",
    );
    assert_eq!(
        stdout(&o),
        "0\ntime 0 fs, delta 0\nbetween cycles\n4\n\"10\"\n'0'\ntime 10 ns, delta 2\nbetween cycles\n"
    );
    assert!(stderr(&o).contains("at 10 ns, delta 2"), "{}", stderr(&o));
    assert_eq!(o.status.code(), Some(0));
}

#[test]
fn run_until_a_condition() {
    let d = setup("until");
    let o = script(&d, "tb", "run until cnt = 4; where; examine cnt");
    assert_eq!(stdout(&o), "time 10 ns, delta 1\nbetween cycles\n4\n");
    // A change is read between cycles, where a variable of a process is not: an unknown name.
    let o = script(
        &d,
        "tb",
        "break tb.vhd:21; run; run until i = 3 and cnt = 2",
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).contains("no local or signal `i` here"),
        "{}",
        stderr(&o)
    );
    // A generic is a name, with nothing to watch.
    let o = script(&d, "tb", "run until N = 3");
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("names no signal"), "{}", stderr(&o));
    let o = script(&d, "tb", "run until M = 3");
    assert!(
        stderr(&o).contains("no local or signal `M` here"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn step_and_next() {
    let d = setup("step");
    let o = script(
        &d,
        "tb",
        "break tb.vhd:20; run; where; step; where; next; where; step delta; step timestep; where",
    );
    let out = stdout(&o);
    let at: Vec<&str> = out
        .lines()
        .filter_map(|l| {
            l.rsplit(['/', '\\'])
                .next()
                .filter(|t| t.starts_with("tb.vhd:"))
        })
        .collect();
    // The set line and the stop, into the function, then its next line; nothing after the steps.
    assert_eq!(
        at,
        ["tb.vhd:20", "tb.vhd:20", "tb.vhd:9", "tb.vhd:10"],
        "{out}"
    );
    assert!(out.ends_with("between cycles\n"), "{out}");
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
}

#[test]
fn force_deposit_release() {
    let d = setup("force");
    let o = script(
        &d,
        "tb",
        "force cnt 5; run 1 ns; examine cnt; release cnt; deposit v \"1111\"; run 1 ns; examine v",
    );
    assert_eq!(
        stdout(&o),
        // The deposit, then `q` inverts it when the release changes `cnt`.
        "tb.cnt = 5\n5\ntb.cnt\ntb.v = \"1111\"\n\"0000\"\n"
    );
}

#[test]
fn breakpoints_on_lines_and_signals() {
    let d = setup("break");
    let o = script(
        &d,
        "tb",
        "break tb.vhd:24; break cnt = 4; break list; run; where; break delete 2; run; where; break delete all; break list",
    );
    let out = stdout(&o);
    assert!(out.contains("breakpoint 1: "), "{out}");
    assert!(out.contains("breakpoint 2: tb.cnt"), "{out}");
    let times: Vec<&str> = out.lines().filter(|l| l.starts_with("time ")).collect();
    assert_eq!(
        times,
        ["time 10 ns, delta 1", "time 30 ns, delta 0"],
        "{out}"
    );
    assert!(out.ends_with("no breakpoints\n"), "{out}");
    // A breakpoint with a condition; a file that is not in the design; a bad number.
    let o = script(&d, "tb", "break tb.vhd:22 if k = 2; run; examine k");
    assert!(stdout(&o).ends_with("2\n"), "{}", stdout(&o));
    let o = script(&d, "tb", "break nope.vhd:1");
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).contains("`nope.vhd` is not a source file"),
        "{}",
        stderr(&o)
    );
    let o = script(&d, "tb", "break delete 9");
    assert!(stderr(&o).contains("no breakpoint 9"), "{}", stderr(&o));
}

/// Back at time 0 the design runs again with the same output; breakpoints stay.
#[test]
fn restart_keeps_breakpoints_and_repeats_the_run() {
    let d = setup("restart");
    let plain = stdout(&run(&d, "tb", &[]));
    let o = script(
        &d,
        "tb",
        "run; restart; where; examine cnt; run; reset; run",
    );
    let want = format!("{plain}time 0 fs, delta 0\nbetween cycles\n0\n{plain}{plain}");
    assert_eq!(stdout(&o), want);
    // From a stop inside a process: what the process would still do is not seen.
    let o = script(
        &d,
        "tb",
        "break tb.vhd:24; run; restart; run; where; restart; break list",
    );
    let out = stdout(&o);
    assert_eq!(
        out.matches("tb.vhd:24").count(),
        3,
        "set, where and list: {out}"
    );
    assert!(!out.contains("done"), "{out}");
}

/// A process that never suspends, stopped at a breakpoint and restarted.
#[test]
fn restart_out_of_a_process_that_never_suspends() {
    let d = setup("spinloop");
    std::fs::write(
        d.join("l.vhd"),
        "entity l is end;\narchitecture a of l is\nbegin\n  process\n    variable i : natural := 0;\n  begin\n    loop\n      i := (i + 1) mod 7;\n    end loop;\n  end process;\nend;\n",
    )
    .expect("write");
    analyze(&d, "l.vhd");
    let o = script(&d, "l", "break l.vhd:8; run; restart; run; examine i; quit");
    assert!(stdout(&o).ends_with("\n0\n"), "{}", stdout(&o));
    assert_eq!(o.status.code(), Some(0));
}

/// `Output` of `cmd`, killed and failed when it does not end: a hang must fail the test, not CI.
fn bounded(mut cmd: Command, input: &str) -> Output {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("yavs");
    child
        .stdin
        .take()
        .expect("piped")
        .write_all(input.as_bytes())
        .expect("write");
    let t0 = std::time::Instant::now();
    while child.try_wait().expect("wait").is_none() {
        if t0.elapsed() > std::time::Duration::from_secs(120) {
            let _ = child.kill();
            panic!("still running: {:?}", child.wait_with_output());
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    child.wait_with_output().expect("output")
}

/// A stop in a loop inside a subprogram, and in a loop with no statement at all: the session
/// can be left and restarted (#918).
#[test]
fn leaving_an_endless_loop_in_a_subprogram() {
    let d = setup("spinsub");
    std::fs::write(
        d.join("l.vhd"),
        "entity l is end;\narchitecture a of l is\n  procedure spin is\n    variable i : natural := 0;\n  begin\n    loop\n      i := (i + 1) mod 7;\n    end loop;\n  end procedure;\nbegin\n  process begin\n    spin;\n    wait;\n  end process;\nend;\n",
    )
    .expect("write");
    std::fs::write(
        d.join("e.vhd"),
        "entity e is end;\narchitecture a of e is\n  procedure spin is\n  begin\n    loop\n    end loop;\n  end procedure;\nbegin\n  process begin\n    report \"before\";\n    spin;\n    wait;\n  end process;\nend;\n",
    )
    .expect("write");
    analyze(&d, "l.vhd");
    analyze(&d, "e.vhd");
    let session = |top: &str, args: &[&str], input: &str| {
        let mut c = yavs();
        c.current_dir(&*d).args(["run", "--work=work:lib", top]);
        c.args(args);
        bounded(c, input)
    };
    let stop = "--do=break l.vhd:7; run";
    for (input, status) in [("quit\n", 0), ("quit 3\n", 3), ("", 0)] {
        let o = session("l", &[stop, "--interactive"], input);
        assert_eq!(o.status.code(), Some(status), "{input:?}: {}", stderr(&o));
    }
    let o = session(
        "l",
        &[stop, "--interactive"],
        "restart\nwhere\nrun\nexamine i\nquit\n",
    );
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert!(stdout(&o).contains("time 0 fs, delta 0"), "{}", stdout(&o));
    // No statement in the loop: nothing to stop at, but the session still ends.
    let mut c = yavs();
    c.current_dir(&*d)
        .args(["run", "--work=work:lib", "e", "--dap=stdio"]);
    let mut child = c
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("yavs");
    let mut w = child.stdin.take().expect("piped");
    let mut r = BufReader::new(child.stdout.take().expect("piped"));
    let send = |w: &mut std::process::ChildStdin, seq: i64, cmd: &str| {
        let m = json!({"type": "request", "seq": seq, "command": cmd, "arguments": {}}).to_string();
        write!(w, "Content-Length: {}\r\n\r\n{m}", m.len()).expect("write");
    };
    let read = |r: &mut BufReader<std::process::ChildStdout>| -> Value {
        let mut n = 0;
        loop {
            let mut l = String::new();
            assert!(r.read_line(&mut l).expect("read") > 0, "the session ended");
            if let Some(v) = l.trim().strip_prefix("Content-Length: ") {
                n = v.parse().expect("a length");
            } else if l.trim().is_empty() {
                break;
            }
        }
        let mut b = vec![0; n];
        std::io::Read::read_exact(r, &mut b).expect("read");
        serde_json::from_slice(&b).expect("json")
    };
    for (seq, cmd) in [(1, "initialize"), (2, "launch"), (3, "configurationDone")] {
        send(&mut w, seq, cmd);
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    send(&mut w, 4, "disconnect");
    while read(&mut r)["command"] != "disconnect" {}
    let t0 = std::time::Instant::now();
    while child.try_wait().expect("wait").is_none() {
        assert!(
            t0.elapsed() < std::time::Duration::from_secs(60),
            "still running"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

const PARTS: &str = "library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t is end;
architecture a of t is
  type rec is record x : integer; y : integer; end record;
  type vec is array (0 to 3) of integer;
  type ptr_t is access rec;
  type node_t;
  type node_ptr is access node_t;
  type node_t is record v : integer; next_n : node_ptr; end record;
  type counter_t is protected
    procedure incr;
  end protected;
  type counter_t is protected body
    variable n : integer := 0;
    variable text : string(1 to 2) := \"ab\";
    procedure incr is
    begin
      n := n + 1;
    end procedure;
  end protected body;
  shared variable sc : counter_t;
  signal cnt : unsigned(7 downto 0) := to_unsigned(3, 8);
  signal sg : signed(3 downto 0) := to_signed(-2, 4);
  signal slv : std_logic_vector(3 downto 0) := \"0101\";
  signal pt : rec := (5, 6);
begin
  process
    variable p : rec := (1, 2);
    variable v : vec := (1, 2, 3, 4);
    variable s : string(1 to 5) := \"hello\";
    variable q : ptr_t := new rec'(x => 7, y => 8);
    variable nul : ptr_t;
    variable lst : node_ptr := new node_t'(1, new node_t'(2, null));
  begin
    sc.incr;
    wait for 1 ns;
    report \"stop here\";
    wait;
  end process;
end;
";

fn parts(name: &str) -> common::Tmp {
    let d = setup(name);
    std::fs::write(d.join("t.vhd"), PARTS).expect("write");
    let ok = yavs()
        .current_dir(&*d)
        .args(["analyze", "--std=2008", "--work=work:lib", "t.vhd"])
        .status()
        .expect("yavs")
        .success();
    assert!(ok, "analyze failed");
    d
}

/// Every line of `exprs` examined at the `report` statement, one answer per line (#930, #936).
#[track_caller]
fn check_examine(d: &Path, cases: &[(&str, &str)]) {
    let cmds: Vec<String> = cases.iter().map(|(e, _)| format!("examine {e}")).collect();
    let o = script(d, "t", &format!("break t.vhd:39; run; {}", cmds.join("; ")));
    let got = stdout(&o);
    let lines: Vec<&str> = got.lines().skip(1).collect();
    let want: Vec<&str> = cases.iter().map(|(_, w)| *w).collect();
    assert_eq!(lines, want, "{}", stderr(&o));
}

#[test]
fn examine_parts_of_variables_and_access_values() {
    let d = parts("parts");
    check_examine(
        &d,
        &[
            ("p", "(x => 1, y => 2)"),
            ("p.x", "1"),
            ("v(2)", "3"),
            ("s(2 to 3)", "\"el\""),
            ("q", "(x => 7, y => 8)"),
            ("q.all", "(x => 7, y => 8)"),
            ("q.all.x", "7"),
            ("q.y", "8"),
            ("nul", "null"),
            ("lst", "(v => 1, next_n => access)"),
            ("lst.next_n.v", "2"),
            ("lst.next_n.next_n", "null"),
            ("cnt(1 downto 0)", "\"11\""),
            ("pt.y", "6"),
        ],
    );
}

#[test]
fn examine_computes_as_a_condition_does() {
    let d = parts("computes");
    check_examine(
        &d,
        &[
            ("cnt = 3", "TRUE"),
            ("3 = cnt", "TRUE"),
            ("cnt > 3", "FALSE"),
            ("sg = -2", "TRUE"),
            ("sg < 0", "TRUE"),
            ("to_integer(cnt) + 1", "4"),
            // Operators and functions of numeric_std and attributes (#364).
            ("cnt + 1 = 4 and resize(cnt, 8) = 3", "TRUE"),
            ("cnt'length + cnt'high", "15"),
            ("to_integer(sg)", "-2"),
            ("v(1) + 1", "3"),
            ("nul = null", "TRUE"),
            ("q /= null", "TRUE"),
        ],
    );
}

#[test]
fn examine_errors_say_what_is_wrong() {
    let d = parts("experr");
    for (e, want) in [
        ("nosuch", "no variable or signal `nosuch` here"),
        ("p.nosuch", "no field `nosuch`"),
        ("v(9)", "index 9 is outside the array"),
        ("nul.x", "the access value is null"),
        ("p.all", "not an access value"),
        ("slv = 3", "`std_ulogic_vector`, which has no numbers"),
        (
            "to_integer(slv)",
            "no `to_integer` here takes (std_ulogic_vector)",
        ),
        ("rising_edge(cnt(0))", "takes a signal parameter"),
        ("cnt'image", "attribute `'image` is not supported"),
    ] {
        let o = script(&d, "t", &format!("break t.vhd:39; run; examine {e}"));
        assert!(stderr(&o).contains(want), "{e}: {}", stderr(&o));
        assert_eq!(o.status.code(), Some(1), "{e}");
    }
}

/// The variables of a protected body, in the frame of its method (#936).
#[test]
fn variables_of_a_protected_body() {
    let d = parts("prot");
    let o = script(
        &d,
        "t",
        "break t.vhd:20; run; examine n; examine text; examine text(1); examine n + 1",
    );
    assert_eq!(
        stdout(&o).lines().skip(1).collect::<Vec<_>>(),
        ["0", "\"ab\"", "'a'", "1"],
        "{}",
        stderr(&o)
    );
}

/// Every name that misses says what is close (rule: one helper for all of them).
#[test]
fn every_name_suggests() {
    let d = parts("suggest");
    for (cmds, want) in [
        ("step dlta", "`delta`"),
        ("wave adn", "needs a viewer"),
        ("break delete al", "`all`"),
        ("break t.vh:1", "similar name exists: `t.vhd`"),
        ("run 1 ns; examine cn", "similar name exists: `cnt`"),
        ("run 1 ns; print cn", "similar name exists: `cnt`"),
        ("force slb '1'", "similar name exists: `slv`"),
        ("deposit cn 1", "`cnt`"),
        ("release cn", "`cnt`"),
        ("break cn", "`cnt`"),
        ("run 1 ns; examine pt.xx", "`x`"),
        ("run 1 ns; examine cn = 3", "`cnt`"),
    ] {
        let o = script(&d, "t", cmds);
        assert!(stderr(&o).contains(want), "{cmds}: {}", stderr(&o));
    }
}

#[test]
fn help_is_aligned_wrapped_and_plain_in_a_pipe() {
    let d = setup("helpwrap");
    let o = script(&d, "tb", "help");
    let out = stdout(&o);
    assert!(out.lines().all(|l| l.len() <= 80), "{out}");
    assert!(!out.contains('\x1b'), "{out}");
    // The meanings start in one column.
    let at = |syntax: &str| {
        let l = out.lines().find(|l| l.starts_with(syntax)).expect(syntax);
        l.len() - l[syntax.len()..].trim_start().len()
    };
    assert_eq!(at("  next"), at("  waits"));
    assert!(out.contains("exit"), "{out}");
}

/// A path from where a name is looked up, like the full one; a miss in it finds the full name.
#[test]
fn a_path_starts_from_the_scope_too() {
    let d = tempdir_with(
        "relpath",
        "entity sub is end;
architecture a of sub is signal q : integer := 7; begin end;
entity t is end;
architecture a of t is
begin
  u_reg : entity work.sub;
  process begin wait for 1 ns; wait; end process;
end;",
    );
    let o = script(
        &d,
        "t",
        "run 1 ns; examine u_reg.q; examine t.u_reg.q; force u_reg.q 8",
    );
    assert_eq!(stdout(&o), "7\n7\nt.u_reg.q = 8\n", "{}", stderr(&o));
    let o = script(&d, "t", "examine u_reg.qq");
    assert!(stderr(&o).contains("`t.u_reg.q`"), "{}", stderr(&o));
}

/// A release with no `-out` ends a force of either kind (#953).
#[test]
fn release_ends_either_force() {
    let d = setup("releaseboth");
    let held = script(&d, "tb", "force cnt 99 -out; run 25 ns; examine cnt");
    assert!(stdout(&held).ends_with("99\n"), "{}", stdout(&held));
    let freed = script(
        &d,
        "tb",
        "force cnt 99 -out; run 1 ns; release cnt; run 25 ns; examine cnt",
    );
    assert!(stdout(&freed).ends_with("6\n"), "{}", stdout(&freed));
}

/// `exit` is `quit`, its status too.
#[test]
fn exit_is_quit() {
    let d = setup("exitquit");
    let o = script(&d, "tb", "exit 3");
    assert_eq!(o.status.code(), Some(3), "{}", stderr(&o));
    let o = script(&d, "tb", "exit");
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
}

/// One parser for values typed in force, deposit and break on value (#953).
#[test]
fn values_can_leave_out_the_quotes_of_a_bit_like_target() {
    let d = tempdir_with(
        "permissive",
        "library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t is end;
architecture a of t is
  signal sl : std_logic := '0';
  signal b : bit := '0';
  signal i : integer := 0;
  signal slv : std_logic_vector(3 downto 0) := \"0000\";
  signal u : unsigned(3 downto 0) := \"0000\";
  signal s : string(1 to 2) := \"ab\";
begin
  process begin wait for 1 ns; wait; end process;
end;",
    );
    let o = script(
        &d,
        "t",
        "force sl 1; force b 1; force i 1; force slv 0101; force u 1010; force slv(0) 0; deposit b 0; run 1 ns; examine sl; examine i; examine slv; examine u",
    );
    assert_eq!(
        stdout(&o),
        "t.sl = '1'\nt.b = '1'\nt.i = 1\nt.slv = \"0101\"\nt.u = \"1010\"\nt.slv(0) = '0'\nt.b = '0'\n'1'\n1\n\"0100\"\n\"1010\"\n",
        "{}",
        stderr(&o)
    );
    let bad = |cmd: &str, want: &str| {
        let o = script(&d, "t", cmd);
        let e = stderr(&o);
        assert!(e.contains(want), "{cmd}: {e}");
        assert_eq!(o.status.code(), Some(1), "{cmd}: {e}");
    };
    bad("force sl 2", "`2` is not a value of `t.sl`, type STD_LOGIC");
    bad("force slv 01", "type STD_LOGIC_VECTOR: expected 4 elements");
    bad("deposit i 0x", "type INTEGER: expected an integer");
    // The quotes stay where a target could take the text otherwise.
    bad("force s ab", "type STRING");
    bad("break sl = 2", "`2` is not a value of `t.sl`");
    assert_eq!(script(&d, "t", "break sl = 1").status.code(), Some(0));
}

/// A typo in a command names the command; a condition on a number works and its error comes when
/// it is set (#935).
#[test]
fn typos_and_conditions_are_helped() {
    let d = parts("typos");
    let o = script(&d, "t", "hepl");
    assert!(
        stderr(&o).contains("similar name exists: `help`"),
        "{}",
        stderr(&o)
    );
    let o = script(&d, "t", "break t.vhd:39 if slv = 3");
    assert!(
        stderr(&o).contains("which has no numbers"),
        "{}",
        stderr(&o)
    );
    assert_eq!(o.status.code(), Some(1));
    let o = script(&d, "t", "break t.vhd:39 if cnt = 4; run; where");
    assert!(stderr(&o).contains("ended"), "{}", stderr(&o));
    let o = script(&d, "t", "break t.vhd:39 if cnt = 3; run; where");
    assert!(stdout(&o).contains("t.vhd:39"), "{}", stdout(&o));
    let o = script(&d, "t", "break t.vhd:39 if to_integer(cnt) = 3; run; where");
    assert!(stdout(&o).contains("t.vhd:39"), "{}", stdout(&o));
}

/// An impure function that reads a signal and `now` and reports, also called by the design.
const WATCH: &str = "\
entity t is
end;
architecture a of t is
  signal cnt : natural := 0;
  impure function watch(x : integer) return integer is
  begin
    report \"watch \" & integer'image(cnt) severity error;
    return cnt + x + now / 1 ns;
  end function;
begin
  p : process
  begin
    for i in 1 to 5 loop
      cnt <= cnt + 1;
      wait for 10 ns;
    end loop;
    report \"done \" & integer'image(watch(0));
    wait;
  end process;
end;
";

/// Conditions that call a read-only impure function change nothing: a run whose conditions never
/// hold prints and exits as a plain run, with line and `run until` conditions, at any `--threads`
/// (#364). One that holds stops where it should.
#[test]
fn read_only_impure_calls_change_nothing() {
    let d = tempdir_with("impure", WATCH);
    for threads in ["--threads=1", "--threads=4"] {
        let plain = run(&d, "t", &[threads]);
        assert!(stdout(&plain).contains("watch 5"), "{}", stdout(&plain));
        for cmds in [
            "break t.vhd:15 if watch(1000) = -1; run; quit",
            "run until watch(1000) = -1 and cnt > 0; quit",
        ] {
            let o = run(&d, "t", &[threads, &format!("--do={cmds}")]);
            // (Less the console's answer to `break`.)
            let out = stdout(&o);
            let out: Vec<&str> = out
                .lines()
                .filter(|l| !l.starts_with("breakpoint 1: "))
                .collect();
            assert_eq!(
                out,
                stdout(&plain).lines().collect::<Vec<_>>(),
                "{cmds}: {}",
                stderr(&o)
            );
            assert_eq!(o.status.code(), plain.status.code(), "{cmds}");
        }
    }
    let o = script(
        &d,
        "t",
        "break t.vhd:15 if watch(0) > 30; run; examine cnt; examine watch(0); examine now",
    );
    let out = stdout(&o);
    let lines: Vec<&str> = out.lines().skip(1).collect();
    assert_eq!(lines, ["3", "33", "30 ns"], "{}", stderr(&o));
}

/// Functions that use a shared variable that is not protected or a signal of a package (#364).
const UNPROTECTED: &str = "\
package spk is
  signal ps : integer := 7;
  impure function getp(x : integer) return integer;
end package;
package body spk is
  impure function getp(x : integer) return integer is
  begin
    return ps + x;
  end function;
end package body;
use work.spk.all;
entity t is end;
architecture a of t is
  shared variable n : natural := 0;
  impure function bumpn(x : integer) return integer is
  begin
    n := n + 1;
    return x + n;
  end function;
begin
  p : process
  begin
    wait for 1 ns;
    report \"here\";
    wait;
  end process;
end;
";

/// A function that may write is refused when the condition is set, naming the variable; one that
/// reads what a condition cannot pass says so (#364).
#[test]
fn writers_and_unreachable_reads_are_refused() {
    let d = tempdir("console-unprotected");
    std::fs::write(d.join("t.vhd"), UNPROTECTED).expect("write");
    let ok = yavs()
        .current_dir(&*d)
        .args(["analyze", "--relaxed", "--work=work:lib", "t.vhd"])
        .status()
        .expect("yavs")
        .success();
    assert!(ok, "analyze failed");
    for (cmds, want) in [
        (
            "break t.vhd:24 if bumpn(1) = 1",
            "`bumpn` may write, so a condition does not call it: it reads or writes shared \
             variable `n` (t.vhd:14)",
        ),
        (
            "break t.vhd:24; run; examine getp(1)",
            "`getp` cannot be called in a condition: it reads `ps`, which a condition cannot pass",
        ),
    ] {
        let o = run(&d, "t", &["--relaxed", &format!("--do={cmds}")]);
        assert!(stderr(&o).contains(want), "{cmds}: {}", stderr(&o));
        assert_eq!(o.status.code(), Some(1), "{cmds}");
    }
    // An impure function with an inout parameter (2019) writes what it is given.
    let src = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/dap/inout_fn.vhd"),
    )
    .expect("read");
    std::fs::write(d.join("m.vhd"), src).expect("write");
    let ok = yavs()
        .current_dir(&*d)
        .args(["analyze", "--std=2019", "--work=work:lib", "m.vhd"])
        .status()
        .expect("yavs")
        .success();
    assert!(ok, "analyze failed");
    let o = run(
        &d,
        "t3",
        &["--std=2019", "--do=break m.vhd:15; run; examine take(1)"],
    );
    assert!(
        stderr(&o).contains("`take` has a parameter of mode out or inout"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn restart_recreates_the_wave_file() {
    let d = setup("wave");
    let o = run(
        &d,
        "tb",
        &["--wave=w.fst", "--do=run; restart; run 15 ns; quit"],
    );
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let f = wellen::simple::read(d.join("w.fst")).expect("readable");
    let t = f.time_table().last().copied().unwrap_or(0);
    // A whole run goes on to 30 ns: the file is the second run's.
    assert!(t < 20_000_000, "{t}");
}

/// `wave` without `--surfer` is an error that says what to do (found testing the console by hand).
#[test]
fn wave_commands_need_a_viewer() {
    let d = setup("wave");
    for cmd in ["wave add cnt", "wave cursor"] {
        let o = script(&d, "tb", cmd);
        assert_eq!(o.status.code(), Some(1), "{cmd}");
        assert!(
            stderr(&o).contains("`wave` needs a viewer: run with `--surfer`"),
            "{cmd}: {}",
            stderr(&o)
        );
    }
}

/// `help COMMAND` prints the forms and the meaning of the table; an alias gives its command's.
#[test]
fn help_for_one_command() {
    let d = setup("help");
    let o = script(&d, "tb", "help break");
    let out = stdout(&o);
    assert!(
        out.starts_with("break [FILE:]LINE [if CONDITION]\n"),
        "{out}"
    );
    assert!(
        out.contains("break delete all\n    Stop at a line"),
        "{out}"
    );
    let o = script(&d, "tb", "help print");
    let out = stdout(&o);
    assert!(
        out.starts_with("examine NAME\nexamine NAME @ TIME\n"),
        "{out}"
    );
    assert!(out.contains("Also written `print`."), "{out}");
    assert!(out.contains("needs a wave file"), "{out}");
    let o = script(&d, "tb", "help brek");
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).contains("no command `brek`: a command with a similar name exists: `break`"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn source_files() {
    let d = setup("source");
    std::fs::write(
        d.join("s.do"),
        "# a comment\nrun 10 ns -- to here\nexamine cnt; examine v\n\nbogus\nexamine cnt\n",
    )
    .expect("write");
    let o = script(&d, "tb", "source s.do; examine cnt");
    assert_eq!(stdout(&o), "4\n\"1010\"\n", "the failure ends the script");
    assert!(
        stderr(&o).contains("unknown command or signal `bogus`"),
        "{}",
        stderr(&o)
    );
    assert_eq!(o.status.code(), Some(1));
    let o = script(&d, "tb", "source nope.do");
    assert!(
        stderr(&o).contains("cannot read `nope.do`"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn quit_statuses() {
    let d = setup("quit");
    assert_eq!(script(&d, "tb", "quit 7").status.code(), Some(7));
    assert_eq!(script(&d, "tb", "run 1 ns; quit").status.code(), Some(0));
    assert_eq!(script(&d, "tb", "quit x").status.code(), Some(1));
    std::fs::write(
        d.join("f.vhd"),
        "entity f is end;\narchitecture a of f is\nbegin\n  process begin\n    report \"bad\" severity failure;\n    wait;\n  end process;\nend;\n",
    )
    .expect("write");
    analyze(&d, "f.vhd");
    let o = script(&d, "f", "run; quit");
    assert_eq!(o.status.code(), run(&d, "f", &[]).status.code());
    assert_eq!(o.status.code(), Some(1));
    // The end stays serviceable until `quit`.
    assert_eq!(script(&d, "f", "run; step").status.code(), Some(1));
    assert_eq!(
        script(&d, "f", "run; run; where; quit 0").status.code(),
        Some(0)
    );
}

#[test]
fn piped_standard_input() {
    let d = setup("piped");
    let mut c = yavs()
        .current_dir(&d)
        .args(["run", "--work=work:lib", "tb", "--interactive"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("yavs");
    c.stdin
        .take()
        .expect("stdin")
        .write_all(b"run 10 ns\nbogus\nexamine cnt\n\nhelp\n")
        .expect("write");
    let o = c.wait_with_output().expect("wait");
    let out = stdout(&o);
    // No prompt on a pipe; a failure does not end a session with a user; end of input is `quit`.
    assert!(out.starts_with("4\n"), "{out}");
    assert!(!out.contains("yavs>"), "{out}");
    assert!(out.contains("back to time 0"), "{out}");
    assert!(stderr(&o).contains("unknown command or signal `bogus`"));
    assert_eq!(o.status.code(), Some(0));
}

#[test]
fn flags_conflict_and_debug_rules() {
    let d = setup("flags");
    let o = run(&d, "tb", &["--interactive", "--dap=stdio"]);
    assert_eq!(o.status.code(), Some(2));
    let o = run(&d, "tb", &["--do=run", "--dap=stdio"]);
    assert_eq!(o.status.code(), Some(2));
    let o = run(&d, "tb", &["--do=run", "--threads=4"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(stderr(&o).contains("W0109"), "{}", stderr(&o));
    assert!(stderr(&o).contains("--interactive"), "{}", stderr(&o));
    let o = run(&d, "tb", &["--do=run", "--profile"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("E0125"), "{}", stderr(&o));
}

/// A subprogram with unconstrained parameters under the debugger (#863).
#[test]
fn string_parameters_under_the_debugger() {
    let d = setup("lean");
    std::fs::write(
        d.join("m.vhd"),
        r#"package p is
  impure function has(d, key : string) return boolean;
  impure function a(cfg : string) return boolean;
end package;
package body p is
  impure function has(d, key : string) return boolean is
  begin
    return d'length > key'length;
  end;
  impure function a(cfg : string) return boolean is
  begin
    if has(cfg, "ab") then
      return cfg = "abc";
    else
      return false;
    end if;
  end;
end package body;
use work.p.all;
entity m is end;
architecture x of m is
begin
  process begin
    report boolean'image(a("abc"));
    wait;
  end process;
end;
"#,
    )
    .expect("write");
    analyze(&d, "m.vhd");
    let o = script(&d, "m", "run");
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert!(stdout(&o).contains("true"), "{}", stdout(&o));
}

#[cfg(unix)]
#[test]
fn ctrl_c_pauses_and_the_console_goes_on() {
    use std::time::{Duration, Instant};
    let d = setup("ctrlc");
    let mut c = yavs()
        .current_dir(&d)
        .args(["run", "--work=work:lib", "spin", "--interactive"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("yavs");
    let mut input = c.stdin.take().expect("stdin");
    input.write_all(b"run\n").expect("write");
    input.flush().expect("flush");
    let mut err = BufReader::new(c.stderr.take().expect("stderr"));
    // Twice: a first SIGINT may arrive before the command, a pause must come of every other one.
    let t0 = Instant::now();
    let pid = c.id().to_string();
    let killer = std::thread::spawn(move || {
        for _ in 0..3 {
            std::thread::sleep(Duration::from_millis(700));
            let _ = Command::new("kill").args(["-INT", &pid]).status();
        }
    });
    let mut line = String::new();
    err.read_line(&mut line).expect("read");
    assert!(line.starts_with("pause at "), "{line}");
    assert!(t0.elapsed() < Duration::from_secs(60));
    input.write_all(b"where\nquit 5\n").expect("write");
    drop(input);
    killer.join().expect("killer");
    let o = c.wait_with_output().expect("wait");
    assert!(stdout(&o).contains("time "), "{}", stdout(&o));
    assert_eq!(o.status.code(), Some(5));
}

/// What the in-test runner does when it gets a call.
struct Runner {
    r: BufReader<TcpStream>,
    w: TcpStream,
}

impl Runner {
    fn send(&mut self, m: Value) {
        writeln!(self.w, "{m}").expect("send");
    }

    fn recv(&mut self) -> Option<Value> {
        let mut l = String::new();
        match self.r.read_line(&mut l) {
            Ok(n) if n > 0 => serde_json::from_str(&l).ok(),
            _ => None,
        }
    }

    /// `exec` or `reload`: the answer.
    fn ask(&mut self, m: Value) -> Value {
        self.send(m);
        self.recv().expect("an answer")
    }
}

/// Serve the hook: `commands` to register; `on_call` handles each call and gives its `done`.
fn hook_server(
    secret: &'static str,
    commands: Value,
    mut on_call: impl FnMut(&mut Runner, &str, &str) -> Value + Send + 'static,
) -> (String, std::thread::JoinHandle<()>) {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let value = format!("{}:{secret}", l.local_addr().expect("addr").port());
    let t = std::thread::spawn(move || {
        let (s, _) = l.accept().expect("accept");
        let mut c = Runner {
            r: BufReader::new(s.try_clone().expect("clone")),
            w: s,
        };
        let hello = c.recv().expect("hello");
        assert_eq!(hello["secret"], secret);
        c.send(json!({"commands": commands}));
        while let Some(m) = c.recv() {
            let name = m["name"].as_str().unwrap_or_default().to_string();
            let args = m["args"].as_str().unwrap_or_default().to_string();
            let mut done = on_call(&mut c, &name, &args);
            done["done"] = m["call"].clone();
            c.send(done);
        }
    });
    (value, t)
}

fn with_hook(d: &Path, top: &str, value: &str, args: &[&str]) -> Output {
    yavs()
        .current_dir(d)
        .env(yavs_hook_var(), value)
        .args(["run", "--work=work:lib", top])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("yavs")
}

fn yavs_hook_var() -> &'static str {
    "YAVS_CONSOLE_HOOK"
}

#[test]
fn hook_commands_run_in_the_runner() {
    let d = setup("hook");
    let (value, t) = hook_server(
        "s3cret",
        json!([
            {"name": "greet", "help": "say hello"},
            {"name": "hidden", "help": ""},
            {"name": "boom", "help": "fail"},
            {"name": "drive", "help": "run a bit"},
        ]),
        |c, name, args| match name {
            "greet" => {
                c.send(json!({"print": format!("hello {args}\n")}));
                json!({"result": "false"})
            }
            "boom" => json!({"result": "", "error": "it broke"}),
            "drive" => {
                let a = c.ask(json!({"exec": 1, "line": "run 10 ns"}));
                assert_eq!(a["ok"], true, "{a}");
                let a = c.ask(json!({"exec": 2, "line": "examine cnt"}));
                assert_eq!(a["text"], "4", "{a}");
                let a = c.ask(json!({"exec": 3, "line": "bogus"}));
                assert_eq!(a["ok"], false, "{a}");
                assert!(a["error"].as_str().unwrap_or("").contains("bogus"), "{a}");
                // To the end: the answer comes at the end, and the session is there.
                let a = c.ask(json!({"exec": 4, "line": "run"}));
                assert_eq!(a["ok"], true, "{a}");
                let a = c.ask(json!({"exec": 5, "line": "restart; examine cnt"}));
                assert_eq!(a["text"], "0", "{a}");
                json!({"result": "true"})
            }
            _ => json!({"result": ""}),
        },
    );
    let o = with_hook(
        &d,
        "tb",
        &value,
        &["--do=help; greet you there; where; drive; hidden; where; boom; where"],
    );
    // A misspelled command of the runner is a misspelling here, not advice to start the runner.
    let (value2, t2) = hook_server(
        "y",
        json!([{"name": "vunit_run", "help": "run it"}]),
        |_, _, _| json!({"result": ""}),
    );
    let typo = with_hook(&d, "tb", &value2, &["--do=vunit_rn"]);
    let e = stderr(&typo);
    assert!(
        e.contains("`vunit_run`") && !e.contains("--yavs-interactive"),
        "{e}"
    );
    t2.join().expect("the runner");
    let out = stdout(&o);
    assert!(out.contains("say hello\n"), "{out}");
    assert!(!out.contains("hidden:"), "{out}");
    assert!(out.contains("hello you there\nfalse\n"), "{out}");
    // In the order of the commands, whatever the timing (#947).
    assert!(
        out.find("say hello").unwrap_or(usize::MAX) < out.find("hello you there").unwrap_or(0),
        "{out}"
    );
    assert!(out.contains("** Note: 30000000fs+0: done"), "{out}");
    assert!(out.contains("\ntrue\n"), "{out}");
    assert!(!out.ends_with("between cycles\nbetween cycles\n"), "{out}");
    assert!(stderr(&o).contains("error: it broke"), "{}", stderr(&o));
    assert_eq!(o.status.code(), Some(1));
    t.join().expect("the runner");
}

#[test]
fn hook_reload_sees_new_code_and_keeps_breakpoints() {
    let d = setup("reload");
    std::fs::write(
        d.join("r.vhd"),
        "entity r is\n  generic (g : integer := 1);\nend;\narchitecture a of r is\nbegin\n  process begin\n    report \"one\" & integer'image(g);\n    wait;\n  end process;\nend;\n",
    )
    .expect("write");
    analyze(&d, "r.vhd");
    let dir = d.to_path_buf();
    let (value, t) = hook_server(
        "x",
        json!([{"name": "again", "help": "reload"}]),
        move |c, _, args| {
            match args {
                "edit" => {
                    let new = "entity r is\n  generic (g : integer := 1);\nend;\narchitecture a of r is\nbegin\n  process begin\n    report \"two\" & integer'image(g);\n    wait;\n  end process;\nend;\n";
                    std::fs::write(dir.join("r.vhd"), new).expect("write");
                    analyze(&dir, "r.vhd");
                    let a = c.ask(json!({"reload": 1, "args": []}));
                    assert_eq!(a["ok"], true, "{a}");
                }
                "flags" => {
                    let a = c.ask(json!({"reload": 2, "args": ["-gg=5"]}));
                    assert_eq!(a["ok"], true, "{a}");
                }
                _ => {
                    // A design that does not elaborate: the old one stays loaded.
                    let bad = c.ask(json!({"reload": 3, "args": ["--no-such-flag"]}));
                    assert_eq!(bad["ok"], false, "{bad}");
                    let w = c.ask(json!({"exec": 4, "line": "where"}));
                    assert!(w["text"].as_str().unwrap_or("").contains("delta"), "{w}");
                }
            }
            json!({"result": "false"})
        },
    );
    let o = with_hook(
        &d,
        "r",
        &value,
        &["--do=break r.vhd:7; again bad; again edit; where; run; run; again flags; run; run"],
    );
    let out = stdout(&o);
    assert_eq!(out.matches("two1").count(), 1, "{out}\n{}", stderr(&o));
    assert_eq!(out.matches("two5").count(), 1, "{out}");
    assert!(!out.contains("one"), "{out}");
    let err = stderr(&o);
    assert_eq!(err.matches("breakpoint at").count(), 2, "{err}");
    assert_eq!(o.status.code(), Some(0), "{err}");
    t.join().expect("the runner");
}

#[test]
fn hook_errors_are_e0703_at_the_start() {
    let d = setup("hookerr");
    let bad = |value: &str| with_hook(&d, "tb", value, &["--do=quit"]);
    for v in ["nonsense", "0:x", "1:"] {
        let o = bad(v);
        assert_eq!(o.status.code(), Some(2), "{v}");
        assert!(stderr(&o).contains("E0703"), "{}", stderr(&o));
    }
    // Nobody listens on this port.
    let o = bad(&format!("{}:x", common::NO_LISTENER));
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("E0703"), "{}", stderr(&o));
    assert!(!stderr(&o).contains(":x"), "the secret is not printed");
    // A runner that would take the name of a yavs command.
    let (v, t) = hook_server("x", json!([{"name": "run", "help": ""}]), |_, _, _| {
        json!({})
    });
    let o = with_hook(&d, "tb", &v, &["--do=quit"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("a yavs command"), "{}", stderr(&o));
    t.join().expect("the runner");
}

#[test]
fn hook_going_away_fails_what_waits() {
    let d = setup("hookgone");
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let value = format!("{}:x", l.local_addr().expect("addr").port());
    let t = std::thread::spawn(move || {
        let (s, _) = l.accept().expect("accept");
        let mut c = Runner {
            r: BufReader::new(s.try_clone().expect("clone")),
            w: s,
        };
        c.recv();
        c.send(json!({"commands": [{"name": "die", "help": "d"}]}));
        c.recv();
    });
    let o = with_hook(&d, "tb", &value, &["--do=die"]);
    t.join().expect("the runner");
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).contains("closed its connection"),
        "{}",
        stderr(&o)
    );
}

const SHARED: &str = "package p is
  type counter_t is protected
    procedure incr;
  end protected;
end package;
package body p is
  type counter_t is protected body
    variable n : integer := 0;
    procedure incr is
    begin
      n := n + 1;
    end procedure;
  end protected body;
end package body;
use work.p.all;
package q is
  shared variable in_pkg : counter_t;
  shared variable plain_pkg : integer := 11;
end package;
use work.p.all;
use work.q.all;
entity t is end;
architecture a of t is
  type rec is record x : integer; y : integer; end record;
  shared variable sc : counter_t;
  shared variable sn : integer := 5;
  shared variable sr : rec := (1, 2);
  signal s : bit;
begin
  g : for i in 0 to 1 generate
    shared variable gs : integer := i;
  begin
  end generate;
  process
  begin
    sc.incr;
    sn := 6;
    s <= '1';
    wait for 1 ns;
    sr.y := 9;
    wait;
  end process;
end;
";

fn shared_design(name: &str) -> common::Tmp {
    let d = tempdir(&format!("console-{name}"));
    std::fs::write(d.join("t.vhd"), SHARED).expect("write");
    let ok = yavs()
        .current_dir(&d)
        .args(["analyze", "--relaxed", "--work=work:lib", "t.vhd"])
        .status()
        .expect("yavs")
        .success();
    assert!(ok, "analyze failed");
    d
}

fn shared_script(d: &Path, args: &[&str], cmds: &str) -> String {
    let mut all = vec!["--relaxed".to_string(), format!("--do={cmds}")];
    all.extend(args.iter().map(|a| a.to_string()));
    let all: Vec<&str> = all.iter().map(String::as_str).collect();
    let o = run(d, "t", &all);
    stdout(&o) + &stderr(&o)
}

/// #938, #959: `examine`, `describe` and `find` of shared variables, at time 0 and inside the
/// process that holds the shared state.
#[test]
fn shared_variables_in_the_console() {
    let d = shared_design("shared");
    let o = shared_script(
        &d,
        &[],
        "examine sc; examine sn; examine sr.y; examine pkg_none; quit",
    );
    assert!(o.contains("protected\n5\n2\n"), "{o}");
    assert!(o.contains("no variable or signal `pkg_none`"), "{o}");
    let o = shared_script(
        &d,
        &[],
        "examine in_pkg; examine plain_pkg; examine q.plain_pkg; examine work.q.plain_pkg; \
         examine t.g(1).gs; examine sc.n; describe sc; describe sn; describe plain_pkg; \
         find variables *; quit",
    );
    assert!(o.contains("protected\n11\n11\n11\n1\n0\n"), "{o}");
    assert!(
        o.contains(
            "t.sc\n  kind       protected object\n  type       counter_t\n  declared   t.vhd:25\n  \
             variables  fields n\n  value      protected\n"
        ),
        "{o}"
    );
    assert!(
        o.contains(
            "t.sn\n  kind      shared variable\n  type      INTEGER\n  declared  t.vhd:26\n  \
             bounds    -2147483648 to 2147483647\n  value     5\n"
        ),
        "{o}"
    );
    assert!(
        o.contains("work.q.plain_pkg\n  kind      shared variable"),
        "{o}"
    );
    for row in [
        "t.g(1).gs         shared variable",
        "t.sc              protected object",
        "work.q.in_pkg     protected object",
    ] {
        assert!(o.contains(row), "{row}: {o}");
    }
    // Inside the process, which holds the shared state: a read must not wait for it.
    let o = shared_script(
        &d,
        &[],
        "break t.vhd:38; run; examine sn; examine sc.n; describe sc; force sn 3",
    );
    assert!(o.contains("\n6\n1\nt.sc\n"), "{o}");
    assert!(o.contains("`sn` is a shared variable, not a signal"), "{o}");
    let o = shared_script(&d, &[], "force sn 3");
    assert!(o.contains("`sn` is a shared variable, not a signal"), "{o}");
    // A near name gets a suggestion.
    let o = shared_script(&d, &[], "examine sq");
    assert!(o.contains("`sc`"), "{o}");
}

/// #938, #959: waves name shared variables as the console does; a protected object is not recorded
/// and the console says so.
#[test]
fn shared_variables_in_the_wave_file() {
    let d = shared_design("shared-wave");
    let o = shared_script(
        &d,
        &["--wave=w.fst"],
        "run 2 ns; examine sn @ 1 ns; examine sr.y @ 1 ns; examine t.g(1).gs @ 1 ns; \
         examine plain_pkg @ 1 ns; examine sc @ 1 ns",
    );
    assert!(o.contains("6\n9\n1\n11\n"), "{o}");
    assert!(o.contains("`sc` is a protected object"), "{o}");
    // A stop inside the process that holds the shared state, with a recording.
    let o = shared_script(
        &d,
        &["--wave=w2.fst"],
        "break t.vhd:38; run; examine sn; quit",
    );
    assert!(o.contains("\n6\n"), "{o}");
}

/// #939: on a terminal's session too, a report before an endless loop comes out at once.
#[test]
fn a_report_before_an_endless_loop_comes_out_at_once() {
    let d = tempdir("console-live");
    std::fs::write(
        d.join("t.vhd"),
        "entity t is end;
architecture a of t is
begin
  process
  begin
    report \"before\";
    loop
      null;
    end loop;
  end process;
end;
",
    )
    .expect("write");
    analyze(&d, "t.vhd");
    let mut c = yavs()
        .current_dir(&d)
        .args(["run", "--work=work:lib", "t", "--do=run"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("yavs");
    let pid = c.id();
    // The watchdog: without the live path, nothing comes out and the kill ends the read.
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(30));
        let _ = Command::new(if cfg!(windows) { "taskkill" } else { "kill" })
            .args(if cfg!(windows) {
                &["/F", "/PID"][..]
            } else {
                &[][..]
            })
            .arg(pid.to_string())
            .status();
    });
    let mut r = BufReader::new(c.stdout.take().expect("stdout"));
    let mut line = String::new();
    r.read_line(&mut line).expect("stdout");
    assert!(line.contains("before"), "{line}");
    c.kill().expect("kill");
    c.wait().expect("wait");
}

/// `break list` shows the line a breakpoint is at, and a `break` that fails keeps its number for
/// the next one (#956).
#[test]
fn break_list_shows_the_line_and_failures_keep_numbers() {
    let d = setup("breaknums");
    let mut c = yavs();
    c.current_dir(&d)
        .args(["run", "--work=work:lib", "tb", "--interactive"]);
    let lines =
        "break tb.vhd:18\nbreak nosuch\nbreak tb.vhd:0\nbreak cnt = zz\nbreak cnt\nbreak list\n";
    let out = stdout(&bounded(c, lines));
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        lines[0].starts_with("breakpoint 1: ") && lines[0].ends_with("tb.vhd:19"),
        "{out}"
    );
    assert!(
        lines[1].starts_with("breakpoint 2: ") && lines[1].ends_with("tb.vhd:6"),
        "{out}"
    );
    assert_eq!(lines[2], "breakpoint 3: tb.cnt", "{out}");
    // The numbers have no gaps; the list shows where each one stops.
    assert!(
        lines[3].starts_with("1 ") && lines[3].ends_with("tb.vhd:19 (asked for line 18)"),
        "{out}"
    );
    assert!(
        lines[4].starts_with("2 ") && lines[4].ends_with("tb.vhd:6 (asked for line 0)"),
        "{out}"
    );
    assert_eq!(lines[5], "3 tb.cnt", "{out}");
    assert_eq!(lines.len(), 6, "{out}");
}

/// A condition is checked when the breakpoint is made, with the text and the help of its
/// evaluation (#957); what only a stop knows is said.
#[test]
fn conditions_are_checked_when_made() {
    let d = setup("condcheck");
    let bad = |cmd: &str, want: &str| {
        let o = script(&d, "tb", cmd);
        assert_eq!(o.status.code(), Some(1), "{cmd}");
        assert!(stderr(&o).contains(want), "{cmd}: {}", stderr(&o));
        assert!(
            !stdout(&o).contains("breakpoint 1"),
            "{cmd}: {}",
            stdout(&o)
        );
    };
    bad(
        "break tb.vhd:21 if nosuch = 1",
        "no local or signal `nosuch` here",
    );
    bad(
        "break tb.vhd:21 if cnt = 1 and nosuch",
        "no local or signal `nosuch` here",
    );
    bad(
        "break tb.vhd:21 if ctn = 1",
        "a signal or variable with a similar name exists: `cnt`",
    );
    bad("break tb.vhd:21 if cnt", "the condition is not a boolean");
    bad("break tb.vhd:21 if cnt = '1'", "expected an integer");
    bad("break tb.vhd:21 if cnt = v", "bit_vector");
    bad(
        "break cnt if nosuch = 1",
        "no local or signal `nosuch` here",
    );
    bad("break cnt = 1 if cnt", "the condition is not a boolean");
    bad("run until cnt", "the condition is not a boolean");
    // A signal's condition is read between cycles: the variables of a process are not there.
    bad("break cnt if i > 1", "no local or signal `i` here");
    bad("run until nosuch = 1", "no local or signal `nosuch` here");
    // A variable of the process is known by name, and its type waits for the stop.
    let o = script(
        &d,
        "tb",
        "break tb.vhd:21 if i > 1 and cnt >= 0; run; examine i",
    );
    let out = stdout(&o);
    assert!(
        out.contains("not checked until the condition is evaluated at a stop: `i`"),
        "{out}{}",
        stderr(&o)
    );
    assert!(out.ends_with("2\n"), "{out}");
    // A value that depends on the stop is no error now.
    let o = script(&d, "tb", "break tb.vhd:21 if cnt / cnt > 0");
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
}

/// The wording of subcommand, argument and time errors (#958).
#[test]
fn subcommand_argument_and_time_errors_say_what_is_needed() {
    let d = setup("wording");
    let err = |cmd: &str| {
        let o = script(&d, "tb", cmd);
        assert_eq!(o.status.code(), Some(1), "{cmd}");
        stderr(&o)
    };
    assert!(err("trace").contains("error: `trace` needs a subcommand: `x`\n"));
    assert!(err("trace y").contains("error: `trace y`: expected `x`\n"));
    assert!(err("find").contains("error: `find` needs a subcommand: `signals` or `variables`\n"));
    assert!(err("find signals").contains("error: `find signals` needs a pattern"));
    assert!(err("drivers").contains("error: `drivers` needs a signal name\n"));
    assert!(err("run 10").contains("error: a time needs a unit: `10 ns`\n"));
    assert!(err("examine cnt @ 12").contains("error: a time needs a unit: `12 ns`\n"));
    assert!(err("run 0.5 fs").contains("is not a whole number of fs: the resolution is 1 fs"));
    // A fraction that is whole in the primary unit is fine.
    let o = script(&d, "tb", "run 0.5 ns; where");
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let d = setup("wording2");
    let o = script(&d, "vals", "force t 5; ");
    assert!(
        stderr(&o).contains("type TIME: expected a number and a unit"),
        "{}",
        stderr(&o)
    );
}

/// A time shows in the largest unit that holds it whole, in `examine`, `force` and `find` (#958).
#[test]
fn times_print_in_real_units() {
    let d = setup("timeunits");
    let o = script(
        &d,
        "vals",
        "force t 10 ns; examine t; deposit t 1500 ps; find signals t; deposit t 7 fs; examine t",
    );
    let out = stdout(&o);
    assert!(out.contains("vals.t = 10 ns\n"), "{out}{}", stderr(&o));
    assert!(out.contains("vals.t = 1500 ps\n"), "{out}");
    assert!(!out.contains("10000000"), "{out}");
}

/// `deposit` has no `-out`; a flag in the middle is named in the error of the value (#960).
#[test]
fn flags_that_have_no_meaning_are_errors() {
    let d = setup("flags");
    let err = |cmd: &str| {
        let o = script(&d, "tb", cmd);
        assert_eq!(o.status.code(), Some(1), "{cmd}");
        stderr(&o)
    };
    assert!(
        err("deposit cnt 1 -out").contains(
            "error: `deposit` has no `-out`: a deposit gives a value once, to the signal\n"
        )
    );
    let e = err("force cnt -out 1");
    assert!(e.contains("a flag goes last, after the value"), "{e}");
    let e = err("force cnt 1 -ou");
    assert!(e.contains("`force` has only `-out`, not `-ou`"), "{e}");
    assert!(e.contains("similar name exists: `-out`"), "{e}");
    assert!(err("release cnt 1").contains("`release` takes no value"));
    // The flag that has a meaning still works, and a negative number is a value.
    let o = script(&d, "tb", "force cnt 1 -out; force cnt -3; release cnt -out");
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
}

/// A generic is a name in an expression: between cycles and in a process (#970).
#[test]
fn generics_are_names() {
    let d = setup("generics");
    let o = script(&d, "tb", "examine N; examine NAME; examine N + 1 > 3");
    assert_eq!(stdout(&o), "3\n\"dap\"\nTRUE\n", "{}", stderr(&o));
    let o = script(&d, "tb", "run until N = 3 and cnt = 4; examine cnt");
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert!(stdout(&o).ends_with("4\n"), "{}{}", stdout(&o), stderr(&o));
    let o = script(&d, "tb", "break tb.vhd:21 if k = N; run; examine k");
    assert!(stdout(&o).ends_with("3\n"), "{}{}", stdout(&o), stderr(&o));
    let o = script(&d, "tb", "break tb.vhd:21 if cnt = M");
    assert!(
        stderr(&o).contains("no local or signal `M` here"),
        "{}",
        stderr(&o)
    );
}

/// A wrong value says what was typed, what the target is and an example (#972).
#[test]
fn literal_errors_are_in_plain_words() {
    let d = setup("literals");
    let err = |top: &str, cmd: &str| {
        let o = script(&d, top, cmd);
        assert_eq!(o.status.code(), Some(1), "{cmd}");
        stderr(&o)
    };
    let e = err("tb", "force cnt abc");
    assert!(
        e.contains("`abc` is not a value of `tb.cnt`, type INTEGER: expected an integer"),
        "{e}"
    );
    let e = err("tb", "force v 7");
    assert!(e.contains("expected 4 elements, such as 0101"), "{e}");
    let e = err("vals", "force d(0) 7");
    assert!(
        e.contains("a part of type STD_LOGIC_VECTOR: expected one of 'U'"),
        "{e}"
    );
    let e = err("vals", "force t 5 xs");
    assert!(
        e.contains("expected a number and a unit, such as 10 fs: similar names exist"),
        "{e}"
    );
    // A condition names what was compared (#972).
    let e = err("tb", "break tb.vhd:21 if cnt = '1'");
    assert!(
        e.contains("`'1'` is not a value of the other operand's type: expected an integer"),
        "{e}"
    );
}

const IN_PORT: &str = "
library ieee;
use ieee.std_logic_1164.all;
entity dut is port (wr : in std_ulogic); end;
architecture a of dut is begin end;
library ieee;
use ieee.std_logic_1164.all;
entity ip is end;
architecture a of ip is
  signal wr : std_ulogic := '0';
begin
  u : entity work.dut port map (wr => wr);
  process begin wait for 5 ns; wr <= '1'; wait; end process;
end;
";

/// A port with no driver of its own shows no driving value; `where` names the process with dots
/// (#974).
#[test]
fn an_in_port_has_no_driving_value() {
    let d = setup("inport");
    std::fs::write(d.join("ip.vhd"), IN_PORT).expect("write");
    let ok = yavs()
        .current_dir(&*d)
        .args(["analyze", "--std=2008", "--work=work:lib", "ip.vhd"])
        .status()
        .expect("yavs")
        .success();
    assert!(ok, "analyze failed");
    let o = script(&d, "ip", "run 10 ns; drivers ip.u.wr");
    let out = stdout(&o);
    assert!(out.contains("no driver of its own"), "{out}{}", stderr(&o));
    assert!(!out.contains("driving value"), "{out}");
    let d = setup("where");
    let o = script(&d, "tb", "break tb.vhd:21; run; where");
    let out = stdout(&o);
    assert!(out.contains("\ntb.p, "), "{out}{}", stderr(&o));
}

/// A shared variable is a name in a condition: known when the breakpoint is made, read at the stop.
#[test]
fn conditions_read_shared_variables() {
    let d = shared_design("shared-cond");
    let o = shared_script(
        &d,
        &[],
        "break t.vhd:40 if sn = 6 and sr.y = 2; run; examine sn; quit",
    );
    assert!(
        o.contains("not checked until the condition is evaluated at a stop"),
        "{o}"
    );
    assert!(o.contains("t.vhd:40"), "{o}");
    assert!(o.contains("\n6\n"), "{o}");
}

/// A protected object's state lists the body's variables, not the bounds it keeps for
/// `variable q : integer range 0 to k` (#1176), and `v : bit_vector(0 to k)`, which it holds by
/// handle, by its value (#1180).
#[test]
fn protected_state_hides_kept_ranges() {
    let d = tempdir_with(
        "protrange",
        "entity t is end;
architecture a of t is
  type bounded_t is protected
    procedure bump;
  end protected;
  type bounded_t is protected body
    variable k : integer := 5;
    variable q : integer range 0 to k := 1;
    variable v : bit_vector(0 to k) := \"101100\";
    procedure bump is
    begin
      q := q + 1;
    end procedure;
  end protected body;
  shared variable sv : bounded_t;
begin
  process
  begin
    sv.bump;
    wait;
  end process;
end;
",
    );
    let o = script(&d, "t", "break t.vhd:12; run; describe sv; examine sv.v");
    let out = stdout(&o) + &stderr(&o);
    assert!(out.contains("fields k, q, v\n"), "{out}");
    assert!(out.contains("\"101100\""), "{out}");
    assert!(!out.contains("'left") && !out.contains("'right"), "{out}");
}
