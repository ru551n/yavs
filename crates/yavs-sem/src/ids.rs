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
/// An arena restored from an analysis image decodes its entries lazily, [`CHUNK`] at a time, the
/// first time one of them is read (`docs/decisions.md` "Compilation cache"): a run that never
/// looks at an entry never pays for it.
pub struct Arena<I, T> {
    /// The entries after the lazily decoded ones (all of them when `lazy` is `None`).
    items: Vec<T>,
    lazy: Option<Lazy<T>>,
    /// A tracked arena counts every mutable access in `version` (visibility caches).
    track: bool,
    /// Mutable accesses so far; not serialized.
    version: u64,
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

/// Entries per lazily decoded chunk.
const CHUNK: usize = 64;

/// The entries of an image, one postcard sequence per chunk.
struct Lazy<T> {
    len: usize,
    /// Start of each chunk in `data`, and the end of the last.
    starts: Vec<u32>,
    data: yavs_diag::Slice,
    chunks: Vec<std::sync::OnceLock<Vec<T>>>,
    syms: Option<yavs_syntax::DecodeTable>,
    /// Decodes `n` entries from a chunk's bytes (monomorphic: made where `T: Deserialize` is known).
    decode: fn(&[u8], usize) -> Vec<T>,
}

impl<T> Lazy<T> {
    fn decode_chunk(&self, c: usize) -> Vec<T> {
        let n = CHUNK.min(self.len - c * CHUNK);
        let bytes = &self.data.get()[self.starts[c] as usize..self.starts[c + 1] as usize];
        // A chunk may be read while a unit of the analysed-unit cache is written or read: its ids
        // and files are the image's own.
        plain(|| match &self.syms {
            Some(t) => t.run(|| (self.decode)(bytes, n)),
            None => (self.decode)(bytes, n),
        })
    }

    fn chunk(&self, c: usize) -> &Vec<T> {
        self.chunks[c].get_or_init(|| self.decode_chunk(c))
    }

    fn chunk_mut(&mut self, c: usize) -> &mut Vec<T> {
        self.chunk(c);
        self.chunks[c].get_mut().expect("chunk decoded just now")
    }
}

fn decode_entries<T: serde::de::DeserializeOwned>(mut rest: &[u8], n: usize) -> Vec<T> {
    (0..n)
        .map(|_| {
            let (t, r) = postcard::take_from_bytes::<T>(rest)
                .unwrap_or_else(|e| panic!("internal error: corrupt image chunk: {e}"));
            rest = r;
            t
        })
        .collect()
}

impl<I: Id, T> Arena<I, T> {
    pub fn new() -> Self {
        Arena {
            items: Vec::new(),
            lazy: None,
            track: false,
            version: 0,
            owner: SESSION,
            runs: Vec::new(),
            watch: None,
            _id: PhantomData,
        }
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
        let i = self.len();
        if self.runs.last().is_none_or(|r| r.1 != self.owner) {
            self.runs.push((i as u32, self.owner));
        }
        self.items.push(item);
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

    /// Drop the entries from `len` on (an analysed-unit cache read that is undone). They were all
    /// pushed after the lazily decoded ones.
    pub(crate) fn truncate(&mut self, len: usize) {
        let base = self.lazy.as_ref().map_or(0, |l| l.len);
        assert!(len >= base, "internal error: truncating restored entries");
        self.items.truncate(len - base);
        while self.runs.last().is_some_and(|r| r.0 as usize >= len) {
            self.runs.pop();
        }
    }

    /// Replace entry `i` without counting it as a write of the unit being recorded.
    pub(crate) fn replace(&mut self, i: usize, item: T) -> T {
        self.version += 1;
        let base = self.lazy.as_ref().map_or(0, |l| l.len);
        let slot = match &mut self.lazy {
            Some(l) if i < base => &mut l.chunk_mut(i / CHUNK)[i % CHUNK],
            _ => &mut self.items[i - base],
        };
        std::mem::replace(slot, item)
    }

    /// Room for `n` more entries.
    pub(crate) fn reserve(&mut self, n: usize) {
        self.items.reserve(n);
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
        self.lazy.as_ref().map_or(0, |l| l.len) + self.items.len()
    }

    /// Entry `i` by index.
    pub(crate) fn get_at(&self, i: usize) -> &T {
        self.at(i)
    }

    fn at(&self, i: usize) -> &T {
        match &self.lazy {
            Some(l) if i < l.len => &l.chunk(i / CHUNK)[i % CHUNK],
            Some(l) => &self.items[i - l.len],
            None => &self.items[i],
        }
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
        let mut starts = Vec::with_capacity(len / CHUNK + 2);
        for c in 0..len.div_ceil(CHUNK) {
            starts.push(data.len() as u32);
            for i in c * CHUNK..len.min((c + 1) * CHUNK) {
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
        let chunks = img.len.div_ceil(CHUNK);
        let starts = img.starts.into_owned();
        if starts.len() != chunks + 1
            || starts.windows(2).any(|w| w[0] > w[1])
            || starts.last().is_none_or(|&e| e as usize != data.len())
        {
            return Err(D::Error::custom("arena chunk table"));
        }
        Ok(Arena {
            items: Vec::new(),
            lazy: Some(Lazy {
                len: img.len,
                starts,
                data,
                chunks: (0..chunks).map(|_| std::sync::OnceLock::new()).collect(),
                syms: yavs_syntax::DecodeTable::current(),
                decode: decode_entries::<T>,
            }),
            track: img.track,
            version: 0,
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

impl<I: Id, T> std::ops::IndexMut<I> for Arena<I, T> {
    fn index_mut(&mut self, id: I) -> &mut T {
        self.version += 1;
        let i = id.index();
        if self.watch.as_ref().is_some_and(|w| i < w.floor) {
            let w = self.watch.as_ref().expect("checked");
            if !w.seen.contains(&(i as u32)) {
                let before = (w.snap)(self.at(i));
                let w = self.watch.as_mut().expect("checked");
                w.seen.insert(i as u32);
                w.touched.push((i as u32, before));
            }
        }
        let base = self.lazy.as_ref().map_or(0, |l| l.len);
        match &mut self.lazy {
            Some(l) if i < base => &mut l.chunk_mut(i / CHUNK)[i % CHUNK],
            _ => &mut self.items[i - base],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An image of `n` strings, read back lazily, behaves like the arena it was made from.
    #[test]
    fn lazy_arena_reads_pushes_and_writes_like_an_eager_one() {
        let mut a: Arena<DeclId, String> = Arena::tracked();
        let n = 3 * CHUNK + 5;
        for i in 0..n {
            a.push(format!("item {i}"));
        }
        let bytes = postcard::to_stdvec(&a).expect("serialize");
        let buf = yavs_diag::Buffer::new(bytes);
        let mut b: Arena<DeclId, String> =
            yavs_diag::sharing(&buf, || postcard::from_bytes(&buf)).expect("deserialize");
        assert_eq!(b.len(), n);
        // Only the chunk read is decoded.
        let l = b.lazy.as_ref().expect("lazy");
        assert!(l.chunks.iter().all(|c| c.get().is_none()));
        assert_eq!(
            b[DeclId::from_index(CHUNK + 1)],
            format!("item {}", CHUNK + 1)
        );
        let l = b.lazy.as_ref().expect("lazy");
        assert_eq!(l.chunks.iter().filter(|c| c.get().is_some()).count(), 1);
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
