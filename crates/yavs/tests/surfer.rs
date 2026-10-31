//! `yavs run --surfer`: a fake WCP endpoint in the test records what yavs sends and answers like
//! Surfer (greeting, acks, ids, `waveforms_loaded`).

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::tempdir;
use serde_json::{Value, json};

fn yavs() -> Command {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
}

fn analyze(d: &Path, file: &str) {
    let ok = yavs()
        .current_dir(d)
        .args(["analyze", "--std=2008", "--work=work:build/work", file])
        .status()
        .expect("yavs")
        .success();
    assert!(ok, "analyze failed");
}

const TB: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
entity tb is end;
architecture a of tb is
  type rec_t is record
    x : integer;
    y : std_logic;
  end record;
  signal cnt : integer := 0;
  signal v : std_logic_vector(3 downto 0) := "0101";
  signal r : rec_t := (0, '0');
begin
  p : process
  begin
    for k in 1 to 3 loop
      cnt <= k;
      v <= not v;
      r.x <= k;
      wait for 10 ns;
    end loop;
    report "done";
    wait;
  end process;
end;
"#;

fn setup(name: &str) -> common::Tmp {
    let d = tempdir(&format!("surfer-{name}"));
    std::fs::write(d.join("tb.vhd"), TB).expect("write");
    analyze(&d, "tb.vhd");
    d
}

/// Every frame yavs sent, and how the fake answers.
#[derive(Clone, Default)]
struct Fake {
    frames: Arc<Mutex<Vec<Value>>>,
    /// Refuse this command with an error.
    refuse: Arc<Mutex<Option<String>>>,
    /// Close the socket after this many frames.
    close_after: Arc<Mutex<Option<usize>>>,
}

impl Fake {
    fn commands(&self) -> Vec<String> {
        let f = self.frames.lock().expect("lock");
        f.iter()
            .filter_map(|m| m["command"].as_str().map(String::from))
            .collect()
    }

    fn first(&self, cmd: &str) -> Value {
        let f = self.frames.lock().expect("lock");
        f.iter()
            .find(|m| m["command"] == cmd)
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn all(&self, cmd: &str) -> Vec<Value> {
        let f = self.frames.lock().expect("lock");
        f.iter().filter(|m| m["command"] == cmd).cloned().collect()
    }

    /// Serve one connection until it closes.
    fn serve(&self, s: TcpStream, events: &[Value]) {
        let mut w = s.try_clone().expect("clone");
        let mut r = BufReader::new(s);
        let mut next_id = 0;
        let send = |w: &mut TcpStream, m: Value| {
            let mut b = m.to_string().into_bytes();
            b.push(0);
            let _ = w.write_all(&b);
        };
        let mut sent = false;
        loop {
            let mut b = Vec::new();
            match r.read_until(0, &mut b) {
                Ok(n) if n > 0 => {}
                _ => return,
            }
            b.pop();
            let m: Value = serde_json::from_slice(&b).expect("a JSON frame");
            let n = {
                let mut f = self.frames.lock().expect("lock");
                f.push(m.clone());
                f.len()
            };
            if self
                .close_after
                .lock()
                .expect("lock")
                .is_some_and(|k| n >= k)
            {
                return;
            }
            if m["type"] == "greeting" {
                send(
                    &mut w,
                    json!({"type": "greeting", "version": "0", "commands": []}),
                );
                continue;
            }
            let cmd = m["command"].as_str().unwrap_or_default();
            if self.refuse.lock().expect("lock").as_deref() == Some(cmd) {
                send(
                    &mut w,
                    json!({"type": "error", "error": cmd, "arguments": [], "message": "no"}),
                );
                continue;
            }
            match cmd {
                "add_items" | "add_markers" => {
                    let k = m["items"]
                        .as_array()
                        .or(m["markers"].as_array())
                        .map_or(0, Vec::len);
                    let ids: Vec<usize> = (next_id..next_id + k).collect();
                    next_id += k;
                    send(
                        &mut w,
                        json!({"type": "response", "command": cmd, "ids": ids}),
                    );
                }
                _ => send(&mut w, json!({"type": "response", "command": "ack"})),
            }
            if matches!(cmd, "load" | "reload") {
                send(
                    &mut w,
                    json!({"type": "event", "event": "waveforms_loaded", "source": "x"}),
                );
                if cmd == "reload" && !sent {
                    sent = true;
                    for e in events {
                        send(&mut w, e.clone());
                    }
                }
            }
        }
    }

    /// Listen on a free port: a connect-mode endpoint.
    fn listen(&self, events: Vec<Value>) -> (String, std::thread::JoinHandle<()>) {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = l.local_addr().expect("addr").to_string();
        let me = self.clone();
        let t = std::thread::spawn(move || {
            if let Ok((s, _)) = l.accept() {
                me.serve(s, &events);
            }
        });
        (addr, t)
    }
}

fn run(d: &Path, args: &[&str]) -> Output {
    yavs()
        .current_dir(d)
        .args(["run", "--work=work:build/work", "tb"])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("yavs")
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn connect_mode_loads_shows_and_fits() {
    let d = setup("connect");
    let fake = Fake::default();
    let (addr, t) = fake.listen(vec![]);
    let plain = run(&d, &[]);
    let o = run(&d, &[&format!("--surfer={addr}")]);
    t.join().expect("fake");
    assert_eq!(stdout(&o), stdout(&plain));
    assert_eq!(o.status.code(), plain.status.code(), "{}", stderr(&o));
    let f = fake.frames.lock().expect("lock").clone();
    assert_eq!(f[0]["type"], "greeting");
    assert_eq!(f[0]["version"], "0");
    assert_eq!(
        f[0]["commands"],
        json!(["waveforms_loaded", "goto_declaration"])
    );
    // The default file, next to the work library, absolute and kept.
    let file = d.join("build/tb.fst");
    assert!(file.exists());
    let load = fake.first("load");
    let absolute = std::path::absolute(&file).expect("absolute");
    assert_eq!(load["source"].as_str(), absolute.to_str());
    assert!(
        stderr(&o).contains("the waveform is written to"),
        "{}",
        stderr(&o)
    );
    // Scalars and vectors of the top, and a record one level down.
    let items = fake.first("add_items");
    let names: Vec<&str> = items["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(names.contains(&"tb.cnt"), "{names:?}");
    assert!(names.contains(&"tb.v"), "{names:?}");
    assert!(names.contains(&"tb.r.x"), "{names:?}");
    // The end: a last reload, then the whole run.
    let c = fake.commands();
    assert_eq!(c[0], "load", "{c:?}");
    assert_eq!(c.last().map(String::as_str), Some("zoom_to_fit"), "{c:?}");
    assert!(c.contains(&"reload".to_string()), "{c:?}");
    // Nothing is added before the file has values: Surfer panics on an empty time table.
    let at = |n: &str| c.iter().position(|x| x == n);
    assert!(at("reload") < at("add_items"), "{c:?}");
    assert_eq!(fake.first("zoom_to_fit")["viewport_idx"], 0);
    wellen::simple::read(&file).expect("readable");
}

#[test]
fn wave_file_and_include_choose_what_is_shown() {
    let d = setup("include");
    let fake = Fake::default();
    let (addr, t) = fake.listen(vec![]);
    let o = run(
        &d,
        &[
            &format!("--surfer={addr}"),
            "--wave=my.fst",
            "--wave-include=tb.cnt",
        ],
    );
    t.join().expect("fake");
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert!(d.join("my.fst").exists());
    assert!(!d.join("build/tb.fst").exists());
    assert!(!stderr(&o).contains("the waveform is written"));
    assert_eq!(fake.first("add_items")["items"], json!(["tb.cnt"]));
}

/// At most 200 signals are added on their own.
#[test]
fn at_most_200_signals_on_their_own() {
    let d = tempdir("surfer-many");
    let mut src = String::from("entity tb is end;\narchitecture a of tb is\n");
    for i in 0..250 {
        src += &format!("  signal s{i} : bit;\n");
    }
    src += "begin\nend;\n";
    std::fs::write(d.join("tb.vhd"), src).expect("write");
    analyze(&d, "tb.vhd");
    let fake = Fake::default();
    let (addr, t) = fake.listen(vec![]);
    let o = run(&d, &[&format!("--surfer={addr}")]);
    t.join().expect("fake");
    assert_eq!(
        fake.first("add_items")["items"].as_array().map(Vec::len),
        Some(200)
    );
    assert!(stderr(&o).contains("50 more signals"), "{}", stderr(&o));
}

#[test]
fn refresh_reloads_while_the_file_grows() {
    let d = tempdir("surfer-refresh");
    std::fs::write(
        d.join("tb.vhd"),
        "entity tb is end;\narchitecture a of tb is\n  signal c : integer := 0;\nbegin\n  process begin\n    for i in 1 to 40 loop\n      c <= i;\n      for j in 1 to 300000 loop\n        null;\n      end loop;\n      wait for 1 ns;\n    end loop;\n    wait;\n  end process;\nend;\n",
    )
    .expect("write");
    analyze(&d, "tb.vhd");
    let fake = Fake::default();
    let (addr, t) = fake.listen(vec![]);
    let start = Instant::now();
    let o = run(
        &d,
        &[
            &format!("--surfer={addr}"),
            "--surfer-refresh=20ms",
            "--tier=interp",
        ],
    );
    t.join().expect("fake");
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    // Only a run longer than some refreshes can show one: never a flaky bound on time.
    if start.elapsed() > Duration::from_millis(500) {
        assert!(fake.all("reload").len() >= 2, "{:?}", fake.commands());
    }
}

#[test]
fn refresh_options() {
    let d = setup("opts");
    for v in ["0s", "x", "5"] {
        let o = run(
            &d,
            &["--surfer=127.0.0.1:1", &format!("--surfer-refresh={v}")],
        );
        assert_eq!(o.status.code(), Some(2), "{v}");
        assert!(
            stderr(&o).contains("use a duration above zero"),
            "{}",
            stderr(&o)
        );
    }
    let o = run(&d, &["--surfer-refresh=1s"]);
    assert_eq!(o.status.code(), Some(2), "needs --surfer");
}

#[test]
fn errors_at_the_start_are_e0130() {
    let d = setup("err");
    // Nobody listens.
    let port = TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port();
    let o = run(&d, &[&format!("--surfer=127.0.0.1:{port}")]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("E0130"), "{}", stderr(&o));
    assert_eq!(stdout(&o), "");
    // `load` refused.
    let fake = Fake::default();
    *fake.refuse.lock().expect("lock") = Some("load".into());
    let (addr, t) = fake.listen(vec![]);
    let o = run(&d, &[&format!("--surfer={addr}")]);
    t.join().expect("fake");
    assert_eq!(o.status.code(), Some(2));
    assert!(
        stderr(&o).contains("Surfer did not take the waveform file"),
        "{}",
        stderr(&o)
    );
    // Greeting refused: the socket closes.
    let fake = Fake::default();
    *fake.close_after.lock().expect("lock") = Some(1);
    let (addr, t) = fake.listen(vec![]);
    let o = run(&d, &[&format!("--surfer={addr}")]);
    t.join().expect("fake");
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("E0130"), "{}", stderr(&o));
}

/// The socket closes during the run: the same stdout and status, a warning that `-W error` does
/// not promote.
#[test]
fn a_lost_viewer_is_a_warning_and_changes_nothing() {
    let d = setup("lost");
    let plain = run(&d, &[]);
    for extra in [&[][..], &["-W", "error"][..]] {
        let fake = Fake::default();
        // The greeting, the load, and at the end's reload it is gone.
        *fake.close_after.lock().expect("lock") = Some(3);
        let (addr, t) = fake.listen(vec![]);
        let flag = format!("--surfer={addr}");
        let mut args = vec![flag.as_str()];
        args.extend(extra);
        let o = run(&d, &args);
        t.join().expect("fake");
        assert_eq!(stdout(&o), stdout(&plain));
        assert_eq!(o.status.code(), plain.status.code(), "{}", stderr(&o));
        assert!(stderr(&o).contains("W0131"), "{}", stderr(&o));
    }
}

/// `--dap` and `--interactive` stops: reload, cursor and viewport at the stop time, a marker at a
/// breakpoint, colours for the stopped process's signals; all before the prompt goes on.
#[test]
fn console_stops_drive_the_viewer() {
    let d = setup("stops");
    let fake = Fake::default();
    let (addr, t) = fake.listen(vec![]);
    let o = run(
        &d,
        &[
            &format!("--surfer={addr}"),
            "--do=break tb.vhd:23; run; wave add tb.r; wave cursor 5 ns; restart; wave add cnt; run 10 ns; quit",
        ],
    );
    t.join().expect("fake");
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let cursors: Vec<u64> = fake
        .all("set_cursor")
        .iter()
        .filter_map(|m| m["timestamp"].as_u64())
        .collect();
    // The breakpoint at 30 ns and `wave cursor` come in this order, 10 ns after the restart last;
    // stops queued while Surfer is busy count as one.
    let pos = |t: u64| cursors.iter().position(|&c| c == t);
    assert!(pos(30_000_000) < pos(5_000_000), "{cursors:?}");
    assert_eq!(cursors.last(), Some(&10_000_000), "{cursors:?}");
    let marker = fake.first("add_markers");
    assert_eq!(marker["markers"][0]["time"], 30_000_000);
    assert_eq!(marker["markers"][0]["move_focus"], false);
    // The stopped process's signals in colour.
    assert!(
        !fake.all("set_item_color").is_empty(),
        "{:?}",
        fake.commands()
    );
    let adds = fake.all("add_items");
    let added = |n: &str| adds.iter().any(|a| a["items"] == json!([n]));
    assert!(added("tb.r") && added("tb.cnt"), "{adds:?}");
    assert!(stdout(&o).contains("1 added"), "{}", stdout(&o));
}

/// A DAP session: the cursor at each stop, a marker at a breakpoint, the whole run at the end.
#[test]
fn dap_stops_drive_the_viewer() {
    let d = setup("dap");
    let fake = Fake::default();
    let (addr, t) = fake.listen(vec![]);
    let mut c = yavs()
        .current_dir(&d)
        .args(["run", "--work=work:build/work", "tb", "--dap=stdio"])
        .arg(format!("--surfer={addr}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("yavs");
    let mut w = c.stdin.take().expect("stdin");
    let mut r = BufReader::new(c.stdout.take().expect("stdout"));
    let mut seq = 0;
    let mut req = |cmd: &str, args: Value| {
        seq += 1;
        let m =
            json!({"type": "request", "seq": seq, "command": cmd, "arguments": args}).to_string();
        write!(w, "Content-Length: {}\r\n\r\n{m}", m.len()).expect("send");
        w.flush().expect("flush");
    };
    let mut event = |name: &str| loop {
        let mut len = 0;
        loop {
            let mut l = String::new();
            r.read_line(&mut l).expect("read");
            if let Some(n) = l.strip_prefix("Content-Length: ") {
                len = n.trim().parse().expect("length");
            }
            if l.trim().is_empty() {
                break;
            }
        }
        let mut b = vec![0u8; len];
        r.read_exact(&mut b).expect("body");
        let m: Value = serde_json::from_slice(&b).expect("json");
        if m["event"] == name {
            return m;
        }
    };
    req("initialize", json!({"adapterID": "yavs"}));
    req("launch", json!({}));
    let src = d.join("tb.vhd");
    req(
        "setBreakpoints",
        json!({"source": {"path": src}, "breakpoints": [{"line": 23}]}),
    );
    req("configurationDone", json!({}));
    event("stopped");
    req("continue", json!({}));
    event("terminated");
    req("disconnect", json!({}));
    drop(w);
    assert_eq!(c.wait().expect("wait").code(), Some(0));
    t.join().expect("fake");
    assert_eq!(fake.first("set_cursor")["timestamp"], 30_000_000);
    assert_eq!(fake.first("set_viewport_to")["timestamp"], 30_000_000);
    assert_eq!(fake.first("add_markers")["markers"][0]["time"], 30_000_000);
    let c = fake.commands();
    assert_eq!(c.last().map(String::as_str), Some("zoom_to_fit"), "{c:?}");
}

#[test]
fn wave_commands_need_a_viewer() {
    let d = setup("noviewer");
    let o = run(&d, &["--do=wave add cnt"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("run with `--surfer`"), "{}", stderr(&o));
}

/// Surfer's "go to declaration": the file and line on the console.
#[test]
fn go_to_declaration_shows_the_place() {
    let d = setup("goto");
    let fake = Fake::default();
    let goto = json!({"type": "event", "event": "goto_declaration", "variable": "tb.cnt"});
    let (addr, t) = fake.listen(vec![goto]);
    let mut c = yavs()
        .current_dir(&d)
        .args(["run", "--work=work:build/work", "tb", "--interactive"])
        .arg(format!("--surfer={addr}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("yavs");
    let mut input = c.stdin.take().expect("stdin");
    let out = BufReader::new(c.stdout.take().expect("stdout"));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for l in out.lines().map_while(Result::ok) {
            let _ = tx.send(l);
        }
    });
    let mut found = None;
    let deadline = Instant::now() + Duration::from_secs(60);
    // The file has values at a stop once its first block is written: the run's end does that.
    input.write_all(b"run\n").expect("write");
    while found.is_none() && Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(l) if l.starts_with("tb.cnt: ") => found = Some(l),
            Ok(_) | Err(_) => {}
        }
    }
    input.write_all(b"quit\n").expect("write");
    drop(input);
    let _ = c.wait();
    t.join().expect("fake");
    let l = found.expect("the place of the declaration");
    assert!(l.ends_with("tb.vhd:10"), "{l}");
}

/// A fake `surfer` first on PATH that writes its arguments and stays.
#[cfg(unix)]
fn fake_surfer(d: &Path, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = d.join("bin");
    std::fs::create_dir_all(&bin).expect("mkdir");
    let p = bin.join("surfer");
    std::fs::write(
        &p,
        format!(
            "#!/bin/sh\necho \"$@\" > \"{}/args\"\n{body}\n",
            d.display()
        ),
    )
    .expect("write");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    bin
}

#[cfg(unix)]
#[test]
fn spawn_mode_connects_back() {
    let d = setup("spawn");
    let bin = fake_surfer(&d, "sleep 30");
    let fake = Fake::default();
    let (fake2, dir) = (fake.clone(), d.to_path_buf());
    let t = std::thread::spawn(move || {
        let start = Instant::now();
        let port = loop {
            if let Ok(a) = std::fs::read_to_string(dir.join("args"))
                && let Some(p) = a.trim().strip_prefix("--wcp-initiate ")
            {
                break p.to_string();
            }
            assert!(start.elapsed() < Duration::from_secs(60));
            std::thread::sleep(Duration::from_millis(10));
        };
        let s = TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
        fake2.serve(s, &[]);
    });
    let o = yavs()
        .current_dir(&d)
        .env("PATH", &bin)
        .args(["run", "--work=work:build/work", "tb", "--surfer"])
        .stdin(Stdio::null())
        .output()
        .expect("yavs");
    t.join().expect("fake");
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(fake.first("add_items")["type"], "command");
    assert_eq!(
        fake.commands().last().map(String::as_str),
        Some("zoom_to_fit")
    );
}

#[cfg(unix)]
#[test]
fn spawn_failures_are_e0130() {
    let d = setup("spawnfail");
    let bin = fake_surfer(&d, "exit 3");
    let o = yavs()
        .current_dir(&d)
        .env("PATH", &bin)
        .args(["run", "--work=work:build/work", "tb", "--surfer"])
        .output()
        .expect("yavs");
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("before it connected"), "{}", stderr(&o));
    let empty = d.join("empty");
    std::fs::create_dir_all(&empty).expect("mkdir");
    let o = yavs()
        .current_dir(&d)
        .env("PATH", &empty)
        .args(["run", "--work=work:build/work", "tb", "--surfer"])
        .output()
        .expect("yavs");
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("not on PATH"), "{}", stderr(&o));
}

/// Nothing starts or connects to Surfer without the flag: not `--wave`, not `--dap`.
#[cfg(unix)]
#[test]
fn strictly_opt_in() {
    let d = setup("optin");
    let bin = fake_surfer(&d, "sleep 1");
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    // A listener on Surfer's default address, if the port is free here.
    let l = TcpListener::bind("127.0.0.1:54321").ok();
    if let Some(l) = &l {
        l.set_nonblocking(true).expect("nonblocking");
    }
    let run_with = |args: &[&str]| {
        yavs()
            .current_dir(&d)
            .env("PATH", &path)
            .args(["run", "--work=work:build/work", "tb"])
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("yavs")
    };
    assert_eq!(run_with(&["--wave=a.fst"]).status.code(), Some(0));
    run_with(&["--dap=stdio"]);
    run_with(&["--dap=stdio", "--wave=b.fst"]);
    assert_eq!(
        run_with(&["--do=run", "--wave=c.fst"]).status.code(),
        Some(0)
    );
    assert!(!d.join("args").exists(), "surfer was started");
    if let Some(l) = l {
        assert!(l.accept().is_err(), "something connected to Surfer's port");
    }
}

/// Without `--surfer` a wave file's bytes do not depend on the run's speed (#666).
#[test]
fn no_clock_flush_without_the_flag() {
    let d = setup("bytes");
    run(&d, &["--wave=a.fst"]);
    run(&d, &["--wave=b.fst"]);
    let read = |f: &str| {
        let mut b = Vec::new();
        std::fs::File::open(d.join(f))
            .expect("open")
            .read_to_end(&mut b)
            .expect("read");
        b
    };
    assert_eq!(read("a.fst"), read("b.fst"));
}
