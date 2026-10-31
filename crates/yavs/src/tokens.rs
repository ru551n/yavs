//! The test runner's token server, named in `YAVS_TOKENS=<port>:<secret>`: the threads of
//! `--threads` above the first run only on tokens it lends (`docs/parallel.md`; the protocol is
//! in `docs/cli-contract.md`). The connection closing, at any exit, gives every token back.

use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

pub(crate) const VAR: &str = "YAVS_TOKENS";
/// For the connect and the server's answer to the secret, at the start only.
const TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct Pool {
    state: Mutex<State>,
}

struct State {
    /// `None` once the server is gone: no tokens from then on.
    sock: Option<TcpStream>,
    /// The start of a line not yet complete.
    buf: Vec<u8>,
    /// Messages sent; the secret is message 0, each `<want>` the next.
    sent: u64,
    want: usize,
    /// Tokens lent for message `sent`: lines answering an older message are stale.
    allot: usize,
}

impl State {
    fn send(&mut self, want: usize) {
        self.sent += 1;
        self.want = want;
        self.allot = 0;
        let msg = format!("{want}\n");
        if !self
            .sock
            .as_mut()
            .is_some_and(|s| s.write_all(msg.as_bytes()).is_ok())
        {
            self.sock = None;
        }
    }

    /// Read every `<message> <tokens>` line the server sent, without blocking.
    fn poll(&mut self) {
        let mut chunk = [0u8; 256];
        while let Some(s) = self.sock.as_mut() {
            match s.read(&mut chunk) {
                Ok(n) if n > 0 => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                // The server is gone (end of stream or an error): the run goes on without tokens.
                _ => {
                    self.sock = None;
                    self.allot = 0;
                    return;
                }
            }
        }
        while let Some(end) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=end).collect();
            if let Some((k, n)) = parse(&line)
                && k == self.sent
            {
                self.allot = n;
            }
        }
    }
}

fn parse(line: &[u8]) -> Option<(u64, usize)> {
    let (k, n) = std::str::from_utf8(line).ok()?.trim().split_once(' ')?;
    Some((k.parse().ok()?, n.parse().ok()?))
}

impl Pool {
    /// The server is gone: the connection closed or a write to it failed.
    pub(crate) fn lost(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .sock
            .is_none()
    }
}

impl yavs_kernel::Tokens for Pool {
    fn take(&self, want: usize) -> usize {
        let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if st.want != want {
            st.send(want);
        }
        st.poll();
        st.allot.min(want)
    }

    fn give(&self) {
        let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if st.want != 0 {
            st.send(0);
        }
    }
}

/// What `connect` found.
pub(crate) enum Server {
    /// `YAVS_TOKENS` is not set.
    None,
    Pool(Arc<Pool>),
    /// Nothing answers at the address (warning W0118: the run is on one thread); the note says why.
    Unreachable(String),
}

/// The token server named in `YAVS_TOKENS`; `Err` is the note of error E0128 (a value that is not
/// `<port>:<secret>`, or a secret the server does not accept).
pub(crate) fn connect() -> Result<Server, String> {
    let Some(value) = std::env::var_os(VAR) else {
        return Ok(Server::None);
    };
    // The secret is never printed.
    let (port, secret) = value
        .to_str()
        .and_then(|v| v.split_once(':'))
        .and_then(|(p, s)| Some((p.parse::<u16>().ok().filter(|&p| p != 0)?, s)))
        .filter(|(_, s)| !s.is_empty() && s.bytes().all(|b| b.is_ascii_graphic()))
        .ok_or_else(|| format!("`{VAR}` is not `<port>:<secret>`"))?;
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let fail = |e: std::io::Error| format!("the token server on {addr}: {e}");
    // Tokens only speed a run up: a server that cannot be reached leaves the run on one thread.
    let gone = |e: std::io::Error| Ok(Server::Unreachable(fail(e)));
    let mut s = match TcpStream::connect_timeout(&addr, TIMEOUT) {
        Ok(s) => s,
        Err(e) => return gone(e),
    };
    // A token line must not wait for more bytes to fill a packet.
    if let Err(e) = s
        .set_nodelay(true)
        .and_then(|()| s.set_read_timeout(Some(TIMEOUT)))
        .and_then(|()| s.write_all(format!("{secret}\n").as_bytes()))
    {
        return gone(e);
    }
    // The server answers the secret with `0 0` (message 0, no tokens), or closes the connection.
    let mut answer = Vec::new();
    let mut byte = [0u8];
    while answer.last() != Some(&b'\n') && answer.len() < 32 {
        match s.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => answer.push(byte[0]),
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                return Ok(Server::Unreachable(format!(
                    "the token server on {addr} did not answer within {} s",
                    TIMEOUT.as_secs()
                )));
            }
            Err(e) => return gone(e),
        }
    }
    if parse(&answer) != Some((0, 0)) {
        return Err(format!(
            "the token server on {addr} did not accept the secret (a stale `{VAR}` from a run that ended?)"
        ));
    }
    if let Err(e) = s.set_nonblocking(true) {
        return gone(e);
    }
    Ok(Server::Pool(Arc::new(Pool {
        state: Mutex::new(State {
            sock: Some(s),
            buf: Vec::new(),
            sent: 0,
            want: 0,
            allot: 0,
        }),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use yavs_kernel::Tokens;

    /// An answer to an older `<want>` (sent before a `0`) is never used.
    #[test]
    fn stale_answers_are_ignored() {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind");
        let c = TcpStream::connect(l.local_addr().expect("addr")).expect("connect");
        c.set_nonblocking(true).expect("nonblocking");
        let (mut srv, _) = l.accept().expect("accept");
        let pool = Pool {
            state: Mutex::new(State {
                sock: Some(c),
                buf: Vec::new(),
                sent: 0,
                want: 0,
                allot: 0,
            }),
        };
        assert_eq!(pool.take(3), 0);
        pool.give();
        assert_eq!(pool.take(3), 0);
        let mut lines = BufReader::new(srv.try_clone().expect("clone")).lines();
        let got: Vec<String> = (0..3).map(|_| lines.next().unwrap().unwrap()).collect();
        assert_eq!(got, ["3", "0", "3"]);
        // Line 1's grant arrives late, then line 3's, split across writes.
        srv.write_all(b"1 3\n3 ").expect("write");
        srv.flush().expect("flush");
        let start = std::time::Instant::now();
        while pool.state.lock().unwrap().buf.is_empty() {
            assert!(start.elapsed() < TIMEOUT);
            assert_eq!(pool.take(3), 0, "a stale answer was used");
        }
        srv.write_all(b"1\n").expect("write");
        let mut n = 0;
        while n == 0 {
            assert!(start.elapsed() < TIMEOUT);
            n = pool.take(3);
        }
        assert_eq!(n, 1);
        // The server going away leaves the run without tokens.
        drop((srv, lines));
        while pool.state.lock().unwrap().sock.is_some() {
            assert!(start.elapsed() < TIMEOUT);
            pool.take(3);
        }
        assert_eq!(pool.take(3), 0);
    }
}
