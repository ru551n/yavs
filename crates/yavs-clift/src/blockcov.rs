//! `YAVS_CLIFT_BLOCKCOV=<dir>` (a measurement switch): generated code stores one byte into a
//! per-body table at the entry of every block it lowers, and the run writes which blocks ran
//! (`cov-<pid>.tsv`: body symbol, IR pc, IR block id or `-`, 0 or 1, `key`). The code differs from
//! the normal code, so byte sizes come from a run without the switch (`YAVS_CLIFT_CODEDIR`).
//! `YAVS_CLIFT_COLD=<cov tsv>` (a lab switch) marks the blocks that file lists as never run cold;
//! `YAVS_CLIFT_CHEAP=<cov tsv>|all|alt` (a lab switch) lowers them cheaply (`cheap`).

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::{Mutex, OnceLock, PoisonError};

/// One block of a body: the pc it starts at and the IR block that starts there, if any (the
/// others are entries after a call).
pub type BlockInfo = (u32, Option<u32>);

struct Table {
    name: String,
    key: String,
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
pub(crate) fn register(name: String, key: String, at: usize, blocks: Vec<BlockInfo>) {
    TABLES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Table {
            name,
            key,
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

/// The blocks of a body that get the cheap lowering (`YAVS_CLIFT_CHEAP`).
#[derive(Clone, Copy)]
pub(crate) enum Cheap {
    /// `all`: every block (a test mode: correctness must not rest on the list).
    All,
    /// `alt`: every second block, so cheap and full blocks meet (a test mode).
    Alt,
    /// A coverage dump's never-run blocks of this body.
    Set(&'static HashSet<u32>),
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
    static LIST: OnceLock<Option<Result<Cheap, Lists>>> = OnceLock::new();
    let l = LIST.get_or_init(|| {
        let p = std::env::var_os("YAVS_CLIFT_CHEAP")?;
        Some(match p.to_str() {
            Some("all") => Ok(Cheap::All),
            Some("alt") => Ok(Cheap::Alt),
            _ => Err(never_run("YAVS_CLIFT_CHEAP", &p)),
        })
    });
    match l.as_ref()? {
        Ok(c) => Some(*c),
        Err(m) => m.get(key).map(Cheap::Set),
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

/// Write `cov-<pid>.tsv` into the directory of the switch, when it is set (a no-op otherwise).
pub fn write() {
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
