//! `yavs run --profile[=FILE]`: a sampling profiler. A thread wakes `HZ` times a second and
//! tallies which process each simulation thread runs (`yavs_kernel::profile`, one relaxed
//! store per activation) and which subprogram the interpreter or compiled code is in
//! (`Runtime::profile_sampler`). The table goes to stderr after the run, the optional file is JSON.
//! Off, nothing is wrapped and no thread exists.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use yavs_diag::SourceMap;
use yavs_interp::Runtime;
use yavs_ir::FuncId;
use yavs_kernel::profile::Profile;

/// Samples per second. Fixed: 1 kHz costs well under 1 percent and a second of run gives a
/// usable table.
const HZ: u64 = 1000;
/// Rows of each table on stderr (the file has all).
const TOP: usize = 20;

/// A tier with no compiled code: its presence makes the interpreter name the subprogram it
/// runs in its sampling slot (every 128 loop iterations or calls).
struct Watch;

impl yavs_interp::Tier for Watch {
    fn run(&self, _: FuncId, _: u32, _: &mut dyn yavs_interp::TierHost) -> Option<u32> {
        None
    }
    fn ready(&self, _: FuncId) -> bool {
        false
    }
}

/// Lets the interpreter publish the subprogram it is in, for a run with `--tier=interp`.
pub fn watch_interpreter(rt: &Runtime) {
    rt.set_tier(Arc::new(Watch));
}

#[derive(Default)]
struct Tally {
    ticks: u64,
    /// Ticks in which no thread ran a process.
    kernel: u64,
    procs: HashMap<u32, u64>,
    funcs: HashMap<(u32, bool), u64>,
    /// Ticks that saw exactly one process and one subprogram (all of them at `--threads 1`).
    pairs: HashMap<(u32, u32, bool), u64>,
    /// Interpreted (function, IR pc) as the interpreter last looked.
    lines: HashMap<(u32, u32), u64>,
}

pub struct Sampler {
    stop: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<Tally>,
    start: Instant,
    prof: Arc<Profile>,
}

impl Sampler {
    pub fn start(
        prof: Arc<Profile>,
        funcs: Option<impl Fn(&mut Vec<(FuncId, bool)>) + Send + 'static>,
        lines: Option<impl Fn(&mut Vec<(FuncId, u32)>) + Send + 'static>,
    ) -> Sampler {
        let stop = Arc::new(AtomicBool::new(false));
        let (flag, p) = (stop.clone(), prof.clone());
        let thread = std::thread::spawn(move || {
            let mut t = Tally::default();
            let (mut ps, mut fs, mut ls) = (Vec::new(), Vec::new(), Vec::new());
            let period = Duration::from_nanos(1_000_000_000 / HZ);
            let mut next = Instant::now() + period;
            while !flag.load(Ordering::Relaxed) {
                std::thread::sleep(next.saturating_duration_since(Instant::now()));
                next = (next + period).max(Instant::now());
                ps.clear();
                fs.clear();
                ls.clear();
                p.sample(&mut ps);
                if let Some(f) = &funcs {
                    f(&mut fs);
                }
                if let Some(l) = &lines {
                    l(&mut ls);
                }
                t.ticks += 1;
                if ps.is_empty() {
                    t.kernel += 1;
                }
                for &x in &ps {
                    *t.procs.entry(x).or_default() += 1;
                }
                for &(f, c) in &fs {
                    *t.funcs.entry((f.0, c)).or_default() += 1;
                }
                for &(f, pc) in &ls {
                    *t.lines.entry((f.0, pc)).or_default() += 1;
                }
                if let ([x], [(f, c)]) = (&ps[..], &fs[..]) {
                    *t.pairs.entry((*x, f.0, *c)).or_default() += 1;
                }
            }
            t
        });
        Sampler {
            stop,
            thread,
            start: Instant::now(),
            prof,
        }
    }

    /// Stops sampling and prints the tables to stderr; with `file`, writes them as JSON.
    pub fn finish(
        self,
        rt: Option<&Runtime>,
        sources: &SourceMap,
        file: Option<&str>,
    ) -> std::io::Result<()> {
        self.stop.store(true, Ordering::Relaxed);
        let secs = self.start.elapsed().as_secs_f64();
        let Ok(t) = self.thread.join() else {
            return Err(std::io::Error::other("the sampler thread failed"));
        };
        let names = self.prof.processes();
        // Source place of a function: its earliest statement in the file of its first one.
        let place = |f: u32| -> Option<(String, usize)> {
            let prog = &rt?.prog;
            let func = prog.raw(FuncId(f))?;
            let spans: Vec<_> = func
                .loc
                .iter()
                .filter_map(|&s| prog.stmts.get(s).map(|i| i.span))
                .filter(|s| s.end > s.start)
                .collect();
            let file = spans.first()?.file;
            let at = spans
                .iter()
                .filter(|s| s.file == file)
                .map(|s| s.start)
                .min()?;
            let f = sources.get(file);
            Some((f.path.clone(), f.line_col(at).0))
        };
        // Source line and owning subprogram of an interpreted (function, IR pc): the optimised
        // code is what ran, and an inlined statement still names its own function.
        let line_of = |f: u32, pc: u32| -> Option<(String, usize, String)> {
            let prog = &rt?.prog;
            let stmt = *prog.func(FuncId(f))?.loc.get(pc as usize)?;
            let info = prog.stmts.get(stmt)?;
            (info.span.end > info.span.start).then_some(())?;
            let file = sources.get(info.span.file);
            let owner = prog
                .funcs
                .get(info.func)
                .map_or(String::new(), |o| o.name.clone());
            Some((file.path.clone(), file.line_col(info.span.start).0, owner))
        };
        let pct = |n: u64| 100.0 * n as f64 / t.ticks.max(1) as f64;
        let name_of = |f: u32| {
            rt.and_then(|r| r.prog.raw(FuncId(f)))
                .map_or_else(|| format!("#{f}"), |f| f.name.clone())
        };
        let at = |p: &Option<(String, usize)>| {
            p.as_ref()
                .map_or(String::new(), |(f, l)| format!("{f}:{l}"))
        };
        let mut procs: Vec<_> = t.procs.iter().map(|(&p, &n)| (p, n)).collect();
        procs.sort_by_key(|&(p, n)| (std::cmp::Reverse(n), p));
        let mut inst: HashMap<&str, u64> = HashMap::new();
        for &(p, n) in &procs {
            let name = names.get(p as usize).map_or("?", |x| x.0.as_str());
            *inst
                .entry(name.rsplit_once(':').map_or(name, |x| x.0))
                .or_default() += n;
        }
        let mut inst: Vec<_> = inst.into_iter().collect();
        inst.sort_by_key(|&(p, n)| (std::cmp::Reverse(n), p));
        let mut subs: HashMap<u32, [u64; 2]> = HashMap::new();
        for (&(f, c), &n) in &t.funcs {
            subs.entry(f).or_default()[usize::from(c)] += n;
        }
        let mut subs: Vec<_> = subs.into_iter().collect();
        subs.sort_by_key(|&(f, n)| (std::cmp::Reverse(n[0] + n[1]), f));

        let mut lines: HashMap<(String, usize, String), u64> = HashMap::new();
        for (&(f, pc), &n) in &t.lines {
            if let Some(k) = line_of(f, pc) {
                *lines.entry(k).or_default() += n;
            }
        }
        let mut lines: Vec<_> = lines.into_iter().collect();
        lines.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        eprintln!(
            "profile: {} samples at {HZ} Hz in {secs:.3} s; {:.1}% in processes, {:.1}% in the kernel (a sample counts every thread running a process)",
            t.ticks,
            100.0 - pct(t.kernel),
            pct(t.kernel)
        );
        eprintln!("profile: processes");
        eprintln!("profile:      %  samples  process (source)");
        for &(p, n) in procs.iter().take(TOP) {
            let name = names.get(p as usize);
            let src = at(&name.and_then(|x| x.1).and_then(place));
            eprintln!(
                "profile: {:6.1} {n:8}  {} ({src})",
                pct(n),
                name.map_or("?", |x| x.0.as_str())
            );
        }
        eprintln!("profile: instances (their own processes)");
        for &(i, n) in inst.iter().take(TOP) {
            eprintln!("profile: {:6.1} {n:8}  {i}", pct(n));
        }
        eprintln!("profile: subprograms (interpreted + compiled samples)");
        for &(f, n) in subs.iter().take(TOP) {
            eprintln!(
                "profile: {:6.1} {:8}  {} ({}) [{} interpreted, {} compiled]",
                pct(n[0] + n[1]),
                n[0] + n[1],
                name_of(f),
                at(&place(f)),
                n[0],
                n[1]
            );
        }
        eprintln!("profile: lines (interpreted code, where the interpreter last looked)");
        for ((f, l, o), n) in lines.iter().take(TOP) {
            eprintln!("profile: {:6.1} {n:8}  {f}:{l} ({o})", pct(*n));
        }
        let Some(path) = file else { return Ok(()) };
        let j = serde_json::json!({
            "hz": HZ,
            "seconds": secs,
            "samples": t.ticks,
            "kernel_samples": t.kernel,
            "processes": procs.iter().map(|&(p, n)| {
                let name = names.get(p as usize);
                let src = name.and_then(|x| x.1).and_then(place);
                let with: Vec<_> = t.pairs.iter().filter(|(k, _)| k.0 == p).map(|(k, &n)| {
                    serde_json::json!({"subprogram": name_of(k.1), "compiled": k.2, "samples": n})
                }).collect();
                serde_json::json!({
                    "path": name.map_or("?", |x| x.0.as_str()),
                    "file": src.as_ref().map(|s| s.0.as_str()),
                    "line": src.as_ref().map(|s| s.1),
                    "samples": n,
                    "subprograms": with,
                })
            }).collect::<Vec<_>>(),
            "instances": inst.iter().map(|&(i, n)| serde_json::json!({"path": i, "samples": n})).collect::<Vec<_>>(),
            "lines": lines.iter().map(|((f, l, o), n)| {
                serde_json::json!({"file": f, "line": l, "subprogram": o, "samples": n})
            }).collect::<Vec<_>>(),
            "subprograms": subs.iter().map(|&(f, n)| {
                let src = place(f);
                serde_json::json!({
                    "name": name_of(f),
                    "file": src.as_ref().map(|s| s.0.as_str()),
                    "line": src.as_ref().map(|s| s.1),
                    "interpreted_samples": n[0],
                    "compiled_samples": n[1],
                })
            }).collect::<Vec<_>>(),
        });
        let text = serde_json::to_string_pretty(&j).map_err(std::io::Error::other)?;
        std::fs::write(path, text + "\n")
    }
}
