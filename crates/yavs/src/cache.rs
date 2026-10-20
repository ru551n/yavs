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

/// The kind name of prefix images: the analysis of the first files of a closure (see [`bounds`]).
const PREFIX_KIND: &str = "pre";

/// The smallest prefix (in files) that gets an image of its own.
const PREFIX_MIN: usize = 8;

/// At most this many prefix images are made for one closure.
const PREFIX_MAX: usize = 3;

/// Everything analysis of `files` depends on besides the sources' content, which `Db::from_image`
/// verifies itself: the options and the files with their recorded hashes.
fn key(kind: &str, opts: &Options, files: &[ClosureFile]) -> Key {
    let mut k = KeyBuilder::new(kind);
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
    fn get(&self, extent: &[usize], kind: &str, part: &[u8]) -> Option<yavs_diag::Buffer> {
        let k = self.key(extent, kind, part)?;
        let e = self.store.get(kind, &k)?;
        debug(format!("hit {kind} {}", k.hex()));
        Some(yavs_diag::Buffer::new(e))
    }

    fn note(&self, msg: &str) {
        debug(msg.to_string());
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

/// Analyze `files` into `db`.
fn analyze(db: &mut Db, files: &[ClosureFile]) -> Result<(), Reported> {
    db.analyze_closure(files)
}

/// Load the package bodies elaboration needs (`yavs_elab::load_bodies`), so that the image holds
/// them too; a body error is reported like any analysis error.
fn bodies(db: &mut Db) -> Result<(), Reported> {
    if let Err(ds) = yavs_elab::load_bodies(db) {
        for d in ds {
            db.report(d);
        }
        return Err(Reported);
    }
    Ok(())
}

/// Where the first files of a closure end and other tests' closures may go on differently: the
/// places where the library changes (a verification library's packages come first, then the
/// next library's, then the testbench's), at least [`PREFIX_MIN`] files in and apart, a few at
/// most.
fn bounds(files: &[ClosureFile]) -> Vec<usize> {
    let mut at: Vec<usize> = Vec::new();
    for k in PREFIX_MIN..files.len() {
        if files[k - 1].library != files[k].library
            && at.last().is_none_or(|&p| k - p >= PREFIX_MIN)
        {
            at.push(k);
        }
    }
    at.truncate(PREFIX_MAX);
    at
}

/// The cache directory of prefix `files[..k]`: the one of the library of its last file, which
/// every closure with this prefix maps (the work library's own cache is per testbench).
fn prefix_store(opts: &Options, files: &[ClosureFile], k: usize) -> Option<Store> {
    let lib = opts.libraries.get(&files[k - 1].library)?;
    Some(Store::of_library(&lib.dir))
}

/// Analyze `files` into a `Db` for `opts`: from the cache of library directory `work_dir` when
/// possible, else from source (and then stored there when it analyzed cleanly). A closure that
/// misses is not always analyzed from scratch: the analysis of its first files (`bounds`) is
/// cached next to the library of the last of them, because the closures of the testbenches of
/// one verification library start with the same packages, and the longest cached prefix is
/// restored and the rest analyzed on top of it.
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
        let r = analyze(&mut db, files).and_then(|()| bodies(&mut db));
        db.settle();
        return (db, r);
    }
    let store = Store::of_library(work_dir);
    let k = key(KIND, &opts, files);
    // The closure verified the content hash of its files (`yavs_lib::closure`), and the key
    // holds those hashes: only the files outside it (`std`, `ieee`) are compared again.
    let verified: std::collections::HashSet<String> =
        files.iter().map(|f| f.path.display().to_string()).collect();
    let restore = |bytes: yavs_cache::Entry, what: &str| match Db::from_image(
        opts.clone(),
        yavs_sem::image::Image::new(yavs_diag::Buffer::new(bytes)),
        &|p| verified.contains(p),
    ) {
        Ok(db) => Some(db),
        Err(e) => {
            debug(format!("{what}: {e}"));
            None
        }
    };
    if let Some(db) = store.get(KIND, &k).and_then(|b| restore(b, "image")) {
        debug(format!("hit {}", k.hex()));
        return (with_stages(db, store, &k), Ok(()));
    }
    let bounds = bounds(files);
    let mut start = 0;
    let mut db = None;
    for &b in bounds.iter().rev() {
        let pk = key(PREFIX_KIND, &opts, &files[..b]);
        let hit = prefix_store(&opts, files, b)
            .and_then(|s| s.get(PREFIX_KIND, &pk))
            .and_then(|e| restore(e, "prefix image"));
        if hit.is_some() {
            debug(format!("hit prefix of {b} files {}", pk.hex()));
            (start, db) = (b, hit);
            break;
        }
    }
    let mut db = db.unwrap_or_else(|| Db::new(opts.clone()));
    let first = start;
    for &b in bounds.iter().filter(|&&b| b > first) {
        if let Err(r) = analyze(&mut db, &files[start..b]) {
            return (db, Err(r));
        }
        start = b;
        // The bodies of the libraries outside the closure (ieee) cost a fifth of what is left
        // after the restore; those of the closure's own files are analyzed with them.
        let own: std::collections::HashSet<&str> =
            files.iter().map(|f| f.library.as_str()).collect();
        if let Err(r) = yavs_elab::load_bodies_in(&mut db, Some(&|l: &str| !own.contains(l))) {
            for d in r {
                db.report(d);
            }
            return (db, Err(Reported));
        }
        db.settle();
        match (db.to_image(), prefix_store(&opts, files, b)) {
            (Ok(image), Some(s)) => {
                let pk = key(PREFIX_KIND, &opts, &files[..b]);
                debug(format!(
                    "store prefix of {b} files {} ({} bytes)",
                    pk.hex(),
                    image.len()
                ));
                s.put(PREFIX_KIND, &pk, &image);
            }
            (Err(why), _) => debug(format!("no prefix image: {why}")),
            _ => {}
        }
    }
    let r = analyze(&mut db, &files[start..]);
    let r = r.and_then(|()| bodies(&mut db));
    db.settle();
    if r.is_err() {
        return (db, r);
    }
    // The image of a closure is made on its second analysis: a design that runs once (one test of
    // a suite) must not pay a fifth of its run for an image nobody reads.
    if !store.noted(KIND, &k) {
        store.note(KIND, &k);
        debug(format!("note {}", k.hex()));
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
