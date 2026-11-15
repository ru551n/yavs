//! Cycle mode (`yavs run --cycle`): the processes that can run cycle-based form one region,
//! run by one kernel process on a static schedule with compiled code; everything else stays in
//! the event kernel. Read `AGENTS.md` first.

mod codegen;
mod lower;
mod region;
mod rt;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use yavs_diag::Diagnostic;
use yavs_elab::{Elaborated, LeafId, Region, RegionDriver};
use yavs_kernel::{EDGE_LEVEL, EdgeTrigger, Range, SimError};
use yavs_sem::Db;

use lower::{Kind, Op, Ref};

pub mod codes {
    /// `--cycle-strict`: a process below the top-level design unit that is not behavioural
    /// cannot run cycle-based.
    pub const LEFT_OUT: &str = "E4101";
    /// `--cycle` with an option that observes or changes signals inside the region.
    pub const NOT_WITH: &str = "E4102";
}

/// What `--cycle` decided for a design: the region report and the region to build.
pub struct Plan {
    /// The region report: what joined and what stays in the event kernel, with reasons.
    pub report: String,
    pub region: Option<Region>,
    /// What the region read as 0 for a metavalue, for [`Metas::summary`] after the run.
    pub metas: Arc<Metas>,
}

/// Elements the region read as 0 because they held a metavalue (`'U'`, `'X'`, `'Z'`, `'W'`,
/// `'-'`): at initialisation (signals and variables without an initial value included), and at
/// the inputs, counted at every read.
#[derive(Default)]
pub struct Metas {
    pub init: AtomicU64,
    pub inputs: AtomicU64,
}

impl Metas {
    /// The summary line of the metavalues the region read as 0, if it read any.
    pub fn summary(&self) -> Option<String> {
        let (i, n) = (
            self.init.load(Ordering::Relaxed),
            self.inputs.load(Ordering::Relaxed),
        );
        (i + n != 0).then(|| {
            format!(
                "cycle mode: {} metavalue elements read as 0 ({i} at initialisation, {n} at the \
                 region's inputs)\n",
                i + n
            )
        })
    }
}

/// Select the region of `e` and compile it. Under `strict`, a process left out that is
/// neither in the top-level design unit nor behavioural is an error (`E4101`), all of them
/// listed.
pub fn plan(db: &Db, e: &Elaborated, strict: bool) -> Result<Plan, Vec<Diagnostic>> {
    let sel = region::select(db, e);
    let reason = |pi: usize, why: &str| {
        if region::behavioural(db, e, pi) {
            "behavioural (timed or several waits, a procedure call or a file)".to_string()
        } else {
            why.to_string()
        }
    };
    if strict {
        let errs: Vec<Diagnostic> = (sel.out.iter())
            .filter(|(pi, _)| !region::in_top_unit(e, *pi) && !region::behavioural(db, e, *pi))
            .map(|(pi, why)| {
                Diagnostic::error(
                    codes::LEFT_OUT,
                    format!(
                        "process {} cannot run cycle-based",
                        e.hier.proc_label(&e.procs[*pi], yavs_elab::Case::Lrm)
                    ),
                )
                .with_primary(db.stmts[e.procs[*pi].stmt].span, why.clone())
                .with_note(
                    "`--cycle-strict` needs every process below the top-level design unit that \
                     is not behavioural to run cycle-based",
                )
                .with_help("run with `--cycle` to leave it in the event kernel")
            })
            .collect();
        if !errs.is_empty() {
            return Err(errs);
        }
    }
    let mut report = String::new();
    let total = e.procs.len();
    let _ = write!(
        report,
        "cycle mode: {} of {total} processes run cycle-based",
        sel.members.len()
    );
    if let Some(c) = sel.clock {
        let _ = write!(
            report,
            ", on the {} edge of {}",
            if c.rising { "rising" } else { "falling" },
            e.leaf_name(c.leaf)
        );
    }
    report.push('\n');
    let mut rows: Vec<(String, String)> = Vec::new();
    for (pi, b) in &sel.members {
        let k = match b.kind {
            Kind::Clocked => "clocked",
            Kind::Comb => "combinational",
        };
        let mut k = match b.dropped {
            0 => k.to_string(),
            1 => format!("{k}, 1 assert or report not evaluated in cycle mode"),
            n => format!("{k}, {n} asserts and reports not evaluated in cycle mode"),
        };
        match b.metas {
            0 => {}
            1 => k.push_str(", 1 literal with a metavalue read as 0"),
            n => {
                let _ = write!(k, ", {n} literals with a metavalue read as 0");
            }
        }
        rows.push((region::where_(db, e, *pi), k));
    }
    let joined = rows.len();
    for (pi, why) in &sel.out {
        rows.push((region::where_(db, e, *pi), reason(*pi, why)));
    }
    let w = rows.iter().map(|r| r.0.chars().count()).max().unwrap_or(0);
    for (i, (name, what)) in rows.iter().enumerate() {
        if i == 0 && joined > 0 {
            report.push_str("  cycle-based:\n");
        }
        if i == joined {
            report.push_str("  in the event kernel:\n");
        }
        let _ = writeln!(report, "    {name:<w$}  {what}");
    }
    let metas = Arc::new(Metas::default());
    if sel.members.is_empty() {
        return Ok(Plan {
            report,
            region: None,
            metas,
        });
    }
    let name = |l: &LeafId| e.leaf_name(*l);
    let _ = writeln!(
        report,
        "  boundary: {} signal leaves exported ({} nothing outside waits on), {} read at each \
         wake, {} wake the region{}",
        sel.outputs.len(),
        sel.outputs.difference(&sel.waited).count(),
        sel.inputs.len(),
        sel.wakes.len(),
        match sel.wakes.iter().map(name).collect::<Vec<_>>() {
            v if v.is_empty() => String::new(),
            v => format!(" ({})", v.join(", ")),
        }
    );
    let region = build(db, e, sel, metas.clone()).map_err(|m| {
        vec![
            Diagnostic::error(
                "E9999",
                format!("cycle mode cannot compile its region: {m}"),
            )
            .with_note("this is a bug in yavs"),
        ]
    })?;
    Ok(Plan {
        report,
        region: Some(region),
        metas,
    })
}

/// Lay out the image, schedule and compile the region, and make its kernel process.
fn build(
    _db: &Db,
    e: &Elaborated,
    sel: region::Selection,
    metas: Arc<Metas>,
) -> Result<Region, String> {
    let nets = &e.nets;
    let ksig = |l: LeafId| nets.loc[l.index()].signal.0;
    // A kernel signal of logic elements: its length; a byte signal (an integer): one word.
    let bytes = |k: u32| match nets.leaf(nets.packs[k as usize].0).ty.elem {
        yavs_kernel::Elem::Bytes(b) => b,
        yavs_kernel::Elem::Logic => 0,
    };
    let logic = |k: u32| bytes(k) == 0;
    let klen = |k: u32| {
        if logic(k) {
            nets.signal_len(yavs_kernel::SignalId(k))
        } else {
            64
        }
    };
    // Image: the words of each kernel signal of the region (its leaves at their kernel offsets),
    // then next-state words, then the slots of each member, then two words for a failed check.
    let mut cur: BTreeMap<u32, u32> = BTreeMap::new();
    let mut n = 0u32;
    let leaves = (sel.driven.iter().chain(&sel.inputs)).chain(
        sel.members
            .iter()
            .flat_map(|(_, b)| b.reads.iter().chain(&b.writes)),
    );
    for &l in leaves {
        let k = ksig(l);
        cur.entry(k).or_insert_with(|| {
            n += codegen::words(klen(k));
            n - codegen::words(klen(k))
        });
    }
    let mut next: BTreeMap<u32, u32> = BTreeMap::new();
    for &l in &sel.next {
        let k = ksig(l);
        next.entry(k).or_insert_with(|| {
            n += codegen::words(klen(k));
            n - codegen::words(klen(k))
        });
    }
    let mut settle = Vec::new();
    let mut edge = Vec::new();
    for (&k, &s) in &next {
        edge.push(Op::Copy {
            dst: Ref::Loc(s),
            src: Ref::Loc(cur[&k]),
            len: klen(k),
        });
    }
    let mut committed = false;
    let mut init: Vec<(u32, u64)> = Vec::new();
    let mut checks: Vec<(lower::Check, usize)> = Vec::new();
    let mut uninit = 0u64;
    for (pi, b) in &sel.members {
        let base = n;
        n += b.locs;
        init.extend(b.init.iter().map(|&(w, v)| (base + w, v)));
        let sites = checks.len() as u32;
        checks.extend(b.checks.iter().map(|c| (c.clone(), *pi)));
        uninit += b.uninit;
        let clocked = b.kind == Kind::Clocked;
        if !clocked && !committed {
            commit(&mut edge, &next, &cur, klen);
            committed = true;
        }
        let rs = Resolve {
            base,
            sites,
            clocked,
            cur: &cur,
            next: &next,
        };
        for op in &b.ops {
            let r = rs.op(op);
            if !clocked {
                settle.push(r.clone());
            }
            edge.push(r);
        }
    }
    if !committed {
        commit(&mut edge, &next, &cur, klen);
    }
    let err = n;
    n += 2;
    let top = (e.hier.scopes.iter())
        .position(|s| s.parent.is_none())
        .ok_or("the design has no top-level instance")?;
    let label = e.hier.dotted(yavs_elab::ScopeId(top as u32));
    let code = Arc::new(codegen::compile(&settle, &edge, n as usize, err, &label)?);
    let whole = |k: u32| {
        let s = yavs_kernel::SignalId(k);
        s.slice(0, nets.signal_len(s))
    };
    let all: Vec<_> = (cur.iter())
        .map(|(&k, &s)| rt::Read {
            r: whole(k),
            slot: s,
            lo: 0,
            len: nets.signal_len(yavs_kernel::SignalId(k)),
            bytes: bytes(k),
        })
        .collect();
    // Inputs leaf by leaf: a kernel signal may also hold leaves the region drives.
    let inputs: Vec<_> = (sel.inputs.iter())
        .map(|&l| {
            let len = nets.leaf(l).ty.len;
            let at = nets.loc[l.index()];
            rt::Read {
                r: nets.kref(l, Range::new(0, len)),
                slot: cur[&at.signal.0],
                lo: at.base,
                len,
                bytes: bytes(at.signal.0),
            }
        })
        .collect();
    let clock = sel.clock.map(|c| {
        let kind = u8::from(c.rising) | if c.legacy { EDGE_LEVEL } else { 0 };
        (nets.kref(c.leaf, Range::new(c.elem, 1)), kind)
    });
    let leaf_ref = |l: LeafId| nets.kref(l, Range::new(0, nets.leaf(l).ty.len));
    let mut waits: Vec<_> = clock.iter().map(|c| c.0).collect();
    waits.extend(sel.wakes.iter().map(|l| leaf_ref(*l)));
    let trigger = sel.clock.filter(|_| sel.wakes.is_empty()).map(|c| {
        let (signal, elem) = nets.kelem(c.leaf, c.elem);
        EdgeTrigger {
            signal,
            elem,
            rising: c.rising,
            legacy: c.legacy,
            reset: None,
        }
    });
    // The leaf of each driver declaration, for the exports.
    let decls: Vec<LeafId> = nets.drivers.iter().map(|d| d.leaf).collect();
    // Exported kernel signals none of whose leaves anything outside waits on: candidates for an
    // update without an event (the kernel decides at run time, `Builder::quiet`).
    let waited: std::collections::BTreeSet<u32> = sel.waited.iter().map(|&l| ksig(l)).collect();
    let quiet: Vec<u32> = (sel.outputs.iter())
        .map(|&l| ksig(l))
        .filter(|k| !waited.contains(k))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let outputs = sel.outputs.clone();
    let names: BTreeMap<usize, String> = (checks.iter())
        .map(|(_, pi)| (*pi, e.hier.proc_label(&e.procs[*pi], yavs_elab::Case::Lrm)))
        .collect();
    let checks: Arc<Vec<(lower::Check, usize, String)>> = Arc::new(
        checks
            .into_iter()
            .map(|(c, pi)| (c, pi, names[&pi].clone()))
            .collect(),
    );
    let sig_bytes: BTreeMap<u32, u32> = cur.keys().map(|&k| (k, bytes(k))).collect();
    let words = n as usize;
    let make: yavs_elab::RegionMake = Arc::new(move |ds: &[RegionDriver]| {
        // One export per kernel driver with an exported leaf: its range of the signal's words.
        let mut outs: BTreeMap<u32, rt::Out> = BTreeMap::new();
        for d in ds.iter().filter(|d| outputs.contains(&decls[d.decl])) {
            let Some(&slot) = cur.get(&d.signal.0) else {
                return Err(SimError::new(yavs_kernel::ErrorKind::Internal(
                    "cycle mode: an exported signal without words".into(),
                )));
            };
            outs.entry(d.driver.0).or_insert(rt::Out {
                driver: d.driver,
                range: d.range,
                slot,
                bytes: sig_bytes[&d.signal.0],
                last: Vec::new(),
            });
        }
        let mut img = vec![0u64; words];
        for &(w, v) in &init {
            img[w as usize] = v;
        }
        Ok(Box::new(rt::RegionProc {
            code: code.clone(),
            img,
            leaves: all.clone(),
            inputs: inputs.clone(),
            outs: outs.into_values().collect(),
            clock,
            waits: waits.clone(),
            started: false,
            checks: checks.clone(),
            err: err as usize,
            metas: metas.clone(),
            uninit,
            buf: Vec::new(),
        }) as Box<dyn yavs_kernel::Process>)
    });
    let mut members: Vec<usize> = sel.members.iter().map(|m| m.0).collect();
    members.sort_unstable();
    Ok(Region {
        name: format!("{}.cycle_region", label),
        members,
        trigger,
        quiet,
        make,
    })
}

fn commit(
    edge: &mut Vec<Op>,
    next: &BTreeMap<u32, u32>,
    cur: &BTreeMap<u32, u32>,
    klen: impl Fn(u32) -> u32,
) {
    for (&k, &s) in next {
        edge.push(Op::Copy {
            dst: Ref::Loc(cur[&k]),
            src: Ref::Loc(s),
            len: klen(k),
        });
    }
}

/// The operations of one member with every reference an image slot: a member's slots from
/// `base`, a signal its current words, except the next-state words where a clocked member writes
/// it; check sites from `sites`.
struct Resolve<'a> {
    base: u32,
    sites: u32,
    clocked: bool,
    cur: &'a BTreeMap<u32, u32>,
    next: &'a BTreeMap<u32, u32>,
}

impl Resolve<'_> {
    fn src(&self, r: Ref) -> Ref {
        match r {
            Ref::Sig(k) => Ref::Loc(self.cur[&k]),
            Ref::Loc(s) => Ref::Loc(self.base + s),
        }
    }

    fn dst(&self, r: Ref) -> Ref {
        match r {
            Ref::Sig(k) if self.clocked && self.next.contains_key(&k) => Ref::Loc(self.next[&k]),
            r => self.src(r),
        }
    }

    fn op(&self, op: &Op) -> Op {
        let (s, d) = (|r| self.src(r), |r| self.dst(r));
        let site = |x: u32| self.sites + x;
        match op {
            Op::Copy { dst, src, len } => Op::Copy {
                dst: d(*dst),
                src: s(*src),
                len: *len,
            },
            Op::Lit { dst, val } => Op::Lit {
                dst: d(*dst),
                val: val.clone(),
            },
            // A blend, insert or element write reads the words it writes: the next-state ones
            // where it writes those.
            Op::Blend { dst, src, mask } => Op::Blend {
                dst: d(*dst),
                src: s(*src),
                mask: mask.clone(),
            },
            Op::Gate { g, dst, a, b, len } => Op::Gate {
                g: *g,
                dst: d(*dst),
                a: s(*a),
                b: s(*b),
                len: *len,
            },
            Op::Not { dst, a, len } => Op::Not {
                dst: d(*dst),
                a: s(*a),
                len: *len,
            },
            Op::Extract { dst, src, lo, len } => Op::Extract {
                dst: d(*dst),
                src: s(*src),
                lo: *lo,
                len: *len,
            },
            Op::Insert { dst, src, lo, len } => Op::Insert {
                dst: d(*dst),
                src: s(*src),
                lo: *lo,
                len: *len,
            },
            Op::Cat {
                dst,
                hi,
                lo,
                lo_len,
                len,
            } => Op::Cat {
                dst: d(*dst),
                hi: s(*hi),
                lo: s(*lo),
                lo_len: *lo_len,
                len: *len,
            },
            Op::Add {
                dst,
                a,
                b,
                len,
                sub,
            } => Op::Add {
                dst: d(*dst),
                a: s(*a),
                b: s(*b),
                len: *len,
                sub: *sub,
            },
            Op::VecEq { dst, a, b, len, ne } => Op::VecEq {
                dst: d(*dst),
                a: s(*a),
                b: s(*b),
                len: *len,
                ne: *ne,
            },
            Op::IntLit { dst, v } => Op::IntLit {
                dst: d(*dst),
                v: *v,
            },
            Op::IntCmp { op, dst, a, b } => Op::IntCmp {
                op: *op,
                dst: d(*dst),
                a: s(*a),
                b: s(*b),
            },
            Op::Int {
                op,
                dst,
                a,
                b,
                lo,
                hi,
                site: x,
            } => Op::Int {
                op: *op,
                dst: d(*dst),
                a: s(*a),
                b: s(*b),
                lo: *lo,
                hi: *hi,
                site: site(*x),
            },
            Op::Range { a, lo, hi, site: x } => Op::Range {
                a: s(*a),
                lo: *lo,
                hi: *hi,
                site: site(*x),
            },
            Op::IndexRead {
                dst,
                src,
                lo,
                n,
                idx,
                left,
                down,
                site: x,
            } => Op::IndexRead {
                dst: d(*dst),
                src: s(*src),
                lo: *lo,
                n: *n,
                idx: s(*idx),
                left: *left,
                down: *down,
                site: site(*x),
            },
            Op::IndexWrite {
                dst,
                src,
                lo,
                n,
                idx,
                left,
                down,
                site: x,
            } => Op::IndexWrite {
                dst: d(*dst),
                src: s(*src),
                lo: *lo,
                n: *n,
                idx: s(*idx),
                left: *left,
                down: *down,
                site: site(*x),
            },
            Op::If { cond, then, else_ } => Op::If {
                cond: s(*cond),
                then: then.iter().map(|o| self.op(o)).collect(),
                else_: else_.iter().map(|o| self.op(o)).collect(),
            },
        }
    }
}
