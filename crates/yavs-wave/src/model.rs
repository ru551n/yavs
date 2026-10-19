use std::sync::Arc;

use crate::select::Selection;

/// Kernel-side identity of a recorded value. Dense small integers are best (a table is indexed
/// by it). Declaring the same id twice makes the second declaration an alias.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SignalId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    To,
    Downto,
}

/// A VHDL range as written: `left to right` or `left downto right`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
#[derive(Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug)]
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

#[derive(Clone, Debug)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug)]
pub struct Signal {
    pub name: String,
    pub id: SignalId,
    pub ty: SigType,
    pub port: PortDir,
    pub class: ObjClass,
}

#[derive(Clone, Debug)]
pub struct Scope {
    pub name: String,
    pub kind: ScopeKind,
    /// Entity/architecture/package name for instances, empty otherwise.
    pub component: String,
    pub children: Vec<Node>,
}

#[derive(Clone, Debug)]
pub enum Node {
    Scope(Scope),
    Signal(Signal),
}

/// An array or memory that was left out because it exceeded the size cap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    pub path: String,
    pub elems: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Hierarchy {
    pub roots: Vec<Node>,
    pub skipped: Vec<Skipped>,
}

struct Frame {
    /// `None`: this subtree is dropped (excluded or over the cap).
    scope: Option<Scope>,
    path: String,
}

/// Builds a [`Hierarchy`], applying the [`Selection`] as nodes are declared.
pub struct HierarchyBuilder {
    sel: Selection,
    roots: Vec<Node>,
    stack: Vec<Frame>,
    skipped: Vec<Skipped>,
}

impl HierarchyBuilder {
    pub fn new(sel: Selection) -> Self {
        Self {
            sel,
            roots: Vec::new(),
            stack: Vec::new(),
            skipped: Vec::new(),
        }
    }

    fn path_of(&self, name: &str) -> String {
        match self.stack.last() {
            Some(f) if !f.path.is_empty() => format!("{}.{name}", f.path),
            _ => name.to_string(),
        }
    }

    fn dropped(&self) -> bool {
        self.stack.last().is_some_and(|f| f.scope.is_none())
    }

    /// Open a scope. Always pair with [`Self::up`], also when the scope is dropped by the selection.
    pub fn scope(&mut self, name: &str, kind: ScopeKind, component: &str) {
        let path = self.path_of(name);
        let keep = !self.dropped() && !self.sel.excluded(&path);
        let scope = keep.then(|| Scope {
            name: name.into(),
            kind,
            component: component.into(),
            children: Vec::new(),
        });
        self.stack.push(Frame { scope, path });
    }

    /// Open an array scope of `len` elements; returns false if it is dropped (over the cap, or
    /// excluded), in which case the caller should skip declaring the elements. Pair with `up`.
    pub fn array(&mut self, name: &str, len: u64) -> bool {
        self.scope(name, ScopeKind::Array, "");
        let f = self.stack.last_mut().expect("just pushed");
        if f.scope.is_some() && len > self.sel.max_array_elems as u64 {
            f.scope = None;
            let path = f.path.clone();
            self.skipped.push(Skipped { path, elems: len });
        }
        self.stack.last().is_some_and(|f| f.scope.is_some())
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
        if self.dropped() {
            return false;
        }
        let path = self.path_of(name);
        if !self.sel.included(&path) || self.sel.excluded(&path) {
            return false;
        }
        if let SigType::Vector { bounds, .. } = &ty
            && bounds.len() > self.sel.max_array_elems as u64
        {
            self.skipped.push(Skipped {
                path,
                elems: bounds.len(),
            });
            return false;
        }
        let sig = Signal {
            name: name.into(),
            id,
            ty,
            port,
            class,
        };
        self.push(Node::Signal(sig));
        true
    }

    fn push(&mut self, node: Node) {
        match self.stack.last_mut() {
            Some(Frame { scope: Some(s), .. }) => s.children.push(node),
            Some(_) => {}
            None => self.roots.push(node),
        }
    }

    /// Close the innermost scope. Scopes left without any signal are removed.
    pub fn up(&mut self) {
        let f = self.stack.pop().expect("up without matching scope");
        if let Some(s) = f.scope.filter(|s| !s.children.is_empty()) {
            self.push(Node::Scope(s));
        }
    }

    pub fn finish(mut self) -> Hierarchy {
        while !self.stack.is_empty() {
            self.up();
        }
        Hierarchy {
            roots: self.roots,
            skipped: self.skipped,
        }
    }
}
