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
    Called, MemoLook, NativeCall, NativeFrame, Stepped, TIER_SUSPEND, TIER_UNWIND, TierFrame,
    TierHost, space,
};

use yavs_ir::{AddrSpace, FOp, Func, FuncId, Inst, IntOp, Program, Reg, Width};

use crate::census::{Cb, cb};
pub const STEP: &str = "yavs_rt_step";

pub const HOST: &str = "yavs_rt_host";

pub const REGION: &str = "yavs_rt_region";

pub const SIGNAL: &str = "yavs_rt_signal";

pub const DRIVE: &str = "yavs_rt_drive";

pub const DRIVE_NAT: &str = "yavs_rt_drive_nat";

pub const DRIVE_BIT: &str = "yavs_rt_drive_bit";

pub const WAIT: &str = "yavs_rt_wait";

pub const WAIT_AGAIN: &str = "yavs_rt_wait_again";

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

pub const MEMEQ: &str = "yavs_rt_memeq";

pub const LAYOUT_WORD: &str = "yavs_rt_layout_word";

pub const SCAN: &str = "yavs_rt_scan";

pub const CODES: &str = "yavs_rt_codes";

pub const SCALAR: &str = "yavs_rt_scalar";

pub const PLANES: &str = "yavs_rt_planes";

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
/// the objects `yavs_rt_obj` and `Deref` found, valid until a step (a direct-mapped table:
/// the object of address high half `hi` is entry `obj_at(hi)`).
#[repr(C)]
pub struct Mem {
    pub fr: TierFrame,
    pub objs: [Obj; OBJS],
    /// Entries of `objs` in use, at most (0: none, nothing to forget).
    pub live: u64,
    /// Of those, the entries of `Life::Runtime`: they stay when another machine of the same
    /// runtime runs (`settle`).
    pub rt_live: u64,
    /// The entries (up to `MINE`) this activation entered or found in the process's record, in
    /// that order: what `save_objs` keeps, not what other machines left in the table.
    pub mine: [u16; MINE],
    pub nmine: u64,
    /// Per entry, the activation (`mine_gen`) that put it in `mine`.
    pub mine_at: [u32; OBJS],
    pub mine_gen: u32,
    /// Entries of the table `nslots` points at (the thread's table never shrinks).
    /// Which entries of `objs` are in use: the walks (`retain`, `save_objs`, `begin`) visit only
    /// those.
    pub occ: [u64; OBJS / 64],
    /// Bit `w` set: word `w` of `occ` has a bit set (the walks skip the empty words).
    pub occ_any: u64,
    /// The entries of `occ` that are not constants: what a change of machine or a host call may
    /// drop, so the walks of `retain_some` visit only those, however many constants are kept.
    pub occv: [u64; OBJS / 64],
    pub occv_any: u64,
    /// The `TierFrame::owner` and `heap_epoch` the objects were found at (`begin`).
    pub objs_owner: u64,
    pub objs_epoch: u64,
    /// `objs` changed since it was loaded from the process's record (`save_objs`).
    pub dirty: bool,
    /// Some entry of `objs` is not one `forget_end` keeps.
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

/// An object in `Mem::objs`: the high half of its addresses (`u64::MAX`: none), start,
/// length, whether it was found for writing, and the access value a `Deref` found it for
/// (`NO_H`: none; its address is `hi << 32`). An entry with an access value is writable
/// (`yavs_rt_obj` keeps the permission of an entry it replaces for the same object), so a
/// `Deref` needs one compare.
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
/// are two objects of one protected-type library function), two objects whose keys agree there
/// evict each other. 16 entries missed three heap objects in four on a test runner's queue
/// code (95 million `Deref` callbacks in a 16 s run, 12 million at 256); 1 024 made that three
/// times rarer; 4 096 (the most the one-word `occ_any` summary covers) since the constants of
/// every process stay in the table (#679: the constants of the LEON3 processes evicted each other
/// 2.0 million times in 150 us at 1 024, 0.5 million at 4 096). A walk visits only the entries in
/// use (`Mem::occ`, `Mem::occv`); ways if a profile shows conflicts again.
pub const OBJS: usize = 4096;

/// The entries a process's record holds (`Mem::mine`): the heap objects and instance data of
/// a large process are all found again at its next activation (#679).
pub const MINE: usize = 64;

const _: () = assert!(4 * MINE + 1 == yavs_interp::TIER_STATE_WORDS);

const _: () = assert!(OBJS.is_power_of_two() && OBJS >= 64 && OBJS <= 1 << 24);

/// The entry of `Mem::objs` for the object whose addresses have high half `hi`: the low bits of
/// its index, bit 3 flipped for a shared heap (space 7, bit 29 of `hi`; private is 4). The
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

pub const NO_OBJ: Obj = Obj {
    hi: u64::MAX,
    ptr: std::ptr::null_mut(),
    len: 0,
    w: 0,
    h: NO_H,
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

pub const MEM_COV_LEN: u64 = std::mem::offset_of!(Mem, fr.cov_len) as u64;

pub const MEM_OBJS: u64 = std::mem::offset_of!(Mem, objs) as u64;

pub const OBJ_PTR: u64 = std::mem::offset_of!(Obj, ptr) as u64;

pub const OBJ_LEN: u64 = std::mem::offset_of!(Obj, len) as u64;

pub const OBJ_W: u64 = std::mem::offset_of!(Obj, w) as u64;

pub const OBJ_H: u64 = std::mem::offset_of!(Obj, h) as u64;

/// Bytes of a `TierSig` and the offsets of its fields.
pub const SIG_ENT: i64 = std::mem::size_of::<yavs_interp::TierSig>() as i64;

pub const SIG_CODES: i64 = std::mem::offset_of!(yavs_interp::TierSig, codes) as i64;

pub const SIG_LEN: i64 = std::mem::offset_of!(yavs_interp::TierSig, len) as i64;

pub const SIG_SIGNAL: i64 = std::mem::offset_of!(yavs_interp::TierSig, signal) as i64;

pub const SIG_OFF: i64 = std::mem::offset_of!(yavs_interp::TierSig, off) as i64;

pub const SIG_CAP: i64 = std::mem::offset_of!(yavs_interp::TierSig, cap) as i64;

pub const SIG_STAMPS: i64 = std::mem::offset_of!(yavs_interp::TierSig, stamps) as i64;

pub const SIG_LAST: i64 = std::mem::offset_of!(yavs_interp::TierSig, last) as i64;

pub const SIG_PLANES: i64 = std::mem::offset_of!(yavs_interp::TierSig, planes) as i64;

pub const SIG_PLEN: i64 = std::mem::offset_of!(yavs_interp::TierSig, plen) as i64;

pub const SIG_BYTES: i64 = std::mem::offset_of!(yavs_interp::TierSig, bytes) as i64;

pub const SIG_ES: i64 = std::mem::offset_of!(yavs_interp::TierSig, es) as i64;

impl Mem {
    pub fn of(fr: TierFrame) -> Mem {
        Mem {
            fr,
            objs: [NO_OBJ; OBJS],
            live: 0,
            rt_live: 0,
            mine: [0; MINE],
            nmine: 0,
            mine_at: [0; OBJS],
            mine_gen: 1,
            occ: [0; OBJS / 64],
            occ_any: 0,
            occv: [0; OBJS / 64],
            occv_any: 0,
            objs_owner: 0,
            objs_epoch: 0,
            dirty: false,
            transient: false,
            temp_seen: fr.temp_len,
            ndeopt: 0,
            ntab: std::ptr::null(),
            stack_lo: 0,
            nslots: std::ptr::null_mut(),
            nslots_len: 0,
        }
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

    /// The start of an activation or a call from the interpreter, on a view just made: keep the
    /// objects found before that are still where they were (`Life`), and count the writes of
    /// the writable heap objects kept as this run's (`TierHost::wrote`).
    #[inline(always)]
    pub fn begin(&mut self, host: &mut dyn TierHost) {
        self.dirty = false;
        self.start_mine();
        // Nothing kept, or constants of this runtime only (a machine of it ran last): nothing
        // to do.
        if self.live != self.rt_live
            || self.live != 0
                && (self.objs_owner ^ self.fr.owner) >> yavs_interp::OWNER_RT_SHIFT != 0
        {
            self.begin_kept(host);
        }
    }

    #[inline(never)]
    fn begin_kept(&mut self, host: &mut dyn TierHost) {
        if self.settle() && self.fr.flags & yavs_interp::TIER_STAMPS != 0 {
            for o in self.in_use_some() {
                if o.w != 0 && life(o.hi) == Life::Heap {
                    host.wrote(o.hi << 32);
                }
            }
        }
    }

    /// Drop the entries the view's machine cannot use: another machine of the same runtime keeps
    /// the constants, another runtime keeps nothing. True when the heap objects are still good.
    fn settle(&mut self) -> bool {
        let same = self.objs_owner == self.fr.owner;
        let rt = (self.objs_owner ^ self.fr.owner) >> yavs_interp::OWNER_RT_SHIFT == 0;
        let epoch = same && self.objs_epoch == self.fr.heap_epoch;
        if !rt && self.rt_live != 0 {
            self.retain(|_| false);
        } else if !epoch || self.transient {
            self.retain_some(|hi| match life(hi) {
                Life::Machine => same,
                Life::Heap => epoch,
                _ => false,
            });
        }
        epoch
    }

    /// The entries of `objs` in use.
    #[inline]
    fn in_use(&self) -> impl Iterator<Item = &Obj> {
        BitIter(self.occ_any).flat_map(move |w| {
            let objs = &self.objs;
            BitIter(self.occ[w]).map(move |b| &objs[w * 64 + b])
        })
    }

    /// The entries of `objs` in use that are not constants.
    #[inline]
    pub fn in_use_some(&self) -> impl Iterator<Item = &Obj> {
        BitIter(self.occv_any).flat_map(move |w| {
            let objs = &self.objs;
            BitIter(self.occv[w]).map(move |b| &objs[w * 64 + b])
        })
    }

    /// The end of an activation or call: what `begin` checks next time.
    #[inline(always)]
    pub fn end(&mut self) {
        // `begin` looks at these only while objects are kept.
        if self.live != 0 {
            self.objs_owner = self.fr.owner;
            self.objs_epoch = self.fr.heap_epoch;
        }
    }

    /// Enter object `o` at its entry.
    #[inline(always)]
    pub fn put_obj(&mut self, o: Obj) {
        let at = obj_at(o.hi);
        // A frame or temporary area object does not outlive the activation: no record entry.
        if life(o.hi) != Life::Volatile {
            self.mine_add(at);
        }
        let e = &mut self.objs[at];
        if e.hi == u64::MAX {
            self.live += 1;
        } else {
            self.rt_live -= u64::from(life(e.hi) == Life::Runtime);
        }
        self.rt_live += u64::from(life(o.hi) == Life::Runtime);
        self.occ[at / 64] |= 1 << (at % 64);
        self.occ_any |= 1 << (at / 64);
        if life(o.hi) == Life::Runtime {
            self.occv[at / 64] &= !(1 << (at % 64));
        } else {
            self.occv[at / 64] |= 1 << (at % 64);
            self.occv_any |= 1 << (at / 64);
        }
        self.transient |= life(o.hi) == Life::Volatile;
        self.dirty = true;
        *e = o;
    }

    /// A new activation: nothing in `mine`.
    #[inline(always)]
    pub fn start_mine(&mut self) {
        self.nmine = 0;
        self.mine_gen = self.mine_gen.wrapping_add(1);
        if self.mine_gen == 0 {
            self.mine_at = [0; OBJS];
            self.mine_gen = 1;
        }
    }

    /// Entry `at` is one of this activation's (`mine`; one past `MINE` is not recorded).
    #[inline(always)]
    fn mine_add(&mut self, at: usize) {
        let n = self.nmine as usize;
        if n < MINE && self.mine_at[at] != self.mine_gen {
            self.mine_at[at] = self.mine_gen;
            self.mine[n] = at as u16;
            self.nmine += 1;
        }
    }

    /// The objects of a process's record (`save_objs`: up to `MINE` of `hi`, address, `len`,
    /// `w`), entered where `begin` left no other entry, or only a constant (another machine's).
    pub fn load_objs(&mut self, w: &[u64; yavs_interp::TIER_STATE_WORDS]) {
        for k in w[..4 * MINE].as_chunks::<4>().0 {
            // The entries are packed from the front (`save_kept`).
            if k[0] == u64::MAX {
                break;
            }
            let at = obj_at(k[0]);
            let held = self.objs[at].hi;
            if held == k[0] {
                self.mine_add(at);
            } else if held == u64::MAX || life(held) == Life::Runtime {
                self.put_obj(Obj {
                    hi: k[0],
                    ptr: k[1] as *mut u8,
                    len: k[2],
                    w: k[3],
                    h: NO_H,
                });
            }
        }
        self.dirty = false;
    }

    /// Keep the objects this activation found or restored (`mine`) for the next activation of
    /// this process, whatever ran on the thread in between (`TierState`).
    #[inline(always)]
    pub fn save_objs(&self, slot: &mut Option<Box<yavs_interp::TierState>>) {
        // Nothing found or dropped since the record was loaded, and it holds something: it stays.
        if !self.dirty && self.nmine != 0 && slot.as_ref().is_some_and(|st| st.used) {
            return;
        }
        self.save_kept(slot);
    }

    #[inline(never)]
    fn save_kept(&self, slot: &mut Option<Box<yavs_interp::TierState>>) {
        let mine = &self.mine[..self.nmine as usize];
        // Dropped since (a heap object after an edit in place) are left out.
        let kept = || {
            mine.iter()
                .map(|&at| &self.objs[at as usize])
                .filter(|o| o.hi != u64::MAX)
        };
        if kept().next().is_none() {
            if let Some(st) = slot {
                st.used = false;
            }
            return;
        }
        let st = slot.get_or_insert_with(Default::default);
        let mut n = 0;
        for o in kept() {
            st.words[n * 4..n * 4 + 4].copy_from_slice(&[o.hi, o.ptr as u64, o.len, o.w]);
            n += 1;
        }
        // The entries are packed from the front and end at the first `u64::MAX` (`load_objs`).
        if n < MINE {
            st.words[n * 4] = u64::MAX;
        }
        st.words[4 * MINE] = self.fr.heap_epoch;
        st.used = true;
    }

    /// The end of an activation: forget the frame and temporary objects found.
    #[inline]
    pub fn forget_end(&mut self) {
        if !self.transient {
            return;
        }
        self.retain_some(|hi| life(hi) != Life::Volatile);
    }

    /// Drop the objects whose high half `keep` refuses, constants included.
    pub fn retain(&mut self, keep: impl Fn(u64) -> bool) {
        let (mut live, mut rt_live) = (0, 0);
        for w in BitIter(self.occ_any) {
            for b in BitIter(self.occ[w]) {
                let o = &mut self.objs[w * 64 + b];
                if keep(o.hi) {
                    live += 1;
                    rt_live += u64::from(life(o.hi) == Life::Runtime);
                } else {
                    *o = NO_OBJ;
                    self.occ[w] &= !(1 << b);
                    self.occv[w] &= !(1 << b);
                    self.dirty = true;
                }
            }
            if self.occ[w] == 0 {
                self.occ_any &= !(1 << w);
            }
            if self.occv[w] == 0 {
                self.occv_any &= !(1 << w);
            }
        }
        self.live = live;
        self.rt_live = rt_live;
        let transient = self.in_use().any(|o| life(o.hi) == Life::Volatile);
        self.transient = transient;
    }

    /// `retain` that keeps every constant: the walk visits the other entries only (`occv`).
    #[inline]
    pub fn retain_some(&mut self, keep: impl Fn(u64) -> bool) {
        let (mut live, mut transient) = (self.rt_live, false);
        for w in BitIter(self.occv_any) {
            for b in BitIter(self.occv[w]) {
                let o = &mut self.objs[w * 64 + b];
                if keep(o.hi) {
                    live += 1;
                    transient |= life(o.hi) == Life::Volatile;
                } else {
                    *o = NO_OBJ;
                    self.occ[w] &= !(1 << b);
                    self.occv[w] &= !(1 << b);
                    self.dirty = true;
                }
            }
            if self.occ[w] == 0 {
                self.occ_any &= !(1 << w);
            }
            if self.occv[w] == 0 {
                self.occv_any &= !(1 << w);
            }
        }
        self.live = live;
        self.transient = transient;
    }

    /// Forget the objects found before a host call that may have moved frames, the temporary
    /// area or heap objects; those of the machine stay (`Life`).
    pub fn forget(&mut self) {
        if self.live == self.rt_live {
            return;
        }
        self.retain_some(|hi| life(hi) == Life::Machine);
    }

    /// `forget` after a host call that started at heap epoch `epoch` (`TierFrame::heap_epoch`):
    /// heap objects stay too while no heap object was deallocated or edited in place.
    pub fn forget_since(&mut self, epoch: u64) {
        let heap = self.fr.heap_epoch == epoch;
        // Nothing volatile found, no heap object freed or edited: nothing to drop.
        if self.live == self.rt_live || heap && !self.transient {
            return;
        }
        self.retain_some(|hi| match life(hi) {
            Life::Volatile => false,
            Life::Heap => heap,
            _ => true,
        });
    }
}

/// The positions of the set bits of a word, lowest first.
struct BitIter(u64);

impl Iterator for BitIter {
    type Item = usize;

    #[inline]
    fn next(&mut self) -> Option<usize> {
        if self.0 == 0 {
            return None;
        }
        let b = self.0.trailing_zeros() as usize;
        self.0 &= self.0 - 1;
        Some(b)
    }
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
    let epoch = unsafe { (*mem).fr.heap_epoch };
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
    if r != GO {
        mem.forget();
    } else if !keep {
        mem.forget_since(epoch);
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
            (word(0) as i64, [word(1), word(2), word(3), word(4)])
        };
        // The usual case, taken by the host without a frame view or a result.
        if unsafe { (*envp).host.drive_now(driver, [lo, len], delay, block) } {
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
        (word(0) as i64, [word(1), word(2), word(3), word(4)])
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
        (word(0) as i64, [word(1), word(2), word(3), word(4)])
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
    if delay == 0 && unsafe { (*envp).host.drive_bit_now(driver, [lo, len], code & !0x100) } {
        return GO;
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

/// The scalar logic assignments compiled code kept (`Batch`): `n` entries of two words, for
/// `TierHost::drive_bits`. `GO`, or the pc of the `Drive` of an entry that failed.
pub extern "C" fn yavs_rt_drive_flush(
    env: *mut c_void,
    _mem: *mut Mem,
    entries: *const [u64; 2],
    n: u32,
) -> u32 {
    cb(Cb::Drive);
    // SAFETY: as in `yavs_rt_step`; compiled code passes `n` entries it wrote.
    let envp = env as *mut Env<'_>;
    let ents = unsafe { std::slice::from_raw_parts(entries, n as usize) };
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
    if unsafe { (*envp).host.wait_again(pc, [lo, len]) } {
        return TIER_SUSPEND;
    }
    unsafe { (*mem).sync(&mut *(*envp).host) };
    let r = catch_unwind(AssertUnwindSafe(|| unsafe {
        (*envp).host.wait_suspend(&mut (*mem).fr, pc, [lo, len])
    }));
    // SAFETY: the pointers are the ones compiled code was given.
    unsafe { stepped(envp, mem, pc, r) }
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
    _lo: u64,
    _len: u64,
) -> u32 {
    cb(Cb::WaitSame);
    // SAFETY: as in `yavs_rt_step`.
    let envp = env as *mut Env<'_>;
    unsafe { (*mem).sync(&mut *(*envp).host) };
    if unsafe { (*envp).host.wait_same(pc) } {
        TIER_SUSPEND
    } else {
        GO
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
    let epoch = unsafe { (*mem).fr.heap_epoch };
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
    m.forget_since(epoch);
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
    let epoch = unsafe { (*mem).fr.heap_epoch };
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
    m.forget_since(epoch);
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
        let (stack, epoch) = ((*mem).fr.stack, (*mem).fr.heap_epoch);
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
        (*mem).forget_since(epoch);
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
            let o = &self.objs[obj_at(hi)];
            if o.hi != hi {
                return None;
            }
            (o.ptr.cast_const(), o.len)
        };
        ((a & 0xffff_ffff) <= len).then_some((base, len))
    }

    /// The bytes of the region address `a` points into, from the region's start (`Gen::host`
    /// holds them for a whole activation, `param_addrs`). The slots of a native caller between
    /// the interpreter's top frame and frame `depth` live as long as the callee: always. The top
    /// frame, the temporary area, the instance data (not for writing) and a heap object of the
    /// table only when `stable` (`stable_activation`: nothing the function does moves them).
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
            let o = &self.objs[obj_at(hi)];
            (o.hi == hi && (!write || o.w != 0)).then_some((o.ptr, o.len))
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
            let o = &self.objs[obj_at(hi)];
            if o.hi != hi || write && o.w == 0 {
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
    let (write, stable) = (flags & 1 != 0, flags & 2 != 0);
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
                    let e = &mut m.objs[obj_at(hi)];
                    if e.hi == hi && e.w != 0 {
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
            let e = &mem.objs[obj_at(hi)];
            let (h, w) = if e.hi == hi {
                (e.h, e.w | u64::from(write))
            } else {
                (NO_H, u64::from(write))
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

/// Run compiled `f` (generated for `nregs` registers) from `pc` (`Tier::run`); `lookup`
/// finds the compiled code of the functions it calls.
#[inline(always)]
pub fn enter(
    f: RunFn,
    nregs: usize,
    pc: u32,
    host: &mut dyn TierHost,
    lookup: Lookup<'_>,
    ntab: *const usize,
    state: Option<&mut Option<Box<yavs_interp::TierState>>>,
) -> Option<u32> {
    // This thread's `Mem`, or a new one inside an activation that holds it.
    let p = MEM.replace(std::ptr::null_mut());
    let mut mem = if p.is_null() {
        new_mem()
    } else {
        // SAFETY: `MEM` holds a `Box` of this thread's that nothing else uses (taken above).
        unsafe { Box::from_raw(p) }
    };
    let r = enter_on(&mut mem, f, nregs, pc, host, lookup, ntab, state);
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
    Box::new(Mem::of(TierFrame::default()))
}

thread_local! {
    /// The `Mem` of the activations on this thread, kept between them so that none builds one
    /// (`enter`). ponytail: one `Mem` per thread is never freed, a thread-exit hook if threads
    /// come and go by the thousand.
    static MEM: std::cell::Cell<*mut Mem> = const { std::cell::Cell::new(std::ptr::null_mut()) };
}

/// `enter` on `mem` (a fresh or reused `Mem`): what the compiled code returns, or the panic it
/// carried out of a callback.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
pub fn enter_on(
    mem: &mut Mem,
    f: RunFn,
    nregs: usize,
    pc: u32,
    host: &mut dyn TierHost,
    lookup: Lookup<'_>,
    ntab: *const usize,
    state: Option<&mut Option<Box<yavs_interp::TierState>>>,
) -> Result<Option<u32>, Box<dyn Any + Send>> {
    if !host.view_again(&mut mem.fr) {
        host.view(&mut mem.fr);
    }
    mem.temp_seen = mem.fr.temp_len;
    mem.ndeopt = 0;
    // The objects found before on this thread, and those of the process's own record, which
    // are good while the heaps did not change.
    mem.begin(host);
    if let Some(Some(st)) = state.as_deref()
        && st.used
        && st.words[4 * MINE] == mem.fr.heap_epoch
    {
        mem.load_objs(&st.words);
        // The heap objects it may write count as written in this activation.
        if mem.fr.flags & yavs_interp::TIER_STAMPS != 0 {
            for o in mem.in_use_some() {
                if o.w != 0 && life(o.hi) == Life::Heap {
                    host.wrote(o.hi << 32);
                }
            }
        }
    }
    // A frame index beyond the address format is the interpreter's error.
    if mem.fr.nregs != nregs as u64 || mem.fr.frame >= 1 << 29 {
        return Ok(None);
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
    // The frames of native calls are at most `max_depth` deep: their slots are registered here.
    let depth = mem.fr.max_depth.min(1 << 20) as usize;
    mem.set_nslots(depth);
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
    mem.sync(env.host);
    mem.forget_end();
    if let Some(slot) = state {
        mem.save_objs(slot);
    }
    mem.end();
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
    mem.begin(host);
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
    mem.forget_end();
    mem.end();
    match env.panic {
        Some(p) => Err(p),
        None => Ok(Some(out)),
    }
}

/// The code of `f<id>_c` (`Gen::adapter`): a native-frame function called from the interpreter
/// with its argument cells in an array.
pub type CallFn = unsafe extern "C" fn(*mut c_void, *mut Mem, u64, *const u64) -> i64;

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
    f.code.iter().all(|i| {
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
    })
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
        Inst::Alloc { dst, .. } => Some((dst, Known::Temp)),
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let hi = u64::from(sp) << 29 | index;
        m.objs[obj_at(hi)].hi == hi
    }

    /// A machine of the same runtime keeps the constants the last one found and drops its
    /// instance data and heap objects; a machine of another runtime keeps nothing.
    #[test]
    fn constants_outlive_the_machine_that_found_them() {
        let rt = |id: u64, n: u64| id << yavs_interp::OWNER_RT_SHIFT | n;
        let mut m = Box::new(Mem::of(TierFrame::default()));
        m.fr.owner = rt(1, 1);
        for o in [
            obj(space::CONST, 2),
            obj(space::INST, 3),
            obj(space::HEAP, 4),
        ] {
            m.put_obj(o);
        }
        m.end();
        assert_eq!((m.live, m.rt_live), (3, 1));
        // Another machine of runtime 1.
        m.fr.owner = rt(1, 2);
        assert!(m.live != m.rt_live);
        m.settle();
        assert_eq!((m.live, m.rt_live), (1, 1));
        assert!(entered(&m, space::CONST, 2));
        assert!(!entered(&m, space::INST, 3) && !entered(&m, space::HEAP, 4));
        // Only constants kept: a machine of the same runtime has nothing to settle, ...
        m.end();
        m.fr.owner = rt(1, 3);
        assert!(
            m.live == m.rt_live && (m.objs_owner ^ m.fr.owner) >> yavs_interp::OWNER_RT_SHIFT == 0
        );
        // ... and one of another runtime drops them.
        m.fr.owner = rt(2, 1);
        m.settle();
        assert_eq!((m.live, m.rt_live), (0, 0));
        assert!(!entered(&m, space::CONST, 2));
    }

    /// The record of a process (`save_objs`) holds what its activation found or restored, not the
    /// constants other machines left in the table; `load_objs` puts a constant of the record
    /// where only another machine's constant stands.
    #[test]
    fn the_process_record_holds_what_the_activation_used() {
        let rt1 = |n: u64| 1 << yavs_interp::OWNER_RT_SHIFT | n;
        let mut m = Box::new(Mem::of(TierFrame::default()));
        // Machine 1 finds a constant and a heap object and keeps them in its record.
        m.fr.owner = rt1(1);
        m.start_mine();
        m.put_obj(obj(space::CONST, 2));
        m.put_obj(obj(space::HEAP, 5));
        let mut rec1 = None;
        m.save_objs(&mut rec1);
        m.end();
        // Machine 2 of the runtime finds another constant: the first one is not its own.
        m.fr.owner = rt1(2);
        m.start_mine();
        m.settle();
        assert!(entered(&m, space::CONST, 2) && !entered(&m, space::HEAP, 5));
        m.put_obj(obj(space::CONST, 9));
        let mut rec2 = None;
        m.save_objs(&mut rec2);
        let st = rec2.as_ref().expect("a record");
        assert_eq!(st.words[0], u64::from(space::CONST) << 29 | 9);
        assert_eq!(st.words[4], u64::MAX);
        m.end();
        // Machine 1 again: its record is put back, the table's other constants stay.
        m.fr.owner = rt1(1);
        m.start_mine();
        m.settle();
        m.load_objs(&rec1.as_ref().expect("a record").words);
        assert!(entered(&m, space::CONST, 2) && entered(&m, space::HEAP, 5));
        // Unchanged since it was loaded: the record stays as it is.
        let before = rec1.as_ref().expect("a record").words;
        m.save_objs(&mut rec1);
        assert_eq!(rec1.expect("a record").words, before);
        // A constant of the record replaces another one at the same entry, not a heap object.
        let clash = u64::from(space::CONST) << 29 | (2 + OBJS as u64);
        assert_eq!(obj_at(clash), obj_at(obj(space::CONST, 2).hi));
        m.start_mine();
        m.put_obj(Obj {
            hi: clash,
            ..obj(space::CONST, 2)
        });
        let mut w = [u64::MAX; yavs_interp::TIER_STATE_WORDS];
        w[..4].copy_from_slice(&[obj(space::CONST, 2).hi, 0, 8, 0]);
        m.load_objs(&w);
        assert!(entered(&m, space::CONST, 2));
    }

    /// Every constant is kept for the next machine of the runtime, however many (#679): the walk
    /// that drops the other entries visits those only (`occv`), and a host call keeps them.
    #[test]
    fn constants_are_kept_however_many() {
        let rt1 = |n: u64| 1 << yavs_interp::OWNER_RT_SHIFT | n;
        let mut m = Box::new(Mem::of(TierFrame::default()));
        m.fr.owner = rt1(1);
        for k in 100..200 {
            m.put_obj(obj(space::CONST, k));
        }
        m.put_obj(obj(space::HEAP, 3));
        m.put_obj(obj(space::TEMP, 4));
        m.end();
        assert_eq!((m.live, m.rt_live), (102, 100));
        assert_eq!(m.in_use_some().count(), 2);
        m.forget();
        assert_eq!((m.live, m.rt_live), (100, 100));
        m.put_obj(obj(space::HEAP, 3));
        m.end();
        m.fr.owner = rt1(2);
        m.settle();
        assert_eq!((m.live, m.rt_live), (100, 100));
        assert!((100..200).all(|k| entered(&m, space::CONST, k)));
        assert!(!entered(&m, space::HEAP, 3) && m.in_use_some().count() == 0);
        // A heap object replaces a constant at its entry: the entry is one to drop again.
        let at = obj_at(obj(space::CONST, 105).hi);
        let clash = (0..4 * OBJS as u64)
            .map(|k| obj(space::HEAP, k))
            .find(|o| obj_at(o.hi) == at)
            .expect("an index");
        m.put_obj(clash);
        assert!(!entered(&m, space::CONST, 105));
        assert_eq!((m.live, m.rt_live, m.in_use_some().count()), (100, 99, 1));
    }
}
