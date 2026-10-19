//! `yavs run --dap` (docs/cli-contract.md 5): scripted Debug Adapter Protocol sessions.
//!
//! Each case in `tests/dap` is a script, `NAME.script`, and the expected transcript,
//! `NAME.expected`. A script line is `request COMMAND {ARGS}` (send, then read up to its
//! response) or `event NAME` (read up to that event); `$SRC` is the absolute path of
//! `tb.vhd`. The transcript has `> ` for what was sent and `< ` for what was read, without
//! `seq`, with the scratch directory as `$TMP`. `YAVS_BLESS=1` rewrites the expected files.

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
fn setup(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("yavs-dap-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("tmp");
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
    let mut child = spawn(&dir, "tb", &[&["--dap=stdio"], extra].concat());
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

/// Pause a simulation that would run forever, then read a signal and disconnect.
#[test]
fn pause() {
    let dir = setup("pause");
    let mut child = spawn(&dir, "spin", &["--dap=stdio", "--threads=4"]);
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
    assert_eq!(child.wait().expect("wait").code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}
