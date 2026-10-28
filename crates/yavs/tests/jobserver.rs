//! `yavs run --threads=N` under a token pool passed in `MAKEFLAGS` (`docs/parallel.md`
//! "Thread budget across parallel test runs"): tokens are taken only up to N - 1, all come
//! back after a normal end, a failure and Ctrl-C, an empty pool runs on one thread, and the
//! output is the same at any token count.
#![cfg(target_os = "linux")]

mod common;

use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
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

/// The pool: an anonymous pipe passed as descriptors 3 and 4 (`--jobserver-auth=3,4`) or a named
/// pipe (`fifo:PATH`), with a non-blocking handle of it to fill and count.
struct Pool {
    pipe: Option<(std::io::PipeReader, std::io::PipeWriter)>,
    fifo: Option<PathBuf>,
    h: std::fs::File,
}

fn nonblocking(path: impl AsRef<Path>) -> std::fs::File {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .expect("pool handle")
}

impl Pool {
    fn pipe(tokens: usize) -> Pool {
        let (reader, writer) = std::io::pipe().expect("pipe");
        let h = nonblocking(format!("/proc/self/fd/{}", reader.as_raw_fd()));
        Pool {
            pipe: Some((reader, writer)),
            fifo: None,
            h,
        }
        .filled(tokens)
    }

    fn fifo(dir: &Path, tokens: usize) -> Pool {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = dir.join(format!("pool{}", N.fetch_add(1, Ordering::Relaxed)));
        let st = Command::new("mkfifo").arg(&path).status().expect("mkfifo");
        assert!(st.success());
        Pool {
            pipe: None,
            h: nonblocking(&path),
            fifo: Some(path),
        }
        .filled(tokens)
    }

    fn new(fifo: bool, dir: &Path, tokens: usize) -> Pool {
        if fifo {
            Pool::fifo(dir, tokens)
        } else {
            Pool::pipe(tokens)
        }
    }

    fn filled(mut self, tokens: usize) -> Pool {
        self.h.write_all(&vec![b'+'; tokens]).expect("fill");
        self
    }

    /// The tokens in the pool now (read out and written back).
    fn count(&mut self) -> usize {
        let mut buf = [0u8; 256];
        let n = self.h.read(&mut buf).unwrap_or(0);
        self.h.write_all(&buf[..n]).expect("refill");
        n
    }
}

fn yavs() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
    c.args(["--color", "never"]);
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

/// `yavs run` under `pool` (none: no `MAKEFLAGS`).
fn spawn(lib: &str, pool: Option<&Pool>, threads: &str, extra: &[&str]) -> Child {
    let mut c = Command::new("bash");
    c.arg("-c")
        .arg("exec 3<&0 4<&0 0</dev/null; exec \"$@\"")
        .arg("bash")
        .arg(env!("CARGO_BIN_EXE_yavs"))
        .args(["--color", "never", "run", lib, threads])
        .args(extra)
        .env("YAVS_POOL_ALL", "1")
        .env("YAVS_COUNTERS", "1")
        .env_remove("MAKEFLAGS")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(p) = pool {
        if let Some(path) = &p.fifo {
            c.env(
                "MAKEFLAGS",
                format!(" -j8 --jobserver-auth=fifo:{}", path.display()),
            );
        }
        if let Some((r, _)) = &p.pipe {
            c.stdin(r.try_clone().expect("clone"))
                .env("MAKEFLAGS", " -j8 --jobserver-auth=3,4");
        }
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

/// Tokens back after the end and a failure, and the same output at any token count.
fn tokens_come_back(fifo: bool) {
    let t = common::tempdir("jobserver");
    let lib = analyze(&t);
    let one = spawn(&lib, None, "--threads=1", &["tok"])
        .wait_with_output()
        .expect("run");
    assert!(one.status.success());
    assert!(String::from_utf8_lossy(&one.stdout).contains("sum "));
    // Without a pool `--threads=4` is as before; with one, every token count gives the same.
    let free = spawn(&lib, None, "--threads=4", &["tok"])
        .wait_with_output()
        .expect("run");
    assert_eq!(free.stdout, one.stdout);
    assert!(counter(&free, "pooled_batches") > 0);
    for tokens in [0, 1, 3, 8] {
        let mut pool = Pool::new(fifo, &t, tokens);
        let o = spawn(&lib, Some(&pool), "--threads=4", &["tok"])
            .wait_with_output()
            .expect("run");
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        assert!(!String::from_utf8_lossy(&o.stderr).contains("E0128"));
        assert_eq!(o.stdout, one.stdout, "{tokens} tokens");
        assert_eq!(pool.count(), tokens, "tokens back after the end");
        let pooled = counter(&o, "pooled_batches");
        if tokens == 0 {
            assert_eq!(pooled, 0, "an empty pool runs every batch on one thread");
        } else {
            assert!(pooled > 0, "{tokens} tokens");
        }
    }
    // A failure ends the run with exit 1: the tokens come back too.
    let mut pool = Pool::new(fifo, &t, 3);
    let o = spawn(&lib, Some(&pool), "--threads=4", &["tok", "-gfail=true"])
        .wait_with_output()
        .expect("run");
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(pool.count(), 3, "tokens back after a failure");
}

#[test]
fn tokens_come_back_to_a_named_pipe() {
    tokens_come_back(true);
}

#[test]
fn tokens_come_back_to_inherited_descriptors() {
    tokens_come_back(false);
}

/// Wait until the run holds tokens (never more than `threads - 1`), interrupt it, and check
/// that every token is back.
fn interrupted(fifo: bool, wave: bool) {
    let t = common::tempdir("jobserver-int");
    let lib = analyze(&t);
    let mut pool = Pool::new(fifo, &t, 8);
    let wave_arg = format!("--wave={}", t.join("w.fst").display());
    let mut args = vec!["tok", "-gforever=true"];
    if wave {
        args.push(&wave_arg);
    }
    let mut child = spawn(&lib, Some(&pool), "--threads=3", &args);
    let start = Instant::now();
    loop {
        let n = pool.count();
        assert!(n >= 6, "took {} tokens with --threads=3", 8 - n);
        if n < 8 {
            break;
        }
        assert!(child.try_wait().expect("wait").is_none(), "ended early");
        assert!(start.elapsed() < Duration::from_secs(60), "no token taken");
        std::thread::sleep(Duration::from_millis(5));
    }
    let st = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("kill");
    assert!(st.success());
    let o = child.wait_with_output().expect("wait");
    assert!(!o.status.success(), "an interrupted run fails");
    assert_eq!(pool.count(), 8, "tokens back after Ctrl-C");
}

#[test]
fn ctrl_c_gives_the_tokens_back() {
    for fifo in [true, false] {
        interrupted(fifo, false);
        interrupted(fifo, true);
    }
}

#[test]
fn an_unusable_pool_is_an_error() {
    let t = common::tempdir("jobserver-bad");
    let lib = analyze(&t);
    std::fs::write(t.join("plain"), "").expect("write");
    let plain = format!("fifo:{}", t.join("plain").display());
    let run = |args: &[&str], auth: &str| {
        yavs()
            .arg("run")
            .args(args)
            .args([lib.as_str(), "tok"])
            .env("MAKEFLAGS", format!("-j --jobserver-auth={auth}"))
            .stdin(Stdio::null())
            .output()
            .expect("run")
    };
    // Descriptors the parent closed, a path that is not a named pipe, a form with no client.
    for auth in ["97,98", &plain, "sem_name"] {
        let o = run(&["--threads=4"], auth);
        assert_eq!(o.status.code(), Some(2));
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(err.contains("error[E0128]"), "{err}");
        assert!(err.contains(&format!("--jobserver-auth={auth}")), "{err}");
        assert!(err.contains("removes `--jobserver-auth`"), "{err}");
        assert!(o.stdout.is_empty(), "nothing ran");
    }
    // Not read at all with one thread or under the debugger (which then starts: W0109).
    let o = run(&["--threads=1"], "97,98");
    assert!(o.status.success());
    assert!(!String::from_utf8_lossy(&o.stderr).contains("E0128"));
    let o = run(&["--threads=4", "--dap=stdio"], "97,98");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(!err.contains("E0128") && err.contains("W0109"), "{err}");
}
