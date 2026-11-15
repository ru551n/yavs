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
        rows.push((region::where_(db, e, *pi), k.into()));
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
    if sel.members.is_empty() {
        return Ok(Plan {
            report,
            region: None,
        });
    }
    let name = |l: &LeafId| e.leaf_name(*l);
    let _ = writeln!(
        report,
        "  boundary: {} signal leaves exported, {} read at each wake, {} wake the region{}",
        sel.outputs.len(),
        sel.inputs.len(),
        sel.wakes.len(),
        match sel.wakes.iter().map(name).collect::<Vec<_>>() {
            v if v.is_empty() => String::new(),
            v => format!(" ({})", v.join(", ")),
        }
    );
    let region = build(db, e, sel).map_err(|m| {
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
    })
}

/// Lay out the image, schedule and compile the region, and make its kernel process.
fn build(_db: &Db, e: &Elaborated, sel: region::Selection) -> Result<Region, String> {
    // Image: the current value of every signal leaf of the region, then next-state blocks,
    // then the slots of each member.
    let mut cur: BTreeMap<LeafId, u32> = BTreeMap::new();
    let mut n = 0u32;
    let leaves = (sel.driven.iter().chain(&sel.inputs)).chain(
        sel.members
            .iter()
            .flat_map(|(_, b)| b.reads.iter().chain(&b.writes)),
    );
    for &l in leaves {
        cur.entry(l).or_insert_with(|| {
            n += 1;
            n - 1
        });
    }
    let mut next: BTreeMap<LeafId, u32> = BTreeMap::new();
    for &l in &sel.next {
        next.insert(l, n);
        n += 1;
    }
    let mut settle = Vec::new();
    let mut edge = Vec::new();
    for (&l, &s) in &next {
        edge.push(Op::Copy {
            dst: Ref::Loc(s),
            src: Ref::Loc(cur[&l]),
        });
    }
    let mut committed = false;
    for (_, b) in &sel.members {
        let base = n;
        n += b.locs;
        let clocked = b.kind == Kind::Clocked;
        if !clocked && !committed {
            commit(&mut edge, &next, &cur);
            committed = true;
        }
        for op in &b.ops {
            let r = resolve(op, base, clocked, &cur, &next);
            if !clocked {
                settle.push(r.clone());
            }
            edge.push(r);
        }
    }
    if !committed {
        commit(&mut edge, &next, &cur);
    }
    let top = (e.hier.scopes.iter())
        .position(|s| s.parent.is_none())
        .ok_or("the design has no top-level instance")?;
    let label = e.hier.dotted(yavs_elab::ScopeId(top as u32));
    let code = Arc::new(codegen::compile(&settle, &edge, n as usize, &label)?);
    let nets = &e.nets;
    let whole = |l: LeafId| nets.kref(l, Range::new(0, nets.leaf(l).ty.len));
    let all: Vec<_> = cur.iter().map(|(&l, &s)| (whole(l), s)).collect();
    let inputs: Vec<_> = sel.inputs.iter().map(|l| (whole(*l), cur[l])).collect();
    let clock = sel.clock.map(|c| {
        let kind = u8::from(c.rising) | if c.legacy { EDGE_LEVEL } else { 0 };
        (nets.kref(c.leaf, Range::new(c.elem, 1)), kind)
    });
    let mut waits: Vec<_> = clock.iter().map(|c| c.0).collect();
    waits.extend(sel.wakes.iter().map(|l| whole(*l)));
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
    // The leaf, kernel signal and base of each driver declaration, for the exports.
    let decls: Vec<_> = nets
        .drivers
        .iter()
        .map(|d| (d.leaf, d.range, nets.loc[d.leaf.index()].base))
        .collect();
    let outputs = sel.outputs.clone();
    let slots = n as usize;
    let make: yavs_elab::RegionMake = Arc::new(move |ds: &[RegionDriver]| {
        let mut outs: BTreeMap<u32, rt::Out> = BTreeMap::new();
        let exported: std::collections::BTreeSet<u32> = (ds.iter())
            .filter(|d| outputs.contains(&decls[d.decl].0))
            .map(|d| d.driver.0)
            .collect();
        for d in ds.iter().filter(|d| exported.contains(&d.driver.0)) {
            if d.range.len > 64 {
                return Err(SimError::new(yavs_kernel::ErrorKind::Usage(
                    "cycle mode: a driver of more than 64 elements".into(),
                )));
            }
            let (leaf, lr, base) = decls[d.decl];
            // Elements `lr` of the leaf, at kernel elements `base + lr` of the signal.
            let start = base + lr.offset;
            let part = (cur[&leaf], lr.offset, start - d.range.offset, lr.len);
            let o = outs.entry(d.driver.0).or_insert_with(|| rt::Out {
                driver: d.driver,
                range: d.range,
                parts: Vec::new(),
                last: None,
            });
            if !o.parts.contains(&part) {
                o.parts.push(part);
            }
        }
        Ok(Box::new(rt::RegionProc {
            code: code.clone(),
            img: vec![[0; 4]; slots],
            leaves: all.clone(),
            inputs: inputs.clone(),
            outs: outs.into_values().collect(),
            clock,
            waits: waits.clone(),
            started: false,
        }) as Box<dyn yavs_kernel::Process>)
    });
    let mut members: Vec<usize> = sel.members.iter().map(|m| m.0).collect();
    members.sort_unstable();
    Ok(Region {
        name: format!("{}.cycle_region", label),
        members,
        trigger,
        make,
    })
}

fn commit(edge: &mut Vec<Op>, next: &BTreeMap<LeafId, u32>, cur: &BTreeMap<LeafId, u32>) {
    for (&l, &s) in next {
        edge.push(Op::Copy {
            dst: Ref::Loc(cur[&l]),
            src: Ref::Loc(s),
        });
    }
}

/// `op` with every reference an image slot: a member's slots from `base`, a signal its
/// current block, except the next-state block where a clocked member writes it.
fn resolve(
    op: &Op,
    base: u32,
    clocked: bool,
    cur: &BTreeMap<LeafId, u32>,
    next: &BTreeMap<LeafId, u32>,
) -> Op {
    let src = |r: Ref| match r {
        Ref::Sig(l) => Ref::Loc(cur[&l]),
        Ref::Loc(s) => Ref::Loc(base + s),
    };
    let dst = |r: Ref| match r {
        Ref::Sig(l) if clocked && next.contains_key(&l) => Ref::Loc(next[&l]),
        r => src(r),
    };
    match *op {
        Op::Copy { dst: d, src: s } => Op::Copy {
            dst: dst(d),
            src: src(s),
        },
        Op::Lit { dst: d, val } => Op::Lit { dst: dst(d), val },
        Op::Gate {
            g,
            dst: d,
            a,
            b,
            len,
        } => Op::Gate {
            g,
            dst: dst(d),
            a: src(a),
            b: src(b),
            len,
        },
        Op::Not { dst: d, a, len } => Op::Not {
            dst: dst(d),
            a: src(a),
            len,
        },
        Op::Extract {
            dst: d,
            src: s,
            lo,
            len,
        } => Op::Extract {
            dst: dst(d),
            src: src(s),
            lo,
            len,
        },
        // An insert reads the block it writes: the next-state one where it writes that.
        Op::Insert {
            dst: d,
            src: s,
            lo,
            len,
        } => Op::Insert {
            dst: dst(d),
            src: src(s),
            lo,
            len,
        },
        Op::Cat {
            dst: d,
            hi,
            lo,
            lo_len,
        } => Op::Cat {
            dst: dst(d),
            hi: src(hi),
            lo: src(lo),
            lo_len,
        },
        Op::AddK { dst: d, a, len, k } => Op::AddK {
            dst: dst(d),
            a: src(a),
            len,
            k,
        },
    }
}
