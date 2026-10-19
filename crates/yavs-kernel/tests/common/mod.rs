#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use yavs_kernel::*;

pub const BYTE: SignalType = SignalType::bytes(1, 1);

pub fn ns(n: u64) -> Time {
    Time::ns(n)
}

/// Records value changes, reports and (optionally) phases as text lines.
#[derive(Clone, Default)]
pub struct Trace {
    pub lines: Arc<Mutex<Vec<String>>>,
    pub phases: bool,
}

impl Trace {
    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut self.lines.lock().unwrap())
    }

    fn push(&self, s: String) {
        self.lines.lock().unwrap().push(s);
    }
}

impl Observer for Trace {
    fn phase(&mut self, h: &mut Hook<'_>, p: Phase) {
        if self.phases {
            self.push(format!("{} +{} {p:?}", h.now(), h.delta()));
        }
    }

    fn value_change(&mut self, h: &mut Hook<'_>, s: SignalId, r: Range) {
        let v = h.value(s.slice(r.offset, r.len)).unwrap();
        let name = h.signal_name(s).unwrap();
        self.push(format!(
            "{} +{} {name}[{}..{}]={v}",
            h.now(),
            h.delta(),
            r.offset,
            r.end()
        ));
    }

    fn report(&mut self, h: &mut Hook<'_>, r: &Report) {
        self.push(format!(
            "{} +{} p{} {:?}: {}",
            h.now(),
            h.delta(),
            r.process.map_or(u32::MAX, |p| p.0),
            r.severity,
            r.message
        ));
    }

    fn output(&mut self, h: &mut Hook<'_>, p: ProcId, bytes: &[u8]) {
        self.push(format!(
            "{} +{} p{} out: {}",
            h.now(),
            h.delta(),
            p.0,
            String::from_utf8_lossy(bytes)
        ));
    }
}

pub fn build(b: Builder, cfg: Config) -> (SimControl, Trace) {
    let mut sim = b.build(cfg).unwrap();
    let t = Trace::default();
    sim.subscribe(Box::new(t.clone()));
    (sim, t)
}

/// Gives a closure the higher-ranked signature `Process` needs.
pub fn p(f: impl FnMut(&mut Ctx<'_>) -> Result<(), SimError> + Send + 'static) -> impl Process {
    f
}

/// Effective value, panicking on a bad reference.
pub fn v(sim: &SimControl, r: impl Into<SigRef>) -> Vec<u8> {
    sim.value(r)
        .unwrap()
        .as_bytes()
        .expect("byte signal")
        .to_vec()
}
