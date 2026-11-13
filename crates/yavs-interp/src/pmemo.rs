//! The memo of pure calls (#701): the results of the calls a process made of the functions
//! `Program::memo_sig` allows, kept by the bytes of their arguments, so a call with arguments
//! it has seen before is answered without running the function (`Machine::memo_probe`).
//!
//! One memo per function and process (no locks; the same hits on every thread count but where the
//! switch off for all, `Pre::memo_off`, flips while other threads run): a
//! direct-mapped table of `SLOTS` entries, each the key (the argument words and, for an array,
//! its descriptor and element bytes), a hash of it and the result. A key is compared whole, so
//! a hit is exact. The first `WARM` calls of a function decide whether it is kept: a function
//! whose calls rarely repeat costs the key and the entry for nothing, and is switched off for
//! the process.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use yavs_ir::{Func, FuncId, MemoArg, MemoArr, MemoSig, Program, Reg, Regs};
use yavs_kernel::Ctx;

use crate::Runtime;
use crate::Tier;
use crate::exec::{Flow, Machine, Outcome, R, add_addr, cold, ice};

/// Entries of the table of a function: it starts at `FIRST` and doubles up to `LAST` while
/// calls keep evicting each other's entries (both powers of two).
const FIRST: usize = 32;
const LAST: usize = 256;
/// Longest key and longest result, in bytes: a larger call is made as usual.
pub(crate) const MAX_BYTES: usize = 1024;
/// Calls of a function after which its hit rate is judged.
const WARM: u32 = 64;
/// Calls that must repeat one made before, out of every `RATE`, for the memo to stay on.
const RATE: u32 = 8;
/// Processes that give a function's memo up before it is off for all of them (fewer when fewer
/// processes call it: off once every caller gave it up, #762).
const VOTES: u32 = 3;
/// Calls run to their end from inside one another (a pure function that calls itself): deeper
/// calls are made as usual, so the native stack stays small.
pub(crate) const NEST: u8 = 8;

static HITS: AtomicU64 = AtomicU64::new(0);
static MISSES: AtomicU64 = AtomicU64::new(0);

static OFF: AtomicU64 = AtomicU64::new(0);

/// Functions whose memo is off for every process.
pub fn pmemo_off() -> u64 {
    OFF.load(Ordering::Relaxed)
}

/// Calls answered from a memo (by the host or compiled code), and calls run and remembered,
/// over all processes.
pub fn pmemo_stats() -> (u64, u64) {
    (
        HITS.load(Ordering::Relaxed) + pmemo_inline_hits(),
        MISSES.load(Ordering::Relaxed),
    )
}

/// Whether the memo is on (`YAVS_NO_PMEMO=1` turns it off).
pub(crate) fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("YAVS_NO_PMEMO").is_none())
}

/// What a call returned.
pub(crate) enum Val {
    Word(u64),
    /// The descriptor bytes, then the element bytes of an array result.
    Array(Box<[u8]>),
}

/// One remembered call, laid out for compiled code too ([`Machine::memo_view`]): it reads
/// `hash`, `word`, `klen`, `vlen` and the bytes at `at`, never writes.
#[repr(C)]
pub struct Slot {
    hash: u64,
    /// A scalar result (`vlen` 0).
    word: u64,
    /// Bytes of the key, then of an array result (descriptor, then elements), in `bytes`.
    klen: u64,
    vlen: u64,
    /// The address of `bytes`.
    at: usize,
    bytes: Box<[u8]>,
}

impl Slot {
    fn new(hash: u64, key: &[u8], val: Val) -> Slot {
        let (word, v): (u64, &[u8]) = match &val {
            Val::Word(w) => (*w, &[]),
            Val::Array(b) => (0, b),
        };
        let bytes: Box<[u8]> = [key, v].concat().into();
        Slot {
            hash,
            word,
            klen: key.len() as u64,
            vlen: v.len() as u64,
            at: bytes.as_ptr() as usize,
            bytes,
        }
    }

    fn key(&self) -> &[u8] {
        &self.bytes[..self.klen as usize]
    }

    fn val(&self) -> ValRef<'_> {
        match self.vlen {
            0 => ValRef::Word(self.word),
            _ => ValRef::Array(&self.bytes[self.klen as usize..]),
        }
    }
}

/// A remembered result.
enum ValRef<'a> {
    Word(u64),
    Array(&'a [u8]),
}

/// Entries of a process's directory of the tables compiled code probes inline, by function
/// number (a power of two).
pub const MEMO_DIR: usize = 256;
/// `MemoEnt::func` of an empty entry.
pub const MEMO_FREE: u64 = u64::MAX;

/// The tables of a process compiled code probes by itself (`TierFrame::pmemo`, docs/decisions.md
/// "Memo of pure calls: the probe in compiled code"): written by the host only.
#[repr(C)]
pub struct MemoDir {
    /// The address of the count of functions off for all processes, and its value when the
    /// entries were made: compiled code goes to the host while they differ.
    pub off_at: u64,
    pub off_seen: AtomicU64,
    /// Calls compiled code answered from these tables.
    pub hits: AtomicU64,
    _pad: u64,
    pub ents: [MemoEnt; MEMO_DIR],
}

/// A function's table in a [`MemoDir`], at `func % MEMO_DIR`.
#[repr(C)]
pub struct MemoEnt {
    /// The function, [`MEMO_FREE`] for none.
    pub func: AtomicU64,
    /// The slots (`Option<Box<Slot>>`: an address or null) and their number less one.
    pub tab: AtomicU64,
    pub mask: AtomicU64,
    _pad: u64,
}

/// Byte offsets compiled code reads at.
pub const MEMO_DIR_OFF_AT: usize = std::mem::offset_of!(MemoDir, off_at);
pub const MEMO_DIR_OFF_SEEN: usize = std::mem::offset_of!(MemoDir, off_seen);
pub const MEMO_DIR_HITS: usize = std::mem::offset_of!(MemoDir, hits);
pub const MEMO_DIR_ENTS: usize = std::mem::offset_of!(MemoDir, ents);
pub const MEMO_ENT_SIZE: usize = std::mem::size_of::<MemoEnt>();
pub const MEMO_ENT_FUNC: usize = std::mem::offset_of!(MemoEnt, func);
pub const MEMO_ENT_TAB: usize = std::mem::offset_of!(MemoEnt, tab);
pub const MEMO_ENT_MASK: usize = std::mem::offset_of!(MemoEnt, mask);
pub const MEMO_SLOT_HASH: usize = std::mem::offset_of!(Slot, hash);
pub const MEMO_SLOT_WORD: usize = std::mem::offset_of!(Slot, word);
pub const MEMO_SLOT_KLEN: usize = std::mem::offset_of!(Slot, klen);
pub const MEMO_SLOT_VLEN: usize = std::mem::offset_of!(Slot, vlen);
pub const MEMO_SLOT_AT: usize = std::mem::offset_of!(Slot, at);

/// The directories of live processes, and the hits of those dropped: `pmemo_stats` counts both
/// (the counters are printed while the processes live).
static DIRS: Mutex<Vec<Weak<MemoDir>>> = Mutex::new(Vec::new());
static GONE: AtomicU64 = AtomicU64::new(0);

impl Drop for MemoDir {
    fn drop(&mut self) {
        GONE.fetch_add(*self.hits.get_mut(), Ordering::Relaxed);
    }
}

impl MemoDir {
    fn new() -> Arc<MemoDir> {
        let d = Arc::new(MemoDir {
            off_at: std::ptr::addr_of!(OFF) as u64,
            off_seen: AtomicU64::new(OFF.load(Ordering::Relaxed)),
            hits: AtomicU64::new(0),
            _pad: 0,
            ents: std::array::from_fn(|_| MemoEnt {
                func: AtomicU64::new(MEMO_FREE),
                tab: AtomicU64::new(0),
                mask: AtomicU64::new(0),
                _pad: 0,
            }),
        });
        let mut ds = DIRS.lock().unwrap_or_else(PoisonError::into_inner);
        ds.retain(|w| w.strong_count() > 0);
        ds.push(Arc::downgrade(&d));
        d
    }
}

/// Calls compiled code answered from a memo by itself (part of `pmemo_stats`'s hits).
pub fn pmemo_inline_hits() -> u64 {
    let ds = DIRS.lock().unwrap_or_else(PoisonError::into_inner);
    GONE.load(Ordering::Relaxed)
        + ds.iter()
            .filter_map(Weak::upgrade)
            .map(|d| d.hits.load(Ordering::Relaxed))
            .sum::<u64>()
}

/// The memo of one function.
pub(crate) struct Memo {
    pub func: u32,
    slots: Vec<Option<Box<Slot>>>,
    calls: u32,
    hits: u32,
    /// Entries replaced by another key since the table last grew.
    evicted: usize,
    off: bool,
    /// The table is in the directory, at this entry.
    shown: Option<usize>,
}

/// The memo state of a process: absent until its first call of a memoised function.
#[derive(Default)]
pub(crate) struct State {
    memos: Vec<Memo>,
    /// The function of the last lookup and the index of its memo in `memos`.
    last: Option<(u32, usize)>,
    /// The key being built (reused).
    pub key: Vec<u8>,
    /// Calls run to their end that are still running.
    pub depth: u8,
    /// The frames of those calls: compiled code does not run in them (it would return into the
    /// caller's frame without telling the loop that waits for the call to end).
    pub frames: Vec<usize>,
    /// Reports made so far: a call during which it moved is not remembered.
    pub reports: u64,
    /// The hash of the key in `key` after a lookup that missed.
    pub miss_hash: u64,
    /// The last lookup made the memo of its function in this process (its first call).
    pub fresh: bool,
    /// The tables compiled code probes by itself, made with the first one shown.
    dir: Option<Arc<MemoDir>>,
}

/// A hash of `key`: eight bytes a step.
fn hash(key: &[u8]) -> u64 {
    const K: u64 = 0x517c_c1b7_2722_0a95;
    let (words, rest) = key.as_chunks::<8>();
    let mut h = key.len() as u64;
    for w in words {
        h = (h.rotate_left(5) ^ u64::from_le_bytes(*w)).wrapping_mul(K);
    }
    let mut tail = [0u8; 8];
    tail[..rest.len()].copy_from_slice(rest);
    (h.rotate_left(5) ^ u64::from_le_bytes(tail)).wrapping_mul(K)
}

/// The answer to a lookup.
pub(crate) enum Look {
    /// The memo of the function is off.
    Off,
    /// The memo of the function has just been turned off in this process.
    GaveUp,
    /// A call with this key was made: slot `.0` holds its result.
    Hit(usize),
    /// None was: remember the call when it ends ([`State::keep`]).
    Miss,
}

impl State {
    /// The index of the memo of `func`, made when the process first calls it.
    fn index(&mut self, func: u32) -> usize {
        if let Some((f, i)) = self.last
            && f == func
        {
            return i;
        }
        let i = match self.memos.iter().position(|m| m.func == func) {
            Some(i) => i,
            None => {
                self.memos.push(Memo {
                    func,
                    slots: Vec::new(),
                    calls: 0,
                    hits: 0,
                    evicted: 0,
                    off: false,
                    shown: None,
                });
                self.fresh = true;
                self.memos.len() - 1
            }
        };
        self.last = Some((func, i));
        i
    }

    /// The memo of `func`, made when the process first calls it.
    fn memo(&mut self, func: u32) -> &mut Memo {
        let i = self.index(func);
        &mut self.memos[i]
    }

    /// Whether this process gave the memo of `func` up.
    pub fn is_off(&self, func: u32) -> bool {
        match self.last {
            Some((f, i)) if f == func => self.memos[i].off,
            _ => self.memos.iter().any(|m| m.func == func && m.off),
        }
    }

    /// A call of the function of memo `at` whose key could not be made (over `MAX_BYTES`):
    /// counted like a miss, so a function whose calls never fit is given up too. True when it
    /// has just been.
    fn unkeyed(&mut self, at: usize) -> bool {
        let m = &mut self.memos[at];
        m.calls += 1;
        let give_up = !m.off && m.calls == WARM && m.hits < WARM / RATE;
        if give_up {
            m.off = true;
            m.slots = Vec::new();
        }
        give_up
    }

    /// Look the call of the function of memo `at` with the key in `self.key` up. Counts the
    /// call, and judges the function after `WARM` calls.
    fn look(&mut self, at: usize) -> Look {
        let hash = hash(&self.key);
        let key = &self.key;
        let m = &mut self.memos[at];
        if m.off {
            Look::Off
        } else {
            if m.slots.is_empty() {
                m.slots.resize_with(FIRST, || None);
            }
            let i = hash as usize & (m.slots.len() - 1);
            m.calls += 1;
            if m.slots[i]
                .as_ref()
                .is_some_and(|s| s.hash == hash && s.key() == &key[..])
            {
                m.hits += 1;
                HITS.fetch_add(1, Ordering::Relaxed);
                Look::Hit(i)
            } else if m.calls == WARM && m.hits < WARM / RATE {
                m.off = true;
                m.slots = Vec::new();
                Look::GaveUp
            } else {
                self.miss_hash = hash;
                Look::Miss
            }
        }
    }

    /// The result in slot `i` of memo `at` (from [`Look::Hit`]).
    fn val(&self, at: usize, i: usize) -> Option<ValRef<'_>> {
        self.memos.get(at)?.slots.get(i)?.as_ref().map(|s| s.val())
    }

    /// Show the table of memo `at` to compiled code, once its function has been judged and kept
    /// (`WARM` calls: the hits compiled code makes by itself are not counted for the judgement).
    /// A function whose entry another holds stays with the host.
    fn show(&mut self, at: usize) {
        let m = &mut self.memos[at];
        if m.shown.is_some() || m.off || m.calls < WARM || m.slots.is_empty() {
            return;
        }
        let d = self.dir.get_or_insert_with(MemoDir::new);
        // ponytail: one function per entry, first come; a probed chain if hot functions collide.
        let k = m.func as usize % MEMO_DIR;
        let e = &d.ents[k];
        let f = e.func.load(Ordering::Relaxed);
        if f != MEMO_FREE && f != u64::from(m.func) {
            return;
        }
        e.tab.store(m.slots.as_ptr() as u64, Ordering::Relaxed);
        e.mask.store(m.slots.len() as u64 - 1, Ordering::Relaxed);
        e.func.store(u64::from(m.func), Ordering::Relaxed);
        m.shown = Some(k);
    }

    /// The table of memo `at` moved (it grew): its entry follows it.
    fn reshow(&mut self, at: usize) {
        let m = &self.memos[at];
        if let (Some(k), Some(d)) = (m.shown, &self.dir) {
            d.ents[k]
                .tab
                .store(m.slots.as_ptr() as u64, Ordering::Relaxed);
            d.ents[k]
                .mask
                .store(m.slots.len() as u64 - 1, Ordering::Relaxed);
        }
    }

    /// Some function went off for all processes since the directory was made: empty it (each
    /// function is shown again at its next host probe if it is still on).
    pub fn recheck_off(&mut self) {
        let Some(d) = &self.dir else {
            return;
        };
        let now = OFF.load(Ordering::Relaxed);
        if d.off_seen.load(Ordering::Relaxed) == now {
            return;
        }
        for e in &d.ents {
            e.func.store(MEMO_FREE, Ordering::Relaxed);
        }
        d.off_seen.store(now, Ordering::Relaxed);
        for m in &mut self.memos {
            m.shown = None;
        }
    }

    /// The directory's address for compiled code, 0 for none (or while calls run `NEST` deep).
    pub fn dir_addr(&self) -> usize {
        match &self.dir {
            Some(d) if self.depth < NEST => Arc::as_ptr(d) as usize,
            _ => 0,
        }
    }

    /// Remember a call that ended with `val` (`hash` and `key` from [`Look::Miss`] and the
    /// call). The slot is found again: calls run in between may have grown the table.
    pub fn keep(&mut self, func: u32, hash: u64, key: &[u8], val: Val) {
        let m = self.memo(func);
        if m.slots.is_empty() {
            return;
        }
        let i = hash as usize & (m.slots.len() - 1);
        if m.slots[i].is_some() {
            m.evicted += 1;
        }
        m.slots[i] = Some(Box::new(Slot::new(hash, key, val)));
        if m.evicted >= m.slots.len() && m.slots.len() < LAST {
            m.grow();
            let at = self.index(func);
            self.reshow(at);
        }
        MISSES.fetch_add(1, Ordering::Relaxed);
    }
}

/// What [`Machine::memo_run`] did.
pub(crate) enum Done {
    /// The call is made as usual (too deep inside others run this way).
    No,
    /// The result is in the destination register.
    Yes,
    /// A report at the stop severity ended the process inside the call (its frames stay).
    Suspend,
}

/// What [`Machine::memo_call`] did.
pub(crate) enum Memoed {
    /// The loop makes the call as usual.
    No,
    /// The result is in the destination register: go on after the call.
    Hit,
    /// The compiled tier made the call: its result is in the destination register and the frame
    /// goes on after it as the tier call leaves it.
    Tiered,
    /// What the loop returns (a frame pushed, a suspension).
    Flow(Flow),
}

/// What [`Machine::memo_probe`] found.
pub(crate) enum Probe {
    /// The call is made as usual and not kept.
    No,
    /// The call was made before: its result (a fat pair in temporary memory for an array).
    Hit(u64),
    /// It was not: make it, then keep its result with [`Machine::memo_keep`] unless the number
    /// of reports (`.0`) has moved.
    Miss(u64),
}

impl Machine {
    /// The memo's answer for the call of `func` whose argument cells are `cell(self, i)`, for
    /// `n` cells. Runs no VHDL code. `No` where the memo cannot be used: elaboration-time code
    /// (`sim` false), a debugger or coverage, a function the memo gave up on, a key over
    /// `MAX_BYTES`, or a call inside `NEST` others being run this way.
    #[inline(always)]
    pub(crate) fn memo_probe(
        &mut self,
        prog: &Program,
        func: FuncId,
        sig: &MemoSig,
        n: usize,
        cell: impl Fn(&Machine, usize) -> u64,
        sim: bool,
    ) -> R<Probe> {
        if !sim || !enabled() || self.has_debugger() || !self.rt.cov.is_empty() {
            return Ok(Probe::No);
        }
        if n != sig.cells as usize {
            return Ok(Probe::No);
        }
        let mut st = self.pm.take().unwrap_or_default();
        if st.depth >= NEST {
            self.pm = Some(st);
            return Ok(Probe::No);
        }
        // (A memo made here is on: `fresh` is read after the key below.)
        let at = st.index(func.0);
        if st.memos[at].off {
            self.pm = Some(st);
            return Ok(Probe::No);
        }
        let mut key = std::mem::take(&mut st.key);
        key.clear();
        let made = self.memo_key(sig, n, &cell, &mut key);
        st.key = key;
        if !made {
            // A key too big, or one that cannot be read, counts as a call that did not repeat.
            let gave_up = st.unkeyed(at);
            let fresh = std::mem::take(&mut st.fresh);
            self.pm = Some(st);
            if fresh {
                self.memo_user(prog, func);
            }
            if gave_up {
                self.memo_vote(prog, func);
            }
            return Ok(Probe::No);
        }
        let look = st.look(at);
        if std::mem::take(&mut st.fresh) {
            self.memo_user(prog, func);
        }
        if matches!(look, Look::Hit(_) | Look::Miss) {
            st.show(at);
            self.pmemo_at = st.dir_addr();
        }
        match look {
            Look::Off => {
                self.pm = Some(st);
                Ok(Probe::No)
            }
            Look::GaveUp => {
                self.pm = Some(st);
                self.memo_vote(prog, func);
                Ok(Probe::No)
            }
            Look::Hit(i) => {
                let r = match st.val(at, i) {
                    Some(ValRef::Word(v)) => Ok(v),
                    Some(ValRef::Array(b)) => match sig.ret {
                        MemoArg::Array(a) => self.memo_array(a, b),
                        MemoArg::Flat(a) => self.memo_flat(a, b),
                        MemoArg::Scalar => Err(cold(|| ice("memo: an array for a scalar result"))),
                    },
                    None => Err(cold(|| ice("memo: a hit on an empty slot"))),
                };
                self.pm = Some(st);
                Ok(Probe::Hit(r?))
            }
            Look::Miss => {
                let reports = st.reports;
                self.pm = Some(st);
                Ok(Probe::Miss(reports))
            }
        }
    }

    /// Whether `v` is what the memo holds for the call of `func` with argument `cells`, looked
    /// up without counting: the check of a hit compiled code made by itself (debug builds). An
    /// array result is the data address, its bounds in `ret_bounds`.
    pub(crate) fn memo_holds(&self, func: FuncId, sig: &MemoSig, cells: &[u64], v: u64) -> bool {
        let Some(st) = self.pm.as_ref() else {
            return false;
        };
        let mut key = Vec::new();
        if !self.memo_key(sig, cells.len(), &|_, i| cells[i], &mut key) {
            return false;
        }
        let h = hash(&key);
        let Some(m) = st.memos.iter().find(|m| m.func == func.0) else {
            return false;
        };
        let Some(Some(s)) = m.slots.get(h as usize & m.slots.len().wrapping_sub(1)) else {
            return false;
        };
        if s.hash != h || s.key() != &key[..] {
            return false;
        }
        match s.val() {
            ValRef::Word(w) => w == v,
            ValRef::Array(b) => {
                let (d, e) = b.split_at(24);
                let bounds: Vec<u8> = self
                    .ret_bounds()
                    .iter()
                    .flat_map(|x| x.to_le_bytes())
                    .collect();
                bounds == d && self.rd(v, e.len() as u64).is_ok_and(|x| x == e)
            }
        }
    }

    /// Keep the result `v` of a call whose [`Probe::Miss`] saw `reports` reports, if none ran
    /// since. The key is made again from the argument cells (the arguments are `in`, so their
    /// bytes are as the probe read them).
    pub(crate) fn memo_keep(
        &mut self,
        func: FuncId,
        sig: &MemoSig,
        n: usize,
        cell: impl Fn(&Machine, usize) -> u64,
        reports: u64,
        v: u64,
    ) {
        let Some(mut st) = self.pm.take() else {
            return;
        };
        if st.reports == reports && !st.is_off(func.0) {
            let mut key = std::mem::take(&mut st.key);
            key.clear();
            if self.memo_key(sig, n, &cell, &mut key)
                && let Some(val) = self.memo_result(sig.ret, v)
            {
                st.keep(func.0, hash(&key), &key, val);
            }
            st.key = key;
        }
        self.pm = Some(st);
    }

    /// `Inst::Call` of a function with a memo signature, from the interpreter's fast loop: the
    /// memo's answer, else the call made by the compiled tier when it takes the call
    /// (`Tier::call`) and its result kept, else run to its end (`memo_run`) and kept.
    /// [`Memoed::No`]: the loop makes the call (and offers it to the tier) as usual.
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    pub(crate) fn memo_call(
        &mut self,
        prog: &Program,
        rt: &Runtime,
        tier: Option<&dyn Tier>,
        f: &Func,
        func: FuncId,
        cf: &Func,
        sig: &MemoSig,
        args: Regs,
        dst: Option<Reg>,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<Memoed> {
        let reports = match self.memo_probe_regs(prog, f, func, sig, args, dst, ctx)? {
            Probe::No => return Ok(Memoed::No),
            Probe::Hit(v) => {
                if let Some(d) = dst {
                    self.set(d, v);
                }
                return Ok(Memoed::Hit);
            }
            Probe::Miss(reports) => reports,
        };
        let Some(dst) = dst else {
            return Ok(Memoed::No);
        };
        if let Some(t) = tier
            && rt.native.callable(func.0)
            && let Some(flow) = self.tier_call(t, rt, f, func, cf, args, Some(dst), None, ctx)?
        {
            if let Flow::Next = flow {
                let v = self.r(dst);
                self.memo_keep_regs(f, func, sig, args, reports, v);
                return Ok(Memoed::Tiered);
            }
            return Ok(Memoed::Flow(flow));
        }
        Ok(match self.memo_run(prog, f, func, sig, args, dst, ctx)? {
            Done::Yes => Memoed::Hit,
            Done::Suspend => Memoed::Flow(Flow::Suspend),
            Done::No => Memoed::No,
        })
    }

    /// The probe of the call of `func` at `args` of `f` for the interpreter's fast loop, which
    /// then makes the call itself (compiled callee or not) and gives its result to
    /// [`Machine::memo_keep_regs`].
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    pub(crate) fn memo_probe_regs(
        &mut self,
        prog: &Program,
        f: &Func,
        func: FuncId,
        sig: &MemoSig,
        args: Regs,
        dst: Option<Reg>,
        ctx: &Option<&mut Ctx<'_>>,
    ) -> R<Probe> {
        if dst.is_none() {
            return Ok(Probe::No);
        }
        let regs = f
            .regs(args)
            .ok_or_else(|| cold(|| ice("operand list out of range")))?;
        self.memo_probe(
            prog,
            func,
            sig,
            regs.len(),
            |m, i| m.r(regs[i]),
            ctx.is_some(),
        )
    }

    /// [`Machine::memo_keep`] for the call at `args` of `f` whose result is `v`.
    pub(crate) fn memo_keep_regs(
        &mut self,
        f: &Func,
        func: FuncId,
        sig: &MemoSig,
        args: Regs,
        reports: u64,
        v: u64,
    ) {
        if let Some(regs) = f.regs(args) {
            self.memo_keep(func, sig, regs.len(), |m, i| m.r(regs[i]), reports, v);
        }
    }

    /// Run the call to its end from here and keep its result (after a [`Probe::Miss`]).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn memo_run(
        &mut self,
        prog: &Program,
        f: &Func,
        func: FuncId,
        sig: &MemoSig,
        args: Regs,
        dst: Reg,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<Done> {
        let mut st = self.pm.take().unwrap_or_default();
        if st.depth >= NEST {
            self.pm = Some(st);
            return Ok(Done::No);
        }
        // The key and hash the probe just left (nothing ran since: `Tier::call` made no call).
        let (key, hash) = (std::mem::take(&mut st.key), st.miss_hash);
        let reports = st.reports;
        st.depth += 1;
        st.frames.push(self.frames.len());
        self.pmemo_at = st.dir_addr();
        self.pm = Some(st);
        let r = self.call_to_end(prog, f, func, args, Some(dst), ctx);
        let mut st = self.pm.take().unwrap_or_default();
        st.depth = st.depth.saturating_sub(1);
        st.frames.pop();
        self.pmemo_at = st.dir_addr();
        let out = match r {
            Ok(Outcome::Returned(Some(v))) => {
                if st.reports == reports
                    && let Some(val) = self.memo_result(sig.ret, v)
                {
                    st.keep(func.0, hash, &key, val);
                }
                self.set(dst, v);
                Ok(Done::Yes)
            }
            Ok(Outcome::Returned(None)) => Err(cold(|| ice("function returned no value"))),
            Ok(Outcome::Suspended) => Ok(Done::Suspend),
            Err(e) => Err(e),
        };
        st.key = key;
        self.pm = Some(st);
        out
    }

    /// This process gave the memo of `func` up: when `VOTES` processes have, or every process that
    /// called it so far, it is off for all. A compiled caller then skips the probe's work too (#762).
    #[cold]
    fn memo_vote(&self, prog: &Program, func: FuncId) {
        if let Some(c) = prog.func(func)
            && let Some(pre) = self.rt.pre(func, c)
            && pre.memo_votes.fetch_add(1, Ordering::Relaxed) + 1
                >= VOTES.min(pre.memo_users.load(Ordering::Relaxed))
            && !pre.memo_off.swap(true, Ordering::Relaxed)
        {
            OFF.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// This process called `func` through the memo for the first time.
    #[cold]
    fn memo_user(&self, prog: &Program, func: FuncId) {
        if let Some(c) = prog.func(func)
            && let Some(pre) = self.rt.pre(func, c)
        {
            pre.memo_users.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The key of a call: the argument words, and for an array its descriptor and element
    /// bytes. `false` when the key would pass `MAX_BYTES` or an argument cannot be read (the
    /// call is then made as usual and reports what is wrong).
    fn memo_key(
        &self,
        sig: &MemoSig,
        n: usize,
        cell: &impl Fn(&Machine, usize) -> u64,
        key: &mut Vec<u8>,
    ) -> bool {
        // (The callers checked that the call has `sig.cells` cells.)
        if n != sig.cells as usize {
            return false;
        }
        let mut at = 0;
        for arg in &sig.args {
            match *arg {
                MemoArg::Scalar => {
                    key.extend_from_slice(&cell(self, at).to_le_bytes());
                    at += 1;
                }
                MemoArg::Flat(a) => {
                    let data = cell(self, at);
                    let mut d = [0u8; 24];
                    for k in 0..3 {
                        d[k * 8..k * 8 + 8].copy_from_slice(&cell(self, at + 1 + k).to_le_bytes());
                    }
                    at += 4;
                    let Some(n) = a.data_bytes(&d) else {
                        return false;
                    };
                    if key.len() as u64 + n + 24 > MAX_BYTES as u64 {
                        return false;
                    }
                    let Ok(b) = self.rd(data, n) else {
                        return false;
                    };
                    key.extend_from_slice(&d);
                    key.extend_from_slice(b);
                }
                MemoArg::Array(a) => {
                    let (data, dope) = (cell(self, at), cell(self, at + 1));
                    at += 2;
                    let Ok(d) = self.rd(dope, a.desc_bytes() as u64) else {
                        return false;
                    };
                    let Some(n) = a.data_bytes(d) else {
                        return false;
                    };
                    if key.len() as u64 + n + d.len() as u64 > MAX_BYTES as u64 {
                        return false;
                    }
                    let Ok(b) = self.rd(data, n) else {
                        return false;
                    };
                    key.extend_from_slice(d);
                    key.extend_from_slice(b);
                }
            }
        }
        key.len() <= MAX_BYTES
    }

    /// The result `v` of a call, as the memo keeps it; `None` when it cannot be read or is
    /// over `MAX_BYTES`.
    fn memo_result(&self, ret: MemoArg, v: u64) -> Option<Val> {
        if let MemoArg::Flat(a) = ret {
            let d: Vec<u8> = (self.ret_bounds().iter())
                .flat_map(|x| x.to_le_bytes())
                .collect();
            let n = a.data_bytes(&d)?;
            if n + d.len() as u64 > MAX_BYTES as u64 {
                return None;
            }
            let mut out = d;
            out.extend_from_slice(self.rd(v, n).ok()?);
            return Some(Val::Array(out.into()));
        }
        let MemoArg::Array(a) = ret else {
            return Some(Val::Word(v));
        };
        let fat = self.rd(v, 16).ok()?;
        let (data, dope) = (
            u64::from_le_bytes(fat[..8].try_into().ok()?),
            u64::from_le_bytes(fat[8..].try_into().ok()?),
        );
        let d = self.rd(dope, a.desc_bytes() as u64).ok()?;
        let n = a.data_bytes(d)?;
        if n + d.len() as u64 > MAX_BYTES as u64 {
            return None;
        }
        let mut out = Vec::with_capacity(d.len() + n as usize);
        out.extend_from_slice(d);
        out.extend_from_slice(self.rd(data, n).ok()?);
        Some(Val::Array(out.into()))
    }

    /// An array result `bytes` (descriptor, then elements) as a function with `Func::ret_flat`
    /// returns it: its data address.
    fn memo_flat(&mut self, a: MemoArr, bytes: &[u8]) -> R<u64> {
        let n = a.desc_bytes();
        let (d, e) = bytes.split_at(n);
        let w = |k: usize| u64::from_le_bytes(d[k * 8..k * 8 + 8].try_into().unwrap_or([0; 8]));
        self.set_ret_bounds([w(0), w(1), w(2)]);
        self.temp_bytes(e)
    }

    /// An array result `bytes` (descriptor, then elements) as a fat pair in temporary memory,
    /// as a function returns it; the address of the pair.
    fn memo_array(&mut self, a: MemoArr, bytes: &[u8]) -> R<u64> {
        let fat = self.temp_alloc(16 + bytes.len() as u64)?;
        let dope = add_addr(fat, 16)?;
        let data = add_addr(fat, 16 + a.desc_bytes() as u64)?;
        let out = self.wr(fat, 16 + bytes.len() as u64)?;
        out[..8].copy_from_slice(&data.to_le_bytes());
        out[8..16].copy_from_slice(&dope.to_le_bytes());
        out[16..].copy_from_slice(bytes);
        Ok(fat)
    }
}

impl Memo {
    /// Twice the slots, the entries in the places their hashes give.
    #[cold]
    fn grow(&mut self) {
        let n = self.slots.len() * 2;
        let old = std::mem::take(&mut self.slots);
        self.slots.resize_with(n, || None);
        for s in old.into_iter().flatten() {
            let i = s.hash as usize & (n - 1);
            self.slots[i] = Some(s);
        }
        self.evicted = 0;
    }
}

impl Machine {
    /// Whether frame `top` is the frame of a call the memo runs to its end.
    #[inline(never)]
    pub(crate) fn is_memo_frame(&self, top: usize) -> bool {
        self.pm.as_ref().is_some_and(|p| p.frames.contains(&top))
    }
}
