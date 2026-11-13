//! What the tier-up policy (`auto.rs`) needs from a code generator: `yavs-clift`'s Cranelift
//! one implements [`Generator`], and the policy tests use a stub.

use std::fmt;

use yavs_ir::{Func, FuncId, Program};

use crate::native::{Compiled, Part};

/// The compile thread could not be started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartError(pub String);

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StartError {}

/// A code generator the compile thread of `AutoTier` drives. It keeps the code it made alive until
/// it is dropped, which is when the tier is.
pub trait Generator: Send {
    /// Make ready for the first compile (open a library). An error leaves hot functions
    /// interpreted, the message is shown once.
    fn open(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Microseconds opening the generator costs, counted against the compile budget until the
    /// first compile.
    fn load_us(&self) -> u64 {
        0
    }

    /// The functions the policy is about to compile together, the root first. A generator that
    /// compiles the callees of a function in the same module makes only these (the policy leaves
    /// out callees that were never seen running: their compile time is not in its prices).
    fn tree(&mut self, _ids: &[FuncId]) {}

    /// Compile the `part` of `f` (`id`).
    fn compile(
        &mut self,
        prog: &Program,
        id: FuncId,
        f: &Func,
        part: Part,
    ) -> Result<Compiled, String>;

    /// Whether it keeps compiled code in a compilation cache.
    fn cache_enabled(&self) -> bool {
        false
    }

    /// Modules linked from its compilation cache so far (a compile that changes it was a hit).
    fn cache_hits(&self) -> usize {
        0
    }

    /// Modules compiled and stored in its compilation cache so far (a compile that changes it made new
    /// code).
    fn cache_stores(&self) -> usize {
        0
    }

    /// The functions (name and IR instruction count) the cache has compiled code of.
    fn cache_list(&self) -> std::collections::HashSet<(String, u32)> {
        Default::default()
    }

    /// Record that `name` (with `len` IR instructions) was compiled, for `cache_list`; kept until
    /// `cache_flush`.
    fn cache_note(&mut self, _name: &str, _len: u32) {}

    /// Store what the compiles since the last call add to the cache's lists (`cache_note`, the
    /// routes): the compile thread calls it when it is idle (at most every `FLUSH_EVERY`) and when
    /// it ends, not after every compile (each store is a file written).
    fn cache_flush(&mut self) {}

    /// The same store as `cache_flush`, as a function any thread may call while the generator is
    /// compiling on another: the end of a run that does not wait for a compile in flight
    /// (`AutoTier::stop`) stores what is noted so far with it. Nothing to store is no file operation.
    fn cache_flusher(&self) -> Option<fn()> {
        None
    }

    /// Until switched off, `compile` links what the cache has and compiles nothing.
    fn cached_only(&mut self, _on: bool) {}

    /// The store of its compilation cache, which also keeps the run-to-run profile (`profile.rs`).
    fn store(&self) -> Option<yavs_cache::Store> {
        None
    }
}
