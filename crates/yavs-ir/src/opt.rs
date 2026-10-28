//! Machine-independent optimisation of a function's code, run once per function when the
//! program is finished (after flag propagation, before resume points are numbered and the
//! verifier runs).
//!
//! Lowering emits every operand where it is used, so the code is full of constants, repeated
//! address and descriptor computations and loads of what was just stored. This pass walks
//! the extended basic blocks of a function (a block and, depth first, the blocks only it can
//! reach), keeping a table of available values, and
//!
//! - folds constants and trivial arithmetic (`x * 1`, `x + 0`, `x * 0`) and branches on
//!   constants,
//! - turns an offset from a known address into a known address (`AddrOf`),
//! - reads constant-pool descriptors and scalars at compile time,
//! - removes an instruction that repeats an earlier one (same operands, same result),
//!   including checks that cannot fail a second time,
//! - forwards a stored value to a later load of the same address,
//! - drops pure definitions nobody reads,
//! - keeps scalar frame slots that only direct loads and stores reach in registers
//!   (`Opt::promote`, then the walk runs again over the result),
//! - in a subprogram that cannot suspend, moves computations that do not change in a loop to
//!   a block run before it (`licm.rs`),
//! - and, last, empties the blocks no branch reaches, drops the `Stmt` markers of statements
//!   that allocate no temporary memory, merges registers holding the same constant or address,
//!   and gives the registers in use consecutive numbers.
//!
//! Invariants it keeps (ir.md 1.2): no register becomes live across a suspension (a
//! register is replaced by another only when the other is defined since the last
//! suspension on the path, or the function never suspends; every table is dropped at a
//! suspension), every surviving instruction keeps its statement, `Stmt` and `Cover` are
//! never touched (but see `drop_statements`), a check is removed only when it provably passes, and stores are never
//! removed (debuggers read frame slots), except those of slots promoted to registers, which no
//! debugger reads (see [`keep_variables`]). Memory facts are dropped at calls, at writes
//! through pointers and, for what a debugger may write, at every `Stmt`. Functions of kind
//! `Elab` and `Thunk` run once per instance and are left alone. `YAVS_NO_IROPT=1` turns the
//! pass off (differential testing).

mod cfg;
mod decl;
mod drives;
mod inline;
mod layout_cache;
mod licm;
mod live;
mod memo;
mod range;
mod share;
mod sink;
mod slots;

pub use inline::FwdCache;

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::rc::Rc;
use std::sync::OnceLock;

use crate::{
    AddrSpace, BlockId, BoolOp, CheckKind, CheckMsgId, CmpOp, FuncFlags, FuncId, FuncKind, Inst,
    IntOp, IntUnOp, LOp, Program, Reg, RegTy, Role, StmtId, Ty, TyId, Width, dim_len,
};

/// Is the pass enabled? `YAVS_NO_IROPT` set to anything but `0` or the empty string turns
/// it off.
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("YAVS_NO_IROPT")
            .map(|v| v.is_empty() || v == "0")
            .unwrap_or(true)
    })
}

/// Is the memoisation of instance-constant values in processes on? Off for a debugger and
/// with `YAVS_NO_MEMO` set to anything but `0` or the empty string (differential testing).
fn memo_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("YAVS_NO_MEMO")
            .map(|v| v.is_empty() || v == "0")
            .unwrap_or(true)
    }) && !KEEP_VARS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Is the clean-up of finished subprograms (`cfg.rs`) on? Off with `YAVS_NO_CFG` set to anything
/// but `0` or the empty string (differential testing).
fn cfg_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("YAVS_NO_CFG")
            .map(|v| v.is_empty() || v == "0")
            .unwrap_or(true)
    })
}

/// Are repeated drives of one driver in a process delayed (`drives.rs`)? Off with `YAVS_NO_DRIVES`
/// set to anything but `0` or the empty string (differential testing).
fn drives_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("YAVS_NO_DRIVES")
            .map(|v| v.is_empty() || v == "0")
            .unwrap_or(true)
    })
}

/// Are the constant-size temporaries of processes kept in their frames (`slots.rs`)? Off with
/// `YAVS_NO_SLOTS` set to anything but `0` or the empty string (differential testing).
fn slots_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("YAVS_NO_SLOTS")
            .map(|v| v.is_empty() || v == "0")
            .unwrap_or(true)
    })
}

static KEEP_VARS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Keep the variables of subprograms in their frame slots, where a debugger reads and writes
/// them (promotion to registers is off). Call it before the first function is asked for,
/// i.e. before elaboration, when a debugger will attach.
pub fn keep_variables() {
    KEEP_VARS.store(true, std::sync::atomic::Ordering::Relaxed);
}

static COMPILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Every function will be compiled to native code (`--tier=llvm`): the memoisation of
/// instance-constant values asks for a larger gain per copy. Call it before the first function
/// is asked for.
pub fn compiled_tier() {
    COMPILED.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn compiled() -> bool {
    COMPILED.load(std::sync::atomic::Ordering::Relaxed)
}

static LAYOUT_SLOTS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Some functions will be compiled to native code (`--tier=auto`): the frames of processes get
/// the layout slots of `layout_cache` as under `--tier=llvm` (user, 2026-10-23: the peak memory
/// is the price of the compiled code's hits). Call it before the first function is asked for.
pub fn layout_slots() {
    LAYOUT_SLOTS.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn layout_slots_on() -> bool {
    compiled() || LAYOUT_SLOTS.load(std::sync::atomic::Ordering::Relaxed)
}

static FRAME_BLOCKS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The 32-byte blocks the interpreter fuses become frame bytes as under [`layout_slots`], without
/// the layout slots (`--tier=clift-auto`: its compiled code is faster with the blocks in the frame
/// and does not use the slots, user 2026-10-26). Call it before the first function is asked for.
pub fn frame_blocks() {
    FRAME_BLOCKS.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn frame_blocks_on() -> bool {
    layout_slots_on() || FRAME_BLOCKS.load(std::sync::atomic::Ordering::Relaxed)
}

static EXTERNAL_VARS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Process variables may be read or written from outside (a plugin deposits a value through
/// VHPI between two activations, the waveform recorder samples them): the named ones stay in
/// their slots, the optimiser knows nothing about a variable across a suspension, and copies of
/// processes are neither stored nor loaded. Call it before the first function is asked for.
pub fn external_variables() {
    EXTERNAL_VARS.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn external() -> bool {
    EXTERNAL_VARS.load(std::sync::atomic::Ordering::Relaxed)
}

/// May processes get copies specialised to their instance data (`copies.rs`)? Not with the
/// pass off, nor while a debugger keeps variables in their slots, nor when a plugin or the
/// waveform recorder reads or writes process variables, nor under `--tier=llvm`, which compiles
/// the program before any copy exists, nor with `YAVS_NO_SPEC` (differential testing).
pub fn specialise_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("YAVS_NO_SPEC")
            .map(|v| v.is_empty() || v == "0")
            .unwrap_or(true)
    }) && enabled()
        && !KEEP_VARS.load(std::sync::atomic::Ordering::Relaxed)
        && !external()
        && !compiled()
}

/// Whether an optimised copy made now is the one every run makes: the pass is on and no debugger
/// keeps variables in their slots ([`keep_variables`]) and the program is not compiled ([`compiled_tier`]). Only such copies are stored and used again
/// (`Program::export_optimized`).
pub fn copies_are_portable() -> bool {
    enabled() && !KEEP_VARS.load(std::sync::atomic::Ordering::Relaxed) && !compiled()
}

/// Every switch that changes the code the optimiser or the tiers generate, as text: part of the
/// key of every cache entry that holds generated code, so a run with other switches never reads
/// what another configuration made (#491). Read when a key is made, i.e. after the command line
/// set [`keep_variables`], [`compiled_tier`] and [`external_variables`].
pub fn fingerprint() -> String {
    fingerprint_of([
        enabled(),
        memo_enabled(),
        cfg_enabled(),
        inline::enabled(),
        KEEP_VARS.load(std::sync::atomic::Ordering::Relaxed),
        compiled(),
        external(),
        layout_slots_on(),
        drives_enabled(),
        slots_enabled(),
        frame_blocks_on(),
    ])
}

fn fingerprint_of(f: [bool; 11]) -> String {
    let names = [
        "iropt", "memo", "cfg", "inline", "keep", "compiled", "external", "layouts", "drives",
        "slots", "blocks",
    ];
    let parts: Vec<String> = names
        .iter()
        .zip(f)
        .map(|(n, v)| format!("{n}={v}"))
        .collect();
    parts.join(" ")
}

#[cfg(test)]
mod fingerprint_tests {
    use super::fingerprint_of;

    /// Every switch is part of the fingerprint (#491).
    #[test]
    fn each_switch_changes_the_fingerprint() {
        let base = fingerprint_of([false; 11]);
        for i in 0..11 {
            let mut f = [false; 11];
            f[i] = true;
            assert_ne!(fingerprint_of(f), base, "switch {i}");
        }
    }
}

/// Field offsets of the static record layouts met by the optimiser, kept in the program so
/// that the functions optimised one by one share them. A copy of the program starts empty.
#[derive(Default, Debug)]
pub struct LayoutCache(std::sync::Mutex<FxMap<TyId, Option<Vec<u64>>>>);

impl Clone for LayoutCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

/// A fast, non-cryptographic hasher: the tables are small, built per function and never
/// see hostile keys.
#[derive(Default)]
struct Fx(u64);

impl Fx {
    #[inline]
    fn add(&mut self, w: u64) {
        self.0 = (self.0.rotate_left(5) ^ w).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

impl Hasher for Fx {
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.add(u64::from(*b));
        }
    }
    fn write_u8(&mut self, i: u8) {
        self.add(u64::from(i));
    }
    fn write_u32(&mut self, i: u32) {
        self.add(u64::from(i));
    }
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }
    fn write_i64(&mut self, i: i64) {
        self.add(i as u64);
    }
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
    fn write_isize(&mut self, i: isize) {
        self.add(i as u64);
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<Fx>>;

/// The optimised copy of function `id`, or `None` when the pass is off or leaves it alone.
/// Reads only what lowering produced of other functions (flags, kinds, types), so the result
/// does not depend on which function was optimised first.
pub(crate) fn optimize_func(prog: &Program, id: FuncId) -> Option<Box<crate::Func>> {
    optimize_func_with(prog, id, None)
}

fn optimize_func_with(
    prog: &Program,
    id: FuncId,
    inst: Option<InstView<'_>>,
) -> Option<Box<crate::Func>> {
    let mut f = optimize_body_with(prog, id, inst)?;
    if inline::run(prog, id, &mut f, 1) | inline::run_wide(prog, id, &mut f) {
        // The inlined bodies are code of this function now: what is known at the call decides
        // their branches and repeated reads are shared.
        if matches!(f.kind, FuncKind::Function | FuncKind::Procedure) {
            let mut layouts = prog
                .layouts
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            tidy(prog, &mut f, &mut layouts, None);
        }
        // What the inlined callees read only on one branch is computed there.
        sink::run(prog, &mut f);
        crate::build::number_func(&mut f, |c| {
            prog.funcs
                .get(c)
                .is_some_and(|c| c.flags.contains(FuncFlags::MAY_SUSPEND))
        });
        #[cfg(debug_assertions)]
        {
            let mut errs = Vec::new();
            crate::verify::verify_func(prog, id, &f, &mut errs);
            assert!(errs.is_empty(), "inlining broke {}: {errs:?}", f.name);
        }
    }
    // Temporaries of a process in its frame, last (inlining and `sink` work on `Alloc`s): the
    // statements that allocate nothing any more lose their markers.
    if slots_enabled()
        && !KEEP_VARS.load(std::sync::atomic::Ordering::Relaxed)
        && slots::run(prog, &mut f)
    {
        {
            let mut layouts = prog
                .layouts
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            drop_markers(prog, &mut f, &mut layouts);
        }
        // (No second `renumber`: sharing the registers again cost the predecoder fusions that
        // want a register defined or read once; the `Alloc` registers left unused are gaps.)
        crate::build::number_func(&mut f, |c| {
            prog.funcs
                .get(c)
                .is_some_and(|c| c.flags.contains(FuncFlags::MAY_SUSPEND))
        });
        #[cfg(debug_assertions)]
        {
            let mut errs = Vec::new();
            crate::verify::verify_func(prog, id, &f, &mut errs);
            assert!(errs.is_empty(), "optimiser broke {}: {errs:?}", f.name);
        }
    }
    Some(f)
}

/// The instance data a copy is made for and the byte ranges of it (offset, length) that are not
/// folded.
/// The instance data a copy is for, its `for` generate parameters (not folded) and the data of the
/// packages its declarative part reads.
type InstView<'a> = (&'a [u8], &'a [(u32, u32)], &'a [(u32, &'a [u8])]);

/// The copy of process `id` for the instance data `data` (`holes` are not folded): the
/// function optimised as [`optimize_func`] does, with reads of the data folded.
pub(crate) fn optimize_copy(
    prog: &Program,
    id: FuncId,
    data: &[u8],
    holes: &[(u32, u32)],
    pkgs: &[(u32, &[u8])],
) -> Option<Box<crate::Func>> {
    optimize_func_with(prog, id, Some((data, holes, pkgs)))
}

/// Is a debugger keeping the variables of subprograms in their slots?
fn keep_variables_on() -> bool {
    KEEP_VARS.load(std::sync::atomic::Ordering::Relaxed)
}

/// The function optimised, without inlining: the copy [`optimize_func`] starts from.
pub(crate) fn optimize_body(prog: &Program, id: FuncId) -> Option<Box<crate::Func>> {
    optimize_body_with(prog, id, None)
}

fn optimize_body_with(
    prog: &Program,
    id: FuncId,
    inst: Option<InstView<'_>>,
) -> Option<Box<crate::Func>> {
    if !enabled() {
        return None;
    }
    let raw = prog.funcs.get(id)?.full();
    if matches!(raw.kind, FuncKind::Elab | FuncKind::Thunk) || raw.code.len() < 2 {
        return None;
    }
    let tabbed = inline::tab_calls(prog, id, raw);
    let raw = tabbed.as_deref().unwrap_or(raw);
    let mut layouts = prog
        .layouts
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut o = Opt::new(
        prog,
        raw,
        raw.code.clone(),
        raw.operands.clone(),
        raw.nested,
        &mut layouts,
    );
    o.inst = inst;
    o.fid = Some(id);
    let had_inst = o.inst.is_some();
    o.run();
    let split = o.split;
    let mut loc = raw.loc.clone();
    let blocks = o.changed.then(|| o.compact(&mut loc, None));
    let mut f = raw.shell();
    if let Some(blocks) = blocks {
        for (i, nb) in blocks.iter().enumerate() {
            if let Some(b) = f.blocks.get_mut(BlockId(i as u32)) {
                b.start = nb.0;
                b.end = nb.1;
            }
        }
    }
    f.code = o.code;
    f.loc = loc;
    f.operands = o.operands;
    f.reg_ty.extend(o.new_regs);
    if split {
        tidy(prog, &mut f, &mut layouts, inst);
    }
    if cfg_enabled()
        && matches!(f.kind, FuncKind::Function | FuncKind::Procedure)
        && f.body.is_none()
        && !f.flags.contains(FuncFlags::MAY_SUSPEND)
        && !f.code.iter().any(|i| matches!(i, Inst::Suspend { .. }))
    {
        let threaded = cfg::run(prog, &mut f);
        if licm::run(prog, &mut f) | threaded {
            tidy(prog, &mut f, &mut layouts, None);
        }
    } else if cfg_enabled() && f.kind == FuncKind::Process {
        // Loops of a process that do not suspend (#541).
        licm::run(prog, &mut f);
    }
    // A default drive that a later drive of the same driver deletes is done only where none
    // follows; the statements it leaves empty lose their markers.
    if drives_enabled() && f.kind == FuncKind::Process && drives::run(prog, &mut f) {
        drop_markers(prog, &mut f, &mut layouts);
    }
    if memo_enabled() {
        memo::run(prog, &mut f);
        // Frame bytes only a compiled tier uses (`--tier=interp` has none).
        if layout_slots_on() {
            layout_cache::run(prog, &mut f);
        }
    }
    // Short-circuit chains of a process branch where their inputs are known (#791), after the
    // memo (which wants the blocks as lowered); not with a debugger reading the variables or with
    // coverage counters.
    if cfg_enabled()
        && f.kind == FuncKind::Process
        && !keep_variables_on()
        && !f.code.iter().any(|i| {
            matches!(
                i,
                Inst::Cover { .. } | Inst::CoverIn { .. } | Inst::CoverCond { .. }
            )
        })
    {
        cfg::run(prog, &mut f);
    }
    if had_inst && f.kind == FuncKind::Process && f.via.is_empty() {
        drop_declarations(prog, &mut f);
    }
    renumber(prog, &mut f);
    crate::build::number_func(&mut f, |c| {
        prog.funcs
            .get(c)
            .is_some_and(|c| c.flags.contains(FuncFlags::MAY_SUSPEND))
    });
    #[cfg(debug_assertions)]
    {
        let mut errs = Vec::new();
        crate::verify::verify_func(prog, id, &f, &mut errs);
        assert!(
            errs.is_empty(),
            "optimiser broke {}: {errs:?}\n{:#?}",
            f.name,
            f.code
        );
    }
    Some(Box::new(f))
}

/// Walk the finished code of `f` once more after the clean-up copied blocks into their
/// predecessors: a value known where a block is entered now decides the branches of its copy.
fn tidy(
    prog: &Program,
    f: &mut crate::Func,
    layouts: &mut FxMap<TyId, Option<Vec<u64>>>,
    inst: Option<InstView<'_>>,
) {
    let done = {
        let mut o = Opt::new(
            prog,
            f,
            f.code.clone(),
            f.operands.clone(),
            f.nested,
            layouts,
        );
        o.inst = inst;
        o.analyse();
        o.pass();
        let mut again = 0;
        while o.pruned && again < 4 {
            o.pruned = false;
            again += 1;
            o.dce();
            o.reanalyse();
            o.pass();
        }
        if !o.changed {
            return;
        }
        o.dce();
        o.kill_unreachable();
        o.dead_defs();
        let mut loc = f.loc.clone();
        let mut via = f.via.clone();
        let blocks = o.compact(&mut loc, (!via.is_empty()).then_some(&mut via));
        (o.code, o.operands, o.new_regs, loc, via, blocks)
    };
    let (code, operands, new_regs, loc, via, blocks) = done;
    for (i, nb) in blocks.iter().enumerate() {
        if let Some(b) = f.blocks.get_mut(BlockId(i as u32)) {
            b.start = nb.0;
            b.end = nb.1;
        }
    }
    f.code = code;
    f.loc = loc;
    f.via = via;
    f.operands = operands;
    f.reg_ty.extend(new_regs);
}

/// A copy of a process, specialised to its instance data, starts after the declarative part:
/// it takes over between two activations of the shared code (`Machine::specialise`), the
/// variables are in their frame slots, and no registers carry over. The blocks of the
/// declarative part (the descriptors and initial values of the variables, often the largest
/// part of a process of records with generic bounds) are never run in it: the entry jumps
/// to the body and the blocks nothing else reaches are emptied.
fn drop_declarations(prog: &Program, f: &mut crate::Func) {
    let Some(body) = f.body else { return };
    let nb = f.blocks.len();
    let body = body.0 as usize;
    let blk = |f: &crate::Func, b: usize| f.blocks.values()[b];
    if body == 0 || body >= nb || blk(f, 0).end <= blk(f, 0).start {
        return;
    }
    // The graph with the entry going straight to the body.
    let mut succ = memo::successors(prog, f);
    succ[0] = vec![body];
    let mut reach = vec![false; nb];
    let mut stack = vec![0usize];
    while let Some(b) = stack.pop() {
        if !std::mem::replace(&mut reach[b], true) {
            stack.extend(succ[b].iter().copied().filter(|&t| t < nb));
        }
    }
    // Frames start with the registers defined once by a constant or an address, wherever the
    // definition is: those the body reads keep their definition, in the entry block.
    let mut defs = vec![0u32; f.reg_ty.len()];
    let mut read = vec![false; f.reg_ty.len()];
    for (b, &live) in reach.iter().enumerate() {
        let r = blk(f, b);
        for i in &f.code[r.start as usize..r.end as usize] {
            for_defs_fast(i, |d| defs[d.0 as usize] += 1);
            if live && b != 0 {
                i.visit(&mut |x, role, _| {
                    if role == Role::Use {
                        read[x.0 as usize] = true;
                    }
                });
                if let Some(l) = i.list() {
                    for x in f
                        .operands
                        .get(l.start as usize..(l.start + l.len) as usize)
                        .unwrap_or(&[])
                    {
                        read[x.0 as usize] = true;
                    }
                }
            }
        }
    }
    // The definitions to keep, from every block that is gone, in order.
    let mut keep: Vec<(Inst, StmtId)> = Vec::new();
    for (b, &live) in reach.iter().enumerate() {
        if live && b != 0 {
            continue;
        }
        let r = blk(f, b);
        for pc in r.start as usize..r.end as usize {
            if let Inst::Const { dst, .. } | Inst::ConstF { dst, .. } | Inst::AddrOf { dst, .. } =
                f.code[pc]
                && defs[dst.0 as usize] == 1
                && read[dst.0 as usize]
            {
                keep.push((f.code[pc], f.loc[pc]));
            }
        }
    }
    let (mut code, mut loc) = (
        Vec::with_capacity(f.code.len()),
        Vec::with_capacity(f.code.len()),
    );
    let at = blk(f, 0).start as usize;
    for (b, &live) in reach.iter().enumerate() {
        let r = blk(f, b);
        let start = code.len() as u32;
        let range = r.start as usize..r.end as usize;
        if b == 0 {
            for (i, l) in &keep {
                code.push(*i);
                loc.push(*l);
            }
            code.push(Inst::Br {
                to: BlockId(body as u32),
            });
            loc.push(f.loc[at]);
        } else if live {
            code.extend_from_slice(&f.code[range.clone()]);
            loc.extend_from_slice(&f.loc[range]);
        } else {
            // Every block holds an instruction: a dead one jumps to the next block, which the
            // predecoder runs through (a jump to itself would stop the fusions of the block
            // before it, #791).
            code.push(Inst::Br {
                to: BlockId(if b + 1 < nb { b as u32 + 1 } else { b as u32 }),
            });
            loc.push(f.loc[range.start]);
        }
        if let Some(x) = f.blocks.get_mut(BlockId(b as u32)) {
            *x = crate::Block {
                start,
                end: code.len() as u32,
            };
        }
    }
    f.code = code;
    f.loc = loc;
}

/// Drop the `Stmt` markers a pass left with nothing to free (`Opt::drop_statements`).
fn drop_markers(prog: &Program, f: &mut crate::Func, layouts: &mut FxMap<TyId, Option<Vec<u64>>>) {
    let mut o = Opt::new(
        prog,
        f,
        f.code.clone(),
        f.operands.clone(),
        f.nested,
        layouts,
    );
    o.drop_statements();
    if !o.changed {
        return;
    }
    let mut loc = f.loc.clone();
    let mut via = f.via.clone();
    let blocks = o.compact(&mut loc, (!via.is_empty()).then_some(&mut via));
    let code = o.code;
    for (i, nb) in blocks.iter().enumerate() {
        if let Some(b) = f.blocks.get_mut(BlockId(i as u32)) {
            b.start = nb.0;
            b.end = nb.1;
        }
    }
    f.code = code;
    f.loc = loc;
    f.via = via;
}

/// Give the registers the code uses consecutive numbers: lowering and the passes leave gaps
/// (registers nobody uses any more), and every call copies a frame template with one cell
/// per register. A signal reference takes two cells, the second of which keeps its place
/// behind the first.
fn renumber(prog: &Program, f: &mut crate::Func) {
    let n = f.reg_ty.len();
    let mut used = vec![false; n];
    for i in &f.code {
        i.visit(&mut |r, _, _| {
            if let Some(u) = used.get_mut(r.0 as usize) {
                *u = true;
            }
        });
        if let Some(l) = i.list() {
            for r in f
                .operands
                .get(l.start as usize..(l.start + l.len) as usize)
                .unwrap_or(&[])
            {
                if let Some(u) = used.get_mut(r.0 as usize) {
                    *u = true;
                }
            }
        }
    }
    for r in 0..n.saturating_sub(1) {
        if f.reg_ty[r] == RegTy::Sig && (used[r] || used[r + 1]) {
            used[r] = true;
            used[r + 1] = true;
        }
    }
    let shared = share::plan(prog, f, &used);
    if shared.is_none() && used.iter().all(|&u| u) {
        return;
    }
    let (map, ty) = shared.unwrap_or_else(|| {
        let mut map = vec![0u32; n];
        let mut ty = Vec::with_capacity(n);
        for r in 0..n {
            if used[r] {
                map[r] = ty.len() as u32;
                ty.push(f.reg_ty[r]);
            }
        }
        (map, ty)
    });
    let rn = |r: &mut Reg| r.0 = map.get(r.0 as usize).copied().unwrap_or(0);
    for i in &mut f.code {
        i.visit_mut(&mut |r, _, _| rn(r));
        // Operands an instruction does not use are not visited (and not renamed), but the
        // interpreter reads their cells: point them at a register that exists.
        match i {
            Inst::Bool {
                op: BoolOp::Not,
                a,
                b,
                ..
            } => *b = *a,
            Inst::LogicOp {
                op: LOp::Not | LOp::ToX01 | LOp::ToX01Z | LOp::ToUx01,
                a,
                b,
                ..
            } => *b = *a,
            Inst::Check {
                kind, v, lo, hi, ..
            } => {
                if *kind == CheckKind::Null {
                    *lo = *v;
                }
                if !matches!(kind, CheckKind::Range | CheckKind::Index) {
                    *hi = *v;
                }
            }
            _ => {}
        }
    }
    let lists: Vec<(usize, usize)> = f
        .code
        .iter()
        .filter_map(|i| i.list())
        .map(|l| (l.start as usize, (l.start + l.len) as usize))
        .collect();
    let mut done = vec![false; f.operands.len()];
    for (a, b) in lists {
        for k in a..b.min(done.len()) {
            if !done[k] {
                done[k] = true;
                rn(&mut f.operands[k]);
            }
        }
    }
    f.reg_ty = ty;
}

/// The registers `i` defines: the common instructions directly, the rest through
/// [`Inst::visit`] (the analysis runs over every instruction, so this one is worth a fast
/// path; debug builds check the two agree).
fn for_defs(i: &Inst, f: impl FnMut(Reg)) {
    #[cfg(debug_assertions)]
    {
        let (mut fast, mut slow) = (Vec::new(), Vec::new());
        for_defs_fast(i, |r| fast.push(r));
        i.visit(&mut |r, role, _| {
            if role == Role::Def {
                slow.push(r);
            }
        });
        assert_eq!(fast, slow, "definitions of {i:?}");
    }
    for_defs_fast(i, f);
}

fn for_defs_fast(i: &Inst, mut f: impl FnMut(Reg)) {
    match *i {
        Inst::Const { dst, .. }
        | Inst::ConstF { dst, .. }
        | Inst::Move { dst, .. }
        | Inst::Select { dst, .. }
        | Inst::AddrOf { dst, .. }
        | Inst::Load { dst, .. }
        | Inst::Index { dst, .. }
        | Inst::Bin { dst, .. }
        | Inst::BinRange { dst, .. }
        | Inst::Cmp { dst, .. }
        | Inst::Bool { dst, .. }
        | Inst::Pos { dst, .. }
        | Inst::SlicePos { dst, .. }
        | Inst::DopeLen { dst, .. }
        | Inst::Field { dst, .. }
        | Inst::Un { dst, .. } => f(dst),
        Inst::Store { .. }
        | Inst::Stmt
        | Inst::Br { .. }
        | Inst::CondBr { .. }
        | Inst::Ret { .. }
        | Inst::Check { .. }
        | Inst::Cover { .. }
        | Inst::Copy { .. } => {}
        _ => i.visit(&mut |r, role, _| {
            if role == Role::Def {
                f(r);
            }
        }),
    }
}

/// The smallest object `defaults_first` builds in place.
const DEFAULTS_MIN: i64 = 4096;

/// Marker for a deleted instruction until the code is compacted: an `Unreachable` with this
/// message never occurs otherwise.
const DEAD_MSG: CheckMsgId = CheckMsgId(u32::MAX);

fn dead() -> Inst {
    Inst::Unreachable { what: DEAD_MSG }
}

fn is_dead(i: &Inst) -> bool {
    matches!(i, Inst::Unreachable { what } if *what == DEAD_MSG)
}

/// A normalised operand: a known constant, a known address, or a register.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Opn {
    K(i64),
    A(AddrSpace, u32),
    R(Reg),
    /// A register defined more than once, at the version of its last definition on the path:
    /// only in keys (value numbering, [`Opt::kopn`]).
    V(Reg, u32),
}

/// Which memory an address points into, where known.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cls {
    Space(AddrSpace),
    Heap,
    Temp,
    /// The block one `Alloc` made (the register), disjoint from every other.
    Block(u32),
}

/// Is the address in memory nothing writes while the function runs: the constant pool and
/// the instance data (frozen before the simulation, `docs/design/ir.md` 2)? Its facts survive
/// calls, stores and statement markers.
fn read_only(cls: &[Option<Cls>], a: Opn) -> bool {
    let c = match a {
        Opn::A(s, _) => Some(Cls::Space(s)),
        Opn::R(r) => cls.get(r.0 as usize).copied().flatten(),
        Opn::K(_) | Opn::V(..) => None,
    };
    matches!(c, Some(Cls::Space(AddrSpace::Const(_) | AddrSpace::Inst)))
}

/// Is the byte range `[a, a + n)` clear of every range in `rs`?
fn clear_of(rs: &[(i64, i64)], a: i64, n: i64) -> bool {
    rs.iter()
        .all(|&(s, z)| a + n <= s || s.saturating_add(z) <= a)
}

/// An available expression. A check or position repeated with another message is the same
/// key: the earlier one failed first, with its message, or this one passes. Memory-dependent ones (loads, descriptor reads) are dropped when
/// something may write the memory they read.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Key {
    Bin(IntOp, Opn, Opn),
    BinRange(IntOp, Opn, Opn, Opn, Opn),
    Un(IntUnOp, Opn),
    Cmp(CmpOp, Opn, Opn),
    Bool(BoolOp, Opn, Opn),
    Index(Opn, Opn, u32),
    Check(CheckKind, Opn, Opn, Opn),
    Load(Opn, Width),
    /// The 8-byte word at the byte offset in the block of the operand that a `MakeDope` wrote:
    /// the register it was made from (a `Load` of the descriptor gives that back).
    Part(Opn, u32),
    Dope(Opn, u8),
    /// The length of a dimension of a parameter's descriptor: a callee never changes the bounds
    /// of a descriptor it was given, so no store, call or marker drops it.
    ParamDope(Opn, u8),
    /// What the optimiser knows of the value in a register since its last definition: kind 0, the
    /// result block of a native call whose descriptor has this length; 1, the address of that
    /// descriptor's word in the block; 2, the descriptor, its first dimension having this length;
    /// 6 and 7, the address of its right and direction word; 8, 9 and 10, the left, right and
    /// direction word read from it. The native writes the block once, so no store or call drops
    /// it.
    Nat(u8, Opn),
    Pos(Opn, u8, Opn),
    /// `SlicePos` of the bounds `left`, `right` and direction in a descriptor.
    Slice(Opn, [Opn; 3]),
    Field(Opn, Opn, u32),
    /// The temporary layout of a type under a descriptor: gone at the next `Stmt`.
    Layout(TyId, Opn),
}

impl Key {
    /// The address operand and the byte range read from it, for memory-dependent keys.
    fn region(&self) -> Option<(Opn, i64, i64)> {
        match *self {
            Key::Load(a, w) => Some((a, 0, i64::from(w.bytes()))),
            Key::Part(a, o) => Some((a, i64::from(o), 8)),
            Key::Dope(a, d) | Key::Pos(a, d, ..) => Some((a, i64::from(d) * 24, 24)),
            Key::Slice(a, _) => Some((a, 0, 24)),
            Key::Field(_, l, n) => Some((l, 16 + 8 * i64::from(n), 8)),
            Key::Layout(_, d) => Some((d, 0, 24 * 8)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum Val {
    R(Reg),
    /// A register defined more than once, valid while it still has this version.
    V(Reg, u32),
    K(i64),
    Unit,
}

/// Most memory facts kept at once.
const MEM_CAP: usize = 64;

/// The available expressions on the current path, with an undo log so that siblings of an
/// extended basic block do not see each other's facts.
#[derive(Default)]
struct Table {
    pure: FxMap<Key, Val>,
    mem: FxMap<Key, Val>,
    log: Vec<(Key, Option<Val>)>,
}

impl Table {
    fn put(&mut self, k: Key, v: Val) {
        let old = if k.region().is_some() {
            // ponytail: a full table stops learning memory facts; every store and statement
            // scans it, so it stays small. An index by address if this loses much.
            if self.mem.len() >= MEM_CAP {
                return;
            }
            self.mem.insert(k.clone(), v)
        } else {
            self.pure.insert(k.clone(), v)
        };
        self.log.push((k, old));
    }

    fn get(&self, k: &Key) -> Option<Val> {
        if k.region().is_some() {
            self.mem.get(k).copied()
        } else {
            self.pure.get(k).copied()
        }
    }

    /// Forget the memory-dependent entries `drop` selects.
    fn forget(&mut self, mut drop: impl FnMut(&Key) -> bool) {
        let gone: Vec<Key> = self.mem.keys().filter(|k| drop(k)).cloned().collect();
        for k in gone {
            if let Some(v) = self.mem.remove(&k) {
                self.log.push((k, Some(v)));
            }
        }
    }

    fn clear(&mut self) {
        self.forget(|_| true);
        let gone: Vec<Key> = self.pure.keys().cloned().collect();
        for k in gone {
            if let Some(v) = self.pure.remove(&k) {
                self.log.push((k, Some(v)));
            }
        }
    }

    fn rollback(&mut self, mark: usize) {
        while self.log.len() > mark {
            let Some((k, old)) = self.log.pop() else {
                break;
            };
            let m = if k.region().is_some() {
                &mut self.mem
            } else {
                &mut self.pure
            };
            match old {
                Some(v) => {
                    m.insert(k, v);
                }
                None => {
                    m.remove(&k);
                }
            }
        }
    }
}

/// What a simplified arithmetic instruction becomes.
enum Simp {
    Fold(i64),
    Same(Reg),
}

/// Where the depth-first walk of the extended basic blocks goes next.
enum Step {
    Enter(usize),
    Leave {
        block: usize,
        log: usize,
        susp: (u32, u32),
        vlog: usize,
    },
}

struct Opt<'a> {
    prog: &'a Program,
    f: &'a crate::Func,
    /// A copy specialised to one instance (`copies.rs`): the frozen instance data it is for and
    /// the byte ranges of it that are not to be folded; reads of the data fold like reads of
    /// the constant pool.
    inst: Option<InstView<'a>>,
    /// What the declarative part of the process left in its frame (`decl.rs`), for a copy.
    wf: Option<decl::Facts>,
    /// The function being optimised, when its nested subprograms are to be found (`decl.rs`).
    fid: Option<FuncId>,
    /// Registers that point into the descriptor of a heap object whose bytes are known: the
    /// bytes and the pointer's offset in them.
    rdesc: FxMap<Reg, (Rc<[u8]>, u32)>,
    code: Vec<Inst>,
    operands: Vec<Reg>,
    blocks: Vec<(u32, u32)>,
    /// Nested subprograms exist: they reach this frame through static links.
    nested: bool,
    /// Frame ranges a debugger may write: scalar locals and parameters.
    named: Vec<(i64, i64)>,
    /// Frame ranges whose address is used other than directly by a load or store (or the
    /// whole frame when nested subprograms can reach it); computed when first needed.
    esc: Option<Vec<(i64, i64)>>,
    /// Definitions per register (saturating).
    defs: Vec<u8>,
    /// Registers defined once, by a `Const`.
    kc: Vec<Option<i64>>,
    /// Registers defined once, by an `AddrOf`.
    ka: Vec<Option<(AddrSpace, u32)>>,
    /// Where single-definition address registers point.
    cls: Vec<Option<Cls>>,
    /// Registers replaced by another (both defined once): `rep[r] == r` if not.
    rep: Vec<u32>,
    nrep: usize,
    /// The function has no suspension point.
    nosusp: bool,
    /// The block being walked.
    cur: usize,
    tbl: Table,
    /// Where each register is (last) defined: block and instruction.
    def_blk: Vec<u32>,
    def_pc: Vec<u32>,
    /// Depth of each block on the current path of the walk (`u32::MAX`: not on it) and the
    /// depth of the walk.
    path_depth: Vec<u32>,
    depth: u32,
    /// Block depth and instruction of the last suspension on the path: what was defined
    /// before it is not usable.
    susp_depth: u32,
    susp_pc: u32,
    /// Registers added by the pass (after the function's own) and instructions to insert
    /// before an instruction: `(pc, instruction)`.
    new_regs: Vec<RegTy>,
    inserts: Vec<(usize, Inst)>,
    /// Result registers of `LayoutOf` of a type whose layout is the same under every
    /// descriptor: the type.
    slay: FxMap<Reg, TyId>,
    /// Result registers of `LayoutOf` whose words are known (a descriptor of known lengths):
    /// the words, and the registers `Index`ing into them: (layout register, byte offset).
    lwords: FxMap<Reg, Vec<u64>>,
    /// Result registers of `LayoutOf` of a one-dimensional array whose stride word (offset 16)
    /// is the same under every descriptor: that stride.
    lstride: FxMap<Reg, u64>,
    lat: FxMap<Reg, (Reg, i64)>,
    /// `Index` results at a constant offset from a once-defined register: (that register, byte
    /// offset), so a `Load` finds the `MakeDope` word it reads (`Key::Part`).
    iat: FxMap<Reg, (Reg, i64)>,
    /// Once-defined results of an addition or subtraction of a constant to a once-defined
    /// register: (that register, the constant added), so the length of `MakeDope [x, x + 3]`
    /// is known.
    aff: FxMap<Reg, (Reg, i64)>,
    /// Linear forms of the values of registers, by register and version (`kopn`): the value is
    /// `mul * base + add` for a register or version the key names (`bounds_len`). Versions are
    /// numbers nothing else has, so an entry stays true.
    lin: FxMap<Opn, (Opn, i64, i64)>,
    /// The form of the result of the instruction being walked, and a fact about its result
    /// (`Key::Nat` kind and length): recorded by `define`, once the result has its version.
    plin: Option<(Reg, Opn, i64, i64)>,
    pfact: Option<(Reg, u8, i64)>,
    /// Native results met: the facts of `Key::Nat` carry the result's number in their upper half,
    /// so words of two descriptors of one length are not taken for one descriptor's.
    nid: i64,
    /// Field offsets of the static record layouts met so far (shared by the functions of one
    /// batch).
    layouts: &'a mut FxMap<TyId, Option<Vec<u64>>>,
    /// Readers per register, as left by the dead-code sweep.
    uses: Vec<u32>,
    /// The blocks some path from the entry reaches; definitions in the others do not count.
    reach: Vec<bool>,
    /// Value numbering of registers defined more than once: the version of each (a number
    /// nothing else has, given at every definition on the path of the walk and undone when
    /// the walk leaves the block: `vlog`), the next number, and the constant or address such a
    /// register holds since its last definition: (version, value, depth and instruction of the
    /// definition).
    vsn: Vec<u32>,
    vnext: u32,
    vlog: Vec<(u32, u32)>,
    mval: FxMap<u32, (u32, Opn, u32, u32)>,
    /// The key of the instruction being walked, to be recorded once its result has a version.
    pend: Option<(Key, Reg)>,
    changed: bool,
    /// A branch on a constant was replaced by a jump: the join after it has one predecessor
    /// less, which the next walk can use.
    pruned: bool,
    /// A `Pos` was split into a check and a subtraction (`pos_to_check`): the checks are only in
    /// the finished code, where `tidy` walks it again.
    split: bool,
}

impl<'a> Opt<'a> {
    fn new(
        prog: &'a Program,
        f: &'a crate::Func,
        code: Vec<Inst>,
        operands: Vec<Reg>,
        nested: bool,
        layouts: &'a mut FxMap<TyId, Option<Vec<u64>>>,
    ) -> Self {
        let n = f.reg_ty.len();
        Opt {
            prog,
            f,
            inst: None,
            wf: None,
            fid: None,
            rdesc: FxMap::default(),
            code,
            operands,
            blocks: f.blocks.values().iter().map(|b| (b.start, b.end)).collect(),
            nested,
            // A debugger writes a scalar local in its slot; a composite through the pointer
            // its slot holds (`write_slot`), so only scalar slots are variables here.
            named: f
                .locals
                .iter()
                .filter(|l| {
                    prog.types
                        .get(l.ty)
                        .is_some_and(|t| t.scalar_size().is_some())
                })
                .map(|l| (i64::from(l.slot), i64::from(l.size)))
                .collect(),
            esc: None,
            defs: vec![0; n],
            kc: vec![None; n],
            ka: vec![None; n],
            cls: vec![None; n],
            rep: (0..n as u32).collect(),
            nrep: 0,
            nosusp: f.body.is_none(),
            cur: 0,
            tbl: Table::default(),
            def_blk: vec![0; n],
            def_pc: vec![0; n],
            path_depth: vec![u32::MAX; f.blocks.len()],
            depth: 0,
            susp_depth: 0,
            susp_pc: 0,
            new_regs: Vec::new(),
            inserts: Vec::new(),
            slay: FxMap::default(),
            lstride: FxMap::default(),
            lwords: FxMap::default(),
            lat: FxMap::default(),
            iat: FxMap::default(),
            aff: FxMap::default(),
            lin: FxMap::default(),
            plin: None,
            pfact: None,
            nid: 0,
            layouts,
            uses: Vec::new(),
            reach: Vec::new(),
            vsn: vec![0; n],
            vnext: 0,
            vlog: Vec::new(),
            mval: FxMap::default(),
            pend: None,
            changed: false,
            pruned: false,
            split: false,
        }
    }

    fn is_susp(&self, i: &Inst) -> bool {
        match *i {
            Inst::Suspend { .. } => true,
            Inst::Call { func, .. } => self
                .prog
                .funcs
                .get(func)
                .is_some_and(|f| f.flags.contains(FuncFlags::MAY_SUSPEND)),
            _ => false,
        }
    }

    /// A register defined exactly once, holding a plain value.
    fn single(&self, r: Reg) -> bool {
        self.defs.get(r.0 as usize) == Some(&1)
            && matches!(self.rty(r), Some(RegTy::I64 | RegTy::Ptr))
    }

    /// A register defined more than once, holding a plain value: the walk numbers its values.
    fn multi(&self, r: Reg) -> bool {
        self.defs.get(r.0 as usize).is_some_and(|&d| d > 1)
            && matches!(self.rty(r), Some(RegTy::I64 | RegTy::Ptr))
    }

    /// Is something defined at (`depth`, `pc`) of the current path still usable: not before the
    /// last suspension on it (a suspension forgets the registers)?
    fn since_susp(&self, depth: u32, pc: u32) -> bool {
        self.nosusp || depth > self.susp_depth || (depth == self.susp_depth && pc > self.susp_pc)
    }

    /// Give the multiply defined register `r` a version no earlier value has.
    fn bump(&mut self, r: Reg) {
        self.vnext += 1;
        let old = std::mem::replace(&mut self.vsn[r.0 as usize], self.vnext);
        self.vlog.push((r.0, old));
    }

    /// After the instruction at `pc` was simplified and its effect recorded: new versions for
    /// the registers it defines, what a constant or address tells about its register, and the
    /// key of its result.
    fn define(&mut self, pc: usize) {
        let i = self.code[pc];
        let known = match i {
            Inst::Const { dst, v } if self.multi(dst) => Some((dst, Opn::K(v))),
            Inst::AddrOf { dst, space, off } if self.multi(dst) => Some((dst, Opn::A(space, off))),
            Inst::Move { dst, src } if self.multi(dst) => match self.opn(src) {
                o @ (Opn::K(_) | Opn::A(..)) => Some((dst, o)),
                _ => None,
            },
            _ => None,
        };
        let mut defs: [Option<Reg>; 2] = [None; 2];
        let mut n = 0;
        for_defs(&i, |r| {
            if n < 2 {
                defs[n] = Some(r);
            }
            n += 1;
        });
        // Instructions with more results than two are left to the plain rule: no versions kept.
        let wide = n > 2;
        for r in defs.into_iter().flatten() {
            if self.multi(r) {
                self.bump(r);
            }
        }
        let key = |o: &Self, d: Reg| {
            if o.multi(d) {
                Some(Opn::V(d, o.vsn[d.0 as usize]))
            } else {
                o.single(d).then_some(Opn::R(d))
            }
        };
        if let Some((d, base, mul, add)) = self.plin.take()
            && matches!(self.code[pc], Inst::Bin { dst, .. } | Inst::BinRange { dst, .. } if dst == d)
            && let Some(k) = key(self, d)
            && k != base
        {
            self.lin.insert(k, (base, mul, add));
        }
        if let Some((d, kind, n)) = self.pfact.take()
            && defs.contains(&Some(d))
            && let Some(k) = key(self, d)
        {
            self.tbl.put(Key::Nat(kind, k), Val::K(n));
        }
        if wide {
            self.pend = None;
            return;
        }
        // A constant is an integer and an address a pointer: a register of the other type holds
        // something this pass does not know the value of.
        if let Some((dst, o)) = known
            && matches!(
                (o, self.rty(dst)),
                (Opn::K(_), Some(RegTy::I64)) | (Opn::A(..), Some(RegTy::Ptr))
            )
        {
            self.mval
                .insert(dst.0, (self.vsn[dst.0 as usize], o, self.depth, pc as u32));
        }
        if let Some((key, d)) = self.pend.take()
            && self.multi(d)
        {
            let v = Val::V(d, self.vsn[d.0 as usize]);
            self.tbl.put(key, v);
        }
    }

    fn rty(&self, r: Reg) -> Option<RegTy> {
        let n0 = self.f.reg_ty.len();
        let i = r.0 as usize;
        if i < n0 {
            self.f.reg_ty.get(i).copied()
        } else {
            self.new_regs.get(i - n0).copied()
        }
    }

    /// A new register holding the constant `v`, defined by an instruction inserted before
    /// `pc` (in block `b`).
    fn const_reg(&mut self, b: usize, pc: usize, v: i64) -> Reg {
        let r = Reg(self.defs.len() as u32);
        self.new_regs.push(RegTy::I64);
        self.defs.push(1);
        self.kc.push(Some(v));
        self.ka.push(None);
        self.cls.push(None);
        self.vsn.push(0);
        self.rep.push(r.0);
        self.def_blk.push(b as u32);
        self.def_pc.push(pc as u32);
        self.inserts.push((pc, Inst::Const { dst: r, v }));
        r
    }

    fn resolve(&self, mut r: Reg) -> Reg {
        while let Some(&x) = self.rep.get(r.0 as usize) {
            if x == r.0 {
                break;
            }
            r = Reg(x);
        }
        r
    }

    fn opn(&self, r: Reg) -> Opn {
        let r = self.resolve(r);
        let i = r.0 as usize;
        if let Some(Some(v)) = self.kc.get(i) {
            Opn::K(*v)
        } else if let Some(Some((s, o))) = self.ka.get(i) {
            Opn::A(*s, *o)
        } else if let Some(&(ver, o, d, pc)) = self.mval.get(&r.0)
            && self.vsn[i] == ver
            && self.since_susp(d, pc)
        {
            o
        } else {
            Opn::R(r)
        }
    }

    fn cls_of(&self, o: Opn) -> Option<Cls> {
        match o {
            Opn::A(s, _) => Some(Cls::Space(s)),
            Opn::R(r) => self.cls.get(r.0 as usize).copied().flatten(),
            Opn::K(_) | Opn::V(..) => None,
        }
    }

    fn set_cls(&mut self, r: Reg, c: Cls) {
        if let Some(x) = self.cls.get_mut(r.0 as usize) {
            *x = Some(c);
        }
    }

    /// Definition counts, constants, addresses, and the offsets of known addresses folded
    /// (`Index` of a known address by a known position).
    fn analyse(&mut self) {
        for b in 0..self.blocks.len() {
            for pc in self.range(b) {
                let i = &self.code[pc];
                if matches!(i, Inst::Suspend { .. }) {
                    self.nosusp = false;
                }
                if let Inst::Call { func, .. } = *i
                    && self
                        .prog
                        .funcs
                        .get(func)
                        .is_some_and(|f| f.flags.contains(FuncFlags::MAY_SUSPEND))
                {
                    self.nosusp = false;
                }
            }
        }
        self.reach = self.reachable();
        for b in 0..self.blocks.len() {
            if !self.reach[b] {
                continue;
            }
            for pc in self.range(b) {
                let i = &self.code[pc];
                let (defs, blk, at) = (&mut self.defs, &mut self.def_blk, &mut self.def_pc);
                for_defs(i, |r| {
                    if let Some(d) = defs.get_mut(r.0 as usize) {
                        *d = d.saturating_add(1);
                        blk[r.0 as usize] = b as u32;
                        at[r.0 as usize] = pc as u32;
                    }
                });
            }
        }
        for b in 0..self.blocks.len() {
            if !self.reach[b] {
                continue;
            }
            for pc in self.range(b) {
                match self.code[pc] {
                    Inst::Const { dst, v } if self.single(dst) => {
                        self.kc[dst.0 as usize] = Some(v);
                    }
                    Inst::AddrOf { dst, space, off } if self.single(dst) => {
                        self.ka[dst.0 as usize] = Some((space, off));
                        self.set_cls(dst, Cls::Space(space));
                    }
                    Inst::Deref { dst, .. } if self.single(dst) => self.set_cls(dst, Cls::Heap),
                    Inst::LayoutOf { dst, ty, .. }
                        if self.single(dst) && Ty::desc_size(&self.prog.types, ty) == Some(0) =>
                    {
                        self.set_cls(dst, Cls::Temp);
                        self.slay.insert(dst, ty);
                        if let Some(w) = self.static_words(ty) {
                            self.lwords.insert(dst, w);
                        }
                    }
                    Inst::Alloc { dst, .. } if self.single(dst) => {
                        self.set_cls(dst, Cls::Block(dst.0));
                    }
                    Inst::LayoutOf { dst, ty, .. } if self.single(dst) => {
                        self.set_cls(dst, Cls::Temp);
                        if let Some(s) = self.static_stride(ty) {
                            self.lstride.insert(dst, s);
                        }
                    }
                    Inst::Index { dst, base, pos, .. } if self.single(dst) => {
                        if self.fold_index(pc) {
                            continue;
                        }
                        if let Some(c) = self.cls_of(Opn::R(base)) {
                            self.set_cls(dst, c);
                        }
                        let _ = pos;
                    }
                    Inst::Field { dst, base, .. } if self.single(dst) => {
                        if let Some(c) = self.cls_of(Opn::R(base)) {
                            self.set_cls(dst, c);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// The blocks some path from the entry reaches. Code that suspends resumes right after
    /// the instruction: in the same block, unless that ends the block, and then the next
    /// block is entered by no branch.
    fn reachable(&self) -> Vec<bool> {
        let mut seen = vec![false; self.blocks.len()];
        // (The block a process pauses at is entered by the interpreter, not by a branch.)
        let mut work: Vec<usize> = std::iter::once(0)
            .chain(self.f.body.map(|b| b.0 as usize))
            .collect();
        while let Some(b) = work.pop() {
            if seen.get(b) != Some(&false) {
                continue;
            }
            seen[b] = true;
            let mut k = 0;
            while let Some(s) = self.succ(b, k) {
                work.push(s);
                k += 1;
            }
            if !self.nosusp
                || self.f.body.is_some()
                || self.f.flags.contains(FuncFlags::MAY_SUSPEND)
            {
                let (start, end) = self.blocks[b];
                for pc in start..end {
                    if self.is_susp(&self.code[pc as usize])
                        && pc + 1 == end
                        && let Some(n) = self.blocks.iter().position(|x| x.0 == end && x.1 > end)
                    {
                        work.push(n);
                    }
                }
            }
        }
        seen
    }

    /// `Index` of a known address by a known position becomes an `AddrOf`.
    fn fold_index(&mut self, pc: usize) -> bool {
        let Inst::Index {
            dst,
            base,
            pos,
            stride,
        } = self.code[pc]
        else {
            return false;
        };
        if let (Opn::A(s, o), Opn::K(c)) = (self.opn(base), self.opn(pos))
            && let Some(off) = i64::from(o)
                .checked_add(c.checked_mul(i64::from(stride)).unwrap_or(-1))
                .and_then(|v| u32::try_from(v).ok())
            && (s != AddrSpace::Frame || off <= self.f.frame)
        {
            self.make_addr(pc, dst, s, off);
            return true;
        }
        false
    }

    /// The `k`-th successor of block `b`.
    fn succ(&self, b: usize, k: usize) -> Option<usize> {
        let (s, e) = self.blocks[b];
        if e <= s {
            return None;
        }
        let t = match self.code[e as usize - 1] {
            Inst::Br { to } => (k == 0).then_some(to),
            Inst::CondBr { t, f, .. } => match k {
                0 => Some(t),
                1 => Some(f),
                _ => None,
            },
            Inst::Case { table, .. } => {
                let t = self.prog.cases.get(table)?;
                if k == 0 {
                    Some(t.default)
                } else {
                    t.arms.get(k - 1).map(|a| a.2)
                }
            }
            _ => None,
        }?;
        ((t.0 as usize) < self.blocks.len()).then_some(t.0 as usize)
    }

    fn run(&mut self) {
        // Without markers a process variable stored in one statement is known in the next
        // (a marker forgets memory facts for a debugger).
        if self.f.kind == FuncKind::Process {
            self.drop_statements();
        }
        self.analyse();
        if self.inst.is_some() && self.f.kind == FuncKind::Process && self.f.body.is_some() {
            self.wf = self.decl_facts();
        }
        if self.frame_objects() {
            self.reanalyse();
        }
        let promoted = self.promote();
        if let Some(inited) = &promoted {
            self.reanalyse();
            self.hide_inits(inited);
        }
        self.pass();
        if let Some(inited) = &promoted {
            // What the first pass folded makes more of the function constant and drops
            // branches: apply its renames and run over the result again.
            self.dce();
            self.reanalyse();
            self.hide_inits(inited);
            self.pass();
        }
        // Folded branches leave joins with fewer predecessors: facts now reach past them.
        let mut again = 0;
        while self.pruned && again < 4 {
            self.pruned = false;
            again += 1;
            self.dce();
            self.reanalyse();
            if let Some(inited) = &promoted {
                self.hide_inits(inited);
            }
            self.pass();
        }
        // The loops of subprograms are where the passes below pay: they cost optimiser time at
        // the first call of every subprogram, most of which run a few times (decisions.md
        // "Interval analysis, jump threading and dead register definitions").
        let process = self.f.kind == FuncKind::Process;
        let loops = self.code.len() >= range::MIN_CODE
            && self.nosusp
            && self.f.body.is_none()
            && !self.f.flags.contains(FuncFlags::MAY_SUSPEND)
            && self.has_loop(false);
        // A process loops forever: only a cycle that does not pass a suspension is work done
        // within one activation, which is what the analysis of a process can say something about.
        let ploops = process && self.code.len() >= range::MIN_CODE && self.has_loop(true);
        if loops || ploops {
            // The analysis wants the renames applied and the tables rebuilt.
            self.dce();
            self.reanalyse();
            if let Some(inited) = &promoted {
                self.hide_inits(inited);
            }
            if self.ranges() {
                // Decided tests and dropped checks make more constants and fewer branches.
                self.dce();
                self.reanalyse();
                if let Some(inited) = &promoted {
                    self.hide_inits(inited);
                }
                self.pass();
            }
        }
        self.kill_unreachable();
        if loops {
            self.dead_defs();
        }
        if self.code.iter().any(|i| matches!(i, Inst::NewFrame { .. })) {
            // The copy a local record variable is made of is read straight from its source
            // when nothing can change the source meanwhile.
            self.dce();
            self.reanalyse();
            if let Some(inited) = &promoted {
                self.hide_inits(inited);
            }
            self.forward_frame_copies();
        }
        self.drop_clean_roots();
        // (Temporaries nothing reads are gone before the markers that free them are judged.)
        self.dce();
        self.drop_statements();
        self.merge_consts();
        self.dce();
        // (After the constants are merged: the instructions it inserts name the final registers.)
        if self.has_big_copy() {
            self.reanalyse();
            self.defaults_first();
            self.dce();
        }
    }

    /// Does the code make an object of a constant size of 4 KiB or more from a copy (`New`,
    /// `NewFill`, `NewFrame`)? A scan of the constants, no analysis: the check that decides
    /// whether `defaults_first` is worth its tables.
    fn has_big_copy(&self) -> bool {
        let mut sizes: FxMap<u32, ()> = FxMap::default();
        for i in &self.code {
            match *i {
                Inst::New { size, .. }
                | Inst::NewFill { size, .. }
                | Inst::NewFrame { size, .. } => {
                    sizes.insert(size.0, ());
                }
                _ => {}
            }
        }
        !sizes.is_empty()
            && self.code.iter().any(|i| {
                matches!(*i, Inst::Const { dst, v } if v >= DEFAULTS_MIN && sizes.contains_key(&dst.0))
            })
    }

    /// A fat pair in a block this function just allocated, whose two words (stored just before
    /// its `TempRoot`) are addresses of the frame, the constant pool, instance or temporary
    /// memory, points into no heap: the record would be empty (`Machine::fat_root`), so the
    /// instruction goes. Strings built for a native call or a report were one in four
    /// instructions of some testbenches; compiled code stepped each `TempRoot` and then every
    /// `Stmt` after it (the markers are stepped while roots are recorded).
    fn drop_clean_roots(&mut self) {
        // Instructions between the stores and the root that change no memory.
        let pure = |i: &Inst| {
            matches!(
                i,
                Inst::Const { .. }
                    | Inst::ConstF { .. }
                    | Inst::Move { .. }
                    | Inst::AddrOf { .. }
                    | Inst::Index { .. }
                    | Inst::Alloc { .. }
                    | Inst::Load { .. }
                    | Inst::Bin { .. }
                    | Inst::Un { .. }
                    | Inst::Cmp { .. }
                    | Inst::Select { .. }
                    | Inst::Bool { .. }
            )
        };
        for b in 0..self.blocks.len() {
            let rg = self.range(b);
            for pc in rg.clone() {
                let Inst::TempRoot {
                    addr: fat,
                    ty: None,
                    ..
                } = self.code[pc]
                else {
                    continue;
                };
                let fat_at = self.opn(fat);
                if !self.single(fat) || !matches!(self.cls_of(fat_at), Some(Cls::Block(_))) {
                    continue;
                }
                // The latest stores of words 0 and 8 of the pair, read forwards from the
                // window's start; anything that may write memory forgets them.
                let (mut w0, mut w8) = (None, None);
                let mut second: Vec<Reg> = Vec::new();
                for k in pc.saturating_sub(16).max(rg.start)..pc {
                    match self.code[k] {
                        Inst::Index {
                            dst,
                            base,
                            pos,
                            stride: 1,
                        } if self.single(dst)
                            && self.opn(base) == fat_at
                            && self.opn(pos) == Opn::K(8) =>
                        {
                            second.push(self.resolve(dst));
                        }
                        Inst::Store {
                            addr,
                            src,
                            w: Width::B8,
                        } => {
                            if self.opn(addr) == fat_at {
                                w0 = Some(src);
                            } else if second.contains(&self.resolve(addr)) {
                                w8 = Some(src);
                            } else if !matches!(self.cls_of(self.opn(addr)), Some(Cls::Block(x))
                                if Some(Cls::Block(x)) != self.cls_of(fat_at))
                            {
                                (w0, w8) = (None, None);
                            }
                        }
                        ref i if pure(i) => {}
                        _ => (w0, w8) = (None, None),
                    }
                }
                let off_heap = |s: &Self, r: Reg| match s.opn(r) {
                    Opn::A(..) => true,
                    Opn::K(v) => !matches!((v as u64) >> 61, 4 | 7),
                    o @ Opn::R(_) => {
                        matches!(s.cls_of(o), Some(Cls::Space(_) | Cls::Temp | Cls::Block(_)))
                    }
                    Opn::V(..) => false,
                };
                if let (Some(a), Some(c)) = (w0, w8)
                    && off_heap(self, a)
                    && off_heap(self, c)
                {
                    self.code[pc] = dead();
                    self.changed = true;
                }
            }
        }
    }

    /// A `NewFrame` copy of constant size that only constant-offset loads read (the local
    /// record variable `variable s : rec_t := st.items(i)`, never written) is dropped: the loads
    /// read the source, which nothing writes between the copy and the load (every instruction on
    /// the way is a read, a computation or a branch: no store, call, statement marker or
    /// allocation, which may change or free the source).
    fn forward_frame_copies(&mut self) {
        let nb = self.blocks.len();
        // Constants the optimiser made and has not inserted yet.
        let pending: FxMap<u32, i64> = self
            .inserts
            .iter()
            .filter_map(|(_, i)| match *i {
                Inst::Const { dst, v } => Some((dst.0, v)),
                _ => None,
            })
            .collect();
        let konst = |s: &Self, r: Reg| match s.opn(r) {
            Opn::K(v) => Some(v),
            Opn::R(r) => pending.get(&r.0).copied(),
            Opn::A(..) | Opn::V(..) => None,
        };
        let mut block_of = vec![0usize; self.code.len()];
        for b in 0..nb {
            for pc in self.range(b) {
                block_of[pc] = b;
            }
        }
        for pc0 in 0..self.code.len() {
            let Inst::NewFrame {
                dst: q,
                dsize: 0,
                data,
                size,
                fill: 0,
                ..
            } = self.code[pc0]
            else {
                continue;
            };
            let Some(total) = konst(self, size) else {
                continue;
            };
            let b0 = block_of[pc0];
            if self.defs.get(q.0 as usize) != Some(&1)
                || self.defs.get(data.0 as usize) != Some(&1)
                || !self.reach[b0]
                || self.in_cycle(b0)
                || total < 0
            {
                continue;
            }
            // The registers that hold an address inside the copy: q and what is indexed from
            // it by constants (offset from q), each defined once.
            let mut at: FxMap<u32, i64> = FxMap::default();
            at.insert(q.0, 0);
            let mut grew = true;
            while grew {
                grew = false;
                for pc in 0..self.code.len() {
                    if let Inst::Index {
                        dst,
                        base,
                        pos,
                        stride,
                    } = self.code[pc]
                        && !at.contains_key(&dst.0)
                        && let Some(&o) = at.get(&base.0)
                        && let Some(c) = konst(self, pos)
                        && let Some(d) = c
                            .checked_mul(i64::from(stride))
                            .and_then(|d| o.checked_add(d))
                        && self.defs.get(dst.0 as usize) == Some(&1)
                    {
                        at.insert(dst.0, d);
                        grew = true;
                    }
                }
            }
            // Every reader is an `Index` that is one of those or a load inside the copy.
            let mut loads: Vec<usize> = Vec::new();
            let mut ok = true;
            for pc in 0..self.code.len() {
                let i = self.code[pc];
                if is_dead(&i) || pc == pc0 {
                    continue;
                }
                let mut reads = false;
                i.visit(&mut |r, role, _| {
                    reads |= role == Role::Use && at.contains_key(&r.0);
                });
                if let Some(l) = i.list() {
                    reads |= self
                        .operands
                        .get(l.start as usize..(l.start + l.len) as usize)
                        .unwrap_or(&[])
                        .iter()
                        .any(|r| at.contains_key(&r.0));
                }
                if !reads {
                    continue;
                }
                match i {
                    Inst::Index { dst, .. } if at.contains_key(&dst.0) => {}
                    Inst::Load { addr, w, .. }
                        if w != Width::Sig
                            && at
                                .get(&addr.0)
                                .is_some_and(|&o| o >= 0 && o + i64::from(w.bytes()) <= total) =>
                    {
                        loads.push(pc);
                    }
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok || loads.is_empty() {
                continue;
            }
            // Dirty: some path from the copy to the start of the block passes an instruction
            // that may change or free the source.
            // A statement marker frees temporaries, not heap objects or constants.
            let stmt_ok = matches!(
                self.cls_of(self.opn(data)),
                Some(Cls::Heap | Cls::Space(AddrSpace::Const(_)))
            );
            let harmless = |i: &Inst| {
                (stmt_ok && matches!(i, Inst::Stmt))
                    || matches!(
                        i,
                        Inst::Const { .. }
                            | Inst::ConstF { .. }
                            | Inst::Move { .. }
                            | Inst::Select { .. }
                            | Inst::AddrOf { .. }
                            | Inst::Load { .. }
                            | Inst::Index { .. }
                            | Inst::Field { .. }
                            | Inst::LayoutOf { .. }
                            | Inst::Bin { .. }
                            | Inst::BinRange { .. }
                            | Inst::Un { .. }
                            | Inst::Cmp { .. }
                            | Inst::Bool { .. }
                            | Inst::Check { .. }
                            | Inst::Pos { .. }
                            | Inst::SlicePos { .. }
                            | Inst::DopeLen { .. }
                            | Inst::Deref { .. }
                            | Inst::Br { .. }
                            | Inst::CondBr { .. }
                            | Inst::Case { .. }
                            | Inst::Ret { .. }
                            | Inst::Unreachable { .. }
                    )
            };
            let bad = |code: &[Inst]| code.iter().any(|i| !harmless(i));
            let mut dirty_in = vec![false; nb];
            let mut seen = vec![false; nb];
            let mut work: Vec<usize> = Vec::new();
            let d0 = bad(&self.code[pc0 + 1..self.blocks[b0].1 as usize]);
            let mut k = 0;
            while let Some(s) = self.succ(b0, k) {
                k += 1;
                if !seen[s] || (d0 && !dirty_in[s]) {
                    seen[s] = true;
                    dirty_in[s] |= d0;
                    work.push(s);
                }
            }
            while let Some(b) = work.pop() {
                let d = dirty_in[b] || bad(&self.code[self.range(b)]);
                let mut k = 0;
                while let Some(s) = self.succ(b, k) {
                    k += 1;
                    if !seen[s] || (d && !dirty_in[s]) {
                        seen[s] = true;
                        dirty_in[s] |= d;
                        work.push(s);
                    }
                }
            }
            for &pc in &loads {
                let b = block_of[pc];
                let dirty = if b == b0 {
                    bad(&self.code[pc0 + 1..pc])
                } else {
                    !seen[b] || dirty_in[b] || bad(&self.code[self.blocks[b].0 as usize..pc])
                };
                if dirty {
                    ok = false;
                    break;
                }
            }
            if !ok {
                continue;
            }
            // Read the source instead.
            for pc in 0..self.code.len() {
                match self.code[pc] {
                    Inst::Load { dst, addr, w } if addr == q => {
                        self.code[pc] = Inst::Load { dst, addr: data, w };
                    }
                    Inst::Index {
                        dst,
                        base,
                        pos,
                        stride,
                    } if base == q => {
                        self.code[pc] = Inst::Index {
                            dst,
                            base: data,
                            pos,
                            stride,
                        };
                    }
                    _ => {}
                }
            }
            self.code[pc0] = dead();
            self.changed = true;
        }
    }

    /// A large record variable (or heap node) whose initial value is built in a temporary
    /// (`Alloc`, then stores and fills) that `NewFrame`, `New` or `NewFill` copies into the
    /// object's own memory is built in that memory: the `Alloc` becomes the zero-filled object
    /// and the copy goes (a record of some hundred kilobytes was zeroed twice and copied once
    /// per call, #792). Only when nothing reads the temporary or keeps its address past the
    /// copy.
    fn defaults_first(&mut self) {
        const MIN: i64 = DEFAULTS_MIN;
        let nb = self.blocks.len();
        let mut block_of = vec![0usize; self.code.len()];
        for b in 0..nb {
            for pc in self.range(b) {
                block_of[pc] = b;
            }
        }
        for pcn in 0..self.code.len() {
            // The copy: its object register, the temporary and its size.
            let (q, t, size, heap) = match self.code[pcn] {
                Inst::NewFrame {
                    dst,
                    desc,
                    dsize: 0,
                    data,
                    size,
                    fill: 0,
                } if desc == data => (dst, data, size, None),
                Inst::New {
                    dst,
                    ty,
                    init,
                    size,
                    shared,
                } => (dst, init, size, Some((ty, shared))),
                Inst::NewFill {
                    dst,
                    ty,
                    dsize: 0,
                    data,
                    size,
                    fill: 0,
                    shared,
                    ..
                } => (dst, data, size, Some((ty, shared))),
                _ => continue,
            };
            let Opn::K(total) = self.opn(size) else {
                continue;
            };
            let bn = block_of[pcn];
            if total < MIN
                || self.defs.get(q.0 as usize) != Some(&1)
                || self.defs.get(t.0 as usize) != Some(&1)
                || !self.reach[bn]
                || self.in_cycle(bn)
            {
                continue;
            }
            // The one `Alloc` of the temporary, of the same size, in code that runs once.
            let mut alloc = None;
            for (pc, i) in self.code.iter().enumerate() {
                if let Inst::Alloc { dst, size: sz } = *i
                    && dst == t
                {
                    alloc = (self.opn(sz) == Opn::K(total)).then_some((pc, sz));
                    break;
                }
            }
            // (The size register of the `Alloc` is the one defined where the object is now.)
            let Some((pca, size)) = alloc else { continue };
            let ba = block_of[pca];
            if !self.reach[ba] || self.in_cycle(ba) {
                continue;
            }
            // A heap object is dereferenced where the temporary was: the check message of the
            // `Deref` that follows.
            let mut what = None;
            if heap.is_some() {
                what = self.code.iter().find_map(|i| match *i {
                    Inst::Deref { handle, what, .. } if handle == q => Some(what),
                    _ => None,
                });
                if what.is_none() {
                    continue;
                }
            }
            // Registers that hold an address inside the temporary.
            let mut at: FxMap<u32, ()> = FxMap::default();
            at.insert(t.0, ());
            let mut grew = true;
            while grew {
                grew = false;
                for i in &self.code {
                    if let Inst::Index { dst, base, .. } | Inst::Field { dst, base, .. } = *i
                        && at.contains_key(&base.0)
                        && !at.contains_key(&dst.0)
                        && self.defs.get(dst.0 as usize) == Some(&1)
                    {
                        at.insert(dst.0, ());
                        grew = true;
                    }
                }
            }
            // Blocks that run after the copy.
            let mut after = vec![false; nb];
            let mut work = Vec::new();
            let mut k = 0;
            while let Some(s) = self.succ(bn, k) {
                work.push(s);
                k += 1;
            }
            while let Some(x) = work.pop() {
                if std::mem::replace(&mut after[x], true) {
                    continue;
                }
                let mut k = 0;
                while let Some(s) = self.succ(x, k) {
                    work.push(s);
                    k += 1;
                }
            }
            // Every use of the temporary, or of an address in it, is a read or a write through
            // it, before the copy (or, for a heap object, the `TempRoot` of the temporary).
            let is_at = |r: Reg| at.contains_key(&r.0);
            let mut ok = true;
            let mut roots = Vec::new();
            for (pc, i) in self.code.iter().enumerate() {
                if pc == pcn || is_dead(i) {
                    continue;
                }
                let mut used = false;
                i.visit(&mut |r, role, _| used |= role == Role::Use && is_at(r));
                if let Some(l) = i.list() {
                    used |= self
                        .operands
                        .get(l.start as usize..(l.start + l.len) as usize)
                        .unwrap_or(&[])
                        .iter()
                        .any(|&r| is_at(r));
                }
                if !used {
                    continue;
                }
                let through = match *i {
                    Inst::Index { pos, .. } => !is_at(pos),
                    Inst::Field { layout, .. } => !is_at(layout),
                    Inst::Load { .. } | Inst::LFill { .. } | Inst::LSet { .. } => true,
                    Inst::Store { src, .. } => !is_at(src),
                    Inst::Copy { bytes, .. } => !is_at(bytes),
                    Inst::TempRoot { addr, desc, .. } if heap.is_some() && addr == t => {
                        roots.push(pc);
                        desc.is_none_or(|d| !is_at(d))
                    }
                    _ => false,
                };
                let before = if block_of[pc] == bn {
                    pc < pcn
                } else {
                    !after[block_of[pc]]
                };
                if !through || !before {
                    ok = false;
                    break;
                }
            }
            if !ok {
                continue;
            }
            // The zero byte the fill reads.
            let one = self.const_reg(ba, pca, 1);
            let z = self.fresh(RegTy::Ptr, 1);
            self.inserts.push((pca, Inst::Alloc { dst: z, size: one }));
            if let (Some((ty, shared)), Some(what)) = (heap, what) {
                self.inserts.push((
                    pca,
                    Inst::NewFill {
                        dst: q,
                        ty,
                        desc: z,
                        dsize: 0,
                        data: z,
                        size,
                        fill: 1,
                        shared,
                    },
                ));
                self.code[pca] = Inst::Deref {
                    dst: t,
                    handle: q,
                    what,
                };
                self.code[pcn] = dead();
                for pc in roots {
                    self.code[pc] = dead();
                }
            } else {
                self.code[pca] = Inst::NewFrame {
                    dst: t,
                    desc: z,
                    dsize: 0,
                    data: z,
                    size,
                    fill: 1,
                };
                self.code[pcn] = Inst::Move { dst: q, src: t };
            }
            self.zero_fills_after(ba, pca, (bn, pcn), t, q);
            self.changed = true;
        }
    }

    /// `LFill`s of the code 0 (all planes zero) of a fresh zeroed object `t` that overwrite
    /// nothing written before in the block (constant offsets and lengths only, anything else
    /// ends the scan) write zeros over zeros: dropped. The default of a `std_logic` array is
    /// 'U' (code 0), and a record of a few hundred kilobytes was filled twice with it. `q` is
    /// the object's handle or alias: whoever has it may write the object, which ends the scan.
    fn zero_fills_after(&mut self, b0: usize, pc0: usize, stop: (usize, usize), t: Reg, q: Reg) {
        // Registers whose address inside `t` is known (byte offset), and the byte ranges written.
        let mut at: FxMap<u32, i64> = FxMap::default();
        at.insert(t.0, 0);
        let mut written: Vec<(i64, i64)> = Vec::new();
        // Constants set in this stretch (a register the optimiser reused is not a known constant).
        let mut kval: FxMap<u32, i64> = FxMap::default();
        // Constants the optimiser made and has not inserted yet.
        let pending: FxMap<u32, i64> = self
            .inserts
            .iter()
            .filter_map(|(_, i)| match *i {
                Inst::Const { dst, v } => Some((dst.0, v)),
                _ => None,
            })
            .collect();
        let hit = |w: &[(i64, i64)], lo: i64, hi: i64| w.iter().any(|&(a, b)| lo < b && a < hi);
        // The stretch is a chain of blocks, each the only successor of the one before, up to
        // the copy's own block.
        let mut npred = vec![0u32; self.blocks.len()];
        for x in 0..self.blocks.len() {
            let mut k = 0;
            while let Some(s) = self.succ(x, k) {
                npred[s] += 1;
                k += 1;
            }
        }
        let mut b = b0;
        let mut hops = 0;
        loop {
            let end = if b == stop.0 {
                (self.blocks[b].1 as usize).min(stop.1)
            } else {
                self.blocks[b].1 as usize
            };
            for pc in self.blocks[b].0 as usize..end {
                let i = self.code[pc];
                if is_dead(&i) {
                    continue;
                }
                if pc <= pc0 {
                    // Before the object: only the constants it leaves in registers.
                    i.visit(&mut |r, role, _| {
                        if role == Role::Def {
                            kval.remove(&r.0);
                        }
                    });
                    if let Inst::Const { dst, v } = i {
                        kval.insert(dst.0, v);
                    }
                    continue;
                }
                let (mut touches, mut shared) = (false, false);
                let mut defs: Vec<u32> = Vec::new();
                i.visit(&mut |r, role, _| match role {
                    Role::Use => {
                        touches |= at.contains_key(&r.0);
                        shared |= r == q;
                    }
                    Role::Def => defs.push(r.0),
                });
                if shared {
                    return;
                }
                for &d in &defs {
                    kval.remove(&d);
                }
                if let Inst::Const { dst, v } = i {
                    kval.insert(dst.0, v);
                }
                if !touches {
                    for d in defs {
                        at.remove(&d);
                    }
                    continue;
                }
                let kv = |s: &Self, r: Reg| match kval.get(&r.0) {
                    Some(&v) => Some(v),
                    None => match s.opn(r) {
                        Opn::K(v) => Some(v),
                        Opn::R(r) => pending.get(&r.0).copied(),
                        _ => None,
                    },
                };
                match i {
                    Inst::Index {
                        dst,
                        base,
                        pos,
                        stride,
                    } if at.contains_key(&base.0) => {
                        let o = at[&base.0];
                        let Some(k) = kv(self, pos) else { return };
                        let Some(d) = k
                            .checked_mul(i64::from(stride))
                            .and_then(|d| o.checked_add(d))
                        else {
                            return;
                        };
                        at.insert(dst.0, d);
                    }
                    Inst::Load { dst, .. } => {
                        at.remove(&dst.0);
                    }
                    Inst::Store { addr, w, .. } => {
                        let Some(&o) = at.get(&addr.0) else { return };
                        written.push((o, o + i64::from(w.bytes())));
                    }
                    Inst::Copy { dst, bytes, .. } => {
                        let (Some(&o), Some(n)) = (at.get(&dst.0), kv(self, bytes)) else {
                            return;
                        };
                        if n > 0 {
                            written.push((o, o.saturating_add(n)));
                        }
                    }
                    Inst::LFill { base, idx, n, v } => {
                        let (Some(&o), Some(i0), Some(cnt), Some(code)) =
                            (at.get(&base.0), kv(self, idx), kv(self, n), kv(self, v))
                        else {
                            return;
                        };
                        if i0 < 0 || cnt < 0 {
                            return;
                        }
                        if cnt == 0 {
                            continue;
                        }
                        let (lo, hi) = (o + i0 / 64 * 32, o + (i0 + cnt + 63) / 64 * 32);
                        if code == 0 && !hit(&written, lo, hi) {
                            self.code[pc] = dead();
                        } else {
                            written.push((lo, hi));
                        }
                    }
                    _ => return,
                }
            }
            let Inst::Br { to } = self.code[self.blocks[b].1 as usize - 1] else {
                return;
            };
            let to = to.0 as usize;
            hops += 1;
            if b == stop.0 || npred[to] != 1 || to == b0 || hops > 64 {
                return;
            }
            b = to;
        }
    }

    /// The storage of a local composite variable (`[data, dope, handle, ...]` slot, object
    /// `[descriptor][value]`) of a subprogram that cannot suspend lives in the frame's
    /// pinned temporary memory (`Inst::NewFrame`) instead of a heap object: no allocation
    /// table entry, no collection. Like the temporaries a callee returns, the memory is freed
    /// by the caller's next statement, so a result that points into the variable stays valid
    /// as long as any result does. Only for types without handles (nothing for the collector
    /// to find in it), objects made once per call (not in a loop), and handles used by
    /// nothing but `Deref` and the store into the variable's handle slot (which stays null).
    fn frame_objects(&mut self) -> bool {
        let types = &self.prog.types;
        if KEEP_VARS.load(std::sync::atomic::Ordering::Relaxed)
            || !matches!(self.f.kind, FuncKind::Function | FuncKind::Procedure)
            || self.f.flags.contains(FuncFlags::MAY_SUSPEND)
            || self.f.body.is_some()
            || !self.nosusp
        {
            return false;
        }
        fn plain(types: &crate::IdVec<TyId, Ty>, ty: TyId, depth: u32) -> bool {
            depth < 32
                && match types.get(ty) {
                    Some(Ty::Int { .. } | Ty::Physical { .. } | Ty::Enum { .. } | Ty::Real) => true,
                    Some(Ty::Array { elem, .. }) => plain(types, *elem, depth + 1),
                    Some(Ty::Record { fields }) => {
                        fields.iter().all(|f| plain(types, f.ty, depth + 1))
                    }
                    _ => false,
                }
        }
        // Handle slots of composite variables.
        let hslots: Vec<i64> = self
            .f
            .locals
            .iter()
            .filter(|l| matches!(types.get(l.ty), Some(Ty::Array { .. } | Ty::Record { .. })))
            .map(|l| i64::from(l.slot) + 16)
            .collect();
        if hslots.is_empty() {
            return false;
        }
        let mut cands: FxMap<u32, usize> = FxMap::default();
        for b in 0..self.blocks.len() {
            if !self.reach[b] {
                continue;
            }
            for pc in self.range(b) {
                let (dst, ty, shared) = match self.code[pc] {
                    Inst::New {
                        dst, ty, shared, ..
                    }
                    | Inst::NewFill {
                        dst, ty, shared, ..
                    } => (dst, ty, shared),
                    _ => continue,
                };
                if !shared
                    && self.defs.get(dst.0 as usize) == Some(&1)
                    && plain(types, ty, 0)
                    && !self.in_cycle(b)
                {
                    cands.insert(dst.0, pc);
                }
            }
        }
        if cands.is_empty() {
            return false;
        }
        // The uses of the handles.
        let mut derefs: FxMap<u32, Vec<usize>> = FxMap::default();
        let mut stores: FxMap<u32, Vec<usize>> = FxMap::default();
        for pc in 0..self.code.len() {
            let i = self.code[pc];
            if is_dead(&i) {
                continue;
            }
            match i {
                Inst::Deref { handle, .. } if cands.contains_key(&handle.0) => {
                    derefs.entry(handle.0).or_default().push(pc);
                    continue;
                }
                Inst::Store { addr, src, w } if cands.contains_key(&src.0) => {
                    let slot = match self.opn(addr) {
                        Opn::A(AddrSpace::Frame, o) => Some(i64::from(o)),
                        _ => None,
                    };
                    if w == Width::B8 && slot.is_some_and(|o| hslots.contains(&o)) {
                        stores.entry(src.0).or_default().push(pc);
                    } else {
                        cands.remove(&src.0);
                    }
                    continue;
                }
                _ => {}
            }
            i.visit(&mut |r, role, _| {
                if role == Role::Use {
                    cands.remove(&r.0);
                }
            });
            if let Some(l) = i.list() {
                for r in self
                    .operands
                    .get(l.start as usize..(l.start + l.len) as usize)
                    .unwrap_or(&[])
                {
                    cands.remove(&r.0);
                }
            }
        }
        let mut done = false;
        let mut cs: Vec<(u32, usize)> = cands.into_iter().collect();
        cs.sort_by_key(|c| c.1);
        for (h, pc) in cs {
            let q = self.fresh(RegTy::Ptr, 1);
            self.code[pc] = match self.code[pc] {
                Inst::New { init, size, .. } => Inst::NewFrame {
                    dst: q,
                    desc: init,
                    dsize: 0,
                    data: init,
                    size,
                    fill: 0,
                },
                Inst::NewFill {
                    desc,
                    dsize,
                    data,
                    size,
                    fill,
                    ..
                } => Inst::NewFrame {
                    dst: q,
                    desc,
                    dsize,
                    data,
                    size,
                    fill,
                },
                i => i,
            };
            for &d in derefs.get(&h).map_or(&[][..], |v| &v[..]) {
                if let Inst::Deref { dst, .. } = self.code[d] {
                    self.code[d] = Inst::Move { dst, src: q };
                }
            }
            for &st in stores.get(&h).map_or(&[][..], |v| &v[..]) {
                self.code[st] = dead();
            }
            done = true;
        }
        if done {
            self.changed = true;
        }
        done
    }

    /// Does the control flow graph have a cycle (a loop)? Code that runs once is not worth
    /// the passes whose cost grows with its size: every subprogram is optimised at its first
    /// call, and most of a library runs a few times.
    ///
    /// With `work`, the blocks that suspend end their paths (a process's outer loop is not
    /// counted).
    fn has_loop(&self, work: bool) -> bool {
        let nb = self.blocks.len();
        // 0 unseen, 1 on the DFS stack, 2 done.
        let mut color = vec![0u8; nb];
        let mut stack = vec![(0usize, 0usize)];
        color[0] = 1;
        while let Some(&(b, k)) = stack.last() {
            let ends = work && self.range(b).any(|pc| self.is_susp(&self.code[pc]));
            if let Some(s) = self.succ(b, k).filter(|_| !ends) {
                if let Some(top) = stack.last_mut() {
                    top.1 += 1;
                }
                match color[s] {
                    1 => return true,
                    0 => {
                        color[s] = 1;
                        stack.push((s, 0));
                    }
                    _ => {}
                }
            } else {
                color[b] = 2;
                stack.pop();
            }
        }
        false
    }

    /// Is block `b` on a cycle of the control flow graph?
    fn in_cycle(&self, b: usize) -> bool {
        let mut seen = vec![false; self.blocks.len()];
        let mut work = Vec::new();
        let mut k = 0;
        while let Some(s) = self.succ(b, k) {
            work.push(s);
            k += 1;
        }
        while let Some(x) = work.pop() {
            if x == b {
                return true;
            }
            if std::mem::replace(&mut seen[x], true) {
                continue;
            }
            let mut k = 0;
            while let Some(s) = self.succ(x, k) {
                work.push(s);
                k += 1;
            }
        }
        false
    }

    /// Does some path from just after the marker at `pc` (every marker when `None`: from the start
    /// of each block) reach an instruction `hit` before the next marker? One entry per `Stmt`.
    fn forward_hits(&self, hit: &dyn Fn(&Inst) -> bool) -> Vec<bool> {
        // What running from the start of a block (or just after a marker) up to the next
        // marker does: reaches it first, hits first, or runs to the end of the block.
        #[derive(Clone, Copy, PartialEq)]
        enum Run {
            Marker,
            Hit,
            Falls,
        }
        let scan = |rg: std::ops::Range<usize>| {
            for pc in rg {
                match &self.code[pc] {
                    Inst::Stmt => return Run::Marker,
                    i if hit(i) => return Run::Hit,
                    _ => {}
                }
            }
            Run::Falls
        };
        let nb = self.blocks.len();
        let local: Vec<Run> = (0..nb).map(|b| scan(self.range(b))).collect();
        // Does some path from the start of the block hit before the next marker?
        let mut hits: Vec<bool> = local.iter().map(|&r| r == Run::Hit).collect();
        loop {
            let mut moved = false;
            for b in 0..nb {
                if hits[b] || local[b] != Run::Falls {
                    continue;
                }
                let mut k = 0;
                while let Some(s) = self.succ(b, k) {
                    k += 1;
                    if hits[s] {
                        hits[b] = true;
                        moved = true;
                        break;
                    }
                }
            }
            if !moved {
                break;
            }
        }
        let mut out = vec![false; self.code.len()];
        for b in 0..nb {
            for pc in self.range(b) {
                if self.code[pc] != Inst::Stmt {
                    continue;
                }
                out[pc] = match scan(pc + 1..self.blocks[b].1 as usize) {
                    Run::Marker => false,
                    Run::Hit => true,
                    Run::Falls => {
                        let mut k = 0;
                        let mut any = false;
                        while let Some(s) = self.succ(b, k) {
                            k += 1;
                            any |= hits[s];
                        }
                        any
                    }
                };
            }
        }
        out
    }

    /// Drop the `Stmt` markers that have nothing to free. At run time a marker frees the
    /// temporaries made since the one before it, so it is needed only where a path from the
    /// previous marker (or the start of the frame) reaches it through an instruction that
    /// leaves some (`leaves_temps`) and the statement after it makes some too, or the statement
    /// after it waits or drives (the predecoder builds fused ops on those markers). The first is a forward dataflow ("dirty") over the markers that stay; dropping a
    /// marker that finds the frame clean changes no other marker's answer. A process starts
    /// clean again after a suspension (the machine empties its temporaries then). Only in
    /// subprograms while no debugger runs (it stops at markers).
    fn drop_statements(&mut self) {
        if KEEP_VARS.load(std::sync::atomic::Ordering::Relaxed)
            || !(self.nosusp && self.f.body.is_none() || self.f.kind == FuncKind::Process)
            || !matches!(
                self.f.kind,
                FuncKind::Function | FuncKind::Procedure | FuncKind::Process
            )
        {
            return;
        }
        let prog = self.prog;
        let clean = |c: FuncId| {
            prog.funcs
                .get(c)
                .is_some_and(|f| f.defined && f.flags.contains(FuncFlags::NO_TEMPS))
        };
        let leaves = |i: &Inst| leaves_temps(i, &self.f.reg_ty, &prog.natives, &clean);
        let pinned = self.forward_hits(&is_wait);
        // (The drives pass moves what a drive builds back to the `Stmt` before it, and the
        // predecoder fuses a run of scalar drives, each from its own `Stmt`, into one op: a
        // statement of one block that scatters a record, or builds a waveform and drives, keeps
        // its marker.)
        let mut pinned = pinned;
        for b in 0..self.blocks.len() {
            let mut start = None;
            let (mut wave, mut drive) = (false, false);
            for pc in self.range(b) {
                match &self.code[pc] {
                    Inst::Stmt => {
                        start = Some(pc);
                        (wave, drive) = (false, false);
                    }
                    Inst::Alloc { .. } => wave = true,
                    Inst::Drive { .. } | Inst::DriveSlot { .. } => drive = true,
                    _ => {}
                }
                if let Some(st) = start
                    && (wave && drive || matches!(self.code[pc], Inst::SigScatter { .. }))
                {
                    pinned[st] = true;
                }
            }
        }
        let ahead = self.forward_hits(&|i| leaves(i) && !is_wait(i));
        let nb = self.blocks.len();
        // The markers left so far; the dirty state at the start of each block and at each marker.
        let kept: Vec<bool> = (0..self.code.len())
            .map(|pc| self.code[pc] == Inst::Stmt && (pinned[pc] || ahead[pc]))
            .collect();
        let process = self.f.kind == FuncKind::Process;
        let walk = |din: &[bool], at: &mut [bool]| -> Vec<bool> {
            let mut out = vec![false; nb];
            for b in 0..nb {
                let mut d = din[b];
                for pc in self.range(b) {
                    let i = &self.code[pc];
                    if kept[pc] {
                        at[pc] = d;
                        d = false;
                    } else if process && matches!(i, Inst::Suspend { .. }) && !is_dead(i) {
                        d = false;
                    } else if !is_wait(i) && leaves(i) {
                        d = true;
                    }
                }
                out[b] = d;
            }
            out
        };
        let mut din = vec![false; nb];
        let mut at = vec![false; self.code.len()];
        loop {
            let out = walk(&din, &mut at);
            let mut moved = false;
            for b in (0..nb).filter(|&b| out[b]) {
                let mut k = 0;
                while let Some(s) = self.succ(b, k) {
                    k += 1;
                    if !din[s] {
                        din[s] = true;
                        moved = true;
                    }
                }
            }
            if !moved {
                break;
            }
        }
        for pc in 0..self.code.len() {
            if self.code[pc] == Inst::Stmt && !(kept[pc] && (pinned[pc] || at[pc])) {
                self.code[pc] = dead();
                self.changed = true;
            }
        }
    }

    /// Registers defined once by the same constant (or address) become one register: each
    /// call copies a frame template with a cell for every register.
    fn merge_consts(&mut self) {
        // (A register must not be live across a suspension: not in code that suspends.)
        if !self.nosusp || self.f.body.is_some() || self.f.flags.contains(FuncFlags::MAY_SUSPEND) {
            return;
        }
        let n = self.defs.len();
        let mut ndefs = vec![0u8; n];
        for i in &self.code {
            if !is_dead(i) {
                for_defs(i, |r| {
                    if let Some(d) = ndefs.get_mut(r.0 as usize) {
                        *d = d.saturating_add(1);
                    }
                });
            }
        }
        // Registers that must stay apart: a promotion's entry initialisation is a second
        // definition the code does not show.
        #[derive(PartialEq, Eq, Hash)]
        enum K {
            I(i64),
            F(u64),
            A(AddrSpace, u32),
        }
        let mut first: FxMap<(K, Option<RegTy>), Reg> = FxMap::default();
        let mut to: Vec<u32> = (0..n as u32).collect();
        let mut any = false;
        for pc in 0..self.code.len() {
            let (dst, k) = match self.code[pc] {
                Inst::Const { dst, v } => (dst, K::I(v)),
                Inst::ConstF { dst, v } => (dst, K::F(v.to_bits())),
                // Only the addresses a frame starts with (the interpreter preloads them).
                Inst::AddrOf {
                    dst,
                    space: space @ (AddrSpace::Frame | AddrSpace::Inst | AddrSpace::Const(_)),
                    off,
                } => (dst, K::A(space, off)),
                _ => continue,
            };
            // (A handle register is never shared: the collector tracks each by its definition.)
            if ndefs.get(dst.0 as usize) != Some(&1) || self.rty(dst) == Some(RegTy::Handle) {
                continue;
            }
            match first.entry((k, self.rty(dst))) {
                std::collections::hash_map::Entry::Occupied(e) => {
                    to[dst.0 as usize] = e.get().0;
                    self.code[pc] = dead();
                    any = true;
                }
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(dst);
                }
            }
        }
        if !any {
            return;
        }
        self.changed = true;
        for i in &mut self.code {
            if !is_dead(i) {
                i.visit_mut(&mut |r, role, _| {
                    if role == Role::Use {
                        *r = Reg(to[r.0 as usize]);
                    }
                });
            }
        }
        let lists: Vec<(usize, usize)> = self
            .code
            .iter()
            .filter(|i| !is_dead(i))
            .filter_map(|i| i.list())
            .map(|l| (l.start as usize, (l.start + l.len) as usize))
            .collect();
        for (a, b) in lists {
            for r in self.operands.get_mut(a..b).unwrap_or(&mut []) {
                *r = Reg(to[r.0 as usize]);
            }
        }
    }

    /// Empty the blocks no path reaches (all but their last instruction): their definitions
    /// would keep registers from counting as defined once. Every entry to a block is a branch,
    /// the start of the code, the block a process pauses at, or the resumption of a suspension
    /// (`reachable`).
    fn kill_unreachable(&mut self) {
        if !matches!(
            self.f.kind,
            FuncKind::Function | FuncKind::Procedure | FuncKind::Process
        ) {
            return;
        }
        let reach = self.reachable();
        // A constant or address defined once, here, that a reachable instruction reads keeps its
        // definition: frames start with it (`merge_consts` made the blocks share it, #714). As
        // `drop_declarations` keeps the definitions the body reads.
        let (mut read, mut ndefs) = (vec![false; self.defs.len()], vec![0u8; self.defs.len()]);
        for (b, &r) in reach.iter().enumerate() {
            for i in &self.code[self.range(b)] {
                for_defs(i, |d| {
                    if let Some(n) = ndefs.get_mut(d.0 as usize) {
                        *n = n.saturating_add(1);
                    }
                });
                if !r || is_dead(i) {
                    continue;
                }
                let list = i.list().and_then(|l| {
                    self.operands
                        .get(l.start as usize..(l.start + l.len) as usize)
                });
                // (A rename not applied yet counts as a read of the register it names.)
                let mut mark = |x: Reg| {
                    if let Some(m) = read.get_mut(self.resolve(x).0 as usize) {
                        *m = true;
                    }
                };
                i.visit(&mut |x, role, _| {
                    if role == Role::Use {
                        mark(x);
                    }
                });
                list.unwrap_or(&[]).iter().for_each(|&x| mark(x));
            }
        }
        for (b, &r) in reach.iter().enumerate() {
            if r {
                continue;
            }
            let rg = self.range(b);
            let last = rg.end.saturating_sub(1);
            for pc in rg.start..last {
                if let Inst::Const { dst, .. }
                | Inst::ConstF { dst, .. }
                | Inst::AddrOf {
                    dst,
                    space: AddrSpace::Frame | AddrSpace::Inst | AddrSpace::Const(_),
                    ..
                } = self.code[pc]
                    && read.get(dst.0 as usize) == Some(&true)
                    && ndefs.get(dst.0 as usize) == Some(&1)
                {
                    continue;
                }
                if !is_dead(&self.code[pc]) {
                    self.code[pc] = dead();
                    self.changed = true;
                }
            }
            // What is left of the block jumps to the block after it, which the interpreter
            // runs through without a jump (`pre.rs`), so the live jumps over it vanish too.
            if rg.end > rg.start
                && b + 1 < self.blocks.len()
                && !is_dead(&self.code[last])
                && self.code[last].is_terminator()
                && !matches!(self.code[last], Inst::Br { to } if to.0 as usize == b + 1)
            {
                self.code[last] = Inst::Br {
                    to: BlockId(b as u32 + 1),
                };
                self.changed = true;
            }
        }
    }

    /// The initialisations promotion inserts at the entry are definitions the tables cannot
    /// see.
    fn hide_inits(&mut self, inited: &[Reg]) {
        for r in inited {
            let i = r.0 as usize;
            self.defs[i] = self.defs[i].saturating_add(1);
            self.kc[i] = None;
            self.ka[i] = None;
        }
    }

    /// One walk over the extended basic blocks.
    fn pass(&mut self) {
        let n = self.blocks.len();
        // Block structure: predecessor counts, the reverse post-order for the walk, and
        // the blocks only one block reaches (as lists through `child`).
        const NONE: u32 = u32::MAX;
        let mut preds = vec![0u32; n];
        let mut first = vec![NONE; n];
        for b in 0..n {
            if !self.reach[b] {
                continue;
            }
            let mut k = 0;
            while let Some(t) = self.succ(b, k) {
                preds[t] += 1;
                first[t] = b as u32;
                k += 1;
            }
        }
        let mut head = vec![NONE; n];
        let mut child = vec![NONE; n];
        let mut is_root = vec![false; n];
        for b in (0..n).rev() {
            is_root[b] = b == 0
                || preds[b] != 1
                || first[b] == b as u32
                || self.f.body.is_some_and(|x| x.0 as usize == b);
            if !is_root[b] {
                let p = first[b] as usize;
                child[b] = head[p];
                head[p] = b as u32;
            }
        }
        let mut order = Vec::with_capacity(n);
        let mut seen = vec![false; n];
        // Post-order by an explicit stack of (block, next successor).
        let mut stack = vec![(0usize, 0usize)];
        seen[0] = true;
        while let Some(&(b, k)) = stack.last() {
            if let Some(s) = self.succ(b, k) {
                if let Some(top) = stack.last_mut() {
                    top.1 += 1;
                }
                if !seen[s] {
                    seen[s] = true;
                    stack.push((s, 0));
                }
            } else {
                order.push(b);
                stack.pop();
            }
        }
        order.reverse();
        let mut visited = vec![false; n];
        let mut work: Vec<Step> = Vec::new();
        self.mval.clear();
        self.rdesc.clear();
        self.lin.clear();
        self.pend = None;
        // The declarative part of a process runs once: its blocks come before the body.
        let cold = self.f.body.map_or(0, |b| b.0 as usize);
        for r in order.iter().copied().chain(0..n) {
            if !is_root[r] || visited[r] || r < cold || !self.reach[r] {
                continue;
            }
            self.susp_depth = 0;
            self.susp_pc = 0;
            work.push(Step::Enter(r));
            while let Some(step) = work.pop() {
                match step {
                    Step::Enter(b) => {
                        visited[b] = true;
                        work.push(Step::Leave {
                            block: b,
                            log: self.tbl.log.len(),
                            susp: (self.susp_depth, self.susp_pc),
                            vlog: self.vlog.len(),
                        });
                        self.depth += 1;
                        self.path_depth[b] = self.depth;
                        self.walk(b);
                        let mut c = head[b];
                        while c != NONE {
                            work.push(Step::Enter(c as usize));
                            c = child[c as usize];
                        }
                    }
                    Step::Leave {
                        block,
                        log,
                        susp,
                        vlog,
                    } => {
                        self.tbl.rollback(log);
                        while self.vlog.len() > vlog {
                            if let Some((r, old)) = self.vlog.pop() {
                                self.vsn[r as usize] = old;
                            }
                        }
                        self.path_depth[block] = u32::MAX;
                        self.depth -= 1;
                        (self.susp_depth, self.susp_pc) = susp;
                    }
                }
            }
        }
    }

    fn range(&self, b: usize) -> std::ops::Range<usize> {
        self.blocks[b].0 as usize..self.blocks[b].1 as usize
    }

    /// The constant `dst` becomes.
    fn make_const(&mut self, pc: usize, dst: Reg, v: i64) {
        self.code[pc] = Inst::Const { dst, v };
        if self.single(dst) {
            self.kc[dst.0 as usize] = Some(v);
        }
        self.changed = true;
    }

    fn make_addr(&mut self, pc: usize, dst: Reg, space: AddrSpace, off: u32) {
        self.code[pc] = Inst::AddrOf { dst, space, off };
        if self.single(dst) {
            self.ka[dst.0 as usize] = Some((space, off));
            self.set_cls(dst, Cls::Space(space));
        }
        self.changed = true;
    }

    /// Drop `pc`, using `x` wherever `dst` was read. Only when both are defined once.
    fn alias(&mut self, pc: usize, dst: Reg, x: Reg) -> bool {
        if self.single(dst) && self.single(x) && dst != x {
            self.rep[dst.0 as usize] = x.0;
            self.nrep += 1;
            self.code[pc] = dead();
            self.changed = true;
            true
        } else {
            false
        }
    }

    /// [`Opt::alias`] when `x` is usable where `dst` is: defined on this path since the
    /// last suspension, or the function never suspends.
    fn alias_ok(&mut self, pc: usize, dst: Reg, x: Reg) -> bool {
        let local = self.nosusp || {
            let pd = self.path_depth[self.def_blk[x.0 as usize] as usize];
            pd != u32::MAX
                && (pd > self.susp_depth
                    || (pd == self.susp_depth && self.def_pc[x.0 as usize] > self.susp_pc))
        };
        local && self.alias(pc, dst, x)
    }

    fn walk(&mut self, b: usize) {
        self.cur = b;
        for pc in self.range(b) {
            match self.code[pc] {
                // Known constants and addresses are tracked by value; nothing to do.
                Inst::Const { dst, .. } | Inst::ConstF { dst, .. } | Inst::AddrOf { dst, .. }
                    if self.single(dst) =>
                {
                    continue;
                }
                Inst::Unreachable { what } if what == DEAD_MSG => continue,
                _ => {}
            }
            self.plin = None;
            self.pfact = None;
            self.simplify(pc);
            let i = self.code[pc];
            if is_dead(&i) {
                continue;
            }
            if self.is_susp(&i) {
                self.lin.clear();
                self.tbl.clear();
                self.susp_depth = self.depth;
                self.susp_pc = pc as u32;
            } else {
                self.effect(&i);
            }
            self.define(pc);
        }
    }

    fn ensure_esc(&mut self) {
        if self.esc.is_none() {
            self.esc = Some(self.escapes());
        }
    }

    /// Frame ranges whose address leaves direct load/store use.
    fn escapes(&self) -> Vec<(i64, i64)> {
        if self.nested {
            return vec![(0, i64::MAX)];
        }
        let range = |o: i64, n: Option<i64>| self.frame_range(o, n);
        // The slot of a local array keeps the address of its own descriptor (`[data, dope, handle,
        // descriptor]`): nothing writes the descriptor through it after the declaration, so that
        // store is no escape (#771).
        let own_slot = |addr: Reg, src: Reg| {
            let at = |r: Reg| match self.ka.get(self.resolve(r).0 as usize) {
                Some(Some((AddrSpace::Frame, o))) => Some(i64::from(*o)),
                _ => None,
            };
            let (Some(a), Some(s)) = (at(addr), at(src)) else {
                return false;
            };
            self.f.locals.iter().any(|l| {
                let (b, e) = (i64::from(l.slot), i64::from(l.slot) + i64::from(l.size));
                b <= a && a < e && b <= s && s < e && a != s
            })
        };
        let mut esc = Vec::new();
        let mut mark = |r: Reg, n: Option<i64>| {
            if let Some(Some((AddrSpace::Frame, o))) = self.ka.get(self.resolve(r).0 as usize) {
                esc.push(range(i64::from(*o), n));
            }
        };
        for i in &self.code {
            match *i {
                Inst::Load { .. } => {}
                Inst::Store { addr, src, .. } if own_slot(addr, src) => {}
                Inst::Store { src, .. } => mark(src, None),
                // Reads and writes through the address: it is used, not kept anywhere (the
                // bytes of a copy that are addresses were marked where they were stored).
                Inst::Copy { .. }
                | Inst::LCopy { .. }
                | Inst::LFill { .. }
                | Inst::LSet { .. }
                | Inst::LGet { .. }
                | Inst::MemEq { .. }
                | Inst::Zero { .. }
                | Inst::MakeDope { .. }
                | Inst::DopeLen { .. }
                | Inst::SlicePos { .. }
                | Inst::Pos { .. }
                | Inst::SubtypeCheck { .. }
                | Inst::LayoutOf { .. }
                // (An object made from bytes at an address reads them; the descriptor of a
                // local array lives in its slot, #771.)
                | Inst::New { .. }
                | Inst::NewFill { .. }
                | Inst::NewFrame { .. } => {}
                _ => {
                    i.visit(&mut |r, role, _| {
                        if role == Role::Use {
                            mark(r, None);
                        }
                    });
                    if let Some(l) = i.list() {
                        for k in l.start as usize..(l.start + l.len) as usize {
                            if let Some(&r) = self.operands.get(k) {
                                mark(r, None);
                            }
                        }
                    }
                }
            }
        }
        esc
    }

    /// Forget what the instruction may overwrite.
    fn effect(&mut self, i: &Inst) {
        match *i {
            Inst::Store { addr, src, w } => {
                let a = self.opn(addr);
                self.kill(a, i64::from(w.bytes()));
                // A stored 8-byte value is what a load of the address gives back.
                if w == Width::B8
                    && let Some(k) = self.kopn(addr)
                {
                    let v = match self.opn(src) {
                        Opn::K(v) => Some(Val::K(v)),
                        Opn::R(r) if self.single(r) => Some(Val::R(r)),
                        Opn::R(r) if self.multi(r) => Some(Val::V(r, self.vsn[r.0 as usize])),
                        // A known address: the register that holds it, when it is defined once.
                        Opn::A(..) if self.single(self.resolve(src)) => {
                            Some(Val::R(self.resolve(src)))
                        }
                        _ => None,
                    };
                    if let Some(v) = v {
                        self.tbl.put(Key::Load(k, Width::B8), v);
                    }
                }
            }
            // A descriptor whose bounds are constants: its lengths are known to the readers.
            // The value goes into a block of its own: no fact is about it.
            Inst::SigRead { dst, .. }
            | Inst::LSet { base: dst, .. }
            | Inst::LFill { base: dst, .. }
            | Inst::LCopy { dst, .. }
            | Inst::Cat { dst, .. } => {
                let a = self.opn(dst);
                self.kill(a, i64::MAX / 4);
            }
            Inst::Zero { addr, bytes } => {
                let a = self.opn(addr);
                self.kill(a, i64::from(bytes));
            }
            Inst::MakeDope { at, parts } => {
                let a = self.opn(at);
                self.kill(a, 8 * i64::from(parts.len));
                for j in 0..parts.len {
                    let part = self.operands.get((parts.start + j) as usize).copied();
                    let v = match part.map(|r| self.opn(r)) {
                        Some(Opn::K(v)) => Some(Val::K(v)),
                        Some(Opn::R(r)) if self.single(r) => Some(Val::R(r)),
                        Some(Opn::R(r)) if self.multi(r) => Some(Val::V(r, self.vsn[r.0 as usize])),
                        _ => None,
                    };
                    if let (Some(v), Some(k)) = (v, self.part_key(a, 8 * i64::from(j))) {
                        self.tbl.put(k, v);
                    }
                }
                if let Some(k) = self.kopn(at) {
                    for d in 0..parts.len / 3 {
                        let w = |j: u32| match self.operands.get((parts.start + 3 * d + j) as usize)
                        {
                            Some(&r) => match self.opn(r) {
                                Opn::K(v) => Some(v),
                                _ => None,
                            },
                            None => None,
                        };
                        if let (Some(l), Some(r), Some(dir)) = (w(0), w(1), w(2))
                            && let Some(n) =
                                dim_len(l, r, dir == 1).filter(|&n| n <= i64::MAX as u64)
                        {
                            self.tbl.put(Key::Dope(k, d as u8), Val::K(n as i64));
                        } else if let Some(dir) = w(2)
                            && let (Some(&l), Some(&r)) = (
                                self.operands.get((parts.start + 3 * d) as usize),
                                self.operands.get((parts.start + 3 * d + 1) as usize),
                            )
                            && let Some(n) = self
                                .affine_len(l, r, dir)
                                .or_else(|| self.lin_len(l, r, dir))
                        {
                            self.tbl.put(Key::Dope(k, d as u8), Val::K(n));
                        } else if let Some(x) = self.len_register(parts, d) {
                            self.tbl.put(Key::Dope(k, d as u8), Val::R(x));
                        }
                    }
                }
            }
            // A debugger stopped here may have written a variable: forget what is known of
            // scalar locals and of memory reached through pointers; the rest stays.
            Inst::Stmt => {
                let named = &self.named;
                let this: &Opt = self;
                let drop_key = |k: &Key| match k.region() {
                    _ if matches!(k, Key::Layout(..)) => true,
                    None => false,
                    Some((a, ..)) if read_only(&this.cls, a) => false,
                    Some((Opn::A(AddrSpace::Frame, o), lo, n)) => {
                        !clear_of(named, i64::from(o) + lo, n)
                    }
                    Some((a, ..)) => !matches!(this.cls_of(a), Some(Cls::Temp | Cls::Block(_))),
                };
                let gone: Vec<Key> = self
                    .tbl
                    .mem
                    .keys()
                    .filter(|k| drop_key(k))
                    .cloned()
                    .collect();
                for k in gone {
                    if let Some(v) = self.tbl.mem.remove(&k) {
                        self.tbl.log.push((k, Some(v)));
                    }
                }
            }
            Inst::Const { .. }
            | Inst::ConstF { .. }
            | Inst::Move { .. }
            | Inst::Select { .. }
            | Inst::AddrOf { .. }
            | Inst::Load { .. }
            | Inst::Alloc { .. }
            | Inst::TempRoot { .. }
            | Inst::PkgAlloc { .. }
            | Inst::Index { .. }
            | Inst::Field { .. }
            | Inst::LayoutOf { .. }
            | Inst::Bin { .. }
            | Inst::Un { .. }
            | Inst::FBin { .. }
            | Inst::FUn { .. }
            | Inst::Phys { .. }
            | Inst::IToF { .. }
            | Inst::FToI { .. }
            | Inst::FToIRange { .. }
            | Inst::Cmp { .. }
            | Inst::FCmp { .. }
            | Inst::Bool { .. }
            | Inst::LEq { .. }
            | Inst::MemEq { .. }
            | Inst::LGet { .. }
            | Inst::Check { .. }
            | Inst::FCheck { .. }
            | Inst::BinRange { .. }
            | Inst::Pos { .. }
            | Inst::SlicePos { .. }
            | Inst::DopeLen { .. }
            | Inst::SubtypeCheck { .. }
            | Inst::Br { .. }
            | Inst::CondBr { .. }
            | Inst::Case { .. }
            | Inst::Ret { .. }
            | Inst::Unreachable { .. }
            | Inst::Deref { .. }
            | Inst::New { .. }
            | Inst::NewFill { .. }
            | Inst::NewFrame { .. }
            | Inst::SigVal { .. }
            | Inst::SigParts { .. }
            | Inst::SigSlice { .. }
            | Inst::SigReadScalar { .. }
            | Inst::SigAttr { .. }
            | Inst::Edge { .. }
            | Inst::Driving { .. }
            | Inst::Drive { .. }
            | Inst::DriveSlot { .. }
            | Inst::Disconnect { .. }
            | Inst::Force { .. }
            | Inst::Release { .. }
            | Inst::WaitOn { .. }
            | Inst::WaitEdge { .. }
            | Inst::WaitFor { .. }
            | Inst::WaitAt { .. }
            | Inst::TimedOut { .. }
            | Inst::Now { .. }
            | Inst::Delta { .. }
            | Inst::Report { .. }
            | Inst::Output { .. }
            | Inst::Cover { .. } => {}
            // Writes through a pointer, calls: anything may have changed.
            _ => self.kill_all(),
        }
    }

    /// A call or an unknown write: nothing survives but constants and frame slots that
    /// neither a debugger shows nor any pointer reaches.
    fn kill_all(&mut self) {
        if self.tbl.mem.is_empty() {
            return;
        }
        self.ensure_esc();
        let esc = self.esc.as_deref().unwrap_or(&[]);
        let named = &self.named;
        let cls = &self.cls;
        self.tbl.forget(|k| match k.region() {
            None => false,
            Some((a, ..)) if read_only(cls, a) => false,
            Some((Opn::A(AddrSpace::Frame, o), lo, n)) => {
                let a = i64::from(o) + lo;
                !(clear_of(named, a, n) && clear_of(esc, a, n))
            }
            Some(_) => true,
        });
    }

    /// A store of `len` bytes at `at`.
    fn kill(&mut self, at: Opn, len: i64) {
        if self.tbl.mem.is_empty() {
            return;
        }
        self.ensure_esc();
        let this: &Opt = self;
        let esc = this.esc.as_deref().unwrap_or(&[]);
        let cs = this.cls_of(at);
        let keep = |k: &Key| {
            let Some((a2, lo, n)) = k.region() else {
                return true;
            };
            if read_only(&this.cls, a2) {
                return true;
            }
            let ce = this.cls_of(a2);
            let apart = matches!((cs, ce), (Some(x), Some(y)) if x != y);
            match (at, a2) {
                (Opn::A(s1, o1), Opn::A(s2, o2)) => {
                    s1 != s2 || {
                        let (a, b) = (i64::from(o1), i64::from(o2) + lo);
                        a + len <= b || b + n <= a
                    }
                }
                // A pointer reaches a frame slot only if the slot's address escaped.
                (_, Opn::A(AddrSpace::Frame, o2)) => apart || clear_of(esc, i64::from(o2) + lo, n),
                (Opn::A(AddrSpace::Frame, o1), _) => apart || clear_of(esc, i64::from(o1), len),
                // Two fields of one block (`Index` of one register by constants): apart when
                // their bytes are.
                (Opn::R(x), Opn::R(y)) => {
                    let at = |r: Reg| this.iat.get(&r).copied().unwrap_or((r, 0));
                    let ((rx, ox), (ry, oy)) = (at(x), at(y));
                    apart || rx == ry && (ox + len <= oy + lo || oy + lo + n <= ox)
                }
                _ => apart,
            }
        };
        let gone: Vec<Key> = self.tbl.mem.keys().filter(|k| !keep(k)).cloned().collect();
        for k in gone {
            if let Some(v) = self.tbl.mem.remove(&k) {
                self.tbl.log.push((k, Some(v)));
            }
        }
    }

    /// May the address be used as part of a key (not a register redefined later)?
    fn key_ok(&self, a: Opn) -> bool {
        match a {
            Opn::R(r) => self.single(r),
            Opn::V(r, v) => self.vsn[r.0 as usize] == v,
            _ => true,
        }
    }

    /// The operand as a key: a multiply defined register at its current version.
    fn kopn(&self, r: Reg) -> Option<Opn> {
        match self.opn(r) {
            Opn::R(x) if self.multi(x) => Some(Opn::V(x, self.vsn[x.0 as usize])),
            o => self.key_ok(o).then_some(o),
        }
    }

    /// The key of the descriptor word `off` bytes into the block at `a` (`None`: not a
    /// stable address).
    fn part_key(&self, a: Opn, off: i64) -> Option<Key> {
        match a {
            Opn::A(s, o) => Some(Key::Part(
                Opn::A(s, u32::try_from(i64::from(o).checked_add(off)?).ok()?),
                0,
            )),
            Opn::R(r) if self.single(r) => Some(Key::Part(a, u32::try_from(off).ok()?)),
            _ => None,
        }
    }

    /// Replace the definition of `d` at `pc` by the value a fact holds, if there is one.
    fn forward(&mut self, pc: usize, d: Reg, key: &Key) -> bool {
        match self.tbl.get(key) {
            Some(Val::R(x)) if self.alias(pc, d, x) => true,
            Some(Val::K(v)) if self.single(d) && self.rty(d) == Some(RegTy::I64) => {
                self.make_const(pc, d, v);
                true
            }
            Some(Val::R(x)) if self.copy_ok(d, x, None) => {
                self.copy(pc, d, x);
                true
            }
            Some(Val::V(x, ver)) if self.copy_ok(d, x, Some(ver)) => {
                self.copy(pc, d, x);
                true
            }
            _ => false,
        }
    }

    /// Reuse an available value, or record this one.
    fn cse(&mut self, pc: usize, dst: Option<Reg>, key: Key) {
        if let Some(v) = self.tbl.get(&key) {
            match (dst, v) {
                (None, Val::Unit) => {
                    self.code[pc] = dead();
                    self.changed = true;
                    return;
                }
                (Some(d), Val::R(x)) if self.rty(d) == self.rty(x) && self.alias(pc, d, x) => {
                    return;
                }
                (Some(d), Val::K(v)) if self.single(d) => {
                    self.make_const(pc, d, v);
                    return;
                }
                // The value is in a register that is not the only one holding it: a copy.
                (Some(d), Val::R(x)) if self.copy_ok(d, x, None) => return self.copy(pc, d, x),
                (Some(d), Val::V(x, ver)) if self.copy_ok(d, x, Some(ver)) => {
                    return self.copy(pc, d, x);
                }
                _ => {}
            }
        }
        match dst {
            None => self.tbl.put(key, Val::Unit),
            Some(d) if self.single(d) => self.tbl.put(key, Val::R(d)),
            Some(d) if self.multi(d) => self.pend = Some((key, d)),
            _ => {}
        }
    }

    /// May `d` take its value from `x` (a register holding the value an earlier instruction
    /// made, still at `ver` when it is defined more than once)?
    fn copy_ok(&self, d: Reg, x: Reg, ver: Option<u32>) -> bool {
        d != x
            && ver.is_none_or(|v| self.vsn[x.0 as usize] == v)
            && (self.single(x) || self.multi(x))
            && (self.single(d) || self.multi(d))
            && self.rty(d) == self.rty(x)
    }

    /// `pc` becomes a move from `x`.
    fn copy(&mut self, pc: usize, d: Reg, x: Reg) {
        self.code[pc] = Inst::Move { dst: d, src: x };
        self.changed = true;
        self.simplify(pc);
    }

    /// The layout words (`[size, align, offsets...]`) of the record type `ty`, which has the
    /// same layout under every descriptor.
    fn static_words(&self, ty: TyId) -> Option<Vec<u64>> {
        let (l, _) = Ty::layout(&self.prog.types, &self.prog.consts, ty, &[]).ok()?;
        if l.offsets.is_empty() {
            return None;
        }
        let mut w = vec![l.size, l.align];
        w.extend(l.offsets);
        Some(w)
    }

    /// The stride (word 2 of the layout) of the one-dimensional array type `ty` when it does
    /// not depend on the bounds: arrays of elements of one size. Checked on three lengths.
    fn static_stride(&self, ty: TyId) -> Option<u64> {
        let types = &self.prog.types;
        if Ty::desc_size(types, ty) != Some(24) || !matches!(types.get(ty), Some(Ty::Array { .. }))
        {
            return None;
        }
        let stride = |n: u64| {
            let mut d = [0u8; 24];
            d[8..16].copy_from_slice(&(n as i64).wrapping_sub(1).to_le_bytes());
            let (l, _) = Ty::layout(types, &self.prog.consts, ty, &d).ok()?;
            l.offsets.is_empty().then_some(l.stride)
        };
        let s = stride(1)?;
        (stride(3)? == s && stride(70)? == s).then_some(s)
    }

    /// Offset of field `n` in record type `ty`, which has the same layout under every
    /// descriptor.
    fn static_offset(&mut self, ty: TyId, n: u32) -> Option<u64> {
        let (types, consts) = (&self.prog.types, &self.prog.consts);
        self.layouts
            .entry(ty)
            .or_insert_with(|| Ty::layout(types, consts, ty, &[]).ok().map(|l| l.0.offsets))
            .as_ref()?
            .get(n as usize)
            .copied()
    }

    /// Read pool constant bytes at `a` into `out` (compact ones in place, not expanded), and
    /// what a copy knows of its frame and of the descriptors of its heap objects (`decl.rs`).
    fn const_read(&self, a: Opn, out: &mut [u8]) -> bool {
        if let Opn::R(r) = a {
            return self.rdesc_read(r, 0, out);
        }
        if let (Opn::A(AddrSpace::Frame, o), Some(wf)) = (a, &self.wf) {
            let o = o as usize;
            let Some(b) = o.checked_add(out.len()).and_then(|e| wf.bytes.get(o..e)) else {
                return false;
            };
            if b.iter().any(Option::is_none) {
                return false;
            }
            for (d, s) in out.iter_mut().zip(b) {
                *d = s.unwrap_or(0);
            }
            return true;
        }
        if let (Opn::A(AddrSpace::Pkg(p), o), Some((_, _, pkgs))) = (a, self.inst) {
            // Package data are written during initialisation only.
            let (o, n) = (o as usize, out.len());
            return pkgs
                .iter()
                .find(|&&(q, _)| q == p)
                .and_then(|(_, d)| d.get(o..o.checked_add(n)?))
                .is_some_and(|b| {
                    out.copy_from_slice(b);
                    true
                });
        }
        if let (Opn::A(AddrSpace::Inst, o), Some((data, holes, _))) = (a, self.inst) {
            let (o, n) = (o as usize, out.len());
            if holes
                .iter()
                .any(|&(h, z)| (h as usize) < o + n && o < (h + z) as usize)
            {
                return false;
            }
            return data.get(o..o + n).is_some_and(|b| {
                out.copy_from_slice(b);
                true
            });
        }
        let Opn::A(AddrSpace::Const(c), o) = a else {
            return false;
        };
        self.prog
            .consts
            .get(c)
            .is_some_and(|d| d.read(o as usize, out))
    }

    /// Read `out.len()` bytes at `delta` bytes from the descriptor at `a`: a pool constant or
    /// instance data, or a pointer to the known descriptor of a heap object.
    fn read_at(&self, a: Opn, delta: u32, out: &mut [u8]) -> bool {
        match a {
            Opn::A(sp, o) => o
                .checked_add(delta)
                .is_some_and(|o| self.const_read(Opn::A(sp, o), out)),
            Opn::R(r) => self.rdesc_read(r, delta, out),
            _ => false,
        }
    }

    fn rdesc_read(&self, r: Reg, delta: u32, out: &mut [u8]) -> bool {
        let Some((bytes, off)) = self.rdesc.get(&r) else {
            return false;
        };
        let o = (*off + delta) as usize;
        o.checked_add(out.len())
            .and_then(|e| bytes.get(o..e))
            .map(|b| out.copy_from_slice(b))
            .is_some()
    }

    /// `(left, right, direction)` of dimension `dim` of a descriptor in the constant pool.
    fn const_dim(&self, dope: Opn, dim: u8) -> Option<(i64, i64, u64)> {
        let mut b = [0u8; 24];
        if !self.read_at(dope, 24 * u32::from(dim), &mut b) {
            return None;
        }
        let w = |k: usize| u64::from_le_bytes(b[k * 8..k * 8 + 8].try_into().unwrap_or([0; 8]));
        Some((w(0) as i64, w(1) as i64, w(2)))
    }

    /// `dst = a + c` or `a - c` for a constant `c` and once-defined `a` and `dst`: kept in `aff`.
    fn note_affine(&mut self, op: IntOp, dst: Reg, a: Reg, b: Reg) {
        if !self.single(dst) {
            return;
        }
        let (x, c) = match (op, self.opn(a), self.opn(b)) {
            (IntOp::Add, Opn::R(x), Opn::K(c)) | (IntOp::Add, Opn::K(c), Opn::R(x)) => (x, c),
            (IntOp::Sub, Opn::R(x), Opn::K(c)) => (x, c.wrapping_neg()),
            _ => return,
        };
        if self.single(x) && c != i64::MIN {
            let (root, o) = self.aff.get(&x).copied().unwrap_or((x, 0));
            if let Some(o) = o.checked_add(c) {
                self.aff.insert(dst, (root, o));
            }
        }
    }

    /// The value of `r` as `mul * base + add`: a constant (no base), or a register at its
    /// current version, or a form `note_lin` kept.
    fn lin_of(&self, r: Reg) -> Option<(Option<Opn>, i64, i64)> {
        match self.opn(r) {
            Opn::K(c) => Some((None, 0, c)),
            Opn::A(..) => None,
            _ => {
                let k = self.kopn(r)?;
                match self.lin.get(&k) {
                    Some(&(b, m, a)) if self.key_ok(b) => Some((Some(b), m, a)),
                    _ => Some((Some(k), 1, 0)),
                }
            }
        }
    }

    /// Keep the linear form of `dst = a op b` (add, subtract; multiply by a constant when the
    /// product is range checked) for `define`, which knows the version `dst` gets.
    // ponytail: an unchecked `Bin` add or subtract is taken not to wrap, as `note_affine` does.
    fn note_lin(&mut self, op: IntOp, dst: Reg, a: Reg, b: Reg, checked: bool) {
        if !(self.single(dst) || self.multi(dst)) {
            return;
        }
        let (Some(x), Some(y)) = (self.lin_of(a), self.lin_of(b)) else {
            return;
        };
        let r = match op {
            IntOp::Add => lin_add(x, y, 1),
            IntOp::Sub => lin_add(x, y, -1),
            IntOp::Mul if checked => lin_mul(x, y),
            _ => None,
        };
        if let Some((Some(base), m, c)) = r
            && m != 0
        {
            self.plin = Some((dst, base, m, c));
        }
    }

    /// The length of a `MakeDope` dimension whose bounds are linear forms of one value with the
    /// same factor (`v(8 * i + 7 downto 8 * i)`), when `note_lin` followed them.
    fn lin_len(&self, l: Reg, r: Reg, dir: i64) -> Option<i64> {
        let ((bl, ml, cl), (br, mr, cr)) = (self.lin_of(l)?, self.lin_of(r)?);
        if bl != br || ml != mr {
            return None;
        }
        let d = match dir {
            0 => cr.checked_sub(cl)?,
            1 => cl.checked_sub(cr)?,
            _ => return None,
        };
        Some(d.checked_add(1)?.max(0))
    }

    /// The length of the result of a native `ieee` call whose operand lengths and sizes are
    /// constants (`Key::Nat`). A null operand gives a null result.
    fn native_len(&self, native: crate::NativeId, args: crate::Regs) -> Option<i64> {
        use crate::ieee::{Form, Op};
        let key = crate::ieee::symbol_key(&self.prog.natives.get(native)?.symbol)?;
        let op = crate::ieee::parse(key)?;
        let regs = self
            .operands
            .get(args.start as usize..(args.start + args.len) as usize)?;
        let mut at = Vec::new();
        let mut p = 0;
        while p < regs.len() {
            at.push(p);
            p += if crate::ieee::array_param(key, at.len() - 1) {
                crate::ieee::ARRAY_REGS
            } else {
                1
            };
        }
        let k = |p: usize| match self.opn(*regs.get(p)?) {
            Opn::K(c) => Some(c),
            _ => None,
        };
        let size = |i: usize| k(*at.get(i)?).map(|n| n.max(0));
        let alen = |i: usize| {
            let p = *at.get(i)?;
            if let (Some(l), Some(r), Some(d)) = (k(p + 1), k(p + 2), k(p + 3)) {
                return dim_len(l, r, d == 1).and_then(|n| i64::try_from(n).ok());
            }
            // The bounds read from a descriptor of known length.
            let word = |j: usize, kind: u8| match self
                .kopn(*regs.get(p + j)?)
                .and_then(|o| self.tbl.get(&Key::Nat(kind, o)))
            {
                Some(Val::K(n)) => Some(n),
                _ => None,
            };
            let n = word(1, 8)?;
            (word(2, 9)? == n && word(3, 10)? == n).then_some(n & 0xffff_ffff)
        };
        match op {
            Op::ToUnsigned | Op::ToSigned | Op::Resize { .. } => size(1),
            Op::Add { form, .. } => match form {
                Form::Vv => {
                    let (a, b) = (alen(0)?, alen(1)?);
                    Some(if a == 0 || b == 0 { 0 } else { a.max(b) })
                }
                Form::Vi => alen(0),
                Form::Iv => alen(1),
            },
            Op::Mul { .. } => {
                let (a, b) = (alen(0)?, alen(1)?);
                Some(if a == 0 || b == 0 {
                    0
                } else {
                    a.checked_add(b)?
                })
            }
            _ => None,
        }
    }

    /// The length of dimension `d` of a `MakeDope` whose bounds are `x + p` and `x + q` for one
    /// once-defined register `x` and constants (a slice `v(4 * i to 4 * i + 3)`).
    fn affine_len(&self, l: Reg, r: Reg, dir: i64) -> Option<i64> {
        let at = |x: Reg| {
            let x = self.resolve(x);
            self.single(x)
                .then(|| self.aff.get(&x).copied().unwrap_or((x, 0)))
        };
        let ((rl, ol), (rr, or)) = (at(l)?, at(r)?);
        if rl != rr {
            return None;
        }
        let d = match dir {
            0 => or.checked_sub(ol)?,
            1 => ol.checked_sub(or)?,
            _ => return None,
        };
        Some(d.checked_add(1)?.max(0))
    }

    /// Dimension `d` of the `MakeDope` with `parts` is `c to x + (c - 1)` for a constant `c` and a
    /// once-defined register `x` that a `DopeLen` made (a length, so not negative): its length is `x`.
    /// (`variable sv : std_ulogic_vector (0 to sl - 1)` with `constant sl := value'length`.)
    fn len_register(&self, parts: crate::Regs, d: u32) -> Option<Reg> {
        let part = |j: u32| {
            self.operands
                .get((parts.start + 3 * d + j) as usize)
                .copied()
        };
        let (Opn::K(c), Opn::K(0)) = (self.opn(part(0)?), self.opn(part(2)?)) else {
            return None;
        };
        let r = self.resolve(part(1)?);
        let (x, o) = self
            .single(r)
            .then(|| self.aff.get(&r).copied().unwrap_or((r, 0)))?;
        let x = self.resolve(x);
        (self.single(x)
            && o.checked_add(1) == Some(c)
            && matches!(
                self.code.get(*self.def_pc.get(x.0 as usize)? as usize),
                Some(Inst::DopeLen { dst, .. }) if *dst == x
            ))
        .then_some(x)
    }

    /// The length of dimension `k` of the descriptor in `r` as a constant (`Ok`) or the register
    /// that holds it (`Err`), when this path says.
    fn dope_len_val(&self, r: Reg, k: u8) -> Option<Result<u64, Reg>> {
        if let Some(n) = self.dope_known(r, k) {
            return Some(Ok(n));
        }
        let d = self.kopn(r)?;
        let v = self
            .tbl
            .get(&Key::ParamDope(d, k))
            .or_else(|| self.tbl.get(&Key::Dope(d, k)));
        match v {
            Some(Val::R(x)) => Some(Err(self.resolve(x))),
            _ => None,
        }
    }

    /// Is `r` the descriptor of a parameter, loaded from the slot that holds it (`X'dope`)?
    fn param_dope(&self, r: Reg) -> bool {
        let r = self.resolve(r);
        if !self.single(r) {
            return false;
        }
        let Some(&Inst::Load {
            addr, w: Width::B8, ..
        }) = self
            .def_pc
            .get(r.0 as usize)
            .and_then(|&p| self.code.get(p as usize))
        else {
            return false;
        };
        let Some(Some((AddrSpace::Frame, off))) = self.ka.get(self.resolve(addr).0 as usize) else {
            return false;
        };
        self.f
            .params
            .iter()
            .any(|p| p.slot == *off && p.name.ends_with("'dope"))
    }

    /// The length of dimension `k` of the descriptor in `r`, when a pool constant or a
    /// `MakeDope` of constants on this path says.
    fn dope_known(&self, r: Reg, k: u8) -> Option<u64> {
        if let Some((l, rr, dir)) = self.const_dim(self.opn(r), k) {
            return dim_len(l, rr, dir == 1);
        }
        match self.kopn(r).and_then(|d| self.tbl.get(&Key::Dope(d, k))) {
            Some(Val::K(n)) => Some(n as u64),
            _ => match self.kopn(r).and_then(|d| self.tbl.get(&Key::Nat(2, d))) {
                Some(Val::K(n)) if k == 0 => Some((n & 0xffff_ffff) as u64),
                _ => None,
            },
        }
    }

    /// The layout words of `ty` under a one-dimensional descriptor of known length: the
    /// layout depends on the length alone.
    fn known_layout(&self, ty: TyId, desc: Reg) -> Option<Vec<u64>> {
        let types = &self.prog.types;
        // A descriptor whose bytes are all known (the pool, instance data, a heap object's)
        // fixes every word of the layout.
        if let Some(ds) = Ty::desc_size(types, ty).filter(|&d| 0 < d && d <= 4096)
            && matches!(types.get(ty), Some(Ty::Record { .. } | Ty::Array { .. }))
        {
            let mut b = vec![0u8; ds as usize];
            if self.read_at(self.opn(desc), 0, &mut b) {
                let (l, _) = Ty::layout(types, &self.prog.consts, ty, &b).ok()?;
                return Some(decl::layout_words(&l));
            }
        }
        if Ty::desc_size(types, ty) != Some(24) || !matches!(types.get(ty), Some(Ty::Array { .. }))
        {
            return None;
        }
        let n = self.dope_known(desc, 0)?;
        let mut d = [0u8; 24];
        d[8..16].copy_from_slice(&(n as i64).wrapping_sub(1).to_le_bytes());
        let (l, _) = Ty::layout(types, &self.prog.consts, ty, &d).ok()?;
        l.offsets
            .is_empty()
            .then(|| vec![l.size, l.align, l.stride, l.count])
    }

    /// A `Pos` in a copy, in an ascending descriptor of instance data with known bounds, of an index
    /// that is no constant and whose result goes into arithmetic: the index check and the
    /// subtraction apart (`Check` of the bounds, `idx - left`), so that the checks of one value
    /// are made once and a range proof can drop one (`Pos` reads the descriptor at every
    /// execution). The failure is the one of `Pos`: the check message of the same name. (A
    /// descending range stays a `Pos`: the message of an index check says `to`.)
    // ponytail: ascending ranges only; descending ones need a message with the other direction,
    // and the optimiser cannot add messages to the program.
    fn pos_to_check(&mut self, pc: usize, dst: Reg, dope: Reg, dim: u8, idx: Reg) -> bool {
        let Inst::Pos { what, .. } = self.code[pc] else {
            return false;
        };
        let Opn::A(AddrSpace::Inst, _) = self.opn(dope) else {
            return false;
        };
        let Some((l, r, 0)) = self.const_dim(self.opn(dope), dim) else {
            return false;
        };
        // The distance of an index inside the bounds is an `i64` (`Pos` allows a `u64`).
        if self.prog.checks.get(what).map(|c| c.dir) != Some(crate::Dir::To)
            || i128::from(r) - i128::from(l) > i128::from(i64::MAX)
            || !self.feeds_arith(pc, dst)
        {
            return false;
        }
        let (lo, hi) = (
            self.const_reg(self.cur, pc, l),
            self.const_reg(self.cur, pc, r),
        );
        self.inserts.push((
            pc,
            Inst::Check {
                kind: CheckKind::Index,
                v: idx,
                lo,
                hi,
                what,
            },
        ));
        self.code[pc] = Inst::Bin {
            op: crate::IntOp::Sub,
            dst,
            a: idx,
            b: lo,
        };
        self.changed = true;
        self.split = true;
        self.simplify(pc);
        true
    }

    /// Is the first instruction after `pc` in its block that reads `r` an add, subtract or
    /// multiply?
    fn feeds_arith(&self, pc: usize, r: Reg) -> bool {
        for q in pc + 1..self.range(self.cur).end {
            let i = &self.code[q];
            let mut read = false;
            i.visit(&mut |x, role, _| read |= role == Role::Use && x == r);
            if let Some(l) = i.list() {
                read |= self
                    .operands
                    .get(l.start as usize..(l.start + l.len) as usize)
                    .is_some_and(|rs| rs.contains(&r));
            }
            if read {
                return matches!(
                    i,
                    Inst::Bin {
                        op: crate::IntOp::Add | crate::IntOp::Sub | crate::IntOp::Mul,
                        ..
                    }
                );
            }
        }
        false
    }

    fn simplify(&mut self, pc: usize) {
        let inst = self.code[pc];
        match inst {
            Inst::Move { dst, src } if self.single(dst) => match self.opn(src) {
                // (A value known from the walk, not from the register's only definition, goes
                // to a register of the type that value has.)
                Opn::K(v) if self.single(src) || self.rty(dst) == Some(RegTy::I64) => {
                    self.make_const(pc, dst, v);
                }
                Opn::A(s, o) if self.single(src) || self.rty(dst) == Some(RegTy::Ptr) => {
                    self.make_addr(pc, dst, s, o);
                }
                Opn::K(_) | Opn::A(..) => {}
                Opn::R(x) => {
                    self.alias_ok(pc, dst, x);
                }
                Opn::V(..) => {}
            },
            Inst::Select { dst, c, a, b } => {
                let src = match self.opn(c) {
                    Opn::K(v) => Some(if v != 0 { a } else { b }),
                    _ => (self.opn(a) == self.opn(b)).then_some(a),
                };
                if let Some(src) = src {
                    self.code[pc] = Inst::Move { dst, src };
                    self.changed = true;
                    self.simplify(pc);
                }
            }
            Inst::Bin { op, dst, a, b } => {
                match simp_bin(op, self.opn(a), self.opn(b)) {
                    Some(Simp::Fold(v)) => return self.make_const(pc, dst, v),
                    Some(Simp::Same(x)) if self.alias_ok(pc, dst, x) => return,
                    _ => {}
                }
                self.note_affine(op, dst, a, b);
                self.note_lin(op, dst, a, b, false);
                if let (Some(x), Some(y)) = (self.kopn(a), self.kopn(b)) {
                    self.cse(pc, Some(dst), Key::Bin(op, x, y));
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
                if let (Opn::K(x), Opn::K(y), Opn::K(l), Opn::K(h)) =
                    (self.opn(a), self.opn(b), self.opn(lo), self.opn(hi))
                    && let Some(v) = fold_int(op, x, y)
                    && l <= v
                    && v <= h
                {
                    return self.make_const(pc, dst, v);
                }
                // (The result is the plain one or the run stops.)
                self.note_affine(op, dst, a, b);
                self.note_lin(op, dst, a, b, true);
                if let (Some(a), Some(b), Some(l), Some(h)) =
                    (self.kopn(a), self.kopn(b), self.kopn(lo), self.kopn(hi))
                {
                    self.cse(pc, Some(dst), Key::BinRange(op, a, b, l, h));
                }
            }
            Inst::Un { op, dst, a } => {
                if let Opn::K(x) = self.opn(a) {
                    let v = match op {
                        IntUnOp::Neg => x.checked_neg(),
                        IntUnOp::Abs => x.checked_abs(),
                    };
                    if let Some(v) = v {
                        return self.make_const(pc, dst, v);
                    }
                }
                if let Some(a) = self.kopn(a) {
                    self.cse(pc, Some(dst), Key::Un(op, a));
                }
            }
            Inst::Cmp { op, dst, a, b } => {
                let v = match (self.opn(a), self.opn(b)) {
                    (Opn::K(x), Opn::K(y)) => Some(cmp(op, x.cmp(&y))),
                    (Opn::R(x), Opn::R(y)) if x == y => {
                        Some(matches!(op, CmpOp::Eq | CmpOp::Le | CmpOp::Ge))
                    }
                    _ => None,
                };
                if let Some(v) = v {
                    return self.make_const(pc, dst, i64::from(v));
                }
                if let (Some(x), Some(y)) = (self.kopn(a), self.kopn(b)) {
                    self.cse(pc, Some(dst), Key::Cmp(op, x, y));
                }
            }
            Inst::Bool { op, dst, a, b } => {
                let v = match (op, self.opn(a), self.opn(b)) {
                    (BoolOp::Not, Opn::K(x), _) => Some(x == 0),
                    (BoolOp::And, Opn::K(x), Opn::K(y)) => Some(x != 0 && y != 0),
                    (BoolOp::Or, Opn::K(x), Opn::K(y)) => Some(x != 0 || y != 0),
                    (BoolOp::Xor, Opn::K(x), Opn::K(y)) => Some((x != 0) != (y != 0)),
                    (BoolOp::And, Opn::K(0), _) | (BoolOp::And, _, Opn::K(0)) => Some(false),
                    (BoolOp::Or, Opn::K(k), _) | (BoolOp::Or, _, Opn::K(k)) if k != 0 => Some(true),
                    _ => None,
                };
                if let Some(v) = v {
                    return self.make_const(pc, dst, i64::from(v));
                }
                let y = if op == BoolOp::Not {
                    Some(Opn::K(0))
                } else {
                    self.kopn(b)
                };
                if let (Some(x), Some(y)) = (self.kopn(a), y) {
                    self.cse(pc, Some(dst), Key::Bool(op, x, y));
                }
            }
            Inst::Index {
                dst,
                base,
                pos,
                stride,
            } => {
                if let (Some(b), Opn::K(c)) = (self.kopn(base), self.opn(pos)) {
                    let at = c.checked_mul(i64::from(stride));
                    let fact = match at {
                        Some(8) => [(0, 1), (2, 6)],
                        Some(16) => [(2, 7), (u8::MAX, 0)],
                        _ => [(u8::MAX, 0); 2],
                    };
                    for (from, to) in fact {
                        if let Some(Val::K(n)) = self.tbl.get(&Key::Nat(from, b)) {
                            self.pfact = Some((dst, to, n));
                        }
                    }
                }
                if self.fold_index(pc) {
                    return;
                }
                if let (Opn::R(x), Opn::K(0)) = (self.opn(base), self.opn(pos))
                    && self.alias_ok(pc, dst, x)
                {
                    return;
                }
                if let (Opn::R(x), Opn::K(c)) = (self.opn(base), self.opn(pos))
                    && self.single(dst)
                    && let Some((b, o)) = self.rdesc.get(&x)
                    && let Some(at) = c
                        .checked_mul(i64::from(stride))
                        .and_then(|d| d.checked_add(i64::from(*o)))
                        .and_then(|v| u32::try_from(v).ok())
                {
                    let b = b.clone();
                    self.rdesc.insert(dst, (b, at));
                }
                if let (Opn::R(x), Opn::K(c)) = (self.opn(base), self.opn(pos))
                    && self.single(dst)
                {
                    let (l, off) = self.lat.get(&x).copied().unwrap_or((x, 0));
                    if (self.lwords.contains_key(&l) || self.lstride.contains_key(&l))
                        && let Some(o) = c
                            .checked_mul(i64::from(stride))
                            .and_then(|d| off.checked_add(d))
                    {
                        self.lat.insert(dst, (l, o));
                    }
                }
                if let (Opn::R(x), Opn::K(c)) = (self.opn(base), self.opn(pos))
                    && self.single(dst)
                    && self.single(x)
                {
                    let (root, off) = self.iat.get(&x).copied().unwrap_or((x, 0));
                    if let Some(o) = c
                        .checked_mul(i64::from(stride))
                        .and_then(|d| off.checked_add(d))
                    {
                        self.iat.insert(dst, (root, o));
                    }
                }
                if let (Some(x), Some(y)) = (self.kopn(base), self.kopn(pos)) {
                    self.cse(pc, Some(dst), Key::Index(x, y, stride));
                }
            }
            Inst::CondBr { c, t, f } => {
                if let Opn::K(v) = self.opn(c) {
                    self.code[pc] = Inst::Br {
                        to: if v != 0 { t } else { f },
                    };
                    self.changed = true;
                    self.pruned = true;
                }
            }
            Inst::Case { v, table } => {
                if let Opn::K(x) = self.opn(v)
                    && let Some(t) = self.prog.cases.get(table)
                {
                    let to = t
                        .arms
                        .iter()
                        .find(|a| a.0 <= x && x <= a.1)
                        .map_or(t.default, |a| a.2);
                    self.code[pc] = Inst::Br { to };
                    self.changed = true;
                    self.pruned = true;
                }
            }
            Inst::Check {
                kind, v, lo, hi, ..
            } => {
                use CheckKind::*;
                let (nv, nl, nh) = (self.opn(v), self.opn(lo), self.opn(hi));
                let ok = match (kind, nv, nl, nh) {
                    (Range | Index, Opn::K(v), Opn::K(l), Opn::K(h)) => l <= v && v <= h,
                    (Length, Opn::K(v), Opn::K(l), _) => v == l,
                    (Null, Opn::K(v), _, _) => v != 0,
                    _ => false,
                };
                if ok {
                    self.code[pc] = dead();
                    self.changed = true;
                    return;
                }
                // Operands the check does not read do not tell checks apart.
                let (nl, nh) = match kind {
                    Null => (Opn::K(0), Opn::K(0)),
                    Length => (nl, Opn::K(0)),
                    _ => (nl, nh),
                };
                // (A loop variable defined more than once is keyed at its current version.)
                let nv = self.kopn(v).unwrap_or(nv);
                if self.key_ok(nv) && self.key_ok(nl) && self.key_ok(nh) {
                    self.cse(pc, None, Key::Check(kind, nv, nl, nh));
                }
            }
            Inst::Load { dst, addr, w } if w != Width::Sig => {
                let a = self.opn(addr);
                if w == Width::B8
                    && let Some(ak) = self.kopn(addr)
                {
                    for (from, to) in [(1, 2), (2, 8), (6, 9), (7, 10)] {
                        if let Some(Val::K(n)) = self.tbl.get(&Key::Nat(from, ak)) {
                            self.pfact = Some((dst, to, n));
                        }
                    }
                }
                if w == Width::B8
                    && let Opn::R(x) = a
                {
                    let (l, off) = self.lat.get(&x).copied().unwrap_or((x, 0));
                    if let Some(words) = self.lwords.get(&l)
                        && off % 8 == 0
                        && let Some(&v) = words.get((off / 8) as usize)
                    {
                        return self.make_const(pc, dst, v as i64);
                    }
                    if off == 16
                        && let Some(&v) = self.lstride.get(&l)
                    {
                        return self.make_const(pc, dst, v as i64);
                    }
                }
                // A word of the frame the declarative part fixed (`decl.rs`).
                if w == Width::B8
                    && let Opn::A(AddrSpace::Frame, o) = a
                    && let Some(wf) = &self.wf
                {
                    if let Some(&(sp, off)) = wf.addrs.get(&o)
                        && self.rty(dst) == Some(RegTy::Ptr)
                    {
                        return self.make_addr(pc, dst, sp, off);
                    }
                    if let Some(d) = wf.descs.get(&o).cloned()
                        && self.single(dst)
                    {
                        self.rdesc.insert(dst, d);
                    }
                }
                let n = w.bytes() as usize;
                let mut x = [0u8; 8];
                // (Instance data folds into integer registers only: a pointer or a handle
                // register keeps its load, the collector's roots rule.)
                let folds = self.inst.is_none()
                    || !matches!(a, Opn::A(AddrSpace::Inst | AddrSpace::Frame, _) | Opn::R(_))
                    || self.rty(dst) == Some(RegTy::I64);
                if folds && self.const_read(a, &mut x[..n]) {
                    // A 4-byte load sign-extends (narrow integer elements, #415).
                    let v = if n == 4 {
                        i64::from(i32::from_le_bytes([x[0], x[1], x[2], x[3]]))
                    } else {
                        u64::from_le_bytes(x) as i64
                    };
                    return self.make_const(pc, dst, v);
                }
                // A word of a descriptor that a `MakeDope` wrote on this path.
                if w == Width::B8 {
                    let (root, off) = match a {
                        Opn::R(x) => match self.iat.get(&x) {
                            Some(&(r, o)) => (Opn::R(r), o),
                            None => (a, 0),
                        },
                        _ => (a, 0),
                    };
                    if let Some(k) = self.part_key(root, off)
                        && self.forward(pc, dst, &k)
                    {
                        return;
                    }
                }
                if let Some(a) = self.kopn(addr) {
                    self.cse(pc, Some(dst), Key::Load(a, w));
                }
            }
            Inst::DopeLen { dst, dope, dim } => {
                if let Some((l, r, dir)) = self.const_dim(self.opn(dope), dim)
                    && let Some(n) = dim_len(l, r, dir == 1).filter(|&n| n <= i64::MAX as u64)
                {
                    return self.make_const(pc, dst, n as i64);
                }
                if dim == 0
                    && let Some(Val::K(n)) =
                        self.kopn(dope).and_then(|d| self.tbl.get(&Key::Nat(2, d)))
                {
                    return self.make_const(pc, dst, n & 0xffff_ffff);
                }
                if let Some(d) = self.kopn(dope) {
                    let key = if self.param_dope(dope) {
                        Key::ParamDope(d, dim)
                    } else {
                        Key::Dope(d, dim)
                    };
                    self.cse(pc, Some(dst), key);
                }
            }
            Inst::CallNative {
                native,
                args,
                dst: Some(d),
            } => {
                if let Some(n) = self.native_len(native, args)
                    && n <= i64::from(i32::MAX)
                {
                    self.nid += 1;
                    self.pfact = Some((d, 0, self.nid << 32 | n));
                }
            }
            // Two descriptors of known, equal lengths: the check passes.
            Inst::SubtypeCheck {
                dope, want, dims, ..
            } => {
                if self.kopn(dope).is_some() && self.kopn(dope) == self.kopn(want)
                    || (0..dims).all(|k| {
                        let a = self.dope_len_val(dope, k);
                        a.is_some() && a == self.dope_len_val(want, k)
                    })
                {
                    self.code[pc] = dead();
                    self.changed = true;
                }
            }
            Inst::Pos {
                dst,
                dope,
                dim,
                idx,
                ..
            } => {
                if let (Some((l, r, dir)), Opn::K(i)) =
                    (self.const_dim(self.opn(dope), dim), self.opn(idx))
                {
                    let pos = match dir {
                        0 if l <= i && i <= r => Some(i.wrapping_sub(l)),
                        1 if r <= i && i <= l => Some(l.wrapping_sub(i)),
                        _ => None,
                    };
                    if let Some(p) = pos {
                        return self.make_const(pc, dst, p);
                    }
                }
                if self.pos_to_check(pc, dst, dope, dim, idx) {
                    return;
                }
                if let (Some(d), Some(i)) = (self.kopn(dope), self.kopn(idx)) {
                    self.cse(pc, Some(dst), Key::Pos(d, dim, i));
                }
            }
            Inst::SlicePos {
                dst,
                dope,
                left,
                right,
                dir,
                ..
            } => {
                if let (Opn::K(a), Opn::K(b), Opn::K(w)) =
                    (self.opn(left), self.opn(right), self.opn(dir))
                {
                    // A null slice: no element, no check.
                    if dim_len(a, b, w == 1) == Some(0) {
                        return self.make_const(pc, dst, 0);
                    }
                    if let Some((l, r, d)) = self.const_dim(self.opn(dope), 0) {
                        let inside = |i: i64| match d {
                            0 => l <= i && i <= r,
                            1 => r <= i && i <= l,
                            _ => false,
                        };
                        if w == d as i64 && inside(a) && inside(b) {
                            let pos = if d == 0 {
                                a.wrapping_sub(l)
                            } else {
                                l.wrapping_sub(a)
                            };
                            return self.make_const(pc, dst, pos);
                        }
                    }
                }
                // The same slice of the same descriptor again: the check passed, the position
                // is the same.
                if let (Some(d), Some(a), Some(b), Some(w)) = (
                    self.kopn(dope),
                    self.kopn(left),
                    self.kopn(right),
                    self.kopn(dir),
                ) {
                    self.cse(pc, Some(dst), Key::Slice(d, [a, b, w]));
                }
            }
            Inst::LayoutOf { dst, ty, desc, .. } => {
                if self.single(dst)
                    && let Some(w) = self.known_layout(ty, desc)
                {
                    self.lwords.insert(dst, w);
                    return;
                }
                if let Some(d) = self.kopn(desc) {
                    self.cse(pc, Some(dst), Key::Layout(ty, d));
                }
            }
            Inst::Field {
                dst,
                base,
                layout,
                n,
            } => {
                // The layout of a type without a descriptor is the same everywhere: the
                // field is a constant offset.
                if let Opn::R(l) = self.opn(layout)
                    && let Some(&ty) = self.slay.get(&l)
                    && let Some(off) = self.static_offset(ty, n)
                {
                    let c = self.const_reg(self.cur, pc, off as i64);
                    self.code[pc] = Inst::Index {
                        dst,
                        base,
                        pos: c,
                        stride: 1,
                    };
                    self.changed = true;
                    return self.simplify(pc);
                }
                // The words of this layout are known (a descriptor of known bounds).
                if let Opn::R(l) = self.opn(layout)
                    && let Some(&off) = self.lwords.get(&l).and_then(|w| w.get(2 + n as usize))
                {
                    let c = self.const_reg(self.cur, pc, off as i64);
                    self.code[pc] = Inst::Index {
                        dst,
                        base,
                        pos: c,
                        stride: 1,
                    };
                    self.changed = true;
                    return self.simplify(pc);
                }
                if let (Some(b), Some(l)) = (self.kopn(base), self.kopn(layout)) {
                    self.cse(pc, Some(dst), Key::Field(b, l, n));
                }
            }
            _ => {}
        }
    }

    /// Scalar frame slots that are only loaded and stored directly live in registers.
    ///
    /// Lowering keeps every variable, parameter and loop bound in a frame slot, so a loop
    /// loads and stores the same few slots on every pass. A slot whose address never leaves
    /// direct loads and stores (the `escapes` of the walk), that no nested subprogram, root
    /// map or debugger reaches, becomes a register: its loads are moves from it, its stores
    /// moves into it, and the moves fold into the instructions around them. Only functions
    /// that cannot suspend qualify (a register is not live across a suspension), and slots of
    /// named variables only while no debugger runs ([`keep_variables`]).
    fn promote(&mut self) -> Option<Vec<Reg>> {
        let f = self.f;
        let process = f.kind == FuncKind::Process;
        if self.nested
            || !(process
                || matches!(f.kind, FuncKind::Function | FuncKind::Procedure)
                    && !f.flags.contains(FuncFlags::MAY_SUSPEND)
                    && f.body.is_none()
                    && self.nosusp)
        {
            return None;
        }
        let n = self.defs.len();
        let mut ndefs = vec![0u8; n];
        for i in &self.code {
            if !is_dead(i) {
                for_defs(i, |r| {
                    if let Some(d) = ndefs.get_mut(r.0 as usize) {
                        *d = d.saturating_add(1);
                    }
                });
            }
        }
        // Registers holding the address of a frame slot (all defined once, or give up).
        let mut faddr: FxMap<u32, u32> = FxMap::default();
        for i in &self.code {
            if let Inst::AddrOf {
                dst,
                space: AddrSpace::Frame,
                off,
            } = *i
            {
                if ndefs.get(dst.0 as usize) != Some(&1) {
                    return None;
                }
                faddr.insert(dst.0, off);
            }
        }
        struct Acc {
            w: Width,
            /// The type of the first stored value, else of the first load.
            ty: Option<RegTy>,
            stored: bool,
            ok: bool,
            /// The register types of the accesses, each once.
            kinds: Vec<Option<RegTy>>,
            stores: usize,
            sites: Vec<usize>,
        }
        let mut acc: std::collections::BTreeMap<u32, Acc> = std::collections::BTreeMap::new();
        // Frame ranges read or written as signal references (two registers).
        let mut wide: Vec<(i64, i64)> = Vec::new();
        for (pc, i) in self.code.iter().enumerate() {
            let (addr, w, ty, i_is_store) = match *i {
                Inst::Load { dst, addr, w } => (addr, w, self.rty(dst), false),
                Inst::Store { addr, src, w } => (addr, w, self.rty(src), true),
                _ => continue,
            };
            let Some(&off) = faddr.get(&addr.0) else {
                continue;
            };
            if w == Width::Sig {
                wide.push((i64::from(off), 16));
                continue;
            }
            let a = acc.entry(off).or_insert(Acc {
                w,
                ty,
                stored: i_is_store,
                ok: true,
                kinds: Vec::new(),
                stores: 0,
                sites: Vec::new(),
            });
            if !a.kinds.contains(&ty) {
                a.kinds.push(ty);
            }
            a.stores += usize::from(i_is_store);
            if a.w != w || !same_class(a.ty, ty) {
                a.ok = false;
            }
            if i_is_store && !a.stored {
                a.ty = ty;
                a.stored = true;
            }
            a.sites.push(pc);
        }
        if acc.is_empty() {
            return None;
        }
        self.esc = None;
        self.ensure_esc();
        let esc = self.esc.clone().unwrap_or_default();
        let keep_vars = KEEP_VARS.load(std::sync::atomic::Ordering::Relaxed);
        // Frame ranges the precise collector scans.
        let roots: Vec<(i64, i64, bool)> = f
            .roots
            .iter()
            .map(|&(s, k)| {
                let s = i64::from(s);
                let z = match k {
                    crate::RootKind::Handle | crate::RootKind::Addr => 8,
                    crate::RootKind::Composite(_) => f
                        .locals
                        .iter()
                        .find(|l| i64::from(l.slot) == s)
                        .map_or(i64::MAX / 4, |l| i64::from(l.size)),
                };
                (s, z, matches!(k, crate::RootKind::Composite(_)))
            })
            .collect();
        let mut cands: Vec<(u32, Reg, RegTy, bool)> = Vec::new();
        let mut prev_end = i64::MIN;
        let offs: Vec<u32> = acc.keys().copied().collect();
        for (k, &off) in offs.iter().enumerate() {
            let a = &acc[&off];
            let o = i64::from(off);
            let end = o + i64::from(a.w.bytes());
            let next = offs.get(k + 1).map_or(i64::MAX, |&x| i64::from(x));
            let overlap = o < prev_end || end > next;
            prev_end = prev_end.max(end);
            let Some(ty) = a.ty else {
                continue;
            };
            // (Handles and addresses of a process stay in their slots, where the collector
            // finds them: only numbers qualify.)
            // (A byte holds a boolean, a bit, a character or another enumeration value: the
            // register keeps the same number.)
            let bytes = i64::from(a.w.bytes());
            if overlap
                || !a.ok
                || !matches!(a.w, Width::B8 | Width::B1)
                || ty == RegTy::Sig
                || (process
                    && !a
                        .kinds
                        .iter()
                        .all(|k| matches!(k, Some(RegTy::I64 | RegTy::F64))))
                || !clear_of(&esc, o, bytes)
                || !clear_of(&wide, o, bytes)
            {
                continue;
            }
            // A slot a root map covers keeps its value for the collector. A slot the function
            // never writes (a parameter) may also be read from a register. A written one may
            // live in a register only when that register is a root as well: a `Handle` one
            // for a handle slot, a `Ptr` one for an address slot (the collector finds the live
            // ones of each frame, `roots::live_refs`), and every access has exactly that type.
            if roots.iter().any(|&(s, z, comp)| {
                o < s + z
                    && s < o + bytes
                    && (comp || (a.stores > 0 && !self.register_root(&a.kinds, ty, f, s)))
            }) {
                continue;
            }
            if let Some(l) = f
                .locals
                .iter()
                .find(|l| i64::from(l.slot) <= o && o < i64::from(l.slot) + i64::from(l.size))
            {
                let scalar = self
                    .prog
                    .types
                    .get(l.ty)
                    .is_some_and(|t| t.scalar_size().is_some());
                // (A plugin may deposit into the variables of a process.)
                if keep_vars
                    || (process && external())
                    || (scalar && (i64::from(l.slot) != o || i64::from(l.size) != bytes))
                {
                    continue;
                }
            }
            let is_param = f.params.iter().any(|p| p.slot == off);
            cands.push((off, Reg(0), ty, is_param));
        }
        if process && !cands.is_empty() {
            // A register is not live across a suspension: only slots no suspension leaves
            // live (read before written on the way on) qualify.
            let offs: Vec<u32> = cands.iter().map(|c| c.0).collect();
            let dead = self.dead_at_suspends(&offs, &faddr);
            let mut k = 0;
            cands.retain(|_| {
                k += 1;
                dead[k - 1]
            });
        }
        if cands.is_empty() {
            return None;
        }
        for c in &mut cands {
            c.1 = self.fresh(c.2, 2);
        }
        let mut pregs = Vec::with_capacity(cands.len());
        for &(off, p, _, is_param) in &cands {
            for &pc in &acc[&off].sites {
                self.code[pc] = match self.code[pc] {
                    Inst::Load { dst, .. } => Inst::Move { dst, src: p },
                    Inst::Store { src, .. } => Inst::Move { dst: p, src },
                    i => i,
                };
            }
            pregs.push((p, is_param));
        }
        // Registers that must hold a value at the start: parameters (from their slots), and
        // locals some path reads before writing (zero, like the slot).
        let first = self.blocks[0].0 as usize;
        let undefined = self.undefined_promoted(&pregs);
        let mut inited = Vec::new();
        for (k, &(off, p, ty, is_param)) in cands.iter().enumerate() {
            if is_param || undefined[k] {
                inited.push(p);
            }
            if is_param {
                let a = self.fresh(RegTy::Ptr, 1);
                self.defs[a.0 as usize] = 1;
                self.inserts.push((
                    first,
                    Inst::AddrOf {
                        dst: a,
                        space: AddrSpace::Frame,
                        off,
                    },
                ));
                self.inserts.push((
                    first,
                    Inst::Load {
                        dst: p,
                        addr: a,
                        w: acc[&off].w,
                    },
                ));
            } else if undefined[k] {
                self.inserts.push((
                    first,
                    match ty {
                        RegTy::F64 => Inst::ConstF { dst: p, v: 0.0 },
                        _ => Inst::Const { dst: p, v: 0 },
                    },
                ));
            }
        }
        self.promote_cleanup(&pregs.iter().map(|x| x.0).collect::<Vec<_>>());
        self.changed = true;
        Some(inited)
    }

    /// Is a promoted register of type `ty`, the only type accessing the root slot at `s`, a
    /// root of the same kind as the slot?
    fn register_root(&self, kinds: &[Option<RegTy>], ty: RegTy, f: &crate::Func, s: i64) -> bool {
        kinds.len() == 1
            && f.roots.iter().any(|&(o, k)| {
                i64::from(o) == s
                    && match k {
                        crate::RootKind::Handle => ty == RegTy::Handle,
                        crate::RootKind::Addr => ty == RegTy::Ptr,
                        crate::RootKind::Composite(_) => false,
                    }
            })
    }

    /// Recompute what `analyse` found, after the code changed.
    fn reanalyse(&mut self) {
        self.defs.fill(0);
        self.kc.fill(None);
        self.ka.fill(None);
        self.cls.fill(None);
        for (i, r) in self.rep.iter_mut().enumerate() {
            *r = i as u32;
        }
        self.nrep = 0;
        self.slay.clear();
        self.lwords.clear();
        self.lstride.clear();
        self.lat.clear();
        self.iat.clear();
        self.aff.clear();
        self.esc = None;
        self.analyse();
    }

    /// A new register of type `ty`, defined more than once.
    fn fresh(&mut self, ty: RegTy, defs: u8) -> Reg {
        let r = Reg(self.defs.len() as u32);
        self.new_regs.push(ty);
        self.defs.push(defs);
        self.kc.push(None);
        self.ka.push(None);
        self.cls.push(None);
        self.vsn.push(0);
        self.rep.push(r.0);
        self.def_blk.push(0);
        self.def_pc.push(0);
        r
    }

    /// For each frame slot of `slots` (sorted offsets of direct 8-byte loads and stores, reached
    /// through the address registers of `faddr`) of a process: does no suspension leave it live?
    /// A slot is live where some path reads it before writing it; the code behind a `wait;`
    /// with nothing to wake it does not run.
    fn dead_at_suspends(&self, slots: &[u32], faddr: &FxMap<u32, u32>) -> Vec<bool> {
        let m = slots.len();
        let w = m.div_ceil(64).max(1);
        let nb = self.blocks.len();
        let perm = self.waits_forever();
        let slot = |i: &Inst| -> Option<(usize, bool)> {
            let (a, st) = match *i {
                Inst::Load { addr, .. } => (addr, false),
                Inst::Store { addr, .. } => (addr, true),
                _ => return None,
            };
            let off = faddr.get(&a.0)?;
            slots.binary_search(off).ok().map(|k| (k, st))
        };
        // The live slots before instruction `i`, given those after it.
        let back = |live: &mut Vec<u64>, i: &Inst| {
            if perm && matches!(i, Inst::Suspend { .. }) {
                live.fill(0);
            }
            match slot(i) {
                Some((k, true)) => live[k / 64] &= !(1 << (k % 64)),
                Some((k, false)) => live[k / 64] |= 1 << (k % 64),
                None => {}
            }
        };
        // Where a block that ends in a suspension goes on: the block after it.
        let after = |b: usize| -> Option<usize> {
            let (s, e) = self.blocks[b];
            (!perm
                && e > s
                && self.is_susp(&self.code[e as usize - 1])
                && !matches!(self.code[e as usize - 1], Inst::Br { .. }))
            .then(|| self.blocks.iter().position(|x| x.0 == e && x.1 > e))
            .flatten()
        };
        let mut live_in: Vec<Vec<u64>> = vec![vec![0; w]; nb];
        let live_out = |live_in: &Vec<Vec<u64>>, b: usize| {
            let mut out = vec![0u64; w];
            let mut k = 0;
            while let Some(s) = self.succ(b, k) {
                k += 1;
                for (o, x) in out.iter_mut().zip(&live_in[s]) {
                    *o |= x;
                }
            }
            if let Some(n) = after(b) {
                for (o, x) in out.iter_mut().zip(&live_in[n]) {
                    *o |= x;
                }
            }
            out
        };
        loop {
            let mut changed = false;
            for b in (0..nb).rev() {
                let mut live = live_out(&live_in, b);
                for pc in self.range(b).rev() {
                    if !is_dead(&self.code[pc]) {
                        back(&mut live, &self.code[pc]);
                    }
                }
                if live != live_in[b] {
                    live_in[b] = live;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let mut bad = vec![0u64; w];
        for b in 0..nb {
            let mut live = live_out(&live_in, b);
            for pc in self.range(b).rev() {
                let i = &self.code[pc];
                if is_dead(i) {
                    continue;
                }
                if self.is_susp(i) && !(perm && matches!(i, Inst::Suspend { .. })) {
                    for (o, x) in bad.iter_mut().zip(&live) {
                        *o |= x;
                    }
                }
                back(&mut live, i);
            }
        }
        (0..m).map(|k| bad[k / 64] >> (k % 64) & 1 == 0).collect()
    }

    /// Per promoted register: is it read on some path before it is written (parameters
    /// count as written)?
    fn undefined_promoted(&self, pregs: &[(Reg, bool)]) -> Vec<bool> {
        let m = pregs.len();
        let w = m.div_ceil(64).max(1);
        let idx: FxMap<u32, usize> = pregs.iter().enumerate().map(|(k, x)| (x.0.0, k)).collect();
        let nb = self.blocks.len();
        let mut gen_ = vec![vec![0u64; w]; nb];
        let mut exposed = vec![vec![0u64; w]; nb];
        for b in 0..nb {
            for pc in self.range(b) {
                let i = &self.code[pc];
                if is_dead(i) {
                    continue;
                }
                let mut uses = Vec::new();
                i.visit(&mut |r, role, _| {
                    if role == Role::Use {
                        uses.push(r);
                    }
                });
                if let Some(l) = i.list() {
                    uses.extend(
                        self.operands
                            .get(l.start as usize..(l.start + l.len) as usize)
                            .unwrap_or(&[]),
                    );
                }
                for r in uses {
                    if let Some(&k) = idx.get(&r.0)
                        && gen_[b][k / 64] >> (k % 64) & 1 == 0
                    {
                        exposed[b][k / 64] |= 1 << (k % 64);
                    }
                }
                for_defs(i, |r| {
                    if let Some(&k) = idx.get(&r.0) {
                        gen_[b][k / 64] |= 1 << (k % 64);
                    }
                });
            }
        }
        let mut entry = vec![0u64; w];
        for (k, x) in pregs.iter().enumerate() {
            if x.1 {
                entry[k / 64] |= 1 << (k % 64);
            }
        }
        let mut input: Vec<Vec<u64>> = vec![vec![!0; w]; nb];
        input[0] = entry;
        let mut work: Vec<usize> = vec![0];
        while let Some(b) = work.pop() {
            let out: Vec<u64> = input[b].iter().zip(&gen_[b]).map(|(a, g)| a | g).collect();
            let mut k = 0;
            while let Some(s) = self.succ(b, k) {
                k += 1;
                let new: Vec<u64> = input[s].iter().zip(&out).map(|(a, o)| a & o).collect();
                if new != input[s] {
                    input[s] = new;
                    work.push(s);
                }
            }
        }
        (0..m)
            .map(|k| {
                (0..nb).any(|b| {
                    exposed[b][k / 64] >> (k % 64) & 1 == 1 && input[b][k / 64] >> (k % 64) & 1 == 0
                })
            })
            .collect()
    }

    /// Fold the moves a promotion left: the copy of a promoted register that a load made is
    /// replaced by the register wherever the copy is still equal to it (on every path from
    /// the load), and a store's move is dropped by letting the instruction that computed the
    /// stored value write the register itself.
    fn promote_cleanup(&mut self, pregs: &[Reg]) {
        let n = self.defs.len();
        let mut ndefs = vec![0u8; n];
        let mut uses = vec![0u32; n];
        for i in &self.code {
            if is_dead(i) {
                continue;
            }
            for_defs(i, |r| {
                if let Some(d) = ndefs.get_mut(r.0 as usize) {
                    *d = d.saturating_add(1);
                }
            });
            i.visit(&mut |r, role, _| {
                if role == Role::Use
                    && let Some(u) = uses.get_mut(r.0 as usize)
                {
                    *u += 1;
                }
            });
            if let Some(l) = i.list() {
                for r in self
                    .operands
                    .get(l.start as usize..(l.start + l.len) as usize)
                    .unwrap_or(&[])
                {
                    if let Some(u) = uses.get_mut(r.0 as usize) {
                        *u += 1;
                    }
                }
            }
        }
        let is_p: std::collections::HashSet<u32> = pregs.iter().map(|r| r.0).collect();
        let ptype: FxMap<u32, Option<RegTy>> = pregs.iter().map(|r| (r.0, self.rty(*r))).collect();
        let rtys: Vec<Option<RegTy>> = (0..n as u32).map(|r| self.rty(Reg(r))).collect();
        let nb = self.blocks.len();
        let reach = self.reachable();
        // The copies: `d := p` with `d` defined once and `p` promoted.
        let mut copies: Vec<(u32, u32)> = Vec::new();
        let mut copy_of: FxMap<u32, usize> = FxMap::default();
        let mut by_p: FxMap<u32, Vec<usize>> = FxMap::default();
        for (b, &r) in reach.iter().enumerate() {
            if !r {
                continue;
            }
            for pc in self.range(b) {
                if let Inst::Move { dst: d, src: p } = self.code[pc]
                    && is_p.contains(&p.0)
                    && !is_p.contains(&d.0)
                    && ndefs[d.0 as usize] == 1
                    && !copy_of.contains_key(&d.0)
                {
                    copy_of.insert(d.0, copies.len());
                    by_p.entry(p.0).or_default().push(copies.len());
                    copies.push((d.0, p.0));
                }
            }
        }
        let w = copies.len().div_ceil(64).max(1);
        // The copies available after instruction `i`, given those before it.
        let step = |avail: &mut Vec<u64>, i: &Inst| {
            for_defs(i, |r| {
                if let Some(ks) = by_p.get(&r.0) {
                    for &k in ks {
                        avail[k / 64] &= !(1 << (k % 64));
                    }
                }
            });
            if let Inst::Move { dst: d, src: p } = *i
                && let Some(&k) = copy_of.get(&d.0)
                && copies[k].1 == p.0
            {
                avail[k / 64] |= 1 << (k % 64);
            }
        };
        let mut input: Vec<Vec<u64>> = vec![vec![!0; w]; nb];
        input[0] = vec![0; w];
        if !copies.is_empty() {
            let mut work: Vec<usize> = vec![0];
            while let Some(b) = work.pop() {
                let mut out = input[b].clone();
                for pc in self.range(b) {
                    if !is_dead(&self.code[pc]) {
                        step(&mut out, &self.code[pc]);
                    }
                }
                let mut k = 0;
                while let Some(s) = self.succ(b, k) {
                    k += 1;
                    let new: Vec<u64> = input[s].iter().zip(&out).map(|(a, o)| a & o).collect();
                    if new != input[s] {
                        input[s] = new;
                        work.push(s);
                    }
                }
            }
        }
        for b in 0..nb {
            if !reach[b] {
                continue;
            }
            let mut avail = input[b].clone();
            let mut def_at: FxMap<u32, usize> = FxMap::default();
            let mut touched: FxMap<u32, usize> = FxMap::default();
            for pc in self.range(b) {
                if is_dead(&self.code[pc]) {
                    continue;
                }
                let mut i = self.code[pc];
                if !copies.is_empty() {
                    let copy = |r: u32| -> Option<u32> {
                        let k = *copy_of.get(&r)?;
                        (avail[k / 64] >> (k % 64) & 1 == 1).then(|| copies[k].1)
                    };
                    i.visit_mut(&mut |r, role, t| {
                        if role == Role::Use
                            && let Some(p) = copy(r.0)
                            && fits(t, ptype[&p])
                        {
                            uses[r.0 as usize] -= 1;
                            uses[p as usize] += 1;
                            *r = Reg(p);
                        }
                    });
                    if let Some(l) = i.list() {
                        for r in self
                            .operands
                            .get_mut(l.start as usize..(l.start + l.len) as usize)
                            .unwrap_or(&mut [])
                        {
                            if let Some(p) = copy(r.0)
                                && (rtys[r.0 as usize] == ptype[&p]
                                    || (rtys[r.0 as usize] == Some(RegTy::I64)
                                        && ptype[&p] == Some(RegTy::Handle)))
                            {
                                uses[r.0 as usize] -= 1;
                                uses[p as usize] += 1;
                                *r = Reg(p);
                            }
                        }
                    }
                    self.code[pc] = i;
                }
                // A store's move: let the defining instruction write the register.
                if let Inst::Move { dst: p, src: s } = i
                    && is_p.contains(&p.0)
                    && !is_p.contains(&s.0)
                    && ndefs[s.0 as usize] == 1
                    && uses[s.0 as usize] == 1
                    && let Some(&pd) = def_at.get(&s.0)
                    && touched.get(&p.0).is_none_or(|&t| t <= pd)
                    && retargetable(&self.code[pd])
                    && {
                        let mut ok = true;
                        self.code[pd].visit(&mut |r, role, t| {
                            if role == Role::Def && r == s {
                                ok = t.is_none() || t == ptype[&p.0];
                            }
                        });
                        ok
                    }
                {
                    let mut d = self.code[pd];
                    d.visit_mut(&mut |r, role, _| {
                        if role == Role::Def && *r == s {
                            *r = p;
                        }
                    });
                    self.code[pd] = d;
                    self.code[pc] = dead();
                    uses[s.0 as usize] = 0;
                    if let Some(ks) = by_p.get(&p.0) {
                        for &k in ks {
                            avail[k / 64] &= !(1 << (k % 64));
                        }
                    }
                    touched.insert(p.0, pc);
                    continue;
                }
                // Reads and writes of promoted registers, and the definitions of this block.
                i.visit(&mut |r, _, _| {
                    if is_p.contains(&r.0) {
                        touched.insert(r.0, pc);
                    }
                });
                if let Some(l) = i.list() {
                    for r in self
                        .operands
                        .get(l.start as usize..(l.start + l.len) as usize)
                        .unwrap_or(&[])
                    {
                        if is_p.contains(&r.0) {
                            touched.insert(r.0, pc);
                        }
                    }
                }
                for_defs(&i, |r| {
                    if ndefs[r.0 as usize] == 1 {
                        def_at.insert(r.0, pc);
                    }
                });
                step(&mut avail, &i);
            }
        }
    }

    /// Remove pure definitions nobody reads.
    fn dce(&mut self) {
        let mut uses = vec![0u32; self.defs.len()];
        let rep = &self.rep;
        let res = |mut r: Reg| {
            while let Some(&x) = rep.get(r.0 as usize) {
                if x == r.0 {
                    break;
                }
                r = Reg(x);
            }
            r
        };
        // Follow the renames and count the readers in one pass. A block that only
        // `MakeDope`s write (`wonly` of its readers) is never read.
        let rename = self.nrep > 0;
        let mut wonly = vec![0u32; self.defs.len()];
        for i in &mut self.code {
            if is_dead(i) {
                continue;
            }
            // (The one register `MakeDope` reads outside its list is the block it writes.)
            let writes = matches!(i, Inst::MakeDope { .. });
            i.visit_mut(&mut |r, role, _| {
                if role == Role::Use {
                    if rename {
                        *r = res(*r);
                    }
                    if let Some(u) = uses.get_mut(r.0 as usize) {
                        *u += 1;
                    }
                    if writes && let Some(w) = wonly.get_mut(r.0 as usize) {
                        *w += 1;
                    }
                }
            });
            if let Some(l) = i.list() {
                for r in self
                    .operands
                    .get_mut(l.start as usize..(l.start + l.len) as usize)
                    .unwrap_or(&mut [])
                {
                    if rename {
                        *r = res(*r);
                    }
                    if let Some(u) = uses.get_mut(r.0 as usize) {
                        *u += 1;
                    }
                }
            }
        }
        // The instructions to be inserted read registers too (a check on an index, its bounds).
        for (_, i) in &mut self.inserts {
            i.visit_mut(&mut |r, role, _| {
                if role == Role::Use {
                    if rename {
                        *r = res(*r);
                    }
                    if let Some(u) = uses.get_mut(r.0 as usize) {
                        *u += 1;
                    }
                }
            });
        }
        self.drop_stored_only_blocks(&mut uses);
        // One backward sweep removes whole chains: a user comes after its operands.
        for pc in (0..self.code.len()).rev() {
            let i = self.code[pc];
            // The writers of a block nobody reads, then its allocation.
            match i {
                Inst::MakeDope { at, .. }
                    if self.single(at)
                        && uses.get(at.0 as usize) == wonly.get(at.0 as usize)
                        && matches!(self.cls_of(Opn::R(at)), Some(Cls::Block(b)) if b == at.0) =>
                {
                    let mut dead_uses: Vec<Reg> = Vec::new();
                    i.visit(&mut |r, role, _| {
                        if role == Role::Use {
                            dead_uses.push(r);
                        }
                    });
                    if let Some(l) = i.list() {
                        dead_uses.extend(
                            self.operands
                                .get(l.start as usize..(l.start + l.len) as usize)
                                .unwrap_or(&[]),
                        );
                    }
                    for r in dead_uses {
                        if let Some(u) = uses.get_mut(r.0 as usize) {
                            *u = u.saturating_sub(1);
                        }
                    }
                    wonly[at.0 as usize] = wonly[at.0 as usize].saturating_sub(1);
                    self.code[pc] = dead();
                    self.changed = true;
                    continue;
                }
                Inst::Alloc { dst, size }
                    if self.single(dst) && uses.get(dst.0 as usize) == Some(&0) =>
                {
                    if let Some(u) = uses.get_mut(size.0 as usize) {
                        *u = u.saturating_sub(1);
                    }
                    self.code[pc] = dead();
                    self.changed = true;
                    continue;
                }
                _ => {}
            }
            let dst = match i {
                Inst::LayoutOf { dst, .. }
                    if self.slay.contains_key(&dst)
                        || self.lwords.contains_key(&dst)
                        || self.lstride.contains_key(&dst) =>
                {
                    Some(dst)
                }
                _ => removable_dst(&i),
            };
            let Some(dst) = dst else {
                continue;
            };
            if uses.get(dst.0 as usize) != Some(&0) {
                continue;
            }
            i.visit(&mut |r, role, _| {
                if role == Role::Use
                    && let Some(u) = uses.get_mut(r.0 as usize)
                {
                    *u = u.saturating_sub(1);
                }
            });
            self.code[pc] = dead();
            self.changed = true;
        }
        self.uses = uses;
    }

    /// A block that is allocated and stored into but never read, passed on or kept (the fat
    /// pair of an inlined table read, once the loads of its two words were forwarded): its
    /// `Alloc`, the `Store`s and the `Index`es that only address them go. `uses` is the count of
    /// readers of each register, kept in step.
    fn drop_stored_only_blocks(&mut self, uses: &mut [u32]) {
        let n = uses.len();
        let mut stored = vec![0u32; n];
        for i in &self.code {
            if let Inst::Store { addr, w, .. } = *i
                && w != Width::Sig
                && !is_dead(i)
                && let Some(c) = stored.get_mut(addr.0 as usize)
            {
                *c += 1;
            }
        }
        if !self.code.iter().any(|i| matches!(i, Inst::Alloc { .. })) {
            return;
        }
        // An `Index` whose result only addresses stores addresses stores itself.
        for pc in (0..self.code.len()).rev() {
            if let Inst::Index { dst, base, .. } = self.code[pc]
                && self.single(dst)
                && uses[dst.0 as usize] > 0
                && uses[dst.0 as usize] == stored[dst.0 as usize]
            {
                stored[base.0 as usize] += 1;
            }
        }
        let mut dead_reg = vec![false; n];
        let mut any = false;
        for pc in 0..self.code.len() {
            match self.code[pc] {
                Inst::Alloc { dst, .. }
                    if self.single(dst)
                        && uses[dst.0 as usize] > 0
                        && uses[dst.0 as usize] == stored[dst.0 as usize] =>
                {
                    dead_reg[dst.0 as usize] = true;
                    any = true;
                }
                Inst::Index { dst, base, .. }
                    if dead_reg[base.0 as usize]
                        && self.single(dst)
                        && uses[dst.0 as usize] == stored[dst.0 as usize] =>
                {
                    dead_reg[dst.0 as usize] = true;
                }
                _ => {}
            }
        }
        if !any {
            return;
        }
        for pc in 0..self.code.len() {
            let i = self.code[pc];
            let gone = match i {
                Inst::Store { addr, w, .. } => w != Width::Sig && dead_reg[addr.0 as usize],
                Inst::Index { dst, .. } | Inst::Alloc { dst, .. } => dead_reg[dst.0 as usize],
                _ => false,
            };
            if gone {
                i.visit(&mut |r, role, _| {
                    if role == Role::Use
                        && let Some(u) = uses.get_mut(r.0 as usize)
                    {
                        *u = u.saturating_sub(1);
                    }
                });
                self.code[pc] = dead();
                self.changed = true;
            }
        }
    }

    /// Remove the dead instructions and add the inserted ones, with their locations; the new
    /// block ranges.
    #[allow(clippy::needless_range_loop)]
    fn compact(&mut self, loc: &mut Vec<StmtId>, via: Option<&mut Vec<u32>>) -> Vec<(u32, u32)> {
        self.inserts.sort_by_key(|x| x.0);
        let mut ins = self.inserts.iter().peekable();
        let mut code = Vec::with_capacity(self.code.len() + self.inserts.len());
        let mut locs = Vec::with_capacity(code.capacity());
        // The inlining record of each instruction, when the function has one.
        let mut vias: Vec<u32> = Vec::new();
        let keep_via = via.is_some();
        let old_via = |pc: usize| via.as_ref().map_or(0, |v| v[pc]);
        let mut blocks = Vec::with_capacity(self.blocks.len());
        for &(s, e) in &self.blocks {
            let start = code.len() as u32;
            for pc in s as usize..e as usize {
                while let Some(&(_, i)) = ins.next_if(|x| x.0 == pc) {
                    // Skip a constant nobody ended up reading.
                    if let Inst::Const { dst, .. } = i
                        && self.uses.get(dst.0 as usize) == Some(&0)
                    {
                        continue;
                    }
                    code.push(i);
                    locs.push(loc[pc]);
                    if keep_via {
                        vias.push(old_via(pc));
                    }
                }
                if !is_dead(&self.code[pc]) {
                    code.push(self.code[pc]);
                    locs.push(loc[pc]);
                    if keep_via {
                        vias.push(old_via(pc));
                    }
                }
            }
            blocks.push((start, code.len() as u32));
        }
        self.code = code;
        *loc = locs;
        if let Some(v) = via {
            *v = vias;
        }
        blocks
    }
}

/// May a register of type `have` be used where an operand of type `want` is expected (the
/// verifier takes a `Handle` register for an `I64` one)?
fn fits(want: Option<RegTy>, have: Option<RegTy>) -> bool {
    want.is_none() || want == have || (want == Some(RegTy::I64) && have == Some(RegTy::Handle))
}

/// Do two register types hold the same kind of value (integers and addresses, or reals)?
/// `x + sign * y` for two linear forms (`Opt::lin_of`), when they have the same base or one is
/// a constant.
fn lin_add(
    x: (Option<Opn>, i64, i64),
    y: (Option<Opn>, i64, i64),
    sign: i64,
) -> Option<(Option<Opn>, i64, i64)> {
    let c = x.2.checked_add(sign.checked_mul(y.2)?)?;
    let my = sign.checked_mul(y.1)?;
    match (x.0, y.0) {
        (b, None) => Some((b, x.1, c)),
        (None, b) => Some((b, my, c)),
        (Some(a), Some(b)) if a == b => {
            let m = x.1.checked_add(my)?;
            Some((if m == 0 { None } else { Some(a) }, m, c))
        }
        _ => None,
    }
}

/// `x * y` for two linear forms, when one is a constant.
fn lin_mul(
    x: (Option<Opn>, i64, i64),
    y: (Option<Opn>, i64, i64),
) -> Option<(Option<Opn>, i64, i64)> {
    let (v, k) = match (x.0, y.0) {
        (None, None) => return Some((None, 0, x.2.checked_mul(y.2)?)),
        (None, _) => (y, x.2),
        (_, None) => (x, y.2),
        _ => return None,
    };
    Some((v.0, v.1.checked_mul(k)?, v.2.checked_mul(k)?))
}

fn same_class(a: Option<RegTy>, b: Option<RegTy>) -> bool {
    let class = |t| match t {
        Some(RegTy::I64 | RegTy::Ptr) => 1,
        Some(RegTy::F64) => 2,
        Some(RegTy::Handle) => 3,
        _ => 0,
    };
    class(a) != 0 && class(a) == class(b)
}

/// Does `i` leave memory in the frame's temporary area, which only the next `Stmt` (or the
/// end of the activation) frees? Allocations, a call that may (`callee_clean` says whether a
/// call of the function leaves none, [`FuncFlags::NO_TEMPS`]; a result that is not a scalar is
/// a composite in temporary memory) and the few signal references and layouts built there.
/// Everything the interpreter keeps in registers or writes to memory somebody else allocated
/// leaves none (checked against every `temp_alloc` of `yavs-interp` and the temporary bumps of
/// the LLVM tier: `Alloc`, `NewFrame`, `LayoutOf`, and what the interpreter's `step` runs for
/// them). `New` stays on the list because the optimiser may turn it into `NewFrame`; a pass that
/// makes another instruction allocate must add it here.
pub(crate) fn leaves_temps(
    i: &Inst,
    reg_ty: &[RegTy],
    natives: &crate::IdVec<crate::NativeId, crate::NativeRef>,
    callee_clean: &dyn Fn(FuncId) -> bool,
) -> bool {
    let scalar = |d: &Option<Reg>| {
        d.is_none_or(|r| {
            matches!(
                reg_ty.get(r.0 as usize),
                Some(RegTy::I64 | RegTy::F64 | RegTy::Handle)
            )
        })
    };
    !is_dead(i)
        && match i {
            Inst::Call { func, dst, .. } | Inst::CallProt { func, dst, .. } => {
                !(callee_clean(*func) && scalar(dst))
            }
            // (A native `ieee` subprogram with a VHDL body, `...@<id>`, gives the call back on
            // metavalues and then builds a descriptor in temporary memory for each array.)
            Inst::CallNative { native, dst, .. } => {
                !scalar(dst)
                    || natives.get(*native).is_none_or(|n| {
                        n.symbol.starts_with("yavs.ieee.") && n.symbol.contains('@')
                    })
            }
            Inst::Alloc { .. }
            | Inst::TempRoot { .. }
            | Inst::LayoutOf { .. }
            | Inst::New { .. }
            | Inst::NewFill { .. }
            | Inst::NewFrame { .. }
            | Inst::SigParts { .. }
            | Inst::SigSlice { .. }
            | Inst::CallForeign { .. }
            | Inst::CallIndirect { .. }
            | Inst::Elab { .. } => true,
            // (A wait or a suspension in a called procedure keeps the caller's temporaries.)
            _ => is_wait(i),
        }
}

/// A wait or a suspension: `static_wait` in the predecoder starts a wait at its `Stmt`, in both
/// tiers, so that marker stays.
fn is_wait(i: &Inst) -> bool {
    matches!(
        i,
        Inst::WaitOn { .. }
            | Inst::WaitEdge { .. }
            | Inst::WaitFor { .. }
            | Inst::WaitAt { .. }
            | Inst::Suspend { .. }
    ) && !is_dead(i)
}

/// An instruction whose one result register can be renamed: no other effect on registers.
fn retargetable(i: &Inst) -> bool {
    match *i {
        Inst::Load { w, .. } => w != Width::Sig,
        Inst::Bin { .. }
        | Inst::BinRange { .. }
        | Inst::Un { .. }
        | Inst::Cmp { .. }
        | Inst::Bool { .. }
        | Inst::Select { .. }
        | Inst::Index { .. }
        | Inst::Pos { .. }
        | Inst::SlicePos { .. }
        | Inst::DopeLen { .. }
        | Inst::Field { .. }
        | Inst::FBin { .. }
        | Inst::FUn { .. }
        | Inst::IToF { .. } => true,
        _ => false,
    }
}

/// The register an instruction defines, if it can be dropped when that is never read: no
/// side effect and no failure other than a broken invariant.
fn removable_dst(i: &Inst) -> Option<Reg> {
    match *i {
        Inst::Const { dst, .. }
        | Inst::ConstF { dst, .. }
        | Inst::Move { dst, .. }
        | Inst::Select { dst, .. }
        | Inst::AddrOf { dst, .. }
        | Inst::Index { dst, .. }
        | Inst::Cmp { dst, .. }
        | Inst::FCmp { dst, .. }
        | Inst::Bool { dst, .. }
        | Inst::IToF { dst, .. } => Some(dst),
        Inst::Load { dst, w, .. } if w != Width::Sig => Some(dst),
        Inst::Field { dst, .. } => Some(dst),
        _ => None,
    }
}

fn cmp(op: CmpOp, o: std::cmp::Ordering) -> bool {
    use std::cmp::Ordering::*;
    match op {
        CmpOp::Eq => o == Equal,
        CmpOp::Ne => o != Equal,
        CmpOp::Lt => o == Less,
        CmpOp::Le => o != Greater,
        CmpOp::Gt => o == Greater,
        CmpOp::Ge => o != Less,
    }
}

/// The checked result of `x op y` when it succeeds (the interpreter's `int_op`).
fn fold_int(op: IntOp, x: i64, y: i64) -> Option<i64> {
    match op {
        IntOp::Add => x.checked_add(y),
        IntOp::Sub => x.checked_sub(y),
        IntOp::Mul => x.checked_mul(y),
        IntOp::Div if y != 0 => x.checked_div(y),
        // (A negative exponent or an overflow is the interpreter's error to raise.)
        IntOp::Pow if y >= 0 => match x {
            0 => Some(i64::from(y == 0)),
            1 => Some(1),
            -1 => Some(if y % 2 == 0 { 1 } else { -1 }),
            _ => u32::try_from(y).ok().and_then(|e| x.checked_pow(e)),
        },
        IntOp::Rem if y != 0 => Some(x.checked_rem(y).unwrap_or(0)),
        IntOp::Mod if y != 0 => {
            let r = x.checked_rem(y).unwrap_or(0);
            Some(if r != 0 && ((r < 0) != (y < 0)) {
                r + y
            } else {
                r
            })
        }
        _ => None,
    }
}

fn simp_bin(op: IntOp, x: Opn, y: Opn) -> Option<Simp> {
    use Opn::*;
    match (op, x, y) {
        (_, K(a), K(b)) => fold_int(op, a, b).map(Simp::Fold),
        (IntOp::Add, R(r), K(0)) | (IntOp::Add, K(0), R(r)) => Some(Simp::Same(r)),
        (IntOp::Sub, R(r), K(0)) => Some(Simp::Same(r)),
        (IntOp::Sub, R(a), R(b)) if a == b => Some(Simp::Fold(0)),
        (IntOp::Mul, R(r), K(1)) | (IntOp::Mul, K(1), R(r)) => Some(Simp::Same(r)),
        (IntOp::Mul, K(0), R(_)) | (IntOp::Mul, R(_), K(0)) => Some(Simp::Fold(0)),
        (IntOp::Div, R(r), K(1)) => Some(Simp::Same(r)),
        (IntOp::Rem | IntOp::Mod, R(_), K(1)) => Some(Simp::Fold(0)),
        _ => None,
    }
}
