//! Images of record signals (#1065): one copy of a record value, in the layout of the value the
//! process code uses, that a process reads with one copy instead of leaf by leaf. A leaf is
//! written when it changes: after the update of a cycle, serially, from the runs each signal
//! lists in `changed`, so force, deposit, ports and conversions come along with the value they
//! gave. Only written between run phases, read during them. Nothing else reads an image.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::signal::{Range, Signal, SignalId};
use crate::value::Buf;

/// How a leaf lies in an image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageKind {
    /// One logic element: its code less the bias, little endian in the leaf's bytes.
    Code(u8),
    /// Logic elements as plane blocks.
    Planes,
    /// One element of a byte signal in a wider or equal cell: four bytes are sign extended.
    Scalar,
    /// Elements of a byte signal as they are kept.
    Raw,
}

/// One leaf instance: elements `k .. k + len` of `sig`, at `voff .. voff + vsize` of the image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageItem {
    pub sig: SignalId,
    pub k: u32,
    pub len: u32,
    pub voff: u32,
    pub vsize: u32,
    pub kind: ImageKind,
}

/// What an image holds. Equal specs share one image, whichever process asked.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ImageSpec {
    pub size: u32,
    pub items: Box<[ImageItem]>,
}

/// An id `Images::find` gives for a spec with no image: its leaves overlap or do not fit.
pub const NO_IMAGE: u32 = u32::MAX;

/// The items of one image on one signal, in element order.
pub(crate) struct SigImage {
    image: u32,
    items: Box<[ImageItem]>,
}

#[derive(Default)]
pub(crate) struct Images {
    pub list: Vec<Box<[u8]>>,
    index: HashMap<ImageSpec, u32>,
    /// What processes asked for in this run phase, read after it.
    asked: Mutex<Vec<ImageSpec>>,
    any_asked: AtomicBool,
    /// Bit `s` set: signal `s` has a leaf in an image.
    has: Vec<u64>,
}

impl Images {
    /// `spec` as soon as no process runs. Only taken once per process and plan.
    pub fn ask(&self, spec: ImageSpec) {
        if let Ok(mut a) = self.asked.lock() {
            a.push(spec);
            self.any_asked.store(true, Ordering::Relaxed);
        }
    }

    pub fn find(&self, spec: &ImageSpec) -> Option<u32> {
        self.index.get(spec).copied()
    }

    #[inline(always)]
    pub fn asked(&self) -> bool {
        self.any_asked.load(Ordering::Relaxed)
    }

    /// Make the images asked for, from the current values: called when no process runs and
    /// nothing is listed in `changed`, before the update that first has to reach them.
    #[cold]
    #[inline(never)]
    pub fn install(&mut self, signals: &mut [Signal]) {
        self.any_asked.store(false, Ordering::Relaxed);
        let asked = match self.asked.get_mut() {
            Ok(a) => std::mem::take(a),
            Err(_) => return,
        };
        for spec in asked {
            if self.index.contains_key(&spec) {
                continue;
            }
            let id = match fits(&spec, signals) {
                true => self.list.len() as u32,
                false => NO_IMAGE,
            };
            if id != NO_IMAGE {
                let mut bytes = vec![0u8; spec.size as usize].into_boxed_slice();
                let mut items = spec.items.to_vec();
                items.sort_unstable_by_key(|t| (t.sig, t.k));
                for g in items.chunk_by(|a, b| a.sig == b.sig) {
                    let sig = &mut signals[g[0].sig.0 as usize];
                    for t in g {
                        write_item(sig, t, &mut bytes);
                    }
                    // The runs of every change, exact: cost follows the leaves that changed.
                    sig.watched = true;
                    sig.fine = true;
                    let w = g[0].sig.0 as usize / 64;
                    if self.has.len() <= w {
                        self.has.resize(w + 1, 0);
                    }
                    self.has[w] |= 1 << (g[0].sig.0 % 64);
                    sig.images.push(SigImage {
                        image: id,
                        items: g.into(),
                    });
                }
                self.list.push(bytes);
            }
            self.index.insert(spec, id);
        }
    }

    /// Write the leaves that changed in this update; the number written.
    pub fn push(&mut self, signals: &[Signal], changed: &[u32]) -> u64 {
        let mut n = 0;
        for &s in changed {
            // A bit per signal: most changed signals have no image, and their `Signal` line
            // stays cold.
            if self
                .has
                .get(s as usize / 64)
                .is_none_or(|w| w >> (s % 64) & 1 == 0)
            {
                continue;
            }
            let sig = &signals[s as usize];
            for im in &sig.images {
                let bytes = &mut self.list[im.image as usize];
                n += write_runs(sig, &im.items, &sig.changed, bytes);
            }
        }
        n
    }

    /// Images and the bytes they and their item lists hold, for the work counters.
    pub fn memory(&self) -> (u64, u64) {
        let items: usize = self.index.keys().map(|s| s.items.len()).sum();
        let bytes: usize = self.list.iter().map(|b| b.len()).sum();
        let item = std::mem::size_of::<ImageItem>();
        (self.list.len() as u64, (bytes + 2 * items * item) as u64)
    }
}

/// Every leaf inside its image and its signal, of the storage it says, and no two sharing a byte
/// or an element (two fields of a port tied to one actual share elements).
fn fits(spec: &ImageSpec, signals: &[Signal]) -> bool {
    let mut by_v: Vec<_> = spec.items.iter().collect();
    by_v.sort_unstable_by_key(|t| t.voff);
    if by_v.windows(2).any(|w| w[0].voff + w[0].vsize > w[1].voff) {
        return false;
    }
    let mut by_k: Vec<_> = spec.items.iter().collect();
    by_k.sort_unstable_by_key(|t| (t.sig, t.k));
    if by_k
        .windows(2)
        .any(|w| w[0].sig == w[1].sig && w[0].k + w[0].len > w[1].k)
    {
        return false;
    }
    spec.items.iter().all(|t| {
        let Some(sig) = signals.get(t.sig.0 as usize) else {
            return false;
        };
        let logic = matches!(sig.cur, Buf::Planes { .. });
        let storage = match t.kind {
            ImageKind::Code(_) => logic && t.len == 1,
            ImageKind::Planes => logic && t.vsize == t.len.div_ceil(64) * 32,
            ImageKind::Scalar => !logic && t.len == 1,
            ImageKind::Raw => !logic,
        };
        storage
            && t.k.checked_add(t.len).is_some_and(|e| e <= sig.ty.len)
            && t.voff.checked_add(t.vsize).is_some_and(|e| e <= spec.size)
            && (!matches!(t.kind, ImageKind::Raw)
                || sig.cur.bytes(t.k as usize, t.len as usize).map(<[u8]>::len)
                    == Some(t.vsize as usize))
    })
}

/// The items among `items` (in element order) that the runs touch, written; how many. Runs come
/// mostly in element order: the walk goes on from the last item and searches only when a run starts
/// before it.
#[inline(always)]
fn write_runs(sig: &Signal, items: &[ImageItem], runs: &[Range], bytes: &mut [u8]) -> u64 {
    let (mut n, mut i, mut at) = (0, 0, 0);
    for &r in runs {
        if r.offset < at {
            i = items.partition_point(|t| t.k + t.len <= r.offset);
        }
        while let Some(t) = items.get(i)
            && t.k + t.len <= r.offset
        {
            i += 1;
        }
        let mut j = i;
        while let Some(t) = items.get(j)
            && t.k < r.end()
        {
            write_item(sig, t, bytes);
            j += 1;
        }
        n += (j - i) as u64;
        // An item may span the next run too: start the next search from it.
        i = j.saturating_sub(1).max(i);
        at = r.end();
    }
    n
}

/// The current value of a leaf into its place (`fits` holds).
#[inline(always)]
fn write_item(sig: &Signal, t: &ImageItem, bytes: &mut [u8]) {
    let o = &mut bytes[t.voff as usize..(t.voff + t.vsize) as usize];
    let (k, len) = (t.k as usize, t.len as usize);
    match t.kind {
        ImageKind::Code(bias) => put_le(o, u64::from(sig.cur_code(k).wrapping_sub(bias))),
        ImageKind::Scalar => {
            let b = sig.cur.bytes(k, 1).unwrap_or(&[]);
            let v = match <[u8; 4]>::try_from(b) {
                Ok(x) => i64::from(i32::from_le_bytes(x)) as u64,
                Err(_) => {
                    let mut x = [0u8; 8];
                    let n = b.len().min(8);
                    x[..n].copy_from_slice(&b[..n]);
                    u64::from_le_bytes(x)
                }
            };
            put_le(o, v);
        }
        ImageKind::Planes | ImageKind::Raw => {
            let ok = sig.cur.write_to(k, len, o);
            debug_assert!(ok, "an image leaf of {} that does not fit", sig.name);
        }
    }
}

fn put_le(o: &mut [u8], v: u64) {
    let n = o.len().min(8);
    o[..n].copy_from_slice(&v.to_le_bytes()[..n]);
}
