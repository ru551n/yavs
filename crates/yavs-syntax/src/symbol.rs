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

struct Interner {
    map: FxHashMap<&'static str, Symbol>,
    strings: Vec<&'static str>,
}

impl Default for Interner {
    /// Sized for a short run's names: the table is touched at random, so every name of a bigger
    /// table costs a page fault (0.15 ms of a short run); a longer one grows by rehashing.
    fn default() -> Self {
        Interner {
            map: FxHashMap::with_capacity_and_hasher(1 << 11, Default::default()),
            strings: Vec::with_capacity(1 << 11),
        }
    }
}

fn interner() -> &'static RwLock<Interner> {
    static I: OnceLock<RwLock<Interner>> = OnceLock::new();
    I.get_or_init(Default::default)
}

/// Per-thread mirror of the table: a hit or `as_str` of a known symbol takes no lock (the
/// table only grows, so a copy never goes stale, it only lacks newer entries).
#[derive(Default)]
struct Local {
    map: FxHashMap<&'static str, Symbol>,
    strings: Vec<&'static str>,
}

thread_local! {
    static LOCAL: std::cell::RefCell<Local> = std::cell::RefCell::new(Local::default());
}

impl Symbol {
    pub fn intern(s: &str) -> Symbol {
        LOCAL.with(|l| {
            let mut l = l.borrow_mut();
            if let Some(&sym) = l.map.get(s) {
                return sym;
            }
            let (sym, text) = Self::intern_global(s);
            l.map.insert(text, sym);
            sym
        })
    }

    fn intern_global(s: &str) -> (Symbol, &'static str) {
        if let Some((&text, &sym)) = interner()
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .map
            .get_key_value(s)
        {
            return (sym, text);
        }
        let mut i = interner().write().unwrap_or_else(|e| e.into_inner());
        if let Some((&text, &sym)) = i.map.get_key_value(s) {
            return (sym, text);
        }
        let s: &'static str = Box::leak(s.to_owned().into_boxed_str());
        let sym = Symbol(i.strings.len() as u32);
        i.strings.push(s);
        i.map.insert(s, sym);
        (sym, s)
    }

    pub fn as_str(self) -> &'static str {
        let i = self.0 as usize;
        LOCAL.with(|l| {
            let mut l = l.borrow_mut();
            if let Some(&s) = l.strings.get(i) {
                return s;
            }
            let g = interner().read().unwrap_or_else(|e| e.into_inner());
            let have = l.strings.len();
            l.strings.extend_from_slice(&g.strings[have..]);
            l.strings[i]
        })
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

    /// A table written by `encode_table` decodes in any order and interns only what is read.
    #[test]
    fn image_table_is_read_lazily() {
        use super::{DecodeTable, encode_table};
        let syms = [
            Symbol::intern("alpha_t"),
            Symbol::intern("beta_t"),
            Symbol::intern(""),
        ];
        let (bytes, table) = encode_table(|| postcard::to_stdvec(&syms).expect("serialize"));
        let table = DecodeTable::new(table);
        assert!(DecodeTable::current().is_none());
        let back: [Symbol; 3] = table.run(|| postcard::from_bytes(&bytes).expect("deserialize"));
        assert_eq!(back, syms);
        assert_eq!(back[1].as_str(), "beta_t");
    }

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

/// Table index by symbol number (`u32::MAX`: not written yet), and the symbols in table order.
type EncTable = (Vec<u32>, Vec<Symbol>);

thread_local! {
    static ENC: std::cell::RefCell<Option<EncTable>> = const { std::cell::RefCell::new(None) };
    static DEC: std::cell::RefCell<Option<DecodeTable>> = const { std::cell::RefCell::new(None) };
}

/// The texts of the symbols an image holds, as one string and the end offset of each.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct SymbolTable {
    text: String,
    ends: Vec<u32>,
}

/// Run `f` (which serializes values holding symbols) and return its result with the table of
/// the symbol texts it used; the written indices refer to that table.
pub fn encode_table<R>(f: impl FnOnce() -> R) -> (R, SymbolTable) {
    ENC.with(|e| *e.borrow_mut() = Some(Default::default()));
    let r = f();
    let (_, list) = ENC.with(|e| e.borrow_mut().take()).unwrap_or_default();
    let mut t = SymbolTable {
        text: String::new(),
        ends: Vec::with_capacity(list.len()),
    };
    for s in list {
        t.text.push_str(s.as_str());
        t.ends.push(t.text.len() as u32);
    }
    (r, t)
}

/// The symbols of a table `encode_table` made, for deserializing values that hold them, now or
/// later (a lazily decoded arena keeps one). A symbol is interned when first read, so the
/// texts nothing reads cost nothing.
#[derive(Clone)]
pub struct DecodeTable(std::sync::Arc<Decoded>);

struct Decoded {
    table: SymbolTable,
    syms: Vec<std::sync::OnceLock<Symbol>>,
}

impl DecodeTable {
    pub fn new(table: SymbolTable) -> Self {
        let syms = (0..table.ends.len()).map(|_| Default::default()).collect();
        DecodeTable(std::sync::Arc::new(Decoded { table, syms }))
    }

    fn get(&self, i: usize) -> Option<Symbol> {
        let t = &self.0.table;
        let end = *t.ends.get(i)? as usize;
        let start = if i == 0 { 0 } else { t.ends[i - 1] as usize };
        let text = t.text.get(start..end)?;
        Some(*self.0.syms[i].get_or_init(|| Symbol::intern(text)))
    }

    /// The table of the enclosing [`DecodeTable::run`], if any.
    pub fn current() -> Option<Self> {
        DEC.with(|d| d.borrow().clone())
    }

    /// Run `f` (which deserializes values holding symbols) against this table.
    pub fn run<R>(&self, f: impl FnOnce() -> R) -> R {
        let before = DEC.with(|d| d.borrow_mut().replace(self.clone()));
        let r = f();
        DEC.with(|d| *d.borrow_mut() = before);
        r
    }
}

impl serde::Serialize for Symbol {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let i = ENC.with(|e| {
            let mut e = e.borrow_mut();
            let (ids, list) = e.as_mut()?;
            let at = self.0 as usize;
            if at >= ids.len() {
                ids.resize(at + 1, u32::MAX);
            }
            if ids[at] == u32::MAX {
                list.push(*self);
                ids[at] = list.len() as u32 - 1;
            }
            Some(ids[at])
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
        DEC.with(|t| t.borrow().as_ref().and_then(|t| t.get(i as usize)))
            .ok_or_else(|| serde::de::Error::custom("symbol outside decode_table or out of range"))
    }
}
