//! Live waves (#325, `docs/design/gui.md` 5.1): `yavs/waveSubscribe` records the selected
//! signals (and variables) with a `yavs_wave::Recorder`, the same hierarchy and final value per
//! time step as `--wave` writes; this module is its [`Sink`]. Changes of completed time steps
//! are collected into a batch, sent as a `yavs/waves` event once `flushMs` have passed or
//! [`BATCH_BYTES`] are pending, and always before `stopped`, `exited` and the answer to
//! `yavs/waveUnsubscribe`. Values are base64 of the recorder's value bytes (the layout the FST
//! path gets, `yavs_wave` crate docs); the subscribe response says how to decode each id.
//!
//! Batches go through a bounded queue to a sender thread, so a slow client never blocks the
//! simulation: when the queue is full, the newest batch is merged into the last one, keeping
//! each value's last change (`merged`: earlier changes were dropped). Every other message
//! writes the queue first, so events keep their order with the rest of the session.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::{Value as Json, json};
use yavs_kernel::{Hook, Observer, Phase, SignalId as KernelId};
use yavs_wave::{
    Dir, Error, HierItem, Hierarchy, ObjClass, PortDir, Recorder, ScalarType, ScopeKind, SigType,
    SignalId, Sink,
};

/// Batches the queue holds before it merges.
const QUEUE: usize = 64;
/// Pending text that sends a batch at the end of a time step without waiting for `flushMs`.
const BATCH_BYTES: usize = 256 * 1024;

/// Changes of completed time steps: `(time fs, id, value)`, in order.
#[derive(Default)]
pub(crate) struct Batch {
    from: Option<u64>,
    to: u64,
    changes: Vec<(u64, u32, String)>,
    merged: bool,
}

impl Batch {
    pub(crate) fn event(&self) -> Json {
        let mut times: Vec<u64> = Vec::new();
        let changes: Vec<Json> = self
            .changes
            .iter()
            .map(|(t, id, v)| {
                if times.last() != Some(t) {
                    times.push(*t);
                }
                json!([times.len() - 1, id, v])
            })
            .collect();
        let mut body = json!({
            "from": self.from.unwrap_or(self.to),
            "to": self.to,
            "times": times,
            "changes": changes,
        });
        if self.merged {
            body["merged"] = json!(true);
        }
        json!({"type": "event", "event": "yavs/waves", "body": body})
    }

    /// Append `b`, keeping only the last change of each value.
    fn merge(&mut self, b: Batch) {
        let ids: HashSet<u32> = b.changes.iter().map(|c| c.1).collect();
        self.changes.retain(|c| !ids.contains(&c.1));
        self.changes.extend(b.changes);
        self.changes.sort_by_key(|c| c.0);
        self.to = b.to;
        self.merged = true;
    }
}

struct Pending {
    batch: Batch,
    /// The `to` of the last batch queued.
    sent: Option<u64>,
    bytes: usize,
    last: Instant,
    every: Duration,
}

/// What the recorder's sink and the server share: the batch being collected and the queue.
pub(crate) struct Stream {
    pending: Mutex<Pending>,
    pub(crate) queue: Mutex<VecDeque<Batch>>,
    wake: Condvar,
}

impl Default for Stream {
    fn default() -> Self {
        Stream {
            pending: Mutex::new(Pending {
                batch: Batch::default(),
                sent: None,
                bytes: 0,
                last: Instant::now(),
                every: Duration::ZERO,
            }),
            queue: Mutex::default(),
            wake: Condvar::new(),
        }
    }
}

impl Stream {
    fn lock(&self) -> MutexGuard<'_, Pending> {
        self.pending.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Queue the batch being collected, if it covers any time step.
    pub(crate) fn push(&self) {
        let mut p = self.lock();
        if p.batch.from.is_none() {
            return;
        }
        // Signals are written at the end of their step, before the step is reported done.
        let last = p.batch.changes.last().map(|c| c.0);
        p.batch.to = p.batch.to.max(last.unwrap_or(0));
        p.sent = Some(p.batch.to);
        let b = std::mem::take(&mut p.batch);
        p.bytes = 0;
        p.last = Instant::now();
        drop(p);
        let mut q = self.queue.lock().unwrap_or_else(|p| p.into_inner());
        let full = q.len() >= QUEUE;
        match q.back_mut() {
            Some(last) if full => last.merge(b),
            _ => q.push_back(b),
        }
        self.wake.notify_one();
    }
}

/// Write the queued batches as they come, until the process ends (`write`: the server's
/// writer of queued batches, which takes the connection's lock first).
pub(crate) fn sender(stream: Arc<Stream>, write: impl Fn()) {
    loop {
        {
            let q = stream.queue.lock().unwrap_or_else(|p| p.into_inner());
            let _q = stream
                .wake
                .wait_while(q, |q: &mut VecDeque<Batch>| q.is_empty())
                .unwrap_or_else(|p| p.into_inner());
        }
        write();
    }
}

/// The recorder's sink: values as base64 of the bytes the recorder gives.
struct StreamSink {
    stream: Arc<Stream>,
}

impl Sink for StreamSink {
    fn change(&mut self, time: u64, id: SignalId, value: &[u8]) -> Result<(), Error> {
        let text = base64(value);
        let mut p = self.stream.lock();
        p.batch.from.get_or_insert(time);
        p.bytes += text.len() + 16;
        p.batch.changes.push((time, id.0, text));
        Ok(())
    }

    fn step(&mut self, time: u64) {
        let mut p = self.stream.lock();
        if p.batch.changes.is_empty() && p.sent.is_some_and(|t| time <= t) {
            return; // already sent
        }
        p.batch.from.get_or_insert(time);
        p.batch.to = time;
        let due = p.bytes >= BATCH_BYTES || p.last.elapsed() >= p.every;
        drop(p);
        if due {
            self.stream.push();
        }
    }

    fn finish(self: Box<Self>) -> Result<(), Error> {
        self.stream.push();
        Ok(())
    }
}

/// A subscription: the recorder, and whether it has seen the simulation start.
pub(crate) struct Sub {
    pub rec: Recorder,
    started: bool,
}

/// A sink streaming into `stream`, sending batches every `every`.
pub(crate) fn sink(stream: &Arc<Stream>, every: Duration) -> Box<dyn Sink> {
    let mut p = stream.lock();
    p.every = every;
    p.last = Instant::now();
    drop(p);
    Box::new(StreamSink {
        stream: stream.clone(),
    })
}

impl Sub {
    pub(crate) fn new(rec: Recorder) -> Sub {
        Sub {
            rec,
            started: false,
        }
    }
}

/// Forwards the value-change bus to the current subscription's recorder; one subscribed after
/// the start records every value from its first callback on, at the time of the values then
/// (at `NextTimeStep` still those of the step before).
pub(crate) struct Tap {
    pub sub: Arc<Mutex<Option<Sub>>>,
    /// The time of the last callback, fs.
    pub last: u64,
}

impl Tap {
    fn sub(&self, start: bool) -> MutexGuard<'_, Option<Sub>> {
        let mut g = self.sub.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(s) = g.as_mut()
            && !s.started
        {
            s.started = true;
            if !start {
                s.rec.start(self.last);
            }
        }
        g
    }
}

impl Observer for Tap {
    fn phase(&mut self, hook: &mut Hook<'_>, phase: Phase) {
        if phase != Phase::NextTimeStep {
            self.last = hook.now().as_fs();
        }
        if let Some(s) = self.sub(phase == Phase::StartOfSim).as_mut() {
            s.rec.phase(hook, phase);
        }
        self.last = hook.now().as_fs();
    }

    fn value_changes(&mut self, hook: &mut Hook<'_>, signals: &[u32]) {
        self.last = hook.now().as_fs();
        if let Some(s) = self.sub(false).as_mut() {
            s.rec.value_changes(hook, signals);
        }
    }

    fn value_change(&mut self, hook: &mut Hook<'_>, signal: KernelId, range: yavs_kernel::Range) {
        self.last = hook.now().as_fs();
        if let Some(s) = self.sub(false).as_mut() {
            s.rec.value_change(hook, signal, range);
        }
    }
}

/// Standard base64 with padding.
fn base64(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The subscribe response: the recorded hierarchy, as `--wave` declares it in the file.
pub(crate) fn hierarchy(h: &Hierarchy) -> Json {
    fn scope_kind(k: ScopeKind) -> &'static str {
        match k {
            ScopeKind::Instance => "instance",
            ScopeKind::Block => "block",
            ScopeKind::Process => "process",
            ScopeKind::Procedure => "procedure",
            ScopeKind::Function => "function",
            ScopeKind::ForGenerate => "forGenerate",
            ScopeKind::IfGenerate => "ifGenerate",
            ScopeKind::Generate => "generate",
            ScopeKind::Package => "package",
            ScopeKind::Record => "record",
            ScopeKind::Array => "array",
        }
    }
    // The open scopes with their children so far, and the finished top level.
    let mut open: Vec<(Json, Vec<Json>)> = Vec::new();
    let mut top: Vec<Json> = Vec::new();
    for it in h.items() {
        let done = match it {
            HierItem::Scope {
                name,
                kind,
                component,
            } => {
                let head = json!({"scope": name, "kind": scope_kind(kind), "component": component});
                open.push((head, Vec::new()));
                continue;
            }
            HierItem::Up => {
                let (mut head, children) = open.pop().expect("a scope is open");
                head["children"] = Json::Array(children);
                head
            }
            HierItem::Signal {
                name,
                id,
                ty,
                port,
                class,
                ..
            } => json!({
                "name": name,
                "id": id.0,
                "type": ty_json(ty),
                "encoding": encoding(ty),
                "class": match class {
                    ObjClass::Signal => "signal",
                    ObjClass::Variable => "variable",
                    ObjClass::Constant => "constant",
                },
                "direction": match port {
                    PortDir::None => "none",
                    PortDir::In => "in",
                    PortDir::Out => "out",
                    PortDir::InOut => "inout",
                    PortDir::Buffer => "buffer",
                    PortDir::Linkage => "linkage",
                },
            }),
        };
        match open.last_mut() {
            Some((_, children)) => children.push(done),
            None => top.push(done),
        }
    }
    /// How a value's bytes read: `layout` and the `width` it holds (elements, bits).
    fn encoding(t: &SigType) -> Json {
        match t {
            SigType::Scalar(ScalarType::Enum(_)) => json!({"layout": "u32le", "width": 1}),
            SigType::Scalar(ScalarType::Integer { bits, .. }) => {
                json!({"layout": "i64le", "width": bits})
            }
            SigType::Scalar(ScalarType::Real) => json!({"layout": "f64le", "width": 64}),
            SigType::Scalar(ScalarType::Time) => json!({"layout": "i64le", "width": 64}),
            SigType::Vector { bounds, .. } => json!({
                "layout": "u8-per-element",
                "width": bounds.left.abs_diff(bounds.right) + 1,
            }),
        }
    }
    fn ty_json(t: &SigType) -> Json {
        match t {
            SigType::Scalar(ScalarType::Enum(e)) => {
                json!({"kind": "enum", "name": e.name, "literals": e.literals})
            }
            SigType::Scalar(ScalarType::Integer { name, bits }) => {
                json!({"kind": "integer", "name": name, "bits": bits})
            }
            SigType::Scalar(ScalarType::Real) => json!({"kind": "real"}),
            SigType::Scalar(ScalarType::Time) => json!({"kind": "time"}),
            SigType::Vector {
                elem,
                bounds,
                type_name,
            } => json!({
                "kind": "vector",
                "name": type_name,
                "left": bounds.left,
                "right": bounds.right,
                "ascending": bounds.dir == Dir::To,
                "element": {"name": elem.name, "literals": elem.literals},
            }),
        }
    }
    let skipped: Vec<Json> = h
        .skipped
        .iter()
        .map(|s| json!({"path": s.path, "elements": s.elems}))
        .collect();
    json!({
        "scopes": top,
        "skipped": skipped,
    })
}

#[test]
fn base64_pads() {
    assert_eq!(base64(&[]), "");
    assert_eq!(base64(b"f"), "Zg==");
    assert_eq!(base64(b"fo"), "Zm8=");
    assert_eq!(base64(b"foo"), "Zm9v");
    assert_eq!(base64(&[2, 3, 0, 4]), "AgMABA==");
}

#[test]
fn merge_keeps_last_changes() {
    let mut a = Batch {
        from: Some(0),
        to: 10,
        changes: vec![(0, 1, "a".into()), (10, 2, "b".into())],
        merged: false,
    };
    a.merge(Batch {
        from: Some(20),
        to: 30,
        changes: vec![(20, 1, "c".into())],
        merged: false,
    });
    assert_eq!(a.changes, [(10, 2, "b".into()), (20, 1, "c".into())]);
    assert_eq!((a.from, a.to, a.merged), (Some(0), 30, true));
}
