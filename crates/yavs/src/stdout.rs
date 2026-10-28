//! Standard output of a run. The VHDL design is the only writer on a pipe or file when no
//! plugin or foreign library is loaded, so its report and `OUTPUT` text is collected and
//! written in blocks (a write per `report` line costs a system call each). Every other writer
//! (diagnostics on standard error, the final status lines) calls [`flush`] first, so the order
//! of the two streams is unchanged; the signal thread (`yavs_wave::interrupt`, which also
//! handles Ctrl-C and SIGUSR1) writes out what is waiting every [`TICK`], so a run that goes
//! quiet or is killed shows its output without a long delay. A terminal, a
//! plugin or a foreign library keeps the line-by-line writes.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

/// When a write to standard output first failed (the reader closed the pipe, a full disk):
/// the run is stopped, and output after that is dropped (#484).
static BROKEN: OnceLock<Instant> = OnceLock::new();
static STOP: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
/// Exit status of a run whose standard output broke (128 + SIGPIPE, as a shell shows it).
pub const BROKEN_STATUS: u8 = 141;
/// A run that is still writing this long after the failure is not between cycles: exit at once.
const GRACE: Duration = Duration::from_secs(1);

static ON: AtomicBool = AtomicBool::new(false);
static BUF: Mutex<Vec<u8>> = Mutex::new(Vec::new());
const BLOCK: usize = 1 << 16;

fn buf() -> MutexGuard<'static, Vec<u8>> {
    BUF.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Write `bytes` and flush; the first failure stops the run, later output is dropped.
fn put(bytes: &[u8]) {
    if let Some(t) = BROKEN.get() {
        if t.elapsed() > GRACE {
            crate::jobserver::close();
            std::process::exit(BROKEN_STATUS.into());
        }
        return;
    }
    let mut o = std::io::stdout().lock();
    if let Err(e) = o.write_all(bytes).and_then(|()| o.flush())
        && BROKEN.set(Instant::now()).is_ok()
    {
        let _ = writeln!(
            std::io::stderr(),
            "yavs: cannot write to standard output ({e}): stopping the run"
        );
        if let Some(stop) = STOP.get() {
            stop();
        }
    }
}

fn drain(b: &mut Vec<u8>) {
    if !b.is_empty() {
        put(b);
        b.clear();
    }
}

/// Call `stop` (end the run between cycles) when writing to standard output fails.
pub fn on_broken(stop: impl Fn() + Send + Sync + 'static) {
    let _ = STOP.set(Box::new(stop));
}

/// Whether writing to standard output failed.
pub fn is_broken() -> bool {
    BROKEN.get().is_some()
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
}

/// How often the signal thread writes out what waits (`yavs_wave::interrupt::Handlers::tick`).
pub const TICK: Duration = Duration::from_millis(100);

/// Stop collecting (the thread that would write the output out could not be started).
pub fn unbuffer() {
    flush();
    ON.store(false, Ordering::Relaxed);
}

/// Whether output is being collected.
pub fn is_on() -> bool {
    ON.load(Ordering::Relaxed)
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
        put(bytes);
    }
}

/// Write out what is collected. Call before writing to standard output or error directly.
pub fn flush() {
    if ON.load(Ordering::Relaxed) {
        drain(&mut buf());
    }
}
