//! Access-type heaps with generational handles and precise roots (ir.md 1.6, rule 6).
//!
//! A handle is `{tag: u8, index: u24, generation: u32}` packed in a u64; null is 0. Deallocating
//! bumps the slot's generation, so a stale handle is a clean error, never a use of the slot's
//! next occupant. Collection is mark and sweep from explicit roots; nothing is scanned
//! conservatively and objects never move.

use rsim_ir::{Program, Ty, TyId};

/// Handle tag of a process heap.
pub const PRIVATE: u8 = 1;
/// Handle tag of the shared heap.
pub const SHARED: u8 = 2;

const MAX_INDEX: u32 = (1 << 24) - 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeapError {
    Null,
    /// The object was deallocated (or collected).
    Stale,
    /// The handle belongs to another heap.
    WrongHeap,
    Full,
}

struct Slot {
    generation: u32,
    ty: TyId,
    data: Option<Vec<u8>>,
}

pub struct Heap {
    tag: u8,
    slots: Vec<Slot>,
    free: Vec<u32>,
    /// Bytes allocated since the last collection.
    pub since_gc: u64,
}

pub fn pack(tag: u8, index: u32, generation: u32) -> u64 {
    (u64::from(tag) << 56) | (u64::from(index) << 32) | u64::from(generation)
}

pub fn tag_of(h: u64) -> u8 {
    (h >> 56) as u8
}

impl Heap {
    pub fn new(tag: u8) -> Heap {
        Heap {
            tag,
            slots: Vec::new(),
            free: Vec::new(),
            since_gc: 0,
        }
    }

    pub fn live(&self) -> usize {
        self.slots.iter().filter(|s| s.data.is_some()).count()
    }

    pub fn alloc(&mut self, ty: TyId, data: Vec<u8>) -> Result<u64, HeapError> {
        self.since_gc = self.since_gc.saturating_add(data.len() as u64 + 32);
        let index = match self.free.pop() {
            Some(i) => i,
            None => {
                let i = self.slots.len() as u32;
                if i > MAX_INDEX {
                    return Err(HeapError::Full);
                }
                self.slots.push(Slot {
                    generation: 0,
                    ty,
                    data: None,
                });
                i
            }
        };
        let s = &mut self.slots[index as usize];
        s.generation = s.generation.wrapping_add(1).max(1);
        s.ty = ty;
        s.data = Some(data);
        Ok(pack(self.tag, index, s.generation))
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
        let i = self.resolve(h)?;
        self.release(i);
        Ok(())
    }

    fn release(&mut self, i: u32) {
        let s = &mut self.slots[i as usize];
        s.data = None;
        s.generation = s.generation.wrapping_add(1).max(1);
        self.free.push(i);
    }

    pub fn data(&self, index: u32) -> Option<&[u8]> {
        self.slots.get(index as usize)?.data.as_deref()
    }

    pub fn data_mut(&mut self, index: u32) -> Option<&mut [u8]> {
        self.slots.get_mut(index as usize)?.data.as_deref_mut()
    }

    /// Mark from `roots` and sweep. Returns false (and frees nothing) when an object's
    /// handles cannot be found from its type, so a collection is never unsound.
    pub fn collect(&mut self, prog: &Program, roots: &[u64]) -> bool {
        let mut mark = vec![false; self.slots.len()];
        let mut work: Vec<u64> = roots.to_vec();
        while let Some(h) = work.pop() {
            if tag_of(h) != self.tag {
                continue;
            }
            let Ok(i) = self.resolve(h) else { continue };
            if std::mem::replace(&mut mark[i as usize], true) {
                continue;
            }
            let s = &self.slots[i as usize];
            let Some(data) = &s.data else { continue };
            if !object_handles(prog, s.ty, data, &mut work) {
                return false;
            }
        }
        for (i, m) in mark.into_iter().enumerate() {
            if !m && self.slots[i].data.is_some() {
                self.release(i as u32);
            }
        }
        self.since_gc = 0;
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

/// The handle at the start of `data` (a `RootKind::Handle` slot).
pub fn value_handles_raw(data: &[u8], out: &mut Vec<u64>) {
    if let Some(h) = read_u64(data, 0) {
        out.push(h);
    }
}

/// Whether values of `ty` can contain handles.
pub fn has_handles(prog: &Program, ty: TyId) -> bool {
    fn go(prog: &Program, ty: TyId, depth: u32) -> bool {
        if depth > 64 {
            return true;
        }
        match prog.types.get(ty) {
            Some(Ty::Access { .. } | Ty::Protected { .. }) => true,
            Some(Ty::Array { elem, .. }) => go(prog, *elem, depth + 1),
            Some(Ty::Record { fields }) => fields.iter().any(|f| go(prog, f.ty, depth + 1)),
            Some(_) => false,
            None => true,
        }
    }
    go(prog, ty, 0)
}

/// Handles inside a heap object of `ty`. An array object is `[descriptor][elements]`.
pub fn object_handles(prog: &Program, ty: TyId, data: &[u8], out: &mut Vec<u64>) -> bool {
    if !has_handles(prog, ty) {
        return true;
    }
    match prog.types.get(ty) {
        Some(Ty::Array { .. }) => {
            let Some(dsz) = Ty::desc_size(&prog.types, ty) else {
                return false;
            };
            let Ok((l, _)) = Ty::layout(&prog.types, &prog.consts, ty, data) else {
                return false;
            };
            let Some(Ty::Array {
                elem, elem_desc, ..
            }) = prog.types.get(ty)
            else {
                return false;
            };
            // ponytail: elements needing their own descriptor are not scanned; collection is
            // skipped for such objects (the return below), add when records of arrays of
            // access values appear.
            if elem_desc.is_none() && Ty::needs_desc(&prog.types, *elem) {
                return false;
            }
            for k in 0..l.count {
                let at = dsz + k * l.stride;
                let Some(rest) = data.get(at as usize..) else {
                    return false;
                };
                if !value_handles(prog, *elem, rest, out) {
                    return false;
                }
            }
            true
        }
        _ => value_handles(prog, ty, data, out),
    }
}

/// Handles inside a statically laid out value of `ty` at the start of `data`.
pub fn value_handles(prog: &Program, ty: TyId, data: &[u8], out: &mut Vec<u64>) -> bool {
    match prog.types.get(ty) {
        Some(Ty::Access { .. } | Ty::Protected { .. }) => match read_u64(data, 0) {
            Some(h) => {
                out.push(h);
                true
            }
            None => false,
        },
        Some(Ty::Record { fields }) => {
            let Ok((l, _)) = Ty::layout(&prog.types, &prog.consts, ty, &[]) else {
                return false;
            };
            fields.iter().zip(&l.offsets).all(|(f, &off)| {
                !has_handles(prog, f.ty)
                    || data
                        .get(off as usize..)
                        .is_some_and(|d| value_handles(prog, f.ty, d, out))
            })
        }
        Some(Ty::Array { .. }) => !has_handles(prog, ty),
        Some(_) => true,
        None => false,
    }
}
