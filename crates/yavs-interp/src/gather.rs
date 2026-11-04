//! `Inst::SigGather` and `Inst::SigScatter` (#677): a record signal, or an array of records, read
//! into, or driven from, a value in one instruction, instance by instance and leaf by leaf in a
//! Rust loop over the leaves of a plan (`yavs_ir::gather`). Each leaf does what the
//! `SigRead`/`SigReadScalar` and `Drive` of the lowered per-leaf code did, minus the instruction
//! dispatch, the staging buffers and the register traffic. A plan is made from the leaf table and
//! the signal's descriptor the first time and kept by the machine that met it (no lock or shared
//! reference count on the hot path, #677); the runtime keeps one copy for the machines that
//! come later, so a record with bounds from generics costs one comparison of its descriptor.

use std::collections::HashMap;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, RwLock};

use yavs_ir::gather::{self, Leaf, LeafKind, Plan, PlanError};
use yavs_ir::{
    ConstData, ConstId, DriveMode, Func, Part, Program, Reg, Regs, SCATTER_PART, SCATTER_REJECT, Ty,
};
use yavs_kernel::{Ctx, Elem, ImageItem, ImageKind, ImageSpec, NO_IMAGE, Range};

use crate::InstSig;
use crate::exec::{CONST, Machine, R, cell, cold, ice, rt_err, split, to_time};

/// The plans of a runtime: per leaf table, the descriptor size of its value type and the plans
/// made so far with the descriptor bytes they are for. A machine copies the ones it uses
/// ([`MPlan`]); this is where the machines that meet a table and descriptor later find the plan.
#[derive(Default)]
pub(crate) struct Plans(RwLock<HashMap<u32, Known>>);

/// No constant-space descriptor address yet ([`MPlan::at`]): address 0 is frame space.
const NO_ADDR: u64 = u64::MAX;

/// The descriptor size of a table's value type and the plans made for it, by descriptor bytes.
type Known = (u64, Vec<(Box<[u8]>, Arc<Plan>)>);

/// A plan of a machine's own: the table and descriptor size it is for, the descriptor bytes, and
/// the plan with the instance signal of each leaf. `bound` is taken out while an instruction uses
/// it (no lock, no shared reference count; boxed, so taking it moves one pointer). `at` is the
/// constant-space address of a descriptor already found equal to `desc`: constants never change,
/// so a descriptor at that address needs no comparison.
pub(crate) struct MPlan {
    tab: u32,
    ds: u64,
    at: u64,
    desc: Box<[u8]>,
    bound: Option<Box<Bound>>,
}

/// A plan and the instance signal of each of its leaves; for a whole value also the instances of
/// its leaves (`items`) in groups read from one plane word (`reads`) and driven as one run
/// (`runs`), found when the plan is bound.
pub(crate) struct Bound {
    plan: Arc<Plan>,
    sigs: Box<[InstSig]>,
    items: Box<[Item]>,
    reads: Box<[Grp]>,
    runs: Box<[Grp]>,
    /// The first driver slot of the first scatter through the plan and, per leaf, the kernel's
    /// facts of its driver when it is a direct byte driver (`Ctx::direct_ref`): they do not
    /// change. A scatter from other driver slots looks them up each time.
    refs: std::cell::OnceCell<(u32, Box<[Option<yavs_kernel::DirectRef>]>)>,
    img: Img,
    cmp: Cmp,
}

/// Where a bound plan stands with its record image (kernel `image.rs`).
enum Img {
    /// Whole gathers so far.
    Count(u32),
    /// Asked for: there from a later run phase.
    Asked(Box<ImageSpec>),
    /// Copied from image `id`; with `check`, also read leaf by leaf and compared. `copies` since
    /// the last review, when the image had `writes` leaves written; `given`: the copies compiled
    /// code may make (`crate::ImageCell::left`) when last told.
    Ready {
        id: u32,
        check: bool,
        copies: u32,
        writes: u64,
        given: u32,
    },
    Off,
}

/// Where a bound plan stands with comparing its whole zero-delay scatters with the record image
/// (`docs/decisions.md` "Record drive compare").
enum Cmp {
    /// Scatters that could compare so far.
    Count(u32),
    /// Asked for: there from a later run phase.
    Asked(Box<ImageSpec>),
    Ready(Box<CmpReady>),
    Off,
}

/// What a plan's scatters compare with.
struct CmpReady {
    id: u32,
    /// Every skip is held against the kernel's full test (`cmp_check`).
    check: bool,
    /// Scatters since the last review, the leaves they spared, the image's writes at that review.
    seen: u32,
    spared: u64,
    writes: u64,
    /// The plan's distinct kernel drivers.
    drivers: Box<[yavs_kernel::DriverId]>,
    /// Run `r` of `Bound::runs` lies at `ranges[at[r]..at[r + 1]]` of the value, adjacent leaves
    /// joined; a null leaf has no range, and its run is never taken for unchanged.
    ranges: Box<[(u32, u32)]>,
    at: Box<[u32]>,
    /// Its bit of the plan gate: compiled code compares too (`crate::ImageCell`).
    inline: Inline,
}

/// Where a compare stands with compiled code.
#[derive(Clone, Copy, PartialEq)]
enum Inline {
    /// No compiled code looked yet: the kernel has not named the process's drivers.
    Unasked,
    /// No bit to give.
    No,
    /// The bit, and the skips the cell was last given.
    Bit(u64, u32),
}

impl CmpReady {
    /// Whether run `r` of `data` holds what image `im` holds.
    #[inline(always)]
    fn run_same(&self, r: usize, data: &[u8], im: &[u8]) -> bool {
        let rs = &self.ranges[self.at[r] as usize..self.at[r + 1] as usize];
        !rs.is_empty()
            && rs.iter().all(|&(o, n)| {
                let (o, n) = (o as usize, n as usize);
                data[o..o + n] == im[o..o + n]
            })
    }
}

/// Whole gathers of a plan, or whole zero-delay scatters, before it asks for an image: a process
/// that runs a few times does not pay one.
const IMAGE_AFTER: u32 = 16;

/// Copies or scatters between two looks at whether an image pays (`image_review`, `cmp_review`).
const IMAGE_REVIEW: u32 = 1024;

/// A leaf written into an image costs about this many leaves read by a gather (#1065): an image
/// written more than that per copy is given up.
const WRITE_COST: u64 = 4;

/// A leaf written into an image costs about this many leaves a scatter's compare spares: an image
/// written more than that per spared leaf is given up by the scatter.
const CMP_WRITE_COST: u64 = 2;

/// The largest value with an image.
const IMAGE_MAX: u64 = 1 << 16;

/// What the environment says about images: `YAVS_NO_IMAGE=1` none, `YAVS_IMAGE_CHECK=1` every
/// copy checked against the leaves and every skip against the kernel's full test,
/// `YAVS_IMAGE_AFTER=n` asked for after `n` gathers or scatters (off, on, after).
fn image_env() -> (bool, bool, u32) {
    static E: std::sync::OnceLock<(bool, bool, u32)> = std::sync::OnceLock::new();
    *E.get_or_init(|| {
        let on = |k: &str| std::env::var_os(k).is_some_and(|v| v != "0");
        let after = std::env::var("YAVS_IMAGE_AFTER")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(IMAGE_AFTER);
        (on("YAVS_NO_IMAGE"), on("YAVS_IMAGE_CHECK"), after)
    })
}

/// The spec of the image of a whole value.
fn image_spec(b: &Bound) -> Option<ImageSpec> {
    if b.plan.size > IMAGE_MAX {
        return None;
    }
    let items = b
        .items
        .iter()
        .filter(|t| t.vsize > 0 && t.len > 0)
        .map(|t| ImageItem {
            sig: b.sigs[t.leaf as usize].signal,
            k: t.k,
            len: t.len,
            voff: t.voff,
            vsize: t.vsize,
            kind: match t.kind {
                LeafKind::LogicScalar => ImageKind::Code(t.bias),
                LeafKind::LogicVec => ImageKind::Planes,
                LeafKind::Scalar => ImageKind::Scalar,
                LeafKind::Bytes => ImageKind::Raw,
            },
        });
    Some(ImageSpec {
        size: b.plan.size as u32,
        items: items.collect(),
    })
}

/// Whether the image of `b` paid for its last `IMAGE_REVIEW` copies: fewer leaves written than a
/// `WRITE_COST`th of those the copies stood for. If not, this process lets it go.
#[cold]
#[inline(never)]
fn image_review(b: &mut Bound, c: &mut Ctx<'_>) {
    let leaves = b.items.len() as u64;
    let Img::Ready {
        id, copies, writes, ..
    } = &mut b.img
    else {
        return;
    };
    let now = c.image_writes(*id);
    let wrote = now.saturating_sub(*writes);
    if wrote * WRITE_COST > u64::from(*copies) * leaves {
        c.image_release(*id);
        b.img = Img::Off;
        return;
    }
    (*copies, *writes) = (0, now);
}

/// `YAVS_IMAGE_CHECK`: the leaves of a value just read leaf by leaf against its image.
#[cold]
#[inline(never)]
fn image_check(b: &Bound, im: Option<&[u8]>, out: &[u8]) -> R<()> {
    let Some(im) = im else {
        return Err(ice("a record image that is gone"));
    };
    for t in &*b.items {
        let (a, e) = (t.voff as usize, (t.voff + t.vsize) as usize);
        if out[a..e] != im[a..e] {
            return Err(ice(format!(
                "record image out of step: signal {:?} element {} length {}: value {:?}, image {:?}",
                b.sigs[t.leaf as usize].signal,
                t.k,
                t.len,
                &out[a..e],
                &im[a..e]
            )));
        }
    }
    Ok(())
}

/// A whole gather read leaf by leaf: the plan's next step towards its image.
#[cold]
#[inline(never)]
fn image_step(b: &mut Bound, c: &mut Ctx<'_>) {
    match &mut b.img {
        Img::Count(n) => {
            *n += 1;
            if *n < image_env().2 {
                return;
            }
            b.img = match image_spec(b) {
                Some(s) if !image_env().0 => {
                    c.image_ask(s.clone());
                    Img::Asked(s.into())
                }
                _ => Img::Off,
            };
        }
        Img::Asked(s) => {
            b.img = match c.image_id(s) {
                None => return,
                Some(NO_IMAGE) => Img::Off,
                Some(id) => Img::Ready {
                    id,
                    check: image_env().1,
                    copies: 0,
                    writes: c.image_writes(id),
                    given: 0,
                },
            };
        }
        Img::Ready { .. } | Img::Off => {}
    }
}

/// Per run its byte ranges in the value (`CmpReady::ranges`, `at`).
fn cmp_ranges(items: &[Item], runs: &[Grp]) -> (Vec<(u32, u32)>, Vec<u32>) {
    let (mut ranges, mut at) = (Vec::new(), vec![0u32]);
    for g in runs {
        let ts = &items[g.first as usize..][..g.n as usize];
        if ts.iter().all(|t| t.vsize > 0 && t.len > 0) {
            let mut v: Vec<(u32, u32)> = ts.iter().map(|t| (t.voff, t.vsize)).collect();
            v.sort_unstable();
            let first = ranges.len();
            for (o, n) in v {
                match ranges[first..].last_mut() {
                    Some((po, pn)) if *po + *pn == o => *pn += n,
                    _ => ranges.push((o, n)),
                }
            }
        }
        at.push(ranges.len() as u32);
    }
    (ranges, at)
}

/// A whole zero-delay scatter that does not compare yet: the plan's next step towards its image.
/// Every leaf must go through a driver of this process that a skip is right for.
#[cold]
#[inline(never)]
fn cmp_step(cmp: Cmp, b: &Bound, c: &mut Ctx<'_>, ds: &[crate::ProcDriver]) -> Cmp {
    match cmp {
        Cmp::Count(n) if n + 1 < image_env().2 => Cmp::Count(n + 1),
        Cmp::Count(_) => {
            let leaves = |t: &&Item| t.vsize > 0 && t.len > 0;
            let ok = !image_env().0
                && b.items.iter().filter(leaves).all(|t| {
                    let (id, s) = (ds[t.leaf as usize].id, b.sigs[t.leaf as usize].signal);
                    c.drive_skip_ok(id, s, Range::new(t.k, t.len))
                });
            match image_spec(b) {
                Some(s) if ok => {
                    c.image_ask(s.clone());
                    Cmp::Asked(s.into())
                }
                _ => Cmp::Off,
            }
        }
        Cmp::Asked(s) => match c.image_id(&s) {
            None => Cmp::Asked(s),
            Some(NO_IMAGE) => Cmp::Off,
            Some(id) => {
                let mut drivers: Vec<_> = ds.iter().map(|d| d.id).collect();
                drivers.sort_unstable_by_key(|d| d.0);
                drivers.dedup();
                let (ranges, at) = cmp_ranges(&b.items, &b.runs);
                Cmp::Ready(Box::new(CmpReady {
                    id,
                    check: cfg!(debug_assertions) || image_env().1,
                    seen: 0,
                    spared: 0,
                    writes: c.image_writes(id),
                    drivers: drivers.into(),
                    ranges: ranges.into(),
                    at: at.into(),
                    inline: Inline::Unasked,
                }))
            }
        },
        other => other,
    }
}

/// Whether the image of a plan paid for its last `IMAGE_REVIEW` scatters: its writes cost no more
/// than the leaves spared. If not, this scatter lets it go.
#[cold]
#[inline(never)]
fn cmp_review(cmp: &mut Cmp, c: &mut Ctx<'_>) {
    let Cmp::Ready(r) = cmp else {
        return;
    };
    let now = c.image_writes(r.id);
    if now.saturating_sub(r.writes) * CMP_WRITE_COST > r.spared {
        c.image_release(r.id);
        *cmp = Cmp::Off;
        return;
    }
    (r.seen, r.spared, r.writes) = (0, 0, now);
}

/// Debug builds and `YAVS_IMAGE_CHECK`: the runs a compare would skip against the kernel's full
/// test, before the scatter makes its first entry, since the test refuses a driver with one.
#[cold]
#[inline(never)]
fn cmp_check(
    b: &Bound,
    r: &CmpReady,
    c: &Ctx<'_>,
    ds: &[crate::ProcDriver],
    data: &[u8],
    im: &[u8],
) -> R<()> {
    for (ri, g) in b.runs.iter().enumerate() {
        if !r.run_same(ri, data, im) {
            continue;
        }
        for t in &b.items[g.first as usize..][..g.n as usize] {
            let v = &data[t.voff as usize..(t.voff + t.vsize) as usize];
            let l = &b.plan.leaves[t.leaf as usize];
            let d = ds[t.leaf as usize].id;
            let res = match t.kind {
                LeafKind::LogicVec if t.len > 64 => (v.as_chunks::<32>().0.iter().enumerate())
                    .try_for_each(|(i, w)| {
                        let p = [0, 1, 2, 3].map(|k| {
                            u64::from_le_bytes(w[k * 8..k * 8 + 8].try_into().unwrap_or_default())
                        });
                        let at = 64 * i as u32;
                        c.gate_check(d, t.k + at, (t.len - at).min(64), p)
                    }),
                LeafKind::LogicScalar | LeafKind::LogicVec => {
                    c.gate_check(d, t.k, t.len, leaf_planes(t.kind, t.len, t.bias, v))
                }
                LeafKind::Scalar | LeafKind::Bytes => (0..t.len).try_for_each(|e| {
                    let mut w = [0u8; 8];
                    let es = l.ew as usize;
                    w[..es].copy_from_slice(&v[e as usize * es..][..es]);
                    c.gate_check(d, t.k + e, 1, [u64::from_le_bytes(w), 0, 0, 0])
                }),
            };
            res.map_err(|e| {
                ice(format!(
                    "a record drive skipped by its image: signal {:?} element {} length {}: {e}",
                    b.sigs[t.leaf as usize].signal, t.k, t.len
                ))
            })?;
        }
    }
    Ok(())
}

/// One instance of a leaf in a whole value: the leaf, its byte offset in the value and its first
/// kernel element. A logic leaf's instances are listed last first, which is their order in the
/// kernel (`Leaf::kstart`), so they join one run.
#[derive(Clone, Copy)]
struct Item {
    leaf: u32,
    voff: u32,
    k: u32,
    /// The leaf's `len`, `vsize`, `kind` and `bias`, at hand.
    len: u32,
    vsize: u32,
    kind: LeafKind,
    bias: u8,
}

/// The most instances of leaves a value may have to get `Bound::items`; larger ones go leaf by
/// leaf.
const MAX_ITEMS: usize = 4096;

/// Items `first..first + n` of a bound plan. In `reads`: logic leaves of one signal inside plane
/// word `word` of it (`NO_WORD`: one leaf read on its own). In `runs`: logic leaves of one signal
/// next to each other, at most 64 elements (`word` the element count; `NO_WORD`: one leaf driven
/// on its own). Kept in registers while a group is read or driven: no per-leaf signal lookup and
/// no plane block built in memory (#790).
#[derive(Clone, Copy)]
struct Grp {
    first: u32,
    n: u32,
    word: u32,
    sig: yavs_kernel::SignalId,
}

const NO_WORD: u32 = u32::MAX;

/// Whether leaf `l` of signal `s` is a logic leaf of 1 to 64 elements (`s.ty` is the leaf's own
/// type; `s.off` where it starts in its kernel signal).
fn logic64(l: &Leaf, s: &InstSig) -> bool {
    l.logic()
        && (1..=64).contains(&l.len)
        && l.len <= s.ty.len
        && s.off.checked_add(l.len).is_some()
}

/// Items, `reads` and `runs` of a `Bound`.
type Grouped = (Box<[Item]>, Box<[Grp]>, Box<[Grp]>);

/// The items of a whole value and their `reads` and `runs` groups (`Bound`); none for a value of
/// more than `MAX_ITEMS` leaf instances.
fn groups(plan: &Plan, sigs: &[InstSig]) -> Grouped {
    let total: u64 = plan.leaves.iter().map(|l| u64::from(l.insts)).sum();
    if total as usize > MAX_ITEMS {
        return (Box::new([]), Box::new([]), Box::new([]));
    }
    let mut items = Vec::with_capacity(total as usize);
    for (i, (l, s)) in plan.leaves.iter().zip(sigs).enumerate() {
        let n = l.insts as usize;
        for x in 0..n {
            let j = if l.logic() { n - 1 - x } else { x };
            let k = u64::from(s.off) + l.kstart(j, 0, plan.count);
            let (Ok(k), Ok(voff)) = (u32::try_from(k), u32::try_from(l.off(j))) else {
                return (Box::new([]), Box::new([]), Box::new([]));
            };
            items.push(Item {
                leaf: i as u32,
                voff,
                k,
                len: l.len,
                vsize: l.vsize,
                kind: l.kind,
                bias: l.bias,
            });
        }
    }
    let (mut reads, mut runs) = (Vec::<Grp>::new(), Vec::<Grp>::new());
    for (i, t) in items.iter().enumerate() {
        let i = i as u32;
        let (l, s) = (&plan.leaves[t.leaf as usize], &sigs[t.leaf as usize]);
        let ok = logic64(l, s) && t.k.checked_add(l.len).is_some();
        let (w, w_end) = (t.k / 64, t.k.saturating_add(l.len.max(1) - 1) / 64);
        match reads.last_mut() {
            Some(g) if ok && w == w_end && g.word == w && g.sig == s.signal => g.n += 1,
            _ => reads.push(Grp {
                first: i,
                n: 1,
                word: if ok && w == w_end { w } else { NO_WORD },
                sig: s.signal,
            }),
        }
        match runs.last_mut() {
            Some(g)
                if ok && g.word != NO_WORD && g.word + l.len <= 64 && {
                    g.sig == s.signal && items[g.first as usize].k + g.word == t.k
                } =>
            {
                g.n += 1;
                g.word += l.len;
            }
            _ => runs.push(Grp {
                first: i,
                n: 1,
                word: if ok { l.len } else { NO_WORD },
                sig: s.signal,
            }),
        }
    }
    (items.into(), reads.into(), runs.into())
}

/// The planes of a logic leaf of `kind`, `len` (1 to 64) elements and `bias` whose value is `v`,
/// in the low bits.
#[inline(always)]
fn leaf_planes(kind: LeafKind, len: u32, bias: u8, v: &[u8]) -> [u64; 4] {
    match kind {
        LeafKind::LogicScalar => {
            let code = v.first().map_or(0, |b| b.wrapping_add(bias));
            std::array::from_fn(|i| u64::from(code >> i & 1))
        }
        _ => {
            let w = v.as_chunks::<8>().0;
            let mask = if len == 64 { u64::MAX } else { (1 << len) - 1 };
            [0, 1, 2, 3].map(|i| w.get(i).map_or(0, |c| u64::from_le_bytes(*c)) & mask)
        }
    }
}

/// The bytes of leaf table `tab`.
fn table(prog: &Program, tab: ConstId) -> R<&[u8]> {
    match prog.consts.get(tab) {
        Some(ConstData::Bytes(b)) => Ok(b),
        _ => Err(cold(|| ice("a leaf table that is not a byte constant"))),
    }
}

/// The low `o.len()` bytes of `v` into `o` (no `memcpy` call for the usual sizes).
#[inline(always)]
fn put_le(o: &mut [u8], v: u64) {
    match o.len() {
        1 => o[0] = v as u8,
        4 => o.copy_from_slice(&(v as u32).to_le_bytes()),
        8 => o.copy_from_slice(&v.to_le_bytes()),
        n => {
            let n = n.min(8);
            o[..n].copy_from_slice(&v.to_le_bytes()[..n]);
        }
    }
}

/// The part of the signal a gather or scatter works on: the first element and the number of
/// elements of the outermost array of the whole signal (the value's own count, for a whole one).
fn part_of(m: &Machine, plan: &Plan, part: Option<Part>) -> R<(u64, u64)> {
    let (first, outer) = match part {
        Some(q) => (m.r(q.first), m.r(q.outer)),
        None => (0, plan.count),
    };
    if first.checked_add(plan.count).is_none_or(|e| e > outer) {
        return Err(cold(|| {
            ice(format!(
                "elements {first} + {} of a record signal of {outer}",
                plan.count
            ))
        }));
    }
    Ok((first, outer))
}

/// Instance of leaf `l` of signal `s` at element `k`, read into its place `o` in the value.
#[inline(always)]
fn read_leaf(c: &Ctx<'_>, s: &InstSig, l: &Leaf, k: u32, o: &mut [u8]) -> R<()> {
    match l.kind {
        LeafKind::LogicScalar => {
            let v = u64::from(c.elem_code(s.signal, k)?.wrapping_sub(l.bias));
            put_le(o, v);
        }
        LeafKind::Scalar => {
            let v = match c.elem_bytes(s.signal, k) {
                Some(b) => cell(b),
                None => cell(c.value(s.signal.slice(k, 1))?),
            };
            put_le(o, v);
        }
        LeafKind::LogicVec => {
            // A vector inside one plane block (every vector leaf of a record pack, #686): its
            // bits straight from the signal's planes.
            if l.len > 0
                && let Some((planes, n)) = c.planes(s.signal)
                && k.checked_add(l.len).is_some_and(|e| e <= n)
                && (k % 64) + l.len <= 64
            {
                let (blk, at) = (&planes[(k / 64) as usize], k % 64);
                let mask = if l.len == 64 {
                    u64::MAX
                } else {
                    (1 << l.len) - 1
                };
                for (i, w) in o.as_chunks_mut::<8>().0.iter_mut().take(4).enumerate() {
                    *w = (blk[i] >> at & mask).to_le_bytes();
                }
            } else {
                c.read_into(s.signal.slice(k, l.len), o)?;
            }
        }
        LeafKind::Bytes => c.read_into(s.signal.slice(k, l.len), o)?,
    }
    Ok(())
}

/// Logic leaves next to each other in one kernel driver (a pack, #686) collected into one plane
/// block: the driver, the elements and the planes so far.
struct Run {
    id: yavs_kernel::DriverId,
    r: Range,
    p: [u64; 4],
}

/// The delay of a scatter, whether it is none (`now`), and the delay mode.
type When = (yavs_kernel::Time, bool, yavs_kernel::DelayMode);

/// Assign the run `run` makes, if there is one.
#[inline(always)]
fn flush_run(c: &mut Ctx<'_>, run: &mut Option<Run>, when: When) -> R<()> {
    let Some(Run { id, r, p }) = run.take() else {
        return Ok(());
    };
    flush_planes(c, id, r, p, when)
}

/// Assign planes `p` to elements `r` (at most 64) of driver `id`.
#[inline(always)]
fn flush_planes(
    c: &mut Ctx<'_>,
    id: yavs_kernel::DriverId,
    r: Range,
    p: [u64; 4],
    (t, now, mode): When,
) -> R<()> {
    if r.len == 1 {
        let code = (0..4).fold(0u8, |a, i| a | ((p[i] & 1) as u8) << i);
        if now && c.assign_code_now(id, r, code) {
            return Ok(());
        }
        c.assign_code(id, Some(r), t, code, mode)?;
        return Ok(());
    }
    if now && c.assign_planes_now(id, Some(r), p) {
        return Ok(());
    }
    let mut b = [0u8; 32];
    for (k, w) in p.iter().enumerate() {
        b[k * 8..k * 8 + 8].copy_from_slice(&w.to_le_bytes());
    }
    c.assign_blocks(id, Some(r), t, &b, mode)?;
    Ok(())
}

/// Instance of leaf `l`, whose value is `v`, driven onto elements `r` of its signal through driver
/// `d` after `when`; a logic leaf joins the run of the leaves before it when it follows them in
/// the same driver.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn write_leaf(
    c: &mut Ctx<'_>,
    d: yavs_kernel::DriverId,
    l: &Leaf,
    r: Range,
    v: &[u8],
    run: &mut Option<Run>,
    when: When,
    dref: Option<&yavs_kernel::DirectRef>,
) -> R<()> {
    let (t, now, mode) = when;
    if matches!(l.kind, LeafKind::LogicScalar | LeafKind::LogicVec) && (1..=64).contains(&l.len) {
        let joins = run.as_ref().is_some_and(|q| {
            q.id == d && q.r.offset + q.r.len == r.offset && q.r.len + r.len <= 64
        });
        if !joins {
            flush_run(c, run, when)?;
            *run = Some(Run {
                id: d,
                r: Range::new(r.offset, 0),
                p: [0; 4],
            });
        }
        let Some(q) = run.as_mut() else {
            return Err(cold(|| ice("a run that was just made")));
        };
        let at = q.r.len;
        let bits = leaf_planes(l.kind, l.len, l.bias, v);
        for (p, b) in q.p.iter_mut().zip(bits) {
            *p |= b << at;
        }
        q.r.len += r.len;
        return Ok(());
    }
    flush_run(c, run, when)?;
    match l.kind {
        LeafKind::LogicScalar | LeafKind::LogicVec => {
            c.assign_blocks(d, Some(r), t, v, mode)?;
        }
        LeafKind::Scalar | LeafKind::Bytes => {
            let v = v.get(..l.len as usize * l.ew as usize).unwrap_or(v);
            if !(now
                && l.len == 1
                && (dref.is_some_and(|x| c.assign_ref_bytes(x, r.offset, v))
                    || c.assign_bytes_now(d, r, v)))
            {
                c.assign_bytes(d, Some(r), t, v, mode)?;
            }
        }
    }
    Ok(())
}

/// Run `g` of a whole value `data`, driven through driver slots `ds`.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn scatter_run(
    c: &mut Ctx<'_>,
    plan: &Plan,
    items: &[Item],
    g: &Grp,
    ds: &[crate::ProcDriver],
    data: &[u8],
    refs: &[Option<yavs_kernel::DirectRef>],
    run: &mut Option<Run>,
    when: When,
) -> R<()> {
    let ts = &items[g.first as usize..][..g.n as usize];
    let id = ds[ts[0].leaf as usize].id;
    // One run of one driver: its planes built in registers, one assignment.
    if g.word != NO_WORD && ts.iter().all(|t| ds[t.leaf as usize].id == id) {
        flush_run(c, run, when)?;
        let (mut p, mut at) = ([0u64; 4], 0);
        for t in ts {
            let o = t.voff as usize;
            let v = data
                .get(o..o + t.vsize as usize)
                .ok_or_else(|| cold(|| ice("a leaf outside its value")))?;
            for (p, b) in p.iter_mut().zip(leaf_planes(t.kind, t.len, t.bias, v)) {
                *p |= b << at;
            }
            at += t.len;
        }
        return flush_planes(c, id, Range::new(ts[0].k, at), p, when);
    }
    for t in ts {
        let l = &plan.leaves[t.leaf as usize];
        let at = t.voff as usize;
        let v = data
            .get(at..at + l.vsize as usize)
            .ok_or_else(|| cold(|| ice("a leaf outside its value")))?;
        let d = ds[t.leaf as usize].id;
        let dref = refs.get(t.leaf as usize).and_then(Option::as_ref);
        write_leaf(c, d, l, Range::new(t.k, l.len), v, run, when, dref)?;
    }
    Ok(())
}

/// A whole zero-delay scatter against its image `im`: the leaves spared. Out of line, so that a
/// scatter that does not compare keeps its shape: inlined, this cost `rec_pair` about 1 percent.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn scatter_compared(
    b: &Bound,
    r: &CmpReady,
    im: &[u8],
    c: &mut Ctx<'_>,
    ds: &[crate::ProcDriver],
    data: &[u8],
    refs: &[Option<yavs_kernel::DirectRef>],
    when: When,
) -> R<u32> {
    if r.check {
        cmp_check(b, r, c, ds, data, im)?;
    }
    if *data == *im {
        c.gate_skipped();
        return Ok(b.items.len() as u32);
    }
    let (mut spared, mut run) = (0, None);
    for (ri, g) in b.runs.iter().enumerate() {
        if r.run_same(ri, data, im) {
            c.gate_skipped();
            spared += g.n;
        } else {
            scatter_run(c, &b.plan, &b.items, g, ds, data, refs, &mut run, when)?;
        }
    }
    flush_run(c, &mut run, when)?;
    Ok(spared)
}

impl Machine {
    /// The cell at `k`, once compiled code looked at this process (the kernel knows of it then).
    fn cell(&mut self, k: usize, c: &Ctx<'_>) -> Option<&mut crate::ImageCell> {
        if self.icells.is_empty() && !c.gate_unset() {
            let n = crate::GATHER_CELLS + self.drivers.len();
            self.icells = (0..n).map(|_| crate::ImageCell::empty()).collect();
            self.icells_at = self.icells.as_ptr() as usize;
        }
        self.icells.get_mut(k)
    }

    /// What compiled code skipped through the cell of a scatter since the host last looked, in
    /// the counts of the review.
    #[cold]
    #[inline(never)]
    fn scat_take(&mut self, b: &mut Bound, tab: ConstId, driver: u32, c: &mut Ctx<'_>) {
        let Cmp::Ready(r) = &mut b.cmp else {
            return;
        };
        let Inline::Bit(_, ref mut given) = r.inline else {
            return;
        };
        let k = crate::GATHER_CELLS + driver as usize;
        let Some(cell) = self.icells.get_mut(k).filter(|x| x.tab == tab.0) else {
            return;
        };
        let n = u64::from(given.saturating_sub(*cell.left.get_mut()));
        *given = *cell.left.get_mut();
        let leaves = n * b.items.len() as u64;
        r.seen += n as u32;
        r.spared += leaves;
        c.note_spared(leaves as u32);
    }

    /// The cell of a scatter through driver slot `driver` for what plan `b` stands at now: no
    /// image while the plan does not compare, so an image let go is not read again.
    #[cold]
    #[inline(never)]
    fn scat_put(
        &mut self,
        b: &mut Bound,
        at: usize,
        tab: ConstId,
        desc: u64,
        driver: u32,
        c: &mut Ctx<'_>,
    ) {
        let k = crate::GATHER_CELLS + driver as usize;
        let keep = self.plans[at].ds == 0 || self.plans[at].at == desc;
        let Cmp::Ready(r) = &mut b.cmp else {
            if let Some(cell) = self.icells.get_mut(k).filter(|x| x.tab == tab.0) {
                cell.image = 0;
            }
            return;
        };
        // (The host compares only through the slots of its first scatter: `Bound::refs`.)
        if !keep {
            r.inline = Inline::No;
            return;
        }
        if c.gate_unset() || b.refs.get().is_none_or(|(base, _)| *base != driver) {
            return;
        }
        if r.inline == Inline::Unasked {
            r.inline = match c.plan_gate(&r.drivers) {
                0 => Inline::No,
                bit => Inline::Bit(bit, 0),
            };
        }
        let Inline::Bit(bit, ref mut given) = r.inline else {
            return;
        };
        let left = IMAGE_REVIEW.saturating_sub(r.seen);
        *given = left;
        let (id, size) = (r.id, b.plan.size as u32);
        let image = c.image(id).map_or(0, |i| i.as_ptr() as u64);
        let Some(cell) = self.cell(k, c) else {
            return;
        };
        *cell.left.get_mut() = left;
        cell.mask = bit;
        cell.tab = tab.0;
        cell.desc = desc;
        cell.size = size;
        cell.image = image;
    }

    /// What compiled code copied through the cell of a gather since the host last looked, in the
    /// counts of the review.
    #[cold]
    #[inline(never)]
    fn gath_take(&mut self, b: &mut Bound, tab: ConstId, c: &mut Ctx<'_>) {
        let Img::Ready { copies, given, .. } = &mut b.img else {
            return;
        };
        let k = tab.0 as usize % crate::GATHER_CELLS;
        let Some(cell) = self.icells.get_mut(k).filter(|x| x.tab == tab.0) else {
            return;
        };
        let n = given.saturating_sub(*cell.left.get_mut());
        *given = *cell.left.get_mut();
        *copies += n;
        c.note_gathers(u64::from(n));
        if *copies >= IMAGE_REVIEW {
            image_review(b, c);
        }
    }

    /// The cell of a gather for what plan `b` stands at now (`scat_put`).
    #[cold]
    #[inline(never)]
    fn gath_put(&mut self, b: &mut Bound, at: usize, tab: ConstId, desc: u64, c: &mut Ctx<'_>) {
        let k = tab.0 as usize % crate::GATHER_CELLS;
        let keep = self.plans[at].ds == 0 || self.plans[at].at == desc;
        let size = b.plan.size as u32;
        // Another table's while it lasts: only this one's gathers go through the host.
        if self
            .icells
            .get(k)
            .is_some_and(|x| x.image != 0 && x.tab != tab.0)
        {
            return;
        }
        let Img::Ready {
            id,
            copies,
            ref mut given,
            ..
        } = b.img
        else {
            if let Some(cell) = self.icells.get_mut(k).filter(|x| x.tab == tab.0) {
                cell.image = 0;
            }
            return;
        };
        if !keep || c.gate_unset() {
            return;
        }
        let left = IMAGE_REVIEW.saturating_sub(copies);
        *given = left;
        let image = c.image(id).map_or(0, |i| i.as_ptr() as u64);
        let Some(cell) = self.cell(k, c) else {
            return;
        };
        *cell.left.get_mut() = left;
        cell.tab = tab.0;
        cell.desc = desc;
        cell.size = size;
        cell.image = image;
    }

    /// `Process::images_tail`.
    pub(crate) fn images_tail(&self) -> (u64, u64) {
        let (mut copied, mut spared) = (0, 0);
        let left = |k: usize, tab: u32| {
            let c = self.icells.get(k).filter(|c| c.tab == tab)?;
            Some(c.left.load(Relaxed))
        };
        for m in &self.plans {
            let Some(b) = &m.bound else {
                continue;
            };
            if let Img::Ready { given, .. } = b.img
                && let Some(l) = left(m.tab as usize % crate::GATHER_CELLS, m.tab)
            {
                copied += u64::from(given.saturating_sub(l));
            }
            if let Cmp::Ready(r) = &b.cmp
                && let Inline::Bit(_, given) = r.inline
                && let Some((base, _)) = b.refs.get()
                && let Some(l) = left(crate::GATHER_CELLS + *base as usize, m.tab)
            {
                spared += u64::from(given.saturating_sub(l)) * b.items.len() as u64;
            }
        }
        (copied, spared)
    }

    /// `YAVS_IMAGE_CHECK`, debug builds: compiled code found the value of the scatter through
    /// `driver` equal to its image and the plan gate open: the kernel's full test, and what the
    /// slow path would have said.
    pub(crate) fn scatter_check(
        &mut self,
        prog: &Program,
        driver: u32,
        src: Reg,
        desc: Reg,
        tab: ConstId,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<()> {
        let (at, b) = self.take_plan(prog, tab, self.r(desc))?;
        let r = self.scatter_check_bound(&b, driver, src, ctx);
        self.put_plan(at, b);
        r
    }

    fn scatter_check_bound(
        &self,
        b: &Bound,
        driver: u32,
        src: Reg,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<()> {
        let Some(c) = ctx.as_deref_mut() else {
            return Err(ice("a compiled record skip without a kernel"));
        };
        let Cmp::Ready(r) = &b.cmp else {
            return Err(ice(
                "a compiled record skip of a plan that does not compare",
            ));
        };
        let data = self.rd(self.r(src), b.plan.size)?;
        let ds = self
            .drivers
            .get(driver as usize..driver as usize + b.plan.leaves.len())
            .ok_or_else(|| ice(format!("no driver slots from {driver}")))?;
        let Some(im) = c.image(r.id) else {
            return Err(ice("a compiled record skip with no image"));
        };
        if data != im {
            return Err(ice(
                "a compiled record skip of a value its image does not hold",
            ));
        }
        if !c.drive_clean(&r.drivers) {
            return Err(ice(
                "a compiled record skip with drivers that are not clean",
            ));
        }
        let k = crate::GATHER_CELLS + driver as usize;
        if self.icells.get(k).map(|x| x.image) != Some(im.as_ptr() as u64) {
            return Err(ice(
                "a compiled record skip through a cell that names no image",
            ));
        }
        cmp_check(b, r, c, ds, data, im)
    }

    /// `YAVS_IMAGE_CHECK`, debug builds: compiled code copied the image into the value at `dst`:
    /// the leaves as the kernel has them against it.
    pub(crate) fn gather_check(
        &mut self,
        prog: &Program,
        dst: Reg,
        desc: Reg,
        tab: ConstId,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<()> {
        let (at, b) = self.take_plan(prog, tab, self.r(desc))?;
        let r = self.gather_check_bound(&b, dst, ctx);
        self.put_plan(at, b);
        r
    }

    fn gather_check_bound(&self, b: &Bound, dst: Reg, ctx: &mut Option<&mut Ctx<'_>>) -> R<()> {
        let Some(c) = ctx.as_deref_mut() else {
            return Err(ice("a compiled record copy without a kernel"));
        };
        let Img::Ready { id, .. } = b.img else {
            return Err(ice("a compiled record copy of a plan with no image"));
        };
        let got = self.rd(self.r(dst), b.plan.size)?;
        if c.image(id) != Some(got) {
            return Err(ice("a compiled record copy that is not its image"));
        }
        let mut leaves = vec![0u8; got.len()];
        for t in &*b.items {
            let (l, s) = (&b.plan.leaves[t.leaf as usize], &b.sigs[t.leaf as usize]);
            let o = &mut leaves[t.voff as usize..(t.voff + t.vsize) as usize];
            read_leaf(c, s, l, t.k, o)?;
        }
        image_check(b, Some(got), &leaves)
    }

    /// The plan of table `tab` for the value whose descriptor is at `desc`, out of this machine's
    /// list; give it back with [`Machine::put_plan`].
    #[inline(always)]
    fn take_plan(&mut self, prog: &Program, tab: ConstId, desc: u64) -> R<(usize, Box<Bound>)> {
        for i in 0..self.plans.len() {
            let m = &self.plans[i];
            if m.tab != tab.0 || m.bound.is_none() {
                continue;
            }
            let hit = m.ds == 0 || m.at == desc || {
                let same = *self.rd(desc, m.ds)? == *m.desc;
                if same && matches!(split(desc).0, CONST | crate::exec::INST) {
                    self.plans[i].at = desc;
                }
                same
            };
            if hit && let Some(b) = self.plans[i].bound.take() {
                return Ok((i, b));
            }
        }
        self.new_bound(prog, tab, desc)
    }

    /// Give back what [`Machine::take_plan`] took.
    #[inline(always)]
    fn put_plan(&mut self, at: usize, b: Box<Bound>) {
        if let Some(m) = self.plans.get_mut(at) {
            m.bound = Some(b);
        }
    }

    /// [`Machine::take_plan`] for a table and descriptor this machine has not met (or lost the plan
    /// of to an error): the plan from the runtime, bound to this machine's signals.
    #[cold]
    #[inline(never)]
    fn new_bound(&mut self, prog: &Program, tab: ConstId, desc: u64) -> R<(usize, Box<Bound>)> {
        let (plan, ds) = self.leaf_plan(prog, tab, desc)?;
        let d = self.rd(desc, ds)?.to_vec();
        let mut sigs = Vec::with_capacity(plan.leaves.len());
        for l in &plan.leaves {
            let s = *self
                .inst
                .sigs
                .get(l.sig as usize)
                .ok_or_else(|| ice(format!("no instance signal {}", l.sig)))?;
            let ok = match l.kind {
                LeafKind::LogicScalar | LeafKind::LogicVec => s.ty.elem == Elem::Logic,
                LeafKind::Scalar => matches!(s.ty.elem, Elem::Bytes(es) if es <= 8),
                LeafKind::Bytes => matches!(s.ty.elem, Elem::Bytes(_)),
            };
            if !ok {
                return Err(ice("a leaf table that does not match its signal"));
            }
            sigs.push(s);
        }
        let at = match self
            .plans
            .iter()
            .position(|m| m.tab == tab.0 && *m.desc == *d)
        {
            Some(at) => at,
            None => {
                self.plans.push(MPlan {
                    tab: tab.0,
                    ds,
                    at: NO_ADDR,
                    desc: d.into(),
                    bound: None,
                });
                self.plans.len() - 1
            }
        };
        Ok((at, {
            let (items, reads, runs) = groups(&plan, &sigs);
            Box::new(Bound {
                plan,
                sigs: sigs.into(),
                items,
                reads,
                runs,
                refs: std::cell::OnceCell::new(),
                img: Img::Count(0),
                cmp: Cmp::Count(0),
            })
        }))
    }

    /// The runtime's plan of table `tab` for the value whose descriptor is at `desc`, and the
    /// descriptor size of the table's type.
    #[inline(never)]
    fn leaf_plan(&self, prog: &Program, tab: ConstId, desc: u64) -> R<(Arc<Plan>, u64)> {
        let key = tab.0;
        if let Ok(m) = self.rt.plans.0.read()
            && let Some((ds, v)) = m.get(&key)
        {
            let d = self.rd(desc, *ds)?;
            if let Some((_, p)) = v.iter().find(|(k, _)| **k == *d) {
                return Ok((Arc::clone(p), *ds));
            }
        }
        self.new_leaf_plan(prog, tab, desc)
    }

    /// [`Machine::leaf_plan`] for a table and descriptor not seen before.
    #[cold]
    #[inline(never)]
    fn new_leaf_plan(&self, prog: &Program, tab: ConstId, desc: u64) -> R<(Arc<Plan>, u64)> {
        let bytes = table(prog, tab)?;
        let Some((ty, _)) = gather::entries(bytes) else {
            return Err(cold(|| ice("a leaf table that is not one")));
        };
        let ds = Ty::desc_size(&prog.types, ty)
            .ok_or_else(|| cold(|| ice("a leaf table of a type without a descriptor size")))?;
        let d = self.rd(desc, ds)?;
        let plan = match gather::plan(&prog.types, &prog.consts, bytes, d) {
            Ok(p) => Arc::new(p),
            Err(PlanError::Layout(yavs_ir::LayoutError::TooLarge)) => {
                return cold(|| rt_err("object size exceeds 64 bits"));
            }
            Err(e) => return Err(cold(|| ice(format!("leaf table: {e:?}")))),
        };
        if let Ok(mut m) = self.rt.plans.0.write() {
            m.entry(tab.0)
                .or_insert_with(|| (ds, Vec::new()))
                .1
                .push((d.into(), Arc::clone(&plan)));
        }
        Ok((plan, ds))
    }

    /// `Inst::SigGather`.
    #[inline(never)]
    pub(crate) fn sig_gather(
        &mut self,
        prog: &Program,
        dst: Reg,
        desc: Reg,
        tab: ConstId,
        part: Option<Part>,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<()> {
        let (at, mut b) = self.take_plan(prog, tab, self.r(desc))?;
        // The cell: what compiled code copied goes into the counts before the review, and it sees
        // what this gather made of the plan.
        let was = matches!(b.img, Img::Ready { .. });
        if let Img::Ready { given, .. } = b.img
            && given != 0
            && part.is_none()
            && (self.icells.get(tab.0 as usize % crate::GATHER_CELLS))
                .is_some_and(|x| x.tab == tab.0 && x.left.load(Relaxed) != given)
            && let Some(c) = ctx.as_deref_mut()
        {
            self.gath_take(&mut b, tab, c);
        }
        // The plan goes back on both arms: an `R` kept across `put_plan` is reloaded through
        // memory, a stall of its own.
        match self.gather_leaves(&mut b, dst, part, ctx) {
            Ok(()) => {
                let put = match b.img {
                    Img::Ready { given, .. } => given == 0,
                    _ => was,
                };
                if put
                    && part.is_none()
                    && let Some(c) = ctx.as_deref_mut()
                    && (was && !matches!(b.img, Img::Ready { .. }) || !c.gate_unset())
                {
                    self.gath_put(&mut b, at, tab, self.r(desc), c);
                }
                self.put_plan(at, b);
                Ok(())
            }
            Err(e) => {
                self.put_plan(at, b);
                Err(e)
            }
        }
    }

    fn gather_leaves(
        &mut self,
        b: &mut Bound,
        dst: Reg,
        part: Option<Part>,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<()> {
        let plan = &*b.plan;
        let (first, outer) = part_of(self, plan, part)?;
        let Some(c) = ctx.as_deref_mut() else {
            return cold(|| rt_err("signal or wait operation during elaboration"));
        };
        let base = self.r(dst);
        let out = self.wr(base, plan.size)?;
        if part.is_none() && !b.items.is_empty() {
            if let Img::Ready {
                id,
                check: false,
                ref mut copies,
                ..
            } = b.img
                && let Some(im) = c.image(id)
            {
                out.copy_from_slice(im);
                c.note_gather(true);
                *copies += 1;
                if *copies >= IMAGE_REVIEW {
                    image_review(b, c);
                }
                return Ok(());
            }
            c.note_gather(false);
            for g in &*b.reads {
                let ts = &b.items[g.first as usize..][..g.n as usize];
                // One plane word of the signal for all items of the group.
                let blk = match g.word {
                    NO_WORD => None,
                    w => c
                        .planes(g.sig)
                        .and_then(|(p, _)| p.get(w as usize).copied()),
                };
                for t in ts {
                    let at = t.voff as usize;
                    let o = out
                        .get_mut(at..at + t.vsize as usize)
                        .ok_or_else(|| cold(|| ice("a leaf outside its value")))?;
                    let Some(blk) = blk else {
                        let (l, s) = (&plan.leaves[t.leaf as usize], &b.sigs[t.leaf as usize]);
                        read_leaf(c, s, l, t.k, o)?;
                        continue;
                    };
                    let sh = t.k % 64;
                    if t.kind == LeafKind::LogicScalar {
                        let code = (0..4).fold(0u8, |a, i| a | ((blk[i] >> sh & 1) as u8) << i);
                        put_le(o, u64::from(code.wrapping_sub(t.bias)));
                    } else {
                        let mask = if t.len == 64 {
                            u64::MAX
                        } else {
                            (1 << t.len) - 1
                        };
                        for (i, w) in o.as_chunks_mut::<8>().0.iter_mut().take(4).enumerate() {
                            *w = (blk[i] >> sh & mask).to_le_bytes();
                        }
                    }
                }
            }
            if let Img::Ready {
                id, check: true, ..
            } = b.img
            {
                image_check(b, c.image(id), out)?;
            }
            if !matches!(b.img, Img::Ready { .. } | Img::Off) {
                image_step(b, c);
            }
            return Ok(());
        }
        for (l, s) in plan.leaves.iter().zip(&*b.sigs) {
            for j in 0..l.insts as usize {
                let at = l.off(j);
                let o = out
                    .get_mut(at..at + l.vsize as usize)
                    .ok_or_else(|| cold(|| ice("a leaf outside its value")))?;
                read_leaf(c, s, l, s.off + l.kstart(j, first, outer) as u32, o)?;
            }
        }
        Ok(())
    }

    /// `Inst::SigScatter`.
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    pub(crate) fn sig_scatter(
        &mut self,
        prog: &Program,
        f: &Func,
        driver: u32,
        src: Reg,
        desc: Reg,
        tab: ConstId,
        ops: Regs,
        mode: DriveMode,
        has: u8,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<()> {
        let Some(&[after, ref rest @ ..]) = f.regs(ops) else {
            return Err(cold(|| ice("a scatter without its operands")));
        };
        let (reject, part) = match (has & SCATTER_REJECT != 0, has & SCATTER_PART != 0, rest) {
            (false, false, []) => (None, None),
            (true, false, [r]) => (Some(*r), None),
            (false, true, [a, b]) => (
                None,
                Some(Part {
                    first: *a,
                    outer: *b,
                }),
            ),
            (true, true, [r, a, b]) => (
                Some(*r),
                Some(Part {
                    first: *a,
                    outer: *b,
                }),
            ),
            _ => return Err(cold(|| ice("a scatter with the wrong operands"))),
        };
        let (at, mut b) = self.take_plan(prog, tab, self.r(desc))?;
        let was = matches!(b.cmp, Cmp::Ready(_));
        // The cell: what compiled code skipped goes into the counts before the review.
        if let Cmp::Ready(r) = &b.cmp
            && let Inline::Bit(_, given) = r.inline
            && has == 0
            && self
                .icells
                .get(crate::GATHER_CELLS + driver as usize)
                .is_some_and(|x| x.left.load(Relaxed) != given)
            && let Some(c) = ctx.as_deref_mut()
        {
            self.scat_take(&mut b, tab, driver, c);
        }
        match self.scatter_leaves(&mut b, driver, src, part, after, mode, reject, ctx) {
            Ok(()) => {
                // Compiled code sees what this scatter made of the plan: it may start, it has
                // used up what it was given, or the image is gone.
                let put = match &b.cmp {
                    Cmp::Ready(r) => match r.inline {
                        Inline::Unasked => true,
                        Inline::Bit(_, given) => given == 0,
                        Inline::No => false,
                    },
                    _ => was,
                };
                if put
                    && has == 0
                    && let Some(c) = ctx.as_deref_mut()
                    && (was && !matches!(b.cmp, Cmp::Ready(_)) || !c.gate_unset())
                {
                    self.scat_put(&mut b, at, tab, self.r(desc), driver, c);
                }
                self.put_plan(at, b);
                Ok(())
            }
            Err(e) => {
                self.put_plan(at, b);
                Err(e)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn scatter_leaves(
        &mut self,
        b: &mut Bound,
        driver: u32,
        src: Reg,
        part: Option<Part>,
        after: Reg,
        mode: DriveMode,
        reject: Option<Reg>,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<()> {
        let plan = &*b.plan;
        let (first, outer) = part_of(self, plan, part)?;
        let t = to_time(self.ri(after))?;
        let now = t == yavs_kernel::Time::ZERO && reject.is_none();
        let mode = self.delay_mode(mode, reject)?;
        let Some(c) = ctx.as_deref_mut() else {
            return cold(|| rt_err("signal or wait operation during elaboration"));
        };
        let data = self.rd(self.r(src), plan.size)?;
        let ds = self
            .drivers
            .get(driver as usize..driver as usize + plan.leaves.len())
            .ok_or_else(|| cold(|| ice(format!("no driver slots from {driver}"))))?;
        let when = (t, now, mode);
        let (base, cached) = b.refs.get_or_init(|| {
            let refs = ds.iter().map(|d| c.direct_ref(d.id)).collect();
            (driver, refs)
        });
        // Another scatter through the same plan (other driver slots): no facts, the usual path.
        let refs: &[Option<yavs_kernel::DirectRef>] = if *base == driver { cached } else { &[] };
        let mut run = None;
        if part.is_none() && !b.items.is_empty() {
            // The value against the image of the signal: what holds its value needs no planes
            // built and no kernel call.
            let mut cmp = None;
            if now && *base == driver {
                if matches!(b.cmp, Cmp::Count(_) | Cmp::Asked(_)) {
                    b.cmp = cmp_step(std::mem::replace(&mut b.cmp, Cmp::Off), b, c, ds);
                } else if let Cmp::Ready(r) = &b.cmp
                    && c.drive_clean(&r.drivers)
                {
                    cmp = c.image(r.id).map(|i| (&**r, i));
                }
            }
            let mut spared = 0;
            if let Some((r, im)) = cmp {
                spared = scatter_compared(b, r, im, c, ds, data, refs, when)?;
            } else {
                for g in &*b.runs {
                    scatter_run(c, plan, &b.items, g, ds, data, refs, &mut run, when)?;
                }
            }
            if let Cmp::Ready(r) = &mut b.cmp {
                c.note_spared(spared);
                r.seen += 1;
                r.spared += u64::from(spared);
                if r.seen >= IMAGE_REVIEW {
                    cmp_review(&mut b.cmp, c);
                }
            }
            return flush_run(c, &mut run, when);
        }
        for (li, ((l, s), d)) in plan.leaves.iter().zip(&*b.sigs).zip(ds).enumerate() {
            let dref = refs.get(li).and_then(Option::as_ref);
            let n = l.insts as usize;
            for i in 0..n {
                // A logic leaf's instances are mirrored in the kernel (`Leaf::kstart`): taken
                // last first, they follow each other and join one run of the driver.
                let j = if l.logic() { n - 1 - i } else { i };
                let at = l.off(j);
                let v = data
                    .get(at..at + l.vsize as usize)
                    .ok_or_else(|| cold(|| ice("a leaf outside its value")))?;
                let r = Range::new(s.off + l.kstart(j, first, outer) as u32, l.len);
                write_leaf(c, d.id, l, r, v, &mut run, when, dref)?;
            }
        }
        flush_run(c, &mut run, when)
    }
}
