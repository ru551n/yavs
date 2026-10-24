//! The memo of pure calls (#701): the results of the calls a process made of the functions
//! `Program::memo_sig` allows, kept by the bytes of their arguments, so a call with arguments
//! it has seen before is answered without running the function (`Machine::memo_call`).
//!
//! One memo per function and process (no locks, and the same hits on every thread count): a
//! direct-mapped table of `SLOTS` entries, each the key (the argument words and, for an array,
//! its descriptor and element bytes), a hash of it and the result. A key is compared whole, so
//! a hit is exact. The first `WARM` calls of a function decide whether it is kept: a function
//! whose calls rarely repeat costs the key and the entry for nothing, and is switched off for
//! the process.

use std::sync::atomic::{AtomicU64, Ordering};

use yavs_ir::{Func, FuncId, MemoArg, MemoArr, MemoSig, Program, Reg, Regs};
use yavs_kernel::Ctx;

use crate::exec::{Machine, Outcome, R, add_addr, cold, ice};

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
/// Processes that give a function's memo up before it is off for all of them.
const VOTES: u32 = 3;
/// Calls run to their end from inside one another (a pure function that calls itself): deeper
/// calls are made as usual, so the native stack stays small.
pub(crate) const NEST: u8 = 8;

static HITS: AtomicU64 = AtomicU64::new(0);
static MISSES: AtomicU64 = AtomicU64::new(0);

/// Calls answered from a memo, and calls run and remembered, over all processes.
pub fn pmemo_stats() -> (u64, u64) {
    (HITS.load(Ordering::Relaxed), MISSES.load(Ordering::Relaxed))
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

struct Slot {
    hash: u64,
    key: Box<[u8]>,
    val: Val,
}

/// The memo of one function.
pub(crate) struct Memo {
    pub func: u32,
    slots: Vec<Option<Slot>>,
    calls: u32,
    hits: u32,
    /// Entries replaced by another key since the table last grew.
    evicted: usize,
    off: bool,
}

/// The memo state of a process: absent until its first call of a memoised function.
#[derive(Default)]
pub(crate) struct State {
    memos: Vec<Memo>,
    /// The key being built (reused).
    pub key: Vec<u8>,
    /// Calls run to their end that are still running.
    pub depth: u8,
    /// Reports made so far: a call during which it moved is not remembered.
    pub reports: u64,
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
    /// None was: remember the call when it ends ([`State::keep`]); `.0` is the hash of the key.
    Miss(u64),
}

impl State {
    /// The memo of `func`, made when the process first calls it.
    fn memo(&mut self, func: u32) -> &mut Memo {
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
                });
                self.memos.len() - 1
            }
        };
        &mut self.memos[i]
    }

    /// Whether this process gave the memo of `func` up.
    pub fn is_off(&self, func: u32) -> bool {
        self.memos.iter().any(|m| m.func == func && m.off)
    }

    /// Look the call of `func` with the key in `self.key` up. Counts the call, and judges the
    /// function after `WARM` calls.
    pub fn look(&mut self, func: u32) -> Look {
        let hash = hash(&self.key);
        let key = std::mem::take(&mut self.key);
        let m = self.memo(func);
        let look = if m.off {
            Look::Off
        } else {
            if m.slots.is_empty() {
                m.slots.resize_with(FIRST, || None);
            }
            let i = hash as usize & (m.slots.len() - 1);
            m.calls += 1;
            if m.slots[i]
                .as_ref()
                .is_some_and(|s| s.hash == hash && *s.key == *key)
            {
                m.hits += 1;
                HITS.fetch_add(1, Ordering::Relaxed);
                Look::Hit(i)
            } else if m.calls == WARM && m.hits < WARM / RATE {
                m.off = true;
                m.slots = Vec::new();
                Look::GaveUp
            } else {
                Look::Miss(hash)
            }
        };
        self.key = key;
        look
    }

    /// The result in slot `i` of `func` (from [`Look::Hit`]).
    pub fn val(&mut self, func: u32, i: usize) -> Option<&Val> {
        self.memo(func).slots.get(i)?.as_ref().map(|s| &s.val)
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
        m.slots[i] = Some(Slot {
            hash,
            key: key.into(),
            val,
        });
        if m.evicted >= m.slots.len() && m.slots.len() < LAST {
            m.grow();
        }
        MISSES.fetch_add(1, Ordering::Relaxed);
    }
}

/// What [`Machine::memo_call`] did.
pub(crate) enum Done {
    /// The call is made as usual.
    No,
    /// The result is in the destination register.
    Yes,
    /// A report at the stop severity ended the process inside the call (its frames stay).
    Suspend,
}

impl Machine {
    /// `Inst::Call` of a function with a memo signature: the result of an earlier call with the
    /// same arguments, or the call run to its end and its result kept. The call is made as usual
    /// (`Done::No`) when the memo cannot be used: elaboration-time code, a debugger or coverage,
    /// a key or a result over `MAX_BYTES`, a function the memo gave up on, or a call inside
    /// `NEST` others being run this way.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn memo_call(
        &mut self,
        prog: &Program,
        f: &Func,
        func: FuncId,
        sig: &MemoSig,
        args: Regs,
        dst: Option<Reg>,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<Done> {
        let Some(dst) = dst else {
            return Ok(Done::No);
        };
        if ctx.is_none() || !enabled() || self.has_debugger() || !self.rt.cov.is_empty() {
            return Ok(Done::No);
        }
        let regs = f
            .regs(args)
            .ok_or_else(|| cold(|| ice("operand list out of range")))?;
        if regs.len() != sig.cells as usize {
            return Ok(Done::No);
        }
        let mut st = self.pm.take().unwrap_or_default();
        if st.depth >= NEST || st.is_off(func.0) {
            self.pm = Some(st);
            return Ok(Done::No);
        }
        let mut key = std::mem::take(&mut st.key);
        key.clear();
        let made = self.memo_key(sig, regs, &mut key);
        st.key = key;
        if !made {
            self.pm = Some(st);
            return Ok(Done::No);
        }
        let hash = match st.look(func.0) {
            Look::Off => {
                self.pm = Some(st);
                return Ok(Done::No);
            }
            Look::GaveUp => {
                self.pm = Some(st);
                self.memo_vote(prog, func);
                return Ok(Done::No);
            }
            Look::Hit(i) => {
                let r = match st.val(func.0, i) {
                    Some(Val::Word(v)) => Ok(*v),
                    Some(Val::Array(b)) => match sig.ret {
                        MemoArg::Array(a) => self.memo_array(a, b),
                        MemoArg::Scalar => Err(cold(|| ice("memo: an array for a scalar result"))),
                    },
                    None => Err(cold(|| ice("memo: a hit on an empty slot"))),
                };
                self.pm = Some(st);
                self.set(dst, r?);
                return Ok(Done::Yes);
            }
            Look::Miss(h) => h,
        };
        // Run the call to its end from here, then keep its result.
        let key = std::mem::take(&mut st.key);
        let reports = st.reports;
        st.depth += 1;
        self.pm = Some(st);
        let r = self.call_to_end(prog, f, func, args, Some(dst), ctx);
        let mut st = self.pm.take().unwrap_or_default();
        st.depth = st.depth.saturating_sub(1);
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

    /// This process gave the memo of `func` up: when `VOTES` processes have, it is off for all.
    #[cold]
    fn memo_vote(&self, prog: &Program, func: FuncId) {
        if let Some(c) = prog.func(func)
            && let Some(pre) = self.rt.pre(func, c)
            && pre.memo_votes.fetch_add(1, Ordering::Relaxed) + 1 >= VOTES
        {
            pre.memo_off.store(true, Ordering::Relaxed);
        }
    }

    /// The key of a call: the argument words, and for an array its descriptor and element
    /// bytes. `false` when the key would pass `MAX_BYTES` or an argument cannot be read (the
    /// call is then made as usual and reports what is wrong).
    fn memo_key(&self, sig: &MemoSig, regs: &[Reg], key: &mut Vec<u8>) -> bool {
        let mut cells = regs.iter();
        for arg in &sig.args {
            match *arg {
                MemoArg::Scalar => {
                    let Some(&r) = cells.next() else {
                        return false;
                    };
                    key.extend_from_slice(&self.r(r).to_le_bytes());
                }
                MemoArg::Array(a) => {
                    let (Some(&data), Some(&dope)) = (cells.next(), cells.next()) else {
                        return false;
                    };
                    let Ok(d) = self.rd(self.r(dope), a.desc_bytes() as u64) else {
                        return false;
                    };
                    let Some(n) = a.data_bytes(d) else {
                        return false;
                    };
                    if key.len() as u64 + n + d.len() as u64 > MAX_BYTES as u64 {
                        return false;
                    }
                    key.extend_from_slice(d);
                    let Ok(b) = self.rd(self.r(data), n) else {
                        return false;
                    };
                    key.extend_from_slice(b);
                }
            }
            if key.len() > MAX_BYTES {
                return false;
            }
        }
        true
    }

    /// The result `v` of a call, as the memo keeps it; `None` when it cannot be read or is
    /// over `MAX_BYTES`.
    fn memo_result(&self, ret: MemoArg, v: u64) -> Option<Val> {
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
