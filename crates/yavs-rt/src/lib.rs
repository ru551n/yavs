//! The runtime of the compiled tier (Cranelift, `yavs-clift`): the tier-up policy (`auto`), the
//! host functions compiled code calls (`rt`), the native-frame loop (`eloop`), the run-to-run
//! profile and the perf map.

// Compiled code calls into this runtime through raw pointers: unsafe is allowed here.
#![allow(unsafe_code)]

pub mod auto;
mod census;
pub mod eloop;
pub mod generator;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub mod hugezone;
pub mod native;
pub mod perfmap;
mod profile;
pub mod regs;
pub mod rt;
mod work;

pub use auto::{AutoTier, Compile, Heat, Note, START_AFTER, count_rule_on, exit_now};
pub use census::callback_counts;
pub use generator::{Generator, StartError};
pub use profile::{program_digest, set_run_id};
