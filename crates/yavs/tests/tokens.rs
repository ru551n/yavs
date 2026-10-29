//! `yavs run --threads=N` under a token server named in `YAVS_TOKENS` (`docs/parallel.md`
//! "Thread budget across parallel test runs"): at most N - 1 tokens are held, the connection
//! closes (every token back) after a normal end, a failure, Ctrl-C and a kill, no token runs on
//! one thread, and the output is the same at any token count.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 64 registers on one clock, so every rising edge is a batch worth pooling (with
/// `YAVS_POOL_ALL=1`); `forever` keeps the clock running until the run is interrupted.
const DESIGN: &str = r#"
entity tok is
  generic (cycles : natural := 300; forever : boolean := false; fail : boolean := false);
end;
architecture a of tok is
  signal clk : bit := '0';
  type arr is array (0 to 63) of natural;
  signal r : arr := (others => 0);
begin
  clkp : process
    variable n : natural := 0;
  begin
    while forever or n < 2 * cycles loop
      wait for 5 ns;
      clk <= not clk;
      n := n + 1;
    end loop;
    wait;
  end process;
  g : for i in 0 to 63 generate
    process (clk) begin
      if clk'event and clk = '1' then
        r(i) <= (r(i) * 7 + i + 1) mod 65521;
      end if;
    end process;
  end generate;
  chk : process
    variable s : natural := 0;
  begin
    wait for 10 ns * cycles + 1 ns;
    for i in 0 to 63 loop
      s := (s * 31 + r(i)) mod 1000003;
    end loop;
    report "sum " & integer'image(s);
    assert not fail report "requested failure" severity failure;
    wait;
  end process;
end;
"#;

const SECRET: &str = "0123456789abcdef";

#[derive(Default)]
struct State {
    /// Connections that sent the secret and are still open.
    open: usize,
    /// Connections that sent the secret.
    accepted: usize,
    /// Tokens lent now and the most at any time.
    held: usize,
    max_held: usize,
}

/// A token server that lends every connection up to `tokens` at once.
struct Server {
    port: u16,
    st: Arc<Mutex<State>>,
}

impl Server {
    fn new(tokens: usize) -> Server {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = l.local_addr().expect("addr").port();
        let st = Arc::new(Mutex::new(State::default()));
        let s = st.clone();
        // ponytail: the accept thread outlives the test; the test process ends anyway.
        std::thread::spawn(move || {
            for c in l.incoming().flatten() {
                let s = s.clone();
                std::thread::spawn(move || serve(c, tokens, &s));
            }
        });
        Server { port, st }
    }

    fn env(&self) -> String {
        format!("{}:{SECRET}", self.port)
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.st.lock().expect("lock")
    }

    /// Wait until `f` holds (the server sees a close a little after the process ends).
    fn wait_for(&self, what: &str, f: impl Fn(&State) -> bool) {
        let start = Instant::now();
        while !f(&self.state()) {
            assert!(start.elapsed() < Duration::from_secs(60), "{what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn closed(&self) {
        self.wait_for("the connection closes", |s| s.open == 0);
        assert_eq!(self.state().held, 0);
    }
}

fn serve(c: TcpStream, tokens: usize, st: &Mutex<State>) {
    c.set_nodelay(true).expect("nodelay");
    let mut w = c.try_clone().expect("clone");
    let mut lines = BufReader::new(c).lines();
    if !matches!(lines.next(), Some(Ok(l)) if l == SECRET) || w.write_all(b"0 0\n").is_err() {
        return;
    }
    {
        let mut s = st.lock().expect("lock");
        s.open += 1;
        s.accepted += 1;
    }
    let mut mine = 0;
    // An error (a reset by a killed client) is a close as much as the end of the stream.
    for (k, line) in (1u64..).zip(lines.map_while(Result::ok)) {
        let want: usize = line.parse().expect("a want line");
        let mut s = st.lock().expect("lock");
        s.held = s.held - mine + want.min(tokens);
        mine = want.min(tokens);
        s.max_held = s.max_held.max(s.held);
        let _ = w.write_all(format!("{k} {mine}\n").as_bytes());
    }
    let mut s = st.lock().expect("lock");
    s.held -= mine;
    s.open -= 1;
}

fn yavs() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
    c.args(["--color", "never"]).env_remove("YAVS_TOKENS");
    c
}

fn analyze(dir: &Path) -> String {
    std::fs::write(dir.join("tok.vhd"), DESIGN).expect("write");
    let lib = format!("--work=work:{}", dir.join("lib").display());
    let o = yavs()
        .args(["analyze", "--std=2008", &lib])
        .arg(dir.join("tok.vhd"))
        .output()
        .expect("analyze");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    lib
}

/// `yavs run` under `server` (none: no `YAVS_TOKENS`).
fn spawn(lib: &str, server: Option<&Server>, threads: &str, extra: &[&str]) -> Child {
    let mut c = yavs();
    c.args(["run", lib, threads])
        .args(extra)
        .env("YAVS_POOL_ALL", "1")
        .env("YAVS_COUNTERS", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(s) = server {
        c.env("YAVS_TOKENS", s.env());
    }
    c.spawn().expect("spawn")
}

fn counter(o: &Output, name: &str) -> u64 {
    let err = String::from_utf8_lossy(&o.stderr);
    let key = format!("counters: {name}=");
    err.lines()
        .find_map(|l| l.strip_prefix(&key))
        .unwrap_or_else(|| panic!("no {name} in\n{err}"))
        .parse()
        .expect("number")
}

#[test]
fn tokens_come_back_and_output_is_the_same() {
    let t = common::tempdir("tokens");
    let lib = analyze(&t);
    let one = spawn(&lib, None, "--threads=1", &["tok"])
        .wait_with_output()
        .expect("run");
    assert!(one.status.success());
    assert!(String::from_utf8_lossy(&one.stdout).contains("sum "));
    // Without a server `--threads=4` is as before; with one, every token count gives the same.
    let free = spawn(&lib, None, "--threads=4", &["tok"])
        .wait_with_output()
        .expect("run");
    assert_eq!(free.stdout, one.stdout);
    assert!(counter(&free, "pooled_batches") > 0);
    for tokens in [0, 1, 3, 8] {
        let s = Server::new(tokens);
        let o = spawn(&lib, Some(&s), "--threads=4", &["tok"])
            .wait_with_output()
            .expect("run");
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        assert_eq!(o.stdout, one.stdout, "{tokens} tokens");
        s.closed();
        let st = s.state();
        assert_eq!(st.accepted, 1);
        assert!(st.max_held <= tokens.min(3), "held {}", st.max_held);
        let pooled = counter(&o, "pooled_batches");
        if tokens == 0 {
            assert_eq!(pooled, 0, "no token runs every batch on one thread");
        } else {
            assert!(pooled > 0, "{tokens} tokens");
            assert_eq!(st.max_held, tokens.min(3));
        }
    }
    // A failure ends the run with exit 1: the connection closes too.
    let s = Server::new(3);
    let o = spawn(&lib, Some(&s), "--threads=4", &["tok", "-gfail=true"])
        .wait_with_output()
        .expect("run");
    assert_eq!(o.status.code(), Some(1));
    s.closed();
}

/// Start an endless run with `--threads=3` and wait until it holds tokens.
fn holding(lib: &str, extra: &[&str]) -> (Server, Child) {
    let s = Server::new(8);
    let mut args = vec!["tok", "-gforever=true"];
    args.extend_from_slice(extra);
    let mut child = spawn(lib, Some(&s), "--threads=3", &args);
    let start = Instant::now();
    while s.state().held == 0 {
        assert!(child.try_wait().expect("wait").is_none(), "ended early");
        assert!(start.elapsed() < Duration::from_secs(60), "no token taken");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(s.state().max_held <= 2, "more than --threads=3 - 1");
    (s, child)
}

#[test]
fn a_killed_run_gives_the_tokens_back() {
    let t = common::tempdir("tokens-kill");
    let lib = analyze(&t);
    let (s, mut child) = holding(&lib, &[]);
    child.kill().expect("kill");
    let _ = child.wait();
    s.closed();
}

#[cfg(unix)]
#[test]
fn ctrl_c_and_sigterm_give_the_tokens_back() {
    let t = common::tempdir("tokens-int");
    let lib = analyze(&t);
    let wave = format!("--wave={}", t.join("w.fst").display());
    for (sig, extra) in [("-INT", None), ("-INT", Some(&wave)), ("-TERM", None)] {
        let extra: Vec<&str> = extra.map(String::as_str).into_iter().collect();
        let (s, child) = holding(&lib, &extra);
        let st = Command::new("kill")
            .args([sig, &child.id().to_string()])
            .status()
            .expect("kill");
        assert!(st.success());
        let o = child.wait_with_output().expect("wait");
        assert!(!o.status.success(), "an interrupted run fails");
        s.closed();
    }
}

#[test]
fn an_unusable_token_server_is_an_error() {
    let t = common::tempdir("tokens-bad");
    let lib = analyze(&t);
    let run = |args: &[&str], value: &str| {
        yavs()
            .arg("run")
            .args(args)
            .args([lib.as_str(), "tok"])
            .env("YAVS_TOKENS", value)
            .stdin(Stdio::null())
            .output()
            .expect("run")
    };
    // A port nothing listens on, and a server that does not know the secret.
    let gone = TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port();
    let s = Server::new(8);
    let bad = [
        ("", "is not `<port>:<secret>`"),
        ("4711", "is not `<port>:<secret>`"),
        ("0:abc", "is not `<port>:<secret>`"),
        ("70000:abc", "is not `<port>:<secret>`"),
        ("4711:", "is not `<port>:<secret>`"),
        (&format!("{gone}:abc"), "the token server on 127.0.0.1:"),
        (&format!("{}:wrong", s.port), "did not accept the secret"),
    ];
    for (value, why) in bad {
        let o = run(&["--threads=4"], value);
        assert_eq!(o.status.code(), Some(2), "{value}");
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(err.contains("error[E0128]"), "{err}");
        assert!(err.contains(why), "{value}: {err}");
        assert!(err.contains("unset it to run without one"), "{err}");
        assert!(!err.contains("wrong"), "the secret is never printed: {err}");
        assert!(o.stdout.is_empty(), "nothing ran");
    }
    assert_eq!(s.state().accepted, 0);
    // Not read at all with one thread or under the debugger (which then starts: W0109).
    let o = run(&["--threads=1"], "junk");
    assert!(o.status.success());
    assert!(!String::from_utf8_lossy(&o.stderr).contains("E0128"));
    let o = run(&["--threads=4", "--dap=stdio"], "junk");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(!err.contains("E0128") && err.contains("W0109"), "{err}");
}
