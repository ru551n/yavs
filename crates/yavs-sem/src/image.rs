//! Analysis images: the settled state of a `Db` as bytes, for the compilation cache
//! (`docs/decisions.md` "Compilation cache").
//!
//! An image is the whole `Db` after a closure was analyzed, not a set of per-unit pieces: ids
//! index shared arenas and are only meaningful together, so a restored `Db` is the same value,
//! id for id, that analysis from source produces. That is what makes a warm run byte-identical
//! to a cold one. What is not stored: the options (the caller's are used), the library sources
//! (manifests are read again), and the transient state of an analysis in progress (which
//! [`Db::settle`] clears and [`Db::to_image`] requires to be empty).

use crate::db::Db;
use crate::ids::{Id, LibId};
use crate::loader::LibSource;
use crate::{LangRev, Options};

/// The bytes of an image as read from the store (a mapped file). The restored `Db` keeps the
/// buffer and decodes from it on demand.
pub struct Image {
    buf: yavs_diag::Buffer,
}

impl Image {
    pub fn new(buf: yavs_diag::Buffer) -> Image {
        Image { buf }
    }
}

/// Why an image could not be turned back into a `Db`; the caller analyzes from source instead.
#[derive(Debug)]
pub enum ImageError {
    /// The bytes do not decode (truncated, written by another build).
    Corrupt(String),
    /// The image no longer matches the files or libraries on disk.
    Stale(String),
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageError::Corrupt(s) => write!(f, "corrupt image: {s}"),
            ImageError::Stale(s) => write!(f, "stale image: {s}"),
        }
    }
}

impl Db {
    /// Drop what only an analysis in progress needs, so that the state a cold run continues
    /// from and the state an image restores are the same. Call after the last analysis.
    pub fn settle(&mut self) {
        self.memo.clear();
        self.name_memo.clear();
        self.ext_memo.clear();
        self.overflow.clear();
        self.context_subtype = None;
        self.top_context = None;
        self.files.clear();
        self.facts = Some(self.scan_facts(self.facts.as_ref()));
        self.unit_facts.clear();
    }

    /// The store for stage results (`Options::stages`).
    pub fn stages(&self) -> Option<&std::sync::Arc<dyn crate::StageCache>> {
        self.opts.stages.as_ref()
    }

    /// Use `s` as the store for stage results from here on (see [`Db::extent`]).
    pub fn set_stages(&mut self, s: std::sync::Arc<dyn crate::StageCache>) {
        self.opts.stages = Some(s);
    }

    /// The size of every arena: equal before and after a stage that does not grow the `Db`
    /// (instantiation appends), so it says whether stage results of one still hold for the other.
    pub fn extent(&self) -> Vec<usize> {
        vec![
            self.libs.len(),
            self.units.len(),
            self.decls.len(),
            self.scopes.len(),
            self.types.len(),
            self.subtypes.len(),
            self.exprs.len(),
            self.stmts.len(),
            self.consts.len(),
            self.insts.len(),
            self.views.len(),
            self.externals.len(),
            self.attr_values.len(),
        ]
    }

    /// [`Db::extent`] and, last, a digest of the files the `Db` was made from (their paths and
    /// content digests, in load order): the key part of a stage entry. Units loaded after the
    /// analysis (while elaborating) grow the extent but are deterministic, so the entry holds
    /// for a `Db` of the same extent made from the same files (#671).
    pub fn stage_extent(&self) -> Vec<usize> {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        for f in self.sources.digests() {
            f.hash(&mut h);
        }
        let mut e = self.extent();
        e.push(h.finish() as usize);
        e
    }

    /// The image of this `Db`, or why it cannot be restored later: an error was reported, an
    /// analysis is in progress, or work is pending.
    pub fn to_image(&mut self) -> Result<Vec<u8>, String> {
        let pending = [
            ("errors", self.error_count()),
            ("stack", self.stack.len()),
            ("pending label attributes", self.pending_label_attrs.len()),
            ("pending out reads", self.out_reads.len()),
            ("pending instance checks", self.pending_inst_checks.len()),
            ("pending groups", self.pending_groups.len()),
            ("pending implicit operations", self.pending_implicit.len()),
            ("pending specifications", self.pending_specs.len()),
            ("statement owners", self.stmt_owners.len()),
            ("hidden types", self.hidden_types.len()),
            ("parameter types", self.param_anon.len()),
            ("instance types", self.inst_types.len()),
        ];
        if let Some((what, n)) = pending.iter().find(|p| p.1 != 0) {
            return Err(format!("{n} {what}"));
        }
        // Found now, while every page is decoded, instead of by every run that reads the image.
        self.run_time_pkgs = Some((self.run_time_key(), self.run_time_package_set()));
        let (payload, table) = yavs_syntax::encode_table(|| postcard::to_stdvec(self));
        let payload = payload.map_err(|e| e.to_string())?;
        let table = postcard::to_stdvec(&table).map_err(|e| e.to_string())?;
        let mut out = Vec::with_capacity(8 + table.len() + payload.len());
        out.extend_from_slice(&(table.len() as u64).to_le_bytes());
        out.extend_from_slice(&table);
        out.extend_from_slice(&payload);
        Ok(out)
    }

    /// Restore the `Db` of an image under the options of this run. `Stale` when a source file
    /// the image was made from changed (or a library cannot be read again). Files for which
    /// `verified` says so are not read again: the caller already knows they hold what the image
    /// was made from.
    pub fn from_image(
        opts: Options,
        image: Image,
        verified: &dyn Fn(&str) -> bool,
    ) -> Result<Db, ImageError> {
        let corrupt = |s: &str| ImageError::Corrupt(s.into());
        let bytes = &image.buf[..];
        let n = bytes
            .get(..8)
            .and_then(|b| <[u8; 8]>::try_from(b).ok())
            .map(u64::from_le_bytes)
            .ok_or_else(|| corrupt("short"))? as usize;
        let table_bytes = bytes.get(8..8 + n).ok_or_else(|| corrupt("short table"))?;
        let table: yavs_syntax::SymbolTable =
            postcard::from_bytes(table_bytes).map_err(|e| ImageError::Corrupt(e.to_string()))?;
        let payload = &bytes[8 + n..];
        let mut db: Db = yavs_syntax::DecodeTable::new(table)
            .run(|| yavs_diag::sharing(&image.buf, || postcard::from_bytes(payload)))
            .map_err(|e| ImageError::Corrupt(e.to_string()))?;
        if db.rev != opts.rev {
            return Err(ImageError::Stale("design revision".into()));
        }
        db.opts = opts;
        let rev: LangRev = db.rev;
        for i in 0..db.libs.len() {
            let id = LibId::from_index(i);
            let key = db.libs[id].name.key.as_str();
            let source = match &db.libs[id].source {
                LibSource::Manifest(_) => match db.opts.libraries.get(key) {
                    Some(l) => LibSource::Manifest(
                        l.read()
                            .map_err(|_| ImageError::Stale(format!("library {key}")))?,
                    ),
                    None => return Err(ImageError::Stale(format!("library {key} unmapped"))),
                },
                LibSource::StdDir(_) => {
                    LibSource::StdDir(db.opts.stdlib.join(key).join(rev.as_str()))
                }
                LibSource::Files => LibSource::Files,
            };
            db.libs[id].source = source;
        }
        db.sources
            .set_reader(|p| crate::stdlib::read(std::path::Path::new(p)));
        db.check_sources(verified)?;
        Ok(db)
    }

    /// Every file the analysis read, except the `verified` ones, must still hold the bytes it read.
    fn check_sources(&self, verified: &dyn Fn(&str) -> bool) -> Result<(), ImageError> {
        for (path, digest) in self.sources.digests().filter(|(p, _)| !verified(p)) {
            match crate::stdlib::read(std::path::Path::new(path)) {
                Ok(now) if yavs_diag::digest(&now) == digest => {}
                _ => return Err(ImageError::Stale(path.to_string())),
            }
        }
        Ok(())
    }
}
