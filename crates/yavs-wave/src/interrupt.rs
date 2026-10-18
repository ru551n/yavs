//! SIGINT/SIGTERM (and Ctrl-C on Windows) support.
//!
//! The handler only sets a flag. The kernel polls it between time steps, stops, and calls
//! [`crate::WaveWriter::finish`], so the file is complete after Ctrl-C.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Install the handler once per process and return the flag it sets.
pub fn install() -> Result<Arc<AtomicBool>, ctrlc::Error> {
    let flag = Arc::new(AtomicBool::new(false));
    let f = flag.clone();
    ctrlc::set_handler(move || f.store(true, Ordering::SeqCst))?;
    Ok(flag)
}
