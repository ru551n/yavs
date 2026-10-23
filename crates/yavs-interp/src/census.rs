//! Executed-op counting of the predecoded dispatch loop (`exec::run_ops`).
//!
//! Debug builds count the ops executed (`ops_executed`), for the work-counter tests of
//! `crates/yavs/tests/perf.rs`. The `op-census` feature (any profile) also keeps a table of ops
//! and of adjacent executed pairs (`dump`), to choose fusions from data. In a release build
//! without the feature `tick` is empty: no cost. Only interpreted ops are counted, none that run
//! as compiled code.

use crate::pre::Op;
use yavs_ir::Inst;

#[cfg(any(debug_assertions, feature = "op-census"))]
static OPS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// One op about to execute; `insts` is the function's IR, which a generic `Op::Step` names.
#[cfg_attr(not(any(debug_assertions, feature = "op-census")), allow(dead_code))]
#[inline(always)]
pub(crate) fn tick(op: &Op, insts: &[Inst]) {
    let _ = (op, insts);
    #[cfg(any(debug_assertions, feature = "op-census"))]
    {
        OPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        #[cfg(feature = "op-census")]
        table::hit(op, insts);
    }
}

/// The ops executed so far in this process; `None` in a release build without `op-census`.
pub fn ops_executed() -> Option<u64> {
    #[cfg(any(debug_assertions, feature = "op-census"))]
    return Some(OPS.load(std::sync::atomic::Ordering::Relaxed));
    #[cfg(not(any(debug_assertions, feature = "op-census")))]
    None
}

/// Print the census (the `top` most frequent ops and pairs) to stderr; nothing without
/// `op-census`.
pub fn dump(top: usize) {
    #[cfg(feature = "op-census")]
    table::dump(top);
    #[cfg(not(feature = "op-census"))]
    let _ = top;
}

#[cfg(feature = "op-census")]
mod table {
    use super::{Inst, Op};
    use std::cell::Cell;
    use std::collections::HashMap;
    use std::hash::{BuildHasherDefault, Hasher};
    use std::mem::{Discriminant, discriminant};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    /// An op by its variant; a generic step by the variant of the IR instruction it runs.
    #[derive(Clone, Copy, PartialEq, Eq, Hash)]
    enum Key {
        Op(Discriminant<Op>),
        Step(Discriminant<Inst>),
    }

    /// Multiplicative hash: the census hashes once or twice per executed op.
    #[derive(Default)]
    struct Fx(u64);
    impl Hasher for Fx {
        fn finish(&self) -> u64 {
            self.0
        }
        fn write(&mut self, b: &[u8]) {
            for &x in b {
                self.0 = (self.0.rotate_left(5) ^ u64::from(x)).wrapping_mul(0x517c_c1b7_2722_0a95);
            }
        }
    }
    type Map<K> = HashMap<K, u64, BuildHasherDefault<Fx>>;

    #[derive(Default)]
    struct Local {
        ops: Map<Key>,
        /// Nanoseconds from the end of the census work of an op to the start of the next one's
        /// (the op's own work, plus the clock reads), by op.
        ns: Map<Key>,
        pairs: Map<(Key, Key)>,
        /// A sample of each key, to name it when the table is printed.
        names: HashMap<Key, String>,
    }

    /// The tables of every thread that executed ops (ponytail: a mutex per op, uncontended with
    /// `--threads 1`; the census is for choosing fusions, not for timing).
    static ALL: Mutex<Vec<Arc<Mutex<Local>>>> = Mutex::new(Vec::new());

    thread_local! {
        static MINE: Arc<Mutex<Local>> = {
            let l = Arc::new(Mutex::new(Local::default()));
            ALL.lock().unwrap_or_else(|e| e.into_inner()).push(l.clone());
            l
        };
        static PREV: Cell<Option<Key>> = const { Cell::new(None) };
        static LAST: Cell<Option<Instant>> = const { Cell::new(None) };
    }

    pub(super) fn hit(op: &Op, insts: &[Inst]) {
        let now = Instant::now();
        let step = match *op {
            Op::Step(pc) => insts.get(pc as usize),
            _ => None,
        };
        let key = match step {
            Some(i) => Key::Step(discriminant(i)),
            None => Key::Op(discriminant(op)),
        };
        let prev = PREV.replace(Some(key));
        let spent = LAST.get().map(|t| now.duration_since(t).as_nanos() as u64);
        MINE.with(|m| {
            let mut m = m.lock().unwrap_or_else(|e| e.into_inner());
            *m.ops.entry(key).or_default() += 1;
            if let (Some(p), Some(ns)) = (prev, spent) {
                *m.ns.entry(p).or_default() += ns;
            }
            if let Some(p) = prev {
                *m.pairs.entry((p, key)).or_default() += 1;
            }
            m.names.entry(key).or_insert_with(|| {
                name_of(&match step {
                    Some(i) => format!("Step:{i:?}"),
                    None => format!("{op:?}"),
                })
            });
        });
        LAST.set(Some(Instant::now()));
    }

    /// The clock reads and the table update that sit in every op's time, per op.
    fn overhead_ns() -> f64 {
        let mut tot = 0u128;
        let n = 200_000;
        let mut last = Instant::now();
        for _ in 0..n {
            let now = Instant::now();
            tot += now.duration_since(last).as_nanos();
            last = Instant::now();
        }
        tot as f64 / n as f64
    }

    /// `Step:Load { .. }` to `Step:Load`, `LoadSlot8 { .. }` to `LoadSlot8`.
    fn name_of(s: &str) -> String {
        let end = s
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':'))
            .unwrap_or(s.len());
        s[..end].to_string()
    }

    pub(super) fn dump(top: usize) {
        // `YAVS_CENSUS_TOP=N`: print N rows of each table instead of the default.
        let top = std::env::var("YAVS_CENSUS_TOP")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(top);
        let mut ops: HashMap<Key, u64> = HashMap::new();
        let mut ns: HashMap<Key, u64> = HashMap::new();
        let mut pairs: HashMap<(Key, Key), u64> = HashMap::new();
        let mut names: HashMap<Key, String> = HashMap::new();
        for l in ALL.lock().unwrap_or_else(|e| e.into_inner()).iter() {
            let l = l.lock().unwrap_or_else(|e| e.into_inner());
            for (k, n) in &l.ops {
                *ops.entry(*k).or_default() += n;
            }
            for (k, n) in &l.ns {
                *ns.entry(*k).or_default() += n;
            }
            for (k, n) in &l.pairs {
                *pairs.entry(*k).or_default() += n;
            }
            for (k, s) in &l.names {
                names.entry(*k).or_insert_with(|| s.clone());
            }
        }
        let total: u64 = ops.values().sum();
        let name = |k: &Key| names.get(k).map_or("?", String::as_str);
        eprintln!(
            "op-census: {total} ops executed, {} distinct ops, {} distinct adjacent pairs",
            ops.len(),
            pairs.len()
        );
        let mut rows: Vec<(String, u64)> =
            ops.iter().map(|(k, n)| (name(k).to_string(), *n)).collect();
        // Ties by name: the table is the same on every run of a deterministic design.
        rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        print_rows("ops", &rows, total, top);
        let mut rows: Vec<(String, u64)> = pairs
            .iter()
            .map(|((a, b), n)| (format!("{} -> {}", name(a), name(b)), *n))
            .collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        print_rows("pairs", &rows, total, top);
        // Time by op. The clock reads are subtracted (a constant per op); ops whose time includes
        // the kernel (`Suspend`, `Ret` of a process) show it.
        let oh = overhead_ns();
        let mut trows: Vec<(String, f64, u64)> = ops
            .iter()
            .map(|(k, n)| (name(k).to_string(), ns.get(k).copied().unwrap_or(0) as f64 - oh * *n as f64, *n))
            .collect();
        let ttot: f64 = trows.iter().map(|r| r.1.max(0.0)).sum();
        trows.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        eprintln!("op-census: time (clock overhead {oh:.1} ns per op subtracted): ms, percent, ns per op, count");
        for (name, t, n) in trows.iter().take(top) {
            eprintln!(
                "op-census: {:>10.1} {:>6.2}% {:>8.1}  {n:>12}  {name}",
                t / 1e6,
                t.max(0.0) * 100.0 / ttot.max(1.0),
                t / (*n).max(1) as f64
            );
        }
    }

    fn print_rows(what: &str, rows: &[(String, u64)], total: u64, top: usize) {
        eprintln!("op-census: {what}: count, percent of ops, cumulative percent");
        let mut cum = 0;
        for (name, n) in rows.iter().take(top) {
            cum += n;
            eprintln!(
                "op-census: {n:>12} {:>6.2}% {:>6.2}%  {name}",
                *n as f64 * 100.0 / total.max(1) as f64,
                cum as f64 * 100.0 / total.max(1) as f64
            );
        }
    }
}
