//! The maps and sets of `Db` that a unit may change, with their writes logged for the
//! analysed-unit cache (`units.rs`, `docs/decisions.md` "Analysed-unit cache", "Writes are
//! tracked").
//!
//! A write appends the key and the hash of the value it replaced to a log while a unit is being
//! recorded (`track`); a unit's own changes are the log since its mark, and the frame forgets
//! them when it ends (`forget`), so a unit loaded inside another is not found again by each
//! unit around it. There is no `DerefMut`: a
//! write has to go through `insert` or `remove`, so a new write path cannot bypass the log.
//! Reading is by `Deref`.

use std::hash::Hash;
use std::ops::Deref;

use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};

use crate::units::{H, hash_raw};

/// A key a unit wrote: its hash, the hash of the value before the first write and now (`None`:
/// absent), and the position of the first write in the log.
pub(crate) type Touched = (H, Option<H>, Option<H>, usize);

/// A `FxHashMap` whose writes are logged.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TrackedMap<K: Eq + Hash, V> {
    map: FxHashMap<K, V>,
    #[serde(skip, default = "Vec::new")]
    log: Vec<(K, Option<H>)>,
    #[serde(skip, default = "bool::default")]
    on: bool,
}

/// A `FxHashSet` whose writes are logged.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TrackedSet<K: Eq + Hash> {
    set: FxHashSet<K>,
    #[serde(skip, default = "Vec::new")]
    log: Vec<(K, Option<H>)>,
    #[serde(skip, default = "bool::default")]
    on: bool,
}

impl<K: Eq + Hash, V> Default for TrackedMap<K, V> {
    fn default() -> Self {
        Self {
            map: Default::default(),
            log: Vec::new(),
            on: false,
        }
    }
}

impl<K: Eq + Hash> Default for TrackedSet<K> {
    fn default() -> Self {
        Self {
            set: Default::default(),
            log: Vec::new(),
            on: false,
        }
    }
}

impl<K: Eq + Hash, V> Deref for TrackedMap<K, V> {
    type Target = FxHashMap<K, V>;
    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl<K: Eq + Hash> Deref for TrackedSet<K> {
    type Target = FxHashSet<K>;
    fn deref(&self) -> &Self::Target {
        &self.set
    }
}

fn hash_of<T: Serialize>(t: &T) -> H {
    hash_raw(t, &mut Vec::new())
}

impl<K: Eq + Hash + Clone + Serialize, V: Serialize> TrackedMap<K, V> {
    pub fn insert(&mut self, k: K, v: V) -> Option<V> {
        if !self.on {
            return self.map.insert(k, v);
        }
        let old = self.map.insert(k.clone(), v);
        self.log.push((k, old.as_ref().map(hash_of)));
        old
    }

    pub fn remove(&mut self, k: &K) -> Option<V> {
        let old = self.map.remove(k)?;
        if self.on {
            self.log.push((k.clone(), Some(hash_of(&old))));
        }
        Some(old)
    }

    /// A write that is not logged: a replayed change, which the frame around it records another way.
    pub(crate) fn insert_quiet(&mut self, k: K, v: V) -> Option<V> {
        self.map.insert(k, v)
    }

    pub(crate) fn remove_quiet(&mut self, k: &K) -> Option<V> {
        self.map.remove(k)
    }

    /// Change the value of `k`, made by default when absent.
    pub fn edit(&mut self, k: K, f: impl FnOnce(&mut V))
    where
        V: Default,
    {
        if self.on {
            self.log.push((k.clone(), self.map.get(&k).map(hash_of)));
        }
        f(self.map.entry(k).or_default());
    }

    /// Start (`true`) or end logging; ending forgets the log.
    pub(crate) fn track(&mut self, on: bool) {
        self.on = on;
        if !on {
            self.log.clear();
        }
    }

    pub(crate) fn mark(&self) -> usize {
        self.log.len()
    }

    /// Forget the writes since `mark`: they are the frame's, which has been told.
    pub(crate) fn forget(&mut self, mark: usize) {
        self.log.truncate(mark);
    }

    /// The keys written since `mark`, by key hash.
    pub(crate) fn touched_since(&self, mark: usize) -> Vec<Touched> {
        touched(&self.log, mark, |k| self.map.get(k).map(hash_of))
    }
}

impl<K: Eq + Hash + Clone + Serialize> TrackedSet<K> {
    /// Whether `k` was new.
    pub fn insert(&mut self, k: K) -> bool {
        if !self.on {
            return self.set.insert(k);
        }
        let new = self.set.insert(k.clone());
        if new {
            self.log.push((k, None));
        }
        new
    }

    pub fn remove(&mut self, k: &K) -> bool {
        let had = self.set.remove(k);
        if had && self.on {
            self.log.push((k.clone(), Some(hash_of(&()))));
        }
        had
    }

    pub(crate) fn insert_quiet(&mut self, k: K) -> bool {
        self.set.insert(k)
    }

    pub(crate) fn remove_quiet(&mut self, k: &K) -> bool {
        self.set.remove(k)
    }

    pub(crate) fn track(&mut self, on: bool) {
        self.on = on;
        if !on {
            self.log.clear();
        }
    }

    pub(crate) fn mark(&self) -> usize {
        self.log.len()
    }

    pub(crate) fn forget(&mut self, mark: usize) {
        self.log.truncate(mark);
    }

    pub(crate) fn touched_since(&self, mark: usize) -> Vec<Touched> {
        touched(&self.log, mark, |k| {
            self.set.contains(k).then(|| hash_of(&()))
        })
    }

    /// The keys that were there at `mark` and are gone now.
    pub(crate) fn removed_since(&self, mark: usize) -> Vec<K> {
        let mut first: FxHashMap<&K, bool> = FxHashMap::default();
        let mut order = Vec::new();
        for (k, pre) in &self.log[mark..] {
            if let std::collections::hash_map::Entry::Vacant(v) = first.entry(k) {
                v.insert(pre.is_some());
                order.push(k);
            }
        }
        order
            .into_iter()
            .filter(|k| first[*k] && !self.set.contains(*k))
            .cloned()
            .collect()
    }
}

fn touched<K: Eq + Hash + Serialize>(
    log: &[(K, Option<H>)],
    mark: usize,
    now: impl Fn(&K) -> Option<H>,
) -> Vec<Touched> {
    let mut first: FxHashMap<&K, (Option<H>, usize)> = FxHashMap::default();
    for (i, (k, pre)) in log.iter().enumerate().skip(mark) {
        first.entry(k).or_insert((*pre, i));
    }
    let mut out: Vec<Touched> = first
        .into_iter()
        .map(|(k, (pre, i))| (hash_of(k), pre, now(k), i))
        .collect();
    out.sort_unstable_by_key(|c| c.0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touched_are_the_keys_written_with_their_value_before_the_first_write() {
        let mut m: TrackedMap<u32, u32> = Default::default();
        m.insert(1, 10);
        m.insert(2, 20);
        m.track(true);
        let mark = m.mark();
        m.insert(3, 30); // added
        m.insert(1, 11); // changed
        m.remove(&2); // removed
        m.insert(2, 20); // put back
        m.insert(4, 40); // added and removed
        m.remove(&4);
        m.insert_quiet(5, 50); // not logged
        let h = |k: u32| hash_of(&k);
        let mut want = vec![
            (h(3), None, Some(h(30)), 0),
            (h(1), Some(h(10)), Some(h(11)), 1),
            (h(2), Some(h(20)), Some(h(20)), 2),
            (h(4), None, None, 4),
        ];
        want.sort_unstable_by_key(|c| c.0);
        assert_eq!(m.touched_since(mark), want);
        assert_eq!(m.touched_since(mark + 4).len(), 1);
        m.forget(mark);
        assert!(m.touched_since(mark).is_empty());
        m.track(false);
        m.insert(9, 9);
        assert_eq!(m.mark(), 0);
    }

    #[test]
    fn set_writes() {
        let mut s: TrackedSet<u32> = Default::default();
        s.insert(1);
        s.track(true);
        let mark = s.mark();
        assert!(s.insert(2));
        assert!(!s.insert(1));
        assert!(s.remove(&1));
        let u = hash_of(&());
        let mut want = vec![
            (hash_of(&2u32), None, Some(u), 0),
            (hash_of(&1u32), Some(u), None, 1),
        ];
        want.sort_unstable_by_key(|c| c.0);
        assert_eq!(s.touched_since(mark), want);
        assert_eq!(s.removed_since(mark), vec![1]);
        s.insert(3);
        s.remove(&3);
        s.insert(1);
        assert!(s.removed_since(mark).is_empty());
    }
}
