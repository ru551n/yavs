//! SIGINT/SIGTERM (and Ctrl-C on Windows) support, and the other things a run waits for on one thread.
//!
//! One helper thread serves: the first signal calls `first` (`yavs run` passes the kernel's
//! `Interrupt::stop`: the kernel ends between cycles and the recorder finishes the file), a second
//! signal runs `at_exit` and exits at once with status 1 (a process stuck inside one cycle never
//! polls; the file is then readable up to its last flush), SIGUSR1 (Unix) calls `usr1`, and
//! `tick` runs every period (the output flusher of `yavs run`), and `after` runs once, after a delay
//! (the start of the compile thread, which a run of a few milliseconds never needs). Each of them
//! costs a thread otherwise, and a thread is about 5% of a run of a millisecond (#562).

use std::time::Duration;

type Hook = Box<dyn Fn() + Send>;

/// What the thread does; every part is optional (a signal with no hook keeps its default action).
#[derive(Default)]
pub struct Handlers {
    /// The first SIGINT, SIGTERM or SIGHUP (Ctrl-C on Windows).
    pub first: Option<Hook>,
    /// Runs before the exit on the second one (to write out what is buffered).
    pub at_exit: Option<Hook>,
    /// SIGUSR1; nothing on Windows.
    pub usr1: Option<Hook>,
    /// Called every period, and after every signal.
    pub tick: Option<(Duration, Hook)>,
    /// Called once, when the thread has been running for the delay (a signal does not hasten it).
    pub after: Option<(Duration, Hook)>,
}

/// Install the handler once per process.
pub fn install(stop: impl Fn() + Send + 'static) -> std::io::Result<()> {
    install_with(stop, || {})
}

/// Like [`install`]; `at_exit` runs before the exit on the second signal (to write out what is
/// buffered).
pub fn install_with(
    stop: impl Fn() + Send + 'static,
    at_exit: impl Fn() + Send + 'static,
) -> std::io::Result<()> {
    start(Handlers {
        first: Some(Box::new(stop)),
        at_exit: Some(Box::new(at_exit)),
        ..Handlers::default()
    })
}

/// Start the thread. Call once per process; a second call adds a second thread.
#[cfg(unix)]
pub fn start(mut h: Handlers) -> std::io::Result<()> {
    use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM, SIGUSR1};
    use signal_hook::{flag, low_level::pipe};
    use std::io::{ErrorKind, Read};
    use std::os::fd::IntoRawFd;
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    // A signal handler writes a byte to a socket; that wakes the thread, which otherwise waits for
    // the tick. `count` gets a byte per SIGINT/SIGTERM/SIGHUP only, so the thread knows how many
    // arrived (the second one exits); SIGUSR1 sets a flag. Safe code on both sides.
    let (mut wake, wake_tx) = UnixStream::pair()?;
    let (mut count, count_tx) = UnixStream::pair()?;
    for tx in [&wake_tx, &count_tx] {
        tx.set_nonblocking(true)?;
    }
    count.set_nonblocking(true)?;
    let period = h.tick.as_ref().map(|t| t.0);
    wake.set_read_timeout(h.after.as_ref().map(|t| t.0).or(period))?;
    let usr1 = Arc::new(AtomicBool::new(false));
    let (has_first, has_usr1) = (h.first.is_some(), h.usr1.is_some());
    let usr1_in = usr1.clone();
    // The writing ends stay open for the life of the process (the handlers write to them), which
    // also means the thread never sees the end of the sockets while it waits for the tick. One
    // descriptor serves every signal (`register_raw` does not take its own copy).
    let (wake_tx, count_tx) = (wake_tx.into_raw_fd(), count_tx.into_raw_fd());
    // Started first: a signal that is registered has a thread that reads it.
    std::thread::Builder::new()
        .name("yavs-signals".into())
        .spawn(move || {
            let mut seen = false;
            let mut buf = [0u8; 64];
            let t0 = std::time::Instant::now();
            let mut after = h.after.take();
            loop {
                let mut signalled = false;
                match wake.read(&mut buf) {
                    Ok(0) => return,
                    Ok(_) => signalled = true,
                    Err(e)
                        if matches!(
                            e.kind(),
                            ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                        ) => {}
                    Err(_) => return,
                }
                for _ in 0..count.read(&mut buf).unwrap_or(0) {
                    if seen {
                        if let Some(f) = &h.at_exit {
                            f();
                        }
                        std::process::exit(1);
                    }
                    seen = true;
                    if let Some(f) = &h.first {
                        f();
                    }
                }
                if usr1_in.swap(false, Ordering::SeqCst)
                    && let Some(f) = &h.usr1
                {
                    f();
                }
                if let Some((_, f)) = after.take_if(|(d, _)| t0.elapsed() >= *d) {
                    f();
                    let _ = wake.set_read_timeout(period);
                    if !signalled {
                        continue; // the wake-up was the delay's, not a tick's
                    }
                } else if let (true, Some((d, _))) = (signalled, &after) {
                    // A signal woke the thread early: wait out the rest of the delay.
                    let left = d.saturating_sub(t0.elapsed()).max(Duration::from_micros(1));
                    let _ = wake.set_read_timeout(Some(left));
                }
                if let Some((_, f)) = &h.tick {
                    f();
                }
            }
        })?;
    if has_first {
        for s in [SIGINT, SIGTERM, SIGHUP] {
            pipe::register_raw(s, count_tx)?;
            pipe::register_raw(s, wake_tx)?;
        }
    }
    if has_usr1 {
        flag::register(SIGUSR1, usr1)?;
        pipe::register_raw(SIGUSR1, wake_tx)?;
    }
    Ok(())
}

/// Start the thread (Windows: the console handler of `ctrlc` and a thread for the tick).
#[cfg(not(unix))]
pub fn start(h: Handlers) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicBool, Ordering};
    if h.tick.is_some() || h.after.is_some() {
        let (tick, after) = (h.tick, h.after);
        std::thread::Builder::new()
            .name("yavs-tick".into())
            .spawn(move || {
                if let Some((delay, f)) = after {
                    std::thread::sleep(delay);
                    f();
                }
                let Some((period, f)) = tick else { return };
                loop {
                    std::thread::sleep(period);
                    f();
                }
            })?;
    }
    if let Some(first) = h.first {
        let seen = AtomicBool::new(false);
        let at_exit = h.at_exit;
        ctrlc::set_handler(move || {
            if seen.swap(true, Ordering::SeqCst) {
                if let Some(f) = &at_exit {
                    f();
                }
                std::process::exit(1);
            }
            first();
        })
        .map_err(std::io::Error::other)?;
    }
    Ok(())
}
