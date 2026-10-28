//! Declarative regions, visibility and lookup (sem.md 1.3, 1.4; LRM 12).
//!
//! Analysis is sequential in textual order, so inserting declarations as they
//! are analyzed gives the LRM scope rule (12.2) without position bookkeeping.

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use smallvec::SmallVec;
use yavs_syntax::Symbol;

use crate::ids::{DeclId, LibId, ScopeId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ScopeKind {
    /// The session root: owns poison and universal entities, never searched by lookup.
    Root,
    /// The context clause region of a design unit (library, use and context items).
    Context,
    Entity,
    Architecture,
    Package,
    PackageBody,
    Configuration,
    Subprogram,
    Process,
    Block,
    Generate,
    Loop,
    Record,
    Protected,
    ProtectedBody,
    Instance,
    Component,
    SeqBlock,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Scope {
    pub kind: ScopeKind,
    /// Lexical parent; a package body's parent is its package, an architecture's its entity.
    pub parent: Option<ScopeId>,
    /// The declaration this region belongs to, for expanded names (`proc_label.v`, `pkg.x`).
    pub owner: Option<DeclId>,
    /// Folded key to declarations, in declaration order.
    pub names: HashMap<Symbol, Vec<DeclId>>,
    /// Declaration order, for cloning and for HIR consumers.
    pub order: Vec<DeclId>,
    /// Use clauses in this region, in order.
    pub uses: Vec<UseItem>,
    /// 2019: index into `order` where `private variable`s start (5.6.2).
    pub private_from: Option<usize>,
    /// A protected body's lexically enclosing region (a package body, say) once
    /// its `parent` became the protected type's declaration region: lookup
    /// continues there after the declaration region (12.3).
    pub outer: Option<ScopeId>,
}

impl Scope {
    pub fn new(kind: ScopeKind, parent: Option<ScopeId>, owner: Option<DeclId>) -> Self {
        Scope {
            kind,
            parent,
            owner,
            names: HashMap::default(),
            order: Vec::new(),
            uses: Vec::new(),
            outer: None,
            private_from: None,
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum UseItem {
    /// `use p.all`: every declaration of the region.
    All(ScopeId),
    /// `use p.x`: these declarations (all overloads of `x`).
    One(Vec<DeclId>),
    /// `use l.all`: the primary units of a library.
    Library(LibId),
}

/// The result of a lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Visible {
    None,
    One(DeclId),
    /// Subprograms and enumeration literals (and aliases of them).
    Overloads(std::sync::Arc<[DeclId]>),
    /// Potentially visible non-overloadable declarations that cancel each other (12.4 c).
    Hidden(Conflict),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub decls: Vec<DeclId>,
}

// ---------------------------------------------------------------------------
// Declaring and looking up.

use yavs_diag::{Diagnostic, Span};
use yavs_syntax::ast::Ident;

use crate::db::Db;
use crate::decl::{Alias, DeclKind};
use crate::diag;
use crate::ids::TypeId;

/// Parameter count, result base type and first parameter base type.
type Shape = (usize, Option<TypeId>, Option<TypeId>);

/// Overloadable declarations seen so far in a lookup, indexed so that "does `d` hide or equal
/// one of them" does not scan all of them (a region can hold hundreds of `"="`).
#[derive(Default)]
struct OverSet {
    entities: Seen,
    shapes: ShapeIndex,
    /// Members that are not poison.
    live: usize,
}

/// The entities a use-clause walk has met: a lookup mostly sees a handful, so they are kept
/// in a small vector and move to a hash set only past [`Seen::SMALL`].
#[derive(Default)]
struct Seen {
    small: SmallVec<[DeclId; 16]>,
    big: HashSet<DeclId>,
}

impl Seen {
    const SMALL: usize = 32;

    /// `true` when `d` was not in the set.
    fn insert(&mut self, d: DeclId) -> bool {
        if self.big.is_empty() {
            if self.small.contains(&d) {
                return false;
            }
            if self.small.len() < Self::SMALL {
                self.small.push(d);
                return true;
            }
            self.big.extend(self.small.drain(..));
        }
        self.big.insert(d)
    }

    fn contains(&self, d: DeclId) -> bool {
        if self.big.is_empty() {
            self.small.contains(&d)
        } else {
            self.big.contains(&d)
        }
    }
}

/// Declarations by profile shape: plain pairs until there are many (a lookup mostly meets a
/// few overloads), then a hash index.
#[derive(Default)]
struct ShapeIndex {
    small: SmallVec<[(Shape, DeclId); 8]>,
    big: HashMap<Shape, Vec<DeclId>>,
}

impl ShapeIndex {
    fn push(&mut self, sh: Shape, d: DeclId) {
        if self.big.is_empty() {
            if self.small.len() < Seen::SMALL {
                self.small.push((sh, d));
                return;
            }
            for (s, x) in self.small.drain(..) {
                self.big.entry(s).or_default().push(x);
            }
        }
        self.big.entry(sh).or_default().push(d);
    }

    /// Does `f` hold for any member, of any shape?
    fn any_member(&self, mut f: impl FnMut(DeclId) -> bool) -> bool {
        if self.big.is_empty() {
            self.small.iter().any(|(_, o)| f(*o))
        } else {
            self.big.values().flatten().any(|&o| f(o))
        }
    }

    /// Does `f` hold for a member of shape `sh`?
    fn any(&self, sh: &Shape, mut f: impl FnMut(DeclId) -> bool) -> bool {
        if self.big.is_empty() {
            self.small.iter().any(|(s, o)| s == sh && f(*o))
        } else {
            self.big.get(sh).is_some_and(|os| os.iter().any(|&o| f(o)))
        }
    }
}

/// Parameter and result type profile (4.5.3): base types only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub params: Vec<TypeId>,
    pub ret: Option<TypeId>,
}

impl Db {
    /// The entity a declaration denotes: non-object aliases are followed.
    pub fn entity(&self, mut d: DeclId) -> DeclId {
        for _ in 0..64 {
            match &self.decls[d].kind {
                DeclKind::Alias(Alias::NonObject { target }) => d = *target,
                _ => return d,
            }
        }
        d
    }

    /// Subprograms and enumeration literals, also through aliases.
    pub fn is_overloadable(&self, d: DeclId) -> bool {
        self.decls[self.entity(d)].kind.is_overloadable()
    }

    /// The profile of an overloadable entity (an enum literal is a parameterless function).
    pub fn profile(&self, d: DeclId) -> Option<Profile> {
        match &self.decls[self.entity(d)].kind {
            DeclKind::Subprogram(s) | DeclKind::GenericSubprogram(s) => Some(Profile {
                params: s.params.iter().map(|p| self.base(p.subtype)).collect(),
                ret: s.ret.map(|r| self.base(r)),
            }),
            DeclKind::EnumLit { ty, .. } => Some(Profile {
                params: Vec::new(),
                ret: Some(*ty),
            }),
            _ => None,
        }
    }

    /// LRM 12.3: same designator (the caller checks), and at most one
    /// overloadable, or both overloadable with the same profile.
    pub(crate) fn homographs(&self, a: DeclId, b: DeclId) -> bool {
        if self.entity(a) == self.entity(b) {
            return false;
        }
        // A failed declaration's poison name conflicts with nothing (8.2).
        if self.is_poison(a) || self.is_poison(b) {
            return false;
        }
        match (self.is_overloadable(a), self.is_overloadable(b)) {
            (true, true) => self.same_profile(a, b),
            _ => true,
        }
    }

    /// Cheap hash key of the profile of an overloadable entity: equal profiles have equal shapes.
    fn profile_shape(&self, d: DeclId) -> Option<Shape> {
        match &self.decls[self.entity(d)].kind {
            DeclKind::Subprogram(s) | DeclKind::GenericSubprogram(s) => Some((
                s.params.len(),
                s.ret.map(|r| self.base(r)),
                s.params.first().map(|p| self.base(p.subtype)),
            )),
            DeclKind::EnumLit { ty, .. } => Some((0, Some(*ty), None)),
            _ => None,
        }
    }

    fn over_insert(&self, set: &mut OverSet, d: DeclId) {
        set.entities.insert(self.entity(d));
        if !self.is_poison(d) {
            set.live += 1;
            if let Some(sh) = self.profile_shape(d) {
                set.shapes.push(sh, d);
            }
        }
    }

    /// `set.iter().any(|o| homographs(o, p) || entity(o) == entity(p))`.
    fn over_hides(&self, set: &OverSet, p: DeclId) -> bool {
        if set.entities.contains(self.entity(p)) {
            return true;
        }
        if self.is_poison(p) || set.live == 0 {
            return false;
        }
        match self.profile_shape(p) {
            // Members are overloadable: homograph iff same profile.
            Some(sh) => set.shapes.any(&sh, |o| self.homographs(o, p)),
            // A non-overloadable declaration conflicts with every other live member.
            None => true,
        }
    }

    /// `profile(a) == profile(b)` without building the profiles (hot in `lookup`).
    pub(crate) fn same_profile(&self, a: DeclId, b: DeclId) -> bool {
        enum P<'a> {
            Sub(&'a crate::decl::Subprogram),
            Lit(TypeId),
        }
        let get = |d: DeclId| match &self.decls[self.entity(d)].kind {
            DeclKind::Subprogram(s) | DeclKind::GenericSubprogram(s) => Some(P::Sub(s)),
            DeclKind::EnumLit { ty, .. } => Some(P::Lit(*ty)),
            _ => None,
        };
        match (get(a), get(b)) {
            (Some(P::Sub(x)), Some(P::Sub(y))) => {
                x.params.len() == y.params.len()
                    && x.ret.map(|r| self.base(r)) == y.ret.map(|r| self.base(r))
                    && x.params
                        .iter()
                        .zip(&y.params)
                        .all(|(p, q)| self.base(p.subtype) == self.base(q.subtype))
            }
            (Some(P::Lit(x)), Some(P::Lit(y))) => x == y,
            (Some(P::Sub(s)), Some(P::Lit(t))) | (Some(P::Lit(t)), Some(P::Sub(s))) => {
                s.params.is_empty() && s.ret.map(|r| self.base(r)) == Some(t)
            }
            (None, None) => true,
            _ => false,
        }
    }

    /// Insert a declaration into a region. A homograph in the same region is
    /// `E1101`, except that an explicit declaration hides an implicit one
    /// (12.3). Completions (bodies, deferred constants, ...) must reuse the
    /// earlier `DeclId` instead of calling this.
    pub(crate) fn declare(&mut self, scope: ScopeId, d: DeclId) {
        let key = self.decls[d].name.key();
        let existing: SmallVec<[DeclId; 4]> = self.scopes[scope]
            .names
            .get(&key)
            .map(|v| v.iter().copied().collect())
            .unwrap_or_default();
        for old in existing {
            if !self.homographs(old, d) {
                continue;
            }
            let (old_implicit, new_implicit) = (self.decls[old].implicit, self.decls[d].implicit);
            if old_implicit && !new_implicit {
                // Explicit hides implicit: the implicit one leaves the region's names.
                if let Some(v) = self.scopes[scope].names.get_mut(&key) {
                    v.retain(|x| *x != old);
                }
                continue;
            }
            if new_implicit && !old_implicit {
                // An implicit op created after an explicit homograph stays hidden.
                self.scopes[scope].order.push(d);
                return;
            }
            let name = self.decls[d].name;
            let (span, old_span) = (self.decls[d].name_span(), self.decls[old].name_span());
            self.report(
                Diagnostic::error(
                    diag::REDECLARED,
                    format!("`{name}` is declared twice in this region"),
                )
                .with_primary(span, "redeclared here")
                .with_secondary(old_span, "first declared here"),
            );
            return;
        }
        if self.entity_homograph(scope, d) {
            return;
        }
        // The region gains a declaration of `key`: the answers for that name change, no other.
        self.decls.bump_key(key.index());
        let s = self.scopes.quiet_mut(scope);
        s.names.entry(key).or_default().push(d);
        s.order.push(d);
    }

    /// An entity and its architecture (12.1), a package and its body (4.8) are one
    /// declarative region: a declaration in the architecture or body may not be a
    /// homograph of one in the entity or package (`E1101`). A deferred constant of
    /// the package is completed by the body's full declaration. Reports and
    /// returns whether it did.
    fn entity_homograph(&mut self, scope: ScopeId, d: DeclId) -> bool {
        let (head_kind, is_body) = match self.scopes[scope].kind {
            ScopeKind::Architecture => (ScopeKind::Entity, false),
            ScopeKind::PackageBody => (ScopeKind::Package, true),
            _ => return false,
        };
        if self.decls[d].implicit {
            return false;
        }
        let head = if is_body {
            self.body_package(scope)
        } else {
            self.scopes[scope]
                .parent
                .and_then(|c| self.scopes[c].parent)
        };
        let Some(entity) = head.filter(|&e| self.scopes[e].kind == head_kind) else {
            return false;
        };
        let key = self.decls[d].name.key();
        let Some(old) = self.scopes[entity]
            .names
            .get(&key)
            .and_then(|v| {
                v.iter().copied().find(|&o| {
                    self.homographs(o, d)
                        && !self.decls[o].implicit
                        && !(is_body
                            && matches!(&self.decls[o].kind, DeclKind::Object(x) if x.role == crate::decl::ObjRole::Deferred))
                })
            })
        else {
            return false;
        };
        let head = if is_body { "package" } else { "entity" };
        let what = match &self.decls[old].kind {
            DeclKind::Object(o) if o.role == crate::decl::ObjRole::Port => "a port",
            DeclKind::Object(o) if o.role == crate::decl::ObjRole::Generic => "a generic",
            _ => "a declaration",
        };
        let ename = self.scopes[entity]
            .owner
            .map(|o| self.decls[o].name.to_string())
            .unwrap_or_default();
        let name = self.decls[d].name;
        let (span, old_span) = (self.decls[d].span, self.decls[old].span);
        self.report(
            Diagnostic::error(
                diag::REDECLARED,
                format!("`{name}` is already declared as {what} of {head} `{ename}`"),
            )
            .with_primary(span, "redeclared here")
            .with_secondary(old_span, format!("declared in the {head} here"))
            .with_note(if is_body {
                "a package and its body form one declarative region (LRM 4.8, 12.1)"
            } else {
                "an entity and its architectures form one declarative region (LRM 12.1)"
            }),
        );
        true
    }

    /// The next enclosing region for visibility: a protected body (or a nested package
    /// body in its outer package's body) continues with its declaration region, then with
    /// the region the body is written in.
    fn next_region(&self, s: ScopeId, outer: &mut Option<ScopeId>) -> Option<ScopeId> {
        let sc = &self.scopes[s];
        if matches!(sc.kind, ScopeKind::ProtectedBody | ScopeKind::PackageBody)
            && sc.outer.is_some()
        {
            *outer = sc.outer;
        } else if matches!(sc.kind, ScopeKind::Protected | ScopeKind::Package) && outer.is_some() {
            return outer.take();
        }
        sc.parent
    }

    /// Counts the mutable accesses of the tracked arenas: a lookup result is valid while it
    /// is unchanged.
    fn cache_epoch(&self) -> u64 {
        self.scopes.version() + self.decls.version() + self.subtypes.version()
    }

    /// Visibility (sem.md 1.4, LRM 12.3 and 12.4).
    pub fn lookup(&mut self, scope: ScopeId, key: Symbol) -> Visible {
        self.lookup_full(scope, key).0
    }

    /// `lookup` with the "direct hit" and "cacheable" flags. A region that declares no `key` and
    /// has no use clauses and continues with its parent (most subprogram, process and block
    /// regions) has its parent's answer: that is asked (and cached) instead of walking the
    /// chain again for every new inner region.
    fn lookup_full(&mut self, scope: ScopeId, key: Symbol) -> (Visible, bool, bool) {
        let (rev, epoch) = (self.file_rev(), self.cache_epoch());
        let kgen = self.decls.key_gen(key.index());
        if let Some((r, e, g, direct, v)) = self.lookup_cache.get(&(scope, key))
            && *r == rev
            && *e == epoch
            && *g == kgen
        {
            let (v, direct) = (v.clone(), *direct);
            #[cfg(debug_assertions)]
            {
                let (mut cacheable, mut d) = (false, false);
                let fresh = self.lookup_uncached(scope, key, &mut cacheable, &mut d);
                assert_eq!(d, direct);
                assert_eq!(fresh, v, "stale lookup cache entry for `{key}`");
            }
            if let (true, Visible::One(d)) = (direct, &v)
                && !self.generic.unused.is_empty()
            {
                // Same side effect as an uncached direct hit.
                self.generic.unused.remove(d);
            }
            return (v, direct, true);
        }
        let (mut cacheable, mut direct) = (true, false);
        let sc = &self.scopes[scope];
        let v = match sc.parent {
            Some(p) if sc.outer.is_none() && sc.uses.is_empty() && !sc.names.contains_key(&key) => {
                let (v, d, c) = self.lookup_full(p, key);
                (direct, cacheable) = (d, c);
                v
            }
            _ => self.lookup_uncached(scope, key, &mut cacheable, &mut direct),
        };
        // A name declared in `scope` itself is as cheap to find again as its cache entry is to
        // probe: only results from further out are kept. Something changed during the lookup
        // (a unit was loaded): do not trust the epoch.
        let own = matches!(&v, Visible::One(d) if direct && self.decls[*d].scope == scope);
        if cacheable
            && !own
            && self.cache_epoch() == epoch
            && self.decls.key_gen(key.index()) == kgen
        {
            self.lookup_cache
                .insert((scope, key), (rev, epoch, kgen, direct, v.clone()));
        }
        (v, direct, cacheable)
    }

    fn lookup_uncached(
        &mut self,
        scope: ScopeId,
        key: Symbol,
        cacheable: &mut bool,
        direct: &mut bool,
    ) -> Visible {
        // 1. Direct visibility, innermost region first.
        let mut over: SmallVec<[DeclId; 8]> = SmallVec::new();
        let mut over_set = OverSet::default();
        let mut single: Option<DeclId> = None;
        let mut regions: SmallVec<[ScopeId; 8]> = SmallVec::new();
        let mut cur = Some(scope);
        let mut outer = None;
        'walk: while let Some(s) = cur {
            regions.push(s);
            if let Some(ds) = self.scopes[s].names.get(&key) {
                for &d in ds {
                    if self.is_overloadable(d) {
                        if !self.over_hides(&over_set, d) {
                            self.over_insert(&mut over_set, d);
                            over.push(d);
                        }
                    } else if over.is_empty() {
                        single = Some(d);
                        break 'walk;
                    } else {
                        // Hidden by the inner overloadables; outer regions are hidden too.
                        break 'walk;
                    }
                }
                if !over.is_empty() && ds.iter().any(|&d| !self.is_overloadable(d)) {
                    break;
                }
            }
            cur = self.next_region(s, &mut outer);
        }
        if let Some(d) = single {
            self.generic.unused.remove(&d);
            *direct = true;
            return Visible::One(d);
        }
        // Keep walking parents for use clauses even when overloadables were found.
        while let Some(s) = cur {
            if !regions.contains(&s) {
                regions.push(s);
            }
            cur = self.next_region(s, &mut outer);
        }

        // 2. Potential visibility through use clauses of every enclosing region.
        let mut pot: SmallVec<[DeclId; 8]> = SmallVec::new();
        let mut seen = Seen::default();
        for &s in &regions {
            for i in 0..self.scopes[s].uses.len() {
                match &self.scopes[s].uses[i] {
                    UseItem::All(p) => {
                        for &d in self.scopes[*p].names.get(&key).into_iter().flatten() {
                            if seen.insert(self.entity(d)) {
                                pot.push(d);
                            }
                        }
                    }
                    UseItem::One(ds) => {
                        for &d in ds.iter().filter(|&&d| self.decls[d].name.key() == key) {
                            if seen.insert(self.entity(d)) {
                                pot.push(d);
                            }
                        }
                    }
                    UseItem::Library(lib) => {
                        let lib = *lib;
                        *cacheable = false;
                        if let Some(d) = self.library_unit_decl(lib, key)
                            && seen.insert(self.entity(d))
                        {
                            pot.push(d);
                        }
                    }
                }
            }
        }
        // sem.md 0.1: a 2008 file does not see std/ieee declarations new in 2019 (no new
        // conflicts or overloads); if nothing else is visible they come back for `E1007`.
        let rev = self.file_rev();
        if self.rev > rev && pot.iter().any(|&d| self.decls[d].since > rev) {
            let (newer, old): (SmallVec<[DeclId; 8]>, SmallVec<[DeclId; 8]>) =
                pot.into_iter().partition(|&d| self.decls[d].since > rev);
            if old.is_empty() && over.is_empty() {
                return match newer[..] {
                    [d] if !self.is_overloadable(d) => Visible::One(d),
                    _ => Visible::Overloads(newer[..].into()),
                };
            }
            pot = old;
        }
        // (a) a directly visible homograph hides the potentially visible declaration.
        if !over.is_empty() {
            pot.retain(|p| !self.over_hides(&over_set, *p));
        }
        // (b) of two potentially visible homographs, explicit beats implicit. Explicit
        // overloadables are bucketed by profile shape, so only equal shapes are compared.
        if pot.iter().any(|&p| self.decls[p].implicit) {
            let mut by_shape = ShapeIndex::default();
            let mut explicit_other: SmallVec<[DeclId; 4]> = SmallVec::new();
            for &q in &pot {
                if self.decls[q].implicit || self.is_poison(q) {
                    continue;
                }
                match self.profile_shape(q) {
                    Some(sh) => by_shape.push(sh, q),
                    None => explicit_other.push(q),
                }
            }
            pot.retain(|p| {
                let p = *p;
                if !self.decls[p].implicit || self.is_poison(p) {
                    return true;
                }
                let hidden = match self.profile_shape(p) {
                    // An implicit subprogram or literal is hidden only by an
                    // explicit homograph subprogram, not by a non-overloadable:
                    // a constant and a subprogram of one designator are not made
                    // directly visible at all (12.4, the conflict below, #628).
                    Some(sh) => by_shape.any(&sh, |q| self.homographs(p, q)),
                    None => {
                        explicit_other.iter().any(|&q| self.homographs(p, q))
                            || by_shape.any_member(|q| self.homographs(p, q))
                    }
                };
                !hidden
            });
        }
        let (pot_over, pot_single): (SmallVec<[DeclId; 8]>, SmallVec<[DeclId; 8]>) =
            pot.into_iter().partition(|&d| self.is_overloadable(d));
        // (c) non-overloadable conflicts make all of them invisible.
        if !pot_single.is_empty() {
            if pot_single.len() == 1 && pot_over.is_empty() && over.is_empty() {
                return Visible::One(pot_single[0]);
            }
            let mut decls = pot_single.into_vec();
            decls.extend(pot_over);
            return Visible::Hidden(Conflict { decls });
        }
        over.extend(pot_over);
        match over.len() {
            0 => Visible::None,
            _ => Visible::Overloads(over[..].into()),
        }
    }

    /// The `DeclKind::Unit` of primary unit `key` in `lib`, loading it (for `use lib.all`).
    fn library_unit_decl(&mut self, lib: LibId, key: Symbol) -> Option<DeclId> {
        let unit = crate::loader::UnitKey::Primary(key);
        // Most names are no unit of the library: that is no error (10.4).
        if !self.unit_known(lib, unit) {
            return None;
        }
        let u = self.load_unit(lib, unit, None).ok()?;
        Some(self.units[u].decl)
    }

    /// Report an undeclared name once per (unit, key), with suggestions (8.3).
    pub(crate) fn undeclared(&mut self, scope: ScopeId, name: Ident) -> crate::Reported {
        let unit = self.cur().map(|c| c.unit);
        if unit.is_some_and(|u| self.tainted.contains(&u))
            || !self.undeclared.insert((unit, name.key))
        {
            return crate::Reported;
        }
        let mut d = Diagnostic::error(
            diag::NOT_DECLARED,
            format!("`{}` is not declared", name.text),
        )
        .with_primary(name.span, "not found in this scope");
        // A package that declares exactly this name beats a near miss.
        let pkgs = self.packages_declaring(scope, name.key);
        if pkgs.is_empty() {
            let cands = self.visible_decls(scope);
            let found = crate::suggest::similar(
                name.key.as_str(),
                cands.iter().filter_map(|&d| self.candidate(d)),
            );
            d = self.suggest_decls(d, name.span, &found);
        }
        for p in pkgs {
            d = d.with_help(format!(
                "`{}` is declared in `{p}`; add `use {p}.all;`",
                name.text
            ));
        }
        self.error(d)
    }

    /// Analyzed packages of the libraries visible from `scope` that declare `key`, as
    /// `lib.pkg`: the use clause an undeclared name probably misses. With `ieee` visible
    /// the packages of `ieee.ieee_std_context` are loaded for this first.
    fn packages_declaring(&mut self, scope: ScopeId, key: Symbol) -> Vec<String> {
        let libs: Vec<LibId> = self.libs.iter().map(|(l, _)| l).collect();
        let visible: Vec<LibId> = libs
            .into_iter()
            .filter(|&l| {
                let name = self.libs[l].name.key;
                matches!(self.lookup(scope, name), Visible::One(d)
                    if matches!(self.decls[d].kind, crate::decl::DeclKind::Library(x) if x == l))
            })
            .collect();
        let ctx = crate::loader::UnitKey::Primary(Symbol::intern("ieee_std_context"));
        if let Some(&l) = visible
            .iter()
            .find(|&&l| self.libs[l].name.key.as_str() == "ieee")
            && self.cur().is_none_or(|c| self.units[c.unit].lib != l)
            && self.locate(l, ctx).is_some()
        {
            let _ = self.load_unit(l, ctx, None);
        }
        let mut out: Vec<String> = self
            .units
            .iter()
            .filter(|(_, u)| {
                u.kind == crate::loader::UnitKind::Package
                    && visible.contains(&u.lib)
                    && self.scopes[u.scope]
                        .names
                        .get(&key)
                        .is_some_and(|v| !v.is_empty())
            })
            // Folded keys: the vendored sources spell some packages in upper case.
            .map(|(_, u)| format!("{}.{}", self.libs[u.lib].name.key, u.name.key))
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Report a use-clause conflict (12.4 c).
    pub(crate) fn conflict(&mut self, name: Ident, c: &Conflict) -> crate::Reported {
        let mut d = Diagnostic::error(
            diag::USE_CONFLICT,
            format!(
                "`{}` is ambiguous: several use clauses make it visible",
                name.text
            ),
        )
        .with_primary(name.span, "used here");
        for &x in &c.decls {
            let scope = self.decls[x].scope;
            let owner = self.scopes[scope]
                .owner
                .map(|o| self.decls[o].name.to_string())
                .unwrap_or_default();
            d = d.with_secondary(self.decls[x].span, format!("declared in `{owner}`"));
        }
        self.error(d.with_help("use a selected name such as `pkg.name`"))
    }

    /// One declaration per name visible from `scope` (direct and potential), innermost first.
    fn visible_decls(&self, scope: ScopeId) -> Vec<DeclId> {
        let mut out: Vec<(Symbol, DeclId)> = Vec::new();
        let mut cur = Some(scope);
        let mut outer = None;
        while let Some(s) = cur {
            out.extend(
                self.scopes[s]
                    .names
                    .iter()
                    .filter_map(|(&k, v)| Some((k, *v.first()?))),
            );
            for u in &self.scopes[s].uses {
                match u {
                    UseItem::All(p) => out.extend(
                        self.scopes[*p]
                            .names
                            .iter()
                            .filter_map(|(&k, v)| Some((k, *v.first()?))),
                    ),
                    UseItem::One(ds) => {
                        out.extend(ds.iter().map(|&d| (self.decls[d].name.key(), d)))
                    }
                    UseItem::Library(_) => {}
                }
            }
            cur = self.next_region(s, &mut outer);
        }
        // Stable sort: the innermost of equal keys stays first.
        out.sort_by_key(|&(k, _)| k);
        out.dedup_by_key(|&mut (k, _)| k);
        out.into_iter().map(|(_, d)| d).collect()
    }

    /// Span helper for diagnostics about a declaration.
    pub fn decl_span(&self, d: DeclId) -> Span {
        self.decls[d].span
    }
}
