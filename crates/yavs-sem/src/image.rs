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
        self.return_node = None;
        self.files.clear();
        self.facts = Some(self.scan_facts(self.facts.as_ref()));
        self.unit_facts.clear();
        self.settled = self.extent();
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
    /// content digests, in load order) and of what elaboration added to it: the key part of a
    /// stage entry. Elaboration appends in the order it meets statements, which the generics
    /// choose (implicit aliases of external names, instances of generic entities, units loaded
    /// on demand), so equal extents can hold other entries under the same ids (#1142): their
    /// content is digested, and the implicit aliases each statement's uses were turned into.
    pub fn stage_extent(&self) -> Vec<usize> {
        // A run asks it a few times at the same extent (packages, their state, the design):
        // the digest is kept while the sizes, the tracked arenas' writes and `ext_uses` stay.
        // Debug builds compute it again and fail on a stale one.
        let now = (
            self.extent(),
            self.decls.version() + self.scopes.version() + self.subtypes.version(),
            self.ext_uses.len(),
        );
        let mut memo = self.stage_memo.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, key)) = memo.as_ref()
            && *at == now
        {
            debug_assert_eq!(
                *key,
                self.stage_extent_now(),
                "an entry past `settled` changed without a write the memo sees"
            );
            return key.clone();
        }
        let key = self.stage_extent_now();
        *memo = Some((now, key.clone()));
        key
    }

    fn stage_extent_now(&self) -> Vec<usize> {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        for f in self.sources.digests() {
            f.hash(&mut h);
        }
        let from = |k: usize| self.settled.get(k).copied().unwrap_or(0);
        // Hash containers in key order: theirs follows symbol numbers and insertion history.
        crate::ids::plain(|| {
            crate::ids::image_order(|| {
                yavs_syntax::text_symbols(|| {
                    tail(&self.libs, from(0), &mut h);
                    tail(&self.units, from(1), &mut h);
                    tail(&self.decls, from(2), &mut h);
                    tail(&self.scopes, from(3), &mut h);
                    tail(&self.types, from(4), &mut h);
                    tail(&self.subtypes, from(5), &mut h);
                    tail(&self.exprs, from(6), &mut h);
                    tail(&self.stmts, from(7), &mut h);
                    tail(&self.consts, from(8), &mut h);
                    tail(&self.insts, from(9), &mut h);
                    tail(&self.views, from(10), &mut h);
                    tail(&self.externals, from(11), &mut h);
                    tail(&self.attr_values, from(12), &mut h);
                })
            })
        });
        // Every statement elaboration meets gets an entry, most of them empty.
        let mut uses: Vec<_> = self
            .ext_uses
            .iter()
            .filter(|(_, d)| !d.is_empty())
            .collect();
        uses.sort_unstable_by_key(|(o, _)| owner_key(**o));
        for (o, ds) in uses {
            owner_key(*o).hash(&mut h);
            ds.iter().for_each(|d| d.index().hash(&mut h));
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
        let (payload, table) =
            yavs_syntax::encode_table(|| crate::ids::image_order(|| postcard::to_stdvec(self)));
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
        db.settled = db.extent();
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

/// A sort key of `o` that does not depend on hash order.
fn owner_key(o: crate::external::ExtOwner) -> (u8, usize) {
    use crate::external::ExtOwner;
    match o {
        ExtOwner::Stmt(s) => (0, s.index()),
        ExtOwner::Alias(d) => (1, d.index()),
        ExtOwner::Expr(e) => (2, e.index()),
        ExtOwner::Sub(d) => (3, d.index()),
    }
}

/// Hash the entries of `a` from index `from` on, as bytes that are the same in every process.
fn tail<I: Id, T: serde::Serialize>(
    a: &crate::ids::Arena<I, T>,
    from: usize,
    h: &mut impl std::hash::Hasher,
) {
    let mut buf = Vec::new();
    for i in from..a.len() {
        buf = match postcard::to_extend(a.get_at(i), buf) {
            Ok(b) => b,
            // Cannot happen for an entry an image holds; a key that never matches if it does.
            Err(_) => {
                let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
                t.map_or(0, |d| d.as_nanos()).to_le_bytes().to_vec()
            }
        };
    }
    h.write(&buf);
}
