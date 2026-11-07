//! The Verilog runtime of the interpreter (`yavs_interp::VerilogHost`): a process lowered to IR
//! (`lower.rs`) runs on an `InterpProcess` that holds the domain's state while it runs and gives
//! it back at its suspension, with the wait it registered.

use std::sync::Arc;

use yavs_interp::{InstData, InterpProcess, Runtime, VerilogHost};
use yavs_ir::VEdge;
use yavs_kernel::{Ctx, ErrorKind, Process, SimError, Time};

use crate::exec::{ArgV, format};
use crate::lower::{ArgSpec, ProcInfo, Task};
use crate::runtime::{Core, Edge, Exec, NetId, Wait};
use crate::value::{V4, logic_of_planes, logic_planes, w64};

/// What the processes of a design share: their functions' tasks and names, the `%t` unit.
pub(crate) struct Tables {
    pub rt: Arc<Runtime>,
    pub tasks: Vec<Task>,
    pub procs: Vec<ProcInfo>,
    pub tf_fs: u64,
}

/// One process's host: the domain's state while the process runs (swapped in), and its wait.
struct Host {
    core: Core,
    tabs: Arc<Tables>,
    p: usize,
    on: Vec<(NetId, Edge)>,
    delay: Option<u64>,
}

impl Host {
    fn net(&self, n: u32) -> NetId {
        NetId(self.tabs.procs[self.p].nets[n as usize])
    }

    fn wait(&mut self) -> Result<Wait, SimError> {
        if !self.on.is_empty() {
            return Ok(Wait::On(std::mem::take(&mut self.on)));
        }
        Ok(match self.delay.take() {
            None => Wait::Done,
            Some(0) => Wait::Zero,
            Some(d) => Wait::Delay(Time::fs(d).ok_or_else(|| {
                SimError::new(ErrorKind::Internal(format!("a Verilog delay of {d} fs")))
            })?),
        })
    }
}

impl VerilogHost for Host {
    fn read(&mut self, net: u32, out: &mut [u64]) -> u32 {
        let v = &self.core.nets[self.net(net).0 as usize].value;
        logic_planes(v, out);
        v.len() as u32
    }

    fn write(
        &mut self,
        ctx: &mut Ctx<'_>,
        net: u32,
        at: i64,
        n: u32,
        planes: &[u64],
        nba: Option<u64>,
    ) -> Result<(), SimError> {
        let g = self.net(net);
        let cur = &self.core.nets[g.0 as usize].value;
        let w = cur.len() as i64;
        // Elements outside the net are not written (9.2, 5.2.1).
        let (a, b) = (at.max(0), at.saturating_add(n.into()).min(w));
        if a >= b {
            return Ok(());
        }
        let (lo, k) = ((a - at) as u32, (b - a) as u32);
        let part = if n <= 64 {
            let v = w64::sel(n, k, (planes[0], planes[1]), lo.into());
            // A blocking write of what a net of up to 64 elements holds changes nothing.
            if nba.is_none() && w <= 64 {
                let mut c = [0u64; 2];
                logic_planes(cur, &mut c);
                let m = w64::mask(k) << a;
                if (c[0] & m, c[1] & m) == (v.0 << a, v.1 << a) {
                    return Ok(());
                }
            }
            logic_of_planes(k as usize, &[v.0], &[v.1])
        } else {
            let h = planes.len() / 2;
            V4::from_planes(n, &planes[..h], &planes[h..])
                .slice(lo.into(), k)
                .to_logic()
        };
        let mut x = Exec::new(&mut self.core, ctx);
        match nba {
            None => x.set_at(g, a as u32, &part),
            Some(d) => {
                let d = Time::fs(d).ok_or_else(|| {
                    x.error(format!(
                        "a Verilog delay of {d} fs is beyond the largest time"
                    ))
                })?;
                x.nba(g, a as u32, part, d)
            }
        }
    }

    fn wait_on(&mut self, net: u32, edge: VEdge) {
        let e = match edge {
            VEdge::Any => Edge::Any,
            VEdge::Pos => Edge::Pos,
            VEdge::Neg => Edge::Neg,
        };
        let n = self.net(net);
        self.on.push((n, e));
    }

    fn wait_delay(&mut self, fs: u64) {
        self.delay = Some(fs);
    }

    fn task(&mut self, ctx: &mut Ctx<'_>, id: u32, args: &[u64]) -> Result<(), SimError> {
        let tabs = self.tabs.clone();
        let info = &tabs.procs[self.p];
        let mut x = Exec::new(&mut self.core, ctx);
        match &tabs.tasks[id as usize] {
            Task::Finish => x.finish(),
            Task::Display(spec, newline) => {
                let mut vals = Vec::with_capacity(spec.len());
                let mut at = 0usize;
                for s in spec {
                    vals.push(match s {
                        ArgSpec::Fmt(f) => ArgV::Fmt(f),
                        ArgSpec::Val(w, signed) => {
                            let n = (yavs_ir::vregs(*w) / 2) as usize;
                            let v = V4::from_planes(*w, &args[at..at + n], &args[at + n..]);
                            at += 2 * n;
                            ArgV::Val(v, *signed)
                        }
                        ArgSpec::Real => {
                            at += 1;
                            ArgV::Real(f64::from_bits(args[at - 1]))
                        }
                    });
                }
                let mut out = format(&vals, &info.inst.path, info.inst.unit_fs, tabs.tf_fs)
                    .map_err(|m| x.error(format!("{m} in `{}`", info.name)))?;
                if *newline {
                    out.push(b'\n');
                }
                x.output(&out);
            }
        }
        Ok(())
    }

    fn name(&self) -> &str {
        &self.tabs.procs[self.p].name
    }

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any + Send> {
        self
    }
}

/// The body of process `p` of the design for the domain: its machine, made at its first run.
pub(crate) fn body(
    tabs: Arc<Tables>,
    p: usize,
) -> impl FnMut(&mut Exec<'_, '_>) -> Result<Wait, SimError> + Send + 'static {
    let mut proc: Option<InterpProcess> = None;
    let mut host = Some(Box::new(Host {
        core: Core::default(),
        tabs,
        p,
        on: Vec::new(),
        delay: None,
    }));
    move |x| {
        let (core, ctx) = x.parts();
        let mut h = host.take().ok_or_else(|| lost(ctx))?;
        let ip = match &mut proc {
            Some(ip) => ip,
            None => proc.insert(InterpProcess::new(
                h.tabs.rt.clone(),
                h.tabs.procs[p].func,
                Arc::new(InstData::default()),
                Vec::new(),
                None,
            )?),
        };
        std::mem::swap(&mut h.core, core);
        ip.set_verilog_host(h);
        let r = ip.run(ctx);
        let back = ip
            .take_verilog_host()
            .and_then(|h| h.into_any().downcast::<Host>().ok());
        let mut h = back.ok_or_else(|| lost(ctx))?;
        std::mem::swap(&mut h.core, core);
        let w = h.wait();
        host = Some(h);
        // The interpreter's trace names statements of the Verilog program, not of the design's.
        r.map_err(|mut e| {
            e.trace.clear();
            e
        })?;
        w
    }
}

fn lost(ctx: &Ctx<'_>) -> SimError {
    ctx.error(ErrorKind::Internal(
        "a Verilog process lost its runtime".into(),
    ))
}
