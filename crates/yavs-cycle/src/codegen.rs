//! Cranelift code of the region: one function per trigger, the operations of the member bodies in
//! schedule order on the flat image of two-state words. A logic value of `len` elements is
//! `ceil(len / 64)` words, element 0 in bit 0 of the first, the bits above `len` zero; an integer
//! is one word.
//!
//! A check that fails (`Op::Int`, `Op::Range`, the dynamic indices) returns its site number plus
//! one through the function's one exit of failed checks; `RegionProc` makes the error. Without `--two-state` it first stores its operands in
//! the two error words, for the message's values (A34: the default drops them). With
//! `YAVS_CYCLE_CHECKS=off` (developers, to measure them) range and overflow checks are left out
//! and a dynamic index is clamped into its vector, so the code stays memory safe.
//!
//! The only `unsafe` of the crate: the region calls the generated code on its image.
#![allow(unsafe_code)]

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use cranelift_codegen::entity::SecondaryMap;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::types::{I32, I64};
use cranelift_codegen::ir::{
    AbiParam, Block as CBlock, InstBuilder, InstructionData, MemFlagsData, Opcode, Value,
};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module, ModuleReloc};
use yavs_cache::{Key, KeyBuilder, Store};

use crate::lower::{Arith, Cmp, Gate, Op, P4F, Ref, ShiftK};

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
    entries: [Vec<Run>; 4],
    /// The edge's own steps in parts: a part runs when one of its words is not 0 (none: always).
    edge_parts: Vec<EdgePart>,
    /// The words each loop group drives: (first, count), and where [`Run::Save`] copies them.
    groups: Vec<(Vec<(u32, u32)>, u32)>,
    /// The runs of words each [`Run::Copy`] copies: (to, from, count).
    copies: Vec<Vec<(u32, u32, u32)>>,
    /// The words the code addresses.
    words: usize,
    /// By instance of a shared body: the byte offset in the image of each of its blocks.
    tables: Vec<Box<[usize]>>,
    pub(crate) guards: Vec<Guard>,
    /// Per step of the settle: the settle's units (`compile`'s `units`) it runs; a step with
    /// none always runs.
    pub(crate) settle_units: Vec<Vec<u32>>,
    /// The steps with units, as a mark: a mark that holds them all runs the settle unmarked.
    pub(crate) every: Box<[u64]>,
    /// The steps without units, which run under any mark; none when one lies behind a guard
    /// (then `steps` looks at every step).
    fixed: Option<Box<[u64]>>,
    /// The words of the region's signals, the first ones of the image (`YAVS_CYCLE_SKIP_CHECK`).
    signal_words: usize,
    /// With `YAVS_CYCLE_SKIP_CHECK`: per step of the settle, the words its members drive.
    pub(crate) settle_outs: Vec<Vec<(u32, u32)>>,
    /// The word of each clocked body run only when it is 1 (`rt::Skip`).
    pub(crate) run_words: Vec<u32>,
    /// The count word of the log of notes (`compile`'s `log`): a run of the skip check logs none.
    log_count: usize,
    /// `YAVS_CYCLE_SKIP_CHECK=1`: a run a guard would skip runs and must change nothing.
    pub(crate) skip_check: bool,
    /// `YAVS_CYCLE_SKIP_CHECK=full` (also the reruns of `1`): the words that hold state (signals,
    /// clocked variables), which a run with every step and no guard must leave as the run did.
    pub(crate) full_check: Option<Vec<(u32, u32)>>,
    /// With `YAVS_CYCLE_STATS`: per guard its runs and skips.
    pub(crate) counts: Option<Vec<[std::sync::atomic::AtomicU64; 2]>>,
    /// The guard whose skipped run changed something (`STALE`).
    pub(crate) stale: std::sync::atomic::AtomicUsize,
}

/// A combinational member the settle skips when the words it reads are what they were at its
/// last run (its outputs are then what that run left): those words (first, count), where their
/// copy from the last run starts, a word saying what the last run was (0 none, 1 without checks,
/// 2 with), and the words it drives (for `YAVS_CYCLE_SKIP_CHECK`).
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Guard {
    pub name: String,
    pub ins: Vec<(u32, u32)>,
    /// Where each span of `ins` starts in the copy, from `copy`.
    pub at: Vec<u32>,
    pub copy: u32,
    pub flag: u32,
    pub outs: Vec<(u32, u32)>,
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
    /// Guard `g` of the next `n` steps, a run with checks or without.
    Guard(usize, usize, bool),
    /// The runs of words of [`Code::copies`] `c`, copied.
    Copy(usize),
}

/// A step of the settle as [`compile`] takes it.
#[derive(Clone)]
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
    /// The next `n` steps that are not guards run behind guard `g` (in functions of their own),
    /// with checks or without.
    Guard(usize, bool, usize),
    /// The edge's steps after it, up to the next gate, run only when one of these words is not 0
    /// (edge classes' words; none: always).
    Gate(Vec<u32>),
    /// Runs of words copied (to, from, count): a commit's next-state copies, as one copy each in
    /// the runner instead of a load and a store per word in the code.
    Copy(Vec<(u32, u32, u32)>),
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

/// A part of the edge's own steps: it runs when one of its words is not 0 (none: always); its
/// steps tracked (`Code::entries[1]`) and untracked (`[3]`), and the word choosing the untracked
/// ones when it is 1 (`u32::MAX`: none).
type EdgePart = (
    Vec<u32>,
    std::ops::Range<usize>,
    std::ops::Range<usize>,
    u32,
);

/// The edge's steps with its clocked bodies untracked, per edge word the mode word of its class,
/// and the mode word of the part before any gate ([`EdgePart`]).
pub(crate) type Untracked<'a> = (&'a [Step], &'a [(u32, u32)], u32);

/// The site [`Code::run`] gives when a loop group changed in a pass after its fixed point.
pub(crate) const UNSETTLED: u32 = u32::MAX;

/// The site [`Code::run`] gives when a run a guard would have skipped changed a word it drives
/// (`YAVS_CYCLE_SKIP_CHECK=1`; the guard in [`Code::stale`]), or with `full` when the run differed
/// from one of every step ([`FULL`] in [`Code::stale`]).
pub(crate) const STALE: u32 = u32::MAX - 1;

/// [`Code::stale`] after `YAVS_CYCLE_SKIP_CHECK=full` saw a difference.
pub(crate) const FULL: usize = usize::MAX - 1;

/// What `YAVS_CYCLE_SKIP_CHECK` asks for (unset or empty: nothing).
#[derive(PartialEq)]
pub(crate) enum SkipCheck {
    Off,
    One,
    Full,
}

/// `YAVS_CYCLE_SKIP_CHECK`; another value is an error naming the accepted ones.
pub(crate) fn skip_check() -> Result<SkipCheck, String> {
    let v = std::env::var_os("YAVS_CYCLE_SKIP_CHECK").unwrap_or_default();
    let v = v.to_string_lossy();
    match &*v {
        "" => Ok(SkipCheck::Off),
        "1" => Ok(SkipCheck::One),
        "full" => Ok(SkipCheck::Full),
        _ => Err(format!(
            "YAVS_CYCLE_SKIP_CHECK takes `1` or `full`, not `{v}`{}",
            yavs_diag::suggest::tail(&v, "value", ["full"].into_iter())
        )),
    }
}

/// A pass after a loop group's schedule confirms its fixed point (debug builds, unless
/// `YAVS_CYCLE_CHECKS=on`; `YAVS_CYCLE_CHECKS=all`).
pub(crate) fn confirm_on() -> bool {
    match std::env::var_os("YAVS_CYCLE_CHECKS") {
        Some(v) if v == "all" => true,
        Some(v) if v == "on" => false,
        _ => cfg!(debug_assertions),
    }
}

impl Code {
    /// Run function `f` on `img`: `Err(site)` when a check failed. With `mark`, a step of the
    /// settle (also the edge's) with units (`settle_units`) whose bit in `mark` is clear is
    /// skipped: what it reads did not change since the last settle.
    pub(crate) fn run(&self, img: &mut [u64], f: Fun, mark: Option<&[u64]>) -> Result<(), u32> {
        assert!(
            img.len() >= self.words,
            "the image is smaller than the code's words"
        );
        if let Some(state) = &self.full_check
            && f != Fun::Init
        {
            return self.full(img, f, mark, state);
        }
        self.run_as(img, f, mark, true)
    }

    /// The edge runs its own steps, then the settle's.
    fn run_as(
        &self,
        img: &mut [u64],
        f: Fun,
        mark: Option<&[u64]>,
        guards: bool,
    ) -> Result<(), u32> {
        let mark = mark.filter(|m| m.iter().zip(&self.every[..]).any(|(m, e)| e & !m != 0));
        if matches!(f, Fun::Edge | Fun::Own) {
            for (ws, r, p, m) in &self.edge_parts {
                if ws.is_empty() || ws.iter().any(|&w| img[w as usize] != 0) {
                    // The reference (no guards) runs every body untracked.
                    let v = !guards || (*m != u32::MAX && img[*m as usize] != 0);
                    let (e, r) = if v && !p.is_empty() { (3, p) } else { (1, r) };
                    self.steps(img, &self.entries[e][r.clone()], None, guards)?;
                }
            }
        }
        match f {
            Fun::Init => self.steps(img, &self.entries[2], None, guards),
            Fun::Own => Ok(()),
            _ => self.steps(img, &self.entries[0], mark, guards),
        }
    }

    /// Every clocked body behind a run word (`run_words`) runs: the reference of the skip check
    /// and the second image of the rail (no `--two-state`).
    pub(crate) fn open(&self, img: &mut [u64]) {
        for &w in &self.run_words {
            img[w as usize] = 1;
        }
    }

    /// `YAVS_CYCLE_SKIP_CHECK=full`: `f` with every step and no guard on a copy first; the run
    /// with the mark and the guards must end the same way and leave the same state.
    #[cold]
    fn full(
        &self,
        img: &mut [u64],
        f: Fun,
        mark: Option<&[u64]>,
        state: &[(u32, u32)],
    ) -> Result<(), u32> {
        let mut whole = img.to_vec();
        self.open(&mut whole);
        let a = self.run_as(&mut whole, f, None, false);
        let b = self.run_as(img, f, mark, true);
        let differs = (state.iter()).any(|&(w, c)| {
            img[w as usize..(w + c) as usize] != whole[w as usize..(w + c) as usize]
        });
        if a != b || (b.is_ok() && differs) {
            self.stale.store(FULL, std::sync::atomic::Ordering::Relaxed);
            return Err(STALE);
        }
        b
    }

    fn steps(
        &self,
        img: &mut [u64],
        rs: &[Run],
        mark: Option<&[u64]>,
        guards: bool,
    ) -> Result<(), u32> {
        // The settle with a mark that holds a quarter of the steps or fewer: only those and the
        // steps without units, found by their bits (a wake reaching few steps walks few).
        if let (Some(m), Some(fx)) = (mark, &self.fixed)
            && !self.skip_check
            && rs.len() == self.settle_units.len()
            && 4 * (m.iter().zip(&fx[..]))
                .map(|(m, f)| (m | f).count_ones() as usize)
                .sum::<usize>()
                <= rs.len()
        {
            let mut i = 0;
            while i < rs.len() {
                let mut w = i / 64;
                let mut bits = (m[w] | fx[w]) & (!0u64 << (i % 64));
                while bits == 0 {
                    w += 1;
                    if w >= fx.len() {
                        return Ok(());
                    }
                    bits = m[w] | fx[w];
                }
                i = 64 * w + bits.trailing_zeros() as usize;
                if i >= rs.len() {
                    break;
                }
                i = self.one(img, rs, i, guards)?;
            }
            return Ok(());
        }
        let mut i = 0;
        while i < rs.len() {
            if let Some(m) = mark
                && m[i / 64] >> (i % 64) & 1 == 0
                && !self.settle_units[i].is_empty()
            {
                let n = match rs[i] {
                    Run::Guard(_, n, _) => n + 1,
                    _ => 1,
                };
                if self.skip_check {
                    self.unmoved(img, &rs[i..i + n], i)?;
                }
                i += n;
                continue;
            }
            i = self.one(img, rs, i, guards)?;
        }
        Ok(())
    }

    /// Step `i` of `rs`; the next step to look at.
    #[inline(always)]
    fn one(&self, img: &mut [u64], rs: &[Run], i: usize, guards: bool) -> Result<usize, u32> {
        match rs[i] {
            // SAFETY: the code reads and writes only words below `words`, checked in `run`.
            Run::Fn(e) => match unsafe { e(img.as_mut_ptr()) } {
                0 => {}
                s => return Err(s - 1),
            },
            Run::Save(g) => {
                let (ws, mut at) = (&self.groups[g].0, self.groups[g].1 as usize);
                for &(w, c) in ws {
                    copy_in(img, w as usize, at, c as usize);
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
            Run::Copy(c) => {
                for &(d, s, n) in &self.copies[c] {
                    copy_in(img, s as usize, d as usize, n as usize);
                }
            }
            Run::Same(g) => {
                let (ws, mut at) = (&self.groups[g].0, self.groups[g].1 as usize);
                for &(w, c) in ws {
                    let (w, c) = (w as usize, c as usize);
                    if !same(&img[w..w + c], &img[at..at + c]) {
                        return Err(UNSETTLED);
                    }
                    at += c;
                }
            }
            Run::Guard(g, n, checked) if guards => {
                let skip = self.guard(img, g, checked);
                if let Some(c) = &self.counts {
                    c[g][usize::from(skip)].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                if skip {
                    if self.skip_check {
                        self.recheck(img, g, &rs[i + 1..i + 1 + n])?;
                    }
                    return Ok(i + n + 1);
                }
            }
            Run::Guard(..) => {}
        }
        Ok(i + 1)
    }

    /// Whether guard `g`'s member may be skipped; if not, the inputs that changed are copied for
    /// the next time. Its flag word: what the last run was (bits 0 and 1: none, without checks,
    /// with), the span of inputs that differed last (bits 8 to 31, compared first), the runs in a
    /// row it did not skip (bits 32 to 39) and, after [`GUARD_MISSES`] of them, the runs left
    /// (bits 40 to 55) of [`GUARD_OFF`] it runs without a compare (an active member costs no
    /// more than a few instructions a run).
    #[inline(always)]
    fn guard(&self, img: &mut [u64], g: usize, checked: bool) -> bool {
        let gd = &self.guards[g];
        let (flag, n) = (gd.flag as usize, gd.ins.len());
        let f = img[flag];
        let off = f >> 40 & 0xffff;
        if off != 0 {
            // Runs, and its copy is not valid after (state none).
            img[flag] = (off - 1) << 40;
            return false;
        }
        let need = if checked { 2 } else { 1 };
        let (state, hint, misses) = (f & 3, (f >> 8 & 0xff_ffff) as usize, f >> 32 & 0xff);
        // Spans compared equal need no copy; from the first that differs on, every one is copied.
        let mut from = 0;
        if state >= need {
            loop {
                if from == n {
                    if misses != 0 {
                        img[flag] = f & !(0xff << 32);
                    }
                    return true;
                }
                let j = wrap(hint + from, n);
                let (w, c) = (gd.ins[j].0 as usize, gd.ins[j].1 as usize);
                let at = gd.copy as usize + gd.at[j] as usize;
                if !same(&img[w..w + c], &img[at..at + c]) {
                    break;
                }
                from += 1;
            }
        }
        if misses + 1 >= GUARD_MISSES {
            img[flag] = GUARD_OFF << 40;
            return false;
        }
        let first = if from < n { wrap(hint + from, n) } else { 0 };
        for i in from..n {
            let j = wrap(hint + i, n);
            let (w, c) = (gd.ins[j].0 as usize, gd.ins[j].1 as usize);
            let at = gd.copy as usize + gd.at[j] as usize;
            copy_in(img, w, at, c);
        }
        img[flag] = need | (first as u64) << 8 | (misses + 1) << 32;
        false
    }

    /// `YAVS_CYCLE_SKIP_CHECK=1`: steps of the settle skipped as outside what the wake's inputs
    /// reach run anyway and must leave every signal word as it was.
    #[cold]
    fn unmoved(&self, img: &mut [u64], rs: &[Run], at: usize) -> Result<(), u32> {
        // The words the skipped steps' members drive (all the signals' when not known).
        let all = [(0, self.signal_words as u32)];
        let spans: Vec<(u32, u32)> = match self.settle_outs.get(at..at + rs.len()) {
            Some(o) if !o.is_empty() => o.concat(),
            _ => all.to_vec(),
        };
        let words = |img: &[u64]| -> Vec<u64> {
            (spans.iter())
                .flat_map(|&(w, c)| img[w as usize..(w + c) as usize].to_vec())
                .collect()
        };
        let (before, logged) = (words(img), img[self.log_count]);
        self.steps(img, rs, None, true)?;
        img[self.log_count] = logged;
        if words(img) != before {
            self.stale
                .store(usize::MAX, std::sync::atomic::Ordering::Relaxed);
            return Err(STALE);
        }
        Ok(())
    }

    /// `YAVS_CYCLE_SKIP_CHECK=1`: the member a guard skipped runs anyway and must leave every word
    /// it drives as it was.
    #[cold]
    fn recheck(&self, img: &mut [u64], g: usize, rs: &[Run]) -> Result<(), u32> {
        let outs = &self.guards[g].outs;
        let before: Vec<u64> = (outs.iter())
            .flat_map(|&(w, c)| img[w as usize..(w + c) as usize].to_vec())
            .collect();
        let logged = img[self.log_count];
        self.steps(img, rs, None, true)?;
        img[self.log_count] = logged;
        let after: Vec<u64> = (outs.iter())
            .flat_map(|&(w, c)| img[w as usize..(w + c) as usize].to_vec())
            .collect();
        if before != after {
            self.stale.store(g, std::sync::atomic::Ordering::Relaxed);
            return Err(STALE);
        }
        Ok(())
    }
}

/// Runs in a row a guard did not skip before it stops comparing for [`GUARD_OFF`] runs.
const GUARD_MISSES: u64 = 16;
const GUARD_OFF: u64 = 256;

/// `i` taken back into `0 .. n` from below `2 * n` (no division).
#[inline(always)]
fn wrap(i: usize, n: usize) -> usize {
    if i >= n { i - n } else { i }
}

/// `a == b` for slices of one length, a loop for a few words (no call of `memcmp`).
#[inline(always)]
fn same(a: &[u64], b: &[u64]) -> bool {
    if a.len() <= 8 {
        a.iter().zip(b).all(|(x, y)| x == y)
    } else {
        a == b
    }
}

/// Words `from .. from + n` of `img` to `to` on, a loop for a few words (no call of `memmove`;
/// the ranges never overlap).
#[inline(always)]
fn copy_in(img: &mut [u64], from: usize, to: usize, n: usize) {
    if n <= 8 {
        for k in 0..n {
            img[to + k] = img[from + k];
        }
    } else {
        img.copy_within(from..from + n, to);
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
    /// The edge's own steps alone: the clocked bodies and the commit (the settle after it runs
    /// on its own, with the mark of what changed).
    Own,
    /// Once, before the first settle: initial values of the variables.
    Init,
}

/// The kind of the entries of compiled region functions in their store (`yavs_cache::CYCLE_DIR`).
const KIND: &str = "cycle";

// ponytail: one store per process, as the compiled tier's (a run simulates one design).
static STORE: Mutex<Option<Store>> = Mutex::new(None);

// ponytail: one budget per process, as the store; set before the region is compiled.
static THREADS: Mutex<(usize, Option<Arc<dyn yavs_kernel::Tokens>>)> = Mutex::new((1, None));

/// Compile the region's functions on up to `n` threads, those past the first only on tokens of
/// `tokens` when a test runner's token server lends them (`--threads`).
pub fn set_threads(n: usize, tokens: Option<Arc<dyn yavs_kernel::Tokens>>) {
    *THREADS.lock().unwrap_or_else(PoisonError::into_inner) = (n.max(1), tokens);
}

/// The threads to compile on, and the token server; developers: `YAVS_CYCLE_THREADS=N` instead
/// of `--threads`'s count.
fn threads() -> (usize, Option<Arc<dyn yavs_kernel::Tokens>>) {
    let (n, t) = THREADS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let env = std::env::var("YAVS_CYCLE_THREADS")
        .ok()
        .and_then(|v| v.parse().ok());
    (env.filter(|&n| n > 0).unwrap_or(n), t)
}

/// Keep the machine code of the region's functions in the compilation cache of library directory
/// `lib` from now on (`None`: compile every function).
pub fn set_cache(lib: Option<&Path>) {
    *STORE.lock().unwrap_or_else(PoisonError::into_inner) = lib.map(crate::store::cycle_store);
}

/// [`crate::store::warn_pressed`] for the store of the functions' code.
pub(crate) fn warn_pressed() {
    if let Some(s) = STORE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
    {
        crate::store::warn_pressed(s);
    }
}

/// The key of a function's machine code in the store: the yavs build ([`KeyBuilder`]), Cranelift's
/// version, the target with its flags (`isa`), `head` (the signature, checks, error and log words,
/// the register cap, a shared body's blocks) and every part's operations and live-out words. The
/// words' `keep` bits are not in it: the entry lists those the code generator read ([`entry`]).
fn fun_key(isa: &str, head: &[i64], parts: &[Part<'_>]) -> Option<Key> {
    let mut b = KeyBuilder::new("cycle-fn");
    b.text(cranelift_codegen::VERSION).text(isa);
    b.part(
        &head
            .iter()
            .flat_map(|x| x.to_le_bytes())
            .collect::<Vec<u8>>(),
    );
    // An empty part only empties the registers, so leading and trailing ones and all but the first
    // of a run change no code (a trigger function of nothing has one per member).
    let empty = |p: &Part<'_>| p.0.is_empty() && p.1.is_none();
    let (lo, hi) = (
        parts.iter().position(|p| !empty(p)).unwrap_or(parts.len()),
        parts.iter().rposition(|p| !empty(p)).map_or(0, |i| i + 1),
    );
    let mut last_empty = false;
    // One buffer for every part's bytes.
    let mut buf = Vec::new();
    for p @ (ops, later) in &parts[lo..hi.max(lo)] {
        if std::mem::replace(&mut last_empty, empty(p)) && empty(p) {
            continue;
        }
        buf.clear();
        buf = postcard::to_extend(&**ops, buf).ok()?;
        b.part(&buf);
        let mut w: Vec<i32> = later.iter().flat_map(|l| l.keys().copied()).collect();
        w.sort_unstable();
        w.insert(0, i32::from(later.is_some()));
        b.part(&w.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>());
    }
    Some(b.finish())
}

/// A function's entry: what of the error and log words it uses (`used`, [`Gen::err`],
/// [`Gen::log`]) with their places, the `keep` bits read (by byte offset), the alignment and the
/// code.
fn entry(seen: &Words<bool>, used: u8, at: (u32, (u32, u32)), align: u64, code: &[u8]) -> Vec<u8> {
    let mut v: Vec<(i32, bool)> = seen.iter().map(|(&o, &k)| (o, k)).collect();
    v.sort_unstable();
    let mut out = Vec::with_capacity(28 + 5 * v.len() + code.len());
    out.push(used);
    for x in [at.0, at.1.0, at.1.1] {
        out.extend(x.to_le_bytes());
    }
    out.extend((v.len() as u32).to_le_bytes());
    for (o, k) in v {
        out.extend(o.to_le_bytes());
        out.push(u8::from(k));
    }
    out.extend(align.to_le_bytes());
    out.extend(code);
    out
}

/// The alignment and code of entry `e` when every place it used and every `keep` bit it read is
/// the same here.
fn from_entry(e: &[u8], keep: &[bool], at: (u32, (u32, u32))) -> Option<(u64, Vec<u8>)> {
    let word = |i: usize| Some(u32::from_le_bytes(e.get(i..i + 4)?.try_into().ok()?));
    let used = *e.first()?;
    if (used & 1 != 0 && word(1)? != at.0) || (used & 2 != 0 && (word(5)?, word(9)?) != at.1) {
        return None;
    }
    let n = word(13)? as usize;
    let seen = e.get(17..17 + 5 * n)?;
    for r in seen.as_chunks::<5>().0 {
        let off = i32::from_le_bytes([r[0], r[1], r[2], r[3]]);
        if kept(keep, off) != (r[4] == 1) {
            return None;
        }
    }
    let rest = &e[17 + 5 * n..];
    let align = u64::from_le_bytes(rest.get(..8)?.try_into().ok()?);
    Some((align, rest[8..].to_vec()))
}

/// Whether word `off` (a byte offset) holds state between calls.
fn kept(keep: &[bool], off: i32) -> bool {
    keep.get(off as usize / 8).is_some_and(|&k| k)
}

/// For `n` blocks of a shared body, given every instance's table (the byte offset of each block):
/// per block the first block whose distance from it is the same in every instance, with that
/// distance in bytes (itself and 0 when none is).
fn base_blocks<'t>(
    n: usize,
    tables: impl Iterator<Item = &'t [usize]> + Clone,
) -> Vec<(usize, i32)> {
    let mut out: Vec<(usize, i32)> = Vec::with_capacity(n);
    for j in 0..n {
        let dist = |r: usize, t: &[usize]| i32::try_from(t[j] as i64 - t[r] as i64).ok();
        let found = (0..j).filter(|&r| out[r].0 == r).find_map(|r| {
            let mut ts = tables.clone();
            let d = dist(r, ts.next()?)?;
            ts.all(|t| dist(r, t) == Some(d)).then_some((r, d))
        });
        out.push(found.unwrap_or((j, 0)));
    }
    out
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

/// [`CHUNK`] without `--two-state`, where code has 3 to 4 times the operations: on 26 tests of an
/// accelerator 1937 cache entries and 25 s of compiling (4000: 2495, 24 s; 16000: 1662, 33 s).
const NINE_CHUNK: usize = 8000;

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

/// Operations per function: [`CHUNK`] ([`NINE_CHUNK`] without `--two-state`), or
/// `YAVS_CYCLE_CHUNK` (developers: a small value cuts every member into parts, to test the cut).
fn chunk() -> usize {
    static N: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    let env = *N.get_or_init(|| {
        std::env::var("YAVS_CYCLE_CHUNK")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|&n| n > 0)
    });
    env.unwrap_or(if crate::region::exact_meta() {
        NINE_CHUNK
    } else {
        CHUNK
    })
}

/// A map keyed by word offsets, with a cheap hash (the keys are the code generator's own).
type Words<V> = HashMap<i32, V, std::hash::BuildHasherDefault<WordHash>>;

#[derive(Default)]
struct WordHash(u64);

impl std::hash::Hasher for WordHash {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, b: &[u8]) {
        for &x in b {
            self.write_u64(u64::from(x) ^ self.0.rotate_left(8));
        }
    }

    fn write_u64(&mut self, n: u64) {
        let x = n.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        self.0 = x ^ (x >> 29);
    }

    fn write_i32(&mut self, n: i32) {
        self.write_u64(u64::from(n as u32));
    }
}

/// A cheap hash of composite keys the code generator makes itself (shapes, `lib.rs`): each word
/// mixed into the state, unlike [`WordHash`], which keeps the last one only.
#[derive(Default)]
pub(crate) struct KeyHash(u64);

impl std::hash::Hasher for KeyHash {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, b: &[u8]) {
        for c in b.chunks(8) {
            let mut w = [0u8; 8];
            w[..c.len()].copy_from_slice(c);
            self.write_u64(u64::from_le_bytes(w));
        }
    }

    fn write_u64(&mut self, n: u64) {
        self.0 = (self.0.rotate_left(5) ^ n).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }

    fn write_u8(&mut self, n: u8) {
        self.write_u64(u64::from(n));
    }

    fn write_u32(&mut self, n: u32) {
        self.write_u64(u64::from(n));
    }

    fn write_i64(&mut self, n: i64) {
        self.write_u64(n as u64);
    }

    fn write_usize(&mut self, n: usize) {
        self.write_u64(n as u64);
    }
}

pub(crate) type KeyBuild = std::hash::BuildHasherDefault<KeyHash>;

/// A part of a piece: its operations and, when its member goes on in the next part, the words
/// the later parts read (scratch words among them are live at the part's end).
type Part<'a> = (Cow<'a, [Op]>, Option<Arc<Words<()>>>);

/// Most words in registers at once: [`CACHE_MAX`], or fewer with `YAVS_CYCLE_REGS` (developers:
/// a small value flushes the registers every few words, to test the code that does it).
fn cache_cap() -> usize {
    static N: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *N.get_or_init(|| {
        std::env::var("YAVS_CYCLE_REGS")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|&n| n > 0)
            .unwrap_or(CACHE_MAX)
    })
}

/// A piece as parts of about `CHUNK` operations each; see [`cut`].
fn parts(ops: &[Op]) -> Vec<Part<'_>> {
    if weight(ops) <= chunk() {
        return vec![(Cow::Borrowed(ops), None)];
    }
    let mut ps = cut(ops, chunk());
    // Every part of a tracked member tracks its own stores (`Op::Track`).
    if let Some(&Op::Track { flag, .. }) = ops.first() {
        for p in &mut ps[1..] {
            p.insert(0, Op::Track { flag, first: false });
        }
    }
    let mut later: Option<Arc<Words<()>>> = None;
    let mut out: Vec<Part<'_>> = Vec::new();
    for p in ps.into_iter().rev() {
        let mut next = later.as_deref().cloned().unwrap_or_default();
        walk(&p, &mut 0, &mut |_, w| {
            next.insert(w, ());
        });
        out.push((Cow::Owned(p), later.take()));
        later = Some(Arc::new(next));
    }
    out.reverse();
    out
}

/// [`parts`] of operations made here.
fn owned_parts(ops: Vec<Op>) -> Vec<Part<'static>> {
    if weight(&ops) <= chunk() {
        return vec![(Cow::Owned(ops), None)];
    }
    parts(&ops)
        .into_iter()
        .map(|(p, l)| (Cow::Owned(p.into_owned()), l))
        .collect()
}

/// Every word `ops` read, with the operation reading it, numbered in pre-order from `pos` (an
/// `If`, then its `then` and `else` operations): the order [`Gen::op`] generates them in.
fn walk(ops: &[Op], pos: &mut u32, f: &mut impl FnMut(u32, i32)) {
    for op in ops {
        let i = *pos;
        *pos += 1;
        reads(op, &mut |off, n| {
            for k in 0..n {
                f(i, off + 8 * k as i32);
            }
        });
        if let Op::If { then, else_, .. } = op {
            walk(then, pos, f);
            walk(else_, pos, f);
        }
    }
}

/// The words the code of `op` reads (not those of an `If`'s branches): byte offset and words.
/// Every `Gen::ld` of the operation and every word it addresses at run time must be in it, or a
/// scratch word it reads is taken for dead before.
fn reads(op: &Op, f: &mut impl FnMut(i32, u32)) {
    // `n` words of `x` from its word `k0`.
    let mut r = |x: &Ref, k0: u32, n: u32| f(word(*x) + 8 * k0 as i32, n);
    match op {
        Op::Lit { .. } | Op::IntLit { .. } => {}
        Op::Copy { src, len, .. } => r(src, 0, words(*len)),
        Op::Ext { src, from, .. } => r(src, 0, words(*from)),
        Op::Blend { dst, src, mask } => {
            for (k, &m) in mask.iter().enumerate().filter(|m| *m.1 != 0) {
                r(src, k as u32, 1);
                if m != !0 {
                    r(dst, k as u32, 1);
                }
            }
        }
        Op::Gate { a, b, len, .. }
        | Op::Add { a, b, len, .. }
        | Op::Mul { a, b, len, .. }
        | Op::VecEq { a, b, len, .. }
        | Op::VCmp { a, b, len, .. } => {
            r(a, 0, words(*len));
            r(b, 0, words(*len));
        }
        Op::Not { a, len, .. } | Op::Reduce { a, len, .. } => r(a, 0, words(*len)),
        Op::Extract { src, lo, len, .. } => r(src, lo / 64, (lo + len - 1) / 64 - lo / 64 + 1),
        Op::Insert { dst, src, lo, len } => {
            r(src, 0, words(*len));
            r(dst, lo / 64, (lo + len - 1) / 64 - lo / 64 + 1);
        }
        Op::Cat {
            hi,
            lo,
            lo_len,
            len,
            ..
        } => {
            r(lo, 0, words(*lo_len));
            r(hi, 0, words(len - lo_len));
        }
        Op::VDiv { a, b, .. } | Op::IntCmp { a, b, .. } | Op::Int { a, b, .. } => {
            r(a, 0, 1);
            r(b, 0, 1);
        }
        Op::Shift { a, n, .. } => {
            r(a, 0, 1);
            r(n, 0, 1);
        }
        Op::Range { a, .. } | Op::ToInt { a, .. } | Op::FromInt { a, .. } | Op::Splat { a, .. } => {
            r(a, 0, 1)
        }
        Op::Select { c, a, b, len, .. } => {
            r(c, 0, 1);
            r(a, 0, words(*len));
            r(b, 0, words(*len));
        }
        // The array in memory, the word after it too (`Gen::spill`).
        Op::IndexRead {
            src,
            lo,
            n,
            stride,
            idx,
            ..
        } => {
            r(idx, 0, 1);
            r(src, lo / 64, (lo + n * stride).div_ceil(64) - lo / 64 + 1);
        }
        Op::IndexWrite {
            dst,
            src,
            lo,
            n,
            stride,
            w,
            idx,
            ..
        } => {
            r(src, 0, words(*w));
            r(idx, 0, 1);
            r(dst, lo / 64, (lo + n * stride).div_ceil(64) - lo / 64 + 1);
        }
        Op::Note { args, .. } => {
            for (a, n) in args {
                r(a, 0, *n);
            }
        }
        Op::If { cond, .. } => r(cond, 0, 1),
        // Its flag is not a word the operations read (`Gen::track_end`).
        Op::Track { .. } => {}
        Op::Nine { .. } => unreachable!("nine-valued operations are expanded before codegen"),
        Op::P4 {
            f,
            a,
            aw,
            b,
            bw,
            len,
            ..
        } => {
            let n = words(*len);
            for p in 0..4 {
                r(&plane(*a, *aw, p), 0, n);
                if matches!(f, P4F::Gate(_) | P4F::Eq { .. }) {
                    r(&plane(*b, *bw, p), 0, n);
                }
                if *f == P4F::Codes {
                    break;
                }
            }
        }
    }
}

/// Plane `p` of a four-plane value at `r` whose planes are `w` words apart (`Op::P4`).
fn plane(r: Ref, w: u32, p: u32) -> Ref {
    match r {
        Ref::Loc(s) => Ref::Loc(s + p * w),
        r => r,
    }
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

/// `ops` (one member's) with consecutive `If`s on equal conditions joined: an `If` whose condition
/// the operations just before it compute (its chain) the same way as the chain of the `If` before
/// it, from words that `If`'s branches do not write, joins that `If`'s branches and its chain
/// goes, when no other code reads what the chain wrote and none of it holds state. An unrolled
/// `for` loop testing one condition per element was a branch per element, each mispredicted
/// when the condition changed (a register file's write per bit).
fn join_ifs(ops: &[Op], keep: &[bool]) -> Vec<Op> {
    let mut reads: Words<u32> = Words::default();
    walk(ops, &mut 0, &mut |_, w| *reads.entry(w).or_default() += 1);
    let mut wrote: Words<u32> = Words::default();
    count_writes(ops, &mut wrote);
    join_level(ops.to_vec(), keep, &reads, &wrote)
}

/// How often an operation of `ops` may write each word (`span` words from its destination).
fn count_writes(ops: &[Op], wrote: &mut Words<u32>) {
    for op in ops {
        if let Op::If { then, else_, .. } = op {
            count_writes(then, wrote);
            count_writes(else_, wrote);
            continue;
        }
        let n = span(op) as i32;
        let d = std::cell::Cell::new(None);
        op.map(
            &|r| r,
            &|r| {
                d.set(Some(word(r)));
                r
            },
            &|x| x,
        );
        if let Some(d) = d.get() {
            for k in 0..n {
                *wrote.entry(d + 8 * k).or_default() += 1;
            }
        }
    }
}

/// The byte offset of the one word `op` writes, when it is a chain operation: no check, no
/// branch, no memory addressed at run time.
fn chain_dst(op: &Op) -> Option<i32> {
    let pure = matches!(
        op,
        Op::Copy { .. }
            | Op::Lit { .. }
            | Op::IntLit { .. }
            | Op::Gate { .. }
            | Op::Not { .. }
            | Op::Extract { .. }
            | Op::VecEq { .. }
            | Op::VCmp { .. }
            | Op::IntCmp { .. }
            | Op::Reduce { .. }
            | Op::ToInt { .. }
    );
    if !pure || span(op) != 1 {
        return None;
    }
    let d = std::cell::Cell::new(0);
    op.map(
        &|r| r,
        &|r| {
            d.set(word(r));
            r
        },
        &|x| x,
    );
    Some(d.get())
}

fn join_level(mut ops: Vec<Op>, keep: &[bool], reads: &Words<u32>, wrote: &Words<u32>) -> Vec<Op> {
    // Operations move to `out`: a clone per level of nested `If`s would be quadratic.
    let take = |o: &mut Op| {
        std::mem::replace(
            o,
            Op::IntLit {
                dst: Ref::Loc(0),
                v: 0,
            },
        )
    };
    let mut out: Vec<Op> = Vec::with_capacity(ops.len());
    // The `If` last put out, its chain (in `out`, before it) and the words the chain reads that
    // it did not write itself.
    let mut last: Option<(usize, usize, Vec<i32>)> = None;
    // The first `If` at or after each place.
    let mut next_if = vec![usize::MAX; ops.len() + 1];
    for k in (0..ops.len()).rev() {
        next_if[k] = if matches!(ops[k], Op::If { .. }) {
            k
        } else {
            next_if[k + 1]
        };
    }
    let mut i = 0;
    while i < ops.len() {
        // An `If` at `j` whose chain starts at `i`: the operations before it, each writing a word a
        // later one of them or the condition reads.
        let j = Some(next_if[i]).filter(|&j| {
            let Some(Op::If { cond, .. }) = ops.get(j) else {
                return false;
            };
            let mut need = vec![word(*cond)];
            // The words the chain's later operations write: an operation reading one of them or
            // its own (`a := a and en`) would read what an earlier run left.
            let mut later: Vec<i32> = Vec::new();
            let mut a = j;
            while a > i {
                match chain_dst(&ops[a - 1]) {
                    Some(d) if need.contains(&d) => {
                        later.push(d);
                        let mut r = Vec::new();
                        reads_of(&ops[a - 1], &mut r);
                        if r.iter().any(|w| later.contains(w)) {
                            break;
                        }
                        need.extend(r);
                        a -= 1;
                    }
                    _ => break,
                }
            }
            a == i
        });
        let (Some(j), Some((at, n, ext))) = (j, last.as_ref()) else {
            // Not a chain and an `If`, or no `If` before it: put out as it is.
            if let Some(j) = j {
                let Op::If { cond, .. } = &ops[j] else {
                    unreachable!()
                };
                let ext = chain_ext(&ops[i..j], word(*cond));
                out.extend(ops[i..=j].iter_mut().map(take));
                last = Some((out.len() - 1, j - i, ext));
                i = j + 1;
            } else {
                out.push(take(&mut ops[i]));
                last = None;
                i += 1;
            }
            continue;
        };
        let (at, n) = (*at, *n);
        let joined = (|| {
            if j - i != n {
                return None;
            }
            let Op::If {
                cond: c0,
                then: t0,
                else_: e0,
            } = &out[at]
            else {
                unreachable!()
            };
            let Op::If { cond, then, else_ } = &ops[j] else {
                unreachable!()
            };
            // The words the earlier chain wrote, and this chain's in the same places.
            let old: Vec<i32> = out[at - n..at]
                .iter()
                .map(|o| chain_dst(o).unwrap())
                .collect();
            let new: Vec<i32> = ops[i..j].iter().map(|o| chain_dst(o).unwrap()).collect();
            let to: Words<i32> = new.iter().copied().zip(old.iter().copied()).collect();
            let back = |r: Ref| match to.get(&word(r)) {
                Some(&o) => Ref::Loc(o as u32 / 8),
                None => r,
            };
            let same = (out[at - n..at].iter().zip(&ops[i..j]))
                .all(|(a, b)| &b.map(&back, &back, &|x| x) == a)
                && back(*cond) == *c0;
            let clean = |body: &[Op], ws: &[i32]| ws.iter().all(|&w| !writes(body, w / 8));
            let walk_count = |body: &[Op], w: i32| {
                let mut c = 0;
                walk(body, &mut 0, &mut |_, x| c += u32::from(x == w));
                c
            };
            // Every read of what this chain writes is in it or in its `If`.
            let only_here = (new.iter()).all(|&w| {
                !kept(keep, w) && reads.get(&w).copied().unwrap_or(0) == walk_count(&ops[i..=j], w)
            });
            // Each chain word is written once in the member: by its chain alone.
            let once = old.iter().chain(&new).all(|w| wrote.get(w) == Some(&1));
            let ok = same
                && once
                && only_here
                && clean(t0, ext)
                && clean(e0, ext)
                && clean(t0, &old)
                && clean(e0, &old)
                && clean(then, &new)
                && clean(else_, &new);
            ok.then(|| {
                let m = |b: &[Op]| {
                    b.iter()
                        .map(|o| o.map(&back, &back, &|x| x))
                        .collect::<Vec<_>>()
                };
                (m(then), m(else_))
            })
        })();
        match joined {
            Some((t, e)) => {
                if let Op::If { then, else_, .. } = &mut out[at] {
                    then.extend(t);
                    else_.extend(e);
                }
                i = j + 1;
            }
            None => last = None,
        }
    }
    // Then inside the branches.
    for op in &mut out {
        if let Op::If { then, else_, .. } = op {
            *then = join_level(std::mem::take(then), keep, reads, wrote);
            *else_ = join_level(std::mem::take(else_), keep, reads, wrote);
        }
    }
    out
}

/// The words `op` reads (`reads`), added to `need`.
fn reads_of(op: &Op, need: &mut Vec<i32>) {
    reads(op, &mut |off, n| {
        need.extend((0..n as i32).map(|k| off + 8 * k))
    });
}

/// The words a chain (and the condition `cond`) reads that the chain did not write before.
fn chain_ext(chain: &[Op], cond: i32) -> Vec<i32> {
    let mut made: Vec<i32> = Vec::new();
    let mut ext = Vec::new();
    for op in chain {
        let mut r = Vec::new();
        reads_of(op, &mut r);
        ext.extend(r.into_iter().filter(|w| !made.contains(w)));
        made.extend(chain_dst(op));
    }
    if !made.contains(&cond) {
        ext.push(cond);
    }
    ext
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
        | Op::Select { len, .. }
        | Op::Nine { len, .. } => words(*len),
        Op::P4 { dw: 0, .. } => 1,
        Op::P4 { dw, len, .. } => 3 * dw + words(*len),
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
        Op::Range { .. } | Op::If { .. } | Op::Note { .. } | Op::Track { .. } => 0,
    }
}

/// The target of the region's code (the host, with `YAVS_CYCLE_FLAGS`) and a module for it, with
/// the target's name: its triple and flags, the key of the code in the cache.
fn module() -> Result<(JITModule, String, cranelift_codegen::isa::OwnedTargetIsa), String> {
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
    let isa_id = (isa.isa_flags().iter())
        .fold(format!("{} {}", isa.triple(), isa.flags()), |a, v| {
            format!("{a} {v}")
        });
    let jb = JITBuilder::with_isa(isa.clone(), cranelift_module::default_libcall_names());
    Ok((JITModule::new(jb), isa_id, isa))
}

/// The name of the target the region's code is made for ([`module`]).
pub(crate) fn isa_id() -> Option<String> {
    static ID: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    ID.get_or_init(|| module().ok().map(|m| m.1)).clone()
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
    units: &[u32],
    edge: &'a [Step],
    plain: Option<Untracked<'a>>,
    init: &'a [Step],
    bodies: &'a [(Vec<Op>, bool)],
    shapes: &'a [Shape],
    groups: Vec<(Vec<(u32, u32)>, u32)>,
    guards: Vec<Guard>,
    keep: &[bool],
    signal_words: usize,
    err: u32,
    log: (u32, u32),
    name: &str,
) -> Result<(Code, CodePlan, Option<crate::store::Blob>), String> {
    let words = keep.len();
    let (mut module, isa, target) = module()?;
    let checks = checks_on();
    let vals = crate::region::exact_meta();
    // The functions to compile (label, parts, whether checks exit), the loop bodies first; and
    // what each trigger function runs, a function by its index. A part says whether its member
    // goes on in the next one (`cut`).
    // A function: label, parts, whether checks exit, the shape it is a body of.
    type Fn<'a> = (String, Vec<Part<'a>>, bool, Option<usize>);
    let mut funs: Vec<Fn<'_>> = Vec::new();
    // Parts packed into functions of about `CHUNK` operations; returns the functions made.
    let pack = |funs: &mut Vec<Fn<'a>>,
                what: &str,
                ps: Vec<Part<'a>>,
                checked: bool,
                shape: Option<usize>| {
        let first = funs.len();
        let mut cur: Vec<Part<'a>> = Vec::new();
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
    let mut tables: Vec<Box<[usize]>> = Vec::new();
    let mut copies: Vec<Vec<(u32, u32, u32)>> = Vec::new();
    let mut runs: [Vec<Pend>; 4] = Default::default();
    let mut settle_units: Vec<Vec<u32>> = Vec::new();
    // The edge's parts of each variant: words and range.
    let mut parts_of: [Vec<(Vec<u32>, std::ops::Range<usize>)>; 2] = Default::default();
    let (plain_steps, modes, mode0) = plain.unwrap_or((&[], &[], u32::MAX));
    for (f, (what, steps)) in [
        ("settle", settle),
        ("edge", edge),
        ("init", init),
        ("plain", plain_steps),
    ]
    .into_iter()
    .enumerate()
    {
        let mut cur: Vec<Part<'a>> = Vec::new();
        // The settle's units (`compile`'s `units`) of the parts in `cur`, and of each step made.
        let mut cur_units: Vec<u32> = Vec::new();
        let flush = |funs: &mut Vec<Fn<'a>>,
                     cur: &mut Vec<Part<'a>>,
                     cur_units: &mut Vec<u32>,
                     runs: &mut Vec<Pend>,
                     eunits: &mut Vec<Vec<u32>>| {
            if !cur.is_empty() {
                let made = pack(funs, what, std::mem::take(cur), true, None);
                cur_units.sort_unstable();
                cur_units.dedup();
                for k in made {
                    runs.push(Pend::Fn(k));
                    eunits.push(cur_units.clone());
                }
            }
            cur_units.clear();
        };
        let mut eunits: Vec<Vec<u32>> = Vec::new();
        // The edge's gates: each one's words and the first step after it.
        let mut gates: Vec<(&[u32], usize)> = vec![(&[], 0)];
        // The guards open: each one's place, what it is and the steps it still covers.
        let mut open: Vec<(usize, usize, bool, usize)> = Vec::new();
        for (si, st) in steps.iter().enumerate() {
            let unit: Vec<u32> = match units.get(si) {
                Some(&u) if f == 0 && u != u32::MAX => vec![u],
                _ => Vec::new(),
            };
            match st {
                Step::Ops(p) => {
                    cur.extend(owned_parts(join_ifs(p, keep)));
                    cur_units.extend(&unit);
                }
                Step::Call(b) => {
                    flush(
                        &mut funs,
                        &mut cur,
                        &mut cur_units,
                        &mut runs[f],
                        &mut eunits,
                    );
                    for k in bodies[*b].clone() {
                        runs[f].push(Pend::Fn(k));
                        eunits.push(unit.clone());
                    }
                }
                Step::Shared {
                    shape,
                    table,
                    sites,
                } => {
                    flush(
                        &mut funs,
                        &mut cur,
                        &mut cur_units,
                        &mut runs[f],
                        &mut eunits,
                    );
                    let blocks = &shapes[*shape].blocks;
                    if table.len() != blocks.len()
                        || (table.iter().zip(blocks)).any(|(&w, &(_, n))| (w + n) as usize > words)
                    {
                        return Err("a shared body's table does not fit its image".into());
                    }
                    let t = tables.len() as u32;
                    tables.push(table.iter().map(|&w| w as usize * 8).collect());
                    for k in shaped[*shape].clone() {
                        runs[f].push(Pend::Shared(k, t, *sites));
                        eunits.push(unit.clone());
                    }
                }
                Step::Guard(g, checked, n) => {
                    if *n == 0 || open.last().is_some_and(|o| o.3 < *n) {
                        return Err("a guard that does not nest in the one around it".into());
                    }
                    flush(
                        &mut funs,
                        &mut cur,
                        &mut cur_units,
                        &mut runs[f],
                        &mut eunits,
                    );
                    open.push((runs[f].len(), *g, *checked, *n));
                    runs[f].push(Pend::Guard(*g, 0, *checked));
                    eunits.push(Vec::new());
                    continue;
                }
                Step::Gate(w) => {
                    flush(
                        &mut funs,
                        &mut cur,
                        &mut cur_units,
                        &mut runs[f],
                        &mut eunits,
                    );
                    gates.push((w, runs[f].len()));
                    continue;
                }
                Step::Save(g) => {
                    flush(
                        &mut funs,
                        &mut cur,
                        &mut cur_units,
                        &mut runs[f],
                        &mut eunits,
                    );
                    runs[f].push(Pend::Save(*g));
                    eunits.push(Vec::new());
                }
                Step::Same(g) => {
                    flush(
                        &mut funs,
                        &mut cur,
                        &mut cur_units,
                        &mut runs[f],
                        &mut eunits,
                    );
                    runs[f].push(Pend::Same(*g));
                    eunits.push(Vec::new());
                }
                Step::Copy(c) => {
                    flush(
                        &mut funs,
                        &mut cur,
                        &mut cur_units,
                        &mut runs[f],
                        &mut eunits,
                    );
                    if (c.iter()).any(|&(d, s, n)| (d.max(s) + n) as usize > words) {
                        return Err("a copy's words do not fit the image".into());
                    }
                    runs[f].push(Pend::Copy(copies.len()));
                    copies.push(c.clone());
                    eunits.push(Vec::new());
                }
            }
            // A guarded step in functions of its own: the guard skips them all.
            for o in &mut open {
                o.3 -= 1;
            }
            while let Some(&(at, g, checked, 0)) = open.last() {
                flush(
                    &mut funs,
                    &mut cur,
                    &mut cur_units,
                    &mut runs[f],
                    &mut eunits,
                );
                let n = runs[f].len() - at - 1;
                runs[f][at] = Pend::Guard(g, n, checked);
                // A guard is in the settle's units of what it covers.
                let mut u: Vec<u32> = eunits[at + 1..].concat();
                u.sort_unstable();
                u.dedup();
                eunits[at] = u;
                open.pop();
            }
        }
        if !open.is_empty() {
            return Err("a guard with fewer steps after it than it covers".into());
        }
        flush(
            &mut funs,
            &mut cur,
            &mut cur_units,
            &mut runs[f],
            &mut eunits,
        );
        if f == 0 {
            settle_units = eunits;
        }
        if f == 1 || f == 3 {
            let n = runs[f].len();
            parts_of[f / 2] = (gates.iter().enumerate())
                .map(|(k, &(w, a))| (w.to_vec(), a..gates.get(k + 1).map_or(n, |g| g.1)))
                .collect();
        }
    }
    // A part runs untracked when its class's mode word is 1 (a part of no class or of several
    // classes holds no body whose variants differ: the tracked one).
    let [tracked, untracked] = parts_of;
    if plain.is_some()
        && (tracked.len() != untracked.len()
            || (tracked.iter().zip(&untracked)).any(|(a, b)| a.0 != b.0))
    {
        return Err("the edge's untracked variant has other gates".into());
    }
    let edge_parts: Vec<EdgePart> = (tracked.into_iter().enumerate())
        .map(|(k, (w, r))| {
            let mode = match &w[..] {
                _ if plain.is_none() => u32::MAX,
                [] => mode0,
                [x] => modes.iter().find(|m| m.0 == *x).map_or(u32::MAX, |m| m.1),
                _ => u32::MAX,
            };
            let p = untracked.get(k).map_or(0..0, |u| u.1.clone());
            (w, r, p, mode)
        })
        .filter(|x| !x.1.is_empty())
        .collect();
    // Per shape, per block: the block whose base it is addressed from and the distance to it, the
    // same in every instance (an instance's signals lie in one order: a body needs few bases).
    let mut inst: Vec<Vec<usize>> = vec![Vec::new(); shapes.len()];
    for r in runs.iter().flatten() {
        if let Pend::Shared(k, t, _) = *r
            && let Some(sh) = funs[k].3
            && !inst[sh].contains(&(t as usize))
        {
            inst[sh].push(t as usize);
        }
    }
    let based: Vec<Vec<(usize, i32)>> = (inst.iter().enumerate())
        .map(|(sh, ts)| base_blocks(shapes[sh].blocks.len(), ts.iter().map(|&t| &tables[t][..])))
        .collect();
    // Developers: `YAVS_CYCLE_STATS=1` prints each function's operations, Cranelift instructions,
    // machine code bytes and compile time.
    let stats = std::env::var_os("YAVS_CYCLE_STATS").is_some();
    if stats {
        for (sh, b) in based.iter().enumerate() {
            let n = (b.iter().enumerate()).filter(|&(j, x)| x.0 == j).count();
            eprintln!(
                "cycle stats: shape {sh}: {} blocks from {n} bases, {} instances",
                b.len(),
                inst[sh].len()
            );
        }
    }
    // Developers: `YAVS_CYCLE_DISASM=1` prints the machine code of each function (compiled, never
    // taken from the cache).
    let disasm = std::env::var_os("YAVS_CYCLE_DISASM").is_some();
    // `YAVS_CYCLE_CACHE_CHECK=1`: every function compiled, and its entry must hold that code.
    let cache_check = std::env::var_os("YAVS_CYCLE_CACHE_CHECK").is_some_and(|v| v == "1");
    // More functions than the store holds can never all be in it, so a rerun is no warmer: it
    // would only write entries that push out other designs', and pay a file per function.
    let store = STORE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .filter(|s| funs.len() <= s.max_entries());
    let mut hits = 0;
    let t0 = std::time::Instant::now();
    let mut ids = Vec::new();
    let (blob, check) = crate::store::code_get();
    let take = blob.filter(|b| b.len() == funs.len() && !check && !disasm);
    // What this compile made, for the cache (none when a function needs relocations).
    let mut made: Option<crate::store::Blob> = Some(Vec::new());
    let plan = CodePlan {
        funs: (funs.iter())
            .map(|f| (format!("cycle_{}_{name}", f.0), f.3.is_some()))
            .collect(),
        runs,
        edge_parts,
        groups,
        copies,
        words,
        tables,
        guards,
        settle_units,
        signal_words,
        log_count: log.0 as usize,
        full_check: (skip_check()? == SkipCheck::Full).then(|| {
            let mut v: Vec<(u32, u32)> = Vec::new();
            for (w, _) in keep.iter().enumerate().filter(|x| *x.1) {
                match v.last_mut() {
                    Some(x) if x.0 + x.1 == w as u32 => x.1 += 1,
                    _ => v.push((w as u32, 1)),
                }
            }
            v
        }),
        settle_outs: Vec::new(),
        run_words: Vec::new(),
    };
    // Three passes: the cache in function order; the functions it does not have built and
    // compiled on up to `threads()` threads; the code defined in function order, so the module,
    // the cache's entries and the output are those of one thread.
    let mut slots: Vec<Slot> = Vec::with_capacity(funs.len());
    for (k, (_, ops, checked, shape)) in funs.iter().enumerate() {
        let t = std::time::Instant::now();
        let (id, sym, sig) = declare(&mut module, &plan.funs[k])?;
        if let Some(b) = &take {
            let (align, bytes) = b[k].clone();
            slots.push(Slot::Blob(id, sym, align, bytes));
            continue;
        }
        let fkeep = shape.map_or(keep, |k| &shapes[k].keep[..]);
        let key = store.as_ref().filter(|_| !disasm).and_then(|_| {
            let mut head = vec![
                i64::from(shape.is_some()),
                i64::from(checks && *checked),
                i64::from(*checked),
                i64::from(vals),
                cache_cap() as i64,
            ];
            if let Some(k) = shape {
                for (&(v, n), &(r, d)) in shapes[*k].blocks.iter().zip(&based[*k]) {
                    head.extend([i64::from(v), i64::from(n), r as i64, i64::from(d)]);
                }
            }
            fun_key(&isa, &head, ops)
        });
        let cached = (store.as_ref().zip(key.as_ref()))
            .and_then(|(s, key)| s.get(KIND, key))
            .and_then(|e| from_entry(&e, fkeep, (err, log)));
        slots.push(match cached {
            Some((align, bytes)) if !cache_check => {
                Slot::Hit(id, sym, align, bytes, t.elapsed().as_micros())
            }
            cached => Slot::Miss(id, sym, sig, key, cached),
        });
    }
    let (want, tokens) = threads();
    let mut order: Vec<usize> = (0..funs.len())
        .filter(|&k| matches!(slots[k], Slot::Miss(..)))
        .collect();
    let want = want.min(order.len()).saturating_sub(1);
    // On several threads the largest first: the last one to finish bounds the time. On one, in
    // function order: largest first measured 0.4 percent more instructions there.
    if want > 0 {
        order.sort_by_key(|&k| {
            std::cmp::Reverse(funs[k].1.iter().map(|(p, _)| weight(p)).sum::<usize>())
        });
    }
    let env = Env {
        isa: &*target,
        shapes,
        based: &based,
        keep,
        err,
        log,
        checks,
        vals,
        disasm,
    };
    let done: Vec<std::sync::OnceLock<Result<Made, String>>> =
        (0..funs.len()).map(|_| Default::default()).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let one = |cx: &mut (cranelift_codegen::Context, FunctionBuilderContext)| {
        let k = *order.get(next.fetch_add(1, std::sync::atomic::Ordering::Relaxed))?;
        if let Slot::Miss(id, _, sig, ..) = &slots[k] {
            let (_, ops, checked, shape) = &funs[k];
            let _ = done[k].set(env.make(cx, sig.clone(), *id, ops, *checked, *shape));
        }
        Some(())
    };
    let new_cx = || {
        (
            cranelift_codegen::Context::new(),
            FunctionBuilderContext::new(),
        )
    };
    let helpers = std::thread::scope(|s| {
        let mut helpers = 0;
        let mut cx = new_cx();
        loop {
            // A token server lends tokens when it can: asked again before each function here.
            if helpers < want {
                let n = tokens.as_ref().map_or(want, |t| t.take(want));
                // The main thread's stack size: a member's nested code recurses as deep there.
                for _ in helpers..n {
                    // A thread the system refuses only leaves the work to the others.
                    let _ = std::thread::Builder::new()
                        .name("yavs-cycle-compile".into())
                        .stack_size(8 << 20)
                        .spawn_scoped(s, || {
                            let mut cx = new_cx();
                            while one(&mut cx).is_some() {}
                        });
                }
                helpers = helpers.max(n);
            }
            if one(&mut cx).is_none() {
                break helpers;
            }
        }
    });
    if stats && !order.is_empty() {
        eprintln!("cycle stats: compiled on {} threads", helpers + 1);
    }
    if want > 0
        && let Some(t) = &tokens
    {
        t.give();
    }
    for (k, (slot, done)) in slots.into_iter().zip(done).enumerate() {
        let ops = &funs[k].1;
        match slot {
            Slot::Blob(id, sym, align, bytes) => {
                module
                    .define_function_bytes(id, align, &bytes, &[])
                    .map_err(|e| format!("{e:?}"))?;
                hits += 1;
                ids.push((id, sym, bytes.len() as u32));
            }
            Slot::Hit(id, sym, align, bytes, us) => {
                module
                    .define_function_bytes(id, align, &bytes, &[])
                    .map_err(|e| format!("{e:?}"))?;
                hits += 1;
                if stats {
                    eprintln!(
                        "cycle stats: {sym}: {} operations, {} bytes, {us} us, from the cache",
                        ops.iter().map(|(p, _)| weight(p)).sum::<usize>(),
                        bytes.len(),
                    );
                }
                ids.push((id, sym, bytes.len() as u32));
                if let Some(m) = made.as_mut() {
                    m.push((align, bytes));
                }
            }
            Slot::Miss(id, sym, _, key, cached) => {
                let m = done
                    .into_inner()
                    .ok_or("a region function was not compiled")??;
                module
                    .define_function_bytes(id, m.align, &m.code, &m.relocs)
                    .map_err(|e| format!("{e:?}"))?;
                // A function with relocations is not kept: its code alone does not link.
                let relocs = !m.relocs.is_empty();
                if relocs {
                    made = None;
                } else if let Some(made) = made.as_mut() {
                    made.push((m.align, m.code.clone()));
                }
                if let Some((a, b)) = &cached
                    && (*a, &b[..]) != (m.align, &m.code[..])
                {
                    return Err(format!(
                        "YAVS_CYCLE_CACHE_CHECK: the machine code of {sym} in the cache differs from the one compiled now"
                    ));
                }
                if let (Some(s), Some(key), false) = (&store, &key, relocs) {
                    s.put(
                        KIND,
                        key,
                        &entry(&m.seen, m.used, (err, log), m.align, &m.code),
                    );
                }
                if let Some(text) = &m.text {
                    eprintln!("{sym}:\n{text}");
                }
                if stats {
                    eprintln!(
                        "cycle stats: {sym}: {} operations, {} instructions, {} bytes, {} us",
                        ops.iter().map(|(p, _)| weight(p)).sum::<usize>(),
                        m.n_ir,
                        m.size,
                        m.us,
                    );
                }
                ids.push((id, sym, m.size));
            }
        }
    }
    module.finalize_definitions().map_err(|e| e.to_string())?;
    if stats {
        eprintln!(
            "cycle stats: {} functions in {} ms, {hits} from the cache",
            ids.len(),
            t0.elapsed().as_millis()
        );
    }
    let code = finish(module, ids, plan.clone())?;
    Ok((code, plan, made.filter(|_| take.is_none())))
}

/// What [`Env::make`] made of one function.
struct Made {
    n_ir: usize,
    align: u64,
    code: Vec<u8>,
    relocs: Vec<ModuleReloc>,
    seen: Words<bool>,
    used: u8,
    /// `YAVS_CYCLE_DISASM`: the machine code as text.
    text: Option<String>,
    size: u32,
    us: u128,
}

/// A function of [`compile`] after the look into the cache: from the region's code in the cache
/// (alignment, bytes), from its own entry (also the time it took), or to compile (its signature,
/// key and, under `YAVS_CYCLE_CACHE_CHECK`, the entry's code).
enum Slot {
    Blob(FuncId, String, u64, Vec<u8>),
    Hit(FuncId, String, u64, Vec<u8>, u128),
    Miss(
        FuncId,
        String,
        cranelift_codegen::ir::Signature,
        Option<Key>,
        Option<(u64, Vec<u8>)>,
    ),
}

/// What every function of one [`compile`] is made with.
struct Env<'a> {
    isa: &'a dyn cranelift_codegen::isa::TargetIsa,
    shapes: &'a [Shape],
    based: &'a [Vec<(usize, i32)>],
    keep: &'a [bool],
    err: u32,
    log: (u32, u32),
    checks: bool,
    /// Failed checks store their operands (no `--two-state`).
    vals: bool,
    disasm: bool,
}

impl Env<'_> {
    /// Build and compile function `id` of parts `ops` in `cx` (its allocations kept from one
    /// function to the next): the same code on any thread.
    fn make(
        &self,
        cx: &mut (cranelift_codegen::Context, FunctionBuilderContext),
        sig: cranelift_codegen::ir::Signature,
        id: FuncId,
        ops: &[Part<'_>],
        checked: bool,
        shape: Option<usize>,
    ) -> Result<Made, String> {
        let t = std::time::Instant::now();
        let (ctx, fbc) = cx;
        let ptr = self.isa.pointer_type();
        let fkeep = shape.map_or(self.keep, |k| &self.shapes[k].keep[..]);
        ctx.clear();
        ctx.func.signature = sig;
        let seen: (Words<bool>, u8);
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, fbc);
            let blk = b.create_block();
            b.append_block_params_for_function_params(blk);
            b.switch_to_block(blk);
            b.seal_block(blk);
            let base = b.block_params(blk)[0];
            // A shared body: each block's address, from the instance's table.
            let mut blocks = Vec::new();
            if let Some(k) = shape {
                let tbl = b.block_params(blk)[1];
                let mut at: Vec<Value> = Vec::new();
                for (j, &(v, _)) in self.shapes[k].blocks.iter().enumerate() {
                    let (r, d) = self.based[k][j];
                    let a = if r == j {
                        let off = b
                            .ins()
                            .load(ptr, MemFlagsData::trusted(), tbl, 8 * j as i32);
                        b.ins().iadd(base, off)
                    } else {
                        at[r]
                    };
                    at.push(a);
                    blocks.push((v as i32 * 8, a, d));
                }
            }
            let mut g = Gen {
                b,
                base,
                err: self.err as i32 * 8,
                log: (self.log.0 as i32 * 8, self.log.1),
                checks: self.checks && checked,
                exits: checked,
                vals: self.vals,
                exit: None,
                ptr,
                cache: BTreeMap::new(),
                cap: CACHE_MAX,
                blocks,
                keep: fkeep,
                seen: Default::default(),
                used: Default::default(),
                zeros: std::collections::HashMap::new(),
                fields: SecondaryMap::new(),
                born: SecondaryMap::new(),
                piece: 0,
                built: SecondaryMap::new(),
                pos: 0,
                next: 0,
                last: Words::default(),
                later: None,
                track: None,
                #[cfg(debug_assertions)]
                gone: Default::default(),
            };
            // A piece starts from memory and the words the piece before left in registers that
            // it reads (memory holds them too): one function runs its pieces one after the other.
            for (p, more) in ops {
                g.cap = if weight(p) > BIG {
                    CACHE_BIG
                } else {
                    CACHE_MAX
                }
                .min(cache_cap());
                g.live_in(p, more.clone());
                let reads = &g.last;
                g.cache.retain(|off, _| reads.contains_key(off));
                for op in p.iter() {
                    g.op(op);
                }
                // A member that goes on in the next part leaves its scratch words there too.
                if more.is_some() {
                    g.spill_all();
                } else {
                    g.write_back();
                }
                g.track_end();
                // What is in memory now: state, and what the next part of the member reads; a
                // value is handed on through at most `CARRY` pieces.
                g.piece += 1;
                g.cache = (g.cache.iter())
                    .filter(|(o, (v, _))| g.state(**o) && g.piece - g.born[*v] < CARRY)
                    .map(|(&o, &(v, _))| (o, (v, false)))
                    .collect();
            }
            let zero = g.b.ins().iconst(I32, 0);
            g.b.ins().return_(&[zero]);
            if let Some(x) = g.exit {
                g.b.switch_to_block(x);
                g.b.seal_block(x);
                let s = g.b.block_params(x)[0];
                g.b.ins().return_(&[s]);
            }
            g.b.finalize(self.isa.frontend_config());
            seen = (g.seen.into_inner(), g.used.get());
        }
        ctx.set_disasm(self.disasm);
        let n_ir = ctx.func.dfg.num_insts();
        ctx.compile(self.isa, &mut Default::default())
            .map_err(|e| format!("{:?}", e.inner))?;
        let c = ctx.compiled_code().ok_or("no compiled code")?;
        let relocs = (c.buffer.relocs().iter())
            .map(|r| ModuleReloc::from_mach_reloc(r, &ctx.func, id))
            .collect();
        Ok(Made {
            n_ir,
            align: u64::from(c.buffer.min_alignment),
            code: c.code_buffer().to_vec(),
            relocs,
            seen: seen.0,
            used: seen.1,
            text: c.vcode.clone().filter(|_| self.disasm),
            size: c.code_info().total_size,
            us: t.elapsed().as_micros(),
        })
    }
}

/// Declare function `f` of a plan (its symbol, whether it is a shape's body) in `module`.
fn declare(
    module: &mut JITModule,
    f: &(String, bool),
) -> Result<(FuncId, String, cranelift_codegen::ir::Signature), String> {
    let ptr = module.target_config().pointer_type();
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(ptr));
    if f.1 {
        sig.params.push(AbiParam::new(ptr));
    }
    sig.returns.push(AbiParam::new(I32));
    let id = module
        .declare_function(&f.0, Linkage::Local, &sig)
        .map_err(|e| e.to_string())?;
    Ok((id, f.0.clone(), sig))
}

/// The region's functions of `plan` from their machine code `blob` (the cache): no function body
/// is built.
pub(crate) fn from_blob(plan: CodePlan, blob: &crate::store::Blob) -> Result<Code, String> {
    if blob.len() != plan.funs.len() {
        return Err("the region's code in the cache does not fit its plan".into());
    }
    let t0 = std::time::Instant::now();
    let (mut module, ..) = module()?;
    let mut ids = Vec::new();
    for (f, (align, bytes)) in plan.funs.iter().zip(blob) {
        let (id, sym, _) = declare(&mut module, f)?;
        module
            .define_function_bytes(id, *align, bytes, &[])
            .map_err(|e| format!("{e:?}"))?;
        ids.push((id, sym, bytes.len() as u32));
    }
    module.finalize_definitions().map_err(|e| e.to_string())?;
    if std::env::var_os("YAVS_CYCLE_STATS").is_some() {
        eprintln!(
            "cycle stats: {} functions in {} ms, {} from the cache",
            ids.len(),
            t0.elapsed().as_millis(),
            ids.len()
        );
    }
    finish(module, ids, plan)
}

/// The code of `plan` once its functions `ids` (with their symbols and sizes) are in `module`.
fn finish(
    module: JITModule,
    ids: Vec<(FuncId, String, u32)>,
    plan: CodePlan,
) -> Result<Code, String> {
    let words = plan.words;
    let addr = |id: FuncId| module.get_finalized_function(id);
    if std::env::var_os("YAVS_CLIFT_PERFMAP").is_some() {
        for (id, sym, bytes) in &ids {
            yavs_rt::perfmap::append(addr(*id) as usize, *bytes as usize, sym);
        }
    }
    let fun = |k: usize| -> Result<*const u8, String> {
        Ok(addr(
            ids.get(k)
                .ok_or("a step names a function the plan lacks")?
                .0,
        ))
    };
    let mut entries: [Vec<Run>; 4] = Default::default();
    for (f, rs) in plan.runs.iter().enumerate() {
        entries[f] = (rs.iter())
            .map(|r| {
                Ok(match *r {
                    // SAFETY: a function of signature `fn(ptr) -> u32` (`declare`).
                    Pend::Fn(k) if !plan.funs[k].1 => {
                        Run::Fn(unsafe { std::mem::transmute::<*const u8, Entry>(fun(k)?) })
                    }
                    // SAFETY: a function of a shape, `fn(ptr, ptr) -> u32` (`declare`).
                    Pend::Shared(k, t, s) if plan.funs[k].1 && (t as usize) < plan.tables.len() => {
                        Run::Shared(
                            unsafe { std::mem::transmute::<*const u8, SharedEntry>(fun(k)?) },
                            t,
                            s,
                        )
                    }
                    Pend::Save(g) => Run::Save(g),
                    Pend::Same(g) => Run::Same(g),
                    Pend::Copy(c) if c < plan.copies.len() => Run::Copy(c),
                    Pend::Guard(g, n, c) => Run::Guard(g, n, c),
                    _ => return Err("a step of another signature than its function".to_string()),
                })
            })
            .collect::<Result<_, String>>()?;
    }
    let guards = plan.guards;
    if guards.iter().any(|g| {
        let end = |v: &[(u32, u32)]| v.iter().map(|&(w, c)| (w + c) as usize).max().unwrap_or(0);
        let n: u32 = g.ins.iter().map(|x| x.1).sum();
        end(&g.ins) > words
            || end(&g.outs) > words
            || (g.copy + n) as usize > words
            || g.flag as usize >= words
    }) {
        return Err("a guard's words do not fit the image".into());
    }
    let stats = std::env::var_os("YAVS_CYCLE_STATS").is_some();
    let counts = stats.then(|| guards.iter().map(|_| Default::default()).collect());
    let settle_units = plan.settle_units;
    let mut every = vec![0u64; settle_units.len().div_ceil(64)];
    for (i, _) in settle_units.iter().enumerate().filter(|x| !x.1.is_empty()) {
        every[i / 64] |= 1 << (i % 64);
    }
    if (plan.copies.iter().flatten()).any(|&(d, s, n)| (d.max(s) + n) as usize > words) {
        return Err("a copy's words do not fit the image".into());
    }
    if (plan.edge_parts.iter()).any(|(ws, _, _, m)| {
        ws.iter().any(|&w| w as usize >= words) || (*m != u32::MAX && *m as usize >= words)
    }) {
        return Err("an edge gate's word does not fit the image".into());
    }
    // A step without units behind a guard runs only after it: `steps` walks every step then.
    let mut fixed = vec![0u64; settle_units.len().div_ceil(64)];
    let mut covered = 0usize;
    let mut fixed_ok = true;
    for (i, us) in settle_units.iter().enumerate() {
        if us.is_empty() {
            fixed[i / 64] |= 1 << (i % 64);
            fixed_ok &= covered == 0;
        }
        covered = match entries[0][i] {
            Run::Guard(_, n, _) => n.max(covered.saturating_sub(1)),
            _ => covered.saturating_sub(1),
        };
    }
    Ok(Code {
        _module: module,
        fixed: fixed_ok.then(|| fixed.into_boxed_slice()),
        entries,
        edge_parts: plan.edge_parts,
        groups: plan.groups,
        copies: plan.copies,
        words,
        tables: plan.tables,
        guards,
        settle_units,
        every: every.into_boxed_slice(),
        signal_words: plan.signal_words,
        settle_outs: plan.settle_outs,
        run_words: plan.run_words,
        log_count: plan.log_count,
        skip_check: skip_check()? != SkipCheck::Off,
        full_check: plan.full_check,
        counts,
        stale: Default::default(),
    })
}

/// What [`compile`] made besides the machine code: the functions, what each trigger function
/// runs and the run-time data of the code. Kept with the machine code in the cache, it makes the
/// region's code again without a function body built ([`from_blob`]).
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct CodePlan {
    /// Per function: its symbol, and whether it is a shape's body (with a table).
    funs: Vec<(String, bool)>,
    runs: [Vec<Pend>; 4],
    /// [`Code::edge_parts`].
    edge_parts: Vec<EdgePart>,
    groups: Vec<(Vec<(u32, u32)>, u32)>,
    copies: Vec<Vec<(u32, u32, u32)>>,
    words: usize,
    tables: Vec<Box<[usize]>>,
    guards: Vec<Guard>,
    settle_units: Vec<Vec<u32>>,
    signal_words: usize,
    log_count: usize,
    full_check: Option<Vec<(u32, u32)>>,
    /// `YAVS_CYCLE_SKIP_CHECK`: set by the caller ([`Code::settle_outs`]).
    pub settle_outs: Vec<Vec<(u32, u32)>>,
    /// Set by the caller ([`Code::run_words`]).
    pub run_words: Vec<u32>,
}

impl CodePlan {
    /// How many functions the region has.
    pub(crate) fn functions(&self) -> usize {
        self.funs.len()
    }
}

/// A step of a trigger function as [`CodePlan`] keeps it: a function by its index, a function of a
/// shared body with its table and first site, or a step of the runner ([`Run`]).
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
enum Pend {
    Fn(usize),
    Shared(usize, u32, u32),
    Save(usize),
    Same(usize),
    Guard(usize, usize, bool),
    Copy(usize),
}

/// Most words a piece keeps in registers at once (`Gen::keep_in`), and for a piece of more than
/// `BIG` operations: in a large piece the register allocator's work grows with the values live at
/// once, while a small piece with many words (a record copied whole) runs faster with them all.
const CACHE_MAX: usize = 256;
/// Most pieces a value in a register is handed on through: the register allocator's work grows
/// with the length of what is live (rtl_ff, 4096 one-bit members reading the words of one
/// vector: 64 pieces cost 8 percent more compile than 16, which reload a word every 16 members).
const CARRY: u32 = 16;
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
    /// A failed check stores its operands in the error words.
    vals: bool,
    /// The exit of failed checks, made at the first: it returns its parameter, the site plus one.
    exit: Option<CBlock>,
    ptr: cranelift_codegen::ir::Type,
    /// The words in registers, by byte offset: their value and whether memory is behind it. Every
    /// value dominates the code that follows (a branch drops what it made at its join).
    cache: BTreeMap<i32, (Value, bool)>,
    /// Most words in `cache` at once, for the piece being generated.
    cap: usize,
    /// A shared body: the first virtual byte offset of each block, the address of the block it is
    /// addressed from and the distance to it, in order; a word's address is from the last block at
    /// or below it (`at`).
    blocks: Vec<(i32, Value, i32)>,
    keep: &'a [bool],
    /// The `keep` bits read, by byte offset, and whether the error words (bit 0) or the log
    /// (bit 1) were: the code is the same wherever those are ([`entry`]).
    seen: std::cell::RefCell<Words<bool>>,
    used: std::cell::Cell<u8>,
    /// Bits known to be zero of some values (constants, extracted bits, words built by inserts).
    zeros: std::collections::HashMap<Value, u64>,
    /// Values known to be bits `s .. s + n` of a word `v` in their low bits: `(v, s, n)`.
    fields: SecondaryMap<Value, Option<(Value, u8, u8)>>,
    /// Words built from a word `d0` by inserting fields of one word `v`, all moved by the same
    /// distance `dl` (left when positive): `d0 & !m | v << dl & m` (`insert`).
    built: SecondaryMap<Value, Option<(Value, Value, i8, u64)>>,
    /// The piece a value entered the registers in, plus one, and the pieces generated so far.
    born: SecondaryMap<Value, u32>,
    piece: u32,
    /// The operation being generated, by its number in [`walk`], and the next number.
    pos: u32,
    next: u32,
    /// Per word the part reads: the last operation reading it (a scratch word is dead after it,
    /// unless a later part reads it, `later`).
    last: Words<u32>,
    later: Option<Arc<Words<()>>>,
    /// `Op::Track` in this part: its flag's byte offset, whether it is the member's first part,
    /// and the OR of what each store of a word that holds state changed.
    track: Option<(i32, bool, cranelift_frontend::Variable)>,
    /// Debug builds: scratch words dropped from registers unstored as dead, which no code may
    /// read from memory before writing them again.
    #[cfg(debug_assertions)]
    gone: std::collections::HashSet<i32>,
}

impl Gen<'_> {
    fn ops(&mut self, ops: &[Op]) {
        for op in ops {
            self.op(op);
        }
    }

    /// The reads of part `p`, which the parts after it read `later` of.
    fn live_in(&mut self, p: &[Op], later: Option<Arc<Words<()>>>) {
        self.last.clear();
        walk(p, &mut 0, &mut |i, w| {
            self.last.insert(w, i);
        });
        (self.pos, self.next) = (0, 0);
        self.later = later;
        #[cfg(debug_assertions)]
        self.gone.clear();
    }

    /// Whether word `off` may still be read: state, read by a later part, or by this operation
    /// or a later one (a word past the image is no state).
    fn live(&self, off: i32) -> bool {
        self.state(off) || self.last.get(&off).is_some_and(|&i| i >= self.pos)
    }

    /// Whether word `off` must outlive the part: state, or read by a later part.
    fn state(&self, off: i32) -> bool {
        self.kept(off) || self.later.as_ref().is_some_and(|l| l.contains_key(&off))
    }

    /// Whether word `off` holds state between calls, noted in `seen`.
    fn kept(&self, off: i32) -> bool {
        let k = kept(self.keep, off);
        self.seen.borrow_mut().insert(off, k);
        k
    }

    fn err(&self) -> i32 {
        self.used.set(self.used.get() | 1);
        self.err
    }

    fn log(&self) -> (i32, u32) {
        self.used.set(self.used.get() | 2);
        self.log
    }

    /// Word `off` is dead: out of the registers, never stored.
    fn drop_dead(&mut self, off: i32) {
        if let Some((_, d)) = self.cache.remove(&off) {
            #[cfg(debug_assertions)]
            if d {
                self.gone.insert(off);
            }
            let _ = d;
        }
    }

    /// Debug builds: words `off ..` (`n`) are read from memory, so none may be a dropped one.
    fn read_memory(&self, off: i32, n: u32) {
        #[cfg(debug_assertions)]
        for k in 0..n as i32 {
            assert!(
                !self.gone.contains(&(off + 8 * k)),
                "cycle mode: the code reads a scratch word it took for dead (this is a bug in yavs)"
            );
        }
        let _ = (off, n);
    }

    /// The address of the word at byte offset `off`: a base and an offset from it.
    fn at(&self, off: i32) -> (Value, i32) {
        match self.blocks.iter().rev().find(|(v, ..)| *v <= off) {
            Some(&(v, b, d)) => (b, off - v + d),
            None => (self.base, off),
        }
    }

    /// Word `k` of `r`: from a register when it has one.
    fn ld(&mut self, r: Ref, k: u32) -> Value {
        let off = word(r) + k as i32 * 8;
        if let Some(&(v, _)) = self.cache.get(&off) {
            return v;
        }
        self.read_memory(off, 1);
        let (a, o) = self.at(off);
        let v = self.b.ins().load(I64, MemFlagsData::trusted(), a, o);
        self.keep_in(off, v, false);
        v
    }

    /// Word `k` of `r` is `v` from here on; memory follows at `write_back` or `spill`.
    fn st(&mut self, r: Ref, k: u32, v: Value) {
        let off = word(r) + k as i32 * 8;
        if !self.live(off) {
            self.drop_dead(off);
            return;
        }
        #[cfg(debug_assertions)]
        self.gone.remove(&off);
        // State no later operation of the part reads goes to memory now: in a register it would
        // wait for the end of the part, spilled to the stack when many do (a commit's copies).
        if self.state(off) && self.last.get(&off).is_none_or(|&i| i <= self.pos) {
            self.store(off, v);
            self.keep_in(off, v, false);
            return;
        }
        self.keep_in(off, v, true);
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
        if self.born[v] == 0 {
            self.born[v] = self.piece + 1;
        }
        self.cache.insert(off, (v, dirty));
    }

    fn store(&mut self, off: i32, v: Value) {
        let (a, o) = self.at(off);
        if self.track.is_some() && self.kept(off) {
            let old = self.b.ins().load(I64, MemFlagsData::trusted(), a, o);
            self.changed(old, v);
        }
        self.b.ins().store(MemFlagsData::trusted(), v, a, o);
    }

    /// Under `Op::Track`: a store of `new` over `old`, ORed into what the part changed.
    fn changed(&mut self, old: Value, new: Value) {
        let Some((_, _, var)) = self.track else {
            return;
        };
        let x = self.b.ins().bxor(old, new);
        let acc = self.b.use_var(var);
        let acc = self.b.ins().bor(acc, x);
        self.b.def_var(var, acc);
    }

    /// The end of a tracked part: its flag word 1 when a store changed a word (ORed into the
    /// flag of the member's earlier parts).
    fn track_end(&mut self) {
        let Some((off, first, var)) = self.track.take() else {
            return;
        };
        let acc = self.b.use_var(var);
        let one = self.b.ins().icmp_imm_s(IntCC::NotEqual, acc, 0);
        let mut f = self.b.ins().uextend(I64, one);
        let (a, o) = self.at(off);
        if !first {
            let old = self.b.ins().load(I64, MemFlagsData::trusted(), a, o);
            f = self.b.ins().bor(old, f);
        }
        self.b.ins().store(MemFlagsData::trusted(), f, a, o);
    }

    /// The words that hold state, into memory (the end of the function).
    fn write_back(&mut self) {
        let dirty: Vec<(i32, Value)> = (self.cache.iter())
            .filter(|(off, (_, d))| *d && self.kept(**off))
            .map(|(&off, &(v, _))| (off, v))
            .collect();
        for (off, v) in dirty {
            self.store(off, v);
        }
    }

    /// Every changed word into memory, scratch ones too.
    fn spill_all(&mut self) {
        let dirty: Vec<(i32, Value)> = (self.cache.iter())
            .filter(|&(&off, (_, d))| *d && self.live(off))
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
            match self.cache.get_mut(&off) {
                Some((v, d)) if *d => {
                    *d = false;
                    let v = *v;
                    self.store(off, v);
                }
                Some(_) => {}
                None => self.read_memory(off, 1),
            }
        }
    }

    /// Words `k0 ..= k1` of `r` changed in memory.
    fn forget(&mut self, r: Ref, k0: u32, k1: u32) {
        for k in k0..=k1 {
            self.cache.remove(&(word(r) + k as i32 * 8));
            #[cfg(debug_assertions)]
            self.gone.remove(&(word(r) + k as i32 * 8));
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

    /// `Op::P4`: the planes of `a` (and `b`) word by word into those of `dst` (`yavs_logic::planes`
    /// formulas: classes `is_u`, `is0` (`0`, `L`), `is1` (`1`, `H`) of each element).
    fn p4(&mut self, f: P4F, dst: Ref, dw: u32, a: (Ref, u32), b: (Ref, u32), len: u32) {
        let n = words(len);
        let mut eq = None;
        for k in 0..n {
            let m = mask((len - 64 * k).min(64));
            let ld4 = |g: &mut Self, (r, w): (Ref, u32)| -> [Value; 4] {
                [0, 1, 2, 3].map(|p| g.ld(plane(r, w, p), k))
            };
            if f == P4F::Codes {
                let v = self.ld(a.0, k);
                let one = self.konst(m);
                let z = self.konst(0);
                for (p, x) in [v, one, z, z].into_iter().enumerate() {
                    self.st(plane(dst, dw, p as u32), k, x);
                }
                continue;
            }
            let x = ld4(self, a);
            if let P4F::Eq { .. } = f {
                let y = ld4(self, b);
                let mut d = self.konst(0);
                for p in 0..4 {
                    let t = self.b.ins().bxor(x[p], y[p]);
                    d = self.b.ins().bor(d, t);
                }
                eq = Some(match eq {
                    None => d,
                    Some(e) => self.b.ins().bor(e, d),
                });
                continue;
            }
            // Classes, masked to the elements.
            let cls = |g: &mut Self, [p0, p1, p2, p3]: [Value; 4]| {
                let o1 = g.b.ins().bor(p0, p1);
                let o2 = g.b.ins().bor(p2, p3);
                let o = g.b.ins().bor(o1, o2);
                let u = g.b.ins().bnot(o);
                let u = g.b.ins().band_imm_s(u, m as i64);
                let k1 = g.b.ins().band_not(p1, p3);
                let one = g.b.ins().band(k1, p0);
                let zero = g.b.ins().band_not(k1, p0);
                (u, zero, one)
            };
            let (ux, zx, ox) = cls(self, x);
            let (r0, r1) = match f {
                P4F::Not => {
                    let a0 = self.b.ins().bor(ux, ox);
                    let r0 = self.b.ins().bnot(a0);
                    let r0 = self.b.ins().band_imm_s(r0, m as i64);
                    (r0, self.b.ins().bor(zx, ox))
                }
                P4F::Gate(g) => {
                    let y = ld4(self, b);
                    let (uy, zy, oy) = cls(self, y);
                    let u = self.b.ins().bor(ux, uy);
                    let mm = self.konst(m);
                    let nu = self.b.ins().band_not(mm, u);
                    let (r0, r1) = match g {
                        Gate::And | Gate::Nand => {
                            let z = self.b.ins().bor(zx, zy);
                            let one = self.b.ins().band(ox, oy);
                            let r0 = self.b.ins().band_not(nu, z);
                            (r0, self.b.ins().bor(z, one))
                        }
                        Gate::Or | Gate::Nor => {
                            let o = self.b.ins().bor(ox, oy);
                            let zz = self.b.ins().band(zx, zy);
                            let a0 = self.b.ins().band_not(nu, zz);
                            let b0 = self.b.ins().band(nu, zz);
                            (self.b.ins().bor(o, a0), self.b.ins().bor(o, b0))
                        }
                        Gate::Xor | Gate::Xnor => {
                            let kx = self.b.ins().bor(zx, ox);
                            let ky = self.b.ins().bor(zy, oy);
                            let kk = self.b.ins().band(kx, ky);
                            let v = self.b.ins().bxor(ox, oy);
                            let nk = self.b.ins().band_not(mm, kk);
                            let a0 = self.b.ins().bor(nk, v);
                            (self.b.ins().band(nu, a0), self.b.ins().band(nu, kk))
                        }
                    };
                    if matches!(g, Gate::Nand | Gate::Nor | Gate::Xnor) {
                        // IEEE `not` of codes 0 to 3: `U` stays, `X` stays, `0` and `1` swap.
                        let known = r1;
                        let nr0 = self.b.ins().band_not(known, r0);
                        let xo = self.b.ins().band_not(r0, known);
                        (self.b.ins().bor(nr0, xo), known)
                    } else {
                        (r0, r1)
                    }
                }
                _ => unreachable!("handled above"),
            };
            let z = self.konst(0);
            for (p, v) in [r0, r1, z, z].into_iter().enumerate() {
                self.st(plane(dst, dw, p as u32), k, v);
            }
        }
        if let (P4F::Eq { ne }, Some(e)) = (f, eq) {
            let c = if ne { IntCC::NotEqual } else { IntCC::Equal };
            let r = self.b.ins().icmp_imm_s(c, e, 0);
            let r = self.bool_of(r);
            self.st(dst, 0, r);
            if dw != 0 {
                let one = self.konst(1);
                let z = self.konst(0);
                for (p, v) in [one, z, z].into_iter().enumerate() {
                    self.st(plane(dst, dw, p as u32 + 1), 0, v);
                }
            }
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
        // Bits of one word: the field they are of, through a field of a field.
        let field = (s + n <= 64).then(|| match self.fields[v[k]] {
            Some((w, s0, n0)) if s + n <= u32::from(n0) => (w, s0 + s as u8, n as u8),
            _ => (v[k], s as u8, n as u8),
        });
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
        if x != v[k] || self.fields[x].is_none() {
            self.fields[x] = field;
        }
        x
    }

    /// The value of `v` when it is a constant.
    fn konst_of(&self, v: Value) -> Option<u64> {
        let i = self.b.func.dfg.value_def(v).inst()?;
        match self.b.func.dfg.insts[i] {
            InstructionData::UnaryImm {
                opcode: Opcode::Iconst,
                imm,
            } => Some(imm.bits() as u64),
            _ => None,
        }
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
            if let Some(r) = self.merge(dk, x, sh, b - a) {
                d[k as usize] = r;
                a = b;
                continue;
            }
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

    /// `dk` with `n` bits from `sh` taken from `x`, when `x` is a field of a word `v`: one move of
    /// `v` and one mask, with the fields of `v` that `dk` was built from at the same distance
    /// (lanes copied lane by lane become a word op).
    fn merge(&mut self, dk: Value, x: Value, sh: u32, n: u32) -> Option<Value> {
        let (v, s0, n0) = self.fields[x]?;
        if u32::from(n0) != n {
            return None;
        }
        let dl = sh as i32 - i32::from(s0);
        let m = mask(n) << sh;
        let (d0, mt) = match self.built[dk] {
            Some((d0, w, l, m2)) if w == v && i32::from(l) == dl => (d0, m | m2),
            _ => (dk, m),
        };
        let zv = self.zero(v);
        let (t, zt) = match dl {
            0 => (v, zv),
            1.. => (
                self.b.ins().ishl_imm_s(v, i64::from(dl)),
                zv << dl | mask(dl as u32),
            ),
            _ => (
                self.b.ins().ushr_imm_s(v, i64::from(-dl)),
                zv >> -dl | !(u64::MAX >> -dl),
            ),
        };
        let t = if zt | mt == !0 {
            t
        } else {
            self.b.ins().band_imm_s(t, mt as i64)
        };
        let zd = self.zero(d0);
        let r = if zd | mt == !0 {
            t
        } else {
            let keep = if zd & mt == mt {
                d0
            } else {
                self.b.ins().band_imm_s(d0, !mt as i64)
            };
            self.b.ins().bor(keep, t)
        };
        self.zeros.insert(r, (zd | mt) & (zt | !mt));
        self.built[r] = Some((d0, v, dl as i8, mt));
        Some(r)
    }

    fn bool_of(&mut self, c: Value) -> Value {
        self.b.ins().uextend(I64, c)
    }

    /// A check: `fail` (an `I8` truth value) jumps to a cold block that stores `vals` in the
    /// error words (only without `--two-state`) and goes to the function's one exit of failed
    /// checks with `site + 1` (one epilogue for all of them).
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
        if self.vals {
            let err = self.err();
            for (k, &v) in vals.iter().enumerate() {
                self.b
                    .ins()
                    .store(MemFlagsData::trusted(), v, self.base, err + k as i32 * 8);
            }
        }
        let s = self.b.ins().iconst(I32, i64::from(site) + 1);
        let b = &mut self.b;
        let exit = *self.exit.get_or_insert_with(|| {
            let x = b.create_block();
            b.set_cold_block(x);
            b.append_block_param(x, I32);
            x
        });
        self.b.ins().jump(exit, &[s.into()]);
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

    /// Whether every field at a bit `lo + k * stride` starts a word.
    fn word_aligned(lo: u32, stride: u32) -> bool {
        lo.is_multiple_of(64) && stride.is_multiple_of(64)
    }

    /// The `w` bits of `r` from the run-time bit `bit` (memory holds them, see `spill`); `whole`:
    /// `bit` is a multiple of 64, so each word is one load.
    fn dyn_read(&mut self, r: Ref, bit: Value, w: u32, one_word: bool, whole: bool) -> Vec<Value> {
        if whole {
            let (a, _) = self.bit_addr(r, bit);
            return (0..words(w))
                .map(|j| {
                    let mut x = self
                        .b
                        .ins()
                        .load(I64, MemFlagsData::trusted(), a, 8 * j as i32);
                    let n = (w - 64 * j).min(64);
                    if n < 64 {
                        x = self.b.ins().band_imm_s(x, mask(n) as i64);
                        self.zeros.insert(x, !mask(n));
                    }
                    x
                })
                .collect();
        }
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

    /// The `w` bits `v` into `r` from the run-time bit `bit`, in memory; `whole`: as in
    /// `dyn_read`, a full word is one store.
    fn dyn_write(&mut self, r: Ref, bit: Value, w: u32, v: &[Value], one_word: bool, whole: bool) {
        if whole {
            let (a, _) = self.bit_addr(r, bit);
            for (j, &x) in v.iter().enumerate() {
                let n = (w - 64 * j as u32).min(64);
                let o = 8 * j as i32;
                let tracked = self.track.is_some() && self.kept(word(r));
                let x = if n < 64 || tracked {
                    let w0 = self.b.ins().load(I64, MemFlagsData::trusted(), a, o);
                    let x = if n < 64 {
                        let keep = self.b.ins().band_imm_s(w0, !mask(n) as i64);
                        self.b.ins().bor(keep, x)
                    } else {
                        x
                    };
                    if tracked {
                        self.changed(w0, x);
                    }
                    x
                } else {
                    x
                };
                self.b.ins().store(MemFlagsData::trusted(), x, a, o);
            }
            return;
        }
        for (j, &x) in v.iter().enumerate() {
            let b = if j == 0 {
                bit
            } else {
                self.b.ins().iadd_imm_s(bit, 64 * j as i64)
            };
            let n = (w - 64 * j as u32).min(64);
            let (a, s) = self.bit_addr(r, b);
            let m = self.konst(mask(n));
            let tracked = self.track.is_some() && self.kept(word(r));
            let old = self.b.ins().load(I64, MemFlagsData::trusted(), a, 0);
            let ms = self.b.ins().ishl(m, s);
            let keep = self.b.ins().band_not(old, ms);
            let xs = self.b.ins().ishl(x, s);
            let w0 = self.b.ins().bor(keep, xs);
            if tracked {
                self.changed(old, w0);
            }
            self.b.ins().store(MemFlagsData::trusted(), w0, a, 0);
            if !one_word {
                // The bits that cross into the next word (none when `s` is 0); its other bits are
                // stored back unchanged.
                let old = self.b.ins().load(I64, MemFlagsData::trusted(), a, 8);
                let mh = self.ushr_rest(m, s);
                let keep = self.b.ins().band_not(old, mh);
                let xh = self.ushr_rest(x, s);
                let w1 = self.b.ins().bor(keep, xh);
                if tracked {
                    self.changed(old, w1);
                }
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
        // A field of a word, signed, to at most a word: two shifts of the word it is of.
        if signed
            && to <= 64
            && let Some((w, s0, n)) = self.fields[v[0]]
            && u32::from(n) == len
        {
            let t = self
                .b
                .ins()
                .ishl_imm_s(w, i64::from(64 - len - u32::from(s0)));
            let mut out = vec![self.b.ins().sshr_imm_s(t, i64::from(64 - len))];
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

    /// Operation `op`; then the scratch words it read last are dropped from the registers, so no
    /// dead word is ever in them (one written that no later operation reads is not put there).
    fn op(&mut self, op: &Op) {
        let i = self.next;
        (self.pos, self.next) = (i, i + 1);
        self.emit(op);
        reads(op, &mut |off, n| {
            for k in 0..n as i32 {
                let w = off + 8 * k;
                if self.last.get(&w) == Some(&i) && !self.state(w) {
                    self.drop_dead(w);
                }
            }
        });
    }

    fn emit(&mut self, op: &Op) {
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
                // One element against a literal: the element is the truth value, or its inverse.
                if *len == 1 {
                    let one = |g: &Self, v| g.konst_of(v).filter(|k| k & !1 == 0);
                    if let Some((v, k)) = (one(self, y[0]).map(|k| (x[0], k)))
                        .or_else(|| one(self, x[0]).map(|k| (y[0], k)))
                    {
                        let r = if (k == 1) != *ne {
                            v
                        } else {
                            self.b.ins().bxor_imm_s(v, 1)
                        };
                        self.zeros.insert(r, !1);
                        self.st(*dst, 0, r);
                        return;
                    }
                }
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
                let v = self.dyn_read(
                    *src,
                    bit,
                    *w,
                    Self::aligned(*lo, *stride, *w),
                    Self::word_aligned(*lo, *stride),
                );
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
                self.dyn_write(
                    *dst,
                    bit,
                    *w,
                    &v,
                    Self::aligned(*lo, *stride, *w),
                    Self::word_aligned(*lo, *stride),
                );
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
                // `MIN / -1` traps in the machine (only a 64-element signed operand can be it), but
                // `numeric_std` gives MIN, `rem` and `mod` 0: divide by 1 there, which gives both.
                let ov = (*signed && *len == 64).then(|| {
                    let m1 = self.b.ins().icmp_imm_s(IntCC::Equal, y, -1);
                    let mn = self.b.ins().icmp_imm_s(IntCC::Equal, x, i64::MIN);
                    self.b.ins().band(m1, mn)
                });
                let y = self.div_guard(z, *site, x, y);
                let y = match ov {
                    Some(ov) => {
                        let one = self.konst(1);
                        self.b.ins().select(ov, one, y)
                    }
                    None => y,
                };
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
            Op::Track { flag, first } => {
                self.track_end();
                let var = self.b.declare_var(I64);
                let zero = self.b.ins().iconst(I64, 0);
                self.b.def_var(var, zero);
                self.track = Some((word(*flag), *first, var));
            }
            Op::Nine { .. } => unreachable!("nine-valued operations are expanded before codegen"),
            Op::P4 {
                f,
                dst,
                dw,
                a,
                aw,
                b,
                bw,
                len,
            } => self.p4(*f, *dst, *dw, (*a, *aw), (*b, *bw), *len),
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
                let (at, cap) = self.log();
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
                // A condition read here last is dead now: not stored before the branch.
                let w = word(*cond);
                if self.last.get(&w) == Some(&self.pos) && !self.state(w) {
                    self.drop_dead(w);
                }
                let (t, e, j): (CBlock, CBlock, CBlock) = (
                    self.b.create_block(),
                    self.b.create_block(),
                    self.b.create_block(),
                );
                // Memory holds every word before the branches; each starts from the words in
                // registers, stores what it changed before the join, and the join reads those
                // from memory again. (Dead words are not in registers: see `Gen::op`.)
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

    /// Whether `x` is outside `lo ..= hi`: `x - lo` above `hi - lo`, unsigned (one compare).
    fn outside(&mut self, x: Value, lo: i64, hi: i64) -> Value {
        let i8 = cranelift_codegen::ir::types::I8;
        if lo > hi {
            return self.b.ins().iconst(i8, 1);
        }
        let span = hi.wrapping_sub(lo);
        if span == -1 {
            return self.b.ins().iconst(i8, 0);
        }
        let d = if lo == 0 {
            x
        } else {
            self.b.ins().iadd_imm_s(x, lo.wrapping_neg())
        };
        self.b.ins().icmp_imm_s(IntCC::UnsignedGreaterThan, d, span)
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
                // Always guarded: the machine division traps on zero and on MIN / -1. The quotient
                // overflows there; `rem` and `mod` are 0, which dividing by 1 gives.
                let z = self.b.ins().icmp_imm_s(IntCC::Equal, y, 0);
                let m1 = self.b.ins().icmp_imm_s(IntCC::Equal, y, -1);
                let mn = self.b.ins().icmp_imm_s(IntCC::Equal, x, i64::MIN);
                let ov = self.b.ins().band(m1, mn);
                let y = if op == Arith::Div {
                    let f = self.b.ins().bor(z, ov);
                    self.div_guard(f, site, x, y)
                } else {
                    let y = self.div_guard(z, site, x, y);
                    let one = self.konst(1);
                    self.b.ins().select(ov, one, y)
                };
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

#[cfg(test)]
mod tests {
    use super::*;

    /// An unrolled loop's `if idx = 0 then if en then q(b) <= d(b)`: bit `b` of word 1 from word
    /// 2, with its chain in words from `t`.
    fn bit_if(t: u32, b: u32, idx: u32) -> Vec<Op> {
        let l = Ref::Loc;
        vec![
            Op::IntLit { dst: l(t), v: 0 },
            Op::ToInt {
                dst: l(t + 1),
                a: l(idx),
                len: 7,
                signed: false,
            },
            Op::IntCmp {
                op: crate::lower::Cmp::Eq,
                dst: l(t + 2),
                a: l(t + 1),
                b: l(t),
            },
            Op::If {
                cond: l(t + 2),
                then: vec![
                    Op::Extract {
                        dst: l(t + 3),
                        src: l(3),
                        lo: 0,
                        len: 1,
                        full: 1,
                    },
                    Op::If {
                        cond: l(t + 3),
                        then: vec![
                            Op::Extract {
                                dst: l(t + 4),
                                src: l(2),
                                lo: b,
                                len: 1,
                                full: 64,
                            },
                            Op::Insert {
                                dst: l(1),
                                src: l(t + 4),
                                lo: b,
                                len: 1,
                            },
                        ],
                        else_: vec![],
                    },
                ],
                else_: vec![],
            },
        ]
    }

    fn ifs(ops: &[Op]) -> usize {
        ops.iter()
            .map(|o| match o {
                Op::If { then, else_, .. } => 1 + ifs(then) + ifs(else_),
                _ => 0,
            })
            .sum()
    }

    #[test]
    fn consecutive_ifs_on_one_condition_join() {
        let keep = vec![false; 64];
        let ops: Vec<Op> = (0..4).flat_map(|b| bit_if(10 + 5 * b, b, 4)).collect();
        let j = join_ifs(&ops, &keep);
        // One outer `If` with one inner `If` holding the four copies, one chain left.
        assert_eq!((ifs(&ops), ifs(&j), j.len()), (8, 2, 4), "{j:?}");
        // A branch writing what the condition reads keeps them apart.
        let mut w = bit_if(10, 0, 4);
        if let Op::If { then, .. } = &mut w[3] {
            then.push(Op::Lit {
                dst: Ref::Loc(4),
                val: vec![1].into(),
            });
        }
        w.extend(bit_if(15, 1, 4));
        assert_eq!(ifs(&join_ifs(&w, &keep)), 4);
        // Another index, a chain word that holds state or that is read later: apart too.
        let other: Vec<Op> = [bit_if(10, 0, 4), bit_if(15, 1, 5)].concat();
        assert_eq!(ifs(&join_ifs(&other, &keep)), 4);
        let mut kept = keep.clone();
        kept[17] = true;
        assert_eq!(ifs(&join_ifs(&ops[..8], &kept)), 4);
        // Variables updated in place before each `If` (`a := a and en; if a = '1'`, then the same
        // with `b`): the chains look alike but read other values.
        let l = Ref::Loc;
        let var = |v: u32, src: u32, c: u32| {
            vec![
                Op::Copy {
                    dst: l(v),
                    src: l(src),
                    len: 1,
                },
                Op::Gate {
                    g: crate::lower::Gate::And,
                    dst: l(v),
                    a: l(v),
                    b: l(3),
                    len: 1,
                },
                Op::If {
                    cond: l(v),
                    then: vec![Op::Lit {
                        dst: l(c),
                        val: vec![1].into(),
                    }],
                    else_: vec![],
                },
            ]
        };
        let inplace = [var(20, 4, 30), var(21, 5, 31)].concat();
        assert_eq!(ifs(&join_ifs(&inplace, &keep)), 2);
        // The same without the copies: each variable is written once, from its own earlier value.
        let own = |v: u32, c: u32| var(v, 0, c)[1..].to_vec();
        assert_eq!(
            ifs(&join_ifs(&[own(20, 30), own(21, 31)].concat(), &keep)),
            2
        );
        // A chain operation reading a word a later one of the chain writes (what the last run left).
        let stale = |t: u32, c: u32| {
            vec![
                Op::Copy {
                    dst: l(t),
                    src: l(t + 1),
                    len: 1,
                },
                Op::Lit {
                    dst: l(t + 1),
                    val: vec![1].into(),
                },
                Op::Gate {
                    g: crate::lower::Gate::And,
                    dst: l(t + 2),
                    a: l(t),
                    b: l(t + 1),
                    len: 1,
                },
                Op::If {
                    cond: l(t + 2),
                    then: vec![Op::Lit {
                        dst: l(c),
                        val: vec![1].into(),
                    }],
                    else_: vec![],
                },
            ]
        };
        assert_eq!(
            ifs(&join_ifs(&[stale(40, 30), stale(44, 31)].concat(), &keep)),
            2
        );
        // A chain word written again later in the member: apart.
        let mut again = ops[..8].to_vec();
        again.push(Op::Lit {
            dst: Ref::Loc(16),
            val: vec![0].into(),
        });
        assert_eq!(ifs(&join_ifs(&again, &keep)), 4);
        let mut late = ops[..8].to_vec();
        late.push(Op::Copy {
            dst: Ref::Loc(5),
            src: Ref::Loc(17),
            len: 1,
        });
        assert_eq!(ifs(&join_ifs(&late, &keep)), 4);
    }

    /// The settle's machine code of one copy into word 2, for an image whose word 2 holds state
    /// or not, from the store in `lib` when given.
    fn copy_code(state: bool, lib: Option<&Path>) -> Vec<(u64, Vec<u8>)> {
        set_cache(lib);
        let op = Op::Copy {
            dst: Ref::Loc(2),
            src: Ref::Loc(0),
            len: 64,
        };
        let keep = [true, true, state, false, false, false];
        let (_, _, made) = compile(
            &[Step::Ops(vec![op])],
            &[0],
            &[],
            None,
            &[],
            &[],
            &[],
            Vec::new(),
            Vec::new(),
            &keep,
            2,
            4,
            (5, 0),
            "keep",
        )
        .expect("compiles");
        set_cache(None);
        made.expect("no relocations")
    }

    // Two images whose words differ only in which hold state share the function's key; the
    // entry is a hit only where the `keep` bits the code generator read are the same.
    #[test]
    fn a_cached_function_is_no_hit_where_a_word_it_read_holds_state() {
        let dir = std::env::temp_dir().join(format!("yavs_cycle_keep_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (scratch, state) = (copy_code(false, None), copy_code(true, None));
        assert_ne!(
            scratch, state,
            "a state word must be stored, a scratch one need not"
        );
        assert_eq!(copy_code(false, Some(&dir)), scratch);
        assert_eq!(
            copy_code(true, Some(&dir)),
            state,
            "the scratch image's code was taken"
        );
        assert_eq!(copy_code(false, Some(&dir)), scratch);
        // One key for both images: the keep bits are not in it (else this test checks nothing).
        let store = dir.join(yavs_cache::DIR).join(yavs_cache::CYCLE_DIR);
        let entries = (std::fs::read_dir(store).expect("the store"))
            .filter(|f| {
                (f.as_ref().ok())
                    .is_some_and(|f| f.file_name().to_string_lossy().starts_with("cycle-"))
            })
            .count();
        assert_eq!(entries, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
