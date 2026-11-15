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
        .unwrap_or(0);
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
    // Signals hold state between calls; so do the variables of clocked processes.
    let signal_words = n;
    // One word per edge class: 1 when its edge happened in this wake (`Ref::Edge` of its
    // members).
    let edge_words: Vec<u32> = (0..sel.clocks.len() as u32).map(|k| n + k).collect();
    n += sel.clocks.len().max(1) as u32;
    let several = sel.clocks.len() > 1;
    let mut state: Vec<(u32, u32)> = Vec::new();
    // Each function as pieces, a member's body each (`codegen::compile` cuts between them).
    let mut settle: Vec<Vec<Op>> = Vec::new();
    let mut edge: Vec<Vec<Op>> = Vec::new();
    let mut init_fn: Vec<Vec<Op>> = Vec::new();
    edge.push(
        (next.iter())
            .map(|(&k, &s)| Op::Copy {
                dst: Ref::Loc(s),
                src: Ref::Loc(cur[&k]),
                len: klen(k),
            })
            .collect(),
    );
    let mut init: Vec<(u32, u64)> = Vec::new();
    let mut checks: Vec<(lower::Check, usize)> = Vec::new();
    let mut uninit = 0u64;
    // Each member's slots and resolution.
    let mut rs: Vec<Resolve> = Vec::new();
    for (pi, b) in &sel.members {
        let base = n;
        n += b.locs;
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
        });
    }
    // The settle: the combinational members and, while their reset holds, the reset branches
    // of clocked ones (written to the current words: the commit is past).
    for &k in &sel.settle {
        let (_, b) = &sel.members[k];
        let r = &rs[k];
        let ops: Vec<Op> = if b.kind == Kind::Clocked {
            let piece = region::reset_piece(&b.ops).ok_or("a reset branch of another shape")?;
            let r = Resolve {
                clocked: false,
                ..*r
            };
            piece.iter().map(|op| r.op(op)).collect()
        } else {
            b.ops.iter().map(|op| r.op(op)).collect()
        };
        settle.push(ops);
    }
    // The edge: the clocked bodies of the classes whose edge happened (with one class only at its
    // edge, so behind no test), the commit, the settle.
    for (k, (_, b)) in sel.members.iter().enumerate() {
        let r = &rs[k];
        init_fn.push(b.init_ops.iter().map(|op| r.op(op)).collect());
        if b.kind != Kind::Clocked {
            continue;
        }
        let mut ops: Vec<Op> = b.ops.iter().map(|op| r.op(op)).collect();
        if several {
            ops = vec![Op::If {
                cond: Ref::Loc(r.edge),
                then: ops,
                else_: Vec::new(),
            }];
        }
        edge.push(ops);
    }
    edge.push(commit(&next, &cur, klen));
    edge.extend(settle.iter().cloned());
    let err = n;
    n += 2;
    let mut keep = vec![false; n as usize];
    keep[..signal_words as usize].fill(true);
    for (w, c) in state {
        keep[w as usize..(w + c) as usize].fill(true);
    }
    let top = (e.hier.scopes.iter())
        .position(|s| s.parent.is_none())
        .ok_or("the design has no top-level instance")?;
    let label = e.hier.dotted(yavs_elab::ScopeId(top as u32));
    let code = Arc::new(codegen::compile(
        [&settle, &edge, &init_fn],
        &keep,
        err,
        &label,
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
        })
        .collect();
    // Inputs leaf by leaf (a kernel signal may also hold leaves the region drives), a leaf of
    // its own signal in chunks of one word, grouped by kernel signal: each chunk is read when the
    // kernel counted a change of it.
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
            };
            chunks.push((read, nets.kpart(l, part).1));
        }
    }
    let (mut chunks, mut ins) = (Vec::new(), Vec::new());
    let import_check = std::env::var_os("YAVS_CYCLE_IMPORT_CHECK").is_some_and(|v| v == "1");
    for (k, rs) in by_sig {
        let items = (rs.iter().enumerate())
            .map(|(i, &(_, r))| yavs_kernel::ImageItem {
                sig: yavs_kernel::SignalId(k),
                k: r.offset,
                len: r.len,
                voff: 8 * i as u32,
                vsize: 8,
                kind: yavs_kernel::ImageKind::Count,
            })
            .collect::<Vec<_>>();
        ins.push(rt::InSig {
            spec: yavs_kernel::ImageSpec {
                size: 8 * items.len() as u32,
                items: items.into(),
            },
            at: chunks.len() as u32,
            n: rs.len() as u32,
            id: None,
            seen: 0,
        });
        chunks.extend(rs.into_iter().map(|c| c.0));
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
            outs.entry(d.driver.0).or_insert(rt::Out {
                driver: d.driver,
                range: d.range,
                slot,
                bytes: sig_bytes[&d.signal.0],
                last: 0,
                wide: Vec::new(),
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
            seen: vec![0; chunks.len()],
            chunks: chunks.clone(),
            ins: ins.clone(),
            looked: false,
            check: import_check,
            exported: false,
            outs: outs.into_values().collect(),
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

/// The next-state words into the current ones.
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
}

impl Resolve<'_> {
    fn src(&self, r: Ref) -> Ref {
        match r {
            Ref::Sig(k) => Ref::Loc(self.cur[&k]),
            Ref::Loc(s) => Ref::Loc(self.base + s),
            Ref::Edge => Ref::Loc(self.edge),
        }
    }

    fn dst(&self, r: Ref) -> Ref {
        match r {
            Ref::Sig(k) if self.clocked && self.next.contains_key(&k) => Ref::Loc(self.next[&k]),
            r => self.src(r),
        }
    }

    fn op(&self, op: &Op) -> Op {
        op.map(&|r| self.src(r), &|r| self.dst(r), &|x| self.sites + x)
    }
}
