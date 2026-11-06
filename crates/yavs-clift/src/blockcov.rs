//! `YAVS_CLIFT_BLOCKCOV=<dir>` (a measurement switch): generated code stores one byte into a
//! per-body table at the entry of every block it lowers, and the run writes which blocks ran
//! (`cov-<pid>.tsv`: body symbol, IR pc, IR block id or `-`, 0 or 1). The code differs from the
//! normal code, so byte sizes come from a run without the switch (`YAVS_CLIFT_CODEDIR`).

use std::io::Write;
use std::sync::{Mutex, PoisonError};

/// One block of a body: the pc it starts at and the IR block that starts there, if any (the
/// others are entries after a call).
pub type BlockInfo = (u32, Option<u32>);

struct Table {
    name: String,
    /// The address of the body's table, one byte per block, in module data that is never freed.
    at: usize,
    blocks: Vec<BlockInfo>,
}

static TABLES: Mutex<Vec<Table>> = Mutex::new(Vec::new());

/// A body and its blocks, whether each ran.
pub struct BodyCov {
    pub name: String,
    pub blocks: Vec<(BlockInfo, bool)>,
}

/// List a finalized body's table.
pub(crate) fn register(name: String, at: usize, blocks: Vec<BlockInfo>) {
    TABLES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Table { name, at, blocks });
}

/// Every body made so far with the blocks that ran until now.
pub fn snapshot() -> Vec<BodyCov> {
    let tables = TABLES.lock().unwrap_or_else(PoisonError::into_inner);
    tables
        .iter()
        .map(|t| BodyCov {
            name: t.name.clone(),
            blocks: t
                .blocks
                .iter()
                .enumerate()
                // SAFETY: `at` is the table of `blocks.len()` bytes in module data that is never
                // freed; compiled code writes it, a byte read at any time is fine.
                .map(|(k, &b)| {
                    (
                        b,
                        unsafe { std::ptr::read_volatile((t.at + k) as *const u8) } != 0,
                    )
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
                writeln!(out, "{}\t{pc}\t{ir}\t{}", body.name, u8::from(ran))?;
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
