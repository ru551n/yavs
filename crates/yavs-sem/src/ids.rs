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
    ($($name:ident),* $(,)?) => {$(
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
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

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), self.index())
            }
        }
    )*};
}

id!(
    LibId,
    UnitId,
    DeclId,
    ScopeId,
    TypeId,
    SubtypeId,
    ExprId,
    StmtId,
    ConstId,
    InstId,
    ViewId,
    ExternalId,
    AttrValueId,
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
    _id: PhantomData<I>,
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
        match &self.syms {
            Some(t) => t.run(|| (self.decode)(bytes, n)),
            None => (self.decode)(bytes, n),
        }
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
        self.items.push(item);
        I::from_index(self.len() - 1)
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
    pub fn len(&self) -> usize {
        self.lazy.as_ref().map_or(0, |l| l.len) + self.items.len()
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
            _id: PhantomData,
        })
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
