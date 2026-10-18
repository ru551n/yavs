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
pub struct Arena<I, T> {
    items: Vec<T>,
    _id: PhantomData<I>,
}

impl<I: Id, T> Arena<I, T> {
    pub fn new() -> Self {
        Arena {
            items: Vec::new(),
            _id: PhantomData,
        }
    }

    pub fn push(&mut self, item: T) -> I {
        self.items.push(item);
        I::from_index(self.items.len() - 1)
    }

    /// The id the next `push` will return.
    pub fn next_id(&self) -> I {
        I::from_index(self.items.len())
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (I, &T)> {
        self.items
            .iter()
            .enumerate()
            .map(|(i, t)| (I::from_index(i), t))
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
        &self.items[id.index()]
    }
}

impl<I: Id, T> std::ops::IndexMut<I> for Arena<I, T> {
    fn index_mut(&mut self, id: I) -> &mut T {
        &mut self.items[id.index()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
