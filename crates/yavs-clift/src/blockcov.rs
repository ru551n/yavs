//! `YAVS_CLIFT_BLOCKCOV=<dir>` (a measurement switch): generated code stores one byte into a
//! per-body table at the entry of every block it lowers, and the run writes which blocks ran
//! (`cov-<pid>.tsv`: body symbol, IR pc, IR block id or `-`, 0 or 1, `key`). The code differs from
//! the normal code, so byte sizes come from a run without the switch (`YAVS_CLIFT_CODEDIR`).
//! `YAVS_CLIFT_COLD=<cov tsv>` (a lab switch) marks the blocks that file lists as never run cold;
//! `YAVS_CLIFT_CHEAP=<cov tsv>|all|alt` (a lab switch) lowers them cheaply (`cheap`).
//!
//! `YAVS_CLIFT_LIST` (a lab switch, `--tier=auto`): the never-run blocks of each function kept in
//! the store per program, as routes are (`list_mode`). Written by a coverage run (`write`) or by
//! ordinary runs from the blocks the interpreter entered (`rec`).

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::{Mutex, OnceLock, PoisonError};

/// One block of a body: the pc it starts at and the IR block that starts there, if any (the
/// others are entries after a call).
pub type BlockInfo = (u32, Option<u32>);

struct Table {
    name: String,
    key: String,
    /// `body_hash` of the function (`YAVS_CLIFT_LIST=write`; 0 otherwise).
    hash: u64,
    /// The address of the body's table, one byte per block, in the module's data while it lives.
    at: usize,
    blocks: Vec<BlockInfo>,
    /// The bytes as the module left them, once it is dropped (`freeze`).
    frozen: Option<Vec<u8>>,
}

static TABLES: Mutex<Vec<Table>> = Mutex::new(Vec::new());

/// A body and its blocks, whether each ran.
pub struct BodyCov {
    pub name: String,
    pub key: String,
    pub blocks: Vec<(BlockInfo, bool)>,
}

/// List a finalized body's table.
pub(crate) fn register(name: String, key: String, hash: u64, at: usize, blocks: Vec<BlockInfo>) {
    TABLES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Table {
            name,
            key,
            hash,
            at,
            blocks,
            frozen: None,
        });
}

/// The module that holds the tables at `ats` goes away: keep their bytes.
pub(crate) fn freeze(ats: &[usize]) {
    let mut tables = TABLES.lock().unwrap_or_else(PoisonError::into_inner);
    for t in tables
        .iter_mut()
        .filter(|t| t.frozen.is_none() && ats.contains(&t.at))
    {
        // SAFETY: the module is still alive here (`Jit::drop`); `at` is its table of
        // `blocks.len()` bytes.
        let b = unsafe { std::slice::from_raw_parts(t.at as *const u8, t.blocks.len()) };
        t.frozen = Some(b.to_vec());
    }
}

/// The name of a body that is the same in every run of a program: `f` or `n` with the function
/// id, or for a copy of a process its `Program::copy_tag` (a copy's id follows the order copies
/// were made in).
pub(crate) fn key(prog: &yavs_ir::Program, id: yavs_ir::FuncId, native: bool) -> String {
    let kind = if native { "n" } else { "f" };
    match prog.copy_tag(id) {
        Some(t) => format!("{kind}c{t:x}"),
        None => format!("{kind}{}", id.0),
    }
}

/// The pcs of never-run blocks by body key.
type Lists = HashMap<String, HashSet<u32>>;

/// The never-run blocks of each body a coverage dump lists (`YAVS_CLIFT_COLD`, `YAVS_CLIFT_CHEAP`).
fn never_run(var: &str, path: &std::ffi::OsStr) -> Lists {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("error: {var}: cannot read {}: {e}", path.to_string_lossy());
        std::process::exit(2)
    });
    let mut out: HashMap<String, HashSet<u32>> = HashMap::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        let (Some(pc), Some(&"0"), Some(key)) = (f.get(1), f.get(3), f.get(4)) else {
            continue;
        };
        if let Ok(pc) = pc.parse() {
            out.entry(key.to_string()).or_default().insert(pc);
        }
    }
    out
}

/// `YAVS_CLIFT_COLD`: the pcs of the blocks of body `key` that file lists as never run.
pub(crate) fn cold(key: &str) -> Option<&'static HashSet<u32>> {
    static LIST: OnceLock<HashMap<String, HashSet<u32>>> = OnceLock::new();
    LIST.get_or_init(|| {
        std::env::var_os("YAVS_CLIFT_COLD")
            .map_or_else(HashMap::new, |p| never_run("YAVS_CLIFT_COLD", &p))
    })
    .get(key)
}

/// The blocks of a body that get the cheap lowering (`YAVS_CLIFT_CHEAP`, `YAVS_CLIFT_LIST`).
#[derive(Clone)]
pub(crate) enum Cheap {
    /// `all`: every block (a test mode: correctness must not rest on the list).
    All,
    /// `alt`: every second block, so cheap and full blocks meet (a test mode).
    Alt,
    /// A coverage dump's never-run blocks of this body.
    Set(std::rc::Rc<HashSet<u32>>),
}

impl Cheap {
    /// Whether the block that starts at `pc`, the `nth` the body makes, is cheap.
    pub(crate) fn has(&self, pc: u32, nth: usize) -> bool {
        match self {
            Cheap::All => true,
            Cheap::Alt => nth % 2 == 1,
            Cheap::Set(s) => s.contains(&pc),
        }
    }
}

/// `YAVS_CLIFT_CHEAP=<cov tsv>|all|alt` (a lab switch, CG3(a)): the blocks of body `key` that are
/// lowered as one call of the interpreter for their instructions (frame-mode code) or that leave
/// at their start (a native frame), instead of the full code.
pub(crate) fn cheap(key: &str) -> Option<Cheap> {
    // (`Ok(true)`: `all`, `Ok(false)`: `alt`.)
    static LIST: OnceLock<Option<Result<bool, Lists>>> = OnceLock::new();
    let l = LIST.get_or_init(|| {
        let p = std::env::var_os("YAVS_CLIFT_CHEAP")?;
        Some(match p.to_str() {
            Some("all") => Ok(true),
            Some("alt") => Ok(false),
            _ => Err(never_run("YAVS_CLIFT_CHEAP", &p)),
        })
    });
    match l.as_ref()? {
        Ok(true) => Some(Cheap::All),
        Ok(false) => Some(Cheap::Alt),
        Err(m) => m.get(key).map(|s| Cheap::Set(std::rc::Rc::new(s.clone()))),
    }
}

/// Every body made so far with the blocks that ran until now.
pub fn snapshot() -> Vec<BodyCov> {
    let tables = TABLES.lock().unwrap_or_else(PoisonError::into_inner);
    tables
        .iter()
        .map(|t| BodyCov {
            name: t.name.clone(),
            key: t.key.clone(),
            blocks: t
                .blocks
                .iter()
                .enumerate()
                .map(|(k, &b)| {
                    let ran = match &t.frozen {
                        Some(f) => f[k],
                        // SAFETY: the module of `at` is alive while the table is not frozen;
                        // compiled code writes it, a byte read at any time is fine.
                        None => unsafe { std::ptr::read_volatile((t.at + k) as *const u8) },
                    };
                    (b, ran != 0)
                })
                .collect(),
        })
        .collect()
}

/// At the end of a run of `rt`: store the never-run list (`YAVS_CLIFT_LIST`), and write
/// `cov-<pid>.tsv` into the directory of `YAVS_CLIFT_BLOCKCOV` when it is set.
pub fn write(rt: Option<&yavs_interp::Runtime>) {
    list_flush(rt);
    let Some(dir) = std::env::var_os("YAVS_CLIFT_BLOCKCOV") else {
        return;
    };
    let path = std::path::Path::new(&dir).join(format!("cov-{}.tsv", std::process::id()));
    let result = std::fs::create_dir_all(&dir).and_then(|()| {
        let mut out = std::io::BufWriter::new(std::fs::File::create(&path)?);
        for body in snapshot() {
            for ((pc, ir), ran) in body.blocks {
                let ir = ir.map_or("-".to_string(), |b| b.to_string());
                writeln!(
                    out,
                    "{}\t{pc}\t{ir}\t{}\t{}",
                    body.name,
                    u8::from(ran),
                    body.key
                )?;
            }
        }
        out.flush()
    });
    if let Err(e) = result {
        eprintln!(
            "error: YAVS_CLIFT_BLOCKCOV: cannot write {}: {e}",
            path.display()
        );
    }
}

/// What `YAVS_CLIFT_LIST` asks of a run of `--tier=auto`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ListMode {
    Off,
    /// `write`: a coverage run. Every block stores that it ran (as `YAVS_CLIFT_BLOCKCOV`), no
    /// compiled code, route or list of compiled functions is stored (none of it is the code of
    /// an ordinary run), and at the end the blocks that never ran replace the program's list.
    Write,
    /// `on`: the listed blocks of a function made now are lowered cheaply (frame-mode code; a
    /// native-frame variant lays them out cold, or with `YAVS_CLIFT_LIST_NATIVE=cheap` leaves at
    /// them), unless compiled code of the function in full is stored already. A function that
    /// steps too much of them is made again in full and leaves the list (`rt::cheap_ran`).
    Read,
    /// `all`: as `on` with every block listed (a test mode: the counter and the recompile run).
    All,
    /// `rec`: as `on`, and the interpreter notes the blocks it enters (`Runtime::record_blocks`):
    /// a function that stayed interpreted all run gets a line of the blocks it never entered (or
    /// its line keeps only blocks no run entered). One that got compiled code gets none: its code
    /// in full is stored, and what the interpreter saw of it is not all that ran.
    Rec,
}

/// `YAVS_CLIFT_LIST=on|write|all` (unset or `off`: none).
pub(crate) fn list_mode() -> ListMode {
    static MODE: OnceLock<ListMode> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("YAVS_CLIFT_LIST").as_deref() {
        Err(_) | Ok("" | "off") => ListMode::Off,
        Ok("on") => ListMode::Read,
        Ok("write") => ListMode::Write,
        Ok("all") => ListMode::All,
        Ok("rec") => ListMode::Rec,
        Ok(v) => {
            eprintln!(
                "error: YAVS_CLIFT_LIST: unknown value `{v}`; expected on, rec, write, all or off"
            );
            std::process::exit(2)
        }
    })
}

/// `YAVS_CLIFT_LIST=rec`: the runtime of `--tier=auto` notes the blocks the interpreter enters.
pub fn list_records() -> bool {
    list_mode() == ListMode::Rec
}

/// `YAVS_CLIFT_LIST_NATIVE=cheap`: native-frame variants leave at their listed blocks too, instead
/// of only laying them out cold.
pub(crate) fn list_native_cheap() -> bool {
    std::env::var("YAVS_CLIFT_LIST_NATIVE").is_ok_and(|v| v == "cheap")
}

/// Bodies made with a list in this run, and frame-mode code made again in full (`YAVS_COUNTERS`:
/// `clift_listed`, `clift_again`).
pub(crate) static LISTED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub(crate) static AGAIN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Lines `rec` wrote or narrowed from the interpreter's notes at the end of the run.
static RECORDED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// `YAVS_COUNTERS`: bodies made with a never-run list, functions whose listed blocks ran too much,
/// of them the ones made again in full, and lines written from the interpreter's notes
/// (`clift_recorded`, after `write`).
pub fn list_stats() -> (usize, usize, usize, usize) {
    (
        LISTED.load(std::sync::atomic::Ordering::Relaxed),
        yavs_llvm::rt::tripped_all().len(),
        AGAIN.load(std::sync::atomic::Ordering::Relaxed),
        RECORDED.load(std::sync::atomic::Ordering::Relaxed),
    )
}

/// The kind of the list's entry in the store.
const LIST: &str = "never";

/// The name of function `id` in the list, the same in every run of the program: its id, or `c`
/// and the `Program::copy_tag` of a copy.
pub(crate) fn func_key(prog: &yavs_ir::Program, id: yavs_ir::FuncId) -> String {
    match prog.copy_tag(id) {
        Some(t) => format!("c{t:x}"),
        None => id.0.to_string(),
    }
}

/// A hash of what the function is (`Program::code_key_bytes`): a list line of another function is
/// not used (pcs mean nothing there).
pub(crate) fn body_hash(prog: &yavs_ir::Program, f: &yavs_ir::Func) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    prog.code_key_bytes(f).hash(&mut h);
    h.finish()
}

/// The program's list: per function key, its hash, the instructions earlier runs stepped in its
/// cheap blocks, and its never-run pcs.
type Stored = HashMap<String, (u64, u64, HashSet<u32>)>;

struct ListState {
    key: yavs_cache::Key,
    stored: Stored,
    /// Functions given a list in this run, by id: their key and their step limit (`list_flush`).
    used: HashMap<u32, (String, u64)>,
    /// `rec`: functions given compiled code in this run, by id (no line from the interpreter).
    built: HashSet<u32>,
}

static LIST_STATE: Mutex<Option<ListState>> = Mutex::new(None);

fn parse_list(text: &str) -> Stored {
    text.lines()
        .filter_map(|l| {
            let mut w = l.split(' ');
            let f = w.next()?.to_string();
            let hash = u64::from_str_radix(w.next()?, 16).ok()?;
            let steps = w.next()?.strip_prefix('s')?.parse().ok()?;
            // (A pc, or `a-b`: the pcs from a to b.)
            let mut pcs = HashSet::new();
            for p in w {
                match p.split_once('-') {
                    Some((a, b)) => pcs.extend(a.parse::<u32>().ok()?..=b.parse().ok()?),
                    None => _ = pcs.insert(p.parse().ok()?),
                }
            }
            Some((f, (hash, steps, pcs)))
        })
        .collect()
}

/// The list of `prog`, read from the store at the first call.
fn with_list<R>(prog: &yavs_ir::Program, f: impl FnOnce(&mut ListState) -> R) -> R {
    let mut st = LIST_STATE.lock().unwrap_or_else(PoisonError::into_inner);
    let st = st.get_or_insert_with(|| {
        let mut k = yavs_cache::KeyBuilder::new(LIST);
        k.text(&yavs_ir::opt::fingerprint())
            .part(&yavs_llvm::program_digest(prog).to_le_bytes());
        let key = k.finish();
        let stored = match list_mode() {
            ListMode::Read | ListMode::Rec => crate::cache::store()
                .and_then(|s| s.get(LIST, &key))
                .map(|e| parse_list(&String::from_utf8_lossy(&e)))
                .unwrap_or_default(),
            _ => Stored::new(),
        };
        ListState {
            key,
            stored,
            used: HashMap::new(),
            built: HashSet::new(),
        }
    });
    f(st)
}

/// Note the program of a coverage run, so `list_flush` knows its key.
pub(crate) fn list_open(prog: &yavs_ir::Program) {
    with_list(prog, |_| ());
}

/// The never-run pcs of function `id` (`f`) that its code made now follows: `None` with the list
/// off or in a coverage run, for a function with no line, a line of another body, or one that
/// stepped too much of them already (`rt::tripped`, in this run or in earlier ones together).
pub(crate) fn listed(
    prog: &yavs_ir::Program,
    id: yavs_ir::FuncId,
    f: &yavs_ir::Func,
) -> Option<std::rc::Rc<HashSet<u32>>> {
    let mode = list_mode();
    if !matches!(mode, ListMode::Read | ListMode::Rec | ListMode::All) || yavs_llvm::rt::tripped(id)
    {
        return None;
    }
    let pcs: HashSet<u32> = if mode == ListMode::All {
        (0..f.code.len() as u32).collect()
    } else {
        let key = func_key(prog, id);
        let hash = body_hash(prog, f);
        let limit = yavs_llvm::rt::cheap_limit(f.code.len() as u32);
        let pcs = with_list(prog, |st| {
            st.stored
                .get(&key)
                .filter(|(h, steps, _)| *h == hash && *steps < limit)
                .map(|(_, _, p)| p.clone())
        })?;
        with_list(prog, |st| st.used.insert(id.0, (key, limit)));
        pcs
    };
    Some(std::rc::Rc::new(pcs))
}

/// At the end of a run: a coverage run stores what never ran in place of the list; a run that
/// read it adds what each function stepped to its line and drops the functions past their limit
/// (made again in full, which the next run links, or compiles once): a block that runs a little in
/// every run costs its compile at most once.
fn list_flush(rt: Option<&yavs_interp::Runtime>) {
    let mode = list_mode();
    // (`rec`: the program's list, also in a run that compiled nothing.)
    if mode == ListMode::Rec
        && let Some(rt) = rt
    {
        list_open(&rt.prog);
    }
    let mut st = LIST_STATE.lock().unwrap_or_else(PoisonError::into_inner);
    let (Some(st), Some(store)) = (st.as_mut(), crate::cache::store()) else {
        return;
    };
    let text = match mode {
        ListMode::Write => {
            // Per function, both its codes merged: a pc ran if either ran it, and is listed only
            // when a table saw it and none saw it run. A function none of whose blocks ran says
            // nothing.
            let mut by: HashMap<String, (u64, HashSet<u32>, HashSet<u32>)> = HashMap::new();
            let hashes: HashMap<String, u64> = TABLES
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .map(|t| (t.key.clone(), t.hash))
                .collect();
            for body in snapshot() {
                let fk = body.key[1..].to_string();
                let e = by.entry(fk).or_insert_with(|| {
                    (
                        hashes.get(&body.key).copied().unwrap_or(0),
                        HashSet::new(),
                        HashSet::new(),
                    )
                });
                for ((pc, _), ran) in body.blocks {
                    if ran { &mut e.1 } else { &mut e.2 }.insert(pc);
                }
            }
            let mut lines: Vec<String> = by
                .into_iter()
                .filter(|(_, (_, ran, _))| !ran.is_empty())
                .map(|(fk, (hash, ran, zero))| {
                    let mut pcs: Vec<u32> = zero.difference(&ran).copied().collect();
                    pcs.sort_unstable();
                    let mut l = format!("{fk} {hash:x} s0");
                    for pc in pcs {
                        l += &format!(" {pc}");
                    }
                    l
                })
                .collect();
            lines.sort();
            lines.join("\n") + "\n"
        }
        ListMode::Read | ListMode::Rec => {
            // (Per function key: what this run stepped, and its limit.)
            let mut ran: HashMap<&str, (u64, u64)> = HashMap::new();
            for (&id, (key, limit)) in &st.used {
                let n = yavs_llvm::rt::cheap_steps(yavs_ir::FuncId(id));
                if n > 0 {
                    let e = ran.entry(key.as_str()).or_insert((0, *limit));
                    e.0 = e.0.saturating_add(n);
                }
            }
            let old = store.get(LIST, &st.key);
            let old = old.as_deref().map(String::from_utf8_lossy);
            let stepped = match (&old, ran.is_empty()) {
                (Some(o), false) => add_steps(o, &ran),
                (Some(o), true) => o.to_string(),
                (None, _) => String::new(),
            };
            let text = match rt {
                Some(rt) if mode == ListMode::Rec => {
                    let (text, n) = recorded(&stepped, &rt.prog, rt.block_entries(), &st.built);
                    RECORDED.store(n, std::sync::atomic::Ordering::Relaxed);
                    text
                }
                _ => stepped,
            };
            if old.as_deref() == Some(text.as_str()) || (old.is_none() && text.is_empty()) {
                return;
            }
            text
        }
        ListMode::Off | ListMode::All => return,
    };
    store.put(LIST, &st.key, text.as_bytes());
}

/// `rec`: function `id` got compiled code in this run (no line from what the interpreter saw).
pub(crate) fn note_built(prog: &yavs_ir::Program, id: yavs_ir::FuncId) {
    with_list(prog, |st| st.built.insert(id.0));
}

/// The list `old` with the lines of the functions the interpreter ran all run (`entries`: per
/// function, the pc of each op and whether it was entered; `built`: the functions with compiled
/// code, left as they are): the pcs of the IR blocks none of whose ops was entered never ran, and
/// a line already there keeps only the pcs both say never ran (none left: no line). Returns the
/// list and how many lines it wrote, changed or dropped.
fn recorded(
    old: &str,
    prog: &yavs_ir::Program,
    entries: Vec<(yavs_ir::FuncId, Vec<(u32, bool)>)>,
    built: &HashSet<u32>,
) -> (String, usize) {
    let mut lines: std::collections::BTreeMap<String, String> = old
        .lines()
        .filter_map(|l| Some((l.split(' ').next()?.to_string(), l.to_string())))
        .collect();
    let mut n = 0;
    for (id, pcs) in entries {
        let Some(f) = prog.func(id).filter(|_| !built.contains(&id.0)) else {
            continue;
        };
        let (key, hash) = (func_key(prog, id), body_hash(prog, f));
        // (A block with ops and none of them entered; one with no op of its own counts as run, and
        // so does the first: a call may start past its ops, `Code::skip`.)
        let mut never: Vec<u32> = f
            .blocks
            .iter()
            .filter(|(_, b)| b.start > 0)
            .filter(|(_, b)| {
                let i = pcs.partition_point(|p| p.0 < b.start);
                let ops = pcs[i..].iter().take_while(|p| p.0 < b.end);
                ops.clone().next().is_some() && !ops.into_iter().any(|p| p.1)
            })
            .flat_map(|(_, b)| b.start..b.end)
            .collect();
        let mut steps = 0;
        if let Some((_, (_, s, was))) = lines
            .get(&key)
            .and_then(|l| parse_list(l).into_iter().next())
            .filter(|(_, (h, _, _))| *h == hash)
        {
            never.retain(|pc| was.contains(pc));
            steps = s;
        }
        let line = (!never.is_empty()).then(|| list_line(&key, hash, steps, &never));
        if lines.get(&key) != line.as_ref() {
            n += 1;
            match line {
                Some(l) => lines.insert(key, l),
                None => lines.remove(&key),
            };
        }
    }
    (lines.into_values().map(|l| l + "\n").collect(), n)
}

/// A line of the list: function key, hash, steps, then the pcs (sorted), runs of them as `a-b`.
fn list_line(key: &str, hash: u64, steps: u64, pcs: &[u32]) -> String {
    let mut l = format!("{key} {hash:x} s{steps}");
    let mut i = 0;
    while i < pcs.len() {
        let mut j = i;
        while j + 1 < pcs.len() && pcs[j + 1] == pcs[j] + 1 {
            j += 1;
        }
        l += &if j > i {
            format!(" {}-{}", pcs[i], pcs[j])
        } else {
            format!(" {}", pcs[i])
        };
        i = j + 1;
    }
    l
}

/// The list `old` with what this run stepped added to each line of `ran` (by function key: the
/// instructions, the limit), and the lines that reach their limit left out.
fn add_steps(old: &str, ran: &HashMap<&str, (u64, u64)>) -> String {
    let mut text = String::new();
    for l in old.lines() {
        let w: Vec<&str> = l.splitn(4, ' ').collect();
        let (Some(&(n, limit)), [f, hash, steps, rest @ ..]) = (ran.get(w[0]), &w[..]) else {
            text += l;
            text.push('\n');
            continue;
        };
        let before: u64 = steps
            .strip_prefix('s')
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let now = before.saturating_add(n);
        if now < limit {
            text += &format!("{f} {hash} s{now}");
            for r in rest {
                text += &format!(" {r}");
            }
            text.push('\n');
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A function that steps a little in every run leaves the list once the runs together reach
    /// its limit; the others keep their lines.
    #[test]
    fn steps_add_up_over_runs_until_the_line_goes() {
        let ran = HashMap::from([("3", (40, 100))]);
        let one = add_steps("3 ab s0 4 9\nc1f 12 s7 2\n", &ran);
        assert_eq!(one, "3 ab s40 4 9\nc1f 12 s7 2\n");
        assert_eq!(parse_list(&one)["3"], (0xab, 40, HashSet::from([4, 9])));
        let two = add_steps(&one, &ran);
        assert_eq!(two, "3 ab s80 4 9\nc1f 12 s7 2\n");
        assert_eq!(add_steps(&two, &ran), "c1f 12 s7 2\n");
    }

    /// `rec` writes runs of pcs as `a-b`; a line reads back as the pcs it names.
    #[test]
    fn runs_of_pcs_read_back_as_their_pcs() {
        let l = list_line("7", 0xab, 3, &[2, 3, 4, 9, 11, 12]);
        assert_eq!(l, "7 ab s3 2-4 9 11-12");
        let want = HashSet::from([2, 3, 4, 9, 11, 12]);
        assert_eq!(parse_list(&l)["7"], (0xab, 3, want));
        let ran = HashMap::from([("7", (5, 100))]);
        assert_eq!(add_steps(&(l + "\n"), &ran), "7 ab s8 2-4 9 11-12\n");
    }
}
