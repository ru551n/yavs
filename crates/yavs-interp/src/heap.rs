//! Access-type heaps with generational handles and precise roots (ir.md 1.6, rule 6).
//!
//! A handle is `{tag: u8, index: u24, generation: u32}` packed in a u64; null is 0. Deallocating
//! bumps the slot's generation, so a stale handle is a clean error, never a use of the slot's
//! next occupant. Collection is mark and sweep from explicit roots; nothing is scanned
//! conservatively and objects never move.
//!
//! An *address* into an object (from `Deref`) carries no generation, so a deallocated slot is
//! not reused at once: it waits in quarantine until the owner suspends (registers are dead
//! then) and [`Heap::recycle`] finds no address to it among the owner's precise roots (#81,
//! #422). An access through a stale address therefore always meets the freed slot, a clean
//! runtime error. A process that runs long without a wait also recycles and collects inside
//! its activation, between instructions; its live `Ptr` registers then count as addresses
//! and its live `Handle` registers as roots (#355, #409, #422).
//!
//! The shared heap is collected the same way, at a serial safepoint between cycles
//! (`crate::SharedGc`): roots are the shared variables area, the objects that existed when
//! the simulation started ([`Heap::seal`]) and the frames every serial process published at
//! its last suspension (#303).

use std::sync::atomic::{AtomicU64, Ordering};
use yavs_ir::{Program, Ty, TyId};

static COLLECTIONS: AtomicU64 = AtomicU64::new(0);

/// Collections finished by all heaps of this process so far (`--time-report`).
pub fn collections() -> u64 {
    COLLECTIONS.load(Ordering::Relaxed)
}

/// Handle tag of a process heap.
pub const PRIVATE: u8 = 1;
/// Handle tag of the shared heap.
pub const SHARED: u8 = 2;

const MAX_INDEX: u32 = (1 << 24) - 1;

/// Epoch changes a heap remembers the slot of ([`Heap::log`]): a holder of host addresses that
/// is at most this many changes behind drops only the objects of those slots.
pub const HEAP_LOG: usize = 16;

/// Quarantined slots that make an in-activation [`Heap::recycle`] due, at least.
const RECLAIM_MIN: usize = 4096;

/// The `gc-stress` feature (test builds): every safepoint after an allocation collects, every
/// one recycles, no slot is reused, and roots that cannot be located are a panic, so a missed
/// root shows (#422).
pub(crate) const STRESS: bool = cfg!(feature = "gc-stress");

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeapError {
    Null,
    /// The object was deallocated (or collected).
    Stale,
    /// The handle belongs to another heap.
    WrongHeap,
    Full,
}

/// What a heap object holds, for the GC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Obj {
    /// A value of this type (arrays as `[descriptor][elements]`).
    Typed(TyId),
    /// Bytes without handles (a `line` string made by a native).
    Bytes,
    /// `[descriptor: 24 bytes][handles]` (a `line_vector` made by a native).
    Handles,
    /// A `std.reflection` mirror (`crate::reflect`).
    Mirror,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Slot {
    generation: u32,
    ty: Obj,
    #[serde(with = "opt_bytes")]
    data: Option<Vec<u8>>,
    /// The object is `data[start..]`: a READ drops a line's prefix in O(1) (#196).
    start: usize,
    /// How often [`Heap::data_mut`] handed the bytes out since the allocation.
    writes: u32,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[repr(C)]
pub struct Heap {
    // What a suspension asks (`Machine::after_suspend`) comes first, in one cache line.
    tag: u8,
    /// Bytes allocated since the last collection.
    pub since_gc: u64,
    /// Deallocated slots not reusable yet (module docs).
    quarantine: Vec<u32>,
    /// Quarantine length at which [`Heap::reclaim_due`].
    reclaim_at: usize,
    /// Bytes of the objects that survived the last collection.
    pub live_bytes: u64,
    /// Changes whenever an object's bytes may have moved or gone (a deallocation, an edit in
    /// place): the host address of an object found at an earlier value is stale (compiled code
    /// keeps the addresses it found between activations).
    #[serde(skip)]
    epoch: u64,
    slots: Vec<Slot>,
    /// The slot of each of the last [`HEAP_LOG`] epoch changes, at `epoch % HEAP_LOG`.
    #[serde(skip)]
    log: [u32; HEAP_LOG],
    free: Vec<u32>,
    /// Handles that are roots for good ([`Heap::seal`]).
    sealed: Vec<u64>,
}

/// An object's bytes as one block in a serialised heap (package state, #407).
mod opt_bytes {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use crate::state::{Block, BlockRef};

    pub fn serialize<S: Serializer>(v: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        v.as_deref().map(BlockRef).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        Ok(Option::<Block>::deserialize(d)?.map(|b| b.0))
    }
}

pub fn pack(tag: u8, index: u32, generation: u32) -> u64 {
    (u64::from(tag) << 56) | (u64::from(index) << 32) | u64::from(generation)
}

#[inline(always)]
pub fn tag_of(h: u64) -> u8 {
    (h >> 56) as u8
}

impl Heap {
    pub fn new(tag: u8) -> Heap {
        Heap {
            tag,
            slots: Vec::new(),
            free: Vec::new(),
            quarantine: Vec::new(),
            reclaim_at: RECLAIM_MIN,
            since_gc: 0,
            live_bytes: 0,
            sealed: Vec::new(),
            epoch: 0,
            log: [0; HEAP_LOG],
        }
    }

    /// See the field `epoch`.
    #[inline]
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The slots of the last [`HEAP_LOG`] epoch changes, the change to epoch `e` at
    /// `e % HEAP_LOG`.
    #[inline]
    pub fn log(&self) -> &[u32; HEAP_LOG] {
        &self.log
    }

    pub fn tag(&self) -> u8 {
        self.tag
    }

    /// Slots in use, deallocated or not (for tests and statistics).
    pub fn slots(&self) -> usize {
        self.slots.len()
    }

    /// Whether this is heap `tag` holding only plain objects and nothing sealed: what a stored
    /// package state may hold (no reflection mirror, which refers into the running program).
    pub(crate) fn is_plain(&self, tag: u8) -> bool {
        self.tag == tag
            && self.sealed.is_empty()
            && !self
                .slots
                .iter()
                .any(|s| s.data.is_some() && s.ty == Obj::Mirror)
    }

    pub fn live(&self) -> usize {
        self.slots.iter().filter(|s| s.data.is_some()).count()
    }

    pub fn alloc(&mut self, ty: Obj, data: Vec<u8>) -> Result<u64, HeapError> {
        self.alloc_from(ty, data, 0)
    }

    /// Allocate the object `data[start..]`.
    pub fn alloc_from(&mut self, ty: Obj, data: Vec<u8>, start: usize) -> Result<u64, HeapError> {
        self.since_gc = self.since_gc.saturating_add(data.len() as u64 + 32);
        // Stress builds never reuse a slot: an access to a wrongly freed object stays an error.
        let index = match self.free.pop().filter(|_| !STRESS) {
            Some(i) => i,
            None => {
                let i = self.slots.len() as u32;
                if i > MAX_INDEX {
                    return Err(HeapError::Full);
                }
                self.slots.push(Slot {
                    generation: 0,
                    ty: Obj::Bytes,
                    data: None,
                    start: 0,
                    writes: 0,
                });
                i
            }
        };
        let s = &mut self.slots[index as usize];
        s.generation = s.generation.wrapping_add(1).max(1);
        s.ty = ty;
        s.data = Some(data);
        s.start = start;
        s.writes = 0;
        Ok(pack(self.tag, index, s.generation))
    }

    /// The bytes of live object `h` for an edit in place, as `(slot, data, start)`: the object
    /// is `data[start..]`. Finish with [`Heap::reissue`], which makes `h` stale and returns the
    /// handle of the edited object (textio READ and WRITE, #428).
    pub fn edit(&mut self, h: u64) -> Result<(u32, &mut Vec<u8>, &mut usize), HeapError> {
        let i = self.resolve(h)?;
        self.epoch += 1;
        self.log[self.epoch as usize % HEAP_LOG] = i;
        let s = &mut self.slots[i as usize];
        s.writes = s.writes.wrapping_add(1);
        match s.data.as_mut() {
            Some(d) => Ok((i, d, &mut s.start)),
            None => Err(HeapError::Stale),
        }
    }

    /// The handle of the object edited by [`Heap::edit`] (`grown` bytes added): a new
    /// generation of the same slot, so every older handle is stale as after a deallocation.
    /// Addresses into the object (not handles) see the edited bytes; the object kept its slot.
    pub fn reissue(&mut self, i: u32, grown: usize) -> u64 {
        self.since_gc = self.since_gc.saturating_add(grown as u64);
        let s = &mut self.slots[i as usize];
        s.generation = s.generation.wrapping_add(1).max(1);
        s.writes = 0;
        pack(self.tag, i, s.generation)
    }

    /// The slot index of a live handle.
    pub fn resolve(&self, h: u64) -> Result<u32, HeapError> {
        if h == 0 {
            return Err(HeapError::Null);
        }
        if tag_of(h) != self.tag {
            return Err(HeapError::WrongHeap);
        }
        let index = ((h >> 32) & u64::from(MAX_INDEX)) as u32;
        match self.slots.get(index as usize) {
            Some(s) if s.data.is_some() && s.generation == h as u32 => Ok(index),
            _ => Err(HeapError::Stale),
        }
    }

    pub fn free(&mut self, h: u64) -> Result<(), HeapError> {
        self.take(h).map(drop)
    }

    /// Deallocate like [`Heap::free`] but hand back the object's bytes as `(data, start)`
    /// (the object was `data[start..]`), so a native can move them to a new object without a
    /// copy (textio READ and WRITE, #196).
    pub fn take(&mut self, h: u64) -> Result<(Vec<u8>, usize), HeapError> {
        let i = self.resolve(h)?;
        let s = &mut self.slots[i as usize];
        let out = (s.data.take().unwrap_or_default(), s.start);
        self.release(i);
        self.quarantine.push(i);
        Ok(out)
    }

    fn release(&mut self, i: u32) {
        self.epoch += 1;
        self.log[self.epoch as usize % HEAP_LOG] = i;
        let s = &mut self.slots[i as usize];
        s.data = None;
        s.generation = s.generation.wrapping_add(1).max(1);
    }

    /// Whether deallocated slots wait for [`Heap::recycle`].
    pub fn has_quarantine(&self) -> bool {
        !self.quarantine.is_empty()
    }

    /// Make quarantined slots reusable, except those in `pinned` (sorted slot indices that
    /// addresses held by the owner may still reach). Call only at a suspension.
    pub fn recycle(&mut self, pinned: &[u32]) {
        let (keep, go): (Vec<u32>, Vec<u32>) = self
            .quarantine
            .iter()
            .partition(|i| pinned.binary_search(i).is_ok());
        self.quarantine = keep;
        self.free.extend(go);
        self.reclaim_at = (2 * self.quarantine.len()).max(RECLAIM_MIN);
    }

    /// Whether enough deallocated slots wait that a [`Heap::recycle`] inside an activation
    /// pays for its address search: twice what the last one had to keep, so slots that stay
    /// pinned do not make every search due. ponytail: the search also scans every frame, so a
    /// process with megabytes of frame data still searches once per few thousand frees; a
    /// bound proportional to the frame size if that shows in a profile.
    pub fn reclaim_due(&self) -> bool {
        self.quarantine.len() >= self.reclaim_at || STRESS && !self.quarantine.is_empty()
    }

    pub fn data(&self, index: u32) -> Option<&[u8]> {
        let s = self.slots.get(index as usize)?;
        s.data.as_deref()?.get(s.start..)
    }

    pub fn kind(&self, index: u32) -> Option<Obj> {
        let s = self.slots.get(index as usize)?;
        s.data.as_ref().map(|_| s.ty)
    }

    pub fn data_mut(&mut self, index: u32) -> Option<&mut [u8]> {
        let s = self.slots.get_mut(index as usize)?;
        s.writes = s.writes.wrapping_add(1);
        s.data.as_deref_mut()?.get_mut(s.start..)
    }

    /// The bytes of live object `dst` (to write) and of a different live object `src`, both at
    /// once (a copy from one object to another of the same heap).
    pub fn data_pair(&mut self, dst: u32, src: u32) -> Option<(&mut [u8], &[u8])> {
        let (d, s) = (dst as usize, src as usize);
        let (dsl, ssl) = if d < s {
            let (lo, hi) = self.slots.split_at_mut(s);
            (lo.get_mut(d)?, hi.first()?)
        } else if s < d {
            let (lo, hi) = self.slots.split_at_mut(d);
            (hi.first_mut()?, lo.get(s)?)
        } else {
            return None;
        };
        dsl.writes = dsl.writes.wrapping_add(1);
        Some((
            dsl.data.as_deref_mut()?.get_mut(dsl.start..)?,
            ssl.data.as_deref()?.get(ssl.start..)?,
        ))
    }

    /// The generation and write count of live object `index`: equal stamps mean its bytes
    /// are unchanged, because every write goes through [`Heap::data_mut`]. ponytail: the
    /// count wraps after 2^32 writes between two stamps.
    pub fn stamp(&self, index: u32) -> Option<(u32, u32)> {
        let s = self.slots.get(index as usize)?;
        s.data.as_ref()?;
        Some((s.generation, s.writes))
    }

    /// Make every live object a root for good. The shared heap is sealed when the simulation
    /// starts: the objects elaboration made are reached from package and instance data, which
    /// have no root maps. ponytail: elaboration garbage is kept with them; root maps for
    /// package and instance data would let it go.
    pub fn seal(&mut self) {
        let tag = self.tag;
        self.sealed = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.data.is_some())
            .map(|(i, s)| pack(tag, i as u32, s.generation))
            .collect();
    }

    /// Whether a collection is due: more allocated since the last one than both `threshold`
    /// and what survived it, so the work stays proportional to the allocation.
    pub fn gc_due(&self, threshold: u64) -> bool {
        let at = if STRESS {
            0
        } else {
            threshold.max(self.live_bytes)
        };
        self.since_gc > at
    }

    /// Mark from `roots` and from the objects in `pinned` (sorted slot indices that addresses
    /// still reach), sweep, then [`Heap::recycle`]. Returns false (and frees nothing) when an
    /// object's handles cannot be found from its type, so a collection is never unsound.
    pub fn collect(&mut self, prog: &Program, roots: &[u64], pinned: &[u32]) -> bool {
        let mut mark = vec![false; self.slots.len()];
        let mut work: Vec<u64> = roots.to_vec();
        work.extend(&self.sealed);
        for &i in pinned {
            if let Some(s) = self.slots.get(i as usize).filter(|s| s.data.is_some()) {
                work.push(pack(self.tag, i, s.generation));
            }
        }
        while let Some(h) = work.pop() {
            if tag_of(h) != self.tag {
                continue;
            }
            let Ok(i) = self.resolve(h) else { continue };
            if std::mem::replace(&mut mark[i as usize], true) {
                continue;
            }
            let Some(data) = self.data(i) else { continue };
            let s = &self.slots[i as usize];
            let ok = match s.ty {
                Obj::Typed(t) => object_handles(prog, t, data, &mut work),
                Obj::Bytes => true,
                Obj::Mirror => crate::reflect::mirror_handles(prog, data, &mut work),
                Obj::Handles => {
                    let mut at = 24;
                    while let Some(h) = read_u64(data, at) {
                        work.push(h);
                        at += 8;
                    }
                    true
                }
            };
            if !ok {
                return false;
            }
        }
        let mut live = 0;
        for (i, m) in mark.into_iter().enumerate() {
            match &self.slots[i].data {
                Some(d) if m => live += d.len() as u64 + 32,
                Some(_) => {
                    self.release(i as u32);
                    self.free.push(i as u32);
                }
                None => {}
            }
        }
        self.recycle(pinned);
        self.since_gc = 0;
        self.live_bytes = live;
        COLLECTIONS.fetch_add(1, Ordering::Relaxed);
        true
    }
}

fn read_u64(b: &[u8], at: u64) -> Option<u64> {
    let at = usize::try_from(at).ok()?;
    let s = b.get(at..at.checked_add(8)?)?;
    let mut a = [0; 8];
    a.copy_from_slice(s);
    Some(u64::from_le_bytes(a))
}

/// What a precise scan finds (plan rule 6, #422): handles that are roots, and addresses whose
/// heap objects stay (an address carries no handle, so its object is pinned).
#[derive(Default)]
pub struct Refs {
    pub handles: Vec<u64>,
    pub addrs: Vec<u64>,
}

impl Refs {
    /// Slot indices of heap address space `space` that the addresses point into, sorted.
    pub fn pinned(&self, space: u8) -> Vec<u32> {
        let mut v = Vec::new();
        self.pinned_into(space, &mut v);
        v
    }

    /// [`Refs::pinned`] into `v`, which keeps its buffer (a serial process publishes at every wake).
    pub fn pinned_into(&self, space: u8, v: &mut Vec<u32>) {
        v.clear();
        v.extend(
            self.addrs
                .iter()
                .map(|&a| crate::exec::split(a))
                .filter(|&(sp, _, _)| sp == space)
                .map(|(_, i, _)| i),
        );
        v.sort_unstable();
        v.dedup();
    }
}

/// Handles and addresses in the slots of `mem` that root map `map` names (a frame, the
/// shared variables area). False when one cannot be located.
pub fn slot_roots(
    prog: &Program,
    map: &[(u32, yavs_ir::RootKind)],
    mem: &[u8],
    out: &mut Refs,
) -> bool {
    map.iter()
        .all(|&(off, kind)| match (mem.get(off as usize..), kind) {
            (Some(d), yavs_ir::RootKind::Handle) => {
                value_handles_raw(d, &mut out.handles);
                true
            }
            (Some(d), yavs_ir::RootKind::Addr) => {
                value_handles_raw(d, &mut out.addrs);
                true
            }
            (Some(d), yavs_ir::RootKind::Composite(t)) => {
                value_handles(prog, t, d, &mut out.handles)
            }
            (None, _) => false,
        })
}

/// The handle at the start of `data` (a `RootKind::Handle` slot).
pub fn value_handles_raw(data: &[u8], out: &mut Vec<u64>) {
    if let Some(h) = read_u64(data, 0) {
        out.push(h);
    }
}

/// Whether values of `ty` can contain handles.
pub fn has_handles(prog: &Program, ty: TyId) -> bool {
    Ty::has_handles(&prog.types, ty)
}

/// Handles inside a heap object of `ty`: `[descriptor][value]`, the descriptor empty when
/// the type needs none.
pub fn object_handles(prog: &Program, ty: TyId, data: &[u8], out: &mut Vec<u64>) -> bool {
    if !has_handles(prog, ty) {
        return true;
    }
    let Some(dsz) = Ty::desc_size(&prog.types, ty) else {
        return false;
    };
    match (data.get(..dsz as usize), data.get(dsz as usize..)) {
        (Some(desc), Some(v)) => handles_in(prog, ty, desc, v, out),
        _ => false,
    }
}

/// Handles inside a statically laid out value of `ty` at the start of `data` (a
/// `RootKind::Composite` slot); a type that needs a descriptor fails (no descriptor here).
pub fn value_handles(prog: &Program, ty: TyId, data: &[u8], out: &mut Vec<u64>) -> bool {
    handles_in(prog, ty, &[], data, out)
}

/// Handles inside a value of `ty` constrained by `desc` (the descriptor format of
/// `yavs_ir::ty`) at the start of `data`. False when the layout cannot be computed.
pub(crate) fn handles_in(
    prog: &Program,
    ty: TyId,
    desc: &[u8],
    data: &[u8],
    out: &mut Vec<u64>,
) -> bool {
    if !has_handles(prog, ty) {
        return true;
    }
    let (types, consts) = (&prog.types, &prog.consts);
    let const_desc = |c| match consts.get(c) {
        Some(yavs_ir::ConstData::Bytes(b)) => Some(b.as_slice()),
        _ => None,
    };
    match types.get(ty) {
        Some(Ty::Access { .. } | Ty::Protected { .. }) => match read_u64(data, 0) {
            Some(h) => {
                out.push(h);
                true
            }
            None => false,
        },
        Some(Ty::Record { fields }) => {
            let Ok((l, _)) = Ty::layout(types, consts, ty, desc) else {
                return false;
            };
            let mut used = 0;
            for (f, &off) in fields.iter().zip(&l.offsets) {
                let fdesc = match f.desc {
                    Some(c) => const_desc(c),
                    None => desc.get(used..),
                };
                if f.desc.is_none() {
                    let Some(n) = Ty::desc_size(types, f.ty) else {
                        return false;
                    };
                    used += n as usize;
                }
                let ok = match (fdesc, data.get(off as usize..)) {
                    (Some(fd), Some(d)) => handles_in(prog, f.ty, fd, d, out),
                    _ => false,
                };
                if !ok {
                    return false;
                }
            }
            true
        }
        Some(Ty::Array {
            elem,
            elem_desc,
            dims,
        }) => {
            let Ok((l, _)) = Ty::layout(types, consts, ty, desc) else {
                return false;
            };
            let edesc = match elem_desc {
                Some(c) => const_desc(*c),
                None => desc.get(usize::from(*dims) * yavs_ir::DOPE_DIM_BYTES as usize..),
            };
            let Some(edesc) = edesc else {
                return false;
            };
            (0..l.count).all(|k| {
                k.checked_mul(l.stride)
                    .and_then(|at| data.get(at as usize..))
                    .is_some_and(|d| handles_in(prog, *elem, edesc, d, out))
            })
        }
        Some(_) => true,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yavs_ir::{Field, IdVec, Program};

    /// Every epoch change names its slot in the log (compiled code drops only that object).
    #[test]
    fn the_log_names_the_slot_of_each_change() {
        let mut h = Heap::new(PRIVATE);
        let hs: Vec<u64> = (0..3)
            .map(|_| h.alloc(Obj::Typed(TyId(0)), vec![0; 8]).unwrap())
            .collect();
        let slot = |x: u64| ((x >> 32) & u64::from(MAX_INDEX)) as u32;
        h.free(hs[1]).unwrap();
        assert_eq!(h.log()[h.epoch() as usize % HEAP_LOG], slot(hs[1]));
        let (i, _, _) = h.edit(hs[2]).unwrap();
        assert_eq!((h.epoch(), h.log()[2]), (2, i));
        assert_eq!(i, slot(hs[2]));
    }

    #[test]
    fn handles_and_collection() {
        let mut types = IdVec::new();
        let int = types.push(Ty::Int { lo: 0, hi: 9 });
        let node = types.push(Ty::Record {
            fields: vec![
                Field {
                    name: "v".into(),
                    ty: int,
                    desc: None,
                },
                Field {
                    name: "next".into(),
                    ty: TyId(2),
                    desc: None,
                },
            ],
        });
        types.push(Ty::Access { target: node });
        let prog = Program {
            types,
            ..Program::default()
        };
        let mut h = Heap::new(PRIVATE);
        // a -> b, c unreachable.
        let b = h.alloc(Obj::Typed(node), vec![0; 16]).unwrap();
        let mut ad = vec![0; 8];
        ad.extend(b.to_le_bytes());
        let a = h.alloc(Obj::Typed(node), ad).unwrap();
        let c = h.alloc(Obj::Typed(node), vec![0; 16]).unwrap();
        assert!(h.collect(&prog, &[a], &[]));
        assert_eq!(h.live(), 2);
        assert_eq!(h.resolve(c), Err(HeapError::Stale));
        assert!(h.resolve(b).is_ok());
        h.free(a).unwrap();
        assert_eq!(h.free(a), Err(HeapError::Stale));
        let (ai, ci) = (h.resolve(b).unwrap() + 1, h.resolve(b).unwrap() + 2);
        // A deallocated slot is not reused before `recycle` (#81); a collected one is. The
        // `gc-stress` build never reuses a slot: each allocation takes the next fresh index
        // and `recycle` only fills the free list.
        let fresh = h.slots.len() as u32;
        let at = |stress: u32, reuse: u32| if STRESS { stress } else { reuse };
        let d = h.alloc(Obj::Typed(int), vec![0; 8]).unwrap();
        assert_eq!(h.resolve(d), Ok(at(fresh, ci)));
        let e = h.alloc(Obj::Typed(int), vec![0; 8]).unwrap();
        assert_eq!(h.resolve(e), Ok(at(fresh + 1, 3)));
        // A pinned slot stays in quarantine; an unpinned one is reused with a new generation.
        h.recycle(&[ai]);
        assert!(!h.free.contains(&ai));
        let f = h.alloc(Obj::Typed(int), vec![0; 8]).unwrap();
        assert_eq!(h.resolve(f), Ok(at(fresh + 2, 4)));
        h.recycle(&[]);
        assert!(h.free.contains(&ai) || !STRESS);
        let g = h.alloc(Obj::Typed(int), vec![0; 8]).unwrap();
        assert_eq!(h.resolve(g), Ok(at(fresh + 3, ai)));
        assert_ne!(g, a);
        assert_eq!(h.resolve(a), Err(HeapError::Stale));
        assert_eq!(h.resolve(0), Err(HeapError::Null));
        assert_eq!(h.resolve(pack(SHARED, 0, 1)), Err(HeapError::WrongHeap));
    }

    /// An array of records whose elements need their own descriptor (an unconstrained
    /// `string` field) next to an access field: the handles are found through the
    /// descriptors (#80).
    #[test]
    fn elements_with_their_own_descriptor() {
        let mut types = IdVec::new();
        let int = types.push(Ty::Int { lo: 0, hi: 9 });
        let chr = types.push(Ty::Enum {
            lits: (0..=255u8).map(|c| format!("'{}'", c as char)).collect(),
            logic: None,
        });
        let string = types.push(Ty::Array {
            elem: chr,
            elem_desc: None,
            dims: 1,
        });
        let acc = types.push(Ty::Access { target: int });
        let rec = types.push(Ty::Record {
            fields: vec![
                Field {
                    name: "s".into(),
                    ty: string,
                    desc: None,
                },
                Field {
                    name: "p".into(),
                    ty: acc,
                    desc: None,
                },
            ],
        });
        let arr = types.push(Ty::Array {
            elem: rec,
            elem_desc: None,
            dims: 1,
        });
        let prog = Program {
            types,
            ..Program::default()
        };
        let mut h = Heap::new(PRIVATE);
        let a = h.alloc(Obj::Typed(int), vec![0; 8]).unwrap();
        let b = h.alloc(Obj::Typed(int), vec![0; 8]).unwrap();
        let junk = h.alloc(Obj::Typed(int), vec![0; 8]).unwrap();
        // [0 to 1] of records with s: string(1 to 3); an element is 3 bytes, pad, 8-byte p.
        let mut obj: Vec<u8> = [0i64, 1, 0, 1, 3, 0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        for p in [a, b] {
            obj.extend(b"abc\0\0\0\0\0");
            obj.extend(p.to_le_bytes());
        }
        let root = h.alloc(Obj::Typed(arr), obj).unwrap();
        assert!(h.collect(&prog, &[root], &[]));
        assert_eq!(h.live(), 3);
        assert!(h.resolve(a).is_ok() && h.resolve(b).is_ok());
        assert_eq!(h.resolve(junk), Err(HeapError::Stale));
    }
}
