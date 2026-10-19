//! Live waves (#325, `docs/design/gui.md` 5.1): `yavs/waveSubscribe` records the selected
//! signals (and variables) with a `yavs_wave::Recorder`, the same hierarchy and final value per
//! time step as `--wave` writes; this module is its [`Sink`]. Changes of completed time steps
//! are collected into a batch, sent as a `yavs/waves` event once `flushMs` have passed or
//! [`BATCH_BYTES`] are pending, and always before `stopped`, `exited` and the answer to
//! `yavs/waveUnsubscribe`. Values are text, as the Variables view shows them.
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
    Dir, Error, Hierarchy, Node, ObjClass, PortDir, Recorder, ScalarType, ScopeKind, SigType,
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

/// The recorder's sink: values as text, by the declared types of the hierarchy.
struct StreamSink {
    stream: Arc<Stream>,
    types: Vec<Option<SigType>>,
}

impl Sink for StreamSink {
    fn change(&mut self, time: u64, id: SignalId, value: &[u8]) -> Result<(), Error> {
        let ty = self.types.get(id.0 as usize).and_then(Option::as_ref);
        let ty = ty.ok_or_else(|| Error::Value(format!("signal {}: not declared", id.0)))?;
        let text = text(ty, value).ok_or_else(|| Error::Value(format!("signal {}", id.0)))?;
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

/// A recorder for `h`'s hierarchy streaming into `stream`, sending batches every `every`.
pub(crate) fn sink(stream: &Arc<Stream>, h: &Hierarchy, every: Duration) -> Box<dyn Sink> {
    fn walk(n: &Node, types: &mut Vec<Option<SigType>>) {
        match n {
            Node::Scope(s) => s.children.iter().for_each(|c| walk(c, types)),
            Node::Signal(s) => {
                let i = s.id.0 as usize;
                if types.len() <= i {
                    types.resize(i + 1, None);
                }
                types[i] = Some(s.ty.clone());
            }
        }
    }
    let mut types = Vec::new();
    h.roots.iter().for_each(|n| walk(n, &mut types));
    let mut p = stream.lock();
    p.every = every;
    p.last = Instant::now();
    drop(p);
    Box::new(StreamSink {
        stream: stream.clone(),
        types,
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

    fn value_change(&mut self, hook: &mut Hook<'_>, signal: KernelId, range: yavs_kernel::Range) {
        self.last = hook.now().as_fs();
        if let Some(s) = self.sub(false).as_mut() {
            s.rec.value_change(hook, signal, range);
        }
    }
}

/// A value in the byte layout of its type (`yavs_wave` crate docs) as the Variables view
/// writes it.
fn text(ty: &SigType, b: &[u8]) -> Option<String> {
    let i64_of = |b: &[u8]| b.try_into().ok().map(i64::from_le_bytes);
    Some(match ty {
        SigType::Scalar(ScalarType::Enum(e)) => {
            let p = u32::from_le_bytes(b.try_into().ok()?);
            e.literals.get(p as usize)?.clone()
        }
        SigType::Scalar(ScalarType::Integer { .. }) => i64_of(b)?.to_string(),
        SigType::Scalar(ScalarType::Time) => format!("{} fs", i64_of(b)?),
        SigType::Scalar(ScalarType::Real) => f64::from_le_bytes(b.try_into().ok()?).to_string(),
        SigType::Vector { elem, .. } => {
            let lits: Option<Vec<&str>> = b
                .iter()
                .map(|&p| elem.literals.get(p as usize).map(String::as_str))
                .collect();
            let lits = lits?;
            let chars: Option<String> = lits
                .iter()
                .map(|l| l.strip_prefix('\'')?.strip_suffix('\''))
                .collect();
            match chars {
                Some(s) => format!("\"{}\"", s.replace('"', "\"\"")),
                None => format!("({})", lits.join(", ")),
            }
        }
    })
}

/// The subscribe response: the recorded hierarchy, as `--wave` declares it in the file.
pub(crate) fn hierarchy(h: &Hierarchy) -> Json {
    fn node(n: &Node) -> Json {
        match n {
            Node::Scope(s) => json!({
                "scope": s.name,
                "kind": match s.kind {
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
                },
                "component": s.component,
                "children": s.children.iter().map(node).collect::<Vec<_>>(),
            }),
            Node::Signal(s) => json!({
                "name": s.name,
                "id": s.id.0,
                "type": ty(&s.ty),
                "class": match s.class {
                    ObjClass::Signal => "signal",
                    ObjClass::Variable => "variable",
                    ObjClass::Constant => "constant",
                },
                "direction": match s.port {
                    PortDir::None => "none",
                    PortDir::In => "in",
                    PortDir::Out => "out",
                    PortDir::InOut => "inout",
                    PortDir::Buffer => "buffer",
                    PortDir::Linkage => "linkage",
                },
            }),
        }
    }
    fn ty(t: &SigType) -> Json {
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
        "scopes": h.roots.iter().map(node).collect::<Vec<_>>(),
        "skipped": skipped,
    })
}

#[test]
fn values_as_text() {
    use yavs_wave::{Bounds, EnumType};
    let v = SigType::Vector {
        elem: EnumType::std_logic(),
        bounds: Bounds {
            left: 3,
            right: 0,
            dir: Dir::Downto,
        },
        type_name: None,
    };
    assert_eq!(text(&v, &[2, 3, 0, 4]).as_deref(), Some("\"01UZ\""));
    let e = SigType::Scalar(ScalarType::Enum(EnumType::new("s", &["IDLE", "RUN"])));
    assert_eq!(text(&e, &1u32.to_le_bytes()).as_deref(), Some("RUN"));
    let t = SigType::Scalar(ScalarType::Time);
    assert_eq!(text(&t, &5i64.to_le_bytes()).as_deref(), Some("5 fs"));
    assert_eq!(text(&t, &[0]), None);
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
