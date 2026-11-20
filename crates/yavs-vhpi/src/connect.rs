//! Connectivity (#830): drivers, port connections, contributors and loads of signals, from the
//! net graph the elaborator made (`Design::net`, `design.rs`), and their transactions. The
//! kernel's collapsed ports share their actual's elements, so everything here is by elaborated
//! object first and by kernel element second. Safe Rust only.

// VHPI names are matched as written in the C standard.
#![allow(non_upper_case_globals)]

use std::collections::{HashMap, HashSet};

use yavs_elab::ObjId;
use yavs_kernel::{DriverId, Hook, ProcId, Range, SigRef, SignalId, Time, Value};

use crate::design::{Layout, NodeId, Origin};
use crate::model::{Fault, Object, State};
use crate::sys::*;
use crate::value::{Read, Scalar};

/// A driver of a process: the elements of one kernel signal it drives.
pub(crate) struct NetDriver {
    pub(crate) proc: u32,
    pub(crate) sig: SignalId,
    pub(crate) range: Range,
    /// The node of the object the process's assignments name.
    pub(crate) owner: Option<NodeId>,
    pub(crate) ty: Scalar,
}

/// A port association: the actual feeds an `in` or `inout` formal, an `out`, `inout` or `buffer`
/// formal its actual.
pub(crate) struct Conn {
    pub(crate) formal: (ObjId, SignalId, Range),
    pub(crate) actual: (ObjId, SignalId, Range),
    pub(crate) mode: vhpiModeT,
}

/// Static facts about a leaf.
pub(crate) struct LeafInfo {
    pub(crate) resolved: bool,
    /// A whole composite is its one basic signal.
    pub(crate) whole: bool,
    pub(crate) sig_kind: vhpiSigKindT,
    pub(crate) resol_func: Option<NodeId>,
}

#[derive(Default)]
pub(crate) struct Net {
    pub(crate) drivers: Vec<NetDriver>,
    pub(crate) conns: Vec<Conn>,
    /// The associations each object is a side of.
    pub(crate) conns_of: HashMap<ObjId, Vec<u32>>,
    pub(crate) leaves: Vec<LeafInfo>,
    /// The node of each elaborated object that has one (component ports have none).
    pub(crate) obj_nodes: HashMap<ObjId, NodeId>,
    /// The drivers of each object, and of each process.
    pub(crate) of_obj: HashMap<ObjId, Vec<u32>>,
    pub(crate) of_proc: HashMap<u32, Vec<u32>>,
    /// The region node of a kernel process, and back.
    pub(crate) proc_node: HashMap<u32, NodeId>,
    pub(crate) node_proc: HashMap<NodeId, u32>,
    /// The processes and equivalent processes that read each signal or port declaration
    /// (#143), in elaboration order.
    pub(crate) readers: HashMap<NodeId, Vec<NodeId>>,
}

/// A transaction of a driver, as `vhpiTransactions` found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Txn {
    pub(crate) driver: u32,
    pub(crate) time: Time,
    pub(crate) range: Range,
    pub(crate) value: Option<Value>,
}

/// The part of `r` that lies in `from`, in the coordinates of `to` (the other side of a port
/// association; a converted association has no such map and is taken whole).
fn across(r: Range, from: Range, to: Range) -> Range {
    if from.len != to.len || !from.overlaps(r) {
        return to;
    }
    let lo = r.offset.max(from.offset);
    let hi = r.end().min(from.end());
    Range::new(to.offset + (lo - from.offset), hi - lo)
}

impl State {
    /// The elaborated object of a signal, a port or a sub-element and its kernel elements.
    fn net_of(&self, o: &Object) -> Option<(ObjId, Vec<(SignalId, Range)>)> {
        let base = match o {
            Object::Node(n) | Object::Elem { base: n, .. } => *n,
            _ => return None,
        };
        let Origin::Signal { obj, .. } = self.design.node(base).origin else {
            return None;
        };
        Some((obj, self.watch(o)?.into_vec()))
    }

    /// The drivers of `obj` on `elems`.
    fn drivers_on(&self, obj: ObjId, elems: &[(SignalId, Range)]) -> Vec<u32> {
        let net = &self.design.net;
        let all = net.of_obj.get(&obj).map_or(&[][..], |v| &v[..]);
        (all.iter().copied())
            .filter(|&d| {
                let d = &net.drivers[d as usize];
                (elems.iter()).any(|&(s, r)| s == d.sig && r.overlaps(d.range))
            })
            .collect()
    }

    /// The port associations of `obj` on `elems` that `keep` picks (an association, whether
    /// `obj` is its formal).
    fn conns_of(
        &self,
        obj: ObjId,
        elems: &[(SignalId, Range)],
        keep: impl Fn(&Conn, bool) -> bool,
    ) -> Vec<u32> {
        let hit = |&(o, s, r): &(ObjId, SignalId, Range)| {
            o == obj && elems.iter().any(|&(x, e)| x == s && e.overlaps(r))
        };
        let net = &self.design.net;
        (net.conns_of.get(&obj).into_iter().flatten().copied())
            .filter(|&i| {
                let c = &net.conns[i as usize];
                hit(&c.formal) && keep(c, true) || hit(&c.actual) && keep(c, false)
            })
            .collect()
    }

    /// The associations that feed `obj` (`up`) or that it feeds.
    fn feeds(&self, obj: ObjId, elems: &[(SignalId, Range)], up: bool) -> Vec<u32> {
        self.conns_of(obj, elems, |c, formal| {
            let inward = matches!(c.mode, vhpiInMode | vhpiInoutMode);
            let outward = c.mode != vhpiInMode;
            if formal == up { inward } else { outward }
        })
    }

    /// The other end of association `c` from `obj`, on `elems`.
    fn other_end(
        &self,
        c: u32,
        obj: ObjId,
        elems: &[(SignalId, Range)],
    ) -> (ObjId, Vec<(SignalId, Range)>) {
        let c = &self.design.net.conns[c as usize];
        let (from, to) = if c.formal.0 == obj {
            (c.formal, c.actual)
        } else {
            (c.actual, c.formal)
        };
        let mapped = (elems.iter())
            .filter(|&&(s, r)| s == from.1 && r.overlaps(from.2))
            .map(|&(_, r)| (to.1, across(r, from.2, to.2)))
            .collect();
        (to.0, mapped)
    }

    /// Everything reachable from `start` through the associations that feed it (`up`) or that
    /// it feeds, `start` included: each object with its elements.
    fn reach(
        &self,
        start: (ObjId, Vec<(SignalId, Range)>),
        up: bool,
    ) -> Vec<(ObjId, Vec<(SignalId, Range)>)> {
        let mut seen: HashSet<(ObjId, u32, u32, u32)> = HashSet::new();
        let (mut out, mut todo) = (Vec::new(), vec![start]);
        while let Some((o, elems)) = todo.pop() {
            let fresh: Vec<_> = (elems.iter().copied())
                .filter(|&(s, r)| seen.insert((o, s.0, r.offset, r.len)))
                .collect();
            if fresh.is_empty() {
                continue;
            }
            for c in self.feeds(o, &fresh, up) {
                todo.push(self.other_end(c, o, &fresh));
            }
            out.push((o, fresh));
        }
        out
    }

    /// `vhpiDrivers`, `vhpiLocalContributors`, `vhpiOptimizedContributors`, `vhpiLocalLoads`,
    /// `vhpiOptimizedLoads`, `vhpiDrivenSigs`, `vhpiBasicSignals`, `vhpiTransactions`;
    /// `None`: the relation does not apply to `o`.
    pub(crate) fn net_iter(&self, rel: vhpiOneToManyT, o: &Object) -> Option<Vec<Object>> {
        let net = &self.design.net;
        let drivers = |ds: Vec<u32>| ds.into_iter().map(Object::NetDriver);
        let readers = |n: NodeId| net.readers.get(&n).into_iter().flatten().copied();
        let conn = |c: u32, up: bool| Object::Conn {
            conn: c,
            inward: up,
        };
        Some(match rel {
            vhpiDrivers => {
                let (obj, elems) = self.net_of(o)?;
                drivers(self.drivers_on(obj, &elems)).collect()
            }
            vhpiLocalContributors => {
                let (obj, elems) = self.net_of(o)?;
                let mut v: Vec<Object> = drivers(self.drivers_on(obj, &elems)).collect();
                v.extend(self.feeds(obj, &elems, true).into_iter().map(|c| {
                    let inward = net.conns[c as usize].formal.0 == obj;
                    conn(c, inward)
                }));
                v
            }
            vhpiOptimizedContributors => {
                let start = self.net_of(o)?;
                let mut ds: Vec<u32> = (self.reach(start, true).iter())
                    .flat_map(|(x, e)| self.drivers_on(*x, e))
                    .collect();
                ds.sort_unstable();
                ds.dedup();
                drivers(ds).collect()
            }
            vhpiLocalLoads => {
                let (obj, elems) = self.net_of(o)?;
                let mut v: Vec<Object> = (self.feeds(obj, &elems, false).into_iter())
                    .map(|c| {
                        let inward = net.conns[c as usize].actual.0 == obj;
                        conn(c, inward)
                    })
                    .collect();
                if let Object::Node(n) | Object::Elem { base: n, .. } = o {
                    v.extend(readers(*n).map(Object::Node));
                }
                v
            }
            vhpiOptimizedLoads => {
                let start = self.net_of(o)?;
                let first = start.0;
                let reached = self.reach(start, false);
                let mut nodes: Vec<NodeId> = (reached.iter())
                    .filter(|(x, _)| *x != first)
                    .filter_map(|(x, _)| net.obj_nodes.get(x).copied())
                    .collect();
                nodes.sort_unstable_by_key(|n| n.0);
                nodes.dedup();
                // Then the processes that read any object of the chain, the start included.
                let mut procs: Vec<NodeId> = (reached.iter())
                    .filter_map(|(x, _)| net.obj_nodes.get(x).copied())
                    .flat_map(readers)
                    .collect();
                procs.sort_unstable_by_key(|n| n.0);
                procs.dedup();
                nodes.into_iter().chain(procs).map(Object::Node).collect()
            }
            vhpiDrivenSigs => {
                let &Object::Node(n) = o else {
                    return None;
                };
                let p = net.node_proc.get(&n)?;
                let mut nodes: Vec<NodeId> = (net.of_proc.get(p).into_iter().flatten())
                    .filter_map(|&d| net.drivers[d as usize].owner)
                    .collect();
                nodes.sort_unstable_by_key(|n| n.0);
                nodes.dedup();
                nodes.into_iter().map(Object::Node).collect()
            }
            vhpiBasicSignals => {
                let (base, path) = match o {
                    Object::Node(n) => (*n, Vec::new()),
                    Object::Elem { base, path } => (*base, path.to_vec()),
                    _ => return None,
                };
                self.net_of(o)?;
                let l = self.design.elem(base, &path)?.layout;
                let mut out = Vec::new();
                // A composite resolved as a whole is one basic signal (LRM 14.7.3.1).
                if self.is_whole(o) {
                    return Some(vec![o.clone()]);
                }
                basic(base, &path, l, &mut out);
                out
            }
            vhpiTransactions => {
                let &Object::NetDriver(d) = o else {
                    return None;
                };
                self.driver_txns(d)
            }
            _ => return None,
        })
    }

    /// The pending transactions of net driver `d`; none outside a simulation callback.
    fn driver_txns(&self, d: u32) -> Vec<Object> {
        let nd = &self.design.net.drivers[d as usize];
        crate::abi::with_hook(|h: &mut Hook<'_>| {
            kernel_drivers(h, nd)
                .into_iter()
                .flat_map(|k| h.driver_transactions(k))
                .filter(|t| t.1.overlaps(nd.range))
                .map(|(time, range, value)| {
                    Object::Transaction(Box::new(Txn {
                        driver: d,
                        time,
                        range,
                        value,
                    }))
                })
                .collect()
        })
        .unwrap_or_default()
    }

    /// `vhpiSimNet`, `vhpiSignal`, `vhpiEqProcessStmt` of a driver, `vhpiFormal`, `vhpiActual`,
    /// `vhpiResolFunc`. `Err(false)`: `o` has no such related object.
    pub(crate) fn net_one(&self, kind: vhpiOneToOneT, o: &Object) -> Result<Object, bool> {
        let net = &self.design.net;
        let node = |x: ObjId| {
            net.obj_nodes
                .get(&x)
                .copied()
                .map(Object::Node)
                .ok_or(false)
        };
        match (kind, o) {
            (vhpiSignal, Object::NetDriver(d)) => net.drivers[*d as usize]
                .owner
                .map(Object::Node)
                .ok_or(false),
            (vhpiEqProcessStmt, Object::NetDriver(d)) => net
                .proc_node
                .get(&net.drivers[*d as usize].proc)
                .map(|&n| Object::Node(n))
                .ok_or(false),
            (vhpiFormal, Object::Conn { conn, .. }) => node(net.conns[*conn as usize].formal.0),
            (vhpiActual, Object::Conn { conn, .. }) => node(net.conns[*conn as usize].actual.0),
            (vhpiSimNet, _) => {
                let (mut obj, mut elems) = self.net_of(o).ok_or(false)?;
                // Up the hierarchy through the actuals: the net is the outermost object.
                let mut seen = HashSet::new();
                while seen.insert(obj) {
                    let up = self.conns_of(obj, &elems, |_, formal| formal);
                    let Some(&c) = up.first() else {
                        break;
                    };
                    (obj, elems) = self.other_end(c, obj, &elems);
                }
                node(obj)
            }
            (vhpiResolFunc, _) => {
                let at = self.at(o, "vhpi_handle").map_err(|_| false)?;
                let Origin::Signal { .. } = at.node.origin else {
                    return Err(false);
                };
                (net.leaves.get(at.leaf() as usize))
                    .and_then(|l| l.resol_func)
                    .map(Object::Node)
                    .ok_or(false)
            }
            _ => Err(false),
        }
    }

    /// The whole composite `o` is, or that its first leaf belongs to and it covers entirely.
    fn is_whole(&self, o: &Object) -> bool {
        let Ok(at) = self.at(o, "vhpi_get") else {
            return false;
        };
        let n = crate::model::leaves(at.l) as usize;
        let leaves = &self.design.net.leaves;
        !matches!(at.l, Layout::Scalar(_))
            && (at.leaf() as usize..at.leaf() as usize + n)
                .all(|l| leaves.get(l).is_some_and(|x| x.whole))
    }

    /// The connectivity properties of a signal, port or sub-element, and of drivers and
    /// transactions; `None`: `prop` does not apply.
    pub(crate) fn net_prop(&self, o: &Object, prop: vhpiIntPropertyT) -> Option<vhpiIntT> {
        let net = &self.design.net;
        if let Object::NetDriver(d) = o {
            return match prop {
                vhpiSizeP => Some(net.drivers[*d as usize].range.len as vhpiIntT),
                _ => None,
            };
        }
        if let Object::Transaction(t) = o {
            return match prop {
                vhpiIsNullP => Some(vhpiIntT::from(t.value.is_none())),
                vhpiSizeP => Some(t.range.len as vhpiIntT),
                _ => None,
            };
        }
        self.net_of(o)?;
        let at = self.at(o, "vhpi_get").ok()?;
        let first = at.leaf() as usize;
        let n = crate::model::leaves(at.l) as usize;
        let leaves = net.leaves.get(first..first + n)?;
        Some(match prop {
            vhpiIsResolvedP => vhpiIntT::from(leaves.iter().any(|l| l.resolved)),
            vhpiIsBasicP => vhpiIntT::from(matches!(at.l, Layout::Scalar(_)) || self.is_whole(o)),
            vhpiSigKindP => leaves.first()?.sig_kind,
            vhpiIsForcedP => {
                let elems = self.watch(o)?;
                let forced = crate::abi::with_hook(|h: &mut Hook<'_>| {
                    (elems.iter()).any(|&(s, r)| {
                        h.forced(SigRef {
                            signal: s,
                            range: Some(r),
                        }) == Ok(true)
                    })
                });
                vhpiIntT::from(forced == Some(true))
            }
            _ => return None,
        })
    }

    /// `vhpi_get_value` of a driver (its current value) or a transaction (its value).
    pub(crate) fn net_read(&self, o: &Object) -> Result<Read, (Fault, String)> {
        let misuse = |m: &str| (Fault::Misuse, format!("vhpi_get_value: {m}"));
        let (nd, v) = match o {
            Object::NetDriver(d) => {
                let nd = &self.design.net.drivers[*d as usize];
                let v = crate::abi::with_hook(|h: &mut Hook<'_>| {
                    let k = *kernel_drivers(h, nd).first()?;
                    let (_, kr) = h.driver_target(k)?;
                    Some(slice(&h.driver_value(k)?, kr, nd.range))
                })
                .ok_or_else(|| misuse("driver values are read in simulation callbacks"))?
                .ok_or_else(|| misuse("this is a bug in yavs: no kernel driver"))?;
                (nd, v)
            }
            Object::Transaction(t) => {
                let nd = &self.design.net.drivers[t.driver as usize];
                let v = t
                    .value
                    .as_ref()
                    .ok_or_else(|| misuse("a null transaction has no value"))?;
                (nd, v.clone())
            }
            _ => return Err(misuse("the object has no value")),
        };
        let n = match o {
            Object::Transaction(t) => t.range.len,
            _ => nd.range.len,
        };
        Read::from_kernel(nd.ty.clone(), n > 1, &v).ok_or_else(|| {
            misuse("this is a bug in yavs: the kernel value does not match the type")
        })
    }

    /// The callbacks of `vhpiCbTransaction` on a driver with a transaction due now: asked before
    /// the update of the cycle, the signals' side after it ([`State::transaction_due`]).
    pub(crate) fn note_driver_due(&mut self, h: &Hook<'_>) {
        if !self.txn_due.is_empty() {
            self.txn_due.clear();
        }
        for id in self.cbs.due(vhpiCbTransaction) {
            if let Some(Object::NetDriver(d)) = self.cbs.get(id).and_then(|cb| cb.obj.as_ref()) {
                let nd = &self.design.net.drivers[*d as usize];
                if kernel_drivers(h, nd).into_iter().any(|k| h.driver_due(k)) {
                    self.txn_due.insert(id);
                }
            }
        }
    }

    /// The enabled `vhpiCbTransaction` callbacks with a transaction in this cycle, after the
    /// update. An element is active when a source of it had a transaction (LRM 14.7.3.1).
    pub(crate) fn transaction_due(&mut self, h: &Hook<'_>) -> Vec<u64> {
        let active = |s: SignalId, r: Range| {
            h.active(SigRef {
                signal: s,
                range: Some(r),
            }) == Ok(true)
        };
        let mut ids = Vec::new();
        for id in self.cbs.due(vhpiCbTransaction) {
            let Some(cb) = self.cbs.get(id) else {
                continue;
            };
            let hit = match &cb.obj {
                Some(Object::NetDriver(d)) => {
                    let nd = &self.design.net.drivers[*d as usize];
                    self.txn_due.contains(&id)
                        || kernel_drivers(h, nd)
                            .into_iter()
                            .any(|k| h.driver_is_sole(k))
                            && active(nd.sig, nd.range)
                }
                _ => cb.watching().iter().any(|&(s, r)| active(s, r)),
            };
            if hit {
                ids.push(id);
            }
        }
        ids
    }
}

/// The scalars under `l`: the basic signals of an object.
fn basic(base: NodeId, path: &[u64], l: &Layout, out: &mut Vec<Object>) {
    if matches!(l, Layout::Scalar(_) | Layout::Open) {
        out.push(if path.is_empty() {
            Object::Node(base)
        } else {
            Object::Elem {
                base,
                path: path.into(),
            }
        });
        return;
    }
    // ponytail: the whole list is built; a lazy iterator if a plugin walks a huge memory.
    for i in 0..l.count() {
        if let Some((sub, _, _)) = l.step(i) {
            let mut p = path.to_vec();
            p.push(i);
            basic(base, &p, sub, out);
        }
    }
}

/// The kernel drivers behind `d` (one, or the merged driver of a pack).
fn kernel_drivers(h: &Hook<'_>, d: &NetDriver) -> Vec<DriverId> {
    h.drivers_of(
        ProcId(d.proc),
        SigRef {
            signal: d.sig,
            range: Some(d.range),
        },
    )
}

/// Elements `r` of the value `v` of elements `of`.
fn slice(v: &Value, of: Range, r: Range) -> Value {
    if of == r {
        return v.clone();
    }
    let (at, n) = ((r.offset - of.offset) as usize, r.len as usize);
    match v {
        Value::Logic(l) => Value::Logic(l.slice(at, n)),
        Value::Bytes(b) => {
            let es = b.len() / of.len.max(1) as usize;
            Value::Bytes(b[at * es..(at + n) * es].to_vec())
        }
    }
}
