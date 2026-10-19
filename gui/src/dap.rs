//! DAP client: the transport to `yavs run --dap` (a child over stdio, or TCP for `--attach`),
//! message framing and the reader threads (docs/design/gui.md section 2).

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// Where the simulation comes from.
pub enum Target {
    /// Start `yavs run <args> --dap=stdio`; `yavs` is the binary given by `--yavs`, if any.
    Launch {
        yavs: Option<PathBuf>,
        args: Vec<String>,
    },
    /// Connect to a `yavs run --dap=PORT` at `HOST:PORT`.
    Attach(String),
}

/// What the reader threads hand to the UI thread.
pub enum Incoming {
    Message(Value),
    /// A line the child wrote to stderr.
    Stderr(String),
    /// The DAP stream ended; the text says why.
    Closed(String),
}

/// Called by the reader threads after each message, so the UI repaints.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

pub struct Session {
    writer: Box<dyn Write + Send>,
    child: Option<Child>,
    seq: i64,
    pub rx: Receiver<Incoming>,
}

impl Session {
    pub fn start(target: &Target, wake: Wake) -> Result<Session, String> {
        let (tx, rx) = channel();
        let (reader, writer, child): (Box<dyn Read + Send>, Box<dyn Write + Send>, _) = match target
        {
            Target::Attach(addr) => {
                let s = TcpStream::connect(addr)
                    .map_err(|e| format!("cannot connect to `{addr}`: {e}"))?;
                let w = s.try_clone().map_err(|e| e.to_string())?;
                (Box::new(s), Box::new(w), None)
            }
            Target::Launch { yavs, args } => {
                let bin = yavs.clone().unwrap_or_else(default_yavs);
                let mut child = Command::new(&bin)
                    .arg("run")
                    .args(args)
                    .arg("--dap=stdio")
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|e| match (e.kind(), yavs) {
                        (io::ErrorKind::NotFound, None) => "cannot find `yavs`: not given with \
                         --yavs, not next to the yavs-gui executable and not on PATH"
                            .to_string(),
                        _ => format!("cannot start `{}`: {e}", bin.display()),
                    })?;
                let err = child.stderr.take().expect("stderr is piped");
                let (tx, wake) = (tx.clone(), wake.clone());
                thread::spawn(move || {
                    for line in BufReader::new(err).lines().map_while(Result::ok) {
                        let _ = tx.send(Incoming::Stderr(line));
                        wake();
                    }
                });
                let out = child.stdout.take().expect("stdout is piped");
                let inp = child.stdin.take().expect("stdin is piped");
                (Box::new(out), Box::new(inp), Some(child))
            }
        };
        thread::spawn(move || read_loop(reader, tx, wake));
        Ok(Session {
            writer,
            child,
            seq: 0,
            rx,
        })
    }

    /// Send a request; returns its `seq`.
    pub fn request(&mut self, command: &str, arguments: Value) -> io::Result<i64> {
        self.seq += 1;
        let m =
            json!({"seq": self.seq, "type": "request", "command": command, "arguments": arguments});
        write_message(&mut self.writer, &m)?;
        Ok(self.seq)
    }
}

impl Drop for Session {
    /// Ends the session: `disconnect`, then the child gets 2 s to exit before it is killed.
    fn drop(&mut self) {
        let _ = self.request("disconnect", json!({}));
        let Some(child) = &mut self.child else {
            return;
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = child.try_wait() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// `yavs` next to this executable, else `yavs` on PATH.
fn default_yavs() -> PathBuf {
    let name = format!("yavs{}", std::env::consts::EXE_SUFFIX);
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(&name)))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

fn read_loop(r: Box<dyn Read + Send>, tx: Sender<Incoming>, wake: Wake) {
    let mut r = BufReader::new(r);
    let why = loop {
        match read_message(&mut r) {
            Ok(Some(m)) => {
                if tx.send(Incoming::Message(m)).is_err() {
                    return;
                }
                wake();
            }
            Ok(None) => break "the debug adapter closed the connection".to_string(),
            Err(e) => break format!("debug adapter connection: {e}"),
        }
    };
    let _ = tx.send(Incoming::Closed(why));
    wake();
}

/// The next `Content-Length` framed message; `None` at the end of input.
pub fn read_message(r: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut len = None;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() && len.is_some() {
            break;
        }
        if let Some((k, v)) = line.split_once(':')
            && k.trim().eq_ignore_ascii_case("content-length")
        {
            len = Some(v.trim().parse::<usize>().map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, format!("Content-Length: {e}"))
            })?);
        }
    }
    let mut body = vec![0; len.unwrap_or(0)];
    r.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn write_message(w: &mut (impl Write + ?Sized), m: &Value) -> io::Result<()> {
    let body = m.to_string();
    write!(w, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    w.flush()
}
