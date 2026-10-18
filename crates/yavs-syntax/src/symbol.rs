//! A global string interner.
//!
//! Strings are leaked, so [`Symbol::as_str`] returns `&'static str`. The table
//! sits behind a mutex: parsing is single-threaded, so the lock is never
//! contended, and symbols stay valid on every thread (the runtime will read
//! names from worker threads).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// An interned string.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Symbol(u32);

#[derive(Default)]
struct Interner {
    map: HashMap<&'static str, Symbol>,
    strings: Vec<&'static str>,
}

fn interner() -> &'static Mutex<Interner> {
    static I: OnceLock<Mutex<Interner>> = OnceLock::new();
    I.get_or_init(Default::default)
}

impl Symbol {
    pub fn intern(s: &str) -> Symbol {
        let mut i = interner().lock().unwrap_or_else(|e| e.into_inner());
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
        interner().lock().unwrap_or_else(|e| e.into_inner()).strings[self.0 as usize]
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
