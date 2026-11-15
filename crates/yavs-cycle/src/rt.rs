//! The region's kernel process: it reads its inputs from the kernel
//! into the image, runs the compiled trigger function, and drives the outputs that changed with
//! zero delay through the drivers of the member processes.

use std::sync::Arc;

use yavs_kernel::{Ctx, DelayMode, DriverId, Process, Range, SigRef, SimError, Time};
use yavs_logic::Block;

use crate::codegen::Code;

/// One kernel driver whose elements the region exports: elements `range` of the block `slot` of
/// its kernel signal.
pub(crate) struct Out {
    pub driver: DriverId,
    pub range: Range,
    pub slot: u32,
    pub last: Option<Block>,
}

pub(crate) struct RegionProc {
    pub code: Arc<Code>,
    pub img: Vec<Block>,
    /// Every kernel signal of the region, whole, with its block: read once at initialisation.
    pub leaves: Vec<(SigRef, u32)>,
    /// Leaves read from the kernel at every wake: block and first element there, length.
    pub inputs: Vec<(SigRef, u32, u32, u32)>,
    pub outs: Vec<Out>,
    /// The clock element and the edge test of `Ctx::edge_by`.
    pub clock: Option<(SigRef, u8)>,
    /// What the region waits on.
    pub waits: Vec<SigRef>,
    pub started: bool,
}

impl RegionProc {
    fn read(ctx: &Ctx<'_>, r: SigRef) -> Result<Block, SimError> {
        let mut buf = [0u8; 32];
        ctx.read_into(r, &mut buf)?;
        let w = buf.as_chunks::<8>().0;
        Ok([0, 1, 2, 3].map(|k| u64::from_le_bytes(w[k])))
    }

    fn read_inputs(&mut self, ctx: &Ctx<'_>) -> Result<(), SimError> {
        for &(r, s, lo, len) in &self.inputs {
            let v = Self::read(ctx, r)?;
            let m = mask(len) << lo;
            let b = &mut self.img[s as usize];
            for k in 0..4 {
                b[k] = b[k] & !m | v[k] << lo & m;
            }
        }
        Ok(())
    }

    /// Drive every output whose value differs from what was driven last (all of them the first
    /// time). Not inlined: profiles show the export apart from the body.
    #[inline(never)]
    fn export(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError> {
        for o in &mut self.outs {
            let src = self.img[o.slot as usize];
            let (lo, m) = (o.range.offset, mask(o.range.len));
            let v: Block = std::array::from_fn(|k| src[k] >> lo & m);
            // Word by word: `==` on the arrays was a `memcmp` call per driver.
            if let Some(l) = o.last
                && (l[0] ^ v[0]) | (l[1] ^ v[1]) | (l[2] ^ v[2]) | (l[3] ^ v[3]) == 0
            {
                continue;
            }
            o.last = Some(v);
            if !ctx.assign_planes_now(o.driver, Some(o.range), v) {
                let mut bytes = [0u8; 32];
                for (k, w) in v.iter().enumerate() {
                    bytes[k * 8..k * 8 + 8].copy_from_slice(&w.to_le_bytes());
                }
                ctx.assign_blocks(
                    o.driver,
                    Some(o.range),
                    Time::ZERO,
                    &bytes,
                    DelayMode::Transport,
                )?;
            }
        }
        Ok(())
    }
}

impl Process for RegionProc {
    fn run(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError> {
        if !self.started {
            // Initialisation: every leaf from the kernel's initial values, then settle.
            for &(r, s) in &self.leaves {
                self.img[s as usize] = Self::read(ctx, r)?;
            }
            self.code.run(&mut self.img, false);
            self.export(ctx)?;
            for &w in &self.waits {
                ctx.wait_on(w)?;
            }
            self.started = true;
            return Ok(());
        }
        self.read_inputs(ctx)?;
        let edge = match self.clock {
            Some((r, kind)) => ctx.edge_by(r, kind)? == Some(true),
            None => false,
        };
        self.code.run(&mut self.img, edge);
        self.export(ctx)?;
        if !ctx.wait_same() {
            for &w in &self.waits {
                ctx.wait_on(w)?;
            }
        }
        Ok(())
    }
}

fn mask(len: u32) -> u64 {
    if len >= 64 { !0 } else { (1u64 << len) - 1 }
}
