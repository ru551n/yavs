//! The region's kernel process: it reads its inputs from the kernel
//! into the image, runs the compiled trigger function, and drives the outputs that changed with
//! zero delay through the drivers of the member processes.

use std::sync::Arc;

use yavs_kernel::{Ctx, DelayMode, DriverId, Process, Range, SigRef, SimError, Time};
use yavs_logic::Block;

use crate::codegen::Code;

/// One kernel driver whose elements the region exports: `(slot, first element of the leaf,
/// element in the driver's range, length)` per leaf part it covers.
pub(crate) struct Out {
    pub driver: DriverId,
    pub range: Range,
    pub parts: Vec<(u32, u32, u32, u32)>,
    pub last: Option<Block>,
}

pub(crate) struct RegionProc {
    pub code: Arc<Code>,
    pub img: Vec<Block>,
    /// Every region leaf with its slot: read once at initialisation.
    pub leaves: Vec<(SigRef, u32)>,
    /// Leaves read from the kernel at every wake.
    pub inputs: Vec<(SigRef, u32)>,
    pub outs: Vec<Out>,
    /// The clock element and the edge test of `Ctx::edge_by`.
    pub clock: Option<(SigRef, u8)>,
    /// What the region waits on.
    pub waits: Vec<SigRef>,
    pub started: bool,
}

impl RegionProc {
    fn read(ctx: &Ctx<'_>, img: &mut [Block], what: &[(SigRef, u32)]) -> Result<(), SimError> {
        let mut buf = [0u8; 32];
        for &(r, s) in what {
            ctx.read_into(r, &mut buf)?;
            let w = buf.as_chunks::<8>().0;
            img[s as usize] = [0, 1, 2, 3].map(|k| u64::from_le_bytes(w[k]));
        }
        Ok(())
    }

    /// Drive every output whose value differs from what was driven last (all of them the first
    /// time). Not inlined: profiles show the export apart from the body.
    #[inline(never)]
    fn export(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError> {
        for o in &mut self.outs {
            let mut v: Block = [0; 4];
            for &(slot, lo, at, len) in &o.parts {
                let m = if len >= 64 { !0 } else { (1u64 << len) - 1 };
                let src = self.img[slot as usize];
                for k in 0..4 {
                    v[k] |= ((src[k] >> lo) & m) << at;
                }
            }
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
            Self::read(ctx, &mut self.img, &self.leaves)?;
            self.code.run(&mut self.img, false);
            self.export(ctx)?;
            for &w in &self.waits {
                ctx.wait_on(w)?;
            }
            self.started = true;
            return Ok(());
        }
        Self::read(ctx, &mut self.img, &self.inputs)?;
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
