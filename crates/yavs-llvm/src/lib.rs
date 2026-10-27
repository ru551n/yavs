//! LLVM tier for yavs.
//!
//! Everything here sits behind the `llvm` cargo feature, so the workspace
//! builds on machines without LLVM 21. Run the tests with
//! `cargo test -p yavs-llvm --features llvm`.
//!
//! The crate JITs LLVM IR through ORC LLJIT (design rule 5: no hand-written
//! object loader, portable CPU baseline). Host runtime functions are handed to
//! the JIT as absolute symbols, so the executable exports nothing.

// This crate is the FFI boundary to the LLVM C API: unsafe is allowed here and
// nowhere else in the workspace. Raw LLVM handles never leave the `jit`,
// `lower` and `spike` modules.
#![allow(unsafe_code)]

#[cfg(feature = "llvm")]
mod arena;
pub mod auto;
mod census;
mod eloop;
pub mod generator;
#[cfg(feature = "llvm")]
mod jit;
#[cfg(feature = "llvm")]
mod lazy;
#[cfg(feature = "llvm")]
mod lower;
pub mod native;
#[cfg(feature = "llvm")]
mod objcache;
mod profile;
pub mod regs;
pub mod rt;
#[cfg(feature = "llvm")]
pub mod spike;
mod work;

/// What the code arena has placed (`arena.rs`): code bytes, huge-page chunks made, collapses given
/// up, sections that did not fit; `None` where the arena is not in use.
#[cfg(feature = "llvm")]
pub fn code_arena_stats() -> Option<[usize; 4]> {
    arena::stats()
}
pub use auto::{AutoTier, Gens, Heat, Note, START_AFTER, exit_now};
pub use census::callback_counts;
pub use generator::{Generator, OptLevel, StartError};
#[cfg(feature = "llvm")]
pub use jit::{Error, Jit, Module, portable_cpu};
#[cfg(feature = "llvm")]
pub use lazy::ensure as load_library;
#[cfg(feature = "llvm")]
pub use lower::{LlvmGenerator, LlvmTier, compile};
#[cfg(feature = "llvm")]
pub use objcache::{cache_stats, set_cache};
pub use profile::set_run_id;
