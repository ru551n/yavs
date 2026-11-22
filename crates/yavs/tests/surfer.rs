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
  type mem_t is array (0 to 1) of std_logic_vector(1 downto 0);
  signal m : mem_t := (others => "00");
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
    /// At the first of this command, answer nothing for this long, then all answers in order:
    /// what a Surfer does whose window is not drawn.
    stall: Arc<Mutex<Option<(String, Duration)>>>,
    /// The first `reload` ends without `waveforms_loaded`: what a load that fails does.
    skip_loaded: Arc<Mutex<bool>>,
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

    /// Waits until yavs has sent `cmd`; fails after `limit`.
    fn wait_for(&self, cmd: &str, limit: Duration) {
        let t0 = Instant::now();
        while self.first(cmd).is_null() {
            assert!(t0.elapsed() < limit, "no `{cmd}` within {limit:?}");
            std::thread::sleep(Duration::from_millis(10));
        }
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
            let stall = self.stall.lock().expect("lock").take_if(|(c, _)| c == cmd);
            if let Some((_, time)) = stall {
                std::thread::sleep(time);
            }
            if self.refuse.lock().expect("lock").as_deref() == Some(cmd) {
                send(
                    &mut w,
                    json!({"type": "error", "error": cmd, "arguments": [], "message": "no"}),
                );
                continue;
            }
            match cmd {
                "get_item_list" => {
                    let ids: Vec<usize> = (1..next_id).collect();
                    send(
                        &mut w,
                        json!({"type": "response", "command": cmd, "ids": ids}),
                    );
                }
                "get_item_info" => {
                    let results: Vec<Value> = m["ids"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|_| json!({"name": "not used", "type": "Variable"}))
                        .collect();
                    send(
                        &mut w,
                        json!({"type": "response", "command": cmd, "results": results}),
                    );
                }
                "add_items" | "add_markers" | "add_scope" => {
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
            if cmd == "reload" && std::mem::take(&mut *self.skip_loaded.lock().expect("lock")) {
                continue;
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
    // As the viewer's file reader names an element of an array.
    assert!(names.contains(&"tb.m.(1)"), "{names:?}");
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
    let o = run(
        &d,
        &[&format!("--surfer=127.0.0.1:{}", common::NO_LISTENER)],
    );
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

/// A console session fed line by line, its stderr read as it comes.
struct Session {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    err: std::sync::mpsc::Receiver<String>,
    out: std::sync::mpsc::Receiver<String>,
    /// What was read so far.
    seen: String,
}

impl Session {
    /// Starts a session with `--surfer` on the fake and Surfer's answer wait in ms.
    fn start(d: &Path, addr: &str, wait_ms: u64) -> Self {
        Self::with(d, addr, wait_ms, &[])
    }

    fn with(d: &Path, addr: &str, wait_ms: u64, more: &[&str]) -> Self {
        let mut child = yavs()
            .current_dir(d)
            .args(["run", "--work=work:build/work", "tb", "--interactive"])
            .arg(format!("--surfer={addr}"))
            .args(more)
            .env("YAVS_SURFER_WAIT", wait_ms.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("yavs");
        fn lines(r: impl Read + Send + 'static) -> std::sync::mpsc::Receiver<String> {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                for l in BufReader::new(r).lines().map_while(Result::ok) {
                    if tx.send(l).is_err() {
                        return;
                    }
                }
            });
            rx
        }
        let err = lines(child.stderr.take().expect("stderr"));
        let out = lines(child.stdout.take().expect("stdout"));
        let stdin = child.stdin.take().expect("stdin");
        Self {
            child,
            stdin,
            err,
            out,
            seen: String::new(),
        }
    }

    fn send(&mut self, line: &str) {
        writeln!(self.stdin, "{line}").expect("write");
    }

    /// Waits for `text` on stdout or stderr; fails after `limit`.
    fn expect(&mut self, text: &str, limit: Duration) {
        let t0 = Instant::now();
        loop {
            for rx in [&self.err, &self.out] {
                while let Ok(l) = rx.try_recv() {
                    self.seen += &l;
                    self.seen.push('\n');
                }
            }
            if self.seen.contains(text) {
                self.seen = self.seen.replacen(text, "", 1);
                return;
            }
            assert!(
                t0.elapsed() < limit,
                "no `{text}` within {limit:?} in:\n{}",
                self.seen
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Quits; the exit status and the rest of what it printed.
    fn quit(mut self) -> (Option<i32>, String) {
        self.send("quit");
        drop(self.stdin);
        let status = self.child.wait().expect("wait").code();
        for rx in [&self.err, &self.out] {
            while let Ok(l) = rx.recv_timeout(Duration::from_secs(1)) {
                self.seen += &l;
                self.seen.push('\n');
            }
        }
        (status, self.seen)
    }
}

const SHORT: Duration = Duration::from_secs(20);

/// A Surfer that does not answer for a while (its window not drawn): the stall is told when it
/// happens, `wave add` answers at once, and when the answers come all at once the connection goes
/// on, with what was added in the meantime and the right count for a later `wave add`.
#[test]
fn a_stalled_viewer_is_told_and_catches_up() {
    let d = setup("stall");
    let fake = Fake::default();
    let held = Duration::from_secs(4);
    *fake.stall.lock().expect("lock") = Some(("reload".into(), held));
    let (addr, t) = fake.listen(vec![]);
    let mut s = Session::start(&d, &addr, 200);
    // The viewer takes the stop at the start in before the run goes on: one that came to it late
    // would reload for it and again for the breakpoint (#980).
    fake.wait_for("set_cursor", SHORT);
    s.send("break tb.vhd:23");
    s.send("run");
    let t0 = Instant::now();
    s.expect("Surfer has not answered for 0.2 s", SHORT);
    s.send("wave add cnt");
    s.expect("`tb.cnt` is not in its view yet", SHORT);
    assert!(t0.elapsed() < held, "the debugger waited for the viewer");
    s.send("wave save nothing.txt");
    s.expect("what its view shows is not known", SHORT);
    assert!(t0.elapsed() < held, "the debugger waited for the viewer");
    s.expect("Surfer answers again", SHORT);
    // Two names: a late answer taken for this one's would say 1.
    s.send("wave add v cnt");
    s.expect("2 added", SHORT);
    let (status, rest) = s.quit();
    t.join().expect("fake");
    assert_eq!(status, Some(0), "{rest}");
    assert!(!rest.contains("W0131"), "{rest}");
    let adds = fake.all("add_items");
    // What was not answered goes with the first add once Surfer answers.
    assert_eq!(
        adds[0]["items"].as_array().expect("items").last(),
        Some(&json!("tb.cnt"))
    );
    assert_eq!(adds[1]["items"], json!(["tb.v", "tb.cnt"]), "{adds:?}");
    // Nothing more was asked while it did not answer, and the end is as usual.
    assert_eq!(fake.all("reload").len(), 2, "{:?}", fake.commands());
    assert_eq!(fake.all("zoom_to_fit").len(), 1, "{:?}", fake.commands());
}

/// The answer to an `add_items` that comes too late still counts for that one, and a later
/// `wave add` gets its own answer.
#[test]
fn a_late_answer_is_not_the_answer_to_a_later_command() {
    let d = setup("late");
    let fake = Fake::default();
    *fake.stall.lock().expect("lock") = Some(("add_items".into(), Duration::from_secs(2)));
    let (addr, t) = fake.listen(vec![]);
    let mut s = Session::start(&d, &addr, 200);
    s.send("break tb.vhd:23");
    s.send("run");
    s.expect("breakpoint at", SHORT);
    // The first add_items at the start of the run has its values at this stop: wait for it.
    s.expect("Surfer answers again", SHORT);
    s.send("wave add v cnt");
    s.expect("2 added", SHORT);
    let (status, rest) = s.quit();
    t.join().expect("fake");
    assert_eq!(status, Some(0), "{rest}");
}

/// A refused command concerns that command: it is told, and the next ones work.
#[test]
fn a_refused_command_does_not_end_the_connection() {
    let d = setup("refused");
    let fake = Fake::default();
    *fake.refuse.lock().expect("lock") = Some("set_viewport_to".into());
    let (addr, t) = fake.listen(vec![]);
    let o = run(
        &d,
        &[
            &format!("--surfer={addr}"),
            "--do=break tb.vhd:23; run; wave add cnt; wave save kept.txt; quit",
        ],
    );
    t.join().expect("fake");
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("Surfer refused `set_viewport_to`: no"),
        "{}",
        stderr(&o)
    );
    assert!(!stderr(&o).contains("W0131"), "{}", stderr(&o));
    assert!(stdout(&o).contains("1 added"), "{}", stdout(&o));
    assert_eq!(fake.all("zoom_to_fit").len(), 1, "{:?}", fake.commands());
}

/// Surfer sends `waveforms_loaded` only for a load that works: one that fails does not hold the
/// view up for good, and the reloads go on.
#[test]
fn a_load_without_its_event_does_not_hold_the_view() {
    let d = setup("noload");
    let fake = Fake::default();
    *fake.skip_loaded.lock().expect("lock") = true;
    let (addr, t) = fake.listen(vec![]);
    let mut s = Session::start(&d, &addr, 50);
    s.send("break tb.vhd:23");
    s.send("run");
    s.expect("did not finish reading", SHORT);
    s.send("wave add cnt");
    s.expect("1 added", SHORT);
    let (status, rest) = s.quit();
    t.join().expect("fake");
    assert_eq!(status, Some(0), "{rest}");
    assert!(!rest.contains("W0131"), "{rest}");
    assert!(fake.all("reload").len() >= 2, "{:?}", fake.commands());
}

/// Surfer closed during a session: told when it happens, not only at the end, and with a text of
/// its own; the session goes on.
#[test]
fn a_lost_connection_is_told_when_it_happens() {
    let d = setup("gone");
    let fake = Fake::default();
    // The greeting, the load, and at the first stop's reload it is gone.
    *fake.close_after.lock().expect("lock") = Some(3);
    let (addr, t) = fake.listen(vec![]);
    let mut s = Session::start(&d, &addr, 200);
    s.send("break tb.vhd:23");
    s.send("run");
    s.expect("The connection to Surfer is gone", SHORT);
    s.send("wave add cnt");
    s.expect("The connection to Surfer is gone", SHORT);
    s.send("print cnt");
    s.expect("\n3\n", SHORT);
    let (status, rest) = s.quit();
    t.join().expect("fake");
    assert_eq!(status, Some(0), "{rest}");
    assert!(rest.contains("W0131"), "{rest}");
    assert!(rest.contains("lost the connection to Surfer"), "{rest}");
}

const STOP_TB: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
entity tb is end;
architecture a of tb is
  signal clk : std_logic := '0';
  signal dout : std_logic_vector(7 downto 0) := (others => 'U');
  signal late : std_logic := '0';
begin
  clk <= not clk after 5 ns;
  reg : process (clk)
  begin
    if rising_edge(clk) then
      dout <= x"26";
    end if;
  end process;
  chk : process
  begin
    wait until falling_edge(clk);
    wait until falling_edge(clk);
    late <= '1';                     -- line 21: stops at 20 ns, clk just fell
    wait for 3 ns;
    late <= '0';                     -- line 23: stops at 23 ns, nothing changed there
    wait for 100 ns;
    std.env.finish;
  end process;
end;
"#;

/// The last time in `file`, and the last value of `name` at or before `time`.
fn wave_at(file: &Path, name: &str, time: u64) -> (u64, String) {
    let mut w = wellen::simple::read(file).expect("a readable file");
    let h = w.hierarchy();
    let var = h
        .all_vars()
        .find(|&v| h[v].full_name(h) == name)
        .unwrap_or_else(|| panic!("no {name}"));
    let r = h[var].signal_ref();
    w.load_signals(&[r]);
    let table = w.time_table();
    let end = *table.last().expect("a time");
    let idx = table.partition_point(|&t| t <= time) - 1;
    let s = w.get_signal(r).expect("signal");
    // As the viewer reads it: the last change of a time.
    let o = s.get_offset(idx as u32).expect("a value");
    (end, s.get_value_at(&o, o.elements - 1).to_string())
}

/// At a stop the file reaches the stop time with what the stopped process reads (#961), also when
/// nothing changed at that time, at a stop between cycles and with a filter; later changes of the
/// same time step win.
#[test]
fn the_file_reaches_a_stop_with_the_values_at_the_stop() {
    let d = tempdir("surfer-stopfile");
    std::fs::write(d.join("tb.vhd"), STOP_TB).expect("write");
    analyze(&d, "tb.vhd");
    let fake = Fake::default();
    let (addr, t) = fake.listen(vec![]);
    let mut s = Session::with(&d, &addr, 5000, &["--wave=w.fst"]);
    let f = d.join("w.fst");
    s.send("break tb.vhd:21");
    s.send("break tb.vhd:23");
    s.send("run");
    s.expect("breakpoint at 20 ns", SHORT);
    assert_eq!(wave_at(&f, "tb.clk", 20_000_000), (20_000_000, "0".into()));
    let (_, dout) = wave_at(&f, "tb.dout", 20_000_000);
    assert_eq!(dout, "00100110");
    s.send("run");
    s.expect("breakpoint at 23 ns", SHORT);
    // `late` changed in a later delta of 20 ns than the stop saw.
    assert_eq!(wave_at(&f, "tb.late", 20_000_000), (23_000_000, "1".into()));
    assert_eq!(wave_at(&f, "tb.clk", 23_000_000).1, "0");
    s.send("run 1 ns");
    s.expect("24 ns", SHORT);
    assert_eq!(wave_at(&f, "tb.clk", 24_000_000).0, 24_000_000);
    s.send("run");
    let (status, rest) = s.quit();
    t.join().expect("fake");
    assert_eq!(status, Some(0), "{rest}");
    assert!(!rest.contains("error"), "{rest}");
    // What changed after a stop at its time is there, and a later step's too.
    assert_eq!(wave_at(&f, "tb.late", 20_000_000).1, "1", "{rest}");
    assert_eq!(wave_at(&f, "tb.late", 24_000_000).1, "0", "{rest}");

    // Only one signal recorded: it marks the time.
    let fake = Fake::default();
    let (addr, t) = fake.listen(vec![]);
    let mut s = Session::with(&d, &addr, 5000, &["--wave=f.fst", "--wave-include=tb.dout"]);
    s.send("break tb.vhd:23");
    s.send("run");
    s.expect("breakpoint at 23 ns", SHORT);
    let f = d.join("f.fst");
    assert_eq!(wave_at(&f, "tb.dout", 23_000_000).0, 23_000_000);
    let (status, rest) = s.quit();
    t.join().expect("fake");
    assert_eq!(status, Some(0), "{rest}");
}

#[test]
fn a_bad_answer_wait_is_e0130() {
    let d = setup("badwait");
    let o = yavs()
        .current_dir(&*d)
        .args([
            "run",
            "--work=work:build/work",
            "tb",
            "--surfer=127.0.0.1:1",
        ])
        .env("YAVS_SURFER_WAIT", "soon")
        .output()
        .expect("yavs");
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("E0130"), "{}", stderr(&o));
    assert!(stderr(&o).contains("YAVS_SURFER_WAIT"), "{}", stderr(&o));
}

/// A Surfer that does not answer at the end does not hold the exit up, and the result is the same.
#[test]
fn a_stalled_viewer_at_the_end_does_not_hold_the_exit() {
    let d = setup("stallend");
    let plain = run(&d, &[]);
    let fake = Fake::default();
    let held = Duration::from_secs(6);
    *fake.stall.lock().expect("lock") = Some(("reload".into(), held));
    let (addr, t) = fake.listen(vec![]);
    let t0 = Instant::now();
    let o = yavs()
        .current_dir(&*d)
        .args(["run", "--work=work:build/work", "tb"])
        .arg(format!("--surfer={addr}"))
        .args(["-W", "error"])
        .env("YAVS_SURFER_WAIT", "200")
        .stdin(Stdio::null())
        .output()
        .expect("yavs");
    assert!(t0.elapsed() < held, "the exit waited for the viewer");
    t.join().expect("fake");
    assert_eq!(stdout(&o), stdout(&plain));
    assert_eq!(o.status.code(), plain.status.code(), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("Surfer did not answer at the end of the run"),
        "{}",
        stderr(&o)
    );
    assert!(stderr(&o).contains("W0131"), "{}", stderr(&o));
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
    // A scope in the file (the record) has its own command.
    assert_eq!(fake.first("add_scope")["scope"], "tb.r");
    let adds = fake.all("add_items");
    let added = |n: &str| adds.iter().any(|a| a["items"] == json!([n]));
    assert!(added("tb.cnt"), "{adds:?}");
    // The markers of a run that starts over go with it.
    assert!(
        !fake.all("remove_items").is_empty(),
        "{:?}",
        fake.commands()
    );
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

/// A debug session records variables without the flag; a plain run does not (the maintainer's
/// rule); `wave add` of one has its history, of a subprogram's local a clear refusal.
#[test]
fn variables_at_a_stop() {
    let d = tempdir("surfer-vars");
    std::fs::create_dir_all(d.join("build")).expect("mkdir");
    std::fs::write(
        d.join("tb.vhd"),
        "entity tb is end;
architecture a of tb is
  signal s : integer := 0;
  function f(x : integer) return integer is
    variable loc : integer;
  begin
    loc := x + 1;
    return loc;
  end function;
begin
  main : process
    variable count : integer := 0;
    variable pair : bit_vector(1 downto 0) := \"01\";
  begin
    for i in 1 to 3 loop
      count := f(count);
      s <= count;
      wait for 10 ns;
    end loop;
    wait;
  end process;
end;
",
    )
    .expect("write");
    analyze(&d, "tb.vhd");
    let plain = run(&d, &["--wave=plain.fst"]);
    assert_eq!(plain.status.code(), Some(0), "{}", stderr(&plain));
    let session = run(&d, &["--wave=debug.fst", "--do=run; quit"]);
    assert_eq!(session.status.code(), Some(0), "{}", stderr(&session));
    let vars = |file: &str| {
        let w = wellen::simple::read(d.join(file)).expect("readable");
        let h = w.hierarchy();
        h.lookup_var(&["tb", "main"], "count").is_some()
    };
    assert!(!vars("plain.fst"), "a plain run records no variables");
    assert!(vars("debug.fst"), "a debug session does");

    let fake = Fake::default();
    let (addr, t) = fake.listen(vec![]);
    let o = run(
        &d,
        &[
            &format!("--surfer={addr}"),
            "--do=break tb.vhd:7; run; wave add loc",
        ],
    );
    t.join().expect("fake");
    assert!(
        stderr(&o).contains("`loc` is a local of a subprogram"),
        "{}",
        stderr(&o)
    );

    let fake = Fake::default();
    let (addr, t) = fake.listen(vec![]);
    let o = run(
        &d,
        &[
            &format!("--surfer={addr}"),
            "--do=break tb.vhd:17; run; wave add count; wave add s; wave add main; wave add tb.main.pair; wave add cnut",
        ],
    );
    t.join().expect("fake");
    let items: Vec<String> = fake
        .all("add_items")
        .iter()
        .flat_map(|a| a["items"].as_array().cloned().unwrap_or_default())
        .map(|i| i.to_string())
        .collect();
    assert!(
        items.iter().any(|i| i.contains("tb.main.count")),
        "{items:?}"
    );
    assert!(items.iter().any(|i| i.contains("tb.s")), "{items:?}");
    assert!(
        items.iter().any(|i| i.contains("tb.main.pair")),
        "{items:?}"
    );
    assert_eq!(fake.first("add_scope")["scope"], "tb.main");
    assert!(
        stderr(&o).contains("is not in the waveform file"),
        "{}",
        stderr(&o)
    );
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
    // The rest of PATH for `sleep`: a fake that ends at once takes its port with it.
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
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
        .env("PATH", &path)
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

/// The real program (opt-in: `YAVS_TEST_SURFER=1`, a display and `surfer` on PATH; it opens a window).
/// Between yavs and Surfer a relay reads Surfer's answers, as the file reader's names are what it
/// finds or not: every item yavs adds must be found.
#[test]
fn real_surfer_finds_what_is_added() {
    let found = std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p)
            .any(|d| d.join("surfer").is_file() || d.join("surfer.exe").is_file())
    });
    if std::env::var_os("YAVS_TEST_SURFER").is_none() || !found {
        return;
    }
    let d = setup("real");
    // Surfer connects to one side, yavs to the other.
    let to_surfer = TcpListener::bind("127.0.0.1:0").expect("bind");
    let to_yavs = TcpListener::bind("127.0.0.1:0").expect("bind");
    let mut surfer = Command::new("surfer")
        .args([
            "--wcp-initiate",
            &to_surfer.local_addr().expect("addr").port().to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("surfer");
    let (s, _) = to_surfer.accept().expect("surfer connects");
    let addr = to_yavs.local_addr().expect("addr");
    let (log_tx, log_rx) = std::sync::mpsc::channel::<Value>();
    let relay = std::thread::spawn(move || {
        let (y, _) = to_yavs.accept().expect("yavs connects");
        let copy =
            |from: TcpStream, mut to: TcpStream, log: Option<std::sync::mpsc::Sender<Value>>| {
                std::thread::spawn(move || {
                    let mut r = BufReader::new(from);
                    let mut b = Vec::new();
                    while r.read_until(0, &mut b).is_ok_and(|n| n > 0) {
                        if let Some(l) = &log
                            && let Ok(m) = serde_json::from_slice::<Value>(&b[..b.len() - 1])
                        {
                            let _ = l.send(m);
                        }
                        if to.write_all(&b).is_err() {
                            break;
                        }
                        b.clear();
                    }
                    let _ = to.shutdown(std::net::Shutdown::Write);
                })
            };
        let a = copy(
            y.try_clone().expect("clone"),
            s.try_clone().expect("clone"),
            Some(log_tx.clone()),
        );
        let b = copy(s, y, Some(log_tx));
        let _ = (a.join(), b.join());
    });
    let o = run(
        &d,
        &[
            &format!("--surfer={addr}"),
            "--do=break tb.vhd:23; run; wave add tb.r; wave add cnt; wave cursor 5 ns; restart; run 10 ns; quit",
        ],
    );
    let _ = surfer.kill();
    let _ = surfer.wait();
    let _ = relay.join();
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert!(!stderr(&o).contains("W0131"), "{}", stderr(&o));
    let log: Vec<Value> = log_rx.try_iter().collect();
    let errors: Vec<&Value> = log.iter().filter(|m| m["type"] == "error").collect();
    assert!(errors.is_empty(), "{errors:?}");
    // Commands and their answers alternate on the connection.
    let mut asked = 0;
    for m in &log {
        if m["command"] == "add_items" && m["type"] == "command" {
            asked = m["items"].as_array().map_or(0, Vec::len);
        }
        if m["command"] == "add_items" && m["type"] == "response" {
            assert_eq!(m["ids"].as_array().map_or(0, Vec::len), asked, "{log:?}");
        }
    }
    assert!(asked > 0, "{log:?}");
}

/// The list of a session comes back in the next one; `wave save` and `wave load` take a file.
#[test]
fn the_wave_layout_survives_a_new_run() {
    let d = setup("layout");
    let session = |cmds: &str, fake: &Fake| {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = l.local_addr().expect("addr");
        let f = fake.clone();
        let t = std::thread::spawn(move || {
            let (s, _) = l.accept().expect("accept");
            f.serve(s, &[]);
        });
        let o = run(&d, &[&format!("--surfer={addr}"), &format!("--do={cmds}")]);
        t.join().expect("fake");
        o
    };
    let first = Fake::default();
    let o = session(
        "run 1 ns; wave add cnt; wave add r; wave save my.layout; wave load nofile; quit",
        &first,
    );
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    assert!(
        stderr(&o).contains("cannot read the wave layout `nofile`"),
        "{}",
        stderr(&o)
    );
    let text = std::fs::read_to_string(d.join("my.layout")).expect("saved");
    assert!(
        text.contains("tb.cnt\n") && text.contains("tb.v\n"),
        "{text}"
    );
    // The file of the testbench is written at the end of a session, on the documented path.
    let file = d.join("build/work/yavs-wave-layout/tb.txt");
    assert!(
        !file.exists()
            || std::fs::read_to_string(&file)
                .expect("read")
                .contains("tb.cnt")
    );
    std::fs::create_dir_all(file.parent().expect("dir")).expect("mkdir");
    std::fs::write(&file, "# a layout\ntb.cnt\ngone.signal\n").expect("write");
    let second = Fake::default();
    let o = session("run 1 ns; run; quit", &second);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(second.first("add_items")["items"], json!(["tb.cnt"]));
    // `wave load` adds a saved list, says what is gone; a misspelled subcommand suggests.
    let third = Fake::default();
    std::fs::write(d.join("my.layout"), "tb.cnt\ngone.signal\n").expect("write");
    let o = session("run 1 ns; wave load my.layout; quit", &third);
    assert!(
        stdout(&o).contains("not in this design: gone.signal"),
        "{}",
        stdout(&o)
    );
    let o = session("wave sav; quit", &Fake::default());
    assert!(stderr(&o).contains("`save`"), "{}", stderr(&o));
}

/// `--surfer` under `--cycle` (owner decision 2026-11-22): the viewer is started as without it, and
/// the file holds the register only the region reads, per time step.
#[test]
fn cycle_mode_shows_the_signals_inside_the_region() {
    const DESIGN: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cyc_dut is
  port (clk : in std_logic; q : out std_logic);
end;
architecture a of cyc_dut is
  signal r : unsigned(3 downto 0) := (others => '0');
begin
  process (clk)
  begin
    if rising_edge(clk) then
      r <= r + 1;
    end if;
  end process;
  q <= r(3);
end;
library ieee;
use ieee.std_logic_1164.all;
entity cyc_tb is end;
architecture a of cyc_tb is
  signal clk, q : std_logic := '0';
begin
  dut : entity work.cyc_dut port map (clk, q);
  p : process
  begin
    for k in 1 to 12 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    report "q=" & std_logic'image(q);
    wait;
  end process;
end;
"#;
    let d = tempdir("surfer-cycle");
    std::fs::write(d.join("cyc.vhd"), DESIGN).expect("write");
    analyze(&d, "cyc.vhd");
    let fake = Fake::default();
    let (addr, t) = fake.listen(vec![]);
    let o = yavs()
        .current_dir(&d)
        .args(["run", "--work=work:build/work", "--cycle"])
        .arg(format!("--surfer={addr}"))
        .arg("cyc_tb")
        .stdin(Stdio::null())
        .output()
        .expect("yavs");
    t.join().expect("fake");
    let err = stderr(&o);
    assert!(o.status.success(), "{err}");
    assert!(
        err.contains("cycle mode: 2 of 3 processes run cycle-based"),
        "{err}"
    );
    assert!(stdout(&o).contains("q='1'"), "{}", stdout(&o));
    assert_eq!(fake.first("load")["command"], "load");
    // `r`: a change at each of the 12 edges.
    let mut w = wellen::simple::read(d.join("build/cyc_tb.fst")).expect("readable");
    let h = w.hierarchy();
    let r = (h.all_vars())
        .find(|&v| h[v].full_name(h).starts_with("cyc_tb.dut.r"))
        .map(|v| h[v].signal_ref())
        .expect("r recorded");
    w.load_signals(&[r]);
    let n = w.get_signal(r).expect("loaded").iter_changes().count();
    assert!(n >= 12, "{n} changes of r");
}
