//! Stages shared by the closures that start with the same library files (docs/decisions.md
//! "Library stages", #407): the lowered packages and every subprogram they declare, the
//! packages' state after initialisation, and the optimised copies of the library functions.
//!
//! A suite that runs one test per process gives every test a new closure, but the verification
//! library under it is the same files in the same order, and the analysis of those files is
//! cached on its own (a prefix of the closure, `yavs_sem::Prefix`). What is lowered from that
//! analysis is then the same for all of them, and the ids lowering hands out (functions, types,
//! constants) are the same too, because the lowering of a prefix is made in a fixed order from
//! the prefix alone: the `Db` it lowers from is the prefix's own image (`StageCache::prefix_db`),
//! never the closure's, whose entries later files may have completed (a package body that
//! follows the prefix completes the subprograms the prefix declared).

use std::collections::HashSet;
use std::sync::Arc;

use yavs_diag::Diagnostic;
use yavs_ir::OptCopy;
use yavs_lower::{LowerOptions, Lowerer};
use yavs_sem::hir::UnitHir;
use yavs_sem::{Db, StageCache, UnitId};

/// The stage entry kinds, in the cache of the library of the prefix's last file.
pub const LIB: &str = "lib";
const OPT: &str = "libopt";
pub(crate) const INIT: &str = "libinit";

/// How many different closures must have asked for a prefix's library before it is made: making
/// it costs about a third of a second (two lowerings and the image) and 10 MB of disk, and a first
/// run of a closure saves about a twentieth of that, so a library under fewer closures than this
/// costs more than it saves (a suite whose components have one or two closures each is the case).
pub(crate) const SIGHTINGS: usize = 4;

/// The smallest lowered library worth storing: packages of constants lower to almost nothing.
const MIN_IMAGE: usize = 256;

/// The part of a key every library stage entry shares: what lowering depends on besides the
/// prefix's files (which the key of the prefix names).
fn part(opts: &LowerOptions) -> Vec<u8> {
    key_part(opts.no_native_ieee)
}

/// [`part`] for the one option it holds, for a caller that asks whether an entry exists.
pub fn key_part(no_native_ieee: bool) -> Vec<u8> {
    vec![u8::from(no_native_ieee)]
}

/// What a library prefix was lowered to: the packages' units, the size of the program and of
/// the shared variables area, then the [`Lowerer::image`].
#[derive(serde::Serialize, serde::Deserialize)]
struct Header {
    packages: Vec<u32>,
    funcs: u32,
    shared: u32,
    /// A digest of the ids the lowering handed out (`Lowerer::fingerprint`): what is stored about
    /// it (optimised copies, package state) names it, so that two lowerings of one prefix that
    /// number anything differently can never be mixed. The image's bytes are no digest: maps
    /// are written in the order of the process that wrote them.
    id: u64,
}

/// A closure that lowers on top of a library prefix.
#[derive(Clone)]
pub(crate) struct LibUse {
    cache: Arc<dyn StageCache>,
    level: usize,
    part: Vec<u8>,
    /// The leading packages of the closure's list the prefix lowered.
    pub(crate) packages: usize,
    /// The functions the prefix lowered: ids below this are the same in every closure of it.
    pub(crate) funcs: usize,
    /// Bytes of the shared variables area the prefix's packages use.
    pub(crate) shared: u32,
    /// Optimised copies the entry held when read ([`LibUse::take_optimized`]).
    copies: Vec<OptCopy>,
    /// Their functions: only others are written back.
    stored: HashSet<u32>,
}

/// Whether the analysis of `db` has the library prefix of `units` units unchanged: no package
/// body that follows the prefix belongs to a package inside it, so that no subprogram or object
/// the prefix declared was completed afterwards.
fn closed(db: &Db, units: usize) -> bool {
    db.units.iter().skip(units).all(|(_, u)| match u.hir {
        UnitHir::PackageBody { package } => yavs_sem::Id::index(package) >= units,
        _ => true,
    })
}

/// A lowerer with the library prefix's packages and subprograms lowered, then the rest of
/// `packages`, for a closure with no entries of its own (`StageCache::closure`); `None` when no prefix
/// serves: the closure is cached itself, coverage counters are on, the `Db` has grown since
/// its analysis, or fewer than [`SIGHTINGS`] closures have asked for the prefix's library yet.
pub(crate) fn lowerer<'a>(
    db: &'a Db,
    packages: &[UnitId],
    opts: LowerOptions,
) -> Option<Result<(Lowerer<'a>, LibUse), Vec<Diagnostic>>> {
    let cache = db
        .stages()
        .filter(|c| !c.closure() && !opts.any_coverage())?;
    if cache.analysed() != db.extent() {
        cache.note(&format!(
            "library prefix: the analysis grew {:?} -> {:?}",
            cache.analysed(),
            db.extent()
        ));
        return None;
    }
    let part = part(&opts);
    for (level, p) in cache.prefixes().iter().enumerate() {
        if !closed(db, p.units) {
            let late: Vec<String> = db
                .units
                .iter()
                .skip(p.units)
                .filter_map(|(_, u)| match u.hir {
                    UnitHir::PackageBody { package } if yavs_sem::Id::index(package) < p.units => {
                        Some(format!("{}.{}", db.libs[u.lib].name.key, u.name.key))
                    }
                    _ => None,
                })
                .collect();
            cache.note(&format!(
                "library prefix: a package body follows the prefix: {late:?}"
            ));
            continue;
        }
        let k = packages
            .iter()
            .take_while(|u| yavs_sem::Id::index(**u) < p.units)
            .count();
        let bytes = match cache.prefix_get(level, LIB, &part) {
            Some(b) => b,
            // Only the longest prefix is made, when enough closures have asked for it (making it
            // costs a third of a second and a run of a closure saves about a twentieth).
            None if level == 0
                && cache.prefix_sightings(level, LIB, &part) >= SIGHTINGS
                && cache.prefix_claim(level, LIB, &part) =>
            {
                let made = make(cache.as_ref(), level, opts);
                let b = match made {
                    Made::Entry(b) => b,
                    // Nothing can be made ahead for this prefix: say so, so that no run tries again.
                    Made::Unusable => Vec::new(),
                    // The prefix's image is gone (pruned): a later run may make it.
                    Made::Transient => {
                        cache.prefix_release(level, LIB, &part);
                        continue;
                    }
                };
                cache.prefix_put(level, LIB, &part, &b);
                cache.prefix_release(level, LIB, &part);
                yavs_diag::Buffer::new(b)
            }
            None => continue,
        };
        let Some((h, image)) = postcard::take_from_bytes::<Header>(&bytes).ok() else {
            continue; // empty: this prefix cannot be lowered ahead
        };
        let ids = || packages[..k].iter().map(|u| yavs_sem::Id::index(*u) as u32);
        if h.packages.len() != k || !h.packages.iter().copied().eq(ids()) {
            cache.note("library prefix: another list of packages");
            continue;
        }
        let Some(mut l) = Lowerer::from_image_in(db, opts, &bytes, image) else {
            cache.note("library prefix: the lowering does not decode");
            continue;
        };
        if l.fingerprint() != h.id {
            cache.note("library prefix: the lowering is not the one that was stored");
            continue;
        }
        for &u in &packages[k..] {
            if let Err(e) = l.package(u) {
                return Some(Err(e));
            }
        }
        let part = [part.as_slice(), &h.id.to_le_bytes()].concat();
        let copies = read_copies(cache.prefix_get(level, OPT, &part));
        let stored = copies.iter().map(|c| c.func.0).collect();
        return Some(Ok((
            l,
            LibUse {
                cache: cache.clone(),
                level,
                part,
                packages: k,
                funcs: h.funcs as usize,
                shared: h.shared,
                copies,
                stored,
            },
        )));
    }
    None
}

/// The optimised copies of an entry, kept in the mapped entry as they are.
fn read_copies(entry: Option<yavs_diag::Buffer>) -> Vec<OptCopy> {
    entry
        .and_then(|b| yavs_diag::sharing(&b, || postcard::from_bytes(&b).ok()))
        .unwrap_or_default()
}

/// Lower every subprogram the packages of `db` declare, except `skip`, with `l`. With `keep_going`
/// one that cannot be lowered ahead of any design is left undefined (the first pass finds out
/// which depend on the design); without it the first is a failure (a note says why).
fn subprograms(
    db: &Db,
    l: &mut Lowerer,
    skip: &HashSet<yavs_sem::DeclId>,
    keep_going: bool,
    cache: &dyn StageCache,
) -> Option<()> {
    for (u, unit) in db.units.iter() {
        if db.libs[unit.lib].name.key.as_str() == "std" {
            continue;
        }
        for s in db.package_regions(u).unwrap_or_default() {
            for d in db.decls_with_instances(s) {
                for m in db.decl_subprograms(d) {
                    if skip.contains(&m) {
                        continue;
                    }
                    if let Err(e) = l.subprogram(m)
                        && !keep_going
                    {
                        cache.note(&format!("library prefix: {}", e[0].message));
                        return None;
                    }
                }
            }
        }
    }
    Some(())
}

/// What [`make`] made.
enum Made {
    Entry(Vec<u8>),
    /// The prefix cannot be lowered ahead of any design.
    Unusable,
    /// It could not be tried (the prefix's image is not there).
    Transient,
}

/// Lower prefix `level` from its own image: its packages' objects, then every subprogram its
/// packages declare that does not depend on the architecture being lowered (a `'path_name` or
/// `'instance_name` is a cell of that architecture's instance data), found by lowering all of
/// them once to see which do. Unusable when something cannot be lowered ahead of any design: an
/// error, an external name, constants over the image limit.
fn make(cache: &dyn StageCache, level: usize, opts: LowerOptions) -> Made {
    let Some(db) = cache.prefix_db(level) else {
        return Made::Transient;
    };
    lowered(cache, &db, opts).map_or(Made::Unusable, Made::Entry)
}

fn lowered(cache: &dyn StageCache, db: &Db, opts: LowerOptions) -> Option<Vec<u8>> {
    let packages = crate::sim::run_time_packages(db);
    let skip: HashSet<yavs_sem::DeclId> = {
        let mut l = Lowerer::new(db, opts);
        for &u in &packages {
            l.package(u).ok()?;
        }
        subprograms(db, &mut l, &HashSet::new(), true, cache)?;
        l.context_dependent().into_iter().collect()
    };
    let mut l = Lowerer::new(db, opts);
    for &u in &packages {
        l.package(u).ok()?;
    }
    let users = l.context_users();
    subprograms(db, &mut l, &skip, false, cache)?;
    if l.context_users() != users {
        cache.note("library prefix: a subprogram depends on the design");
        return None;
    }
    if l.externals_early()
        || l.package_irs()
            .iter()
            .any(|p| !p.ext_consts.is_empty() || !p.ext_vars.is_empty())
    {
        cache.note("library prefix: external names");
        return None;
    }
    let shared = l.shared_size();
    let image = l.image().ok()??;
    if image.len() < MIN_IMAGE {
        return None; // nothing worth reading back
    }
    let (funcs, id) = (l.funcs(), l.fingerprint());
    // The lowering is not needed any more: free it before the entry is copied together.
    drop(l);
    let mut out = postcard::to_stdvec(&Header {
        packages: packages
            .iter()
            .map(|u| yavs_sem::Id::index(*u) as u32)
            .collect(),
        funcs: funcs as u32,
        shared,
        id,
    })
    .ok()?;
    out.extend_from_slice(&image);
    Some(out)
}

impl LibUse {
    /// The optimised copies earlier runs of the prefix made, for `Program::load_optimized`.
    pub(crate) fn take_optimized(&mut self) -> Vec<OptCopy> {
        std::mem::take(&mut self.copies)
    }

    fn entry(&self) -> Vec<OptCopy> {
        read_copies(self.cache.prefix_get(self.level, OPT, &self.part))
    }

    /// Whether the entry lacks the optimised copy of `f`, a function of the prefix.
    pub(crate) fn wants(&self, f: yavs_ir::FuncId) -> bool {
        (f.0 as usize) < self.funcs && !self.stored.contains(&f.0)
    }

    /// Store the optimised copies of library functions this run made (`wants`) with those the
    /// entry holds.
    pub(crate) fn learn(&mut self, new: Vec<OptCopy>) {
        if new.is_empty() {
            return;
        }
        // Another run may have added copies since this one read the entry.
        let mut all: Vec<OptCopy> = self.entry();
        let have: HashSet<u32> = all.iter().map(|c| c.func.0).collect();
        self.stored.extend(have.iter().copied());
        for c in new {
            if have.contains(&c.func.0) {
                continue;
            }
            self.stored.insert(c.func.0);
            all.push(c);
        }
        all.sort_by_key(|c| c.func.0);
        if let Ok(bytes) = postcard::to_stdvec(&all) {
            self.cache.prefix_put(self.level, OPT, &self.part, &bytes);
        }
    }

    /// Where the state of the prefix's packages after their initialisation is kept.
    pub(crate) fn init_entry(&self) -> (Arc<dyn StageCache>, usize, Vec<u8>) {
        (self.cache.clone(), self.level, self.part.clone())
    }
}
