//! The language boundary: the Verilog domain as one serial kernel process, with VHDL inputs
//! read through `Ctx` and outputs published on kernel drivers in order.
//!
//! Each activation reads the inputs, runs the regions to empty, and assigns one batch of the
//! publication queue with zero delay. Batch 0 holds the Active-region changes of this
//! activation, so VHDL sees them in the next delta; every NBA region moves later changes one
//! batch on (decision D4: NBA results reach VHDL one delta after Active results). While batches
//! are left, the domain waits for `Time::ZERO` too, so the kernel's time step does not end.

use yavs_kernel::{Ctx, DelayMode, DriverId, ErrorKind, Process, SigRef, SimError, Time};

use crate::runtime::{Core, Domain, NetId};

impl Domain {
    /// VHDL signal `sig` drives `net` (read as `TO_X01Z`).
    pub fn input(&mut self, sig: impl Into<SigRef>, net: NetId) {
        self.inputs.push((sig.into(), net));
    }

    /// `net` drives VHDL through kernel driver `d` (a logic signal of the net's width).
    pub fn output(&mut self, net: NetId, d: DriverId) {
        self.core.nets[net.0 as usize].out = Some(self.outputs.len() as u32);
        self.outputs.push((net, d, false));
    }

    /// `output` to a `bit` or `bit_vector` signal: x and z are published as '0'.
    pub fn output_bit(&mut self, net: NetId, d: DriverId) {
        self.output(net, d);
        self.outputs.last_mut().expect("just pushed").2 = true;
    }
}

impl Core {
    /// Output `o` changed to `v`: it goes to the current batch unless an entry of that port is
    /// in it or a later one, then to the batch after that entry, and the cursor follows, so
    /// every change of a port, and the order of changes across ports, survives.
    pub(crate) fn record(&mut self, o: u32, v: yavs_kernel::LogicVec) {
        // ponytail: linear scan of the queue; an index per port if batches grow long.
        let last = self
            .publish
            .iter()
            .rposition(|b| b.iter().any(|(k, _)| *k == o));
        let at = last.map_or(self.cursor, |i| self.cursor.max(i + 1));
        self.cursor = at;
        if self.publish.len() <= at {
            self.publish.resize_with(at + 1, Vec::new);
        }
        self.publish[at].push((o, v));
    }
}

impl Process for Domain {
    fn run(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError> {
        self.core.cursor = 0;
        let init = !std::mem::replace(&mut self.started, true);
        // The VHDL initial values are where the inputs start, not an event (as in VHDL).
        // ponytail: every input read and compared per activation; `Ctx::event` first if many.
        for &(sig, net) in &self.inputs {
            let v = ctx.logic(sig)?.to_x01z();
            if v.len() != self.core.nets[net.0 as usize].value.len() {
                return Err(ctx.error(ErrorKind::Internal(format!(
                    "Verilog input net {} and its VHDL signal differ in width",
                    net.0
                ))));
            }
            self.core.write(net, 0, &v);
        }
        let start: Vec<_> = match init {
            true => {
                self.core.active.extend(0..self.bodies.len() as u32);
                self.outputs
                    .iter()
                    .map(|&(n, _, _)| self.core.nets[n.0 as usize].value.clone())
                    .collect()
            }
            false => {
                self.core.mature(ctx.now());
                Vec::new()
            }
        };
        let done = self.settle(ctx)?;
        // Every output is published at initialisation: its initial value when the Active
        // region did not change it.
        for (o, v) in start.into_iter().enumerate() {
            if self.core.publish.is_empty() {
                self.core.publish.push_back(Vec::new());
            }
            if !self.core.publish[0].iter().any(|(k, _)| *k == o as u32) {
                self.core.publish[0].push((o as u32, v));
            }
        }
        if let Some(batch) = self.core.publish.pop_front() {
            for (o, v) in batch {
                let (_, d, bit) = self.outputs[o as usize];
                let v = match bit {
                    // ponytail: element by element; a plane mask if bit ports get wide.
                    true => yavs_kernel::LogicVec::from_fn(v.len(), |i| match v.get(i).to_x01() {
                        yavs_logic::StdULogic::One => yavs_logic::StdULogic::One,
                        _ => yavs_logic::StdULogic::Zero,
                    }),
                    false => v,
                };
                ctx.assign(d, None, &[(Time::ZERO, &v)], DelayMode::INERTIAL)?;
            }
        }
        for &(sig, _) in &self.inputs {
            ctx.wait_on(sig)?;
        }
        if !done || !self.core.publish.is_empty() {
            ctx.wait_for(Time::ZERO)?;
        } else if let Some(t) = self.core.next_time() {
            ctx.wait_at(t)?;
        }
        Ok(())
    }

    /// One scheduler for all Verilog: never beside another process on a worker.
    fn serial(&self) -> bool {
        true
    }
}
