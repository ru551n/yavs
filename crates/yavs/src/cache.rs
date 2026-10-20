//! The compilation cache in front of analysis (`docs/decisions.md` "Compilation cache").
//!
//! `elab` and `run` analyze the closure of the top level. When the same closure was analyzed
//! before with the same options and the same sources, the analyzed `Db` is read back from the
//! work library's cache directory instead; anything that does not match is analyzed from source
//! as before, so the cache can only save time. Diagnostics are part of the image and come out
//! identically.

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use yavs_cache::{Key, KeyBuilder, Store};
use yavs_diag::{Diagnostic, MessageFormat, SourceMap};
use yavs_lib::{ClosureFile, Libraries};
use yavs_sem::{Db, Options, Reported, StageCache};

/// The kind name of analysis images in the store.
const KIND: &str = "sem";

/// Everything analysis of `files` depends on besides the sources' content, which `Db::from_image`
/// verifies itself: the options and the files with their recorded hashes.
fn key(opts: &Options, files: &[ClosureFile]) -> Key {
    let mut k = KeyBuilder::new(KIND);
    k.text(opts.rev.as_str())
        .text(&format!(
            "{} {} {:?}",
            opts.relaxed, opts.psl, opts.encoding
        ))
        .text(&opts.stdlib.to_string_lossy());
    for (n, v) in &opts.defines {
        k.text(n).text(v);
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    for f in files {
        k.text(&f.library).text(&f.hash).text(&f.std);
        // A relative path means a different file from another directory.
        if f.path.is_relative() {
            k.text(&cwd.to_string_lossy());
        }
        k.text(&f.path.to_string_lossy());
    }
    k.finish()
}

/// What later stages cache for one analysis: entries keyed by the analysis' key and the stage's
/// own part, valid only for a `Db` of the extent it was analyzed to (`StageCache`).
#[derive(Debug)]
struct Stages {
    store: Store,
    /// The analysis' key.
    base: String,
    extent: Vec<usize>,
}

impl Stages {
    fn key(&self, extent: &[usize], kind: &str, part: &[u8]) -> Option<Key> {
        (extent == self.extent).then(|| {
            let mut k = KeyBuilder::new(kind);
            k.text(&self.base).part(part);
            k.finish()
        })
    }
}

impl StageCache for Stages {
    fn get(&self, extent: &[usize], kind: &str, part: &[u8]) -> Option<Vec<u8>> {
        let k = self.key(extent, kind, part)?;
        let e = self.store.get(kind, &k)?;
        debug(format!("hit {kind} {}", k.hex()));
        Some(e.to_vec())
    }

    fn put(&self, extent: &[usize], kind: &str, part: &[u8], bytes: &[u8]) {
        if let Some(k) = self.key(extent, kind, part) {
            debug(format!("store {kind} {} ({} bytes)", k.hex(), bytes.len()));
            self.store.put(kind, &k, bytes);
        }
    }
}

/// Let the stages after analysis keep their results next to the analysis image.
fn with_stages(mut db: Db, store: Store, k: &Key) -> Db {
    let extent = db.extent();
    db.set_stages(Arc::new(Stages {
        store,
        base: k.hex(),
        extent,
    }));
    db
}

fn debug(msg: String) {
    if std::env::var("YAVS_LOG").is_ok_and(|v| v == "debug") {
        eprintln!("debug: cache: {msg}");
    }
}

/// Analyze `files` and load the package bodies elaboration needs (`yavs_elab::load_bodies`),
/// so that the image holds them too; a body error is reported like any analysis error.
fn analyze(db: &mut Db, files: &[ClosureFile]) -> Result<(), Reported> {
    db.analyze_closure(files)?;
    if let Err(ds) = yavs_elab::load_bodies(db) {
        for d in ds {
            db.report(d);
        }
        return Err(Reported);
    }
    Ok(())
}

/// Analyze `files` into a `Db` for `opts`: from the cache of library directory `work_dir` when
/// possible, else from source (and then stored there when it analyzed cleanly).
pub fn analyze_closure(
    opts: Options,
    files: &[ClosureFile],
    work_dir: &Path,
    enabled: bool,
) -> (Db, Result<(), Reported>) {
    // ponytail: a custom std/ieee tree ($YAVS_STDLIB) is not cached (its files are not verified)
    let enabled = enabled && opts.stdlib == Path::new(yavs_sem::stdlib::EMBEDDED);
    if !enabled {
        let mut db = Db::new(opts);
        let r = analyze(&mut db, files);
        db.settle();
        return (db, r);
    }
    let store = Store::of_library(work_dir);
    let k = key(&opts, files);
    if let Some(bytes) = store.get(KIND, &k) {
        // The closure verified the content hash of its files (`yavs_lib::closure`), and the key
        // holds those hashes: only the files outside it (`std`, `ieee`) are compared again.
        let verified: std::collections::HashSet<String> =
            files.iter().map(|f| f.path.display().to_string()).collect();
        let (buf, start) = bytes.into_parts();
        match Db::from_image(
            opts.clone(),
            yavs_sem::image::Image::new(buf, start),
            &|p| verified.contains(p),
        ) {
            Ok(db) => {
                debug(format!("hit {}", k.hex()));
                return (with_stages(db, store, &k), Ok(()));
            }
            Err(e) => {
                debug(e.to_string());
            }
        }
    }
    let mut db = Db::new(opts);
    let r = analyze(&mut db, files);
    db.settle();
    if r.is_err() {
        return (db, r);
    }
    match db.to_image() {
        Ok(image) => {
            debug(format!("store {} ({} bytes)", k.hex(), image.len()));
            store.put(KIND, &k, &image);
            (with_stages(db, store, &k), r)
        }
        Err(why) => {
            debug(format!("no image: {why}"));
            (db, r)
        }
    }
}

#[derive(clap::Subcommand)]
pub enum Command {
    /// Remove the cache entries of libraries.
    Clean(CleanArgs),
}

#[derive(clap::Args)]
pub struct CleanArgs {
    /// A library, `NAME:PATH`; the cache of its directory is removed.
    #[arg(
        long = "work",
        alias = "map",
        value_name = "NAME:PATH",
        required = true
    )]
    libs: Vec<String>,
}

pub fn run(c: Command, format: MessageFormat, color: bool) -> ExitCode {
    match c {
        Command::Clean(a) => clean(&a, format, color),
    }
}

fn clean(a: &CleanArgs, format: MessageFormat, color: bool) -> ExitCode {
    let mut libs = Libraries::new();
    let mut code = ExitCode::SUCCESS;
    let fail = |d: Diagnostic| {
        eprint!(
            "{}",
            yavs_diag::render(&SourceMap::default(), &d, format, color)
        );
    };
    for spec in &a.libs {
        if let Err(d) = libs.add_spec(spec) {
            fail(*d);
            return ExitCode::from(2);
        }
    }
    for spec in &a.libs {
        let dir = spec.split_once(':').map_or("", |(_, d)| d);
        match Store::of_library(dir.as_ref()).clean() {
            Ok(c) => println!(
                "{dir}: removed {} {} ({} bytes)",
                c.entries,
                if c.entries == 1 { "entry" } else { "entries" },
                c.bytes
            ),
            Err(e) => {
                fail(Diagnostic::error(
                    "E0123",
                    format!("cannot clean the cache of `{dir}`: {e}"),
                ));
                code = ExitCode::from(1);
            }
        }
    }
    code
}
