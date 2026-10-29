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
    let dir = setup(&format!("{name}{}", extra.len()));
    let src = dir.join("tb.vhd").display().to_string();
    let script = std::fs::read_to_string(cases().join(format!("{name}.script"))).expect("script");
    // An optional first line `top NAME` picks another top level than `tb`.
    let (top, script) = match script.strip_prefix("top ") {
        Some(rest) => rest.split_once('\n').expect("script after `top`"),
        None => ("tb", script.as_str()),
    };
    let mut child = spawn(&dir, top.trim(), &[&["--dap=stdio"], extra].concat());
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
        .filter_map(|l| l.strip_prefix("counters: ")?.split_once('='))
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
    let dir = setup("tcp");
    let mut child = spawn(&dir, "tb", &["--dap=0"]);
    let mut err = BufReader::new(child.stderr.take().expect("stderr"));
    let mut line = String::new();
    err.read_line(&mut line).expect("stderr");
    let port = line
        .trim()
        .strip_prefix("yavs: DAP listening on 127.0.0.1:")
        .unwrap_or_else(|| panic!("first stderr line: {line}"));
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
#[test]
fn missing_tier_is_e0120_under_dap() {
    let tier = if !cfg!(feature = "clift") {
        "clift"
    } else if !cfg!(feature = "llvm") {
        "llvm"
    } else {
        return; // every tier is built in
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
