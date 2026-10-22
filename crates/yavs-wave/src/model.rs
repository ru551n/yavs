use std::collections::HashMap;
use std::sync::Arc;

use crate::select::Selection;

/// Kernel-side identity of a recorded value. Dense small integers are best (a table is indexed
/// by it). Declaring the same id twice makes the second declaration an alias.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SignalId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dir {
    To,
    Downto,
}

/// A VHDL range as written: `left to right` or `left downto right`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Bounds {
    pub left: i64,
    pub right: i64,
    pub dir: Dir,
}

impl Bounds {
    pub fn len(&self) -> u64 {
        let n = match self.dir {
            Dir::To => self.right - self.left + 1,
            Dir::Downto => self.left - self.right + 1,
        };
        n.max(0) as u64
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// An enumeration type with literal names in original spelling, e.g. `'U'`, `IDLE`.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct EnumType {
    pub name: String,
    pub literals: Vec<String>,
}

impl EnumType {
    pub fn new(name: &str, literals: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            name: name.into(),
            literals: literals.iter().map(|s| s.to_string()).collect(),
        })
    }

    /// `std_ulogic`: `U X 0 1 Z W L H -`.
    pub fn std_ulogic() -> Arc<Self> {
        Self::new(
            "std_ulogic",
            &[
                "'U'", "'X'", "'0'", "'1'", "'Z'", "'W'", "'L'", "'H'", "'-'",
            ],
        )
    }

    /// `std_logic` (the resolved subtype, same literals).
    pub fn std_logic() -> Arc<Self> {
        Self::new(
            "std_logic",
            &[
                "'U'", "'X'", "'0'", "'1'", "'Z'", "'W'", "'L'", "'H'", "'-'",
            ],
        )
    }

    pub fn bit() -> Arc<Self> {
        Self::new("bit", &["'0'", "'1'"])
    }

    pub fn boolean() -> Arc<Self> {
        Self::new("boolean", &["false", "true"])
    }

    /// FST value characters per position when every literal is a 9-value logic character
    /// (`std_logic` family, `bit`). Such types are written as bits, so viewers show `U`, `X`, ...
    pub(crate) fn logic_chars(&self) -> Option<Vec<u8>> {
        self.literals
            .iter()
            .map(|l| {
                let c = l.trim_matches('\'');
                match c.as_bytes() {
                    [b] if b"UXZWLH01-uxzwlh".contains(b) && c.len() < l.len() => {
                        Some(b.to_ascii_lowercase())
                    }
                    _ => None,
                }
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ScalarType {
    Enum(Arc<EnumType>),
    /// `bits` is the width of the two's complement encoding (1..=64).
    Integer {
        name: String,
        bits: u32,
    },
    Real,
    Time,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SigType {
    Scalar(ScalarType),
    /// One-dimensional array of a logic-like enum (bit/std_logic): a single multi-bit signal.
    Vector {
        elem: Arc<EnumType>,
        bounds: Bounds,
        type_name: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortDir {
    None,
    In,
    Out,
    InOut,
    Buffer,
    Linkage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjClass {
    Signal,
    Variable,
    Constant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeKind {
    /// Entity instance (architecture).
    Instance,
    Block,
    Process,
    Procedure,
    Function,
    ForGenerate,
    IfGenerate,
    Generate,
    Package,
    Record,
    Array,
}

/// A string in `Hierarchy::names`.
#[derive(Clone, Copy, Debug)]
struct Str {
    at: u32,
    len: u32,
}

/// One step of the depth-first walk of the hierarchy, as stored.
#[derive(Clone, Copy, Debug)]
enum Item {
    Scope {
        name: Str,
        comp: Str,
        kind: ScopeKind,
    },
    Signal {
        name: Str,
        id: SignalId,
        /// Index into `Hierarchy::types`.
        ty: u32,
        port: PortDir,
        class: ObjClass,
    },
    /// Closes the innermost open scope.
    Up,
}

/// One step of [`Hierarchy::items`].
#[derive(Clone, Copy, Debug)]
pub enum HierItem<'a> {
    /// Opens a scope; its content follows, up to the matching [`HierItem::Up`].
    Scope {
        name: &'a str,
        kind: ScopeKind,
        /// Entity/architecture/package name for instances, empty otherwise.
        component: &'a str,
    },
    Signal {
        name: &'a str,
        id: SignalId,
        ty: &'a SigType,
        /// Index of `ty` in [`Hierarchy::types`]: equal types have equal indices.
        ty_index: usize,
        port: PortDir,
        class: ObjClass,
    },
    Up,
}

/// An array or memory that was left out because it exceeded the size cap (`elems`), or a
/// variable whose bounds are known only when the simulation runs (`None`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    pub path: String,
    pub elems: Option<u64>,
}

/// The declared design as a flat depth-first list (a design can have millions of signals):
/// scopes, signals and the ends of scopes, with names in one string and the signal types
/// shared. Build it with [`HierarchyBuilder`], walk it with [`Hierarchy::items`].
#[derive(Clone, Debug, Default)]
pub struct Hierarchy {
    items: Vec<Item>,
    names: String,
    types: Vec<Arc<SigType>>,
    signals: usize,
    pub skipped: Vec<Skipped>,
}

impl Hierarchy {
    /// The number of signals declared.
    pub fn signals(&self) -> usize {
        self.signals
    }

    fn text(&self, s: Str) -> &str {
        &self.names[s.at as usize..(s.at + s.len) as usize]
    }

    /// Scopes and signals depth first, each scope closed by [`HierItem::Up`].
    pub fn items(&self) -> impl Iterator<Item = HierItem<'_>> {
        self.items.iter().map(|it| match *it {
            Item::Scope { name, comp, kind } => HierItem::Scope {
                name: self.text(name),
                kind,
                component: self.text(comp),
            },
            Item::Signal {
                name,
                id,
                ty,
                port,
                class,
            } => HierItem::Signal {
                name: self.text(name),
                id,
                ty: &self.types[ty as usize],
                ty_index: ty as usize,
                port,
                class,
            },
            Item::Up => HierItem::Up,
        })
    }

    /// The distinct signal types, by `ty_index`.
    pub fn types(&self) -> &[Arc<SigType>] {
        &self.types
    }
}

struct Frame {
    /// `None`: this subtree is dropped (excluded or over the cap). Else the lengths of
    /// `items` and `names` before the scope was declared, to take it back if it stays empty.
    start: Option<(usize, usize)>,
    /// The scope's name, for the paths of what is declared below it.
    name: Box<str>,
}

/// Builds a [`Hierarchy`], applying the [`Selection`] as nodes are declared.
pub struct HierarchyBuilder {
    sel: Selection,
    /// The selection has patterns, so every declared path is matched.
    globs: bool,
    h: Hierarchy,
    stack: Vec<Frame>,
    /// Type indices by value, and by address for the types handed in as `Arc`s (kept alive
    /// here, so an address is not reused for another type).
    by_value: HashMap<Arc<SigType>, u32>,
    by_addr: HashMap<*const SigType, (u32, Arc<SigType>)>,
}

impl HierarchyBuilder {
    pub fn new(sel: Selection) -> Self {
        Self {
            globs: !sel.include.is_empty() || !sel.exclude.is_empty(),
            sel,
            h: Hierarchy::default(),
            stack: Vec::new(),
            by_value: HashMap::new(),
            by_addr: HashMap::new(),
        }
    }

    /// The dotted path of `name` in the current scope.
    fn path_of(&self, name: &str) -> String {
        let mut p = String::new();
        for f in &self.stack {
            p.push_str(&f.name);
            p.push('.');
        }
        p.push_str(name);
        p
    }

    fn dropped(&self) -> bool {
        self.stack.last().is_some_and(|f| f.start.is_none())
    }

    fn put(&mut self, text: &str) -> Str {
        let at = self.h.names.len() as u32;
        self.h.names.push_str(text);
        Str {
            at,
            len: text.len() as u32,
        }
    }

    /// Open a scope. Always pair with [`Self::up`], also when the scope is dropped by the selection.
    pub fn scope(&mut self, name: &str, kind: ScopeKind, component: &str) {
        let keep = !self.dropped() && !(self.globs && self.sel.excluded(&self.path_of(name)));
        let start = keep.then(|| {
            let start = (self.h.items.len(), self.h.names.len());
            let (name, comp) = (self.put(name), self.put(component));
            self.h.items.push(Item::Scope { name, comp, kind });
            start
        });
        self.stack.push(Frame {
            start,
            name: name.into(),
        });
    }

    /// Open an array scope of `len` elements; returns false if it is dropped (over the cap, or
    /// excluded), in which case the caller should skip declaring the elements. Pair with `up`.
    pub fn array(&mut self, name: &str, len: u64) -> bool {
        self.scope(name, ScopeKind::Array, "");
        let f = self.stack.last_mut().expect("just pushed");
        if let Some((items, names)) = f.start
            && len > self.sel.max_array_elems as u64
        {
            f.start = None;
            self.h.items.truncate(items);
            self.h.names.truncate(names);
            let path = self.path_of_open();
            self.h.skipped.push(Skipped {
                path,
                elems: Some(len),
            });
        }
        !self.dropped()
    }

    /// The dotted path of the innermost open scope.
    fn path_of_open(&self) -> String {
        let names: Vec<&str> = self.stack.iter().map(|f| &*f.name).collect();
        names.join(".")
    }

    /// The index of `ty` among the hierarchy's types.
    fn intern(&mut self, ty: &Arc<SigType>) -> u32 {
        if let Some((i, _)) = self.by_addr.get(&Arc::as_ptr(ty)) {
            return *i;
        }
        let i = match self.by_value.get(ty) {
            Some(&i) => i,
            None => {
                let i = self.h.types.len() as u32;
                self.h.types.push(ty.clone());
                self.by_value.insert(ty.clone(), i);
                i
            }
        };
        self.by_addr.insert(Arc::as_ptr(ty), (i, ty.clone()));
        i
    }

    /// Declare a signal; returns false if the selection or the size cap drops it, so the caller
    /// need not record its values.
    pub fn signal(
        &mut self,
        name: &str,
        id: SignalId,
        ty: SigType,
        port: PortDir,
        class: ObjClass,
    ) -> bool {
        self.signal_shared(name, id, &Arc::new(ty), port, class)
    }

    /// [`Self::signal`] for a type shared by many signals: declaring them is cheaper when
    /// they hand in the same `Arc`.
    pub fn signal_shared(
        &mut self,
        name: &str,
        id: SignalId,
        ty: &Arc<SigType>,
        port: PortDir,
        class: ObjClass,
    ) -> bool {
        if self.dropped() {
            return false;
        }
        if self.globs {
            let path = self.path_of(name);
            if !self.sel.included(&path) || self.sel.excluded(&path) {
                return false;
            }
        }
        if let SigType::Vector { bounds, .. } = &**ty
            && bounds.len() > self.sel.max_array_elems as u64
        {
            let path = self.path_of(name);
            self.h.skipped.push(Skipped {
                path,
                elems: Some(bounds.len()),
            });
            return false;
        }
        let ty = self.intern(ty);
        let name = self.put(name);
        self.h.signals += 1;
        self.h.items.push(Item::Signal {
            name,
            id,
            ty,
            port,
            class,
        });
        true
    }

    /// A variable `name` that is left out because its bounds are not known before the
    /// simulation runs; listed in [`Hierarchy::skipped`] when the selection keeps it.
    pub fn unknown(&mut self, name: &str) {
        if self.dropped() {
            return;
        }
        let path = self.path_of(name);
        if self.sel.included(&path) && !self.sel.excluded(&path) {
            self.h.skipped.push(Skipped { path, elems: None });
        }
    }

    /// Close the innermost scope. Scopes left without any signal are removed.
    pub fn up(&mut self) {
        let f = self.stack.pop().expect("up without matching scope");
        if let Some((items, names)) = f.start {
            if self.h.items.len() == items + 1 {
                self.h.items.truncate(items);
                self.h.names.truncate(names);
            } else {
                self.h.items.push(Item::Up);
            }
        }
    }

    pub fn finish(mut self) -> Hierarchy {
        while !self.stack.is_empty() {
            self.up();
        }
        self.h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int() -> SigType {
        SigType::Scalar(ScalarType::Integer {
            name: "integer".into(),
            bits: 32,
        })
    }

    fn shape(h: &Hierarchy) -> Vec<String> {
        h.items()
            .map(|i| match i {
                HierItem::Scope { name, .. } => format!("+{name}"),
                HierItem::Signal { name, ty_index, .. } => format!("{name}:{ty_index}"),
                HierItem::Up => "-".into(),
            })
            .collect()
    }

    #[test]
    fn empty_scopes_and_capped_arrays_leave_nothing() {
        let sel = Selection {
            max_array_elems: 2,
            ..Selection::default()
        };
        let mut b = HierarchyBuilder::new(sel);
        let (cls, port) = (ObjClass::Signal, PortDir::None);
        b.scope("top", ScopeKind::Instance, "e(a)");
        b.scope("empty", ScopeKind::Block, "");
        b.scope("inner", ScopeKind::Block, "");
        b.up();
        b.up();
        assert!(!b.array("big", 3), "over the cap");
        b.up();
        assert!(b.array("small", 2));
        assert!(b.signal("a", SignalId(0), int(), port, cls));
        b.up();
        assert!(b.signal("b", SignalId(1), int(), port, cls));
        b.up();
        let h = b.finish();
        // Equal types share one index; the big array is listed as skipped.
        assert_eq!(shape(&h), ["+top", "+small", "a:0", "-", "b:0", "-"]);
        assert_eq!(h.types().len(), 1);
        assert_eq!(
            h.skipped,
            [Skipped {
                path: "top.big".into(),
                elems: Some(3)
            }]
        );
    }

    #[test]
    fn globs_drop_whole_scopes() {
        let sel = Selection {
            exclude: vec!["top.dbg".into()],
            ..Selection::default()
        };
        let mut b = HierarchyBuilder::new(sel);
        b.scope("top", ScopeKind::Instance, "");
        b.scope("dbg", ScopeKind::Block, "");
        assert!(!b.signal("x", SignalId(0), int(), PortDir::None, ObjClass::Signal));
        b.up();
        assert!(b.signal("y", SignalId(1), int(), PortDir::None, ObjClass::Signal));
        let h = b.finish();
        assert_eq!(shape(&h), ["+top", "y:0", "-"]);
    }
}
