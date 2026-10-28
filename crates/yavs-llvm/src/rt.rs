//! The runtime half of the compiled tiers: the frame view (`Mem`), the callbacks compiled code
//! calls (`yavs_rt_*`), `enter`, and the per-instruction facts both code generators share.
//! No code generator here: it builds without LLVM or Cranelift.

// The callbacks are `unsafe extern "C"` functions called only by generated code, which passes the
// `env` and `mem` pointers `enter` made: one safety contract for all of them, stated on `enter`.
#![allow(clippy::missing_safety_doc, clippy::not_unsafe_ptr_arg_deref)]

use std::any::Any;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use yavs_interp::{
    Called, HEAP_LOG, MemoLook, NativeCall, NativeFrame, Stepped, TIER_SUSPEND, TIER_UNWIND,
    TierFrame, TierHost, space,
};

use yavs_ir::{AddrSpace, FOp, Func, FuncId, FuncKind, Inst, IntOp, Program, Reg, Width};

use crate::census::{Cb, cb};
/// The code of `f<id>_c` (`Gen::adapter`): a native-frame function called from the interpreter
/// with its argument cells in an array.
pub type CallFn = unsafe extern "C" fn(*mut c_void, *mut Mem, u64, *const u64) -> i64;
pub const STEP: &str = "yavs_rt_step";
pub const HOST: &str = "yavs_rt_host";

pub const REGION: &str = "yavs_rt_region";

pub const SIGNAL: &str = "yavs_rt_signal";
pub const DRIVE: &str = "yavs_rt_drive";

pub const DRIVE_NAT: &str = "yavs_rt_drive_nat";

pub const WAIT: &str = "yavs_rt_wait";
pub const WAIT_ON: &str = "yavs_rt_wait_on";

pub const WAIT_AGAIN: &str = "yavs_rt_wait_again";
pub const WAIT_EDGE: &str = "yavs_rt_wait_edge";

pub const WAIT_SAME: &str = "yavs_rt_wait_same";
pub const LCOPY: &str = "yavs_rt_lcopy";
pub const LAYOUT: &str = "yavs_rt_layout";
pub const FAST: &str = "yavs_rt_fast";
pub const DEOPT: &str = "yavs_rt_deopt";
pub const DEOPT_SLOTS: &str = "yavs_rt_deopt_slots";
pub const DEOPT_END: &str = "yavs_rt_deopt_end";
pub const FAST_ALLOC: u32 = 0;
pub const FAST_DOPE_LEN: u32 = 1;
pub const FAST_SUBTYPE: u32 = 2;
pub const FAST_FAT_ROOT: u32 = 3;
pub const FAST_DEREF: u32 = 4;
pub const DRIVE_BIT: &str = "yavs_rt_drive_bit";

pub const LAYOUT_WORD: &str = "yavs_rt_layout_word";
pub const SCAN: &str = "yavs_rt_scan";
pub const ELOOP: &str = "yavs_rt_eloop";
pub const ELOOP_EXEC: &str = "yavs_rt_eloop_exec";

pub const CODES: &str = "yavs_rt_codes";

pub const SCALAR: &str = "yavs_rt_scalar";

pub const PLANES: &str = "yavs_rt_planes";
pub const MEMEQ: &str = "yavs_rt_memeq";
pub const NATIVE: &str = "yavs_rt_native";
/// The bytes `ieee::put_small` writes: a fat pair, a descriptor and one block of planes.
pub const PUT_SMALL_BYTES: u32 = 16 + yavs_ir::DOPE_DIM_BYTES as u32 + 32;
pub const NATIVE_LIGHT: &str = "yavs_rt_native_light";
pub const MEMO: &str = "yavs_rt_memo";
pub const MEMO_KEEP: &str = "yavs_rt_memo_keep";
pub const TROOT: &str = "yavs_rt_troot";
pub const ENDSTMT: &str = "yavs_rt_endstmt";
pub const XSTEP: &str = "yavs_rt_xstep";
pub const DRIVE_FLUSH: &str = "yavs_rt_drive_flush";

/// The most plane blocks an inline `LCopy` reads or writes (`yavs_rt_lcopy`'s stack copy).
pub const LCOPY_BLOCKS: u64 = 8;

pub type RunFn = unsafe extern "C" fn(*mut c_void, *mut u64, *mut Mem, u32) -> u32;

/// The compiled code of a function and its register count, if it has some.
pub type Lookup<'a> = &'a dyn Fn(FuncId) -> Option<(RunFn, usize)>;

/// `yavs_rt_step` returns it when compiled code goes on with the next instruction; anything
/// else is what the compiled function returns at once (a pc, `TIER_UNWIND`, `TIER_SUSPEND`).
pub const GO: u32 = u32::MAX;

/// Native stack compiled code may use for calls between compiled functions (`yavs_rt_step`):
/// deeper calls are left to the interpreter, whose frames are not on the native stack.
/// Simulation threads have 2 MiB.
pub const NEST_STACK: usize = 256 << 10;

/// Set in the pc compiled code passes `yavs_rt_step` when the instruction cannot free or
/// resize a memory object (`keeps_objects`): the objects `Mem` found stay valid.
pub const KEEP: u32 = 1 << 31;

/// What compiled code reaches through its `mem` argument (field offsets: `MEM_*`): the
/// frame view the interpreter keeps current ([`TierFrame`]; compiled code keeps the
/// temporary area's length in it and hands it back before every step and at the end), and
/// the objects `yavs_rt_obj` and `Deref` found (a direct-mapped table keyed by the object's
/// address high half and a tag: `Mem::slot`, `Mem::key`).
#[repr(C)]
pub struct Mem {
    pub fr: TierFrame,
    pub objs: [Obj; OBJS],
    /// Per address space (bits 29 to 31 of the high half), the tag of its objects shifted to
    /// bits 32 and up (an entry's key is the high half of the object's addresses or-ed with it),
    /// and the entry offset, scaled by the entry size, its keys are moved by, so the objects of
    /// two instances of one process do not share entries (#679). Instance data, package data and
    /// shared variables have the machine's tag, heap objects the machine's at its heap epoch,
    /// constants the runtime's, frame and temporary objects the activation's. Generated code
    /// reads both (`MEM_TAGS`, `MEM_MIXES`).
    pub tags: [u64; 8],
    pub mixes: [u64; 8],
    /// The next tag to issue; at `TAG_END` the table is emptied and this `Mem` gets a new `id`
    /// (`wipe`), so no record of a process names a tag twice.
    next_tag: u64,
    id: u64,
    /// The owner the machine's tags are for, and the epochs of the process heap and the shared
    /// heap its heap tag is for (`TierFrame::owner`, `heap_epoch`, `shared_epoch`): the next
    /// activation or call of that machine keeps them, dropping the objects the heaps changed since.
    cur_owner: u64,
    cur_epochs: [u64; 2],
    /// The owner whose runtime the constants' tag is for (`OWNER_RT_SHIFT` and up compare).
    rt_owner: u64,
    /// Some frame or temporary object is in the table (`forget_end` drops them).
    pub transient: bool,
    /// The temporary area's length the interpreter last gave: no hand-back while unchanged.
    pub temp_seen: u64,
    /// Nonzero while native frames unwind to be materialised (`yavs_rt_deopt`).
    pub ndeopt: u64,
    /// Per function, the address of its native-frame code (`Gen::native_call`), 0 for none.
    pub ntab: *const usize,
    /// Below this native stack address no native callee is entered (`NEST_STACK`).
    pub stack_lo: u64,
    /// Per frame index, the slots of the native frame living there (`NSlot`): a native callee
    /// reaches the frames of its native callers through it.
    pub nslots: *mut NSlot,
    /// Entries of the table `nslots` points at (the thread's table never shrinks).
    nslots_len: usize,
    /// The `Suspend` a wait ended on the kernel itself stopped at (`TIER_ROOT_WAIT`), for the
    /// process's record (`TierState::at`); `NO_PC` otherwise.
    wait_at: u32,
    /// The check mode of `lean` activations (debug builds, feature `table-check`): set while one
    /// runs, with the machine and heap tags it replaced by `POISON` (`begin_lean`, `end_lean`).
    lean: bool,
    saved: [u64; 8],
}

/// The slots of a native frame, registered at its entry in `Mem::nslots`.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct NSlot {
    pub ptr: *mut u8,
    pub len: u64,
}

/// The slots of the native frames in the table of `Mem::nslots`, for the machine to reach.
pub struct TabSlots(*mut NSlot);

// SAFETY: the table is the thread's own (`NSLOTS`) and the value lives only while compiled code
// of that thread calls into the interpreter.
unsafe impl Send for TabSlots {}

impl yavs_interp::NativeSlots for TabSlots {
    fn slots(&self, idx: usize) -> Option<&[u8]> {
        // SAFETY: the machine asks for frame indices of live native frames, whose entries
        // `Gen::func` wrote at their entry; each is a distinct `alloca` of its frame.
        unsafe {
            let e = *self.0.add(idx);
            Some(std::slice::from_raw_parts(e.ptr, e.len as usize))
        }
    }

    fn slots_mut(&mut self, idx: usize) -> Option<&mut [u8]> {
        // SAFETY: as `slots`.
        unsafe {
            let e = *self.0.add(idx);
            Some(std::slice::from_raw_parts_mut(e.ptr, e.len as usize))
        }
    }
}

thread_local! {
    /// The table behind `Mem::nslots` of this simulation thread, grown to the call depth limit.
    static NSLOTS: std::cell::RefCell<Vec<NSlot>> = const { std::cell::RefCell::new(Vec::new()) };
    /// `NSLOTS`' address and length, read without a borrow at every entry (`nslots`).
    static NSLOTS_AT: std::cell::Cell<(*mut NSlot, usize)> =
        const { std::cell::Cell::new((std::ptr::null_mut(), 0)) };
}

/// This thread's `NSLOTS` table with at least `depth` entries.
#[inline(always)]
pub fn nslots(depth: usize) -> *mut NSlot {
    match NSLOTS_AT.get() {
        (p, n) if n >= depth => p,
        _ => nslots_grow(depth),
    }
}

#[inline(never)]
pub fn nslots_grow(depth: usize) -> *mut NSlot {
    let at = NSLOTS.with(|t| {
        let mut t = t.borrow_mut();
        if t.len() < depth {
            t.resize(
                depth,
                NSlot {
                    ptr: std::ptr::null_mut(),
                    len: 0,
                },
            );
        }
        (t.as_mut_ptr(), t.len())
    });
    NSLOTS_AT.set(at);
    at.0
}

/// An object in `Mem::objs`: its key (the high half of its addresses or-ed with the tag of their
/// space, `Mem::slot`; tag 0: no object), start, length, whether it was found for writing, and
/// the access value a `Deref` found it for (`NO_H`: none; its address is `hi << 32`). An entry
/// with an access value is writable (`yavs_rt_obj` keeps the permission of an entry it replaces
/// for the same object), so a `Deref` compares the value and the tag only.
#[derive(Clone, Copy)]
#[repr(C, align(64))]
pub struct Obj {
    pub hi: u64,
    pub ptr: *mut u8,
    pub len: u64,
    pub w: u64,
    pub h: u64,
}

/// An entry is one cache line, so generated code scales the key by a shift.
pub const OBJ_SHIFT: u32 = 6;
const _: () = assert!(std::mem::size_of::<Obj>() == 1 << OBJ_SHIFT);

/// Entries of `Mem::objs`, a power of two. ponytail: direct-mapped by the low bits of the
/// object index xor its space (`obj_at`: a private and a shared heap object of the same index
/// are two objects of one protected-type library function) xor the tag's offset (`mix`): two
/// objects whose slots agree evict each other. 16 entries missed three heap objects in four on
/// a test runner's queue code (95 million `Deref` callbacks in a 16 s run, 12 million at 256);
/// 1 024 made that three times rarer; since the objects of every machine stay in the table
/// (#679), 16 384: the LEON3 run finds objects again 21.7 million times at 4 096, 7.2 million
/// at 16 384 and 6.6 million at 65 536. Ways if a profile shows conflicts again.
pub const OBJS: usize = 16384;

/// Tags are issued below this: then the table starts again empty (`Mem::wipe`).
const TAG_END: u64 = 1 << 31;

/// A tag in the low half of a record word (`Mem::save_tag`).
const TAG_MASK: u64 = (1 << 32) - 1;

/// The machine and heap tags of a `lean` activation in the check mode: never issued (tags stay
/// below `TAG_END`), so every lookup of those spaces misses into the runtime, which panics.
const POISON: u64 = TAG_MASK << 32;

/// The check mode of `lean` activations is on.
const LEAN_CHECK: bool = cfg!(any(debug_assertions, feature = "table-check"));

/// A process's record (`TierState`): the `Mem` identity, the process heap's epoch, the heap tag and
/// the machine tag of its objects in one word, the shared heap's epoch (`Mem::save_tag`).
const _: () = assert!(yavs_interp::TIER_STATE_WORDS == 4);

/// Identities of `Mem`s, so a record names the table its tag is from.
static MEM_IDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The entry offset (scaled by the entry size) the keys of tag `t` are moved by.
#[inline(always)]
pub fn mix(t: u64) -> u64 {
    (t.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> (64 - OBJS.trailing_zeros())) << OBJ_SHIFT
}

const _: () = assert!(OBJS.is_power_of_two() && OBJS >= 64 && OBJS <= 1 << 24);

/// The entry of `Mem::objs` for the object whose addresses have high half `hi`, before the tag's
/// offset (`Mem::slot`): the low bits of its index, bit 3 flipped for a shared heap (space 7, bit 29 of `hi`; private is 4). The
/// access value of a heap object has the same index in bits 32 to 55 and, as its tag, a shared
/// flag in bit 57 (`handle_at`, and `obj_entry` in the generated code, which must agree with
/// both: `obj_keys_agree`).
#[inline(always)]
pub fn obj_at(hi: u64) -> usize {
    (hi ^ (hi >> KEY_SHIFT_ADDR & KEY_FLAG)) as usize & (OBJS - 1)
}

/// `obj_at` from the high half of an access value (`handle >> 32`).
#[cfg(test)]
pub fn handle_at(hi: u64) -> usize {
    (hi ^ (hi >> KEY_SHIFT_HANDLE & KEY_FLAG)) as usize & (OBJS - 1)
}

/// The shared-heap flag of the key, as bit 3: shifted down from bit 29 of an address's high half
/// and from bit 25 of an access value's.
pub const KEY_FLAG: u64 = 8;
pub const KEY_SHIFT_ADDR: u32 = 26;
pub const KEY_SHIFT_HANDLE: u32 = 22;

/// The access value of an entry no `Deref` found: no handle (not null either: a null handle
/// must miss).
pub const NO_H: u64 = 1;

/// The empty entry: all zero, a key of tag 0, which no space has (`Mem::restart`).
pub const NO_OBJ: Obj = Obj {
    hi: 0,
    ptr: std::ptr::null_mut(),
    len: 0,
    w: 0,
    h: 0,
};

pub const MEM_SLOTS: u64 = std::mem::offset_of!(Mem, fr.slots) as u64;
pub const MEM_SLOTS_LEN: u64 = std::mem::offset_of!(Mem, fr.slots_len) as u64;
pub const MEM_FRAME: u64 = std::mem::offset_of!(Mem, fr.frame_hi) as u64;
pub const MEM_TEMP: u64 = std::mem::offset_of!(Mem, fr.temp) as u64;
pub const MEM_TEMP_LEN: u64 = std::mem::offset_of!(Mem, fr.temp_len) as u64;
pub const MEM_TEMP_CAP: u64 = std::mem::offset_of!(Mem, fr.temp_cap) as u64;
pub const MEM_TEMP_BASE: u64 = std::mem::offset_of!(Mem, fr.temp_base) as u64;
pub const MEM_UP: u64 = std::mem::offset_of!(Mem, fr.up) as u64;
pub const MEM_UP_SLOTS: u64 = std::mem::offset_of!(Mem, fr.up_slots) as u64;
pub const MEM_UP_LEN: u64 = std::mem::offset_of!(Mem, fr.up_len) as u64;
pub const MEM_INST: u64 = std::mem::offset_of!(Mem, fr.inst) as u64;
pub const MEM_INST_LEN: u64 = std::mem::offset_of!(Mem, fr.inst_len) as u64;
pub const MEM_NSIGS: u64 = std::mem::offset_of!(Mem, fr.nsigs) as u64;
pub const MEM_TEMP_ROOTS: u64 = std::mem::offset_of!(Mem, fr.temp_roots) as u64;
pub const MEM_SIGTAB: u64 = std::mem::offset_of!(Mem, fr.sigtab) as u64;
pub const MEM_SIGTAB_LEN: u64 = std::mem::offset_of!(Mem, fr.sigtab_len) as u64;
pub const MEM_KNOWN: u64 = std::mem::offset_of!(Mem, fr.known) as u64;
pub const MEM_CYCLE: u64 = std::mem::offset_of!(Mem, fr.cycle) as u64;
pub const MEM_FRAME_IDX: u64 = std::mem::offset_of!(Mem, fr.frame) as u64;
pub const MEM_MAX_DEPTH: u64 = std::mem::offset_of!(Mem, fr.max_depth) as u64;
pub const MEM_NDEOPT: u64 = std::mem::offset_of!(Mem, ndeopt) as u64;
pub const MEM_NTAB: u64 = std::mem::offset_of!(Mem, ntab) as u64;
pub const MEM_STACK_LO: u64 = std::mem::offset_of!(Mem, stack_lo) as u64;
pub const MEM_NSLOTS: u64 = std::mem::offset_of!(Mem, nslots) as u64;
pub const MEM_FLAGS: u64 = std::mem::offset_of!(Mem, fr.flags) as u64;
pub const MEM_COV: u64 = std::mem::offset_of!(Mem, fr.cov) as u64;

pub const MEM_STOP: u64 = std::mem::offset_of!(Mem, fr.stop) as u64;
pub const MEM_SPIN: u64 = std::mem::offset_of!(Mem, fr.spin) as u64;

pub const MEM_COV_LEN: u64 = std::mem::offset_of!(Mem, fr.cov_len) as u64;
pub const MEM_OBJS: u64 = std::mem::offset_of!(Mem, objs) as u64;
pub const MEM_TAGS: u64 = std::mem::offset_of!(Mem, tags) as u64;
pub const MEM_MIXES: u64 = std::mem::offset_of!(Mem, mixes) as u64;

pub const OBJ_PTR: u64 = std::mem::offset_of!(Obj, ptr) as u64;
pub const OBJ_LEN: u64 = std::mem::offset_of!(Obj, len) as u64;
pub const OBJ_W: u64 = std::mem::offset_of!(Obj, w) as u64;
pub const OBJ_H: u64 = std::mem::offset_of!(Obj, h) as u64;

/// Bytes of a `TierSig` and the offsets of its fields.
pub const SIG_ENT: i64 = std::mem::size_of::<yavs_interp::TierSig>() as i64;
pub const SIG_CODES: i64 = std::mem::offset_of!(yavs_interp::TierSig, codes) as i64;
pub const SIG_LEN: i64 = std::mem::offset_of!(yavs_interp::TierSig, len) as i64;
pub const SIG_SIGNAL: i64 = std::mem::offset_of!(yavs_interp::TierSig, signal) as i64;
pub const SIG_CAP: i64 = std::mem::offset_of!(yavs_interp::TierSig, cap) as i64;
pub const SIG_STAMPS: i64 = std::mem::offset_of!(yavs_interp::TierSig, stamps) as i64;
pub const SIG_LAST: i64 = std::mem::offset_of!(yavs_interp::TierSig, last) as i64;
pub const SIG_PLANES: i64 = std::mem::offset_of!(yavs_interp::TierSig, planes) as i64;
pub const SIG_PLEN: i64 = std::mem::offset_of!(yavs_interp::TierSig, plen) as i64;
pub const SIG_BYTES: i64 = std::mem::offset_of!(yavs_interp::TierSig, bytes) as i64;
pub const SIG_ES: i64 = std::mem::offset_of!(yavs_interp::TierSig, es) as i64;
pub const SIG_OFF: i64 = std::mem::offset_of!(yavs_interp::TierSig, off) as i64;

/// The address spaces of each kind of tag (`Mem::tags`).
const MACHINE_SPACES: [u8; 3] = [space::INST, space::PKG, space::SHARED];
const HEAP_SPACES: [u8; 2] = [space::HEAP, space::SHARED_HEAP];
const VOLATILE_SPACES: [u8; 2] = [space::FRAME, space::TEMP];

impl Mem {
    /// A `Mem` for view `fr` with an empty table, made in place (the table is a megabyte: a
    /// value on the stack would overflow a simulation thread's).
    pub fn new(fr: TierFrame) -> Box<Mem> {
        let m = Box::<Mem>::new_zeroed();
        // SAFETY: every field of `Mem` is an integer, a bool, a raw pointer, or an array or
        // `repr(C)` struct of those (`TierFrame`, `Obj`): all zero bytes are a valid value of
        // each, and the empty entry is all zero (`NO_OBJ`).
        let mut m = unsafe { m.assume_init() };
        m.fr = fr;
        m.temp_seen = fr.temp_len;
        m.cur_owner = u64::MAX;
        m.rt_owner = u64::MAX;
        m.restart();
        m
    }

    /// Point `nslots` at this thread's table of at least `depth` entries.
    #[inline(always)]
    fn set_nslots(&mut self, depth: usize) {
        if self.nslots_len < depth {
            self.nslots = nslots(depth);
            self.nslots_len = depth;
        }
    }

    /// Give the temporary area the length compiled code left it.
    pub fn sync(&mut self, host: &mut dyn TierHost) {
        if self.fr.temp_len != self.temp_seen {
            // SAFETY: compiled code only shrinks the length or grows it within the capacity
            // after zeroing the new bytes (`Inst::Alloc`), so they are initialised.
            unsafe { host.temp().set_len(self.fr.temp_len as usize) }
        }
    }

    /// The entry of `objs` for the object of high half `hi` and its key there.
    #[inline(always)]
    pub fn slot(&self, hi: u64) -> (usize, u64) {
        let sp = (hi >> 29) as usize & 7;
        (
            obj_at(hi) ^ (self.mixes[sp] >> OBJ_SHIFT) as usize,
            hi | self.tags[sp],
        )
    }

    /// The entry of the object of high half `hi`, if the table holds it.
    #[inline(always)]
    pub fn get(&self, hi: u64) -> Option<&Obj> {
        self.check_lean(hi);
        let (at, k) = self.slot(hi);
        let o = &self.objs[at];
        (o.hi == k).then_some(o)
    }

    /// Tag `t` for the objects of `spaces`.
    #[inline(always)]
    fn set(&mut self, spaces: &[u8], t: u64) {
        for &sp in spaces {
            self.tags[sp as usize] = t << 32;
            self.mixes[sp as usize] = mix(t);
        }
    }

    /// The start of an activation (with the process's record) or a call from the interpreter,
    /// on a view just made: the machine's tags if the record or the last activation on this
    /// thread had them (its heap tag only at the same heap epoch), else new ones. A new tag drops
    /// nothing: the old keys simply miss (#679).
    #[inline(always)]
    pub fn begin(&mut self, rec: Option<&yavs_interp::TierState>) {
        let (owner, epoch) = (self.fr.owner, self.fr.heap_epoch);
        if (owner ^ self.rt_owner) >> yavs_interp::OWNER_RT_SHIFT != 0 {
            self.new_runtime();
        }
        let rec = rec.filter(|st| st.used && st.words[0] == self.id);
        let same = owner == self.cur_owner;
        if let Some(st) = rec {
            self.set(&MACHINE_SPACES, st.words[2] >> 32);
        } else if !same {
            let t = self.issue();
            self.set(&MACHINE_SPACES, t);
        }
        // Someone compares heap objects by their write counts: a kept writable heap object
        // would not count this activation's writes (`Heap::data_mut`), so none is kept.
        let stamps = self.fr.flags & yavs_interp::TIER_STAMPS != 0;
        let kept = !stamps
            && match rec {
                Some(st) => self.keep_heap(st.words[2] & TAG_MASK, [st.words[1], st.words[3]]),
                None => {
                    same && self.keep_heap(self.tags[space::HEAP as usize] >> 32, self.cur_epochs)
                }
            };
        if !kept {
            let t = self.issue();
            self.set(&HEAP_SPACES, t);
        }
        (self.cur_owner, self.cur_epochs) = (owner, [epoch, self.fr.shared_epoch]);
    }

    /// Heap tag `t`, whose objects were found at the heaps' epochs `from`, for the view's
    /// epochs: the objects of the slots the heaps changed since are dropped (`drop_changed`);
    /// `false` when more changed than their logs name (then nothing is kept).
    #[inline(always)]
    fn keep_heap(&mut self, t: u64, from: [u64; 2]) -> bool {
        let to = [self.fr.heap_epoch, self.fr.shared_epoch];
        if self.tags[space::HEAP as usize] != t << 32 {
            self.set(&HEAP_SPACES, t);
        }
        // Word by word: the array compare went through the stack.
        from[0] == to[0] && from[1] == to[1] || self.drop_changed(from, to)
    }

    /// Drop the table's entries of the heap slots the heaps changed from epochs `from` to `to`
    /// (`TierFrame::heap_log`); `false`, nothing dropped, when a heap is more than `HEAP_LOG`
    /// changes on or has no log.
    #[inline(never)]
    fn drop_changed(&mut self, from: [u64; 2], to: [u64; 2]) -> bool {
        let n = [to[0].wrapping_sub(from[0]), to[1].wrapping_sub(from[1])];
        for (&d, log) in n.iter().zip(self.fr.heap_log) {
            if d != 0 && (d > HEAP_LOG as u64 || log.is_null()) {
                return false;
            }
        }
        for (k, sp) in [space::HEAP, space::SHARED_HEAP].into_iter().enumerate() {
            for e in 1..=n[k] {
                let at = from[k].wrapping_add(e) as usize % HEAP_LOG;
                // SAFETY: the log of the view's heap has `HEAP_LOG` entries and lives as long
                // as the machine the view is of.
                let i = unsafe { *self.fr.heap_log[k].add(at) };
                let (at, key) = self.slot(u64::from(sp) << 29 | u64::from(i));
                if self.objs[at].hi == key {
                    self.objs[at] = NO_OBJ;
                }
            }
        }
        true
    }

    /// The tag of the constants of a runtime other than the last one's.
    #[cold]
    #[inline(never)]
    fn new_runtime(&mut self) {
        let t = self.issue();
        self.set(&[space::CONST], t);
        self.rt_owner = self.fr.owner;
    }

    #[inline(always)]
    fn issue(&mut self) -> u64 {
        if self.next_tag == TAG_END {
            self.wipe();
        }
        self.next_tag += 1;
        self.next_tag - 1
    }

    /// Every tag was issued: empty the table and start again.
    #[cold]
    #[inline(never)]
    fn wipe(&mut self) {
        self.objs.fill(NO_OBJ);
        self.restart();
    }

    /// A new identity (records of processes name the old one) and a tag for every space, 1 to
    /// 4: no key of the table has tag 0 but the empty entries'.
    fn restart(&mut self) {
        self.id = MEM_IDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.transient = false;
        self.set(&[space::CONST], 1);
        self.set(&MACHINE_SPACES, 2);
        self.set(&HEAP_SPACES, 3);
        self.set(&VOLATILE_SPACES, 4);
        self.next_tag = 5;
    }

    /// The start of a `lean` activation (`regs::no_table`, #766): it reaches no object of the
    /// machine's or the heaps' tags, so they are not set and the record is not read. The owner is
    /// forgotten: a `forget` on a cold path may set this machine's heap epochs, which the next
    /// `begin` without a record must not take for the last owner's. Constants keep their tag.
    #[inline(always)]
    pub fn begin_lean(&mut self) {
        if (self.fr.owner ^ self.rt_owner) >> yavs_interp::OWNER_RT_SHIFT != 0 {
            self.new_runtime();
        }
        self.cur_owner = u64::MAX;
        cb(Cb::Lean);
        if LEAN_CHECK {
            self.lean = true;
            self.saved = self.tags;
            self.set(&MACHINE_SPACES, POISON >> 32);
            self.set(&HEAP_SPACES, POISON >> 32);
        }
    }

    /// The end of a `lean` activation: the check mode puts the tags back.
    #[inline(always)]
    pub fn end_lean(&mut self) {
        if LEAN_CHECK {
            self.lean = false;
            for sp in MACHINE_SPACES.into_iter().chain(HEAP_SPACES) {
                let t = self.saved[sp as usize] >> 32;
                self.set(&[sp], t);
            }
        }
    }

    /// The check mode: a `lean` activation reached an object of the machine's or a heap's tag.
    #[inline(always)]
    fn check_lean(&self, hi: u64) {
        if LEAN_CHECK && self.lean && matches!(life(hi), Life::Machine | Life::Heap) {
            lean_touched(hi);
        }
    }

    /// Keep the machine's tags in the process's record for its next activation.
    #[inline(always)]
    pub fn save_tag(&self, st: &mut yavs_interp::TierState) {
        let t = |sp: u8| self.tags[sp as usize] >> 32;
        // Tags are below `TAG_END`: the heap's and the machine's share a word.
        st.words = [
            self.id,
            self.cur_epochs[0],
            t(space::HEAP) | t(space::INST) << 32,
            self.cur_epochs[1],
        ];
        st.used = true;
        st.at = self.wait_at;
    }

    /// Enter object `o` (`o.hi`: the high half of its addresses) at its entry.
    #[inline(always)]
    pub fn put_obj(&mut self, mut o: Obj) {
        self.check_lean(o.hi);
        let (at, k) = self.slot(o.hi);
        self.transient |= life(o.hi) == Life::Volatile;
        o.hi = k;
        self.objs[at] = o;
    }

    /// The end of an activation or a call: forget the frame and temporary objects found.
    #[inline(always)]
    pub fn forget_end(&mut self) {
        if self.transient {
            self.drop_volatile();
        }
    }

    #[inline(never)]
    fn drop_volatile(&mut self) {
        let t = self.issue();
        self.set(&VOLATILE_SPACES, t);
        self.transient = false;
    }

    /// After a host call that may have pushed frames, moved the temporary area or freed or
    /// edited heap objects: the frame and temporary objects go, and the heap objects the heaps
    /// changed (their epochs count every deallocation and edit in place), all of them when more
    /// changed than the heaps' logs name.
    #[inline(always)]
    pub fn forget(&mut self) {
        self.forget_end();
        let to = [self.fr.heap_epoch, self.fr.shared_epoch];
        if to != self.cur_epochs {
            if !self.drop_changed(self.cur_epochs, to) {
                let t = self.issue();
                self.set(&HEAP_SPACES, t);
            }
            self.cur_epochs = to;
        }
    }
}

#[cold]
#[inline(never)]
fn lean_touched(hi: u64) -> ! {
    panic!(
        "a lean activation (regs::no_table) reached the object table for address high half {hi:#x}: this is a bug in yavs"
    )
}

/// How long an object of `Mem::objs` stays where it was found: until the next host call that
/// may move frames or the temporary area; while the heaps' epoch is unchanged (heap objects);
/// while the same machine runs (instance data, package data, shared variables: another machine
/// may run another instance); while a machine of the same runtime runs (constants: the program's).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Life {
    Volatile,
    Heap,
    Machine,
    Runtime,
}

pub fn life(hi: u64) -> Life {
    match (hi >> 29) as u8 {
        space::CONST => Life::Runtime,
        space::INST | space::PKG | space::SHARED => Life::Machine,
        space::HEAP | space::SHARED_HEAP => Life::Heap,
        _ => Life::Volatile,
    }
}

/// What a running compiled function reaches through `env`.
pub struct Env<'a> {
    pub host: &'a mut dyn TierHost,
    pub lookup: Lookup<'a>,
    /// Below this native stack address no compiled callee is entered (`NEST_STACK`).
    pub stack_lo: usize,
    pub panic: Option<Box<dyn Any + Send>>,
    /// Native frames recorded on their way to `TierHost::materialise`, innermost first.
    pub pending: Vec<NativeFrame>,
    /// The slots of the native frames for calls into the interpreter (`TabSlots`), made when
    /// first needed in the activation.
    pub windows: Option<Box<dyn yavs_interp::NativeSlots>>,
}

/// The kernel context, the machine's drivers and the view's signals of the activation `fr`
/// belongs to, when it offers them (`TierFrame::kctx`): the drives that go to the kernel straight.
///
/// # Safety
/// `fr` is the view of the running activation or call (`kctx` is cleared when it ends) and
/// nothing else uses the context, the drivers or the signal table while the result lives.
#[inline(always)]
unsafe fn direct<'a>(
    fr: &TierFrame,
) -> Option<(
    &'a mut yavs_kernel::Ctx<'a>,
    &'a [yavs_interp::ProcDriver],
    &'a [yavs_interp::TierSig],
)> {
    if fr.kctx.is_null() {
        return None;
    }
    // SAFETY: the host wrote `kctx` from the activation's `&mut Ctx` and the tables from the
    // machine's vectors (`view`, `view_again`), which do not move within the activation; the
    // host does not run while compiled code is in this callback.
    unsafe {
        Some((
            &mut *fr.kctx.cast::<yavs_kernel::Ctx<'a>>(),
            std::slice::from_raw_parts(fr.drivers, fr.ndrivers as usize),
            std::slice::from_raw_parts(fr.sigtab, fr.sigtab_len as usize),
        ))
    }
}

/// The address of a local of the caller: where the native stack is.
#[inline(always)]
pub fn stack_here() -> usize {
    let x = 0u8;
    std::hint::black_box(&x) as *const u8 as usize
}

/// Executes one instruction in the interpreter (`TierHost::step`) and returns `GO`, or what
/// the compiled code returns: the pc after a fault, `TIER_SUSPEND`, `TIER_UNWIND`. A frame the
/// instruction pushed runs here when its function has compiled code, and the call continues
/// as `GO` once that returns (`TierHost::ret`); otherwise the frame is the interpreter's
/// (`TIER_UNWIND`).
pub unsafe extern "C" fn yavs_rt_step(
    env: *mut c_void,
    _regs: *mut u64,
    mem: *mut Mem,
    pc: u32,
) -> u32 {
    cb(Cb::Step);
    // SAFETY: `env` and `mem` are what `enter` (or this function, for a callee) passed to the
    // compiled function, which is blocked in this call; `regs` points into the register
    // stack, which the compiled code does not touch until we return (it passes it only so
    // LLVM reloads registers after the call). `env` is reborrowed briefly: a compiled callee
    // run from here reaches it again through the raw pointer.
    // `env` and `mem` are reborrowed briefly: a compiled callee run from here reaches them
    // again through the raw pointers.
    let envp = env as *mut Env<'_>;
    let (keep, pc) = (pc & KEEP != 0, pc & !KEEP);
    unsafe { (*mem).sync(&mut *(*envp).host) };
    // No unwinding through JIT frames: a panic is carried over the call and resumed after.
    let r = match catch_unwind(AssertUnwindSafe(|| unsafe { step_in(envp, mem, pc) })) {
        Ok(r) => r,
        Err(p) => {
            unsafe { (*envp).panic = Some(p) };
            pc
        }
    };
    let mem = unsafe { &mut *mem };
    mem.temp_seen = mem.fr.temp_len;
    if r != GO || !keep {
        mem.forget();
    }
    r
}

/// Executes the signal instruction at `pc` in the interpreter (`TierHost::signal`; `edge` 1 or 2
/// for a `rising_edge` / `falling_edge` native) and returns `GO`, or the pc after a fault. The
/// objects `Mem` found stay valid: these instructions free nothing.
pub unsafe extern "C" fn yavs_rt_signal(
    env: *mut c_void,
    _regs: *mut u64,
    mem: *mut Mem,
    pc: u32,
    edge: u32,
) -> u32 {
    cb(Cb::Signal);
    // SAFETY: as in `yavs_rt_step`.
    let envp = env as *mut Env<'_>;
    unsafe { (*mem).sync(&mut *(*envp).host) };
    let r = catch_unwind(AssertUnwindSafe(|| unsafe {
        (*envp).host.signal(&mut (*mem).fr, pc, edge)
    }));
    // SAFETY: the pointers are the ones compiled code was given.
    unsafe { stepped(envp, mem, pc, r) }
}

/// What a `TierHost::signal`-like call returns to compiled code: `GO`, or the pc after a
/// fault, `TIER_SUSPEND`, `TIER_UNWIND`.
pub unsafe fn stepped(
    envp: *mut Env<'_>,
    mem: *mut Mem,
    pc: u32,
    r: Result<Stepped, Box<dyn Any + Send>>,
) -> u32 {
    // SAFETY: as in `yavs_rt_step`.
    let mem = unsafe { &mut *mem };
    mem.temp_seen = mem.fr.temp_len;
    match r {
        Ok(Stepped::Next) => GO,
        Ok(Stepped::Fault) => pc,
        Ok(Stepped::Suspend) => TIER_SUSPEND,
        // Only where `signal` stepped another instruction (never for those `Gen` sends).
        Ok(Stepped::Call(_)) => {
            mem.forget();
            TIER_UNWIND
        }
        Err(p) => {
            unsafe { (*envp).panic = Some(p) };
            pc
        }
    }
}

/// Record a native frame of `func` at `pc` for materialisation (`docs/decisions.md` "Compiled
/// frames"): its registers are copied from `regs` (the function's spill buffer, filled at the
/// exit site with the registers live at `pc`), its slots go to `yavs_rt_deopt_slots`.
pub unsafe extern "C" fn yavs_rt_deopt(
    env: *mut c_void,
    mem: *mut Mem,
    func: u32,
    pc: u32,
    temp_base: u64,
    nregs: u64,
    slen: u64,
    link: u64,
    regs: *const u64,
) -> *mut u64 {
    cb(Cb::Deopt);
    // SAFETY: as in `yavs_rt_step`; `regs` holds `nregs` words.
    let (env, mem) = unsafe { (&mut *(env as *mut Env<'_>), &mut *mem) };
    mem.ndeopt = 1;
    env.pending.push(NativeFrame {
        func: FuncId(func),
        pc,
        regs: unsafe { std::slice::from_raw_parts(regs, nregs as usize) }.to_vec(),
        slots: vec![0; slen as usize],
        temp_base,
        link,
    });
    let last = env.pending.len() - 1;
    env.pending[last].regs.as_mut_ptr()
}

/// The slot buffer of the frame `yavs_rt_deopt` recorded last.
pub unsafe extern "C" fn yavs_rt_deopt_slots(env: *mut c_void) -> *mut u8 {
    // SAFETY: as in `yavs_rt_step`; called right after `yavs_rt_deopt`.
    let env = unsafe { &mut *(env as *mut Env<'_>) };
    let last = env.pending.len() - 1;
    env.pending[last].slots.as_mut_ptr()
}

/// A frame-mode function's native callee unwound for materialisation: push the recorded
/// frames on the top frame, which waits at its call at `pc`. Returns `TIER_UNWIND` (the
/// interpreter goes on with the top frame), or `pc` after a fault.
pub unsafe extern "C" fn yavs_rt_deopt_end(env: *mut c_void, mem: *mut Mem, pc: u32) -> u32 {
    // SAFETY: as in `yavs_rt_step`.
    let (env, mem) = unsafe { (&mut *(env as *mut Env<'_>), &mut *mem) };
    mem.sync(&mut *env.host);
    let mut frames = std::mem::take(&mut env.pending);
    frames.reverse();
    let r = catch_unwind(AssertUnwindSafe(|| {
        env.host.materialise(&mut mem.fr, pc, &frames)
    }));
    mem.forget();
    mem.temp_seen = mem.fr.temp_len;
    mem.ndeopt = 0;
    match r {
        Ok(Stepped::Next) => TIER_UNWIND,
        // A native one of the frames ran ended the process.
        Ok(Stepped::Suspend) => TIER_SUSPEND,
        Ok(_) => pc,
        Err(p) => {
            env.panic = Some(p);
            pc
        }
    }
}

/// `TierHost::drive_block` for the `Drive` at `pc`: `wave` (40 readable bytes, checked by the
/// caller) holds the time and the first block of planes of its one waveform element.
pub unsafe extern "C" fn yavs_rt_drive(
    env: *mut c_void,
    regs: *mut u64,
    mem: *mut Mem,
    pc: u32,
    driver: u32,
    lo: u64,
    len: u64,
    wave: *const u8,
    flags: u32,
) -> u32 {
    cb(Cb::Drive);
    // SAFETY: as in `yavs_rt_step`; the caller checked 40 readable bytes at `wave`, 16 where
    // the target keeps bytes (bit 1 of `flags`: one scalar, whose cell is the first word).
    let envp = env as *mut Env<'_>;
    if flags & 4 == 0 {
        let word = |k: usize| unsafe { std::ptr::read_unaligned(wave.add(k * 8).cast::<u64>()) };
        let (delay, block) = if flags & 2 != 0 {
            (word(0) as i64, [word(1), 0, 0, 0])
        } else {
            (word(0) as i64, unsafe { plane_block(wave) })
        };
        // The usual case, taken by the kernel without the host (#766), else by the host
        // without a frame view or a result.
        // SAFETY: `mem` is the running activation's.
        if let Some((c, ds, ss)) = unsafe { direct(&(*mem).fr) } {
            if yavs_interp::direct_drive_now(c, ds, ss, driver, [lo, len], delay, block) {
                return GO;
            }
        } else if unsafe { (*envp).host.drive_now(driver, [lo, len], delay, block) } {
            return GO;
        }
    }
    yavs_rt_drive_slow(env, regs, mem, pc, driver, lo, len, wave, flags)
}

/// `yavs_rt_drive` for a native frame (`TierHost::drive_try`): no frame view and no pc, any delay.
/// 1 when done, 0 when nothing was done: the caller materialises its frames and the interpreter
/// runs the `Drive`. `wave` is read as in `yavs_rt_drive`.
pub unsafe extern "C" fn yavs_rt_drive_nat(
    env: *mut c_void,
    driver: u32,
    lo: u64,
    len: u64,
    wave: *const u8,
    flags: u32,
) -> u32 {
    cb(Cb::Drive);
    // SAFETY: as in `yavs_rt_drive`; the caller checked 8 + 32 bytes per 64 elements at `wave`.
    let envp = env as *mut Env<'_>;
    let word = |k: usize| unsafe { std::ptr::read_unaligned(wave.add(k * 8).cast::<u64>()) };
    let host = unsafe { &mut *(*envp).host };
    let inertial = flags & 1 != 0;
    if flags & 4 != 0 {
        let n = (len.div_ceil(64) * 32) as usize;
        // A copy, as `yavs_rt_drive_wide` makes.
        let planes = unsafe { std::slice::from_raw_parts(wave.add(8), n) }.to_vec();
        return u32::from(host.drive_wide_try(
            driver,
            [lo, len],
            word(0) as i64,
            &planes,
            inertial,
        ));
    }
    let (delay, block) = if flags & 2 != 0 {
        (word(0) as i64, [word(1), 0, 0, 0])
    } else {
        (word(0) as i64, unsafe { plane_block(wave) })
    };
    u32::from(host.drive_try(driver, [lo, len], delay, block, inertial))
}

/// `yavs_rt_drive` for a wide target and for the assignments the kernel's fastest path does not
/// take.
#[cold]
#[inline(never)]
extern "C" fn yavs_rt_drive_slow(
    env: *mut c_void,
    _regs: *mut u64,
    mem: *mut Mem,
    pc: u32,
    driver: u32,
    lo: u64,
    len: u64,
    wave: *const u8,
    flags: u32,
) -> u32 {
    // SAFETY: as in `yavs_rt_drive`.
    let envp = env as *mut Env<'_>;
    let inertial = flags & 1;
    if flags & 4 != 0 {
        return yavs_rt_drive_wide(envp, mem, pc, driver, lo, len, wave, inertial);
    }
    let word = |k: usize| unsafe { std::ptr::read_unaligned(wave.add(k * 8).cast::<u64>()) };
    let (delay, block) = if flags & 2 != 0 {
        (word(0) as i64, [word(1), 0, 0, 0])
    } else {
        (word(0) as i64, unsafe { plane_block(wave) })
    };
    unsafe { (*mem).sync(&mut *(*envp).host) };
    let r = catch_unwind(AssertUnwindSafe(|| unsafe {
        (*envp).host.drive_block(
            &mut (*mem).fr,
            pc,
            driver,
            [lo, len],
            delay,
            block,
            inertial != 0,
        )
    }));
    // SAFETY: the pointers are the ones compiled code was given.
    unsafe { stepped(envp, mem, pc, r) }
}

/// `TierHost::drive_wide` for the `Drive` at `pc` of more than 64 elements: `wave` holds the
/// time and the plane blocks of the one waveform element (checked by the caller).
#[allow(clippy::too_many_arguments)]
fn yavs_rt_drive_wide(
    envp: *mut Env<'_>,
    mem: *mut Mem,
    pc: u32,
    driver: u32,
    lo: u64,
    len: u64,
    wave: *const u8,
    inertial: u32,
) -> u32 {
    // SAFETY: as in `yavs_rt_step`; the caller checked 8 + 32 bytes per 64 elements at `wave`.
    let delay = unsafe { std::ptr::read_unaligned(wave.cast::<i64>()) };
    let n = (len.div_ceil(64) * 32) as usize;
    // A copy: the host reads the machine's memory while the kernel takes the planes.
    let src = unsafe { std::slice::from_raw_parts(wave.add(8), n) };
    let mut stack = [0u8; 256];
    let heap;
    let planes: &[u8] = if n <= stack.len() {
        stack[..n].copy_from_slice(src);
        &stack[..n]
    } else {
        heap = src.to_vec();
        &heap
    };
    unsafe { (*mem).sync(&mut *(*envp).host) };
    let r = catch_unwind(AssertUnwindSafe(|| unsafe {
        (*envp).host.drive_wide(
            &mut (*mem).fr,
            pc,
            driver,
            [lo, len],
            delay,
            planes,
            inertial != 0,
        )
    }));
    unsafe { stepped(envp, mem, pc, r) }
}

/// The scalar logic assignment of a `yavs_interp::ScalarDrive` whose `Drive` is at `pc`: the
/// kernel's fastest path for a zero delay (`TierHost::drive_bit_now`, no frame view, no result),
/// else `TierHost::drive_bit`, which does everything the five instructions would have done.
/// `code` holds the logic code and, in bit 8, whether the assignment is inertial.
pub unsafe extern "C" fn yavs_rt_drive_bit(
    env: *mut c_void,
    regs: *mut u64,
    mem: *mut Mem,
    pc: u32,
    driver: u32,
    lo: u64,
    len: u64,
    delay: u64,
    code: u64,
) -> u32 {
    cb(Cb::Drive);
    // SAFETY: as in `yavs_rt_step`.
    let envp = env as *mut Env<'_>;
    if delay == 0 {
        // SAFETY: `mem` is the running activation's.
        let done = match unsafe { direct(&(*mem).fr) } {
            Some((c, ds, ss)) => {
                yavs_interp::direct_drive_bit(c, ds, ss, driver, [lo, len], code & !0x100)
            }
            None => unsafe { (*envp).host.drive_bit_now(driver, [lo, len], code & !0x100) },
        };
        if done {
            return GO;
        }
    }
    yavs_rt_drive_bit_slow(env, regs, mem, pc, driver, lo, len, delay, code)
}

/// `yavs_rt_drive_bit` when the kernel's fastest path does not take the assignment.
#[cold]
#[inline(never)]
extern "C" fn yavs_rt_drive_bit_slow(
    env: *mut c_void,
    _regs: *mut u64,
    mem: *mut Mem,
    pc: u32,
    driver: u32,
    lo: u64,
    len: u64,
    delay: u64,
    code: u64,
) -> u32 {
    // SAFETY: as in `yavs_rt_step`.
    let envp = env as *mut Env<'_>;
    let (inertial, code) = (code & 0x100 != 0, code & !0x100);
    unsafe { (*mem).sync(&mut *(*envp).host) };
    let r = catch_unwind(AssertUnwindSafe(|| unsafe {
        (*envp).host.drive_bit(
            &mut (*mem).fr,
            pc,
            driver,
            [lo, len],
            delay as i64,
            code,
            inertial,
        )
    }));
    unsafe { stepped(envp, mem, pc, r) }
}

/// The plane block of a logic waveform element (the four words after its time), read as one 32-byte
/// copy: two 16-byte loads, each forwarded from the 16-byte stores of the code that made the element
/// (the Cranelift tier's `Copy`) or from one wider store; four reads that the compiler pairs its own
/// way can span two stores and wait for them to reach the cache.
#[inline(always)]
unsafe fn plane_block(wave: *const u8) -> [u64; 4] {
    // SAFETY: the caller checked the 40 bytes of the element.
    unsafe { std::ptr::read_unaligned(wave.add(8).cast::<[u64; 4]>()) }
}

/// The scalar logic assignments compiled code kept (`Batch`): `n` entries of two words, for
/// `TierHost::drive_bits`. `GO`, or the pc of the `Drive` of an entry that failed.
pub extern "C" fn yavs_rt_drive_flush(
    env: *mut c_void,
    mem: *mut Mem,
    entries: *const [u64; 2],
    n: u32,
) -> u32 {
    cb(Cb::Drive);
    // SAFETY: as in `yavs_rt_step`; compiled code passes `n` entries it wrote.
    let envp = env as *mut Env<'_>;
    let mut ents = unsafe { std::slice::from_raw_parts(entries, n as usize) };
    // The leading entries the kernel takes itself (#766).
    if let Some((c, ds, ss)) = unsafe { direct(&(*mem).fr) } {
        ents = &ents[yavs_interp::direct_drive_bits(c, ds, ss, ents)..];
        if ents.is_empty() {
            return GO;
        }
    }
    match catch_unwind(AssertUnwindSafe(|| unsafe {
        (*envp).host.drive_bits(ents)
    })) {
        Ok(None) => GO,
        Ok(Some(pc)) => pc,
        Err(p) => {
            unsafe { (*envp).panic = Some(p) };
            0
        }
    }
}

/// `TierHost::wait_suspend` for the `WaitOn` at `pc` and the `Suspend` after it.
pub unsafe extern "C" fn yavs_rt_wait(
    env: *mut c_void,
    _regs: *mut u64,
    mem: *mut Mem,
    pc: u32,
    lo: u64,
    len: u64,
) -> u32 {
    cb(Cb::Wait);
    // SAFETY: as in `yavs_rt_step`.
    let envp = env as *mut Env<'_>;
    // On the kernel itself (#766), else through the host.
    let fr = unsafe { &(*mem).fr };
    if fr.flags & yavs_interp::TIER_ROOT_WAIT != 0
        && let Some((c, _, ss)) = unsafe { direct(fr) }
    {
        if yavs_interp::direct_wait_again(c, ss, [lo, len]) {
            unsafe { (*mem).wait_at = pc + 1 };
            return TIER_SUSPEND;
        }
    } else if unsafe { (*envp).host.wait_again(pc, [lo, len]) } {
        return TIER_SUSPEND;
    }
    unsafe { (*mem).sync(&mut *(*envp).host) };
    let r = catch_unwind(AssertUnwindSafe(|| unsafe {
        (*envp).host.wait_suspend(&mut (*mem).fr, pc, [lo, len])
    }));
    // SAFETY: the pointers are the ones compiled code was given.
    unsafe { stepped(envp, mem, pc, r) }
}

/// `lo` of `yavs_rt_wait_same` for a wait with a `WaitEdge` (`yavs_interp::static_wait_edge`):
/// it short-cuts only in an activation the process's edge trigger resumed (`TierFrame::known`),
/// where `Ctx::wait_edge` does nothing (#772).
pub const WAIT_SAME_EDGE: u64 = 1;

/// A `WaitOn` that is not the last of its wait (a sensitivity list of several signals, #532):
/// the kernel's registration of a waiter it had at the last wait (`TierHost::wait_on_again`),
/// else the instruction through `TierHost::signal`. Returns `GO` or what `yavs_rt_signal` does.
pub extern "C" fn yavs_rt_wait_on(
    env: *mut c_void,
    regs: *mut u64,
    mem: *mut Mem,
    pc: u32,
    lo: u64,
    len: u64,
) -> u32 {
    cb(Cb::WaitOn);
    // SAFETY: as in `yavs_rt_step`.
    let envp = env as *mut Env<'_>;
    if unsafe { (*envp).host.wait_on_again([lo, len]) } {
        return GO;
    }
    // SAFETY: as in `yavs_rt_step`.
    unsafe { yavs_rt_signal(env, regs, mem, pc, 0) }
}

/// A `WaitOn` that is not the last of its wait (a sensitivity list of several signals, #532):
/// whether the kernel took the registration of a waiter it had at the last wait
/// (`TierHost::wait_on_again`); 0 otherwise, and compiled code runs the instruction through
/// `yavs_rt_signal` in a slow path. It reads no register, so the caller has stored none.
pub extern "C" fn yavs_rt_wait_again(env: *mut c_void, lo: u64, len: u64) -> u32 {
    cb(Cb::WaitOn);
    // SAFETY: as in `yavs_rt_step`.
    let envp = env as *mut Env<'_>;
    u32::from(unsafe { (*envp).host.wait_on_again([lo, len]) })
}

/// The wait of a process that names the same signals at every activation
/// (`yavs_interp::static_wait`), at its first statement: `TIER_SUSPEND` at the `Suspend` at `pc`
/// when the kernel still holds the last wait (`TierHost::wait_same`), else `GO` (the wait runs).
pub extern "C" fn yavs_rt_wait_same(
    env: *mut c_void,
    _regs: *mut u64,
    mem: *mut Mem,
    pc: u32,
    lo: u64,
    _len: u64,
) -> u32 {
    cb(Cb::WaitSame);
    // SAFETY: as in `yavs_rt_step`.
    let envp = env as *mut Env<'_>;
    // On the kernel itself (#766), else through the host. Neither reads the temporary area.
    let fr = unsafe { &(*mem).fr };
    // The wait's `WaitEdge` changes nothing only when the edge trigger resumed the activation.
    if lo & WAIT_SAME_EDGE != 0 && fr.known == u64::MAX {
        return GO;
    }
    if fr.flags & yavs_interp::TIER_ROOT_WAIT != 0
        && let Some((c, _, _)) = unsafe { direct(fr) }
        && yavs_interp::direct_wait_same(c)
    {
        unsafe { (*mem).wait_at = pc };
        return TIER_SUSPEND;
    }
    // The host also notes why it refused.
    if unsafe { (*envp).host.wait_same(pc) } {
        TIER_SUSPEND
    } else {
        GO
    }
}

/// `Inst::WaitEdge` of the signal reference `lo`, `len` with edge code `kind` on the kernel
/// itself (#766): 1 when done, 0 when the instruction must run through `yavs_rt_signal`. It
/// reads no register.
pub extern "C" fn yavs_rt_wait_edge(
    _env: *mut c_void,
    mem: *mut Mem,
    lo: u64,
    len: u64,
    kind: u32,
) -> u32 {
    cb(Cb::WaitOn);
    // SAFETY: `mem` is the running activation's.
    match unsafe { direct(&(*mem).fr) } {
        Some((c, _, ss)) => u32::from(yavs_interp::direct_wait_edge(c, ss, [lo, len], kind as u8)),
        None => 0,
    }
}

/// `depth` of `yavs_rt_native` with this bit set: a frame-mode function's call at the pc in the
/// low 32 bits (`TierHost::native_frame`).
pub const NATIVE_FRAME_MODE: u64 = 1 << 63;

/// `TierHost::native_direct` for a `CallNative` of a native frame: `cells` (`n`) are the
/// argument cells, `depth` the frame index of the calling native frame (or `NATIVE_FRAME_MODE`
/// and the pc of a frame-mode caller). Returns 0 where the
/// interpreter must execute the instruction, 1 when the native ran, 2 when it ran and the
/// process ends (compiled code leaves at the next instruction); bit 8 says the native returned
/// a value, written to `out`.
pub extern "C" fn yavs_rt_native(
    env: *mut c_void,
    mem: *mut Mem,
    native: u32,
    cells: *const u64,
    n: u32,
    depth: u64,
    out: *mut u64,
) -> u32 {
    cb(Cb::Native);
    // SAFETY: as in `yavs_rt_step`; `cells` holds `n` words of the caller (an `alloca` of the
    // compiled code), `out` one word.
    let envp = env as *mut Env<'_>;
    unsafe { (*mem).sync(&mut *(*envp).host) };
    let r = catch_unwind(AssertUnwindSafe(|| unsafe {
        let cells = std::slice::from_raw_parts(cells, n as usize);
        let env = &mut *envp;
        if depth & NATIVE_FRAME_MODE != 0 {
            return env
                .host
                .native_frame(&mut (*mem).fr, depth as u32, native, cells);
        }
        let w = env
            .windows
            .get_or_insert_with(|| Box::new(TabSlots((*mem).nslots)));
        let _ = w;
        env.host
            .native_direct(&mut (*mem).fr, native, cells, depth, &mut env.windows)
    }));
    // SAFETY: as above.
    let m = unsafe { &mut *mem };
    m.temp_seen = m.fr.temp_len;
    m.forget();
    let put = |v: Option<u64>| {
        if let Some(v) = v {
            // SAFETY: `out` is one word of the caller.
            unsafe { *out = v };
        }
        u32::from(v.is_some()) << 8
    };
    match r {
        Ok(NativeCall::Done(v)) => 1 | put(v),
        Ok(NativeCall::Stop(v)) => 2 | put(v),
        Ok(NativeCall::Slow) => 0,
        Err(p) => {
            // SAFETY: as above.
            unsafe { (*envp).panic = Some(p) };
            0
        }
    }
}

/// `yavs_rt_native` for a light native (`yavs_interp::light_native`, `TierHost::native_light`):
/// same arguments and results. The native allocates only temporary memory, so no heap object
/// the compiled code found can have moved; the volatile ones are forgotten when any are held.
pub unsafe extern "C" fn yavs_rt_native_light(
    env: *mut c_void,
    mem: *mut Mem,
    native: u32,
    cells: *const u64,
    n: u32,
    depth: u64,
    out: *mut u64,
) -> u32 {
    cb(Cb::Native);
    // SAFETY: as in `yavs_rt_native`.
    let envp = env as *mut Env<'_>;
    unsafe { (*mem).sync(&mut *(*envp).host) };
    let r = catch_unwind(AssertUnwindSafe(|| unsafe {
        let cells = std::slice::from_raw_parts(cells, n as usize);
        let env = &mut *envp;
        let at = (depth & NATIVE_FRAME_MODE != 0).then_some(depth as u32);
        if at.is_none() {
            let _ = env
                .windows
                .get_or_insert_with(|| Box::new(TabSlots((*mem).nslots)));
        }
        env.host
            .native_light(&mut (*mem).fr, native, cells, at, depth, &mut env.windows)
    }));
    // SAFETY: as above.
    let m = unsafe { &mut *mem };
    m.temp_seen = m.fr.temp_len;
    m.forget_end();
    match r {
        Ok(NativeCall::Done(v)) => 1 | put_out(v, out),
        Ok(NativeCall::Stop(v)) => 2 | put_out(v, out),
        Ok(NativeCall::Slow) => 0,
        Err(p) => {
            // SAFETY: as above.
            unsafe { (*envp).panic = Some(p) };
            0
        }
    }
}

/// `TierHost::memo_probe` for a call of a memoised function from a native frame (`depth` its
/// index) or a frame-mode function (`NATIVE_FRAME_MODE`): `cells` (`n`) are the argument cells.
/// Returns 0 when the call is made as usual, 1 on a hit (the result is `out[0]`), 2 on a miss
/// (the reports so far are `out[1]`: the call is made, then `yavs_rt_memo_keep`).
pub unsafe extern "C" fn yavs_rt_memo(
    env: *mut c_void,
    mem: *mut Mem,
    func: u32,
    cells: *const u64,
    n: u32,
    depth: u64,
    out: *mut u64,
) -> u32 {
    cb(Cb::Memo);
    // SAFETY: as in `yavs_rt_native`; `out` is two words of the caller.
    let envp = env as *mut Env<'_>;
    unsafe { (*mem).sync(&mut *(*envp).host) };
    let r = catch_unwind(AssertUnwindSafe(|| unsafe {
        let cells = std::slice::from_raw_parts(cells, n as usize);
        let env = &mut *envp;
        let depth = (depth & NATIVE_FRAME_MODE == 0).then_some(depth);
        if depth.is_some() {
            let _ = env
                .windows
                .get_or_insert_with(|| Box::new(TabSlots((*mem).nslots)));
        }
        env.host
            .memo_probe(&mut (*mem).fr, FuncId(func), cells, depth, &mut env.windows)
    }));
    // SAFETY: as above.
    let m = unsafe { &mut *mem };
    m.temp_seen = m.fr.temp_len;
    m.forget_end();
    match r {
        Ok(MemoLook::Hit(v)) => {
            unsafe { *out = v };
            1
        }
        Ok(MemoLook::Miss(reports)) => {
            unsafe { *out.add(1) = reports };
            2
        }
        Ok(MemoLook::No) => 0,
        Err(p) => {
            unsafe { (*envp).panic = Some(p) };
            0
        }
    }
}

/// `TierHost::memo_keep` for a call that `yavs_rt_memo` answered with 2: the result is `out[0]`,
/// the reports then `out[1]`.
pub unsafe extern "C" fn yavs_rt_memo_keep(
    env: *mut c_void,
    mem: *mut Mem,
    func: u32,
    cells: *const u64,
    n: u32,
    depth: u64,
    out: *mut u64,
) -> u32 {
    cb(Cb::Memo);
    // SAFETY: as `yavs_rt_memo`.
    let envp = env as *mut Env<'_>;
    unsafe { (*mem).sync(&mut *(*envp).host) };
    let r = catch_unwind(AssertUnwindSafe(|| unsafe {
        let cells = std::slice::from_raw_parts(cells, n as usize);
        let env = &mut *envp;
        let depth = (depth & NATIVE_FRAME_MODE == 0).then_some(depth);
        if depth.is_some() {
            let _ = env
                .windows
                .get_or_insert_with(|| Box::new(TabSlots((*mem).nslots)));
        }
        let (v, reports) = (*out, *out.add(1));
        env.host.memo_keep(
            &mut (*mem).fr,
            FuncId(func),
            cells,
            depth,
            &mut env.windows,
            reports,
            v,
        );
    }));
    // SAFETY: as above.
    let m = unsafe { &mut *mem };
    // The machine's temporary area has the length `sync` gave it.
    m.temp_seen = m.fr.temp_len;
    if let Err(p) = r {
        // SAFETY: as above.
        unsafe { (*envp).panic = Some(p) };
    }
    0
}

/// Write a native's result to `out` (one word of the caller); bit 8 says there was one.
#[inline(always)]
pub fn put_out(v: Option<u64>, out: *mut u64) -> u32 {
    if let Some(v) = v {
        // SAFETY: `out` is one word of the caller.
        unsafe { *out = v };
    }
    u32::from(v.is_some()) << 8
}

/// `TierHost::temp_root` for a `TempRoot` of a native frame: `ty` is `u32::MAX` for a fat pair,
/// else the type with its `n` descriptor bytes at `desc`. 1 when recorded, 0 where the
/// interpreter must execute the instruction.
pub extern "C" fn yavs_rt_troot(
    env: *mut c_void,
    mem: *mut Mem,
    addr: u64,
    ty: u32,
    desc: *const u8,
    n: u64,
) -> u32 {
    cb(Cb::Troot);
    // SAFETY: as in `yavs_rt_step`; compiled code checked `n` readable bytes at `desc`.
    let envp = env as *mut Env<'_>;
    unsafe { (*mem).sync(&mut *(*envp).host) };
    let r = catch_unwind(AssertUnwindSafe(|| unsafe {
        let d: &[u8] = if n == 0 {
            &[]
        } else {
            std::slice::from_raw_parts(desc, n as usize)
        };
        let ty = (ty != u32::MAX).then_some(ty);
        (*envp).host.temp_root(&mut (*mem).fr, addr, ty, d)
    }));
    // SAFETY: as above.
    let m = unsafe { &mut *mem };
    m.temp_seen = m.fr.temp_len;
    match r {
        Ok(ok) => u32::from(ok),
        Err(p) => {
            // SAFETY: as above.
            unsafe { (*envp).panic = Some(p) };
            0
        }
    }
}

/// `TierHost::end_stmt`: the end of a statement of a native frame whose temporaries start at
/// `base`, while temporaries are recorded for the collector.
pub extern "C" fn yavs_rt_endstmt(env: *mut c_void, mem: *mut Mem, base: u64) {
    cb(Cb::Endstmt);
    // SAFETY: as in `yavs_rt_step`.
    let envp = env as *mut Env<'_>;
    unsafe { (*mem).sync(&mut *(*envp).host) };
    let r = catch_unwind(AssertUnwindSafe(|| unsafe {
        (*envp).host.end_stmt(&mut (*mem).fr, base);
    }));
    // SAFETY: as above.
    let m = unsafe { &mut *mem };
    m.temp_seen = m.fr.temp_len;
    if let Err(p) = r {
        // SAFETY: as above.
        unsafe { (*envp).panic = Some(p) };
    }
}

/// `TierHost::native_step`: instruction `pc` of `func` for a native frame at frame index `depth`
/// whose register values `ins` (registers `use_idx`) compiled code passes; the registers the
/// instruction writes (`def_idx`) come back in `outs`. 0 where the interpreter must execute the
/// instruction, 1 when it ran, 2 when it ran and the process ends.
pub extern "C" fn yavs_rt_xstep(
    env: *mut c_void,
    mem: *mut Mem,
    func: u32,
    pc: u32,
    depth: u64,
    use_idx: *const u32,
    ins: *const u64,
    nin: u32,
    def_idx: *const u32,
    outs: *mut u64,
    nout: u32,
    temp_base: u64,
) -> u32 {
    cb(Cb::Xstep);
    // SAFETY: as in `yavs_rt_step`; `ins` holds `nin` words, `outs` `nout`, the index arrays as
    // many `u32`, all of the calling native frame (allocas and constants of compiled code).
    let envp = env as *mut Env<'_>;
    unsafe { (*mem).sync(&mut *(*envp).host) };
    let r = catch_unwind(AssertUnwindSafe(|| unsafe {
        let ins = std::slice::from_raw_parts(ins, nin as usize);
        let outs = std::slice::from_raw_parts_mut(outs, nout as usize);
        let uses = std::slice::from_raw_parts(use_idx, nin as usize);
        let defs = std::slice::from_raw_parts(def_idx, nout as usize);
        let env = &mut *envp;
        env.windows
            .get_or_insert_with(|| Box::new(TabSlots((*mem).nslots)));
        env.host.native_step(
            &mut (*mem).fr,
            func,
            pc,
            depth,
            (uses, defs),
            ins,
            outs,
            &mut env.windows,
            temp_base,
        )
    }));
    // SAFETY: as above.
    let m = unsafe { &mut *mem };
    m.temp_seen = m.fr.temp_len;
    m.forget();
    match r {
        Ok(NativeCall::Done(_)) => 1,
        Ok(NativeCall::Stop(_)) => 2,
        Ok(NativeCall::Slow) => 0,
        Err(p) => {
            // SAFETY: as above.
            unsafe { (*envp).panic = Some(p) };
            0
        }
    }
}

/// `Inst::LCopy` on the `ns` source and `nd` destination plane blocks compiled code found
/// (`Gen::host`): `yavs_interp::copy_bits`, the interpreter's own copy, on stack copies of both
/// (they may be one vector).
pub unsafe extern "C" fn yavs_rt_lcopy(
    dst: *mut u8,
    nd: u64,
    d0: u64,
    src: *const u8,
    ns: u64,
    s0: u64,
    n: u64,
) {
    cb(Cb::Lcopy);
    // Up to 64 elements: the run read into registers, then written (so also where the two
    // overlap), no staging of the blocks.
    if n <= 64 && s0 < 64 && d0 < 64 && (s0 + n).div_ceil(64) <= ns && (d0 + n).div_ceil(64) <= nd {
        // SAFETY: compiled code checked `ns` readable blocks at `src` and `nd` writable blocks at
        // `dst` (`Gen::host`), and the run lies inside them (the test above).
        unsafe {
            let v = crate::eloop::get_run(src, 0, s0 as usize, n as usize);
            crate::eloop::put_run(dst, 0, d0 as usize, n as usize, v);
        }
        return;
    }
    let blocks = |b: &[u8], out: &mut [[u64; 4]]| {
        for (o, c) in out.iter_mut().zip(b.as_chunks::<32>().0) {
            *o = std::array::from_fn(|k| u64::from_le_bytes(std::array::from_fn(|j| c[k * 8 + j])));
        }
    };
    let (ns, nd) = (ns.min(LCOPY_BLOCKS) as usize, nd.min(LCOPY_BLOCKS) as usize);
    let mut sw = [[0u64; 4]; LCOPY_BLOCKS as usize];
    let mut dw = [[0u64; 4]; LCOPY_BLOCKS as usize];
    // SAFETY: compiled code checked `ns` readable blocks at `src` and `nd` writable blocks at
    // `dst` (`Gen::host`, both at most `LCOPY_BLOCKS`); each slice is dead before the next
    // one exists.
    unsafe {
        blocks(std::slice::from_raw_parts(src, ns * 32), &mut sw[..ns]);
        blocks(std::slice::from_raw_parts(dst, nd * 32), &mut dw[..nd]);
    }
    yavs_interp::copy_bits(&sw[..ns], &mut dw[..nd], s0, d0, n);
    // SAFETY: as above.
    let d = unsafe { std::slice::from_raw_parts_mut(dst, nd * 32) };
    for (c, w) in d
        .as_chunks_mut::<8>()
        .0
        .iter_mut()
        .zip(dw[..nd].as_flattened())
    {
        *c = w.to_le_bytes();
    }
}

/// `Inst::MemEq` on the `n` bytes compiled code found at `a` and `b` (`Gen::host`): 1 when equal.
pub unsafe extern "C" fn yavs_rt_memeq(a: *const u8, b: *const u8, n: u64) -> u64 {
    // SAFETY: compiled code checked `n` readable bytes at each.
    unsafe {
        u64::from(
            std::slice::from_raw_parts(a, n as usize) == std::slice::from_raw_parts(b, n as usize),
        )
    }
}

/// Word `k` of the layout of type `ty` for the `dsz` descriptor bytes at `desc`: an
/// `Inst::LayoutOf` whose layout only the `Field` after it reads (`Gen::layout_field`), answered
/// from the interpreter's layout memo (`TierHost::layout_word`) without a step, a view or a
/// change to the objects compiled code found. `u64::MAX` where the host declines: compiled code
/// then leaves to the interpreter at the `LayoutOf`, which reports any error.
pub unsafe extern "C" fn yavs_rt_layout_word(
    env: *mut c_void,
    ty: u32,
    desc: *const u8,
    dsz: u64,
    k: u32,
) -> u64 {
    cb(Cb::Layout);
    let envp = env as *mut Env<'_>;
    // SAFETY: compiled code checked `dsz` readable bytes at `desc`; `env` is the activation's.
    let r = catch_unwind(AssertUnwindSafe(|| unsafe {
        let d = std::slice::from_raw_parts(desc, dsz as usize);
        (*envp).host.layout_word(ty, d, k)
    }));
    match r {
        Ok(Some(w)) => w,
        Ok(None) => u64::MAX,
        Err(p) => {
            // SAFETY: as above.
            unsafe { (*envp).panic = Some(p) };
            u64::MAX
        }
    }
}

/// The plane block of `groups` groups of eight `std_ulogic` codes at `codes`, as `Inst::SigRead`
/// of up to 64 elements makes it: the four planes (element 0 in bit 0) masked by `mask` into the
/// 32 bytes at `out`. Pure, no host state: compiled code calls it where it does not know the
/// length (`Gen::sig_read_block`).
pub unsafe extern "C" fn yavs_rt_codes(codes: *const u8, groups: u64, mask: u64, out: *mut u64) {
    let mut planes = [0u64; 4];
    let mut j = 0usize;
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::*;
        // Sixteen codes at a time: bit p of every byte is the top bit after a shift by 7 - p.
        // SAFETY: the caller checked `groups * 8` readable bytes at `codes`; SSE2 is the baseline.
        unsafe {
            while j + 2 <= groups as usize {
                let v = _mm_loadu_si128(codes.add(8 * j).cast());
                planes[0] |= (_mm_movemask_epi8(_mm_slli_epi16::<7>(v)) as u64) << (8 * j);
                planes[1] |= (_mm_movemask_epi8(_mm_slli_epi16::<6>(v)) as u64) << (8 * j);
                planes[2] |= (_mm_movemask_epi8(_mm_slli_epi16::<5>(v)) as u64) << (8 * j);
                planes[3] |= (_mm_movemask_epi8(_mm_slli_epi16::<4>(v)) as u64) << (8 * j);
                j += 2;
            }
        }
    }
    while j < groups as usize {
        // SAFETY: the caller checked `groups * 8` readable bytes at `codes`.
        let w = unsafe { std::ptr::read_unaligned(codes.add(8 * j).cast::<u64>()) };
        for (p, plane) in planes.iter_mut().enumerate() {
            let bits = (w >> p) & 0x0101_0101_0101_0101;
            *plane |= (bits.wrapping_mul(0x0102_0408_1020_4080) >> 56) << (8 * j);
        }
        j += 1;
    }
    for (p, plane) in planes.iter().enumerate() {
        // SAFETY: the caller checked 32 writable bytes at `out`.
        unsafe { std::ptr::write_unaligned(out.add(p), plane & mask) };
    }
}

/// `Inst::SigReadScalar` of element `off` of the signal at `ent` when its codes are not offered:
/// the four plane bits of a logic signal, or the bytes of an enumeration, integer, boolean or real
/// one (a four-byte element sign-extended), stored at `out`. Pure, no host state. 0 (nothing
/// stored): the element is not within what the entry offers, or it has an element size compiled
/// code does not read; compiled code runs the instruction in the interpreter's way then.
pub unsafe extern "C" fn yavs_rt_scalar(
    ent: *const yavs_interp::TierSig,
    off: u64,
    out: *mut u64,
) -> u32 {
    // SAFETY: `ent` is an entry of the table compiled code holds.
    let e = unsafe { &*ent };
    if off >= e.plen {
        return 0;
    }
    let el = e.off + off;
    let v = if e.planes != 0 {
        // SAFETY: `el` is within the elements the entry offers, in whole blocks of four planes.
        let blk = unsafe { (e.planes as *const u64).add(4 * (el >> 6) as usize) };
        let sh = el & 63;
        (0..4).fold(0u64, |acc, p| {
            // SAFETY: as above.
            acc | (((unsafe { *blk.add(p) } >> sh) & 1) << p)
        })
    } else if e.bytes != 0 {
        let at = (e.bytes + el * e.es) as usize;
        // SAFETY: the element is within the bytes the entry offers.
        unsafe {
            match e.es {
                1 => u64::from(*(at as *const u8)),
                4 => i64::from(std::ptr::read_unaligned(at as *const i32)) as u64,
                8 => std::ptr::read_unaligned(at as *const u64),
                _ => return 0,
            }
        }
    } else {
        return 0;
    };
    // SAFETY: the register slot compiled code passes.
    unsafe { *out = v };
    1
}

/// `Inst::SigRead` of `n` elements from element `bit0` of the `nblk` plane blocks of a kernel
/// signal at `src`: the planes of the elements, four words per 64 elements, the bits past `n`
/// zero, as `Ctx::read_into` writes them to `dst`. Pure, no host state: compiled code calls it
/// where the signal keeps no codes or the read is wider than 64 elements.
pub unsafe extern "C" fn yavs_rt_planes(
    src: *const u64,
    nblk: u64,
    bit0: u64,
    n: u64,
    dst: *mut u64,
) {
    for c in 0..n.div_ceil(64) {
        let k = (n - c * 64).min(64);
        let from = bit0 + c * 64;
        let (w, sh) = ((from >> 6) as usize, from & 63);
        let mask = if k == 64 { !0 } else { (1u64 << k) - 1 };
        for p in 0..4usize {
            // SAFETY: compiled code checked that `bit0 + n` lies within the signal's `nblk`
            // blocks and that `dst` has `n.div_ceil(64)` blocks.
            unsafe {
                let lo = *src.add(w * 4 + p) >> sh;
                let hi = if sh != 0 && (w as u64 + 1) < nblk {
                    *src.add((w + 1) * 4 + p) << (64 - sh)
                } else {
                    0
                };
                *dst.add(c as usize * 4 + p) = (lo | hi) & mask;
            }
        }
    }
}

/// The body of `yavs_rt_step`.
///
/// # Safety
/// `env` and `mem` are the live `Env` and `Mem` of the compiled code calling `yavs_rt_step`.
pub unsafe fn step_in(env: *mut Env<'_>, mem: *mut Mem, pc: u32) -> u32 {
    // SAFETY: see `yavs_rt_step`.
    unsafe {
        let stack = (*mem).fr.stack;
        let stepped = (*env).host.step(&mut (*mem).fr, pc);
        // The interpreter's view: the temporary area has the length it shows.
        (*mem).temp_seen = (*mem).fr.temp_len;
        let callee = match stepped {
            Stepped::Next => return GO,
            Stepped::Fault => return pc,
            Stepped::Suspend => return TIER_SUSPEND,
            Stepped::Call(callee) => callee,
        };
        // The callee runs here when it has compiled code, the register stack did not move
        // (the callers' compiled code holds pointers into it) and native stack is left.
        let Some((run, nregs)) = ((*env).lookup)(callee) else {
            return TIER_UNWIND;
        };
        let fr = &(*mem).fr;
        if fr.stack != stack
            || fr.nregs != nregs as u64
            || fr.frame >= 1 << 29
            || stack_here() < (*env).stack_lo
        {
            return TIER_UNWIND;
        }
        // The callee runs on the same `Mem`: the interpreter keeps its view current. The push
        // may have moved the slot stack under the objects found before.
        (*mem).forget();
        let next = run(env as *mut c_void, fr.regs, mem, 0);
        (*mem).sync(&mut *(*env).host);
        if (*env).panic.is_some() || next == TIER_UNWIND || next == TIER_SUSPEND {
            return next;
        }
        let ok = (*env).host.ret(&mut (*mem).fr, next);
        (*mem).temp_seen = (*mem).fr.temp_len;
        if ok { GO } else { TIER_UNWIND }
    }
}

impl Mem {
    /// The object `a` points into, as its start and length, where `find` reaches it (reading),
    /// and the slots of the native frames between the interpreter's top frame and frame
    /// `depth` (registered in `nslots`; a closed frame is not there).
    fn span(&self, a: u64, depth: u64) -> Option<(*const u8, u64)> {
        let hi = a >> 32;
        let fr = &self.fr;
        let (base, len) = if hi == fr.frame_hi {
            (fr.slots.cast_const(), fr.slots_len)
        } else if hi == u64::from(space::TEMP) << 29 {
            (fr.temp.cast_const(), fr.temp_len)
        } else if hi == u64::from(space::INST) << 29 {
            (fr.inst, fr.inst_len)
        } else if hi >> 29 == u64::from(space::FRAME) {
            let idx = hi & ((1 << 29) - 1);
            if idx <= fr.frame || idx > depth || self.nslots.is_null() {
                return None;
            }
            // SAFETY: the table has an entry for every frame index up to the depth limit and
            // the entries of frames above the interpreter's top frame and up to `depth` are those
            // of live native frames (their entry writes them, closed frames write length 0).
            let e = unsafe { *self.nslots.add(idx as usize) };
            if e.len == 0 {
                return None;
            }
            (e.ptr.cast_const(), e.len)
        } else {
            let o = self.get(hi)?;
            (o.ptr.cast_const(), o.len)
        };
        ((a & 0xffff_ffff) <= len).then_some((base, len))
    }

    /// The bytes of the region address `a` points into, from the region's start (`Gen::host`
    /// holds them for a whole activation, `param_addrs`). The slots of a native caller between
    /// the interpreter's top frame and frame `depth` live as long as the callee: always. The top
    /// frame, the temporary area, the instance data (not for writing) and a heap object of the
    /// table only when `stable` (`stable_activation`: nothing the function does moves them).
    #[inline]
    fn region(&self, a: u64, depth: u64, write: bool, stable: bool) -> Option<(*mut u8, u64)> {
        let hi = a >> 32;
        let fr = &self.fr;
        if hi >> 29 == u64::from(space::FRAME) && hi != fr.frame_hi {
            let idx = hi & ((1 << 29) - 1);
            if idx > fr.frame && idx < depth && !self.nslots.is_null() {
                // SAFETY: as in `span`.
                let e = unsafe { *self.nslots.add(idx as usize) };
                return (e.len != 0).then_some((e.ptr, e.len));
            }
        }
        if !stable {
            return None;
        }
        if hi == fr.frame_hi {
            Some((fr.slots, fr.slots_len))
        } else if hi == u64::from(space::TEMP) << 29 {
            Some((fr.temp, fr.temp_len))
        } else if hi == u64::from(space::INST) << 29 {
            (!write).then(|| (fr.inst.cast_mut(), fr.inst_len))
        } else {
            self.get(hi)
                .filter(|o| !write || o.w != 0)
                .map(|o| (o.ptr, o.len))
        }
    }

    /// The host pointer to `n` bytes at address `a` plus `extra` where they lie in the frame's
    /// slots, the temporary area, (reading) the instance data or a cached object.
    pub fn find(&self, a: u64, extra: u64, n: u64, write: bool) -> Option<*mut u8> {
        let hi = a >> 32;
        // `extra` is below 2^63: no wrap.
        let off = (a & 0xffff_ffff) + extra;
        let fr = &self.fr;
        let (base, len) = if hi == fr.frame_hi {
            (fr.slots, fr.slots_len)
        } else if hi == u64::from(space::TEMP) << 29 {
            (fr.temp, fr.temp_len)
        } else if !write && hi == u64::from(space::INST) << 29 {
            (fr.inst.cast_mut(), fr.inst_len)
        } else if hi == fr.up[0] {
            // A variable of the enclosing subprogram passed by reference (length 0: not offered).
            (fr.up_slots[0], fr.up_len[0])
        } else if hi == fr.up[1] {
            (fr.up_slots[1], fr.up_len[1])
        } else {
            let o = self.get(hi)?;
            if write && o.w == 0 {
                return None;
            }
            (o.ptr, o.len)
        };
        (n <= len && off <= len - n).then(|| base.wrapping_add(off as usize))
    }
}

/// The host pointer for an access of compiled code (`Gen::host`), or null when the access is
/// the interpreter's (stepped): what `rd`/`wr` would reach without an error.
pub unsafe extern "C" fn yavs_rt_host(
    env: *mut c_void,
    mem: *mut Mem,
    a: u64,
    extra: u64,
    n: u64,
    write: u32,
) -> *mut u8 {
    cb(Cb::Host);
    // SAFETY: as in `yavs_rt_step`.
    let m = unsafe { &*mem };
    // The view's bytes are a leaf: no register saves for the common hit (the slow path is apart).
    if let Some(p) = m.find(a, extra, n, write != 0) {
        return p;
    }
    host_miss(env, mem, a, extra, n, write)
}

/// `yavs_rt_host` where the view does not hold the bytes: the interpreter's object table.
#[cold]
#[inline(never)]
extern "C" fn host_miss(
    env: *mut c_void,
    mem: *mut Mem,
    a: u64,
    extra: u64,
    n: u64,
    write: u32,
) -> *mut u8 {
    // SAFETY: the pointers are the ones compiled code was given.
    if unsafe { yavs_rt_obj(env, mem, a, write) } == 0 {
        return std::ptr::null_mut();
    }
    // SAFETY: as above.
    let m = unsafe { &*mem };
    m.find(a, extra, n, write != 0)
        .unwrap_or(std::ptr::null_mut())
}

/// `yavs_rt_region(env, mem, a, depth, flags, out)`: where the region address `a` (a parameter of
/// a native frame) points into, `out = [host pointer of the region's start, length]` (0, 0 where
/// the lookup does not reach it). Flags: 1 resolve for writing, 2 the function is a stable
/// activation (`stable_activation`). Compiled code tests its accesses against the pair; what
/// fails the test takes the ordinary lookup (`Gen::host`).
pub unsafe extern "C" fn yavs_rt_region(
    env: *mut c_void,
    mem: *mut Mem,
    a: u64,
    depth: u64,
    flags: u32,
    out: *mut [u64; 2],
) {
    cb(Cb::Region);
    // Flag 4: everything but the temporary area stays where it is (`fixed_activation`).
    let fixed = flags & 4 != 0 && a >> 32 != u64::from(space::TEMP) << 29;
    let (write, stable) = (flags & 1 != 0, flags & 2 != 0 || fixed);
    // SAFETY: as in `yavs_rt_step`.
    let mut r = unsafe { &*mem }.region(a, depth, write, stable);
    if r.is_none() && stable && unsafe { yavs_rt_obj(env, mem, a, u32::from(write)) } != 0 {
        // SAFETY: as above; the object is in the table now.
        r = unsafe { &*mem }.region(a, depth, write, stable);
    }
    let (p, len) = r.map_or((0, 0), |(p, len)| (p as u64, len));
    // SAFETY: compiled code gave the address of its two-word buffer.
    unsafe { *out = [p, len] };
}

/// `yavs_rt_scan(env, mem, ctl, dope, base, i0, x, e, delta, out)`: the iterations of a search loop
/// (`yavs_interp::search_loops`) compiled code is at the head of, as the interpreter runs them
/// (`yavs_interp::scan_bytes`); `ctl` packs the shape (`scan_ctl`). The descriptor and the array's
/// object are looked up once; `out` gets the index, the element, the exit taken and the iterations.
/// Returns the iterations done, 0 where the generic code must go on (memory the table does not
/// reach, an index outside the bounds): it errs at the same iteration then.
pub unsafe extern "C" fn yavs_rt_scan(
    env: *mut c_void,
    mem: *mut Mem,
    ctl: u64,
    dope: u64,
    base: u64,
    i0: i64,
    x: i64,
    e: i64,
    delta: i64,
    out: *mut i64,
    depth: u64,
) -> u32 {
    cb(Cb::Scan);
    let shape = yavs_interp::ScanShape {
        stride: (ctl >> 8) as u32,
        w: match ctl >> 40 & 3 {
            0 => yavs_ir::Width::B1,
            1 => yavs_ir::Width::B4,
            _ => yavs_ir::Width::B8,
        },
        m: (ctl >> 42 & 7) as u8,
        m2: (ctl >> 45 & 7) as u8,
        idx_left: ctl >> 48 & 1 != 0,
        delta,
    };
    // The object `a` points into, looked up in the object table when it is not in the view.
    let span = |a: u64| {
        // SAFETY: as in `yavs_rt_step`.
        if let Some(s) = unsafe { &*mem }.span(a, depth) {
            return Some(s);
        }
        if unsafe { yavs_rt_obj(env, mem, a, 0) } == 0 {
            return None;
        }
        // SAFETY: as above.
        unsafe { &*mem }.span(a, depth)
    };
    let Some((dp, dlen)) = span(dope) else {
        return 0;
    };
    let at = (dope & 0xffff_ffff) + (ctl & 0xff) * 24;
    if at + 24 > dlen {
        return 0;
    }
    // SAFETY: the 24 bytes at `dp + at` lie in the descriptor's object (checked).
    let word =
        |k: usize| unsafe { std::ptr::read_unaligned(dp.add(at as usize + k * 8).cast::<i64>()) };
    let dim = (word(0), word(1), word(2) as u64);
    let Some((bp, blen)) = span(base) else {
        return 0;
    };
    // SAFETY: the object's `blen` bytes at `bp` are what `Mem::find` hands out for reading;
    // nothing runs between here and the end of the scan that could free them.
    let bytes = unsafe { std::slice::from_raw_parts(bp, blen as usize) };
    let from = yavs_interp::ScanFrom {
        i0,
        x,
        e,
        limit: 1 << 24,
    };
    let sc = yavs_interp::scan_bytes(&shape, dim, bytes, base & 0xffff_ffff, from);
    // SAFETY: `out` is four words of the caller.
    unsafe {
        *out = sc.i;
        *out.add(1) = sc.ch as i64;
        *out.add(2) = i64::from(sc.hit);
    }
    sc.iters
}

/// The memory of an element loop in a native frame: the table of objects, the frames of the
/// native callers and the calling frame's own slots.
struct LoopObjects {
    env: *mut c_void,
    mem: *mut Mem,
    depth: u64,
    /// The calling native frame's own slots (`Mem::region` finds the frames above it only).
    fhi: u64,
    slots: *mut u8,
    slots_len: u64,
}

impl crate::eloop::LoopMem for LoopObjects {
    fn region(&mut self, a: u64, write: bool) -> Option<(*mut u8, u64)> {
        if a >> 32 == self.fhi && !self.slots.is_null() {
            return Some((self.slots, self.slots_len));
        }
        // SAFETY: the pointers are the ones compiled code was given.
        let m = unsafe { &*self.mem };
        if let Some(r) = m.region(a, self.depth, write, true) {
            return Some(r);
        }
        // SAFETY: as above; the object is in the table afterwards.
        if unsafe { yavs_rt_obj(self.env, self.mem, a, u32::from(write)) } == 0 {
            return None;
        }
        // SAFETY: as above.
        unsafe { &*self.mem }.region(a, self.depth, write, true)
    }
}

/// `yavs_rt_eloop(env, mem, prog, nprog, inputs, ind, bound, step, depth) -> iterations`: all but
/// the last iterations of an element loop (`yavs_interp::elem_loops`) compiled code is at the
/// head of (`crate::eloop::run`; `prog` is the loop's steps, a constant of the module, `inputs` the
/// registers they read). Returns the iterations done, 0 where the generic code must go on: it
/// then does every iteration and fails where and as it would.
pub unsafe extern "C" fn yavs_rt_eloop(
    env: *mut c_void,
    mem: *mut Mem,
    prog: *const i64,
    nprog: u64,
    inputs: *const u64,
    ind: i64,
    bound: i64,
    step: i64,
    depth: u64,
    fhi: u64,
    slots: *mut u8,
    slots_len: u64,
) -> i64 {
    cb(Cb::Eloop);
    let mut objs = LoopObjects {
        env,
        mem,
        depth,
        fhi,
        slots,
        slots_len,
    };
    // SAFETY: compiled code passes its constant program of `nprog` aligned words and an array
    // of `MAX_INPUTS` words with the inputs it filled.
    let (words, inputs) = unsafe {
        (
            std::slice::from_raw_parts(prog, nprog as usize),
            std::slice::from_raw_parts(inputs, yavs_interp::eloop::MAX_INPUTS),
        )
    };
    crate::eloop::run(words, inputs, ind, bound, step, ELOOP_LIMIT, &mut objs)
}

/// `yavs_rt_eloop_exec(src, s_e0, s_step, dst, d_e0, d_step, code, m) -> iterations`: the
/// iterations a loop planned in compiled code (`Gen::eloop_plan`) has checked (`crate::eloop::exec_planned`).
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn yavs_rt_eloop_exec(
    src: *mut u8,
    s_e0: i64,
    s_step: i64,
    dst: *mut u8,
    d_e0: i64,
    d_step: i64,
    code: u64,
    m: i64,
) -> i64 {
    cb(Cb::Eloop);
    // SAFETY: compiled code checked every element against the memory the pointers point into.
    unsafe { crate::eloop::exec_planned(src, s_e0, s_step, dst, d_e0, d_step, code as u8, m) }
}

/// The most iterations one call of `yavs_rt_eloop` does (the generic code asks again after it).
pub const ELOOP_LIMIT: i64 = 1 << 24;

/// The `ctl` word of `yavs_rt_scan` for a search loop.
pub fn scan_ctl(l: &yavs_interp::SearchLoop) -> u64 {
    let s = &l.shape;
    let w = match s.w {
        yavs_ir::Width::B1 => 0,
        yavs_ir::Width::B4 => 1,
        _ => 2,
    };
    u64::from(l.dim)
        | u64::from(s.stride) << 8
        | w << 40
        | u64::from(s.m) << 42
        | u64::from(s.m2) << 45
        | u64::from(s.idx_left) << 48
}

/// `Inst::LayoutOf` of an array whose element layout is fixed (`StaticLayout::Array`): reads
/// the `dims` dimensions of the descriptor at `desc`, builds the four layout words in temporary
/// memory and puts its address in register `dst`; 1 when done, 0 where `Ty::layout` or the
/// temporary area would fail (the interpreter then executes the instruction, with its error).
pub unsafe extern "C" fn yavs_rt_layout(
    env: *mut c_void,
    regs: *mut u64,
    mem: *mut Mem,
    desc: u64,
    dst: u32,
    dims: u32,
    stride: u64,
    align: u64,
    el_count: u64,
    logic: u32,
) -> u32 {
    cb(Cb::Layout);
    // SAFETY: the pointers are the ones compiled code was given.
    let p = unsafe { yavs_rt_host(env, mem, desc, 0, u64::from(dims) * 24, 0) };
    if p.is_null() {
        return 0;
    }
    let word = |k: usize| -> i64 {
        // SAFETY: `yavs_rt_host` checked `dims * 24` readable bytes at `p`.
        unsafe { std::ptr::read_unaligned(p.add(k * 8).cast::<i64>()) }
    };
    let mut count: u64 = 1;
    for k in 0..dims as usize {
        let (l, r, dir) = (word(k * 3), word(k * 3 + 1), word(k * 3 + 2));
        if dir as u64 > 1 {
            return 0;
        }
        let (lo, hi) = if dir == 1 { (r, l) } else { (l, r) };
        let n = hi.wrapping_sub(lo).wrapping_add(1) as u64;
        let len = if hi < lo {
            0
        } else if n == 0 {
            return 0;
        } else {
            n
        };
        let Some(c) = count.checked_mul(len) else {
            return 0;
        };
        count = c;
    }
    let words = if logic != 0 {
        let Some(n) = count.checked_mul(el_count) else {
            return 0;
        };
        [n.div_ceil(64) * 32, 8, 0, n]
    } else {
        let Some(size) = stride.checked_mul(count) else {
            return 0;
        };
        [size, align, stride, count]
    };
    // SAFETY: as in `yavs_rt_step`; the bytes written lie within the temporary area's capacity.
    let m = unsafe { &mut *mem };
    let len = m.fr.temp_len;
    let off = (len + 7) & !7;
    let end = off + 32;
    if end > 0xffff_ffff || end > m.fr.temp_cap {
        return 0;
    }
    // SAFETY: `temp` has `temp_cap >= end` bytes; `regs` has the function's registers
    // (`bad_reg` checked `dst`).
    unsafe {
        std::ptr::write_bytes(m.fr.temp.add(len as usize), 0, (end - len) as usize);
        for (k, w) in words.iter().enumerate() {
            std::ptr::write_unaligned(m.fr.temp.add(off as usize + k * 8).cast::<u64>(), *w);
        }
        *regs.add(dst as usize) = u64::from(space::TEMP) << 61 | off;
    }
    m.fr.temp_len = end;
    1
}

/// The length of dimension `k` of the descriptor at `dope` as `yavs_ir::dim_len`, `None`
/// where that is not defined or the memory is the interpreter's.
pub unsafe fn dim_at(env: *mut c_void, mem: *mut Mem, dope: u64, k: u64) -> Option<u64> {
    // SAFETY: the pointers are the ones compiled code was given.
    let p = unsafe { yavs_rt_host(env, mem, dope, k.checked_mul(24)?, 24, 0) };
    if p.is_null() {
        return None;
    }
    // SAFETY: `yavs_rt_host` checked 24 readable bytes at `p`.
    let w = |i: usize| unsafe { std::ptr::read_unaligned(p.add(i * 8).cast::<i64>()) };
    yavs_ir::dim_len(w(0), w(1), w(2) == 1)
}

/// Instructions whose inline form was large and no faster than this call: `FAST_ALLOC`
/// (`dst` = `Alloc`'s result register, `a` the size), `FAST_DOPE_LEN` (`dope` `a`, dimension
/// `b`, result register `dst`), `FAST_SUBTYPE` (`dst` dimensions of `a` against `b`),
/// `FAST_FAT_ROOT` (the fat pair at `a` points into no heap). 1 when
/// done, 0 where the interpreter must execute the instruction (its error, its memory).
pub unsafe extern "C" fn yavs_rt_fast(
    env: *mut c_void,
    regs: *mut u64,
    mem: *mut Mem,
    op: u32,
    dst: u32,
    a: u64,
    b: u64,
) -> u32 {
    cb(Cb::Fast);
    match op {
        FAST_ALLOC => {
            // SAFETY: as in `yavs_rt_step`.
            let m = unsafe { &mut *mem };
            let len = m.fr.temp_len;
            let off = (len + 7) & !7;
            let Some(end) = off.checked_add(a) else {
                return 0;
            };
            if end > 0xffff_ffff || end > m.fr.temp_cap {
                return 0;
            }
            // SAFETY: `temp` has `temp_cap >= end` bytes; `regs` holds register `dst`.
            unsafe {
                std::ptr::write_bytes(m.fr.temp.add(len as usize), 0, (end - len) as usize);
                *regs.add(dst as usize) = u64::from(space::TEMP) << 61 | off;
            }
            m.fr.temp_len = end;
            1
        }
        // SAFETY: the pointers are the ones compiled code was given.
        FAST_DOPE_LEN => match unsafe { dim_at(env, mem, a, b) } {
            // Lengths above i64::MAX are the interpreter's error.
            Some(n) if n <= i64::MAX as u64 => {
                // SAFETY: `regs` holds register `dst`.
                unsafe { *regs.add(dst as usize) = n };
                1
            }
            _ => 0,
        },
        FAST_FAT_ROOT => {
            // SAFETY: as in `yavs_rt_step`.
            let m = unsafe { &*mem };
            let Some(p) = m.find(a, 0, 16, false) else {
                return 0;
            };
            // 1 when neither word points into a heap (HEAP 4, SHARED_HEAP 7): nothing to record.
            let heap = |k: usize| {
                // SAFETY: `find` checked 16 readable bytes at `p`.
                let w = unsafe { std::ptr::read_unaligned(p.add(k * 8).cast::<u64>()) };
                matches!(w >> 61, 4 | 7)
            };
            u32::from(!heap(0) && !heap(1))
        }
        FAST_DEREF => {
            // SAFETY: as in `yavs_rt_step`.
            let env = unsafe { &mut *(env as *mut Env<'_>) };
            match catch_unwind(AssertUnwindSafe(|| env.host.deref(a))) {
                Ok(Some(addr)) => {
                    // SAFETY: `regs` holds register `dst`; `mem` as in `yavs_rt_step`.
                    unsafe { *regs.add(dst as usize) = addr };
                    // The object goes to its entry with `a`, writable (the accesses after a
                    // `Deref` read and write it): the next `Deref` of `a` and the accesses are
                    // inline.
                    let m = unsafe { &mut *mem };
                    let hi = addr >> 32;
                    m.check_lean(hi);
                    let (at, k) = m.slot(hi);
                    let e = &mut m.objs[at];
                    if e.hi == k && e.w != 0 {
                        e.h = a;
                    } else if let Ok(Some((p, len))) =
                        catch_unwind(AssertUnwindSafe(|| env.host.object(addr, true)))
                    {
                        m.put_obj(Obj {
                            hi,
                            ptr: p,
                            len: len as u64,
                            w: 1,
                            h: a,
                        });
                    }
                    1
                }
                Ok(None) => 0,
                Err(p) => {
                    env.panic = Some(p);
                    0
                }
            }
        }
        FAST_SUBTYPE => {
            for k in 0..u64::from(dst) {
                // SAFETY: as above.
                match (unsafe { dim_at(env, mem, a, k) }, unsafe {
                    dim_at(env, mem, b, k)
                }) {
                    (Some(x), Some(y)) if x == y => {}
                    _ => return 0,
                }
            }
            1
        }
        _ => 0,
    }
}

/// Finds the object `addr` points into for `Mem::find`: 1 when found, 0 when the access is
/// the interpreter's (stepped).
pub unsafe fn yavs_rt_obj(env: *mut c_void, mem: *mut Mem, addr: u64, write: u32) -> u32 {
    // SAFETY: as in `yavs_rt_step`.
    let env = unsafe { &mut *(env as *mut Env<'_>) };
    let mem = unsafe { &mut *mem };
    match catch_unwind(AssertUnwindSafe(|| env.host.object(addr, write != 0))) {
        Ok(Some((p, len))) => {
            let hi = addr >> 32;
            // The same object found before (by a `Deref`, for the other direction) keeps the
            // access value it was found for.
            let (h, w) = match mem.get(hi) {
                Some(e) => (e.h, e.w | u64::from(write)),
                None => (NO_H, u64::from(write)),
            };
            mem.put_obj(Obj {
                hi,
                ptr: p,
                len: len as u64,
                w,
                h,
            });
            1
        }
        Ok(None) => 0,
        Err(p) => {
            env.panic = Some(p);
            0
        }
    }
}

/// How an activation makes its view: any host through `TierHost::view_again` (`Tier::run`), the
/// interpreter's own `Host` of a process activation (`Tier::run_cached`) without the virtual call.
pub trait View {
    fn make_view(&mut self, fr: &mut TierFrame);
    fn as_host(&mut self) -> &mut dyn TierHost;
}

impl View for dyn TierHost + '_ {
    #[inline(always)]
    fn make_view(&mut self, fr: &mut TierFrame) {
        if !self.view_again(fr) {
            self.view(fr);
        }
    }
    #[inline(always)]
    fn as_host(&mut self) -> &mut dyn TierHost {
        self
    }
}

impl View for yavs_interp::Host<'_, '_, '_> {
    #[inline(always)]
    fn make_view(&mut self, fr: &mut TierFrame) {
        if !self.view_root_again(fr) {
            TierHost::view(self, fr);
        }
    }
    #[inline(always)]
    fn as_host(&mut self) -> &mut dyn TierHost {
        self
    }
}

/// Run compiled `f` (generated for `nregs` registers) from `pc` (`Tier::run`); `lookup`
/// finds the compiled code of the functions it calls. `lean` (with a record only): `f` is of a
/// process `regs::no_table` passes, which calls nothing and reaches no object of the machine's or
/// the heaps' tags.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
pub fn enter<H: View + ?Sized>(
    f: RunFn,
    nregs: usize,
    pc: u32,
    host: &mut H,
    lookup: Lookup<'_>,
    ntab: *const usize,
    state: Option<&mut yavs_interp::TierState>,
    lean: bool,
) -> Option<u32> {
    // This thread's `Mem`, or a new one inside an activation that holds it.
    let p = MEM.replace(std::ptr::null_mut());
    let mut mem = if p.is_null() {
        new_mem()
    } else {
        // SAFETY: `MEM` holds a `Box` of this thread's that nothing else uses (taken above).
        unsafe { Box::from_raw(p) }
    };
    host.make_view(&mut mem.fr);
    // Two copies of the rest, so that neither tests `lean` again (the view is made once: two
    // copies of it were a call).
    let r = if lean {
        enter_on::<H, true>(&mut mem, f, nregs, pc, host, lookup, ntab, state)
    } else {
        enter_on::<H, false>(&mut mem, f, nregs, pc, host, lookup, ntab, state)
    };
    if MEM.get().is_null() {
        MEM.set(Box::into_raw(mem));
    }
    match r {
        Ok(next) => next,
        Err(p) => resume_unwind(p),
    }
}

/// A new `Mem` (out of line: its 64 KB would otherwise be a stack probe in every `enter`).
#[inline(never)]
fn new_mem() -> Box<Mem> {
    Mem::new(TierFrame::default())
}

thread_local! {
    /// The `Mem` of the activations on this thread, kept between them so that none builds one
    /// (`enter`). ponytail: one `Mem` per thread is never freed, a thread-exit hook if threads
    /// come and go by the thousand.
    static MEM: std::cell::Cell<*mut Mem> = const { std::cell::Cell::new(std::ptr::null_mut()) };
}

/// `enter` on `mem` (a fresh or reused `Mem` whose view was just made): what the compiled code
/// returns, or the panic it carried out of a callback. `LEAN` as `enter`'s `lean`.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
pub fn enter_on<H: View + ?Sized, const LEAN: bool>(
    mem: &mut Mem,
    f: RunFn,
    nregs: usize,
    pc: u32,
    host: &mut H,
    lookup: Lookup<'_>,
    ntab: *const usize,
    state: Option<&mut yavs_interp::TierState>,
) -> Result<Option<u32>, Box<dyn Any + Send>> {
    // Only an activation with a record can leave the pc of its wait there.
    if state.is_none() {
        mem.fr.flags &= !yavs_interp::TIER_ROOT_WAIT;
    }
    let lean = LEAN && state.is_some();
    mem.wait_at = yavs_interp::NO_PC;
    mem.temp_seen = mem.fr.temp_len;
    if !lean {
        mem.ndeopt = 0;
        // The objects this machine found before, on this thread, at this heap epoch.
        mem.begin(state.as_deref());
    }
    // A frame index beyond the address format is the interpreter's error.
    if mem.fr.nregs != nregs as u64 || mem.fr.frame >= 1 << 29 {
        return Ok(None);
    }
    if lean {
        mem.begin_lean();
    }
    let mut env = Env {
        host: host.as_host(),
        lookup,
        stack_lo: 0,
        panic: None,
        pending: Vec::new(),
        windows: None,
    };
    // A lean function calls nothing: no native callee, no deoptimisation, no compiled callee.
    if !lean {
        // `env` is a local whose address the call takes anyway: where the native stack is.
        env.stack_lo = (&raw const env as usize).saturating_sub(NEST_STACK);
        mem.ntab = ntab;
        mem.stack_lo = env.stack_lo as u64;
        // The frames of native calls are at most `max_depth` deep: their slots are registered here.
        let depth = mem.fr.max_depth.min(1 << 20) as usize;
        mem.set_nslots(depth);
    }
    // SAFETY: `f` was generated for this function with `nregs` registers and only
    // indexes `regs` below that (checked at compile time); it accesses memory only
    // within the bounds in `mem`; `env` and `mem` outlive the call.
    let next = unsafe {
        f(
            &mut env as *mut Env<'_> as *mut c_void,
            mem.fr.regs,
            mem,
            pc,
        )
    };
    mem.fr.kctx = std::ptr::null_mut();
    mem.sync(env.host);
    mem.forget_end();
    if let Some(slot) = state {
        if lean {
            mem.end_lean();
            slot.at = mem.wait_at;
        } else {
            mem.save_tag(slot);
        }
    }
    match env.panic {
        Some(p) => Err(p),
        None => Ok(Some(next)),
    }
}

/// `Tier::call` of the function whose `CallFn` is at `adapter` (0: none).
pub fn call(
    adapter: usize,
    cells: &[u64],
    obj: Option<u64>,
    pc: u32,
    host: &mut dyn TierHost,
    lookup: Lookup<'_>,
    ntab: *const usize,
) -> Option<Called> {
    if adapter == 0 {
        return None;
    }
    let p = MEM.replace(std::ptr::null_mut());
    let mut mem = if p.is_null() {
        new_mem()
    } else {
        // SAFETY: as in `enter`.
        unsafe { Box::from_raw(p) }
    };
    let r = call_on(&mut mem, adapter, cells, obj, pc, host, lookup, ntab);
    if MEM.get().is_null() {
        MEM.set(Box::into_raw(mem));
    }
    match r {
        Ok(c) => c,
        Err(p) => resume_unwind(p),
    }
}

/// `call` on `mem`: as a frame-mode caller's `Gen::native_call` does, from the interpreter's
/// top frame (the view), which waits at its call at `pc`.
#[allow(clippy::too_many_arguments)]
pub fn call_on(
    mem: &mut Mem,
    adapter: usize,
    cells: &[u64],
    obj: Option<u64>,
    pc: u32,
    host: &mut dyn TierHost,
    lookup: Lookup<'_>,
    ntab: *const usize,
) -> Result<Option<Called>, Box<dyn Any + Send>> {
    if !host.view_again(&mut mem.fr) {
        host.view(&mut mem.fr);
    }
    mem.fr.flags &= !yavs_interp::TIER_ROOT_WAIT;
    let depth = mem.fr.frame + 1;
    // The depth limit and a frame index beyond the address format are the interpreter's
    // errors; a protected call outside the shared state is too.
    if depth >= mem.fr.max_depth
        || depth >= 1 << 29
        || obj.is_some() && mem.fr.flags & yavs_interp::TIER_SERIAL == 0
    {
        return Ok(None);
    }
    mem.temp_seen = mem.fr.temp_len;
    mem.ndeopt = 0;
    // The calling machine's record: the objects its compiled code found stay found.
    let mut rec = host.tier_state();
    mem.begin(Some(&rec));
    // The callee of a protected call allocates in the shared heap when the object is there.
    if obj.is_some_and(|h| (h >> 56) as u8 == yavs_interp::SHARED_TAG) {
        mem.fr.flags |= yavs_interp::TIER_SHARED_ALLOC;
    }
    let mut env = Env {
        host,
        lookup,
        stack_lo: stack_here().saturating_sub(NEST_STACK),
        panic: None,
        pending: Vec::new(),
        windows: None,
    };
    mem.ntab = ntab;
    mem.stack_lo = env.stack_lo as u64;
    mem.set_nslots(mem.fr.max_depth.min(1 << 20) as usize);
    // SAFETY: `adapter` is a `CallFn` generated for this function (`Gen::adapter`), which reads
    // one cell per parameter (two for a signal): `cells` holds them, checked by the caller.
    let v = unsafe {
        let f = std::mem::transmute::<usize, CallFn>(adapter);
        f(
            &mut env as *mut Env<'_> as *mut c_void,
            mem,
            depth,
            cells.as_ptr(),
        )
    };
    mem.sync(env.host);
    let out = if mem.ndeopt != 0 {
        // Frames left to the interpreter: on top of the caller, as `yavs_rt_deopt_end` does.
        let mut frames = std::mem::take(&mut env.pending);
        frames.reverse();
        mem.ndeopt = 0;
        match catch_unwind(AssertUnwindSafe(|| {
            env.host.materialise(&mut mem.fr, pc, &frames)
        })) {
            Ok(Stepped::Next) => Called::Pushed,
            Ok(Stepped::Suspend) => Called::Suspend,
            Ok(_) => Called::Fault,
            Err(p) => {
                env.panic.get_or_insert(p);
                Called::Fault
            }
        }
    } else {
        Called::Done(v as u64)
    };
    mem.fr.kctx = std::ptr::null_mut();
    mem.forget_end();
    // (`at` is the activation's, not the call's.)
    let at = rec.at;
    mem.save_tag(&mut rec);
    rec.at = at;
    env.host.set_tier_state(rec);
    match env.panic {
        Some(p) => Err(p),
        None => Ok(Some(out)),
    }
}

/// A register operand outside the function's register file, if any.
pub fn bad_reg(f: &Func) -> Option<Reg> {
    let n = f.reg_ty.len() as u32;
    f.code
        .iter()
        .flat_map(|i| inline_regs(f, i))
        .find(|r| r.0 >= n)
}

/// The registers an inline instruction touches (the compiled code indexes `regs` with them);
/// `Reg(u32::MAX)` for an operand list outside the function.
pub fn inline_regs(f: &Func, i: &Inst) -> Vec<Reg> {
    let pair = |r: Reg, w: Width| {
        if w == Width::Sig {
            vec![r, Reg(r.0.saturating_add(1))]
        } else {
            vec![r]
        }
    };
    match *i {
        Inst::AddrOf { dst, .. } => vec![dst],
        Inst::Alloc { dst, size } => vec![dst, size],
        Inst::LayoutOf { dst, desc, .. } => vec![dst, desc],
        Inst::Load { dst, addr, w } => [vec![addr], pair(dst, w)].concat(),
        Inst::Store { addr, src, w } => [vec![addr], pair(src, w)].concat(),
        Inst::Copy { dst, src, bytes } => vec![dst, src, bytes],
        Inst::Zero { addr, .. } => vec![addr],
        Inst::Pos { dst, dope, idx, .. } => vec![dst, dope, idx],
        Inst::SlicePos {
            dst,
            dope,
            left,
            right,
            dir,
            ..
        } => vec![dst, dope, left, right, dir],
        Inst::Field {
            dst, base, layout, ..
        } => vec![dst, base, layout],
        Inst::DopeLen { dst, dope, .. } => vec![dst, dope],
        Inst::Deref { dst, handle, .. } => vec![dst, handle],
        Inst::MemEq { dst, a, b, n } => vec![dst, a, b, n],
        Inst::LGet { dst, base, idx } => vec![dst, base, idx],
        Inst::LCopy {
            dst,
            dst_off,
            src,
            src_off,
            n,
        } => vec![dst, dst_off, src, src_off, n],
        Inst::LSet { base, idx, v } => vec![base, idx, v],
        Inst::LEq { dst, a, b, n } => vec![dst, a, b, n],
        Inst::LFill { base, idx, n, v } => vec![base, idx, n, v],
        Inst::SigVal {
            dst, offset, len, ..
        } => vec![dst, Reg(dst.0.saturating_add(1)), offset, len],
        Inst::SubtypeCheck { dope, want, .. } => vec![dope, want],
        Inst::MakeDope { at, parts } => match f.regs(parts) {
            Some(rs) => [&[at], rs].concat(),
            None => vec![Reg(u32::MAX)],
        },
        Inst::Const { dst, .. } | Inst::ConstF { dst, .. } => vec![dst],
        Inst::Move { dst, src } | Inst::IToF { dst, src } => vec![dst, src],
        Inst::Select { dst, c, a, b } => vec![dst, c, a, b],
        Inst::CoverIn { dst, acc, src, .. } => vec![dst, acc, src],
        Inst::Bin { dst, a, b, .. }
        | Inst::FBin { dst, a, b, .. }
        | Inst::Cmp { dst, a, b, .. }
        | Inst::FCmp { dst, a, b, .. }
        | Inst::Bool { dst, a, b, .. } => vec![dst, a, b],
        Inst::Un { dst, a, .. } | Inst::FUn { dst, a, .. } => vec![dst, a],
        Inst::Check { v, lo, hi, .. } | Inst::FCheck { v, lo, hi, .. } => vec![v, lo, hi],
        Inst::BinRange {
            dst, a, b, lo, hi, ..
        } => vec![dst, a, b, lo, hi],
        Inst::Index { dst, base, pos, .. } => vec![dst, base, pos],
        Inst::CondBr { c, .. } => vec![c],
        Inst::Case { v, .. } => vec![v],
        _ => vec![],
    }
}

/// How compiled code runs a native call it does not step.
pub enum NativeKind {
    /// A function of `std_ulogic` codes (`yavs_interp::code_table`): the results by code, the
    /// operand registers and the result register.
    Table(Vec<u8>, Vec<Reg>, Reg),
    /// `rising_edge` (true) or `falling_edge` through `TierHost::signal`.
    Edge(bool),
}

pub fn native_kind(
    prog: &Program,
    f: &Func,
    native: yavs_ir::NativeId,
    args: yavs_ir::Regs,
    dst: Option<Reg>,
) -> Option<NativeKind> {
    let sym = &prog.natives.get(native)?.symbol;
    let (rs, d) = (f.regs(args)?, dst?);
    let nregs = f.reg_ty.len() as u32;
    if let Some(t) = yavs_interp::code_table(sym) {
        // One operand register per `std_ulogic` (no signal operand), all in the frame.
        let fits = t.len() == 9usize.pow(rs.len() as u32)
            && rs.iter().chain([&d]).all(|r| r.0 < nregs)
            && rs
                .iter()
                .all(|r| f.reg_ty.get(r.0 as usize) != Some(&yavs_ir::RegTy::Sig));
        return fits.then(|| NativeKind::Table(t, rs.to_vec(), d));
    }
    yavs_interp::edge_native(sym).map(NativeKind::Edge)
}

/// A `CallNative` of a frame-mode function that runs through `TierHost::native_frame`: a native
/// `yavs_interp::direct_native` allows, its operands in the frame.
pub fn frame_direct(prog: &Program, f: &Func, i: &Inst) -> bool {
    let Inst::CallNative { native, args, dst } = *i else {
        return false;
    };
    let n = f.reg_ty.len() as u32;
    prog.natives
        .get(native)
        .is_some_and(|n| yavs_interp::direct_native(&n.symbol))
        && f.regs(args)
            .is_some_and(|rs| rs.iter().all(|r| r.0 < n) && rs.len() <= 8)
        && dst.is_none_or(|d| d.0 < n)
}

/// The address `AddrOf` yields in `space`, without the frame index for `Frame`; `None` where
/// the interpreter computes it (static links, an index beyond the address format).
pub fn addr_const(space: AddrSpace, off: u32) -> Option<u64> {
    let (sp, idx) = match space {
        AddrSpace::Frame => (space::FRAME, 0),
        AddrSpace::Up(_) => return None,
        AddrSpace::Inst => (space::INST, 0),
        AddrSpace::Pkg(p) => (space::PKG, p),
        AddrSpace::Const(c) => (space::CONST, c.0),
        AddrSpace::Shared => (space::SHARED, 0),
    };
    (idx < 1 << 29).then(|| u64::from(sp) << 61 | u64::from(idx) << 32 | u64::from(off))
}

/// Instructions the interpreter executes without freeing or resizing a memory object or
/// pushing a frame: the objects compiled code found stay valid across their step.
pub fn keeps_objects(i: &Inst) -> bool {
    matches!(
        i,
        Inst::Const { .. }
            | Inst::ConstF { .. }
            | Inst::Move { .. }
            | Inst::Select { .. }
            | Inst::AddrOf { .. }
            | Inst::CopyConst { .. }
            | Inst::Alloc { .. }
            | Inst::Load { .. }
            | Inst::Store { .. }
            | Inst::Copy { .. }
            | Inst::Fill { .. }
            | Inst::Zero { .. }
            | Inst::Pos { .. }
            | Inst::SlicePos { .. }
            | Inst::Index { .. }
            | Inst::Field { .. }
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
            | Inst::LogicOp { .. }
            | Inst::LNum { .. }
            | Inst::LEq { .. }
            | Inst::MemEq { .. }
            | Inst::LGet { .. }
            | Inst::LSet { .. }
            | Inst::LCopy { .. }
            | Inst::Cat { .. }
            | Inst::Check { .. }
            | Inst::FCheck { .. }
            | Inst::BinRange { .. }
            | Inst::MakeDope { .. }
            | Inst::DopeLen { .. }
            | Inst::LayoutOf { .. }
            | Inst::SubtypeCheck { .. }
            | Inst::Deref { .. }
            | Inst::SigVal { .. }
            | Inst::SigParts { .. }
            | Inst::SigSlice { .. }
            | Inst::SigRead { .. }
            | Inst::SigReadScalar { .. }
            | Inst::SigLastValue { .. }
            | Inst::SigAttr { .. }
            | Inst::Edge { .. }
            | Inst::Driving { .. }
            | Inst::DrivingValue { .. }
            | Inst::SigGather { .. }
            | Inst::SigScatter { .. }
            | Inst::Drive { .. }
            | Inst::DriveSlot { .. }
            | Inst::WaitOn { .. }
            | Inst::WaitEdge { .. }
            | Inst::WaitFor { .. }
            | Inst::WaitAt { .. }
            | Inst::TimedOut { .. }
            | Inst::Now { .. }
            | Inst::Delta { .. }
            | Inst::Cover { .. }
            | Inst::CoverIn { .. }
            | Inst::CoverCond { .. }
            | Inst::Stmt
    )
}

/// Instructions after which the interpreter may enter the compiled code: a call it finished
/// (`TIER_UNWIND`), a suspension, the return of a native that handed its call to a VHDL body
/// (`TierHost::step`). Not a native the compiled code runs itself (`native_kind`): it leaves
/// there only with an error, and an entry would store and reload every register live across it.
pub fn resumes(prog: &Program, f: &Func, i: &Inst) -> bool {
    match *i {
        Inst::CallNative { native, args, dst } => native_kind(prog, f, native, args, dst).is_none(),
        _ => matches!(
            i,
            Inst::Call { .. }
                | Inst::CallIndirect { .. }
                | Inst::CallProt { .. }
                | Inst::Report { .. }
                | Inst::Suspend { .. }
        ),
    }
}

/// The scalar logic assignments of the process `f` (`yavs_interp::scalar_drives`) and the pcs of
/// those `Gen::drive_bit` keeps for one host call (`Batch`): no delay, a driver index the entry has
/// room for, and another such assignment (or itself, in a loop) can follow it within the
/// activation. A process that drives once per activation gains nothing and keeps the direct call.
pub fn batched_drives(
    prog: &Program,
    f: &Func,
    fixed: &[Option<i64>],
) -> (
    Vec<yavs_interp::ScalarDrive>,
    std::collections::HashSet<u32>,
) {
    let all = yavs_interp::scalar_drives(prog, f);
    let cand: std::collections::HashSet<u32> = all
        .iter()
        .filter(|d| fixed.get(d.time.0 as usize) == Some(&Some(0)) && d.driver <= 0xffff)
        .map(|d| d.drive)
        .collect();
    let n = f.code.len();
    // ponytail: a search per candidate, quadratic; above the cap every candidate stays.
    if cand.len() * n > 4_000_000 {
        return (all, cand);
    }
    let kept = cand
        .iter()
        .copied()
        .filter(|&d| {
            let mut seen = vec![false; n];
            let mut work = vec![d as usize];
            let mut first = true;
            while let Some(pc) = work.pop() {
                if !std::mem::replace(&mut first, false) && cand.contains(&(pc as u32)) {
                    return true;
                }
                if pc >= n || std::mem::replace(&mut seen[pc], true) {
                    continue;
                }
                let i = &f.code[pc];
                if matches!(i, Inst::Suspend { .. }) {
                    continue;
                }
                match crate::regs::succs(prog, f, pc, i) {
                    Some(s) => work.extend(s.into_iter().map(|t| t as usize)),
                    None => return true,
                }
            }
            false
        })
        .collect();
    (all, kept)
}

/// Instructions that neither drive a signal, look at a driver, wait nor run other code, on any
/// path (a slow path steps the same instruction): scalar drives kept in `Batch` stay kept
/// across them. Everything else hands them over first.
pub fn batch_pure(prog: &Program, f: &Func, i: &Inst) -> bool {
    match *i {
        Inst::CallNative { native, args, dst } => native_kind(prog, f, native, args, dst).is_some(),
        _ => matches!(
            i,
            Inst::Const { .. }
                | Inst::ConstF { .. }
                | Inst::Move { .. }
                | Inst::Select { .. }
                | Inst::Bin { .. }
                | Inst::BinRange { .. }
                | Inst::Un { .. }
                | Inst::FBin { .. }
                | Inst::FUn { .. }
                | Inst::IToF { .. }
                | Inst::Cmp { .. }
                | Inst::FCmp { .. }
                | Inst::Bool { .. }
                | Inst::Check { .. }
                | Inst::FCheck { .. }
                | Inst::Index { .. }
                | Inst::Pos { .. }
                | Inst::AddrOf { .. }
                | Inst::Load { .. }
                | Inst::Store { .. }
                | Inst::Zero { .. }
                | Inst::Alloc { .. }
                | Inst::Br { .. }
                | Inst::CondBr { .. }
                | Inst::Case { .. }
                | Inst::Stmt
                | Inst::LGet { .. }
                | Inst::LSet { .. }
                | Inst::SigVal { .. }
                | Inst::SigReadScalar { .. }
                | Inst::SigRead { .. }
                | Inst::Edge { .. }
        ),
    }
}

/// A `LayoutOf` and the instructions after it that read one word of its layout (`fuse_layout`).
#[derive(Clone, Copy)]
pub struct LayoutFuse {
    /// The pc of the last instruction fused.
    pub last: usize,
    /// The word of the layout.
    pub k: u32,
    /// The base address of a `Field`; none: the word itself is the result.
    pub base: Option<Reg>,
    /// The register that gets the result.
    pub out: Reg,
}

/// Per pc: a `LayoutOf` that would be stepped and whose layout only one word is read of, by the
/// instructions right after it (no entry between, `enters`): one call of the interpreter's layout
/// memo for that word (`yavs_rt_layout_word`), as the interpreter's own `FieldOf` and
/// `LayoutWord` ops do. `cached`: the generator makes a layout the frame keeps (`layout_cache`)
/// inline, so those are left to it.
pub fn fuse_layout(
    prog: &Program,
    f: &Func,
    enters: &[bool],
    fixed: &[Option<i64>],
    cached: bool,
) -> Vec<Option<LayoutFuse>> {
    let n = f.code.len();
    let mut uses = vec![0u32; f.reg_ty.len()];
    let mut count = |r: Reg| {
        if let Some(u) = uses.get_mut(r.0 as usize) {
            *u = u.saturating_add(1);
        }
    };
    for inst in &f.code {
        inst.visit(&mut |r, role, _| {
            if role == yavs_ir::Role::Use {
                count(r);
            }
        });
    }
    for &r in &f.operands {
        count(r);
    }
    let once = |r: Reg| uses.get(r.0 as usize) == Some(&1);
    let inside = |p: usize| p < n && !enters[p];
    (0..n)
        .map(|pc| {
            let Inst::LayoutOf { dst, ty, cache, .. } = f.code[pc] else {
                return None;
            };
            if !once(dst)
                || static_layout(prog, ty).is_some()
                || cached && cache.is_some() && layout_shape(prog, ty).is_some()
                || !yavs_ir::Ty::desc_size(&prog.types, ty).is_some_and(|d| d > 0 && d <= 4096)
                || !inside(pc + 1)
            {
                return None;
            }
            let word = |p: usize, k: u32, base, out| {
                Some(LayoutFuse {
                    last: p,
                    k,
                    base,
                    out,
                })
            };
            match (f.code[pc + 1], f.code.get(pc + 2)) {
                (
                    Inst::Field {
                        dst: d,
                        base,
                        layout,
                        n: k,
                    },
                    _,
                ) if layout == dst && base != dst => word(pc + 1, 2 + k, Some(base), d),
                (
                    Inst::Load {
                        dst: d,
                        addr,
                        w: Width::B8,
                    },
                    _,
                ) if addr == dst => word(pc + 1, 0, None, d),
                // A word at a constant offset: `Index` of the layout, then a load.
                (
                    Inst::Index {
                        dst: x,
                        base,
                        pos,
                        stride,
                    },
                    Some(&Inst::Load {
                        dst: d,
                        addr,
                        w: Width::B8,
                    }),
                ) if base == dst && addr == x && once(x) && inside(pc + 2) => {
                    let off = fixed.get(pos.0 as usize).copied().flatten()?;
                    let off = off.checked_mul(i64::from(stride))?;
                    (off >= 0 && off % 8 == 0)
                        .then(|| u32::try_from(off / 8).ok())
                        .flatten()
                        .and_then(|k| word(pc + 2, k, None, d))
                }
                _ => None,
            }
        })
        .collect()
}

/// The descriptor bytes and layout words of a type whose layout a frame slot keeps
/// (`yavs_ir::opt` `layout_cache`).
pub fn layout_shape(prog: &Program, ty: yavs_ir::TyId) -> Option<(u32, u32)> {
    use yavs_ir::Ty;
    let words = match prog.types.get(ty)? {
        Ty::Record { fields } => 2 + fields.len() as u32,
        Ty::Array { .. } => 4,
        _ => return None,
    };
    let dsz = u32::try_from(Ty::desc_size(&prog.types, ty)?).ok()?;
    (dsz > 0).then_some((dsz, words))
}

/// A `LayoutOf` result the generator can build: fixed words, or an array whose element
/// layout is fixed, so only the descriptor's lengths are read at run time.
pub enum StaticLayout {
    Words(Vec<u64>),
    Array {
        dims: u8,
        elem: yavs_ir::Layout,
        logic: bool,
    },
}

/// The `LayoutOf` of `ty` as far as it does not depend on the descriptor, as `Ty::layout`
/// computes it; `None` where the interpreter computes it.
pub fn static_layout(prog: &Program, ty: yavs_ir::TyId) -> Option<StaticLayout> {
    use yavs_ir::Ty;
    let (types, consts) = (&prog.types, &prog.consts);
    let words = |l: yavs_ir::Layout| {
        let mut w = vec![l.size, l.align];
        if l.offsets.is_empty() {
            w.extend([l.stride, l.count]);
        } else {
            w.extend(&l.offsets);
        }
        StaticLayout::Words(w)
    };
    if Ty::desc_size(types, ty)? == 0 {
        return Ty::layout(types, consts, ty, &[])
            .ok()
            .map(|(l, _)| words(l));
    }
    let Some(Ty::Array {
        elem,
        elem_desc,
        dims,
    }) = types.get(ty)
    else {
        return None;
    };
    let el = match elem_desc {
        Some(c) => match consts.get(*c)? {
            yavs_ir::ConstData::Bytes(b) => Ty::layout(types, consts, *elem, b).ok()?.0,
            _ => return None,
        },
        None if Ty::desc_size(types, *elem)? == 0 => Ty::layout(types, consts, *elem, &[]).ok()?.0,
        None => return None,
    };
    // A scalar element has its element size (`Ty::elem_size`), as in `Ty::layout`.
    let el = match types.get(*elem).and_then(Ty::elem_size) {
        Some(n) => yavs_ir::Layout {
            size: n,
            align: n,
            ..el
        },
        None => el,
    };
    Some(StaticLayout::Array {
        dims: *dims,
        elem: el,
        logic: Ty::logic_leaf(types, ty).is_some(),
    })
}

/// What the generator knows about a register's value within a basic block.
#[derive(Clone, Copy)]
pub enum Known<V = ()> {
    Int(i64),
    /// The address of byte `.0` of the frame's slots.
    Frame(u64),
    /// The address of byte `.1` of constant `.0`.
    Const(yavs_ir::ConstId, u64),
    /// The address of byte `.0` of the instance data.
    Inst(u64),
    /// An address in the instance data at an offset the generator does not know (an `Index` by a
    /// run-time position, or a fact carried from another block, `regs::spaces`).
    InstAny,
    /// An address in the slots of the frame `.0` static links up (1 or 2), at an offset the
    /// generator does not track: frame-mode code reads them from the view (`TierFrame::up_slots`).
    Up(u8),
    /// An address in the temporary area (`Alloc`, or an index into one).
    Temp,
    /// A guess, not a fact: probably an address in the temporary area (the fat pair a call
    /// returned, the words loaded from it). Frame-mode code tests it at run time and asks the
    /// runtime when wrong (`Gen::host`).
    LikelyTemp,
    /// A guess: probably an address in a heap object (a pointer loaded from the slot of a
    /// variable). Only frame-mode code uses it, as `LikelyTemp`.
    LikelyHeap,
    /// An address in a heap object (what a `Deref` makes, or an index into it): not in a frame, the
    /// temporary area, the instance data or a constant. In a native frame, the object itself while
    /// nothing that could free or move it ran (`ObjRef`).
    Heap(Option<ObjRef<V>>),
    /// An address a native frame got as a parameter (or an `Index` of one), in the region
    /// `Cur::pres[.0]` pinned at the function's entry (`param_addrs`, `yavs_rt_region`).
    Param(u16),
}

/// An object a `Deref` found in a native frame, as the generated code holds it: valid up to the
/// next instruction outside `keeps_objects` (the facts are dropped there, `Gen::func`), always
/// writable (a `Deref` hit requires it). Its bytes are `ptr[..len]`; a register that points into
/// it has the address `hi | off`.
#[derive(Clone, Copy)]
pub struct ObjRef<V> {
    pub ptr: V,
    pub len: V,
    /// The high half of the addresses (low 32 bits zero).
    pub hi: V,
    /// The low 32 bits of this register's address: the offset in the object.
    pub off: V,
}

/// The registers `f` defines exactly once, by a `Const` or `ConstF` (its bits), and their values.
pub fn fixed_consts(f: &Func) -> Vec<Option<i64>> {
    let n = f.reg_ty.len();
    let mut defs = vec![0u32; n + 1];
    let mut val = vec![None; n + 1];
    for i in &f.code {
        i.visit(&mut |r, role, ty| {
            if role == yavs_ir::Role::Def {
                let w = if ty == Some(yavs_ir::RegTy::Sig) {
                    2
                } else {
                    1
                };
                for d in &mut defs[r.0 as usize..(r.0 as usize + w).min(n + 1)] {
                    *d += 1;
                }
            }
        });
        if let Inst::Const { dst, .. } | Inst::ConstF { dst, .. } = *i
            && let Some(x) = val.get_mut(dst.0 as usize)
        {
            *x = Some(match *i {
                Inst::ConstF { v, .. } => v.to_bits() as i64,
                Inst::Const { v, .. } => v,
                _ => unreachable!(),
            });
        }
    }
    (0..n)
        .map(|r| if defs[r] == 1 { val[r] } else { None })
        .collect()
}

/// Multiplications by a small constant whose product only feeds the `Index` right after them
/// (the element offset `pos * size` of an array access): the operand and the constant factor.
/// Compiled code tests the operand once for both the overflow of the product and the address
/// bound, instead of an overflow test of its own and the `Index` guards.
pub fn index_muls(f: &Func, fixed: &[Option<i64>]) -> Vec<Option<(Reg, u64)>> {
    let n = f.reg_ty.len();
    let (mut defs, mut uses) = (vec![0u32; n + 2], vec![0u32; n + 2]);
    for i in &f.code {
        let (u, d) = yavs_interp::inst_regs(f, i);
        for r in u {
            if let Some(x) = uses.get_mut(r.0 as usize) {
                *x += 1;
            }
        }
        for r in d {
            if let Some(x) = defs.get_mut(r.0 as usize) {
                *x += 1;
            }
        }
    }
    let k = |r: Reg| fixed.get(r.0 as usize).copied().flatten();
    (0..f.code.len())
        .map(|pc| {
            let Inst::Bin {
                op: IntOp::Mul,
                dst,
                a,
                b,
            } = f.code[pc]
            else {
                return None;
            };
            let Some(&Inst::Index { pos, stride, .. }) = f.code.get(pc + 1) else {
                return None;
            };
            let once =
                |r: Reg| defs.get(r.0 as usize) == Some(&1) && uses.get(r.0 as usize) == Some(&1);
            if pos != dst || stride == 0 || !once(dst) {
                return None;
            }
            let (x, m) = match (k(a), k(b)) {
                (None, Some(m)) => (a, m),
                (Some(m), None) => (b, m),
                _ => return None,
            };
            (1..=1 << 16).contains(&m).then_some((x, m as u64))
        })
        .collect()
}

/// A preloaded register: its value, or the address space and offset of its address.
pub type Preload = (Reg, Option<(AddrSpace, u32)>, i64);

/// What the interpreter preloads into a frame's registers (`Pre`): the registers `f` defines
/// exactly once, by a `Const`, a `ConstF` or an `AddrOf` (not of a static link), with the value
/// or the address space and offset. A native frame stores them at entry, since the definition
/// may not have run on the way to a use.
pub fn preloads(f: &Func) -> Vec<Preload> {
    let n = f.reg_ty.len();
    let mut defs = vec![0u32; n + 1];
    for i in &f.code {
        i.visit(&mut |r, role, ty| {
            if role == yavs_ir::Role::Def {
                let w = if ty == Some(yavs_ir::RegTy::Sig) {
                    2
                } else {
                    1
                };
                for d in &mut defs[r.0 as usize..(r.0 as usize + w).min(n + 1)] {
                    *d += 1;
                }
            }
        });
    }
    f.code
        .iter()
        .filter_map(|i| match *i {
            Inst::Const { dst, v } => Some((dst, None, v)),
            Inst::ConstF { dst, v } => Some((dst, None, v.to_bits() as i64)),
            Inst::AddrOf { dst, space, off } if !matches!(space, AddrSpace::Up(_)) => {
                Some((dst, Some((space, off)), 0))
            }
            _ => None,
        })
        .filter(|(r, ..)| defs.get(r.0 as usize) == Some(&1))
        .collect()
}

/// A parameter of a native-frame function that holds an address the function uses in a loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParamAddr {
    /// The register the entry block loads the parameter into (defined once, never used before).
    pub reg: Reg,
    /// The parameter's cell in the native function's argument list.
    pub cell: usize,
    /// Written through: the region is resolved for writing.
    pub write: bool,
}

/// The instructions of a function none of which can move the interpreter's slot stack, the
/// temporary area or a heap object while it runs (nothing allocates, calls, or ends a statement),
/// so the host pointers of the regions its parameters point into stay good for the whole
/// activation (`yavs_rt_region`).
pub fn stable_activation(f: &Func) -> bool {
    f.code.iter().all(steady)
}

/// The instructions of a function none of which can free or move a heap object or the
/// interpreter's slot stack while it runs: those of `stable_activation` and the ones that only
/// allocate in or end statements of the temporary area (which may move, `yavs_rt_region` flag 4).
pub fn fixed_activation(f: &Func) -> bool {
    f.code.iter().all(|i| {
        steady(i)
            || matches!(
                i,
                Inst::Alloc { .. }
                    | Inst::NewFrame { .. }
                    | Inst::Stmt
                    | Inst::TempRoot { .. }
                    | Inst::MakeDope { .. }
                    | Inst::LayoutOf { .. }
            )
    })
}

fn steady(i: &Inst) -> bool {
    matches!(
        i,
        Inst::Const { .. }
            | Inst::ConstF { .. }
            | Inst::Move { .. }
            | Inst::Select { .. }
            | Inst::AddrOf { .. }
            | Inst::Load { .. }
            | Inst::Store { .. }
            | Inst::Copy { .. }
            | Inst::Fill { .. }
            | Inst::Zero { .. }
            | Inst::Pos { .. }
            | Inst::SlicePos { .. }
            | Inst::Index { .. }
            | Inst::Field { .. }
            | Inst::Bin { .. }
            | Inst::Un { .. }
            | Inst::FBin { .. }
            | Inst::FUn { .. }
            | Inst::IToF { .. }
            | Inst::FToI { .. }
            | Inst::Cmp { .. }
            | Inst::FCmp { .. }
            | Inst::Bool { .. }
            | Inst::LogicOp { .. }
            | Inst::LEq { .. }
            | Inst::MemEq { .. }
            | Inst::LGet { .. }
            | Inst::LSet { .. }
            | Inst::LFill { .. }
            | Inst::LCopy { .. }
            | Inst::Check { .. }
            | Inst::FCheck { .. }
            | Inst::BinRange { .. }
            | Inst::DopeLen { .. }
            | Inst::SubtypeCheck { .. }
            | Inst::Br { .. }
            | Inst::CondBr { .. }
            | Inst::Case { .. }
            | Inst::Ret { .. }
            | Inst::Unreachable { .. }
    )
}

/// The address parameters of `f` worth resolving once at entry: a register loaded from a
/// parameter slot by the entry block's straight-line code, defined once, whose slot nothing
/// else touches (the parameter is the caller's argument for the whole call), and which the
/// function uses as an address (directly, or through the single-definition `Index` and `Move`
/// registers derived from it) inside a loop. A value that is no address, or one used rarely is
/// left alone: the resolution is a callback.
pub fn param_addrs(f: &Func) -> Vec<ParamAddr> {
    let n = f.reg_ty.len();
    let (mut defs, mut first_use) = (vec![0u32; n + 1], vec![usize::MAX; n + 1]);
    for (pc, i) in f.code.iter().enumerate() {
        let (uses, d) = yavs_interp::inst_regs(f, i);
        for r in uses {
            if let Some(u) = first_use.get_mut(r.0 as usize) {
                *u = (*u).min(pc);
            }
        }
        for r in d {
            if let Some(x) = defs.get_mut(r.0 as usize) {
                *x += 1;
            }
        }
    }
    let end0 = f
        .blocks
        .iter()
        .map(|(_, b)| b.start as usize)
        .filter(|&s| s > 0)
        .min()
        .unwrap_or(f.code.len());
    // The cells of the 8-byte parameters by slot.
    let mut cells = std::collections::HashMap::new();
    let mut cell = 0;
    for p in &f.params {
        if p.width == Width::B8 {
            cells.insert(u64::from(p.slot), cell);
        }
        cell += 1 + usize::from(p.width == Width::Sig);
    }
    // The registers that hold the address of a slot, and the slots whose address is used for
    // anything but a load of 8 bytes: those may change under the function.
    let mut at: Vec<Option<u64>> = vec![None; n + 1];
    for i in &f.code {
        if let Inst::AddrOf {
            dst,
            space: AddrSpace::Frame,
            off,
        } = *i
            && let Some(a) = at.get_mut(dst.0 as usize)
        {
            *a = Some(off.into());
        }
    }
    let mut touched = std::collections::HashSet::new();
    for i in &f.code {
        let load = match *i {
            Inst::Load {
                addr, w: Width::B8, ..
            } => Some(addr),
            _ => None,
        };
        for r in yavs_interp::inst_regs(f, i).0 {
            if let Some(Some(o)) = at.get(r.0 as usize)
                && load != Some(r)
            {
                touched.insert(*o);
            }
        }
    }
    let mut cands: Vec<(Reg, usize, usize)> = Vec::new();
    for (pc, i) in f.code.iter().enumerate().take(end0) {
        let Inst::Load {
            dst,
            addr,
            w: Width::B8,
        } = *i
        else {
            continue;
        };
        let Some(Some(o)) = at.get(addr.0 as usize) else {
            continue;
        };
        if let Some(&c) = cells.get(o)
            && !touched.contains(o)
            && defs.get(dst.0 as usize) == Some(&1)
            && first_use.get(dst.0 as usize).is_some_and(|&u| u > pc)
            && f.reg_ty.get(dst.0 as usize) != Some(&yavs_ir::RegTy::Sig)
            && !cands.iter().any(|x| x.1 == c)
        {
            cands.push((dst, c, pc));
        }
    }
    if cands.is_empty() {
        return Vec::new();
    }
    // Which candidate each register derives from (single definitions only).
    let mut origin: Vec<Option<usize>> = vec![None; n + 1];
    for (k, &(r, ..)) in cands.iter().enumerate() {
        origin[r.0 as usize] = Some(k);
    }
    for _ in 0..2 {
        for i in &f.code {
            let (dst, src) = match *i {
                Inst::Index { dst, base, .. } => (dst, base),
                Inst::Move { dst, src } => (dst, src),
                _ => continue,
            };
            if defs.get(dst.0 as usize) == Some(&1)
                && let Some(Some(k)) = origin.get(src.0 as usize).copied()
            {
                origin[dst.0 as usize] = Some(k);
            }
        }
    }
    // Spans of the backward branches.
    let start = |b: yavs_ir::BlockId| f.blocks.get(b).map(|b| b.start as usize);
    let mut loops: Vec<(usize, usize)> = Vec::new();
    for (pc, i) in f.code.iter().enumerate() {
        let targets = match *i {
            Inst::Br { to } => [Some(to), None],
            Inst::CondBr { t, f: e, .. } => [Some(t), Some(e)],
            _ => continue,
        };
        for t in targets.into_iter().flatten() {
            if let Some(s) = start(t)
                && s <= pc
            {
                loops.push((s, pc));
            }
        }
    }
    let (mut uses, mut in_loop, mut write) = (
        vec![0u32; cands.len()],
        vec![0u32; cands.len()],
        vec![false; cands.len()],
    );
    for (pc, i) in f.code.iter().enumerate() {
        let (reads, writes): (&[Reg], &[Reg]) = match i {
            Inst::Load { addr, .. } => (&[*addr], &[]),
            Inst::Store { addr, .. } => (&[], &[*addr]),
            Inst::Copy { dst, src, .. } => (&[*src], &[*dst]),
            Inst::Fill { dst, .. } | Inst::Zero { addr: dst, .. } => (&[], &[*dst]),
            Inst::Pos { dope, .. } | Inst::DopeLen { dope, .. } | Inst::SlicePos { dope, .. } => {
                (&[*dope], &[])
            }
            Inst::SubtypeCheck { dope, want, .. } => (&[*dope, *want], &[]),
            Inst::LGet { base, .. } => (&[*base], &[]),
            Inst::LSet { base, .. } | Inst::LFill { base, .. } => (&[], &[*base]),
            Inst::LCopy { dst, src, .. } => (&[*src], &[*dst]),
            Inst::LEq { a, b, .. } | Inst::MemEq { a, b, .. } => (&[*a, *b], &[]),
            _ => (&[], &[]),
        };
        let looped = loops.iter().any(|&(s, e)| s <= pc && pc <= e);
        for (rs, w) in [(reads, false), (writes, true)] {
            for r in rs {
                if let Some(Some(k)) = origin.get(r.0 as usize).copied() {
                    uses[k] += 1;
                    in_loop[k] += u32::from(looped);
                    write[k] |= w;
                }
            }
        }
    }
    cands
        .iter()
        .enumerate()
        .filter(|&(k, _)| in_loop[k] > 0 && uses[k] >= 3)
        .map(|(k, &(reg, cell, _))| ParamAddr {
            reg,
            cell,
            write: write[k],
        })
        .collect()
}

/// Update `known` for what `i` writes.
pub fn track<V: Copy>(known: &mut [Option<Known<V>>], fixed: &[Option<i64>], f: &Func, i: &Inst) {
    let get = |r: Reg| known.get(r.0 as usize).copied().flatten();
    // Before the definitions clear what they overwrite.
    let len_known = match *i {
        Inst::SigVal { len, .. } => match get(len) {
            Some(k @ Known::Int(_)) => Some(k),
            _ => fixed.get(len.0 as usize).copied().flatten().map(Known::Int),
        },
        _ => None,
    };
    let off_known = match *i {
        Inst::SigVal { offset, .. } => match get(offset) {
            Some(Known::Int(v)) => Some(Known::<V>::Int(v)),
            _ => fixed
                .get(offset.0 as usize)
                .copied()
                .flatten()
                .map(Known::Int),
        },
        _ => None,
    };
    let new = match *i {
        Inst::Const { dst, v } => Some((dst, Known::Int(v))),
        Inst::AddrOf {
            dst,
            space: AddrSpace::Frame,
            off,
        } => Some((dst, Known::Frame(off.into()))),
        Inst::AddrOf {
            dst,
            space: AddrSpace::Const(c),
            off,
        } => Some((dst, Known::Const(c, off.into()))),
        Inst::AddrOf {
            dst,
            space: AddrSpace::Inst,
            off,
        } => Some((dst, Known::Inst(off.into()))),
        Inst::AddrOf {
            dst,
            space: AddrSpace::Up(d @ 1..=2),
            ..
        } => Some((dst, Known::Up(d))),
        // A local composite's object (`NewFrame`) is made in the temporary area too.
        Inst::Alloc { dst, .. } | Inst::NewFrame { dst, .. } => Some((dst, Known::Temp)),
        // A result that is an array or a record is the address of a fat pair in the temporary
        // area, and the words of a layout are made there too.
        Inst::Call { dst: Some(dst), .. }
        | Inst::CallNative { dst: Some(dst), .. }
        | Inst::LayoutOf { dst, .. } => Some((dst, Known::LikelyTemp)),
        Inst::Load {
            dst,
            addr,
            w: yavs_ir::Width::B8,
        } => match get(addr) {
            Some(Known::LikelyTemp) => Some((dst, Known::LikelyTemp)),
            // A variable's object, not the address a caller passed: that is in any space.
            Some(Known::Frame(o)) if !f.params.iter().any(|p| u64::from(p.slot) == o) => {
                Some((dst, Known::LikelyHeap))
            }
            Some(Known::Up(_)) => Some((dst, Known::LikelyHeap)),
            _ => None,
        },
        Inst::Deref { dst, .. } => Some((dst, Known::Heap(None))),
        Inst::Move { dst, src } => get(src).map(|k| (dst, k)),
        // As the interpreter computes it when it is no error.
        Inst::Index {
            dst,
            base,
            pos,
            stride,
        } => {
            let at = |o: u64, p: i64| {
                (p as u64)
                    .checked_mul(stride.into())
                    .and_then(|d| o.checked_add(d))
                    .filter(|&a| a <= u64::from(u32::MAX))
            };
            match (get(base), get(pos)) {
                (Some(Known::Frame(o)), Some(Known::Int(p))) => {
                    at(o, p).map(|a| (dst, Known::Frame(a)))
                }
                (Some(Known::Const(c, o)), Some(Known::Int(p))) => {
                    at(o, p).map(|a| (dst, Known::Const(c, a)))
                }
                (Some(Known::Inst(o)), Some(Known::Int(p))) => {
                    at(o, p).map(|a| (dst, Known::Inst(a)))
                }
                // An offset past 4 GiB leaves the block through the instruction's own exit.
                (Some(Known::Inst(_) | Known::InstAny), _) => Some((dst, Known::InstAny)),
                (Some(Known::Up(d)), _) => Some((dst, Known::Up(d))),
                (Some(Known::Temp), _) => Some((dst, Known::Temp)),
                (Some(Known::LikelyTemp), _) => Some((dst, Known::LikelyTemp)),
                (Some(Known::LikelyHeap), _) => Some((dst, Known::LikelyHeap)),
                (Some(Known::Heap(_)), _) => Some((dst, Known::Heap(None))),
                (Some(Known::Param(k)), _) => Some((dst, Known::Param(k))),
                _ => None,
            }
        }
        _ => None,
    };
    i.visit(&mut |r, role, ty| {
        if role == yavs_ir::Role::Def {
            // (A signal reference takes two cells, any other value one.)
            let w = 1 + u32::from(ty == Some(yavs_ir::RegTy::Sig));
            for k in r.0..r.0.saturating_add(w) {
                if let Some(x) = known.get_mut(k as usize) {
                    *x = None;
                }
            }
        }
    });
    if let Some((r, k)) = new
        && let Some(x) = known.get_mut(r.0 as usize)
    {
        *x = Some(k);
    }
    // The length word of a signal reference is the `len` it was made with, the first word the
    // signal and the offset.
    if let Inst::SigVal { dst, sig, .. } = *i
        && let Some(Known::Int(l)) = len_known
        && let Some(x) = known.get_mut(dst.0 as usize + 1)
    {
        *x = Some(Known::Int(l));
        if let Some(Known::Int(o)) = off_known
            && let Some(0..=0xffff_ffff) = Some(o)
            && let Some(x) = known.get_mut(dst.0 as usize)
        {
            *x = Some(Known::Int(i64::from(sig) << 32 | o));
        }
    }
}

/// Below this many IR instructions a function without a loop stays interpreted (`too_short`).
pub const SHORT: usize = 64;

/// Why `f` stays interpreted although hot: shorter than `SHORT` IR instructions and without a
/// jump back, so each entry into compiled code from the interpreter (the frame view, the
/// entry switch, the way back) would cost about what interpreting its body costs: the small
/// helpers a testbench calls millions of times ran slower compiled (#432).
pub fn too_short(prog: &yavs_ir::Program, f: &yavs_ir::Func) -> Option<String> {
    (f.code.len() < SHORT && !loops(prog, f)).then(|| {
        format!(
            "{} IR instructions without a loop: entering compiled code would cost about as much",
            f.code.len()
        )
    })
}

/// The instructions of the loops of `f`, as ranges of the blocks of each natural loop: per back
/// edge of a depth-first walk from the entry, the blocks that reach its latch without passing its
/// header. A process's way back from its suspension is no loop (as in `loops`). A jump to a lower
/// pc is not enough: the optimiser appends blocks (preheaders, inlined bodies) after the code.
pub fn back_edges(prog: &yavs_ir::Program, f: &yavs_ir::Func) -> Vec<std::ops::Range<usize>> {
    use yavs_ir::Inst;
    let blocks: Vec<std::ops::Range<usize>> = f
        .blocks
        .iter()
        .map(|(_, b)| b.start as usize..(b.end as usize).min(f.code.len()))
        .collect();
    let nb = blocks.len();
    let waits: Vec<bool> = blocks
        .iter()
        .map(|r| {
            f.kind == FuncKind::Process
                && f.code[r.clone()]
                    .iter()
                    .any(|i| matches!(i, Inst::Suspend { .. }))
        })
        .collect();
    let succ = |b: usize| -> Vec<usize> {
        if waits[b] {
            return Vec::new();
        }
        let mut to = Vec::new();
        for i in &f.code[blocks[b].clone()] {
            match i {
                Inst::Br { to: t } => to.push(t.0 as usize),
                Inst::CondBr { t, f: e, .. } => to.extend([t.0 as usize, e.0 as usize]),
                Inst::Case { table, .. } => {
                    if let Some(t) = prog.cases.get(*table) {
                        to.push(t.default.0 as usize);
                        to.extend(t.arms.iter().map(|a| a.2.0 as usize));
                    }
                }
                _ => {}
            }
        }
        to.retain(|&t| t < nb);
        to
    };
    let mut preds = vec![Vec::new(); nb];
    for b in 0..nb {
        for t in succ(b) {
            preds[t].push(b);
        }
    }
    let mut edges = Vec::new();
    let mut state = vec![0u8; nb]; // 0 new, 1 on the path, 2 done
    if nb > 0 {
        let mut stack = vec![(0usize, succ(0), 0usize)];
        state[0] = 1;
        while let Some((b, next, k)) = stack.last_mut() {
            match next.get(*k).copied() {
                Some(t) => {
                    *k += 1;
                    match state[t] {
                        1 => edges.push((*b, t)),
                        0 => {
                            state[t] = 1;
                            stack.push((t, succ(t), 0));
                        }
                        _ => {}
                    }
                }
                None => {
                    state[*b] = 2;
                    stack.pop();
                }
            }
        }
    }
    let mut out = Vec::new();
    let mut seen = vec![usize::MAX; nb];
    for (k, (latch, h)) in edges.into_iter().enumerate() {
        seen[h] = k;
        let mut work = vec![latch];
        while let Some(b) = work.pop() {
            if std::mem::replace(&mut seen[b], k) == k && b != latch {
                continue;
            }
            out.push(blocks[b].clone());
            if b != h {
                work.extend(preds[b].iter().copied().filter(|&p| seen[p] != k));
            }
        }
        out.push(blocks[h].clone());
    }
    out
}

/// Whether the blocks of `f` form a cycle, not counting a process's way through its
/// suspension (each activation runs its body once).
pub fn loops(prog: &yavs_ir::Program, f: &yavs_ir::Func) -> bool {
    use yavs_ir::Inst;
    let blocks: Vec<&[Inst]> = f
        .blocks
        .iter()
        .map(|(_, b)| f.code.get(b.start as usize..b.end as usize).unwrap_or(&[]))
        .collect();
    let waits: Vec<bool> = blocks
        .iter()
        .map(|c| f.kind == FuncKind::Process && c.iter().any(|i| matches!(i, Inst::Suspend { .. })))
        .collect();
    let succ = |b: usize| -> Vec<usize> {
        let mut to = Vec::new();
        for i in blocks[b] {
            match i {
                Inst::Br { to: t } => to.push(t.0 as usize),
                Inst::CondBr { t, f: e, .. } => to.extend([t.0 as usize, e.0 as usize]),
                Inst::Case { table, .. } => {
                    if let Some(t) = prog.cases.get(*table) {
                        to.push(t.default.0 as usize);
                        to.extend(t.arms.iter().map(|a| a.2.0 as usize));
                    }
                }
                _ => {}
            }
        }
        to.retain(|&t| t < blocks.len() && !waits[b] && !waits[t]);
        to
    };
    // Blocks the optimiser left unreachable (a self-jump) are no loop of the process.
    let mut reach = vec![false; blocks.len()];
    let mut work = vec![0usize];
    while let Some(b) = work.pop() {
        if b >= blocks.len() || std::mem::replace(&mut reach[b], true) {
            continue;
        }
        for i in blocks[b] {
            match i {
                Inst::Br { to } => work.push(to.0 as usize),
                Inst::CondBr { t, f: e, .. } => work.extend([t.0 as usize, e.0 as usize]),
                Inst::Case { table, .. } => {
                    if let Some(t) = prog.cases.get(*table) {
                        work.push(t.default.0 as usize);
                        work.extend(t.arms.iter().map(|a| a.2.0 as usize));
                    }
                }
                _ => {}
            }
        }
    }
    // Depth-first search for an edge back to a block on the current path.
    let mut state = vec![0u8; blocks.len()]; // 0 new, 1 on the path, 2 done
    for root in 0..blocks.len() {
        if state[root] != 0 || !reach[root] {
            continue;
        }
        let mut stack = vec![(root, succ(root), 0)];
        state[root] = 1;
        while let Some((b, next, k)) = stack.last_mut() {
            match next.get(*k).copied() {
                Some(t) => {
                    *k += 1;
                    match state[t] {
                        1 => return true,
                        0 => {
                            state[t] = 1;
                            stack.push((t, succ(t), 0));
                        }
                        _ => {}
                    }
                }
                None => {
                    state[*b] = 2;
                    stack.pop();
                }
            }
        }
    }
    false
}

/// Instructions a native frame has the interpreter execute on a frame of its own
/// (`TierHost::native_step`): heap objects, vector fills and compares, layouts, reports.
pub fn xstep_ok(f: &Func, i: &Inst) -> bool {
    let n = f.reg_ty.len() as u32;
    matches!(
        i,
        Inst::New { .. }
            | Inst::NewFill { .. }
            | Inst::Free { .. }
            | Inst::LayoutOf { .. }
            | Inst::LFill { .. }
            | Inst::LEq { .. }
            | Inst::Report { .. }
            | Inst::FToI { .. }
            | Inst::FToIRange { .. }
            | Inst::Phys { .. }
            | Inst::LNum { .. }
            | Inst::LogicOp { .. }
            | Inst::SigSlice { .. }
            | Inst::Cat { logic: true, .. }
            | Inst::Bin { op: IntOp::Pow, .. }
            | Inst::BinRange { op: IntOp::Pow, .. }
            | Inst::FBin {
                op: FOp::Pow | FOp::PowI,
                ..
            }
    ) && {
        let (uses, defs) = yavs_interp::inst_regs(f, i);
        uses.iter().chain(&defs).all(|r| r.0 < n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yavs_interp::TierState;

    fn obj(sp: u8, index: u64) -> Obj {
        Obj {
            hi: u64::from(sp) << 29 | index,
            ptr: std::ptr::null_mut(),
            len: 8,
            w: 0,
            h: NO_H,
        }
    }

    fn entered(m: &Mem, sp: u8, index: u64) -> bool {
        m.get(u64::from(sp) << 29 | index).is_some()
    }

    fn rt(id: u64, n: u64) -> u64 {
        id << yavs_interp::OWNER_RT_SHIFT | n
    }

    /// An activation of machine `owner` at heap epoch `epoch` with the process record `rec`.
    fn activate(m: &mut Mem, owner: u64, epoch: u64, rec: &mut yavs_interp::TierState) {
        m.fr.owner = owner;
        m.fr.heap_epoch = epoch;
        m.begin(Some(rec));
    }

    fn finish(m: &mut Mem, rec: &mut yavs_interp::TierState) {
        m.forget_end();
        m.save_tag(rec);
    }

    /// Two instances of one process use the same addresses for their own objects: each finds its
    /// own after the other ran, with no walk and no restore (#679).
    #[test]
    fn instances_keep_their_own_objects() {
        let mut m = Mem::new(TierFrame::default());
        let (mut r1, mut r2) = (TierState::default(), TierState::default());
        let mine = |n: usize| Obj {
            ptr: std::ptr::null_mut::<u8>().wrapping_add(n),
            ..obj(space::HEAP, 5)
        };
        activate(&mut m, rt(1, 1), 0, &mut r1);
        m.put_obj(mine(1));
        m.put_obj(obj(space::INST, 3));
        finish(&mut m, &mut r1);
        activate(&mut m, rt(1, 2), 0, &mut r2);
        assert!(!entered(&m, space::HEAP, 5) && !entered(&m, space::INST, 3));
        m.put_obj(mine(2));
        finish(&mut m, &mut r2);
        activate(&mut m, rt(1, 1), 0, &mut r1);
        let hi = u64::from(space::HEAP) << 29 | 5;
        assert_eq!(m.get(hi).map(|o| o.ptr as usize), Some(1));
        assert!(entered(&m, space::INST, 3));
        finish(&mut m, &mut r1);
        activate(&mut m, rt(1, 2), 0, &mut r2);
        assert_eq!(m.get(hi).map(|o| o.ptr as usize), Some(2));
        finish(&mut m, &mut r2);
        // Machine 1's heap changed (a deallocation): its heap objects are not found again, its
        // instance data is.
        activate(&mut m, rt(1, 1), 1, &mut r1);
        assert!(!entered(&m, space::HEAP, 5) && entered(&m, space::INST, 3));
        finish(&mut m, &mut r1);
        // A record of another table is no record here.
        activate(&mut m, rt(1, 2), 0, &mut r2);
        r1.words[0] ^= 1 << 40;
        activate(&mut m, rt(1, 1), 1, &mut r1);
        assert!(!entered(&m, space::INST, 3));
    }

    /// Constants are the runtime's: every machine of it finds them, a machine of another
    /// runtime does not.
    #[test]
    fn constants_are_shared_by_the_machines_of_a_runtime() {
        let mut m = Mem::new(TierFrame::default());
        let mut r = TierState::default();
        activate(&mut m, rt(1, 1), 0, &mut r);
        for k in 100..200 {
            m.put_obj(obj(space::CONST, k));
        }
        finish(&mut m, &mut r);
        activate(&mut m, rt(1, 2), 7, &mut TierState::default());
        assert!((100..200).all(|k| entered(&m, space::CONST, k)));
        activate(&mut m, rt(2, 1), 0, &mut TierState::default());
        assert!(!entered(&m, space::CONST, 100));
    }

    /// A host call drops the frame and temporary objects; the machine's objects stay while its
    /// heap epoch does.
    #[test]
    fn host_calls_drop_volatile_objects_and_a_changed_heap() {
        let mut m = Mem::new(TierFrame::default());
        activate(&mut m, rt(1, 1), 0, &mut TierState::default());
        m.put_obj(obj(space::HEAP, 3));
        m.put_obj(obj(space::FRAME, 4));
        assert!(m.transient && entered(&m, space::FRAME, 4));
        m.forget();
        assert!(!m.transient && !entered(&m, space::FRAME, 4) && entered(&m, space::HEAP, 3));
        m.fr.heap_epoch = 1;
        m.forget();
        assert!(!entered(&m, space::HEAP, 3));
        m.put_obj(obj(space::HEAP, 3));
        // The next activation at that epoch keeps it.
        activate(&mut m, rt(1, 1), 1, &mut TierState::default());
        assert!(entered(&m, space::HEAP, 3));
    }

    /// A heap change drops the objects of the slots the heaps' logs name and keeps the rest, in
    /// the activation and from the process record; more changes than a log holds drop them all.
    #[test]
    fn a_heap_change_drops_only_the_slots_it_names() {
        let (h, s) = (space::HEAP, space::SHARED_HEAP);
        let mut m = Mem::new(TierFrame::default());
        let (mut log, mut slog) = ([0u32; HEAP_LOG], [0u32; HEAP_LOG]);
        let mut r = TierState::default();
        activate(&mut m, rt(1, 1), 0, &mut r);
        for k in 1..=4 {
            m.put_obj(obj(h, k));
            m.put_obj(obj(s, k));
        }
        // Epoch 1 of each heap: private slot 2 and shared slot 3 were deallocated.
        (log[1], slog[1]) = (2, 3);
        m.fr.heap_log = [log.as_ptr(), slog.as_ptr()];
        (m.fr.heap_epoch, m.fr.shared_epoch) = (1, 1);
        m.forget();
        assert!(!entered(&m, h, 2) && entered(&m, h, 3));
        assert!(!entered(&m, s, 3) && entered(&m, s, 2));
        finish(&mut m, &mut r);
        // Another machine runs, then the first comes back two changes later (slots 4 and 1).
        activate(&mut m, rt(1, 2), 1, &mut TierState::default());
        (log[2], log[3]) = (4, 1);
        m.fr.heap_log = [log.as_ptr(), slog.as_ptr()];
        activate(&mut m, rt(1, 1), 3, &mut r);
        assert!(entered(&m, h, 3) && !entered(&m, h, 4) && !entered(&m, h, 1));
        assert!(entered(&m, s, 1) && entered(&m, s, 4));
        // More changes than the log holds: nothing is kept.
        m.fr.heap_epoch = 4 + HEAP_LOG as u64;
        m.forget();
        assert!(!entered(&m, h, 3) && !entered(&m, s, 1));
    }

    /// When the tags run out the table starts again empty under a new identity, so a record
    /// made before names no tag of it.
    #[test]
    fn the_last_tag_empties_the_table() {
        let mut m = Mem::new(TierFrame::default());
        let mut r = TierState::default();
        activate(&mut m, rt(1, 1), 0, &mut r);
        m.put_obj(obj(space::HEAP, 3));
        m.put_obj(obj(space::CONST, 4));
        finish(&mut m, &mut r);
        let id = m.id;
        m.next_tag = TAG_END;
        activate(&mut m, rt(1, 2), 0, &mut TierState::default());
        assert_ne!(m.id, id);
        assert!(m.objs.iter().all(|o| o.hi == 0));
        assert!(!entered(&m, space::CONST, 4));
        activate(&mut m, rt(1, 1), 0, &mut r);
        assert!(!entered(&m, space::HEAP, 3));
        let mut t = m.tags.map(|t| t >> 32);
        assert!(t.iter().all(|&t| t < 16));
        t.sort();
        assert_eq!(t.windows(2).filter(|w| w[0] != w[1]).count(), 3);
    }
}
