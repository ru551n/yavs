//! A global string interner.
//!
//! Strings are leaked, so [`Symbol::as_str`] returns `&'static str`. The table
//! sits behind a read-write lock (hits and `as_str` only read; the hash is
//! not DoS-hardened because names come from the user's own sources), and symbols stay valid on every thread (the runtime will read
//! names from worker threads).

use std::sync::{OnceLock, RwLock};

use rustc_hash::FxHashMap;

/// An interned string.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Symbol(u32);

#[derive(Default)]
struct Interner {
    map: FxHashMap<&'static str, Symbol>,
    strings: Vec<&'static str>,
}

fn interner() -> &'static RwLock<Interner> {
    static I: OnceLock<RwLock<Interner>> = OnceLock::new();
    I.get_or_init(Default::default)
}

impl Symbol {
    pub fn intern(s: &str) -> Symbol {
        if let Some(&sym) = interner()
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .map
            .get(s)
        {
            return sym;
        }
        let mut i = interner().write().unwrap_or_else(|e| e.into_inner());
        if let Some(&sym) = i.map.get(s) {
            return sym;
        }
        let s: &'static str = Box::leak(s.to_owned().into_boxed_str());
        let sym = Symbol(i.strings.len() as u32);
        i.strings.push(s);
        i.map.insert(s, sym);
        sym
    }

    pub fn as_str(self) -> &'static str {
        interner().read().unwrap_or_else(|e| e.into_inner()).strings[self.0 as usize]
    }
}

impl std::fmt::Debug for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl std::fmt::Display for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::Symbol;

    #[test]
    fn roundtrip() {
        let a = Symbol::intern("Foo");
        assert_eq!(a, Symbol::intern("Foo"));
        assert_ne!(a, Symbol::intern("foo"));
        assert_eq!(a.as_str(), "Foo");
    }
}

// ---------------------------------------------------------------------------
// Images (the compilation cache): symbols are written as indices into a table of
// their texts, so one image holds each text once and loads without hashing it per use.

type EncTable = (HashMap<Symbol, u32>, Vec<Symbol>);

thread_local! {
    static ENC: std::cell::RefCell<Option<EncTable>> = const { std::cell::RefCell::new(None) };
    static DEC: std::cell::RefCell<Option<Vec<Symbol>>> = const { std::cell::RefCell::new(None) };
}

/// Run `f` (which serializes values holding symbols) and return its result with the table of
/// the symbol texts it used; the written indices refer to that table.
pub fn encode_table<R>(f: impl FnOnce() -> R) -> (R, Vec<&'static str>) {
    ENC.with(|e| *e.borrow_mut() = Some(Default::default()));
    let r = f();
    let (_, list) = ENC.with(|e| e.borrow_mut().take()).unwrap_or_default();
    (r, list.into_iter().map(Symbol::as_str).collect())
}

/// Run `f` (which deserializes values holding symbols) against the table `encode_table` made.
pub fn decode_table<R, S: AsRef<str>>(table: &[S], f: impl FnOnce() -> R) -> R {
    let syms = table.iter().map(|t| Symbol::intern(t.as_ref())).collect();
    DEC.with(|d| *d.borrow_mut() = Some(syms));
    let r = f();
    DEC.with(|d| *d.borrow_mut() = None);
    r
}

impl serde::Serialize for Symbol {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let i = ENC.with(|e| {
            let mut e = e.borrow_mut();
            let (ids, list) = e.as_mut()?;
            Some(*ids.entry(*self).or_insert_with(|| {
                list.push(*self);
                list.len() as u32 - 1
            }))
        });
        match i {
            Some(i) => s.serialize_u32(i),
            None => Err(serde::ser::Error::custom(
                "symbol written outside encode_table",
            )),
        }
    }
}

impl<'de> serde::Deserialize<'de> for Symbol {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let i = <u32 as serde::Deserialize>::deserialize(d)?;
        DEC.with(|t| t.borrow().as_ref().and_then(|t| t.get(i as usize).copied()))
            .ok_or_else(|| serde::de::Error::custom("symbol outside decode_table or out of range"))
    }
}
