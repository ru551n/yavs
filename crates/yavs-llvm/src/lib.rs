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
mod auto;
#[cfg(feature = "llvm")]
mod jit;
#[cfg(feature = "llvm")]
mod lazy;
#[cfg(feature = "llvm")]
mod lower;
#[cfg(feature = "llvm")]
mod regs;
pub mod rt;
#[cfg(feature = "llvm")]
pub mod spike;

#[cfg(feature = "llvm")]
pub use auto::{AutoTier, Heat, Note, exit_now};
#[cfg(feature = "llvm")]
pub use jit::{Error, Jit, Module, OptLevel, portable_cpu};
#[cfg(feature = "llvm")]
pub use lower::{LlvmTier, compile};
