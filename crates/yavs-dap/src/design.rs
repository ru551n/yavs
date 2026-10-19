//! What the debugger shows of the elaborated design, extracted once before the run so the
//! server owns it: statement locations, the line table for breakpoints, and per process the
//! signals of its enclosing regions and the generics of its instance.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use yavs_elab::{Elaborated, ObjKind, ScopeId, Shape};
use yavs_ir::{Program, StmtId};
use yavs_kernel::{Elem, SigRef, SignalId, Value};
use yavs_sem::Db;
use yavs_sem::decl::{DeclKind, Designator, ObjRole};
use yavs_sem::stat::{ConstValue, Elems};
use yavs_sem::types::TypeKind;

/// A source position, 1-based.
#[derive(Clone)]
pub(crate) struct Loc {
    pub path: String,
    pub line: u32,
    pub col: u32,
}

/// How a signal's kernel leaves become text.
enum Fmt {
    /// A logic leaf element: code minus `base` indexes the literals.
    Logic(Arc<[String]>, u8),
    /// A byte leaf element of this many bytes.
    Bytes(u32, Scalar),
    /// `n` elements in row-major order; `chars` when the elements are character literals.
    Array(u64, Box<Fmt>, bool),
    Record(Vec<(String, Fmt)>),
}

enum Scalar {
    Enum(Arc<[String]>),
    Int,
    Real,
    /// In base units, named.
    Phys(String),
}

impl Fmt {
    fn leaves(&self) -> usize {
        match self {
            Fmt::Logic(..) | Fmt::Bytes(..) => 1,
            Fmt::Array(_, e, _) => e.leaves(),
            Fmt::Record(fs) => fs.iter().map(|(_, f)| f.leaves()).sum(),
        }
    }

    /// Element `k` (row major among all elements this format repeats over) of `vals`, the
    /// values of its leaves.
    fn show(&self, vals: &[Option<Value>], k: u64) -> String {
        const MAX: u64 = 256;
        let unknown = || "?".to_string();
        match self {
            Fmt::Logic(lits, base) => match vals.first() {
                // Kernel offset 0 is the rightmost element (yavs-elab `NetGraph::select`).
                Some(Some(Value::Logic(l))) => (l.len() as u64)
                    .checked_sub(k + 1)
                    .and_then(|i| {
                        let code = (l.get(i as usize) as u8).checked_sub(*base)?;
                        lits.get(code as usize).cloned()
                    })
                    .unwrap_or_else(unknown),
                _ => unknown(),
            },
            Fmt::Bytes(size, sc) => match vals.first() {
                Some(Some(Value::Bytes(b))) => {
                    let at = k as usize * *size as usize;
                    b.get(at..at + *size as usize)
                        .map_or_else(unknown, |e| sc.show(e))
                }
                _ => unknown(),
            },
            Fmt::Array(n, e, chars) => {
                let items: Vec<String> = (0..(*n).min(MAX))
                    .map(|j| e.show(vals, k * n + j))
                    .collect();
                let more = if *n > MAX { ", ..." } else { "" };
                if *chars && items.iter().all(|s| s.len() == 3) && *n <= MAX {
                    let s: String = items.iter().filter_map(|s| s.chars().nth(1)).collect();
                    format!("\"{s}\"")
                } else {
                    format!("({}{more})", items.join(", "))
                }
            }
            Fmt::Record(fs) => {
                let mut l = 0;
                let items: Vec<String> = fs
                    .iter()
                    .map(|(name, f)| {
                        let t = f.show(vals.get(l..).unwrap_or(&[]), k);
                        l += f.leaves();
                        format!("{name} => {t}")
                    })
                    .collect();
                format!("({})", items.join(", "))
            }
        }
    }
}

impl Scalar {
    fn show(&self, b: &[u8]) -> String {
        let mut a = [0u8; 8];
        let n = b.len().min(8);
        a[..n].copy_from_slice(&b[..n]);
        let v = u64::from_le_bytes(a);
        match self {
            Scalar::Enum(lits) => lits
                .get(v as usize)
                .cloned()
                .unwrap_or_else(|| v.to_string()),
            Scalar::Int => (v as i64).to_string(),
            Scalar::Real => f64::from_bits(v).to_string(),
            Scalar::Phys(u) => format!("{} {u}", v as i64),
        }
    }
}

/// A signal or port.
pub(crate) struct Sig {
    pub name: String,
    pub ty: String,
    leaf: u32,
    leaves: u32,
    fmt: Fmt,
}

impl Sig {
    /// The value as text, read through `read` (`None`: not readable here).
    pub fn show(&self, read: &dyn Fn(SigRef) -> Option<Value>) -> String {
        let vals: Vec<Option<Value>> = (self.leaf..self.leaf + self.leaves)
            .map(|l| read(SignalId(l).into()))
            .collect();
        self.fmt.show(&vals, 0)
    }
}

/// What one process sees.
#[derive(Default)]
pub(crate) struct Proc {
    /// Path prefixes (`:top:u1:`) of its enclosing regions up to its instance, innermost
    /// first, for name lookup.
    regions: Vec<String>,
    /// Indices into `Design::sigs`.
    pub signals: Vec<usize>,
    /// Name and value of each generic of its instance.
    pub generics: Vec<(String, String)>,
}

pub(crate) struct Design {
    /// Per statement id.
    pub locs: Vec<Option<Loc>>,
    /// File key, then line: the outermost statements starting on that line.
    lines: HashMap<String, BTreeMap<u32, Vec<StmtId>>>,
    pub sigs: Vec<Sig>,
    /// Lower-case path (`:top:u1:s`) to `sigs` index.
    by_path: HashMap<String, usize>,
    /// By process name ([`Elaborated::proc_name`]).
    procs: HashMap<String, Proc>,
}

/// A file path as the client sees it: absolute.
pub(crate) fn absolute(p: &str) -> String {
    std::path::absolute(p).map_or_else(|_| p.to_string(), |c| c.display().to_string())
}

/// The key a breakpoint's file is matched by. Clients differ in the case of Windows drive
/// letters.
fn file_key(p: &str) -> String {
    let a = absolute(p);
    if cfg!(windows) { a.to_lowercase() } else { a }
}

impl Design {
    pub fn new(db: &Db, e: &Elaborated, prog: &Program) -> Design {
        let mut paths: HashMap<yavs_diag::FileId, String> = HashMap::new();
        let mut locs = Vec::with_capacity(prog.stmts.len());
        for (_, st) in prog.stmts.iter() {
            let f = db.sources.get(st.span.file);
            let path = paths
                .entry(st.span.file)
                .or_insert_with(|| absolute(&f.path))
                .clone();
            let (line, col) = f.line_col(st.span.start);
            locs.push(Some(Loc {
                path,
                line: line as u32,
                col: col as u32,
            }));
        }
        let mut lines: HashMap<String, BTreeMap<u32, Vec<StmtId>>> = HashMap::new();
        for (id, st) in prog.stmts.iter() {
            let Some(l) = &locs[id.0 as usize] else {
                continue;
            };
            // A statement nested in one that starts on the same line stops with it.
            let nested = st
                .parent
                .and_then(|p| locs.get(p.0 as usize))
                .and_then(Option::as_ref);
            if nested.is_some_and(|p| p.path == l.path && p.line == l.line) {
                continue;
            }
            lines
                .entry(file_key(&l.path))
                .or_default()
                .entry(l.line)
                .or_default()
                .push(id);
        }
        let mut d = Design {
            locs,
            lines,
            sigs: Vec::new(),
            by_path: HashMap::new(),
            procs: HashMap::new(),
        };
        let mut scope_sigs: HashMap<ScopeId, Vec<usize>> = HashMap::new();
        for (i, sc) in e.hier.scopes.iter().enumerate() {
            let s = ScopeId(i as u32);
            let prefix = e.hier.path_name(s, yavs_elab::Case::Lrm);
            for &o in &sc.objs {
                let obj = e.nets.obj(o);
                if !matches!(obj.kind, ObjKind::Signal | ObjKind::Port(_)) {
                    continue;
                }
                let k = d.sigs.len();
                d.sigs.push(Sig {
                    name: obj.name.text.to_string(),
                    ty: db.types[obj.shape.ty()].name.text.to_string(),
                    leaf: obj.leaves.0.0,
                    leaves: obj.leaves.1.0 - obj.leaves.0.0,
                    fmt: fmt(db, &obj.shape),
                });
                d.by_path
                    .insert(format!("{prefix}{}", obj.name.key).to_ascii_lowercase(), k);
                scope_sigs.entry(s).or_default().push(k);
            }
        }
        for p in &e.procs {
            let inst = e.instance_of(p.scope);
            let mut pr = Proc::default();
            let mut cur = e.hier.scope(p.scope).parent;
            while let Some(s) = cur {
                pr.regions.push(e.hier.path_name(s, yavs_elab::Case::Lrm));
                pr.signals
                    .extend(scope_sigs.get(&s).into_iter().flatten().copied());
                cur = if s == inst {
                    None
                } else {
                    e.hier.scope(s).parent
                };
            }
            for r in e.regions.get(&inst).into_iter().flatten() {
                for &decl in &db.scopes[*r].order {
                    let DeclKind::Object(o) = &db.decls[decl].kind else {
                        continue;
                    };
                    if o.role != ObjRole::Generic {
                        continue;
                    }
                    let v = e.value(inst, decl).map_or_else(
                        || "?".to_string(),
                        |v| const_text(db, db.base(o.subtype), v),
                    );
                    pr.generics.push((db.decls[decl].name.to_string(), v));
                }
            }
            d.procs.insert(e.proc_name(p), pr);
        }
        d
    }

    pub fn loc(&self, s: StmtId) -> Option<&Loc> {
        self.locs.get(s.0 as usize)?.as_ref()
    }

    /// The statements a breakpoint on `line` of `path` stops at: those on the first line at or
    /// after it that has any, and that line.
    pub fn line_stmts(&self, path: &str, line: u32) -> Option<(u32, &[StmtId])> {
        let (&l, ids) = self.lines.get(&file_key(path))?.range(line..).next()?;
        Some((l, ids))
    }

    pub fn proc(&self, name: &str) -> Option<&Proc> {
        self.procs.get(name)
    }

    /// A signal by simple name seen from process `p`, or by path (`:top:u1:s`, `top.u1.s`).
    pub fn lookup(&self, p: Option<&Proc>, name: &str) -> Option<&Sig> {
        let n = name.trim().to_ascii_lowercase();
        let path = if n.starts_with(':') {
            n.clone()
        } else {
            format!(":{}", n.replace('.', ":"))
        };
        let local = p
            .into_iter()
            .flat_map(|p| &p.regions)
            .find_map(|r| self.by_path.get(&format!("{r}{n}")));
        local
            .or_else(|| self.by_path.get(&path))
            .map(|&k| &self.sigs[k])
    }
}

fn lit_names(db: &Db, lits: &[yavs_sem::DeclId]) -> Arc<[String]> {
    lits.iter()
        .map(|&l| match db.decls[l].name {
            Designator::Char(c) => format!("'{c}'"),
            Designator::Ident(i) => i.text.to_string(),
            Designator::Operator { text, .. } => text.to_string(),
        })
        .collect()
}

fn scalar(db: &Db, t: yavs_sem::TypeId) -> Scalar {
    match &db.types[t].kind {
        TypeKind::Enum { lits } => Scalar::Enum(lit_names(db, lits)),
        TypeKind::Floating | TypeKind::UniversalReal => Scalar::Real,
        TypeKind::Physical { units } => match units.first() {
            Some(u) => Scalar::Phys(db.decls[u.decl].name.to_string()),
            None => Scalar::Int,
        },
        _ => Scalar::Int,
    }
}

fn fmt(db: &Db, shape: &Shape) -> Fmt {
    match shape {
        Shape::Scalar { ty, elem, .. } => match (elem, scalar(db, *ty)) {
            (Elem::Logic, Scalar::Enum(lits)) => {
                let wk = &db.well_known;
                let base = if Some(*ty) == wk.boolean || Some(*ty) == wk.bit {
                    2
                } else {
                    0
                };
                Fmt::Logic(lits, base)
            }
            (Elem::Bytes(n), sc) => Fmt::Bytes(*n, sc),
            // A logic leaf is always an enumeration.
            (Elem::Logic, sc) => Fmt::Bytes(8, sc),
        },
        Shape::Array { elem, .. } => {
            let e = fmt(db, elem);
            let chars = match &e {
                Fmt::Logic(l, _) | Fmt::Bytes(_, Scalar::Enum(l)) => {
                    l.iter().all(|s| s.len() == 3 && s.starts_with('\''))
                }
                _ => false,
            };
            Fmt::Array(shape.len(), Box::new(e), chars)
        }
        Shape::Record { fields, .. } => Fmt::Record(
            fields
                .iter()
                .map(|(n, f)| (n.text.to_string(), fmt(db, f)))
                .collect(),
        ),
    }
}

/// An elaboration-time value of base type `t` as text.
fn const_text(db: &Db, t: yavs_sem::TypeId, v: &ConstValue) -> String {
    match (&db.types[t].kind, v) {
        (TypeKind::Enum { lits }, ConstValue::Enum(p)) => lit_names(db, lits)
            .get(*p as usize)
            .cloned()
            .unwrap_or_else(|| p.to_string()),
        (TypeKind::Physical { units }, ConstValue::Phys(n)) => match units.first() {
            Some(u) => format!("{n} {}", db.decls[u.decl].name),
            None => n.to_string(),
        },
        (_, ConstValue::Int(n) | ConstValue::Phys(n)) => n.to_string(),
        (_, ConstValue::Real(x)) => x.to_string(),
        (TypeKind::Array { elem, .. }, ConstValue::Array(a)) => {
            let et = db.base(*elem);
            match (&db.types[et].kind, &a.elems) {
                (TypeKind::Enum { lits }, Elems::Bytes(b)) => {
                    let names = lit_names(db, lits);
                    let items: Vec<String> = b
                        .iter()
                        .map(|&p| {
                            names
                                .get(p as usize)
                                .cloned()
                                .unwrap_or_else(|| p.to_string())
                        })
                        .collect();
                    if items.iter().all(|s| s.len() == 3 && s.starts_with('\'')) {
                        let s: String = items.iter().filter_map(|s| s.chars().nth(1)).collect();
                        format!("\"{s}\"")
                    } else {
                        format!("({})", items.join(", "))
                    }
                }
                // ponytail: run-length arrays and arrays of composites are not shown yet.
                _ => "(...)".to_string(),
            }
        }
        (_, ConstValue::Null) => "null".to_string(),
        _ => "(...)".to_string(),
    }
}
