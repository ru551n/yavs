//! `yavs-ir` to LLVM: the compiled tier (ir.md 5, `docs/decisions.md` "LLVM tier").
//!
//! One LLVM function per IR function, `fn(env, regs, mem, pc) -> u32`, working on the
//! registers of the interpreter frame and, through [`Mem`], on the frame's slots, the
//! temporary area and the instance data (the interpreter's [`TierFrame`] view). It enters at
//! `pc` (a switch over the block starts and the instructions after a call or a suspension),
//! runs register arithmetic, compares, checks, branches, loads, stores, copies, temporary
//! allocation, descriptor and signal-reference arithmetic itself, and executes every other
//! instruction in place through the `yavs_rt_step` callback (the interpreter's own code):
//! calls, natives, reports, signal operations and the slow paths. A call whose callee has
//! compiled code runs that directly on the frame the interpreter pushed, and goes on when it
//! returns (#336). It returns at a return or a suspension, or when an inline instruction
//! fails (re-executed by the interpreter, so messages are identical): compiled code never
//! reports an error itself (rule 5).

use std::ffi::CString;

use std::sync::Mutex;

use llvm_sys::analysis::{LLVMVerifierFailureAction, LLVMVerifyFunction};

use llvm_sys::core::*;

use llvm_sys::prelude::LLVMMetadataRef;
use llvm_sys::prelude::*;

use llvm_sys::{LLVMIntPredicate as IP, LLVMRealPredicate as RP};

use yavs_interp::{Called, TIER_SUSPEND, Tier, TierHost, space};

use yavs_ir::{
    AddrSpace, BoolOp, CheckKind, CmpOp, EdgeKind, FOp, FUnOp, Func, FuncId, FuncKind, Inst, IntOp,
    IntUnOp, Program, Reg, Width,
};

use crate::generator::Generator;

use crate::native::*;

use crate::rt::*;

use crate::{Error, Jit, Module, OptLevel};

type Known = crate::rt::Known<LLVMValueRef>;

type ObjRef = crate::rt::ObjRef<LLVMValueRef>;

/// Ask the kernel to back the compiled code of a whole-program module with huge pages (Linux): the
/// code of a design of thousands of processes is megabytes that every clock cycle runs through,
/// and each 4 KiB page of it is an entry of the instruction TLB. `MADV_COLLAPSE` makes the pages
/// now; a failure changes nothing, `YAVS_LLVM_HUGE=0` leaves it out. (The CPU design: instruction
/// TLB misses 52 M to 9 M, 6 percent of the cycles.)
#[cfg(target_os = "linux")]
fn huge_code(addrs: impl Iterator<Item = usize>) {
    use std::ffi::c_void;
    unsafe extern "C" {
        fn madvise(addr: *mut c_void, len: usize, advice: i32) -> i32;
    }
    const HUGE: usize = 2 << 20;
    // The arena (`arena.rs`) places and backs the code itself where it is available.
    if crate::arena::enabled() || std::env::var("YAVS_LLVM_HUGE").is_ok_and(|v| v == "0") {
        return;
    }
    let (lo, hi) = addrs.fold((usize::MAX, 0), |(lo, hi), a| (lo.min(a), hi.max(a)));
    let (start, end) = (lo.next_multiple_of(HUGE), hi & !(HUGE - 1));
    if lo == usize::MAX || end <= start || end - start < 2 * HUGE {
        return;
    }
    // SAFETY: advice on the pages of our own JIT code; the call changes no content.
    unsafe {
        madvise(start as *mut c_void, end - start, 14);
        madvise(start as *mut c_void, end - start, 25);
    }
}

#[cfg(not(target_os = "linux"))]
fn huge_code(_: impl Iterator<Item = usize>) {}

/// Measuring aid (`YAVS_LLVM_KINDS=1`): LLVM instructions generated per IR instruction kind,
/// printed by `--tier=llvm` after the whole-program compile (frame-mode code, before the passes).
static KIND_STATS: Mutex<Vec<(String, usize, usize)>> = Mutex::new(Vec::new());

/// Measuring aid (`YAVS_LLVM_ABLATE=Kind,Kind`): those IR kinds are stepped by the interpreter
/// instead of compiled, to measure how much code the generator spends on them. Only for kinds
/// that have a step path (not control flow).
fn ablate(i: &Inst) -> bool {
    static SET: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let set = SET.get_or_init(|| {
        std::env::var("YAVS_LLVM_ABLATE")
            .map(|v| v.split(',').map(str::to_string).collect())
            .unwrap_or_default()
    });
    !set.is_empty() && set.contains(&kind_name(i))
}

/// The variant name of `i` (`CallNative` for every native call).
fn kind_name(i: &Inst) -> String {
    let d = format!("{i:?}");
    d.split([' ', '{', '(']).next().unwrap_or("").to_string()
}

/// Measuring aid (`YAVS_LLVM_KINDLOC=<file>`): every LLVM instruction gets a debug line, the
/// number of the IR instruction kind it was made for (1000 more in a cold block); the numbers
/// are listed in `<file>`. With `YAVS_LLVM_OBJ` the machine code of an object can then be summed
/// per kind (`objdump -dl`). Line tables only: no code changes. With `YAVS_LLVM_PCLOC` set too
/// the line is `2 * pc + 2` of the IR instruction (one more in a cold block) and the file is
/// `i<function id>`, so `perf` samples map to IR instructions of one function (#679).
struct KindLoc {
    b: llvm_sys::prelude::LLVMDIBuilderRef,
    sp: LLVMMetadataRef,
}

fn kind_line(k: &str) -> u32 {
    static KINDS: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let mut v = KINDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(i) = v.iter().position(|x| x == k) {
        return i as u32 + 1;
    }
    v.push(k.to_string());
    if let Some(path) = std::env::var_os("YAVS_LLVM_KINDLOC") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{} {k}", v.len());
        }
    }
    v.len() as u32
}

impl Gen {
    /// Start line tables for `fv` when `YAVS_LLVM_KINDLOC` is set.
    unsafe fn kindloc_begin(&self, fv: LLVMValueRef, name: &str, id: u32) -> Option<KindLoc> {
        use llvm_sys::debuginfo::*;
        std::env::var_os("YAVS_LLVM_KINDLOC")?;
        // SAFETY: a fresh builder on our module; the strings are copied.
        unsafe {
            let b = LLVMCreateDIBuilder(self.module);
            let fname = format!("i{id}");
            let file =
                LLVMDIBuilderCreateFile(b, fname.as_ptr().cast(), fname.len(), c".".as_ptr(), 1);
            LLVMDIBuilderCreateCompileUnit(
                b,
                LLVMDWARFSourceLanguage::LLVMDWARFSourceLanguageC,
                file,
                c"yavs".as_ptr(),
                4,
                0,
                c"".as_ptr(),
                0,
                0,
                c"".as_ptr(),
                0,
                LLVMDWARFEmissionKind::LLVMDWARFEmissionKindLineTablesOnly,
                0,
                0,
                0,
                c"".as_ptr(),
                0,
                c"".as_ptr(),
                0,
            );
            let ty = LLVMDIBuilderCreateSubroutineType(b, file, std::ptr::null_mut(), 0, 0);
            let sp = LLVMDIBuilderCreateFunction(
                b,
                file,
                name.as_ptr().cast(),
                name.len(),
                name.as_ptr().cast(),
                name.len(),
                file,
                1,
                ty,
                0,
                1,
                1,
                0,
                0,
            );
            LLVMSetSubprogram(fv, sp);
            // (A whole-program module has many functions and the flag may be set once, #773.)
            if LLVMGetModuleFlag(self.module, c"Debug Info Version".as_ptr(), 18).is_null() {
                let three = LLVMValueAsMetadata(self.c32(3));
                LLVMAddModuleFlag(
                    self.module,
                    llvm_sys::LLVMModuleFlagBehavior::LLVMModuleFlagBehaviorWarning,
                    c"Debug Info Version".as_ptr(),
                    18,
                    three,
                );
            }
            Some(KindLoc { b, sp })
        }
    }

    /// Instructions made from now on belong to IR kind `inst` at `pc` (`KindLoc`).
    unsafe fn kindloc_at(&self, k: &KindLoc, inst: &Inst, pc: u32) {
        // SAFETY: the location is made in our context with a scope of this module.
        unsafe {
            let line = if std::env::var_os("YAVS_LLVM_PCLOC").is_some() {
                2 * pc + 2
            } else {
                kind_line(&kind_name(inst))
            };
            let loc = llvm_sys::debuginfo::LLVMDIBuilderCreateDebugLocation(
                self.ctx,
                line,
                0,
                k.sp,
                std::ptr::null_mut(),
            );
            LLVMSetCurrentDebugLocation2(self.b, loc);
        }
    }
}

/// Mark every instruction of the blocks `cold` (`KindLoc`).
unsafe fn kindloc_cold(fv: LLVMValueRef, cold: &[LLVMBasicBlockRef]) {
    use llvm_sys::debuginfo::*;
    // SAFETY: plain walks over blocks of a function being generated.
    unsafe {
        let sp = LLVMGetSubprogram(fv);
        if sp.is_null() {
            return;
        }
        let ctx = LLVMGetModuleContext(LLVMGetGlobalParent(fv));
        // Kind lines below 1000 get 1000 more; pc lines (`YAVS_LLVM_PCLOC`, even) get 1.
        let pcloc = std::env::var_os("YAVS_LLVM_PCLOC").is_some();
        for &bb in cold {
            let mut i = LLVMGetFirstInstruction(bb);
            while !i.is_null() {
                let old = LLVMInstructionGetDebugLoc(i);
                if !old.is_null() {
                    let line = LLVMDILocationGetLine(old);
                    let cold_line = match pcloc {
                        true if line.is_multiple_of(2) => Some(line + 1),
                        false if line < 1000 => Some(line + 1000),
                        _ => None,
                    };
                    if let Some(line) = cold_line {
                        let loc = LLVMDIBuilderCreateDebugLocation(
                            ctx,
                            line,
                            0,
                            sp,
                            std::ptr::null_mut(),
                        );
                        LLVMInstructionSetDebugLoc(i, loc);
                    }
                }
                i = LLVMGetNextInstruction(i);
            }
        }
    }
}

/// The LLVM instructions in `f`.
// ponytail: a walk per IR instruction, quadratic in a function's size; fine for a measuring aid.
unsafe fn fn_count(f: LLVMValueRef) -> usize {
    let mut n = 0;
    // SAFETY: plain iteration over a live function.
    unsafe {
        let mut bb = LLVMGetFirstBasicBlock(f);
        while !bb.is_null() {
            let mut i = LLVMGetFirstInstruction(bb);
            while !i.is_null() {
                n += 1;
                i = LLVMGetNextInstruction(i);
            }
            bb = LLVMGetNextBasicBlock(bb);
        }
    }
    n
}

/// One IR instruction of kind `k` and its share, 1/`n`, of the `llvm` instructions made for `n`.
fn kind_add_frac(k: String, llvm: usize, n: usize) {
    kind_add(k, 1, llvm / n);
}

fn kind_add(k: String, ir: usize, llvm: usize) {
    let mut v = KIND_STATS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(e) = v.iter_mut().find(|e| e.0 == k) {
        e.1 += ir;
        e.2 += llvm;
    } else {
        v.push((k, ir, llvm));
    }
}

fn kind_report() {
    let mut v = KIND_STATS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    v.sort_by_key(|e| std::cmp::Reverse(e.2));
    let tot: usize = v.iter().map(|e| e.2).sum();
    eprintln!("kinds: total llvm insts (pre-opt, frame mode) {tot}");
    for (k, ir, l) in v.iter().take(40) {
        eprintln!(
            "kinds: {k:20} ir={ir:8} llvm={l:9} per={:.1} share={:.1}%",
            *l as f64 / (*ir).max(1) as f64,
            100.0 * *l as f64 / tot.max(1) as f64
        );
    }
}

/// Compiled code for the functions of one program.
pub struct LlvmTier {
    /// Keeps the code alive; never used after construction.
    _jit: Mutex<Jit>,
    funcs: Vec<Option<(RunFn, usize)>>,
    /// Per function, `regs::no_table` (`rt::enter`'s `lean`).
    lean: Vec<bool>,
    /// Per function, its native-frame code's address or 0 (`Mem::ntab`).
    natives: Vec<usize>,
    /// Per function, its `CallFn` or 0 (`Tier::call`).
    calls: Vec<usize>,
    /// Functions that stay interpreted, with the reason.
    pub notes: Vec<(FuncId, String)>,
    /// Instructions of the compiled functions the interpreter executes (stepped or at an
    /// exit), per instruction kind, most frequent first; and the number compiled inline.
    pub interpreted: Vec<(String, usize)>,
    pub inline: usize,
}

impl LlvmTier {
    /// The functions compiled code calls on native frames (`docs/decisions.md` "Compiled
    /// frames").
    pub fn native_frames(&self) -> usize {
        self.natives.iter().filter(|&&a| a != 0).count()
    }
}

impl Tier for LlvmTier {
    fn run(&self, func: FuncId, pc: u32, host: &mut dyn TierHost) -> Option<u32> {
        let &(f, nregs) = self.funcs.get(func.0 as usize)?.as_ref()?;
        let lookup = |id: FuncId| self.funcs.get(id.0 as usize).copied().flatten();
        enter(
            f,
            nregs,
            pc,
            host,
            &lookup,
            self.natives.as_ptr(),
            None,
            false,
        )
    }

    fn run_cached(
        &self,
        func: FuncId,
        pc: u32,
        host: &mut yavs_interp::Host<'_, '_, '_>,
        state: &mut yavs_interp::TierState,
    ) -> Option<u32> {
        let &(f, nregs) = self.funcs.get(func.0 as usize)?.as_ref()?;
        let lookup = |id: FuncId| self.funcs.get(id.0 as usize).copied().flatten();
        enter(
            f,
            nregs,
            pc,
            host,
            &lookup,
            self.natives.as_ptr(),
            Some(state),
            self.lean.get(func.0 as usize) == Some(&true),
        )
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

/// Microseconds of opening the LLVM library (about 40 million cycles), counted with the first
/// compile or look-up in the compilation cache: a run too short to pay for it loads nothing.
const LOAD_US: u64 = 10_000;

/// The LLVM generator of `AutoTier` (`Generator`): a JIT per level, each made with the first
/// function compiled at that level.
#[derive(Default)]
pub struct LlvmGenerator {
    jit: Option<Jit>,
    jit2: Option<Jit>,
    jitf: Option<Jit>,
    /// For functions recompiled at `OptLevel::FastRa`.
    jitr: Option<Jit>,
}

impl LlvmGenerator {
    pub fn new() -> LlvmGenerator {
        LlvmGenerator::default()
    }
}

impl Generator for LlvmGenerator {
    fn open(&mut self) -> Result<(), String> {
        crate::lazy::ensure().map_err(|e| e.to_string())
    }

    fn load_us(&self) -> u64 {
        LOAD_US
    }

    fn level_for(&self, prog: &Program, f: &Func, level: OptLevel) -> OptLevel {
        level_for(prog, f, level)
    }

    fn compile(
        &mut self,
        prog: &Program,
        id: FuncId,
        f: &Func,
        level: OptLevel,
        part: Part,
    ) -> Result<Compiled, String> {
        let j = match level {
            OptLevel::Fast => &mut self.jitf,
            OptLevel::FastRa => &mut self.jitr,
            OptLevel::O2 => &mut self.jit2,
            _ => &mut self.jit,
        };
        let j = match j {
            Some(j) => j,
            None => j.insert(new_jit(level).map_err(|e| format!("LLVM: {e} (a bug in yavs)"))?),
        };
        compile_one(j, prog, id, f, level, part)
    }

    fn cache_enabled(&self) -> bool {
        crate::objcache::enabled()
    }

    fn cache_hits(&self) -> usize {
        crate::objcache::cache_stats().0
    }

    fn cache_list(&self) -> std::collections::HashSet<(String, u32)> {
        crate::objcache::compiled_list()
    }

    fn store(&self) -> Option<yavs_cache::Store> {
        crate::objcache::store()
    }

    fn cache_note(&self, name: &str, len: u32) {
        crate::objcache::add_compiled(name, len);
    }

    fn cached_only(&mut self, on: bool) {
        crate::objcache::set_cached_only(on);
    }
}

/// A JIT with the runtime callbacks compiled code calls.
pub(crate) fn new_jit(opt: OptLevel) -> Result<Jit, Error> {
    Jit::new(
        opt,
        &[
            (STEP, yavs_rt_step as *const () as usize),
            (HOST, yavs_rt_host as *const () as usize),
            (REGION, yavs_rt_region as *const () as usize),
            (SIGNAL, yavs_rt_signal as *const () as usize),
            (DRIVE, yavs_rt_drive as *const () as usize),
            (DRIVE_NAT, yavs_rt_drive_nat as *const () as usize),
            (DRIVE_BIT, yavs_rt_drive_bit as *const () as usize),
            (WAIT, yavs_rt_wait as *const () as usize),
            (WAIT_AGAIN, yavs_rt_wait_again as *const () as usize),
            (WAIT_EDGE, yavs_rt_wait_edge as *const () as usize),
            (WAIT_SAME, yavs_rt_wait_same as *const () as usize),
            (LCOPY, yavs_rt_lcopy as *const () as usize),
            (LAYOUT, yavs_rt_layout as *const () as usize),
            (FAST, yavs_rt_fast as *const () as usize),
            (DEOPT, yavs_rt_deopt as *const () as usize),
            (DEOPT_SLOTS, yavs_rt_deopt_slots as *const () as usize),
            (DEOPT_END, yavs_rt_deopt_end as *const () as usize),
            (MEMEQ, yavs_rt_memeq as *const () as usize),
            (LAYOUT_WORD, yavs_rt_layout_word as *const () as usize),
            (SCAN, yavs_rt_scan as *const () as usize),
            (ELOOP, yavs_rt_eloop as *const () as usize),
            (ELOOP_EXEC, yavs_rt_eloop_exec as *const () as usize),
            (CODES, yavs_rt_codes as *const () as usize),
            (SCALAR, yavs_rt_scalar as *const () as usize),
            (PLANES, yavs_rt_planes as *const () as usize),
            (NATIVE, yavs_rt_native as *const () as usize),
            (NATIVE_LIGHT, yavs_rt_native_light as *const () as usize),
            (MEMO, yavs_rt_memo as *const () as usize),
            (MEMO_KEEP, yavs_rt_memo_keep as *const () as usize),
            (TROOT, yavs_rt_troot as *const () as usize),
            (ENDSTMT, yavs_rt_endstmt as *const () as usize),
            (XSTEP, yavs_rt_xstep as *const () as usize),
            (DRIVE_FLUSH, yavs_rt_drive_flush as *const () as usize),
        ],
    )
}

/// A `--tier=llvm` module of more LLVM instructions than this is compiled at `OptLevel::Fast`
/// (`YAVS_LLVM_FAST_MIN` overrides it, a tuning aid like `YAVS_LLVM_PIPELINE`).
const EAGER_FAST_MIN: usize = 150_000;

/// Predecessors of one exit block of a frame-mode function (`Gen::exit_for`).
const EXIT_PHI: usize = 64;

/// The level `f` is compiled at when `level` is asked for: `OptLevel::Fast` for a large function
/// (`native::big`), compute loops keep theirs.
pub(crate) fn level_for(prog: &Program, f: &Func, level: OptLevel) -> OptLevel {
    if level == OptLevel::O2 && crate::native::big(prog, f) {
        OptLevel::Fast
    } else {
        level
    }
}

/// Compile `f` alone into `jit`: its entry (none for `Part::Native`) and register count, and the
/// address of its native-frame code (0 for none), or why it stays interpreted.
pub(crate) fn compile_one(
    jit: &mut Jit,
    prog: &Program,
    id: FuncId,
    f: &Func,
    opt: OptLevel,
    part: Part,
) -> Result<Compiled, String> {
    if let Some(r) = bad_reg(f) {
        return Err(format!("register r{} beyond the frame", r.0));
    }
    // A copy of a process (`Program::copy_of`) is named by what it is for, not by its id, which
    // depends on the order the processes switched in: the same copy is the same module in every
    // run, so the compilation cache finds it (#661).
    let stem = prog.copy_tag(id).map_or_else(
        || format!("f{}", id.0),
        |t| format!("f{}c{t:016x}", prog.base_of(id).0),
    );
    let times = std::env::var_os("YAVS_LLVM_TIMES").is_some();
    let t0 = std::time::Instant::now();
    // The cache's object of the function found before its code is generated (#785); under
    // `objcache::check` the code is generated anyway and must be that object.
    let full = crate::auto::compute(prog, f);
    let pre = crate::objcache::pre_key(prog, id, f, &format!("{opt:?} {full} {part:?}"));
    let stored = pre.as_ref().and_then(crate::objcache::pre_get);
    if let Some(e) = stored.as_ref().filter(|_| !crate::objcache::check())
        && let Some(c) = linked(jit, f, &stem, part, e)?
    {
        if times {
            eprintln!(
                "times f{} {opt:?} ir={} prehit total={} us",
                id.0,
                f.code.len(),
                t0.elapsed().as_micros()
            );
        }
        return Ok(c);
    }
    let what = || format!("`{}` (f{}, {part:?})", f.name, id.0);
    // Native callees are reached through `Mem::ntab`, so only the reachable set's shape
    // matters here, not what is compiled yet. The function's own callees are roots too: a
    // function that does not run on native frames still calls those that do directly (#510).
    let nc = native_set(
        prog,
        std::iter::once(id).chain(f.code.iter().filter_map(call_of).map(|c| c.0)),
    );
    // A copy is generated under a fixed name and named by its module afterwards: another copy of
    // the process whose module is the same up to its names (instances that differ in data the code
    // does not read) is that code, in this run and in the compilation cache.
    let twin = prog.copy_tag(id).is_some();
    let gname = if twin { "ftwin" } else { stem.as_str() };
    let nname = format!("{gname}_n");
    let llvm = |e: Error| format!("LLVM: {e} (a bug in yavs)");
    let cname = format!("{gname}_c");
    let name = gname.to_string();
    let mut native = nc.contains(&id) && part != Part::Frame;
    if part == Part::Native && !native {
        return Err("no native-frame code for the function".into());
    }
    if let Some(dir) = std::env::var_os("YAVS_LLVM_DUMP") {
        let _ = std::fs::write(
            std::path::Path::new(&dir).join(format!("f{}.ir", id.0)),
            prog.dump_func(id),
        );
    }
    let m = loop {
        let m = Module::new(&name);
        let g = Gen::new(&m);
        // SAFETY: generation of well-formed IR in our own module; a frame-mode function that
        // fails the verifier is deleted again, a native one makes us start again without it.
        if native && unsafe { g.func(prog, f, id, &nname, true, &nc) }.is_err() {
            if part == Part::Native {
                return Err("the generated LLVM function does not verify (a bug in yavs)".into());
            }
            native = false;
            continue;
        }
        if native && f.parent.is_none() {
            // SAFETY: as above, after the native function.
            unsafe { g.adapter(f, &nname, &cname) };
        }
        if part != Part::Native {
            unsafe { g.func(prog, f, id, &name, false, &nc) }?;
        }
        drop(g);
        break m;
    };
    let t1 = std::time::Instant::now();
    let ninst = if times { m.instruction_count() } else { 0 };
    let mut m = m;
    let (mut name, mut nname, mut cname) = (name, nname, cname);
    let twin_key = twin.then(|| {
        let mut k = yavs_cache::KeyBuilder::new("twin");
        k.text(&format!("{opt:?} {full} {part:?} {native}"))
            .part(&m.bitcode());
        k.finish().hex()
    });
    if let Some(key) = &twin_key {
        if let Some(&twin) = jit.twins.get(key) {
            crate::objcache::note_twin();
            if let Some(k) = &pre {
                let obj = crate::objcache::key(&m, opt, full, Some(key));
                let now = crate::objcache::Pre {
                    obj,
                    native,
                    twin: twin_key.clone(),
                };
                crate::objcache::pre_note(k, stored.as_ref(), &now, what);
            }
            return Ok(twin_of(f, twin));
        }
        let stem = format!("t{}", &key[..16]);
        m.rename(&name, &stem);
        (name, nname, cname) = (stem.clone(), format!("{stem}_n"), format!("{stem}_c"));
    }
    let (mut t2, mut t3, mut nopt) = (t1, t1, 0);
    // Verified and optimised only when it is not linked from the compilation cache (#505).
    // A copy's module is keyed by its twin key: the bitcode made for it, which names the module.
    let obj = crate::objcache::add(jit, m, opt, full, twin_key.as_deref(), |m| {
        m.verify()?;
        t2 = std::time::Instant::now();
        m.optimize_as(opt, full)?;
        t3 = std::time::Instant::now();
        nopt = if times { m.instruction_count() } else { 0 };
        Ok(())
    })
    .map_err(llvm)?;
    if let (Some(k), Some(obj)) = (&pre, obj) {
        let now = crate::objcache::Pre {
            obj,
            native,
            twin: twin_key.clone(),
        };
        crate::objcache::pre_note(k, stored.as_ref(), &now, what);
    }
    let c = look_up(jit, f, [&name, &nname, &cname], native, part, twin_key)?;
    if times {
        let us = |a: std::time::Instant, b: std::time::Instant| (b - a).as_micros();
        eprintln!(
            "times f{} {opt:?} ir={} llvm={ninst} opt={nopt} gen={} verify={} pass={} codegen={} total={} us native={native}",
            id.0,
            f.code.len(),
            us(t0, t1),
            us(t1, t2),
            us(t2, t3),
            us(t3, std::time::Instant::now()),
            t0.elapsed().as_micros(),
        );
    }
    Ok(c)
}

/// `compile_one` of a function whose pre-key names `e` (#785): the object linked (or the twin
/// linked before), no code generated; `None` when the object is not there or `part` needs code it
/// does not have.
fn linked(
    jit: &mut Jit,
    f: &Func,
    stem: &str,
    part: Part,
    e: &crate::objcache::Pre,
) -> Result<Option<Compiled>, String> {
    if part == Part::Native && !e.native {
        return Ok(None);
    }
    if let Some(&twin) = e.twin.as_ref().and_then(|t| jit.twins.get(t)) {
        crate::objcache::note_twin();
        return Ok(Some(twin_of(f, twin)));
    }
    if !crate::objcache::link(jit, &e.obj) {
        return Ok(None);
    }
    let name = e
        .twin
        .as_ref()
        .map_or_else(|| stem.to_string(), |t| format!("t{}", &t[..16]));
    let names = [&name, &format!("{name}_n"), &format!("{name}_c")];
    look_up(jit, f, names, e.native, part, e.twin.clone()).map(Some)
}

/// The entry points of a module just linked into `jit` under `names` (frame-mode entry, native-frame
/// code, its `CallFn`), recorded as `twin`'s when it is a copy.
fn look_up(
    jit: &mut Jit,
    f: &Func,
    [name, nname, cname]: [&String; 3],
    native: bool,
    part: Part,
    twin: Option<String>,
) -> Result<Compiled, String> {
    let llvm = |e: Error| format!("LLVM: {e} (a bug in yavs)");
    let run = if part == Part::Native {
        None
    } else {
        let addr = jit.lookup(name).map_err(llvm)?;
        // SAFETY: `name` was generated by `Gen::func` with the `RunFn` signature.
        Some(unsafe { std::mem::transmute::<usize, RunFn>(addr) })
    };
    let naddr = if native {
        jit.lookup(nname).map_err(llvm)?
    } else {
        0
    };
    let caddr = if native && f.parent.is_none() {
        jit.lookup(cname).map_err(llvm)?
    } else {
        0
    };
    if let Some(key) = twin {
        jit.twins
            .insert(key, (run.map_or(0, |r| r as usize), naddr, caddr));
    }
    perf_map(&[
        (run.map_or(0, |r| r as usize), &f.name, name),
        (naddr, &f.name, nname),
    ]);
    Ok(Compiled {
        run,
        nregs: f.reg_ty.len(),
        native: naddr,
        call: caddr,
        extra: Vec::new(),
    })
}

/// The `Compiled` of a copy whose twin is linked in the same JIT already (entry, native-frame code,
/// `CallFn`).
fn twin_of(f: &Func, (run, naddr, caddr): (usize, usize, usize)) -> Compiled {
    Compiled {
        // SAFETY: `run` is the entry of a module equal to the one asked for.
        run: (run != 0).then(|| unsafe { std::mem::transmute::<usize, RunFn>(run) }),
        nregs: f.reg_ty.len(),
        native: naddr,
        call: caddr,
        extra: Vec::new(),
    }
}

/// Tuning aid (`YAVS_LLVM_PERFMAP=1`): the compiled functions so far, as `perf` reads them from
/// `/tmp/perf-<pid>.map` (`address size name`); a function's size is taken up to the next
/// function's address. The whole file is written again at every call.
fn perf_map(new: &[(usize, &str, &str)]) {
    static SYMS: Mutex<Vec<(usize, String)>> = Mutex::new(Vec::new());
    if std::env::var_os("YAVS_LLVM_PERFMAP").is_none() {
        return;
    }
    let mut syms = SYMS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    syms.extend(
        new.iter()
            .filter(|s| s.0 != 0)
            .map(|(a, n, s)| (*a, format!("{s}_{n}"))),
    );
    syms.sort();
    let mut out = String::new();
    for (k, (a, n)) in syms.iter().enumerate() {
        let next = syms.get(k + 1).map_or(a + 4096, |s| s.0);
        out += &format!("{a:x} {:x} {n}\n", (next - a).min(1 << 18));
    }
    let _ = std::fs::write(format!("/tmp/perf-{}.map", std::process::id()), out);
}

/// Compile every function of `prog` that the tier can take; the others (among them those over
/// `max_size` IR instructions) stay interpreted and are listed in [`LlvmTier::notes`].
pub fn compile(prog: &Program, opt: OptLevel, max_size: usize) -> Result<LlvmTier, Error> {
    crate::lazy::ensure()?;
    let mut notes = Vec::new();
    let mut todo = Vec::new();
    let only: Option<Vec<u32>> = std::env::var("YAVS_LLVM_ONLY")
        .ok()
        .map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect());
    for (id, f) in prog
        .funcs
        .iter()
        .filter_map(|(id, _)| prog.func(id).map(|f| (id, f)))
    {
        // Elaboration code runs before the simulation, in the interpreter.
        if f.kind == FuncKind::Elab {
            continue;
        }
        if let Some(why) = too_big(f, max_size) {
            notes.push((id, why));
            continue;
        }
        if let Some(r) = bad_reg(f) {
            notes.push((id, format!("register r{} beyond the frame", r.0)));
            continue;
        }
        if let Some(dir) = std::env::var_os("YAVS_LLVM_DUMP") {
            // Debug aid next to the module dumps of `Module::optimize`.
            let _ = std::fs::write(
                std::path::Path::new(&dir).join(format!("f{}.ir", id.0)),
                prog.dump_func(id),
            );
        }
        // Measuring aid: `YAVS_LLVM_ONLY=12,34` compiles just these function ids.
        if only.as_ref().is_some_and(|o| !o.contains(&id.0)) {
            continue;
        }
        todo.push((id, f));
    }
    let ids: std::collections::HashSet<FuncId> = todo.iter().map(|t| t.0).collect();
    let mut nc = native_set(prog, ids.iter().copied());
    nc.retain(|id| ids.contains(id));
    let gen_all = |nc: &std::collections::HashSet<FuncId>| {
        let m = Module::new("yavs");
        let mut g = Gen::new(&m);
        g.direct = true;
        let mut names = Vec::new();
        let mut notes = Vec::new();
        for &(id, f) in &todo {
            if nc.contains(&id) {
                let nname = format!("f{}_n", id.0);
                // SAFETY: generation of well-formed IR in our own module.
                unsafe { g.func(prog, f, id, &nname, true, nc) }.ok()?;
                if f.parent.is_none() {
                    // SAFETY: as above.
                    unsafe { g.adapter(f, &nname, &format!("f{}_c", id.0)) };
                }
            }
        }
        for &(id, f) in &todo {
            let name = format!("f{}", id.0);
            // SAFETY: generation of well-formed IR in our own module; a function that
            // fails the verifier is deleted again.
            match unsafe { g.func(prog, f, id, &name, false, nc) } {
                Ok(()) => names.push((id, name, f.reg_ty.len())),
                Err(why) => notes.push((id, why)),
            }
        }
        drop(g);
        Some((m, names, notes))
    };
    // A native function that does not verify: everything again without native frames.
    let tgen = std::time::Instant::now();
    let (m, names, more) = match gen_all(&nc) {
        Some(r) => r,
        None => {
            nc.clear();
            gen_all(&nc).ok_or_else(|| Error("no module without native frames".into()))?
        }
    };
    notes.extend(more);
    if std::env::var_os("YAVS_LLVM_KINDS").is_some() {
        kind_report();
    }
    // Optimised code generation of a module this big takes minutes (#501).
    let fast_min = std::env::var("YAVS_LLVM_FAST_MIN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(EAGER_FAST_MIN);
    let opt = if opt == OptLevel::O2 && m.instruction_count() > fast_min {
        OptLevel::Fast
    } else {
        opt
    };
    let times = std::env::var_os("YAVS_LLVM_TIMES").is_some();
    let (t0, mut tv, mut to, mut nopt) = (std::time::Instant::now(), None, None, 0);
    let ninst = if times { m.instruction_count() } else { 0 };
    let mut jit = new_jit(opt)?;
    crate::objcache::add(&mut jit, m, opt, false, None, |m| {
        let t = std::time::Instant::now();
        m.verify()?;
        tv = Some(t.elapsed());
        let t = std::time::Instant::now();
        m.optimize_as(opt, false)?;
        to = Some(t.elapsed());
        nopt = if times { m.instruction_count() } else { 0 };
        Ok(())
    })?;
    // (The whole program's module stays keyed by its bitcode: no pre-key, #785.)
    let mut funcs = vec![None; prog.funcs.len()];
    for (id, name, nregs) in names {
        let addr = jit.lookup(&name)?;
        // SAFETY: `name` was generated by `Gen::func` with the `RunFn` signature.
        let f: RunFn = unsafe { std::mem::transmute::<usize, RunFn>(addr) };
        funcs[id.0 as usize] = Some((f, nregs));
    }
    if times {
        eprintln!(
            "times whole program {opt:?}: llvm={ninst} opt={nopt} gen={:?} verify={:?} pass={:?} add+codegen={:?}",
            t0.duration_since(tgen),
            tv,
            to,
            t0.elapsed()
        );
    }
    huge_code(funcs.iter().flatten().map(|(f, _)| *f as usize));
    let mut natives = vec![0; prog.funcs.len()];
    let mut calls = vec![0; prog.funcs.len()];
    for &id in &nc {
        natives[id.0 as usize] = jit.lookup(&format!("f{}_n", id.0))?;
        if prog.func(id).is_some_and(|f| f.parent.is_none()) {
            calls[id.0 as usize] = jit.lookup(&format!("f{}_c", id.0))?;
        }
    }
    if std::env::var_os("YAVS_LLVM_PERFMAP").is_some() {
        let mut syms = Vec::new();
        for (k, f) in funcs.iter().enumerate() {
            let name = prog.func(FuncId(k as u32)).map_or("?", |f| f.name.as_str());
            if let Some((r, _)) = f {
                syms.push((*r as usize, name, format!("f{k}")));
            }
            syms.push((natives[k], name, format!("f{k}_n")));
        }
        let syms: Vec<(usize, &str, &str)> =
            syms.iter().map(|(a, n, s)| (*a, *n, s.as_str())).collect();
        perf_map(&syms);
    }
    let mut kinds = std::collections::HashMap::new();
    let mut inline = 0;
    for (id, f) in prog
        .funcs
        .iter()
        .filter_map(|(id, _)| prog.func(id).map(|f| (id, f)))
    {
        if funcs[id.0 as usize].is_none() {
            continue;
        }
        for i in &f.code {
            if is_inline(prog, f, i) {
                inline += 1;
            } else {
                let d = format!("{i:?}");
                let k = d.split([' ', '{', '(']).next().unwrap_or("").to_string();
                *kinds.entry(k).or_insert(0) += 1;
            }
        }
    }
    let mut interpreted: Vec<(String, usize)> = kinds.into_iter().collect();
    interpreted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Ok(LlvmTier {
        _jit: Mutex::new(jit),
        lean: (0..prog.funcs.len())
            .map(|i| {
                funcs[i].is_some()
                    && prog
                        .func(FuncId(i as u32))
                        .is_some_and(crate::regs::no_table)
            })
            .collect(),
        funcs,
        natives,
        calls,
        notes,
        interpreted,
        inline,
    })
}

/// Orders the blocks of `fv` so that the likelier successor of a block follows it (the
/// unoptimised code generator emits blocks in this order and a branch to the next one is free);
/// `cold` blocks go last. Creation order puts a join block before the fast path that leads to
/// it, which costs two taken jumps per guarded instruction (#679).
unsafe fn layout(fv: LLVMValueRef, cold: &[LLVMBasicBlockRef]) {
    use std::collections::{HashMap, HashSet};
    // SAFETY: plain walks over the blocks of a function being generated; moving a block keeps
    // the function valid (the entry block stays first).
    unsafe {
        let mut blocks = Vec::new();
        let mut b = LLVMGetFirstBasicBlock(fv);
        while !b.is_null() {
            blocks.push(b);
            b = LLVMGetNextBasicBlock(b);
        }
        if blocks.len() < 3 {
            for &b in cold {
                LLVMMoveBasicBlockAfter(b, LLVMGetLastBasicBlock(fv));
            }
            return;
        }
        let cold: HashSet<_> = cold.iter().copied().collect();
        let pos: HashMap<_, _> = blocks.iter().enumerate().map(|(i, &b)| (b, i)).collect();
        let prof = LLVMGetMDKindIDInContext(
            LLVMGetModuleContext(LLVMGetGlobalParent(fv)),
            c"prof".as_ptr(),
            4,
        );
        let mut placed = HashSet::new();
        let mut order = Vec::with_capacity(blocks.len());
        for &start in blocks.iter().filter(|b| !cold.contains(*b)) {
            let mut cur = start;
            while placed.insert(cur) {
                order.push(cur);
                let t = LLVMGetBasicBlockTerminator(cur);
                if t.is_null() {
                    break;
                }
                // The heavier edge of a branch with weights, else the earliest successor.
                let weights = {
                    let md = LLVMGetMetadata(t, prof);
                    let mut w = [0u64; 2];
                    if !md.is_null() && LLVMGetMDNodeNumOperands(md) == 3 {
                        let mut ops = [std::ptr::null_mut(); 3];
                        LLVMGetMDNodeOperands(md, ops.as_mut_ptr());
                        if !ops[1].is_null() && !ops[2].is_null() {
                            w = [
                                LLVMConstIntGetZExtValue(ops[1]),
                                LLVMConstIntGetZExtValue(ops[2]),
                            ];
                        }
                    }
                    w
                };
                let n = LLVMGetNumSuccessors(t);
                let mut best: Option<(LLVMBasicBlockRef, u64, usize)> = None;
                for i in 0..n {
                    let s = LLVMGetSuccessor(t, i);
                    if placed.contains(&s) || cold.contains(&s) {
                        continue;
                    }
                    let w = if n == 2 { weights[i as usize] } else { 0 };
                    let at = pos[&s];
                    if best.is_none_or(|(_, bw, bp)| w > bw || (w == bw && at < bp)) {
                        best = Some((s, w, at));
                    }
                }
                match best {
                    Some((s, ..)) => cur = s,
                    None => break,
                }
            }
        }
        order.extend(blocks.iter().filter(|b| cold.contains(*b)));
        for w in order.windows(2) {
            LLVMMoveBasicBlockAfter(w[1], w[0]);
        }
    }
}

/// A constant `LCopy` of 1 to 64 elements that lies inside one plane block on each side:
/// the blocks and the bit offsets in them.
struct LcopyShape {
    k: u32,
    sb: i64,
    s0: u32,
    db: i64,
    d0: u32,
}

fn lcopy_one_block(k: i64, so: i64, d: i64) -> Option<LcopyShape> {
    let ok = (1..=64).contains(&k) && (0..1 << 32).contains(&so) && (0..1 << 32).contains(&d);
    (ok && so >> 6 == (so + k - 1) >> 6 && d >> 6 == (d + k - 1) >> 6).then_some(LcopyShape {
        k: k as u32,
        sb: so >> 6,
        s0: (so & 63) as u32,
        db: d >> 6,
        d0: (d & 63) as u32,
    })
}

/// `ModRef` of an LLVM `memory(...)` attribute: reads, writes, both.
const REF: u64 = 1;

const MODS: u64 = 2;

const RW: u64 = 3;

/// The value of a `memory(...)` attribute (LLVM 21 `MemoryEffects`: two bits per location,
/// argument memory, inaccessible memory, errno, other memory).
fn mem_effects(arg: u64, inaccessible: u64, other: u64) -> u64 {
    arg | inaccessible << 2 | other << 6
}

/// Instructions whose step neither collects garbage nor runs other code: it cannot look at a
/// frame variable no address of which was made (`regs::Scalars`), so they are written back
/// only if it fails.
fn quiet(i: &Inst) -> bool {
    // A call is stepped here, so it runs other code.
    !crate::regs::loud(i, &|_| false)
}

struct Gen {
    ctx: LLVMContextRef,
    module: LLVMModuleRef,
    b: LLVMBuilderRef,
    i8: LLVMTypeRef,
    i32: LLVMTypeRef,
    i64: LLVMTypeRef,
    f64: LLVMTypeRef,
    ptr: LLVMTypeRef,
    fn_ty: LLVMTypeRef,
    step: LLVMValueRef,
    /// `yavs_rt_host(env, mem, addr, extra, n, write) -> ptr`, null for the slow path.
    host_ty: LLVMTypeRef,
    host: LLVMValueRef,
    /// `yavs_rt_region(env, mem, addr, depth, flags, out)`: the region a parameter points into.
    region_ty: LLVMTypeRef,
    region: LLVMValueRef,
    /// `yavs_rt_signal(env, regs, mem, pc, edge) -> u32`, as `step`.
    signal_ty: LLVMTypeRef,
    signal: LLVMValueRef,
    /// `yavs_rt_drive(env, regs, mem, pc, driver, lo, len, wave, inertial) -> u32`.
    drive_ty: LLVMTypeRef,
    drive: LLVMValueRef,
    /// `yavs_rt_drive_nat(env, driver, lo, len, wave, flags) -> u32`: `drive` for a native frame,
    /// 1 when done and 0 when nothing was done (no frame view).
    drive_nat_ty: LLVMTypeRef,
    drive_nat: LLVMValueRef,
    /// `yavs_rt_drive_bit(env, regs, mem, pc, driver, lo, len, delay, code) -> u32`: the scalar
    /// logic assignment of a `yavs_interp::ScalarDrive`, the code in the low byte and bit 8 for
    /// an inertial one.
    drive_bit_ty: LLVMTypeRef,
    drive_bit: LLVMValueRef,
    /// `yavs_rt_wait(env, regs, mem, pc, lo, len) -> u32`, and `yavs_rt_wait_same` of that type.
    wait_ty: LLVMTypeRef,
    wait: LLVMValueRef,
    /// `yavs_rt_wait_again(env, lo, len) -> u32`.
    wait_again_ty: LLVMTypeRef,
    wait_again: LLVMValueRef,
    wait_same: LLVMValueRef,
    /// `yavs_rt_wait_edge(env, mem, lo, len, kind) -> u32`.
    wait_edge_ty: LLVMTypeRef,
    wait_edge: LLVMValueRef,
    /// `yavs_rt_lcopy(dst, nd, d0, src, ns, s0, n)`.
    lcopy_ty: LLVMTypeRef,
    lcopy: LLVMValueRef,
    /// `yavs_rt_layout(env, regs, mem, desc, dst, dims, stride, align, el_count, logic) -> u32`.
    layout_ty: LLVMTypeRef,
    layout: LLVMValueRef,
    /// `yavs_rt_fast(env, regs, mem, op, dst, a, b) -> u32`.
    fast_ty: LLVMTypeRef,
    fast: LLVMValueRef,
    /// `yavs_rt_deopt(env, mem, func, pc, temp_base, nregs, slen, link, regs) -> ptr`.
    deopt_ty: LLVMTypeRef,
    deopt: LLVMValueRef,
    /// `yavs_rt_deopt_slots(env) -> ptr`.
    deopt_slots_ty: LLVMTypeRef,
    deopt_slots: LLVMValueRef,
    /// `yavs_rt_deopt_end(env, mem, pc) -> u32`.
    deopt_end_ty: LLVMTypeRef,
    deopt_end: LLVMValueRef,
    /// `yavs_rt_memeq(a, b, n) -> i64`.
    memeq_ty: LLVMTypeRef,
    memeq: LLVMValueRef,
    /// `yavs_rt_layout_word(env, ty, desc, dsz, k) -> i64`.
    layout_word_ty: LLVMTypeRef,
    layout_word: LLVMValueRef,
    /// `yavs_rt_scan(env, mem, ctl, dope, base, i0, x, e, delta, out, depth) -> iterations`.
    scan_ty: LLVMTypeRef,
    scan: LLVMValueRef,
    /// `yavs_rt_eloop(env, mem, prog, nprog, inputs, ind, bound, step, depth, fhi, slots, slots_len)
    /// -> iterations`.
    eloop_ty: LLVMTypeRef,
    eloop: LLVMValueRef,
    /// `yavs_rt_eloop_exec(src, s_e0, s_step, dst, d_e0, d_step, code, m) -> iterations`.
    eloop_exec_ty: LLVMTypeRef,
    eloop_exec: LLVMValueRef,
    /// `yavs_rt_codes(codes, groups, mask, out)`.
    codes_ty: LLVMTypeRef,
    codes: LLVMValueRef,
    /// `yavs_rt_scalar(ent, off, out) -> u32`.
    scalar_ty: LLVMTypeRef,
    scalar: LLVMValueRef,
    /// `yavs_rt_planes(src, nblk, bit0, n, dst)`.
    planes_ty: LLVMTypeRef,
    planes: LLVMValueRef,
    /// `yavs_rt_native(env, mem, native, cells, n, depth, out) -> u32`.
    native_ty: LLVMTypeRef,
    native: LLVMValueRef,
    /// `yavs_rt_native_light`: the type of `yavs_rt_native`.
    native_light: LLVMValueRef,
    /// `yavs_rt_memo` and `yavs_rt_memo_keep`: the type of `yavs_rt_native`.
    memo: LLVMValueRef,
    memo_keep: LLVMValueRef,
    /// `yavs_rt_troot(env, mem, addr, ty, desc, n) -> u32`.
    troot_ty: LLVMTypeRef,
    troot: LLVMValueRef,
    /// `yavs_rt_endstmt(env, mem, base)`.
    endstmt_ty: LLVMTypeRef,
    endstmt: LLVMValueRef,
    /// `yavs_rt_xstep(env, mem, func, pc, depth, use_idx, ins, nin, def_idx, outs, nout, temp_base) -> u32`.
    xstep_ty: LLVMTypeRef,
    xstep: LLVMValueRef,
    /// `yavs_rt_drive_flush(env, mem, entries, n) -> u32`: the kept scalar drives (`Batch`).
    flush_ty: LLVMTypeRef,
    flush: LLVMValueRef,
    /// `llvm.frameaddress.p0(i32) -> ptr`.
    frame_addr: (LLVMTypeRef, LLVMValueRef),
    /// Native callees are called by symbol (one module, `compile`), else through `Mem::ntab`.
    pub(crate) direct: bool,
    /// Above 0 while a slow path of an inline fast path is generated: its callbacks are marked
    /// `cold` and its blocks go behind the rest of the function (`Cur::cold`).
    cold_depth: std::cell::Cell<u32>,
    /// Which memory a pointer value points into (`AA_*`), for alias scope metadata.
    kinds: std::cell::RefCell<std::collections::HashMap<usize, usize>>,
    /// `alias.scope` and `noalias` list nodes per kind, and their metadata kind ids.
    aa: std::cell::RefCell<Option<AaNodes>>,
    sadd: (LLVMTypeRef, LLVMValueRef),
    ssub: (LLVMTypeRef, LLVMValueRef),
    smul: (LLVMTypeRef, LLVMValueRef),
    umul: (LLVMTypeRef, LLVMValueRef),
    uadd: (LLVMTypeRef, LLVMValueRef),
}

/// Memory kinds that never overlap: the frame's slots, the temporary area, the instance data,
/// constants, the signals' code bytes. A pointer of a known kind gets alias scope metadata, so
/// LLVM keeps values in registers across stores to the others.
const AA_SLOTS: usize = 0;

const AA_TEMP: usize = 1;

const AA_INST: usize = 2;

const AA_CONST: usize = 3;

const AA_CODES: usize = 4;
/// The jump count and the stop flag a process that never suspends polls at its jumps back: kept
/// in registers across a loop that makes no call.
const AA_SPIN: usize = 5;
const AA_STOP: usize = 6;
/// The fields of `Mem` that hold those two pointers.
const AA_MEM: usize = 7;
const AA_N: usize = 8;

struct AaNodes {
    scope: [LLVMValueRef; AA_N],
    others: [LLVMValueRef; AA_N],
    k_scope: u32,
    k_noalias: u32,
}

/// Register facts that differ from a block's start (`Dirty::diff`).
type Facts = Vec<(u32, Option<Known>)>;

/// The registers whose entry in `Cur::known` may differ from what a block starts with (the
/// function's preloaded addresses): a block start puts back only those, and the objects found are
/// forgotten only in those, instead of a pass over every register (#785: O(registers x blocks)).
struct Dirty {
    regs: Vec<u32>,
    set: Vec<bool>,
}

impl Dirty {
    fn new(nregs: usize) -> Dirty {
        Dirty {
            regs: Vec::new(),
            set: vec![false; nregs],
        }
    }

    fn mark(&mut self, r: u32) {
        if let Some(s) = self.set.get_mut(r as usize)
            && !*s
        {
            *s = true;
            self.regs.push(r);
        }
    }

    /// Mark what `track` may have written for `i`: its definitions (two cells for a signal
    /// reference, and the length word of `SigVal`).
    fn defs(&mut self, i: &Inst) {
        i.visit(&mut |r, role, ty| {
            if role == yavs_ir::Role::Def {
                self.mark(r.0);
                if ty == Some(yavs_ir::RegTy::Sig) || matches!(i, Inst::SigVal { .. }) {
                    self.mark(r.0 + 1);
                }
            }
        });
    }

    /// Put `known` back to `base` and forget the marks.
    fn reset(&mut self, known: &mut [Option<Known>], base: &[Option<Known>]) {
        for r in self.regs.drain(..) {
            known[r as usize] = base[r as usize];
            self.set[r as usize] = false;
        }
    }

    /// The entries of `known` that may differ from the base.
    fn diff(&self, known: &[Option<Known>]) -> Facts {
        self.regs.iter().map(|&r| (r, known[r as usize])).collect()
    }
}

/// The function being generated.
struct Cur<'p> {
    prog: &'p Program,
    /// The private globals holding constants this function reads (`Gen::host`).
    consts: std::cell::RefCell<std::collections::HashMap<yavs_ir::ConstId, LLVMValueRef>>,
    f: LLVMValueRef,
    env: LLVMValueRef,
    regs: LLVMValueRef,
    mem: LLVMValueRef,
    /// What the generator knows about each register at the current instruction (`track`).
    known: std::cell::RefCell<Vec<Option<Known>>>,
    /// The registers whose entry in `known` may differ from what a block starts with (`Dirty`).
    dirty: std::cell::RefCell<Dirty>,
    /// Per pc, a multiplication fused with the `Index` after it (`index_muls`).
    imul: Vec<Option<(Reg, u64)>>,
    /// What the fused multiplication just generated settled for the `Index` at this pc: 1, its
    /// position is under 2^32; 2, its offset is within 32 bits too (the `Index` handler).
    fused: std::cell::Cell<(u32, u8)>,
    /// The last pc of the instructions whose code the `LayoutOf` before them made
    /// (`fuse_layout`), with the object a `Field`'s result points into, for `objnew` there.
    layout_fused: std::cell::Cell<Option<Fused>>,
    /// Registers the function defines once, by a `Const`: that value wherever they are read
    /// (the interpreter's loop skips such a definition and relies on the same).
    fixed: Vec<Option<i64>>,
    /// The blocks that return the function's result (`exit_for`): block, phi, predecessors.
    exits: std::cell::RefCell<Vec<(LLVMBasicBlockRef, LLVMValueRef, usize)>>,
    /// The native frame's exit block (`exit_for`) and its phi.
    exit: (LLVMBasicBlockRef, LLVMValueRef),
    /// The blocks that record the core registers for the exits that share them (`core_stub`):
    /// block, phi of the pc, predecessors.
    cores: std::cell::RefCell<Vec<(LLVMBasicBlockRef, LLVMValueRef, usize)>>,
    /// The frame's size in bytes (`Func::frame`): the slots of every frame of the function.
    frame_len: u64,
    /// The native frame, when this is a function's native-frame code (`f<id>_n`).
    nat: Option<Nat>,
    /// Frame-mode code that stores a register only where the interpreter can look at it.
    defer: Option<Defer>,
    /// Frame variables kept as values (with `defer`): slot `k` is deferred register `nregs + k`.
    scal: Option<crate::regs::Scalars>,
    /// The frame's base before each slow path being generated (`slow_path`), and whether its
    /// step may push a frame; `join` takes them back.
    joins: std::cell::RefCell<Vec<(Option<LLVMValueRef>, bool)>>,
    /// Blocks that run only when a guard fails: moved behind the rest at the end, so the hot
    /// path falls through instead of jumping over them.
    cold: std::cell::RefCell<Vec<LLVMBasicBlockRef>>,
    /// The object the instruction being generated found for its result register (`ObjRef`):
    /// entered in `known` once `track` has run for it.
    objnew: std::cell::Cell<Option<(Reg, ObjRef)>>,
    /// The scalar drives of this activation kept for one host call, when the function has any.
    batch: Option<Batch>,
    /// The jump count of a process that never suspends (`TierFrame::spin`) while compiled code
    /// runs: a local that LLVM keeps in a register across the loop, loaded at the entry and stored
    /// back at every exit (`exit_for`).
    spin: Option<LLVMValueRef>,
    /// The regions the address parameters of a native frame point into (`param_addrs`), resolved at
    /// the entry: `Known::Param(k)` is `pres[k]`.
    pres: std::cell::RefCell<Vec<PRes>>,
}

/// A region held for a whole native activation: the host pointer of its start and its length;
/// `write`: resolved for writing too.
#[derive(Clone, Copy)]
struct PRes {
    ptr: LLVMValueRef,
    len: LLVMValueRef,
    write: bool,
}

/// The last pc a fused `LayoutOf` made the code of, and the object a `Field` result points into.
type Fused = (usize, Option<(Reg, ObjRef)>);

/// Zero-delay scalar logic drives (`yavs_interp::ScalarDrive`) are not handed to the kernel one
/// by one: the function appends `(signal reference, driver | code | pc)` to `entries`
/// (`yavs_interp::DRIVE_BATCH` of them, `count` in use) and `Gen::batch_flush` hands them over in
/// one `TierHost::drive_bits` call before anything that could look at a driver or run other
/// code (every instruction but `batch_pure` ones) and at every exit of the function.
#[derive(Clone, Copy)]
struct Batch {
    entries: LLVMValueRef,
    count: LLVMValueRef,
}

/// Deferred register stores (`regs.rs`): a definition of a pure instruction is kept as a value
/// in `pend` and stored by `Gen::flush` where it is live and compiled code leaves the frame
/// to the interpreter (a failing guard, a stepped instruction, a block edge); registers the
/// interpreter preloads are never stored.
struct Defer {
    live: crate::regs::Live,
    pre: Vec<bool>,
    pend: std::cell::RefCell<Vec<Option<LLVMValueRef>>>,
    /// Registers deferred by the instruction being generated.
    fresh: std::cell::RefCell<Vec<u32>>,
    /// The pc of the instruction being generated.
    pc: std::cell::Cell<u32>,
    /// Registers live over so many instructions (`MAX_SPAN`) that a deferred definition would be
    /// stored again at every call and guard exit it is live at, and be a value alive across
    /// blocks, which the unoptimised code generator keeps in a stack slot anyway: they are stored
    /// at the definition.
    keep: Vec<bool>,
    /// Registers carried across blocks as phis, when the function is small enough.
    plan: Option<Plan>,
    /// The instructions inside a loop. Outside loops a result is stored at once: a deferred
    /// one is stored at every block edge and guard exit it is live at (code, and codegen time
    /// beyond linear, for a function of hundreds of registers), and saves nothing that runs
    /// more than once.
    loopy: Vec<bool>,
}

/// The registers live into each block that are phis there (`regs`, ascending, and the phi of
/// each), per entry pc; empty for pcs that are no block start or are never reached.
struct Plan {
    regs: Vec<Vec<u32>>,
    phis: Vec<Vec<LLVMValueRef>>,
    /// Block starts reached by exactly one edge, from an earlier instruction: they take that
    /// edge's deferred values (`inherit`) as they are, with no phi.
    single: Vec<bool>,
    inherit: std::cell::RefCell<Vec<Option<Vec<Option<LLVMValueRef>>>>>,
}

/// The instructions a register may be live over and still be deferred (`Defer::keep`).
const MAX_SPAN: u32 = 4;

/// Phis of a function over this many (summed over its blocks) keep the stores at block edges.
const MAX_PHIS: usize = 100_000;

/// A native frame (`docs/decisions.md` "Compiled frames"): slots in an `alloca`, the high half
/// of its frame addresses, its frame index, the temporary area's length at entry, a word for
/// the helpers that write a register, and the function.
struct Nat {
    slots: LLVMValueRef,
    fhi: LLVMValueRef,
    depth: LLVMValueRef,
    /// The high halves of the addresses of the frames one and two static links up (as
    /// `TierFrame::up`): parameters of a nested subprogram, else `u64::MAX`.
    up: [LLVMValueRef; 2],
    temp_base: LLVMValueRef,
    scratch: LLVMValueRef,
    id: FuncId,
    /// Where the exit sites put the registers live at their pc, for `yavs_rt_deopt`: an
    /// `alloca` that escapes to the call only, so the registers themselves stay SSA values.
    spill: LLVMValueRef,
    /// The registers live before each instruction (none: every register is recorded).
    live: Option<crate::regs::Live>,
    /// The registers the interpreter starts a frame with (`regs::preloaded`): `materialise`
    /// keeps their preload and ignores the recorded word, so an exit does not store them.
    pre: Vec<bool>,
    /// The registers every exit from pc `core_from` on records the same way (`regs::entry_core`),
    /// and which they are (`is_core`): `core_stub` stores them for those exits together.
    core: Vec<u32>,
    is_core: Vec<bool>,
    core_from: u32,
    /// No address of the frame's slots ever leaves the function's own loads and stores
    /// (`closed_frame`, the ranges of its slots in use): the slots are not registered in
    /// `Mem::nslots` and LLVM may keep them in registers.
    closed: Option<Vec<(u64, u64)>>,
}

impl Gen {
    fn new(m: &Module) -> Gen {
        let (ctx, module) = (m.context(), m.raw());
        // SAFETY: ctx and module live as long as `m`; the builder is disposed in Drop.
        unsafe {
            let i64 = LLVMInt64TypeInContext(ctx);
            let i32 = LLVMInt32TypeInContext(ctx);
            let ptr = LLVMPointerTypeInContext(ctx, 0);
            let intrinsic = |name: &std::ffi::CStr| {
                let id = LLVMLookupIntrinsicID(name.as_ptr(), name.count_bytes());
                assert_ne!(id, 0, "unknown intrinsic {name:?}");
                let mut tys = [i64];
                (
                    LLVMIntrinsicGetType(ctx, id, tys.as_mut_ptr(), 1),
                    LLVMGetIntrinsicDeclaration(module, id, tys.as_mut_ptr(), 1),
                )
            };
            let mut params = [ptr, ptr, ptr, i32];
            let fn_ty = LLVMFunctionType(i32, params.as_mut_ptr(), 4, 0);
            let step_name = CString::new(STEP).expect("no NUL");
            let host_name = CString::new(HOST).expect("no NUL");
            let mut hparams = [ptr, ptr, i64, i64, i64, i32];
            let host_ty = LLVMFunctionType(ptr, hparams.as_mut_ptr(), 6, 0);
            let region_name = CString::new(REGION).expect("no NUL");
            let mut rparams = [ptr, ptr, i64, i64, i32, ptr];
            let region_ty =
                LLVMFunctionType(LLVMVoidTypeInContext(ctx), rparams.as_mut_ptr(), 6, 0);
            let signal_name = CString::new(SIGNAL).expect("no NUL");
            let mut sparams = [ptr, ptr, ptr, i32, i32];
            let signal_ty = LLVMFunctionType(i32, sparams.as_mut_ptr(), 5, 0);
            let drive_name = CString::new(DRIVE).expect("no NUL");
            let mut dparams = [ptr, ptr, ptr, i32, i32, i64, i64, ptr, i32];
            let drive_ty = LLVMFunctionType(i32, dparams.as_mut_ptr(), 9, 0);
            let drive_nat_name = CString::new(DRIVE_NAT).expect("no NUL");
            let mut nparams = [ptr, i32, i64, i64, ptr, i32];
            let drive_nat_ty = LLVMFunctionType(i32, nparams.as_mut_ptr(), 6, 0);
            let drive_bit_name = CString::new(DRIVE_BIT).expect("no NUL");
            let mut bparams = [ptr, ptr, ptr, i32, i32, i64, i64, i64, i64];
            let drive_bit_ty = LLVMFunctionType(i32, bparams.as_mut_ptr(), 9, 0);
            let wait_name = CString::new(WAIT).expect("no NUL");
            let wait_again_name = CString::new(WAIT_AGAIN).expect("no NUL");
            let wait_same_name = CString::new(WAIT_SAME).expect("no NUL");
            let mut wparams = [ptr, ptr, ptr, i32, i64, i64];
            let wait_ty = LLVMFunctionType(i32, wparams.as_mut_ptr(), 6, 0);
            let mut aparams = [ptr, i64, i64];
            let wait_again_ty = LLVMFunctionType(i32, aparams.as_mut_ptr(), 3, 0);
            let wait_edge_name = CString::new(WAIT_EDGE).expect("no NUL");
            let mut eparams = [ptr, ptr, i64, i64, i32];
            let wait_edge_ty = LLVMFunctionType(i32, eparams.as_mut_ptr(), 5, 0);
            let lcopy_name = CString::new(LCOPY).expect("no NUL");
            let mut lparams = [ptr, i64, i64, ptr, i64, i64, i64];
            let lcopy_ty = LLVMFunctionType(LLVMVoidTypeInContext(ctx), lparams.as_mut_ptr(), 7, 0);
            let fast_name = CString::new(FAST).expect("no NUL");
            let mut fparams = [ptr, ptr, ptr, i32, i32, i64, i64];
            let fast_ty = LLVMFunctionType(i32, fparams.as_mut_ptr(), 7, 0);
            let layout_name = CString::new(LAYOUT).expect("no NUL");
            let mut yparams = [ptr, ptr, ptr, i64, i32, i32, i64, i64, i64, i32];
            let layout_ty = LLVMFunctionType(i32, yparams.as_mut_ptr(), 10, 0);
            let mut dparams = [ptr, ptr, i32, i32, i64, i64, i64, i64, ptr];
            let deopt_ty = LLVMFunctionType(ptr, dparams.as_mut_ptr(), 9, 0);
            let mut dsparams = [ptr];
            let deopt_slots_ty = LLVMFunctionType(ptr, dsparams.as_mut_ptr(), 1, 0);
            let mut deparams = [ptr, ptr, i32];
            let deopt_end_ty = LLVMFunctionType(i32, deparams.as_mut_ptr(), 3, 0);
            let mut mparams = [ptr, ptr, i64];
            let memeq_ty = LLVMFunctionType(i64, mparams.as_mut_ptr(), 3, 0);
            let mut lwparams = [ptr, i32, ptr, i64, i32];
            let layout_word_ty = LLVMFunctionType(i64, lwparams.as_mut_ptr(), 5, 0);
            let mut scparams = [ptr, ptr, i64, i64, i64, i64, i64, i64, i64, ptr, i64];
            let scan_ty = LLVMFunctionType(i32, scparams.as_mut_ptr(), 11, 0);
            let mut elparams = [ptr, ptr, ptr, i64, ptr, i64, i64, i64, i64, i64, ptr, i64];
            let eloop_ty = LLVMFunctionType(i64, elparams.as_mut_ptr(), 12, 0);
            let mut exparams = [ptr, i64, i64, ptr, i64, i64, i64, i64];
            let eloop_exec_ty = LLVMFunctionType(i64, exparams.as_mut_ptr(), 8, 0);
            let mut cparams = [ptr, i64, i64, ptr];
            let codes_ty = LLVMFunctionType(LLVMVoidTypeInContext(ctx), cparams.as_mut_ptr(), 4, 0);
            let mut scaparams = [ptr, i64, ptr];
            let scalar_ty = LLVMFunctionType(i32, scaparams.as_mut_ptr(), 3, 0);
            let mut plparams = [ptr, i64, i64, i64, ptr];
            let planes_ty =
                LLVMFunctionType(LLVMVoidTypeInContext(ctx), plparams.as_mut_ptr(), 5, 0);
            let mut nparams = [ptr, ptr, i32, ptr, i32, i64, ptr];
            let native_ty = LLVMFunctionType(i32, nparams.as_mut_ptr(), 7, 0);
            let mut tparams = [ptr, ptr, i64, i32, ptr, i64];
            let troot_ty = LLVMFunctionType(i32, tparams.as_mut_ptr(), 6, 0);
            let mut eparams = [ptr, ptr, i64];
            let endstmt_ty =
                LLVMFunctionType(LLVMVoidTypeInContext(ctx), eparams.as_mut_ptr(), 3, 0);
            let mut xparams = [ptr, ptr, i32, i32, i64, ptr, ptr, i32, ptr, ptr, i32, i64];
            let xstep_ty = LLVMFunctionType(i32, xparams.as_mut_ptr(), 12, 0);
            let mut fl_params = [ptr, ptr, ptr, i32];
            let flush_ty = LLVMFunctionType(i32, fl_params.as_mut_ptr(), 4, 0);
            let add = |name: &str, ty| {
                let n = CString::new(name).expect("no NUL");
                LLVMAddFunction(module, n.as_ptr(), ty)
            };
            let frame_addr = {
                let id = LLVMLookupIntrinsicID(c"llvm.frameaddress".as_ptr(), 17);
                assert_ne!(id, 0, "unknown intrinsic llvm.frameaddress");
                let mut tys = [ptr];
                (
                    LLVMIntrinsicGetType(ctx, id, tys.as_mut_ptr(), 1),
                    LLVMGetIntrinsicDeclaration(module, id, tys.as_mut_ptr(), 1),
                )
            };
            let g = Gen {
                ctx,
                module,
                b: LLVMCreateBuilderInContext(ctx),
                i8: LLVMInt8TypeInContext(ctx),
                i32,
                i64,
                f64: LLVMDoubleTypeInContext(ctx),
                ptr,
                fn_ty,
                step: LLVMAddFunction(module, step_name.as_ptr(), fn_ty),
                host_ty,
                host: LLVMAddFunction(module, host_name.as_ptr(), host_ty),
                region_ty,
                region: LLVMAddFunction(module, region_name.as_ptr(), region_ty),
                signal_ty,
                signal: LLVMAddFunction(module, signal_name.as_ptr(), signal_ty),
                drive_ty,
                drive: LLVMAddFunction(module, drive_name.as_ptr(), drive_ty),
                drive_nat_ty,
                drive_nat: LLVMAddFunction(module, drive_nat_name.as_ptr(), drive_nat_ty),
                drive_bit_ty,
                drive_bit: LLVMAddFunction(module, drive_bit_name.as_ptr(), drive_bit_ty),
                wait_ty,
                wait: LLVMAddFunction(module, wait_name.as_ptr(), wait_ty),
                wait_again_ty,
                wait_again: LLVMAddFunction(module, wait_again_name.as_ptr(), wait_again_ty),
                wait_same: LLVMAddFunction(module, wait_same_name.as_ptr(), wait_ty),
                wait_edge_ty,
                wait_edge: LLVMAddFunction(module, wait_edge_name.as_ptr(), wait_edge_ty),
                lcopy_ty,
                lcopy: LLVMAddFunction(module, lcopy_name.as_ptr(), lcopy_ty),
                layout_ty,
                layout: LLVMAddFunction(module, layout_name.as_ptr(), layout_ty),
                fast_ty,
                fast: LLVMAddFunction(module, fast_name.as_ptr(), fast_ty),
                deopt_ty,
                deopt: add(DEOPT, deopt_ty),
                deopt_slots_ty,
                deopt_slots: add(DEOPT_SLOTS, deopt_slots_ty),
                deopt_end_ty,
                deopt_end: add(DEOPT_END, deopt_end_ty),
                memeq_ty,
                memeq: add(MEMEQ, memeq_ty),
                layout_word_ty,
                layout_word: add(LAYOUT_WORD, layout_word_ty),
                scan_ty,
                scan: add(SCAN, scan_ty),
                eloop_ty,
                eloop: add(ELOOP, eloop_ty),
                eloop_exec_ty,
                eloop_exec: add(ELOOP_EXEC, eloop_exec_ty),
                codes_ty,
                codes: add(CODES, codes_ty),
                scalar_ty,
                scalar: add(SCALAR, scalar_ty),
                planes_ty,
                planes: add(PLANES, planes_ty),
                native_ty,
                native: add(NATIVE, native_ty),
                native_light: add(NATIVE_LIGHT, native_ty),
                memo: add(MEMO, native_ty),
                memo_keep: add(MEMO_KEEP, native_ty),
                troot_ty,
                troot: add(TROOT, troot_ty),
                endstmt_ty,
                endstmt: add(ENDSTMT, endstmt_ty),
                xstep_ty,
                xstep: add(XSTEP, xstep_ty),
                flush_ty,
                flush: add(DRIVE_FLUSH, flush_ty),
                frame_addr,
                direct: false,
                cold_depth: Default::default(),
                kinds: Default::default(),
                aa: Default::default(),
                sadd: intrinsic(c"llvm.sadd.with.overflow"),
                ssub: intrinsic(c"llvm.ssub.with.overflow"),
                smul: intrinsic(c"llvm.smul.with.overflow"),
                umul: intrinsic(c"llvm.umul.with.overflow"),
                uadd: intrinsic(c"llvm.uadd.with.overflow"),
            };
            g.effects();
            g
        }
    }

    /// Attribute `name` (with `val` for an integer attribute) on function `f` or call `f` at
    /// `idx` (`LLVMAttributeFunctionIndex` for the function).
    unsafe fn attr(&self, f: LLVMValueRef, call: bool, name: &str, val: u64) {
        // SAFETY: a function or call of our context; the attribute exists in LLVM 21.
        unsafe {
            let kind = LLVMGetEnumAttributeKindForName(name.as_ptr().cast(), name.len());
            assert_ne!(kind, 0, "unknown attribute {name}");
            let a = LLVMCreateEnumAttribute(self.ctx, kind, val);
            let idx = llvm_sys::LLVMAttributeFunctionIndex;
            if call {
                LLVMAddCallSiteAttribute(f, idx, a);
            } else {
                LLVMAddAttributeAtIndex(f, idx, a);
            }
        }
    }

    /// What the runtime helpers do, so LLVM keeps values in registers across them and moves
    /// loads past them (`docs/decisions.md` "Compiled frames", effects): none unwinds (they
    /// catch panics); `yavs_rt_host` touches only `Mem`, `Env` and the interpreter's own
    /// bookkeeping, never object bytes, registers or slots; `yavs_rt_lcopy` only the blocks it
    /// is given. `yavs_rt_step` and `yavs_rt_signal` run interpreter code that may collect or
    /// change any memory: no memory attribute, so registers holding handles are reloaded after
    /// them and the precise roots (#422) see what compiled code stored.
    unsafe fn effects(&self) {
        // SAFETY: functions of our module.
        unsafe {
            for f in [
                self.step,
                self.host,
                self.region,
                self.signal,
                self.drive,
                self.drive_nat,
                self.wait,
                self.wait_again,
                self.wait_same,
                self.lcopy,
                self.layout,
                self.fast,
                self.deopt,
                self.deopt_slots,
                self.deopt_end,
                self.native,
                self.native_light,
                self.memo,
                self.memo_keep,
                self.troot,
                self.endstmt,
                self.xstep,
                self.scan,
                self.flush,
            ] {
                self.attr(f, false, "nounwind", 0);
            }
            self.attr(self.memeq, false, "nounwind", 0);
            self.attr(self.codes, false, "nounwind", 0);
            for a in ["nounwind", "willreturn", "nofree", "nosync"] {
                self.attr(self.scalar, false, a, 0);
            }
            self.attr(self.scalar, false, "memory", mem_effects(RW, 0, 0));
            self.attr(self.planes, false, "nounwind", 0);
            self.attr(self.planes, false, "willreturn", 0);
            self.attr(self.planes, false, "memory", mem_effects(RW, 0, 0));
            self.attr(self.planes, false, "nofree", 0);
            self.attr(self.planes, false, "nosync", 0);
            for a in ["nounwind", "willreturn", "nofree", "nosync"] {
                self.attr(self.eloop_exec, false, a, 0);
            }
            self.attr(self.eloop_exec, false, "memory", mem_effects(RW, 0, 0));
            for f in [
                self.host,
                self.region,
                self.lcopy,
                self.deopt_slots,
                self.memeq,
                self.codes,
            ] {
                self.attr(f, false, "willreturn", 0);
            }
            self.attr(self.memeq, false, "memory", mem_effects(REF, 0, 0));
            // The layout memo is host memory (and a panic is noted in `env`): the views stay.
            for a in ["nounwind", "willreturn", "nosync"] {
                self.attr(self.layout_word, false, a, 0);
            }
            self.attr(self.layout_word, false, "memory", mem_effects(RW, RW, 0));
            self.attr(self.memeq, false, "nofree", 0);
            self.attr(self.memeq, false, "nosync", 0);
            self.attr(self.host, false, "memory", mem_effects(RW, RW, 0));
            self.attr(self.region, false, "memory", mem_effects(RW, RW, 0));
            self.attr(self.lcopy, false, "memory", mem_effects(RW, 0, 0));
            self.attr(self.lcopy, false, "nofree", 0);
            self.attr(self.lcopy, false, "nosync", 0);
            self.attr(self.codes, false, "memory", mem_effects(RW, 0, 0));
            self.attr(self.codes, false, "nofree", 0);
            self.attr(self.codes, false, "nosync", 0);
            self.attr(self.deopt_slots, false, "memory", mem_effects(REF, 0, 0));
            if crate::jit::small_model() {
                let mut f = LLVMGetFirstFunction(self.module);
                while !f.is_null() {
                    if LLVMGetIntrinsicID(f) == 0 {
                        self.attr(f, false, "nonlazybind", 0);
                    }
                    f = LLVMGetNextFunction(f);
                }
            }
        }
    }

    fn c64(&self, v: i64) -> LLVMValueRef {
        // SAFETY: constant creation.
        unsafe { LLVMConstInt(self.i64, v as u64, 1) }
    }

    fn c32(&self, v: u32) -> LLVMValueRef {
        // SAFETY: constant creation.
        unsafe { LLVMConstInt(self.i32, u64::from(v), 0) }
    }

    /// The entry switch of frame-mode code: a tree of range compares over the sorted entry pcs,
    /// at most `LEAF` cases per `switch`. The unoptimised code generator lowers a `switch` to a
    /// chain of compares, and a large process is entered at one of thousands of pcs at every
    /// resume (a third of the instructions of a LEON3 `comb` copy were that chain, #679).
    /// The builder is at the end of `from`.
    unsafe fn entry_dispatch(
        &self,
        c: &Cur,
        from: LLVMBasicBlockRef,
        pc: LLVMValueRef,
        cases: &[(u32, LLVMBasicBlockRef)],
        default: LLVMBasicBlockRef,
    ) {
        const LEAF: usize = 4;
        // SAFETY: blocks and values are of the function being generated; the builder is at the
        // end of the block being filled.
        unsafe {
            if cases.len() <= LEAF {
                let sw = LLVMBuildSwitch(self.b, pc, default, cases.len() as u32);
                for &(v, bb) in cases {
                    LLVMAddCase(sw, self.c32(v), bb);
                }
                return;
            }
            let (lo, hi) = cases.split_at(cases.len() / 2);
            let (below, above) = (self.block(c), self.block(c));
            LLVMMoveBasicBlockAfter(above, from);
            LLVMMoveBasicBlockAfter(below, from);
            let lt = LLVMBuildICmp(self.b, IP::LLVMIntULT, pc, self.c32(hi[0].0), c"".as_ptr());
            LLVMBuildCondBr(self.b, lt, below, above);
            LLVMPositionBuilderAtEnd(self.b, below);
            self.entry_dispatch(c, below, pc, lo, default);
            LLVMPositionBuilderAtEnd(self.b, above);
            self.entry_dispatch(c, above, pc, hi, default);
        }
    }

    unsafe fn block(&self, c: &Cur) -> LLVMBasicBlockRef {
        // SAFETY: c.f is a function of our context.
        unsafe { LLVMAppendBasicBlockInContext(self.ctx, c.f, c"".as_ptr()) }
    }

    unsafe fn slot(&self, c: &Cur, r: Reg) -> LLVMValueRef {
        // SAFETY: r is below the register count (`bad_reg`).
        unsafe {
            let mut idx = [self.c64(i64::from(r.0))];
            LLVMBuildGEP2(self.b, self.i64, c.regs, idx.as_mut_ptr(), 1, c"".as_ptr())
        }
    }

    /// What the generator knows about address register `r` (not an integer).
    fn known_addr(&self, c: &Cur, r: Reg) -> Option<Known> {
        match c.known.borrow().get(r.0 as usize) {
            Some(&Some(k)) if !matches!(k, Known::Int(_)) => Some(k),
            _ => None,
        }
    }

    /// The value of register `r` when the generator knows it.
    fn kint(&self, c: &Cur, r: Reg) -> Option<i64> {
        match c.known.borrow().get(r.0 as usize) {
            Some(Some(Known::Int(v))) => Some(*v),
            _ => c.fixed.get(r.0 as usize).copied().flatten(),
        }
    }

    unsafe fn ld(&self, c: &Cur, r: Reg) -> LLVMValueRef {
        if let Some(v) = self.kint(c, r) {
            return self.c64(v);
        }
        if let Some(d) = &c.defer
            && let Some(Some(v)) = d.pend.borrow().get(r.0 as usize)
        {
            return *v;
        }
        // SAFETY: see `slot`.
        unsafe { LLVMBuildLoad2(self.b, self.i64, self.slot(c, r), c"".as_ptr()) }
    }

    /// Return `v` (an i32): a branch to the function's exit block.
    unsafe fn leave(&self, c: &Cur, v: LLVMValueRef) {
        // SAFETY: the builder is in a block of c.f; the phi is the exit block's.
        unsafe {
            let shared = match &c.nat {
                Some(n) => self.spill_live(c, n, v),
                None => false,
            };
            let to = if shared {
                self.core_stub(c, v)
            } else {
                self.exit_for(c, v)
            };
            LLVMBuildBr(self.b, to);
        }
    }

    /// A native frame leaves to be materialised at the pc `v`: store the registers the
    /// interpreter reads from there (all of them when `v` is no constant or liveness is not
    /// known) in the spill buffer. Only these are live up to this exit, so the other registers
    /// need not stay in machine registers or stack slots up to every guard. Not stored: the
    /// registers the interpreter preloads, and, after the first block, the core registers,
    /// which `core_stub` stores for the exit (the result is true then).
    unsafe fn spill_live(&self, c: &Cur, n: &Nat, v: LLVMValueRef) -> bool {
        // SAFETY: the builder is in a block of c.f; `spill` has one word per register.
        unsafe {
            let pc =
                (!LLVMIsAConstantInt(v).is_null()).then(|| LLVMConstIntGetZExtValue(v) as usize);
            let row = pc.zip(n.live.as_ref()).map(|(pc, l)| l.row(pc));
            let shared = !n.core.is_empty() && pc.is_some_and(|pc| pc >= n.core_from as usize);
            let nregs = c.known.borrow().len();
            for r in 0..nregs {
                if row.is_some_and(|w| w[r / 64] >> (r % 64) & 1 == 0)
                    || n.pre[r]
                    || shared && n.is_core[r]
                {
                    continue;
                }
                let at = self.gep(n.spill, self.c64(r as i64 * 8));
                self.store(self.ld(c, Reg(r as u32)), at);
            }
            shared
        }
    }

    /// The block that records the core registers and goes on to the exit with the pc `v`, for
    /// the exit being generated: one block for up to `EXIT_PHI` exits, with a phi of their pcs.
    /// The core registers are defined in the first block, which dominates every such exit, and
    /// have no other definition, so their current value is the one they have there.
    unsafe fn core_stub(&self, c: &Cur, v: LLVMValueRef) -> LLVMBasicBlockRef {
        // SAFETY: well-formed blocks of c.f; a native frame, so `nat` is set.
        unsafe {
            let n = c.nat.as_ref().expect("native frame");
            let from = LLVMGetInsertBlock(self.b);
            let mut cs = c.cores.borrow_mut();
            if cs.last().is_none_or(|x| x.2 >= EXIT_PHI) {
                let blk = self.block(c);
                LLVMPositionBuilderAtEnd(self.b, blk);
                let phi = LLVMBuildPhi(self.b, self.i32, c"".as_ptr());
                for &r in &n.core {
                    let val = LLVMBuildLoad2(self.b, self.i64, self.slot(c, Reg(r)), c"".as_ptr());
                    let at = self.gep(n.spill, self.c64(i64::from(r) * 8));
                    self.store(val, at);
                }
                let exit = self.exit_for(c, phi);
                LLVMBuildBr(self.b, exit);
                LLVMPositionBuilderAtEnd(self.b, from);
                cs.push((blk, phi, 0));
            }
            let x = cs.last_mut().expect("core block");
            LLVMAddIncoming(x.1, &mut { v }, &mut { from }, 1);
            x.2 += 1;
            x.0
        }
    }

    /// The exit block to branch to from the current block with `v` (an i32) to return: `v` is
    /// added to its phi. A frame-mode function has several (`EXIT_PHI` predecessors each): one
    /// phi of thousands of incoming values makes the simplifier and the phi elimination
    /// quadratic.
    unsafe fn exit_for(&self, c: &Cur, v: LLVMValueRef) -> LLVMBasicBlockRef {
        // SAFETY: well-formed blocks of c.f.
        unsafe {
            let from = LLVMGetInsertBlock(self.b);
            let mut xs = c.exits.borrow_mut();
            if xs.last().is_none_or(|x| x.2 >= EXIT_PHI) {
                let blk = self.block(c);
                LLVMPositionBuilderAtEnd(self.b, blk);
                let phi = LLVMBuildPhi(self.b, self.i32, c"".as_ptr());
                if c.batch.is_some() {
                    self.ret_exit(c, phi);
                } else if c.nat.is_none() {
                    self.spin_out(c);
                    LLVMBuildRet(self.b, phi);
                } else {
                    // The native frame's one exit (materialisation) takes the chunks' values.
                    let (exit, exit_pc) = c.exit;
                    LLVMBuildBr(self.b, exit);
                    LLVMAddIncoming(exit_pc, &mut { phi }, &mut { blk }, 1);
                }
                LLVMPositionBuilderAtEnd(self.b, from);
                xs.push((blk, phi, 0));
            }
            let x = xs.last_mut().expect("exit block");
            LLVMAddIncoming(x.1, &mut { v }, &mut { from }, 1);
            x.2 += 1;
            x.0
        }
    }

    /// Hand the jump count of a process that never suspends back to the machine, before a return.
    unsafe fn spin_out(&self, c: &Cur) {
        // SAFETY: `spin` is the entry block's local; the builder is in a block of c.f.
        unsafe {
            if let Some(local) = c.spin {
                let n = LLVMBuildLoad2(self.b, self.i64, local, c"".as_ptr());
                let at = self.tag(self.mem_ptr(c, MEM_SPIN), AA_MEM);
                let p = self.tag(self.load(self.ptr, at), AA_SPIN);
                self.store(n, p);
            }
        }
    }

    /// End the exit block the builder is in (its phi `v`): hand over the kept scalar drives, if
    /// any, then return `v`, or the pc of a drive the kernel refused when that failed.
    unsafe fn ret_exit(&self, c: &Cur, v: LLVMValueRef) {
        // SAFETY: well-formed blocks of c.f; `batch` is set.
        unsafe {
            let b = c.batch.expect("batch");
            self.spin_out(c);
            let (here, flush, done) = (LLVMGetInsertBlock(self.b), self.block(c), self.block(c));
            let n = self.load(self.i32, b.count);
            let any = self.icmp(IP::LLVMIntNE, n, self.c32(0));
            self.cond_br(any, flush, done, false);
            LLVMPositionBuilderAtEnd(self.b, flush);
            let r = self.batch_call(c, n);
            let bad = self.icmp(IP::LLVMIntNE, r, self.c32(GO));
            let w = LLVMBuildSelect(self.b, bad, r, v, c"".as_ptr());
            LLVMBuildBr(self.b, done);
            LLVMPositionBuilderAtEnd(self.b, done);
            let out = LLVMBuildPhi(self.b, self.i32, c"".as_ptr());
            LLVMAddIncoming(out, &mut { v }, &mut { here }, 1);
            LLVMAddIncoming(out, &mut { w }, &mut { flush }, 1);
            LLVMBuildRet(self.b, out);
        }
    }

    /// `yavs_rt_drive_flush` of the `n` kept entries.
    unsafe fn batch_call(&self, c: &Cur, n: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: the arguments match `flush_ty`.
        unsafe {
            let b = c.batch.expect("batch");
            let mut args = [c.env, c.mem, b.entries, n];
            let r = LLVMBuildCall2(
                self.b,
                self.flush_ty,
                self.flush,
                args.as_mut_ptr(),
                4,
                c"".as_ptr(),
            );
            self.attr(r, true, "cold", 0);
            r
        }
    }

    /// Hand over the kept scalar drives before the instruction being generated, when there are
    /// some; a refused one leaves the function.
    unsafe fn batch_flush(&self, c: &Cur) {
        // SAFETY: well-formed blocks of c.f; `batch` is set.
        unsafe {
            let b = c.batch.expect("batch");
            let (flush, cont) = (self.block(c), self.block(c));
            c.cold.borrow_mut().push(flush);
            let n = self.load(self.i32, b.count);
            self.cond_br(self.icmp(IP::LLVMIntNE, n, self.c32(0)), flush, cont, false);
            LLVMPositionBuilderAtEnd(self.b, flush);
            let r = self.batch_call(c, n);
            LLVMBuildStore(self.b, self.c32(0), b.count);
            let exit = self.exit_for(c, r);
            self.cond_br(self.icmp(IP::LLVMIntEQ, r, self.c32(GO)), cont, exit, true);
            LLVMPositionBuilderAtEnd(self.b, cont);
        }
    }

    unsafe fn ldf(&self, c: &Cur, r: Reg) -> LLVMValueRef {
        // SAFETY: see `slot`.
        unsafe { LLVMBuildBitCast(self.b, self.ld(c, r), self.f64, c"".as_ptr()) }
    }

    /// Store `v` in register `r` at once.
    unsafe fn st(&self, c: &Cur, r: Reg, v: LLVMValueRef) {
        if let Some(d) = &c.defer
            && let Some(p) = d.pend.borrow_mut().get_mut(r.0 as usize)
        {
            *p = None;
        }
        // SAFETY: see `slot`; `v` is an i64.
        unsafe {
            LLVMBuildStore(self.b, v, self.slot(c, r));
        }
    }

    /// Define register `r` as `v` without a store: `flush` stores it where the frame is looked
    /// at. Only in straight-line code (not between the fast and slow path of an instruction).
    unsafe fn std(&self, c: &Cur, r: Reg, v: LLVMValueRef) {
        // The interpreter starts every frame with a register defined once by a constant or an
        // address (`Pre::fill`): its definition stores nothing, in loops or not.
        if let Some(d) = &c.defer
            && d.pre.get(r.0 as usize) == Some(&true)
        {
            if let Some(p) = d.pend.borrow_mut().get_mut(r.0 as usize) {
                *p = None;
            }
            return;
        }
        if let Some(d) = &c.defer
            && d.loopy[d.pc.get() as usize]
            && !d.keep.get(r.0 as usize).copied().unwrap_or(false)
            && let Some(p) = d.pend.borrow_mut().get_mut(r.0 as usize)
        {
            *p = Some(v);
            d.fresh.borrow_mut().push(r.0);
            return;
        }
        // SAFETY: as `st`.
        unsafe { self.st(c, r, v) }
    }

    unsafe fn stfd(&self, c: &Cur, r: Reg, v: LLVMValueRef) {
        // SAFETY: `v` is a double.
        unsafe { self.std(c, r, LLVMBuildBitCast(self.b, v, self.i64, c"".as_ptr())) }
    }

    /// Store the deferred registers that `live` (a row of `Live`) has live.
    unsafe fn flush_row(&self, c: &Cur, live: &[u64]) {
        let Some(d) = &c.defer else { return };
        let mut base = None;
        for (r, v) in d.pend.borrow().iter().enumerate() {
            if let Some(v) = v
                && live.get(r / 64).is_some_and(|w| w >> (r % 64) & 1 != 0)
            {
                // SAFETY: r is a register of the function, or a slot variable.
                unsafe {
                    let nregs = c.known.borrow().len();
                    if r < nregs {
                        LLVMBuildStore(self.b, *v, self.slot(c, Reg(r as u32)));
                    } else if c.scal.as_ref().is_some_and(|s| s.stored[r - nregs]) {
                        let b = *base.get_or_insert_with(|| self.tag(self.own_slots(c), AA_SLOTS));
                        self.vslot_store_at(c, r - nregs, *v, b);
                    }
                }
            }
        }
    }

    /// Write deferred value `v` of register (or, past the registers, slot variable) `r` to the frame.
    unsafe fn store_back(&self, c: &Cur, r: usize, v: LLVMValueRef) {
        let nregs = c.known.borrow().len();
        // SAFETY: r is a register of the function, or a slot variable.
        unsafe {
            if r < nregs {
                LLVMBuildStore(self.b, v, self.slot(c, Reg(r as u32)));
            } else if c.scal.as_ref().is_some_and(|s| s.stored[r - nregs]) {
                self.vslot_store(c, r - nregs, v);
            }
        }
    }

    /// Store the registers (not the slot variables) live at `pc`; the slot variables are
    /// returned as (slot, value) pairs, for `leave_slots`.
    unsafe fn flush_regs(&self, c: &Cur, pc: u32) -> Vec<(usize, usize)> {
        let Some(d) = &c.defer else { return Vec::new() };
        let nregs = c.known.borrow().len();
        let row = d.live.row(pc as usize);
        let mut key = Vec::new();
        for (r, v) in d.pend.borrow().iter().enumerate() {
            let Some(v) = v else { continue };
            if row[r / 64] >> (r % 64) & 1 == 0 {
                continue;
            }
            if r < nregs {
                // SAFETY: r is a register of the function.
                unsafe { LLVMBuildStore(self.b, *v, self.slot(c, Reg(r as u32))) };
            } else if c.scal.as_ref().is_some_and(|s| s.stored[r - nregs]) {
                key.push((r - nregs, *v as usize));
            }
        }
        key
    }

    /// Leave with `val` (an i32), the slot variables of `key` written back first.
    unsafe fn leave_slots(&self, c: &Cur, key: Vec<(usize, usize)>, val: LLVMValueRef) {
        // SAFETY: well-formed blocks of c.f; the values dominate the builder's block.
        unsafe {
            if !key.is_empty() {
                let base = self.tag(self.own_slots(c), AA_SLOTS);
                for &(k, v) in &key {
                    self.vslot_store_at(c, k, v as LLVMValueRef, base);
                }
            }
            self.leave(c, val);
        }
    }

    /// Leave to the interpreter at `pc` (a guard failed), with the frame as it reads it.
    unsafe fn flush_exit(&self, c: &Cur, pc: u32) {
        // SAFETY: as `leave_slots`.
        unsafe {
            let key = self.flush_regs(c, pc);
            self.leave_slots(c, key, self.c32(pc));
        }
    }

    /// Store what the interpreter reads when it takes over at instruction `pc`.
    unsafe fn flush(&self, c: &Cur, pc: usize) {
        if let Some(d) = &c.defer {
            // SAFETY: as `flush_row`.
            unsafe { self.flush_row(c, d.live.row(pc)) };
        }
    }

    /// Store what the successors `pcs` read.
    unsafe fn flush_edge(&self, c: &Cur, pcs: &[usize]) {
        let Some(d) = &c.defer else { return };
        let mut row = d.live.row(pcs[0]).to_vec();
        for &p in &pcs[1..] {
            for (a, b) in row.iter_mut().zip(d.live.row(p)) {
                *a |= b;
            }
        }
        // SAFETY: as `flush_row`.
        unsafe { self.flush_row(c, &row) };
    }

    /// At a branch to the block starts `pcs`: store what they read (without `Plan`), or the
    /// values their phis take.
    unsafe fn edge_prep(&self, c: &Cur, pcs: &[usize]) -> Vec<(u32, LLVMValueRef)> {
        let Some(d) = &c.defer else { return Vec::new() };
        let Some(pl) = &d.plan else {
            // SAFETY: as `flush_edge`.
            unsafe { self.flush_edge(c, pcs) };
            return Vec::new();
        };
        let pcs: Vec<usize> = pcs.iter().copied().filter(|&p| !pl.single[p]).collect();
        let mut regs: Vec<u32> = pcs
            .iter()
            .flat_map(|&p| pl.regs[p].iter().copied())
            .collect();
        regs.sort_unstable();
        regs.dedup();
        // A deferred value no phi takes is stored for the successor, as without phis.
        let mut back: Vec<usize> = Vec::new();
        for &p in &pcs {
            let row = d.live.row(p);
            for (r, v) in d.pend.borrow().iter().enumerate() {
                if v.is_some()
                    && row[r / 64] >> (r % 64) & 1 != 0
                    && pl.regs[p].binary_search(&(r as u32)).is_err()
                    && !back.contains(&r)
                {
                    back.push(r);
                }
            }
        }
        for r in back {
            // SAFETY: as `flush_row`.
            unsafe { self.store_back(c, r, d.pend.borrow()[r].expect("deferred")) };
        }
        regs.into_iter()
            .map(|r| {
                // SAFETY: r is a register of the function.
                let v =
                    d.pend.borrow()[r as usize].unwrap_or_else(|| unsafe { self.frame_load(c, r) });
                (r, v)
            })
            .collect()
    }

    /// The edge from the current block to block start `pc`: its phis take `snap`.
    unsafe fn edge_to(&self, c: &Cur, snap: &[(u32, LLVMValueRef)], pc: usize) {
        let Some(d) = &c.defer else { return };
        let Some(pl) = &d.plan else { return };
        if pl.single[pc] {
            pl.inherit.borrow_mut()[pc].get_or_insert_with(|| d.pend.borrow().clone());
            return;
        }
        // SAFETY: the phis are of blocks of c.f and take i64.
        unsafe {
            let from = LLVMGetInsertBlock(self.b);
            for (&r, &phi) in pl.regs[pc].iter().zip(&pl.phis[pc]) {
                if let Some(&(_, mut v)) = snap.iter().find(|x| x.0 == r) {
                    LLVMAddIncoming(phi, &mut v, &mut { from }, 1);
                }
            }
        }
    }

    /// Register `r` (below the register count) or slot variable `r - nregs` as the frame has it.
    unsafe fn frame_load(&self, c: &Cur, r: u32) -> LLVMValueRef {
        let nregs = c.known.borrow().len();
        let base = c
            .defer
            .as_ref()
            .map_or(usize::MAX, |d| d.pend.borrow().len() - 1);
        // SAFETY: r is a register of the function, a slot variable, or the slots' base.
        unsafe {
            if r as usize == base {
                self.own_slots(c)
            } else if (r as usize) < nregs {
                LLVMBuildLoad2(self.b, self.i64, self.slot(c, Reg(r)), c"".as_ptr())
            } else {
                self.vslot_load(c, r as usize - nregs, false)
            }
        }
    }

    /// The address of slot variable `k` in the frame's slots.
    unsafe fn vslot_ptr(&self, c: &Cur, k: usize, cached: bool) -> (LLVMValueRef, Width) {
        let (off, w) = c.scal.as_ref().expect("slot variables").slots[k];
        // SAFETY: the slot lies within the frame (`Scalars::new`).
        unsafe {
            let base = if cached {
                self.own_slots_cached(c)
            } else {
                self.own_slots(c)
            };
            (
                self.gep(self.tag(base, AA_SLOTS), self.c64(i64::from(off))),
                w,
            )
        }
    }

    /// Slot variable `k` as the frame holds it, widened as `Load` does.
    unsafe fn vslot_load(&self, c: &Cur, k: usize, cached: bool) -> LLVMValueRef {
        // SAFETY: as `vslot_ptr`.
        unsafe {
            let (p, w) = self.vslot_ptr(c, k, cached);
            let v = self.load(self.width_ty(w), p);
            match w {
                Width::B8 => v,
                Width::B4 => LLVMBuildSExt(self.b, v, self.i64, c"".as_ptr()),
                _ => LLVMBuildZExt(self.b, v, self.i64, c"".as_ptr()),
            }
        }
    }

    /// Write slot variable `k` (widened as `vslot_load` reads it) to the frame.
    unsafe fn vslot_store(&self, c: &Cur, k: usize, v: LLVMValueRef) {
        // SAFETY: as `vslot_ptr`.
        unsafe { self.vslot_store_at(c, k, v, self.tag(self.own_slots(c), AA_SLOTS)) }
    }

    /// `vslot_store` with the frame's slots at `base`.
    unsafe fn vslot_store_at(&self, c: &Cur, k: usize, v: LLVMValueRef, base: LLVMValueRef) {
        let (off, w) = c.scal.as_ref().expect("slot variables").slots[k];
        // SAFETY: the slot lies within the frame (`Scalars::new`).
        unsafe {
            let p = self.gep(base, self.c64(i64::from(off)));
            let v = if w == Width::B8 {
                v
            } else {
                LLVMBuildTrunc(self.b, v, self.width_ty(w), c"".as_ptr())
            };
            self.store(v, p);
        }
    }

    /// The value of slot variable `k` here: the deferred one, else the frame's (kept for the block).
    unsafe fn vslot_value(&self, c: &Cur, k: usize) -> LLVMValueRef {
        let nregs = c.known.borrow().len();
        let d = c.defer.as_ref().expect("slot variables");
        if let Some(v) = d.pend.borrow()[nregs + k] {
            return v;
        }
        // SAFETY: as `vslot_load`.
        let v = unsafe { self.vslot_load(c, k, true) };
        d.pend.borrow_mut()[nregs + k] = Some(v);
        v
    }

    /// The builder is at the start of the block of `pc`: what its phis hold is in registers.
    fn begin_block(&self, c: &Cur, pc: usize) {
        self.drop_pend(c);
        if let Some(d) = &c.defer
            && let Some(pl) = &d.plan
        {
            if pl.single[pc] {
                if let Some(p) = pl.inherit.borrow_mut()[pc].take() {
                    *d.pend.borrow_mut() = p;
                }
                return;
            }
            let mut pend = d.pend.borrow_mut();
            for (&r, &phi) in pl.regs[pc].iter().zip(&pl.phis[pc]) {
                pend[r as usize] = Some(phi);
            }
        }
    }

    /// Forget the deferred registers (the frame has them all, or nothing reaches the code).
    fn drop_pend(&self, c: &Cur) {
        if let Some(d) = &c.defer {
            d.pend.borrow_mut().fill(None);
        }
    }

    unsafe fn zext(&self, v: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: v is an i1.
        unsafe { LLVMBuildZExt(self.b, v, self.i64, c"".as_ptr()) }
    }

    unsafe fn icmp(&self, p: IP, a: LLVMValueRef, b: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: a and b have one integer type.
        unsafe { LLVMBuildICmp(self.b, p, a, b, c"".as_ptr()) }
    }

    unsafe fn or(&self, a: LLVMValueRef, b: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: two i1.
        unsafe { LLVMBuildOr(self.b, a, b, c"".as_ptr()) }
    }

    /// Leave to the interpreter at `pc` when `bad` holds; the builder continues after it.
    unsafe fn exit_if(&self, c: &Cur, bad: LLVMValueRef, pc: u32) {
        // SAFETY: well-formed blocks of c.f.
        unsafe {
            let out = self.block(c);
            let cont = self.block(c);
            c.cold.borrow_mut().push(out);
            self.cond_br(bad, out, cont, false);
            LLVMPositionBuilderAtEnd(self.b, out);
            self.flush_exit(c, pc);
            LLVMPositionBuilderAtEnd(self.b, cont);
        }
    }

    /// `intrinsic(a, b)` with its overflow flag: (value, overflowed).
    unsafe fn ovf(
        &self,
        (ty, f): (LLVMTypeRef, LLVMValueRef),
        a: LLVMValueRef,
        b: LLVMValueRef,
    ) -> (LLVMValueRef, LLVMValueRef) {
        // SAFETY: the intrinsic takes two i64.
        unsafe {
            let mut args = [a, b];
            let r = LLVMBuildCall2(self.b, ty, f, args.as_mut_ptr(), 2, c"".as_ptr());
            (
                LLVMBuildExtractValue(self.b, r, 0, c"".as_ptr()),
                LLVMBuildExtractValue(self.b, r, 1, c"".as_ptr()),
            )
        }
    }

    /// `x op y` as `yavs_std::arith`, leaving at `pc` wherever it reports an error.
    /// `None` for `**`, which goes through the interpreter.
    unsafe fn int_op(
        &self,
        c: &Cur,
        op: IntOp,
        x: LLVMValueRef,
        y: LLVMValueRef,
        pc: u32,
    ) -> Option<LLVMValueRef> {
        // SAFETY: well-formed IR in the current block.
        unsafe {
            let checked = |i| {
                let (v, o) = self.ovf(i, x, y);
                self.exit_if(c, o, pc);
                v
            };
            Some(match op {
                IntOp::Add => checked(self.sadd),
                IntOp::Sub => checked(self.ssub),
                IntOp::Mul => checked(self.smul),
                IntOp::Div | IntOp::Rem | IntOp::Mod => {
                    let zero = self.icmp(IP::LLVMIntEQ, y, self.c64(0));
                    let m1 = self.icmp(IP::LLVMIntEQ, y, self.c64(-1));
                    if op == IntOp::Div {
                        let min = self.icmp(IP::LLVMIntEQ, x, self.c64(i64::MIN));
                        let o = LLVMBuildAnd(self.b, min, m1, c"".as_ptr());
                        self.exit_if(c, self.or(zero, o), pc);
                        return Some(LLVMBuildSDiv(self.b, x, y, c"".as_ptr()));
                    }
                    self.exit_if(c, zero, pc);
                    // x rem -1 is 0; srem would trap on MIN rem -1.
                    let ys = LLVMBuildSelect(self.b, m1, self.c64(1), y, c"".as_ptr());
                    let r = LLVMBuildSRem(self.b, x, ys, c"".as_ptr());
                    if op == IntOp::Rem {
                        return Some(r);
                    }
                    let nz = self.icmp(IP::LLVMIntNE, r, self.c64(0));
                    let rn = self.icmp(IP::LLVMIntSLT, r, self.c64(0));
                    let yn = self.icmp(IP::LLVMIntSLT, y, self.c64(0));
                    let diff = LLVMBuildXor(self.b, rn, yn, c"".as_ptr());
                    let fix = LLVMBuildAnd(self.b, nz, diff, c"".as_ptr());
                    let sum = LLVMBuildAdd(self.b, r, y, c"".as_ptr());
                    LLVMBuildSelect(self.b, fix, sum, r, c"".as_ptr())
                }
                IntOp::Pow => return None,
            })
        }
    }

    /// Generate `name` for `f`; on a verifier failure the function is deleted again.
    /// The type of `f`'s native-frame code: `(env, mem, depth, argument cells...) -> i64`.
    unsafe fn native_ty(&self, f: &Func) -> LLVMTypeRef {
        let cells: usize = f
            .params
            .iter()
            .map(|p| 1 + usize::from(p.width == Width::Sig))
            .sum();
        let mut params = vec![self.ptr, self.ptr, self.i64];
        if f.parent.is_some() {
            params.extend([self.i64, self.i64]);
        }
        params.extend(std::iter::repeat_n(self.i64, cells));
        // SAFETY: types of our context.
        unsafe { LLVMFunctionType(self.i64, params.as_mut_ptr(), params.len() as u32, 0) }
    }

    /// `f<id>_c(env, mem, depth, cells) -> i64` (`CallFn`): the native-frame code `nname` of `f`
    /// with its argument cells read from an array, for calls from the interpreter (`Tier::call`).
    unsafe fn adapter(&self, f: &Func, nname: &str, name: &str) {
        // SAFETY: values of our context with matching types.
        unsafe {
            let mut ps = [self.ptr, self.ptr, self.i64, self.ptr];
            let ty = LLVMFunctionType(self.i64, ps.as_mut_ptr(), 4, 0);
            let fv = self.declare(name, ty);
            let bb = LLVMAppendBasicBlockInContext(self.ctx, fv, c"".as_ptr());
            LLVMPositionBuilderAtEnd(self.b, bb);
            let cells = LLVMGetParam(fv, 3);
            let n: usize = f
                .params
                .iter()
                .map(|p| 1 + usize::from(p.width == Width::Sig))
                .sum();
            let mut args = vec![
                LLVMGetParam(fv, 0),
                LLVMGetParam(fv, 1),
                LLVMGetParam(fv, 2),
            ];
            for k in 0..n {
                args.push(self.load(self.i64, self.gep(cells, self.c64(8 * k as i64))));
            }
            let nty = self.native_ty(f);
            let callee = self.declare(nname, nty);
            let v = LLVMBuildCall2(
                self.b,
                nty,
                callee,
                args.as_mut_ptr(),
                args.len() as u32,
                c"".as_ptr(),
            );
            LLVMBuildRet(self.b, v);
        }
    }

    /// The function named `name` with type `ty`, declared if not yet in the module.
    unsafe fn declare(&self, name: &str, ty: LLVMTypeRef) -> LLVMValueRef {
        let cname = CString::new(name).expect("no NUL");
        // SAFETY: a function of our module.
        unsafe {
            let f = LLVMGetNamedFunction(self.module, cname.as_ptr());
            if f.is_null() {
                let f = LLVMAddFunction(self.module, cname.as_ptr(), ty);
                if crate::jit::small_model() {
                    self.attr(f, false, "nonlazybind", 0);
                }
                f
            } else {
                f
            }
        }
    }

    /// Generate `f` (`id`) as `name`: its frame-mode code, or with `native` its native-frame
    /// code (`docs/decisions.md` "Compiled frames"). Calls of functions in `nc` are native
    /// calls.
    #[allow(clippy::too_many_arguments)]
    unsafe fn func(
        &self,
        prog: &Program,
        f: &Func,
        id: FuncId,
        name: &str,
        native: bool,
        nc: &std::collections::HashSet<FuncId>,
    ) -> Result<(), String> {
        // SAFETY: all values are built in this function with matching types.
        unsafe {
            let fty = if native {
                self.native_ty(f)
            } else {
                self.fn_ty
            };
            let fv = self.declare(name, fty);
            let kloc = self.kindloc_begin(fv, name, id.0);
            // `regs` and `mem` overlap no other memory the function reaches (frame slots,
            // temporaries, objects), so LLVM may keep their values in registers between
            // calls; the callbacks get both pointers.
            let kind = LLVMGetEnumAttributeKindForName(c"noalias".as_ptr(), 7);
            let restrict: &[u32] = if native { &[2] } else { &[2, 3] };
            for &i in restrict {
                LLVMAddAttributeAtIndex(fv, i, LLVMCreateEnumAttribute(self.ctx, kind, 0));
            }
            // The scalar logic assignments the interpreter runs as one op: the waveform is not
            // made here either (its instructions are `skipped`, the `Drive` is the step). Those
            // without a delay are kept and handed over together (`Batch`).
            let fixed = fixed_consts(f);
            let (scalar_drives, batched) = if !native && f.kind == FuncKind::Process {
                batched_drives(prog, f, &fixed)
            } else {
                Default::default()
            };
            let exit = LLVMAppendBasicBlockInContext(self.ctx, fv, c"".as_ptr());
            LLVMPositionBuilderAtEnd(self.b, exit);
            let exit_pc = LLVMBuildPhi(self.b, self.i32, c"".as_ptr());
            if !native && batched.is_empty() {
                LLVMBuildRet(self.b, exit_pc);
            }
            let entry = LLVMAppendBasicBlockInContext(self.ctx, fv, c"".as_ptr());
            LLVMMoveBasicBlockBefore(entry, exit);
            LLVMPositionBuilderAtEnd(self.b, entry);
            let nregs = f.reg_ty.len();
            let (regs, mem, nat) = if native {
                let rty = LLVMArrayType2(self.i64, nregs.max(1) as u64);
                let regs = LLVMBuildAlloca(self.b, rty, c"".as_ptr());
                let sty = LLVMArrayType2(self.i64, (u64::from(f.frame)).div_ceil(8).max(1));
                let slots = LLVMBuildAlloca(self.b, sty, c"".as_ptr());
                let scratch = LLVMBuildAlloca(self.b, self.i64, c"".as_ptr());
                let spill = LLVMBuildAlloca(self.b, rty, c"".as_ptr());
                let tbase = LLVMBuildAlloca(self.b, self.i64, c"".as_ptr());
                let depth = LLVMGetParam(fv, 2);
                let tag = self.c64((u64::from(space::FRAME) << 29) as i64);
                let fhi = LLVMBuildOr(self.b, tag, depth, c"".as_ptr());
                let (up1, up2) = if f.parent.is_some() {
                    (LLVMGetParam(fv, 3), LLVMGetParam(fv, 4))
                } else {
                    (self.c64(-1), self.c64(-1))
                };
                let live = crate::regs::Live::new(prog, f, None);
                let pre = crate::regs::preloaded(f);
                let (core, core_from) = live.as_ref().map_or((Vec::new(), 0), |l| {
                    crate::regs::entry_core(prog, f, l, &pre)
                });
                let mut is_core = vec![false; nregs];
                for &r in &core {
                    is_core[r as usize] = true;
                }
                let nat = Nat {
                    slots,
                    fhi,
                    depth,
                    up: [up1, up2],
                    temp_base: tbase,
                    scratch,
                    id,
                    spill,
                    live,
                    pre,
                    core,
                    is_core,
                    core_from,
                    closed: closed_frame(prog, f, id),
                };
                (regs, LLVMGetParam(fv, 1), Some(nat))
            } else {
                (LLVMGetParam(fv, 1), LLVMGetParam(fv, 2), None)
            };
            // The jumps back that poll the stop request (#737), see `Inst::Br`.
            let backs: std::collections::HashSet<u32> = if !native
                && f.kind == FuncKind::Process
                && !f.flags.contains(yavs_ir::FuncFlags::MAY_SUSPEND)
            {
                yavs_interp::retreating_brs(prog, f)
                    .into_iter()
                    .map(|(br, _)| br)
                    .collect()
            } else {
                Default::default()
            };
            let static_wait = if !native && f.kind == FuncKind::Process {
                yavs_interp::static_wait_edge(prog, f, 1, true)
            } else {
                Vec::new()
            };
            let skipped: std::collections::HashSet<u32> = scalar_drives
                .iter()
                .flat_map(|d| {
                    d.head
                        .into_iter()
                        .chain([d.lset])
                        .filter(|&p| p != u32::MAX)
                })
                .collect();
            let mut c = Cur {
                prog,
                consts: Default::default(),
                f: fv,
                env: LLVMGetParam(fv, 0),
                regs,
                mem,
                known: std::cell::RefCell::new(vec![None; f.reg_ty.len()]),
                dirty: std::cell::RefCell::new(Dirty::new(f.reg_ty.len())),
                imul: index_muls(f, &fixed),
                fused: Default::default(),
                layout_fused: Default::default(),
                fixed,
                exits: std::cell::RefCell::new(if native {
                    Vec::new()
                } else {
                    vec![(exit, exit_pc, 0)]
                }),
                exit: (exit, exit_pc),
                cores: Default::default(),
                frame_len: u64::from(f.frame),
                nat,
                defer: None,
                scal: None,
                joins: Default::default(),
                cold: Default::default(),
                objnew: Default::default(),
                batch: None,
                spin: None,
                pres: Default::default(),
            };
            if !backs.is_empty() {
                let local = LLVMBuildAlloca(self.b, self.i64, c"".as_ptr());
                let at = self.tag(self.mem_ptr(&c, MEM_SPIN), AA_MEM);
                let p = self.tag(self.load(self.ptr, at), AA_SPIN);
                let n = self.load(self.i64, p);
                LLVMBuildStore(self.b, n, local);
                c.spin = Some(local);
                if batched.is_empty() {
                    // The first exit block returns at once (built above).
                    let ret = LLVMGetBasicBlockTerminator(exit);
                    LLVMPositionBuilderBefore(self.b, ret);
                    self.spin_out(&c);
                    LLVMPositionBuilderAtEnd(self.b, entry);
                }
            }
            if !batched.is_empty() {
                // At the start of the entry block: every way in passes it.
                let entries = LLVMBuildAlloca(
                    self.b,
                    LLVMArrayType2(self.i64, 2 * yavs_interp::DRIVE_BATCH as u64),
                    c"".as_ptr(),
                );
                let count = LLVMBuildAlloca(self.b, self.i32, c"".as_ptr());
                LLVMBuildStore(self.b, self.c32(0), count);
                c.batch = Some(Batch { entries, count });
                LLVMPositionBuilderAtEnd(self.b, exit);
                self.ret_exit(&c, exit_pc);
                LLVMPositionBuilderAtEnd(self.b, entry);
            }
            // Address parameters whose region a native frame resolves once, at its entry.
            let pas = if native { param_addrs(f) } else { Vec::new() };
            // Registers the interpreter starts the frame with hold the same address everywhere.
            let mut base_known: Vec<Option<Known>> = vec![None; nregs];
            {
                // A native frame stores them at entry (`preloads`).
                let pre = if native {
                    let mut p = vec![false; nregs];
                    for (r, a, _) in preloads(f) {
                        p[r.0 as usize] = a.is_some();
                    }
                    p
                } else {
                    crate::regs::preloaded(f)
                };
                for i in &f.code {
                    if let Inst::AddrOf { dst, space, off } = *i
                        && pre.get(dst.0 as usize) == Some(&true)
                    {
                        base_known[dst.0 as usize] = match space {
                            AddrSpace::Frame => Some(Known::Frame(off.into())),
                            AddrSpace::Const(k) => Some(Known::Const(k, off.into())),
                            AddrSpace::Inst => Some(Known::Inst(off.into())),
                            _ => None,
                        };
                    }
                }
                // The address parameters resolved at the entry (`param_addrs`).
                for (k, pa) in pas.iter().enumerate() {
                    if let Some(x) = base_known.get_mut(pa.reg.0 as usize) {
                        *x = Some(Known::Param(k as u16));
                    }
                }
                *c.known.borrow_mut() = base_known.clone();
            }
            let n = f.code.len();
            // Where the interpreter enters: the start and what follows a call, a wait or a report.
            let mut e0 = vec![false; n + 1];
            e0[0] = true;
            for (i, inst) in f.code.iter().enumerate() {
                if resumes(prog, f, inst) {
                    e0[i + 1] = true;
                }
            }
            let rc = if native {
                None
            } else {
                let roots: Vec<usize> = (0..n).filter(|&i| e0[i]).collect();
                crate::regs::reach(prog, f, &roots)
            };
            let scal = rc.as_ref().and_then(|r| {
                crate::regs::Scalars::new(f, r, &|i| match *i {
                    Inst::Call { func, args, .. } => {
                        nc.contains(&func)
                            && prog
                                .func(func)
                                .is_some_and(|g| call_fits(f, id, g, args, false))
                    }
                    _ => false,
                })
            });
            if !native && let Some(live) = crate::regs::Live::new(prog, f, scal.as_ref()) {
                let live_spans = live.spans();
                c.defer = Some(Defer {
                    live,
                    pre: crate::regs::preloaded(f),
                    // Registers, slot variables, and the base of the frame's slots last.
                    pend: std::cell::RefCell::new(vec![
                        None;
                        nregs
                            + scal
                                .as_ref()
                                .map_or(0, |s| s.slots.len())
                            + 1
                    ]),
                    fresh: Default::default(),
                    pc: Default::default(),
                    keep: live_spans.iter().map(|&n| n > MAX_SPAN).collect(),
                    plan: None,
                    loopy: {
                        let mut l = vec![false; n + 1];
                        for &x in rc.iter().flat_map(|r| r.loops.iter().flatten()) {
                            l[x] = true;
                        }
                        l
                    },
                });
                c.scal = scal;
            }
            // Entry points: 0, block starts, and the instruction after each that may leave
            // frames to the interpreter or suspend.
            let mut leader = vec![false; n + 1];
            leader[0] = true;
            for (_, b) in f.blocks.iter() {
                if let Some(l) = leader.get_mut(b.start as usize) {
                    *l = true;
                }
            }
            // What the generator knows about registers holds until the next block: the
            // interpreter enters after a call only having run the block up to it.
            let block_start = leader.clone();
            // Address spaces that hold at the start of a block, whatever path reached it (#684).
            let space_facts = crate::regs::spaces(prog, f, &block_start)
                .unwrap_or_else(|| vec![Vec::new(); n + 1]);
            for (i, inst) in f.code.iter().enumerate() {
                if resumes(prog, f, inst) {
                    leader[i + 1] = true;
                }
            }
            // The interpreter enters a function after the constants and addresses it preloads
            // into the frame's registers (`Pre::next`): the pcs after such definitions at a
            // leader are entry points too (what is known there is what the registers hold).
            let preload_at = |i: usize| match f.code[i] {
                Inst::Const { .. } | Inst::ConstF { .. } => true,
                Inst::AddrOf { space, .. } => !matches!(space, AddrSpace::Up(_)),
                _ => false,
            };
            let mut enters = leader.clone();
            if !native {
                for i in 1..n {
                    enters[i] |= enters[i - 1] && preload_at(i - 1);
                }
            }
            // A `LayoutOf` that would be stepped and whose layout only one word is read of, by the
            // instructions right after it (no entry between): one call of the interpreter's
            // layout memo for that word (`yavs_rt_layout_word`), as the interpreter's own
            // `FieldOf` and `LayoutWord` ops do. Per pc: the last pc fused, the word, the base of
            // a `Field`, the register that gets the result.
            let fuse_layout = fuse_layout(prog, f, &enters, &c.fixed, !native);
            // With registers carried across blocks (`Plan`) only the places the interpreter
            // can usefully enter at get an entry stub: the start, what follows a call, a wait or
            // a report, and loop headers (a hand-over comes at a jump, and the interpreter runs
            // on to the next one when the compiled code has no entry at the one it lands on).
            let mut plan_regs: Option<Vec<Vec<u32>>> = None;
            let mut plan_single: Vec<bool> = Vec::new();
            let mut is_entry = enters.clone();
            if !native
                && let Some(d) = &c.defer
                && let Some(rc) = &rc
            {
                let mut e = e0.clone();
                {
                    let (reached, head) = (&rc.reached, &rc.head);
                    for (_, b) in f.blocks.iter() {
                        if let Some(h) = e.get_mut(b.start as usize) {
                            *h |= head.get(b.start as usize) == Some(&true);
                        }
                    }
                    for i in 1..n {
                        e[i] |= e[i - 1] && preload_at(i - 1);
                    }
                    let mut at = vec![false; n + 1];
                    for i in 0..n {
                        at[i] = leader[i] || e[i];
                    }
                    let mut total = 0;
                    let deferrable = crate::regs::deferrable(f);
                    let mut loopy = vec![false; n + 1];
                    for &x in rc.loops.iter().flatten() {
                        loopy[x] = true;
                    }
                    let has_frame = f.code.iter().any(|i| {
                        matches!(
                            i,
                            Inst::AddrOf {
                                space: AddrSpace::Frame,
                                ..
                            }
                        )
                    });
                    let mut npred = vec![0u32; n + 1];
                    let mut sole = vec![usize::MAX; n + 1];
                    for j in (0..n).filter(|&j| reached[j]) {
                        for t in crate::regs::succs(prog, f, j, &f.code[j]).unwrap_or_default() {
                            npred[t as usize] += 1;
                            sole[t as usize] = j;
                        }
                    }
                    let single: Vec<bool> = (0..=n)
                        .map(|i| {
                            i < n && at[i] && reached[i] && !e[i] && npred[i] == 1 && sole[i] < i
                        })
                        .collect();
                    plan_single = single.clone();
                    let regs: Vec<Vec<u32>> = (0..n)
                        .map(|i| {
                            // Phis only in loops: elsewhere a block takes its registers from the frame.
                            if !at[i] || !reached[i] || single[i] || !loopy[i] {
                                return Vec::new();
                            }
                            let row = d.live.row(i);
                            let mut v: Vec<u32> = (0..d.pend.borrow().len() as u32 - 1)
                                .filter(|&r| {
                                    row[r as usize / 64] >> (r % 64) & 1 != 0
                                        && (r as usize >= nregs
                                            || (deferrable[r as usize]
                                                && !d.keep[r as usize]
                                                && !d.pre[r as usize]
                                                && c.fixed[r as usize].is_none()))
                                })
                                .collect();
                            // The base of the slots is carried through the loops of a function
                            // that has frame variables.
                            if has_frame {
                                v.push(d.pend.borrow().len() as u32 - 1);
                            }
                            total += v.len();
                            v
                        })
                        .collect();
                    if total <= MAX_PHIS {
                        enters = at;
                        is_entry = e;
                        plan_regs = Some(regs);
                    }
                }
            }
            let mut bbs = vec![std::ptr::null_mut(); n];
            for (i, bb) in bbs.iter_mut().enumerate() {
                if enters[i] {
                    *bb = self.block(&c);
                }
            }
            let mut plan = None;
            if let Some(regs) = plan_regs {
                let phis = regs
                    .iter()
                    .enumerate()
                    .map(|(i, rs)| {
                        if rs.is_empty() {
                            return Vec::new();
                        }
                        LLVMPositionBuilderAtEnd(self.b, bbs[i]);
                        let base = c
                            .defer
                            .as_ref()
                            .map_or(0, |d| d.pend.borrow().len() as u32 - 1);
                        rs.iter()
                            .map(|&r| {
                                let ty = if r == base { self.ptr } else { self.i64 };
                                LLVMBuildPhi(self.b, ty, c"".as_ptr())
                            })
                            .collect()
                    })
                    .collect();
                plan = Some(Plan {
                    regs,
                    phis,
                    inherit: std::cell::RefCell::new(vec![None; plan_single.len()]),
                    single: plan_single,
                });
            }
            if native && bbs.is_empty() {
                return Err("a function without instructions".into());
            }
            if native {
                // The frame as the interpreter's push makes it: zeroed slots, the arguments
                // in the parameter slots.
                let nat = c.nat.as_ref().expect("native");
                let slots = nat.slots;
                let zero = LLVMConstInt(self.i8, 0, 0);
                match &nat.closed {
                    // Only the bytes in use: the rest is never read (`closed_frame`).
                    Some(rs) => rs.iter().for_each(|&(lo, hi)| {
                        let at = self.gep(slots, self.c64(lo as i64));
                        LLVMBuildMemSet(self.b, at, zero, self.c64((hi - lo) as i64), 1);
                    }),
                    None => {
                        LLVMBuildMemSet(self.b, slots, zero, self.c64(i64::from(f.frame)), 8);
                    }
                }
                let mut k = if f.parent.is_some() { 5 } else { 3 };
                for p in &f.params {
                    let at = self.gep(slots, self.c64(i64::from(p.slot)));
                    let v = LLVMGetParam(fv, k);
                    k += 1;
                    match p.width {
                        Width::Sig => {
                            self.store(v, at);
                            self.store(LLVMGetParam(fv, k), self.gep(at, self.c64(8)));
                            k += 1;
                        }
                        Width::B8 => self.store(v, at),
                        w => self.store(
                            LLVMBuildTrunc(self.b, v, self.width_ty(w), c"".as_ptr()),
                            at,
                        ),
                    }
                }
                let entry_len = LLVMBuildLoad2(
                    self.b,
                    self.i64,
                    self.gep(mem, self.c64(MEM_TEMP_LEN as i64)),
                    c"".as_ptr(),
                );
                LLVMBuildStore(self.b, entry_len, nat.temp_base);
                // Callees reach these slots through their frame index. A closed frame has no
                // address they could be asked for: its entry only says it holds nothing.
                let tab = LLVMBuildLoad2(
                    self.b,
                    self.ptr,
                    self.gep(mem, self.c64(MEM_NSLOTS as i64)),
                    c"".as_ptr(),
                );
                let ent = self.gep(
                    tab,
                    LLVMBuildMul(self.b, nat.depth, self.c64(16), c"".as_ptr()),
                );
                if nat.closed.is_some() {
                    self.store(self.c64(0), self.gep(ent, self.c64(8)));
                } else {
                    self.store(slots, ent);
                    self.store(self.c64(i64::from(f.frame)), self.gep(ent, self.c64(8)));
                }
                for (r, addr, v) in preloads(f) {
                    let x = match addr {
                        None => self.c64(v),
                        Some((AddrSpace::Frame, off)) => {
                            let hi = LLVMBuildShl(self.b, nat.fhi, self.c64(32), c"".as_ptr());
                            LLVMBuildOr(self.b, hi, self.c64(i64::from(off)), c"".as_ptr())
                        }
                        Some((sp, off)) => match addr_const(sp, off) {
                            Some(a) => self.c64(a as i64),
                            None => continue,
                        },
                    };
                    self.st(&c, r, x);
                }
                // Where the address parameters point (`yavs_rt_region`), held for the activation.
                let stable = stable_activation(f);
                let fixed = !stable && fixed_activation(f);
                let first = if f.parent.is_some() { 5 } else { 3 };
                for pa in &pas {
                    let a = LLVMGetParam(fv, (first + pa.cell) as u32);
                    let out = self.entry_alloca(&c, 16);
                    // An object the table holds (not a frame or the temporary area: those are
                    // the runtime's to judge) is found inline where the activation keeps it in
                    // place; the rest asks the runtime.
                    let probe = (stable || fixed).then(|| {
                        let hi = LLVMBuildLShr(self.b, a, self.c64(32), c"".as_ptr());
                        let sp = LLVMBuildLShr(self.b, hi, self.c64(29), c"".as_ptr());
                        let (tk, mx) = self.obj_tag(&c, Some(hi));
                        let e = self.obj_entry(&c, a, false, mx);
                        let key = LLVMBuildOr(self.b, hi, tk, c"".as_ptr());
                        let mut hit = self.icmp(IP::LLVMIntEQ, self.load(self.i64, e), key);
                        if pa.write {
                            let w = self.load(self.i64, self.gep(e, self.c64(OBJ_W as i64)));
                            let w = self.icmp(IP::LLVMIntNE, w, self.c64(0));
                            hit = LLVMBuildAnd(self.b, hit, w, c"".as_ptr());
                        }
                        let other = |k: u8| self.icmp(IP::LLVMIntNE, sp, self.c64(i64::from(k)));
                        let ok = LLVMBuildAnd(
                            self.b,
                            hit,
                            LLVMBuildAnd(
                                self.b,
                                other(space::FRAME),
                                other(space::TEMP),
                                c"".as_ptr(),
                            ),
                            c"".as_ptr(),
                        );
                        let (fast, call) = (self.block(&c), self.block(&c));
                        self.cond_br(ok, fast, call, true);
                        LLVMPositionBuilderAtEnd(self.b, fast);
                        let base = self.load(self.ptr, self.gep(e, self.c64(OBJ_PTR as i64)));
                        let len = self.load(self.i64, self.gep(e, self.c64(OBJ_LEN as i64)));
                        LLVMPositionBuilderAtEnd(self.b, call);
                        (fast, base, len)
                    });
                    let flags = self
                        .c32(u32::from(pa.write) | u32::from(stable) << 1 | u32::from(fixed) << 2);
                    let mut args = [c.env, mem, a, nat.depth, flags, out];
                    LLVMBuildCall2(
                        self.b,
                        self.region_ty,
                        self.region,
                        args.as_mut_ptr(),
                        6,
                        c"".as_ptr(),
                    );
                    let mut ptr = self.load(self.ptr, out);
                    let mut len = self.load(self.i64, self.gep(out, self.c64(8)));
                    if let Some((fast, base, blen)) = probe {
                        let (from, join) = (LLVMGetInsertBlock(self.b), self.block(&c));
                        LLVMBuildBr(self.b, join);
                        LLVMPositionBuilderAtEnd(self.b, fast);
                        LLVMBuildBr(self.b, join);
                        LLVMPositionBuilderAtEnd(self.b, join);
                        let (p, l) = (
                            LLVMBuildPhi(self.b, self.ptr, c"".as_ptr()),
                            LLVMBuildPhi(self.b, self.i64, c"".as_ptr()),
                        );
                        LLVMAddIncoming(p, [base, ptr].as_mut_ptr(), [fast, from].as_mut_ptr(), 2);
                        LLVMAddIncoming(l, [blen, len].as_mut_ptr(), [fast, from].as_mut_ptr(), 2);
                        (ptr, len) = (p, l);
                    }
                    c.pres.borrow_mut().push(PRes {
                        ptr,
                        len,
                        write: pa.write,
                    });
                }
                LLVMBuildBr(self.b, bbs[0]);
            } else {
                let pc_in = LLVMGetParam(fv, 3);
                let other = self.block(&c);
                LLVMPositionBuilderAtEnd(self.b, other);
                LLVMBuildRet(self.b, pc_in);
                LLVMPositionBuilderAtEnd(self.b, entry);
                let mut cases = Vec::new();
                for (i, &bb) in bbs.iter().enumerate() {
                    if !is_entry[i] {
                        continue;
                    }
                    // The interpreter's registers are in the frame: a stub loads the ones the
                    // block takes as phis.
                    let Some(pl) = &plan else {
                        cases.push((i as u32, bb));
                        continue;
                    };
                    let stub = self.block(&c);
                    cases.push((i as u32, stub));
                    LLVMPositionBuilderAtEnd(self.b, stub);
                    for (&r, &phi) in pl.regs[i].iter().zip(&pl.phis[i]) {
                        let v = self.frame_load(&c, r);
                        LLVMAddIncoming(phi, &mut { v }, &mut { stub }, 1);
                    }
                    LLVMBuildBr(self.b, bb);
                }
                LLVMPositionBuilderAtEnd(self.b, entry);
                self.entry_dispatch(&c, entry, pc_in, &cases, other);
            }
            if let Some(d) = &mut c.defer {
                d.plan = plan;
            }
            let c = c;
            let block_bb = |b: yavs_ir::BlockId| -> Result<LLVMBasicBlockRef, String> {
                f.blocks
                    .get(b)
                    .and_then(|b| bbs.get(b.start as usize).copied())
                    .filter(|bb| !bb.is_null())
                    .ok_or_else(|| format!("branch to missing block {}", b.0))
            };
            let start_of = |b: yavs_ir::BlockId| -> Result<usize, String> {
                f.blocks
                    .get(b)
                    .map(|b| b.start as usize)
                    .ok_or_else(|| format!("branch to missing block {}", b.0))
            };
            // A native frame's block with a single way in takes what its predecessor knew at the
            // jump: the values defined there dominate it (`inherit`).
            let mut inherit: Vec<Option<Facts>> = vec![None; n + 1];
            let mut ways = vec![0u32; n + 1];
            ways[0] = 2;
            if native {
                for inst in &f.code {
                    let to_all: Vec<yavs_ir::BlockId> = match *inst {
                        Inst::Br { to } => vec![to],
                        Inst::CondBr { t, f: e, .. } => vec![t, e],
                        Inst::Case { table, .. } => prog
                            .cases
                            .get(table)
                            .map(|t| {
                                std::iter::once(t.default)
                                    .chain(t.arms.iter().map(|a| a.2))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        _ => continue,
                    };
                    for b in to_all {
                        if let Some(w) =
                            f.blocks.get(b).and_then(|b| ways.get_mut(b.start as usize))
                        {
                            *w += 1;
                        }
                    }
                }
            }
            // Search loops (`yavs_interp::search_loops`) of a native frame run through
            // `yavs_rt_scan` at their head; the helper's exits jump to blocks it skips the way to.
            let scans: std::collections::HashMap<u32, yavs_interp::SearchLoop> = if native {
                yavs_interp::search_loops(f)
                    .into_iter()
                    .map(|l| (l.head, l))
                    .collect()
            } else {
                Default::default()
            };
            // Element loops (`yavs_interp::elem_loops`) of a native frame ask `yavs_rt_eloop` for
            // all their iterations but the last at the head (#757).
            let eloops: std::collections::HashMap<u32, yavs_interp::ElemLoop> =
                if native && std::env::var_os("YAVS_LLVM_NO_ELOOP").is_none() {
                    yavs_interp::elem_loops(f)
                        .into_iter()
                        .map(|l| (l.head, l))
                        .collect()
                } else {
                    Default::default()
                };
            for l in scans.values() {
                for b in [l.found, l.end] {
                    if let Some(w) = f.blocks.get(b).and_then(|b| ways.get_mut(b.start as usize)) {
                        *w += 1;
                    }
                }
            }
            let mut open = false;
            let mut prev = None;
            let kinds = std::env::var_os("YAVS_LLVM_KINDS").is_some() && !native;
            // Counting the function's instructions is a walk: every `kstride` IR instructions
            // (1 for a small function), the growth shared among those since the last count.
            let kstride = 1 + f.code.len() / 5000;
            let mut kwas = 0;
            let mut kpend: Vec<String> = Vec::new();
            for (i, &inst) in f.code.iter().enumerate() {
                let pc = i as u32;
                if let Some(k) = &kloc {
                    self.kindloc_at(k, &inst, pc);
                }
                if kinds {
                    if i % kstride == 0 {
                        let now = fn_count(fv);
                        let n = kpend.len().max(1);
                        if kpend.is_empty() {
                            kind_add("<head>".into(), 0, now - kwas);
                        }
                        for k in kpend.drain(..) {
                            kind_add_frac(k, now - kwas, n);
                        }
                        kwas = now;
                    }
                    kpend.push(kind_name(&inst));
                }
                if let Some(p) = prev.replace(inst) {
                    track(&mut c.known.borrow_mut(), &c.fixed, f, &p);
                    c.dirty.borrow_mut().defs(&p);
                    // The load of an address parameter (`param_addrs`) defines the register once:
                    // what `track` cleared for the definition holds again.
                    if let Inst::Load { dst, .. } = p
                        && let Some(k) = pas.iter().position(|pa| pa.reg == dst)
                        && let Some(x) = c.known.borrow_mut().get_mut(dst.0 as usize)
                    {
                        *x = Some(Known::Param(k as u16));
                    }
                    // The objects found stay valid up to an instruction that may free or move
                    // one (`keeps_objects`: the rule of the object table too). Only a register
                    // written in this block can hold one.
                    if !keeps_objects(&p) {
                        c.objnew.set(None);
                        let mut k = c.known.borrow_mut();
                        for &r in &c.dirty.borrow().regs {
                            if let Some(Some(Known::Heap(o))) = k.get_mut(r as usize) {
                                *o = None;
                            }
                        }
                    }
                    if let Some((r, o)) = c.objnew.take()
                        && let Some(k) = c.known.borrow_mut().get_mut(r.0 as usize)
                    {
                        *k = Some(Known::Heap(Some(o)));
                        c.dirty.borrow_mut().mark(r.0);
                    }
                    // What the previous instruction defined and did not defer is in the frame.
                    if let Some(d) = &c.defer {
                        let fresh = std::mem::take(&mut *d.fresh.borrow_mut());
                        let mut pend = d.pend.borrow_mut();
                        p.visit(&mut |r, role, _| {
                            if role == yavs_ir::Role::Def {
                                let sig = f.reg_ty.get(r.0 as usize) == Some(&yavs_ir::RegTy::Sig);
                                for k in [r.0, r.0 + 1].into_iter().take(1 + usize::from(sig)) {
                                    if !fresh.contains(&k)
                                        && let Some(x) = pend.get_mut(k as usize)
                                    {
                                        *x = None;
                                    }
                                }
                            }
                        });
                    }
                }
                if let Some(d) = &c.defer {
                    d.pc.set(pc);
                }
                if block_start[i] {
                    let mut k = c.known.borrow_mut();
                    let mut d = c.dirty.borrow_mut();
                    d.reset(&mut k, &base_known);
                    match inherit[i].take() {
                        Some(from) if native && ways[i] == 1 => {
                            for (r, x) in from {
                                k[r as usize] = x;
                                d.mark(r);
                            }
                        }
                        _ => {
                            for &(r, sp) in &space_facts[i] {
                                if let Some(x) = k.get_mut(r as usize)
                                    && x.is_none()
                                {
                                    *x = Some(match sp {
                                        crate::regs::Sp::Inst => Known::InstAny,
                                        crate::regs::Sp::Temp => Known::Temp,
                                        crate::regs::Sp::Heap => Known::Heap(None),
                                        crate::regs::Sp::Likely => Known::LikelyHeap,
                                    });
                                    d.mark(r);
                                }
                            }
                        }
                    }
                }
                if native {
                    let to_all: Vec<yavs_ir::BlockId> = match inst {
                        Inst::Br { to } => vec![to],
                        Inst::CondBr { t, f: e, .. } => vec![t, e],
                        Inst::Case { table, .. } => prog
                            .cases
                            .get(table)
                            .map(|t| {
                                std::iter::once(t.default)
                                    .chain(t.arms.iter().map(|a| a.2))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        _ => Vec::new(),
                    };
                    for b in to_all {
                        if let Some(at) = f.blocks.get(b).map(|b| b.start as usize)
                            && ways.get(at) == Some(&1)
                        {
                            inherit[at] = Some(c.dirty.borrow().diff(&c.known.borrow()));
                        }
                    }
                }
                // What is known about an address register (`Gen::host`).
                let fk = |r: Reg| self.known_addr(&c, r);
                if enters[i] {
                    if open {
                        let snap = self.edge_prep(&c, &[i]);
                        self.edge_to(&c, &snap, i);
                        LLVMBuildBr(self.b, bbs[i]);
                    }
                    LLVMPositionBuilderAtEnd(self.b, bbs[i]);
                    self.begin_block(&c, i);
                    open = true;
                } else if !open {
                    // Dead code after a terminator: give it a block, LLVM drops it.
                    let bb = self.block(&c);
                    LLVMPositionBuilderAtEnd(self.b, bb);
                    self.drop_pend(&c);
                    open = true;
                }
                // The interpreter returns (`TierHost::ret` for a frame compiled code pushed):
                // compiled code leaves the frame to it there.
                if let Inst::Ret { v } = inst {
                    match &c.nat {
                        // A native frame returns its value itself.
                        Some(_) => {
                            let v = v.map_or_else(|| self.c64(0), |r| self.ld(&c, r));
                            LLVMBuildRet(self.b, v);
                        }
                        None => {
                            self.flush(&c, i);
                            self.leave(&c, self.c32(pc))
                        }
                    }
                    open = false;
                    continue;
                }
                if ablate(&inst) {
                    self.step(&c, pc, &inst);
                    continue;
                }
                // Made by the `LayoutOf` before it (`fuse_layout`).
                if let Some((last, o)) = c.layout_fused.get()
                    && i <= last
                {
                    if i == last {
                        c.layout_fused.set(None);
                        c.objnew.set(o);
                    }
                    continue;
                }
                if let Some(l) = eloops.get(&pc)
                    && c.nat.as_ref().is_some_and(|n| n.closed.is_none())
                {
                    // The iterations but the last run planned in the code itself (the program of
                    // steps as straight-line code), or in the helper for a shape it does not
                    // handle; what either leaves (a fault, an overlap, memory it does not reach,
                    // nothing to gain) is the generic code below.
                    if std::env::var_os("YAVS_LLVM_KINDLOC").is_some() {
                        eprintln!("eloop `{}` pc{}: {:?}", f.name, l.head, l);
                    }
                    let generic = self.block(&c);
                    match crate::eloop::invariant_steps(&l.prog, l.inputs.len()) {
                        Some(inv) if std::env::var_os("YAVS_LLVM_ELOOP_HELPER").is_none() => {
                            self.eloop_plan(&c, l, &inv, generic);
                        }
                        _ => self.eloop_call(&c, l, generic),
                    }
                    LLVMPositionBuilderAtEnd(self.b, generic);
                }
                if let Some(l) = scans.get(&pc) {
                    // The iterations that need nothing special run in the helper; what it leaves
                    // (a fault, a long loop, memory it does not reach) is the generic code below.
                    let out = self.entry_alloca(&c, 32);
                    let mut args = [
                        c.env,
                        c.mem,
                        self.c64(scan_ctl(l) as i64),
                        self.ld(&c, l.dope),
                        self.ld(&c, l.base),
                        self.ld(&c, l.idx),
                        self.ld(&c, l.x),
                        self.ld(&c, l.e),
                        self.c64(l.shape.delta),
                        out,
                        c.nat.as_ref().map_or(self.c64(0), |n| n.depth),
                    ];
                    let iters = LLVMBuildCall2(
                        self.b,
                        self.scan_ty,
                        self.scan,
                        args.as_mut_ptr(),
                        11,
                        c"".as_ptr(),
                    );
                    let (done, generic) = (self.block(&c), self.block(&c));
                    LLVMBuildCondBr(
                        self.b,
                        self.icmp(IP::LLVMIntNE, iters, self.c32(0)),
                        done,
                        generic,
                    );
                    LLVMPositionBuilderAtEnd(self.b, done);
                    let word = |k: i64| self.load(self.i64, self.gep(out, self.c64(k * 8)));
                    self.st(&c, l.idx, word(0));
                    self.st(&c, l.ch, word(1));
                    let sw = LLVMBuildSwitch(self.b, word(2), generic, 2);
                    LLVMAddCase(sw, self.c64(1), block_bb(l.found)?);
                    LLVMAddCase(sw, self.c64(2), block_bb(l.end)?);
                    LLVMPositionBuilderAtEnd(self.b, generic);
                }
                if c.batch.is_some() && !batched.contains(&pc) && !batch_pure(prog, f, &inst) {
                    self.batch_flush(&c);
                }
                // A frame address used inside a branch of an instruction's own code (a fast path)
                // takes the slots pointer of the stretch: fetch it here, where it dominates.
                if c.nat.is_none()
                    && !skipped.contains(&pc)
                    && inline_regs(f, &inst)
                        .into_iter()
                        .any(|r| matches!(self.known_addr(&c, r), Some(Known::Frame(_))))
                {
                    self.own_slots_cached(&c);
                }
                match inst {
                    _ if skipped.contains(&pc) => {}
                    Inst::Drive { .. }
                        if let Some(d) = scalar_drives.iter().find(|d| d.drive == pc) =>
                    {
                        self.drive_bit(&c, pc, d, &inst)
                    }
                    Inst::Const { dst, v } => self.std(&c, dst, self.c64(v)),
                    Inst::ConstF { dst, v } => self.std(&c, dst, self.c64(v.to_bits() as i64)),
                    Inst::Move { dst, src } => self.std(&c, dst, self.ld(&c, src)),
                    // Coverage counters: one atomic add on the instance's counters (the view's
                    // pointer, so the module holds no address); a missing counter is the
                    // interpreter's error.
                    Inst::Cover { item } => {
                        let len = self.mem_ld(&c, MEM_COV_LEN, self.i64);
                        let bad = self.icmp(IP::LLVMIntULE, len, self.c64(i64::from(item.0)));
                        self.exit_if(&c, bad, pc);
                        let base = self.mem_ld(&c, MEM_COV, self.ptr);
                        let p = self.gep(base, self.c64(8 * i64::from(item.0)));
                        LLVMBuildAtomicRMW(
                            self.b,
                            llvm_sys::LLVMAtomicRMWBinOp::LLVMAtomicRMWBinOpAdd,
                            p,
                            self.c64(1),
                            llvm_sys::LLVMAtomicOrdering::LLVMAtomicOrderingMonotonic,
                            0,
                        );
                    }
                    Inst::CoverIn {
                        dst,
                        acc,
                        src,
                        shift,
                        ulogic,
                    } => {
                        // 1 for false, 2 for true, 3 for another value (`Inst::CoverIn`).
                        let p = self.ld(&c, src);
                        let (f0, t0) = if ulogic {
                            let q = LLVMBuildOr(self.b, p, self.c64(4), c"".as_ptr());
                            (
                                self.icmp(IP::LLVMIntEQ, q, self.c64(6)),
                                self.icmp(IP::LLVMIntEQ, q, self.c64(7)),
                            )
                        } else {
                            (
                                self.icmp(IP::LLVMIntEQ, p, self.c64(0)),
                                self.icmp(IP::LLVMIntEQ, p, self.c64(1)),
                            )
                        };
                        let hi =
                            LLVMBuildSelect(self.b, t0, self.c64(2), self.c64(3), c"".as_ptr());
                        let code = LLVMBuildSelect(self.b, f0, self.c64(1), hi, c"".as_ptr());
                        let code =
                            LLVMBuildShl(self.b, code, self.c64(i64::from(shift)), c"".as_ptr());
                        let v = if shift == 0 {
                            code
                        } else {
                            LLVMBuildOr(self.b, self.ld(&c, acc), code, c"".as_ptr())
                        };
                        // The state is read by the host's `CoverCond`: in the frame at once.
                        self.st(&c, dst, v);
                    }
                    Inst::Select { dst, c: cond, a, b } => {
                        let t = self.icmp(IP::LLVMIntNE, self.ld(&c, cond), self.c64(0));
                        let v = LLVMBuildSelect(
                            self.b,
                            t,
                            self.ld(&c, a),
                            self.ld(&c, b),
                            c"".as_ptr(),
                        );
                        self.std(&c, dst, v);
                    }
                    Inst::Bin {
                        op: IntOp::Mul,
                        dst,
                        ..
                    } if let Some((r, m)) = c.imul[pc as usize] => {
                        // The operand is at most `max`, or the interpreter multiplies and
                        // indexes. Then the product is under 2^32, and when the `Index` after
                        // it has a constant offset in an object the held offset plus the
                        // product is too: that `Index` has nothing left to test.
                        let Some(&Inst::Index { base, stride, .. }) = f.code.get(pc as usize + 1)
                        else {
                            return Err("a fused multiplication without its index".into());
                        };
                        let off = match self.known_addr(&c, base) {
                            Some(Known::Heap(Some(o))) if LLVMIsConstant(o.off) != 0 => {
                                Some(LLVMConstIntGetZExtValue(o.off))
                            }
                            _ => None,
                        };
                        let (max, kind) = match off {
                            Some(o) if o <= 0xffff_ffff => {
                                ((0xffff_ffff - o) / (m * u64::from(stride)), 2)
                            }
                            _ => (0xffff_ffff / m, 1),
                        };
                        let x = self.ld(&c, r);
                        self.exit_if(&c, self.icmp(IP::LLVMIntUGT, x, self.c64(max as i64)), pc);
                        let v = LLVMBuildMul(self.b, x, self.c64(m as i64), c"".as_ptr());
                        self.std(&c, dst, v);
                        c.fused.set((pc + 1, kind));
                    }
                    Inst::Bin { op, dst, a, b } => {
                        let (x, y) = (self.ld(&c, a), self.ld(&c, b));
                        match self.int_op(&c, op, x, y, pc) {
                            Some(v) => self.std(&c, dst, v),
                            None => self.slow_inst(&c, pc, &inst, f)?,
                        }
                    }
                    Inst::BinRange {
                        op,
                        dst,
                        a,
                        b,
                        lo,
                        hi,
                        ..
                    } => {
                        let (x, y) = (self.ld(&c, a), self.ld(&c, b));
                        match self.int_op(&c, op, x, y, pc) {
                            Some(v) => {
                                self.range_exit(&c, v, self.ld(&c, lo), self.ld(&c, hi), pc);
                                self.std(&c, dst, v);
                            }
                            None => self.slow_inst(&c, pc, &inst, f)?,
                        }
                    }
                    Inst::Un { op, dst, a } => {
                        let x = self.ld(&c, a);
                        let min = self.icmp(IP::LLVMIntEQ, x, self.c64(i64::MIN));
                        self.exit_if(&c, min, pc);
                        let neg = LLVMBuildNeg(self.b, x, c"".as_ptr());
                        let v = match op {
                            IntUnOp::Neg => neg,
                            IntUnOp::Abs => {
                                let lt = self.icmp(IP::LLVMIntSLT, x, self.c64(0));
                                LLVMBuildSelect(self.b, lt, neg, x, c"".as_ptr())
                            }
                        };
                        self.std(&c, dst, v);
                    }
                    Inst::FBin { op, dst, a, b } => {
                        let (x, y) = (self.ldf(&c, a), self.ldf(&c, b));
                        let v = match op {
                            FOp::Add => LLVMBuildFAdd(self.b, x, y, c"".as_ptr()),
                            FOp::Sub => LLVMBuildFSub(self.b, x, y, c"".as_ptr()),
                            FOp::Mul => LLVMBuildFMul(self.b, x, y, c"".as_ptr()),
                            FOp::Div => {
                                let z = LLVMBuildFCmp(
                                    self.b,
                                    RP::LLVMRealOEQ,
                                    y,
                                    LLVMConstReal(self.f64, 0.0),
                                    c"".as_ptr(),
                                );
                                self.exit_if(&c, z, pc);
                                LLVMBuildFDiv(self.b, x, y, c"".as_ptr())
                            }
                            FOp::PowI | FOp::Pow => {
                                self.slow_inst(&c, pc, &inst, f)?;
                                continue;
                            }
                        };
                        // Not finite: |v| is not below infinity (NaN compares false).
                        let fin = LLVMBuildFCmp(
                            self.b,
                            RP::LLVMRealUGE,
                            self.fabs(v),
                            LLVMConstReal(self.f64, f64::INFINITY),
                            c"".as_ptr(),
                        );
                        self.exit_if(&c, fin, pc);
                        self.stfd(&c, dst, v);
                    }
                    Inst::FUn { op, dst, a } => {
                        let x = self.ldf(&c, a);
                        let v = match op {
                            FUnOp::Neg => LLVMBuildFNeg(self.b, x, c"".as_ptr()),
                            FUnOp::Abs => self.fabs(x),
                        };
                        self.stfd(&c, dst, v);
                    }
                    Inst::IToF { dst, src } => {
                        let v = LLVMBuildSIToFP(self.b, self.ld(&c, src), self.f64, c"".as_ptr());
                        self.stfd(&c, dst, v);
                    }
                    Inst::Cmp { op, dst, a, b } => {
                        let p = match op {
                            CmpOp::Eq => IP::LLVMIntEQ,
                            CmpOp::Ne => IP::LLVMIntNE,
                            CmpOp::Lt => IP::LLVMIntSLT,
                            CmpOp::Le => IP::LLVMIntSLE,
                            CmpOp::Gt => IP::LLVMIntSGT,
                            CmpOp::Ge => IP::LLVMIntSGE,
                        };
                        let v = self.icmp(p, self.ld(&c, a), self.ld(&c, b));
                        self.std(&c, dst, self.zext(v));
                    }
                    Inst::FCmp { op, dst, a, b } => {
                        // Ordered: every comparison with a NaN is false, as `partial_cmp`.
                        let p = match op {
                            CmpOp::Eq => RP::LLVMRealOEQ,
                            CmpOp::Ne => RP::LLVMRealONE,
                            CmpOp::Lt => RP::LLVMRealOLT,
                            CmpOp::Le => RP::LLVMRealOLE,
                            CmpOp::Gt => RP::LLVMRealOGT,
                            CmpOp::Ge => RP::LLVMRealOGE,
                        };
                        let (x, y) = (self.ldf(&c, a), self.ldf(&c, b));
                        let v = LLVMBuildFCmp(self.b, p, x, y, c"".as_ptr());
                        self.std(&c, dst, self.zext(v));
                    }
                    Inst::Bool { op, dst, a, b } => {
                        let x = self.icmp(IP::LLVMIntNE, self.ld(&c, a), self.c64(0));
                        let v = if op == BoolOp::Not {
                            LLVMBuildNot(self.b, x, c"".as_ptr())
                        } else {
                            let y = self.icmp(IP::LLVMIntNE, self.ld(&c, b), self.c64(0));
                            match op {
                                BoolOp::And => LLVMBuildAnd(self.b, x, y, c"".as_ptr()),
                                BoolOp::Or => self.or(x, y),
                                _ => LLVMBuildXor(self.b, x, y, c"".as_ptr()),
                            }
                        };
                        self.std(&c, dst, self.zext(v));
                    }
                    Inst::Check {
                        kind, v, lo, hi, ..
                    } => {
                        let x = self.ld(&c, v);
                        match kind {
                            CheckKind::Range | CheckKind::Index => {
                                self.range_exit(&c, x, self.ld(&c, lo), self.ld(&c, hi), pc)
                            }
                            CheckKind::Length => {
                                let ne = self.icmp(IP::LLVMIntNE, x, self.ld(&c, lo));
                                self.exit_if(&c, ne, pc);
                            }
                            CheckKind::Null => {
                                let z = self.icmp(IP::LLVMIntEQ, x, self.c64(0));
                                self.exit_if(&c, z, pc);
                            }
                        }
                    }
                    Inst::FCheck { v, lo, hi, .. } => {
                        let (x, l, h) = (self.ldf(&c, v), self.ldf(&c, lo), self.ldf(&c, hi));
                        // !(l <= x && x <= h), NaN included.
                        let a = LLVMBuildFCmp(self.b, RP::LLVMRealUGT, l, x, c"".as_ptr());
                        let b = LLVMBuildFCmp(self.b, RP::LLVMRealUGT, x, h, c"".as_ptr());
                        self.exit_if(&c, self.or(a, b), pc);
                    }
                    Inst::Index {
                        dst,
                        base,
                        pos,
                        stride,
                    } => {
                        // Address offset in the low 32 bits; anything over 4 GiB is the
                        // interpreter's error. With a position under 2^32 and a stride under
                        // 2^32 the product and the sum cannot wrap, so one test of both
                        // against 32 bits does for the overflow checks (a stride of 0 takes no
                        // position into account).
                        let known_obj = match fk(base) {
                            Some(Known::Heap(Some(o))) => Some(o),
                            _ => None,
                        };
                        let a = self.ld(&c, base);
                        let p = self.ld(&c, pos);
                        let off =
                            LLVMBuildMul(self.b, p, self.c64(i64::from(stride)), c"".as_ptr());
                        let lo = match known_obj {
                            Some(o) => o.off,
                            None => LLVMBuildAnd(self.b, a, self.c64(0xffff_ffff), c"".as_ptr()),
                        };
                        let sum = LLVMBuildAdd(self.b, lo, off, c"".as_ptr());
                        // What the multiplication before tested (`Cur::fused`).
                        let fused = match c.fused.get() {
                            (at, k) if at == pc && bbs[pc as usize].is_null() => k,
                            _ => 0,
                        };
                        let wide = if stride == 0 || fused == 1 {
                            sum
                        } else {
                            LLVMBuildOr(self.b, p, sum, c"".as_ptr())
                        };
                        if fused != 2 {
                            self.exit_if(
                                &c,
                                self.icmp(IP::LLVMIntUGT, wide, self.c64(0xffff_ffff)),
                                pc,
                            );
                        }
                        let hi = match known_obj {
                            Some(o) => o.hi,
                            None => LLVMBuildAnd(self.b, a, self.c64(!0xffff_ffff), c"".as_ptr()),
                        };
                        self.st(&c, dst, LLVMBuildOr(self.b, hi, sum, c"".as_ptr()));
                        if let Some(o) = known_obj {
                            c.objnew.set(Some((dst, ObjRef { off: sum, ..o })));
                        }
                    }
                    Inst::AddrOf { dst, space, off } => match addr_const(space, off) {
                        Some(a) if space == AddrSpace::Frame => {
                            let hi = self.own_hi(&c);
                            let hi = LLVMBuildShl(self.b, hi, self.c64(32), c"".as_ptr());
                            self.std(
                                &c,
                                dst,
                                LLVMBuildOr(self.b, hi, self.c64(a as i64), c"".as_ptr()),
                            );
                        }
                        Some(a) => self.std(&c, dst, self.c64(a as i64)),
                        // A frame one or two static links up, from the view.
                        None => match space {
                            AddrSpace::Up(d @ 1..=2) => {
                                let (slow, join) = self.slow_path(&c, pc, &inst);
                                let hi = match &c.nat {
                                    Some(n) => n.up[usize::from(d - 1)],
                                    None => {
                                        self.mem_ld(&c, MEM_UP + 8 * u64::from(d - 1), self.i64)
                                    }
                                };
                                let ok = self.icmp(IP::LLVMIntNE, hi, self.c64(-1));
                                self.guard(&c, ok, slow);
                                let hi = LLVMBuildShl(self.b, hi, self.c64(32), c"".as_ptr());
                                let a = LLVMBuildOr(self.b, hi, self.c64(off.into()), c"".as_ptr());
                                self.st(&c, dst, a);
                                self.join(&c, join);
                            }
                            _ => self.step(&c, pc, &inst),
                        },
                    },
                    // A fat pair is recorded only when a word points into a heap
                    // (`Machine::fat_root`); the helper says when none does.
                    Inst::TempRoot { addr, ty: None, .. } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        // The roots are recorded by the host's own function, in a native frame
                        // and in frame-mode code (no frame view, no registers to store).
                        let miss = {
                            let here = LLVMGetInsertBlock(self.b);
                            let rec = self.block(&c);
                            LLVMPositionBuilderAtEnd(self.b, rec);
                            let null = LLVMConstPointerNull(self.ptr);
                            let ok = self.troot_call(&c, self.ld(&c, addr), u32::MAX, null, 0);
                            LLVMBuildCondBr(
                                self.b,
                                self.icmp(IP::LLVMIntNE, ok, self.c32(0)),
                                join,
                                slow,
                            );
                            LLVMPositionBuilderAtEnd(self.b, here);
                            rec
                        };
                        // Both words read in place (a pair in a frame, the temporary area or an
                        // object found before); `yavs_rt_fast` where they cannot be.
                        let a = self.ld(&c, addr);
                        let p = self.host(
                            &c,
                            a,
                            self.known_addr(&c, addr),
                            self.c64(0),
                            self.c64(16),
                            false,
                            miss,
                        );
                        let in_heap = |w: LLVMValueRef| {
                            let sp = LLVMBuildLShr(self.b, w, self.c64(61), c"".as_ptr());
                            self.or(
                                self.icmp(IP::LLVMIntEQ, sp, self.c64(i64::from(space::HEAP))),
                                self.icmp(
                                    IP::LLVMIntEQ,
                                    sp,
                                    self.c64(i64::from(space::SHARED_HEAP)),
                                ),
                            )
                        };
                        let (w0, w1) = (
                            self.load(self.i64, p),
                            self.load(self.i64, self.gep(p, self.c64(8))),
                        );
                        let heap = self.or(in_heap(w0), in_heap(w1));
                        self.guard(&c, LLVMBuildNot(self.b, heap, c"".as_ptr()), miss);
                        self.join(&c, join);
                    }
                    Inst::Stmt => {
                        // With recorded temporaries the host's own function drops them, in a native
                        // frame and in frame-mode code.
                        let endstmt = |g: &Self| {
                            let mut a = [c.env, c.mem, g.temp_base(&c)];
                            LLVMBuildCall2(
                                g.b,
                                g.endstmt_ty,
                                g.endstmt,
                                a.as_mut_ptr(),
                                3,
                                c"".as_ptr(),
                            );
                        };
                        let (slow, join) = if c.nat.is_some() {
                            let (join, here) = (self.block(&c), LLVMGetInsertBlock(self.b));
                            let rec = self.block(&c);
                            LLVMPositionBuilderAtEnd(self.b, rec);
                            endstmt(self);
                            LLVMBuildBr(self.b, join);
                            LLVMPositionBuilderAtEnd(self.b, here);
                            (rec, join)
                        } else {
                            self.slow_path_with(&c, pc, &inst, false, |g, _| endstmt(g))
                        };
                        let roots = self.mem_ld(&c, MEM_TEMP_ROOTS, self.i64);
                        self.guard(&c, self.icmp(IP::LLVMIntEQ, roots, self.c64(0)), slow);
                        let len = self.mem_ld(&c, MEM_TEMP_LEN, self.i64);
                        let base = self.temp_base(&c);
                        let lt = self.icmp(IP::LLVMIntULT, base, len);
                        let v = LLVMBuildSelect(self.b, lt, base, len, c"".as_ptr());
                        self.mem_st(&c, MEM_TEMP_LEN, v);
                        self.join(&c, join);
                        // The wait that names the same signals every time: the kernel keeps
                        // the last one, and the process suspends without naming them (#530).
                        // A `wait until <edge>` short-cuts only where the edge trigger
                        // resumed the activation (`WAIT_SAME_EDGE`, #772).
                        if let Some(&(_, at, edge)) = static_wait.iter().find(|w| w.0 == i) {
                            let mut args = [
                                c.env,
                                c.regs,
                                c.mem,
                                self.c32(at as u32),
                                self.c64(if edge { WAIT_SAME_EDGE as i64 } else { 0 }),
                                self.c64(0),
                            ];
                            self.call_go(&c, self.wait_ty, self.wait_same, &mut args, true);
                        }
                    }
                    Inst::Alloc { dst, size } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let (a, _) = self.temp_alloc(&c, self.ld(&c, size), slow);
                        self.st(&c, dst, a);
                        self.join(&c, join);
                    }
                    // The field's offset from the interpreter's layout memo, the field's address as
                    // `Field` makes it; the interpreter runs both where the memo declines.
                    Inst::LayoutOf { ty, desc, .. } if fuse_layout[i].is_some() => {
                        let fu = fuse_layout[i].expect("checked");
                        let dsz = yavs_ir::Ty::desc_size(&prog.types, ty).expect("checked") as i64;
                        let slow = self.block(&c);
                        c.cold.borrow_mut().push(slow);
                        let here = LLVMGetInsertBlock(self.b);
                        LLVMPositionBuilderAtEnd(self.b, slow);
                        self.flush_exit(&c, pc);
                        LLVMPositionBuilderAtEnd(self.b, here);
                        let pd = self.host(
                            &c,
                            self.ld(&c, desc),
                            fk(desc),
                            self.c64(0),
                            self.c64(dsz),
                            false,
                            slow,
                        );
                        let mut args = [c.env, self.c32(ty.0), pd, self.c64(dsz), self.c32(fu.k)];
                        let w = LLVMBuildCall2(
                            self.b,
                            self.layout_word_ty,
                            self.layout_word,
                            args.as_mut_ptr(),
                            5,
                            c"".as_ptr(),
                        );
                        self.guard(&c, self.icmp(IP::LLVMIntNE, w, self.c64(-1)), slow);
                        let mut obj = None;
                        let v = match fu.base {
                            None => w,
                            // The field's address, as `Field` makes it.
                            Some(base) => {
                                let a = self.ld(&c, base);
                                let known_obj = match fk(base) {
                                    Some(Known::Heap(Some(o))) => Some(o),
                                    _ => None,
                                };
                                let lo = match known_obj {
                                    Some(o) => o.off,
                                    None => {
                                        LLVMBuildAnd(self.b, a, self.c64(0xffff_ffff), c"".as_ptr())
                                    }
                                };
                                let (sum, o) = self.ovf(self.uadd, lo, w);
                                let big = self.icmp(IP::LLVMIntUGT, sum, self.c64(0xffff_ffff));
                                self.guard(
                                    &c,
                                    LLVMBuildNot(self.b, self.or(o, big), c"".as_ptr()),
                                    slow,
                                );
                                let hi = match known_obj {
                                    Some(o) => o.hi,
                                    None => LLVMBuildAnd(
                                        self.b,
                                        a,
                                        self.c64(!0xffff_ffff),
                                        c"".as_ptr(),
                                    ),
                                };
                                obj = known_obj
                                    .filter(|_| c.nat.is_some())
                                    .map(|o| (fu.out, ObjRef { off: sum, ..o }));
                                LLVMBuildOr(self.b, hi, sum, c"".as_ptr())
                            }
                        };
                        self.st(&c, fu.out, v);
                        c.layout_fused.set(Some((fu.last, obj)));
                    }
                    // The layout the process's frame keeps for this type, when the descriptor is
                    // the one it was made for (`layout_cache`); else the interpreter makes it and
                    // the frame keeps that.
                    Inst::LayoutOf {
                        dst,
                        ty,
                        desc,
                        cache: Some(at),
                    } if c.nat.is_none() && layout_shape(prog, ty).is_some() => {
                        let (dsz, nw) = layout_shape(prog, ty).expect("checked");
                        let (slow, join) = self.slow_path_then(&c, pc, &inst, |g, join| {
                            g.layout_keep(&c, dst, desc, fk(desc), at, dsz, nw, join)
                        });
                        let slots = self.tag(self.own_slots_cached(&c), AA_SLOTS);
                        let base = self.gep(slots, self.c64(i64::from(at)));
                        let kept = self.load(self.i64, base);
                        self.guard(&c, self.icmp(IP::LLVMIntNE, kept, self.c64(0)), slow);
                        let pd = self.host(
                            &c,
                            self.ld(&c, desc),
                            fk(desc),
                            self.c64(0),
                            self.c64(i64::from(dsz)),
                            false,
                            slow,
                        );
                        if dsz <= 32 {
                            let mut diff = self.c64(0);
                            for k in 0..i64::from(dsz / 8) {
                                let x = LLVMBuildXor(
                                    self.b,
                                    self.load(self.i64, self.gep(pd, self.c64(8 * k))),
                                    self.load(self.i64, self.gep(base, self.c64(8 + 8 * k))),
                                    c"".as_ptr(),
                                );
                                diff = self.or(diff, x);
                            }
                            self.guard(&c, self.icmp(IP::LLVMIntEQ, diff, self.c64(0)), slow);
                        } else {
                            // A long descriptor (a record of arrays): one call, not a word each.
                            let mut args =
                                [pd, self.gep(base, self.c64(8)), self.c64(i64::from(dsz))];
                            let eq = LLVMBuildCall2(
                                self.b,
                                self.memeq_ty,
                                self.memeq,
                                args.as_mut_ptr(),
                                3,
                                c"".as_ptr(),
                            );
                            self.guard(&c, self.icmp(IP::LLVMIntNE, eq, self.c64(0)), slow);
                        }
                        let hi = LLVMBuildShl(self.b, self.own_hi(&c), self.c64(32), c"".as_ptr());
                        let words = self.c64(i64::from(at) + 8 + i64::from(dsz));
                        self.st(&c, dst, LLVMBuildOr(self.b, hi, words, c"".as_ptr()));
                        self.join(&c, join);
                    }
                    Inst::LayoutOf { dst, ty, desc, .. } => match static_layout(prog, ty) {
                        Some(StaticLayout::Array { dims, elem, logic })
                            if c.nat.is_none() && dims > 1 =>
                        {
                            // `yavs_rt_layout` does what `Ty::layout` would; 0 is the interpreter's.
                            let (slow, join) = self.slow_path(&c, pc, &inst);
                            let stride = elem.size.next_multiple_of(elem.align.max(1));
                            let (rp, d) = self.reg_out(&c, dst);
                            let mut args = [
                                c.env,
                                rp,
                                c.mem,
                                self.ld(&c, desc),
                                self.c32(d),
                                self.c32(u32::from(dims)),
                                self.c64(stride as i64),
                                self.c64(elem.align as i64),
                                self.c64(elem.count as i64),
                                self.c32(u32::from(logic)),
                            ];
                            let ok = LLVMBuildCall2(
                                self.b,
                                self.layout_ty,
                                self.layout,
                                args.as_mut_ptr(),
                                10,
                                c"".as_ptr(),
                            );
                            self.guard(&c, self.icmp(IP::LLVMIntNE, ok, self.c32(0)), slow);
                            self.reg_in(&c, dst);
                            self.join(&c, join);
                        }
                        Some(sl) => {
                            let (slow, join) = self.slow_path_x(&c, pc, &inst, f);
                            let words = self.layout_words(&c, &sl, desc, fk(desc), slow);
                            let n = self.c64(words.len() as i64 * 8);
                            let (a, p) = self.temp_alloc(&c, n, slow);
                            for (k, w) in words.into_iter().enumerate() {
                                self.store(w, self.gep(p, self.c64(k as i64 * 8)));
                            }
                            self.st(&c, dst, a);
                            self.join(&c, join);
                        }
                        None if c.nat.is_some() => self.native_step(&c, pc, &inst, f)?,
                        None => self.step(&c, pc, &inst),
                    },
                    // A variable kept as a value (`Scalars`): no memory access until a write-back.
                    Inst::Load { dst, w, .. }
                        if c.scal.as_ref().is_some_and(|s| s.at[i].is_some()) =>
                    {
                        let _ = w;
                        let k = c.scal.as_ref().and_then(|s| s.at[i]).expect("slot") as usize;
                        let v = self.vslot_value(&c, k);
                        self.std(&c, dst, v);
                    }
                    Inst::Store { src, w, .. }
                        if c.scal.as_ref().is_some_and(|s| s.at[i].is_some()) =>
                    {
                        let k = c.scal.as_ref().and_then(|s| s.at[i]).expect("slot") as usize;
                        let mut v = self.ld(&c, src);
                        if w != Width::B8 {
                            // As the frame would hold it, read back.
                            let t = LLVMBuildTrunc(self.b, v, self.width_ty(w), c"".as_ptr());
                            v = if w == Width::B4 {
                                LLVMBuildSExt(self.b, t, self.i64, c"".as_ptr())
                            } else {
                                LLVMBuildZExt(self.b, t, self.i64, c"".as_ptr())
                            };
                        }
                        let nregs = c.known.borrow().len();
                        if let Some(d) = &c.defer {
                            d.pend.borrow_mut()[nregs + k] = Some(v);
                        }
                    }
                    // A variable of the frame: no check, no slow path, so the result can be
                    // deferred like any other (`Gen::host` takes the same case).
                    Inst::Load { dst, addr, w }
                        if w != Width::Sig
                            && matches!(fk(addr), Some(Known::Frame(o))
                                if o.checked_add(w.bytes() as u64).is_some_and(|e| e <= c.frame_len)) =>
                    {
                        let n = self.c64(i64::from(w.bytes()));
                        let p = self.host(
                            &c,
                            self.ld(&c, addr),
                            fk(addr),
                            self.c64(0),
                            n,
                            false,
                            std::ptr::null_mut(),
                        );
                        let v = self.load(self.width_ty(w), p);
                        let v = match w {
                            Width::B8 => v,
                            Width::B4 => LLVMBuildSExt(self.b, v, self.i64, c"".as_ptr()),
                            _ => LLVMBuildZExt(self.b, v, self.i64, c"".as_ptr()),
                        };
                        self.std(&c, dst, v);
                    }
                    Inst::Load { dst, addr, w } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n = self.c64(i64::from(w.bytes()));
                        let p =
                            self.host(&c, self.ld(&c, addr), fk(addr), self.c64(0), n, false, slow);
                        match w {
                            Width::Sig => {
                                self.st(&c, dst, self.load(self.i64, p));
                                let p8 = self.gep(p, self.c64(8));
                                self.st(&c, Reg(dst.0 + 1), self.load(self.i64, p8));
                            }
                            _ => {
                                let ty = self.width_ty(w);
                                let v = self.load(ty, p);
                                let v = match w {
                                    Width::B8 => v,
                                    Width::B4 => LLVMBuildSExt(self.b, v, self.i64, c"".as_ptr()),
                                    _ => LLVMBuildZExt(self.b, v, self.i64, c"".as_ptr()),
                                };
                                self.st(&c, dst, v);
                            }
                        }
                        self.join(&c, join);
                    }
                    Inst::Store { addr, src, w } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n = self.c64(i64::from(w.bytes()));
                        let p =
                            self.host(&c, self.ld(&c, addr), fk(addr), self.c64(0), n, true, slow);
                        match w {
                            Width::Sig => {
                                self.store(self.ld(&c, src), p);
                                let p8 = self.gep(p, self.c64(8));
                                self.store(self.ld(&c, Reg(src.0 + 1)), p8);
                            }
                            _ => {
                                let v = self.ld(&c, src);
                                let v = if w == Width::B8 {
                                    v
                                } else {
                                    LLVMBuildTrunc(self.b, v, self.width_ty(w), c"".as_ptr())
                                };
                                self.store(v, p);
                            }
                        }
                        self.join(&c, join);
                    }
                    Inst::Copy { dst, src, bytes } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n = self.ld(&c, bytes);
                        let ps =
                            self.host(&c, self.ld(&c, src), fk(src), self.c64(0), n, false, slow);
                        let pd =
                            self.host(&c, self.ld(&c, dst), fk(dst), self.c64(0), n, true, slow);
                        LLVMBuildMemMove(self.b, pd, 1, ps, 1, n);
                        self.join(&c, join);
                    }
                    Inst::Zero { addr, bytes } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n = self.c64(i64::from(bytes));
                        let p =
                            self.host(&c, self.ld(&c, addr), fk(addr), self.c64(0), n, true, slow);
                        LLVMBuildMemSet(self.b, p, LLVMConstInt(self.i8, 0, 0), n, 1);
                        self.join(&c, join);
                    }
                    Inst::MakeDope { at, parts } => {
                        let rs = f.regs(parts).ok_or("operand list out of range")?;
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n = self.c64(rs.len() as i64 * 8);
                        let p = self.host(&c, self.ld(&c, at), fk(at), self.c64(0), n, true, slow);
                        for (k, &r) in rs.iter().enumerate() {
                            let pk = self.gep(p, self.c64(k as i64 * 8));
                            self.store(self.ld(&c, r), pk);
                        }
                        self.join(&c, join);
                    }
                    Inst::DopeLen { dst, dope, dim } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n24 = self.c64(24);
                        let p = self.host(
                            &c,
                            self.ld(&c, dope),
                            fk(dope),
                            self.c64(i64::from(dim) * 24),
                            n24,
                            false,
                            slow,
                        );
                        let (l, r, dir) = self.dope_words(p);
                        let (n, fits) = self.len_of(l, r, dir);
                        // A length over i64::MAX is the interpreter's error.
                        let small = self.icmp(IP::LLVMIntSGE, n, self.c64(0));
                        self.guard(&c, LLVMBuildAnd(self.b, fits, small, c"".as_ptr()), slow);
                        self.st(&c, dst, n);
                        self.join(&c, join);
                    }
                    Inst::SubtypeCheck {
                        dope, want, dims, ..
                    } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        if dims <= 4 {
                            // The lengths of both descriptors, read where they are (a native
                            // frame's own slots too); a mismatch is the interpreter's error.
                            let n24 = self.c64(24);
                            for k in 0..i64::from(dims) {
                                let len = |r: Reg| {
                                    let p = self.host(
                                        &c,
                                        self.ld(&c, r),
                                        fk(r),
                                        self.c64(k * 24),
                                        n24,
                                        false,
                                        slow,
                                    );
                                    let (l, rt, dir) = self.dope_words(p);
                                    self.len_of(l, rt, dir)
                                };
                                let ((a, fa), (b, fb)) = (len(dope), len(want));
                                let both = LLVMBuildAnd(self.b, fa, fb, c"".as_ptr());
                                let same = self.icmp(IP::LLVMIntEQ, a, b);
                                self.guard(
                                    &c,
                                    LLVMBuildAnd(self.b, both, same, c"".as_ptr()),
                                    slow,
                                );
                            }
                        } else {
                            let (d, w) = (self.ld(&c, dope), self.ld(&c, want));
                            self.fast_op(&c, FAST_SUBTYPE, u32::from(dims), d, w, slow);
                        }
                        self.join(&c, join);
                    }
                    Inst::SlicePos {
                        dst,
                        dope,
                        left,
                        right,
                        dir,
                        ..
                    } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let p = self.host(
                            &c,
                            self.ld(&c, dope),
                            fk(dope),
                            self.c64(0),
                            self.c64(24),
                            false,
                            slow,
                        );
                        let (l, r, d) = self.dope_words(p);
                        let (a, b, w) = (self.ld(&c, left), self.ld(&c, right), self.ld(&c, dir));
                        let and = |x, y| LLVMBuildAnd(self.b, x, y, c"".as_ptr());
                        let le = |x, y| self.icmp(IP::LLVMIntSLE, x, y);
                        let down = self.icmp(IP::LLVMIntEQ, w, self.c64(1));
                        // A null slice names no element and checks nothing.
                        let null = LLVMBuildSelect(
                            self.b,
                            down,
                            self.icmp(IP::LLVMIntSLT, a, b),
                            self.icmp(IP::LLVMIntSGT, a, b),
                            c"".as_ptr(),
                        );
                        let up = self.icmp(IP::LLVMIntEQ, d, self.c64(0));
                        let inside = |i| {
                            let u = and(up, and(le(l, i), le(i, r)));
                            let n = and(
                                self.icmp(IP::LLVMIntEQ, d, self.c64(1)),
                                and(le(r, i), le(i, l)),
                            );
                            self.or(u, n)
                        };
                        let valid = and(self.icmp(IP::LLVMIntEQ, w, d), and(inside(a), inside(b)));
                        self.guard(&c, self.or(null, valid), slow);
                        let from_l = LLVMBuildSub(self.b, a, l, c"".as_ptr());
                        let from_a = LLVMBuildSub(self.b, l, a, c"".as_ptr());
                        let pos = LLVMBuildSelect(self.b, up, from_l, from_a, c"".as_ptr());
                        let zero = self.c64(0);
                        self.st(
                            &c,
                            dst,
                            LLVMBuildSelect(self.b, null, zero, pos, c"".as_ptr()),
                        );
                        self.join(&c, join);
                    }
                    Inst::Pos {
                        dst,
                        dope,
                        dim,
                        idx,
                        ..
                    } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n24 = self.c64(24);
                        let p = self.host(
                            &c,
                            self.ld(&c, dope),
                            fk(dope),
                            self.c64(i64::from(dim) * 24),
                            n24,
                            false,
                            slow,
                        );
                        let (l, r, dir) = self.dope_words(p);
                        let i = self.ld(&c, idx);
                        // Upwards `l..r` (dir 0) or downwards `r..l` (dir 1): `i` is in the
                        // range exactly when `i - lo` is at most `hi - lo` as unsigned numbers,
                        // and a null range (`lo > hi`) holds no `i`. One arm per direction with
                        // the tests of that arm only (any other `dir` is the interpreter's):
                        // fewer instructions on the chain from the descriptor to the address.
                        let sub = |a, b| LLVMBuildSub(self.b, a, b, c"".as_ptr());
                        let (up_bb, dn_bb, done) = (self.block(&c), self.block(&c), self.block(&c));
                        let up = self.icmp(IP::LLVMIntEQ, dir, self.c64(0));
                        self.cond_br(up, up_bb, dn_bb, true);
                        LLVMPositionBuilderAtEnd(self.b, up_bb);
                        let pu = sub(i, l);
                        let ok = LLVMBuildAnd(
                            self.b,
                            self.icmp(IP::LLVMIntSLE, l, r),
                            self.icmp(IP::LLVMIntULE, pu, sub(r, l)),
                            c"".as_ptr(),
                        );
                        self.guard(&c, ok, slow);
                        let up_end = LLVMGetInsertBlock(self.b);
                        LLVMBuildBr(self.b, done);
                        LLVMPositionBuilderAtEnd(self.b, dn_bb);
                        let pd = sub(l, i);
                        let ok = LLVMBuildAnd(
                            self.b,
                            LLVMBuildAnd(
                                self.b,
                                self.icmp(IP::LLVMIntEQ, dir, self.c64(1)),
                                self.icmp(IP::LLVMIntSLE, r, l),
                                c"".as_ptr(),
                            ),
                            self.icmp(IP::LLVMIntULE, pd, sub(l, r)),
                            c"".as_ptr(),
                        );
                        self.guard(&c, ok, slow);
                        let dn_end = LLVMGetInsertBlock(self.b);
                        LLVMBuildBr(self.b, done);
                        LLVMPositionBuilderAtEnd(self.b, done);
                        let pos = LLVMBuildPhi(self.b, self.i64, c"".as_ptr());
                        let mut vals = [pu, pd];
                        let mut from = [up_end, dn_end];
                        LLVMAddIncoming(pos, vals.as_mut_ptr(), from.as_mut_ptr(), 2);
                        self.st(&c, dst, pos);
                        self.join(&c, join);
                    }
                    Inst::Field {
                        dst,
                        base,
                        layout,
                        n,
                    } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n8 = self.c64(8);
                        let p = self.host(
                            &c,
                            self.ld(&c, layout),
                            fk(layout),
                            self.c64(16 + 8 * i64::from(n)),
                            n8,
                            false,
                            slow,
                        );
                        let off = self.load(self.i64, p);
                        let a = self.ld(&c, base);
                        let known_obj = match fk(base) {
                            Some(Known::Heap(Some(o))) => Some(o),
                            _ => None,
                        };
                        let lo = match known_obj {
                            Some(o) => o.off,
                            None => LLVMBuildAnd(self.b, a, self.c64(0xffff_ffff), c"".as_ptr()),
                        };
                        let (sum, o) = self.ovf(self.uadd, lo, off);
                        let big = self.icmp(IP::LLVMIntUGT, sum, self.c64(0xffff_ffff));
                        self.guard(
                            &c,
                            LLVMBuildNot(self.b, self.or(o, big), c"".as_ptr()),
                            slow,
                        );
                        let hi = match known_obj {
                            Some(o) => o.hi,
                            None => LLVMBuildAnd(self.b, a, self.c64(!0xffff_ffff), c"".as_ptr()),
                        };
                        self.st(&c, dst, LLVMBuildOr(self.b, hi, sum, c"".as_ptr()));
                        self.join(&c, join);
                        if let (Some(o), true) = (known_obj, c.nat.is_some()) {
                            c.objnew.set(Some((dst, ObjRef { off: sum, ..o })));
                        }
                    }
                    Inst::LGet { dst, base, idx } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let (p, sh) = self.logic_word(&c, base, fk(base), idx, false, slow);
                        let mut code = self.c64(0);
                        for k in 0..4 {
                            let w = self.load(self.i64, self.gep(p, self.c64(k * 8)));
                            let bit = LLVMBuildLShr(self.b, w, sh, c"".as_ptr());
                            let bit = LLVMBuildAnd(self.b, bit, self.c64(1), c"".as_ptr());
                            let bit = LLVMBuildShl(self.b, bit, self.c64(k), c"".as_ptr());
                            code = LLVMBuildOr(self.b, code, bit, c"".as_ptr());
                        }
                        self.st(&c, dst, code);
                        self.join(&c, join);
                    }
                    // A run of 1 to 64 elements inside one plane block set to one code; any other
                    // run is stepped.
                    Inst::LFill { base, idx, n, v } => {
                        let (slow, join) = self.slow_path_x(&c, pc, &inst, f);
                        let code = self.ld(&c, v);
                        self.guard(&c, self.icmp(IP::LLVMIntULE, code, self.c64(8)), slow);
                        let cnt = self.ld(&c, n);
                        let n1 = LLVMBuildSub(self.b, cnt, self.c64(1), c"".as_ptr());
                        self.guard(&c, self.icmp(IP::LLVMIntULT, n1, self.c64(64)), slow);
                        let (p, sh) = self.logic_word(&c, base, fk(base), idx, true, slow);
                        let end = LLVMBuildAdd(self.b, sh, cnt, c"".as_ptr());
                        self.guard(&c, self.icmp(IP::LLVMIntULE, end, self.c64(64)), slow);
                        let low = LLVMBuildSub(
                            self.b,
                            LLVMBuildShl(self.b, self.c64(1), cnt, c"".as_ptr()),
                            self.c64(1),
                            c"".as_ptr(),
                        );
                        let low = LLVMBuildSelect(
                            self.b,
                            self.icmp(IP::LLVMIntEQ, cnt, self.c64(64)),
                            self.c64(-1),
                            low,
                            c"".as_ptr(),
                        );
                        let mask = LLVMBuildShl(self.b, low, sh, c"".as_ptr());
                        let keep = LLVMBuildNot(self.b, mask, c"".as_ptr());
                        for k in 0..4 {
                            let pk = self.gep(p, self.c64(k * 8));
                            let w =
                                LLVMBuildAnd(self.b, self.load(self.i64, pk), keep, c"".as_ptr());
                            let bit = LLVMBuildAnd(
                                self.b,
                                LLVMBuildLShr(self.b, code, self.c64(k), c"".as_ptr()),
                                self.c64(1),
                                c"".as_ptr(),
                            );
                            let set = LLVMBuildAnd(
                                self.b,
                                mask,
                                LLVMBuildSub(self.b, self.c64(0), bit, c"".as_ptr()),
                                c"".as_ptr(),
                            );
                            self.store(self.or(w, set), pk);
                        }
                        self.join(&c, join);
                    }
                    // Equality of two logic arrays of 1 to 64 elements: the four planes under the
                    // mask of the elements; any other count is stepped.
                    Inst::LEq { dst, a, b, n } => {
                        let (slow, join) = self.slow_path_x(&c, pc, &inst, f);
                        let cnt = self.ld(&c, n);
                        let n1 = LLVMBuildSub(self.b, cnt, self.c64(1), c"".as_ptr());
                        self.guard(&c, self.icmp(IP::LLVMIntULT, n1, self.c64(64)), slow);
                        let pa = self.host(
                            &c,
                            self.ld(&c, a),
                            fk(a),
                            self.c64(0),
                            self.c64(32),
                            false,
                            slow,
                        );
                        let pb = self.host(
                            &c,
                            self.ld(&c, b),
                            fk(b),
                            self.c64(0),
                            self.c64(32),
                            false,
                            slow,
                        );
                        let mut diff = self.c64(0);
                        for k in 0..4 {
                            let x = LLVMBuildXor(
                                self.b,
                                self.load(self.i64, self.gep(pa, self.c64(k * 8))),
                                self.load(self.i64, self.gep(pb, self.c64(k * 8))),
                                c"".as_ptr(),
                            );
                            diff = self.or(diff, x);
                        }
                        // The low `cnt` bits (a shift by 64 is poison, `cnt` is 1 to 64).
                        let mask = LLVMBuildLShr(
                            self.b,
                            self.c64(-1),
                            LLVMBuildSub(self.b, self.c64(64), cnt, c"".as_ptr()),
                            c"".as_ptr(),
                        );
                        let masked = LLVMBuildAnd(self.b, diff, mask, c"".as_ptr());
                        let eq = self.icmp(IP::LLVMIntEQ, masked, self.c64(0));
                        self.st(&c, dst, self.zext(eq));
                        self.join(&c, join);
                    }
                    Inst::LSet { base, idx, v } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let code = self.ld(&c, v);
                        // A code beyond 'U' to '-' is the interpreter's error.
                        self.guard(&c, self.icmp(IP::LLVMIntULE, code, self.c64(8)), slow);
                        let (p, sh) = self.logic_word(&c, base, fk(base), idx, true, slow);
                        let one = LLVMBuildShl(self.b, self.c64(1), sh, c"".as_ptr());
                        let keep = LLVMBuildNot(self.b, one, c"".as_ptr());
                        for k in 0..4 {
                            let pk = self.gep(p, self.c64(k * 8));
                            let w =
                                LLVMBuildAnd(self.b, self.load(self.i64, pk), keep, c"".as_ptr());
                            let bit = LLVMBuildLShr(self.b, code, self.c64(k), c"".as_ptr());
                            let bit = LLVMBuildAnd(self.b, bit, self.c64(1), c"".as_ptr());
                            let bit = LLVMBuildShl(self.b, bit, sh, c"".as_ptr());
                            self.store(LLVMBuildOr(self.b, w, bit, c"".as_ptr()), pk);
                        }
                        self.join(&c, join);
                    }
                    // An access value a `Deref` found before is in its object's entry of
                    // `Mem::objs` (bits 32 and up of the value are the object's index): its
                    // address inline. Else the heap's own lookup through a helper; a failure is
                    // the interpreter's.
                    Inst::Deref { dst, handle, .. } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let h = self.ld(&c, handle);
                        let (tk, mx) = self.obj_tag(&c, None);
                        let e = self.obj_entry(&c, h, true, mx);
                        let eh = self.load(self.i64, self.gep(e, self.c64(OBJ_H as i64)));
                        // An entry with an access value is writable, and a null handle is no
                        // entry's value (`NO_H`); the key's tag says the entry is this machine's
                        // (another instance holds the same access values).
                        let ek = LLVMBuildAnd(
                            self.b,
                            self.load(self.i64, e),
                            self.c64(!0xffff_ffff),
                            c"".as_ptr(),
                        );
                        let hit = LLVMBuildAnd(
                            self.b,
                            self.icmp(IP::LLVMIntEQ, eh, h),
                            self.icmp(IP::LLVMIntEQ, ek, tk),
                            c"".as_ptr(),
                        );
                        let miss = self.block(&c);
                        if c.nat.is_some() {
                            // The object stays in hand for the instructions that follow
                            // (`ObjRef`).
                            self.cond_br(hit, join, miss, true);
                            LLVMPositionBuilderAtEnd(self.b, miss);
                            // The callback enters the object at `e`, writable, for `h`; a native
                            // frame's slow path leaves, so the join is reached by these two.
                            self.fast_reg(&c, FAST_DEREF, dst, h, self.c64(0), slow);
                            // The callback may have found the address and failed to enter the
                            // object (`yavs_rt_fast`): the entry must hold what `dst` points into.
                            let found = self.load(self.i64, e);
                            let at =
                                LLVMBuildLShr(self.b, self.ld(&c, dst), self.c64(32), c"".as_ptr());
                            let at =
                                LLVMBuildOr(self.b, at, self.obj_tag(&c, None).0, c"".as_ptr());
                            self.guard(&c, self.icmp(IP::LLVMIntEQ, found, at), slow);
                            LLVMBuildBr(self.b, join);
                            LLVMPositionBuilderAtEnd(self.b, join);
                            self.join_here(&c, join);
                            let hi = self.load(self.i64, e);
                            let o = ObjRef {
                                ptr: self.load(self.ptr, self.gep(e, self.c64(OBJ_PTR as i64))),
                                len: self.load(self.i64, self.gep(e, self.c64(OBJ_LEN as i64))),
                                hi: LLVMBuildShl(self.b, hi, self.c64(32), c"".as_ptr()),
                                off: self.c64(0),
                            };
                            self.st(&c, dst, o.hi);
                            c.objnew.set(Some((dst, o)));
                        } else {
                            let fast = self.block(&c);
                            self.cond_br(hit, fast, miss, true);
                            LLVMPositionBuilderAtEnd(self.b, fast);
                            let hi = self.load(self.i64, e);
                            self.st(
                                &c,
                                dst,
                                LLVMBuildShl(self.b, hi, self.c64(32), c"".as_ptr()),
                            );
                            LLVMBuildBr(self.b, join);
                            LLVMPositionBuilderAtEnd(self.b, miss);
                            self.fast_reg(&c, FAST_DEREF, dst, h, self.c64(0), slow);
                            self.join(&c, join);
                        }
                    }
                    Inst::MemEq { dst, a, b, n } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let k = self.ld(&c, n);
                        let pa = self.host(&c, self.ld(&c, a), fk(a), self.c64(0), k, false, slow);
                        let pb = self.host(&c, self.ld(&c, b), fk(b), self.c64(0), k, false, slow);
                        let mut args = [pa, pb, k];
                        let v = LLVMBuildCall2(
                            self.b,
                            self.memeq_ty,
                            self.memeq,
                            args.as_mut_ptr(),
                            3,
                            c"".as_ptr(),
                        );
                        self.st(&c, dst, v);
                        self.join(&c, join);
                    }
                    Inst::SigVal {
                        dst,
                        sig,
                        offset,
                        len,
                    } => {
                        // `Machine::sig_val`: element range within u32, an instance signal; a
                        // reference that is not leaves to the interpreter, which reports it.
                        let (o, l) = (self.ld(&c, offset), self.ld(&c, len));
                        let max = self.c64(i64::from(u32::MAX));
                        let n = self.mem_ld(&c, MEM_NSIGS, self.i64);
                        let ok = LLVMBuildAnd(
                            self.b,
                            self.icmp(IP::LLVMIntULE, o, max),
                            self.icmp(IP::LLVMIntULE, l, max),
                            c"".as_ptr(),
                        );
                        let s = self.c64(i64::from(sig));
                        let has = self.icmp(IP::LLVMIntULT, s, n);
                        let bad = LLVMBuildNot(
                            self.b,
                            LLVMBuildAnd(self.b, ok, has, c"".as_ptr()),
                            c"".as_ptr(),
                        );
                        self.exit_if(&c, bad, pc);
                        let hi = self.c64(i64::from(sig) << 32);
                        self.std(&c, dst, LLVMBuildOr(self.b, hi, o, c"".as_ptr()));
                        self.std(&c, Reg(dst.0 + 1), l);
                    }
                    Inst::Br { to } => {
                        let bb = block_bb(to)?;
                        // A process that never suspends polls the stop request (#737) and
                        // counts its jumps back in the machine (`TierFrame::spin`, #583) at
                        // every retreating `Br`: at a request, at every `SPIN_CHECK`th jump
                        // (trim, limit) it leaves at this `Br`, whose interpreter step suspends,
                        // trims or reports the process.
                        // The retreating `Br`s only (a `Br` to a join laid out earlier is no
                        // loop), and as the interpreter steps only those that jump back.
                        if backs.contains(&pc) && start_of(to)? <= pc as usize {
                            let at = self.tag(self.mem_ptr(&c, MEM_STOP), AA_MEM);
                            let flags = self.load(self.ptr, at);
                            let byte = LLVMBuildLoad2(self.b, self.i8, flags, c"".as_ptr());
                            self.tag(flags, AA_STOP);
                            self.scope(byte, flags);
                            LLVMSetOrdering(
                                byte,
                                llvm_sys::LLVMAtomicOrdering::LLVMAtomicOrderingMonotonic,
                            );
                            let hit = LLVMBuildAnd(
                                self.b,
                                byte,
                                LLVMConstInt(self.i8, u64::from(yavs_interp::STOP_BIT), 0),
                                c"".as_ptr(),
                            );
                            let stop = self.icmp(IP::LLVMIntNE, hit, LLVMConstInt(self.i8, 0, 0));
                            let local = c.spin.expect("spin local");
                            let n = LLVMBuildLoad2(self.b, self.i64, local, c"".as_ptr());
                            let n = LLVMBuildAdd(self.b, n, self.c64(1), c"".as_ptr());
                            // Leaves before it stores: the interpreter's step counts this jump.
                            let low = LLVMBuildAnd(
                                self.b,
                                n,
                                self.c64(yavs_interp::SPIN_CHECK as i64 - 1),
                                c"".as_ptr(),
                            );
                            let check = self.icmp(IP::LLVMIntEQ, low, self.c64(0));
                            let bad = LLVMBuildOr(self.b, stop, check, c"".as_ptr());
                            self.exit_if(&c, bad, pc);
                            LLVMBuildStore(self.b, n, local);
                        }
                        let snap = self.edge_prep(&c, &[start_of(to)?]);
                        self.edge_to(&c, &snap, start_of(to)?);
                        LLVMBuildBr(self.b, bb);
                        open = false;
                    }
                    Inst::CondBr { c: r, t, f: e } => {
                        let x = self.icmp(IP::LLVMIntNE, self.ld(&c, r), self.c64(0));
                        let (bt, be) = (block_bb(t)?, block_bb(e)?);
                        let snap = self.edge_prep(&c, &[start_of(t)?, start_of(e)?]);
                        self.edge_to(&c, &snap, start_of(t)?);
                        self.edge_to(&c, &snap, start_of(e)?);
                        LLVMBuildCondBr(self.b, x, bt, be);
                        open = false;
                    }
                    Inst::Case { v, table } => {
                        let t = prog
                            .cases
                            .get(table)
                            .ok_or_else(|| format!("no case table {}", table.0))?;
                        let x = self.ld(&c, v);
                        let mut to_all = vec![start_of(t.default)?];
                        for &(_, _, to) in &t.arms {
                            to_all.push(start_of(to)?);
                        }
                        let snap = self.edge_prep(&c, &to_all);
                        // ponytail: a compare chain per arm; a switch for dense tables if
                        // case-heavy code ever shows in profiles.
                        for &(lo, hi, to) in &t.arms {
                            let ge = self.icmp(IP::LLVMIntSGE, x, self.c64(lo));
                            let le = self.icmp(IP::LLVMIntSLE, x, self.c64(hi));
                            let hit = LLVMBuildAnd(self.b, ge, le, c"".as_ptr());
                            let next = self.block(&c);
                            self.edge_to(&c, &snap, start_of(to)?);
                            LLVMBuildCondBr(self.b, hit, block_bb(to)?, next);
                            LLVMPositionBuilderAtEnd(self.b, next);
                        }
                        self.edge_to(&c, &snap, start_of(t.default)?);
                        LLVMBuildBr(self.b, block_bb(t.default)?);
                        open = false;
                    }
                    // A constant range inside one plane block on each side (most copies of record
                    // fields): masks and shifts are constants, no block arithmetic.
                    Inst::LCopy {
                        dst,
                        dst_off,
                        src,
                        src_off,
                        n,
                    } if self.lcopy_shape(&c, n, src_off, dst_off).is_some() => {
                        let shape = self.lcopy_shape(&c, n, src_off, dst_off).expect("checked");
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        self.lcopy_const(&c, dst, src, fk(dst), fk(src), shape, slow);
                        self.join(&c, join);
                    }
                    Inst::LCopy {
                        dst,
                        dst_off,
                        src,
                        src_off,
                        n,
                    } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let (k, so, d) =
                            (self.ld(&c, n), self.ld(&c, src_off), self.ld(&c, dst_off));
                        // Nothing to copy: no memory is touched, as in the interpreter.
                        let go = self.block(&c);
                        let z = self.icmp(IP::LLVMIntEQ, k, self.c64(0));
                        LLVMBuildCondBr(self.b, z, join, go);
                        LLVMPositionBuilderAtEnd(self.b, go);
                        let small = |v| self.icmp(IP::LLVMIntULT, v, self.c64(1 << 32));
                        let ok = LLVMBuildAnd(self.b, small(k), small(so), c"".as_ptr());
                        self.guard(&c, LLVMBuildAnd(self.b, ok, small(d), c"".as_ptr()), slow);
                        // First block, block count, first element in it.
                        let blocks = |off| {
                            let b0 = LLVMBuildLShr(self.b, off, self.c64(6), c"".as_ptr());
                            let last = LLVMBuildAdd(self.b, off, k, c"".as_ptr());
                            let last = LLVMBuildSub(self.b, last, self.c64(1), c"".as_ptr());
                            let b1 = LLVMBuildLShr(self.b, last, self.c64(6), c"".as_ptr());
                            let nb = LLVMBuildSub(self.b, b1, b0, c"".as_ptr());
                            let nb = LLVMBuildAdd(self.b, nb, self.c64(1), c"".as_ptr());
                            let first = LLVMBuildShl(self.b, b0, self.c64(6), c"".as_ptr());
                            (b0, nb, LLVMBuildSub(self.b, off, first, c"".as_ptr()))
                        };
                        let (sb0, ns, s0) = blocks(so);
                        let (db0, nd, d0) = blocks(d);
                        let max = self.c64(LCOPY_BLOCKS as i64);
                        let fit = LLVMBuildAnd(
                            self.b,
                            self.icmp(IP::LLVMIntULE, ns, max),
                            self.icmp(IP::LLVMIntULE, nd, max),
                            c"".as_ptr(),
                        );
                        self.guard(&c, fit, slow);
                        let b32 = |v| LLVMBuildShl(self.b, v, self.c64(5), c"".as_ptr());
                        let ps = self.host(
                            &c,
                            self.ld(&c, src),
                            fk(src),
                            b32(sb0),
                            b32(ns),
                            false,
                            slow,
                        );
                        let pd =
                            self.host(&c, self.ld(&c, dst), fk(dst), b32(db0), b32(nd), true, slow);
                        // Up to 64 elements: at most two blocks each side, shifted in registers. A
                        // process does it per activation; in a subprogram the 225 LLVM
                        // instructions are not worth the call they save.
                        // A count known only at run time makes the inline body dead weight in
                        // the wide-vector processes that never copy so few (#634, round 6).
                        let inline_ok = self.kint(&c, n).is_some_and(|v| v <= 64);
                        if f.kind == FuncKind::Process && inline_ok {
                            let few = self.icmp(IP::LLVMIntULE, k, self.c64(64));
                            let (inl, call) = (self.block(&c), self.block(&c));
                            LLVMBuildCondBr(self.b, few, inl, call);
                            LLVMPositionBuilderAtEnd(self.b, inl);
                            self.lcopy_inline(&c, pd, nd, d0, ps, ns, s0, k);
                            LLVMBuildBr(self.b, join);
                            LLVMPositionBuilderAtEnd(self.b, call);
                        }
                        let mut args = [pd, nd, d0, ps, ns, s0, k];
                        LLVMBuildCall2(
                            self.b,
                            self.lcopy_ty,
                            self.lcopy,
                            args.as_mut_ptr(),
                            7,
                            c"".as_ptr(),
                        );
                        self.join(&c, join);
                    }
                    Inst::Cat {
                        dst,
                        n,
                        m,
                        parts,
                        scalars,
                        bias,
                        logic: true,
                        ..
                    } => {
                        let rs = f.regs(parts).ok_or("operand list out of range")?;
                        if !self.cat_block(&c, pc, &inst, dst, n, m, rs, scalars, bias, fk(dst)) {
                            self.slow_inst(&c, pc, &inst, f)?;
                        }
                    }
                    Inst::SigReadScalar { dst, sig } => self.sig_read_scalar(&c, pc, dst, sig),
                    Inst::Edge { dst, kind, sig } => self.edge(&c, pc, kind, sig, dst, 0),
                    Inst::SigAttr {
                        dst,
                        kind: yavs_ir::AttrKind::Event,
                        sig,
                    } if c.nat.is_none() => self.sig_event(&c, pc, dst, sig),
                    Inst::Drive {
                        driver,
                        target: Some(t),
                        wave,
                        n,
                        mode,
                        reject: None,
                    } => self.drive_block(&c, pc, Ok(driver), t, wave, n, mode),
                    Inst::DriveSlot {
                        slot,
                        target,
                        wave,
                        n,
                        mode,
                        reject: None,
                    } => self.drive_block(&c, pc, Err(slot), target, wave, n, mode),
                    Inst::WaitOn { sig }
                        if c.nat.is_none()
                            && matches!(f.code.get(i + 1), Some(Inst::Suspend { .. })) =>
                    {
                        self.wait_suspend(&c, pc, sig)
                    }
                    Inst::WaitOn { sig } if c.nat.is_none() => {
                        // The kernel keeps the waiter of the last wait: the call needs no
                        // register in the frame. Only if it does not, the instruction runs in the
                        // interpreter's way, and the frame is stored for that.
                        let (slow, join) = self.slow_signal(&c, pc, 0);
                        let mut args = [c.env, self.ld(&c, sig), self.ld(&c, Reg(sig.0 + 1))];
                        let r = LLVMBuildCall2(
                            self.b,
                            self.wait_again_ty,
                            self.wait_again,
                            args.as_mut_ptr(),
                            3,
                            c"".as_ptr(),
                        );
                        self.guard(&c, self.icmp(IP::LLVMIntNE, r, self.c32(0)), slow);
                        self.join(&c, join);
                    }
                    Inst::WaitEdge { sig, kind } if c.nat.is_none() => {
                        // On the kernel itself (no register read); else the instruction runs in
                        // the interpreter's way.
                        let (slow, join) = self.slow_signal(&c, pc, 0);
                        let mut args = [
                            c.env,
                            c.mem,
                            self.ld(&c, sig),
                            self.ld(&c, Reg(sig.0 + 1)),
                            self.c32(u32::from(kind.code())),
                        ];
                        let r = LLVMBuildCall2(
                            self.b,
                            self.wait_edge_ty,
                            self.wait_edge,
                            args.as_mut_ptr(),
                            5,
                            c"".as_ptr(),
                        );
                        self.guard(&c, self.icmp(IP::LLVMIntNE, r, self.c32(0)), slow);
                        self.join(&c, join);
                    }
                    Inst::SigRead { dst, sig } if self.sig_read_block(&c, pc, dst, sig) => {}
                    // Through the signal callback rather than a step (#790); a native frame
                    // keeps its own way.
                    Inst::SigGather { .. } | Inst::SigScatter { .. } if c.nat.is_none() => {
                        self.signal(&c, pc, 0)
                    }
                    Inst::SigRead { .. }
                    | Inst::WaitOn { .. }
                    | Inst::WaitEdge { .. }
                    | Inst::WaitFor { .. }
                    | Inst::DriveSlot { .. }
                    | Inst::Drive { .. }
                    | Inst::Suspend { .. } => self.signal(&c, pc, 0),
                    Inst::CallNative { native, args, dst } => {
                        match native_kind(prog, f, native, args, dst) {
                            Some(NativeKind::Table(t, rs, d)) => {
                                let (slow, join) = self.slow_path(&c, pc, &inst);
                                let mut idx = self.c64(0);
                                for r in rs {
                                    // A code beyond the nine is the native's error.
                                    let x = self.ld(&c, r);
                                    self.guard(&c, self.icmp(IP::LLVMIntULE, x, self.c64(8)), slow);
                                    let i9 = LLVMBuildMul(self.b, idx, self.c64(9), c"".as_ptr());
                                    idx = LLVMBuildAdd(self.b, i9, x, c"".as_ptr());
                                }
                                let p = self.gep(self.table(&t), idx);
                                self.st(&c, d, self.zext8(self.load(self.i8, p)));
                                self.join(&c, join);
                            }
                            Some(NativeKind::Edge(rising)) => match (f.regs(args), dst) {
                                (Some(&[s]), Some(d)) => {
                                    let (kind, e) = if rising {
                                        (EdgeKind::Rising, 1)
                                    } else {
                                        (EdgeKind::Falling, 2)
                                    };
                                    self.edge(&c, pc, kind, s, d, e)
                                }
                                _ => self.signal(&c, pc, if rising { 1 } else { 2 }),
                            },
                            None if let Some(v) = uniform_regs(prog, f, native, args, dst) => {
                                self.uniform_fast(&c, pc, &inst, f, native, args, v)?
                            }
                            None if c.nat.is_some()
                                && prog
                                    .natives
                                    .get(native)
                                    .is_some_and(|n| yavs_interp::cold_native(&n.symbol)) =>
                            {
                                self.step(&c, pc, &inst)
                            }
                            None if (c.nat.is_some() && nat_inline(prog, f, &inst))
                                || (c.nat.is_none() && frame_direct(prog, f, &inst)) =>
                            {
                                let light = yavs_interp::light_native(prog, f, native, args);
                                match ieee_fast(prog, f, native, args, dst).filter(|_| light) {
                                    Some(op) => {
                                        self.ieee_fast(&c, pc, &inst, f, native, args, dst, op)?
                                    }
                                    None => self.native_direct(
                                        &c, pc, &inst, f, native, args, dst, light,
                                    )?,
                                }
                            }
                            None => self.step(&c, pc, &inst),
                        }
                    }
                    Inst::Call { func, args, dst }
                        if nc.contains(&func)
                            && prog
                                .func(func)
                                .is_some_and(|g| call_fits(f, id, g, args, false)) =>
                    {
                        let g = prog.func(func).ok_or("callee vanished")?;
                        let rs = f.regs(args).ok_or("operand list out of range")?;
                        let memo = prog
                            .memo_sig(func)
                            .is_some_and(|m| m.cells as usize == rs.len());
                        self.native_call(&c, pc, &inst, id, f, func, g, rs, dst, None, memo);
                    }
                    Inst::CallProt {
                        func,
                        obj,
                        args,
                        dst,
                    } if nc.contains(&func)
                        && obj.0 < f.reg_ty.len() as u32
                        && prog
                            .func(func)
                            .is_some_and(|g| call_fits(f, id, g, args, true)) =>
                    {
                        let g = prog.func(func).ok_or("callee vanished")?;
                        let rs = f.regs(args).ok_or("operand list out of range")?;
                        self.native_call(&c, pc, &inst, id, f, func, g, rs, dst, Some(obj), false);
                    }
                    Inst::NewFrame {
                        dst,
                        desc,
                        dsize,
                        data,
                        size,
                        fill,
                    } if c.nat.is_some() => {
                        let fk = |r: Reg| self.known_addr(&c, r);
                        self.new_frame(&c, pc, &inst, dst, desc, dsize, data, size, fill, &fk);
                    }
                    Inst::Fill { dst, elem, n, size } if c.nat.is_some() => {
                        let fk = |r: Reg| self.known_addr(&c, r);
                        self.fill(&c, pc, &inst, dst, elem, n, size, &fk);
                    }
                    Inst::Cat {
                        dst,
                        n: _,
                        m: elem,
                        parts,
                        scalars,
                        bias: _,
                        logic: false,
                        w,
                    } if c.nat.is_some() || nat_inline(prog, f, &inst) => {
                        let rs = f.regs(parts).ok_or("operand list out of range")?;
                        let fk = |r: Reg| self.known_addr(&c, r);
                        self.cat_bytes(&c, pc, &inst, dst, elem, rs, scalars, w, &fk);
                    }
                    Inst::TempRoot {
                        addr,
                        desc,
                        ty: Some(t),
                    } if c.nat.is_some() => {
                        let fk = |r: Reg| self.known_addr(&c, r);
                        let n =
                            yavs_ir::Ty::desc_size(&prog.types, t).ok_or("no descriptor size")?;
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let dp = match (n, desc) {
                            (0, _) => LLVMConstPointerNull(self.ptr),
                            (_, Some(d)) => self.host(
                                &c,
                                self.ld(&c, d),
                                fk(d),
                                self.c64(0),
                                self.c64(n as i64),
                                false,
                                slow,
                            ),
                            _ => return Err("recorded temporary without its descriptor".into()),
                        };
                        let ok = self.troot_call(&c, self.ld(&c, addr), t.0, dp, n);
                        self.guard(&c, self.icmp(IP::LLVMIntNE, ok, self.c32(0)), slow);
                        self.join(&c, join);
                    }
                    Inst::Unreachable { .. } if c.nat.is_some() => self.step(&c, pc, &inst),
                    _ if c.nat.is_some() && xstep_ok(f, &inst) => {
                        self.native_step(&c, pc, &inst, f)?;
                    }
                    _ => self.step(&c, pc, &inst),
                }
            }
            if open {
                // Falling off the end: the interpreter reports it.
                self.leave(&c, self.c32(n as u32));
            }
            let kend = if kinds {
                let now = fn_count(fv);
                let n = kpend.len().max(1);
                for k in kpend.drain(..) {
                    kind_add_frac(k, now - kwas, n);
                }
                now
            } else {
                0
            };
            if let Some(nat) = &c.nat {
                // Materialisation: record the registers, slots, pc and temporary base, and
                // return with `Mem::ndeopt` set.
                LLVMPositionBuilderAtEnd(self.b, exit);
                c.known.borrow_mut().fill(None);
                let mut args = [
                    c.env,
                    c.mem,
                    self.c32(nat.id.0),
                    exit_pc,
                    LLVMBuildLoad2(self.b, self.i64, nat.temp_base, c"".as_ptr()),
                    self.c64(nregs as i64),
                    self.c64(i64::from(f.frame)),
                    nat.up[0],
                    nat.spill,
                ];
                LLVMBuildCall2(
                    self.b,
                    self.deopt_ty,
                    self.deopt,
                    args.as_mut_ptr(),
                    9,
                    c"".as_ptr(),
                );
                let mut env = [c.env];
                let sp = LLVMBuildCall2(
                    self.b,
                    self.deopt_slots_ty,
                    self.deopt_slots,
                    env.as_mut_ptr(),
                    1,
                    c"".as_ptr(),
                );
                match &nat.closed {
                    // The rest of the buffer is zero and so were these bytes (`closed_frame`).
                    Some(rs) => rs.iter().for_each(|&(lo, hi)| {
                        let (to, from) = (
                            self.gep(sp, self.c64(lo as i64)),
                            self.gep(nat.slots, self.c64(lo as i64)),
                        );
                        LLVMBuildMemCpy(self.b, to, 1, from, 1, self.c64((hi - lo) as i64));
                    }),
                    None => {
                        LLVMBuildMemCpy(self.b, sp, 1, nat.slots, 8, self.c64(i64::from(f.frame)));
                    }
                }
                LLVMBuildRet(self.b, self.c64(0));
            }
            if let Some(k) = &kloc {
                kindloc_cold(fv, &c.cold.borrow());
                LLVMSetCurrentDebugLocation2(self.b, std::ptr::null_mut());
                llvm_sys::debuginfo::LLVMDIBuilderFinalize(k.b);
                llvm_sys::debuginfo::LLVMDisposeDIBuilder(k.b);
            }
            layout(fv, &c.cold.borrow());
            // A block nothing branches to (the generator and `regs::reach` agree on all but
            // dead code) has phis without incoming values.
            if let Some(pl) = c.defer.as_ref().and_then(|d| d.plan.as_ref()) {
                for &phi in pl.phis.iter().flatten() {
                    if LLVMCountIncoming(phi) == 0 {
                        LLVMReplaceAllUsesWith(phi, LLVMGetUndef(self.i64));
                        LLVMInstructionEraseFromParent(phi);
                    }
                }
            }
            if kinds {
                kind_add("<tail>".into(), 0, fn_count(fv) - kend);
            }
            debug_assert!(c.joins.borrow().is_empty(), "a slow path was not joined");
            if LLVMVerifyFunction(fv, LLVMVerifierFailureAction::LLVMReturnStatusAction) != 0 {
                // A native function may have callers in the module already: the whole module
                // is generated again without native frames (`compile`).
                if !native {
                    LLVMDeleteFunction(fv);
                }
                return Err("the generated LLVM function does not verify (a bug in yavs)".into());
            }
            Ok(())
        }
    }

    /// Like `slow_path`, but the interpreter runs the signal instruction at `pc` through
    /// `TierHost::signal` (`edge` as in `signal`).
    unsafe fn slow_signal(
        &self,
        c: &Cur,
        pc: u32,
        edge: u32,
    ) -> (LLVMBasicBlockRef, LLVMBasicBlockRef) {
        // SAFETY: well-formed blocks of c.f.
        unsafe {
            let here = LLVMGetInsertBlock(self.b);
            let (slow, join) = (self.block(c), self.block(c));
            LLVMPositionBuilderAtEnd(self.b, slow);
            // As in `slow_path`; a signal operation pushes no frame.
            let base = c
                .defer
                .as_ref()
                .map(|d| *d.pend.borrow().last().expect("base"));
            c.cold.borrow_mut().push(slow);
            self.cold_depth.set(self.cold_depth.get() + 1);
            self.signal(c, pc, edge);
            self.cold_depth.set(self.cold_depth.get() - 1);
            if let (Some(d), Some(b)) = (&c.defer, base) {
                *d.pend.borrow_mut().last_mut().expect("base") = b;
                c.joins.borrow_mut().push((b, false));
            }
            LLVMBuildBr(self.b, join);
            LLVMPositionBuilderAtEnd(self.b, here);
            (slow, join)
        }
    }

    /// The `TierSig` entry of the instance signal that reference registers `sig`, `sig + 1`
    /// name, and the element offset in it; taking `slow` unless the reference is a plain one
    /// of `len` elements to an instance signal the table has.
    unsafe fn sig_entry(
        &self,
        c: &Cur,
        sig: Reg,
        len: Option<i64>,
        slow: LLVMBasicBlockRef,
    ) -> (LLVMValueRef, LLVMValueRef) {
        // SAFETY: loads of `Mem` fields and of a `TierSig` the guard checked is in the table.
        unsafe {
            let (v, l) = (self.ld(c, sig), self.ld(c, Reg(sig.0 + 1)));
            let base = LLVMBuildLShr(self.b, v, self.c64(32), c"".as_ptr());
            let off = LLVMBuildAnd(self.b, v, self.c64(0xffff_ffff), c"".as_ptr());
            let n = self.mem_ld(c, MEM_SIGTAB_LEN, self.i64);
            let has = self.icmp(IP::LLVMIntULT, base, n);
            let want = match len {
                Some(k) => self.icmp(IP::LLVMIntEQ, l, self.c64(k)),
                None => self.icmp(IP::LLVMIntNE, l, self.c64(0)),
            };
            self.guard(c, LLVMBuildAnd(self.b, has, want, c"".as_ptr()), slow);
            let tab = self.mem_ld(c, MEM_SIGTAB, self.ptr);
            let at = LLVMBuildMul(self.b, base, self.c64(SIG_ENT), c"".as_ptr());
            (self.gep(tab, at), off)
        }
    }

    /// `Inst::SigReadScalar` of a logic signal: the byte of its codes, in place, else the four
    /// plane bits of the kernel's blocks (`TierSig::planes`: a signal wider than the codes are
    /// kept for, or a bus of many sources); anything else through `TierHost::signal`.
    unsafe fn sig_read_scalar(&self, c: &Cur, pc: u32, dst: Reg, sig: Reg) {
        // SAFETY: see `sig_entry`; the guards check the element is within the codes or planes.
        unsafe {
            let (slow, join) = self.slow_signal(c, pc, 0);
            let (ent, off) = self.sig_entry(c, sig, None, slow);
            let len = self.load(self.i64, self.gep(ent, self.c64(SIG_LEN)));
            let (bytes, planes) = (self.block(c), self.block(c));
            self.cond_br(self.icmp(IP::LLVMIntULT, off, len), bytes, planes, true);
            LLVMPositionBuilderAtEnd(self.b, bytes);
            let codes = self.tag(
                self.load(self.ptr, self.gep(ent, self.c64(SIG_CODES))),
                AA_CODES,
            );
            let byte = self.load(self.i8, self.gep(codes, off));
            self.st(c, dst, self.zext8(byte));
            LLVMBuildBr(self.b, join);
            LLVMPositionBuilderAtEnd(self.b, planes);
            // Planes, enumeration, integer, boolean or real elements: a call, the same for every
            // signal (the representation is the signal's, not the code's).
            if let Some(d) = &c.defer {
                d.pend.borrow_mut()[dst.0 as usize] = None;
            }
            let mut args = [ent, off, self.slot(c, dst)];
            let ok = LLVMBuildCall2(
                self.b,
                self.scalar_ty,
                self.scalar,
                args.as_mut_ptr(),
                3,
                c"".as_ptr(),
            );
            self.guard(c, self.icmp(IP::LLVMIntNE, ok, self.c32(0)), slow);
            LLVMBuildBr(self.b, join);
            self.join_here(c, join);
        }
    }

    /// `Inst::SigRead` of `l` elements (a count of at most 2^32) from the kernel's plane blocks
    /// of the signal whose table entry is `ent` (`off` the first element in it), through
    /// `yavs_rt_planes`; `slow` when it offers none or has fewer elements. The builder ends in
    /// a block that branches to `join`.
    #[allow(clippy::too_many_arguments)]
    unsafe fn sig_read_planes(
        &self,
        c: &Cur,
        ent: LLVMValueRef,
        off: LLVMValueRef,
        l: LLVMValueRef,
        dst: Reg,
        slow: LLVMBasicBlockRef,
        join: LLVMBasicBlockRef,
    ) {
        // SAFETY: loads from a table entry the caller checked; the guards check the elements
        // lie within the signal and the destination holds their blocks.
        unsafe {
            let blocks = self.load(self.ptr, self.gep(ent, self.c64(SIG_PLANES)));
            let (logic, other) = (self.block(c), self.block(c));
            self.cond_br(
                LLVMBuildIsNotNull(self.b, blocks, c"".as_ptr()),
                logic,
                other,
                true,
            );
            // A count and an offset are below 2^32 where the reference was made; checked, so the
            // sum below cannot wrap.
            LLVMPositionBuilderAtEnd(self.b, other);
            self.guard(
                c,
                self.icmp(IP::LLVMIntULE, l, self.c64(i64::from(u32::MAX))),
                slow,
            );
            let plen = self.load(self.i64, self.gep(ent, self.c64(SIG_PLEN)));
            let used = LLVMBuildAdd(self.b, off, l, c"".as_ptr());
            self.guard(c, self.icmp(IP::LLVMIntULE, used, plen), slow);
            // A signal of enumeration, integer, boolean or real elements: its bytes, `Buf::write_to`.
            let data = self.load(self.ptr, self.gep(ent, self.c64(SIG_BYTES)));
            self.guard(c, LLVMBuildIsNotNull(self.b, data, c"".as_ptr()), slow);
            let es = self.load(self.i64, self.gep(ent, self.c64(SIG_ES)));
            let first = self.load(self.i64, self.gep(ent, self.c64(SIG_OFF)));
            let e = LLVMBuildAdd(self.b, first, off, c"".as_ptr());
            self.guard(
                c,
                self.icmp(IP::LLVMIntULE, es, self.c64(i64::from(u32::MAX))),
                slow,
            );
            // At most 2^32 elements of at most 2^32 bytes: no wrap.
            let nbytes = LLVMBuildMul(self.b, l, es, c"".as_ptr());
            let pd = self.host(
                c,
                self.ld(c, dst),
                self.known_addr(c, dst),
                self.c64(0),
                nbytes,
                true,
                slow,
            );
            let src = self.gep(data, LLVMBuildMul(self.b, e, es, c"".as_ptr()));
            LLVMBuildMemCpy(self.b, pd, 1, src, 1, nbytes);
            LLVMBuildBr(self.b, join);
            LLVMPositionBuilderAtEnd(self.b, logic);
            self.guard(
                c,
                self.icmp(IP::LLVMIntULE, l, self.c64(i64::from(u32::MAX))),
                slow,
            );
            let plen = self.load(self.i64, self.gep(ent, self.c64(SIG_PLEN)));
            let used = LLVMBuildAdd(self.b, off, l, c"".as_ptr());
            self.guard(c, self.icmp(IP::LLVMIntULE, used, plen), slow);
            let first = self.load(self.i64, self.gep(ent, self.c64(SIG_OFF)));
            let bit0 = LLVMBuildAdd(self.b, first, off, c"".as_ptr());
            let nelem = LLVMBuildAdd(self.b, first, plen, c"".as_ptr());
            let nblk = LLVMBuildLShr(
                self.b,
                LLVMBuildAdd(self.b, nelem, self.c64(63), c"".as_ptr()),
                self.c64(6),
                c"".as_ptr(),
            );
            let bytes = LLVMBuildShl(
                self.b,
                LLVMBuildLShr(
                    self.b,
                    LLVMBuildAdd(self.b, l, self.c64(63), c"".as_ptr()),
                    self.c64(6),
                    c"".as_ptr(),
                ),
                self.c64(5),
                c"".as_ptr(),
            );
            let pd = self.host(
                c,
                self.ld(c, dst),
                self.known_addr(c, dst),
                self.c64(0),
                bytes,
                true,
                slow,
            );
            let mut args = [blocks, nblk, bit0, l, pd];
            LLVMBuildCall2(
                self.b,
                self.planes_ty,
                self.planes,
                args.as_mut_ptr(),
                5,
                c"".as_ptr(),
            );
            LLVMBuildBr(self.b, join);
        }
    }

    /// `Inst::SigRead` of a plain reference of more than 64 elements (the count known): from the
    /// kernel's plane blocks; anything else through `TierHost::signal`.
    unsafe fn sig_read_wide(&self, c: &Cur, pc: u32, dst: Reg, sig: Reg, l: i64) {
        // SAFETY: see `sig_entry` and `sig_read_planes`.
        unsafe {
            let (slow, join) = self.slow_signal(c, pc, 0);
            let (ent, off) = self.sig_entry(c, sig, Some(l), slow);
            self.sig_read_planes(c, ent, off, self.c64(l), dst, slow, join);
            self.join_here(c, join);
        }
    }

    /// `Inst::SigRead` of a plain reference of 1 to 64 elements (the count known) to a logic
    /// signal that keeps its codes: the planes built from the code bytes in place, eight
    /// elements at a time; anything else through `TierHost::signal`. `false`: not that case,
    /// nothing generated.
    unsafe fn sig_read_block(&self, c: &Cur, pc: u32, dst: Reg, sig: Reg) -> bool {
        let Some(l) = self.kint(c, Reg(sig.0 + 1)) else {
            // SAFETY: as below.
            unsafe { self.sig_read_dyn(c, pc, dst, sig) };
            return true;
        };
        if l > 64 && l <= i64::from(u32::MAX) {
            // SAFETY: as below.
            unsafe { self.sig_read_wide(c, pc, dst, sig, l) };
            return true;
        }
        if !(1..=64).contains(&l) {
            return false;
        }
        let groups = (l as u64).div_ceil(8);
        // SAFETY: see `sig_entry`; the guards check the groups lie within the codes, and the
        // elements within the plane blocks and one word of each plane.
        unsafe {
            let (slow, join) = self.slow_signal(c, pc, 0);
            let (ent, off) = self.sig_entry(c, sig, Some(l), slow);
            let mask = if l == 64 { !0 } else { (1u64 << l) - 1 };
            // The elements in one word of each of the kernel's planes (the current value; the
            // codes are made from them): four loads, no call (#766).
            let codes_at = self.block(c);
            let blocks = self.tag(
                self.load(self.ptr, self.gep(ent, self.c64(SIG_PLANES))),
                AA_CODES,
            );
            self.guard(
                c,
                LLVMBuildIsNotNull(self.b, blocks, c"".as_ptr()),
                codes_at,
            );
            let plen = self.load(self.i64, self.gep(ent, self.c64(SIG_PLEN)));
            let used = LLVMBuildAdd(self.b, off, self.c64(l), c"".as_ptr());
            self.guard(c, self.icmp(IP::LLVMIntULE, used, plen), codes_at);
            let first = self.load(self.i64, self.gep(ent, self.c64(SIG_OFF)));
            let bit0 = LLVMBuildAdd(self.b, first, off, c"".as_ptr());
            let sh = LLVMBuildAnd(self.b, bit0, self.c64(63), c"".as_ptr());
            let end = LLVMBuildAdd(self.b, sh, self.c64(l), c"".as_ptr());
            self.guard(c, self.icmp(IP::LLVMIntULE, end, self.c64(64)), codes_at);
            let w0 = LLVMBuildShl(
                self.b,
                LLVMBuildLShr(self.b, bit0, self.c64(6), c"".as_ptr()),
                self.c64(5),
                c"".as_ptr(),
            );
            let pd = self.host(
                c,
                self.ld(c, dst),
                self.known_addr(c, dst),
                self.c64(0),
                self.c64(32),
                true,
                slow,
            );
            for p in 0..4 {
                let at = LLVMBuildAdd(self.b, w0, self.c64(8 * p), c"".as_ptr());
                let w = self.load(self.i64, self.gep(blocks, at));
                let v = LLVMBuildAnd(
                    self.b,
                    LLVMBuildLShr(self.b, w, sh, c"".as_ptr()),
                    self.c64(mask as i64),
                    c"".as_ptr(),
                );
                self.store(v, self.gep(pd, self.c64(8 * p)));
            }
            LLVMBuildBr(self.b, join);
            LLVMPositionBuilderAtEnd(self.b, codes_at);
            // The elements are there, and so are the whole groups of eight codes that hold them
            // (the codes are kept in whole plane blocks; the planes are masked to the elements);
            // else the kernel's plane blocks.
            let alt = self.block(c);
            let tl = self.load(self.i64, self.gep(ent, self.c64(SIG_LEN)));
            let used = LLVMBuildAdd(self.b, off, self.c64(l), c"".as_ptr());
            self.guard(c, self.icmp(IP::LLVMIntULE, used, tl), alt);
            let cap = self.load(self.i64, self.gep(ent, self.c64(SIG_CAP)));
            let end = LLVMBuildAdd(self.b, off, self.c64(8 * groups as i64), c"".as_ptr());
            self.guard(c, self.icmp(IP::LLVMIntULE, end, cap), alt);
            let codes = self.tag(
                self.load(self.ptr, self.gep(ent, self.c64(SIG_CODES))),
                AA_CODES,
            );
            let base = self.gep(codes, off);
            let pd = self.host(
                c,
                self.ld(c, dst),
                self.known_addr(c, dst),
                self.c64(0),
                self.c64(32),
                true,
                slow,
            );
            // One call for the planes of the codes: unrolled, the loop over the groups of eight
            // codes was most of the code of a process that reads vectors (`yavs_rt_codes`).
            let mut args = [base, self.c64(groups as i64), self.c64(mask as i64), pd];
            LLVMBuildCall2(
                self.b,
                self.codes_ty,
                self.codes,
                args.as_mut_ptr(),
                4,
                c"".as_ptr(),
            );
            LLVMBuildBr(self.b, join);
            LLVMPositionBuilderAtEnd(self.b, alt);
            self.sig_read_planes(c, ent, off, self.c64(l), dst, slow, join);
            self.join_here(c, join);
        }
        true
    }

    /// `Inst::SigRead` of a plain reference whose count (`sig + 1`) is only known at run time, to
    /// a logic signal that keeps its codes: when the count is 1 to 64 and the codes are there,
    /// the planes come from `yavs_rt_codes`, a call that touches no host state; anything else
    /// through `TierHost::signal`.
    unsafe fn sig_read_dyn(&self, c: &Cur, pc: u32, dst: Reg, sig: Reg) {
        // SAFETY: see `sig_entry`; the guards check the groups lie within the codes and the
        // count is 1 to 64.
        unsafe {
            let (slow, join) = self.slow_signal(c, pc, 0);
            let l = self.ld(c, Reg(sig.0 + 1));
            let (ent, off) = self.sig_entry(c, sig, None, slow);
            // Up to 64 elements from the codes, else (or without the codes) from the planes.
            let alt = self.block(c);
            let l1 = LLVMBuildSub(self.b, l, self.c64(1), c"".as_ptr());
            self.guard(c, self.icmp(IP::LLVMIntULT, l1, self.c64(64)), alt);
            let tl = self.load(self.i64, self.gep(ent, self.c64(SIG_LEN)));
            let used = LLVMBuildAdd(self.b, off, l, c"".as_ptr());
            self.guard(c, self.icmp(IP::LLVMIntULE, used, tl), alt);
            let groups = LLVMBuildLShr(
                self.b,
                LLVMBuildAdd(self.b, l, self.c64(7), c"".as_ptr()),
                self.c64(3),
                c"".as_ptr(),
            );
            let end = LLVMBuildAdd(
                self.b,
                off,
                LLVMBuildShl(self.b, groups, self.c64(3), c"".as_ptr()),
                c"".as_ptr(),
            );
            let cap = self.load(self.i64, self.gep(ent, self.c64(SIG_CAP)));
            self.guard(c, self.icmp(IP::LLVMIntULE, end, cap), alt);
            let codes = self.tag(
                self.load(self.ptr, self.gep(ent, self.c64(SIG_CODES))),
                AA_CODES,
            );
            let base = self.gep(codes, off);
            // (1 << l) - 1, all ones for 64 (a shift by 64 is poison).
            let ones = self.c64(-1);
            let mask = LLVMBuildLShr(
                self.b,
                ones,
                LLVMBuildSub(self.b, self.c64(64), l, c"".as_ptr()),
                c"".as_ptr(),
            );
            let pd = self.host(
                c,
                self.ld(c, dst),
                self.known_addr(c, dst),
                self.c64(0),
                self.c64(32),
                true,
                slow,
            );
            let mut args = [base, groups, mask, pd];
            LLVMBuildCall2(
                self.b,
                self.codes_ty,
                self.codes,
                args.as_mut_ptr(),
                4,
                c"".as_ptr(),
            );
            LLVMBuildBr(self.b, join);
            LLVMPositionBuilderAtEnd(self.b, alt);
            self.sig_read_planes(c, ent, off, l, dst, slow, join);
            self.join_here(c, join);
        }
    }

    /// `Inst::SigAttr` `'event` of one element: its last event was in this cycle, from the
    /// kernel's event stamp (`TierSig::stamps`, offered for a signal of at most 64 elements);
    /// anything else through `TierHost::signal`.
    unsafe fn sig_event(&self, c: &Cur, pc: u32, dst: Reg, sig: Reg) {
        // SAFETY: see `sig_entry`; the stamp array has an entry for every kernel element.
        unsafe {
            let (slow, join) = self.slow_signal(c, pc, 0);
            let (ent, off) = self.sig_entry(c, sig, Some(1), slow);
            let stamps = self.load(self.ptr, self.gep(ent, self.c64(SIG_STAMPS)));
            self.guard(c, LLVMBuildIsNotNull(self.b, stamps, c"".as_ptr()), slow);
            let first = self.load(self.i64, self.gep(ent, self.c64(SIG_OFF)));
            let elem = LLVMBuildAdd(self.b, first, off, c"".as_ptr());
            let at = LLVMBuildMul(self.b, elem, self.c64(8), c"".as_ptr());
            let stamp = self.load(self.i64, self.gep(stamps, at));
            let cycle = self.mem_ld(c, MEM_CYCLE, self.i64);
            self.st(c, dst, self.zext(self.icmp(IP::LLVMIntEQ, stamp, cycle)));
            self.join(c, join);
        }
    }

    /// The clock-edge test `kind` of one element (`Inst::Edge`, or the `rising_edge` /
    /// `falling_edge` native, whose `TierHost::signal` entry is `edge` 1 or 2): true in place
    /// when the kernel resumed the process for that edge (`TierFrame::known`), else from the
    /// event stamp, the codes and, for the IEEE function, the `'last_value` planes; anything
    /// else through `TierHost::signal`.
    unsafe fn edge(&self, c: &Cur, pc: u32, kind: EdgeKind, sig: Reg, dst: Reg, edge: u32) {
        // SAFETY: see `sig_entry`; the stamps and the `'last_value` block are the kernel's, read
        // only here, and offered (nonzero) for a signal of at most 64 elements.
        unsafe {
            let (slow, join) = self.slow_signal(c, pc, edge);
            let (ent, off) = self.sig_entry(c, sig, Some(1), slow);
            let signal = self.load(self.i64, self.gep(ent, self.c64(SIG_SIGNAL)));
            let first = self.load(self.i64, self.gep(ent, self.c64(SIG_OFF)));
            let elem = LLVMBuildAdd(self.b, first, off, c"".as_ptr());
            let held = |a: LLVMValueRef, b: LLVMValueRef| self.icmp(IP::LLVMIntEQ, a, b);
            // `yavs_kernel::edge_key`.
            let key = self.or(
                self.or(
                    LLVMBuildShl(self.b, signal, self.c64(32), c"".as_ptr()),
                    LLVMBuildShl(self.b, elem, self.c64(2), c"".as_ptr()),
                ),
                self.c64(i64::from(kind.code())),
            );
            let ok = held(key, self.mem_ld(c, MEM_KNOWN, self.i64));
            // The kernel resumed the process for this very edge: it holds.
            let (fast, rest) = (self.block(c), self.block(c));
            self.cond_br(ok, fast, rest, true);
            LLVMPositionBuilderAtEnd(self.b, fast);
            self.st(c, dst, self.c64(1));
            LLVMBuildBr(self.b, join);
            LLVMPositionBuilderAtEnd(self.b, rest);
            // Else the answer is in the event stamp, the codes and the `'last_value` plane bits.
            let stamps = self.load(self.ptr, self.gep(ent, self.c64(SIG_STAMPS)));
            self.guard(c, LLVMBuildIsNotNull(self.b, stamps, c"".as_ptr()), slow);
            let last = self.load(self.ptr, self.gep(ent, self.c64(SIG_LAST)));
            if !kind.to_level() {
                self.guard(c, LLVMBuildIsNotNull(self.b, last, c"".as_ptr()), slow);
            }
            let len = self.load(self.i64, self.gep(ent, self.c64(SIG_LEN)));
            self.guard(c, self.icmp(IP::LLVMIntULT, off, len), slow);
            let at = LLVMBuildMul(self.b, elem, self.c64(8), c"".as_ptr());
            let stamp = self.load(self.i64, self.gep(stamps, at));
            let cycle = self.mem_ld(c, MEM_CYCLE, self.i64);
            let (ev, none, merge) = (self.block(c), self.block(c), self.block(c));
            LLVMBuildCondBr(self.b, held(stamp, cycle), ev, none);
            LLVMPositionBuilderAtEnd(self.b, none);
            LLVMBuildBr(self.b, merge);
            LLVMPositionBuilderAtEnd(self.b, ev);
            let codes = self.tag(
                self.load(self.ptr, self.gep(ent, self.c64(SIG_CODES))),
                AA_CODES,
            );
            let cur = self.zext8(self.load(self.i8, self.gep(codes, off)));
            let res = if kind.to_level() {
                // An event ending on the level: the code is exactly '1' (3) or '0' (2).
                self.zext(held(cur, self.c64(if kind.rising() { 3 } else { 2 })))
            } else {
                let plane = |k: i64| {
                    let w = self.load(self.i64, self.gep(last, self.c64(8 * k)));
                    let bit = LLVMBuildAnd(
                        self.b,
                        LLVMBuildLShr(self.b, w, elem, c"".as_ptr()),
                        self.c64(1),
                        c"".as_ptr(),
                    );
                    LLVMBuildShl(self.b, bit, self.c64(k), c"".as_ptr())
                };
                let before = self.or(plane(0), plane(1));
                // `To_X01` of a code is '1' or '0' when its low two bits are 1 x (3 and 7 are '1'
                // and 'H', 2 and 6 '0' and 'L'); the level is bit 0.
                let (now_want, before_want) = if kind.rising() { (3, 2) } else { (2, 3) };
                let m3 = |v: LLVMValueRef| LLVMBuildAnd(self.b, v, self.c64(3), c"".as_ptr());
                let a = held(m3(cur), self.c64(now_want));
                let b2 = held(m3(before), self.c64(before_want));
                self.zext(LLVMBuildAnd(self.b, a, b2, c"".as_ptr()))
            };
            let from_ev = LLVMGetInsertBlock(self.b);
            LLVMBuildBr(self.b, merge);
            LLVMPositionBuilderAtEnd(self.b, merge);
            let phi = LLVMBuildPhi(self.b, self.i64, c"".as_ptr());
            LLVMAddIncoming(phi, &mut self.c64(0), &mut { none }, 1);
            LLVMAddIncoming(phi, &mut { res }, &mut { from_ev }, 1);
            self.st(c, dst, phi);
            self.join(c, join);
        }
    }

    /// Execute the instruction `i` at `pc` in the interpreter; return what `yavs_rt_step`
    /// returns unless it is `GO`.
    unsafe fn step(&self, c: &Cur, pc: u32, i: &Inst) {
        if c.nat.is_some() {
            // SAFETY: as `deopt_here`.
            return unsafe { self.deopt_here(c, pc) };
        }
        // Instructions that only change the temporary area or a register go to the interpreter's own
        // function for them (`TierHost::signal`), without its generic dispatch.
        if matches!(
            i,
            Inst::TempRoot { .. }
                | Inst::NewFrame { .. }
                | Inst::Stmt
                | Inst::CoverCond { .. }
                | Inst::Deref { .. }
                | Inst::MemEq { .. }
                | Inst::Cat { .. }
        ) {
            // SAFETY: as `signal`.
            return unsafe { self.signal(c, pc, 0) };
        }
        let at = if keeps_objects(i) { pc | KEEP } else { pc };
        // SAFETY: `step` has the run-function signature.
        unsafe {
            self.call_go(
                c,
                self.fn_ty,
                self.step,
                &mut [c.env, c.regs, c.mem, self.c32(at)],
                quiet(i),
            )
        }
    }

    fn lcopy_shape(&self, c: &Cur, n: Reg, so: Reg, d: Reg) -> Option<LcopyShape> {
        lcopy_one_block(self.kint(c, n)?, self.kint(c, so)?, self.kint(c, d)?)
    }

    /// `Inst::LCopy` of a constant range inside one plane block on each side (`lcopy_one_block`).
    #[allow(clippy::too_many_arguments)]
    unsafe fn lcopy_const(
        &self,
        c: &Cur,
        dst: Reg,
        src: Reg,
        kd: Option<Known>,
        ks: Option<Known>,
        sh: LcopyShape,
        slow: LLVMBasicBlockRef,
    ) {
        // SAFETY: `host` checks 32 bytes at each block.
        unsafe {
            let ps = self.host(
                c,
                self.ld(c, src),
                ks,
                self.c64(sh.sb * 32),
                self.c64(32),
                false,
                slow,
            );
            let pd = self.host(
                c,
                self.ld(c, dst),
                kd,
                self.c64(sh.db * 32),
                self.c64(32),
                true,
                slow,
            );
            let b = self.b;
            let mask = if sh.k == 64 {
                u64::MAX
            } else {
                (1u64 << sh.k) - 1
            };
            let hole = mask << sh.d0;
            // The source planes first: the ranges may overlap.
            let mut v = [self.c64(0); 4];
            for (plane, x) in v.iter_mut().enumerate() {
                let w = self.load(self.i64, self.gep(ps, self.c64(plane as i64 * 8)));
                *x = if sh.s0 == sh.d0 {
                    LLVMBuildAnd(b, w, self.c64(hole as i64), c"".as_ptr())
                } else {
                    let r = LLVMBuildLShr(b, w, self.c64(i64::from(sh.s0)), c"".as_ptr());
                    let r = LLVMBuildAnd(b, r, self.c64(mask as i64), c"".as_ptr());
                    LLVMBuildShl(b, r, self.c64(i64::from(sh.d0)), c"".as_ptr())
                };
            }
            for (plane, x) in v.iter().enumerate() {
                let at = self.gep(pd, self.c64(plane as i64 * 8));
                let out = if hole == u64::MAX {
                    *x
                } else {
                    let keep = LLVMBuildAnd(
                        b,
                        self.load(self.i64, at),
                        self.c64(!hole as i64),
                        c"".as_ptr(),
                    );
                    LLVMBuildOr(b, keep, *x, c"".as_ptr())
                };
                self.store(out, at);
            }
        }
    }

    /// `Inst::LCopy` of `k` (1 to 64) elements from bit `s0` of the one or two plane blocks at
    /// `ps` (`ns` of them) to bit `d0` of those at `pd` (`nd`), as `copy_bits` does: the
    /// source planes are read first (the ranges may overlap), the destination bits outside
    /// the copied range stay.
    #[allow(clippy::too_many_arguments)]
    unsafe fn lcopy_inline(
        &self,
        c: &Cur,
        pd: LLVMValueRef,
        nd: LLVMValueRef,
        d0: LLVMValueRef,
        ps: LLVMValueRef,
        ns: LLVMValueRef,
        s0: LLVMValueRef,
        k: LLVMValueRef,
    ) {
        // SAFETY: the pointers cover `ns` and `nd` blocks (`host`, checked by the caller).
        unsafe {
            let i128t = LLVMIntTypeInContext(self.ctx, 128);
            let b = self.b;
            let z128 = |v| LLVMBuildZExt(b, v, i128t, c"".as_ptr());
            let c128 = |v: u64| LLVMConstInt(i128t, v, 0);
            // Low `k` bits.
            let mask = {
                let full = self.icmp(IP::LLVMIntUGE, k, self.c64(64));
                let sh = LLVMBuildShl(b, self.c64(1), k, c"".as_ptr());
                let low = LLVMBuildSub(b, sh, self.c64(1), c"".as_ptr());
                LLVMBuildSelect(b, full, self.c64(-1), low, c"".as_ptr())
            };
            let two_s = self.icmp(IP::LLVMIntEQ, ns, self.c64(2));
            let two_d = self.icmp(IP::LLVMIntEQ, nd, self.c64(2));
            // Block 1 is only read (written) when there is one.
            let word = |p: LLVMValueRef, plane: i64, blk1: LLVMValueRef| {
                let lo = self.gep(p, self.c64(plane * 8));
                let hi_at = LLVMBuildSelect(
                    b,
                    blk1,
                    self.gep(p, self.c64(32 + plane * 8)),
                    lo,
                    c"".as_ptr(),
                );
                (lo, hi_at)
            };
            let mut vals = [self.c64(0); 4];
            for (plane, v) in vals.iter_mut().enumerate() {
                let (lo, hi_at) = word(ps, plane as i64, two_s);
                let (l, h) = (self.load(self.i64, lo), self.load(self.i64, hi_at));
                let h = LLVMBuildSelect(b, two_s, h, self.c64(0), c"".as_ptr());
                let wide = LLVMBuildOr(
                    b,
                    z128(l),
                    LLVMBuildShl(b, z128(h), c128(64), c"".as_ptr()),
                    c"".as_ptr(),
                );
                let sh = LLVMBuildLShr(b, wide, z128(s0), c"".as_ptr());
                let low = LLVMBuildTrunc(b, sh, self.i64, c"".as_ptr());
                *v = LLVMBuildAnd(b, low, mask, c"".as_ptr());
            }
            let hole = LLVMBuildShl(b, z128(mask), z128(d0), c"".as_ptr());
            let keep = LLVMBuildNot(b, hole, c"".as_ptr());
            for (plane, v) in vals.iter().enumerate() {
                let (lo, hi_at) = word(pd, plane as i64, two_d);
                let (l, h) = (self.load(self.i64, lo), self.load(self.i64, hi_at));
                let h = LLVMBuildSelect(b, two_d, h, self.c64(0), c"".as_ptr());
                let wide = LLVMBuildOr(
                    b,
                    z128(l),
                    LLVMBuildShl(b, z128(h), c128(64), c"".as_ptr()),
                    c"".as_ptr(),
                );
                let put = LLVMBuildShl(b, z128(*v), z128(d0), c"".as_ptr());
                let out = LLVMBuildOr(
                    b,
                    LLVMBuildAnd(b, wide, keep, c"".as_ptr()),
                    put,
                    c"".as_ptr(),
                );
                self.store(LLVMBuildTrunc(b, out, self.i64, c"".as_ptr()), lo);
                // The second block: the high half, when the destination has one.
                let (hi_bb, done) = (self.block(c), self.block(c));
                LLVMBuildCondBr(b, two_d, hi_bb, done);
                LLVMPositionBuilderAtEnd(b, hi_bb);
                let top = LLVMBuildLShr(b, out, c128(64), c"".as_ptr());
                self.store(LLVMBuildTrunc(b, top, self.i64, c"".as_ptr()), hi_at);
                LLVMBuildBr(b, done);
                LLVMPositionBuilderAtEnd(b, done);
            }
        }
    }

    /// A `Drive` of one waveform element to a single-range target through `yavs_rt_drive`
    /// (`TierHost::drive_block`: at most 64 logic elements assigned from the element's first
    /// plane block); the instruction through `yavs_rt_signal` unless `n` is 1 and the 40 bytes
    /// of the waveform are in memory compiled code reaches.
    #[allow(clippy::too_many_arguments)]
    unsafe fn drive_block(
        &self,
        c: &Cur,
        pc: u32,
        driver: Result<u32, Reg>,
        target: Reg,
        wave: Reg,
        n: Reg,
        mode: yavs_ir::DriveMode,
    ) {
        // SAFETY: the arguments match `drive_ty`; well-formed blocks of c.f.
        unsafe {
            let (slow, join) = self.slow_signal(c, pc, 0);
            self.guard(
                c,
                self.icmp(IP::LLVMIntEQ, self.ld(c, n), self.c64(1)),
                slow,
            );
            // A slot register: the driver index of the process (a `DriveSlot`).
            let driver = match driver {
                Ok(d) => self.c32(d),
                Err(slot) => {
                    let v = self.ld(c, slot);
                    self.guard(
                        c,
                        self.icmp(IP::LLVMIntULE, v, self.c64(i64::from(u32::MAX))),
                        slow,
                    );
                    LLVMBuildTrunc(self.b, v, self.i32, c"".as_ptr())
                }
            };
            // A signal of enumeration, integer, boolean or real elements has a waveform element
            // of 16 bytes (time and cell), a logic one 40 (time and the first plane block).
            let (ent, _) = self.sig_entry(c, target, None, slow);
            let es = self.load(self.i64, self.gep(ent, self.c64(SIG_ES)));
            let bytes = self.icmp(IP::LLVMIntNE, es, self.c64(0));
            // A wider than 64 elements: the time and a block of planes per 64.
            let len = self.ld(c, Reg(target.0 + 1));
            let wide = self.icmp(IP::LLVMIntUGT, len, self.c64(64));
            self.guard(
                c,
                self.icmp(IP::LLVMIntULE, len, self.c64(i64::from(u32::MAX))),
                slow,
            );
            let blocks = LLVMBuildLShr(
                self.b,
                LLVMBuildAdd(self.b, len, self.c64(63), c"".as_ptr()),
                self.c64(6),
                c"".as_ptr(),
            );
            let wide_bytes = LLVMBuildAdd(
                self.b,
                LLVMBuildShl(self.b, blocks, self.c64(5), c"".as_ptr()),
                self.c64(8),
                c"".as_ptr(),
            );
            let logic = LLVMBuildSelect(self.b, wide, wide_bytes, self.c64(40), c"".as_ptr());
            let want = LLVMBuildSelect(self.b, bytes, self.c64(16), logic, c"".as_ptr());
            let w = self.host(
                c,
                self.ld(c, wave),
                self.known_addr(c, wave),
                self.c64(0),
                want,
                false,
                slow,
            );
            let inertial = LLVMBuildOr(
                self.b,
                self.c32(u32::from(mode == yavs_ir::DriveMode::Inertial)),
                LLVMBuildSelect(self.b, bytes, self.c32(2), self.c32(0), c"".as_ptr()),
                c"".as_ptr(),
            );
            let inertial = LLVMBuildOr(
                self.b,
                inertial,
                LLVMBuildSelect(
                    self.b,
                    LLVMBuildAnd(
                        self.b,
                        wide,
                        LLVMBuildNot(self.b, bytes, c"".as_ptr()),
                        c"".as_ptr(),
                    ),
                    self.c32(4),
                    self.c32(0),
                    c"".as_ptr(),
                ),
                c"".as_ptr(),
            );
            if c.nat.is_some() {
                // A native frame has no frame view: the kernel's answer or the materialised
                // frames, whose interpreter runs the `Drive` (and reports its error).
                let mut args = [
                    c.env,
                    driver,
                    self.ld(c, target),
                    self.ld(c, Reg(target.0 + 1)),
                    w,
                    inertial,
                ];
                let ok = LLVMBuildCall2(
                    self.b,
                    self.drive_nat_ty,
                    self.drive_nat,
                    args.as_mut_ptr(),
                    6,
                    c"".as_ptr(),
                );
                self.guard(c, self.icmp(IP::LLVMIntNE, ok, self.c32(0)), slow);
                self.join(c, join);
                return;
            }
            let mut args = [
                c.env,
                c.regs,
                c.mem,
                self.c32(pc),
                driver,
                self.ld(c, target),
                self.ld(c, Reg(target.0 + 1)),
                w,
                inertial,
            ];
            self.call_go(c, self.drive_ty, self.drive, &mut args, false);
            self.join(c, join);
        }
    }

    /// The `Drive` of a scalar logic assignment the interpreter fuses (`yavs_interp::ScalarDrive`):
    /// the waveform is never made, the delay and the code go to the kernel in registers. With
    /// no delay (`batch`) it is kept in `Batch::entries` when it is a plain logic element of a
    /// signal the kernel offers (`TierSig::codes` or `planes`), else it goes at once, after the
    /// ones kept before it.
    unsafe fn drive_bit(&self, c: &Cur, pc: u32, s: &yavs_interp::ScalarDrive, inst: &Inst) {
        // SAFETY: the arguments match `drive_bit_ty`; well-formed blocks of c.f.
        unsafe {
            let Some(b) = c.batch.filter(|_| self.kint(c, s.time) == Some(0)) else {
                return self.drive_bit_now(c, pc, s);
            };
            let (slow, join) = self.slow_path_with(c, pc, inst, true, |g, _| {
                g.batch_flush(c);
                g.drive_bit_now(c, pc, s);
            });
            let (v, l) = (self.ld(c, s.target), self.ld(c, Reg(s.target.0 + 1)));
            let base = LLVMBuildLShr(self.b, v, self.c64(32), c"".as_ptr());
            let n = self.mem_ld(c, MEM_SIGTAB_LEN, self.i64);
            let code = self.ld(c, s.value);
            let ok = LLVMBuildAnd(
                self.b,
                self.icmp(IP::LLVMIntULT, base, n),
                LLVMBuildAnd(
                    self.b,
                    self.icmp(IP::LLVMIntEQ, l, self.c64(1)),
                    self.icmp(IP::LLVMIntULE, code, self.c64(8)),
                    c"".as_ptr(),
                ),
                c"".as_ptr(),
            );
            self.guard(c, ok, slow);
            let tab = self.mem_ld(c, MEM_SIGTAB, self.ptr);
            let at = LLVMBuildMul(self.b, base, self.c64(SIG_ENT), c"".as_ptr());
            let ent = self.gep(tab, at);
            let logic = LLVMBuildOr(
                self.b,
                self.load(self.i64, self.gep(ent, self.c64(SIG_CODES))),
                self.load(self.i64, self.gep(ent, self.c64(SIG_PLANES))),
                c"".as_ptr(),
            );
            let n = self.load(self.i32, b.count);
            let room = self.icmp(IP::LLVMIntULT, n, self.c32(yavs_interp::DRIVE_BATCH as u32));
            let fits = LLVMBuildAnd(
                self.b,
                self.icmp(IP::LLVMIntNE, logic, self.c64(0)),
                room,
                c"".as_ptr(),
            );
            self.guard(c, fits, slow);
            let at = LLVMBuildMul(
                self.b,
                LLVMBuildZExt(self.b, n, self.i64, c"".as_ptr()),
                self.c64(16),
                c"".as_ptr(),
            );
            let p = self.gep(b.entries, at);
            self.store(v, p);
            let info = u64::from(s.driver) | u64::from(s.inertial) << 24 | u64::from(pc) << 32;
            let w = LLVMBuildOr(
                self.b,
                LLVMBuildShl(self.b, code, self.c64(16), c"".as_ptr()),
                self.c64(info as i64),
                c"".as_ptr(),
            );
            self.store(w, self.gep(p, self.c64(8)));
            LLVMBuildStore(
                self.b,
                LLVMBuildAdd(self.b, n, self.c32(1), c"".as_ptr()),
                b.count,
            );
            self.join(c, join);
        }
    }

    /// The scalar logic assignment of `s` as one callback.
    unsafe fn drive_bit_now(&self, c: &Cur, pc: u32, s: &yavs_interp::ScalarDrive) {
        // SAFETY: the arguments match `drive_bit_ty`; well-formed blocks of c.f.
        unsafe {
            let code = LLVMBuildOr(
                self.b,
                self.ld(c, s.value),
                self.c64(if s.inertial { 0x100 } else { 0 }),
                c"".as_ptr(),
            );
            let mut args = [
                c.env,
                c.regs,
                c.mem,
                self.c32(pc),
                self.c32(s.driver),
                self.ld(c, s.target),
                self.ld(c, Reg(s.target.0 + 1)),
                self.ld(c, s.time),
                code,
            ];
            self.call_go(c, self.drive_bit_ty, self.drive_bit, &mut args, true);
        }
    }

    /// `Inst::Cat` of a logic array of at most 64 elements whose piece lengths the generator
    /// knows: the planes of all pieces combined in registers and stored once, as
    /// `Machine::cat_block` does; `false` (nothing generated) where a length is not known or
    /// does not fit that pattern. A runtime case it does not take (a code beyond 'U' to '-',
    /// memory compiled code does not reach) steps the whole instruction before any store.
    #[allow(clippy::too_many_arguments)]
    unsafe fn cat_block(
        &self,
        c: &Cur,
        pc: u32,
        inst: &Inst,
        dst: Reg,
        n: Reg,
        m: Reg,
        parts: &[Reg],
        scalars: u16,
        bias: u8,
        at: Option<Known>,
    ) -> bool {
        let (Some(n), Some(m)) = (self.kint(c, n), self.kint(c, m)) else {
            return false;
        };
        let Some(total) = u64::try_from(n)
            .ok()
            .zip(u64::try_from(m).ok())
            .and_then(|(n, m)| n.checked_mul(m))
            .filter(|&t| t <= 64)
        else {
            return false;
        };
        // (register, units, scalar) per piece, empty ones dropped, as `cat_block` skips them.
        let mut pieces = Vec::new();
        let mut k = 0u64;
        for (j, pr) in parts.as_chunks::<2>().0.iter().enumerate() {
            if scalars >> j & 1 == 1 {
                pieces.push((pr[0], 1u64, true));
                k += 1;
            } else {
                let Some(len) = self.kint(c, pr[1]).and_then(|l| u64::try_from(l).ok()) else {
                    return false;
                };
                let Some(units) = len.checked_mul(m as u64).filter(|&u| u <= 64) else {
                    return false;
                };
                if units == 0 {
                    continue;
                }
                pieces.push((pr[0], units, false));
                k += units;
            }
            if k > total {
                return false;
            }
        }
        // SAFETY: loads through `host`, integer arithmetic, well-formed blocks of c.f.
        unsafe {
            let (slow, join) = self.slow_path(c, pc, inst);
            let mut words = [self.c64(0); 4];
            let mut mask = 0u64;
            let mut at_k = 0u64;
            for &(r, units, scalar) in &pieces {
                let bits: [LLVMValueRef; 4] = if scalar {
                    let code = LLVMBuildAdd(
                        self.b,
                        self.ld(c, r),
                        self.c64(i64::from(bias)),
                        c"".as_ptr(),
                    );
                    // A code beyond 'U' to '-' is the interpreter's error.
                    self.guard(c, self.icmp(IP::LLVMIntULE, code, self.c64(8)), slow);
                    std::array::from_fn(|p| {
                        let b = LLVMBuildLShr(self.b, code, self.c64(p as i64), c"".as_ptr());
                        LLVMBuildAnd(self.b, b, self.c64(1), c"".as_ptr())
                    })
                } else {
                    let ptr = self.host(
                        c,
                        self.ld(c, r),
                        self.known_addr(c, r),
                        self.c64(0),
                        self.c64(32),
                        false,
                        slow,
                    );
                    std::array::from_fn(|p| {
                        let w = self.load(self.i64, self.gep(ptr, self.c64(p as i64 * 8)));
                        let pm = if units == 64 {
                            !0u64
                        } else {
                            (1u64 << units) - 1
                        };
                        LLVMBuildAnd(self.b, w, self.c64(pm as i64), c"".as_ptr())
                    })
                };
                let off = total - at_k - units;
                let pm = if units == 64 {
                    !0u64
                } else {
                    (1u64 << units) - 1
                };
                for p in 0..4 {
                    let sh = LLVMBuildShl(self.b, bits[p], self.c64(off as i64), c"".as_ptr());
                    words[p] = LLVMBuildOr(self.b, words[p], sh, c"".as_ptr());
                }
                mask |= pm << off;
                at_k += units;
            }
            if mask == 0 {
                self.join(c, join);
                return true;
            }
            let d = self.ld(c, dst);
            let pd = self.host(c, d, at, self.c64(0), self.c64(32), true, slow);
            // A block just allocated (zeroed) and covered by the pieces needs no read.
            let fresh = matches!(at, Some(Known::Temp)) && at_k == total;
            for (p, &word) in words.iter().enumerate() {
                let pp = self.gep(pd, self.c64(p as i64 * 8));
                let v = if fresh {
                    word
                } else {
                    let cur = self.load(self.i64, pp);
                    let keep = LLVMBuildAnd(self.b, cur, self.c64(!mask as i64), c"".as_ptr());
                    LLVMBuildOr(self.b, keep, word, c"".as_ptr())
                };
                self.store(v, pp);
            }
            self.join(c, join);
        }
        true
    }

    /// A `WaitOn` and the `Suspend` after it through `yavs_rt_wait` (`TierHost::wait_suspend`).
    unsafe fn wait_suspend(&self, c: &Cur, pc: u32, sig: Reg) {
        // SAFETY: the arguments match `wait_ty`.
        unsafe {
            let mut args = [
                c.env,
                c.regs,
                c.mem,
                self.c32(pc),
                self.ld(c, sig),
                self.ld(c, Reg(sig.0 + 1)),
            ];
            self.call_go(c, self.wait_ty, self.wait, &mut args, false);
        }
    }

    /// The signal instruction at `pc` through `yavs_rt_signal` (`edge`: `TierHost::signal`).
    unsafe fn signal(&self, c: &Cur, pc: u32, edge: u32) {
        if c.nat.is_some() {
            // SAFETY: as `deopt_here`.
            return unsafe { self.deopt_here(c, pc) };
        }
        let mut args = [c.env, c.regs, c.mem, self.c32(pc), self.c32(edge)];
        // SAFETY: the arguments match `signal_ty`.
        unsafe { self.call_go(c, self.signal_ty, self.signal, &mut args, false) }
    }

    /// `Inst::Call` of `callee` (`g`) on a native frame (`docs/decisions.md` "Compiled frames"):
    /// one frame index deeper, within the depth limit and the native stack allowance, when it
    /// has native code; the call is stepped (frame mode) or the frame materialised otherwise.
    /// When the callee unwinds for materialisation, a native caller records itself at the call,
    /// a frame-mode caller pushes the recorded frames and leaves them to the interpreter.
    #[allow(clippy::too_many_arguments)]
    unsafe fn native_call(
        &self,
        c: &Cur,
        pc: u32,
        i: &Inst,
        caller: FuncId,
        f: &Func,
        callee: FuncId,
        g: &Func,
        args: &[Reg],
        dst: Option<Reg>,
        obj: Option<Reg>,
        memo: bool,
    ) {
        // SAFETY: `Mem` fields, the callee's native type, well-formed blocks of c.f.
        unsafe {
            let (slow, join) = self.slow_path(c, pc, i);
            // A protected call: the process must be allowed the shared state (else the
            // interpreter's error), and the callee allocates in the shared heap when the object
            // is there (`Machine::call_prot`).
            let flags = self.mem_ld(c, MEM_FLAGS, self.i64);
            if let Some(o) = obj {
                let serial = LLVMBuildAnd(
                    self.b,
                    flags,
                    self.c64(yavs_interp::TIER_SERIAL as i64),
                    c"".as_ptr(),
                );
                self.guard(c, self.icmp(IP::LLVMIntNE, serial, self.c64(0)), slow);
                let tag = LLVMBuildLShr(self.b, self.ld(c, o), self.c64(56), c"".as_ptr());
                let shared = self.icmp(
                    IP::LLVMIntEQ,
                    tag,
                    self.c64(i64::from(yavs_interp::SHARED_TAG)),
                );
                let inner = LLVMBuildOr(self.b, flags, self.zext(shared), c"".as_ptr());
                self.mem_st(c, MEM_FLAGS, inner);
            }
            let depth = match &c.nat {
                Some(n) => n.depth,
                None => self.mem_ld(c, MEM_FRAME_IDX, self.i64),
            };
            let d1 = LLVMBuildAdd(self.b, depth, self.c64(1), c"".as_ptr());
            let max = self.mem_ld(c, MEM_MAX_DEPTH, self.i64);
            let and = |a, b| LLVMBuildAnd(self.b, a, b, c"".as_ptr());
            let mut ok = and(
                self.icmp(IP::LLVMIntULT, d1, max),
                self.icmp(IP::LLVMIntULT, d1, self.c64(1 << 29)),
            );
            let mut zero = [self.c32(0)];
            let fa = LLVMBuildCall2(
                self.b,
                self.frame_addr.0,
                self.frame_addr.1,
                zero.as_mut_ptr(),
                1,
                c"".as_ptr(),
            );
            let fa = LLVMBuildPtrToInt(self.b, fa, self.i64, c"".as_ptr());
            let lo = self.mem_ld(c, MEM_STACK_LO, self.i64);
            ok = and(ok, self.icmp(IP::LLVMIntUGT, fa, lo));
            let ty = self.native_ty(g);
            let fp = if self.direct {
                self.declare(&format!("f{}_n", callee.0), ty)
            } else {
                let tab = self.mem_ld(c, MEM_NTAB, self.ptr);
                let at = self.gep(tab, self.c64(i64::from(callee.0) * 8));
                let fp = LLVMBuildLoad2(self.b, self.ptr, at, c"".as_ptr());
                LLVMSetOrdering(
                    fp,
                    llvm_sys::LLVMAtomicOrdering::LLVMAtomicOrderingMonotonic,
                );
                ok = and(ok, LLVMBuildIsNotNull(self.b, fp, c"".as_ptr()));
                fp
            };
            self.guard(c, ok, slow);
            // The callee collects nothing and cannot see the slot variables; the registers go to
            // the frame for the way out below.
            self.flush_regs(c, pc);
            let mut cells = vec![c.env, c.mem, d1];
            if let Some(p) = g.parent {
                // The enclosing frames: the caller's own frame when it encloses the callee, else
                // those its own static links name.
                let up = |k: u64| match &c.nat {
                    Some(n) => n.up[k as usize],
                    None => self.mem_ld(c, MEM_UP + 8 * k, self.i64),
                };
                let (u1, u2) = if p == caller {
                    (self.own_hi(c), up(0))
                } else {
                    (up(0), up(1))
                };
                debug_assert!(f.parent == Some(p) || p == caller);
                let known = self.icmp(IP::LLVMIntNE, u1, self.c64(-1));
                self.guard(c, known, slow);
                cells.extend([u1, u2]);
            }
            if let Some(o) = obj {
                cells.push(self.ld(c, o));
            }
            for (r, p) in args
                .iter()
                .zip(g.params.iter().skip(usize::from(obj.is_some())))
            {
                cells.push(self.ld(c, *r));
                if p.width == Width::Sig {
                    cells.push(self.ld(c, Reg(r.0 + 1)));
                }
            }
            // The memo of pure calls (`TierHost::memo_probe`): a hit is the result, a miss makes
            // the call and keeps its result below.
            let memo = if memo {
                let n = args.len();
                let (buf, out) = (self.entry_alloca(c, n as u64 * 8), self.entry_alloca(c, 16));
                for (k, r) in args.iter().enumerate() {
                    self.store(self.ld(c, *r), self.gep(buf, self.c64(k as i64 * 8)));
                }
                let dep = match &c.nat {
                    Some(n) => n.depth,
                    None => self.c64(NATIVE_FRAME_MODE as i64),
                };
                let mut a = [
                    c.env,
                    c.mem,
                    self.c32(callee.0),
                    buf,
                    self.c32(n as u32),
                    dep,
                    out,
                ];
                let st = LLVMBuildCall2(
                    self.b,
                    self.native_ty,
                    self.memo,
                    a.as_mut_ptr(),
                    7,
                    c"".as_ptr(),
                );
                let (hit, go, merge) = (self.block(c), self.block(c), self.block(c));
                let is_hit = self.icmp(IP::LLVMIntEQ, st, self.c32(1));
                self.cond_br(is_hit, hit, go, true);
                LLVMPositionBuilderAtEnd(self.b, hit);
                let hv = LLVMBuildLoad2(self.b, self.i64, out, c"".as_ptr());
                LLVMBuildBr(self.b, merge);
                LLVMPositionBuilderAtEnd(self.b, go);
                Some((st, buf, out, dep, n, hit, hv, merge))
            } else {
                None
            };
            let v = LLVMBuildCall2(
                self.b,
                ty,
                fp,
                cells.as_mut_ptr(),
                cells.len() as u32,
                c"".as_ptr(),
            );
            if obj.is_some() {
                self.mem_st(c, MEM_FLAGS, flags);
            }
            let nd = self.mem_ld(c, MEM_NDEOPT, self.i64);
            let (out, cont) = (self.block(c), self.block(c));
            self.cond_br(self.icmp(IP::LLVMIntNE, nd, self.c64(0)), out, cont, false);
            LLVMPositionBuilderAtEnd(self.b, out);
            match &c.nat {
                Some(_) => self.leave(c, self.c32(pc)),
                None => {
                    self.flush(c, pc as usize);
                    let mut a = [c.env, c.mem, self.c32(pc)];
                    let r = LLVMBuildCall2(
                        self.b,
                        self.deopt_end_ty,
                        self.deopt_end,
                        a.as_mut_ptr(),
                        3,
                        c"".as_ptr(),
                    );
                    self.leave(c, r);
                }
            }
            LLVMPositionBuilderAtEnd(self.b, cont);
            let v = match memo {
                Some((st, buf, out, dep, n, hit, hv, merge)) => {
                    let (keep, after) = (self.block(c), self.block(c));
                    let miss = self.icmp(IP::LLVMIntEQ, st, self.c32(2));
                    self.cond_br(miss, keep, after, false);
                    LLVMPositionBuilderAtEnd(self.b, keep);
                    self.store(v, out);
                    let mut a = [
                        c.env,
                        c.mem,
                        self.c32(callee.0),
                        buf,
                        self.c32(n as u32),
                        dep,
                        out,
                    ];
                    LLVMBuildCall2(
                        self.b,
                        self.native_ty,
                        self.memo_keep,
                        a.as_mut_ptr(),
                        7,
                        c"".as_ptr(),
                    );
                    LLVMBuildBr(self.b, after);
                    LLVMPositionBuilderAtEnd(self.b, after);
                    LLVMBuildBr(self.b, merge);
                    LLVMPositionBuilderAtEnd(self.b, merge);
                    let phi = LLVMBuildPhi(self.b, self.i64, c"".as_ptr());
                    let mut vals = [v, hv];
                    let mut from = [after, hit];
                    LLVMAddIncoming(phi, vals.as_mut_ptr(), from.as_mut_ptr(), 2);
                    phi
                }
                None => v,
            };
            if let Some(d) = dst {
                self.st(c, d, v);
            }
            self.join(c, join);
        }
    }

    /// `n` bytes of stack (8-aligned) allocated in the entry block, so a loop does not grow
    /// the stack; the builder stays where it is.
    unsafe fn entry_alloca(&self, c: &Cur, bytes: u64) -> LLVMValueRef {
        // SAFETY: the entry block of c.f exists; the builder returns to its block.
        unsafe {
            let here = LLVMGetInsertBlock(self.b);
            let entry = LLVMGetEntryBasicBlock(c.f);
            match LLVMGetFirstInstruction(entry) {
                first if first.is_null() => LLVMPositionBuilderAtEnd(self.b, entry),
                first => LLVMPositionBuilderBefore(self.b, first),
            }
            let ty = LLVMArrayType2(self.i64, bytes.div_ceil(8).max(1));
            let a = LLVMBuildAlloca(self.b, ty, c"".as_ptr());
            LLVMPositionBuilderAtEnd(self.b, here);
            a
        }
    }

    /// `yavs_rt_troot(addr, ty, desc, n)`: 1 when recorded.
    unsafe fn troot_call(
        &self,
        c: &Cur,
        addr: LLVMValueRef,
        ty: u32,
        desc: LLVMValueRef,
        n: u64,
    ) -> LLVMValueRef {
        // SAFETY: the arguments match `troot_ty`.
        unsafe {
            let mut a = [c.env, c.mem, addr, self.c32(ty), desc, self.c64(n as i64)];
            LLVMBuildCall2(
                self.b,
                self.troot_ty,
                self.troot,
                a.as_mut_ptr(),
                6,
                c"".as_ptr(),
            )
        }
    }

    /// `n` copies of the `size` bytes at `src` from `dst` on (`Inst::Fill`, the pattern of a
    /// `NewFrame`); `src` must not overlap `dst`.
    unsafe fn fill_loop(
        &self,
        c: &Cur,
        dst: LLVMValueRef,
        src: LLVMValueRef,
        n: LLVMValueRef,
        size: u32,
    ) {
        // SAFETY: well-formed blocks of c.f; `dst` has room for `n * size` bytes.
        unsafe {
            if size == 1 {
                let b = self.load(self.i8, src);
                LLVMBuildMemSet(self.b, dst, b, n, 1);
                return;
            }
            let (head, body, done) = (self.block(c), self.block(c), self.block(c));
            let pre = LLVMGetInsertBlock(self.b);
            LLVMBuildBr(self.b, head);
            LLVMPositionBuilderAtEnd(self.b, head);
            let i = LLVMBuildPhi(self.b, self.i64, c"".as_ptr());
            LLVMBuildCondBr(self.b, self.icmp(IP::LLVMIntULT, i, n), body, done);
            LLVMPositionBuilderAtEnd(self.b, body);
            let at = self.gep(
                dst,
                LLVMBuildMul(self.b, i, self.c64(i64::from(size)), c"".as_ptr()),
            );
            LLVMBuildMemCpy(self.b, at, 1, src, 1, self.c64(i64::from(size)));
            let next = LLVMBuildAdd(self.b, i, self.c64(1), c"".as_ptr());
            LLVMBuildBr(self.b, head);
            let mut vals = [self.c64(0), next];
            let mut from = [pre, body];
            LLVMAddIncoming(i, vals.as_mut_ptr(), from.as_mut_ptr(), 2);
            LLVMPositionBuilderAtEnd(self.b, done);
        }
    }

    /// `Inst::NewFrame` on a native frame: a frame-pinned temporary of `dsize` descriptor bytes
    /// and `size` data bytes (copied from `data`, or the `fill` bytes at `data` repeated), and
    /// the frame's statements then keep it (`temp_base`).
    #[allow(clippy::too_many_arguments)]
    unsafe fn new_frame(
        &self,
        c: &Cur,
        pc: u32,
        i: &Inst,
        dst: Reg,
        desc: Reg,
        dsize: u32,
        data: Reg,
        size: Reg,
        fill: u32,
        fk: &dyn Fn(Reg) -> Option<Known>,
    ) {
        // SAFETY: integer arithmetic on registers, `host` pointers, `Mem` fields.
        unsafe {
            let (slow, join) = self.slow_path(c, pc, i);
            let sz = self.ld(c, size);
            // Over 4 GiB is the interpreter's error; a size that is no multiple of the
            // element is its internal error.
            let mut ok = self.icmp(IP::LLVMIntULE, sz, self.c64(1 << 32));
            if fill != 0 {
                let r = LLVMBuildURem(self.b, sz, self.c64(i64::from(fill)), c"".as_ptr());
                ok = LLVMBuildAnd(
                    self.b,
                    ok,
                    self.icmp(IP::LLVMIntEQ, r, self.c64(0)),
                    c"".as_ptr(),
                );
            }
            self.guard(c, ok, slow);
            // The sources before the allocation: a failure leaves nothing changed.
            let ds = self.c64(i64::from(dsize));
            let pdesc = (dsize != 0)
                .then(|| self.host(c, self.ld(c, desc), fk(desc), self.c64(0), ds, false, slow));
            let n_src = if fill == 0 {
                sz
            } else {
                self.c64(i64::from(fill))
            };
            let psrc = self.host(
                c,
                self.ld(c, data),
                fk(data),
                self.c64(0),
                n_src,
                false,
                slow,
            );
            let total = LLVMBuildAdd(self.b, ds, sz, c"".as_ptr());
            let (at, p) = self.temp_alloc(c, total, slow);
            if let Some(pd) = pdesc {
                LLVMBuildMemCpy(self.b, p, 1, pd, 1, ds);
            }
            let val = self.gep(p, ds);
            if fill == 0 {
                LLVMBuildMemCpy(self.b, val, 1, psrc, 1, sz);
            } else {
                let count = LLVMBuildUDiv(self.b, sz, self.c64(i64::from(fill)), c"".as_ptr());
                self.fill_loop(c, val, psrc, count, fill);
            }
            // The frame's own statements must not free the object.
            if let Some(n) = &c.nat {
                let end = self.mem_ld(c, MEM_TEMP_LEN, self.i64);
                LLVMBuildStore(self.b, end, n.temp_base);
            }
            self.st(c, dst, at);
            self.join(c, join);
        }
    }

    /// `Inst::Fill` on a native frame: `n` copies of the `size` bytes at `elem` from `dst` on.
    #[allow(clippy::too_many_arguments)]
    unsafe fn fill(
        &self,
        c: &Cur,
        pc: u32,
        i: &Inst,
        dst: Reg,
        elem: Reg,
        n: Reg,
        size: u32,
        fk: &dyn Fn(Reg) -> Option<Known>,
    ) {
        // SAFETY: integer arithmetic on registers and `host` pointers.
        unsafe {
            let (slow, join) = self.slow_path(c, pc, i);
            let cnt = self.ld(c, n);
            let (total, o) = self.ovf(self.umul, cnt, self.c64(i64::from(size)));
            self.guard(c, LLVMBuildNot(self.b, o, c"".as_ptr()), slow);
            let sz = self.c64(i64::from(size));
            let pe = self.host(c, self.ld(c, elem), fk(elem), self.c64(0), sz, false, slow);
            let pd = self.host(c, self.ld(c, dst), fk(dst), self.c64(0), total, true, slow);
            // The element is read before anything is written (it may lie in the target).
            let src = if size == 1 {
                pe
            } else {
                let stage = self.entry_alloca(c, u64::from(size));
                LLVMBuildMemCpy(self.b, stage, 1, pe, 1, sz);
                stage
            };
            self.fill_loop(c, pd, src, cnt, size);
            self.join(c, join);
        }
    }

    /// `Inst::Cat` of non-logic elements on a native frame: the pieces (`rs`: pairs of an
    /// address and a count, or an element value for a scalar piece) one after another from
    /// `dst` on, `elem` bytes per element. Every address is found before anything is written.
    #[allow(clippy::too_many_arguments)]
    unsafe fn cat_bytes(
        &self,
        c: &Cur,
        pc: u32,
        i: &Inst,
        dst: Reg,
        elem: Reg,
        rs: &[Reg],
        scalars: u16,
        w: Width,
        fk: &dyn Fn(Reg) -> Option<Known>,
    ) {
        // SAFETY: integer arithmetic on registers and `host` pointers.
        unsafe {
            enum Piece {
                Scalar(LLVMValueRef, LLVMValueRef),
                Bytes(LLVMValueRef, LLVMValueRef, LLVMValueRef),
            }
            let (slow, join) = self.slow_path(c, pc, i);
            let m = self.ld(c, elem);
            let d = self.ld(c, dst);
            let mut k = self.c64(0);
            let mut pieces = Vec::new();
            for (j, pr) in rs.as_chunks::<2>().0.iter().enumerate() {
                let (off, o) = self.ovf(self.umul, k, m);
                let small = self.icmp(IP::LLVMIntULE, off, self.c64(0xffff_ffff));
                self.guard(
                    c,
                    LLVMBuildAnd(
                        self.b,
                        LLVMBuildNot(self.b, o, c"".as_ptr()),
                        small,
                        c"".as_ptr(),
                    ),
                    slow,
                );
                if scalars >> j & 1 == 1 {
                    let nb = self.c64(i64::from(w.bytes()));
                    let pd = self.host(c, d, fk(dst), off, nb, true, slow);
                    pieces.push(Piece::Scalar(pd, self.ld(c, pr[0])));
                    k = LLVMBuildAdd(self.b, k, self.c64(1), c"".as_ptr());
                } else {
                    let len = self.ld(c, pr[1]);
                    let (nbytes, o) = self.ovf(self.umul, len, m);
                    self.guard(c, LLVMBuildNot(self.b, o, c"".as_ptr()), slow);
                    let ps = self.host(
                        c,
                        self.ld(c, pr[0]),
                        fk(pr[0]),
                        self.c64(0),
                        nbytes,
                        false,
                        slow,
                    );
                    let pd = self.host(c, d, fk(dst), off, nbytes, true, slow);
                    pieces.push(Piece::Bytes(pd, ps, nbytes));
                    k = LLVMBuildAdd(self.b, k, len, c"".as_ptr());
                }
            }
            for p in pieces {
                match p {
                    Piece::Scalar(pd, v) => {
                        let v = if w == Width::B8 {
                            v
                        } else {
                            LLVMBuildTrunc(self.b, v, self.width_ty(w), c"".as_ptr())
                        };
                        self.store(v, pd);
                    }
                    Piece::Bytes(pd, ps, n) => {
                        LLVMBuildMemMove(self.b, pd, 1, ps, 1, n);
                    }
                }
            }
            self.join(c, join);
        }
    }

    /// `Inst::CallNative` of a native `yavs_interp::direct_native` allows: the argument cells
    /// go to `TierHost::native_direct` (a native frame; where it does not run the native the
    /// frames are materialised and the interpreter executes the call) or `native_frame` (a
    /// frame-mode function: the interpreter steps the call; no register is flushed or reloaded
    /// on the way).
    #[allow(clippy::too_many_arguments)]
    unsafe fn native_direct(
        &self,
        c: &Cur,
        pc: u32,
        i: &Inst,
        f: &Func,
        native: yavs_ir::NativeId,
        args: yavs_ir::Regs,
        dst: Option<Reg>,
        light: bool,
    ) -> Result<(), String> {
        // SAFETY: the arguments match `native_ty`; well-formed blocks of c.f.
        unsafe {
            let rs = f.regs(args).ok_or("operand list out of range")?;
            let mut cells = Vec::new();
            for &r in rs {
                cells.push(r);
                if f.reg_ty.get(r.0 as usize) == Some(&yavs_ir::RegTy::Sig) {
                    cells.push(Reg(r.0 + 1));
                }
            }
            let (slow, join) = self.slow_path(c, pc, i);
            let buf = self.entry_alloca(c, cells.len() as u64 * 8);
            for (k, &r) in cells.iter().enumerate() {
                self.store(self.ld(c, r), self.gep(buf, self.c64(k as i64 * 8)));
            }
            let (depth, scratch) = match &c.nat {
                Some(nat) => (nat.depth, nat.scratch),
                None => (
                    self.c64((NATIVE_FRAME_MODE | u64::from(pc)) as i64),
                    self.entry_alloca(c, 8),
                ),
            };
            let mut a = [
                c.env,
                c.mem,
                self.c32(native.0),
                buf,
                self.c32(cells.len() as u32),
                depth,
                scratch,
            ];
            let st = LLVMBuildCall2(
                self.b,
                self.native_ty,
                if light {
                    self.native_light
                } else {
                    self.native
                },
                a.as_mut_ptr(),
                7,
                c"".as_ptr(),
            );
            let kind = LLVMBuildAnd(self.b, st, self.c32(0xff), c"".as_ptr());
            let mut ok = self.icmp(IP::LLVMIntNE, kind, self.c32(0));
            if dst.is_some() {
                let has = LLVMBuildAnd(self.b, st, self.c32(0x100), c"".as_ptr());
                ok = LLVMBuildAnd(
                    self.b,
                    ok,
                    self.icmp(IP::LLVMIntNE, has, self.c32(0)),
                    c"".as_ptr(),
                );
            }
            self.guard(c, ok, slow);
            if let Some(d) = dst {
                let v = LLVMBuildLoad2(self.b, self.i64, scratch, c"".as_ptr());
                self.st(c, d, v);
            }
            // The native ended the process: a native frame is materialised after the call, a
            // frame-mode function suspends with the registers in the frame.
            let (out, cont) = (self.block(c), self.block(c));
            LLVMBuildCondBr(
                self.b,
                self.icmp(IP::LLVMIntEQ, kind, self.c32(2)),
                out,
                cont,
            );
            LLVMPositionBuilderAtEnd(self.b, out);
            if c.nat.is_some() {
                self.leave(c, self.c32(pc + 1));
            } else {
                self.flush(c, pc as usize + 1);
                self.leave(c, self.c32(TIER_SUSPEND));
                c.cold.borrow_mut().push(out);
            }
            LLVMPositionBuilderAtEnd(self.b, cont);
            self.join(c, join);
            Ok(())
        }
    }

    /// `math_real.uniform` computed in place: both seeds read and the generator's step made
    /// in integers (`yavs_interp::ieee::uniform_step`), the new seeds and the real written
    /// back. A seed out of range, which the native reports, or an address `host` does not
    /// find takes `slow`: the native, which has changed nothing by then.
    #[allow(clippy::too_many_arguments)]
    unsafe fn uniform_fast(
        &self,
        c: &Cur,
        pc: u32,
        i: &Inst,
        f: &Func,
        native: yavs_ir::NativeId,
        args: yavs_ir::Regs,
        v: [Reg; 3],
    ) -> Result<(), String> {
        // SAFETY: integer arithmetic, reads and writes through `host`; well-formed blocks of c.f.
        unsafe {
            let mut called = Ok(());
            let (slow, join) = self.slow_path_with(c, pc, i, true, |g, _| {
                called = g.native_direct(c, pc, i, f, native, args, None, false);
            });
            called?;
            let b = self.b;
            let fk = |r: Reg| self.known_addr(c, r);
            let n8 = self.c64(8);
            let p: Vec<LLVMValueRef> = v
                .iter()
                .map(|&r| self.host(c, self.ld(c, r), fk(r), self.c64(0), n8, true, slow))
                .collect();
            let s1 = self.load(self.i64, p[0]);
            let s2 = self.load(self.i64, p[1]);
            let ok1 = self.icmp(IP::LLVMIntSLE, s1, self.c64(2147483562));
            let ok2 = self.icmp(IP::LLVMIntSLE, s2, self.c64(2147483398));
            self.guard(c, LLVMBuildAnd(b, ok1, ok2, c"".as_ptr()), slow);
            // One generator: k = s / q; t = a * (s - k * q) - k * r; t + m when negative.
            let lane = |s, q: i64, a: i64, r: i64, m: i64| {
                let k = LLVMBuildSDiv(b, s, self.c64(q), c"".as_ptr());
                let rem = LLVMBuildSub(
                    b,
                    s,
                    LLVMBuildMul(b, k, self.c64(q), c"".as_ptr()),
                    c"".as_ptr(),
                );
                let t = LLVMBuildSub(
                    b,
                    LLVMBuildMul(b, rem, self.c64(a), c"".as_ptr()),
                    LLVMBuildMul(b, k, self.c64(r), c"".as_ptr()),
                    c"".as_ptr(),
                );
                let neg = self.icmp(IP::LLVMIntSLT, t, self.c64(0));
                let wrapped = LLVMBuildAdd(b, t, self.c64(m), c"".as_ptr());
                LLVMBuildSelect(b, neg, wrapped, t, c"".as_ptr())
            };
            let t1 = lane(s1, 53668, 40014, 12211, 2147483563);
            let t2 = lane(s2, 52774, 40692, 3791, 2147483399);
            let z = LLVMBuildSub(b, t1, t2, c"".as_ptr());
            let small = self.icmp(IP::LLVMIntSLT, z, self.c64(1));
            let zw = LLVMBuildAdd(b, z, self.c64(2147483562), c"".as_ptr());
            let z = LLVMBuildSelect(b, small, zw, z, c"".as_ptr());
            let zf = LLVMBuildSIToFP(b, z, self.f64, c"".as_ptr());
            let x = LLVMBuildFMul(b, zf, LLVMConstReal(self.f64, 4.656613e-10), c"".as_ptr());
            let bits = LLVMBuildBitCast(b, x, self.i64, c"".as_ptr());
            // The native's order: the seeds, then the real.
            self.store(t1, p[0]);
            self.store(t2, p[1]);
            self.store(bits, p[2]);
            self.join(c, join);
            Ok(())
        }
    }

    /// A light `numeric_std` native computed in place (`ieee_fast`): operands of at most 64
    /// elements without a metavalue and results the native's own fast path gives; every other
    /// case, also each one that warns or fails, takes `slow`: the callback, which runs the
    /// native as before.
    #[allow(clippy::too_many_arguments)]
    unsafe fn ieee_fast(
        &self,
        c: &Cur,
        pc: u32,
        i: &Inst,
        f: &Func,
        native: yavs_ir::NativeId,
        args: yavs_ir::Regs,
        dst: Option<Reg>,
        op: yavs_ir::ieee::Op,
    ) -> Result<(), String> {
        use IP::*;
        use yavs_ir::ieee::{ARRAY_REGS, Cmp, Form, Gate, Op, Shift};
        // SAFETY: integer arithmetic on registers, reads through `host`, a result written to a
        // fresh temporary block; well-formed blocks of c.f.
        unsafe {
            let rs = f.regs(args).ok_or("operand list out of range")?;
            let dst = dst.ok_or("a native without a result")?;
            let layout = ieee_layout(op).ok_or("not a native computed in place")?;
            let mut called = Ok(());
            let (slow, join) = self.slow_path_with(c, pc, i, true, |g, _| {
                called = g.native_direct(c, pc, i, f, native, args, Some(dst), true);
            });
            called?;
            let b = self.b;
            let (c0, c1) = (self.c64(0), self.c64(1));
            let add = |x, y| LLVMBuildAdd(b, x, y, c"".as_ptr());
            let sub = |x, y| LLVMBuildSub(b, x, y, c"".as_ptr());
            let and = |x, y| LLVMBuildAnd(b, x, y, c"".as_ptr());
            let or = |x, y| LLVMBuildOr(b, x, y, c"".as_ptr());
            let xor = |x, y| LLVMBuildXor(b, x, y, c"".as_ptr());
            let shl = |x, y| LLVMBuildShl(b, x, y, c"".as_ptr());
            let lshr = |x, y| LLVMBuildLShr(b, x, y, c"".as_ptr());
            let not = |x| LLVMBuildNot(b, x, c"".as_ptr());
            let pick = |t, x, y| LLVMBuildSelect(b, t, x, y, c"".as_ptr());
            let cmp = |p, x, y| self.icmp(p, x, y);
            let umin = |x, y| pick(cmp(LLVMIntULT, x, y), x, y);
            // `low(k)` for k from 0 to 64: the k low bits.
            let lowk = |k| {
                let m = sub(shl(c1, and(k, self.c64(63))), c1);
                pick(cmp(LLVMIntEQ, k, self.c64(64)), self.c64(-1), m)
            };
            // The value of `bits` of `n` elements as a signed integer.
            let sext = |x, n| {
                let k = sub(self.c64(64), n);
                LLVMBuildAShr(b, shl(x, k), k, c"".as_ptr())
            };
            let pure = matches!(op, Op::Resize { .. } | Op::Shift(_) | Op::ShiftFn(_));
            // The operands in argument order.
            let (mut arrs, mut ints) = (Vec::new(), Vec::new());
            let mut at = 0;
            for a in layout {
                match a {
                    Arg::Arr => {
                        arrs.push(self.small_operand(c, &rs[at..at + ARRAY_REGS], pure, slow));
                        at += ARRAY_REGS;
                    }
                    Arg::Int => {
                        ints.push(self.ld(c, rs[at]));
                        at += 1;
                    }
                }
            }
            // `natural`: a negative integer is the native's error.
            let nonneg = |x| cmp(LLVMIntSGE, x, c0);
            let ok = |x: LLVMValueRef| self.guard(c, x, slow);
            // Unsigned or signed `size` low bits hold `v` (the native warns when they do not).
            let fits = |v, n, mask, signed| {
                if signed {
                    cmp(LLVMIntEQ, sext(and(v, mask), n), v)
                } else {
                    and(nonneg(v), cmp(LLVMIntEQ, and(v, not(mask)), c0))
                }
            };
            enum Out {
                Scalar(LLVMValueRef),
                // Count, bits, and whether the bounds are `1 to n` (else `n - 1 downto 0`).
                Vector(LLVMValueRef, LLVMValueRef, bool),
            }
            let out = match op {
                Op::ToInteger { signed } => {
                    let (n, bits, mask) = arrs[0];
                    let top = and(lshr(bits, sub(n, c1)), c1);
                    let neg = if signed {
                        cmp(LLVMIntNE, top, c0)
                    } else {
                        cmp(LLVMIntNE, c0, c0)
                    };
                    if signed {
                        // `to_signed (1, 1)` truncates: the native words that warning.
                        ok(not(and(cmp(LLVMIntEQ, n, c1), neg)));
                    }
                    let mag = pick(neg, and(not(bits), mask), bits);
                    // Beyond `integer'high` of the shortest revision the callback decides.
                    ok(cmp(LLVMIntULE, mag, self.c64(0x7fff_ffff)));
                    Out::Scalar(pick(neg, not(mag), mag))
                }
                Op::Add {
                    sub: is_sub,
                    signed,
                    form,
                } => {
                    let (n, x, y) = match form {
                        Form::Vv => {
                            let ((n1, b1, _), (n2, b2, _)) = (arrs[0], arrs[1]);
                            let n = pick(cmp(LLVMIntUGT, n1, n2), n1, n2);
                            let mask = lowk(n);
                            let ext = |bits, k| {
                                if signed {
                                    and(sext(bits, k), mask)
                                } else {
                                    bits
                                }
                            };
                            (n, ext(b1, n1), ext(b2, n2))
                        }
                        _ => {
                            let (n, bits, mask) = arrs[0];
                            let v = ints[0];
                            ok(fits(v, n, mask, signed));
                            let iv = and(v, mask);
                            if form == Form::Vi {
                                (n, bits, iv)
                            } else {
                                (n, iv, bits)
                            }
                        }
                    };
                    Out::Vector(n, if is_sub { sub(x, y) } else { add(x, y) }, false)
                }
                Op::Compare {
                    cmp: k,
                    signed,
                    form,
                } => {
                    let (x, y) = match form {
                        Form::Vv => {
                            let ((n1, b1, _), (n2, b2, _)) = (arrs[0], arrs[1]);
                            if signed {
                                (sext(b1, n1), sext(b2, n2))
                            } else {
                                (b1, b2)
                            }
                        }
                        _ => {
                            let (n, bits, _) = arrs[0];
                            let v = ints[0];
                            if !signed {
                                ok(nonneg(v));
                            }
                            let e = if signed { sext(bits, n) } else { bits };
                            if form == Form::Vi { (e, v) } else { (v, e) }
                        }
                    };
                    let p = match (k, signed) {
                        (Cmp::Gt, false) => LLVMIntUGT,
                        (Cmp::Lt, false) => LLVMIntULT,
                        (Cmp::Le, false) => LLVMIntULE,
                        (Cmp::Ge, false) => LLVMIntUGE,
                        (Cmp::Gt, true) => LLVMIntSGT,
                        (Cmp::Lt, true) => LLVMIntSLT,
                        (Cmp::Le, true) => LLVMIntSLE,
                        (Cmp::Ge, true) => LLVMIntSGE,
                        (Cmp::Eq, _) => LLVMIntEQ,
                        (Cmp::Ne, _) => LLVMIntNE,
                    };
                    Out::Scalar(self.zext(cmp(p, x, y)))
                }
                Op::ToUnsigned | Op::ToSigned => {
                    let signed = op == Op::ToSigned;
                    let (v, size) = (ints[0], ints[1]);
                    ok(and(
                        cmp(LLVMIntSGE, size, c1),
                        cmp(LLVMIntSLE, size, self.c64(64)),
                    ));
                    let mask = lowk(size);
                    ok(fits(v, size, mask, signed));
                    Out::Vector(size, and(v, mask), false)
                }
                Op::Resize { signed } => {
                    let (n, bits, _) = arrs[0];
                    let new = ints[0];
                    ok(and(
                        cmp(LLVMIntSGE, new, c1),
                        cmp(LLVMIntSLE, new, self.c64(64)),
                    ));
                    let res = if signed {
                        let keep = sub(umin(n, new), c1);
                        let sign = cmp(LLVMIntNE, and(lshr(bits, sub(n, c1)), c1), c0);
                        let kept = and(bits, lowk(keep));
                        let ext = and(lowk(new), not(lowk(keep)));
                        or(kept, pick(sign, ext, c0))
                    } else {
                        and(bits, lowk(umin(n, new)))
                    };
                    Out::Vector(new, res, false)
                }
                Op::Shift(k) | Op::ShiftFn(k) => {
                    let (n, bits, mask) = arrs[0];
                    let count = ints[0];
                    ok(nonneg(count));
                    let big = cmp(LLVMIntUGE, count, n);
                    let res = match k {
                        Shift::Sll => pick(big, c0, shl(bits, pick(big, c0, count))),
                        Shift::Srl => pick(big, c0, lshr(bits, pick(big, c0, count))),
                        Shift::Sra => {
                            // For 0 or 1 elements and a zero count the native keeps the
                            // argument's own bounds.
                            ok(and(cmp(LLVMIntUGT, n, c1), cmp(LLVMIntNE, count, c0)));
                            let k = umin(count, sub(n, c1));
                            and(LLVMBuildAShr(b, sext(bits, n), k, c"".as_ptr()), mask)
                        }
                        // A rotation by `count mod n`; by zero both shifts are by zero.
                        Shift::Rol => {
                            let k = LLVMBuildURem(b, count, n, c"".as_ptr());
                            let inv = pick(cmp(LLVMIntEQ, k, c0), c0, sub(n, k));
                            or(shl(bits, k), lshr(bits, inv))
                        }
                        Shift::Ror => {
                            let k = LLVMBuildURem(b, count, n, c"".as_ptr());
                            let inv = pick(cmp(LLVMIntEQ, k, c0), c0, sub(n, k));
                            or(lshr(bits, k), shl(bits, inv))
                        }
                    };
                    Out::Vector(n, res, false)
                }
                Op::NumGate(g) | Op::VecGate(g) => {
                    let ((n1, b1, _), (n2, b2, _)) = (arrs[0], arrs[1]);
                    ok(cmp(LLVMIntEQ, n1, n2));
                    let res = match g {
                        Gate::And => and(b1, b2),
                        Gate::Nand => not(and(b1, b2)),
                        Gate::Or => or(b1, b2),
                        Gate::Nor => not(or(b1, b2)),
                        Gate::Xor => xor(b1, b2),
                        Gate::Xnor => not(xor(b1, b2)),
                    };
                    Out::Vector(n1, res, matches!(op, Op::VecGate(_)))
                }
                Op::NumNot | Op::VecNot => {
                    let (n, bits, _) = arrs[0];
                    Out::Vector(n, not(bits), op == Op::VecNot)
                }
                _ => return Err("not a native computed in place".into()),
            };
            match out {
                Out::Scalar(v) => self.st(c, dst, v),
                Out::Vector(n, bits, up) => {
                    let mask = lowk(n);
                    let (at, p) = self.temp_alloc(c, self.c64(i64::from(PUT_SMALL_BYTES)), slow);
                    let (left, right, down) = if up {
                        (c1, n, c0)
                    } else {
                        (sub(n, c1), c0, c1)
                    };
                    let words = [
                        add(at, self.c64((16 + yavs_ir::DOPE_DIM_BYTES) as i64)),
                        add(at, self.c64(16)),
                        left,
                        right,
                        down,
                        and(bits, mask),
                        mask,
                        c0,
                        c0,
                    ];
                    for (k, w) in words.into_iter().enumerate() {
                        self.store(w, self.gep(p, self.c64(8 * k as i64)));
                    }
                    self.st(c, dst, at);
                }
            }
            self.join(c, join);
            Ok(())
        }
    }

    /// An array operand of a light native (`data, left, right, dir`) of 1 to 64 elements that
    /// holds only `0 1 L H` (and, with `pure`, no `L` or `H`), as `ieee::small` reads it:
    /// `(n, bits, mask)`, taking `slow` for any other.
    unsafe fn small_operand(
        &self,
        c: &Cur,
        r: &[Reg],
        pure: bool,
        slow: LLVMBasicBlockRef,
    ) -> (LLVMValueRef, LLVMValueRef, LLVMValueRef) {
        // SAFETY: integer arithmetic and a read of four words through `host`.
        unsafe {
            let b = self.b;
            let (l, rt, d) = (self.ld(c, r[1]), self.ld(c, r[2]), self.ld(c, r[3]));
            let down = self.icmp(IP::LLVMIntEQ, d, self.c64(1));
            let hi = LLVMBuildSelect(b, down, l, rt, c"".as_ptr());
            let lo = LLVMBuildSelect(b, down, rt, l, c"".as_ptr());
            let diff = LLVMBuildSub(b, hi, lo, c"".as_ptr());
            let ok = LLVMBuildAnd(
                b,
                self.icmp(IP::LLVMIntSGE, hi, lo),
                self.icmp(IP::LLVMIntULT, diff, self.c64(64)),
                c"".as_ptr(),
            );
            self.guard(c, ok, slow);
            let n = LLVMBuildAdd(b, diff, self.c64(1), c"".as_ptr());
            let mask = LLVMBuildSub(
                b,
                LLVMBuildShl(b, self.c64(2), diff, c"".as_ptr()),
                self.c64(1),
                c"".as_ptr(),
            );
            let p = self.host(
                c,
                self.ld(c, r[0]),
                self.known_addr(c, r[0]),
                self.c64(0),
                self.c64(32),
                false,
                slow,
            );
            let w = |k: i64| self.load(self.i64, self.gep(p, self.c64(8 * k)));
            let (p0, p1, p3) = (w(0), w(1), w(3));
            let live = LLVMBuildAnd(b, LLVMBuildNot(b, p3, c"".as_ptr()), p1, c"".as_ptr());
            // `0 L`: plane 1 only; `1 H`: planes 0 and 1.
            let zero = LLVMBuildAnd(b, live, LLVMBuildNot(b, p0, c"".as_ptr()), c"".as_ptr());
            let one = LLVMBuildAnd(b, live, p0, c"".as_ptr());
            let all = LLVMBuildAnd(
                b,
                LLVMBuildOr(b, zero, one, c"".as_ptr()),
                mask,
                c"".as_ptr(),
            );
            self.guard(c, self.icmp(IP::LLVMIntEQ, all, mask), slow);
            if pure {
                // Plane 2 is the weak bit of `L` and `H`.
                let weak = LLVMBuildAnd(b, w(2), mask, c"".as_ptr());
                self.guard(c, self.icmp(IP::LLVMIntEQ, weak, self.c64(0)), slow);
            }
            (n, LLVMBuildAnd(b, one, mask, c"".as_ptr()), mask)
        }
    }

    /// The instruction at `pc` where the generator has no code for it: a native frame has the
    /// interpreter execute it on a frame of its own (`xstep_ok`), a frame-mode function steps it.
    unsafe fn slow_inst(&self, c: &Cur, pc: u32, i: &Inst, f: &Func) -> Result<(), String> {
        // SAFETY: as `native_step` and `step`.
        unsafe {
            if c.nat.is_some() && xstep_ok(f, i) {
                self.native_step(c, pc, i, f)
            } else {
                self.step(c, pc, i);
                Ok(())
            }
        }
    }

    /// An instruction of a native frame the interpreter executes on a frame of its own
    /// (`TierHost::native_step`); the registers it reads go in, those it writes come back.
    unsafe fn native_step(&self, c: &Cur, pc: u32, i: &Inst, f: &Func) -> Result<(), String> {
        // SAFETY: the arguments match `xstep_ty`; well-formed blocks of c.f.
        unsafe {
            let nat = c.nat.as_ref().ok_or("native step outside a native frame")?;
            let (uses, defs) = yavs_interp::inst_regs(f, i);
            let (slow, join) = self.slow_path(c, pc, i);
            let ins = self.entry_alloca(c, uses.len() as u64 * 8);
            let outs = self.entry_alloca(c, defs.len() as u64 * 8);
            for (k, &r) in uses.iter().enumerate() {
                self.store(self.ld(c, r), self.gep(ins, self.c64(k as i64 * 8)));
            }
            let idx = |rs: &[Reg]| self.u32_table(&rs.iter().map(|r| r.0).collect::<Vec<_>>());
            let mut a = [
                c.env,
                c.mem,
                self.c32(nat.id.0),
                self.c32(pc),
                nat.depth,
                idx(&uses),
                ins,
                self.c32(uses.len() as u32),
                idx(&defs),
                outs,
                self.c32(defs.len() as u32),
                self.temp_base(c),
            ];
            let st = LLVMBuildCall2(
                self.b,
                self.xstep_ty,
                self.xstep,
                a.as_mut_ptr(),
                12,
                c"".as_ptr(),
            );
            self.guard(c, self.icmp(IP::LLVMIntNE, st, self.c32(0)), slow);
            for (k, &r) in defs.iter().enumerate() {
                let v = self.load(self.i64, self.gep(outs, self.c64(k as i64 * 8)));
                self.st(c, r, v);
            }
            let (out, cont) = (self.block(c), self.block(c));
            self.cond_br(self.icmp(IP::LLVMIntEQ, st, self.c32(2)), out, cont, false);
            LLVMPositionBuilderAtEnd(self.b, out);
            self.leave(c, self.c32(pc + 1));
            LLVMPositionBuilderAtEnd(self.b, cont);
            self.join(c, join);
            Ok(())
        }
    }

    /// Materialise a native frame at `pc` (its exit block); the builder goes on in a new block,
    /// which nothing reaches.
    unsafe fn deopt_here(&self, c: &Cur, pc: u32) {
        // SAFETY: well-formed blocks of c.f.
        unsafe {
            self.leave(c, self.c32(pc));
            let dead = self.block(c);
            LLVMPositionBuilderAtEnd(self.b, dead);
        }
    }

    /// The temporary area's length the frame's statements truncate it to (`Inst::Stmt`).
    unsafe fn temp_base(&self, c: &Cur) -> LLVMValueRef {
        match &c.nat {
            // SAFETY: the frame's cell is an i64 `alloca`.
            Some(n) => unsafe { LLVMBuildLoad2(self.b, self.i64, n.temp_base, c"".as_ptr()) },
            // SAFETY: a `Mem` field.
            None => unsafe { self.mem_ld(c, MEM_TEMP_BASE, self.i64) },
        }
    }

    /// The high half of the addresses of the function's own frame.
    unsafe fn own_hi(&self, c: &Cur) -> LLVMValueRef {
        match &c.nat {
            Some(n) => n.fhi,
            // SAFETY: a `Mem` field.
            None => unsafe { self.mem_ld(c, MEM_FRAME, self.i64) },
        }
    }

    /// The function's own slots, as `own_slots` finds them, once per stretch of straight-line
    /// code and carried across the blocks of a loop: the base moves only when a callback
    /// pushes a frame (`call_go`, and `join` after a slow path that may). Main-path code only
    /// (the value must dominate what follows).
    unsafe fn own_slots_cached(&self, c: &Cur) -> LLVMValueRef {
        let Some(d) = c.defer.as_ref().filter(|_| c.nat.is_none()) else {
            // SAFETY: as `own_slots`.
            return unsafe { self.own_slots(c) };
        };
        let b = d.pend.borrow().len() - 1;
        if let Some(v) = d.pend.borrow()[b] {
            return v;
        }
        // SAFETY: as `own_slots`.
        let v = unsafe { self.own_slots(c) };
        d.pend.borrow_mut()[b] = Some(v);
        v
    }

    /// The function's own slots.
    unsafe fn own_slots(&self, c: &Cur) -> LLVMValueRef {
        match &c.nat {
            Some(n) => n.slots,
            // SAFETY: a `Mem` field.
            None => unsafe { self.mem_ld(c, MEM_SLOTS, self.ptr) },
        }
    }

    /// Where a helper writes register `dst` (the registers and its index; a native frame's
    /// scratch word and 0); `reg_in` moves it into the register.
    unsafe fn reg_out(&self, c: &Cur, dst: Reg) -> (LLVMValueRef, u32) {
        match &c.nat {
            Some(n) => (n.scratch, 0),
            None => {
                // The helper writes the frame's register.
                if let Some(d) = &c.defer
                    && let Some(p) = d.pend.borrow_mut().get_mut(dst.0 as usize)
                {
                    *p = None;
                }
                (c.regs, dst.0)
            }
        }
    }

    unsafe fn reg_in(&self, c: &Cur, dst: Reg) {
        if let Some(n) = &c.nat {
            // SAFETY: the scratch word is an i64 `alloca`.
            unsafe {
                let v = LLVMBuildLoad2(self.b, self.i64, n.scratch, c"".as_ptr());
                self.st(c, dst, v);
            }
        }
    }

    /// `fast_op` of an operation that writes register `dst`.
    unsafe fn fast_reg(
        &self,
        c: &Cur,
        op: u32,
        dst: Reg,
        a: LLVMValueRef,
        b: LLVMValueRef,
        slow: LLVMBasicBlockRef,
    ) {
        // SAFETY: as `fast_op`.
        unsafe {
            let (_, d) = self.reg_out(c, dst);
            self.fast_op(c, op, d, a, b, slow);
            self.reg_in(c, dst);
        }
    }

    /// `yavs_rt_fast` operation `op` on `a` and `b` (result register or count `dst`), taking
    /// `slow` when it returns 0: the interpreter then executes the instruction.
    unsafe fn fast_op(
        &self,
        c: &Cur,
        op: u32,
        dst: u32,
        a: LLVMValueRef,
        b: LLVMValueRef,
        slow: LLVMBasicBlockRef,
    ) {
        // SAFETY: the arguments match `fast_ty`; well-formed blocks of c.f.
        unsafe {
            // A native frame's registers are not the helpers' to see.
            let rp = c.nat.as_ref().map_or(c.regs, |n| n.scratch);
            let mut args = [c.env, rp, c.mem, self.c32(op), self.c32(dst), a, b];
            let ok = LLVMBuildCall2(
                self.b,
                self.fast_ty,
                self.fast,
                args.as_mut_ptr(),
                7,
                c"".as_ptr(),
            );
            // Alloc writes the temporary area, the others only read memory besides the
            // register and the object cache.
            let other = match op {
                FAST_ALLOC => MODS,
                FAST_DEREF => 0,
                _ => REF,
            };
            self.attr(ok, true, "memory", mem_effects(RW, RW, other));
            self.attr(ok, true, "willreturn", 0);
            self.guard(c, self.icmp(IP::LLVMIntNE, ok, self.c32(0)), slow);
        }
    }

    /// Call a callback that returns `GO` or what the compiled function returns at once. A
    /// `quiet` one (an instruction that neither collects nor runs other code) cannot look at
    /// slot variables, so they are written back only when it fails.
    unsafe fn call_go(
        &self,
        c: &Cur,
        ty: LLVMTypeRef,
        f: LLVMValueRef,
        args: &mut [LLVMValueRef],
        quiet: bool,
    ) {
        // SAFETY: `args` match `ty`; well-formed blocks of c.f.
        unsafe {
            // The callback reads the registers of its instruction from the frame.
            let mut key = Vec::new();
            if let Some(d) = &c.defer {
                if quiet {
                    key = self.flush_regs(c, d.pc.get());
                } else {
                    self.flush(c, d.pc.get() as usize);
                }
            }
            let r = LLVMBuildCall2(
                self.b,
                ty,
                f,
                args.as_mut_ptr(),
                args.len() as u32,
                c"".as_ptr(),
            );
            let cold = self.cold_depth.get() > 0;
            if cold {
                self.attr(r, true, "cold", 0);
            }
            // The callback may have pushed a frame, which moves the slots.
            if let Some(d) = &c.defer {
                *d.pend.borrow_mut().last_mut().expect("base") = None;
            }
            let cont = self.block(c);
            if cold {
                c.cold.borrow_mut().push(cont);
            }
            let ok = self.icmp(IP::LLVMIntEQ, r, self.c32(GO));
            if key.is_empty() {
                let exit = self.exit_for(c, r);
                self.cond_br(ok, cont, exit, true);
            } else {
                let fail = self.block(c);
                if cold {
                    c.cold.borrow_mut().push(fail);
                }
                self.cond_br(ok, cont, fail, true);
                LLVMPositionBuilderAtEnd(self.b, fail);
                self.leave_slots(c, key, r);
            }
            LLVMPositionBuilderAtEnd(self.b, cont);
        }
    }

    unsafe fn range_exit(
        &self,
        c: &Cur,
        x: LLVMValueRef,
        lo: LLVMValueRef,
        hi: LLVMValueRef,
        pc: u32,
    ) {
        // SAFETY: i64 operands.
        unsafe {
            let a = self.icmp(IP::LLVMIntSLT, x, lo);
            let b = self.icmp(IP::LLVMIntSGT, x, hi);
            self.exit_if(c, self.or(a, b), pc);
        }
    }

    /// A block that steps the instruction `i` at `pc` in the interpreter and goes on at the
    /// returned join block: the slow path of a native instruction. The builder stays.
    unsafe fn slow_path(
        &self,
        c: &Cur,
        pc: u32,
        i: &Inst,
    ) -> (LLVMBasicBlockRef, LLVMBasicBlockRef) {
        // SAFETY: as `slow_path_then`.
        unsafe { self.slow_path_then(c, pc, i, |_, _| {}) }
    }

    /// `slow_path` for an instruction of the `xstep_ok` kinds: a native frame hands it to the
    /// interpreter on a frame of its own (`slow_inst`) instead of materialising (#648).
    unsafe fn slow_path_x(
        &self,
        c: &Cur,
        pc: u32,
        i: &Inst,
        f: &Func,
    ) -> (LLVMBasicBlockRef, LLVMBasicBlockRef) {
        // SAFETY: as `slow_path_with`.
        unsafe {
            self.slow_path_with(c, pc, i, true, |g, _| {
                g.slow_inst(c, pc, i, f)
                    .expect("a native step only fails outside a native frame");
            })
        }
    }

    /// `slow_path`, with `after` generating code in the slow block once the instruction has
    /// been stepped (it gets the join block to go to instead of continuing).
    unsafe fn slow_path_then(
        &self,
        c: &Cur,
        pc: u32,
        i: &Inst,
        after: impl FnOnce(&Self, LLVMBasicBlockRef),
    ) -> (LLVMBasicBlockRef, LLVMBasicBlockRef) {
        // SAFETY: as `slow_path_with`.
        unsafe {
            self.slow_path_with(c, pc, i, true, |g, join| {
                g.step(c, pc, i);
                after(g, join);
            })
        }
    }

    /// A block that runs `body` (given the join block) in place of the fast path of the
    /// instruction `i` and goes on at the returned join block. `cold`: it is the rare case.
    unsafe fn slow_path_with(
        &self,
        c: &Cur,
        _pc: u32,
        i: &Inst,
        cold: bool,
        body: impl FnOnce(&Self, LLVMBasicBlockRef),
    ) -> (LLVMBasicBlockRef, LLVMBasicBlockRef) {
        // SAFETY: well-formed blocks of c.f.
        unsafe {
            let here = LLVMGetInsertBlock(self.b);
            let (slow, join) = (self.block(c), self.block(c));
            LLVMPositionBuilderAtEnd(self.b, slow);
            // The slow path is no business of the fast path's: what it found or forgot about the
            // frame's base does not hold at the join (the fast path may define the base after the
            // branch, which does not dominate the join); the join takes the value from before,
            // unless the step may push a frame.
            let base = c
                .defer
                .as_ref()
                .map(|d| *d.pend.borrow().last().expect("base"));
            if cold {
                c.cold.borrow_mut().push(slow);
                self.cold_depth.set(self.cold_depth.get() + 1);
            }
            body(self, join);
            if cold {
                self.cold_depth.set(self.cold_depth.get() - 1);
            }
            if let (Some(d), Some(b)) = (&c.defer, base) {
                *d.pend.borrow_mut().last_mut().expect("base") = b;
                c.joins.borrow_mut().push((b, !quiet(i)));
            }
            LLVMBuildBr(self.b, join);
            LLVMPositionBuilderAtEnd(self.b, here);
            (slow, join)
        }
    }

    /// Go on in a new block when `ok`, else take `slow`.
    unsafe fn guard(&self, c: &Cur, ok: LLVMValueRef, slow: LLVMBasicBlockRef) {
        // SAFETY: well-formed blocks of c.f.
        unsafe {
            let go = self.block(c);
            self.cond_br(ok, go, slow, true);
            LLVMPositionBuilderAtEnd(self.b, go);
        }
    }

    /// A conditional branch whose true edge is the common one (`likely`) or the rare one: block
    /// placement keeps the hot path straight.
    unsafe fn cond_br(
        &self,
        cond: LLVMValueRef,
        t: LLVMBasicBlockRef,
        f: LLVMBasicBlockRef,
        likely: bool,
    ) {
        // SAFETY: blocks of the function being built.
        unsafe {
            // The unoptimised code generator fuses a compare into the branch and falls through to
            // the next block, but a condition it has to test (`ok = a & b`) becomes `jcc t; jmp f`
            // with `t` the next block: a taken branch over a jump on the hot path. Branching on
            // the negation sends the rare edge to the `jcc` and the hot one falls through (#679).
            if likely && LLVMIsAICmpInst(cond).is_null() && LLVMIsAFCmpInst(cond).is_null() {
                let not = LLVMBuildNot(self.b, cond, c"".as_ptr());
                let br = LLVMBuildCondBr(self.b, not, f, t);
                self.weights(br, (1, 2000));
                return;
            }
            let br = LLVMBuildCondBr(self.b, cond, t, f);
            self.weights(br, if likely { (2000, 1) } else { (1, 2000) });
        }
    }

    /// Branch weights on the conditional branch `br`.
    unsafe fn weights(&self, br: LLVMValueRef, (a, b): (u64, u64)) {
        // SAFETY: metadata nodes of our context attached to a branch of our module.
        unsafe {
            let name = LLVMMDStringInContext2(self.ctx, c"branch_weights".as_ptr(), 14);
            let w = |n: u64| LLVMValueAsMetadata(LLVMConstInt(self.i32, n, 0));
            let mut ops = [name, w(a), w(b)];
            let node = LLVMMDNodeInContext2(self.ctx, ops.as_mut_ptr(), ops.len());
            let kind = LLVMGetMDKindIDInContext(self.ctx, c"prof".as_ptr(), 4);
            LLVMSetMetadata(br, kind, LLVMMetadataAsValue(self.ctx, node));
        }
    }

    /// End the fast path at the slow path's join block.
    unsafe fn join(&self, c: &Cur, join: LLVMBasicBlockRef) {
        // SAFETY: the builder is in a block of the function of `join`.
        unsafe { LLVMBuildBr(self.b, join) };
        self.join_here(c, join);
    }

    /// `join` for paths that all branched to `join` already: continue there.
    fn join_here(&self, c: &Cur, join: LLVMBasicBlockRef) {
        // SAFETY: a block of the function being built.
        unsafe { LLVMPositionBuilderAtEnd(self.b, join) };
        if let Some(d) = &c.defer
            && let Some((b, loud)) = c.joins.borrow_mut().pop()
        {
            *d.pend.borrow_mut().last_mut().expect("base") = if loud { None } else { b };
        }
    }

    /// After the interpreter made the layout of a `LayoutOf` with a frame slot (its address in
    /// register `dst`): keep the descriptor and the layout words in the slot at `at`, `dsz`
    /// and `nw` words, and mark it, when the slot is empty (a kept layout is never replaced:
    /// registers hold its address). Anything compiled code cannot reach goes on to `join`
    /// without keeping.
    #[allow(clippy::too_many_arguments)]
    unsafe fn layout_keep(
        &self,
        c: &Cur,
        dst: Reg,
        desc: Reg,
        at_desc: Option<Known>,
        at: u32,
        dsz: u32,
        nw: u32,
        join: LLVMBasicBlockRef,
    ) {
        // SAFETY: well-formed blocks of c.f; the frame has the slot (`layout_cache`).
        unsafe {
            // The step wrote `dst` to the frame: a value deferred for an earlier definition of
            // it is stale here (#710).
            let stale = c
                .defer
                .as_ref()
                .and_then(|d| d.pend.borrow_mut()[dst.0 as usize].take());
            self.reg_in(c, dst);
            // Kept once: the address of a kept layout is handed out and lives as long as the
            // process, so a slot is never written again.
            let slot = self.gep(
                self.tag(self.own_slots(c), AA_SLOTS),
                self.c64(i64::from(at)),
            );
            let kept = self.load(self.i64, slot);
            self.guard(c, self.icmp(IP::LLVMIntEQ, kept, self.c64(0)), join);
            let pw = self.host(
                c,
                self.ld(c, dst),
                Some(Known::Temp),
                self.c64(0),
                self.c64(8 * i64::from(nw)),
                false,
                join,
            );
            let pd = self.host(
                c,
                self.ld(c, desc),
                at_desc,
                self.c64(0),
                self.c64(i64::from(dsz)),
                false,
                join,
            );
            let slot = self.gep(
                self.tag(self.own_slots(c), AA_SLOTS),
                self.c64(i64::from(at)),
            );
            let d0 = self.gep(slot, self.c64(8));
            LLVMBuildMemCpy(self.b, d0, 1, pd, 1, self.c64(i64::from(dsz)));
            let w0 = self.gep(slot, self.c64(8 + i64::from(dsz)));
            LLVMBuildMemCpy(self.b, w0, 1, pw, 1, self.c64(8 * i64::from(nw)));
            self.store(self.c64(1), slot);
            if let (Some(d), Some(v)) = (&c.defer, stale) {
                d.pend.borrow_mut()[dst.0 as usize] = Some(v);
            }
        }
    }

    /// A private constant array of bytes.
    unsafe fn table(&self, t: &[u8]) -> LLVMValueRef {
        // SAFETY: a global of our module with a matching initialiser.
        unsafe {
            let mut vals: Vec<LLVMValueRef> = t
                .iter()
                .map(|&b| LLVMConstInt(self.i8, u64::from(b), 0))
                .collect();
            let ty = LLVMArrayType2(self.i8, t.len() as u64);
            let g = LLVMAddGlobal(self.module, ty, c"".as_ptr());
            LLVMSetInitializer(
                g,
                LLVMConstArray2(self.i8, vals.as_mut_ptr(), t.len() as u64),
            );
            LLVMSetGlobalConstant(g, 1);
            LLVMSetLinkage(g, llvm_sys::LLVMLinkage::LLVMPrivateLinkage);
            g
        }
    }

    /// The head of an element loop in a native frame asks `yavs_rt_eloop` for its iterations but
    /// the last (the shapes `eloop_plan` leaves alone); the builder ends at `generic`'s entry edge.
    unsafe fn eloop_call(&self, c: &Cur, l: &yavs_interp::ElemLoop, generic: LLVMBasicBlockRef) {
        // SAFETY: blocks of c.f, loads of its registers, a call with the declared signature.
        unsafe {
            let (ind, bound) = (self.ld(c, l.ind), self.ld(c, l.bound));
            let step = self.ld(c, l.step);
            let (call, set) = (self.block(c), self.block(c));
            // At least two iterations to go (the helper leaves the last to the generic code).
            let next = LLVMBuildAdd(self.b, ind, step, c"".as_ptr());
            let more = LLVMBuildAnd(
                self.b,
                self.icmp(IP::LLVMIntNE, ind, bound),
                self.icmp(IP::LLVMIntNE, next, bound),
                c"".as_ptr(),
            );
            LLVMBuildCondBr(self.b, more, call, generic);
            LLVMPositionBuilderAtEnd(self.b, call);
            let inputs = self.entry_alloca(c, 8 * yavs_interp::eloop::MAX_INPUTS as u64);
            for (k, r) in l.inputs.iter().enumerate() {
                self.store(self.ld(c, *r), self.gep(inputs, self.c64(k as i64 * 8)));
            }
            let mut args = [
                c.env,
                c.mem,
                self.i64_table(&l.prog),
                self.c64(l.prog.len() as i64),
                inputs,
                ind,
                bound,
                step,
                c.nat.as_ref().map_or(self.c64(0), |n| n.depth),
                c.nat.as_ref().map_or(self.c64(0), |n| n.fhi),
                c.nat
                    .as_ref()
                    .map_or(LLVMConstPointerNull(self.ptr), |n| n.slots),
                self.c64(c.frame_len as i64),
            ];
            let done = LLVMBuildCall2(
                self.b,
                self.eloop_ty,
                self.eloop,
                args.as_mut_ptr(),
                12,
                c"".as_ptr(),
            );
            let some = self.icmp(IP::LLVMIntNE, done, self.c64(0));
            LLVMBuildCondBr(self.b, some, set, generic);
            LLVMPositionBuilderAtEnd(self.b, set);
            let moved = LLVMBuildMul(self.b, done, step, c"".as_ptr());
            self.st(c, l.ind, LLVMBuildAdd(self.b, ind, moved, c"".as_ptr()));
            LLVMBuildBr(self.b, generic);
        }
    }

    /// The host pointer to `n` bytes at `a` plus `extra` for `eloop_plan`: `host`'s inline tests,
    /// the temporary area first where the generator knows nothing (a caller's argument or a fresh
    /// object there is the common case); `slow` is taken where the bytes are not found.
    #[allow(clippy::too_many_arguments)]
    unsafe fn eloop_host(
        &self,
        c: &Cur,
        at: Option<Known>,
        a: LLVMValueRef,
        extra: LLVMValueRef,
        n: LLVMValueRef,
        write: bool,
        slow: LLVMBasicBlockRef,
    ) -> LLVMValueRef {
        // SAFETY: as `host`.
        unsafe { self.host(c, a, at.or(Some(Known::LikelyTemp)), extra, n, write, slow) }
    }

    /// The iterations of an element loop but the last, planned and done in the code itself
    /// (#757): the program of steps (`yavs_interp::elem_loops`) as straight-line code that holds
    /// each value as a pair (value at the first iteration, change per iteration; `inv` says which
    /// have none), checks every position, descriptor and element range at the first and the last
    /// iteration as `crate::eloop::run` does, and calls `yavs_rt_eloop_exec` for the elements. Any
    /// check that fails, and any memory the loop would reach that the objects do not, takes
    /// `generic` (nothing was written); the loop variable moves by the iterations done and the
    /// builder leaves for `generic`, which runs the last iteration (or the whole loop).
    unsafe fn eloop_plan(
        &self,
        c: &Cur,
        l: &yavs_interp::ElemLoop,
        inv: &[bool],
        generic: LLVMBasicBlockRef,
    ) {
        use yavs_interp::eloop::{ADD, ADDR, GET, IND, KONST, LD, LEN, MOV, MUL, POS, SET, SUB};
        // SAFETY: integer arithmetic, loads of the loop's registers, blocks of c.f, calls with the
        // declared signatures; every pointer is used for the bytes `eloop_host` was asked for.
        unsafe {
            let b = self.b;
            let (ind, bound, step) = (self.ld(c, l.ind), self.ld(c, l.bound), self.ld(c, l.step));
            let (c0, c1) = (self.c64(0), self.c64(1));
            let i1 = LLVMInt1TypeInContext(self.ctx);
            let no = LLVMConstInt(i1, 0, 0);
            let not = |v| LLVMBuildNot(b, v, c"".as_ptr());
            let and = |x, y| LLVMBuildAnd(b, x, y, c"".as_ptr());
            let sel = |k, x, y| LLVMBuildSelect(b, k, x, y, c"".as_ptr());
            // Two iterations to go at least, 1 or -1 at a time: `m` iterations, the last at `bound`.
            let (d0, o1) = self.ovf(self.ssub, bound, ind);
            let (dist, o2) = self.ovf(self.smul, d0, step);
            let lim = self.c64(ELOOP_LIMIT);
            let m = sel(self.icmp(IP::LLVMIntSLT, dist, lim), dist, lim);
            let unit = self.or(
                self.icmp(IP::LLVMIntEQ, step, c1),
                self.icmp(IP::LLVMIntEQ, step, self.c64(-1)),
            );
            let ok = and(
                and(unit, not(self.or(o1, o2))),
                self.icmp(IP::LLVMIntSGE, m, self.c64(2)),
            );
            self.guard(c, ok, generic);
            let last = LLVMBuildSub(b, m, c1, c"".as_ptr());
            // (value at iteration 0, change per iteration) of each step.
            let mut val: Vec<(LLVMValueRef, LLVMValueRef)> = Vec::new();
            let mut bad = no;
            // The memory read besides the vectors: host address and bytes.
            let mut reads: Vec<(LLVMValueRef, u64)> = Vec::new();
            #[allow(clippy::type_complexity)]
            let mut gets: Option<(
                Option<Known>,
                LLVMValueRef,
                (LLVMValueRef, LLVMValueRef),
                bool,
            )> = None;
            // What the generator knows of the register an operand is an input of.
            let known = |v: i64| -> Option<Known> {
                (v <= -2)
                    .then(|| l.inputs.get((-2 - v) as usize))
                    .flatten()
                    .and_then(|r| self.known_addr(c, *r))
            };
            let get =
                |v: i64, val: &Vec<(LLVMValueRef, LLVMValueRef)>| -> (LLVMValueRef, LLVMValueRef) {
                    if v >= 0 {
                        val[v as usize]
                    } else if v == IND {
                        (ind, step)
                    } else {
                        (self.ld(c, l.inputs[(-2 - v) as usize]), c0)
                    }
                };
            // `x0 + x1 * last` and whether it overflowed; `x1` is not 0 unless `still`.
            let at_last = |x: (LLVMValueRef, LLVMValueRef), still: bool, bad: &mut LLVMValueRef| {
                if still {
                    return x.0;
                }
                let (p, o) = self.ovf(self.smul, x.1, last);
                let (e, o2) = self.ovf(self.sadd, x.0, p);
                *bad = self.or(*bad, self.or(o, o2));
                e
            };
            let val_inv =
                |v: i64, inv: &[bool]| -> bool { if v >= 0 { inv[v as usize] } else { v != IND } };
            for (j, st) in l.prog.as_chunks::<4>().0.iter().enumerate() {
                let (a, bb, cc) = (st[1], st[2], st[3]);
                let r = match st[0] {
                    MOV => get(a, &val),
                    KONST => (self.c64(a), c0),
                    ADD | SUB => {
                        let (x, y) = (get(a, &val), get(bb, &val));
                        let f = if st[0] == ADD { self.sadd } else { self.ssub };
                        let (r0, o) = self.ovf(f, x.0, y.0);
                        bad = self.or(bad, o);
                        let r1 = if inv[j] {
                            c0
                        } else {
                            let (r1, o) = self.ovf(f, x.1, y.1);
                            bad = self.or(bad, o);
                            r1
                        };
                        let r = (r0, r1);
                        at_last(r, inv[j], &mut bad);
                        r
                    }
                    MUL => {
                        // One of them does not change (`invariant_steps`).
                        let (x, y) = (get(a, &val), get(bb, &val));
                        let (k, v, vinv) = if val_inv(a, inv) {
                            (x.0, y, val_inv(bb, inv))
                        } else {
                            (y.0, x, false)
                        };
                        let (r0, o) = self.ovf(self.smul, k, v.0);
                        bad = self.or(bad, o);
                        let r1 = if vinv {
                            c0
                        } else {
                            let (r1, o) = self.ovf(self.smul, k, v.1);
                            bad = self.or(bad, o);
                            r1
                        };
                        let r = (r0, r1);
                        at_last(r, inv[j], &mut bad);
                        r
                    }
                    POS | LEN => {
                        let d = get(a, &val).0;
                        let p = self.eloop_host(
                            c,
                            known(a),
                            d,
                            self.c64(bb * 24),
                            self.c64(24),
                            false,
                            generic,
                        );
                        reads.push((LLVMBuildPtrToInt(b, p, self.i64, c"".as_ptr()), 24));
                        let (dl, dr, dir) = self.dope_words(p);
                        if st[0] == LEN {
                            let (n, fits) = self.len_of(dl, dr, dir);
                            let big = self.icmp(IP::LLVMIntSLT, n, c0);
                            bad = self.or(bad, self.or(not(fits), big));
                            (n, c0)
                        } else {
                            let x = get(cc, &val);
                            let xinv = val_inv(cc, inv);
                            let up = self.icmp(IP::LLVMIntEQ, dir, c0);
                            let down = self.icmp(IP::LLVMIntEQ, dir, c1);
                            bad = self.or(bad, not(self.or(up, down)));
                            let e1 = at_last(x, xinv, &mut bad);
                            let lo = sel(down, dr, dl);
                            let hi = sel(down, dl, dr);
                            let (p0, o) =
                                self.ovf(self.ssub, sel(down, dl, x.0), sel(down, x.0, dl));
                            bad = self.or(bad, o);
                            let p1 = if xinv {
                                c0
                            } else {
                                let (neg, o) = self.ovf(self.ssub, c0, x.1);
                                bad = self.or(bad, and(o, down));
                                sel(down, neg, x.1)
                            };
                            let le = |u, v| self.icmp(IP::LLVMIntSLE, u, v);
                            let inside =
                                and(and(le(lo, x.0), le(x.0, hi)), and(le(lo, e1), le(e1, hi)));
                            bad = self.or(bad, not(inside));
                            (p0, p1)
                        }
                    }
                    LD => {
                        let base = get(a, &val).0;
                        let p = self.eloop_host(
                            c,
                            known(a),
                            base,
                            self.c64(bb),
                            self.c64(cc),
                            false,
                            generic,
                        );
                        reads.push((LLVMBuildPtrToInt(b, p, self.i64, c"".as_ptr()), cc as u64));
                        let ty = match cc {
                            1 => self.i8,
                            4 => self.i32,
                            _ => self.i64,
                        };
                        let w = self.load(ty, p);
                        let w = if cc == 8 {
                            w
                        } else {
                            LLVMBuildZExt(b, w, self.i64, c"".as_ptr())
                        };
                        (w, c0)
                    }
                    ADDR => {
                        let base = get(a, &val).0;
                        let lo = LLVMBuildAnd(b, base, self.c64(0xffff_ffff), c"".as_ptr());
                        let lo = LLVMBuildAdd(b, lo, self.c64(bb), c"".as_ptr());
                        bad = self.or(bad, self.icmp(IP::LLVMIntUGT, lo, self.c64(0xffff_ffff)));
                        let hi = LLVMBuildAnd(b, base, self.c64(!0xffff_ffff), c"".as_ptr());
                        (LLVMBuildOr(b, hi, lo, c"".as_ptr()), c0)
                    }
                    GET => {
                        let base = get(a, &val).0;
                        gets = Some((known(a), base, get(bb, &val), val_inv(bb, inv)));
                        (c0, c0)
                    }
                    SET => {
                        let base = get(a, &val).0;
                        let x = get(bb, &val);
                        let xinv = val_inv(bb, inv);
                        // The elements `x0 + x1 * n`, `n` up to `last`, of the vector at `base`,
                        // all in its object: the pointer to its first byte and the blocks'
                        // byte range `lo..hi`.
                        let stream = |at: Option<Known>,
                                      base: LLVMValueRef,
                                      x: (LLVMValueRef, LLVMValueRef),
                                      xinv: bool,
                                      write: bool,
                                      bad: &mut LLVMValueRef| {
                            let e1 = at_last(x, xinv, bad);
                            let big = self.c64(1 << 32);
                            let out = self.or(
                                self.icmp(IP::LLVMIntUGE, x.0, big),
                                self.icmp(IP::LLVMIntUGE, e1, big),
                            );
                            let bads = self.or(*bad, out);
                            self.guard(c, not(bads), generic);
                            *bad = no;
                            let hi = sel(self.icmp(IP::LLVMIntUGT, x.0, e1), x.0, e1);
                            let lo = sel(self.icmp(IP::LLVMIntULT, x.0, e1), x.0, e1);
                            let blk = |e| {
                                LLVMBuildShl(
                                    b,
                                    LLVMBuildLShr(b, e, self.c64(6), c"".as_ptr()),
                                    self.c64(5),
                                    c"".as_ptr(),
                                )
                            };
                            let end = LLVMBuildAdd(b, blk(hi), self.c64(32), c"".as_ptr());
                            let p = self.eloop_host(c, at, base, c0, end, write, generic);
                            let pi = LLVMBuildPtrToInt(b, p, self.i64, c"".as_ptr());
                            (
                                p,
                                LLVMBuildAdd(b, pi, blk(lo), c"".as_ptr()),
                                LLVMBuildAdd(b, pi, end, c"".as_ptr()),
                            )
                        };
                        let (dp, dlo, dhi) = stream(known(a), base, x, xinv, true, &mut bad);
                        // Nothing it read is among the elements it writes.
                        for &(rs, n) in &reads {
                            let re = LLVMBuildAdd(b, rs, self.c64(n as i64), c"".as_ptr());
                            let hit = and(
                                self.icmp(IP::LLVMIntULT, rs, dhi),
                                self.icmp(IP::LLVMIntULT, dlo, re),
                            );
                            bad = self.or(bad, hit);
                        }
                        let (sp, s_e0, s_step, code) = if cc >= 0 {
                            let (sat, sb, sx, sinv) =
                                gets.expect("`invariant_steps` has a GET before a copy");
                            let (sp, _, _) = stream(sat, sb, sx, sinv, false, &mut bad);
                            (sp, sx.0, sx.1, c0)
                        } else {
                            let code = get(cc, &val).0;
                            bad = self.or(bad, self.icmp(IP::LLVMIntUGT, code, self.c64(8)));
                            (LLVMConstPointerNull(self.ptr), c0, c0, code)
                        };
                        self.guard(c, not(bad), generic);
                        // A copy within one plane block each side, the same direction, the blocks
                        // apart: the plane copy of `eloop::exec` inline.
                        let short = (cc >= 0).then(|| {
                            let lo = |e0, d| {
                                let e = LLVMBuildAdd(
                                    b,
                                    e0,
                                    LLVMBuildMul(b, d, last, c"".as_ptr()),
                                    c"".as_ptr(),
                                );
                                sel(self.icmp(IP::LLVMIntULT, e, e0), e, e0)
                            };
                            let (ls, ld) = (lo(s_e0, s_step), lo(x.0, x.1));
                            let bits = |e| LLVMBuildAnd(b, e, self.c64(63), c"".as_ptr());
                            let blk = |p: LLVMValueRef, e| {
                                let k = LLVMBuildLShr(b, e, self.c64(6), c"".as_ptr());
                                self.gep(p, LLVMBuildShl(b, k, self.c64(5), c"".as_ptr()))
                            };
                            let (os, od) = (bits(ls), bits(ld));
                            let (ps, pd) = (blk(sp, ls), blk(dp, ld));
                            let fits = |o| {
                                let end = LLVMBuildAdd(b, o, m, c"".as_ptr());
                                self.icmp(IP::LLVMIntULE, end, self.c64(64))
                            };
                            // The blocks' bytes apart, as `exec` compares them.
                            let (is, id) = (
                                LLVMBuildPtrToInt(b, ps, self.i64, c"".as_ptr()),
                                LLVMBuildPtrToInt(b, pd, self.i64, c"".as_ptr()),
                            );
                            let before = |u, v| {
                                let end = LLVMBuildAdd(b, u, self.c64(32), c"".as_ptr());
                                self.icmp(IP::LLVMIntULE, end, v)
                            };
                            let unit = self.or(
                                self.icmp(IP::LLVMIntEQ, s_step, c1),
                                self.icmp(IP::LLVMIntEQ, s_step, self.c64(-1)),
                            );
                            let ok = and(
                                and(
                                    and(unit, self.icmp(IP::LLVMIntEQ, s_step, x.1)),
                                    and(fits(os), fits(od)),
                                ),
                                self.or(before(is, id), before(id, is)),
                            );
                            let (inl, call, join) = (self.block(c), self.block(c), self.block(c));
                            self.cond_br(ok, inl, call, true);
                            LLVMPositionBuilderAtEnd(b, inl);
                            // `m` is 2 to 64: `(2 << (m - 1)) - 1` is its mask.
                            let sh = LLVMBuildShl(
                                b,
                                self.c64(2),
                                LLVMBuildSub(b, m, c1, c"".as_ptr()),
                                c"".as_ptr(),
                            );
                            let mask = LLVMBuildSub(b, sh, c1, c"".as_ptr());
                            let hole = LLVMBuildNot(
                                b,
                                LLVMBuildShl(b, mask, od, c"".as_ptr()),
                                c"".as_ptr(),
                            );
                            for k in 0..4 {
                                let (qs, qd) =
                                    (self.gep(ps, self.c64(8 * k)), self.gep(pd, self.c64(8 * k)));
                                let w = LLVMBuildAnd(
                                    b,
                                    LLVMBuildLShr(b, self.load(self.i64, qs), os, c"".as_ptr()),
                                    mask,
                                    c"".as_ptr(),
                                );
                                let keep =
                                    LLVMBuildAnd(b, self.load(self.i64, qd), hole, c"".as_ptr());
                                self.store(self.or(keep, LLVMBuildShl(b, w, od, c"".as_ptr())), qd);
                            }
                            LLVMBuildBr(b, join);
                            LLVMPositionBuilderAtEnd(b, call);
                            (inl, join)
                        });
                        let mut args = [sp, s_e0, s_step, dp, x.0, x.1, code, m];
                        let mut done = LLVMBuildCall2(
                            b,
                            self.eloop_exec_ty,
                            self.eloop_exec,
                            args.as_mut_ptr(),
                            8,
                            c"".as_ptr(),
                        );
                        if let Some((inl, join)) = short {
                            let from = LLVMGetInsertBlock(b);
                            LLVMBuildBr(b, join);
                            LLVMPositionBuilderAtEnd(b, join);
                            let phi = LLVMBuildPhi(b, self.i64, c"".as_ptr());
                            LLVMAddIncoming(
                                phi,
                                [m, done].as_mut_ptr(),
                                [inl, from].as_mut_ptr(),
                                2,
                            );
                            done = phi;
                        }
                        let moved = LLVMBuildMul(b, done, step, c"".as_ptr());
                        self.st(c, l.ind, LLVMBuildAdd(b, ind, moved, c"".as_ptr()));
                        LLVMBuildBr(b, generic);
                        return;
                    }
                    _ => unreachable!("`invariant_steps` knows the operations"),
                };
                // The flags so far, tested here: a flag kept for the whole plan is a value live
                // across the blocks of every memory access after it.
                if !LLVMIsAConstantInt(bad).is_null() && LLVMConstIntGetZExtValue(bad) == 0 {
                } else {
                    self.guard(c, not(bad), generic);
                    bad = no;
                }
                val.push(r);
            }
        }
    }

    /// A private constant array of `i64`, 8-byte aligned.
    unsafe fn i64_table(&self, t: &[i64]) -> LLVMValueRef {
        // SAFETY: a global of our module with a matching initialiser.
        unsafe {
            let mut vals: Vec<LLVMValueRef> = t.iter().map(|&b| self.c64(b)).collect();
            let ty = LLVMArrayType2(self.i64, t.len() as u64);
            let g = LLVMAddGlobal(self.module, ty, c"".as_ptr());
            LLVMSetInitializer(
                g,
                LLVMConstArray2(self.i64, vals.as_mut_ptr(), t.len() as u64),
            );
            LLVMSetGlobalConstant(g, 1);
            LLVMSetAlignment(g, 8);
            LLVMSetLinkage(g, llvm_sys::LLVMLinkage::LLVMPrivateLinkage);
            g
        }
    }

    /// A private constant array of `u32`.
    unsafe fn u32_table(&self, t: &[u32]) -> LLVMValueRef {
        // SAFETY: a global of our module with a matching initialiser.
        unsafe {
            let mut vals: Vec<LLVMValueRef> = t.iter().map(|&b| self.c32(b)).collect();
            let ty = LLVMArrayType2(self.i32, t.len() as u64);
            let g = LLVMAddGlobal(self.module, ty, c"".as_ptr());
            LLVMSetInitializer(
                g,
                LLVMConstArray2(self.i32, vals.as_mut_ptr(), t.len() as u64),
            );
            LLVMSetGlobalConstant(g, 1);
            LLVMSetLinkage(g, llvm_sys::LLVMLinkage::LLVMPrivateLinkage);
            g
        }
    }

    /// A private global copy of constant `k` and its length, made once per function; `None`
    /// for a constant the interpreter expands (compact forms) or one over 4 KiB.
    unsafe fn const_global(&self, c: &Cur, k: yavs_ir::ConstId) -> Option<(LLVMValueRef, u64)> {
        let yavs_ir::ConstData::Bytes(b) = c.prog.consts.get(k)? else {
            return None;
        };
        if b.len() > 4096 {
            return None;
        }
        let mut m = c.consts.borrow_mut();
        // SAFETY: a global of our module (`table`).
        let g = *m.entry(k).or_insert_with(|| unsafe { self.table(b) });
        Some((g, b.len() as u64))
    }

    unsafe fn zext8(&self, v: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: v is an i8.
        unsafe { LLVMBuildZExt(self.b, v, self.i64, c"".as_ptr()) }
    }

    /// The address of the entry of `Mem::objs` of the object that `v` (an address, or with
    /// `handle` an access value) points to: `obj_at` of its high half, already scaled by the
    /// entry size. The index sits at bits 32 and up of both, and the shared-heap flag at bit 61
    /// of an address and bit 57 of an access value (`obj_keys_agree`); two shifts and masks
    /// whose chains run side by side, then an exclusive or.
    unsafe fn obj_entry(
        &self,
        c: &Cur,
        v: LLVMValueRef,
        handle: bool,
        mixs: LLVMValueRef,
    ) -> LLVMValueRef {
        // SAFETY: integer arithmetic on values of c.f; the entry lies inside `Mem`.
        unsafe {
            let idx_shift = 32 - OBJ_SHIFT;
            let flag_shift = 32 - OBJ_SHIFT
                + if handle {
                    KEY_SHIFT_HANDLE
                } else {
                    KEY_SHIFT_ADDR
                };
            let idx = LLVMBuildLShr(self.b, v, self.c64(i64::from(idx_shift)), c"".as_ptr());
            let idx = LLVMBuildAnd(
                self.b,
                idx,
                self.c64(((OBJS as i64) - 1) << OBJ_SHIFT),
                c"".as_ptr(),
            );
            let flag = LLVMBuildLShr(self.b, v, self.c64(i64::from(flag_shift)), c"".as_ptr());
            let flag = LLVMBuildAnd(
                self.b,
                flag,
                self.c64((KEY_FLAG as i64) << OBJ_SHIFT),
                c"".as_ptr(),
            );
            let off = LLVMBuildXor(self.b, idx, flag, c"".as_ptr());
            let off = LLVMBuildXor(self.b, off, mixs, c"".as_ptr());
            let off = LLVMBuildAdd(self.b, off, self.c64(MEM_OBJS as i64), c"".as_ptr());
            self.gep(c.mem, off)
        }
    }

    /// The tag bits and the entry offset of the keys `Mem::slot` gives an object (`MEM_TAGS`,
    /// `MEM_MIXES`, by address space): of a heap object, or with `hi` of the space of the address
    /// whose high half that is.
    unsafe fn obj_tag(&self, c: &Cur, hi: Option<LLVMValueRef>) -> (LLVMValueRef, LLVMValueRef) {
        // SAFETY: loads of `Mem` fields (eight words each, `hi` is below 2^32) and integer
        // arithmetic.
        unsafe {
            let Some(hi) = hi else {
                let k = 8 * u64::from(space::HEAP);
                return (
                    self.mem_ld(c, MEM_TAGS + k, self.i64),
                    self.mem_ld(c, MEM_MIXES + k, self.i64),
                );
            };
            let sp = LLVMBuildLShr(self.b, hi, self.c64(29), c"".as_ptr());
            let k = LLVMBuildShl(self.b, sp, self.c64(3), c"".as_ptr());
            let at = |base: u64| {
                let off = LLVMBuildAdd(self.b, k, self.c64(base as i64), c"".as_ptr());
                self.load(self.i64, self.gep(c.mem, off))
            };
            (at(MEM_TAGS), at(MEM_MIXES))
        }
    }

    unsafe fn mem_ptr(&self, c: &Cur, field: u64) -> LLVMValueRef {
        // SAFETY: `field` is an offset in `Mem`.
        unsafe { self.gep(c.mem, self.c64(field as i64)) }
    }

    /// Whether `n` bytes at offset `o` lie within `len` bytes (`o + n <= len`, as an `i1`). For a
    /// constant `n` the sum cannot wrap (`o` is under 2^63 + 2^32: a 32-bit offset plus an
    /// `extra` under 2^63, see `host`), so one add and one compare do for the two compares and
    /// the subtraction that a dynamic `n` needs.
    unsafe fn in_range(&self, o: LLVMValueRef, n: LLVMValueRef, len: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: integer arithmetic on i64 values.
        unsafe {
            if !LLVMIsAConstantInt(n).is_null() && LLVMConstIntGetZExtValue(n) < 1 << 31 {
                let end = LLVMBuildNUWAdd(self.b, o, n, c"".as_ptr());
                return self.icmp(IP::LLVMIntULE, end, len);
            }
            let fits = self.icmp(IP::LLVMIntULE, n, len);
            let rest = LLVMBuildSub(self.b, len, n, c"".as_ptr());
            let inside = self.icmp(IP::LLVMIntULE, o, rest);
            LLVMBuildAnd(self.b, fits, inside, c"".as_ptr())
        }
    }

    unsafe fn mem_ld(&self, c: &Cur, field: u64, ty: LLVMTypeRef) -> LLVMValueRef {
        // SAFETY: the field at `field` has type `ty`.
        unsafe { LLVMBuildLoad2(self.b, ty, self.mem_ptr(c, field), c"".as_ptr()) }
    }

    unsafe fn mem_st(&self, c: &Cur, field: u64, v: LLVMValueRef) {
        // SAFETY: the field at `field` has the type of `v`.
        unsafe {
            LLVMBuildStore(self.b, v, self.mem_ptr(c, field));
        }
    }

    unsafe fn gep(&self, p: LLVMValueRef, off: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: byte offset from a pointer.
        unsafe {
            let mut idx = [off];
            let g = LLVMBuildGEP2(self.b, self.i8, p, idx.as_mut_ptr(), 1, c"".as_ptr());
            let k = self.kinds.borrow().get(&(p as usize)).copied();
            if let Some(k) = k {
                self.tag(g, k);
            }
            g
        }
    }

    /// Remember that `p` points into memory of kind `k` (`AA_*`).
    fn tag(&self, p: LLVMValueRef, k: usize) -> LLVMValueRef {
        self.kinds.borrow_mut().insert(p as usize, k);
        p
    }

    /// Alias scope metadata on the access `inst` through `p`, if its kind is known.
    unsafe fn scope(&self, inst: LLVMValueRef, p: LLVMValueRef) {
        let Some(k) = self.kinds.borrow().get(&(p as usize)).copied() else {
            return;
        };
        // SAFETY: metadata nodes of our context attached to an instruction of our module.
        unsafe {
            let mut aa = self.aa.borrow_mut();
            let n = aa.get_or_insert_with(|| {
                let md = |ops: &mut [LLVMMetadataRef]| {
                    LLVMMDNodeInContext2(self.ctx, ops.as_mut_ptr(), ops.len())
                };
                let name = |s: &str| LLVMMDStringInContext2(self.ctx, s.as_ptr().cast(), s.len());
                let domain = md(&mut [name("yavs memory")]);
                let names = [
                    "slots", "temp", "inst", "const", "codes", "spin", "stop", "mem",
                ];
                let scopes: Vec<LLVMMetadataRef> =
                    names.iter().map(|n| md(&mut [name(n), domain])).collect();
                let v = |m| LLVMMetadataAsValue(self.ctx, m);
                let lists = |ix: &[usize]| {
                    let mut ops: Vec<LLVMMetadataRef> = ix.iter().map(|&i| scopes[i]).collect();
                    v(md(&mut ops))
                };
                AaNodes {
                    scope: std::array::from_fn(|i| lists(&[i])),
                    others: std::array::from_fn(|i| {
                        let ix: Vec<usize> = (0..AA_N).filter(|&j| j != i).collect();
                        lists(&ix)
                    }),
                    k_scope: LLVMGetMDKindIDInContext(self.ctx, c"alias.scope".as_ptr(), 11),
                    k_noalias: LLVMGetMDKindIDInContext(self.ctx, c"noalias".as_ptr(), 7),
                }
            });
            LLVMSetMetadata(inst, n.k_scope, n.scope[k]);
            LLVMSetMetadata(inst, n.k_noalias, n.others[k]);
        }
    }

    /// An unaligned load.
    unsafe fn load(&self, ty: LLVMTypeRef, p: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: `p` points to a value of `ty`.
        unsafe {
            let v = LLVMBuildLoad2(self.b, ty, p, c"".as_ptr());
            LLVMSetAlignment(v, 1);
            self.scope(v, p);
            v
        }
    }

    /// An unaligned store.
    unsafe fn store(&self, v: LLVMValueRef, p: LLVMValueRef) {
        // SAFETY: `p` points to memory for a value of `v`'s type.
        unsafe {
            let s = LLVMBuildStore(self.b, v, p);
            LLVMSetAlignment(s, 1);
            self.scope(s, p);
        }
    }

    fn width_ty(&self, w: Width) -> LLVMTypeRef {
        match w {
            Width::B1 => self.i8,
            Width::B4 => self.i32,
            _ => self.i64,
        }
    }

    /// The host pointer to `n` bytes at address `a` plus `extra`, taking `slow` unless they
    /// lie in the frame's slots (inline when the generator knows the address), a constant it
    /// has a copy of, or memory `yavs_rt_host` finds: what the interpreter's `rd`/`wr` would
    /// find there without an error. `at` is what the generator knows about `a`: an offset in the
    /// frame's slots, or in a constant, whose bytes compiled code then reads from a copy of
    /// its own (constants never change).
    #[allow(clippy::too_many_arguments)]
    unsafe fn host(
        &self,
        c: &Cur,
        a: LLVMValueRef,
        at: Option<Known>,
        extra: LLVMValueRef,
        n: LLVMValueRef,
        write: bool,
        slow: LLVMBasicBlockRef,
    ) -> LLVMValueRef {
        // SAFETY: loads of the pinned region, integer arithmetic, well-formed blocks of c.f.
        unsafe {
            // An address parameter (or an `Index` of one) of a native frame: its region was resolved
            // at the entry, so a test against the held pair replaces the lookup. What fails the test
            // (another space, a write to a region resolved for reading, an address outside) takes the
            // ordinary lookup.
            if let (Some(Known::Param(k)), Some(_)) = (at, &c.nat)
                && let Some(r) = c.pres.borrow().get(usize::from(k)).copied()
                && (r.write || !write)
            {
                let lo = LLVMBuildAnd(self.b, a, self.c64(0xffff_ffff), c"".as_ptr());
                let konst = |v: LLVMValueRef| {
                    (!LLVMIsAConstantInt(v).is_null()).then(|| LLVMConstIntGetZExtValue(v))
                };
                let (off, ok) = match (konst(extra), konst(n)) {
                    // `lo` is below 2^32: the sums cannot wrap.
                    (Some(e), Some(k)) if e < 1 << 31 && k < 1 << 31 => {
                        let off = LLVMBuildAdd(self.b, lo, extra, c"".as_ptr());
                        let end = LLVMBuildAdd(self.b, lo, self.c64((e + k) as i64), c"".as_ptr());
                        (off, self.icmp(IP::LLVMIntULE, end, r.len))
                    }
                    _ => {
                        // `extra` is below 2^63, the sum cannot wrap.
                        let off = LLVMBuildAdd(self.b, lo, extra, c"".as_ptr());
                        let fits = self.icmp(IP::LLVMIntULE, n, r.len);
                        let rest = LLVMBuildSub(self.b, r.len, n, c"".as_ptr());
                        let inside = self.icmp(IP::LLVMIntULE, off, rest);
                        (off, LLVMBuildAnd(self.b, fits, inside, c"".as_ptr()))
                    }
                };
                let (fast, miss, join) = (self.block(c), self.block(c), self.block(c));
                self.cond_br(ok, fast, miss, true);
                LLVMPositionBuilderAtEnd(self.b, fast);
                let pf = self.gep(r.ptr, off);
                LLVMBuildBr(self.b, join);
                LLVMPositionBuilderAtEnd(self.b, miss);
                // What the entry could not hold is mostly an argument in the temporary area.
                let pm = self.host_lookup(c, a, Some(Known::LikelyTemp), extra, n, write, slow);
                let from = LLVMGetInsertBlock(self.b);
                LLVMBuildBr(self.b, join);
                LLVMPositionBuilderAtEnd(self.b, join);
                let phi = LLVMBuildPhi(self.b, self.ptr, c"".as_ptr());
                LLVMAddIncoming(phi, &mut { pf }, &mut { fast }, 1);
                LLVMAddIncoming(phi, &mut { pm }, &mut { from }, 1);
                return phi;
            }
            self.host_lookup(c, a, at, extra, n, write, slow)
        }
    }

    /// `host` for an address of the space and region the generator does not hold.
    #[allow(clippy::too_many_arguments)]
    unsafe fn host_lookup(
        &self,
        c: &Cur,
        a: LLVMValueRef,
        at: Option<Known>,
        extra: LLVMValueRef,
        n: LLVMValueRef,
        write: bool,
        slow: LLVMBasicBlockRef,
    ) -> LLVMValueRef {
        // SAFETY: loads of `Mem` fields, integer arithmetic, well-formed blocks of c.f.
        unsafe {
            // (A native frame finds its own slots by their address: a guess about heap is no use.)
            let at = if c.nat.is_some() && matches!(at, Some(Known::LikelyHeap)) {
                None
            } else {
                at
            };
            if let Some(Known::Const(k, o)) = at {
                match self.const_global(c, k) {
                    // A write to a constant is the interpreter's error.
                    Some((g, len)) if !write => {
                        let o = LLVMBuildAdd(self.b, self.c64(o as i64), extra, c"".as_ptr());
                        let len = self.c64(len as i64);
                        self.guard(c, self.in_range(o, n, len), slow);
                        return self.gep(self.tag(g, AA_CONST), o);
                    }
                    _ => {}
                }
            }
            // An object a `Deref` found in this stretch of straight-line code: its bytes are in
            // hand, no lookup.
            if let Some(Known::Heap(Some(o))) = at {
                let off = LLVMBuildAdd(self.b, o.off, extra, c"".as_ptr());
                self.guard(c, self.in_range(off, n, o.len), slow);
                return self.gep(o.ptr, off);
            }
            if let Some(Known::Frame(o)) = at {
                // Within the frame whatever the frame: no check.
                if !LLVMIsAConstantInt(extra).is_null() && !LLVMIsAConstantInt(n).is_null() {
                    let (e, k) = (LLVMConstIntGetZExtValue(extra), LLVMConstIntGetZExtValue(n));
                    if o.checked_add(e)
                        .and_then(|x| x.checked_add(k))
                        .is_some_and(|end| end <= c.frame_len)
                    {
                        let at = self.c64((o + e) as i64);
                        return self.gep(self.tag(self.own_slots_cached(c), AA_SLOTS), at);
                    }
                }
                // `extra` is below 2^63, the sum cannot wrap.
                let o = LLVMBuildAdd(self.b, self.c64(o as i64), extra, c"".as_ptr());
                let len = match &c.nat {
                    Some(_) => self.c64(c.frame_len as i64),
                    None => self.mem_ld(c, MEM_SLOTS_LEN, self.i64),
                };
                self.guard(c, self.in_range(o, n, len), slow);
                return self.gep(self.tag(self.own_slots_cached(c), AA_SLOTS), o);
            }
            // A variable of an enclosing subprogram, from the view (frame-mode code; a native
            // frame finds the frames of its callers below).
            if let (Some(Known::Up(d)), None) = (at, &c.nat) {
                let k = 8 * u64::from(d - 1);
                let base = self.mem_ld(c, MEM_UP_SLOTS + k, self.ptr);
                let len = self.mem_ld(c, MEM_UP_LEN + k, self.i64);
                let lo = LLVMBuildAnd(self.b, a, self.c64(0xffff_ffff), c"".as_ptr());
                let o = LLVMBuildAdd(self.b, lo, extra, c"".as_ptr());
                self.guard(c, self.in_range(o, n, len), slow);
                return self.gep(base, o);
            }
            // The temporary area and (reading) the instance data, whose space the generator
            // knows: the bytes the view holds, bounds checked, no call.
            let area = match at {
                Some(Known::Temp) => Some((MEM_TEMP, MEM_TEMP_LEN)),
                Some(Known::Inst(_) | Known::InstAny) if !write => Some((MEM_INST, MEM_INST_LEN)),
                _ => None,
            };
            if let Some((base, len)) = area {
                let lo = LLVMBuildAnd(self.b, a, self.c64(0xffff_ffff), c"".as_ptr());
                let o = LLVMBuildAdd(self.b, lo, extra, c"".as_ptr());
                let len = self.mem_ld(c, len, self.i64);
                self.guard(c, self.in_range(o, n, len), slow);
                let kind = if base == MEM_TEMP { AA_TEMP } else { AA_INST };
                return self.gep(self.tag(self.mem_ld(c, base, self.ptr), kind), o);
            }
            // An address that is probably in the temporary area (a call's result, the words loaded
            // from it) gets that one test first (#711); a miss asks the runtime as ever, or in a
            // native frame takes the frame and object tests below.
            let likely = matches!(at, Some(Known::LikelyTemp));
            // The same for the object cache: a heap object the code found here with a `Deref`, or
            // a pointer loaded from a variable's slot (a composite variable's object).
            let likely_heap =
                c.nat.is_none() && matches!(at, Some(Known::LikelyHeap | Known::Heap(None)));
            let direct = c.nat.is_none() && !likely && !likely_heap;
            let mut found = Vec::new();
            let merge_at = self.block(c);
            if likely {
                let hi = LLVMBuildLShr(self.b, a, self.c64(32), c"".as_ptr());
                let lo = LLVMBuildAnd(self.b, a, self.c64(0xffff_ffff), c"".as_ptr());
                let o = LLVMBuildAdd(self.b, lo, extra, c"".as_ptr());
                let hit = self.icmp(IP::LLVMIntEQ, hi, self.c64(i64::from(space::TEMP) << 29));
                let len = self.mem_ld(c, MEM_TEMP_LEN, self.i64);
                let ok = LLVMBuildAnd(self.b, hit, self.in_range(o, n, len), c"".as_ptr());
                let (fast, miss) = (self.block(c), self.block(c));
                self.cond_br(ok, fast, miss, true);
                LLVMPositionBuilderAtEnd(self.b, fast);
                let pf = self.gep(self.tag(self.mem_ld(c, MEM_TEMP, self.ptr), AA_TEMP), o);
                LLVMBuildBr(self.b, merge_at);
                found.push((pf, fast));
                LLVMPositionBuilderAtEnd(self.b, miss);
            }
            // A native frame's own slots are found here: the interpreter has no frame for it.
            let heap = matches!(at, Some(Known::Heap(_) | Known::LikelyHeap));
            let (own, other, merge) = match &c.nat {
                // A closed frame is asked for by no address of unknown space (`closed_frame`): its
                // slots stay out of the test of its own frame, so they may live in registers; the
                // frames of its native callers (by-reference parameters) are found as ever.
                Some(nat) if !heap => {
                    let (other, merge) = (self.block(c), self.block(c));
                    let hi = LLVMBuildLShr(self.b, a, self.c64(32), c"".as_ptr());
                    let mut own_v = Vec::new();
                    if nat.closed.is_some() {
                        LLVMBuildBr(self.b, other);
                    } else {
                        let own = self.block(c);
                        LLVMBuildCondBr(self.b, self.icmp(IP::LLVMIntEQ, hi, nat.fhi), own, other);
                        LLVMPositionBuilderAtEnd(self.b, own);
                        let lo = LLVMBuildAnd(self.b, a, self.c64(0xffff_ffff), c"".as_ptr());
                        let o = LLVMBuildAdd(self.b, lo, extra, c"".as_ptr());
                        let len = self.c64(c.frame_len as i64);
                        self.guard(c, self.in_range(o, n, len), slow);
                        let p = self.gep(nat.slots, o);
                        let from = LLVMGetInsertBlock(self.b);
                        LLVMBuildBr(self.b, merge);
                        own_v.push((p, from));
                    }
                    LLVMPositionBuilderAtEnd(self.b, other);
                    // The slots of a native caller, registered at its entry: a frame above the
                    // interpreter's top frame and below this one.
                    let (up, rest_blk) = (self.block(c), self.block(c));
                    let hi = LLVMBuildLShr(self.b, a, self.c64(32), c"".as_ptr());
                    let top = self.mem_ld(c, MEM_FRAME_IDX, self.i64);
                    let above = self.icmp(IP::LLVMIntUGT, hi, top);
                    let below = self.icmp(IP::LLVMIntULT, hi, nat.depth);
                    LLVMBuildCondBr(
                        self.b,
                        LLVMBuildAnd(self.b, above, below, c"".as_ptr()),
                        up,
                        rest_blk,
                    );
                    LLVMPositionBuilderAtEnd(self.b, up);
                    let tab = self.mem_ld(c, MEM_NSLOTS, self.ptr);
                    let ent = self.gep(tab, LLVMBuildMul(self.b, hi, self.c64(16), c"".as_ptr()));
                    let sp = self.load(self.ptr, ent);
                    let len = self.load(self.i64, self.gep(ent, self.c64(8)));
                    let lo = LLVMBuildAnd(self.b, a, self.c64(0xffff_ffff), c"".as_ptr());
                    let o = LLVMBuildAdd(self.b, lo, extra, c"".as_ptr());
                    self.guard(c, self.in_range(o, n, len), slow);
                    let pu = self.gep(sp, o);
                    let from_up = LLVMGetInsertBlock(self.b);
                    LLVMBuildBr(self.b, merge);
                    LLVMPositionBuilderAtEnd(self.b, rest_blk);
                    own_v.push((pu, from_up));
                    (own_v, other, merge)
                }
                _ => (Vec::new(), std::ptr::null_mut(), std::ptr::null_mut()),
            };
            // Other memory. Frame-mode code asks `yavs_rt_host` at once (`Mem::find`: the object's
            // entry of `Mem::objs`, the temporary area, the instance data, the frame, then the
            // interpreter): the same lookup once in the runtime instead of a chain of tests at
            // every access, which was most of the code of a process and a branch site for each test
            // (compiled clocked RTL -5 to -10 percent cycles, #634). A native frame keeps the
            // chain inline (first the object's entry, then the temporary area and, reading, the
            // instance data, then `yavs_rt_host`): its accesses are the hot loops of library code
            // (#432, #510).
            // Frame-mode code asks `yavs_rt_host` (the same lookup, once, in the runtime): the inline
            // chain of four tests per access was most of the code of a process and branch sites
            // the front end paid for (#634); native frames keep it.
            if !direct && (!likely || c.nat.is_some()) {
                let hi = LLVMBuildLShr(self.b, a, self.c64(32), c"".as_ptr());
                let lo = LLVMBuildAnd(self.b, a, self.c64(0xffff_ffff), c"".as_ptr());
                let o = LLVMBuildAdd(self.b, lo, extra, c"".as_ptr());
                let (tk, mx) = self.obj_tag(c, (!heap).then_some(hi));
                let e = self.obj_entry(c, a, false, mx);
                let key = LLVMBuildOr(self.b, hi, tk, c"".as_ptr());
                let mut hit = self.icmp(IP::LLVMIntEQ, self.load(self.i64, e), key);
                if write {
                    let w = self.load(self.i64, self.gep(e, self.c64(OBJ_W as i64)));
                    let w = self.icmp(IP::LLVMIntNE, w, self.c64(0));
                    hit = LLVMBuildAnd(self.b, hit, w, c"".as_ptr());
                }
                let base = self.load(self.ptr, self.gep(e, self.c64(OBJ_PTR as i64)));
                let len = self.load(self.i64, self.gep(e, self.c64(OBJ_LEN as i64)));
                // The object, then on a miss the temporary area and (reading) the instance data:
                // the view's bytes, found by the address's high half (array descriptors and
                // planes a caller made as temporaries, #527).
                // (Each test is made in the block of its own try: made all at once at the top, the
                // results are values alive across the blocks, in stack slots.)
                let at = |sp: u8| self.icmp(IP::LLVMIntEQ, hi, self.c64(i64::from(sp) << 29));
                let mut tries = vec![None];
                if !heap {
                    // The interpreter's top frame: the function's own (the address of a layout
                    // the frame keeps), or for a native frame the frame its chain started in
                    // (the variables of the process or function that called into compiled
                    // code, reached through by-reference parameters).
                    tries.push(Some((space::FRAME, MEM_SLOTS, MEM_SLOTS_LEN)));
                    tries.push(Some((space::TEMP, MEM_TEMP, MEM_TEMP_LEN)));
                    if !write {
                        tries.push(Some((space::INST, MEM_INST, MEM_INST_LEN)));
                    }
                }
                let mut hit = Some(hit);
                for view in tries {
                    let (hit, base, len) = match view {
                        None => (hit.take().expect("the object's test"), base, len),
                        Some((sp, b, l)) => (
                            if sp == space::FRAME {
                                self.icmp(IP::LLVMIntEQ, hi, self.mem_ld(c, MEM_FRAME, self.i64))
                            } else {
                                at(sp)
                            },
                            self.mem_ld(c, b, self.ptr),
                            self.mem_ld(c, l, self.i64),
                        ),
                    };
                    let ok = LLVMBuildAnd(self.b, hit, self.in_range(o, n, len), c"".as_ptr());
                    let (fast, miss) = (self.block(c), self.block(c));
                    self.cond_br(ok, fast, miss, true);
                    LLVMPositionBuilderAtEnd(self.b, fast);
                    let pf = self.gep(base, o);
                    LLVMBuildBr(self.b, merge_at);
                    found.push((pf, fast));
                    LLVMPositionBuilderAtEnd(self.b, miss);
                }
            }
            let mut args = [c.env, c.mem, a, extra, n, self.c32(u32::from(write))];
            // The lookup in the objects `Mem` keeps failed: the rare case.
            if !direct {
                c.cold.borrow_mut().push(LLVMGetInsertBlock(self.b));
            }
            let phi = LLVMBuildCall2(
                self.b,
                self.host_ty,
                self.host,
                args.as_mut_ptr(),
                6,
                c"".as_ptr(),
            );
            if !direct {
                self.attr(phi, true, "cold", 0);
            }
            self.guard(c, LLVMBuildIsNotNull(self.b, phi, c"".as_ptr()), slow);
            if !direct {
                c.cold.borrow_mut().push(LLVMGetInsertBlock(self.b));
            }
            let mut p = phi;
            {
                let here = LLVMGetInsertBlock(self.b);
                LLVMBuildBr(self.b, merge_at);
                LLVMPositionBuilderAtEnd(self.b, merge_at);
                let m = LLVMBuildPhi(self.b, self.ptr, c"".as_ptr());
                let mut vals: Vec<LLVMValueRef> = found.iter().map(|f| f.0).collect();
                let mut bbs: Vec<LLVMBasicBlockRef> = found.iter().map(|f| f.1).collect();
                vals.push(p);
                bbs.push(here);
                LLVMAddIncoming(m, vals.as_mut_ptr(), bbs.as_mut_ptr(), vals.len() as u32);
                p = m;
            }
            if !own.is_empty() {
                let here = LLVMGetInsertBlock(self.b);
                LLVMBuildBr(self.b, merge);
                LLVMPositionBuilderAtEnd(self.b, merge);
                let phi = LLVMBuildPhi(self.b, self.ptr, c"".as_ptr());
                let mut vals: Vec<LLVMValueRef> = own.iter().map(|o| o.0).collect();
                let mut bbs: Vec<LLVMBasicBlockRef> = own.iter().map(|o| o.1).collect();
                vals.push(p);
                bbs.push(here);
                LLVMAddIncoming(phi, vals.as_mut_ptr(), bbs.as_mut_ptr(), vals.len() as u32);
                p = phi;
            }
            let _ = other;
            p
        }
    }

    /// `n` zeroed bytes of temporary memory as `Machine::temp_alloc` makes them: the address
    /// and the host pointer, taking `slow` when the area must grow or exceeds 4 GiB.
    unsafe fn temp_alloc(
        &self,
        c: &Cur,
        n: LLVMValueRef,
        slow: LLVMBasicBlockRef,
    ) -> (LLVMValueRef, LLVMValueRef) {
        // SAFETY: integer arithmetic on `Mem` fields; the zeroed bytes lie within the capacity.
        unsafe {
            let len = self.mem_ld(c, MEM_TEMP_LEN, self.i64);
            let off = LLVMBuildAdd(self.b, len, self.c64(7), c"".as_ptr());
            let off = LLVMBuildAnd(self.b, off, self.c64(!7), c"".as_ptr());
            let cap = self.mem_ld(c, MEM_TEMP_CAP, self.i64);
            let end = if !LLVMIsAConstantInt(n).is_null() && LLVMConstIntGetZExtValue(n) < 1 << 31 {
                // `off` is under 2^32 (the capacity), so the sum cannot wrap.
                let end = LLVMBuildNUWAdd(self.b, off, n, c"".as_ptr());
                self.guard(c, self.icmp(IP::LLVMIntULE, end, cap), slow);
                end
            } else {
                let (end, o) = self.ovf(self.uadd, off, n);
                let big = self.icmp(IP::LLVMIntUGT, end, cap);
                self.guard(c, LLVMBuildNot(self.b, self.or(o, big), c"".as_ptr()), slow);
                end
            };
            let temp = self.tag(self.mem_ld(c, MEM_TEMP, self.ptr), AA_TEMP);
            let at = self.gep(temp, len);
            let fill = LLVMBuildSub(self.b, end, len, c"".as_ptr());
            if LLVMIsAConstantInt(n).is_null() {
                LLVMBuildMemSet(self.b, at, LLVMConstInt(self.i8, 0, 0), fill, 1);
            } else {
                // A known size at an aligned end is zeroed by stores, not a `memset` call.
                let (even, odd, merge) = (self.block(c), self.block(c), self.block(c));
                LLVMBuildCondBr(self.b, self.icmp(IP::LLVMIntEQ, off, len), even, odd);
                LLVMPositionBuilderAtEnd(self.b, even);
                LLVMBuildMemSet(self.b, at, LLVMConstInt(self.i8, 0, 0), n, 1);
                LLVMBuildBr(self.b, merge);
                LLVMPositionBuilderAtEnd(self.b, odd);
                LLVMBuildMemSet(self.b, at, LLVMConstInt(self.i8, 0, 0), fill, 1);
                LLVMBuildBr(self.b, merge);
                LLVMPositionBuilderAtEnd(self.b, merge);
            }
            self.mem_st(c, MEM_TEMP_LEN, end);
            let tag = self.c64((u64::from(space::TEMP) << 61) as i64);
            (
                LLVMBuildOr(self.b, tag, off, c"".as_ptr()),
                self.gep(temp, off),
            )
        }
    }

    /// The words `LayoutOf` writes for a layout known up to the descriptor's lengths,
    /// taking `slow` wherever `Ty::layout` would fail.
    unsafe fn layout_words(
        &self,
        c: &Cur,
        sl: &StaticLayout,
        desc: Reg,
        at: Option<Known>,
        slow: LLVMBasicBlockRef,
    ) -> Vec<LLVMValueRef> {
        // SAFETY: integer arithmetic on descriptor words read through `host`.
        unsafe {
            let dims = match sl {
                StaticLayout::Words(_) => 0,
                StaticLayout::Array { dims, .. } => *dims,
            };
            // The interpreter reads the descriptor even when it has no bytes.
            let n = self.c64(i64::from(dims) * 24);
            let d = self.ld(c, desc);
            let p = self.host(c, d, at, self.c64(0), n, false, slow);
            let (el, logic) = match sl {
                StaticLayout::Words(w) => return w.iter().map(|&x| self.c64(x as i64)).collect(),
                StaticLayout::Array { elem, logic, .. } => (elem, *logic),
            };
            let not = |x| LLVMBuildNot(self.b, x, c"".as_ptr());
            let and = |a, b| LLVMBuildAnd(self.b, a, b, c"".as_ptr());
            let mut count = self.c64(1);
            let mut ok = LLVMConstInt(LLVMInt1TypeInContext(self.ctx), 1, 0);
            for k in 0..i64::from(dims) {
                let (l, r, dir) = self.dope_words(self.gep(p, self.c64(k * 24)));
                let dir_ok = self.icmp(IP::LLVMIntULE, dir, self.c64(1));
                let (len, fits) = self.len_of(l, r, dir);
                let (m, o) = self.ovf(self.umul, count, len);
                count = m;
                ok = and(ok, and(and(dir_ok, fits), not(o)));
            }
            let (count, size, align, stride) = if logic {
                let (n, o) = self.ovf(self.umul, count, self.c64(el.count as i64));
                ok = and(ok, not(o));
                // div_ceil(n, 64) * 32 cannot overflow.
                let blocks = LLVMBuildLShr(self.b, n, self.c64(6), c"".as_ptr());
                let rest = LLVMBuildAnd(self.b, n, self.c64(63), c"".as_ptr());
                let part = self.icmp(IP::LLVMIntNE, rest, self.c64(0));
                let blocks = LLVMBuildAdd(self.b, blocks, self.zext(part), c"".as_ptr());
                let size = LLVMBuildShl(self.b, blocks, self.c64(5), c"".as_ptr());
                (n, size, self.c64(8), self.c64(0))
            } else {
                let stride = el.size.next_multiple_of(el.align.max(1));
                let (size, o) = self.ovf(self.umul, self.c64(stride as i64), count);
                ok = and(ok, not(o));
                (
                    count,
                    size,
                    self.c64(el.align as i64),
                    self.c64(stride as i64),
                )
            };
            self.guard(c, ok, slow);
            vec![size, align, stride, count]
        }
    }

    /// `yavs_ir::dim_len(l, r, dir == 1)` and whether it is not `None`.
    unsafe fn len_of(
        &self,
        l: LLVMValueRef,
        r: LLVMValueRef,
        dir: LLVMValueRef,
    ) -> (LLVMValueRef, LLVMValueRef) {
        // SAFETY: integer arithmetic.
        unsafe {
            let down = self.icmp(IP::LLVMIntEQ, dir, self.c64(1));
            let lo = LLVMBuildSelect(self.b, down, r, l, c"".as_ptr());
            let hi = LLVMBuildSelect(self.b, down, l, r, c"".as_ptr());
            let empty = self.icmp(IP::LLVMIntSLT, hi, lo);
            // hi - lo is exact as a u64 when hi >= lo; + 1 wraps only for the full i64 range.
            let d = LLVMBuildSub(self.b, hi, lo, c"".as_ptr());
            let n = LLVMBuildAdd(self.b, d, self.c64(1), c"".as_ptr());
            let fits = self.or(empty, self.icmp(IP::LLVMIntNE, n, self.c64(0)));
            (
                LLVMBuildSelect(self.b, empty, self.c64(0), n, c"".as_ptr()),
                fits,
            )
        }
    }

    /// The 32-byte bit-plane block holding element `idx` of the logic array at `base`, and
    /// the element's bit position in each plane word.
    #[allow(clippy::too_many_arguments)]
    unsafe fn logic_word(
        &self,
        c: &Cur,
        base: Reg,
        at: Option<Known>,
        idx: Reg,
        write: bool,
        slow: LLVMBasicBlockRef,
    ) -> (LLVMValueRef, LLVMValueRef) {
        // SAFETY: integer arithmetic and `host`.
        unsafe {
            let i = self.ld(c, idx);
            let blk = LLVMBuildLShr(self.b, i, self.c64(6), c"".as_ptr());
            let off = LLVMBuildShl(self.b, blk, self.c64(5), c"".as_ptr());
            let p = self.host(c, self.ld(c, base), at, off, self.c64(32), write, slow);
            (p, LLVMBuildAnd(self.b, i, self.c64(63), c"".as_ptr()))
        }
    }

    /// `left, right, dir` of the descriptor dimension at `p`.
    unsafe fn dope_words(&self, p: LLVMValueRef) -> (LLVMValueRef, LLVMValueRef, LLVMValueRef) {
        // SAFETY: `p` points to 24 readable bytes.
        unsafe {
            (
                self.load(self.i64, p),
                self.load(self.i64, self.gep(p, self.c64(8))),
                self.load(self.i64, self.gep(p, self.c64(16))),
            )
        }
    }

    unsafe fn fabs(&self, x: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: the intrinsic takes one double.
        unsafe {
            let id = LLVMLookupIntrinsicID(c"llvm.fabs".as_ptr(), 9);
            let mut tys = [self.f64];
            let ty = LLVMIntrinsicGetType(self.ctx, id, tys.as_mut_ptr(), 1);
            let f = LLVMGetIntrinsicDeclaration(self.module, id, tys.as_mut_ptr(), 1);
            let mut args = [x];
            LLVMBuildCall2(self.b, ty, f, args.as_mut_ptr(), 1, c"".as_ptr())
        }
    }
}

impl Drop for Gen {
    fn drop(&mut self) {
        // SAFETY: we own the builder.
        unsafe { LLVMDisposeBuilder(self.b) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The entry of an object found through its address (`obj_at`) is the entry of its access
    /// value (`handle_at`, what `obj_entry` builds for a `Deref`), private and shared heaps.
    #[test]
    fn obj_keys_agree() {
        for idx in [
            0u64, 1, 7, 8, 15, 16, 255, 256, 257, 1000, 65_535, 0xff_ffff,
        ] {
            for (space, tag) in [(space::HEAP, 1u64), (space::SHARED_HEAP, 2)] {
                let addr_hi = u64::from(space) << 29 | idx;
                let handle_hi = tag << 24 | idx;
                assert_eq!(obj_at(addr_hi), handle_at(handle_hi), "idx {idx} tag {tag}");
            }
            // A private and a shared object of one index get two entries.
            assert_ne!(
                obj_at(u64::from(space::HEAP) << 29 | idx),
                obj_at(u64::from(space::SHARED_HEAP) << 29 | idx)
            );
        }
    }

    /// The key `obj_entry` builds (shifts and masks of the whole address or access value)
    /// is `obj_at` of the high half, scaled by the entry size.
    #[test]
    fn obj_entry_expression_is_obj_at() {
        let expr = |v: u64, handle: bool| {
            let idx = (v >> (32 - OBJ_SHIFT)) & (((OBJS as u64) - 1) << OBJ_SHIFT);
            let sh = 32 - OBJ_SHIFT
                + if handle {
                    KEY_SHIFT_HANDLE
                } else {
                    KEY_SHIFT_ADDR
                };
            (idx ^ ((v >> sh) & (KEY_FLAG << OBJ_SHIFT))) >> OBJ_SHIFT
        };
        for idx in [0u64, 1, 7, 8, 15, 255, 256, 1000, 0xff_ffff] {
            for off in [0u64, 24, 0xffff_ffff] {
                for (space, tag) in [(space::HEAP, 1u64), (space::SHARED_HEAP, 2)] {
                    let addr = (u64::from(space) << 29 | idx) << 32 | off;
                    assert_eq!(expr(addr, false) as usize, obj_at(addr >> 32));
                    let handle = tag << 56 | idx << 32 | 0xdead_beef;
                    assert_eq!(expr(handle, true) as usize, obj_at(addr >> 32));
                }
            }
        }
    }

    /// A frame whose address goes into the operand list of a call is not closed; the callee
    /// that only loads and stores through its own parameter slot is.
    #[test]
    fn closed_frames_exclude_addresses_in_operand_lists() {
        use yavs_ir::{FuncBuilder, Mode, ParamClass, ProgramBuilder, RegTy};
        let mut pb = ProgramBuilder::new();
        let leaf = pb.declare("leaf", FuncKind::Function, None);
        let top = pb.declare("top", FuncKind::Function, None);
        let mut fb: FuncBuilder<'_> = pb.define(leaf);
        let s = fb.param("p", Width::B8, ParamClass::Value, Mode::In, None);
        fb.stmt(
            yavs_ir::StmtKind::Return,
            yavs_diag::Span::new(yavs_diag::FileId(0), 0, 1),
        );
        let a = fb.addr(AddrSpace::Frame, s);
        let p = fb.reg(RegTy::I64);
        fb.emit(Inst::Load {
            dst: p,
            addr: a,
            w: Width::B8,
        });
        fb.emit(Inst::Ret { v: Some(p) });
        fb.finish();
        let mut fb = pb.define(top);
        let v = fb.slot(8, 8);
        fb.stmt(
            yavs_ir::StmtKind::Return,
            yavs_diag::Span::new(yavs_diag::FileId(0), 0, 1),
        );
        let a = fb.addr(AddrSpace::Frame, v);
        let args = fb.regs(&[a]);
        let r = fb.reg(RegTy::I64);
        fb.emit(Inst::Call {
            func: leaf,
            args,
            dst: Some(r),
        });
        fb.emit(Inst::Ret { v: Some(r) });
        fb.finish();
        let prog = pb.finish().unwrap();
        assert!(closed_frame(&prog, prog.func(leaf).unwrap(), leaf).is_some());
        assert!(closed_frame(&prog, prog.func(top).unwrap(), top).is_none());
    }

    /// The helper declarations carry the effects `Gen::effects` means (the encoding of
    /// `memory(...)` is LLVM's own, checked here against its printer).
    #[test]
    fn helper_effects() {
        crate::lazy::ensure().unwrap();
        let m = Module::new("effects");
        let g = Gen::new(&m);
        // SAFETY: our module; the string is disposed after the copy.
        let ir = unsafe {
            let p = LLVMPrintModuleToString(m.raw());
            let s = std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned();
            LLVMDisposeMessage(p);
            s
        };
        drop(g);
        let decl = |name: &str| {
            ir.lines()
                .find(|l| l.starts_with("declare") && l.contains(name))
                .unwrap_or_else(|| panic!("no {name} in {ir}"))
                .to_string()
        };
        let group = |name: &str| {
            let d = decl(name);
            let id = d.rsplit('#').next().unwrap_or("").trim().to_string();
            ir.lines()
                .find(|l| l.starts_with(&format!("attributes #{id} ")))
                .unwrap_or_else(|| panic!("no attributes for {d}"))
                .to_string()
        };
        let host = group(HOST);
        assert!(
            host.contains("memory(argmem: readwrite, inaccessiblemem: readwrite)"),
            "{host}"
        );
        assert!(
            host.contains("nounwind") && host.contains("willreturn"),
            "{host}"
        );
        let lcopy = group(LCOPY);
        assert!(lcopy.contains("memory(argmem: readwrite)"), "{lcopy}");
        assert!(group(DEOPT_SLOTS).contains("memory(argmem: read)"));
        let step = group(STEP);
        assert!(
            step.contains("nounwind") && !step.contains("memory"),
            "{step}"
        );
    }
}
