//! The region's kernel process: it reads the inputs that changed since its last wake from the
//! kernel into the image (nine values to two: `'1'` and `'H'` are 1, everything else 0, the
//! metavalues counted), runs the compiled trigger function, and drives the outputs that changed
//! with zero delay through the drivers of the member processes (`'0'` and `'1'`).

use std::sync::Arc;
use std::sync::atomic::Ordering;

use yavs_kernel::{
    Ctx, DelayMode, DriverId, ErrorKind, ImageSpec, NO_IMAGE, ProcId, Process, Range, Severity,
    SigRef, SimError, Time,
};

use crate::codegen::{Code, Fun, mask, words};
use crate::lower::Check;

/// A check of the code: what it says, the source places (the statement, then the calls around
/// it), the member process and its name.
pub type Site = (Check, Vec<yavs_diag::Span>, usize, String);

/// The check sites of a region; from the cache as their encoding, decoded when a check first
/// fails or a note first fires (most runs have neither).
pub(crate) struct Sites {
    bytes: Vec<u8>,
    sites: std::sync::OnceLock<Vec<Site>>,
}

impl Sites {
    pub(crate) fn new(sites: Vec<Site>) -> Sites {
        Sites {
            bytes: Vec::new(),
            sites: sites.into(),
        }
    }

    /// From their encoding (postcard).
    pub(crate) fn lazy(bytes: Vec<u8>) -> Sites {
        Sites {
            bytes,
            sites: std::sync::OnceLock::new(),
        }
    }

    /// Site `i`; `None` also for an encoding that does not decode.
    pub(crate) fn get(&self, i: usize) -> Option<&Site> {
        let all =
            (self.sites).get_or_init(|| postcard::from_bytes(&self.bytes).unwrap_or_default());
        all.get(i)
    }
}

/// A kernel range read into the image: its words `slot`, from element `lo` there, `len`
/// elements; or, for a byte signal (`bytes` 1, 4 or 8, else 0), `len` integers, one per word,
/// the last element of the range in word `slot` (the image keeps arrays from the right, the
/// kernel's byte signals from the left).
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Read {
    pub r: SigRef,
    pub slot: u32,
    pub lo: u32,
    pub len: u32,
    pub bytes: u32,
    /// A change is settled and exported at once (`region::Selection::at_once`, a reset); any
    /// other change is read when it comes and settled at the next edge.
    pub now: bool,
}

/// The input chunks of a group of kernel signals, `RegionProc::chunks[at .. at + n]`, and the
/// kernel image (`ImageKind::Count`, one item per chunk) that counts the updates that changed
/// each.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct InSig {
    pub spec: ImageSpec,
    pub at: u32,
    pub n: u32,
    /// The image once made (`None`: every chunk is read at every wake), and its writes seen.
    pub id: Option<u32>,
    pub seen: u64,
}

/// One kernel driver whose elements the region exports: elements `range` of the words `slot` of
/// its kernel signal (integers: the words from `slot` on, its last element first).
pub(crate) struct Out {
    pub driver: DriverId,
    pub range: Range,
    pub slot: u32,
    /// An integer of 4 or 8 bytes; 0 for logic elements.
    pub bytes: u32,
    /// The word driven last (an integer, or at most 64 elements in one word), kept in place: the
    /// compare of an unchanged output is one load from the image and one from here.
    pub last: u64,
    /// The words driven last of a wider driver.
    pub wide: Vec<u64>,
    /// A memory's driver: the writes of the edge that may have changed it (after the first
    /// export only those words are compared and driven).
    pub ram: Vec<RamWrite>,
}

/// A write of a memory kept back to the commit (`lib.rs` `defer`): its flag word (set when it
/// happened in this wake, cleared by the export), its index word, and where it writes (the
/// operands of its `IndexWrite`).
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct RamWrite {
    pub flag: usize,
    pub idx: usize,
    pub lo: u32,
    pub n: u32,
    pub stride: u32,
    pub w: u32,
    pub left: i64,
    pub down: bool,
    /// The edge class of its member ([`NO_CLASS`]: every class).
    pub class: u32,
}

/// [`RamWrite::class`] of a member that runs at every edge.
pub(crate) const NO_CLASS: u32 = u32::MAX;

impl RamWrite {
    /// The elements of the kernel signal the write changed (it passed its index check).
    fn elems(&self, img: &[u64]) -> std::ops::Range<u32> {
        let i = img[self.idx] as i64;
        let p = if self.down {
            self.left - i
        } else {
            i - self.left
        };
        let first = self.lo as i64 + (i64::from(self.n) - 1 - p) * i64::from(self.stride);
        first as u32..first as u32 + self.w
    }
}

impl Out {
    /// The image holds what was driven last.
    fn driven(&self, img: &[u64]) -> bool {
        let (slot, lo, len) = (self.slot as usize, self.range.offset, self.range.len);
        if self.bytes != 0 {
            return match &img[slot..][..len as usize] {
                [v] => *v == self.last,
                w => *w == self.wide[..],
            };
        }
        if len <= 64 && lo % 64 + len <= 64 {
            return img[slot + lo as usize / 64] >> (lo % 64) & mask(len) == self.last;
        }
        let w = &img[slot..];
        (0..words(len) as usize)
            .all(|j| self.wide[j] == word_at(w, lo + 64 * j as u32, len - 64 * j as u32))
    }

    /// The image words its elements are in.
    fn words(&self) -> std::ops::Range<usize> {
        let (slot, lo, len) = (
            self.slot as usize,
            self.range.offset as usize,
            self.range.len as usize,
        );
        if self.bytes != 0 {
            // Integers: a word each, from `slot`.
            return slot..slot + len;
        }
        slot + lo / 64..slot + (lo + len - 1) / 64 + 1
    }
}

/// Outputs whose words are adjacent in the image (`outs[outs]`, ordered by word), compared at
/// each export as one slice with their copy at the last one (`RegionProc::shadow` from `sh`):
/// when no word changed none of them is looked at. Memories are in no span (their writes
/// say what changed).
pub(crate) struct Span {
    pub words: std::ops::Range<usize>,
    pub sh: usize,
    pub outs: std::ops::Range<usize>,
}

/// Bit sets, one per cone, class or step.
pub(crate) type Marks = Vec<Box<[u64]>>;

/// A bit set as its words that are not 0, with their places.
pub(crate) type Sparse = Box<[(u32, u64)]>;

/// `m |= s`.
#[inline]
fn or_into(m: &mut [u64], s: &[(u32, u64)]) {
    for &(k, w) in s {
        m[k as usize] |= w;
    }
}

/// For each cone of members (`cones`, bit sets over the members) and each edge class's (its clocked
/// members and those they reach): the spans (bits) holding words those members write
/// (`member_words`), and for a class also the memories its members write (`rams`: the classes of
/// each memory output's writes; bit `spans.len()` on).
pub(crate) fn span_marks(
    spans: &[Span],
    member_words: &[Vec<(u32, u32)>],
    cones: &[Box<[u64]>],
    class_members: &[Box<[u64]>],
    rams: &[Vec<u32>],
) -> (Vec<Sparse>, Vec<Sparse>) {
    let n = (spans.len() + rams.len()).div_ceil(64);
    // Per member: the spans it writes.
    let by_member: Vec<Vec<u32>> = (member_words.iter())
        .map(|ws| {
            let mut v = Vec::new();
            for &(w, c) in ws {
                let (w, z) = (w as usize, (w + c) as usize);
                let mut j = spans.partition_point(|sp| sp.words.end <= w);
                while j < spans.len() && spans[j].words.start < z {
                    v.push(j as u32);
                    j += 1;
                }
            }
            v
        })
        .collect();
    let of_cone = |cone: &[u64]| -> Vec<u64> {
        let mut m = vec![0; n];
        for (k, &w) in cone.iter().enumerate() {
            let mut w = w;
            while w != 0 {
                let i = 64 * k + w.trailing_zeros() as usize;
                w &= w - 1;
                for &j in &by_member[i] {
                    m[j as usize / 64] |= 1 << (j % 64);
                }
            }
        }
        m
    };
    let sparse = |m: Vec<u64>| -> Sparse {
        (m.into_iter().enumerate())
            .filter(|x| x.1 != 0)
            .map(|(k, w)| (k as u32, w))
            .collect()
    };
    let chunks = cones.iter().map(|c| sparse(of_cone(c))).collect();
    let classes = (class_members.iter().enumerate())
        .map(|(k, c)| {
            let mut m = of_cone(c);
            for (j, cs) in rams.iter().enumerate() {
                if cs.iter().any(|&x| x == k as u32 || x == NO_CLASS) {
                    let b = spans.len() + j;
                    m[b / 64] |= 1 << (b % 64);
                }
            }
            sparse(m)
        })
        .collect();
    (chunks, classes)
}

/// Words a span grows to by joining adjacent outputs (outputs sharing a word always join).
const SPAN_MAX: usize = 8;

/// Order `outs` by their first word, the memories last, and join the others of shared words, and
/// of adjacent words up to [`SPAN_MAX`], into spans; the length of the copy they need.
pub(crate) fn spans(outs: &mut [Out]) -> (Vec<Span>, usize) {
    outs.sort_by_key(|o| (!o.ram.is_empty(), o.words().start, o.range.offset));
    let mut v: Vec<Span> = Vec::new();
    for (i, o) in outs.iter().enumerate().filter(|(_, o)| o.ram.is_empty()) {
        let w = o.words();
        match v.last_mut() {
            Some(sp)
                if w.start < sp.words.end
                    || w.start == sp.words.end && w.end - sp.words.start <= SPAN_MAX =>
            {
                sp.words.end = sp.words.end.max(w.end);
                sp.outs.end = i + 1;
            }
            _ => v.push(Span {
                words: w,
                sh: 0,
                outs: i..i + 1,
            }),
        }
    }
    let mut n = 0;
    for sp in &mut v {
        sp.sh = n;
        n += sp.words.len();
    }
    (v, n)
}

pub(crate) struct RegionProc {
    pub code: Arc<Code>,
    pub img: Vec<u64>,
    /// Every kernel signal of the region, whole: read once at initialisation.
    pub leaves: Vec<Read>,
    /// The inputs in chunks of at most 64 elements (a word of the image), read when their count
    /// moved past `seen`; grouped in `ins`.
    pub chunks: Vec<Read>,
    pub seen: Vec<u64>,
    pub ins: Vec<InSig>,
    /// The images of `ins` were looked up (the wake after initialisation).
    pub looked: bool,
    /// `YAVS_CYCLE_IMPORT_CHECK=1`: after each import every chunk is read again and compared.
    pub check: bool,
    /// The first export, which drives every output, is done.
    pub exported: bool,
    /// Inputs read since the last settle changed no `Read::now` chunk: the settle waits for the
    /// next edge, where it runs before that wake's inputs are read.
    pub pending: bool,
    pub outs: Vec<Out>,
    pub spans: Vec<Span>,
    /// The words of every span as the last export left them.
    pub shadow: Vec<u64>,
    /// Per edge class: the clock element, the edge test of `Ctx::edge_by` and the word that
    /// tells the code whether the edge happened.
    pub clocks: Vec<(SigRef, u8, usize)>,
    /// Nothing but the clocks wakes the region: a wake without an edge does nothing.
    pub clocks_only: bool,
    /// What the region waits on.
    pub waits: Vec<SigRef>,
    pub started: bool,
    /// The checks of the code by site: what they say, and the member process and its name.
    pub checks: Arc<Sites>,
    /// The first of the two words a failed check stores its operands in.
    pub err: usize,
    /// The log of the notes that fired in a run (`Op::Note`): its first word and how many it
    /// holds; and the normal mode's report path for them.
    pub log: (usize, usize),
    pub reporter: crate::Reporter,
    /// The reports of the calls members fold (A22): member, severity, text; made at
    /// initialisation.
    pub reports: Arc<Vec<(usize, u8, String)>>,
    pub metas: Arc<crate::Metas>,
    /// Elements of member variables without an initial value, counted at initialisation.
    pub uninit: u64,
    pub buf: Vec<u8>,
    /// `YAVS_CYCLE_STATS=1`: how often each member's inputs were unchanged.
    pub stats: bool,
    /// With `stats`: export and import counts since the last wake ([`EX_NAMES`]).
    pub ex: [u64; EX_N],
    /// With `stats`: the chunks the last import read.
    pub read_now: Vec<usize>,
    /// Per input chunk (`chunk_cone`, an index into `cones`): the steps of the settle a change of
    /// it can reach; `mark`, those of the chunks the last import read.
    pub cones: Arc<Vec<Box<[u64]>>>,
    pub chunk_cone: Arc<Vec<u32>>,
    pub mark: Vec<u64>,
    /// Per edge class: the steps of the settle its clocked members' runs can reach (their reset
    /// branches, and what their writes reach); an edge marks those of the classes that had one.
    pub class_cones: Arc<Vec<Box<[u64]>>>,
    /// The spans of outputs the steps run since the last export can have changed (`spans`): those
    /// of the cones of the chunks read (`chunk_spans`, by cone) and of the classes that had an edge
    /// (`class_spans`: their cones and what their clocked members write).
    pub touched: Vec<u64>,
    pub chunk_spans: Vec<Sparse>,
    pub class_spans: Vec<Sparse>,
}

/// Developers (`YAVS_CYCLE_STATS=1`): the region's wakes, the input chunks read in wakes without an
/// edge, and each guard's runs and skips; printed with the summary line.
pub(crate) struct Stats {
    pub wakes: u64,
    pub edges: u64,
    /// Per input chunk: its name, and the wakes without an edge that read it.
    pub chunks: Vec<(String, u64)>,
    pub code: Arc<Code>,
    /// Per edge class: its name and edges.
    pub classes: Vec<(String, u64)>,
    /// Per member: its operations.
    pub ops: Vec<u64>,
    /// Over the wakes with an edge and those without: the operations of the settle's members, and
    /// of those the mark lets run.
    pub settled: [[u64; 2]; 2],
    /// Export and import counts ([`EX_NAMES`]).
    pub ex: [u64; EX_N],
    /// The operations of every member of the settle; per step those of its members no other step
    /// has, and its members other steps have too, which count once per wake through the wake that
    /// last counted each (`seen`).
    pub all: u64,
    pub own: Vec<(u64, Vec<u32>)>,
    pub seen: Vec<u64>,
}

/// What [`Stats::ex`] counts.
pub(crate) const EX_NAMES: [&str; EX_N] = [
    "exports",
    "spans in all",
    "spans touched",
    "span words compared",
    "spans changed",
    "outputs in changed spans",
    "outputs compared alone",
    "outputs driven",
    "memory writes exported",
    "imports",
    "image groups written",
    "chunk counts compared",
    "chunks read",
];
pub(crate) const EX_N: usize = 13;

impl Stats {
    /// One wake that runs code: the edges of each class, and the members the mark lets run.
    fn count(&mut self, img: &[u64], clocks: &[(SigRef, u8, usize)], mark: &[u64], edge: bool) {
        for (c, &(_, _, w)) in self.classes.iter_mut().zip(clocks) {
            c.1 += img[w];
        }
        let wake = self.wakes;
        // A mark that holds every step runs them all (`Code::run_as`).
        let run = if (mark.iter().zip(&self.code.every[..])).all(|(m, e)| e & !m == 0) {
            self.all
        } else {
            let mut run = 0;
            for (j, &m) in mark.iter().enumerate() {
                let mut m = m;
                while m != 0 {
                    let i = 64 * j + m.trailing_zeros() as usize;
                    m &= m - 1;
                    let Some((n, shared)) = self.own.get(i) else {
                        continue;
                    };
                    run += n;
                    for &u in shared {
                        if std::mem::replace(&mut self.seen[u as usize], wake) != wake {
                            run += self.ops[u as usize];
                        }
                    }
                }
            }
            run
        };
        let k = usize::from(!edge);
        self.settled[k][0] += self.all;
        self.settled[k][1] += run;
    }
}

impl Drop for Stats {
    fn drop(&mut self) {
        eprintln!(
            "cycle stats: {} wakes ran code, {} of them at an edge",
            self.wakes, self.edges
        );
        for (name, n) in &self.classes {
            eprintln!("cycle stats: edge class {name}: {n} edges");
        }
        for (name, n) in EX_NAMES.iter().zip(self.ex) {
            eprintln!("cycle stats: {name}: {n}");
        }
        for (k, what) in ["with an edge", "without an edge"].iter().enumerate() {
            let [all, run] = self.settled[k];
            eprintln!(
                "cycle stats: wakes {what}: settle operations {all}, {run} of them in steps the mark lets run"
            );
        }
        let mut c: Vec<&(String, u64)> = self.chunks.iter().filter(|c| c.1 > 0).collect();
        c.sort_by_key(|c| std::cmp::Reverse(c.1));
        for (name, n) in c.iter().take(40) {
            eprintln!("cycle stats: input {name} read in {n} wakes without an edge");
        }
        let Some(counts) = &self.code.counts else {
            return;
        };
        let n = |x: &std::sync::atomic::AtomicU64| x.load(Ordering::Relaxed);
        let (runs, skips): (u64, u64) =
            (counts.iter()).fold((0, 0), |a, c| (a.0 + n(&c[0]), a.1 + n(&c[1])));
        eprintln!(
            "cycle stats: {} guards: {runs} runs, {skips} skipped",
            counts.len()
        );
        let mut v: Vec<(usize, u64, u64)> = (counts.iter().enumerate())
            .map(|(g, c)| (g, n(&c[0]), n(&c[1])))
            .collect();
        v.sort_by_key(|x| std::cmp::Reverse(x.1 + x.2));
        for (g, r, k) in v.into_iter().take(30) {
            let gd = &self.code.guards[g];
            eprintln!(
                "cycle stats: guard {} ({} spans, {} words) ran {r}, skipped {k}",
                gd.name,
                gd.ins.len(),
                gd.ins.iter().map(|x| x.1).sum::<u32>()
            );
        }
    }
}

impl RegionProc {
    /// `rd` from the kernel into the image: inserted at its place, metavalues as 0; returns how
    /// many elements were metavalues.
    fn read(ctx: &Ctx<'_>, img: &mut [u64], buf: &mut Vec<u8>, rd: &Read) -> Result<u64, SimError> {
        if rd.bytes != 0 {
            let (n, b) = (rd.len.max(1) as usize, rd.bytes as usize);
            buf.resize(n * b, 0);
            ctx.read_into(rd.r, buf)?;
            for (j, x) in buf.chunks_exact(b).enumerate() {
                img[rd.slot as usize + n - 1 - j] = match *x {
                    [a] => u64::from(a),
                    [a, b, c, d] => i64::from(i32::from_le_bytes([a, b, c, d])) as u64,
                    _ => u64::from_le_bytes(x.try_into().expect("8 bytes")),
                };
            }
            return Ok(0);
        }
        let nw = words(rd.len) as usize;
        buf.resize(32 * nw, 0);
        ctx.read_into(rd.r, buf)?;
        let mut metas = 0;
        let slot = rd.slot as usize;
        for j in 0..nw {
            let w = |p: usize| {
                let at = 32 * j + 8 * p;
                u64::from_le_bytes(buf[at..at + 8].try_into().expect("8 bytes"))
            };
            let m = mask((rd.len - 64 * j as u32).min(64));
            // A known element is `0`, `1`, `L` or `H`: plane 1 set; plane 0 is then its value.
            let (p0, p1) = (w(0), w(1));
            metas += u64::from((!p1 & m).count_ones());
            let v = p0 & p1 & m;
            if nw > 1 {
                img[slot + j] = v;
            } else {
                // One word (a chunk, or a leaf inside a pack of at most 64 elements): only its
                // elements, the other leaves of the word may not be read again now.
                let mm = m << rd.lo;
                img[slot] = img[slot] & !mm | v << rd.lo & mm;
            }
        }
        Ok(metas)
    }

    /// Read the input chunks that changed since the last wake (all of a signal without an
    /// image); whether any was read. A count moves when an update changed an element of the
    /// chunk (or of its word's cover): back to an old value included, so nothing is missed.
    fn import(&mut self, ctx: &Ctx<'_>) -> Result<(bool, bool), SimError> {
        if !self.looked {
            self.looked = true;
            for g in &mut self.ins {
                g.id = ctx.image_id(&g.spec).filter(|&id| id != NO_IMAGE);
            }
        }
        let (mut metas, mut any, mut now) = (0, false, false);
        for g in &mut self.ins {
            let counts = match g.id {
                Some(id) => {
                    let w = ctx.image_writes(id);
                    if w == g.seen {
                        continue;
                    }
                    g.seen = w;
                    if self.stats {
                        self.ex[10] += 1;
                        self.ex[11] += u64::from(g.n);
                    }
                    ctx.image(id)
                }
                None => None,
            };
            for (k, i) in (g.at as usize..(g.at + g.n) as usize).enumerate() {
                if let Some(b) = counts {
                    let n = u64::from_le_bytes(b[8 * k..8 * k + 8].try_into().expect("8 bytes"));
                    if n == self.seen[i] {
                        continue;
                    }
                    self.seen[i] = n;
                }
                metas += Self::read(ctx, &mut self.img, &mut self.buf, &self.chunks[i])?;
                any = true;
                now |= self.chunks[i].now;
                let c = self.chunk_cone[i] as usize;
                for (m, c) in (self.mark.iter_mut()).zip(&self.cones[c][..]) {
                    *m |= c;
                }
                or_into(&mut self.touched, &self.chunk_spans[c]);
                if self.stats {
                    self.read_now.push(i);
                }
            }
        }
        if metas != 0 {
            self.metas.inputs.fetch_add(metas, Ordering::Relaxed);
        }
        if self.stats {
            self.ex[9] += 1;
            self.ex[12] += self.read_now.len() as u64;
        }
        if self.check {
            self.check_import(ctx)?;
        }
        Ok((any, now))
    }

    /// Every input chunk read again: the image must hold what the kernel holds.
    #[cold]
    fn check_import(&mut self, ctx: &Ctx<'_>) -> Result<(), SimError> {
        let mut img = self.img.clone();
        for rd in &self.chunks {
            Self::read(ctx, &mut img, &mut self.buf, rd)?;
            if img != self.img {
                return Err(ctx.error(ErrorKind::Internal(format!(
                    "cycle mode: the input {:?} changed in the kernel and was not read again",
                    rd.r
                ))));
            }
        }
        Ok(())
    }

    /// `YAVS_CYCLE_SKIP_CHECK=full`: no output of a span the export skips changed.
    #[cold]
    fn untouched(&self, ctx: &Ctx<'_>) -> Result<(), SimError> {
        // By output: an input may share a word with outputs (another field of a record).
        for (j, sp) in self.spans.iter().enumerate() {
            if self.touched[j / 64] >> (j % 64) & 1 != 0 {
                continue;
            }
            if let Some(o) = self.outs[sp.outs.clone()]
                .iter()
                .find(|o| !o.driven(&self.img))
            {
                return Err(ctx.error(ErrorKind::Internal(format!(
                    "cycle mode: output {:?} changed, which no step run since the last export writes (YAVS_CYCLE_SKIP_CHECK)",
                    o.driver
                ))));
            }
        }
        let first = self.spans.last().map_or(0, |sp| sp.outs.end);
        for (m, o) in self.outs[first..].iter().enumerate() {
            let b = self.spans.len() + m;
            if self.touched[b / 64] >> (b % 64) & 1 == 0
                && o.ram.iter().any(|wr| self.img[wr.flag] != 0)
            {
                return Err(ctx.error(ErrorKind::Internal(format!(
                    "cycle mode: memory {:?} was written by a class that had no edge (YAVS_CYCLE_SKIP_CHECK)",
                    o.driver
                ))));
            }
        }
        Ok(())
    }

    /// `YAVS_CYCLE_STATS`: what this export compares and drives, counted before it does.
    #[cold]
    fn census(&mut self) {
        let ex = &mut self.ex;
        ex[0] += 1;
        ex[1] += self.spans.len() as u64;
        for (j, sp) in self.spans.iter().enumerate() {
            let n = sp.words.len();
            if self.touched[j / 64] >> (j % 64) & 1 == 0 {
                continue;
            }
            ex[2] += 1;
            ex[3] += n as u64;
            if self.img[sp.words.clone()] == self.shadow[sp.sh..sp.sh + n] {
                continue;
            }
            ex[4] += 1;
            for o in &self.outs[sp.outs.clone()] {
                let w = o.words();
                let at = sp.sh + w.start - sp.words.start;
                ex[5] += 1;
                if w.len() == 1 || self.img[w.clone()] != self.shadow[at..at + w.len()] {
                    ex[6] += 1;
                    ex[7] += u64::from(!o.driven(&self.img));
                }
            }
        }
        let first = self.spans.last().map_or(0, |sp| sp.outs.end);
        for o in &self.outs[first..] {
            ex[8] += o.ram.iter().filter(|wr| self.img[wr.flag] != 0).count() as u64;
        }
    }

    /// Drive every output whose value differs from what was driven last (all of them the first
    /// time). Not inlined: profiles show the export apart from the body.
    #[inline(never)]
    fn export(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError> {
        let all = !std::mem::replace(&mut self.exported, true);
        if all {
            self.touched.fill(!0);
        } else {
            if self.code.full_check.is_some() {
                self.untouched(ctx)?;
            }
            if self.stats {
                self.census();
            }
        }
        // Only the spans a step run since the last export writes, 64 at a time.
        let (mut j, end) = (0, self.spans.len());
        while j < end {
            let w = self.touched[j / 64] >> (j % 64);
            if w == 0 {
                j = (j / 64 + 1) * 64;
                continue;
            }
            j += w.trailing_zeros() as usize;
            if j >= end {
                break;
            }
            let sp = &self.spans[j];
            j += 1;
            let n = sp.words.len();
            if !all && same(&self.img[sp.words.clone()], &self.shadow[sp.sh..sp.sh + n]) {
                continue;
            }
            for o in &mut self.outs[sp.outs.clone()] {
                // An output of one word compares with what it drove last itself.
                let w = o.words();
                let at = sp.sh + w.start - sp.words.start;
                if all
                    || w.len() == 1
                    || !same(&self.img[w.clone()], &self.shadow[at..at + w.len()])
                {
                    Self::export_one(ctx, &self.img, &mut self.buf, o, all)?;
                }
            }
            copy(
                &mut self.shadow[sp.sh..sp.sh + n],
                &self.img[sp.words.clone()],
            );
        }
        let mut written: Vec<usize> = Vec::new();
        // The memories, bits `spans.len()` on: those of the classes that had an edge.
        let first = self.spans.last().map_or(0, |sp| sp.outs.end);
        for (m, o) in self.outs[first..].iter_mut().enumerate() {
            let b = end + m;
            if self.touched[b / 64] >> (b % 64) & 1 == 0 {
                continue;
            }
            let slot = o.slot as usize;
            if !o.ram.is_empty() && !all {
                // A memory: the 64-element words its writes of this wake touched, each driven once.
                let mut chunks: Vec<u32> = Vec::new();
                for wr in o.ram.iter().filter(|wr| self.img[wr.flag] != 0) {
                    written.push(wr.flag);
                    let e = wr.elems(&self.img);
                    let (a, z) = (e.start.max(o.range.offset), e.end.min(o.range.end()));
                    chunks.extend((a / 64..z.div_ceil(64)).filter(|_| a < z));
                }
                chunks.sort_unstable();
                chunks.dedup();
                for c in chunks {
                    let a = (64 * c).max(o.range.offset);
                    let z = (64 * c + 64).min(o.range.end());
                    let len = z - a;
                    let v = word_at(&self.img[slot..], a, len);
                    let r = Range::new(a, len);
                    if !ctx.assign_planes_now(o.driver, Some(r), [v, mask(len), 0, 0]) {
                        let b = blocks(&[v], len);
                        ctx.assign_blocks(o.driver, Some(r), Time::ZERO, &b, DelayMode::Transport)?;
                    }
                }
                continue;
            }
            written.extend(o.ram.iter().map(|wr| wr.flag));
            Self::export_one(ctx, &self.img, &mut self.buf, o, all)?;
        }
        for f in written {
            self.img[f] = 0;
        }
        self.touched.fill(0);
        Ok(())
    }

    /// Drive output `o` when its value differs from what was driven last (or `all`).
    #[inline(always)]
    fn export_one(
        ctx: &mut Ctx<'_>,
        img: &[u64],
        buf: &mut Vec<u8>,
        o: &mut Out,
        all: bool,
    ) -> Result<(), SimError> {
        let slot = o.slot as usize;
        if o.bytes != 0 {
            // Integers the checks kept in their subtype, a word each: their low bytes are the
            // values.
            let w = &img[slot..][..o.range.len as usize];
            let same = !all
                && match w {
                    [v] => *v == o.last,
                    w => *w == o.wide[..],
                };
            if same {
                return Ok(());
            }
            match w {
                [v] => o.last = *v,
                w => {
                    o.wide.clear();
                    o.wide.extend_from_slice(w);
                }
            }
            buf.clear();
            for v in w.iter().rev() {
                buf.extend_from_slice(&v.to_le_bytes()[..o.bytes as usize]);
            }
            if !ctx.assign_bytes_now(o.driver, o.range, buf) {
                ctx.assign_bytes(
                    o.driver,
                    Some(o.range),
                    Time::ZERO,
                    buf,
                    DelayMode::Transport,
                )?;
            }
            return Ok(());
        }
        let (lo, len) = (o.range.offset, o.range.len);
        if len <= 64 && lo % 64 + len <= 64 {
            // One word (a register, or a driver of a pack).
            let v = img[slot + lo as usize / 64] >> (lo % 64) & mask(len);
            if v == o.last && !all {
                return Ok(());
            }
            o.last = v;
            // The whole driver, once per run, only when it changed: the kernel's short entry.
            if !ctx.publish_planes(o.driver, [v, mask(len), 0, 0])
                && !ctx.assign_planes_now(o.driver, Some(o.range), [v, mask(len), 0, 0])
            {
                let b = blocks(&[v], len);
                ctx.assign_blocks(
                    o.driver,
                    Some(o.range),
                    Time::ZERO,
                    &b,
                    DelayMode::Transport,
                )?;
            }
            return Ok(());
        }
        // Wider: word by word against what was driven last, the blocks in `buf` (no
        // allocation per export).
        let nw = words(len) as usize;
        let w = &img[slot..];
        let same = !all
            && if lo % 64 == 0 {
                // At a word boundary: whole words compared as they are, the last masked.
                let (k, full, rest) = ((lo / 64) as usize, (len / 64) as usize, len % 64);
                w[k..k + full] == o.wide[..full]
                    && (rest == 0 || w[k + full] & mask(rest) == o.wide[full])
            } else {
                (0..nw).all(|j| o.wide[j] == word_at(w, lo + 64 * j as u32, len - 64 * j as u32))
            };
        if same {
            return Ok(());
        }
        o.wide.resize(nw, 0);
        buf.clear();
        for j in 0..nw {
            let x = word_at(w, lo + 64 * j as u32, len - 64 * j as u32);
            o.wide[j] = x;
            let m = mask((len - 64 * j as u32).min(64));
            for p in [x, m, 0, 0] {
                buf.extend_from_slice(&p.to_le_bytes());
            }
        }
        if !ctx.assign_wide_now(o.driver, Some(o.range), buf) {
            ctx.assign_blocks(
                o.driver,
                Some(o.range),
                Time::ZERO,
                buf,
                DelayMode::Transport,
            )?;
        }
        Ok(())
    }

    /// The code at `site` failed: the error of the normal mode, in the member process.
    #[cold]
    fn failed(&self, ctx: &Ctx<'_>, site: u32) -> SimError {
        if site == crate::codegen::STALE {
            let g = self.code.stale.load(Ordering::Relaxed);
            let what = match self.code.guards.get(g) {
                Some(gd) => format!(
                    "process {} was skipped as its inputs had not changed, but running it changed what it drives",
                    gd.name
                ),
                None if g == crate::codegen::FULL => {
                    "the run with steps skipped ended otherwise than a run of every step".into()
                }
                None => "a step of the settle was skipped as no input it reads had changed, but running it changed what it drives".into(),
            };
            return ctx.error(ErrorKind::Internal(format!(
                "cycle mode: {what} (YAVS_CYCLE_SKIP_CHECK)"
            )));
        }
        if site == crate::codegen::UNSETTLED {
            return ctx.error(ErrorKind::Internal(
                "cycle mode: a loop group of the region changed in a pass after its fixed point"
                    .into(),
            ));
        }
        let (a, b) = (self.img[self.err] as i64, self.img[self.err + 1] as i64);
        let Some((check, at, pi, name)) = self.checks.get(site as usize) else {
            return ctx.error(ErrorKind::Internal(format!(
                "cycle mode: check {site} failed, which the region does not have"
            )));
        };
        let vals = crate::region::exact_meta().then_some((a, b));
        let mut e = ctx.error(ErrorKind::Runtime(check.message(vals)));
        e.process = Some((ProcId(*pi as u32), name.clone()));
        e.at.clone_from(at);
        e
    }

    fn run_code(&mut self, ctx: &mut Ctx<'_>, f: Fun) -> Result<(), SimError> {
        self.run_marked(ctx, f, false)
    }

    /// Run function `f`; with `marked`, the settle only where the inputs read since the last
    /// settle reach (`mark`).
    fn run_marked(&mut self, ctx: &mut Ctx<'_>, f: Fun, marked: bool) -> Result<(), SimError> {
        let mark = marked.then_some(&self.mark[..]);
        let r = self.code.run(&mut self.img, f, mark);
        // The notes of the run come first, as in the normal mode before a failed check.
        if self.log.1 > 0 && self.img[self.log.0] > 0 {
            self.notes(ctx)?;
        }
        r.map_err(|site| self.failed(ctx, site))
    }

    /// Report the notes the last run logged, in the order they fired, as their members.
    #[cold]
    fn notes(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError> {
        let (at, cap) = self.log;
        let n = std::mem::take(&mut self.img[at]) as usize;
        let internal =
            |ctx: &Ctx<'_>, m: &str| ctx.error(ErrorKind::Internal(format!("cycle mode: {m}")));
        if n > cap {
            return Err(internal(
                ctx,
                "more notes fired in one run than the region has",
            ));
        }
        let mut fired = Vec::with_capacity(n);
        for k in 0..n {
            let site = self.img[at + 1 + k] as usize;
            let Some((Check::Note(nt), _, pi, _)) = self.checks.get(site) else {
                return Err(internal(ctx, "a note of a site that is no note"));
            };
            fired.push((*pi, nt.severity, nt.text(&self.img)));
        }
        self.report(ctx, fired.into_iter())
    }

    /// Report `(member, severity, text)` in order, as their members, through the normal mode's
    /// report path.
    fn report(
        &self,
        ctx: &mut Ctx<'_>,
        all: impl Iterator<Item = (usize, u8, String)>,
    ) -> Result<(), SimError> {
        let Some(report) = self.reporter.get().cloned() else {
            return Err(ctx.error(ErrorKind::Internal(
                "cycle mode: no report path for the region's reports".into(),
            )));
        };
        // A report at the stop severity ends its process's run in the normal mode: the member's
        // later reports of this run are not made.
        let mut stopped: Vec<usize> = Vec::new();
        for (pi, s, text) in all {
            if stopped.contains(&pi) {
                continue;
            }
            let sev = match s {
                0 => Severity::Note,
                1 => Severity::Warning,
                2 => Severity::Error,
                _ => Severity::Failure,
            };
            if let Some(text) = report(ctx, sev, text) {
                ctx.report_as(ProcId(pi as u32), sev, text);
                if sev >= ctx.stop_severity() {
                    stopped.push(pi);
                }
            }
        }
        Ok(())
    }
}

/// `a == b` for slices of one length, for a few words the differences or-ed without a branch
/// (mostly equal; no call of `memcmp`).
#[inline(always)]
fn same(a: &[u64], b: &[u64]) -> bool {
    if a.len() <= 8 {
        a.iter().zip(b).fold(0, |d, (x, y)| d | (x ^ y)) == 0
    } else {
        a == b
    }
}

/// `copy_from_slice`, a loop for a few words (no call of `memcpy`).
#[inline(always)]
fn copy(to: &mut [u64], from: &[u64]) {
    if to.len() <= 8 {
        to.iter_mut().zip(from).for_each(|(t, f)| *t = *f);
    } else {
        to.copy_from_slice(from);
    }
}

/// The (at most 64) elements of `w` from element `off`, `left` of them or 64.
fn word_at(w: &[u64], off: u32, left: u32) -> u64 {
    let (k, s) = ((off / 64) as usize, off % 64);
    let mut x = w[k] >> s;
    if s != 0 && k + 1 < w.len() {
        x |= w[k + 1] << (64 - s);
    }
    x & mask(left.min(64))
}

/// Two-state words as the kernel's plane blocks (`Ctx::assign_blocks`): `'0'` and `'1'`.
fn blocks(v: &[u64], len: u32) -> Vec<u8> {
    let mut b = Vec::with_capacity(32 * v.len());
    for (j, &x) in v.iter().enumerate() {
        let m = mask((len - 64 * j as u32).min(64));
        for p in [x & m, m, 0, 0] {
            b.extend_from_slice(&p.to_le_bytes());
        }
    }
    b
}

impl Process for RegionProc {
    fn run(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError> {
        if !self.started {
            // Initialisation: every leaf from the kernel's initial values, then settle.
            let mut metas = self.uninit;
            for rd in &self.leaves {
                metas += Self::read(ctx, &mut self.img, &mut self.buf, rd)?;
            }
            self.metas.init.fetch_add(metas, Ordering::Relaxed);
            if !self.reports.is_empty() {
                let r = self.reports.clone();
                self.report(ctx, r.iter().map(|(pi, s, t)| (*pi, *s, t.clone())))?;
            }
            // From the next update on the kernel counts the changes of each input chunk.
            for g in &self.ins {
                ctx.image_ask(g.spec.clone());
            }
            self.run_code(ctx, Fun::Init)?;
            // A reset that holds at time 0 applies (its branch is in the settle), as when the
            // normal mode runs every process once.
            self.run_code(ctx, Fun::Settle)?;
            self.export(ctx)?;
            for &w in &self.waits {
                ctx.wait_on(w)?;
            }
            self.started = true;
            return Ok(());
        }
        // The edge of every class: the clocked bodies of those that happened run.
        let mut edge = false;
        for &(r, kind, w) in &self.clocks {
            let x = ctx.edge_by(r, kind)? == Some(true);
            self.img[w] = u64::from(x);
            edge |= x;
        }
        if !edge && self.clocks_only {
            if !ctx.wait_same() {
                for &w in &self.waits {
                    ctx.wait_on(w)?;
                }
            }
            return Ok(());
        }
        // The settle a change seen earlier left for the edge, on the inputs before this wake's:
        // the steps the inputs read since the last settle reach (`mark`).
        if edge && std::mem::take(&mut self.pending) {
            self.run_marked(ctx, Fun::Settle, true)?;
            self.mark.fill(0);
        }
        // Without an edge, only a change something outside can see before the next edge is
        // settled now (with no input changed the settle would give what it gave last).
        if self.stats {
            self.read_now.clear();
        }
        let (any, now) = self.import(ctx)?;
        if edge || now {
            self.pending = false;
            // Without an edge only the inputs changed: the settle runs what they reach; at an edge
            // also what the clocked members of the classes that had one may have changed.
            if edge {
                for (k, &(_, _, w)) in self.clocks.iter().enumerate() {
                    if self.img[w] != 0 {
                        (self.mark.iter_mut())
                            .zip(&self.class_cones[k][..])
                            .for_each(|(m, c)| *m |= c);
                        or_into(&mut self.touched, &self.class_spans[k]);
                    }
                }
            }
            if self.stats
                && let Some(st) = self.metas.stats.lock().expect("not poisoned").as_mut()
            {
                if !edge {
                    for &i in &self.read_now {
                        st.chunks[i].1 += 1;
                    }
                }
                st.wakes += 1;
                st.edges += u64::from(edge);
                for (a, b) in st.ex.iter_mut().zip(std::mem::take(&mut self.ex)) {
                    *a += b;
                }
                st.count(&self.img, &self.clocks, &self.mark, edge);
            }
            if edge {
                self.run_marked(ctx, Fun::Edge, true)?;
            } else {
                self.run_marked(ctx, Fun::Settle, true)?;
            }
            self.mark.fill(0);
            self.export(ctx)?;
        } else if any {
            self.pending = true;
        }
        if !ctx.wait_same() {
            for &w in &self.waits {
                ctx.wait_on(w)?;
            }
        }
        Ok(())
    }
}
