//! `yavs run --dap` (docs/cli-contract.md 5): scripted Debug Adapter Protocol sessions.
//!
//! Each case in `tests/dap` is a script, `NAME.script`, and the expected transcript,
//! `NAME.expected`. A script line is `request COMMAND {ARGS}` (send, then read up to its
//! response) or `event NAME` (read up to that event); `$SRC` is the absolute path of
//! `tb.vhd`. The transcript has `> ` for what was sent and `< ` for what was read, without
//! `seq`, with the scratch directory as `$TMP`. `YAVS_BLESS=1` rewrites the expected files.

mod common;
use common::tempdir;

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

fn yavs() -> Command {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
}

fn cases() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/dap")
}

/// A scratch directory with `tb.vhd` analyzed into `lib`.
fn setup(name: &str) -> common::Tmp {
    let d = tempdir(&format!("dap-{name}"));
    std::fs::copy(cases().join("tb.vhd"), d.join("tb.vhd")).expect("copy");
    let ok = yavs()
        .current_dir(&d)
        .args(["analyze", "--work=work:lib"])
        .arg(d.join("tb.vhd"))
        .status()
        .expect("yavs")
        .success();
    assert!(ok, "analyze failed");
    d
}

fn spawn(dir: &Path, top: &str, extra: &[&str]) -> Child {
    yavs()
        .current_dir(dir)
        .args(["run", "--work=work:lib", top])
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("yavs")
}

/// Kills the process `pid` unless dropped within `secs`: a session that is stuck fails its test
/// (end of input on its stdout) instead of hanging CI (#441).
struct Watchdog(#[allow(dead_code)] std::sync::mpsc::Sender<()>); // dropped to disarm

impl Watchdog {
    fn new(pid: u32, secs: u64) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            if rx.recv_timeout(std::time::Duration::from_secs(secs))
                == Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            {
                let (tool, args): (_, &[&str]) = if cfg!(windows) {
                    ("taskkill", &["/F", "/PID"])
                } else {
                    ("kill", &[])
                };
                let _ = Command::new(tool).args(args).arg(pid.to_string()).status();
            }
        });
        Self(tx)
    }
}

fn read(r: &mut impl BufRead) -> Option<Value> {
    let mut len = None;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line).expect("read") == 0 {
            return None;
        }
        let line = line.trim();
        if line.is_empty() && len.is_some() {
            break;
        }
        if let Some(v) = line.strip_prefix("Content-Length:") {
            len = Some(v.trim().parse::<usize>().expect("length"));
        }
    }
    let mut body = vec![0; len?];
    r.read_exact(&mut body).expect("body");
    Some(serde_json::from_slice(&body).expect("json"))
}

/// A client: sends requests, keeps a transcript of everything.
struct Client<W: Write, R: BufRead> {
    w: W,
    r: R,
    seq: i64,
    log: Vec<String>,
}

impl<W: Write, R: BufRead> Client<W, R> {
    fn send(&mut self, command: &str, args: Value) -> i64 {
        self.seq += 1;
        let m = json!({"seq": self.seq, "type": "request", "command": command, "arguments": args});
        self.log.push(format!("> {command} {args}"));
        let b = m.to_string();
        write!(self.w, "Content-Length: {}\r\n\r\n{b}", b.len()).expect("write");
        self.w.flush().expect("flush");
        self.seq
    }

    /// Read until `stop` holds; panics at the end of input.
    fn until(&mut self, stop: impl Fn(&Value) -> bool) -> Value {
        loop {
            let mut m = read(&mut self.r)
                .unwrap_or_else(|| panic!("end of input; so far:\n{}", self.log.join("\n")));
            if let Some(o) = m.as_object_mut() {
                o.remove("seq");
            }
            self.log.push(format!("< {m}"));
            if stop(&m) {
                return m;
            }
        }
    }

    fn request(&mut self, command: &str, args: Value) -> Value {
        let s = self.send(command, args);
        self.until(|m| m["type"] == "response" && m["request_seq"] == s)
    }

    fn event(&mut self, name: &str) -> Value {
        self.until(|m| m["event"] == name)
    }
}

fn stdio_client(c: &mut Child) -> Client<std::process::ChildStdin, BufReader<ChildStdout>> {
    Client {
        w: c.stdin.take().expect("stdin"),
        r: BufReader::new(c.stdout.take().expect("stdout")),
        seq: 0,
        log: Vec::new(),
    }
}

/// The transcript with the scratch directory as `$TMP` and `/` separators.
fn normalize(log: &[String], dir: &Path) -> String {
    let d = dir.display().to_string();
    let escaped = serde_json::to_string(&d).expect("json");
    let escaped = &escaped[1..escaped.len() - 1];
    log.iter()
        .map(|l| {
            l.replace(escaped, "$TMP")
                .replace(&d, "$TMP")
                .replace("\\\\", "/")
                + "\n"
        })
        .collect()
}

fn script_case(name: &str, extra: &[&str]) {
    script_case_within(name, extra, 120);
}

/// [`script_case`] with a session that is killed after `secs`.
fn script_case_within(name: &str, extra: &[&str], secs: u64) {
    let dir = setup(&format!("{name}{}", extra.len()));
    let src = dir.join("tb.vhd").display().to_string();
    let script = std::fs::read_to_string(cases().join(format!("{name}.script"))).expect("script");
    // An optional first line `top NAME` picks another top level than `tb`.
    let (top, script) = match script.strip_prefix("top ") {
        Some(rest) => rest.split_once('\n').expect("script after `top`"),
        None => ("tb", script.as_str()),
    };
    let mut child = spawn(&dir, top.trim(), &[&["--dap=stdio"], extra].concat());
    // A script that waits for an event that never comes fails with its transcript (#971).
    let _watchdog = Watchdog::new(child.id(), secs);
    let mut c = stdio_client(&mut child);
    let esc = serde_json::to_string(&src).expect("json");
    let esc = &esc[1..esc.len() - 1];
    for line in script.lines().filter(|l| !l.trim().is_empty()) {
        let line = line.replace("$SRC", esc);
        match line.split_once(' ') {
            Some(("request", rest)) => {
                let (cmd, args) = rest.split_once(' ').unwrap_or((rest, "{}"));
                let args: Value = serde_json::from_str(args).expect("script args");
                c.request(cmd, args);
            }
            Some(("event", e)) => {
                c.event(e.trim());
            }
            _ => panic!("bad script line `{line}`"),
        }
    }
    drop(c.w);
    let status = child.wait().expect("wait");
    c.log.push(format!("exit {}", status.code().unwrap_or(-1)));
    let got = normalize(&c.log, &dir);
    let path = cases().join(format!("{name}.expected"));
    if std::env::var_os("YAVS_BLESS").is_some() && extra.is_empty() {
        std::fs::write(&path, &got).expect("bless");
    } else {
        let want = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(
            got, want,
            "transcript of {name} differs (YAVS_BLESS=1 to update)"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A design with no process runs under a client: the session starts and ends (#995).
#[test]
fn design_without_a_process() {
    let dir = tempdir("dap-noproc");
    std::fs::write(
        dir.join("t.vhd"),
        "entity t is end; architecture a of t is signal s : bit; begin end;",
    )
    .expect("write");
    let ok = yavs()
        .current_dir(&dir)
        .args(["analyze", "--work=work:lib", "t.vhd"])
        .status()
        .expect("yavs")
        .success();
    assert!(ok, "analyze failed");
    let mut child = spawn(&dir, "t", &["--dap=stdio"]);
    let _watchdog = Watchdog::new(child.id(), 60);
    let mut c = stdio_client(&mut child);
    c.request("initialize", json!({"adapterID": "yavs"}));
    c.request("launch", json!({}));
    c.request("configurationDone", json!({}));
    c.event("terminated");
    drop(c.w);
    let status = child.wait().expect("wait");
    assert_eq!(status.code(), Some(0));
}

/// A script that waits for an event that never comes fails with its transcript, not by hanging
/// (#971): `hang.script` stops on entry and then waits for a second stop.
#[test]
#[should_panic(expected = "end of input; so far")]
fn a_script_waiting_for_a_missing_event_fails() {
    script_case_within("hang", &[], 5);
}

/// Line breakpoints, stack, scopes, variables, evaluate, step over and out, a waiting
/// process's frame, clearing breakpoints and the end of the run. The same with
/// `--threads=4`: the debugger runs processes on one thread, so the session is identical.
#[test]
fn breakpoints_and_stepping() {
    script_case("breakpoints", &[]);
    script_case("breakpoints", &["--threads=4"]);
}

/// Stop on entry, step in across a call, hit counts, a line without statements, an unknown
/// name, and disconnect while stopped.
#[test]
fn entry_hits_and_disconnect() {
    script_case("entry", &[]);
}

/// Run for a time and until a time (from a stop inside a process and between cycles), a line
/// breakpoint on the way, next delta and next time step, bad times. Identical at
/// `--threads=4`.
#[test]
fn run_to_time_and_steps() {
    script_case("runto", &[]);
    script_case("runto", &["--threads=4"]);
}

/// A timed wait (`wait on s until s = 99 for 45 ns`) that three events wake with its condition
/// false: a run that stops at 25 ns inside it and continues reaches the breakpoint after it at
/// 45 ns, and stepping over the next timed wait stops after it at 55 ns. Identical at
/// `--threads=4`.
#[test]
fn timed_wait_paused_and_stepped() {
    script_case("timed_wait", &[]);
    script_case("timed_wait", &["--threads=4"]);
}

/// The console's commands in `evaluate` with context `repl`, a run from there and a restart.
#[test]
fn console_commands_in_evaluate() {
    script_case("console", &[]);
}

/// Signal breakpoints: data breakpoint info for a signal, a local and unknown names, stops on
/// changes (`setDataBreakpoints`, one with a condition that never holds) and on values
/// (`yavs/setSignalBreakpoints`), rejected values. Identical at `--threads=4`.
#[test]
fn signal_breakpoints() {
    script_case("signals", &[]);
    script_case("signals", &["--threads=4"]);
}

/// Force, deposit and release signals (whole, from a stop inside a process and between
/// cycles, in and out), `setVariable` on composite and scalar locals and on signals, and
/// rejected values, names and modes. Identical at `--threads=4`.
#[test]
fn force_deposit_release() {
    script_case("values", &[]);
    script_case("values", &["--threads=4"]);
}

/// Force, deposit and release of signal parts (#364): a slice, an element, a field and an
/// element of a field, only those elements changing, and rejected selections.
#[test]
fn signal_parts() {
    script_case("parts", &[]);
}

/// Breakpoint conditions: on line breakpoints (held, not held inside a function stepped over,
/// an error in one, with a hit count), on data breakpoints (names seen from the signal's
/// region, with a hit count), and unparsable or unsupported ones. Identical at `--threads=4`.
#[test]
fn conditions() {
    script_case("conditions", &[]);
    script_case("conditions", &["--threads=4"]);
}

/// Composite variables (records by field, arrays by index, paged), setting a field or an
/// element of a local and of a signal, the Hierarchy scope and `yavs/hierarchy`.
#[test]
fn composites_and_hierarchy() {
    script_case("composite", &[]);
}

/// Live waves: subscriptions replaced before the run, batches per time step and before stops,
/// unsubscribe and a new subscription at a stop (recorded from there on), with `--wave`
/// recording at the same time. Composite signals and variables in one batch, a selection of
/// nothing, an array over the cap. Identical at `--threads=4`.
#[test]
fn live_waves() {
    script_case("waves", &[]);
    script_case("waves", &["--threads=4", "--wave=waves.fst"]);
    script_case("waves_composite", &[]);
}

/// Pause a simulation that would run forever, then read a signal and disconnect. The response
/// to `pause` comes before the stop, which may follow it at once (#441).
#[test]
fn pause() {
    let dir = setup("pause");
    let mut child = spawn(&dir, "spin", &["--dap=stdio", "--threads=4"]);
    let _watchdog = Watchdog::new(child.id(), 120);
    let mut c = stdio_client(&mut child);
    c.request("initialize", json!({"adapterID": "yavs"}));
    c.request("launch", json!({}));
    c.request("configurationDone", json!({}));
    c.request("pause", json!({"threadId": 1}));
    let stop = c.event("stopped");
    assert_eq!(stop["body"]["reason"], "pause", "{stop}");
    let v = c.request("evaluate", json!({"expression": ":spin:cnt"}));
    let n = v["body"]["result"].as_str().map(str::parse::<u64>);
    assert!(matches!(n, Some(Ok(_))), "{v}");
    let t = c.request("stackTrace", json!({"threadId": 1}));
    // At the process statement before it first runs, in the loop, or (paused between
    // cycles) at its wait.
    let line = t["body"]["stackFrames"][0]["line"].as_i64().unwrap_or(0);
    assert!((41..=45).contains(&line), "{t}");
    c.request("disconnect", json!({}));
    drop(c.w);
    assert_eq!(child.wait().expect("wait").code(), Some(1));
    let mut err = String::new();
    child
        .stderr
        .take()
        .expect("stderr")
        .read_to_string(&mut err)
        .expect("stderr");
    assert!(err.contains("W0109"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A scratch directory with `src` analyzed into `lib`, with `flags`.
fn design(name: &str, src: &str, flags: &[&str]) -> common::Tmp {
    let d = tempdir(&format!("dap-{name}"));
    std::fs::write(d.join("t.vhd"), src).expect("write");
    let ok = yavs()
        .current_dir(&d)
        .args(["analyze", "--work=work:lib"])
        .args(flags)
        .arg("t.vhd")
        .status()
        .expect("yavs")
        .success();
    assert!(ok, "analyze failed");
    d
}

/// #939: what a process says before it loops for ever reaches the client while it runs, with the
/// text a report has always had.
#[test]
fn a_report_before_an_endless_loop_arrives_at_once() {
    let d = design(
        "live",
        "use std.textio.all;
entity t is end;
architecture a of t is
  procedure spin is
  begin
    loop
      null;
    end loop;
  end procedure;
begin
  named : process
    variable l : line;
  begin
    report \"before\";
    write(l, string'(\"text\"));
    writeline(output, l);
    spin;
    wait;
  end process;
end;
",
        &[],
    );
    let mut child = spawn(&d, "t", &["--dap=stdio"]);
    // Without the live path nothing arrives, and this ends the session and the test.
    let _w = Watchdog::new(child.id(), 60);
    let mut c = stdio_client(&mut child);
    c.request("initialize", json!({"adapterID": "yavs"}));
    c.request("launch", json!({}));
    c.request("configurationDone", json!({}));
    let out = c.event("output");
    assert_eq!(
        out["body"]["output"], "** Note: 0fs+0: before\n    Process t.named\n",
        "{out}"
    );
    let out = c.event("output");
    assert_eq!(out["body"]["output"], "text\n", "{out}");
    c.request("pause", json!({"threadId": 1}));
    assert_eq!(c.event("stopped")["body"]["reason"], "pause");
    c.request("disconnect", json!({}));
    drop(c.w);
    child.wait().expect("wait");
}

/// #942: a process without a label has a name a user recognises in Threads and the Call Stack, and
/// keeps its path everywhere a name is looked up or a report says where it comes from.
#[test]
fn threads_without_a_label_are_named_by_what_they_are() {
    let d = design(
        "threads",
        "entity t is end;
architecture a of t is
  signal clk : bit := '0';
  signal v : bit_vector(1 downto 0);
  signal other : bit;
  procedure pr(signal s : in bit) is
  begin
    null;
  end procedure;
begin
  clk <= not clk after 5 ns when now < 10 ns;
  v(1) <= clk;
  labelled : other <= clk;
  pr(clk);
  assert clk = '0' or clk = '1';
  process
  begin
    report \"here\";
    wait;
  end process;
  g : for i in 0 to 0 generate
    v(0) <= clk;
  end generate;
end;
",
        &[],
    );
    let path = std::path::absolute(d.join("t.vhd")).expect("path");
    let mut child = spawn(&d, "t", &["--dap=stdio"]);
    let _w = Watchdog::new(child.id(), 60);
    let mut c = stdio_client(&mut child);
    c.request("initialize", json!({"adapterID": "yavs"}));
    c.request("launch", json!({}));
    c.request(
        "setBreakpoints",
        json!({"source": {"path": path}, "breakpoints": [{"line": 18}]}),
    );
    c.request("configurationDone", json!({}));
    let stop = c.event("stopped");
    let threads = c.request("threads", json!({}));
    let names: Vec<&str> = threads["body"]["threads"]
        .as_array()
        .expect("threads")
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert_eq!(
        names,
        [
            "t.clk <= ... (t.vhd:11)",
            "t.v(1) <= ... (t.vhd:12)",
            "t.labelled",
            "t.pr(...) (t.vhd:14)",
            "t.assert (t.vhd:15)",
            "t.process (t.vhd:16)",
            "t.g(0).v(0) <= ... (t.vhd:22)",
        ],
        "{threads}"
    );
    // The frames of the stopped process and of one that waits.
    let tid = stop["body"]["threadId"].as_i64().expect("thread");
    let st = c.request("stackTrace", json!({"threadId": tid}));
    assert_eq!(
        st["body"]["stackFrames"][0]["name"], "t.process (t.vhd:16)",
        "{st}"
    );
    // The console's tables and a report say the same; a lookup keeps the path.
    let repl = |e: &str| json!({"expression": e, "context": "repl"});
    let waits = c.request("evaluate", repl("waits"));
    assert!(
        waits["body"]["result"]
            .as_str()
            .unwrap_or("")
            .contains("t.process (t.vhd:16)"),
        "{waits}"
    );
    let h = c.request("yavs/hierarchy", json!({}));
    assert!(h["body"].to_string().contains("t._p4"), "{h}");
    let c2 = c.send("continue", json!({"threadId": tid}));
    let out =
        c.until(|m| m["event"] == "output" && m["body"]["output"].to_string().contains("here"));
    assert!(
        out["body"]["output"]
            .as_str()
            .unwrap_or("")
            .contains("Process t.process (t.vhd:16)"),
        "{out}"
    );
    c.until(|m| m["type"] == "response" && m["request_seq"] == c2 || m["event"] == "exited");
    c.request("disconnect", json!({}));
    drop(c.w);
    child.wait().expect("wait");
}

/// A client over TCP on the port of `yavs run --dap=0` with `extra`, initialized.
fn attach_tcp(
    child: &mut Child,
) -> (
    Client<std::net::TcpStream, BufReader<std::net::TcpStream>>,
    Value,
) {
    let mut err = BufReader::new(child.stderr.take().expect("stderr"));
    let mut line = String::new();
    while !line.contains("DAP listening on") {
        line.clear();
        err.read_line(&mut line).expect("stderr");
    }
    let port = line.trim().rsplit(':').next().expect("port").to_string();
    std::thread::spawn(move || std::io::copy(&mut err, &mut std::io::sink()));
    let s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
    let mut c = Client {
        w: s.try_clone().expect("clone"),
        r: BufReader::new(s),
        seq: 0,
        log: Vec::new(),
    };
    let init = c.request("initialize", json!({"adapterID": "yavs"}));
    (c, init)
}

const RESTART: &str = "entity t is end;
architecture a of t is
  signal n : integer := 0;
begin
  process
  begin
    n <= n + 1;
    wait for 1 ns;
    n <= n + 1;
    wait for 1 ns;
    report \"done\";
    wait;
  end process;
end;
";

/// #963: over TCP a client restarts the run in place (nothing else could start it again) and it
/// runs to the same breakpoint with the state of time 0, the breakpoints kept; while it runs too.
/// A session a client launched stays the client's to start again (it analyzes first).
#[test]
fn an_attached_client_restarts_the_run() {
    let d = design("restart", RESTART, &[]);
    let path = std::path::absolute(d.join("t.vhd")).expect("path");
    let mut child = spawn(&d, "t", &["--dap=0"]);
    let _w = Watchdog::new(child.id(), 60);
    let (mut c, init) = attach_tcp(&mut child);
    assert_eq!(init["body"]["supportsRestartRequest"], true, "{init}");
    c.request("attach", json!({}));
    c.request(
        "setBreakpoints",
        json!({"source": {"path": path}, "breakpoints": [{"line": 9}]}),
    );
    c.request("configurationDone", json!({}));
    assert_eq!(c.event("stopped")["body"]["reason"], "breakpoint");
    let n = c.request("evaluate", json!({"expression": ":t:n"}));
    assert_eq!(n["body"]["result"], "1", "{n}");
    let r = c.request("restart", json!({"arguments": {}}));
    assert_eq!(r["success"], true, "{r}");
    let stop = c.event("stopped");
    assert_eq!(stop["body"]["reason"], "breakpoint", "{stop}");
    assert!(
        stop["body"]["description"]
            .as_str()
            .unwrap_or("")
            .contains("at 1 ns"),
        "{stop}"
    );
    // Asking for a stop at entry, then a restart while it runs.
    let r = c.request("restart", json!({"arguments": {"stopOnEntry": true}}));
    assert_eq!(r["success"], true, "{r}");
    let stop = c.event("stopped");
    assert_eq!(stop["body"]["reason"], "entry", "{stop}");
    c.request(
        "setBreakpoints",
        json!({"source": {"path": path}, "breakpoints": []}),
    );
    c.request("disconnect", json!({}));
    drop(c.w);
    child.wait().expect("wait");

    // Launched: no restart announced.
    let mut child = spawn(&d, "t", &["--dap=stdio"]);
    let _w = Watchdog::new(child.id(), 60);
    let mut c = stdio_client(&mut child);
    let init = c.request("initialize", json!({"adapterID": "yavs"}));
    assert!(
        init["body"].get("supportsRestartRequest").is_none(),
        "{init}"
    );
    c.request("disconnect", json!({}));
    drop(c.w);
    child.wait().expect("wait");
    let _ = std::fs::remove_dir_all(&d);
}

/// #963: a run with a loaded library cannot restart: the client gets a notice naming what to do
/// and an ordinary answer (VS Code takes a failed `restart` for the end of the session), and the
/// stop goes on.
#[cfg(target_os = "linux")]
#[test]
fn a_refused_restart_says_why_and_the_session_stays() {
    let d = design("restart-no", RESTART, &[]);
    let path = std::path::absolute(d.join("t.vhd")).expect("path");
    let mut child = spawn(&d, "t", &["--load=libm.so.6", "--dap=0"]);
    let _w = Watchdog::new(child.id(), 60);
    let (mut c, _) = attach_tcp(&mut child);
    c.request("attach", json!({}));
    c.request(
        "setBreakpoints",
        json!({"source": {"path": path}, "breakpoints": [{"line": 9}]}),
    );
    c.request("configurationDone", json!({}));
    let stop = c.event("stopped");
    let tid = stop["body"]["threadId"].as_i64().expect("thread");
    c.send("restart", json!({"arguments": {}}));
    let out = c.event("output");
    assert_eq!(out["body"]["category"], "important", "{out}");
    assert!(
        out["body"]["output"]
            .to_string()
            .contains("start the run again"),
        "{out}"
    );
    let st = c.request("stackTrace", json!({"threadId": tid}));
    assert_eq!(st["body"]["stackFrames"][0]["line"], 9, "{st}");
    c.request("disconnect", json!({}));
    drop(c.w);
    child.wait().expect("wait");
    let _ = std::fs::remove_dir_all(&d);
}

/// #963: a restart while the run is in a process that never suspends.
#[test]
fn an_attached_client_restarts_a_running_process() {
    let d = design(
        "restart-run",
        "entity t is end;
architecture a of t is
begin
  process
  begin
    report \"again\";
    loop
      null;
    end loop;
  end process;
end;
",
        &[],
    );
    let mut child = spawn(&d, "t", &["--dap=0"]);
    let _w = Watchdog::new(child.id(), 60);
    let (mut c, _) = attach_tcp(&mut child);
    c.request("attach", json!({}));
    c.request("configurationDone", json!({}));
    c.event("output");
    let r = c.request("restart", json!({"arguments": {}}));
    assert_eq!(r["success"], true, "{r}");
    let out = c.event("output");
    assert!(out["body"]["output"].to_string().contains("again"), "{out}");
    c.request("disconnect", json!({}));
    drop(c.w);
    child.wait().expect("wait");
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
begin
  uses : process
  begin
    sc.incr;
    in_pkg.incr;
    sn := 6;
    wait for 1 ns;
    wait;
  end process;
  other : process
    variable z : integer := 0;
  begin
    z := 1;
    wait for 2 ns;
    wait;
  end process;
end;
";

/// #938, #959: shared variables at every surface of a client, at a stop in a process that holds
/// the shared state, in one that does not and between cycles.
#[test]
fn shared_variables_in_a_client() {
    let d = design("shared", SHARED, &["--relaxed"]);
    let path = std::path::absolute(d.join("t.vhd")).expect("path");
    let mut child = spawn(&d, "t", &["--relaxed", "--dap=stdio"]);
    let _w = Watchdog::new(child.id(), 60);
    let mut c = stdio_client(&mut child);
    c.request("initialize", json!({"adapterID": "yavs"}));
    c.request("launch", json!({}));
    // In `uses` (holds the shared state), in `other` (does not).
    c.request(
        "setBreakpoints",
        json!({"source": {"path": path}, "breakpoints": [{"line": 34}, {"line": 41}]}),
    );
    c.request("configurationDone", json!({}));
    for (line, count) in [(34, "1"), (41, "1")] {
        let stop = c.event("stopped");
        let tid = stop["body"]["threadId"].as_i64().expect("thread");
        let st = c.request("stackTrace", json!({"threadId": tid}));
        assert_eq!(st["body"]["stackFrames"][0]["line"], line, "{st}");
        let frame = st["body"]["stackFrames"][0]["id"].as_i64().expect("frame");
        let eval = |e: &str, ctx: &str| json!({"expression": e, "frameId": frame, "context": ctx});
        // Watch and hover.
        for ctx in ["watch", "hover"] {
            let v = c.request("evaluate", eval("sc", ctx));
            assert_eq!(v["body"]["result"], "protected", "{v}");
            assert_eq!(v["body"]["type"], "counter_t", "{v}");
            assert_eq!(v["body"]["evaluateName"], "t.sc", "{v}");
        }
        let v = c.request("evaluate", eval("sc.n", "watch"));
        assert_eq!(v["body"]["result"], count, "{v}");
        let v = c.request("evaluate", eval("in_pkg.n", "hover"));
        assert_eq!(v["body"]["result"], count, "{v}");
        let v = c.request("evaluate", eval("work.q.plain_pkg", "watch"));
        assert_eq!(v["body"]["result"], "11", "{v}");
        let v = c.request("evaluate", eval("sn", "watch"));
        assert_eq!(v["body"]["result"], "6", "{v}");
        // A record's children are named so that a Watch can take them.
        let v = c.request("evaluate", eval("sr", "watch"));
        let r = c.request(
            "variables",
            json!({"variablesReference": v["body"]["variablesReference"]}),
        );
        assert_eq!(r["body"]["variables"][1]["evaluateName"], "t.sr.y", "{r}");
        // The Variables view.
        let scopes = c.request("scopes", json!({"frameId": frame}));
        let list = scopes["body"]["scopes"].as_array().expect("scopes");
        let scope = list
            .iter()
            .find(|s| s["name"] == "Shared Variables")
            .unwrap_or_else(|| panic!("{scopes}"));
        let r = c.request(
            "variables",
            json!({"variablesReference": scope["variablesReference"]}),
        );
        let vars = r["body"]["variables"].as_array().expect("vars");
        let shown: Vec<(&str, &str)> = vars
            .iter()
            .filter_map(|v| Some((v["name"].as_str()?, v["value"].as_str()?)))
            .collect();
        assert_eq!(
            shown,
            [
                ("sc", "protected"),
                ("sn", "6"),
                ("sr", "(x => 1, y => 2)"),
                ("in_pkg", "protected"),
                ("plain_pkg", "11"),
            ],
            "{r}"
        );
        // A protected object expands to its variables.
        let sc = c.request(
            "variables",
            json!({"variablesReference": vars[0]["variablesReference"]}),
        );
        assert_eq!(sc["body"]["variables"][0]["name"], "n", "{sc}");
        assert_eq!(sc["body"]["variables"][0]["value"], count, "{sc}");
        // Shown, not written; and not a signal.
        let w = c.request(
            "setVariable",
            json!({"variablesReference": scope["variablesReference"], "name": "sn", "value": "9"}),
        );
        assert_eq!(w["success"], false, "{w}");
        assert!(
            w["message"].to_string().contains("shown, not written"),
            "{w}"
        );
        let f = c.request(
            "yavs/force",
            json!({"signal": "sn", "value": "9", "frameId": frame}),
        );
        assert!(
            f["message"].to_string().contains("is a shared variable"),
            "{f}"
        );
        // The hierarchy lists a region's.
        let h = list
            .iter()
            .find(|s| s["name"] == "Hierarchy")
            .expect("Hierarchy");
        let roots = c.request(
            "variables",
            json!({"variablesReference": h["variablesReference"]}),
        );
        let t = (roots["body"]["variables"].as_array().expect("roots").iter())
            .find(|v| v["name"] == "t")
            .unwrap_or_else(|| panic!("{roots}"));
        let top = c.request(
            "variables",
            json!({"variablesReference": t["variablesReference"]}),
        );
        assert!(
            top["body"]
                .to_string()
                .contains("\"evaluateName\":\"t.sc\""),
            "{top}"
        );
        // A condition takes one, and `yavs/find` lists them.
        let found = c.request("yavs/find", json!({"kind": "variables", "pattern": "s*"}));
        let items = found["body"]["items"].to_string();
        assert!(
            items.contains("\"path\":\"t.sc\"") && items.contains("protected object"),
            "{found}"
        );
        if line == 34 {
            c.send("continue", json!({"threadId": tid}));
        }
    }
    // Between cycles: nothing holds the shared state, no process is current.
    c.request("yavs/runUntil", json!({"time": "1 ns"}));
    c.event("stopped");
    for (e, want) in [
        ("sc", "protected"),
        ("sc.n", "1"),
        ("in_pkg.n", "1"),
        ("sn", "6"),
    ] {
        let v = c.request("evaluate", json!({"expression": e}));
        assert_eq!(v["body"]["result"], want, "{e}: {v}");
    }
    c.request("disconnect", json!({}));
    drop(c.w);
    child.wait().expect("wait");
    let _ = std::fs::remove_dir_all(&d);
}

/// #938, #959: the conditions of a line breakpoint read a shared variable.
#[test]
fn a_breakpoint_condition_reads_a_shared_variable() {
    let d = design("shared-cond", SHARED, &["--relaxed"]);
    let path = std::path::absolute(d.join("t.vhd")).expect("path");
    let mut child = spawn(&d, "t", &["--relaxed", "--dap=stdio"]);
    let _w = Watchdog::new(child.id(), 60);
    let mut c = stdio_client(&mut child);
    c.request("initialize", json!({"adapterID": "yavs"}));
    c.request("launch", json!({}));
    let bp = c.request(
        "setBreakpoints",
        json!({"source": {"path": path}, "breakpoints": [
            {"line": 33, "condition": "sn = 7"},
            {"line": 34, "condition": "sn = 6 and sc.n = 1"},
        ]}),
    );
    assert_eq!(bp["body"]["breakpoints"][1]["verified"], true, "{bp}");
    c.request("configurationDone", json!({}));
    let stop = c.event("stopped");
    assert_eq!(stop["body"]["reason"], "breakpoint", "{stop}");
    let tid = stop["body"]["threadId"].as_i64().expect("thread");
    let st = c.request("stackTrace", json!({"threadId": tid}));
    assert_eq!(st["body"]["stackFrames"][0]["line"], 34, "{st}");
    c.request("disconnect", json!({}));
    drop(c.w);
    child.wait().expect("wait");
    let _ = std::fs::remove_dir_all(&d);
}

/// `YAVS_COUNTERS` reports a debugger session on stderr, including the counters that show
/// activity observers and full signal histories are kept (#696).
#[test]
fn counters() {
    let dir = setup("counters");
    let mut child = yavs()
        .current_dir(&dir)
        .env("YAVS_COUNTERS", "1")
        .args(["run", "--work=work:lib", "tb", "--dap=stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("yavs");
    let _watchdog = Watchdog::new(child.id(), 120);
    let mut c = stdio_client(&mut child);
    c.request("initialize", json!({"adapterID": "yavs"}));
    c.request("launch", json!({}));
    c.request("configurationDone", json!({}));
    assert_eq!(c.event("exited")["body"]["exitCode"], 0);
    c.request("disconnect", json!({}));
    drop(c.w);
    assert_eq!(child.wait().expect("wait").code(), Some(0));
    let mut err = String::new();
    child
        .stderr
        .take()
        .expect("stderr")
        .read_to_string(&mut err)
        .expect("stderr");
    let counters: std::collections::BTreeMap<_, _> = err
        .lines()
        // The value follows the last `=`: `ieee_general` keys spell operators such as "=".
        .filter_map(|l| l.strip_prefix("counters: ")?.rsplit_once('='))
        .map(|(k, v)| (k, v.parse::<u64>().expect("counter value")))
        .collect();
    assert!(counters["cycles"] > 0, "{err}");
    assert!(counters["process_runs"] > 0, "{err}");
    assert_eq!(counters["skip_signals"], 0, "{err}");
    assert_eq!(counters["lean_signals"], 0, "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--dap=0` listens on a free port and announces it on stderr (docs/cli-contract.md 6).
#[test]
fn tcp() {
    tcp_session(false);
}

/// A connection closed at once (an editor looking whether the port is open) is no session.
#[test]
fn tcp_probe_is_no_session() {
    tcp_session(true);
}

fn tcp_session(probe: bool) {
    let dir = setup("tcp");
    let mut child = spawn(&dir, "tb", &["--dap=0"]);
    let mut err = BufReader::new(child.stderr.take().expect("stderr"));
    let mut line = String::new();
    err.read_line(&mut line).expect("stderr");
    let port = line
        .trim()
        .strip_prefix("yavs: DAP listening on 127.0.0.1:")
        .unwrap_or_else(|| panic!("first stderr line: {line}"));
    if probe {
        drop(std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("probe"));
    }
    let s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
    let mut c = Client {
        w: s.try_clone().expect("clone"),
        r: BufReader::new(s),
        seq: 0,
        log: Vec::new(),
    };
    c.request("initialize", json!({"adapterID": "yavs"}));
    c.request("launch", json!({}));
    c.request("configurationDone", json!({}));
    let out = c.event("output");
    assert!(
        out["body"]["output"]
            .as_str()
            .unwrap_or("")
            .contains("done"),
        "{out}"
    );
    let exit = c.event("exited");
    assert_eq!(exit["body"]["exitCode"], 0);
    c.request("disconnect", json!({}));
    // The server closes its side; reading on sees the end, not a reset.
    assert!(read(&mut c.r).is_none());
    drop(c);
    assert_eq!(child.wait().expect("wait").code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A test runner's flow: the arguments in a response file with `--dap`, the run held until a
/// client attaches (after `waiting for a debugger` on stderr), a line breakpoint sent before
/// `configurationDone` stops on its line, and after `continue` and `disconnect` the exit
/// status is the run's.
#[test]
fn attach_through_response_file() {
    let dir = setup("attach");
    std::fs::write(dir.join("args.rsp"), "--work=work:lib\n--dap=0\ntb\n").expect("rsp");
    let mut child = yavs()
        .current_dir(&dir)
        .args(["run", "@args.rsp"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("yavs");
    let mut err = BufReader::new(child.stderr.take().expect("stderr"));
    let mut line = String::new();
    err.read_line(&mut line).expect("stderr");
    let port = line
        .trim()
        .strip_prefix("yavs: DAP listening on 127.0.0.1:")
        .unwrap_or_else(|| panic!("first stderr line: {line}"))
        .to_string();
    line.clear();
    err.read_line(&mut line).expect("stderr");
    assert_eq!(
        line.trim(),
        format!("yavs: waiting for a debugger on port {port}")
    );
    let s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
    let mut c = Client {
        w: s.try_clone().expect("clone"),
        r: BufReader::new(s),
        seq: 0,
        log: Vec::new(),
    };
    c.request("initialize", json!({"adapterID": "yavs"}));
    c.request("launch", json!({}));
    let src = dir.join("tb.vhd").display().to_string();
    let bp = c.request(
        "setBreakpoints",
        json!({"source": {"path": src}, "breakpoints": [{"line": 20}]}),
    );
    assert_eq!(bp["body"]["breakpoints"][0]["verified"], true, "{bp}");
    c.request("configurationDone", json!({}));
    c.event("stopped");
    let st = c.request("stackTrace", json!({"threadId": 1}));
    assert_eq!(st["body"]["stackFrames"][0]["line"], 20, "{st}");
    c.request(
        "setBreakpoints",
        json!({"source": {"path": src}, "breakpoints": []}),
    );
    c.request("continue", json!({"threadId": 1}));
    assert_eq!(c.event("exited")["body"]["exitCode"], 0);
    c.request("disconnect", json!({}));
    assert!(read(&mut c.r).is_none());
    drop(c);
    assert_eq!(child.wait().expect("wait").code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--wave` with `--dap`: the FST is flushed at every stop, so a viewer reading it while the run
/// is stopped finds every change up to the stop time.
#[test]
fn wave_file_is_readable_at_a_stop() {
    let dir = setup("stopwave");
    let fst = dir.join("tb.fst");
    let mut child = spawn(
        &dir,
        "tb",
        &["--dap=stdio", &format!("--wave={}", fst.display())],
    );
    let mut c = stdio_client(&mut child);
    let src = dir.join("tb.vhd").display().to_string();
    c.request("initialize", json!({"adapterID": "yavs"}));
    c.request("launch", json!({}));
    c.request(
        "setBreakpoints",
        json!({"source": {"path": src}, "breakpoints": [{"line": 12}]}),
    );
    c.request("configurationDone", json!({}));
    c.event("stopped"); // k = 1 at 0 ns
    c.request("continue", json!({"threadId": 1}));
    c.event("stopped"); // k = 2 at 10 ns: cnt became 2 at 0 ns
    let mut w = wellen::simple::read(&fst).expect("FST readable while stopped");
    let h = w.hierarchy();
    let var = h
        .all_vars()
        .map(|r| &h[r])
        .find(|v| v.full_name(h) == "tb.cnt")
        .expect("tb.cnt");
    let sr = var.signal_ref();
    w.load_signals(&[sr]);
    let tt = w.time_table().to_vec();
    let last = w
        .get_signal(sr)
        .expect("loaded")
        .iter_changes()
        .last()
        .map(|(t, v)| (tt[t as usize], v.to_bit_string()));
    assert_eq!(last, Some((0, Some(format!("{:032b}", 2)))), "{last:?}");
    c.request(
        "setBreakpoints",
        json!({"source": {"path": src}, "breakpoints": []}),
    );
    c.request("continue", json!({"threadId": 1}));
    c.event("exited");
    c.request("disconnect", json!({}));
    drop(c.w);
    child.wait().expect("wait");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--tier` under `--dap` is checked like without it (#835): a tier whose feature this build
/// lacks is `E0120`, exit 2, not a quiet interpreted session.
/// A session that cannot start tells the client: the diagnostic is the failure of `launch` (#934).
#[test]
fn a_launch_that_fails_early_says_why() {
    let d = setup("refuse");
    let surfer_path = tempdir("dap-refuse-path");
    for (top, extra, code) in [
        ("nosuch", vec![], "E0104"),
        // No `surfer` in an empty PATH: this one fails after elaboration.
        ("tb", vec!["--surfer"], "E0130"),
    ] {
        let mut args = vec!["--dap=stdio"];
        args.extend(extra);
        let mut c = yavs()
            .current_dir(&d)
            .args(["run", "--work=work:lib", top])
            .args(&args)
            .env("PATH", &*surfer_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("yavs");
        let _w = Watchdog::new(c.id(), 120);
        let mut cl = stdio_client(&mut c);
        let init = cl.request("initialize", json!({"adapterID": "yavs"}));
        assert_eq!(init["success"], true, "{init}");
        let launch = cl.request("launch", json!({}));
        assert_eq!(launch["success"], false, "{launch}");
        let m = launch["message"].as_str().expect("a message");
        assert!(m.contains(&format!("error[{code}]")), "{m}");
        let d2 = cl.request("disconnect", json!({}));
        assert_eq!(d2["success"], true, "{d2}");
        drop(cl);
        let o = c.wait_with_output().expect("wait");
        assert_eq!(o.status.code(), Some(2));
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(err.contains(&format!("error[{code}]")), "{err}");
    }
}

/// An analysis error in a terminal is not a session: no waiting for a client. Pipes with nothing
/// behind them end at once too.
#[test]
fn a_failed_launch_ends_when_the_client_is_gone() {
    let d = setup("refuse-eof");
    let o = yavs()
        .current_dir(&d)
        .args(["run", "--work=work:lib", "nosuch", "--dap=stdio"])
        .stdin(Stdio::null())
        .output()
        .expect("yavs");
    assert_eq!(o.status.code(), Some(2));
    assert!(o.stdout.is_empty());
}

/// Tab of a client's console: commands, names in scope, fields, subcommands, files.
#[test]
fn completions() {
    let d = setup("complete");
    let mut c = spawn(&d, "tb", &["--dap=stdio"]);
    let _w = Watchdog::new(c.id(), 120);
    let mut cl = stdio_client(&mut c);
    cl.request("initialize", json!({"adapterID": "yavs"}));
    cl.request("launch", json!({"stopOnEntry": true}));
    cl.request("configurationDone", json!({}));
    cl.until(|m| m["event"] == "stopped");
    let mut ask = |text: &str| -> Vec<String> {
        let r = cl.request(
            "completions",
            json!({"text": text, "column": text.len() + 1}),
        );
        r["body"]["targets"]
            .as_array()
            .expect("targets")
            .iter()
            .map(|t| t["label"].as_str().expect("a label").to_string())
            .collect()
    };
    assert_eq!(ask("exam"), ["examine"]);
    assert_eq!(ask("step "), ["delta", "timestep"]);
    assert_eq!(ask("wave "), ["add", "cursor", "load", "save"]);
    assert_eq!(ask("wave cu"), ["cursor"]);
    assert_eq!(ask("break del"), ["delete"]);
    assert_eq!(ask("break tb"), ["tb.vhd:"]);
    // An editor's list takes its first item on Tab, so that is what all have in common.
    assert_eq!(ask("run; st"), ["st", "step", "stop"]);
    assert_eq!(ask("re"), ["re", "release", "reset", "restart"]);
    assert_eq!(ask("e"), ["ex", "examine", "exit"]);
    assert_eq!(ask("examine cn"), ["cnt"]);
    assert!(ask("help x").is_empty());
    // The first word is a command, never a signal; what follows is what the command takes.
    assert!(ask("cn").is_empty());
    assert!(ask("").iter().all(|c| c != "cnt" && c != "v"));
    assert!(ask("").contains(&"exit".to_string()));
    assert!(ask("run ").iter().eq(["until"].iter()));
    assert!(ask("run 1").is_empty());
    assert!(ask("force cnt ").is_empty());
    assert!(ask("where ").is_empty());
    for cmd in [
        "examine", "describe", "drivers", "force", "release", "trace x",
    ] {
        assert!(
            ask(&format!("{cmd} ")).contains(&"cnt".to_string()),
            "{cmd}"
        );
    }
    assert_eq!(ask("find "), ["signals", "variables"]);
    assert_eq!(ask("source t"), ["tb.vhd"]);
    assert_eq!(ask("examine t"), ["tb."]);
    assert_eq!(ask("examine tb.c"), ["tb.cnt"]);
    assert_eq!(ask("help exa"), ["examine"]);
    let d2 = cl.request("disconnect", json!({}));
    assert_eq!(d2["success"], true);
}

/// A vector is one line; arrays of vectors and records expand, and each vector in them is one line.
/// A name that resolves to nothing is an error with help, in the form it would have been (#953).
#[test]
fn vectors_are_one_line_and_watch_names_are_dotted() {
    let d = tempdir("dap-vectors");
    let src = "library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t is end;
architecture a of t is
  type rec is record a : std_ulogic; v : unsigned(3 downto 0); end record;
  type mem_t is array (0 to 1) of std_ulogic_vector(1 downto 0);
  signal slv : std_logic_vector(3 downto 0) := \"0101\";
  signal bv : bit_vector(1 downto 0) := \"01\";
  signal r : rec := ('0', \"1010\");
  signal mem : mem_t := (\"01\", \"10\");
  signal long : bit_vector(299 downto 0);
  signal count : natural := 2;
begin
  main : process
    variable lv : signed(1 downto 0) := \"10\";
    variable st : string(1 to 2) := \"ab\";
  begin
    wait for 1 ns;
    wait;
  end process;
end;
";
    std::fs::write(d.join("t.vhd"), src).unwrap();
    let st = yavs()
        .current_dir(&*d)
        .args(["analyze", "--work=work:lib", "t.vhd"])
        .status()
        .unwrap();
    assert!(st.success());
    let mut c = spawn(&d, "t", &["--dap=stdio"]);
    let _w = Watchdog::new(c.id(), 120);
    let mut cl = stdio_client(&mut c);
    cl.request(
        "initialize",
        json!({"adapterID": "yavs", "supportsANSIStyling": true}),
    );
    cl.request("launch", json!({}));
    let bp = cl.request(
        "setBreakpoints",
        json!({"source": {"path": d.join("t.vhd")}, "breakpoints": [{"line": 19}]}),
    );
    assert_eq!(bp["body"]["breakpoints"][0]["verified"], true, "{bp}");
    cl.request("configurationDone", json!({}));
    let tid = cl.until(|m| m["event"] == "stopped")["body"]["threadId"].clone();
    let st = cl.request("stackTrace", json!({"threadId": tid}));
    let frame = st["body"]["stackFrames"][0]["id"].clone();
    let mut watch = |e: &str| {
        cl.request(
            "evaluate",
            json!({"expression": e, "frameId": frame, "context": "watch"}),
        )
    };
    let one_line = |r: &Value, shown: &str| {
        assert_eq!(r["body"]["result"], shown, "{r}");
        assert_eq!(r["body"]["variablesReference"], 0, "{r}");
        assert!(r["body"]["indexedVariables"].is_null(), "{r}");
    };
    one_line(&watch("slv"), "\"0101\"");
    one_line(&watch("bv"), "\"01\"");
    one_line(&watch("t.r.v"), "\"1010\"");
    one_line(&watch("lv"), "\"10\"");
    one_line(&watch("st"), "\"ab\"");
    one_line(&watch("slv(2)"), "'1'");
    // The maintainer's two forms.
    one_line(&watch("t.count"), "2");
    one_line(&watch(":t:count"), "2");
    one_line(&watch("t.main.lv"), "\"10\"");
    // Cut short, so its elements stay reachable.
    let l = watch("long");
    assert!(
        l["body"]["result"].as_str().unwrap().ends_with("\"..."),
        "{l}"
    );
    assert_eq!(l["body"]["indexedVariables"], 300, "{l}");
    let refused = |r: Value, want: &str| {
        assert_eq!(r["success"], false, "{r}");
        let m = r["message"].as_str().unwrap();
        assert!(m.contains(want), "{m}");
    };
    refused(watch("t.cout"), "`t.count`");
    refused(watch(":t:cout"), "`t.count`");
    refused(watch("cout"), "`count`");
    refused(watch("cout(1)"), "`count`");
    refused(
        watch("t.nosuchthing"),
        "no variable or signal `t.nosuchthing`",
    );
    let m = watch("mem");
    let r = watch("r");
    let kids = |cl: &mut Client<_, _>, v: &Value| {
        let k = cl.request(
            "variables",
            json!({"variablesReference": v["body"]["variablesReference"]}),
        );
        k["body"]["variables"].as_array().unwrap().clone()
    };
    let in_mem = kids(&mut cl, &m);
    assert_eq!(in_mem.len(), 2);
    assert_eq!(in_mem[1]["value"], "\"10\"");
    assert_eq!(in_mem[1]["variablesReference"], 0);
    assert_eq!(in_mem[1]["evaluateName"], "t.mem(1)");
    let in_r = kids(&mut cl, &r);
    assert_eq!(in_r[1]["variablesReference"], 0, "{in_r:?}");
    // Bold for a client that says it shows it.
    let h = cl.request("evaluate", json!({"expression": "help", "context": "repl"}));
    assert_eq!(h["body"]["result"], "", "{h}");
    let o = cl.until(|m| m["event"] == "output" && m["body"]["category"] == "console");
    let text = o["body"]["output"].as_str().unwrap();
    assert!(text.contains("\x1b[1mexamine\x1b[0m NAME"), "{text}");
    assert!(text.contains("\x1b[1mhelp\x1b[0m "), "{text}");
    cl.request("disconnect", json!({}));
}

/// What the Variables view and a Watch see of access values, protected bodies and parts (#930, #936).
#[test]
fn views_of_access_values_protected_bodies_and_parts() {
    let d = tempdir("dap-views");
    let src = "entity t is end;
architecture a of t is
  type rec is record x : integer; y : integer; end record;
  type ptr_t is access rec;
  type vec is array (0 to 3) of integer;
  type counter_t is protected
    procedure incr;
  end protected;
  type counter_t is protected body
    variable n : integer := 0;
    procedure incr is
    begin
      n := n + 1;
    end procedure;
  end protected body;
  shared variable sc : counter_t;
  signal pt : rec := (5, 6);
  signal sv : bit_vector(3 downto 0) := \"0101\";
begin
  process
    variable q : ptr_t := new rec'(x => 7, y => 8);
    variable nul : ptr_t;
    variable v : vec := (1, 2, 3, 4);
  begin
    sc.incr;
    wait for 1 ns;
    wait;
  end process;
end;
";
    std::fs::write(d.join("t.vhd"), src).expect("write");
    let ok = yavs()
        .current_dir(&d)
        .args(["analyze", "--work=work:lib", "t.vhd"])
        .status()
        .expect("yavs")
        .success();
    assert!(ok, "analyze failed");
    let path = std::path::absolute(d.join("t.vhd")).expect("path");
    let mut c = spawn(&d, "t", &["--dap=stdio"]);
    let _w = Watchdog::new(c.id(), 120);
    let mut cl = stdio_client(&mut c);
    cl.request("initialize", json!({"adapterID": "yavs"}));
    cl.request("launch", json!({}));
    cl.request(
        "setBreakpoints",
        json!({"source": {"path": path}, "breakpoints": [{"line": 13}, {"line": 27}]}),
    );
    cl.request("configurationDone", json!({}));
    let stop = cl.event("stopped");
    let thread = stop["body"]["threadId"].as_i64().expect("thread");
    let frames = cl.request("stackTrace", json!({"threadId": thread}));
    let frame = frames["body"]["stackFrames"][0]["id"]
        .as_i64()
        .expect("frame");
    let locals = |cl: &mut Client<_, _>, frame: i64| -> Vec<Value> {
        let r = cl.request("variables", json!({"variablesReference": frame << 2 | 1}));
        r["body"]["variables"]
            .as_array()
            .expect("variables")
            .clone()
    };
    // In the protected body: its variable, not the object's handle.
    let vars = locals(&mut cl, frame);
    let names: Vec<&str> = vars.iter().filter_map(|v| v["name"].as_str()).collect();
    assert_eq!(names, ["n"], "{vars:?}");
    assert_eq!(vars[0]["value"], "0");
    let n = cl.request("evaluate", json!({"expression": "n + 1", "frameId": frame}));
    assert_eq!(n["body"]["result"], "1", "{n}");
    cl.request("continue", json!({"threadId": thread}));
    let stop = cl.event("stopped");
    let thread = stop["body"]["threadId"].as_i64().expect("thread");
    let frames = cl.request("stackTrace", json!({"threadId": thread}));
    let frame = frames["body"]["stackFrames"][0]["id"]
        .as_i64()
        .expect("frame");
    let vars = locals(&mut cl, frame);
    let q = vars.iter().find(|v| v["name"] == "q").expect("q");
    assert_eq!(q["value"], "(x => 7, y => 8)");
    assert_eq!(q["namedVariables"], 2, "{q}");
    let kids = cl.request(
        "variables",
        json!({"variablesReference": q["variablesReference"]}),
    );
    assert_eq!(kids["body"]["variables"][1]["name"], "y");
    assert_eq!(kids["body"]["variables"][1]["value"], "8");
    let nul = vars.iter().find(|v| v["name"] == "nul").expect("nul");
    assert_eq!(nul["value"], "null");
    assert_eq!(nul["variablesReference"], 0);
    // A Watch of a part, of a variable and of a signal.
    for (e, want) in [
        ("q.x", "7"),
        ("q.all.y", "8"),
        ("v(2)", "3"),
        ("pt.x", "5"),
        ("sv(1 downto 0)", "\"01\""),
    ] {
        let r = cl.request("evaluate", json!({"expression": e, "frameId": frame}));
        assert_eq!(r["body"]["result"], want, "{e}: {r}");
    }
    let bad = cl.request("evaluate", json!({"expression": "q.z", "frameId": frame}));
    assert_eq!(bad["success"], false);
    assert!(
        bad["message"]
            .as_str()
            .is_some_and(|m| m.contains("no field")),
        "{bad}"
    );
    cl.request("disconnect", json!({}));
}

#[test]
fn missing_tier_is_e0120_under_dap() {
    // (A removed tier is E0120 in every build.)
    let tier = if !cfg!(feature = "clift") {
        "clift"
    } else {
        "llvm"
    };
    let dir = setup("tier");
    let out = yavs()
        .current_dir(&dir)
        .args(["run", "--work=work:lib", "tb", "--dap=stdio"])
        .arg(format!("--tier={tier}"))
        .stdin(Stdio::null())
        .output()
        .expect("yavs");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{err}");
    assert!(err.contains("error[E0120]"), "{err}");
}

#[test]
fn coverage_with_dap_reports_a_command_limitation() {
    let d = setup("cov-unsupported");
    let out = yavs()
        .current_dir(&d)
        .args([
            "--color",
            "never",
            "run",
            "--work=work:lib",
            "--dap=stdio",
            "--cov=sbt",
            "--cov-file=a.ycov",
            "tb",
        ])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("error[E0199]"), "{err}");
    assert!(err.contains("not implemented yet"), "{err}");
    assert!(!err.contains("E9999"), "{err}");
    // No internal plan in a user message (#996).
    for plan in ["(M8)", "milestone", "slice ", "docs/design"] {
        assert!(!err.contains(plan), "{plan}: {err}");
    }
    assert!(err.contains("`--cov` together with `--dap`"), "{err}");
}

/// The exception filters of VS Code stop at reports of those severities; `yavs/find` lists.
#[test]
fn exception_filters_and_find() {
    let d = tempdir("dap-filters");
    let src =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli/debug_tools.vhd");
    std::fs::copy(src, d.join("debug_tools.vhd")).expect("copy");
    let ok = std::process::Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(&d)
        .args([
            "analyze",
            "--std=2008",
            "--work=work:lib",
            "debug_tools.vhd",
        ])
        .status()
        .expect("yavs");
    assert!(ok.success());
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(&d)
        .args(["run", "--work=work:lib", "debug_tools", "--dap=stdio"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn");
    let mut to = child.stdin.take().expect("stdin");
    let mut from = std::io::BufReader::new(child.stdout.take().expect("stdout"));
    let mut seq = 0;
    let mut send = |cmd: &str, args: serde_json::Value| {
        seq += 1;
        let m =
            serde_json::json!({"seq": seq, "type": "request", "command": cmd, "arguments": args})
                .to_string();
        use std::io::Write;
        write!(to, "Content-Length: {}\r\n\r\n{m}", m.len()).expect("send");
        to.flush().expect("flush");
    };
    let mut until = |pred: &dyn Fn(&serde_json::Value) -> bool| -> serde_json::Value {
        use std::io::{BufRead, Read};
        loop {
            let mut n = 0;
            loop {
                let mut l = String::new();
                from.read_line(&mut l).expect("line");
                let l = l.trim();
                if l.is_empty() {
                    break;
                }
                n = l
                    .strip_prefix("Content-Length: ")
                    .expect("header")
                    .parse()
                    .expect("n");
            }
            let mut b = vec![0; n];
            from.read_exact(&mut b).expect("body");
            let m: serde_json::Value = serde_json::from_slice(&b).expect("json");
            if pred(&m) {
                return m;
            }
        }
    };
    send("initialize", serde_json::json!({}));
    let caps = until(&|m| m["command"] == "initialize");
    assert_eq!(
        caps["body"]["exceptionBreakpointFilters"][0]["filter"],
        "failure"
    );
    send("launch", serde_json::json!({}));
    send(
        "setExceptionBreakpoints",
        serde_json::json!({"filters": ["warning"]}),
    );
    send("configurationDone", serde_json::json!({}));
    let s = until(&|m| m["event"] == "stopped");
    assert_eq!(s["body"]["reason"], "exception");
    assert_eq!(
        s["body"]["text"], "fatal now",
        "only the warning, not the error before it"
    );
    send(
        "yavs/find",
        serde_json::json!({"kind": "signals", "pattern": "*cn*"}),
    );
    let f = until(&|m| m["command"] == "yavs/find");
    assert_eq!(f["body"]["items"][0]["path"], "debug_tools.cnt", "{f}");
    send(
        "yavs/find",
        serde_json::json!({"kind": "variables", "pattern": "tot*"}),
    );
    let f = until(&|m| m["command"] == "yavs/find");
    assert_eq!(
        f["body"]["items"][0]["path"], "debug_tools.counter.total",
        "{f}"
    );
    send(
        "evaluate",
        serde_json::json!({"expression": "drivers cnt", "context": "repl"}),
    );
    let f = until(&|m| m["command"] == "evaluate");
    assert!(
        f["body"]["result"]
            .as_str()
            .expect("text")
            .contains("process debug_tools.counter"),
        "{f}"
    );
    send("disconnect", serde_json::json!({}));
    let _ = child.wait();
}

/// #1032: `analyzed` is the spelling the file is analyzed under (relative to the scratch
/// directory), `bp` the client's; both name one file. `map` is `--source-map` FROM=TO. Whether the
/// breakpoint on line 5 (the `report`) is verified.
fn spelled_breakpoint(analyzed: &str, bp: &str, map: Option<(&str, &str)>) -> bool {
    let d = tempdir("dap-spelling");
    std::fs::create_dir(d.join("sub")).expect("sub");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&*d, d.join("link")).expect("symlink");
    std::fs::write(
        d.join("t.vhd"),
        "entity t is end;\narchitecture a of t is\nbegin\n  process begin\n    report \"here\";\n    wait;\n  end process;\nend;\n",
    )
    .expect("write");
    let ok = yavs()
        .current_dir(&d)
        .args(["analyze", "--work=work:lib"])
        .arg(analyzed)
        .status()
        .expect("yavs")
        .success();
    assert!(ok, "analyze failed");
    let flag = map.map(|(from, to)| {
        format!(
            "--source-map={}={}",
            d.join(from).display(),
            d.join(to).display()
        )
    });
    let mut extra = vec!["--dap=stdio"];
    extra.extend(flag.as_deref());
    let mut child = spawn(&d, "t", &extra);
    let _w = Watchdog::new(child.id(), 60);
    let mut c = stdio_client(&mut child);
    c.request("initialize", json!({"adapterID": "yavs"}));
    c.request("launch", json!({}));
    let r = c.request(
        "setBreakpoints",
        json!({"source": {"path": d.join(bp)}, "breakpoints": [{"line": 5}]}),
    );
    let verified = r["body"]["breakpoints"][0]["verified"] == true;
    c.request("disconnect", json!({}));
    let _ = child.kill();
    verified
}

#[test]
fn a_breakpoint_matches_a_file_spelled_through_dotdot() {
    assert!(spelled_breakpoint("t.vhd", "t.vhd", None));
    assert!(spelled_breakpoint("sub/../t.vhd", "t.vhd", None));
    assert!(spelled_breakpoint("t.vhd", "sub/../t.vhd", None));
}

#[cfg(unix)]
#[test]
fn a_breakpoint_matches_a_file_spelled_through_a_symlink() {
    assert!(spelled_breakpoint("link/t.vhd", "t.vhd", None));
    assert!(spelled_breakpoint("t.vhd", "link/t.vhd", None));
}

/// The original of a source map need not exist: it is cleaned of `..` as far as it can be.
#[test]
fn a_source_map_matches_a_file_spelled_through_dotdot() {
    assert!(spelled_breakpoint(
        "t.vhd",
        "o.vhd",
        Some(("t.vhd", "o.vhd"))
    ));
    assert!(spelled_breakpoint(
        "sub/../t.vhd",
        "o.vhd",
        Some(("t.vhd", "o.vhd"))
    ));
    assert!(spelled_breakpoint(
        "t.vhd",
        "o.vhd",
        Some(("sub/../t.vhd", "o.vhd"))
    ));
    assert!(spelled_breakpoint(
        "t.vhd",
        "sub/../o.vhd",
        Some(("t.vhd", "o.vhd"))
    ));
}

#[cfg(unix)]
#[test]
fn a_source_map_matches_a_file_spelled_through_a_symlink() {
    assert!(spelled_breakpoint(
        "link/t.vhd",
        "o.vhd",
        Some(("t.vhd", "o.vhd"))
    ));
    assert!(spelled_breakpoint(
        "t.vhd",
        "o.vhd",
        Some(("link/t.vhd", "o.vhd"))
    ));
}

/// #1036: breakpoints set under one spelling of a file are replaced by a later list under another
/// (here an empty one), so the run does not stop. True when the run ends without stopping.
fn cleared_through(first: &str, second: &str) -> bool {
    let d = tempdir("dap-spelling-clear");
    std::fs::create_dir(d.join("sub")).expect("sub");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&*d, d.join("link")).expect("symlink");
    std::fs::write(
        d.join("t.vhd"),
        "entity t is end;\narchitecture a of t is\nbegin\n  process begin\n    report \"here\";\n    wait;\n  end process;\nend;\n",
    )
    .expect("write");
    let ok = yavs()
        .current_dir(&d)
        .args(["analyze", "--work=work:lib", "t.vhd"])
        .status()
        .expect("yavs")
        .success();
    assert!(ok, "analyze failed");
    let mut child = spawn(&d, "t", &["--dap=stdio"]);
    let _w = Watchdog::new(child.id(), 30);
    let mut c = stdio_client(&mut child);
    c.request("initialize", json!({"adapterID": "yavs"}));
    c.request("launch", json!({}));
    let r = c.request(
        "setBreakpoints",
        json!({"source": {"path": d.join(first)}, "breakpoints": [{"line": 5}]}),
    );
    assert_eq!(r["body"]["breakpoints"][0]["verified"], true, "{r}");
    c.request(
        "setBreakpoints",
        json!({"source": {"path": d.join(second)}, "breakpoints": []}),
    );
    c.request("configurationDone", json!({}));
    let end = c.until(|m| m["event"] == "stopped" || m["event"] == "terminated");
    let _ = child.kill();
    end["event"] == "terminated"
}

#[test]
fn a_later_list_replaces_the_breakpoints_of_another_spelling() {
    assert!(cleared_through("t.vhd", "t.vhd"));
    assert!(cleared_through("t.vhd", "sub/../t.vhd"));
    #[cfg(unix)]
    {
        assert!(cleared_through("t.vhd", "link/t.vhd"));
        assert!(cleared_through("link/t.vhd", "t.vhd"));
    }
}
