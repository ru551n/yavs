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
mod cold;
#[cfg(all(feature = "clift", target_os = "linux", target_arch = "x86_64"))]
mod hugearena;
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
    /// Prototype (perf/inv-postelab, `YAVS_CLIFT_POSTELAB`): bodies per process instance.
    pub post: Option<PostSet>,
}

/// The body of a process compiled for one instance (`lower::Post`): run only while the view's
/// `checks` (`MEM_*` offset, value) hold for that `owner`.
#[cfg(feature = "clift")]
pub(crate) struct PostBody {
    owner: u64,
    checks: Vec<(u64, u64)>,
    f: Option<RunFn>,
}

/// Instances of one process that get a body of their own; the others run the shared body.
#[cfg(feature = "clift")]
pub(crate) const POST_INSTANCES: usize = 4;

/// The view fields a post body takes as constants: fixed for a process in its root frame.
#[cfg(feature = "clift")]
const POST_VIEW: [u64; 15] = {
    use yavs_llvm::rt::*;
    [
        MEM_SLOTS,
        MEM_SLOTS_LEN,
        MEM_FRAME,
        MEM_FRAME_IDX,
        MEM_ROOT_SLOTS,
        MEM_INST,
        MEM_INST_LEN,
        MEM_NSIGS,
        MEM_SIGTAB,
        MEM_SIGTAB_LEN,
        MEM_STOP,
        MEM_SPIN,
        MEM_COV,
        MEM_COV_LEN,
        MEM_MAX_DEPTH,
    ]
};

/// The word at `off` (a `MEM_*` offset: `Mem` starts with its `TierFrame`) of the view.
#[cfg(feature = "clift")]
fn view_word(fr: &yavs_interp::TierFrame, off: u64) -> u64 {
    // SAFETY: every `POST_VIEW` offset is that of a u64 or pointer field of the `repr(C)` view.
    unsafe {
        (fr as *const yavs_interp::TierFrame)
            .cast::<u8>()
            .add(off as usize)
            .cast::<u64>()
            .read()
    }
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
        if let Some(ps) = self.post.as_ref().filter(|ps| ps.on(func)) {
            return yavs_llvm::rt::enter_post(
                f,
                nregs,
                pc,
                host,
                &lookup,
                self.natives.as_ptr(),
                state,
                lean,
                &|fr, f| ps.choose(func, fr, f),
            );
        }
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

/// Prototype (perf/inv-postelab, `YAVS_CLIFT_POSTELAB=all|name,...`): the processes that get a
/// body of their own per instance (up to `POST_INSTANCES`), made at its first entry from its view
/// (`lower::Post`), in a module of their own.
#[cfg(feature = "clift")]
pub struct PostSet {
    jit: Mutex<Jit>,
    prog: std::sync::Arc<yavs_ir::Program>,
    /// The filter, and per function whether it passes (asked at its first entry: a copy is made late).
    want: String,
    on: Vec<std::sync::OnceLock<bool>>,
    posts: Vec<[std::sync::OnceLock<PostBody>; POST_INSTANCES]>,
    /// Under `--tier=auto`: callees through `Mem::ntab` (the policy did not choose them).
    auto: bool,
    /// Entries that took the shared body although the process has post bodies.
    refused: std::sync::atomic::AtomicU64,
}

#[cfg(feature = "clift")]
impl PostSet {
    /// `None` without `YAVS_CLIFT_POSTELAB`.
    pub fn new(prog: std::sync::Arc<yavs_ir::Program>, auto: bool) -> Option<PostSet> {
        let want = std::env::var("YAVS_CLIFT_POSTELAB").ok()?;
        let n = prog.funcs.len();
        let on = (0..n).map(|_| Default::default()).collect();
        Some(PostSet {
            jit: Mutex::new(Jit::new().ok()?),
            want,
            on,
            posts: (0..n)
                .map(|_| std::array::from_fn(|_| Default::default()))
                .collect(),
            prog,
            auto,
            refused: Default::default(),
        })
    }

    fn on(&self, func: FuncId) -> bool {
        let Some(c) = self.on.get(func.0 as usize) else {
            return false;
        };
        *c.get_or_init(|| {
            self.prog.func(func).is_some_and(|f| {
                let w = &self.want;
                f.kind == yavs_ir::FuncKind::Process
                    && (w == "all" || w.split(',').any(|x| f.name.contains(x)))
            })
        })
    }

    /// Under `--tier=auto`: run this set's bodies for the compiled processes (`rt::POST_HOOK`).
    pub fn install(self) {
        let ps = self;
        let _ = yavs_llvm::rt::POST_HOOK.set(Box::new(move |func, fr, f| {
            if ps.on(func) {
                ps.choose(func, fr, f)
            } else {
                f
            }
        }));
    }

    /// The body to run for this view: the instance's own (made at its first entry), else `f`.
    fn choose(&self, func: FuncId, fr: &yavs_interp::TierFrame, f: RunFn) -> RunFn {
        for cell in &self.posts[func.0 as usize] {
            let pb = match cell.get() {
                Some(pb) if pb.owner != fr.owner => continue,
                Some(pb) => pb,
                // Only a root frame with its whole signal table and a kernel has the facts.
                None if fr.frame != 0 || fr.sigtab_len != fr.nsigs || fr.kctx.is_null() => {
                    return f;
                }
                None => cell.get_or_init(|| self.build_post(func, fr)),
            };
            if let Some(p) = pb.f
                && pb.owner == fr.owner
                && pb.checks.iter().all(|&(o, v)| view_word(fr, o) == v)
            {
                return p;
            }
            break;
        }
        let n = self
            .refused
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        if n.is_power_of_two() {
            eprintln!(
                "note: postelab: {n} entries took the shared body (last f{})",
                func.0
            );
        }
        f
    }

    fn build_post(&self, func: FuncId, fr: &yavs_interp::TierFrame) -> PostBody {
        let checks: Vec<(u64, u64)> = POST_VIEW.iter().map(|&o| (o, view_word(fr, o))).collect();
        let mut pb = PostBody {
            owner: fr.owner,
            checks: checks.clone(),
            f: None,
        };
        let prog = &self.prog;
        let Some(fun) = prog.func(func) else {
            return pb;
        };
        // SAFETY: the view's table has `sigtab_len` entries for the activation (`TierHost::view`).
        let sigs =
            unsafe { std::slice::from_raw_parts(fr.sigtab, fr.sigtab_len as usize) }.to_vec();
        let post = lower::Post {
            view: checks,
            tab: fr.sigtab as u64,
            sigs,
        };
        let t = std::time::Instant::now();
        let Ok(mut jit) = self.jit.lock() else {
            return pb;
        };
        if self.auto {
            jit.allow(&[func]);
        }
        match jit.define_post(prog, func, fun, post) {
            Ok(p) => pb.f = Some(p),
            Err(e) => eprintln!("note: postelab: f{} `{}`: {e}", func.0, fun.name),
        }
        eprintln!(
            "note: postelab: f{} `{}` ({} IR) built in {} us",
            func.0,
            fun.name,
            fun.code.len(),
            t.elapsed().as_micros()
        );
        pb
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
        self.jit.flush_routes();
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
