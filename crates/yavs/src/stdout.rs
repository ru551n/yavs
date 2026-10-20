//! Standard output of a run. The VHDL design is the only writer on a pipe or file when no
//! plugin or foreign library is loaded, so its report and `OUTPUT` text is collected and
//! written in blocks (a write per `report` line costs a system call each). Every other writer
//! (diagnostics on standard error, the final status lines) calls [`flush`] first, so the order
//! of the two streams is unchanged; a thread writes out what is waiting every 100 ms, so a
//! run that goes quiet or is killed shows its output without a long delay. A terminal, a
//! plugin or a foreign library keeps the line-by-line writes.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

static ON: AtomicBool = AtomicBool::new(false);
static BUF: Mutex<Vec<u8>> = Mutex::new(Vec::new());
const BLOCK: usize = 1 << 16;

fn buf() -> MutexGuard<'static, Vec<u8>> {
    BUF.lock().unwrap_or_else(PoisonError::into_inner)
}

fn drain(b: &mut Vec<u8>) {
    if !b.is_empty() {
        let mut o = std::io::stdout().lock();
        let _ = o.write_all(b);
        let _ = o.flush();
        b.clear();
    }
}

/// Start collecting output, unless standard output is a terminal.
pub fn buffer() {
    if std::io::stdout().is_terminal() || ON.swap(true, Ordering::Relaxed) {
        ON.store(false, Ordering::Relaxed);
        return;
    }
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Ok(mut b) = BUF.try_lock() {
            drain(&mut b);
        }
        prev(info);
    }));
    let spawned = std::thread::Builder::new()
        .name("stdout-flush".into())
        .spawn(|| {
            loop {
                std::thread::sleep(Duration::from_millis(100));
                drain(&mut buf());
            }
        });
    if spawned.is_err() {
        ON.store(false, Ordering::Relaxed);
    }
}

/// Write `bytes` to standard output (flushed at once unless [`buffer`] is on).
pub fn write(bytes: &[u8]) {
    if ON.load(Ordering::Relaxed) {
        let mut b = buf();
        b.extend_from_slice(bytes);
        if b.len() >= BLOCK {
            drain(&mut b);
        }
    } else {
        let mut o = std::io::stdout().lock();
        let _ = o.write_all(bytes);
        let _ = o.flush();
    }
}

/// Write out what is collected. Call before writing to standard output or error directly.
pub fn flush() {
    if ON.load(Ordering::Relaxed) {
        drain(&mut buf());
    }
}
