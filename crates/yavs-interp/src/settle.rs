//! Census for settle-phase deferral (#1064): per process, what one activation reads, what each
//! driven range depends on (data and control), and what stops the process from being a pure
//! function of the signals it waits on. An abstract run over the finished IR of the code the
//! process runs now (its specialised copy when it has one), from the instruction after its one
//! `Suspend` to that `Suspend`. Anything it cannot place is a reason, never an assumption.
//! ponytail: a measuring tool with a text output, out of the hot paths; a purity fact for real
//! use would be this pass over the copy when it is made, kept with it.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use yavs_ir::gather::{self, Plan};
use yavs_ir::{AddrSpace, Func, FuncId, FuncKind, Inst, Mode, Reg, Role, Ty, Width};
use yavs_kernel::Elem;

use crate::exec::Machine;

/// Taint: an interned sorted set of read numbers; number 0 is `STATE`, a value the last
/// activation left in the process's memory.
const STATE: u32 = 0;
const NONE: u32 = 0;

#[derive(Default)]
struct Sets {
    list: Vec<Vec<u32>>,
    map: HashMap<Vec<u32>, u32>,
    unions: HashMap<(u32, u32), u32>,
}

impl Sets {
    fn new() -> Sets {
        let mut s = Sets::default();
        s.intern(Vec::new());
        s
    }
    fn intern(&mut self, v: Vec<u32>) -> u32 {
        if let Some(&i) = self.map.get(&v) {
            return i;
        }
        let i = self.list.len() as u32;
        self.list.push(v.clone());
        self.map.insert(v, i);
        i
    }
    fn one(&mut self, x: u32) -> u32 {
        self.intern(vec![x])
    }
    fn u(&mut self, a: u32, b: u32) -> u32 {
        if a == b || b == NONE {
            return a;
        }
        if a == NONE {
            return b;
        }
        let k = (a.min(b), a.max(b));
        if let Some(&c) = self.unions.get(&k) {
            return c;
        }
        let (x, y) = (&self.list[a as usize], &self.list[b as usize]);
        let mut o: Vec<u32> = x.iter().chain(y).copied().collect();
        o.sort_unstable();
        o.dedup();
        let c = self.intern(o);
        self.unions.insert(k, c);
        c
    }
    fn state(&self, t: u32) -> bool {
        self.list[t as usize].first() == Some(&STATE)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum V {
    Bot,
    Int(i64),
    /// An address in object `o`; not `exact`: somewhere in the extent of the place at `off`.
    Addr {
        o: u32,
        off: i64,
        exact: bool,
    },
    /// Read-only memory: 0 constant `id`, 1 instance data, 2 package; `off` -1 unknown.
    Ro {
        space: u8,
        id: u32,
        off: i64,
    },
    /// `off` -1: somewhere in the signal, by an index.
    Sig {
        sig: u32,
        off: i64,
        len: i64,
    },
    /// A value or address a call returned: what its memory holds depends on `ct`; the
    /// register's own taint is the shape (an `ieee` native's result length follows from the
    /// lengths and integer arguments it got).
    Res {
        ct: u32,
    },
    Top,
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Av {
    v: V,
    t: u32,
}

const BOT: Av = Av { v: V::Bot, t: NONE };

/// One memory object: a taint per byte, and per plane block (32 bytes, 64 logic elements) that
/// logic instructions touched a taint per element.
#[derive(Clone, PartialEq, Default)]
struct Mem {
    b: Vec<u32>,
    ov: BTreeMap<usize, Box<[u32; 64]>>,
}

#[derive(Clone, PartialEq)]
struct St {
    regs: BTreeMap<u32, Av>,
    mem: Vec<Rc<Mem>>,
    driven: Rc<BTreeSet<(u32, u32, u32)>>,
}

#[derive(Default)]
struct Facts {
    reasons: BTreeMap<String, u32>,
    pcs: BTreeMap<String, Vec<usize>>,
    state_vars: BTreeSet<String>,
    written: Vec<Vec<bool>>,
}

struct An<'a> {
    m: &'a Machine,
    f: &'a Func,
    sets: Sets,
    reads: Vec<(u32, u32, u32)>,
    read_ids: HashMap<(u32, u32, u32), u32>,
    global: Vec<Option<V>>,
    objs: Vec<usize>,
    alloc_obj: HashMap<usize, u32>,
    outs: BTreeMap<(u32, u32, u32), u32>,
    conds: Vec<u32>,
    any_cond: u32,
    facts: Facts,
    state_t: u32,
    plans: HashMap<(u32, Vec<u8>), Option<Rc<Plan>>>,
    waits: BTreeSet<(u32, u32, u32)>,
    hist: BTreeSet<(u32, u32, u32)>,
    pc: usize,
    /// The successor a branch on a constant takes (0: the first).
    taken: Option<usize>,
    read_pcs: Vec<usize>,
    /// Frame offsets holding the address of a heap object (a variable outside the frame), read
    /// from the suspended process: the object and the offset in it.
    heap_slots: HashMap<i64, (u32, i64)>,
    persistent: Vec<bool>,
    written_before: Vec<Vec<bool>>,
    /// Every frame offset the code takes an address of, and every local's start.
    frame_bases: BTreeSet<i64>,
}

fn effect(i: &Inst) -> Option<&'static str> {
    Some(match i {
        Inst::Report { .. } => "report",
        Inst::Output { .. } => "output",
        Inst::Now { .. } | Inst::Delta { .. } | Inst::TimedOut { .. } => "time",
        Inst::WaitFor { .. } | Inst::WaitAt { .. } => "timed_wait",
        Inst::SigAttr { .. }
        | Inst::SigLastValue { .. }
        | Inst::Edge { .. }
        | Inst::WaitEdge { .. } => "history",
        Inst::Driving { .. }
        | Inst::DrivingValue { .. }
        | Inst::SigDriving { .. }
        | Inst::SigDrivingValue { .. } => "driving",
        Inst::Force { .. } | Inst::Release { .. } => "force",
        Inst::Disconnect { .. } | Inst::DisconnectSlot { .. } | Inst::DriveSlot { .. } => {
            "disconnect"
        }
        Inst::SigParts { .. } => "sig_parts",
        Inst::Cover { .. } | Inst::CoverIn { .. } | Inst::CoverCond { .. } => "cover",
        Inst::CallForeign { .. } | Inst::CallIndirect { .. } | Inst::CallProt { .. } => {
            "call_other"
        }
        Inst::PkgAlloc { .. } | Inst::New { .. } | Inst::NewFill { .. } | Inst::Free { .. } => {
            "heap"
        }
        _ => return None,
    })
}

/// The natives of `std_logic_1164`-level logic never report.
fn may_warn(sym: &str) -> bool {
    !(sym.starts_with("yavs.ieee.std_logic_1164") || sym.starts_with("yavs.attr."))
}

impl An<'_> {
    fn why(&mut self, r: &str) {
        *self.facts.reasons.entry(r.to_string()).or_insert(0) += 1;
        let p = self.facts.pcs.entry(r.to_string()).or_default();
        if p.len() < 4 && !p.contains(&self.pc) {
            p.push(self.pc);
        }
    }

    fn u(&mut self, a: u32, b: u32) -> u32 {
        self.sets.u(a, b)
    }

    fn read_id(&mut self, k: (u32, u32, u32)) -> u32 {
        if let Some(&i) = self.read_ids.get(&k) {
            return i;
        }
        let i = self.reads.len() as u32 + 1;
        self.reads.push(k);
        self.read_pcs.push(self.pc);
        self.read_ids.insert(k, i);
        i
    }

    /// The kernel range of a signal value (`off` -1: the whole signal).
    fn krange(&self, sig: u32, off: i64, len: i64) -> Option<(u32, u32, u32)> {
        let s = self.m.inst.sigs.get(sig as usize)?;
        Some(if off < 0 || len < 0 {
            (s.signal.0, s.off, s.ty.len)
        } else {
            (s.signal.0, s.off + off as u32, len as u32)
        })
    }

    fn read_sig(&mut self, a: Av) -> u32 {
        if let V::Sig { sig, off, len } = a.v
            && let Some(k) = self.krange(sig, off, len)
        {
            let i = self.read_id(k);
            let t = self.sets.one(i);
            return self.u(t, a.t);
        }
        self.why("sig_unknown");
        a.t
    }

    /// A read of a signal's history (`'event`, `'last_value`, an edge, ...): kept apart, since
    /// a removed intermediate value changes it in later time steps too.
    fn hist_read(&mut self, a: Av) {
        let k = match a.v {
            V::Sig { sig, off, len } => self.krange(sig, off, len),
            _ => None,
        };
        match k {
            Some(k) => {
                self.hist.insert(k);
            }
            None => self.why("history_unknown"),
        }
    }

    fn get(&self, regs: &HashMap<u32, Av>, r: Reg) -> Av {
        if let Some(Some(v)) = self.global.get(r.0 as usize) {
            return Av { v: *v, t: NONE };
        }
        regs.get(&r.0).copied().unwrap_or(BOT)
    }

    /// The extent of the place at `off` of object `o`.
    fn extent(&mut self, o: u32, off: i64) -> (usize, usize) {
        let size = self.objs[o as usize];
        if o == 0 {
            for l in &self.f.locals {
                let (s, e) = (l.slot as i64, l.slot as i64 + l.size as i64);
                if off >= s && off < e {
                    return (s as usize, (e as usize).min(size));
                }
            }
            // A temporary of the optimiser: up to the next place the code names.
            let next = (self.frame_bases.range(off + 1..).next().copied())
                .map_or(size, |n| (n as usize).min(size));
            self.why("coarse_frame");
            return (off.max(0) as usize, next);
        }
        (0, size)
    }

    /// The bytes an access of `n` bytes may touch, and whether it surely touches all of them.
    fn span(&mut self, o: u32, off: i64, exact: bool, n: Option<i64>) -> (usize, usize, bool) {
        let size = self.objs[o as usize];
        match (exact, n) {
            (true, Some(n)) if off >= 0 && (off + n) as usize <= size => {
                (off as usize, (off + n) as usize, true)
            }
            _ if size <= 1 => (0, size, false),
            _ => {
                let (a, b) = self.extent(o, off);
                (a, b, false)
            }
        }
    }

    fn note_state(&mut self, o: u32, x: usize, y: usize, t: u32) {
        if t != NONE && self.sets.state(t) {
            self.why("state_read");
            if o == 0 {
                for l in &self.f.locals {
                    if (l.slot as usize) < y && (l.slot + l.size) as usize > x {
                        self.facts.state_vars.insert(l.name.clone());
                    }
                }
            }
        }
    }

    fn mark_written(&mut self, o: u32, x: usize, y: usize) {
        if let Some(w) = self.facts.written.get_mut(o as usize) {
            let y = y.min(w.len());
            for b in &mut w[x.min(y)..y] {
                *b = true;
            }
        }
    }

    fn bytes_rd(&mut self, st: &St, o: u32, x: usize, y: usize) -> u32 {
        let mem = Rc::clone(&st.mem[o as usize]);
        let mut t = NONE;
        let mut last = NONE;
        for &b in &mem.b[x..y] {
            if b != last {
                t = self.u(t, b);
                last = b;
            }
        }
        for (_, e) in mem.ov.range(x.saturating_sub(31)..y) {
            for &b in e.iter() {
                if b != last {
                    t = self.u(t, b);
                    last = b;
                }
            }
        }
        self.note_state(o, x, y, t);
        t
    }

    fn bytes_wr(&mut self, st: &mut St, o: u32, x: usize, y: usize, t: u32, strong: bool) {
        self.mark_written(o, x, y);
        let mem = Rc::make_mut(&mut st.mem[o as usize]);
        for b in &mut mem.b[x..y] {
            *b = if strong { t } else { self.sets.u(*b, t) };
        }
        let keys: Vec<usize> = mem
            .ov
            .range(x.saturating_sub(31)..y)
            .map(|(&k, _)| k)
            .collect();
        for k in keys {
            if strong && k >= x && k + 32 <= y {
                mem.ov.remove(&k);
            } else if let Some(e) = mem.ov.get_mut(&k) {
                for b in e.iter_mut() {
                    *b = self.sets.u(*b, t);
                }
            }
        }
    }

    /// Elements `i .. i + n` of the logic array whose first plane block is at `base`.
    fn elems(&mut self, base: i64, i: i64, n: i64) -> Vec<(usize, std::ops::Range<usize>)> {
        let mut v = Vec::new();
        let (mut e, end) = (i, i + n);
        while e < end {
            let blk = e / 64;
            let to = end.min((blk + 1) * 64);
            v.push((
                (base + blk * 32) as usize,
                (e % 64) as usize..(to - blk * 64) as usize,
            ));
            e = to;
        }
        v
    }

    fn ov_init(&mut self, st: &St, o: u32, k: usize) -> [u32; 64] {
        if let Some(e) = st.mem[o as usize].ov.get(&k) {
            return **e;
        }
        let y = (k + 32).min(st.mem[o as usize].b.len());
        let mut t = NONE;
        for k2 in k..y {
            let b = st.mem[o as usize].b[k2];
            t = self.u(t, b);
        }
        [t; 64]
    }

    /// A read of logic elements; an address or range not known reads the whole place.
    fn erd(&mut self, st: &St, a: V, i: Option<i64>, n: Option<i64>) -> u32 {
        match (a, i, n) {
            (
                V::Addr {
                    o,
                    off,
                    exact: true,
                },
                Some(i),
                Some(n),
            ) if off >= 0
                && n >= 0
                && (off + (i + n + 63) / 64 * 32) as usize <= self.objs[o as usize] =>
            {
                let mut t = NONE;
                for (k, r) in self.elems(off, i, n) {
                    let e = self.ov_init(st, o, k);
                    for &b in &e[r] {
                        t = self.u(t, b);
                    }
                }
                self.note_state(o, off as usize, (off + (i + n + 63) / 64 * 32) as usize, t);
                t
            }
            _ => self.mem_read(st, a, None),
        }
    }

    /// A write of logic elements, strong where the elements are known.
    fn ewr(&mut self, st: &mut St, a: V, i: Option<i64>, n: Option<i64>, t: u32) {
        match (a, i, n) {
            (
                V::Addr {
                    o,
                    off,
                    exact: true,
                },
                Some(i),
                Some(n),
            ) if off >= 0
                && n >= 0
                && (off + (i + n + 63) / 64 * 32) as usize <= self.objs[o as usize] =>
            {
                for (k, r) in self.elems(off, i, n) {
                    let mut e = self.ov_init(st, o, k);
                    for b in &mut e[r] {
                        *b = t;
                    }
                    self.mark_written(o, k, k + 32);
                    Rc::make_mut(&mut st.mem[o as usize])
                        .ov
                        .insert(k, Box::new(e));
                }
            }
            _ => self.weak_write(st, a, None, t),
        }
    }

    fn mem_read(&mut self, st: &St, a: V, n: Option<i64>) -> u32 {
        match a {
            V::Addr { o, off, exact } => {
                let (x, y, _) = self.span(o, off, exact, n);
                self.bytes_rd(st, o, x, y)
            }
            V::Ro { .. } => NONE,
            V::Res { ct } => ct,
            _ => {
                self.why("read_through_unknown_pointer");
                NONE
            }
        }
    }

    fn mem_write(&mut self, st: &mut St, a: V, n: Option<i64>, t: u32) {
        match a {
            V::Addr { o, off, exact } => {
                let (x, y, strong) = self.span(o, off, exact, n);
                self.bytes_wr(st, o, x, y, t, strong);
            }
            V::Ro { .. } => self.why("write_read_only"),
            _ => self.why("write_through_unknown_pointer"),
        }
    }

    fn weak_write(&mut self, st: &mut St, a: V, n: Option<i64>, t: u32) {
        match a {
            V::Addr { o, off, exact } => {
                let (x, y, _) = self.span(o, off, exact && n.is_some(), n);
                self.bytes_wr(st, o, x, y, t, false);
            }
            V::Ro { .. } => self.why("write_read_only"),
            _ => self.why("write_through_unknown_pointer"),
        }
    }

    fn copy_bytes(&mut self, st: &mut St, d: V, s: V, n: Option<i64>, base: u32) {
        if let (
            V::Addr {
                o: dob,
                off: doff,
                exact: true,
            },
            V::Addr {
                o: sob,
                off: soff,
                exact: true,
            },
            Some(n),
        ) = (d, s, n)
        {
            let (a, b, ds) = self.span(dob, doff, true, Some(n));
            let (x, y, ss) = self.span(sob, soff, true, Some(n));
            if ds && ss && b - a == y - x {
                let _ = self.bytes_rd(st, sob, x, y);
                let src: Vec<u32> = st.mem[sob as usize].b[x..y].to_vec();
                let ovs: Vec<(usize, [u32; 64])> = (st.mem[sob as usize].ov.range(x..y))
                    .filter(|(k, _)| **k + 32 <= y)
                    .map(|(&k, e)| (k, **e))
                    .collect();
                self.bytes_wr(st, dob, a, b, NONE, true);
                let mut tb: Vec<u32> = Vec::with_capacity(src.len());
                for t in src {
                    tb.push(self.sets.u(t, base));
                }
                let mem = Rc::make_mut(&mut st.mem[dob as usize]);
                mem.b[a..b].copy_from_slice(&tb);
                for (k, mut e) in ovs {
                    for b in e.iter_mut() {
                        *b = self.sets.u(*b, base);
                    }
                    Rc::make_mut(&mut st.mem[dob as usize])
                        .ov
                        .insert(k - x + a, Box::new(e));
                }
                return;
            }
        }
        let t = self.mem_read(st, s, n);
        let t = self.u(t, base);
        self.mem_write(st, d, n, t);
    }

    fn plan(&mut self, tab: yavs_ir::ConstId, desc: V) -> Option<Rc<Plan>> {
        let prog = &self.m.rt.prog;
        let bytes = match prog.consts.get(tab)? {
            yavs_ir::ConstData::Bytes(b) => b.clone(),
            _ => return None,
        };
        let (ty, _) = gather::entries(&bytes)?;
        let ds = Ty::desc_size(&prog.types, ty)? as usize;
        let mut d = vec![0u8; ds];
        match desc {
            _ if ds == 0 => {}
            V::Ro { space: 0, id, off } if off >= 0 => {
                prog.consts
                    .get(yavs_ir::ConstId(id))?
                    .read(off as usize, &mut d)
                    .then_some(())?;
            }
            V::Ro { space: 1, off, .. } if off >= 0 => {
                d.copy_from_slice(self.m.inst.data.get(off as usize..off as usize + ds)?);
            }
            _ => return None,
        }
        let key = (tab.0, d.clone());
        if let Some(p) = self.plans.get(&key) {
            return p.clone();
        }
        let p = gather::plan(&prog.types, &prog.consts, &bytes, &d)
            .ok()
            .map(Rc::new);
        self.plans.insert(key, p.clone());
        p
    }

    /// Instances of a leaf: (byte offset in the value, kernel range).
    fn leaf_parts(
        &self,
        p: &Plan,
        k: usize,
        first: Option<(i64, i64)>,
    ) -> Vec<(usize, (u32, u32, u32))> {
        let l = &p.leaves[k];
        let Some(s) = self.m.inst.sigs.get(l.sig as usize) else {
            return Vec::new();
        };
        let (fst, outer) = first.map_or((0, p.count), |(f, o)| (f as u64, o as u64));
        (0..l.insts as usize)
            .map(|j| {
                (
                    l.off(j),
                    (s.signal.0, s.off + l.kstart(j, fst, outer) as u32, l.len),
                )
            })
            .collect()
    }

    fn out(&mut self, kr: (u32, u32, u32), t: u32) {
        let e = self.outs.entry(kr).or_insert(NONE);
        *e = self.sets.u(*e, t);
    }

    fn addr(&self, space: AddrSpace, off: u32) -> V {
        match space {
            AddrSpace::Frame => V::Addr {
                o: 0,
                off: off as i64,
                exact: true,
            },
            AddrSpace::Const(c) => V::Ro {
                space: 0,
                id: c.0,
                off: off as i64,
            },
            AddrSpace::Inst => V::Ro {
                space: 1,
                id: 0,
                off: off as i64,
            },
            AddrSpace::Pkg(p) => V::Ro {
                space: 2,
                id: p,
                off: off as i64,
            },
            AddrSpace::Up(_) | AddrSpace::Shared => V::Top,
        }
    }

    /// Kernel signal of a range, and its kind.
    fn sig_logic(&self, kr: (u32, u32, u32)) -> Option<Elem> {
        self.m
            .inst
            .sigs
            .iter()
            .find(|s| s.signal.0 == kr.0)
            .map(|s| s.ty.elem)
    }

    fn dope_len(&self, a: V) -> V {
        let mut b = [0u8; 24];
        let ok = match a {
            V::Ro { space: 0, id, off } if off >= 0 => (self.m.rt.prog.consts)
                .get(yavs_ir::ConstId(id))
                .is_some_and(|c| c.read(off as usize, &mut b)),
            V::Ro { space: 1, off, .. } if off >= 0 => {
                match self.m.inst.data.get(off as usize..off as usize + 24) {
                    Some(d) => {
                        b.copy_from_slice(d);
                        true
                    }
                    None => false,
                }
            }
            _ => false,
        };
        if !ok {
            return V::Top;
        }
        let w = |k: usize| i64::from_le_bytes(b[k * 8..k * 8 + 8].try_into().unwrap_or([0; 8]));
        let (l, r, d) = (w(0), w(1), w(2));
        V::Int(if d == 0 {
            (r - l + 1).max(0)
        } else {
            (l - r + 1).max(0)
        })
    }

    /// What a called subprogram may do that its arguments do not show.
    fn callee(&mut self, func: FuncId, tainted: bool) {
        let mut seen = BTreeSet::new();
        let mut todo = vec![func];
        while let Some(c) = todo.pop() {
            if !seen.insert(c.0) {
                continue;
            }
            let Some(f) = self.m.rt.prog.func(c) else {
                self.why("call_unknown");
                continue;
            };
            for i in &f.code {
                if let Some(r) = effect(i) {
                    self.why(&format!("callee_{r}"));
                }
                match i {
                    Inst::Call { func, .. } => todo.push(*func),
                    Inst::SigVal { .. } => self.why("callee_reads_signal"),
                    Inst::Drive { .. } | Inst::SigScatter { .. } => self.why("callee_drives"),
                    Inst::WaitOn { .. } | Inst::Suspend { .. } => self.why("callee_waits"),
                    Inst::AddrOf {
                        space: AddrSpace::Up(_),
                        ..
                    } => self.why("callee_up_frame"),
                    Inst::AddrOf {
                        space: AddrSpace::Shared,
                        ..
                    } => self.why("callee_shared"),
                    Inst::Check { .. }
                    | Inst::Pos { .. }
                    | Inst::SlicePos { .. }
                    | Inst::BinRange { .. }
                    | Inst::SubtypeCheck { .. }
                    | Inst::Unreachable { .. }
                        if tainted =>
                    {
                        self.why("callee_may_fail")
                    }
                    Inst::CallNative { native, .. } => {
                        let sym = self.native(*native);
                        self.native_ok(&sym);
                    }
                    _ => {}
                }
            }
        }
    }

    fn native(&self, n: yavs_ir::NativeId) -> String {
        self.m
            .rt
            .prog
            .natives
            .get(n)
            .map(|n| n.symbol.clone())
            .unwrap_or_default()
    }

    fn native_ok(&mut self, sym: &str) {
        if !yavs_ir::native_is_pure(sym) {
            self.why(&format!("native_impure:{sym}"));
        } else if may_warn(sym) {
            self.why(&format!("native_may_warn:{sym}"));
        }
    }

    #[allow(clippy::too_many_lines)]
    fn step(
        &mut self,
        pc: usize,
        i: &Inst,
        regs: &mut HashMap<u32, Av>,
        st: &mut St,
        ctrl: u32,
        cwords: &mut HashMap<(u32, i64), i64>,
    ) -> Option<u32> {
        if let Some(r) = effect(i) {
            self.why(r);
        }
        let list: Vec<Reg> = (i.list())
            .map(|l| self.f.operands[l.start as usize..(l.start + l.len) as usize].to_vec())
            .unwrap_or_default();
        let mut uses = NONE;
        let mut used = Vec::new();
        i.visit(&mut |r, role, _| {
            if role == Role::Use {
                used.push(r);
            }
        });
        used.extend(&list);
        for &r in &used {
            let a = self.get(regs, r);
            let t = match a.v {
                V::Res { ct } => self.u(a.t, ct),
                _ => a.t,
            };
            uses = self.u(uses, t);
        }
        let int = |s: &Self, regs: &HashMap<u32, Av>, r: Reg| match s.get(regs, r).v {
            V::Int(x) => Some(x),
            _ => None,
        };
        let set = |regs: &mut HashMap<u32, Av>, r: Reg, v: V, t: u32| {
            regs.insert(r.0, Av { v, t });
        };
        let wt = |s: &mut Self, t: u32| s.u(t, ctrl);
        let fail = |s: &mut Self, t: u32, what: &str| {
            if t != NONE {
                s.why(what);
            }
        };
        match *i {
            Inst::Const { dst, v } => set(regs, dst, V::Int(v), NONE),
            Inst::ConstF { dst, .. } => set(regs, dst, V::Top, NONE),
            Inst::Move { dst, src } => {
                let a = self.get(regs, src);
                regs.insert(dst.0, a);
            }
            Inst::AddrOf { dst, space, off } => set(regs, dst, self.addr(space, off), NONE),
            Inst::Select { dst, c, a, b } => {
                let (x, y, k) = (self.get(regs, a), self.get(regs, b), self.get(regs, c));
                let v = match k.v {
                    V::Int(0) => y.v,
                    V::Int(_) => x.v,
                    _ if x.v == y.v => x.v,
                    _ => V::Top,
                };
                set(regs, dst, v, uses);
            }
            Inst::CopyConst { dst, c } => {
                let a = self.get(regs, dst);
                let n = self.m.rt.prog.consts.get(c).and_then(const_len);
                let t = wt(self, a.t);
                self.mem_write(st, a.v, n, t);
            }
            Inst::Alloc { dst, size } => {
                let o = self.alloc_obj[&pc];
                let n = self.objs[o as usize];
                let t = wt(self, uses);
                st.mem[o as usize] = Rc::new(Mem {
                    b: vec![t; n],
                    ov: BTreeMap::new(),
                });
                let exact = int(self, regs, size).is_some();
                set(regs, dst, V::Addr { o, off: 0, exact }, uses);
                cwords.retain(|k, _| k.0 != o);
                for w in (0..n as i64).step_by(8) {
                    cwords.insert((o, w), 0);
                }
            }
            Inst::TempRoot { .. } | Inst::Stmt | Inst::Br { .. } | Inst::Ret { .. } => {}
            Inst::Load { dst, addr, w } => {
                let a = self.get(regs, addr);
                let t = self.mem_read(st, a.v, Some(w.bytes() as i64));
                let t = self.u(t, a.t);
                let v = match a.v {
                    V::Addr {
                        o: 0,
                        off,
                        exact: true,
                    } if w == Width::B8
                        && self.heap_slots.contains_key(&off)
                        && !self.written_before.first().is_some_and(|f| {
                            f.get(off as usize..off as usize + 8)
                                .is_none_or(|b| b.iter().any(|&x| x))
                        }) =>
                    {
                        let (o, k) = self.heap_slots[&off];
                        set(
                            regs,
                            dst,
                            V::Addr {
                                o,
                                off: k,
                                exact: true,
                            },
                            a.t,
                        );
                        return None;
                    }
                    V::Ro { space: 1, off, .. } if off >= 0 && w == Width::B8 => (self.m.inst.data)
                        .get(off as usize..off as usize + 8)
                        .map_or(V::Top, |b| {
                            V::Int(i64::from_le_bytes(b.try_into().unwrap_or([0; 8])))
                        }),
                    V::Res { ct } => {
                        set(regs, dst, V::Res { ct }, a.t);
                        return None;
                    }
                    _ => V::Top,
                };
                set(regs, dst, v, t);
            }
            Inst::Store { addr, src, w } => {
                let a = self.get(regs, addr);
                let s = self.get(regs, src);
                let sv = match s.v {
                    V::Res { ct } => self.u(s.t, ct),
                    _ => s.t,
                };
                let t = self.u(a.t, sv);
                let t = wt(self, t);
                self.mem_write(st, a.v, Some(w.bytes() as i64), t);
                if let V::Addr {
                    o,
                    off,
                    exact: true,
                } = a.v
                {
                    match s.v {
                        V::Int(x) => cwords.insert((o, off), x),
                        _ => cwords.remove(&(o, off)),
                    };
                }
            }
            Inst::Copy { dst, src, bytes } => {
                let (d, s, b) = (
                    self.get(regs, dst),
                    self.get(regs, src),
                    self.get(regs, bytes),
                );
                let n = match b.v {
                    V::Int(x) => Some(x),
                    _ => None,
                };
                let base = {
                    let t = self.u(d.t, s.t);
                    let t = self.u(t, b.t);
                    wt(self, t)
                };
                self.copy_bytes(st, d.v, s.v, n, base);
                if let (
                    V::Addr {
                        o,
                        off,
                        exact: true,
                    },
                    V::Addr {
                        o: so,
                        off: soff,
                        exact: true,
                    },
                    Some(n),
                ) = (d.v, s.v, n)
                {
                    for k in (0..n).step_by(8) {
                        match cwords.get(&(so, soff + k)).copied() {
                            Some(x) => cwords.insert((o, off + k), x),
                            None => cwords.remove(&(o, off + k)),
                        };
                    }
                }
            }
            Inst::Zero { addr, bytes } => {
                let a = self.get(regs, addr);
                let t = wt(self, a.t);
                self.mem_write(st, a.v, Some(bytes as i64), t);
                if let V::Addr {
                    o,
                    off,
                    exact: true,
                } = a.v
                {
                    for k in (0..bytes as i64).step_by(8) {
                        cwords.insert((o, off + k), 0);
                    }
                }
            }
            Inst::Fill { dst, elem, n, size } => {
                let (d, e, c) = (self.get(regs, dst), self.get(regs, elem), self.get(regs, n));
                let t = self.mem_read(st, e.v, Some(size as i64));
                let t = self.u(t, uses);
                let t = wt(self, t);
                let b = match c.v {
                    V::Int(x) => Some(x * size as i64),
                    _ => None,
                };
                self.mem_write(st, d.v, b, t);
            }
            Inst::Pos { dst, idx, dope, .. } => {
                let a = self.get(regs, idx);
                let t = match a.v {
                    V::Res { ct } => self.u(a.t, ct),
                    _ => a.t,
                };
                let d = self.get(regs, dope);
                let dt = self.mem_read(st, d.v, Some(24));
                let dt = self.u(dt, d.t);
                let all = self.u(t, dt);
                fail(self, all, "may_fail");
                set(regs, dst, V::Top, all);
            }
            Inst::SlicePos {
                dst,
                dope,
                left,
                right,
                dir,
                ..
            } => {
                let d = self.get(regs, dope);
                let mut t = self.mem_read(st, d.v, Some(24));
                t = self.u(t, d.t);
                for r in [left, right, dir] {
                    let a = self.get(regs, r);
                    let x = match a.v {
                        V::Res { ct } => self.u(a.t, ct),
                        _ => a.t,
                    };
                    t = self.u(t, x);
                }
                fail(self, t, "may_fail");
                set(regs, dst, V::Top, t);
            }
            Inst::Index {
                dst,
                base,
                pos,
                stride,
            } => {
                let (b, p) = (self.get(regs, base), self.get(regs, pos));
                let v = match (b.v, p.v) {
                    (
                        V::Addr {
                            o,
                            off,
                            exact: true,
                        },
                        V::Int(x),
                    ) => V::Addr {
                        o,
                        off: off + x * stride as i64,
                        exact: true,
                    },
                    (V::Addr { o, off, .. }, _) => V::Addr {
                        o,
                        off,
                        exact: false,
                    },
                    (V::Ro { space, id, off }, V::Int(x)) if off >= 0 => V::Ro {
                        space,
                        id,
                        off: off + x * stride as i64,
                    },
                    (V::Ro { space, id, .. }, _) => V::Ro { space, id, off: -1 },
                    (V::Res { ct }, _) => V::Res { ct },
                    _ => V::Top,
                };
                let t = match b.v {
                    V::Res { .. } => self.u(b.t, p.t),
                    _ => uses,
                };
                set(regs, dst, v, t);
            }
            Inst::Field { dst, base, .. } => {
                let b = self.get(regs, base);
                let v = match b.v {
                    V::Addr { o, off, .. } => V::Addr {
                        o,
                        off,
                        exact: false,
                    },
                    V::Ro { space, id, .. } => V::Ro { space, id, off: -1 },
                    x => x,
                };
                self.why("field_not_folded");
                set(regs, dst, v, uses);
            }
            Inst::Bin { op, dst, a, b } => {
                use yavs_ir::IntOp::*;
                let v = match (op, int(self, regs, a), int(self, regs, b)) {
                    (Add, Some(x), Some(y)) => V::Int(x.wrapping_add(y)),
                    (Sub, Some(x), Some(y)) => V::Int(x.wrapping_sub(y)),
                    (Mul, Some(x), Some(y)) => V::Int(x.wrapping_mul(y)),
                    _ => V::Top,
                };
                fail(
                    self,
                    uses,
                    if matches!(op, Add | Sub | Mul) {
                        "may_overflow"
                    } else {
                        "may_fail"
                    },
                );
                set(regs, dst, v, uses);
            }
            Inst::BinRange { dst, .. } | Inst::FToIRange { dst, .. } | Inst::FToI { dst, .. } => {
                fail(self, uses, "may_fail");
                set(regs, dst, V::Top, uses);
            }
            Inst::Check { .. } | Inst::FCheck { .. } => fail(self, uses, "may_fail"),
            Inst::Unreachable { .. } => fail(self, ctrl, "may_fail"),
            Inst::SubtypeCheck { dope, want, .. } => {
                let (d, w) = (self.get(regs, dope), self.get(regs, want));
                let x = self.mem_read(st, d.v, Some(24));
                let y = match w.v {
                    V::Res { .. } => NONE,
                    _ => self.mem_read(st, w.v, Some(24)),
                };
                let x = match d.v {
                    V::Res { .. } => NONE,
                    _ => x,
                };
                let t = self.u(x, y);
                let t = self.u(t, d.t);
                let t = self.u(t, w.t);
                fail(self, t, "may_fail");
            }
            Inst::Un { dst, .. }
            | Inst::FBin { dst, .. }
            | Inst::FUn { dst, .. }
            | Inst::Phys { dst, .. }
            | Inst::IToF { dst, .. }
            | Inst::FCmp { dst, .. } => set(regs, dst, V::Top, uses),
            // Constants fold, so a branch on generics takes one way.
            Inst::Cmp { op, dst, a, b } => {
                use yavs_ir::CmpOp::*;
                let v = match (int(self, regs, a), int(self, regs, b)) {
                    (Some(x), Some(y)) => V::Int(i64::from(match op {
                        Eq => x == y,
                        Ne => x != y,
                        Lt => x < y,
                        Le => x <= y,
                        Gt => x > y,
                        Ge => x >= y,
                    })),
                    _ => V::Top,
                };
                set(regs, dst, v, uses);
            }
            Inst::Bool { op, dst, a, b } => {
                use yavs_ir::BoolOp::*;
                let (x, y) = (int(self, regs, a), int(self, regs, b));
                let v = match (op, x, y) {
                    (Not, Some(x), _) => V::Int(i64::from(x == 0)),
                    (And, Some(0), _) | (And, _, Some(0)) => V::Int(0),
                    (Or, Some(x), _) | (Or, _, Some(x)) if x != 0 => V::Int(1),
                    (And, Some(x), Some(y)) => V::Int(i64::from(x != 0 && y != 0)),
                    (Or, Some(_), Some(_)) => V::Int(0),
                    (Xor, Some(x), Some(y)) => V::Int(i64::from((x != 0) != (y != 0))),
                    _ => V::Top,
                };
                set(regs, dst, v, uses);
            }
            Inst::LogicOp { dst, a, b, n, op } => {
                let c = int(self, regs, n);
                let (x, y, d) = (
                    self.get(regs, a).v,
                    self.get(regs, b).v,
                    self.get(regs, dst),
                );
                let mut t = self.erd(st, x, Some(0), c);
                if !matches!(
                    op,
                    yavs_ir::LOp::Not
                        | yavs_ir::LOp::ToX01
                        | yavs_ir::LOp::ToX01Z
                        | yavs_ir::LOp::ToUx01
                ) {
                    let u = self.erd(st, y, Some(0), c);
                    t = self.u(t, u);
                }
                let t = self.u(t, uses);
                let t = wt(self, t);
                self.ewr(st, d.v, Some(0), c, t);
            }
            Inst::LNum {
                op, dst, a, b, n, ..
            } => {
                let c = int(self, regs, n);
                let (x, y) = (self.get(regs, a).v, self.get(regs, b).v);
                let t = self.erd(st, x, Some(0), c);
                let u = self.erd(st, y, Some(0), c);
                let t = self.u(t, u);
                let t = self.u(t, uses);
                if let yavs_ir::LNumOp::Cmp(_) = op {
                    set(regs, dst, V::Top, t);
                } else {
                    let d = self.get(regs, dst);
                    let t = wt(self, t);
                    self.ewr(st, d.v, Some(0), c, t);
                }
            }
            Inst::LEq { dst, a, b, n } => {
                let c = int(self, regs, n);
                let (x, y) = (self.get(regs, a).v, self.get(regs, b).v);
                let t = self.erd(st, x, Some(0), c);
                let u = self.erd(st, y, Some(0), c);
                let t = self.u(t, u);
                let t = self.u(t, uses);
                set(regs, dst, V::Top, t);
            }
            Inst::LKey { dst, a, n } => {
                let c = int(self, regs, n);
                let x = self.get(regs, a).v;
                let t = self.erd(st, x, Some(0), c);
                let t = self.u(t, uses);
                set(regs, dst, V::Top, t);
            }
            Inst::MemEq { dst, a, b, n } => {
                let c = int(self, regs, n);
                let (x, y) = (self.get(regs, a).v, self.get(regs, b).v);
                let t = self.mem_read(st, x, c);
                let u = self.mem_read(st, y, c);
                let t = self.u(t, u);
                let t = self.u(t, uses);
                set(regs, dst, V::Top, t);
            }
            Inst::LGet { dst, base, idx } => {
                let b = self.get(regs, base).v;
                let t = self.erd(st, b, int(self, regs, idx), Some(1));
                let t = self.u(t, uses);
                set(regs, dst, V::Top, t);
            }
            Inst::LSet { base, idx, .. } => {
                let b = self.get(regs, base).v;
                let t = wt(self, uses);
                self.ewr(st, b, int(self, regs, idx), Some(1), t);
            }
            Inst::LFill { base, idx, n, .. } => {
                let b = self.get(regs, base).v;
                let t = wt(self, uses);
                let (k, c) = (int(self, regs, idx), int(self, regs, n));
                self.ewr(st, b, k, c, t);
            }
            Inst::LCopy {
                dst,
                dst_off,
                src,
                src_off,
                n,
            } => {
                let (d, s) = (self.get(regs, dst).v, self.get(regs, src).v);
                let c = int(self, regs, n);
                let t = self.erd(st, s, int(self, regs, src_off), c);
                let t = self.u(t, uses);
                let t = wt(self, t);
                let k = int(self, regs, dst_off);
                self.ewr(st, d, k, c, t);
            }
            Inst::Cat {
                dst,
                n,
                m,
                parts,
                scalars,
                logic,
                ..
            } => {
                let mut t = wt(self, NONE);
                let ps: Vec<Reg> = self.f.operands
                    [parts.start as usize..(parts.start + parts.len) as usize]
                    .to_vec();
                let mm = int(self, regs, m);
                for (k, q) in ps.chunks(2).enumerate() {
                    let a = self.get(regs, q[0]);
                    let x = match a.v {
                        V::Res { ct } => self.u(a.t, ct),
                        _ => a.t,
                    };
                    t = self.u(t, x);
                    let ct = self.get(regs, q[1]).t;
                    t = self.u(t, ct);
                    if scalars >> k & 1 == 0 {
                        let c = int(self, regs, q[1]);
                        let u = if logic {
                            self.erd(st, a.v, Some(0), c.zip(mm).map(|(x, y)| x * y))
                        } else {
                            self.mem_read(st, a.v, c.zip(mm).map(|(x, y)| x * y))
                        };
                        t = self.u(t, u);
                    }
                }
                let d = self.get(regs, dst).v;
                let total = int(self, regs, n).zip(mm).map(|(x, y)| x * y);
                if logic {
                    self.ewr(st, d, Some(0), total, t);
                } else {
                    self.mem_write(st, d, total, t);
                }
            }
            Inst::MakeDope { at, parts } => {
                let a = self.get(regs, at);
                let t = wt(self, uses);
                self.mem_write(st, a.v, Some(parts.len as i64 * 8), t);
            }
            Inst::DopeLen { dst, dope, .. } => {
                let a = self.get(regs, dope);
                let (v, t) = match a.v {
                    V::Res { .. } => (V::Top, a.t),
                    _ => {
                        let t = self.mem_read(st, a.v, Some(24));
                        (self.dope_len(a.v), self.u(t, a.t))
                    }
                };
                set(regs, dst, v, t);
            }
            Inst::LayoutOf { dst, desc, .. } => {
                let a = self.get(regs, desc);
                let t = self.mem_read(st, a.v, Some(24));
                let t = self.u(t, a.t);
                self.why("layout_not_folded");
                set(regs, dst, V::Top, t);
            }
            Inst::CondBr { c, .. } if matches!(self.get(regs, c).v, V::Int(_)) => {
                self.taken = Some(usize::from(self.get(regs, c).v == V::Int(0)));
            }
            Inst::CondBr { c, .. } | Inst::Case { v: c, .. } => {
                let a = self.get(regs, c);
                let t = match a.v {
                    V::Res { ct } => self.u(a.t, ct),
                    _ => a.t,
                };
                self.any_cond = self.u(self.any_cond, t);
                return Some(t);
            }
            Inst::Call { func, dst, .. } => {
                self.callee(func, uses != NONE || ctrl != NONE);
                let mut t = uses;
                let (procedure, out_params) =
                    self.m.rt.prog.func(func).map_or((false, false), |c| {
                        (
                            c.kind == FuncKind::Procedure,
                            c.params.iter().any(|p| p.mode != Mode::In),
                        )
                    });
                for &r in &list {
                    let a = self.get(regs, r).v;
                    if matches!(a, V::Addr { .. } | V::Res { .. }) {
                        let u = self.mem_read(st, addr_any(a), None);
                        t = self.u(t, u);
                    }
                }
                if procedure && out_params {
                    let w = wt(self, t);
                    for &r in &list {
                        let a = self.get(regs, r).v;
                        if matches!(a, V::Addr { .. }) {
                            self.weak_write(st, addr_any(a), None, w);
                        }
                    }
                }
                if let Some(d) = dst {
                    set(regs, d, V::Res { ct: t }, t);
                }
            }
            Inst::CallNative { native, dst, .. } => {
                let sym = self.native(native);
                self.native_ok(&sym);
                let mut ct = uses;
                let mut shape = NONE;
                for &r in &list {
                    let a = self.get(regs, r);
                    shape = self.u(shape, a.t);
                }
                let key = sym.strip_prefix("yavs.ieee.");
                let mut k = 0;
                let mut p = 0;
                while k < list.len() {
                    let arr = key.is_some_and(|key| yavs_ir::ieee::array_param(key, p));
                    let a = self.get(regs, list[k]);
                    if arr && k + 3 < list.len() {
                        let (l, r) = (int(self, regs, list[k + 1]), int(self, regs, list[k + 2]));
                        let n = l.zip(r).map(|(l, r)| (l - r).abs() + 1);
                        let u = self.erd(st, a.v, Some(0), n);
                        ct = self.u(ct, u);
                        k += 4;
                    } else {
                        if matches!(a.v, V::Addr { .. } | V::Res { .. }) {
                            let u = self.mem_read(st, addr_any(a.v), None);
                            ct = self.u(ct, u);
                        }
                        k += 1;
                    }
                    p += 1;
                }
                let shape = if sym.starts_with("yavs.ieee.") {
                    shape
                } else {
                    ct
                };
                if let Some(d) = dst {
                    set(regs, d, V::Res { ct }, shape);
                }
            }
            Inst::NewFrame { dst, .. } | Inst::New { dst, .. } | Inst::NewFill { dst, .. } => {
                self.why("heap_in_body");
                set(regs, dst, V::Top, uses);
            }
            Inst::Deref { dst, .. } => {
                self.why("deref");
                set(regs, dst, V::Top, uses);
            }
            Inst::SigVal {
                dst,
                sig,
                offset,
                len,
            } => {
                let (o, l) = (int(self, regs, offset), int(self, regs, len));
                let v = match (o, l) {
                    (Some(o), Some(l)) => V::Sig {
                        sig,
                        off: o,
                        len: l,
                    },
                    _ => V::Sig {
                        sig,
                        off: -1,
                        len: -1,
                    },
                };
                set(regs, dst, v, uses);
            }
            Inst::SigSlice {
                dst,
                sig,
                offset,
                len,
            } => {
                let a = self.get(regs, sig);
                let (o, l) = (int(self, regs, offset), int(self, regs, len));
                let v = match (a.v, o, l) {
                    (V::Sig { sig, off, .. }, Some(o), Some(l)) if off >= 0 => V::Sig {
                        sig,
                        off: off + o,
                        len: l,
                    },
                    (V::Sig { sig, .. }, _, _) => V::Sig {
                        sig,
                        off: -1,
                        len: -1,
                    },
                    _ => V::Top,
                };
                set(regs, dst, v, uses);
            }
            Inst::SigRead { dst, sig } => {
                let a = self.get(regs, sig);
                let t = self.read_sig(a);
                let t = wt(self, t);
                let d = self.get(regs, dst).v;
                if let V::Sig { sig, off, len } = a.v
                    && let Some(s) = self.m.inst.sigs.get(sig as usize)
                {
                    let n = if off < 0 { s.ty.len as i64 } else { len };
                    match s.ty.elem {
                        Elem::Logic => self.ewr(st, d, Some(0), Some(n), t),
                        Elem::Bytes(k) => self.mem_write(st, d, Some(n * k as i64), t),
                    }
                } else {
                    self.weak_write(st, d, None, t);
                }
            }
            Inst::SigReadScalar { dst, sig } => {
                let a = self.get(regs, sig);
                let t = self.read_sig(a);
                set(regs, dst, V::Top, t);
            }
            Inst::SigLastValue { dst, sig } => {
                let a = self.get(regs, sig);
                self.hist_read(a);
                let t = self.read_sig(a);
                let t = wt(self, t);
                let d = self.get(regs, dst).v;
                self.weak_write(st, d, None, t);
            }
            Inst::SigAttr { dst, sig, .. } | Inst::Edge { dst, sig, .. } => {
                let a = self.get(regs, sig);
                self.hist_read(a);
                let t = self.read_sig(a);
                set(regs, dst, V::Top, t);
            }
            Inst::SigGather {
                dst,
                desc,
                tab,
                part,
            } => {
                let (d, ds) = (self.get(regs, dst), self.get(regs, desc));
                match self.plan(tab, ds.v) {
                    Some(p) => {
                        let first = part
                            .and_then(|q| int(self, regs, q.first).zip(int(self, regs, q.outer)));
                        if part.is_some() && first.is_none() {
                            self.why("gather_dynamic_part");
                        }
                        for k in 0..p.leaves.len() {
                            let (logic, vsize, len) =
                                (p.leaves[k].logic(), p.leaves[k].vsize, p.leaves[k].len);
                            for (off, kr) in self.leaf_parts(&p, k, first) {
                                let id = self.read_id(kr);
                                let t = self.sets.one(id);
                                let t = self.u(t, uses);
                                let t = wt(self, t);
                                let at = offset(d.v, off as i64);
                                if logic && vsize >= 32 {
                                    self.ewr(st, at, Some(0), Some(len as i64), t);
                                } else {
                                    self.mem_write(st, at, Some(vsize as i64), t);
                                }
                            }
                        }
                    }
                    None => {
                        self.why("gather_no_plan");
                        self.weak_write(st, d.v, None, uses);
                    }
                }
            }
            Inst::SigScatter {
                src,
                desc,
                tab,
                ops,
                ..
            } => {
                let (s, ds) = (self.get(regs, src), self.get(regs, desc));
                let o: Vec<Reg> =
                    self.f.operands[ops.start as usize..(ops.start + ops.len) as usize].to_vec();
                if int(self, regs, o[0]) != Some(0) {
                    self.why("delay");
                }
                let part = (ops.len >= 3).then(|| (o[o.len() - 2], o[o.len() - 1]));
                let first = part.and_then(|(a, b)| int(self, regs, a).zip(int(self, regs, b)));
                if part.is_some() && first.is_none() {
                    self.why("scatter_dynamic_part");
                }
                match self.plan(tab, ds.v) {
                    Some(p) => {
                        for k in 0..p.leaves.len() {
                            let (logic, vsize, len) =
                                (p.leaves[k].logic(), p.leaves[k].vsize, p.leaves[k].len);
                            for (off, kr) in self.leaf_parts(&p, k, first) {
                                let at = offset(s.v, off as i64);
                                let t = if logic && vsize >= 32 {
                                    self.erd(st, at, Some(0), Some(len as i64))
                                } else {
                                    self.mem_read(st, at, Some(vsize as i64))
                                };
                                let t = self.u(t, uses);
                                let t = wt(self, t);
                                self.out(kr, t);
                                if part.is_none() || first.is_some() {
                                    Rc::make_mut(&mut st.driven).insert(kr);
                                }
                            }
                        }
                    }
                    None => self.why("scatter_no_plan"),
                }
            }
            Inst::Drive {
                driver,
                target,
                wave,
                n,
                reject,
                ..
            } => {
                if reject.is_some() {
                    self.why("reject");
                }
                if int(self, regs, n) != Some(1) {
                    self.why("waveform_elements");
                }
                let w = self.get(regs, wave);
                let dr = self.m.drivers.get(driver as usize).copied();
                let tv = target.map(|t| self.get(regs, t));
                let (kr, exact) = match (tv.map(|t| t.v), dr) {
                    (Some(V::Sig { sig, off, len }), _) if off >= 0 && len >= 0 => {
                        (self.krange(sig, off, len), true)
                    }
                    (_, Some(d)) => {
                        let s = self.m.inst.sigs.get(d.sig as usize);
                        (
                            s.map(|s| (s.signal.0, d.range.offset, d.range.len)),
                            tv.is_none(),
                        )
                    }
                    _ => (None, false),
                };
                match kr {
                    Some(kr) => {
                        let at = offset(w.v, 8);
                        let t = match self.sig_logic(kr) {
                            Some(Elem::Logic) => self.erd(st, at, Some(0), Some(kr.2 as i64)),
                            Some(Elem::Bytes(k)) => {
                                self.mem_read(st, at, Some(kr.2 as i64 * k as i64))
                            }
                            None => self.mem_read(st, at, None),
                        };
                        let t = self.u(t, uses);
                        let t = wt(self, t);
                        self.out(kr, t);
                        if exact {
                            Rc::make_mut(&mut st.driven).insert(kr);
                        }
                    }
                    None => self.why("drive_unknown"),
                }
                match w.v {
                    V::Addr {
                        o,
                        off,
                        exact: true,
                    } => match cwords.get(&(o, off)) {
                        Some(0) => {}
                        Some(_) => self.why("delay"),
                        None => self.why("delay_unknown"),
                    },
                    _ => self.why("delay_unknown"),
                }
            }
            Inst::WaitOn { sig } => {
                if let V::Sig { sig, off, len } = self.get(regs, sig).v
                    && let Some(k) = self.krange(sig, off, len)
                {
                    self.waits.insert(k);
                } else {
                    self.why("wait_unknown");
                }
            }
            Inst::Suspend { .. }
            | Inst::Report { .. }
            | Inst::Output { .. }
            | Inst::Cover { .. } => {}
            _ => {
                let mut defs = Vec::new();
                i.visit(&mut |r, role, _| {
                    if role == Role::Def {
                        defs.push(r);
                    }
                });
                for d in defs {
                    set(regs, d, V::Top, uses);
                }
            }
        }
        None
    }
}

fn const_len(c: &yavs_ir::ConstData) -> Option<i64> {
    use yavs_ir::ConstData::*;
    Some(match c {
        Bytes(b) => b.len() as i64,
        Repeat { elem, n } => elem.len() as i64 * *n as i64,
        Runs(r) => r.iter().map(|(n, e)| *n as i64 * e.len() as i64).sum(),
        Logic(r) => (r.iter().map(|(n, _)| *n as i64).sum::<i64>() + 63) / 64 * 32,
    })
}

fn addr_any(a: V) -> V {
    match a {
        V::Addr { o, off, .. } => V::Addr {
            o,
            off,
            exact: false,
        },
        x => x,
    }
}

fn offset(a: V, k: i64) -> V {
    match a {
        V::Addr { o, off, exact } => V::Addr {
            o,
            off: off + k,
            exact,
        },
        V::Ro { space, id, off } => V::Ro {
            space,
            id,
            off: if off < 0 { off } else { off + k },
        },
        x => x,
    }
}

struct Graph {
    nodes: Vec<(usize, usize)>,
    succ: Vec<Vec<usize>>,
    entry: usize,
}

/// The blocks, each cut after its `Suspend` (copies of the one wait the memo pass made are
/// several); the entry is a node of no code whose successors are the code after each.
fn graph(m: &Machine, f: &Func, sus: &[usize]) -> Graph {
    let mut nodes = Vec::new();
    let mut of_block = HashMap::new();
    let mut resumes = Vec::new();
    for (b, bl) in f.blocks.iter() {
        let (s, e) = (bl.start as usize, bl.end as usize);
        of_block.insert(b.0, nodes.len());
        match sus.iter().find(|&&p| (s..e).contains(&p)) {
            Some(&p) => {
                nodes.push((s, p + 1));
                resumes.push(nodes.len());
                nodes.push((p + 1, e));
            }
            None => nodes.push((s, e)),
        }
    }
    let succ_of = |(s, e): (usize, usize)| -> Vec<usize> {
        if s >= e {
            return Vec::new();
        }
        match f.code[e - 1] {
            Inst::Br { to } => vec![of_block[&to.0]],
            Inst::CondBr { t, f: x, .. } => vec![of_block[&t.0], of_block[&x.0]],
            Inst::Case { table, .. } => {
                let Some(c) = m.rt.prog.cases.get(table) else {
                    return Vec::new();
                };
                let mut v: Vec<usize> = c.arms.iter().map(|a| of_block[&a.2.0]).collect();
                v.push(of_block[&c.default.0]);
                v.sort_unstable();
                v.dedup();
                v
            }
            _ => Vec::new(),
        }
    };
    let mut succ: Vec<Vec<usize>> = nodes.iter().map(|&r| succ_of(r)).collect();
    let entry = nodes.len();
    nodes.push((0, 0));
    succ.push(resumes);
    Graph { nodes, succ, entry }
}

/// Control dependence: per node, the branch nodes it depends on.
fn control_deps(g: &Graph) -> Vec<Vec<usize>> {
    let n = g.nodes.len();
    let exit = n;
    let mut pred = vec![Vec::new(); n + 1];
    for (a, s) in g.succ.iter().enumerate() {
        for &b in s {
            pred[b].push(a);
        }
        if s.is_empty() {
            pred[exit].push(a);
        }
    }
    // A node with no path to the end goes to the exit too.
    let mut can = vec![false; n + 1];
    let mut todo = vec![exit];
    while let Some(x) = todo.pop() {
        if !can[x] {
            can[x] = true;
            todo.extend(pred[x].iter().copied());
        }
    }
    for (x, &c) in can.iter().enumerate().take(n) {
        if !c {
            pred[exit].push(x);
        }
    }
    let succ = |x: usize| -> Vec<usize> {
        let mut v = g.succ[x].clone();
        if g.succ[x].is_empty() || !can[x] {
            v.push(exit);
        }
        v
    };
    let mut order = Vec::new();
    let mut seen = vec![false; n + 1];
    let mut stack = vec![(exit, 0usize)];
    seen[exit] = true;
    while let Some(&mut (x, ref mut k)) = stack.last_mut() {
        if let Some(&y) = pred[x].get(*k) {
            *k += 1;
            if !seen[y] {
                seen[y] = true;
                stack.push((y, 0));
            }
        } else {
            order.push(x);
            stack.pop();
        }
    }
    let mut rank = vec![0usize; n + 1];
    for (i, &x) in order.iter().enumerate() {
        rank[x] = i;
    }
    let mut ipdom = vec![usize::MAX; n + 1];
    ipdom[exit] = exit;
    let mut changed = true;
    while changed {
        changed = false;
        for &x in order.iter().rev() {
            if x == exit {
                continue;
            }
            let mut d = usize::MAX;
            for y in succ(x) {
                if ipdom[y] == usize::MAX {
                    continue;
                }
                d = if d == usize::MAX {
                    y
                } else {
                    let (mut a, mut b) = (d, y);
                    while a != b {
                        while rank[a] < rank[b] {
                            a = ipdom[a];
                        }
                        while rank[b] < rank[a] {
                            b = ipdom[b];
                        }
                    }
                    a
                };
            }
            if d != ipdom[x] {
                ipdom[x] = d;
                changed = true;
            }
        }
    }
    let mut cd = vec![Vec::new(); n];
    for a in 0..n {
        if g.succ[a].len() < 2 {
            continue;
        }
        for &s in &g.succ[a] {
            let mut r = s;
            while r != ipdom[a] && r != exit && r != usize::MAX {
                cd[r].push(a);
                r = ipdom[r];
            }
        }
    }
    for c in &mut cd {
        c.sort_unstable();
        c.dedup();
    }
    cd
}

fn liveness(f: &Func, g: &Graph, global: &[Option<V>]) -> Vec<BTreeSet<u32>> {
    let n = g.nodes.len();
    let mut live: Vec<BTreeSet<u32>> = vec![BTreeSet::new(); n];
    let mut changed = true;
    while changed {
        changed = false;
        for x in (0..n).rev() {
            let mut l: BTreeSet<u32> = BTreeSet::new();
            for &y in &g.succ[x] {
                l.extend(live[y].iter().copied());
            }
            for pc in (g.nodes[x].0..g.nodes[x].1).rev() {
                let i = &f.code[pc];
                i.visit(&mut |r, role, _| {
                    if role == Role::Def {
                        l.remove(&r.0);
                    }
                });
                i.visit(&mut |r, role, _| {
                    if role == Role::Use && global[r.0 as usize].is_none() {
                        l.insert(r.0);
                    }
                });
                if let Some(ls) = i.list() {
                    for r in &f.operands[ls.start as usize..(ls.start + ls.len) as usize] {
                        if global[r.0 as usize].is_none() {
                            l.insert(r.0);
                        }
                    }
                }
            }
            if l != live[x] {
                live[x] = l;
                changed = true;
            }
        }
    }
    live
}

fn join_v(a: V, b: V) -> V {
    if a == b {
        return a;
    }
    match (a, b) {
        (V::Bot, q) | (q, V::Bot) => q,
        (V::Addr { o, off, .. }, V::Addr { o: p, .. }) if o == p => V::Addr {
            o,
            off,
            exact: false,
        },
        (V::Res { .. }, V::Res { .. }) => V::Top,
        _ => V::Top,
    }
}

fn join(sets: &mut Sets, a: &St, b: &St) -> St {
    let mut regs = a.regs.clone();
    for (&r, &y) in &b.regs {
        let e = regs.entry(r).or_insert(y);
        let v = match (e.v, y.v) {
            (V::Res { ct: p }, V::Res { ct: q }) => V::Res { ct: sets.u(p, q) },
            (x, z) => join_v(x, z),
        };
        let t = sets.u(e.t, y.t);
        *e = Av { v, t };
    }
    let mem = (a.mem.iter().zip(&b.mem))
        .map(|(x, y)| {
            if Rc::ptr_eq(x, y) {
                return Rc::clone(x);
            }
            let b: Vec<u32> = x.b.iter().zip(&y.b).map(|(&p, &q)| sets.u(p, q)).collect();
            let mut ov = BTreeMap::new();
            let keys: BTreeSet<usize> = x.ov.keys().chain(y.ov.keys()).copied().collect();
            let whole = |m: &Mem, k: usize, sets: &mut Sets| {
                let mut t = NONE;
                for &q in &m.b[k..(k + 32).min(m.b.len())] {
                    t = sets.u(t, q);
                }
                [t; 64]
            };
            for k in keys {
                let p = x.ov.get(&k).map_or_else(|| whole(x, k, sets), |e| **e);
                let q = y.ov.get(&k).map_or_else(|| whole(y, k, sets), |e| **e);
                let mut e = [NONE; 64];
                for j in 0..64 {
                    e[j] = sets.u(p[j], q[j]);
                }
                ov.insert(k, Box::new(e));
            }
            Rc::new(Mem { b, ov })
        })
        .collect();
    let driven = Rc::new(a.driven.intersection(&b.driven).copied().collect());
    St { regs, mem, driven }
}

/// The census lines of the process this machine runs.
pub(crate) fn census(m: &Machine) -> Vec<String> {
    let prog = &m.rt.prog;
    let mut out = Vec::new();
    let Some(f) = prog.func(m.entry) else {
        return vec!["reason no_function 1".to_string()];
    };
    out.push(format!(
        "code func={} base={} ops={} frame={} regs={}",
        m.entry.0,
        prog.base_of(m.entry).0,
        f.code.len(),
        f.frame,
        f.reg_ty.len()
    ));
    let sus: Vec<usize> = (f.code.iter().enumerate())
        .filter_map(|(k, i)| matches!(i, Inst::Suspend { .. }).then_some(k))
        .collect();
    if !prog.single_suspend(m.entry) || sus.is_empty() {
        out.push(format!("reason suspends:{} 1", sus.len()));
        return out;
    }
    if sus.len() > 1 {
        out.push(format!("reason wait_copies:{} 1", sus.len()));
    }
    let g = graph(m, f, &sus);
    let mut defs = vec![0u32; f.reg_ty.len()];
    for i in &f.code {
        i.visit(&mut |r, role, _| {
            if role == Role::Def {
                defs[r.0 as usize] += 1;
            }
        });
    }
    let frame_len = (f.frame as usize + 1).max(m.fslots(0).map_or(0, <[u8]>::len));
    let mut a = An {
        m,
        f,
        sets: Sets::new(),
        reads: Vec::new(),
        read_ids: HashMap::new(),
        global: vec![None; f.reg_ty.len()],
        objs: vec![frame_len],
        alloc_obj: HashMap::new(),
        outs: BTreeMap::new(),
        conds: vec![NONE; g.nodes.len()],
        any_cond: NONE,
        facts: Facts::default(),
        state_t: NONE,
        plans: HashMap::new(),
        waits: BTreeSet::new(),
        hist: BTreeSet::new(),
        pc: 0,
        taken: None,
        read_pcs: Vec::new(),
        heap_slots: HashMap::new(),
        persistent: Vec::new(),
        written_before: Vec::new(),
        frame_bases: BTreeSet::new(),
    };
    for i in &f.code {
        if let Inst::AddrOf {
            space: AddrSpace::Frame,
            off,
            ..
        } = *i
        {
            a.frame_bases.insert(off as i64);
        }
    }
    a.frame_bases.extend(f.locals.iter().map(|l| l.slot as i64));
    for i in &f.code {
        match *i {
            Inst::Const { dst, v } if defs[dst.0 as usize] == 1 => {
                a.global[dst.0 as usize] = Some(V::Int(v))
            }
            Inst::AddrOf { dst, space, off } if defs[dst.0 as usize] == 1 => {
                a.global[dst.0 as usize] = Some(a.addr(space, off));
            }
            _ => {}
        }
    }
    for (pc, i) in f.code.iter().enumerate() {
        if let Inst::Alloc { size, .. } = *i {
            let n = match a.global[size.0 as usize] {
                Some(V::Int(x)) if x > 0 => x as usize,
                _ => 1,
            };
            a.alloc_obj.insert(pc, a.objs.len() as u32);
            a.objs.push(n);
        }
    }
    // Variables kept in heap objects: their address is in a frame slot of the suspended process.
    if let Some(root) = m.fslots(0) {
        let mut seen: HashMap<(u8, u32), u32> = HashMap::new();
        for i in &f.code {
            let Inst::Load {
                addr, w: Width::B8, ..
            } = *i
            else {
                continue;
            };
            let Some(V::Addr { o: 0, off, .. }) = a.global[addr.0 as usize] else {
                continue;
            };
            let Some(b) = root.get(off as usize..off as usize + 8) else {
                continue;
            };
            let p = u64::from_le_bytes(b.try_into().unwrap_or([0; 8]));
            let (sp, idx, k) = crate::exec::split(p);
            if sp != crate::exec::HEAP || p == 0 {
                continue;
            }
            let Ok(obj) = m.obj(sp, idx) else {
                continue;
            };
            let n = obj.len().max(1);
            let o = *seen.entry((sp, idx)).or_insert_with(|| {
                a.objs.push(n);
                a.objs.len() as u32 - 1
            });
            a.heap_slots.insert(off, (o, k as i64));
        }
    }
    a.persistent = (0..a.objs.len())
        .map(|o| o == 0 || a.heap_slots.values().any(|h| h.0 as usize == o))
        .collect();
    a.state_t = a.sets.intern(vec![STATE]);
    let cd = control_deps(&g);
    let live = liveness(f, &g, &a.global);
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); g.nodes.len()];
    for (x, c) in cd.iter().enumerate() {
        for &b in c {
            dependents[b].push(x);
        }
    }
    // Twice: the first pass finds which bytes the activation writes; only they can hold state
    // from the last activation.
    let mut written: Vec<Vec<bool>> = (a.objs.iter().zip(&a.persistent))
        .map(|(&n, &p)| vec![p; if p { n } else { 0 }])
        .collect();
    for pass in 0..2 {
        a.written_before = written.clone();
        a.facts = Facts {
            written: (a.objs.iter().zip(&a.persistent))
                .map(|(&n, &p)| vec![false; if p { n } else { 0 }])
                .collect(),
            ..Facts::default()
        };
        a.outs.clear();
        a.reads.clear();
        a.read_pcs.clear();
        a.read_ids.clear();
        a.waits.clear();
        a.hist.clear();
        a.conds = vec![NONE; g.nodes.len()];
        a.any_cond = NONE;
        let mem: Vec<Rc<Mem>> = (0..a.objs.len())
            .map(|o| {
                let b = if a.persistent[o] {
                    written[o]
                        .iter()
                        .map(|&w| if w { a.state_t } else { NONE })
                        .collect()
                } else {
                    vec![NONE; a.objs[o]]
                };
                Rc::new(Mem {
                    b,
                    ov: BTreeMap::new(),
                })
            })
            .collect();
        let start = St {
            regs: BTreeMap::new(),
            mem,
            driven: Rc::new(BTreeSet::new()),
        };
        let mut inn: Vec<Option<St>> = vec![None; g.nodes.len()];
        inn[g.entry] = Some(start);
        let mut work: BTreeSet<usize> = BTreeSet::new();
        work.insert(g.entry);
        let mut final_driven: Option<BTreeSet<(u32, u32, u32)>> = None;
        let mut steps = 0u64;
        while let Some(x) = work.pop_first() {
            steps += 1;
            if steps > 200_000 {
                a.why("census_gave_up");
                break;
            }
            let Some(mut st) = inn[x].clone() else {
                continue;
            };
            let mut ctrl = NONE;
            for &b in &cd[x] {
                ctrl = a.sets.u(ctrl, a.conds[b]);
            }
            let mut regs: HashMap<u32, Av> = st.regs.iter().map(|(&r, &v)| (r, v)).collect();
            let mut cwords = HashMap::new();
            let mut cond = None;
            a.taken = None;
            for pc in g.nodes[x].0..g.nodes[x].1 {
                a.pc = pc;
                if let Some(c) = a.step(pc, &f.code[pc], &mut regs, &mut st, ctrl, &mut cwords) {
                    cond = Some(c);
                }
                if matches!(f.code[pc], Inst::Suspend { .. }) {
                    final_driven = Some(match final_driven {
                        Some(d) => d.intersection(&st.driven).copied().collect(),
                        None => (*st.driven).clone(),
                    });
                }
            }
            if let Some(c) = cond {
                let n = a.sets.u(a.conds[x], c);
                if n != a.conds[x] {
                    a.conds[x] = n;
                    work.extend(dependents[x].iter().copied());
                }
            }
            let succ: Vec<usize> = match a.taken {
                Some(k) => g.succ[x].get(k).copied().into_iter().collect(),
                None => g.succ[x].clone(),
            };
            for y in succ {
                let mut o = St {
                    regs: live[y]
                        .iter()
                        .filter_map(|r| regs.get(r).map(|v| (*r, *v)))
                        .collect(),
                    mem: st.mem.clone(),
                    driven: Rc::clone(&st.driven),
                };
                if let Some(old) = &inn[y] {
                    o = join(&mut a.sets, old, &o);
                    if &o == old {
                        continue;
                    }
                }
                inn[y] = Some(o);
                work.insert(y);
            }
        }
        written = std::mem::take(&mut a.facts.written);
        if pass == 0 {
            continue;
        }
        let cond_state = a.sets.state(a.any_cond);
        let out_state = a.outs.values().any(|&t| a.sets.state(t));
        if cond_state || out_state {
            a.why("state");
        }
        let driven = final_driven.unwrap_or_default();
        for (r, n) in &a.facts.reasons {
            let pcs = a.facts.pcs.get(r).cloned().unwrap_or_default();
            out.push(format!("reason {r} {n} at {pcs:?}"));
        }
        if !a.facts.state_vars.is_empty() {
            let v: Vec<String> = a.facts.state_vars.iter().cloned().collect();
            out.push(format!("statevars {}", v.join(",")));
        }
        for (k, r) in a.reads.iter().enumerate() {
            out.push(format!(
                "read {} {} {} {} pc {}",
                k + 1,
                r.0,
                r.1,
                r.2,
                a.read_pcs[k]
            ));
        }
        for w in &a.waits {
            out.push(format!("wait {} {} {}", w.0, w.1, w.2));
        }
        for h in &a.hist {
            out.push(format!("hist {} {} {}", h.0, h.1, h.2));
        }
        let j = |v: &[u32]| {
            v.iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        };
        out.push(format!("conds {}", j(&a.sets.list[a.any_cond as usize])));
        for (k, &t) in &a.outs {
            let must = driven.contains(k);
            out.push(format!(
                "out {} {} {} must={} deps {}",
                k.0,
                k.1,
                k.2,
                u8::from(must),
                j(&a.sets.list[t as usize])
            ));
        }
    }
    out
}
