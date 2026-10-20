//! `yavs run --time-report`: wall and CPU time of the phases of a run, printed to stderr.
//!
//! Off by default: `mark` is one relaxed atomic load then. CPU time is the whole process
//! (all threads, including finished ones) from `/proc/self/stat`, so it is reported on Linux
//! only, in clock ticks of 10 ms (ponytail: no safe cross-platform CPU clock without a new
//! dependency; use many runs or repeat for short phases).

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

static ON: AtomicBool = AtomicBool::new(false);
static STATE: Mutex<Option<State>> = Mutex::new(None);

type Stamp = (Instant, Option<f64>);

struct State {
    last: Stamp,
    start: Stamp,
    rows: Vec<(&'static str, f64, Option<f64>)>,
}

/// Process CPU seconds (user + system, all threads), if the platform tells.
fn cpu() -> Option<f64> {
    let s = std::fs::read_to_string("/proc/self/stat").ok()?;
    // Fields after the parenthesised command name; utime and stime are the 12th and 13th.
    let rest = s.get(s.rfind(')')? + 2..)?;
    let mut f = rest.split(' ').skip(11);
    let (u, s): (f64, f64) = (f.next()?.parse().ok()?, f.next()?.parse().ok()?);
    Some((u + s) / 100.0)
}

pub fn enable() {
    ON.store(true, Ordering::Relaxed);
    let now = (Instant::now(), cpu());
    *STATE.lock().unwrap() = Some(State {
        last: now,
        start: now,
        rows: Vec::new(),
    });
}

/// Ends the phase `name`, which began at the previous mark (or `enable`).
pub fn mark(name: &'static str) {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    let now = (Instant::now(), cpu());
    if let Some(s) = STATE.lock().unwrap().as_mut() {
        let cpu = now.1.zip(s.last.1).map(|(a, b)| a - b);
        s.rows
            .push((name, now.0.duration_since(s.last.0).as_secs_f64(), cpu));
        s.last = now;
    }
}

/// Prints one `time-report: PHASE wall=S cpu=S` line per phase, a `time-report: gc collections=N` line and a `total` line.
pub fn report() {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    let Some(s) = STATE.lock().unwrap().take() else {
        return;
    };
    let line = |n: &str, w: f64, c: Option<f64>| {
        let c = c.map_or("n/a".to_string(), |c| format!("{c:.2}"));
        eprintln!("time-report: {n} wall={w:.3} cpu={c}");
    };
    for (n, w, c) in &s.rows {
        line(n, *w, *c);
    }
    eprintln!(
        "time-report: gc collections={}",
        yavs_interp::gc_collections()
    );
    let now = (Instant::now(), cpu());
    line(
        "total",
        now.0.duration_since(s.start.0).as_secs_f64(),
        now.1.zip(s.start.1).map(|(a, b)| a - b),
    );
}
