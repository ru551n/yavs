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
mod cache;
#[cfg(feature = "clift")]
mod lower;

#[cfg(feature = "clift")]
pub use cache::{cache_stats, set_cache};

#[cfg(feature = "clift")]
pub use lower::{BREAK, Error, Jit, compile};

#[cfg(feature = "clift")]
use std::sync::Mutex;
#[cfg(feature = "clift")]
use yavs_interp::{Called, Tier, TierHost};
#[cfg(feature = "clift")]
use yavs_ir::FuncId;
#[cfg(feature = "clift")]
use yavs_llvm::rt::{RunFn, call, enter};

/// Compiled code for the functions of one program.
#[cfg(feature = "clift")]
pub struct ClifTier {
    /// Keeps the code alive; never used after construction.
    pub(crate) _jit: Mutex<Jit>,
    pub(crate) funcs: Vec<Option<(RunFn, usize)>>,
    /// Per function, `yavs_llvm::regs::no_table` (`enter`'s `lean`).
    pub(crate) lean: Vec<bool>,
    /// Per function, the address of its native-frame code (`Mem::ntab`) and of its `CallFn`
    /// (`Tier::call`), 0 for none.
    pub(crate) natives: Vec<usize>,
    pub(crate) calls: Vec<usize>,
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
    fn go<H: yavs_llvm::rt::View + ?Sized>(
        &self,
        func: FuncId,
        pc: u32,
        host: &mut H,
        state: Option<&mut yavs_interp::TierState>,
    ) -> Option<u32> {
        let &(f, nregs) = self.funcs.get(func.0 as usize)?.as_ref()?;
        let lookup = |id: FuncId| self.funcs.get(id.0 as usize).copied().flatten();
        let lean = self.lean.get(func.0 as usize) == Some(&true);
        enter(
            f,
            nregs,
            pc,
            host,
            &lookup,
            self.natives.as_ptr(),
            state,
            lean,
        )
    }
}

/// The Cranelift code generator of `yavs-llvm`'s `AutoTier` (`Generator`): the only one in a
/// build without LLVM (`--tier=clift-auto`), the first level of the hybrid with LLVM
/// (`--tier=auto3`). It compiles the callees of a native-frame function into the same module (direct
/// calls, inlined when small), and reports them with the root (`Compiled::extra`).
#[cfg(feature = "clift")]
pub struct First {
    jit: Jit,
    /// Functions whose native-frame code was reported already.
    reported: std::collections::HashSet<FuncId>,
}

#[cfg(feature = "clift")]
impl First {
    pub fn new() -> Result<First, Error> {
        Ok(First {
            jit: Jit::new()?,
            reported: Default::default(),
        })
    }
}

#[cfg(feature = "clift")]
impl yavs_llvm::Generator for First {
    fn tree(&mut self, ids: &[FuncId]) {
        self.jit.allow(ids);
    }

    fn cache_enabled(&self) -> bool {
        cache::store().is_some()
    }

    fn cache_hits(&self) -> usize {
        cache::cache_stats().0
    }

    fn cache_list(&self) -> std::collections::HashSet<(String, u32)> {
        cache::compiled_list()
    }

    fn cache_note(&self, name: &str, len: u32) {
        cache::add_compiled(name, len);
    }

    fn cached_only(&mut self, on: bool) {
        self.jit.set_cached_only(on);
    }

    fn store(&self) -> Option<yavs_cache::Store> {
        cache::store()
    }

    fn compile(
        &mut self,
        prog: &yavs_ir::Program,
        id: FuncId,
        f: &yavs_ir::Func,
        _level: yavs_llvm::OptLevel,
        part: yavs_llvm::native::Part,
    ) -> Result<yavs_llvm::native::Compiled, String> {
        use yavs_llvm::native::Part;
        let frame = (part != Part::Native)
            .then(|| self.jit.define(prog, id, f))
            .transpose()?;
        if part != Part::Frame && !self.jit.make_native(prog, id) && self.jit.missed() {
            return Err(lower::NOT_CACHED.into());
        }
        self.jit.finalize()?;
        let mut out = yavs_llvm::native::Compiled {
            run: frame.map(|(cid, _)| self.jit.entry(cid)),
            nregs: f.reg_ty.len(),
            native: 0,
            call: 0,
            extra: Vec::new(),
        };
        for (g, native, call) in self.jit.variants() {
            if g == id {
                (out.native, out.call) = (native, call);
            } else if self.reported.insert(g) {
                out.extra.push((g, native, call));
            }
        }
        self.reported.insert(id);
        Ok(out)
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
        host: &mut yavs_interp::Host<'_, '_, '_>,
        state: &mut yavs_interp::TierState,
    ) -> Option<u32> {
        self.go(func, pc, host, Some(state))
    }

    fn ready(&self, func: FuncId) -> bool {
        self.funcs.get(func.0 as usize).is_some_and(Option::is_some)
    }

    fn callable(&self, func: FuncId) -> bool {
        self.calls.get(func.0 as usize).is_some_and(|&a| a != 0)
    }

    fn call(
        &self,
        func: FuncId,
        cells: &[u64],
        obj: Option<u64>,
        pc: u32,
        host: &mut dyn TierHost,
    ) -> Option<Called> {
        let c = *self.calls.get(func.0 as usize)?;
        let lookup = |id: FuncId| self.funcs.get(id.0 as usize).copied().flatten();
        call(c, cells, obj, pc, host, &lookup, self.natives.as_ptr())
    }
}
