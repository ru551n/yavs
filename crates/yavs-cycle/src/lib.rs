//! Cycle mode (`yavs run --cycle`): the processes that can run cycle-based form one region,
//! run by one kernel process on a static schedule with compiled code; everything else stays in
//! the event kernel. Read `AGENTS.md` first.

mod check;
mod codegen;
mod dep;
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

/// Codes of cycle mode. Under `--cycle-strict` every reason a process below the top-level unit
/// stays out is an error with its code; under `--cycle` the report gives the same codes.
pub mod codes {
    /// The process is in the subset, but cycle mode does not implement something it needs yet.
    pub const LEFT_OUT: &str = "E4101";
    /// `--cycle` with an option that observes or changes signals inside the region.
    pub const NOT_WITH: &str = "E4102";
    /// The file of `--cycle-report=FILE` cannot be written.
    pub const REPORT_FILE: &str = "E4103";
    /// A wait other than the one `wait until <edge>;` of a clocked process.
    pub const WAIT: &str = "E4110";
    /// `after`, `transport`, `reject`, a waveform of several elements.
    pub const DELAY: &str = "E4111";
    /// Signal history: `'event` outside the clock's edge test, `'last_value`, `'stable`, ...
    pub const HISTORY: &str = "E4112";
    /// A real or physical (time) value computed at run time.
    pub const NOT_TWO_STATE: &str = "E4113";
    /// Shared variables, methods of protected objects, allocators and access values.
    pub const TESTBENCH: &str = "E4114";
    /// Postponed processes, guarded blocks, signals and assignments.
    pub const GUARDED: &str = "E4115";
    /// `force` and `release`.
    pub const FORCE: &str = "E4116";
    /// A call the analysis cannot follow: foreign, no body, or (combinational) changing state
    /// outside the subprogram.
    pub const CALL: &str = "E4117";
    /// A `'Z'` value.
    pub const HIGH_Z: &str = "E4118";
    /// A combinational process that does not assign a driven element on every path.
    pub const LATCH: &str = "E4119";
    /// A variable of a combinational process read before it is assigned.
    pub const STATE: &str = "E4120";
    /// A combinational process reading a signal its sensitivity list does not name.
    pub const SENSITIVITY: &str = "E4121";
    /// A loop in the combinational logic, element by element.
    pub const LOOP: &str = "E4122";
    /// A clocked process of a shape cycle mode does not take.
    pub const SHAPE: &str = "E4123";
    /// A signal it drives has another source: another process, a resolution, a guard.
    pub const SHARED: &str = "E4124";
    /// A clock made inside the region: its combinational maker, or the users of a register-made
    /// one.
    pub const CLOCK_MADE: &str = "E4125";
    /// The analysis cannot prove the process fits (it does not follow a construct).
    pub const ANALYSIS: &str = "E4126";
    /// A process outside the region reads the history of a signal the process drives
    /// (`'active`, `'last_active`, `'last_event`, `'last_value`, `'transaction`, `'stable`,
    /// `'quiet`, `'delayed`).
    pub const HISTORY_READ: &str = "E4127";
}

/// What the command line asked of cycle mode.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// `--cycle`: leave out what does not fit, say so in the summary line.
    Run,
    /// `--cycle-strict`: what does not fit below the top-level unit is an error.
    Strict,
    /// `--cycle-report`: the region report only; nothing is compiled or run.
    Report,
}

/// What cycle mode decided for a design: the region report and the region to build.
pub struct Plan {
    /// The region report: what joined and what stays in the event kernel, with reasons.
    pub report: String,
    /// One line for `--cycle`: how many processes joined (ends with a newline).
    pub summary: String,
    /// None under [`Mode::Report`] and when no process joined.
    pub region: Option<Region>,
    /// What the region read as 0 for a metavalue, for [`Metas::summary`] after the run.
    pub metas: Arc<Metas>,
}

/// A short name of what a code stands for, for the headings of the region report.
fn title(code: &str) -> &'static str {
    use codes::*;
    match code {
        WAIT => "a wait that is not the clock edge",
        DELAY => "a delay, `transport`, `reject` or a waveform",
        HISTORY => "signal history read",
        NOT_TWO_STATE => "a real or time value computed at run time",
        TESTBENCH => "a shared variable, protected object or access value",
        GUARDED => "a postponed process or a guarded signal",
        FORCE => "`force` or `release`",
        CALL => "a call the analysis cannot follow",
        HIGH_Z => "a `'Z'` value",
        LATCH => "a latch",
        STATE => "a variable read before it is assigned",
        SENSITIVITY => "an incomplete sensitivity list",
        LOOP => "a combinational loop",
        SHAPE => "a clocked process of another shape",
        SHARED => "a signal with another source",
        CLOCK_MADE => "a clock made inside the region",
        ANALYSIS => "the analysis cannot prove it fits",
        HISTORY_READ => "its history is read outside the region",
        _ => "",
    }
}

/// Elements the region read as 0 because they held a metavalue (`'U'`, `'X'`, `'Z'`, `'W'`,
/// `'-'`): at initialisation (signals and variables without an initial value included), and at
/// the inputs, counted when a chunk of them is read again after a change.
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
                "cycle mode: {} metavalue elements read as 0 ({i} at initialisation, {n} at \
                 changes of the region's inputs)\n",
                i + n
            )
        })
    }
}

/// `file:line` of a span.
fn at(db: &Db, span: yavs_diag::Span) -> String {
    let f = db.sources.get(span.file);
    let file = std::path::Path::new(&f.path)
        .file_name()
        .map_or(f.path.clone(), |n| n.to_string_lossy().into_owned());
    format!("{file}:{}", f.line_col(span.start).0)
}

/// Select the region of `e` and compile it. Under [`Mode::Strict`], a process left out that is
/// neither in the top-level design unit nor behavioural is an error with the code of its
/// reason, all of them listed. Under [`Mode::Report`] only the report is made, nothing is
/// compiled.
pub fn plan(db: &Db, e: &Elaborated, mode: Mode) -> Result<Plan, Vec<Diagnostic>> {
    let sel = region::select(db, e);
    let label = |pi: usize| e.hier.proc_label(&e.procs[pi], yavs_elab::Case::Lrm);
    if mode == Mode::Strict {
        let errs: Vec<Diagnostic> = (sel.out.iter())
            .filter(|(pi, why)| {
                !region::in_top_unit(e, *pi) && !matches!(why, region::Why::Behavioural(_))
            })
            .map(|(pi, why)| {
                let note = "`--cycle-strict` needs every process below the top-level design \
                            unit that is not behavioural to run cycle-based";
                let help = "run with `--cycle` to leave it in the event kernel";
                match why {
                    region::Why::Subset(r) => Diagnostic::error(
                        r.code,
                        format!("process {} is outside what cycle mode can run", label(*pi)),
                    )
                    .with_primary(r.span, r.text.clone()),
                    _ => Diagnostic::error(
                        codes::LEFT_OUT,
                        format!("process {} cannot run cycle-based yet", label(*pi)),
                    )
                    .with_primary(
                        db.stmts[e.procs[*pi].stmt].span,
                        match why {
                            region::Why::NotYet(w) => w.clone(),
                            _ => String::new(),
                        },
                    ),
                }
                .with_note(note)
                .with_help(help)
            })
            .collect();
        if !errs.is_empty() {
            return Err(errs);
        }
    }
    let mut report = String::new();
    let total = e.procs.len();
    let head = format!(
        "cycle mode: {} of {total} processes run cycle-based",
        sel.members.len()
    );
    let summary = match total - sel.members.len() {
        0 => format!("{head}\n"),
        n => format!("{head}, {n} in the event kernel (see --cycle-report)\n"),
    };
    report.push_str(&head);
    let edges: Vec<String> = (sel.clocks.iter())
        .map(|c| {
            format!(
                "the {} edge of {}{}",
                if c.rising { "rising" } else { "falling" },
                region::clock_name(e, c),
                if c.legacy { " (`'event` form)" } else { "" }
            )
        })
        .collect();
    if !edges.is_empty() {
        let _ = write!(report, ", on {}", edges.join(" and "));
    }
    report.push('\n');
    // The processes left out, grouped by reason (its code; behavioural processes and the ones
    // not implemented yet are one group each), the most common first. Rows: where, what.
    let mut left: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for (pi, why) in &sel.out {
        let (head, text) = match why {
            region::Why::Subset(r) => (
                format!("{} {}", r.code, title(r.code)),
                format!("{} {} ({})", r.code, r.text, at(db, r.span)),
            ),
            region::Why::Behavioural(w) => {
                ("behavioural, in the event kernel".to_string(), w.clone())
            }
            region::Why::NotYet(w) => (
                format!("{} not implemented in cycle mode yet", codes::LEFT_OUT),
                w.clone(),
            ),
        };
        left.entry(head)
            .or_default()
            .push((region::where_(db, e, *pi), text));
    }
    let mut left: Vec<_> = left.into_iter().collect();
    left.sort_by_key(|g| std::cmp::Reverse(g.1.len()));
    let members: Vec<(String, String)> = (sel.members.iter())
        .map(|(pi, b)| {
            let k = match b.kind {
                Kind::Clocked => "clocked",
                Kind::Comb => "combinational",
            };
            let mut k = k.to_string();
            match b.metas {
                0 => {}
                1 => k.push_str(", 1 literal with a metavalue read as 0"),
                n => {
                    let _ = write!(k, ", {n} literals with a metavalue read as 0");
                }
            }
            (region::where_(db, e, *pi), k)
        })
        .collect();
    let w = (left.iter().flat_map(|g| &g.1).chain(&members))
        .map(|r| r.0.chars().count())
        .max()
        .unwrap_or(0)
        .min(60); // one long name does not pad every row
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    if !sel.out.is_empty() {
        let _ = writeln!(
            report,
            "  {} stay in the event kernel:",
            plural(sel.out.len(), "process", "processes")
        );
    }
    for (head, rows) in &left {
        let _ = writeln!(
            report,
            "    {head} ({}):",
            plural(rows.len(), "process", "processes")
        );
        for (name, what) in rows {
            let _ = writeln!(report, "      {name:<w$}  {what}");
        }
    }
    // Asserts and reports in members are compiled to nothing.
    let held: Vec<(String, u32)> = (sel.members.iter())
        .filter(|(_, b)| b.dropped > 0)
        .map(|(pi, b)| (region::where_(db, e, *pi), b.dropped))
        .collect();
    if !held.is_empty() {
        let _ = writeln!(
            report,
            "  {} and reports not evaluated in cycle mode, held by {}:",
            plural(held.iter().map(|h| h.1 as usize).sum(), "assert", "asserts"),
            plural(held.len(), "cycle-based process", "cycle-based processes")
        );
        for (name, n) in &held {
            let _ = writeln!(report, "    {n:>4}  {name}");
        }
    }
    if !sel.members.is_empty() {
        report.push_str(
            "  cycle-based processes give no warnings of the ieee libraries (numeric_std \
             metavalue and compare warnings)\n",
        );
    }
    let _ = writeln!(
        report,
        "  subset: {} of {total} processes fit, {} loop group{} among them",
        sel.fits,
        sel.groups.len(),
        if sel.groups.len() == 1 { "" } else { "s" }
    );
    for g in &sel.groups {
        let names: Vec<String> = g.procs.iter().take(3).map(|&pi| label(pi)).collect();
        let more = match g.procs.len() {
            n if n > 3 => format!(" and {} more", n - 3),
            _ => String::new(),
        };
        let _ = writeln!(
            report,
            "    loop group of {} process{}: {} passes, or {} pieces after a split: {}{more}",
            g.procs.len(),
            if g.procs.len() == 1 { "" } else { "es" },
            g.k,
            g.slices,
            names.join(", ")
        );
    }
    for l in &sel.loops {
        let _ = writeln!(
            report,
            "    in the region: a loop group of {} process{}, {} process runs per settle ({} passes \
             of all would be {})",
            l.procs,
            if l.procs == 1 { "" } else { "es" },
            l.at.len(),
            l.k,
            l.procs * l.k as usize
        );
    }
    let metas = Arc::new(Metas::default());
    if sel.members.is_empty() {
        return Ok(Plan {
            report,
            summary,
            region: None,
            metas,
        });
    }
    let name = |l: &LeafId| e.leaf_name(*l);
    let _ = writeln!(
        report,
        "  boundary: {} signal leaves exported ({} nothing outside waits on), {} inputs, {} wake \
         the region{}",
        sel.outputs.len(),
        sel.outputs.difference(&sel.waited).count(),
        sel.inputs.len(),
        sel.wakes.len(),
        match sel.wakes.iter().map(name).collect::<Vec<_>>() {
            v if v.is_empty() => String::new(),
            v => format!(" ({})", v.join(", ")),
        }
    );
    // Which wakes are settled at once, and why; the others wait for the next edge.
    if !sel.wakes.is_empty() {
        let why = |r: &region::Reach| match *r {
            region::Reach::Output(o, Some(p)) => format!(
                "{} read by {}",
                e.leaf_name(o),
                e.hier.proc_label(&e.procs[p], yavs_elab::Case::Lrm)
            ),
            region::Reach::Output(o, None) => format!("{} read outside", e.leaf_name(o)),
            region::Reach::Check(p) => format!(
                "a check in {}",
                e.hier.proc_label(&e.procs[p], yavs_elab::Case::Lrm)
            ),
        };
        let at_once: Vec<String> = (sel.at_once.iter())
            .map(|(l, r)| format!("{} to {}", name(l), why(r)))
            .collect();
        let _ = writeln!(
            report,
            "  wakes: {} settled at once, through combinational code to what outside sees{}; {} \
             read when they change and settled at the next edge",
            at_once.len(),
            if at_once.is_empty() {
                String::new()
            } else {
                format!(" ({})", at_once.join(", "))
            },
            sel.wakes.len() - at_once.len()
        );
    }
    let _ = writeln!(
        report,
        "  cycle-based, {}:",
        plural(members.len(), "process", "processes")
    );
    for (name, what) in &members {
        let _ = writeln!(report, "    {name:<w$}  {what}");
    }
    if mode == Mode::Report {
        return Ok(Plan {
            report,
            summary,
            region: None,
            metas,
        });
    }
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
        summary,
        region: Some(region),
        metas,
    })
}

/// Lay out the image, schedule and compile the region, and make its kernel process.
fn build(
    db: &Db,
    e: &Elaborated,
    sel: region::Selection,
    metas: Arc<Metas>,
) -> Result<Region, String> {
    let nets = &e.nets;
    let ksig = |l: LeafId| nets.loc[l.index()].signal.0;
    // A kernel signal of logic elements: its length; a byte signal (integers or enumerations): a
    // word per element.
    let bytes = |k: u32| match nets.leaf(nets.packs[k as usize].0).ty.elem {
        yavs_kernel::Elem::Bytes(b) => b,
        yavs_kernel::Elem::Logic => 0,
    };
    let logic = |k: u32| bytes(k) == 0;
    let klen = |k: u32| {
        let n = nets.signal_len(yavs_kernel::SignalId(k));
        if logic(k) { n } else { 64 * n }
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
    // Memories clocked members write: their writes wait for the commit (`defer`), so they need no
    // next-state block (`region::ram_rule` kept only writes at run-time indices).
    let mut rams: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    for (_, b) in sel.members.iter().filter(|(_, b)| b.kind == Kind::Clocked) {
        let mut w = BTreeMap::new();
        region::sig_writes(&b.ops, &mut w);
        rams.extend(
            (w.into_iter())
                .filter(|&(k, h)| h.indexed && region::big_signal(e, k))
                .map(|(k, _)| k),
        );
    }
    let mut next: BTreeMap<u32, u32> = BTreeMap::new();
    for &l in &sel.next {
        let k = ksig(l);
        if rams.contains(&k) {
            continue;
        }
        next.entry(k).or_insert_with(|| {
            n += codegen::words(klen(k));
            n - codegen::words(klen(k))
        });
    }
    // Signals hold state between calls; so do the variables of clocked processes.
    let signal_words = n;
    // One word per edge class: 1 when its edge happened in this wake (`Ref::Edge` of its
    // members).
    let edge_words: Vec<u32> = (0..sel.clocks.len() as u32).map(|k| n + k).collect();
    n += sel.clocks.len().max(1) as u32;
    let several = sel.clocks.len() > 1;
    let mut state: Vec<(u32, u32)> = Vec::new();
    // Each function as pieces, a member's body each (`codegen::compile` cuts between them).
    let mut settle: Vec<codegen::Step> = Vec::new();
    let mut edge: Vec<codegen::Step> = Vec::new();
    let mut init_fn: Vec<codegen::Step> = Vec::new();
    edge.push(codegen::Step::Ops(
        (next.iter())
            .map(|(&k, &s)| Op::Copy {
                dst: Ref::Loc(s),
                src: Ref::Loc(cur[&k]),
                len: klen(k),
            })
            .collect(),
    ));
    let mut init: Vec<(u32, u64)> = Vec::new();
    let mut checks: Vec<(lower::Check, usize)> = Vec::new();
    let mut uninit = 0u64;
    // Each member's slots and resolution.
    let none = BTreeMap::new();
    let mut rs: Vec<Resolve> = Vec::new();
    // Per member: its clocked body with the memory writes kept back, and those writes.
    let mut deferred: Vec<(Vec<Op>, Vec<Pending>)> = Vec::new();
    for (pi, b) in &sel.members {
        let base = n;
        n += b.locs;
        let mut pend = Vec::new();
        let ops = if b.kind == Kind::Clocked && !rams.is_empty() {
            let mut at = b.locs;
            let ops = defer(&b.ops, &rams, &mut at, &mut pend);
            // The writes' words hold state from the body to the commit.
            state.push((base + b.locs, at - b.locs));
            n += at - b.locs;
            ops
        } else {
            Vec::new()
        };
        deferred.push((ops, pend));
        init.extend(b.init.iter().map(|&(w, v)| (base + w, v)));
        let sites = checks.len() as u32;
        checks.extend(b.checks.iter().map(|c| (c.clone(), *pi)));
        uninit += b.uninit;
        let clocked = b.kind == Kind::Clocked;
        if clocked {
            state.extend(b.vars.iter().map(|&(w, c)| (base + w, c)));
        }
        let class = b
            .clock
            .and_then(|c| sel.clocks.iter().position(|&k| k == c));
        rs.push(Resolve {
            base,
            sites,
            clocked,
            edge: class.map_or(signal_words, |k| edge_words[k]),
            cur: &cur,
            next: &next,
            snap: &none,
        });
    }
    // The settle: the combinational members and, while their reset holds, the reset branches
    // of clocked ones (written to the current words: the commit is past). A member of a loop group
    // is a body of its own, run at each pass level of what it drives: checked at its last run, on
    // settled values, and without checks before.
    let piece = |k: usize, snap: &BTreeMap<u32, u32>| -> Result<Vec<Op>, String> {
        let (_, b) = &sel.members[k];
        let mut ops: Vec<Op> = (snap.iter())
            .map(|(&s, &w)| Op::Copy {
                dst: Ref::Loc(w),
                src: Ref::Loc(cur[&s]),
                len: klen(s),
            })
            .collect();
        let r = Resolve {
            clocked: false,
            snap,
            ..rs[k]
        };
        if b.kind == Kind::Clocked {
            let piece = region::reset_piece(&b.ops).ok_or("a reset branch of another shape")?;
            ops.extend(piece.iter().map(|op| r.op(op)));
        } else {
            ops.extend(b.ops.iter().map(|op| r.op(op)));
        }
        Ok(ops)
    };
    let stats = std::env::var_os("YAVS_CYCLE_STATS").is_some();
    if stats {
        shapes(db, e, &sel, &next);
    }
    // One body per shape: members of `SHARE_MIN` operations or more, not in a loop group, whose
    // code is the same but for where their words lie; a combinational member's settle body, a
    // clocked member's edge body.
    let looped: std::collections::BTreeSet<usize> = (sel.loops.iter())
        .flat_map(|l| sel.settle[l.at.clone()].iter().copied())
        .collect();
    let mut by_key: std::collections::HashMap<ShapeKey, Vec<usize>> = Default::default();
    for (k, (_, b)) in sel.members.iter().enumerate() {
        // A member with memory writes kept back for the commit keeps its own code.
        if !looped.contains(&k)
            && deferred[k].1.is_empty()
            && b.ops.iter().map(codegen::weight_of).sum::<usize>() >= SHARE_MIN
        {
            by_key.entry(shape_key(b, &next)).or_default().push(k);
        }
    }
    // In the order of their first members, so the code is the same at every run.
    let mut by_key: Vec<Vec<usize>> = by_key.into_values().collect();
    by_key.sort_unstable();
    // Per member that shares: its shape and its blocks.
    let mut shared: BTreeMap<usize, (usize, Blocks)> = BTreeMap::new();
    // Per shape: its leader's body and blocks.
    let mut leaders: Vec<(Vec<Op>, Blocks, Resolve)> = Vec::new();
    for ks in by_key.into_iter().filter(|ks| ks.len() > 1) {
        let body = |k: usize| {
            let b = &sel.members[k].1;
            shared_body(&b.ops, b.kind == Kind::Clocked, several)
        };
        let lead = blocks(&body(ks[0]), &sel.members[ks[0]].1, &rs[ks[0]], klen);
        let mut members = Vec::new();
        for &k in &ks {
            let bl = blocks(&body(k), &sel.members[k].1, &rs[k], klen);
            // The same blocks in the same order with the same lengths, or the member keeps
            // its own code.
            if bl.len() == lead.len()
                && (bl.iter().zip(&lead)).all(|(x, y)| same_kind(x.0, y.0) && x.2 == y.2)
            {
                members.push((k, bl));
            }
        }
        if members.len() > 1 {
            let sh = leaders.len();
            leaders.push((body(members[0].0), lead, rs[members[0].0]));
            shared.extend(members.into_iter().map(|(k, bl)| (k, (sh, bl))));
        }
    }
    if stats {
        eprintln!(
            "cycle stats: {} members share {} bodies",
            shared.len(),
            leaders.len()
        );
    }
    let call = |k: usize| -> codegen::Step {
        let (shape, bl) = &shared[&k];
        codegen::Step::Shared {
            shape: *shape,
            table: bl.iter().map(|b| b.1).collect(),
            sites: rs[k].sites,
        }
    };
    let confirm = codegen::confirm_on();
    let mut bodies: Vec<(Vec<Op>, bool)> = Vec::new();
    let mut groups: Vec<(Vec<(u32, u32)>, u32)> = Vec::new();
    let mut at = 0;
    for l in &sel.loops {
        for &k in &sel.settle[at..l.at.start] {
            if sel.members[k].1.kind != Kind::Clocked && shared.contains_key(&k) {
                settle.push(call(k));
            } else {
                settle.push(codegen::Step::Ops(piece(k, &none)?));
            }
        }
        at = l.at.end;
        let steps = &sel.settle[l.at.clone()];
        // Per member: its bodies (checked, and unchecked when it runs more than once).
        let mut body: BTreeMap<usize, (usize, Option<usize>)> = BTreeMap::new();
        let mut ks: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
        for &k in steps {
            if body.contains_key(&k) {
                continue;
            }
            let b = &sel.members[k].1;
            ks.extend(b.writes.iter().map(|&w| ksig(w)));
            // It may read what it drives: from a copy made at its start, the values from before
            // it ran, as a process reads them (its writes are in place).
            let mut snap: BTreeMap<u32, u32> = BTreeMap::new();
            for &l in b.reads.intersection(&b.writes) {
                snap.entry(ksig(l)).or_insert_with(|| {
                    n += codegen::words(klen(ksig(l)));
                    n - codegen::words(klen(ksig(l)))
                });
            }
            let ops = piece(k, &snap)?;
            let early = (steps.iter().filter(|&&s| s == k).count() > 1).then(|| {
                bodies.push((ops.clone(), false));
                bodies.len() - 1
            });
            bodies.push((ops, true));
            body.insert(k, (bodies.len() - 1, early));
        }
        for (i, &k) in steps.iter().enumerate() {
            let (checked, early) = body[&k];
            let last = !steps[i + 1..].contains(&k);
            settle.push(codegen::Step::Call(if last {
                checked
            } else {
                early.expect("a member run twice has a body without checks")
            }));
        }
        // Debug builds: one more pass changes nothing the group drives.
        let words: Vec<(u32, u32)> = (ks.into_iter())
            .map(|k| (cur[&k], codegen::words(klen(k))))
            .collect();
        if confirm {
            let g = groups.len();
            settle.push(codegen::Step::Save(g));
            settle.extend(body.values().map(|b| codegen::Step::Call(b.0)));
            settle.push(codegen::Step::Same(g));
        }
        groups.push((words, 0));
    }
    for &k in &sel.settle[at..] {
        if sel.members[k].1.kind != Kind::Clocked && shared.contains_key(&k) {
            settle.push(call(k));
        } else {
            settle.push(codegen::Step::Ops(piece(k, &none)?));
        }
    }
    // The edge: the clocked bodies of the classes whose edge happened (with one class only at its
    // edge, so behind no test), the commit; then the settle's functions.
    for (k, (_, b)) in sel.members.iter().enumerate() {
        let r = &rs[k];
        init_fn.push(codegen::Step::Ops(
            b.init_ops.iter().map(|op| r.op(op)).collect(),
        ));
        if b.kind != Kind::Clocked {
            continue;
        }
        if shared.contains_key(&k) {
            edge.push(call(k));
        } else {
            // A body with memory writes kept back has them in `deferred`.
            let ops = if deferred[k].1.is_empty() {
                &b.ops
            } else {
                &deferred[k].0
            };
            let body = shared_body(ops, true, several);
            let ops = body.iter().map(|op| r.op(op)).collect();
            edge.push(codegen::Step::Ops(ops));
        }
    }
    // The memory writes of this edge, in member and program order, then the next-state blocks.
    // A memory something outside reads keeps the flags of its writes for the export, which
    // drives only what they changed and clears them (`rt::RamWrite`).
    let exported = |k: u32| {
        let (a, z) = nets.packs[k as usize];
        (a.0..z.0).any(|l| sel.outputs.contains(&LeafId(l)))
    };
    let mut writes = Vec::new();
    let mut ram_writes: BTreeMap<u32, Vec<rt::RamWrite>> = BTreeMap::new();
    for (k, (_, pend)) in deferred.iter().enumerate() {
        let r = &rs[k];
        for p in pend {
            let Op::IndexWrite {
                dst: Ref::Sig(sig),
                idx: Ref::Loc(i),
                lo,
                n,
                stride,
                w,
                left,
                down,
                ..
            } = p.write
            else {
                return Err("cycle mode: a kept-back memory write of another shape".into());
            };
            let keep = exported(sig);
            if keep {
                ram_writes.entry(sig).or_default().push(rt::RamWrite {
                    flag: (r.base + p.flag) as usize,
                    idx: (r.base + i) as usize,
                    lo,
                    n,
                    stride,
                    w,
                    left,
                    down,
                });
            }
            let mut then = vec![p.write.clone()];
            if !keep {
                then.push(Op::Lit {
                    dst: Ref::Loc(p.flag),
                    val: Box::new([0]),
                });
            }
            writes.push(r.op(&Op::If {
                cond: Ref::Loc(p.flag),
                then,
                else_: Vec::new(),
            }));
        }
    }
    let mut c = commit(&next, &cur, klen);
    writes.append(&mut c);
    edge.push(codegen::Step::Ops(writes));
    let err = n;
    n += 2;
    // The save words of each loop group.
    for g in &mut groups {
        g.1 = n;
        n += g.0.iter().map(|w| w.1).sum::<u32>();
    }
    let mut keep = vec![false; n as usize];
    keep[..signal_words as usize].fill(true);
    for (w, c) in state {
        keep[w as usize..(w + c) as usize].fill(true);
    }
    let top = (e.hier.scopes.iter())
        .position(|s| s.parent.is_none())
        .ok_or("the design has no top-level instance")?;
    let label = e.hier.dotted(yavs_elab::ScopeId(top as u32));
    // Each shape on virtual words: its leader's blocks one after the other.
    let shapes: Vec<codegen::Shape> = (leaders.iter())
        .map(|(body, bl, r)| {
            let mut v = Vec::new();
            let mut at = 0;
            for &(_, _, n) in bl {
                v.push((at, n));
                at += n;
            }
            let place = |b: Block| {
                let j = (bl.iter())
                    .position(|x| x.0 == b)
                    .expect("every block the body addresses is listed");
                v[j].0
            };
            let src = |x: Ref| match x {
                Ref::Sig(k) => Ref::Loc(place(Block::Cur(k))),
                Ref::Loc(s) => Ref::Loc(place(Block::Locals) + s),
                Ref::Edge => Ref::Loc(place(Block::Edge)),
            };
            let dst = |x: Ref| match x {
                Ref::Sig(k) if r.clocked && next.contains_key(&k) => {
                    Ref::Loc(place(Block::Next(k)))
                }
                x => src(x),
            };
            codegen::Shape {
                ops: body.iter().map(|op| op.map(&src, &dst, &|x| x)).collect(),
                keep: (bl.iter())
                    .flat_map(|&(_, w, n)| keep[w as usize..(w + n) as usize].iter().copied())
                    .collect(),
                blocks: v,
            }
        })
        .collect();
    let code = Arc::new(codegen::compile(
        &settle, &edge, &init_fn, &bodies, &shapes, groups, &keep, err, &label,
    )?);
    let whole = |k: u32| {
        let s = yavs_kernel::SignalId(k);
        s.slice(0, nets.signal_len(s))
    };
    // A null array has no elements to read or drive.
    let all: Vec<_> = (cur.iter())
        .filter(|&(&k, _)| !logic(k) || nets.signal_len(yavs_kernel::SignalId(k)) > 0)
        .map(|(&k, &s)| rt::Read {
            r: whole(k),
            slot: s,
            lo: 0,
            len: nets.signal_len(yavs_kernel::SignalId(k)),
            bytes: bytes(k),
            now: true,
        })
        .collect();
    // Inputs leaf by leaf (a kernel signal may also hold leaves the region drives), a leaf of
    // its own signal in chunks of one word, grouped by kernel signal: each chunk is read when the
    // kernel counted a change of it.
    // The inputs whose change is settled and exported at once: the resets from outside and the
    // wakes with a combinational path to something outside can see (`Selection::at_once`).
    // `YAVS_CYCLE_EDGE_OUTPUTS=1`, an experiment: only the resets, so the outputs change at
    // clock edges only.
    let edge_outputs = std::env::var_os("YAVS_CYCLE_EDGE_OUTPUTS").is_some_and(|v| v == "1");
    let now: std::collections::BTreeSet<LeafId> = (sel.members.iter())
        .filter_map(|(_, b)| b.reset.map(|r| r.0))
        .chain(sel.at_once.keys().copied().filter(|_| !edge_outputs))
        .collect();
    let mut by_sig: BTreeMap<u32, Vec<(rt::Read, Range)>> = BTreeMap::new();
    for &l in sel.inputs.iter().filter(|&&l| nets.leaf(l).ty.len > 0) {
        let len = nets.leaf(l).ty.len;
        let at = nets.loc[l.index()];
        let (slot, b) = (cur[&at.signal.0], bytes(at.signal.0));
        let step = if at.base == 0 && b == 0 { 64 } else { len };
        let chunks = by_sig.entry(at.signal.0).or_default();
        for k in (0..len).step_by(step as usize) {
            let part = Range::new(k, step.min(len - k));
            let read = rt::Read {
                r: nets.kref(l, part),
                slot: slot + k / 64,
                lo: at.base,
                len: part.len,
                bytes: b,
                now: now.contains(&l),
            };
            chunks.push((read, nets.kpart(l, part).1));
        }
    }
    let (mut chunks, mut ins) = (Vec::new(), Vec::new());
    let import_check = std::env::var_os("YAVS_CYCLE_IMPORT_CHECK").is_some_and(|v| v == "1");
    // One image per group of input signals of about `IMAGE_CHUNKS` chunks: a wake asks each
    // group's write count, and compares the chunk counts only of a group that was written.
    let mut items: Vec<yavs_kernel::ImageItem> = Vec::new();
    let mut groups = by_sig.into_iter().peekable();
    while let Some((k, rs)) = groups.next() {
        for &(_, r) in &rs {
            items.push(yavs_kernel::ImageItem {
                sig: yavs_kernel::SignalId(k),
                k: r.offset,
                len: r.len,
                voff: 8 * items.len() as u32,
                vsize: 8,
                kind: yavs_kernel::ImageKind::Count,
            });
        }
        chunks.extend(rs.into_iter().map(|c| c.0));
        if items.len() >= IMAGE_CHUNKS || groups.peek().is_none() {
            let n = items.len() as u32;
            ins.push(rt::InSig {
                spec: yavs_kernel::ImageSpec {
                    size: 8 * n,
                    items: std::mem::take(&mut items).into(),
                },
                at: chunks.len() as u32 - n,
                n,
                id: None,
                seen: 0,
            });
        }
    }
    // Each class: its clock element, the edge test of `Ctx::edge_by`, its word.
    let clocks: Vec<(yavs_kernel::SigRef, u8, usize)> = (sel.clocks.iter().zip(&edge_words))
        .map(|(c, &w)| {
            let kind = u8::from(c.rising) | if c.legacy { EDGE_LEVEL } else { 0 };
            (nets.kref(c.leaf, Range::new(c.elem, 1)), kind, w as usize)
        })
        .collect();
    let leaf_ref = |l: LeafId| nets.kref(l, Range::new(0, nets.leaf(l).ty.len));
    let mut waits: Vec<yavs_kernel::SigRef> = Vec::new();
    for c in &clocks {
        if !waits.contains(&c.0) {
            waits.push(c.0);
        }
    }
    waits.extend(
        (sel.wakes.iter())
            .filter(|&&l| nets.leaf(l).ty.len > 0)
            .map(|l| leaf_ref(*l)),
    );
    // The asynchronous resets from outside: their events wake the region too (one made inside
    // changes only in the region's own settle).
    let resets: std::collections::BTreeSet<(LeafId, u32, u8)> = (sel.members.iter())
        .filter_map(|(_, b)| b.reset)
        .filter(|r| !sel.driven.contains(&r.0))
        .collect();
    for &(l, el, _) in &resets {
        waits.push(nets.kref(l, Range::new(el, 1)));
    }
    // Only clocks wake it: a wake without an edge has nothing to do.
    let clocks_only = sel.wakes.is_empty() && resets.is_empty();
    let one_reset = match resets.iter().collect::<Vec<_>>()[..] {
        [] => Some(None),
        [&(l, el, code)] => {
            let (s, e) = nets.kelem(l, el);
            Some(Some((s, e, code)))
        }
        _ => None,
    };
    let trigger = match &sel.clocks[..] {
        [c] if sel.wakes.is_empty() => one_reset.map(|reset| {
            let (signal, elem) = nets.kelem(c.leaf, c.elem);
            EdgeTrigger {
                signal,
                elem,
                rising: c.rising,
                legacy: c.legacy,
                reset,
            }
        }),
        _ => None,
    };
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
    let sig_len: BTreeMap<u32, u32> = (cur.keys())
        .map(|&k| (k, nets.signal_len(yavs_kernel::SignalId(k))))
        .collect();
    let words = n as usize;
    let make: yavs_elab::RegionMake = Arc::new(move |ds: &[RegionDriver]| {
        // One export per kernel driver with an exported leaf: its range of the signal's words.
        let mut outs: BTreeMap<u32, rt::Out> = BTreeMap::new();
        for d in (ds.iter()).filter(|d| outputs.contains(&decls[d.decl]) && d.range.len > 0) {
            let Some(&slot) = cur.get(&d.signal.0) else {
                return Err(SimError::new(yavs_kernel::ErrorKind::Internal(
                    "cycle mode: an exported signal without words".into(),
                )));
            };
            // Integers: the image keeps the signal's elements from the right.
            let slot = match sig_bytes[&d.signal.0] {
                0 => slot,
                _ => slot + sig_len[&d.signal.0] - d.range.offset - d.range.len,
            };
            outs.entry(d.driver.0).or_insert(rt::Out {
                driver: d.driver,
                range: d.range,
                slot,
                bytes: sig_bytes[&d.signal.0],
                last: 0,
                wide: Vec::new(),
                ram: ram_writes.get(&d.signal.0).cloned().unwrap_or_default(),
            });
        }
        let mut outs: Vec<rt::Out> = outs.into_values().collect();
        let (spans, shadow) = rt::spans(&mut outs);
        let mut img = vec![0u64; words];
        for &(w, v) in &init {
            img[w as usize] = v;
        }
        Ok(Box::new(rt::RegionProc {
            code: code.clone(),
            img,
            leaves: all.clone(),
            seen: vec![0; chunks.len()],
            chunks: chunks.clone(),
            ins: ins.clone(),
            looked: false,
            check: import_check,
            exported: false,
            pending: false,
            outs,
            spans,
            shadow: vec![0; shadow],
            clocks: clocks.clone(),
            clocks_only,
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

/// The input chunks one kernel image counts the changes of (`rt::InSig`).
const IMAGE_CHUNKS: usize = 32;

/// The next-state words into the current ones.
/// A memory write of a clocked member kept back to the commit: the write, from the member's
/// words that hold its index and value, and the word that says it happened (member slots).
struct Pending {
    write: Op,
    flag: u32,
}

/// `ops` with every write of a memory in `rams` replaced by saving its index and value in new
/// member slots from `at` on and setting its flag; the writes, for the commit.
fn defer(
    ops: &[Op],
    rams: &std::collections::BTreeSet<u32>,
    at: &mut u32,
    out: &mut Vec<Pending>,
) -> Vec<Op> {
    let mut res = Vec::with_capacity(ops.len());
    for op in ops {
        match op {
            Op::If { cond, then, else_ } => res.push(Op::If {
                cond: *cond,
                then: defer(then, rams, at, out),
                else_: defer(else_, rams, at, out),
            }),
            Op::IndexWrite {
                dst: Ref::Sig(k),
                src,
                lo,
                n,
                stride,
                w,
                idx,
                left,
                down,
                site,
            } if rams.contains(k) => {
                let (i, v, f) = (*at, *at + 1, *at + 1 + codegen::words(*w));
                *at = f + 1;
                res.extend([
                    Op::Copy {
                        dst: Ref::Loc(i),
                        src: *idx,
                        len: 64,
                    },
                    Op::Copy {
                        dst: Ref::Loc(v),
                        src: *src,
                        len: *w,
                    },
                    Op::Lit {
                        dst: Ref::Loc(f),
                        val: Box::new([1]),
                    },
                ]);
                out.push(Pending {
                    write: Op::IndexWrite {
                        dst: Ref::Sig(*k),
                        src: Ref::Loc(v),
                        lo: *lo,
                        n: *n,
                        stride: *stride,
                        w: *w,
                        idx: Ref::Loc(i),
                        left: *left,
                        down: *down,
                        site: *site,
                    },
                    flag: f,
                });
            }
            op => res.push(op.clone()),
        }
    }
    res
}

fn commit(
    next: &BTreeMap<u32, u32>,
    cur: &BTreeMap<u32, u32>,
    klen: impl Fn(u32) -> u32,
) -> Vec<Op> {
    (next.iter())
        .map(|(&k, &s)| Op::Copy {
            dst: Ref::Loc(cur[&k]),
            src: Ref::Loc(s),
            len: klen(k),
        })
        .collect()
}

/// Developers (`YAVS_CYCLE_STATS=1`): members whose code would be the same but for where their
/// signals and slots lie (everything folded into the code the same: constants, widths, offsets
/// within signals), as distinct shapes; the operations one body per shape would compile, also
/// counting only shapes of at least some size; the largest shapes by operations times instances.
fn shapes(db: &Db, e: &Elaborated, sel: &region::Selection, next: &BTreeMap<u32, u32>) {
    let mut by: std::collections::HashMap<ShapeKey, (usize, usize, usize)> = Default::default();
    for (pi, b) in &sel.members {
        let key = shape_key(b, next);
        let w: usize = b.ops.iter().map(codegen::weight_of).sum();
        let e = by.entry(key).or_insert((0, w, *pi));
        e.0 += 1;
    }
    let mut v: Vec<_> = by.values().collect();
    v.sort_by_key(|&&(n, w, pi)| (std::cmp::Reverse(n * w), pi));
    let all: usize = v.iter().map(|&&(n, w, _)| n * w).sum();
    let once: usize = v.iter().map(|&&(_, w, _)| w).sum();
    eprintln!(
        "cycle stats: {} members, {} shapes, {all} operations, {once} with one body per shape",
        sel.members.len(),
        v.len()
    );
    for t in [16, 64, 256, 1024] {
        let saved: usize = (v.iter())
            .filter(|&&&(_, w, _)| w >= t)
            .map(|&&(n, w, _)| (n - 1) * w)
            .sum();
        eprintln!("cycle stats:   shapes of {t} operations or more: {saved} operations saved");
    }
    for &&(n, w, pi) in v.iter().take(8) {
        eprintln!(
            "cycle stats:   {n} x {w} operations, {}",
            region::where_(db, e, pi)
        );
    }
}

/// Members of at least this many operations whose code is the same but for where their words lie
/// share one compiled body; below it a call per member would cost more than the code it saves.
const SHARE_MIN: usize = 256;

/// A place a member's code addresses, which each member of a shape has at an address of its own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Block {
    /// The current words of kernel signal `k`, its next-state words, the member's slots, its
    /// edge word.
    Cur(u32),
    Next(u32),
    Locals,
    Edge,
}

/// Blocks with their first word and length.
type Blocks = Vec<(Block, u32, u32)>;

fn same_kind(a: Block, b: Block) -> bool {
    std::mem::discriminant(&a) == std::mem::discriminant(&b)
}

/// The body a shared member runs: a combinational member's operations; a clocked member's, behind
/// its class's edge word when there are several classes.
fn shared_body(ops: &[Op], clocked: bool, several: bool) -> Vec<Op> {
    if clocked && several {
        vec![Op::If {
            cond: Ref::Edge,
            then: ops.to_vec(),
            else_: Vec::new(),
        }]
    } else {
        ops.to_vec()
    }
}

/// The blocks `ops` (of member `b`, resolved by `r`) address, in order of first use: each with its
/// first word in the image and its length.
fn blocks(ops: &[Op], b: &lower::Body, r: &Resolve, klen: impl Fn(u32) -> u32) -> Blocks {
    let seen = std::cell::RefCell::new(Vec::<Block>::new());
    let add = |x: Block| {
        let mut s = seen.borrow_mut();
        if !s.contains(&x) {
            s.push(x);
        }
    };
    let src = |x: Ref| {
        add(match x {
            Ref::Sig(k) => Block::Cur(k),
            Ref::Loc(_) => Block::Locals,
            Ref::Edge => Block::Edge,
        });
        x
    };
    let dst = |x: Ref| {
        match x {
            Ref::Sig(k) if r.clocked && r.next.contains_key(&k) => add(Block::Next(k)),
            x => {
                src(x);
            }
        }
        x
    };
    for op in ops {
        op.map(&src, &dst, &|x| x);
    }
    (seen.into_inner().into_iter())
        .map(|x| match x {
            Block::Cur(k) => (x, r.cur[&k], codegen::words(klen(k))),
            Block::Next(k) => (x, r.next[&k], codegen::words(klen(k))),
            Block::Locals => (x, r.base, b.locs.max(1)),
            Block::Edge => (x, r.edge, 1),
        })
        .collect()
}

/// What makes the code of member `b` what it is, but for where its words lie: kind, slots, state
/// variables, the operations with each kernel signal by its order of first use, and which of
/// those have next-state words.
fn shape_key(b: &lower::Body, next: &BTreeMap<u32, u32>) -> ShapeKey {
    let order = std::cell::RefCell::new(Vec::<u32>::new());
    let ord = |r: Ref| match r {
        Ref::Sig(k) => {
            let mut o = order.borrow_mut();
            let i = o.iter().position(|&x| x == k).unwrap_or_else(|| {
                o.push(k);
                o.len() - 1
            });
            Ref::Sig(i as u32)
        }
        r => r,
    };
    let ops: Vec<Op> = (b.ops.iter().chain(&b.init_ops))
        .map(|op| op.map(&ord, &ord, &|x| x))
        .collect();
    let sigs: Vec<bool> = (order.borrow().iter())
        .map(|k| next.contains_key(k))
        .collect();
    (b.kind, b.locs, b.vars.clone(), ops, sigs)
}

type ShapeKey = (Kind, u32, Vec<(u32, u32)>, Vec<Op>, Vec<bool>);

/// The operations of one member with every reference an image slot: a member's slots from
/// `base`, a signal its current words, except the next-state words where a clocked member writes
/// it; the edge word; check sites from `sites`.
#[derive(Clone, Copy)]
struct Resolve<'a> {
    base: u32,
    sites: u32,
    clocked: bool,
    edge: u32,
    cur: &'a BTreeMap<u32, u32>,
    next: &'a BTreeMap<u32, u32>,
    /// Signals read from a copy made at the piece's start (a loop group's piece reading what it
    /// drives), and the copy's words.
    snap: &'a BTreeMap<u32, u32>,
}

impl Resolve<'_> {
    fn src(&self, r: Ref) -> Ref {
        match r {
            Ref::Sig(k) => Ref::Loc(*self.snap.get(&k).unwrap_or(&self.cur[&k])),
            r => self.dst(r),
        }
    }

    fn dst(&self, r: Ref) -> Ref {
        match r {
            Ref::Sig(k) if self.clocked && self.next.contains_key(&k) => Ref::Loc(self.next[&k]),
            Ref::Sig(k) => Ref::Loc(self.cur[&k]),
            Ref::Loc(s) => Ref::Loc(self.base + s),
            Ref::Edge => Ref::Loc(self.edge),
        }
    }

    fn op(&self, op: &Op) -> Op {
        op.map(&|r| self.src(r), &|r| self.dst(r), &|x| self.sites + x)
    }
}
