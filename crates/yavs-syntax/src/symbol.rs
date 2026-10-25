//! A global string interner.
//!
//! Strings are leaked, so [`Symbol::as_str`] returns `&'static str`. The table
//! sits behind a read-write lock (hits and `as_str` only read; the hash is
//! not DoS-hardened because names come from the user's own sources), and symbols stay valid on every thread (the runtime will read
//! names from worker threads).

use std::hash::Hasher;
use std::sync::{OnceLock, RwLock};

/// An interned string.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Symbol(u32);

/// Open-addressing set of interned strings, probed with a hash made once: a miss in the
/// thread's mirror and in the shared table (and the insert into both) hash the text once, and
/// growing never reads a text again.
struct Index {
    /// `(text, hash, symbol number)`; number `u32::MAX` is an empty slot.
    slots: Vec<(&'static str, u32, u32)>,
    len: usize,
}

const EMPTY: (&str, u32, u32) = ("", 0, u32::MAX);

impl Index {
    fn with_capacity(n: usize) -> Self {
        Index {
            slots: vec![EMPTY; (n * 3 / 2).next_power_of_two()],
            len: 0,
        }
    }

    fn hash(s: &str) -> u32 {
        let mut h = rustc_hash::FxHasher::default();
        h.write(s.as_bytes());
        let h = h.finish();
        (h ^ (h >> 32)) as u32
    }

    fn find(&self, h: u32, s: &str) -> Option<(Symbol, &'static str)> {
        let mask = self.slots.len() - 1;
        let mut i = h as usize & mask;
        loop {
            let (text, tag, n) = self.slots[i];
            if n == u32::MAX {
                return None;
            }
            if tag == h && text == s {
                return Some((Symbol(n), text));
            }
            i = (i + 1) & mask;
        }
    }

    /// Add `text` (not in the set) with hash `h`.
    fn insert(&mut self, h: u32, text: &'static str, sym: Symbol) {
        self.reserve(1);
        self.place((text, h, sym.0));
        self.len += 1;
    }

    fn place(&mut self, e: (&'static str, u32, u32)) {
        let mask = self.slots.len() - 1;
        let mut i = e.1 as usize & mask;
        while self.slots[i].2 != u32::MAX {
            i = (i + 1) & mask;
        }
        self.slots[i] = e;
    }

    /// Room for `extra` more strings with the table at most two thirds full.
    fn reserve(&mut self, extra: usize) {
        if (self.len + extra) * 3 <= self.slots.len() * 2 {
            return;
        }
        let mut n = self.slots.len();
        while (self.len + extra) * 3 > n * 2 {
            n *= 2;
        }
        let old = std::mem::replace(&mut self.slots, vec![EMPTY; n]);
        for e in old.into_iter().filter(|e| e.2 != u32::MAX) {
            self.place(e);
        }
    }
}

struct Interner {
    map: Index,
    strings: Vec<&'static str>,
}

impl Default for Interner {
    /// Sized for a short run's names (450 to 900 for a tiny test): the table is touched at
    /// random, so every page of a bigger table costs a page fault; a longer run grows it by
    /// rehashing.
    fn default() -> Self {
        Interner {
            map: Index::with_capacity(NAMES),
            strings: Vec::with_capacity(NAMES),
        }
    }
}

fn interner() -> &'static RwLock<Interner> {
    static I: OnceLock<RwLock<Interner>> = OnceLock::new();
    I.get_or_init(Default::default)
}

/// Names a table starts with room for.
const NAMES: usize = 1 << 10;

/// Per-thread mirror of the table: a hit or `as_str` of a known symbol takes no lock (the
/// table only grows, so a copy never goes stale, it only lacks newer entries).
struct Local {
    map: Index,
    strings: Vec<&'static str>,
}

impl Default for Local {
    /// Sized like the table, so a short run does not grow it by doubling.
    fn default() -> Self {
        Local {
            map: Index::with_capacity(NAMES),
            strings: Vec::with_capacity(NAMES),
        }
    }
}

thread_local! {
    static LOCAL: std::cell::RefCell<Local> = std::cell::RefCell::new(Local::default());
}

impl Symbol {
    /// Room for `n` more symbols, before reading values that hold that many new ones (the
    /// analysed-unit cache knows how many its entries name).
    pub fn reserve(n: usize) {
        interner()
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .map
            .reserve(n);
        LOCAL.with(|l| l.borrow_mut().map.reserve(n));
    }

    pub fn intern(s: &str) -> Symbol {
        let h = Index::hash(s);
        LOCAL.with(|l| {
            let mut l = l.borrow_mut();
            if let Some((sym, _)) = l.map.find(h, s) {
                return sym;
            }
            let (sym, text) = Self::intern_global(h, s);
            l.map.insert(h, text, sym);
            sym
        })
    }

    fn intern_global(h: u32, s: &str) -> (Symbol, &'static str) {
        if let Some(found) = interner()
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .map
            .find(h, s)
        {
            return (found.0, found.1);
        }
        let mut i = interner().write().unwrap_or_else(|e| e.into_inner());
        if let Some(found) = i.map.find(h, s) {
            return (found.0, found.1);
        }
        let s: &'static str = Box::leak(s.to_owned().into_boxed_str());
        let sym = Symbol(i.strings.len() as u32);
        i.strings.push(s);
        i.map.insert(h, s, sym);
        (sym, s)
    }

    /// The number of this symbol in the table: small and dense, for tables indexed by name.
    pub fn index(self) -> u32 {
        self.0
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
    static RAW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `f` with symbols written and read as their number in this process instead of through a
/// table: the bytes compare equal exactly when the values do, within one process (the
/// analysed-unit cache compares state before and after a unit this way). Such bytes are only
/// read back by the process that wrote them.
pub fn raw_symbols<R>(f: impl FnOnce() -> R) -> R {
    let before = RAW.with(|r| r.replace(true));
    let r = f();
    RAW.with(|r| r.set(before));
    r
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
    let before = ENC.with(|e| e.borrow_mut().replace(Default::default()));
    let raw = RAW.with(|r| r.replace(false));
    let r = f();
    RAW.with(|r| r.set(raw));
    let (_, list) = ENC
        .with(|e| std::mem::replace(&mut *e.borrow_mut(), before))
        .unwrap_or_default();
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
    /// The symbol of each text once interned, as its number plus one (0: not yet). Two threads
    /// that intern the same text get the same symbol, so a race stores the same value twice.
    syms: Vec<std::sync::atomic::AtomicU32>,
}

impl DecodeTable {
    /// The number of symbols in the table.
    pub fn len(&self) -> usize {
        self.0.syms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.syms.is_empty()
    }

    pub fn new(table: SymbolTable) -> Self {
        let syms = (0..table.ends.len()).map(|_| Default::default()).collect();
        DecodeTable(std::sync::Arc::new(Decoded { table, syms }))
    }

    fn get(&self, i: usize) -> Option<Symbol> {
        let t = &self.0.table;
        let end = *t.ends.get(i)? as usize;
        let start = if i == 0 { 0 } else { t.ends[i - 1] as usize };
        let text = t.text.get(start..end)?;
        let slot = &self.0.syms[i];
        match slot.load(std::sync::atomic::Ordering::Relaxed) {
            0 => {
                let sym = Symbol::intern(text);
                slot.store(sym.0 + 1, std::sync::atomic::Ordering::Relaxed);
                Some(sym)
            }
            n => Some(Symbol(n - 1)),
        }
    }

    /// The table of the enclosing [`DecodeTable::run`], if any.
    pub fn current() -> Option<Self> {
        DEC.with(|d| d.borrow().clone())
    }

    /// Run `f` (which deserializes values holding symbols) against this table.
    pub fn run<R>(&self, f: impl FnOnce() -> R) -> R {
        let before = DEC.with(|d| d.borrow_mut().replace(self.clone()));
        let raw = RAW.with(|r| r.replace(false));
        let r = f();
        RAW.with(|r| r.set(raw));
        DEC.with(|d| *d.borrow_mut() = before);
        r
    }
}

impl serde::Serialize for Symbol {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if RAW.with(|r| r.get()) {
            return s.serialize_u32(self.0);
        }
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
        if RAW.with(|r| r.get()) {
            return Ok(Symbol(i));
        }
        DEC.with(|t| t.borrow().as_ref().and_then(|t| t.get(i as usize)))
            .ok_or_else(|| serde::de::Error::custom("symbol outside decode_table or out of range"))
    }
}
