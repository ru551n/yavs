//! The region's kernel process: it reads the inputs that changed since its last wake from the
//! kernel into the image (nine values to two: `'1'` and `'H'` are 1, everything else 0, the
//! metavalues counted), runs the compiled trigger function, and drives the outputs that changed
//! with zero delay through the drivers of the member processes (`'0'` and `'1'`).

use std::sync::Arc;
use std::sync::atomic::Ordering;

use yavs_kernel::{
    Ctx, DelayMode, DriverId, ErrorKind, ImageSpec, NO_IMAGE, ProcId, Process, Range, SigRef,
    SimError, Time,
};

use crate::codegen::{Code, Fun, mask, words};
use crate::lower::Check;

/// A kernel range read into the image: its words `slot`, from element `lo` there, `len`
/// elements; or, for a byte signal (`bytes` 1, 4 or 8, else 0), `len` integers, one per word,
/// the last element of the range in word `slot` (the image keeps arrays from the right, the
/// kernel's byte signals from the left).
#[derive(Clone)]
pub(crate) struct Read {
    pub r: SigRef,
    pub slot: u32,
    pub lo: u32,
    pub len: u32,
    pub bytes: u32,
}

/// The input chunks of one kernel signal, `RegionProc::chunks[at .. at + n]`, and the kernel
/// image (`ImageKind::Count`, one item per chunk) that counts the updates that changed each.
#[derive(Clone)]
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
#[derive(Clone)]
pub(crate) struct RamWrite {
    pub flag: usize,
    pub idx: usize,
    pub lo: u32,
    pub n: u32,
    pub stride: u32,
    pub w: u32,
    pub left: i64,
    pub down: bool,
}

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

/// Order `outs` by their first word, the memories last, and join the others of adjacent or shared
/// words into spans; the length of the copy they need.
pub(crate) fn spans(outs: &mut [Out]) -> (Vec<Span>, usize) {
    outs.sort_by_key(|o| (!o.ram.is_empty(), o.words().start, o.range.offset));
    let mut v: Vec<Span> = Vec::new();
    for (i, o) in outs.iter().enumerate().filter(|(_, o)| o.ram.is_empty()) {
        let w = o.words();
        match v.last_mut() {
            Some(sp) if w.start <= sp.words.end => {
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
    /// moved past `seen`; grouped by kernel signal in `ins`.
    pub chunks: Vec<Read>,
    pub seen: Vec<u64>,
    pub ins: Vec<InSig>,
    /// The images of `ins` were looked up (the wake after initialisation).
    pub looked: bool,
    /// `YAVS_CYCLE_IMPORT_CHECK=1`: after each import every chunk is read again and compared.
    pub check: bool,
    /// The first export, which drives every output, is done.
    pub exported: bool,
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
    pub checks: Arc<Vec<(Check, usize, String)>>,
    /// The first of the two words a failed check stores its operands in.
    pub err: usize,
    pub metas: Arc<crate::Metas>,
    /// Elements of member variables without an initial value, counted at initialisation.
    pub uninit: u64,
    pub buf: Vec<u8>,
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
    fn import(&mut self, ctx: &Ctx<'_>) -> Result<bool, SimError> {
        if !self.looked {
            self.looked = true;
            for g in &mut self.ins {
                g.id = ctx.image_id(&g.spec).filter(|&id| id != NO_IMAGE);
            }
        }
        let (mut metas, mut any) = (0, false);
        for g in &mut self.ins {
            let counts = match g.id {
                Some(id) => {
                    let w = ctx.image_writes(id);
                    if w == g.seen {
                        continue;
                    }
                    g.seen = w;
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
            }
        }
        if metas != 0 {
            self.metas.inputs.fetch_add(metas, Ordering::Relaxed);
        }
        if self.check {
            self.check_import(ctx)?;
        }
        Ok(any)
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

    /// Drive every output whose value differs from what was driven last (all of them the first
    /// time). Not inlined: profiles show the export apart from the body.
    #[inline(never)]
    fn export(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError> {
        let all = !std::mem::replace(&mut self.exported, true);
        for sp in &self.spans {
            if !all && self.img[sp.words.clone()] == self.shadow[sp.sh..sp.sh + sp.words.len()] {
                continue;
            }
            for o in &mut self.outs[sp.outs.clone()] {
                let w = o.words();
                let at = sp.sh + w.start - sp.words.start;
                if all || self.img[w.clone()] != self.shadow[at..at + w.len()] {
                    Self::export_one(ctx, &self.img, &mut self.buf, o, all)?;
                }
            }
            self.shadow[sp.sh..sp.sh + sp.words.len()].copy_from_slice(&self.img[sp.words.clone()]);
        }
        let mut written: Vec<usize> = Vec::new();
        let first = self.spans.last().map_or(0, |sp| sp.outs.end);
        for o in &mut self.outs[first..] {
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
        if site == crate::codegen::UNSETTLED {
            return ctx.error(ErrorKind::Internal(
                "cycle mode: a loop group of the region changed in a pass after its fixed point"
                    .into(),
            ));
        }
        let (a, b) = (self.img[self.err] as i64, self.img[self.err + 1] as i64);
        let Some((check, pi, name)) = self.checks.get(site as usize) else {
            return ctx.error(ErrorKind::Internal(format!(
                "cycle mode: check {site} failed, which the region does not have"
            )));
        };
        let mut e = ctx.error(ErrorKind::Runtime(check.message(a, b)));
        e.process = Some((ProcId(*pi as u32), name.clone()));
        e
    }

    fn run_code(&mut self, ctx: &Ctx<'_>, f: Fun) -> Result<(), SimError> {
        self.code
            .run(&mut self.img, f)
            .map_err(|site| self.failed(ctx, site))
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
        // Without an edge and with no input changed the settle would give what it gave last.
        if self.import(ctx)? || edge {
            self.run_code(ctx, if edge { Fun::Edge } else { Fun::Settle })?;
            self.export(ctx)?;
        }
        if !ctx.wait_same() {
            for &w in &self.waits {
                ctx.wait_on(w)?;
            }
        }
        Ok(())
    }
}
