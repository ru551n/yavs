//! Cranelift tier for yavs: the compiled-code contract of `yavs-llvm` (`rt.rs`) with
//! Cranelift as the code generator, so a release binary has a compiled tier without an LLVM
//! install. Everything is behind the `clift` cargo feature (off by default).
//!
//! `compile` translates every function up front (`--tier=clift`); [`AutoTier`]
//! (`--tier=clift-auto`) translates the functions where the sampled interpreter time reaches
//! their [`Heat`], on one background thread.

// Machine code and raw function pointers: unsafe is allowed here, in `lower` and `auto`.
#![allow(unsafe_code)]

#[cfg(feature = "clift")]
mod auto;
#[cfg(feature = "clift")]
mod lower;

#[cfg(feature = "clift")]
pub use auto::{AutoTier, Heat, Note};
#[cfg(feature = "clift")]
pub use lower::{Error, Jit, compile};

#[cfg(feature = "clift")]
use std::sync::Mutex;
#[cfg(feature = "clift")]
use yavs_interp::{Tier, TierHost};
#[cfg(feature = "clift")]
use yavs_ir::FuncId;
#[cfg(feature = "clift")]
use yavs_llvm::rt::{RunFn, enter};

/// Compiled code for the functions of one program.
#[cfg(feature = "clift")]
pub struct ClifTier {
    /// Keeps the code alive; never used after construction.
    pub(crate) _jit: Mutex<Jit>,
    pub(crate) funcs: Vec<Option<(RunFn, usize)>>,
    /// Functions that stay interpreted, with the reason.
    pub notes: Vec<(FuncId, String)>,
    /// Time spent translating and finalizing every function.
    pub compile_time: std::time::Duration,
    /// Instructions of the compiled functions the interpreter executes whole, per kind, most
    /// frequent first; and the number of the others (compiled, or with a compiled fast path).
    pub interpreted: Vec<(String, usize)>,
    pub inline: usize,
    /// IR instructions of the compiled functions.
    pub instrs: usize,
    /// Per function: IR instructions and the time `Jit::define` took.
    pub times: Vec<(usize, std::time::Duration)>,
}

#[cfg(feature = "clift")]
impl ClifTier {
    /// The number of functions with compiled code.
    pub fn compiled(&self) -> usize {
        self.funcs.iter().filter(|f| f.is_some()).count()
    }
}

#[cfg(feature = "clift")]
impl ClifTier {
    fn go(
        &self,
        func: FuncId,
        pc: u32,
        host: &mut dyn TierHost,
        state: Option<&mut Option<Box<yavs_interp::TierState>>>,
    ) -> Option<u32> {
        let &(f, nregs) = self.funcs.get(func.0 as usize)?.as_ref()?;
        let lookup = |id: FuncId| self.funcs.get(id.0 as usize).copied().flatten();
        enter(f, nregs, pc, host, &lookup, std::ptr::null(), state)
    }
}

/// The first level of `yavs-llvm`'s `AutoTier` (`--tier=auto3`): Cranelift translation of one
/// function at a time.
#[cfg(feature = "clift")]
pub struct First(Jit);

#[cfg(feature = "clift")]
impl First {
    pub fn new() -> Result<First, Error> {
        Jit::new().map(First)
    }
}

#[cfg(feature = "clift")]
impl yavs_llvm::rt::FirstLevel for First {
    fn compile(
        &mut self,
        prog: &yavs_ir::Program,
        id: FuncId,
        f: &yavs_ir::Func,
    ) -> Result<(RunFn, usize), String> {
        let (cid, _) = self.0.define(prog, id, f)?;
        self.0.finalize()?;
        Ok((self.0.entry(cid), f.reg_ty.len()))
    }
}

#[cfg(feature = "clift")]
impl Tier for ClifTier {
    fn run(&self, func: FuncId, pc: u32, host: &mut dyn TierHost) -> Option<u32> {
        self.go(func, pc, host, None)
    }

    fn run_cached(
        &self,
        func: FuncId,
        pc: u32,
        host: &mut dyn TierHost,
        state: &mut Option<Box<yavs_interp::TierState>>,
    ) -> Option<u32> {
        self.go(func, pc, host, Some(state))
    }

    fn ready(&self, func: FuncId) -> bool {
        self.funcs.get(func.0 as usize).is_some_and(Option::is_some)
    }
}
