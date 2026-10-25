//! Typed 32-bit ids and the arenas they index (sem.md 1.1).
//!
//! `Option<Id>` is free: ids hold a `NonZeroU32` (index + 1).

use std::fmt;
use std::marker::PhantomData;
use std::num::NonZeroU32;

/// An arena index.
pub trait Id: Copy + Eq {
    fn from_index(i: usize) -> Self;
    fn index(self) -> usize;
}

macro_rules! id {
    ($($name:ident = $kind:ident),* $(,)?) => {$(
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(NonZeroU32);

        impl Id for $name {
            fn from_index(i: usize) -> Self {
                // More than 4 billion entities in one session is an invariant violation.
                let n = u32::try_from(i + 1).expect(concat!(stringify!($name), " arena overflow"));
                $name(NonZeroU32::new(n).expect("index + 1 is non-zero"))
            }
            fn index(self) -> usize {
                (self.0.get() - 1) as usize
            }
        }

        impl IdKind for $name {
            const KIND: Kind = Kind::$kind;
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), self.index())
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                ser_id(*self, s)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                de_id(d)
            }
        }
    )*};
}

/// The arena an id indexes, for [`Reloc`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Lib,
    Unit,
    Decl,
    Scope,
    Type,
    Subtype,
    Expr,
    Stmt,
    Const,
    Inst,
    View,
    External,
    AttrValue,
}

/// The number of [`Kind`]s.
pub const KINDS: usize = 13;

pub trait IdKind: Id {
    const KIND: Kind;
}

/// How ids are written and read while a unit of the analysed-unit cache is (`units.rs`): as an
/// owner (an index into the entry's table of the units it refers to) and a position among that
/// owner's entries, so that they mean the same in a session where the arenas are filled in
/// another order.
pub trait Reloc {
    fn enc(&self, kind: Kind, index: u32) -> Option<(u32, u32)>;
    fn dec(&self, kind: Kind, owner: u32, offset: u32) -> Option<u32>;
}

thread_local! {
    static RELOC: std::cell::RefCell<Option<std::rc::Rc<dyn Reloc>>> =
        const { std::cell::RefCell::new(None) };
}

/// Run `f` with ids written and read through `r`.
pub fn with_reloc<R>(r: std::rc::Rc<dyn Reloc>, f: impl FnOnce() -> R) -> R {
    let before = RELOC.with(|c| c.borrow_mut().replace(r));
    let out = f();
    RELOC.with(|c| *c.borrow_mut() = before);
    out
}

/// Run `f` with ids and file ids written and read as they are (no [`Reloc`], no file map).
pub(crate) fn plain<R>(f: impl FnOnce() -> R) -> R {
    let before = RELOC.with(|c| c.borrow_mut().take());
    let out = yavs_diag::unmapped_file_ids(f);
    RELOC.with(|c| *c.borrow_mut() = before);
    out
}

fn reloc_on() -> bool {
    RELOC.with(|c| c.borrow().is_some())
}

fn reloc_enc(kind: Kind, i: u32) -> Option<(u32, u32)> {
    RELOC.with(|c| c.borrow().as_ref()?.enc(kind, i))
}

fn reloc_dec(kind: Kind, owner: u32, offset: u32) -> Option<u32> {
    RELOC.with(|c| c.borrow().as_ref()?.dec(kind, owner, offset))
}

fn ser_id<I: IdKind, S: serde::Serializer>(id: I, s: S) -> Result<S::Ok, S::Error> {
    use serde::Serialize;
    let i = id.index() as u32;
    if !reloc_on() {
        return s.serialize_newtype_struct("Id", &(i + 1));
    }
    match reloc_enc(I::KIND, i) {
        Some(pair) => pair.serialize(s),
        None => Err(serde::ser::Error::custom("id outside the unit's owners")),
    }
}

fn de_id<'de, I: IdKind, D: serde::Deserializer<'de>>(d: D) -> Result<I, D::Error> {
    use serde::Deserialize;
    use serde::de::Error;
    match reloc_on() {
        false => {
            struct V;
            impl<'de> serde::de::Visitor<'de> for V {
                type Value = u32;
                fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    f.write_str("an id")
                }
                fn visit_newtype_struct<D: serde::Deserializer<'de>>(
                    self,
                    d: D,
                ) -> Result<u32, D::Error> {
                    u32::deserialize(d)
                }
            }
            let n = d.deserialize_newtype_struct("Id", V)?;
            if n == 0 {
                return Err(D::Error::custom("id 0"));
            }
            Ok(I::from_index(n as usize - 1))
        }
        true => {
            let (owner, offset) = <(u32, u32)>::deserialize(d)?;
            reloc_dec(I::KIND, owner, offset)
                .map(|i| I::from_index(i as usize))
                .ok_or_else(|| D::Error::custom("id outside the unit's owners"))
        }
    }
}

id!(
    LibId = Lib,
    UnitId = Unit,
    DeclId = Decl,
    ScopeId = Scope,
    TypeId = Type,
    SubtypeId = Subtype,
    ExprId = Expr,
    StmtId = Stmt,
    ConstId = Const,
    InstId = Inst,
    ViewId = View,
    ExternalId = External,
    AttrValueId = AttrValue,
);

/// A growable vector indexed by a typed id. Entries are never removed.
///
/// Entries live in pages of [`PAGE`]. A page restored from an analysis image or read from the
/// analysed-unit cache may stay bytes until one of its entries is read (`docs/decisions.md`
/// "Compilation cache", "Analysed-unit cache"): a run that never looks at an entry never pays for
/// decoding it.
pub struct Arena<I, T> {
    /// Every page but the last holds [`PAGE`] entries.
    pages: Vec<Page<T>>,
    len: usize,
    /// A tracked arena counts every mutable access in `version` (visibility caches).
    track: bool,
    /// Mutable accesses so far; not serialized.
    version: u64,
    /// Per name key (`Symbol::index`) of a declaration arena: the mutable accesses of the
    /// declarations with that name; not serialized. A lookup of a key depends on the
    /// declarations of that key only (and on `version`), so a write to another name leaves its
    /// cached answer valid.
    key_gen: Vec<u32>,
    /// Who the next `push` belongs to (`units.rs`: a load of a unit, or [`SESSION`]).
    owner: u32,
    /// The owner of every entry: `(first index, owner)` at each change of owner.
    runs: Vec<(u32, u32)>,
    /// What the unit being recorded writes below `Watch::floor` (`units.rs`).
    watch: Option<Box<Watch<T>>>,
    _id: PhantomData<I>,
}

/// The owner of the entries no unit made: those of `Db::new` and the libraries.
pub const SESSION: u32 = u32::MAX;

/// The entries below `floor` that were written through `IndexMut` while a unit was recorded, each
/// with its bytes before the first write (`snap`), for the analysed-unit cache.
pub(crate) struct Watch<T> {
    pub floor: usize,
    snap: fn(&T) -> Vec<u8>,
    seen: rustc_hash::FxHashSet<u32>,
    pub touched: Vec<(u32, Vec<u8>)>,
}

/// The bytes of a value that compare equal exactly when the values do, within this process.
pub(crate) fn raw_bytes<T: serde::Serialize>(t: &T) -> Vec<u8> {
    plain(|| yavs_syntax::raw_symbols(|| postcard::to_stdvec(t)))
        .unwrap_or_else(|e| panic!("internal error: cannot write a value: {e}"))
}

/// Entries per page.
pub const PAGE: usize = 64;

/// A file id map that a lazily decoded page keeps (`yavs_diag::map_file_ids`).
pub(crate) type SharedFileMap =
    std::sync::Arc<dyn Fn(yavs_diag::FileId) -> Option<yavs_diag::FileId> + Send + Sync>;

/// What a page of bytes is decoded under: the symbol table of its image or entry, and for the
/// analysed-unit cache the relocation of its ids and files.
#[derive(Clone)]
pub(crate) struct PageCtx {
    pub syms: Option<yavs_syntax::DecodeTable>,
    pub reloc: Option<std::sync::Arc<dyn Reloc + Send + Sync>>,
    pub files: Option<SharedFileMap>,
}

struct ArcReloc(std::sync::Arc<dyn Reloc + Send + Sync>);

impl Reloc for ArcReloc {
    fn enc(&self, kind: Kind, index: u32) -> Option<(u32, u32)> {
        self.0.enc(kind, index)
    }
    fn dec(&self, kind: Kind, owner: u32, offset: u32) -> Option<u32> {
        self.0.dec(kind, owner, offset)
    }
}

thread_local! {
    /// The `Rc` forms of the last `Reloc` and file map a [`PageCtx`] ran under: entries are read
    /// one at a time, mostly from the same unit, so wrapping them afresh for each was two
    /// allocations per entry.
    static WRAPPED: std::cell::RefCell<Wrapped> = const { std::cell::RefCell::new(Wrapped { reloc: None, files: None }) };
}

#[allow(clippy::type_complexity)]
struct Wrapped {
    reloc: Option<(
        std::sync::Arc<dyn Reloc + Send + Sync>,
        std::rc::Rc<dyn Reloc>,
    )>,
    files: Option<(SharedFileMap, yavs_diag::FileIdMap)>,
}

impl PageCtx {
    /// Run `f` with this context set, whatever the caller had set (a page may be read while a
    /// unit of the analysed-unit cache is written or read).
    pub(crate) fn run<R>(&self, f: impl FnOnce() -> R) -> R {
        let (reloc, files) = WRAPPED.with(|w| {
            let mut w = w.borrow_mut();
            let reloc = self.reloc.as_ref().map(|r| match &w.reloc {
                Some((a, rc))
                    if std::ptr::addr_eq(std::sync::Arc::as_ptr(a), std::sync::Arc::as_ptr(r)) =>
                {
                    rc.clone()
                }
                _ => {
                    let rc: std::rc::Rc<dyn Reloc> = std::rc::Rc::new(ArcReloc(r.clone()));
                    w.reloc = Some((r.clone(), rc.clone()));
                    rc
                }
            });
            let files = self.files.as_ref().map(|m| match &w.files {
                Some((a, rc))
                    if std::ptr::addr_eq(std::sync::Arc::as_ptr(a), std::sync::Arc::as_ptr(m)) =>
                {
                    rc.clone()
                }
                _ => {
                    let m2 = m.clone();
                    let rc: yavs_diag::FileIdMap = std::rc::Rc::new(move |id| m2(id));
                    w.files = Some((m.clone(), rc.clone()));
                    rc
                }
            });
            (reloc, files)
        });
        plain(|| {
            let with_syms = || match &self.syms {
                Some(t) => t.run(f),
                None => f(),
            };
            let with_files = || match files {
                Some(m) => yavs_diag::map_file_ids(m, with_syms),
                None => with_syms(),
            };
            match reloc {
                Some(r) => with_reloc(r, with_files),
                None => with_files(),
            }
        })
    }
}

enum Page<T> {
    Eager(Vec<T>),
    Lazy(Box<LazyPage<T>>),
    Slots(Box<SlotPage<T>>),
}

struct LazyPage<T> {
    n: usize,
    data: yavs_diag::Slice,
    ctx: PageCtx,
    cell: std::sync::OnceLock<Vec<T>>,
    /// Decodes `n` entries (monomorphic: made where `T: Deserialize` is known).
    decode: fn(&[u8], usize) -> Vec<T>,
}

impl<T> LazyPage<T> {
    fn get(&self) -> &Vec<T> {
        self.cell
            .get_or_init(|| self.ctx.run(|| (self.decode)(self.data.get(), self.n)))
    }

    fn get_mut(&mut self) -> &mut Vec<T> {
        self.get();
        self.cell.get_mut().expect("page decoded just now")
    }

    fn take(mut self) -> Vec<T> {
        self.get();
        self.cell.take().expect("page decoded just now")
    }
}

/// The bytes of the entries of one replayed run, and how to read one of them.
struct Src<T> {
    data: yavs_diag::Slice,
    ctx: PageCtx,
    /// Decodes the entry at the start of the bytes.
    decode: fn(&[u8]) -> T,
    /// How many entries a read decodes at once (the one read and those after it of the same
    /// bytes): 1 for entries that are read here and there, more for those read in runs.
    window: usize,
}

/// Where the bytes of an entry of a [`SlotPage`] are: index into the page's `srcs` and offset into
/// that source's bytes.
struct Slot {
    src: u32,
    off: u32,
}

/// Up to [`PAGE`] entries that stay bytes one by one until read: what the replay of a unit of the
/// analysed-unit cache pushes (a package's thousand declarations are read a tenth at a time). The
/// decoded entries live in one box of cells, made when the page is first read (a box per entry
/// cost an allocation and a free each, #665).
struct SlotPage<T> {
    slots: Vec<Slot>,
    cells: std::sync::OnceLock<Box<[std::sync::OnceLock<T>]>>,
    srcs: Vec<std::sync::Arc<Src<T>>>,
    /// The end of the last run of entries a read decoded, and its length (`SlotPage::fill`).
    last: std::sync::atomic::AtomicU32,
}

impl<T> SlotPage<T> {
    fn new() -> Self {
        SlotPage {
            slots: Vec::with_capacity(PAGE),
            cells: std::sync::OnceLock::new(),
            srcs: Vec::new(),
            last: std::sync::atomic::AtomicU32::new(0),
        }
    }

    fn from_eager(v: Vec<T>) -> Self {
        let mut p = SlotPage::new();
        for item in v {
            p.push(item);
        }
        p
    }

    fn cells(&self) -> &[std::sync::OnceLock<T>] {
        self.cells
            .get_or_init(|| (0..PAGE).map(|_| std::sync::OnceLock::new()).collect())
    }

    fn push(&mut self, item: T) {
        let _ = self.cells()[self.slots.len()].set(item);
        self.slots.push(Slot { src: 0, off: 0 });
    }

    fn get(&self, j: usize) -> &T {
        match self.cells.get().and_then(|c| c[j].get()) {
            Some(v) => v,
            None => self.fill(j),
        }
    }

    /// Decode entry `j` and the entries after it that come from the same bytes: `Src::window` of
    /// them, or, for entries read here and there (window 1), twice as many as the last run while
    /// reads follow each other directly (a lowering reads the declarations of a package in order,
    /// an analysis a few of them here and there), up to the page: the context of a decode costs
    /// as much as a small entry (#665). Windows that also took reads a few entries on cost cold
    /// analysis more than they gave elaboration.
    #[cold]
    fn fill(&self, j: usize) -> &T {
        use std::sync::atomic::Ordering::Relaxed;
        let s = &self.slots[j];
        let src = &self.srcs[s.src as usize];
        let cells = self.cells();
        let window = if src.window > 1 {
            src.window
        } else {
            let last = self.last.load(Relaxed);
            let (end, len) = ((last >> 16) as usize, (last & 0xffff) as usize);
            if len > 0 && j == end {
                (2 * len).min(PAGE)
            } else {
                1
            }
        };
        let mut done = 0;
        src.ctx.run(|| {
            for (k, t) in self.slots[j..].iter().enumerate() {
                if k >= window || t.src != s.src || (k > 0 && cells[j + k].get().is_some()) {
                    break;
                }
                let v = (src.decode)(&src.data.get()[t.off as usize..]);
                // Another thread may have decoded it meanwhile: both are the same entry.
                let _ = cells[j + k].set(v);
                done = k + 1;
            }
        });
        if src.window == 1 {
            self.last.store(((j + done) << 16 | done) as u32, Relaxed);
        }
        cells[j].get().expect("entry decoded just now")
    }

    fn get_mut(&mut self, j: usize) -> &mut T {
        self.get(j);
        self.cells
            .get_mut()
            .and_then(|c| c[j].get_mut())
            .expect("entry decoded just now")
    }

    /// Drop the entries from `len` on.
    fn truncate(&mut self, len: usize) {
        if let Some(c) = self.cells.get_mut() {
            for cell in &mut c[len.min(self.slots.len())..self.slots.len()] {
                cell.take();
            }
        }
        self.slots.truncate(len);
    }

    /// Decode everything and hand the entries over.
    fn take(self) -> Vec<T> {
        for j in 0..self.slots.len() {
            self.get(j);
        }
        let n = self.slots.len();
        self.cells
            .into_inner()
            .map(|c| {
                c.into_vec()
                    .into_iter()
                    .take(n)
                    .map(|c| c.into_inner().expect("entry decoded just now"))
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl<T> Page<T> {
    #[inline]
    fn at(&self, j: usize) -> &T {
        match self {
            Page::Eager(v) => &v[j],
            Page::Lazy(l) => &l.get()[j],
            Page::Slots(s) => s.get(j),
        }
    }

    #[inline]
    fn at_mut(&mut self, j: usize) -> &mut T {
        match self {
            Page::Eager(v) => &mut v[j],
            Page::Lazy(l) => &mut l.get_mut()[j],
            Page::Slots(s) => s.get_mut(j),
        }
    }

    /// Decode the page for good (it is about to shrink).
    fn make_eager(&mut self) -> &mut Vec<T> {
        if !matches!(self, Page::Eager(_)) {
            let v = match std::mem::replace(self, Page::Eager(Vec::new())) {
                Page::Eager(v) => v,
                Page::Lazy(l) => l.take(),
                Page::Slots(s) => s.take(),
            };
            *self = Page::Eager(v);
        }
        match self {
            Page::Eager(v) => v,
            _ => unreachable!("made eager above"),
        }
    }

    /// Append an entry that is already a value.
    fn push(&mut self, item: T) {
        match self {
            Page::Eager(v) => v.push(item),
            Page::Slots(s) => s.push(item),
            Page::Lazy(_) => self.make_eager().push(item),
        }
    }

    #[cfg(test)]
    fn undecoded(&self) -> usize {
        match self {
            Page::Eager(_) => 0,
            Page::Lazy(l) => usize::from(l.cell.get().is_none()),
            Page::Slots(s) => (0..s.slots.len())
                .filter(|&j| s.cells.get().is_none_or(|c| c[j].get().is_none()))
                .count(),
        }
    }
}

pub(crate) fn decode_entries<T: serde::de::DeserializeOwned>(mut rest: &[u8], n: usize) -> Vec<T> {
    (0..n)
        .map(|_| {
            let (t, r) = postcard::take_from_bytes::<T>(rest)
                .unwrap_or_else(|e| panic!("internal error: corrupt page of entries: {e}"));
            rest = r;
            t
        })
        .collect()
}

fn decode_one<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> T {
    postcard::take_from_bytes::<T>(bytes)
        .unwrap_or_else(|e| panic!("internal error: corrupt entry: {e}"))
        .0
}

/// Write `n` as a varint (7 bits at a time, low bits first, the top bit says more follow).
pub(crate) fn put_varint(out: &mut Vec<u8>, mut n: u32) {
    while n >= 0x80 {
        out.push((n & 0x7f) as u8 | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

/// Read a varint written by [`put_varint`] from the front of `b`.
pub(crate) fn take_varint(b: &mut &[u8]) -> Option<u32> {
    let mut n = 0u32;
    for shift in (0..35).step_by(7) {
        let (&x, rest) = b.split_first()?;
        *b = rest;
        n |= u32::from(x & 0x7f) << shift;
        if x & 0x80 == 0 {
            return Some(n);
        }
    }
    None
}

impl<I: Id, T> Arena<I, T> {
    pub fn new() -> Self {
        Arena {
            pages: Vec::new(),
            len: 0,
            track: false,
            version: 0,
            key_gen: Vec::new(),
            owner: SESSION,
            runs: Vec::new(),
            watch: None,
            _id: PhantomData,
        }
    }

    /// `index_mut` that does not count as a write for the visibility caches: for a caller that
    /// bumps what it changes itself (`bump_key`).
    pub(crate) fn quiet_mut(&mut self, id: I) -> &mut T
    where
        I: Id,
    {
        let i = id.index();
        self.note_write(i);
        self.pages[i / PAGE].at_mut(i % PAGE)
    }

    /// Keep the bytes of entry `i` before the first write of the unit being recorded.
    fn note_write(&mut self, i: usize) {
        if self.watch.as_ref().is_some_and(|w| i < w.floor) {
            let w = self.watch.as_ref().expect("checked");
            if !w.seen.contains(&(i as u32)) {
                let before = (w.snap)(self.at(i));
                let w = self.watch.as_mut().expect("checked");
                w.seen.insert(i as u32);
                w.touched.push((i as u32, before));
            }
        }
    }

    /// How often a declaration named `key` was written (see `key_gen`).
    pub(crate) fn key_gen(&self, key: u32) -> u32 {
        self.key_gen.get(key as usize).copied().unwrap_or(0)
    }

    /// A declaration named `key` changed, or one is about to join a region.
    pub(crate) fn bump_key(&mut self, key: u32) {
        let k = key as usize;
        if self.key_gen.len() <= k {
            self.key_gen.resize((k + 1).next_power_of_two(), 0);
        }
        self.key_gen[k] = self.key_gen[k].wrapping_add(1);
    }

    /// Counts the mutable accesses of a tracked arena: equal values mean nothing a cache could
    /// depend on changed in between.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn tracked() -> Self {
        Arena {
            track: true,
            ..Self::new()
        }
    }

    pub fn push(&mut self, item: T) -> I {
        let i = self.len;
        self.note_owner(i);
        match self.pages.last_mut() {
            Some(p) if !i.is_multiple_of(PAGE) => p.push(item),
            _ => {
                let mut v = Vec::with_capacity(PAGE);
                v.push(item);
                self.pages.push(Page::Eager(v));
            }
        }
        self.len += 1;
        I::from_index(i)
    }

    /// The id the next `push` will return.
    pub fn next_id(&self) -> I {
        I::from_index(self.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = (I, &T)> {
        (0..self.len()).map(|i| (I::from_index(i), self.at(i)))
    }
}

impl<I, T> Arena<I, T> {
    fn note_owner(&mut self, i: usize) {
        if self.runs.last().is_none_or(|r| r.1 != self.owner) {
            self.runs.push((i as u32, self.owner));
        }
    }

    /// The page the next entry goes in, as a [`SlotPage`] (a partial page of values becomes one
    /// whose entries are already decoded).
    fn slots_tail(&mut self) -> &mut SlotPage<T> {
        if self.len.is_multiple_of(PAGE) {
            self.pages.push(Page::Slots(Box::new(SlotPage::new())));
        }
        let last = self.pages.last_mut().expect("a page was pushed above");
        if !matches!(last, Page::Slots(_)) {
            let v = std::mem::take(last.make_eager());
            *last = Page::Slots(Box::new(SlotPage::from_eager(v)));
        }
        match last {
            Page::Slots(s) => s,
            _ => unreachable!("made a page of slots above"),
        }
    }

    /// Append `n` entries as bytes, each decoded under `ctx` the first time it is read
    /// (`units.rs`). `lens` holds the length of each entry as a varint ([`take_varint`]); `data`
    /// the entries one after the other.
    pub(crate) fn push_run(
        &mut self,
        n: usize,
        mut lens: &[u8],
        data: yavs_diag::Slice,
        ctx: PageCtx,
        window: usize,
    ) -> Result<(), &'static str>
    where
        T: serde::de::DeserializeOwned,
    {
        let src = std::sync::Arc::new(Src {
            data,
            ctx,
            decode: decode_one::<T>,
            window,
        });
        let mut off = 0u32;
        let mut left = n;
        if n > 0 {
            self.note_owner(self.len);
        }
        // A page at a time: its source list is checked once, not once per entry.
        while left > 0 {
            let take = left.min(PAGE - self.len % PAGE);
            let page = self.slots_tail();
            if page
                .srcs
                .last()
                .is_none_or(|l| !std::sync::Arc::ptr_eq(l, &src))
            {
                page.srcs.push(src.clone());
            }
            let from = page.srcs.len() as u32 - 1;
            let mut done = 0;
            let mut bad = false;
            for _ in 0..take {
                let Some(len) = take_varint(&mut lens) else {
                    bad = true;
                    break;
                };
                page.slots.push(Slot { src: from, off });
                done += 1;
                match off.checked_add(len) {
                    Some(o) => off = o,
                    None => {
                        bad = true;
                        break;
                    }
                }
            }
            self.len += done;
            if bad {
                return Err("entry lengths");
            }
            left -= take;
        }
        if off as usize != src.data.len() {
            return Err("entry lengths do not add up");
        }
        Ok(())
    }

    /// The number of pages that are still bytes (tests).
    #[cfg(test)]
    pub(crate) fn undecoded_pages(&self) -> usize {
        self.pages.iter().map(|p| p.undecoded()).sum()
    }

    /// The owner of the entries pushed from now on (`units.rs`).
    pub(crate) fn set_owner(&mut self, owner: u32) {
        self.owner = owner;
    }

    /// `(first index, owner)` at each change of owner.
    pub(crate) fn runs_raw(&self) -> &[(u32, u32)] {
        &self.runs
    }

    /// `(first index, length)` of each run of entries of `owner`, in order.
    pub(crate) fn runs_of(&self, owner: u32) -> Vec<(u32, u32)> {
        let len = self.len() as u32;
        let mut out = Vec::new();
        for (k, r) in self.runs.iter().enumerate() {
            if r.1 == owner {
                let end = self.runs.get(k + 1).map_or(len, |n| n.0);
                if end > r.0 {
                    out.push((r.0, end - r.0));
                }
            }
        }
        out
    }

    /// Drop the entries from `len` on (an analysed-unit cache read that is undone).
    pub(crate) fn truncate(&mut self, len: usize) {
        if len >= self.len {
            return;
        }
        self.pages.truncate(len.div_ceil(PAGE));
        if !len.is_multiple_of(PAGE)
            && let Some(p) = self.pages.last_mut()
        {
            match p {
                Page::Slots(s) => s.truncate(len % PAGE),
                _ => p.make_eager().truncate(len % PAGE),
            }
        }
        self.len = len;
        while self.runs.last().is_some_and(|r| r.0 as usize >= len) {
            self.runs.pop();
        }
    }

    /// Replace entry `i` without counting it as a write of the unit being recorded.
    pub(crate) fn replace(&mut self, i: usize, item: T) -> T {
        self.version += 1;
        std::mem::replace(self.pages[i / PAGE].at_mut(i % PAGE), item)
    }

    /// Room for `n` more entries.
    pub(crate) fn reserve(&mut self, n: usize) {
        self.pages.reserve(n / PAGE + 1);
    }

    /// Count a change no `IndexMut` made (visibility caches see it).
    pub(crate) fn bump(&mut self) {
        self.version += 1;
    }

    /// End watching; the entries written and their bytes before.
    pub(crate) fn unwatch(&mut self) -> Option<Box<Watch<T>>> {
        self.watch.take()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// Entry `i` by index.
    pub(crate) fn get_at(&self, i: usize) -> &T {
        self.at(i)
    }

    #[inline]
    fn at(&self, i: usize) -> &T {
        self.pages[i / PAGE].at(i % PAGE)
    }
}

/// The image form of an arena: the entries of each chunk are one postcard sequence, so that a
/// chunk can be decoded on its own.
#[derive(serde::Serialize, serde::Deserialize)]
struct Image<'a> {
    track: bool,
    len: usize,
    starts: std::borrow::Cow<'a, [u32]>,
    runs: std::borrow::Cow<'a, [(u32, u32)]>,
}

impl<I, T: serde::Serialize> serde::Serialize for Arena<I, T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error;
        let len = self.len();
        let mut data = Vec::new();
        let mut starts = Vec::with_capacity(len / PAGE + 2);
        for c in 0..len.div_ceil(PAGE) {
            starts.push(data.len() as u32);
            for i in c * PAGE..len.min((c + 1) * PAGE) {
                data = postcard::to_extend(self.at(i), data).map_err(S::Error::custom)?;
            }
        }
        starts.push(data.len() as u32);
        use serde::ser::SerializeTuple;
        let mut t = s.serialize_tuple(2)?;
        t.serialize_element(&Image {
            track: self.track,
            len,
            starts: starts.into(),
            runs: self.runs[..].into(),
        })?;
        t.serialize_element(&BytesRef(&data))?;
        t.end()
    }
}

struct BytesRef<'a>(&'a [u8]);

impl serde::Serialize for BytesRef<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(self.0)
    }
}

impl<'de, I, T: serde::de::DeserializeOwned> serde::Deserialize<'de> for Arena<I, T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let (img, data) = <(Image, yavs_diag::Slice)>::deserialize(d)?;
        let chunks = img.len.div_ceil(PAGE);
        let starts = img.starts.into_owned();
        if starts.len() != chunks + 1
            || starts.windows(2).any(|w| w[0] > w[1])
            || starts.last().is_none_or(|&e| e as usize != data.len())
        {
            return Err(D::Error::custom("arena chunk table"));
        }
        let ctx = PageCtx {
            syms: yavs_syntax::DecodeTable::current(),
            reloc: None,
            files: None,
        };
        let pages = (0..chunks)
            .map(|c| {
                Page::Lazy(Box::new(LazyPage {
                    n: PAGE.min(img.len - c * PAGE),
                    data: data.range(starts[c] as usize, starts[c + 1] as usize),
                    ctx: ctx.clone(),
                    cell: std::sync::OnceLock::new(),
                    decode: decode_entries::<T>,
                }))
            })
            .collect();
        Ok(Arena {
            pages,
            len: img.len,
            track: img.track,
            version: 0,
            key_gen: Vec::new(),
            owner: SESSION,
            runs: img.runs.into_owned(),
            watch: None,
            _id: PhantomData,
        })
    }
}

impl<I, T: serde::Serialize> Arena<I, T> {
    /// Watch the writes of entries below `floor` (`units.rs`).
    pub(crate) fn watch(&mut self, floor: usize) {
        self.watch = Some(Box::new(Watch {
            floor,
            snap: raw_bytes::<T>,
            seen: Default::default(),
            touched: Vec::new(),
        }));
    }
}

impl<I: Id, T> Default for Arena<I, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<I: Id, T> std::ops::Index<I> for Arena<I, T> {
    type Output = T;
    fn index(&self, id: I) -> &T {
        self.at(id.index())
    }
}

impl<I: Id, T: 'static> std::ops::IndexMut<I> for Arena<I, T> {
    fn index_mut(&mut self, id: I) -> &mut T {
        let i = id.index();
        self.note_write(i);
        let item = self.pages[i / PAGE].at_mut(i % PAGE);
        // A write to a declaration changes the answers for its name only; any other write
        // changes every answer.
        match (item as &mut dyn std::any::Any).downcast_ref::<crate::decl::Decl>() {
            Some(d) => {
                let k = d.name.key().index() as usize;
                if self.key_gen.len() <= k {
                    self.key_gen.resize((k + 1).next_power_of_two(), 0);
                }
                self.key_gen[k] = self.key_gen[k].wrapping_add(1);
            }
            None => self.version += 1,
        }
        item
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An image of `n` strings, read back lazily, behaves like the arena it was made from.
    #[test]
    fn lazy_arena_reads_pushes_and_writes_like_an_eager_one() {
        let mut a: Arena<DeclId, String> = Arena::tracked();
        let n = 3 * PAGE + 5;
        for i in 0..n {
            a.push(format!("item {i}"));
        }
        let bytes = postcard::to_stdvec(&a).expect("serialize");
        let buf = yavs_diag::Buffer::new(bytes);
        let mut b: Arena<DeclId, String> =
            yavs_diag::sharing(&buf, || postcard::from_bytes(&buf)).expect("deserialize");
        assert_eq!(b.len(), n);
        // Only the page read is decoded.
        assert_eq!(b.undecoded_pages(), 4);
        assert_eq!(
            b[DeclId::from_index(PAGE + 1)],
            format!("item {}", PAGE + 1)
        );
        assert_eq!(b.undecoded_pages(), 3);
        // A write decodes its chunk, a push lands after the lazy entries.
        b[DeclId::from_index(n - 1)].push('!');
        let id = b.push("new".into());
        assert_eq!(
            (id.index(), b.len(), b.next_id().index()),
            (n, n + 1, n + 1)
        );
        assert_eq!(b[id], "new");
        assert_eq!(b[DeclId::from_index(n - 1)], format!("item {}!", n - 1));
        // Reading everything, and writing the arena out again, gives the same entries.
        let all: Vec<_> = b.iter().map(|(_, s)| s.clone()).collect();
        assert_eq!(all.len(), n + 1);
        let again = postcard::to_stdvec(&b).expect("serialize again");
        let c: Arena<DeclId, String> = postcard::from_bytes(&again).expect("deserialize again");
        assert!(c.iter().map(|(_, s)| s).eq(all.iter()));
    }

    /// Runs of bytes appended after eager entries, read one by one, written, truncated and grown
    /// again.
    #[test]
    fn reads_in_order_decode_ahead_and_scattered_reads_do_not() {
        let mut a: Arena<DeclId, String> = Arena::new();
        let (mut lens, mut bytes) = (Vec::new(), Vec::new());
        for i in 0..PAGE {
            let before = bytes.len();
            bytes = postcard::to_extend(&format!("l{i}"), bytes).expect("write");
            put_varint(&mut lens, (bytes.len() - before) as u32);
        }
        let ctx = PageCtx {
            syms: None,
            reloc: None,
            files: None,
        };
        a.push_run(PAGE, &lens, yavs_diag::Slice::from(bytes), ctx, 1)
            .expect("a run");
        // Entries read here and there are decoded one by one.
        for i in [40, 10, 25] {
            assert_eq!(a[DeclId::from_index(i)], format!("l{i}"));
        }
        assert_eq!(a.undecoded_pages(), PAGE - 3);
        // Entries read one after the other are decoded ahead (1 + 2 + 4 + 8 after the first).
        for i in 50..54 {
            assert_eq!(a[DeclId::from_index(i)], format!("l{i}"));
        }
        assert!(a.undecoded_pages() < PAGE - 3 - 4);
        a.truncate(45);
        assert_eq!(a.len(), 45);
        assert_eq!(a[DeclId::from_index(44)], "l44");
    }

    #[test]
    fn runs_of_bytes_mix_with_eager_entries() {
        let mut a: Arena<DeclId, String> = Arena::new();
        let start = PAGE - 3;
        for i in 0..start {
            a.push(format!("e{i}"));
        }
        let run = |from: usize, n: usize| {
            let (mut lens, mut bytes) = (Vec::new(), Vec::new());
            for i in from..from + n {
                let before = bytes.len();
                bytes = postcard::to_extend(&format!("l{i}"), bytes).expect("write");
                put_varint(&mut lens, (bytes.len() - before) as u32);
            }
            (lens, yavs_diag::Slice::from(bytes))
        };
        let ctx = PageCtx {
            syms: None,
            reloc: None,
            files: None,
        };
        let (lens, data) = run(0, 2 * PAGE);
        a.push_run(2 * PAGE, &lens, data, ctx.clone(), 1)
            .expect("a run");
        let (lens, data) = run(2 * PAGE, 5);
        a.push_run(5, &lens, data, ctx, 3).expect("another run");
        let id = a.push("tail".into());
        let n = start + 2 * PAGE + 5;
        assert_eq!((a.len(), id.index()), (n + 1, n));
        // Nothing but what is read is decoded.
        assert_eq!(a.undecoded_pages(), 2 * PAGE + 5);
        assert_eq!(a[DeclId::from_index(start + 7)], "l7");
        assert_eq!(
            a[DeclId::from_index(start + 2 * PAGE + 4)],
            format!("l{}", 2 * PAGE + 4)
        );
        assert_eq!(a.undecoded_pages(), 2 * PAGE + 3);
        assert_eq!(a[DeclId::from_index(start - 1)], format!("e{}", start - 1));
        a[DeclId::from_index(start)].push('!');
        assert_eq!(a[DeclId::from_index(start)], "l0!");
        a.truncate(start + 2);
        assert_eq!(a.len(), start + 2);
        let id = a.push("again".into());
        assert_eq!(a[id], "again");
        assert_eq!(a[DeclId::from_index(start + 1)], "l1");
        // Writing the arena out reads every entry.
        let bytes = postcard::to_stdvec(&a).expect("serialize");
        let b: Arena<DeclId, String> = postcard::from_bytes(&bytes).expect("deserialize");
        assert!(b.iter().map(|(_, s)| s).eq(a.iter().map(|(_, s)| s)));
    }

    #[test]
    fn varints_roundtrip() {
        let mut b = Vec::new();
        let xs = [0, 1, 127, 128, 300, 16_383, 16_384, u32::MAX >> 4];
        for x in xs {
            put_varint(&mut b, x);
        }
        let mut r = &b[..];
        for x in xs {
            assert_eq!(take_varint(&mut r), Some(x));
        }
        assert!(r.is_empty() && take_varint(&mut r).is_none());
    }

    #[test]
    fn an_empty_arena_roundtrips() {
        let a: Arena<DeclId, u32> = Arena::new();
        let bytes = postcard::to_stdvec(&a).expect("serialize");
        let mut b: Arena<DeclId, u32> = postcard::from_bytes(&bytes).expect("deserialize");
        assert!(b.is_empty());
        assert_eq!(b.push(7).index(), 0);
        assert_eq!(b[DeclId::from_index(0)], 7);
    }

    #[test]
    fn ids_roundtrip_and_option_is_free() {
        let mut a: Arena<DeclId, &str> = Arena::new();
        let x = a.push("x");
        let y = a.push("y");
        assert_eq!((x.index(), y.index()), (0, 1));
        assert_eq!(a[y], "y");
        assert_eq!(std::mem::size_of::<Option<DeclId>>(), 4);
    }
}
