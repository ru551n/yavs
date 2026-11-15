//! The region's kernel process: it reads its inputs from the kernel into the image (nine values
//! to two: `'1'` and `'H'` are 1, everything else 0, the metavalues counted), runs the compiled
//! trigger function, and drives the outputs that changed with zero delay through the drivers of
//! the member processes (`'0'` and `'1'`).

use std::sync::Arc;
use std::sync::atomic::Ordering;

use yavs_kernel::{
    Ctx, DelayMode, DriverId, ErrorKind, ProcId, Process, Range, SigRef, SimError, Time,
};

use crate::codegen::{Code, Fun, mask, words};
use crate::lower::Check;

/// A kernel range read into the image: its words `slot`, from element `lo` there, `len`
/// elements; or, for a byte signal (`bytes` 4 or 8, else 0), one integer.
#[derive(Clone)]
pub(crate) struct Read {
    pub r: SigRef,
    pub slot: u32,
    pub lo: u32,
    pub len: u32,
    pub bytes: u32,
}

/// One kernel driver whose elements the region exports: elements `range` of the words `slot` of
/// its kernel signal (an integer: the word).
pub(crate) struct Out {
    pub driver: DriverId,
    pub range: Range,
    pub slot: u32,
    /// An integer of 4 or 8 bytes; 0 for logic elements.
    pub bytes: u32,
    /// The words driven last (none yet: empty).
    pub last: Vec<u64>,
}

pub(crate) struct RegionProc {
    pub code: Arc<Code>,
    pub img: Vec<u64>,
    /// Every kernel signal of the region, whole: read once at initialisation.
    pub leaves: Vec<Read>,
    /// Leaves read from the kernel at every wake.
    pub inputs: Vec<Read>,
    pub outs: Vec<Out>,
    /// The clock element and the edge test of `Ctx::edge_by`.
    pub clock: Option<(SigRef, u8)>,
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
    /// The word that tells the code whether the clock edge happened.
    pub edge_word: usize,
    /// A member has an asynchronous reset: the edge function runs at every wake.
    pub has_reset: bool,
    pub buf: Vec<u8>,
}

impl RegionProc {
    /// `rd` from the kernel into the image: inserted at its place, metavalues as 0; returns how
    /// many elements were metavalues.
    fn read(ctx: &Ctx<'_>, img: &mut [u64], buf: &mut Vec<u8>, rd: &Read) -> Result<u64, SimError> {
        if rd.bytes == 1 {
            let mut b = [0u8; 1];
            ctx.read_into(rd.r, &mut b)?;
            img[rd.slot as usize] = u64::from(b[0]);
            return Ok(0);
        }
        if rd.bytes == 4 {
            let mut b = [0u8; 4];
            ctx.read_into(rd.r, &mut b)?;
            img[rd.slot as usize] = i64::from(i32::from_le_bytes(b)) as u64;
            return Ok(0);
        }
        if rd.bytes == 8 {
            let mut b = [0u8; 8];
            ctx.read_into(rd.r, &mut b)?;
            img[rd.slot as usize] = u64::from_le_bytes(b);
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
            if rd.lo == 0 {
                img[slot + j] = v;
            } else {
                // A leaf inside a pack of at most 64 elements: one word.
                let mm = m << rd.lo;
                img[slot] = img[slot] & !mm | v << rd.lo & mm;
            }
        }
        Ok(metas)
    }

    /// Drive every output whose value differs from what was driven last (all of them the first
    /// time). Not inlined: profiles show the export apart from the body.
    #[inline(never)]
    fn export(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError> {
        for o in &mut self.outs {
            let slot = o.slot as usize;
            if o.bytes != 0 {
                let v = self.img[slot];
                if o.last.first() == Some(&v) {
                    continue;
                }
                o.last.clear();
                o.last.push(v);
                // An integer the checks kept in its subtype: its low bytes are the value.
                let b = &v.to_le_bytes()[..o.bytes as usize];
                if !ctx.assign_bytes_now(o.driver, o.range, b) {
                    ctx.assign_bytes(o.driver, Some(o.range), Time::ZERO, b, DelayMode::Transport)?;
                }
                continue;
            }
            let (lo, len) = (o.range.offset, o.range.len);
            if len <= 64 && lo % 64 + len <= 64 {
                // One word (a register, or a driver of a pack).
                let v = self.img[slot + lo as usize / 64] >> (lo % 64) & mask(len);
                if o.last.first() == Some(&v) {
                    continue;
                }
                o.last.clear();
                o.last.push(v);
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
                continue;
            }
            let v = extract(&self.img[slot..], lo, len);
            if o.last == v {
                continue;
            }
            let b = blocks(&v, len);
            o.last = v;
            if !ctx.assign_wide_now(o.driver, Some(o.range), &b) {
                ctx.assign_blocks(
                    o.driver,
                    Some(o.range),
                    Time::ZERO,
                    &b,
                    DelayMode::Transport,
                )?;
            }
        }
        Ok(())
    }

    /// The code at `site` failed: the error of the normal mode, in the member process.
    #[cold]
    fn failed(&self, ctx: &Ctx<'_>, site: u32) -> SimError {
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

    /// The edge function where the edge happened or a reset may hold, else the settle function.
    fn wake(&mut self, ctx: &Ctx<'_>, edge: bool) -> Result<(), SimError> {
        self.img[self.edge_word] = u64::from(edge);
        let f = if edge || self.has_reset {
            Fun::Edge
        } else {
            Fun::Settle
        };
        self.run_code(ctx, f)
    }
}

/// Elements `lo .. lo + len` of `w` as words.
fn extract(w: &[u64], lo: u32, len: u32) -> Vec<u64> {
    (0..words(len))
        .map(|j| {
            let off = lo + 64 * j;
            let (k, s) = ((off / 64) as usize, off % 64);
            let mut x = w[k] >> s;
            if s != 0 && k + 1 < w.len() {
                x |= w[k + 1] << (64 - s);
            }
            x & mask((len - 64 * j).min(64))
        })
        .collect()
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
            self.run_code(ctx, Fun::Init)?;
            // A reset that holds at time 0 applies, as when the normal mode runs every process once.
            self.wake(ctx, false)?;
            self.export(ctx)?;
            for &w in &self.waits {
                ctx.wait_on(w)?;
            }
            self.started = true;
            return Ok(());
        }
        let mut metas = 0;
        for rd in &self.inputs {
            metas += Self::read(ctx, &mut self.img, &mut self.buf, rd)?;
        }
        if metas != 0 {
            self.metas.inputs.fetch_add(metas, Ordering::Relaxed);
        }
        let edge = match self.clock {
            Some((r, kind)) => ctx.edge_by(r, kind)? == Some(true),
            None => false,
        };
        self.wake(ctx, edge)?;
        self.export(ctx)?;
        if !ctx.wait_same() {
            for &w in &self.waits {
                ctx.wait_on(w)?;
            }
        }
        Ok(())
    }
}
