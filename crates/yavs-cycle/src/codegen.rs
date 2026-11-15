//! Cranelift code of the region: one function per trigger, the operations of the member bodies in
//! schedule order on the flat image of two-state words. A logic value of `len` elements is
//! `ceil(len / 64)` words, element 0 in bit 0 of the first, the bits above `len` zero; an integer
//! is one word.
//!
//! A check that fails (`Op::Int`, `Op::Range`, the dynamic indices) stores its operands in the
//! two error words and returns its site number plus one; `RegionProc` makes the error. With
//! `YAVS_CYCLE_CHECKS=off` (developers, to measure them) range and overflow checks are left out
//! and a dynamic index is clamped into its vector, so the code stays memory safe.
//!
//! The only `unsafe` of the crate: the region calls the generated code on its image.
#![allow(unsafe_code)]

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use cranelift_codegen::incremental_cache::CacheKvStore;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::types::{I32, I64};
use cranelift_codegen::ir::{AbiParam, Block as CBlock, InstBuilder, MemFlagsData, Value};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module, ModuleReloc};
use yavs_cache::{Key, KeyBuilder, Store};

use crate::lower::{Arith, Cmp, Gate, Op, Ref, ShiftK};

/// One compiled trigger function: the address of the image's first word; 0, or the failed check's
/// site plus one.
pub(crate) type Entry = unsafe extern "C" fn(*mut u64) -> u32;

/// A function of a shared body ([`Shape`]): the image and the instance's table, the byte offset
/// of each block; 0, or the failed check's site in the body plus one.
type SharedEntry = unsafe extern "C" fn(*mut u64, *const usize) -> u32;

/// The compiled functions of a region; the module owns their code.
pub(crate) struct Code {
    _module: JITModule,
    /// By [`Fun`]: what it runs, in order.
    entries: [Vec<Run>; 3],
    /// The words each loop group drives: (first, count), and where [`Run::Save`] copies them.
    groups: Vec<(Vec<(u32, u32)>, u32)>,
    /// The words the code addresses.
    words: usize,
    /// By instance of a shared body: the byte offset in the image of each of its blocks.
    tables: Vec<Box<[usize]>>,
}

/// One step of a trigger function at run time.
#[derive(Clone, Copy)]
enum Run {
    Fn(Entry),
    /// The words of loop group `g`, saved.
    Save(usize),
    /// The words of loop group `g` must equal those saved, or the run stops ([`UNSETTLED`]).
    Same(usize),
    /// A function of a shared body on the instance of table `t`, its sites from `sites`.
    Shared(SharedEntry, u32, u32),
}

/// A step of the settle as [`compile`] takes it.
pub(crate) enum Step {
    /// Operations, cut into functions with their neighbours.
    Ops(Vec<Op>),
    /// Body `b` of [`compile`]'s `bodies`, a function of its own (a member of a loop group,
    /// run at several steps).
    Call(usize),
    /// [`Run::Save`] and [`Run::Same`] of a loop group.
    Save(usize),
    Same(usize),
    /// Shape `shape` of [`compile`]'s `shapes` on one member: the first word of each of its
    /// blocks (in the order of [`Shape::blocks`]) and the member's first check site.
    Shared {
        shape: usize,
        table: Vec<u32>,
        sites: u32,
    },
}

/// One compiled body for several members whose code is the same but for where their words lie:
/// its operations on virtual words, the blocks (first virtual word, length) each member gives
/// a place of its own in the image (a signal's words, its next-state words, its slots, its edge
/// word), and which virtual words hold state.
pub(crate) struct Shape {
    pub ops: Vec<Op>,
    pub blocks: Vec<(u32, u32)>,
    pub keep: Vec<bool>,
}

/// The site [`Code::run`] gives when a loop group changed in a pass after its fixed point.
pub(crate) const UNSETTLED: u32 = u32::MAX;

/// A pass after a loop group's schedule confirms its fixed point (debug builds,
/// `YAVS_CYCLE_CHECKS=all`).
pub(crate) fn confirm_on() -> bool {
    cfg!(debug_assertions) || std::env::var_os("YAVS_CYCLE_CHECKS").is_some_and(|v| v == "all")
}

impl Code {
    /// Run function `f` on `img`: `Err(site)` when a check failed.
    pub(crate) fn run(&self, img: &mut [u64], f: Fun) -> Result<(), u32> {
        assert!(
            img.len() >= self.words,
            "the image is smaller than the code's words"
        );
        for r in &self.entries[f as usize] {
            match *r {
                // SAFETY: the code reads and writes only words below `words`, checked above.
                Run::Fn(e) => match unsafe { e(img.as_mut_ptr()) } {
                    0 => {}
                    s => return Err(s - 1),
                },
                Run::Save(g) => {
                    let (ws, mut at) = (&self.groups[g].0, self.groups[g].1 as usize);
                    for &(w, c) in ws {
                        img.copy_within(w as usize..(w + c) as usize, at);
                        at += c as usize;
                    }
                }
                Run::Shared(e, t, sites) => {
                    // SAFETY: every block of every table lies below `words` (checked in `compile`).
                    match unsafe { e(img.as_mut_ptr(), self.tables[t as usize].as_ptr()) } {
                        0 => {}
                        s => return Err(sites + s - 1),
                    }
                }
                Run::Same(g) => {
                    let (ws, mut at) = (&self.groups[g].0, self.groups[g].1 as usize);
                    for &(w, c) in ws {
                        let (w, c) = (w as usize, c as usize);
                        if img[w..w + c] != img[at..at + c] {
                            return Err(UNSETTLED);
                        }
                        at += c;
                    }
                }
            }
        }
        Ok(())
    }
}

// SAFETY: the code is finished and never changed again; the module is only kept alive.
unsafe impl Send for Code {}
// SAFETY: as above, nothing in it is mutated after `compile`.
unsafe impl Sync for Code {}

/// The trigger functions of a region.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Fun {
    /// The combinational bodies.
    Settle,
    /// The clocked bodies, the commit, the combinational bodies.
    Edge,
    /// Once, before the first settle: initial values of the variables.
    Init,
}

/// The kind of the entries of compiled region functions in their store (`yavs_cache::CYCLE_DIR`).
const KIND: &str = "cycle";

// ponytail: one store per process, as the compiled tier's (a run simulates one design).
static STORE: Mutex<Option<Store>> = Mutex::new(None);

/// Keep the machine code of the region's functions in the compilation cache of library directory
/// `lib` from now on (`None`: compile every function).
pub fn set_cache(lib: Option<&Path>) {
    *STORE.lock().unwrap_or_else(PoisonError::into_inner) = lib.map(Store::cycle_of_library);
}

/// The store as Cranelift's cache of compiled functions. Cranelift's key covers the function body
/// (every operation and image offset, so the checks and the cut too) and the target with its
/// flags and host features; [`KeyBuilder`] adds the yavs build. A missing or damaged entry is a
/// miss, and Cranelift rejects an entry of another version of itself.
struct Kv<'a>(&'a Store);

impl Kv<'_> {
    fn key(k: &[u8]) -> Key {
        let mut b = KeyBuilder::new(KIND);
        b.text(cranelift_codegen::VERSION).part(k);
        b.finish()
    }
}

impl CacheKvStore for Kv<'_> {
    fn get(&self, key: &[u8]) -> Option<Cow<'_, [u8]>> {
        self.0
            .get(KIND, &Self::key(key))
            .map(|e| Cow::Owned(e.to_vec()))
    }

    fn insert(&mut self, key: &[u8], val: Vec<u8>) {
        self.0.put(KIND, &Self::key(key), &val);
    }
}

/// Compile `ctx` into `id`: from `store` when it has the function (whether it did), else compiled
/// and stored there.
fn define(
    module: &mut JITModule,
    id: FuncId,
    ctx: &mut cranelift_codegen::Context,
    store: Option<&Store>,
) -> Result<bool, String> {
    let Some(s) = store else {
        module
            .define_function(id, ctx)
            .map_err(|e| format!("{e:?}"))?;
        return Ok(false);
    };
    let hit = ctx
        .compile_with_cache(module.isa(), &mut Kv(s), &mut Default::default())
        .map_err(|e| format!("{:?}", e.inner))?
        .1;
    let code = ctx.compiled_code().ok_or("no compiled code")?;
    let relocs: Vec<ModuleReloc> = (code.buffer.relocs().iter())
        .map(|r| ModuleReloc::from_mach_reloc(r, &ctx.func, id))
        .collect();
    module
        .define_function_bytes(
            id,
            u64::from(code.buffer.min_alignment),
            code.code_buffer(),
            &relocs,
        )
        .map_err(|e| format!("{e:?}"))?;
    Ok(hit)
}

/// Ones in the low `len` bits.
pub(crate) fn mask(len: u32) -> u64 {
    if len >= 64 { !0 } else { (1u64 << len) - 1 }
}

/// Words of a logic value of `len` elements.
pub(crate) fn words(len: u32) -> u32 {
    len.div_ceil(64).max(1)
}

/// Whether range, overflow and index checks are compiled (`YAVS_CYCLE_CHECKS=off`: not).
pub(crate) fn checks_on() -> bool {
    std::env::var_os("YAVS_CYCLE_CHECKS").is_none_or(|v| v != "off")
}

fn word(r: Ref) -> i32 {
    match r {
        Ref::Loc(s) => (s * 8) as i32,
        Ref::Sig(_) | Ref::Edge => {
            unreachable!("signal references are resolved to slots before code generation")
        }
    }
}

/// Operations per compiled function, about: a trigger function is cut between members into
/// several, and a member longer than this into parts (`cut`). Cranelift's work on one function
/// grows faster than the function: a member of 39 000 operations as one function took twice the
/// cycles of the same member in parts, for the same instructions.
const CHUNK: usize = 4000;

/// The operations of `ops`, nested ones included.
fn weight(ops: &[Op]) -> usize {
    ops.iter().map(weight_of).sum()
}

/// The operations of `o`, nested ones included.
pub(crate) fn weight_of(o: &Op) -> usize {
    match o {
        Op::If { then, else_, .. } => 1 + weight(then) + weight(else_),
        _ => 1,
    }
}

/// Operations per function: [`CHUNK`], or `YAVS_CYCLE_CHUNK` (developers: a small value cuts every
/// member into parts, to test the cut).
fn chunk() -> usize {
    static N: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *N.get_or_init(|| {
        std::env::var("YAVS_CYCLE_CHUNK")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|&n| n > 0)
            .unwrap_or(CHUNK)
    })
}

/// A piece as parts of about `CHUNK` operations each, each but the last with `true` (its member
/// goes on in the next part); see [`cut`].
fn parts(ops: &[Op]) -> Vec<(Cow<'_, [Op]>, bool)> {
    if weight(ops) <= chunk() {
        return vec![(Cow::Borrowed(ops), false)];
    }
    let ps = cut(ops, chunk());
    let n = ps.len();
    (ps.into_iter().enumerate())
        .map(|(i, p)| (Cow::Owned(p), i + 1 < n))
        .collect()
}

/// `ops` as consecutive parts of about `max` operations: run one after the other, through
/// memory, they do what `ops` does. An `If` larger than `max` is cut inside its branches, every
/// part of it behind the same condition, when no branch writes the condition's word; a larger
/// operation that cannot be cut is a part of its own.
fn cut(ops: &[Op], max: usize) -> Vec<Vec<Op>> {
    let mut out: Vec<Vec<Op>> = Vec::new();
    let mut cur: Vec<Op> = Vec::new();
    let mut w = 0;
    for op in ops {
        let ow = weight_of(op);
        if w > 0 && w + ow > max {
            out.push(std::mem::take(&mut cur));
            w = 0;
        }
        match op {
            Op::If { cond, then, else_ }
                if ow > max
                    && !writes(then, word(*cond) / 8)
                    && !writes(else_, word(*cond) / 8) =>
            {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    w = 0;
                }
                let (t, e) = (cut(then, max), cut(else_, max));
                for k in 0..t.len().max(e.len()) {
                    out.push(vec![Op::If {
                        cond: *cond,
                        then: t.get(k).cloned().unwrap_or_default(),
                        else_: e.get(k).cloned().unwrap_or_default(),
                    }]);
                }
            }
            _ => {
                cur.push(op.clone());
                w += ow;
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Whether an operation of `ops` may write word `c`.
fn writes(ops: &[Op], c: i32) -> bool {
    ops.iter().any(|op| match op {
        Op::If { then, else_, .. } => writes(then, c) || writes(else_, c),
        _ => {
            let hit = std::cell::Cell::new(false);
            let n = span(op) as i32;
            op.map(
                &|r| r,
                &|r| {
                    let d = word(r) / 8;
                    if d <= c && c < d + n {
                        hit.set(true);
                    }
                    r
                },
                &|x| x,
            );
            hit.get()
        }
    })
}

/// The words from its destination an operation (not an `If`) may write.
fn span(op: &Op) -> u32 {
    match op {
        Op::Copy { len, .. }
        | Op::Gate { len, .. }
        | Op::Not { len, .. }
        | Op::Extract { len, .. }
        | Op::Cat { len, .. }
        | Op::Add { len, .. }
        | Op::Mul { len, .. }
        | Op::VDiv { len, .. }
        | Op::Shift { len, .. }
        | Op::FromInt { len, .. }
        | Op::Splat { len, .. }
        | Op::Select { len, .. } => words(*len),
        Op::Lit { val, .. } => val.len() as u32,
        Op::Blend { mask, .. } => mask.len() as u32,
        Op::Insert { lo, len, .. } => words(lo + len),
        Op::Ext { to, .. } => words(*to),
        Op::IndexRead { w, .. } => words(*w),
        // The word after the array too: an element may reach into it.
        Op::IndexWrite { lo, n, stride, .. } => (lo + n * stride).div_ceil(64) + 1,
        Op::VecEq { .. }
        | Op::VCmp { .. }
        | Op::IntLit { .. }
        | Op::IntCmp { .. }
        | Op::Int { .. }
        | Op::Reduce { .. }
        | Op::ToInt { .. } => 1,
        Op::Range { .. } | Op::If { .. } | Op::Note { .. } => 0,
    }
}

/// Compile the trigger functions: the settle as steps, the edge's own pieces (the settle follows
/// them) and the initialisation's, each piece needing nothing from another but memory (the
/// bodies of members; a member longer than `CHUNK` operations is cut into parts, `cut`, which hand
/// on everything through memory); `bodies` are the loop groups' members, each in functions of its
/// own, compiled
/// with checks or without (an early run of a member may see values not settled yet: no check
/// exits it, a division by zero divides by one), `groups` the words each loop group drives and
/// its save words. Every `Ref` is an image slot (`Ref::Loc`); `err` is the first of the two
/// error words; `keep[w]`: word `w` holds state between calls (signals, variables of clocked
/// processes), every other word is scratch, which the code keeps in registers and never stores
/// unless an element of it is addressed at run time. `name` labels the functions in the perf
/// map.
#[allow(clippy::too_many_arguments)]
pub(crate) fn compile<'a>(
    settle: &'a [Step],
    edge: &'a [Step],
    init: &'a [Step],
    bodies: &'a [(Vec<Op>, bool)],
    shapes: &'a [Shape],
    groups: Vec<(Vec<(u32, u32)>, u32)>,
    keep: &[bool],
    err: u32,
    log: (u32, u32),
    name: &str,
) -> Result<Code, String> {
    let words = keep.len();
    let mut flags = settings::builder();
    let set = |f: &mut settings::Builder, k: &str, v: &str| f.set(k, v).map_err(|e| e.to_string());
    set(&mut flags, "opt_level", "speed")?;
    set(
        &mut flags,
        "enable_verifier",
        if cfg!(debug_assertions) {
            "true"
        } else {
            "false"
        },
    )?;
    // Developers: `YAVS_CYCLE_FLAGS=name=value,...` sets more Cranelift flags (to measure them).
    if let Some(v) = std::env::var_os("YAVS_CYCLE_FLAGS") {
        for kv in v.to_string_lossy().split(',').filter(|kv| !kv.is_empty()) {
            let (k, v) = kv
                .split_once('=')
                .ok_or("YAVS_CYCLE_FLAGS takes name=value pairs")?;
            set(&mut flags, k, v)?;
        }
    }
    let isa = cranelift_native::builder()
        .map_err(|e| e.to_string())?
        .finish(settings::Flags::new(flags))
        .map_err(|e| e.to_string())?;
    let jb = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    let mut module = JITModule::new(jb);
    let ptr = module.target_config().pointer_type();
    let checks = checks_on();
    let perf = std::env::var_os("YAVS_CLIFT_PERFMAP").is_some();
    // The functions to compile (label, parts, whether checks exit), the loop bodies first; and
    // what each trigger function runs, a function by its index. A part says whether its member
    // goes on in the next one (`cut`).
    // A function: label, parts, whether checks exit, the shape it is a body of.
    type Fn<'a> = (String, Vec<(Cow<'a, [Op]>, bool)>, bool, Option<usize>);
    let mut funs: Vec<Fn<'_>> = Vec::new();
    // Parts packed into functions of about `CHUNK` operations; returns the functions made.
    let pack = |funs: &mut Vec<Fn<'a>>,
                what: &str,
                ps: Vec<(Cow<'a, [Op]>, bool)>,
                checked: bool,
                shape: Option<usize>| {
        let first = funs.len();
        let mut cur: Vec<(Cow<'a, [Op]>, bool)> = Vec::new();
        let mut w = 0;
        for (p, more) in ps {
            let pw = weight(&p);
            if w > 0 && w + pw > chunk() {
                funs.push((
                    format!("{what}_{}", funs.len()),
                    std::mem::take(&mut cur),
                    checked,
                    shape,
                ));
                w = 0;
            }
            w += pw;
            cur.push((p, more));
        }
        if !cur.is_empty() {
            funs.push((format!("{what}_{}", funs.len()), cur, checked, shape));
        }
        first..funs.len()
    };
    let bodies: Vec<std::ops::Range<usize>> = (bodies.iter().enumerate())
        .map(|(b, (ops, checked))| {
            pack(&mut funs, &format!("loop_{b}"), parts(ops), *checked, None)
        })
        .collect();
    let shaped: Vec<std::ops::Range<usize>> = (shapes.iter().enumerate())
        .map(|(k, sh)| {
            pack(
                &mut funs,
                &format!("shape_{k}"),
                parts(&sh.ops),
                true,
                Some(k),
            )
        })
        .collect();
    // What each trigger function runs: a function by its index, a function of a shared body
    // with its table and first site, or a step of the runner.
    enum Pend {
        Fn(usize),
        Shared(usize, u32, u32),
        Run(Run),
    }
    let mut tables: Vec<Box<[usize]>> = Vec::new();
    let mut runs: [Vec<Pend>; 3] = Default::default();
    for (f, (what, steps)) in [("settle", settle), ("edge", edge), ("init", init)]
        .into_iter()
        .enumerate()
    {
        let mut cur: Vec<(Cow<'a, [Op]>, bool)> = Vec::new();
        let flush =
            |funs: &mut Vec<Fn<'a>>, cur: &mut Vec<(Cow<'a, [Op]>, bool)>, runs: &mut Vec<Pend>| {
                if !cur.is_empty() {
                    runs.extend(pack(funs, what, std::mem::take(cur), true, None).map(Pend::Fn));
                }
            };
        for st in steps {
            let r = match st {
                Step::Ops(p) => {
                    cur.extend(parts(p));
                    continue;
                }
                Step::Call(b) => {
                    flush(&mut funs, &mut cur, &mut runs[f]);
                    runs[f].extend(bodies[*b].clone().map(Pend::Fn));
                    continue;
                }
                Step::Shared {
                    shape,
                    table,
                    sites,
                } => {
                    flush(&mut funs, &mut cur, &mut runs[f]);
                    let blocks = &shapes[*shape].blocks;
                    if table.len() != blocks.len()
                        || (table.iter().zip(blocks)).any(|(&w, &(_, n))| (w + n) as usize > words)
                    {
                        return Err("a shared body's table does not fit its image".into());
                    }
                    let t = tables.len() as u32;
                    tables.push(table.iter().map(|&w| w as usize * 8).collect());
                    runs[f].extend(shaped[*shape].clone().map(|k| Pend::Shared(k, t, *sites)));
                    continue;
                }
                Step::Save(g) => Run::Save(*g),
                Step::Same(g) => Run::Same(*g),
            };
            flush(&mut funs, &mut cur, &mut runs[f]);
            runs[f].push(Pend::Run(r));
        }
        flush(&mut funs, &mut cur, &mut runs[f]);
    }
    // Developers: `YAVS_CYCLE_STATS=1` prints each function's operations, Cranelift instructions,
    // machine code bytes and compile time.
    let stats = std::env::var_os("YAVS_CYCLE_STATS").is_some();
    // Developers: `YAVS_CYCLE_DISASM=1` prints the machine code of each function (compiled, never
    // taken from the cache).
    let disasm = std::env::var_os("YAVS_CYCLE_DISASM").is_some();
    let store = STORE.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let mut hits = 0;
    let t0 = std::time::Instant::now();
    let mut ids = Vec::new();
    for (sym, ops, checked, shape) in &funs {
        let t = std::time::Instant::now();
        let sym = format!("cycle_{sym}_{name}");
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(ptr));
        if shape.is_some() {
            sig.params.push(AbiParam::new(ptr));
        }
        sig.returns.push(AbiParam::new(I32));
        let id = module
            .declare_function(&sym, Linkage::Local, &sig)
            .map_err(|e| e.to_string())?;
        let mut ctx = module.make_context();
        ctx.func.signature = sig;
        let mut fbc = FunctionBuilderContext::new();
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbc);
            let blk = b.create_block();
            b.append_block_params_for_function_params(blk);
            b.switch_to_block(blk);
            b.seal_block(blk);
            let base = b.block_params(blk)[0];
            // A shared body: each block's address, from the instance's table.
            let mut blocks = Vec::new();
            if let Some(k) = shape {
                let tbl = b.block_params(blk)[1];
                for (j, &(v, _)) in shapes[*k].blocks.iter().enumerate() {
                    let off = b
                        .ins()
                        .load(ptr, MemFlagsData::trusted(), tbl, 8 * j as i32);
                    blocks.push((v as i32 * 8, b.ins().iadd(base, off)));
                }
            }
            let mut g = Gen {
                b,
                base,
                err: err as i32 * 8,
                log: (log.0 as i32 * 8, log.1),
                checks: checks && *checked,
                exits: *checked,
                ptr,
                cache: BTreeMap::new(),
                cap: CACHE_MAX,
                blocks,
                keep: shape.map_or(keep, |k| &shapes[k].keep[..]),
                zeros: std::collections::HashMap::new(),
            };
            // Each piece starts from memory: the words it keeps in registers are few, and a
            // branch saves and forgets only those.
            for (p, more) in ops {
                g.cap = if weight(p) > BIG {
                    CACHE_BIG
                } else {
                    CACHE_MAX
                };
                for op in p.iter() {
                    g.op(op);
                }
                // A member that goes on in the next part leaves its scratch words there too.
                if *more {
                    g.spill_all();
                } else {
                    g.write_back();
                }
                g.cache.clear();
                g.zeros.clear();
            }
            let zero = g.b.ins().iconst(I32, 0);
            g.b.ins().return_(&[zero]);
            g.b.finalize(module.target_config());
        }
        ctx.set_disasm(disasm);
        let n_ir = ctx.func.dfg.num_insts();
        let hit = define(
            &mut module,
            id,
            &mut ctx,
            store.as_ref().filter(|_| !disasm),
        )?;
        hits += usize::from(hit);
        if disasm && let Some(text) = ctx.compiled_code().and_then(|c| c.vcode.as_ref()) {
            eprintln!("{sym}:\n{text}");
        }
        if stats {
            let bytes = ctx.compiled_code().map_or(0, |c| c.code_info().total_size);
            eprintln!(
                "cycle stats: {sym}: {} operations, {n_ir} instructions, {bytes} bytes, {} us{}",
                ops.iter().map(|(p, _)| weight(p)).sum::<usize>(),
                t.elapsed().as_micros(),
                if hit { ", from the cache" } else { "" }
            );
        }
        ids.push((id, sym));
    }
    module.finalize_definitions().map_err(|e| e.to_string())?;
    if stats {
        eprintln!(
            "cycle stats: {} functions in {} ms, {hits} from the cache",
            ids.len(),
            t0.elapsed().as_millis()
        );
    }
    let addr = |id: FuncId| module.get_finalized_function(id);
    if perf {
        for (id, sym) in &ids {
            // The size is not kept by the module; one page is an upper bound for a label.
            yavs_rt::perfmap::append(addr(*id) as usize, 4096, sym);
        }
    }
    let mut entries: [Vec<Run>; 3] = Default::default();
    for (f, rs) in runs.iter().enumerate() {
        entries[f] = (rs.iter())
            .map(|r| match *r {
                // SAFETY: a function of signature `fn(ptr) -> u32` defined above.
                Pend::Fn(k) => {
                    Run::Fn(unsafe { std::mem::transmute::<*const u8, Entry>(addr(ids[k].0)) })
                }
                // SAFETY: a function of a shape, `fn(ptr, ptr) -> u32`, defined above.
                Pend::Shared(k, t, s) => Run::Shared(
                    unsafe { std::mem::transmute::<*const u8, SharedEntry>(addr(ids[k].0)) },
                    t,
                    s,
                ),
                Pend::Run(r) => r,
            })
            .collect();
    }
    // The edge runs the settle after its own pieces.
    let after = entries[0].clone();
    entries[1].extend(after);
    Ok(Code {
        _module: module,
        entries,
        groups,
        words,
        tables,
    })
}

/// Most words a piece keeps in registers at once (`Gen::keep_in`), and for a piece of more than
/// `BIG` operations: in a large piece the register allocator's work grows with the values live at
/// once, while a small piece with many words (a record copied whole) runs faster with them all.
const CACHE_MAX: usize = 256;
const CACHE_BIG: usize = 16;
const BIG: usize = 500;

struct Gen<'a> {
    b: FunctionBuilder<'a>,
    base: Value,
    /// Byte offset of the first error word.
    err: i32,
    /// The log of the notes that fired (`Op::Note`): the byte offset of its count, which the
    /// sites follow, and how many it holds.
    log: (i32, u32),
    checks: bool,
    /// A failed check (a zero divisor: the one check `checks` off keeps) exits the function;
    /// without, a division by zero divides by one.
    exits: bool,
    ptr: cranelift_codegen::ir::Type,
    /// The words in registers, by byte offset: their value and whether memory is behind it. Every
    /// value dominates the code that follows (a branch drops what it made at its join).
    cache: BTreeMap<i32, (Value, bool)>,
    /// Most words in `cache` at once, for the piece being generated.
    cap: usize,
    /// A shared body: the first virtual byte offset of each block and the block's address, in
    /// order; a word's address is from the last block at or below it (`at`).
    blocks: Vec<(i32, Value)>,
    keep: &'a [bool],
    /// Bits known to be zero of some values (constants, extracted bits, words built by inserts).
    zeros: std::collections::HashMap<Value, u64>,
}

impl Gen<'_> {
    fn ops(&mut self, ops: &[Op]) {
        for op in ops {
            self.op(op);
        }
    }

    /// The address of the word at byte offset `off`: a base and an offset from it.
    fn at(&self, off: i32) -> (Value, i32) {
        match self.blocks.iter().rev().find(|(v, _)| *v <= off) {
            Some(&(v, b)) => (b, off - v),
            None => (self.base, off),
        }
    }

    /// Word `k` of `r`: from a register when it has one.
    fn ld(&mut self, r: Ref, k: u32) -> Value {
        let off = word(r) + k as i32 * 8;
        if let Some(&(v, _)) = self.cache.get(&off) {
            return v;
        }
        let (a, o) = self.at(off);
        let v = self.b.ins().load(I64, MemFlagsData::trusted(), a, o);
        self.keep_in(off, v, false);
        v
    }

    /// Word `k` of `r` is `v` from here on; memory follows at `write_back` or `spill`.
    fn st(&mut self, r: Ref, k: u32, v: Value) {
        self.keep_in(word(r) + k as i32 * 8, v, true);
    }

    /// Word `off` in a register. Past `cap` words every changed one goes to memory and the cache
    /// starts again: a register allocator's work grows faster than the values live at once.
    fn keep_in(&mut self, off: i32, v: Value, dirty: bool) {
        if self.cache.len() >= self.cap && !self.cache.contains_key(&off) {
            let changed: Vec<(i32, Value)> = (self.cache.iter())
                .filter(|(_, (_, d))| *d)
                .map(|(&o, &(v, _))| (o, v))
                .collect();
            for (o, v) in changed {
                self.store(o, v);
            }
            self.cache.clear();
        }
        self.cache.insert(off, (v, dirty));
    }

    fn store(&mut self, off: i32, v: Value) {
        let (a, o) = self.at(off);
        self.b.ins().store(MemFlagsData::trusted(), v, a, o);
    }

    /// The words that hold state, into memory (the end of the function).
    fn write_back(&mut self) {
        let dirty: Vec<(i32, Value)> = (self.cache.iter())
            .filter(|(off, (_, d))| *d && self.keep[**off as usize / 8])
            .map(|(&off, &(v, _))| (off, v))
            .collect();
        for (off, v) in dirty {
            self.store(off, v);
        }
    }

    /// Every changed word into memory, scratch ones too.
    fn spill_all(&mut self) {
        let dirty: Vec<(i32, Value)> = (self.cache.iter())
            .filter(|(_, (_, d))| *d)
            .map(|(&off, &(v, _))| (off, v))
            .collect();
        for (off, v) in dirty {
            self.store(off, v);
        }
    }

    /// Words `k0 ..= k1` of `r` into memory, scratch ones too: code is about to address one of
    /// their elements at run time.
    fn spill(&mut self, r: Ref, k0: u32, k1: u32) {
        for k in k0..=k1 {
            let off = word(r) + k as i32 * 8;
            if let Some((v, d)) = self.cache.get_mut(&off)
                && *d
            {
                *d = false;
                let v = *v;
                self.store(off, v);
            }
        }
    }

    /// Words `k0 ..= k1` of `r` changed in memory.
    fn forget(&mut self, r: Ref, k0: u32, k1: u32) {
        for k in k0..=k1 {
            self.cache.remove(&(word(r) + k as i32 * 8));
        }
    }

    fn lds(&mut self, r: Ref, len: u32) -> Vec<Value> {
        (0..words(len)).map(|k| self.ld(r, k)).collect()
    }

    fn sts(&mut self, r: Ref, v: &[Value]) {
        for (k, &x) in v.iter().enumerate() {
            self.st(r, k as u32, x);
        }
    }

    fn konst(&mut self, v: u64) -> Value {
        let x = self.b.ins().iconst(I64, v as i64);
        self.zeros.insert(x, !v);
        x
    }

    /// The top word of a value of `len` elements with the bits above `len` cleared.
    fn top(&mut self, v: &mut [Value], len: u32) {
        if !len.is_multiple_of(64)
            && let Some(t) = v.last_mut()
        {
            *t = self.b.ins().band_imm_s(*t, mask(len % 64) as i64);
        }
    }

    /// `n` (at most 64) bits of the words `v` from bit `off`, in the low bits; `full`: the bits
    /// of `v` from `full` on are zero, so no mask is needed past them.
    fn bits(&mut self, v: &[Value], off: u32, n: u32, full: u32) -> Value {
        let (k, s) = ((off / 64) as usize, off % 64);
        let mut x = v[k];
        let mut end = 64 * (k as u32 + 1);
        if s != 0 {
            x = self.b.ins().ushr_imm_s(x, i64::from(s));
            if s + n > 64 {
                let h = self.b.ins().ishl_imm_s(v[k + 1], i64::from(64 - s));
                x = self.b.ins().bor(x, h);
                end = off + 64;
            }
        }
        if n < 64 && off + n < full.min(end) {
            x = self.b.ins().band_imm_s(x, mask(n) as i64);
        }
        if n < 64 {
            self.zeros.insert(x, !mask(n));
        }
        x
    }

    /// The bits of `v` known to be zero.
    fn zero(&self, v: Value) -> u64 {
        self.zeros.get(&v).copied().unwrap_or(0)
    }

    /// `d` with bits `lo .. lo + len` taken from `s` (`len` elements).
    fn insert(&mut self, d: &mut [Value], s: &[Value], lo: u32, len: u32) {
        let mut a = lo;
        while a < lo + len {
            let k = a / 64;
            let b = (lo + len).min(k * 64 + 64);
            let x = self.bits(s, a - lo, b - a, len);
            let sh = a - k * 64;
            let m = mask(b - a) << sh;
            let dk = d[k as usize];
            let zd = self.zero(dk);
            // Bits known to be zero need no clearing (a word built field by field from 0).
            let keep = if zd & m == m {
                dk
            } else {
                self.b.ins().band_imm_s(dk, !m as i64)
            };
            let x = if sh != 0 {
                self.b.ins().ishl_imm_s(x, i64::from(sh))
            } else {
                x
            };
            let r = self.b.ins().bor(keep, x);
            let zx = self.zero(x) << sh | mask(sh);
            self.zeros.insert(r, (zd | m) & (zx | !m));
            d[k as usize] = r;
            a = b;
        }
    }

    fn bool_of(&mut self, c: Value) -> Value {
        self.b.ins().uextend(I64, c)
    }

    /// A check: `fail` (an `I8` truth value) jumps to a cold block that stores `vals` in the
    /// error words and returns `site + 1`.
    /// The divisor `y` of `x`: a failed check where `fail` (zero, or `MIN / -1`); without exits,
    /// one instead, so the machine division never traps.
    fn div_guard(&mut self, fail: Value, site: u32, x: Value, y: Value) -> Value {
        if self.exits {
            self.check(fail, site, &[x, y]);
            return y;
        }
        let one = self.konst(1);
        self.b.ins().select(fail, one, y)
    }

    fn check(&mut self, fail: Value, site: u32, vals: &[Value]) {
        let bad = self.b.create_block();
        let ok = self.b.create_block();
        self.b.set_cold_block(bad);
        self.b.ins().brif(fail, bad, &[], ok, &[]);
        self.b.switch_to_block(bad);
        self.b.seal_block(bad);
        for (k, &v) in vals.iter().enumerate() {
            self.b.ins().store(
                MemFlagsData::trusted(),
                v,
                self.base,
                self.err + k as i32 * 8,
            );
        }
        let s = self.b.ins().iconst(I32, i64::from(site) + 1);
        self.b.ins().return_(&[s]);
        self.b.switch_to_block(ok);
        self.b.seal_block(ok);
    }

    /// The element of a dynamic index: `idx - left` (ascending) or `left - idx`, checked to be
    /// below `n` and, for `count` elements from it to the left, at least `count - 1` (or clamped
    /// when checks are off); returns its first bit, `lo + (n - 1 - p) * w`.
    #[allow(clippy::too_many_arguments)]
    fn element(
        &mut self,
        idx: Ref,
        left: i64,
        down: bool,
        n: u32,
        w: u32,
        count: u32,
        lo: u32,
        site: u32,
    ) -> Value {
        let i = self.ld(idx, 0);
        let p = if down {
            let l = self.b.ins().iconst(I64, left);
            self.b.ins().isub(l, i)
        } else {
            self.b.ins().iadd_imm_s(i, -left)
        };
        // `p - (count - 1)` below `n - count + 1`, unsigned.
        let q = self.b.ins().iadd_imm_s(p, 1 - i64::from(count));
        let out = self.b.ins().icmp_imm_s(
            IntCC::UnsignedGreaterThanOrEqual,
            q,
            i64::from(n) - i64::from(count) + 1,
        );
        let p = if self.checks {
            self.check(out, site, &[i]);
            p
        } else {
            let z = self.konst(i64::from(count - 1) as u64);
            self.b.ins().select(out, z, p)
        };
        let l = self.b.ins().iconst(I64, i64::from(n - 1));
        let e = self.b.ins().isub(l, p);
        let e = if w == 1 {
            e
        } else {
            self.b.ins().imul_imm_s(e, i64::from(w))
        };
        self.b.ins().iadd_imm_s(e, i64::from(lo))
    }

    /// `x << (64 - s)` for `s` in `0 .. 64`: 0 when `s` is 0 (a shift by 64 is a shift by 0).
    fn shl_rest(&mut self, x: Value, s: Value) -> Value {
        let t = self.b.ins().ishl_imm_s(x, 1);
        let k = self.b.ins().iconst(I64, 63);
        let r = self.b.ins().isub(k, s);
        self.b.ins().ishl(t, r)
    }

    /// `x >> (64 - s)` for `s` in `0 .. 64`: 0 when `s` is 0.
    fn ushr_rest(&mut self, x: Value, s: Value) -> Value {
        let t = self.b.ins().ushr_imm_s(x, 1);
        let k = self.b.ins().iconst(I64, 63);
        let r = self.b.ins().isub(k, s);
        self.b.ins().ushr(t, r)
    }

    /// Whether a `w`-element field at a bit `lo + k * stride` never crosses a word.
    fn aligned(lo: u32, stride: u32, w: u32) -> bool {
        w.is_power_of_two() && w <= 64 && lo.is_multiple_of(w) && stride.is_multiple_of(w)
    }

    /// The `w` bits of `r` from the run-time bit `bit` (memory holds them, see `spill`).
    fn dyn_read(&mut self, r: Ref, bit: Value, w: u32, one_word: bool) -> Vec<Value> {
        (0..words(w))
            .map(|j| {
                let b = if j == 0 {
                    bit
                } else {
                    self.b.ins().iadd_imm_s(bit, i64::from(64 * j))
                };
                let (a, s) = self.bit_addr(r, b);
                let lo = self.b.ins().load(I64, MemFlagsData::trusted(), a, 0);
                let mut x = self.b.ins().ushr(lo, s);
                if !one_word {
                    // The next word may lie past the value: its bits are masked off below.
                    let hi = self.b.ins().load(I64, MemFlagsData::trusted(), a, 8);
                    let h = self.shl_rest(hi, s);
                    x = self.b.ins().bor(x, h);
                }
                let n = (w - 64 * j).min(64);
                if n < 64 {
                    x = self.b.ins().band_imm_s(x, mask(n) as i64);
                    self.zeros.insert(x, !mask(n));
                }
                x
            })
            .collect()
    }

    /// The `w` bits `v` into `r` from the run-time bit `bit`, in memory.
    fn dyn_write(&mut self, r: Ref, bit: Value, w: u32, v: &[Value], one_word: bool) {
        for (j, &x) in v.iter().enumerate() {
            let b = if j == 0 {
                bit
            } else {
                self.b.ins().iadd_imm_s(bit, 64 * j as i64)
            };
            let n = (w - 64 * j as u32).min(64);
            let (a, s) = self.bit_addr(r, b);
            let m = self.konst(mask(n));
            let w0 = self.b.ins().load(I64, MemFlagsData::trusted(), a, 0);
            let ms = self.b.ins().ishl(m, s);
            let keep = self.b.ins().band_not(w0, ms);
            let xs = self.b.ins().ishl(x, s);
            let w0 = self.b.ins().bor(keep, xs);
            self.b.ins().store(MemFlagsData::trusted(), w0, a, 0);
            if !one_word {
                // The bits that cross into the next word (none when `s` is 0); its other bits are
                // stored back unchanged.
                let w1 = self.b.ins().load(I64, MemFlagsData::trusted(), a, 8);
                let mh = self.ushr_rest(m, s);
                let keep = self.b.ins().band_not(w1, mh);
                let xh = self.ushr_rest(x, s);
                let w1 = self.b.ins().bor(keep, xh);
                self.b.ins().store(MemFlagsData::trusted(), w1, a, 8);
            }
        }
    }

    /// The value of `len` elements in `v` sign- or zero-extended to `to` elements.
    fn extend(&mut self, v: &[Value], len: u32, to: u32, signed: bool) -> Vec<Value> {
        let mut out: Vec<Value> = v[..words(len.min(to)) as usize].to_vec();
        if to <= len {
            self.top(&mut out, to);
            return out;
        }
        let k = ((len - 1) / 64) as usize;
        let fill = if signed {
            // The sign bit spread over a word: `(x << (63 - b)) >> 63`, arithmetic; the bits
            // above it in its word are zero (the invariant), so an OR sets them.
            let b = (len - 1) % 64;
            let t = self.b.ins().ishl_imm_s(v[k], i64::from(63 - b));
            let f = self.b.ins().sshr_imm_s(t, 63);
            if b != 63 {
                let hi = self.b.ins().band_imm_s(f, (!mask(b + 1)) as i64);
                out[k] = self.b.ins().bor(out[k], hi);
            }
            f
        } else {
            self.konst(0)
        };
        out.resize(words(to) as usize, fill);
        self.top(&mut out, to);
        out
    }

    /// `x < y` (`le`: `<=`) of two values of `n` words, unsigned, or signed by the top word.
    fn less(&mut self, x: &[Value], y: &[Value], signed: bool, le: bool) -> Value {
        // From the low word up: the result so far, overridden where a higher word differs.
        let mut r = if le {
            self.b.ins().iconst(cranelift_codegen::ir::types::I8, 1)
        } else {
            self.b.ins().iconst(cranelift_codegen::ir::types::I8, 0)
        };
        let nw = x.len();
        for k in 0..nw {
            let top = k + 1 == nw;
            let lt = if top && signed {
                IntCC::SignedLessThan
            } else {
                IntCC::UnsignedLessThan
            };
            let l = self.b.ins().icmp(lt, x[k], y[k]);
            let e = self.b.ins().icmp(IntCC::Equal, x[k], y[k]);
            r = self.b.ins().select(e, r, l);
        }
        r
    }

    /// The product of `x` and `y` modulo `2 ** (64 * nw)`, schoolbook on words.
    fn mul_words(&mut self, x: &[Value], y: &[Value], nw: usize) -> Vec<Value> {
        let zero = self.konst(0);
        let mut acc = vec![zero; nw];
        for (i, &a) in x.iter().enumerate().take(nw) {
            let mut carry = zero;
            for (j, &bb) in y.iter().enumerate() {
                let k = i + j;
                if k >= nw {
                    break;
                }
                let lo = self.b.ins().imul(a, bb);
                let hi = if k + 1 < nw {
                    self.b.ins().umulhi(a, bb)
                } else {
                    zero
                };
                // acc[k] + lo + carry, with the carries into the next word.
                let s1 = self.b.ins().iadd(acc[k], lo);
                let c1 = self.b.ins().icmp(IntCC::UnsignedLessThan, s1, lo);
                let s2 = self.b.ins().iadd(s1, carry);
                let c2 = self.b.ins().icmp(IntCC::UnsignedLessThan, s2, carry);
                acc[k] = s2;
                let c1 = self.bool_of(c1);
                let c2 = self.bool_of(c2);
                let c = self.b.ins().iadd(c1, c2);
                carry = self.b.ins().iadd(hi, c);
            }
            let k = i + y.len();
            if k < nw {
                acc[k] = self.b.ins().iadd(acc[k], carry);
            }
        }
        acc
    }

    /// The address of the word holding bit `bit` of `r`, and the bit's place in it.
    fn bit_addr(&mut self, r: Ref, bit: Value) -> (Value, Value) {
        let w = self.b.ins().ushr_imm_s(bit, 6);
        let w = self.b.ins().ishl_imm_s(w, 3);
        let w = if self.ptr == I64 {
            w
        } else {
            self.b.ins().ireduce(self.ptr, w)
        };
        let (b, o) = self.at(word(r));
        let a = self.b.ins().iadd(b, w);
        let a = self.b.ins().iadd_imm_s(a, i64::from(o));
        let s = self.b.ins().band_imm_s(bit, 63);
        (a, s)
    }

    fn op(&mut self, op: &Op) {
        match op {
            Op::Copy { dst, src, len } => {
                let v = self.lds(*src, *len);
                self.sts(*dst, &v);
            }
            Op::Lit { dst, val } => {
                let v: Vec<Value> = val.iter().map(|&w| self.konst(w)).collect();
                self.sts(*dst, &v);
            }
            Op::Blend { dst, src, mask } => {
                for (k, &m) in mask.iter().enumerate() {
                    if m == 0 {
                        continue;
                    }
                    let s = self.ld(*src, k as u32);
                    let x = if m == !0 {
                        s
                    } else {
                        let d = self.ld(*dst, k as u32);
                        let a = self.b.ins().band_imm_s(d, !m as i64);
                        let x = self.b.ins().band_imm_s(s, m as i64);
                        self.b.ins().bor(a, x)
                    };
                    self.st(*dst, k as u32, x);
                }
            }
            Op::Gate { g, dst, a, b, len } => {
                let (x, y) = (self.lds(*a, *len), self.lds(*b, *len));
                let mut v: Vec<Value> = x
                    .iter()
                    .zip(&y)
                    .map(|(&x, &y)| match g {
                        Gate::And => self.b.ins().band(x, y),
                        Gate::Or => self.b.ins().bor(x, y),
                        Gate::Xor => self.b.ins().bxor(x, y),
                        Gate::Nand => {
                            let t = self.b.ins().band(x, y);
                            self.b.ins().bnot(t)
                        }
                        Gate::Nor => {
                            let t = self.b.ins().bor(x, y);
                            self.b.ins().bnot(t)
                        }
                        Gate::Xnor => {
                            let t = self.b.ins().bxor(x, y);
                            self.b.ins().bnot(t)
                        }
                    })
                    .collect();
                if matches!(g, Gate::Nand | Gate::Nor | Gate::Xnor) {
                    self.top(&mut v, *len);
                }
                self.sts(*dst, &v);
            }
            Op::Not { dst, a, len } => {
                let x = self.lds(*a, *len);
                let mut v: Vec<Value> = x.iter().map(|&x| self.b.ins().bnot(x)).collect();
                self.top(&mut v, *len);
                self.sts(*dst, &v);
            }
            Op::Extract {
                dst,
                src,
                lo,
                len,
                full,
            } => {
                // Only the source words the bits are in, from word `k0`.
                let (k0, k1) = (lo / 64, (lo + len - 1) / 64);
                let s: Vec<Value> = (k0..=k1).map(|k| self.ld(*src, k)).collect();
                let base = 64 * k0;
                let v: Vec<Value> = (0..words(*len))
                    .map(|j| {
                        let off = lo + 64 * j - base;
                        let n = (*len - 64 * j).min(64);
                        self.bits(&s, off, n, full.saturating_sub(base))
                    })
                    .collect();
                self.sts(*dst, &v);
            }
            Op::Insert { dst, src, lo, len } => {
                let s = self.lds(*src, *len);
                // Only the words the elements go into, from word `k0`.
                let (k0, k1) = (lo / 64, (lo + len - 1) / 64);
                let mut d: Vec<Value> = (k0..=k1).map(|k| self.ld(*dst, k)).collect();
                self.insert(&mut d, &s, lo - 64 * k0, *len);
                for k in k0..=k1 {
                    self.st(*dst, k, d[(k - k0) as usize]);
                }
            }
            Op::Cat {
                dst,
                hi,
                lo,
                lo_len,
                len,
            } => {
                let l = self.lds(*lo, *lo_len);
                let h = self.lds(*hi, *len - *lo_len);
                let mut v = l.clone();
                v.resize(words(*len) as usize, l[0]);
                for x in v.iter_mut().skip(l.len()) {
                    *x = self.b.ins().iconst(I64, 0);
                }
                self.insert(&mut v, &h, *lo_len, *len - *lo_len);
                self.sts(*dst, &v);
            }
            Op::Add {
                dst,
                a,
                b,
                len,
                sub,
            } => {
                let (x, y) = (self.lds(*a, *len), self.lds(*b, *len));
                let mut v = Vec::new();
                let mut c: Option<Value> = None;
                let nw = x.len();
                for (k, (&x, &y)) in x.iter().zip(&y).enumerate() {
                    let last = k + 1 == nw;
                    let (r, c1) = if *sub {
                        let r = self.b.ins().isub(x, y);
                        (r, self.b.ins().icmp(IntCC::UnsignedLessThan, x, y))
                    } else {
                        let r = self.b.ins().iadd(x, y);
                        (r, self.b.ins().icmp(IntCC::UnsignedLessThan, r, x))
                    };
                    let (r, c1) = match c {
                        None => (r, c1),
                        Some(cin) => {
                            let ci = self.bool_of(cin);
                            if *sub {
                                let r2 = self.b.ins().isub(r, ci);
                                let c2 = self.b.ins().icmp(IntCC::UnsignedLessThan, r, ci);
                                (r2, self.b.ins().bor(c1, c2))
                            } else {
                                let r2 = self.b.ins().iadd(r, ci);
                                let c2 = self.b.ins().icmp(IntCC::UnsignedLessThan, r2, r);
                                (r2, self.b.ins().bor(c1, c2))
                            }
                        }
                    };
                    v.push(r);
                    c = (!last).then_some(c1);
                }
                self.top(&mut v, *len);
                self.sts(*dst, &v);
            }
            Op::VecEq { dst, a, b, len, ne } => {
                let (x, y) = (self.lds(*a, *len), self.lds(*b, *len));
                let mut d = self.konst(0);
                for (&x, &y) in x.iter().zip(&y) {
                    let t = self.b.ins().bxor(x, y);
                    d = self.b.ins().bor(d, t);
                }
                let cc = if *ne { IntCC::NotEqual } else { IntCC::Equal };
                let c = self.b.ins().icmp_imm_s(cc, d, 0);
                let r = self.bool_of(c);
                self.st(*dst, 0, r);
            }
            Op::IntLit { dst, v } => {
                let x = self.b.ins().iconst(I64, *v);
                self.st(*dst, 0, x);
            }
            Op::IntCmp { op, dst, a, b } => {
                let (x, y) = (self.ld(*a, 0), self.ld(*b, 0));
                let cc = match op {
                    Cmp::Eq => IntCC::Equal,
                    Cmp::Ne => IntCC::NotEqual,
                    Cmp::Lt => IntCC::SignedLessThan,
                    Cmp::Le => IntCC::SignedLessThanOrEqual,
                    Cmp::Gt => IntCC::SignedGreaterThan,
                    Cmp::Ge => IntCC::SignedGreaterThanOrEqual,
                };
                let c = self.b.ins().icmp(cc, x, y);
                let r = self.bool_of(c);
                self.st(*dst, 0, r);
            }
            Op::Int {
                op,
                dst,
                a,
                b,
                lo,
                hi,
                site,
            } => {
                let (x, y) = (self.ld(*a, 0), self.ld(*b, 0));
                let r = self.arith(*op, x, y, *lo, *hi, *site);
                self.st(*dst, 0, r);
            }
            Op::Range { a, lo, hi, site } => {
                if self.checks {
                    let x = self.ld(*a, 0);
                    let f = self.outside(x, *lo, *hi);
                    self.check(f, *site, &[x]);
                }
            }
            Op::IndexRead {
                dst,
                src,
                lo,
                n,
                stride,
                w,
                idx,
                left,
                down,
                site,
            } => {
                // Array elements the value spans: one for a part of one element.
                let count = (w / stride).max(1);
                let bit = self.element(*idx, *left, *down, *n, *stride, count, *lo, *site);
                let end = lo + n * stride;
                // The word after the value too: an element may reach into it.
                self.spill(*src, lo / 64, end.div_ceil(64));
                let v = self.dyn_read(*src, bit, *w, Self::aligned(*lo, *stride, *w));
                self.sts(*dst, &v);
            }
            Op::IndexWrite {
                dst,
                src,
                lo,
                n,
                stride,
                w,
                idx,
                left,
                down,
                site,
            } => {
                let v = self.lds(*src, *w);
                // Array elements the value spans: one for a part of one element.
                let count = (w / stride).max(1);
                let bit = self.element(*idx, *left, *down, *n, *stride, count, *lo, *site);
                let (k0, k1) = (lo / 64, (lo + n * stride).div_ceil(64));
                self.spill(*dst, k0, k1);
                self.forget(*dst, k0, k1);
                self.dyn_write(*dst, bit, *w, &v, Self::aligned(*lo, *stride, *w));
            }
            Op::Select { dst, c, a, b, len } => {
                let c = self.ld(*c, 0);
                let (x, y) = (self.lds(*a, *len), self.lds(*b, *len));
                let v: Vec<Value> = (x.iter().zip(&y))
                    .map(|(&x, &y)| self.b.ins().select(c, x, y))
                    .collect();
                self.sts(*dst, &v);
            }
            Op::Splat { dst, a, len } => {
                let x = self.ld(*a, 0);
                let z = self.konst(0);
                let ones = self.b.ins().isub(z, x);
                let mut v = vec![ones; words(*len) as usize];
                self.top(&mut v, *len);
                self.sts(*dst, &v);
            }
            Op::Ext {
                dst,
                src,
                from,
                to,
                signed,
            } => {
                let x = self.lds(*src, *from);
                let v = self.extend(&x, *from, *to, *signed);
                self.sts(*dst, &v);
            }
            Op::VCmp {
                cmp,
                signed,
                dst,
                a,
                b,
                len,
            } => {
                let full = 64 * words(*len);
                let x = self.lds(*a, *len);
                let y = self.lds(*b, *len);
                let (x, y) = if *signed {
                    (
                        self.extend(&x, *len, full, true),
                        self.extend(&y, *len, full, true),
                    )
                } else {
                    (x, y)
                };
                let c = match cmp {
                    Cmp::Lt => self.less(&x, &y, *signed, false),
                    Cmp::Le => self.less(&x, &y, *signed, true),
                    Cmp::Gt => self.less(&y, &x, *signed, false),
                    Cmp::Ge => self.less(&y, &x, *signed, true),
                    Cmp::Eq | Cmp::Ne => {
                        let mut d = self.konst(0);
                        for (&x, &y) in x.iter().zip(&y) {
                            let t = self.b.ins().bxor(x, y);
                            d = self.b.ins().bor(d, t);
                        }
                        let cc = if *cmp == Cmp::Ne {
                            IntCC::NotEqual
                        } else {
                            IntCC::Equal
                        };
                        self.b.ins().icmp_imm_s(cc, d, 0)
                    }
                };
                let r = self.bool_of(c);
                self.st(*dst, 0, r);
            }
            Op::Mul { dst, a, b, len } => {
                let (x, y) = (self.lds(*a, *len), self.lds(*b, *len));
                let mut v = self.mul_words(&x, &y, words(*len) as usize);
                self.top(&mut v, *len);
                self.sts(*dst, &v);
            }
            Op::VDiv {
                op,
                signed,
                dst,
                a,
                b,
                len,
                site,
            } => {
                // One word (`len` at most 64): extend to the word, divide, cut back.
                let x = self.ld(*a, 0);
                let y = self.ld(*b, 0);
                let (x, y) = if *signed {
                    let e = |g: &mut Self, v| g.extend(&[v], *len, 64, true)[0];
                    (e(self, x), e(self, y))
                } else {
                    (x, y)
                };
                let z = self.b.ins().icmp_imm_s(IntCC::Equal, y, 0);
                // `MIN / -1` traps in the machine: only a 64-element signed operand can be it.
                let f = if *signed && *len == 64 {
                    let m1 = self.b.ins().icmp_imm_s(IntCC::Equal, y, -1);
                    let mn = self.b.ins().icmp_imm_s(IntCC::Equal, x, i64::MIN);
                    let ov = self.b.ins().band(m1, mn);
                    self.b.ins().bor(z, ov)
                } else {
                    z
                };
                let y = self.div_guard(f, *site, x, y);
                let r = match (op, signed) {
                    (Arith::Div, false) => self.b.ins().udiv(x, y),
                    (Arith::Div, true) => self.b.ins().sdiv(x, y),
                    (_, false) => self.b.ins().urem(x, y),
                    (_, true) => {
                        let r = self.b.ins().srem(x, y);
                        if *op == Arith::Mod {
                            self.mod_fix(r, y)
                        } else {
                            r
                        }
                    }
                };
                let mut v = vec![r];
                self.top(&mut v, *len);
                self.st(*dst, 0, v[0]);
            }
            Op::Shift { k, dst, a, len, n } => {
                // One word: the count from the integer word `n`, at least 0.
                let x = self.ld(*a, 0);
                let c = self.ld(*n, 0);
                let l = i64::from(*len);
                let r = match k {
                    ShiftK::Sll | ShiftK::Srl | ShiftK::Sra => {
                        let big = self
                            .b
                            .ins()
                            .icmp_imm_s(IntCC::UnsignedGreaterThanOrEqual, c, l);
                        let x2 = if *k == ShiftK::Sra {
                            self.extend(&[x], *len, 64, true)[0]
                        } else {
                            x
                        };
                        let cm = self.b.ins().band_imm_s(c, 63);
                        let s = match k {
                            ShiftK::Sll => self.b.ins().ishl(x2, cm),
                            ShiftK::Srl => self.b.ins().ushr(x2, cm),
                            _ => self.b.ins().sshr(x2, cm),
                        };
                        let full = if *k == ShiftK::Sra {
                            self.b.ins().sshr_imm_s(x2, 63)
                        } else {
                            self.konst(0)
                        };
                        self.b.ins().select(big, full, s)
                    }
                    ShiftK::Rol | ShiftK::Ror => {
                        // By `m = c mod len`: `x << p | x >> q` with `p + q = len`, both below 64
                        // unless `m` is 0, which is `x` itself.
                        let m = self.b.ins().urem_imm_s(c, l);
                        let lk = self.b.ins().iconst(I64, l);
                        let back = self.b.ins().isub(lk, m);
                        let (p, q) = if *k == ShiftK::Rol {
                            (m, back)
                        } else {
                            (back, m)
                        };
                        let hi = self.b.ins().ishl(x, p);
                        let lo = self.b.ins().ushr(x, q);
                        let r = self.b.ins().bor(hi, lo);
                        let none = self.b.ins().icmp_imm_s(IntCC::Equal, m, 0);
                        self.b.ins().select(none, x, r)
                    }
                };
                let mut v = vec![r];
                self.top(&mut v, *len);
                self.st(*dst, 0, v[0]);
            }
            Op::Reduce { g, dst, a, len } => {
                let x = self.lds(*a, *len);
                let r = match g {
                    Gate::And | Gate::Nand => {
                        let mut d = self.konst(0);
                        for (k, &w) in x.iter().enumerate() {
                            let m = mask((*len - 64 * k as u32).min(64));
                            let t = self.b.ins().bxor_imm_s(w, m as i64);
                            d = self.b.ins().bor(d, t);
                        }
                        self.b.ins().icmp_imm_s(IntCC::Equal, d, 0)
                    }
                    Gate::Or | Gate::Nor => {
                        let mut d = self.konst(0);
                        for &w in &x {
                            d = self.b.ins().bor(d, w);
                        }
                        self.b.ins().icmp_imm_s(IntCC::NotEqual, d, 0)
                    }
                    Gate::Xor | Gate::Xnor => {
                        let mut d = self.konst(0);
                        for &w in &x {
                            d = self.b.ins().bxor(d, w);
                        }
                        let p = self.b.ins().popcnt(d);
                        let p = self.b.ins().band_imm_s(p, 1);
                        self.b.ins().icmp_imm_s(IntCC::NotEqual, p, 0)
                    }
                };
                let mut r = self.bool_of(r);
                if matches!(g, Gate::Nand | Gate::Nor | Gate::Xnor) {
                    r = self.b.ins().bxor_imm_s(r, 1);
                }
                self.st(*dst, 0, r);
            }
            Op::ToInt {
                dst,
                a,
                len,
                signed,
            } => {
                let x = self.ld(*a, 0);
                let r = if *signed {
                    self.extend(&[x], *len, 64, true)[0]
                } else {
                    x
                };
                self.st(*dst, 0, r);
            }
            Op::FromInt { dst, a, len } => {
                let x = self.ld(*a, 0);
                let s = self.b.ins().sshr_imm_s(x, 63);
                let mut v = vec![s; words(*len) as usize];
                v[0] = x;
                self.top(&mut v, *len);
                self.sts(*dst, &v);
            }
            // Only in a member's last run, which has the exits of its checks.
            Op::Note { .. } if !self.exits => {}
            Op::Note { site, args } => {
                for &(r, n) in args {
                    for k in 0..n {
                        let off = word(r) + k as i32 * 8;
                        let v = self.ld(r, k);
                        self.store(off, v);
                        if let Some(e) = self.cache.get_mut(&off) {
                            e.1 = false;
                        }
                    }
                }
                // log[min(count, cap)] = site; count += 1 (the run reads a count over the cap
                // as an internal error).
                let (at, cap) = self.log;
                let cnt = self
                    .b
                    .ins()
                    .load(I64, MemFlagsData::trusted(), self.base, at);
                let top = self.konst(u64::from(cap));
                let k = self.b.ins().umin(cnt, top);
                let k = self.b.ins().ishl_imm_s(k, 3);
                let a = self.b.ins().iadd(self.base, k);
                let sv = self.konst(u64::from(*site));
                self.b.ins().store(MemFlagsData::trusted(), sv, a, at + 8);
                let one = self.b.ins().iadd_imm_s(cnt, 1);
                self.b
                    .ins()
                    .store(MemFlagsData::trusted(), one, self.base, at);
            }
            Op::If { cond, then, else_ } => {
                let c = self.ld(*cond, 0);
                let (t, e, j): (CBlock, CBlock, CBlock) = (
                    self.b.create_block(),
                    self.b.create_block(),
                    self.b.create_block(),
                );
                // Memory holds every word before the branches; each starts from the words in
                // registers, stores what it changed before the join, and the join reads those
                // from memory again.
                let dirty: Vec<(i32, Value)> = (self.cache.iter_mut())
                    .filter(|(_, (_, d))| *d)
                    .map(|(&off, (v, d))| {
                        *d = false;
                        (off, *v)
                    })
                    .collect();
                for (off, v) in dirty {
                    self.store(off, v);
                }
                self.b.ins().brif(c, t, &[], e, &[]);
                let before = self.cache.clone();
                let mut touched = Vec::new();
                for (blk, body) in [(t, then), (e, else_)] {
                    self.b.switch_to_block(blk);
                    self.b.seal_block(blk);
                    self.cache = before.clone();
                    self.ops(body);
                    let made: Vec<(i32, Value, bool)> = (self.cache.iter())
                        .filter(|(off, x)| before.get(off) != Some(x))
                        .map(|(&off, &(v, d))| (off, v, d))
                        .collect();
                    for (off, v, d) in made {
                        if d {
                            self.store(off, v);
                        }
                        touched.push(off);
                    }
                    // Words the branch dropped (it wrote them in memory) are stale after it too.
                    touched.extend(before.keys().filter(|k| !self.cache.contains_key(k)));
                    self.b.ins().jump(j, &[]);
                }
                self.b.switch_to_block(j);
                self.b.seal_block(j);
                self.cache = before;
                for off in touched {
                    self.cache.remove(&off);
                }
            }
        }
    }

    /// `mod` from the remainder `r` of a division by `y`: the sign of the divisor (LRM 9.2.7),
    /// `r + y` when `r` and `y` differ in sign.
    fn mod_fix(&mut self, r: Value, y: Value) -> Value {
        let s = self.b.ins().bxor(r, y);
        let neg = self.b.ins().icmp_imm_s(IntCC::SignedLessThan, s, 0);
        let nz = self.b.ins().icmp_imm_s(IntCC::NotEqual, r, 0);
        let fix = self.b.ins().band(neg, nz);
        let ry = self.b.ins().iadd(r, y);
        self.b.ins().select(fix, ry, r)
    }

    /// Whether `x` is outside `lo ..= hi`.
    fn outside(&mut self, x: Value, lo: i64, hi: i64) -> Value {
        let a = self.b.ins().icmp_imm_s(IntCC::SignedLessThan, x, lo);
        let b = self.b.ins().icmp_imm_s(IntCC::SignedGreaterThan, x, hi);
        self.b.ins().bor(a, b)
    }

    /// `x op y` in `lo ..= hi`, the result type: a failed check stores both operands.
    fn arith(&mut self, op: Arith, x: Value, y: Value, lo: i64, hi: i64, site: u32) -> Value {
        match op {
            Arith::Add | Arith::Sub | Arith::Mul => {
                if !self.checks {
                    return match op {
                        Arith::Add => self.b.ins().iadd(x, y),
                        Arith::Sub => self.b.ins().isub(x, y),
                        _ => self.b.ins().imul(x, y),
                    };
                }
                let (r, of) = match op {
                    Arith::Add => self.b.ins().sadd_overflow(x, y),
                    Arith::Sub => self.b.ins().ssub_overflow(x, y),
                    _ => self.b.ins().smul_overflow(x, y),
                };
                let out = self.outside(r, lo, hi);
                let f = self.b.ins().bor(of, out);
                self.check(f, site, &[x, y]);
                r
            }
            Arith::Div | Arith::Mod | Arith::Rem => {
                // Always guarded: the machine division traps on zero and on MIN / -1.
                let z = self.b.ins().icmp_imm_s(IntCC::Equal, y, 0);
                let m1 = self.b.ins().icmp_imm_s(IntCC::Equal, y, -1);
                let mn = self.b.ins().icmp_imm_s(IntCC::Equal, x, i64::MIN);
                let ov = self.b.ins().band(m1, mn);
                let f = self.b.ins().bor(z, ov);
                let y = self.div_guard(f, site, x, y);
                let r = match op {
                    Arith::Div => self.b.ins().sdiv(x, y),
                    _ => self.b.ins().srem(x, y),
                };
                let r = if op == Arith::Mod {
                    self.mod_fix(r, y)
                } else {
                    r
                };
                if self.checks && op == Arith::Div {
                    let out = self.outside(r, lo, hi);
                    self.check(out, site, &[x, y]);
                }
                r
            }
        }
    }
}
