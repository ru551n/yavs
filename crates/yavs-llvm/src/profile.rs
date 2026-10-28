//! The run-to-run profile of `--tier=auto` (#754, docs/decisions.md "`--tier=auto` run-to-run
//! profile"): what a run found hot and what it lost, kept next to the compiled code (one small text
//! entry per program and command line in the compiled-code store of the generator of the first level, `key`), so the next run
//! of the same program compiles its hot functions when it first sees them and leaves the losers
//! alone. The text is the whole interface: a damaged or foreign one is no profile.

/// A function (a process, not its copies) is hot when the run saw it for this part of the sampled
/// time, and for at least `HOT_MIN_US` microseconds of work.
pub(crate) const HOT_DIV: u64 = 32;
pub(crate) const HOT_MIN_US: u64 = 20_000;

/// Only a process of a kind that is expected to run at least this many times faster compiled
/// (`Model::speedup`) is compiled early: the other kinds (large combinational processes, clocked
/// ones of few instances) were measured to gain nothing or lose, and a hot one of them is a
/// loser the run after cannot afford to find out.
pub(crate) const EARLY_SPEEDUP: f64 = 1.4;

/// The runs among which a program that ran before shares the price of a compile (`Worker::repays`):
/// the compile is paid once and the cache keeps it for the runs after. (A hot function of the profile
/// is compiled early once its seen time summed over the runs repays its compile, #679.)
pub(crate) const EARLY_RUNS: u64 = 4;

/// A run is judged, and written to the profile, only when the simulation threads used this much CPU
/// time (microseconds): host noise is a few percent of that.
pub(crate) const JUDGE_MIN_US: u64 = 100_000;

/// The names kept: the hottest functions, and the most recent losses.
pub(crate) const HOT_MAX: usize = 48;
pub(crate) const LOST_MAX: usize = 256;

/// Functions compiled early stay on the list when the run that compiled them took this many percent
/// less simulation CPU time than the run before (`better`). Less is no gain worth the compile and
/// the link: they are lost.
pub(crate) const GAIN_PERCENT: u64 = 3;

const HEAD: &str = "yavs-auto-profile 3";

/// What the earlier runs of a program left.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Profile {
    /// Simulation CPU microseconds of the latest judged run.
    pub base: Option<u64>,
    /// Functions to compile when first seen: id, microseconds the runs saw it (summed over the runs
    /// since it became hot, #679), and whether a run that compiled it early was judged better than
    /// the run before. The hottest first.
    pub hot: Vec<(u32, u64, bool)>,
    /// Functions not compiled early: a trial took their compiled code back, or the run that
    /// compiled them early was not better. The oldest first.
    pub lost: Vec<u32>,
}

impl Profile {
    /// The text of the profile.
    pub fn render(&self) -> String {
        let mut s = format!("{HEAD}\n");
        if let Some(cpu) = self.base {
            s += &format!("base {cpu}\n");
        }
        for (id, us, ok) in &self.hot {
            s += &format!("hot {id} {us} {}\n", if *ok { 'a' } else { 'n' });
        }
        for id in &self.lost {
            s += &format!("lost {id}\n");
        }
        s
    }

    /// Rename every function with `f`; one it maps to `None` is dropped.
    pub fn remap(&mut self, f: impl Fn(u32) -> Option<u32>) {
        self.hot = self
            .hot
            .iter()
            .filter_map(|&(id, us, ok)| Some((f(id)?, us, ok)))
            .collect();
        self.lost = self.lost.iter().filter_map(|&id| f(id)).collect();
    }

    /// The profile in `text`, `None` for anything this version did not write.
    pub fn parse(text: &str) -> Option<Profile> {
        let mut lines = text.lines();
        if lines.next()? != HEAD {
            return None;
        }
        let mut p = Profile::default();
        for l in lines {
            let mut w = l.split(' ');
            let num = |w: &mut std::str::Split<'_, char>| w.next()?.parse::<u64>().ok();
            match w.next()? {
                "base" => p.base = Some(num(&mut w)?),
                "hot" => {
                    let (id, us) = (u32::try_from(num(&mut w)?).ok()?, num(&mut w)?);
                    let ok = match w.next()? {
                        "a" => true,
                        "n" => false,
                        _ => return None,
                    };
                    p.hot.push((id, us, ok));
                }
                "lost" => p.lost.push(u32::try_from(num(&mut w)?).ok()?),
                _ => return None,
            }
            if w.next().is_some() {
                return None;
            }
        }
        Some(p)
    }
}

/// What a run has found so far: the compile thread publishes it as it goes, and the end of the run
/// (`AutoTier::stop`) writes the profile from it, even when the thread is in the middle of a compile.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Snapshot {
    /// The microseconds of work sampled.
    pub total: u64,
    /// The functions (processes, not copies) seen for `HOT_MIN_US` or more, with the microseconds
    /// they and their copies were seen, interpreted or compiled.
    pub seen: Vec<(u32, u64)>,
    /// The functions a trial took back.
    pub lost: Vec<u32>,
    /// The functions compiled at once because of the profile.
    pub early: Vec<u32>,
}

/// The profile after a run that took `cpu` simulation CPU microseconds, from the one before it. The
/// functions compiled early for the first time are judged by the run as a whole: it took clearly
/// less CPU time than the run before (`better`), or they are lost. The functions a trial took back
/// are lost. The functions seen for a good part of the sampled time join the hot ones; the ones
/// already there stay and add this run's seen time to theirs (#679).
pub(crate) fn next(earlier: &Profile, snap: &Snapshot, cpu: u64) -> Profile {
    let mut p = earlier.clone();
    let lose = |p: &mut Profile, b: u32| {
        p.hot.retain(|h| h.0 != b);
        if !p.lost.contains(&b) {
            p.lost.push(b);
        }
    };
    for &b in &snap.lost {
        lose(&mut p, b);
    }
    let mut tried: Vec<u32> = snap
        .early
        .iter()
        .copied()
        .filter(|b| p.hot.iter().any(|h| h.0 == *b && !h.2))
        .collect();
    tried.sort_unstable();
    tried.dedup();
    match (tried.is_empty(), earlier.base) {
        (true, _) => p.base = Some(cpu),
        (false, Some(base)) if better(base, cpu) => {
            p.hot
                .iter_mut()
                .filter(|h| tried.contains(&h.0))
                .for_each(|h| h.2 = true);
            p.base = Some(cpu);
        }
        (false, _) => tried.iter().for_each(|&b| lose(&mut p, b)),
    }
    let floor = HOT_MIN_US.max(snap.total / HOT_DIV);
    for &(b, us) in &snap.seen {
        if let Some(h) = p.hot.iter_mut().find(|h| h.0 == b) {
            // The time of the runs adds up until it repays the compile (`Worker::load_profile`); a
            // function compiled early has passed that already.
            h.1 = h.1.saturating_add(us);
        } else if us >= floor && !p.lost.contains(&b) {
            p.hot.push((b, us, false));
        }
    }
    p.hot.sort_by_key(|h| (std::cmp::Reverse(h.1), h.0));
    p.hot.truncate(HOT_MAX);
    p.lost.drain(..p.lost.len().saturating_sub(LOST_MAX));
    p
}

/// Whether a run of `now` simulation CPU microseconds took clearly less than `base`.
pub(crate) fn better(base: u64, now: u64) -> bool {
    u128::from(now) * 100 <= u128::from(base) * u128::from(100 - GAIN_PERCENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_round_trips_and_foreign_text_is_no_profile() {
        let p = Profile {
            base: Some(1_500_000),
            hot: vec![(12, 900_000, true), (7, 40_000, false)],
            lost: vec![3, 9],
        };
        assert_eq!(Profile::parse(&p.render()), Some(p.clone()));
        assert_eq!(
            Profile::parse(&Profile::default().render()),
            Some(Profile::default())
        );
        for bad in [
            "",
            "yavs-auto-profile 2\n",
            "yavs-auto-profile 3\nhot 1 2\n",
            "yavs-auto-profile 3\nhot 1 2 x\n",
            "yavs-auto-profile 3\nhot 1 2 a 3\n",
            "yavs-auto-profile 3\nhot x 2 a\n",
            "yavs-auto-profile 3\nwarm 1 2\n",
        ] {
            assert_eq!(Profile::parse(bad), None, "{bad:?}");
        }
        // A cut-off entry (a half-written file) is no profile either.
        let text = p.render();
        assert_eq!(Profile::parse(&text[..text.len() - 3]), None);
    }

    fn snap(seen: &[(u32, u64)], early: &[u32], lost: &[u32]) -> Snapshot {
        Snapshot {
            total: 1_000_000,
            seen: seen.to_vec(),
            lost: lost.to_vec(),
            early: early.to_vec(),
        }
    }

    /// The first run only lists what was hot; the next compiles it early and is judged against the
    /// run before: better keeps the functions, anything else loses them, once.
    #[test]
    fn functions_compiled_early_are_judged_by_the_run_after_them() {
        let one = next(
            &Profile::default(),
            &snap(&[(5, 400_000), (6, 90_000), (7, 10_000)], &[], &[]),
            2_000_000,
        );
        // 7 is under 20 ms and under a thirty-second of the run.
        assert_eq!(one.hot, [(5, 400_000, false), (6, 90_000, false)]);
        assert_eq!(one.base, Some(2_000_000));
        // A run that compiled 5 and 6 early and took 30 percent less keeps them.
        let kept = next(
            &one,
            &snap(&[(5, 300_000), (6, 80_000)], &[5, 6], &[]),
            1_400_000,
        );
        assert_eq!(kept.hot, [(5, 700_000, true), (6, 170_000, true)]);
        assert_eq!(kept.base, Some(1_400_000));
        assert!(kept.lost.is_empty());
        // One that took as long loses them: out of the hot ones and on the list of losers.
        let lost = next(
            &one,
            &snap(&[(5, 400_000), (6, 90_000)], &[5, 6], &[]),
            2_050_000,
        );
        assert!(lost.hot.is_empty());
        assert_eq!(lost.lost, [5, 6]);
        assert_eq!(
            lost.base,
            Some(2_000_000),
            "the run before stays the one to beat"
        );
        // Judged ones are not judged again; a new hot function is tried on top of them.
        let more = next(
            &kept,
            &snap(&[(5, 300_000), (6, 80_000), (8, 120_000)], &[5, 6], &[]),
            1_410_000,
        );
        assert_eq!(
            more.hot,
            [
                (5, 1_000_000, true),
                (6, 250_000, true),
                (8, 120_000, false)
            ]
        );
        assert_eq!(more.base, Some(1_410_000));
        // A trial that took a function back loses it whatever else happened.
        let trial = next(
            &kept,
            &snap(&[(5, 300_000), (6, 80_000)], &[5, 6], &[6]),
            1_300_000,
        );
        assert_eq!(trial.hot, [(5, 1_000_000, true)]);
        assert_eq!(trial.lost, [6]);
        // A run that compiled nothing early adds its seen time to the hot ones' (#679).
        let two = next(&one, &snap(&[(5, 380_000)], &[], &[]), 2_000_000);
        assert_eq!(two.hot, [(5, 780_000, false), (6, 90_000, false)]);
        // A lost function is not hot again.
        let again = next(&lost, &snap(&[(5, 400_000)], &[], &[]), 2_000_000);
        assert!(again.hot.is_empty());
    }

    #[test]
    fn a_run_is_better_by_its_cpu_time() {
        assert!(better(1_000_000, 900_000));
        assert!(better(1_000_000, 970_000));
        assert!(!better(1_000_000, 971_000), "less than the gain wanted");
        assert!(!better(1_000_000, 1_050_000));
    }
}

/// The kind of the run-to-run profile of a program in the store (`key`, #754).
const KIND: &str = "prof";

/// What the profile of a run is keyed by besides the program: the command line (the generics, the
/// stop time, the test case a runner names), set by the CLI.
static RUN_ID: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Name the run for its profile (`--tier=auto` keeps one per program and run). Without it there is
/// no profile.
pub fn set_run_id(id: Option<&str>) {
    *RUN_ID
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = id.map(str::to_string);
}

/// The key of the profile of `prog`: the build, the optimiser switches, the run's id and a digest
/// of the set of functions' identities (name, kind, parent's name and kind, size as lowered, and how
/// many equal ones come before it; the reserved slots of copies are left out, they are filled in as
/// the run goes), so the profile of a run is never read for another one but is found whatever order
/// elaboration lowered the functions in (#774). With it the ids in identity order
/// (`order[rank] = id`): the profile names a function by its rank, not by its id. `None` without a
/// run id.
// ponytail: functions equal in all of that (the same label and size in two architectures) swap
// places when their order changes; a profile applied to the wrong one only mis-steers the heat.
pub(crate) fn key(prog: &yavs_ir::Program) -> Option<(yavs_cache::Key, Vec<u32>)> {
    use std::hash::{Hash, Hasher};
    let id = RUN_ID
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()?;
    let mut count: std::collections::HashMap<u64, u32> = Default::default();
    let mut ident: Vec<(u64, u32)> = Vec::new();
    for (i, f) in prog.funcs.iter().filter(|(_, f)| !f.spare) {
        let parent = f.parent.and_then(|p| prog.funcs.get(p));
        let mut h = std::hash::DefaultHasher::new();
        (&f.name, f.kind, parent.map(|p| (&p.name, p.kind)), f.size).hash(&mut h);
        let same = count.entry(h.finish()).or_insert(0);
        *same += 1;
        same.hash(&mut h);
        ident.push((h.finish(), i.0));
    }
    ident.sort_unstable();
    let mut h = std::hash::DefaultHasher::new();
    ident.iter().for_each(|i| i.0.hash(&mut h));
    let mut k = yavs_cache::KeyBuilder::new(KIND);
    k.text(&yavs_ir::opt::fingerprint())
        .text(&id)
        .part(&h.finish().to_le_bytes());
    Some((k.finish(), ident.into_iter().map(|i| i.1).collect()))
}

/// The profile stored under `key`, if any.
pub(crate) fn load(store: &yavs_cache::Store, key: &yavs_cache::Key) -> Option<String> {
    String::from_utf8(store.get(KIND, key)?.to_vec()).ok()
}

/// Store the profile (atomically; a run that writes after another one replaces its profile).
pub(crate) fn save(store: &yavs_cache::Store, key: &yavs_cache::Key, text: &str) {
    store.put(KIND, key, text.as_bytes());
}

#[cfg(test)]
mod store_tests {
    use super::{key, load, save, set_run_id};

    /// The tests that set the run id (process-wide) take turns.
    static GLOBALS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// The profile of a run is kept per program and per run id (the command line), is read back
    /// whole, and exists only with an id.
    #[test]
    fn the_profile_is_kept_per_program_and_run() {
        let _g = GLOBALS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prog = {
            let mut pb = yavs_ir::ProgramBuilder::new();
            let f = pb.declare("p", yavs_ir::FuncKind::Process, None);
            let mut fb = pb.define(f);
            let body = fb.block();
            fb.stmt(
                yavs_ir::StmtKind::Wait,
                yavs_diag::Span::new(yavs_diag::FileId(0), 0, 1),
            );
            fb.emit(yavs_ir::Inst::Br { to: body });
            fb.at(body);
            fb.emit(yavs_ir::Inst::Suspend { resume: 0 });
            fb.emit(yavs_ir::Inst::Br { to: body });
            fb.finish();
            pb.finish().unwrap()
        };
        let dir = tempfile::tempdir().unwrap();
        let store = yavs_cache::Store::objects_of_library(dir.path());
        set_run_id(None);
        assert!(key(&prog).is_none(), "no run id");
        set_run_id(Some("a"));
        let a = key(&prog).expect("a key").0;
        set_run_id(Some("b"));
        let b = key(&prog).expect("a key").0;
        assert_ne!(a, b);
        assert_eq!(load(&store, &a), None);
        save(&store, &a, "yavs-auto-profile 2\nbase 5\n");
        assert_eq!(
            load(&store, &a).as_deref(),
            Some("yavs-auto-profile 2\nbase 5\n")
        );
        assert_eq!(load(&store, &b), None);
        set_run_id(None);
    }

    /// The profile key and the ranks do not depend on the order functions were lowered in (#774):
    /// the same functions in another order give the same key and the same function at each rank.
    #[test]
    fn the_profile_key_ignores_the_order_of_functions() {
        let _g = GLOBALS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        use yavs_ir::{FuncKind, Inst, ProgramBuilder, StmtKind};
        let build = |order: &[(&str, usize)]| {
            let mut pb = ProgramBuilder::new();
            let ids: Vec<_> = order
                .iter()
                .map(|&(n, _)| pb.declare(n, FuncKind::Procedure, None))
                .collect();
            for (&id, &(_, len)) in ids.iter().zip(order) {
                let mut fb = pb.define(id);
                let spin = fb.block();
                fb.stmt(
                    StmtKind::Return,
                    yavs_diag::Span::new(yavs_diag::FileId(0), 0, 1),
                );
                let ptr = fb.reg(yavs_ir::RegTy::Ptr);
                fb.emit(Inst::AddrOf {
                    dst: ptr,
                    space: yavs_ir::AddrSpace::Frame,
                    off: 0,
                });
                for _ in 0..len {
                    fb.emit(Inst::Free { ptr });
                }
                fb.emit(Inst::Br { to: spin });
                fb.at(spin);
                fb.emit(Inst::Br { to: spin });
                fb.finish();
            }
            pb.finish().unwrap()
        };
        let names = |p: &yavs_ir::Program, order: &[u32]| -> Vec<String> {
            order
                .iter()
                .map(|&i| p.funcs.get(yavs_ir::FuncId(i)).unwrap().name.clone())
                .collect()
        };
        set_run_id(Some("a"));
        let one = build(&[("f", 3), ("g", 5), ("h", 3), ("h", 3)]);
        let two = build(&[("h", 3), ("g", 5), ("h", 3), ("f", 3)]);
        let other = build(&[("f", 3), ("g", 6), ("h", 3), ("h", 3)]);
        let (k1, o1) = key(&one).unwrap();
        let (k2, o2) = key(&two).unwrap();
        assert_eq!(k1, k2, "the same functions in another order");
        assert_ne!(o1, o2, "the ids do move");
        assert_eq!(names(&one, &o1), names(&two, &o2));
        assert_ne!(k1, key(&other).unwrap().0, "a function differs");
        set_run_id(None);
    }
}
