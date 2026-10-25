//! `Handlers::after` runs once, on the helper thread, when the delay is over (the start of the
//! compile thread of `--tier=auto`, which a run of a few milliseconds never needs, #426).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use yavs_wave::interrupt::{self, Handlers};

#[test]
fn after_runs_once_after_the_delay() {
    let ran = Arc::new(AtomicUsize::new(0));
    let r = ran.clone();
    let t0 = Instant::now();
    interrupt::start(Handlers {
        after: Some((
            Duration::from_millis(200),
            Box::new(move || {
                r.fetch_add(1, Ordering::SeqCst);
            }),
        )),
        // A tick shorter than the delay does not hasten it, nor repeat it.
        tick: Some((Duration::from_millis(20), Box::new(|| {}))),
        ..Handlers::default()
    })
    .unwrap();
    std::thread::sleep(Duration::from_millis(60));
    assert_eq!(
        ran.load(Ordering::SeqCst),
        0,
        "ran after {:?}",
        t0.elapsed()
    );
    while ran.load(Ordering::SeqCst) == 0 {
        assert!(t0.elapsed() < Duration::from_secs(30), "never ran");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        t0.elapsed() >= Duration::from_millis(200),
        "{:?}",
        t0.elapsed()
    );
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(ran.load(Ordering::SeqCst), 1);
}
