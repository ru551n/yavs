//! What the debugger shows of the elaborated design, extracted once before the run so the
//! server owns it: statement locations, the line table for breakpoints, and per process the
//! signals of its enclosing regions and the generics of its instance.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use yavs_elab::{Elaborated, ObjKind, ScopeId, ScopeKind, Shape};
use yavs_ir::{Program, StmtId, Ty, TyId};
use yavs_kernel::{Elem, LogicVec, Range, SigRef, SignalId, Value};
use yavs_logic::StdULogic;
use yavs_sem::Db;
use yavs_sem::decl::{DeclKind, Designator, ObjRole};
use yavs_sem::stat::{ConstValue, Elems};
use yavs_sem::types::{Direction, TypeKind};
use yavs_std::types::{
    Bounds as StdBounds, EnumLit, EnumType, PhysicalType, RealBounds, Unit, to_latin1,
};
use yavs_std::value::{Lit, LitType, literal, literal_image};

/// A source position, 1-based.
#[derive(Clone)]
pub(crate) struct Loc {
    pub path: String,
    pub line: u32,
    pub col: u32,
}

/// Where a signal's kernel leaves hold its value; walked together with its [`LitType`].
enum Fmt {
    /// A logic leaf element: code minus the base is the literal position.
    Logic(u8),
    /// A byte leaf element of this many bytes.
    Bytes(u32),
    /// `n` elements in row-major order.
    Array(u64, Box<Fmt>),
    Record(Vec<Fmt>),
}

impl Fmt {
    fn leaves(&self) -> usize {
        match self {
            Fmt::Logic(_) | Fmt::Bytes(_) => 1,
            Fmt::Array(_, e) => e.leaves(),
            Fmt::Record(fs) => fs.iter().map(Fmt::leaves).sum(),
        }
    }

    /// Element `k` (row major among all elements this format repeats over) of `vals`, the
    /// values of its leaves.
    fn decode(&self, ty: &LitType, vals: &[Option<Value>], k: u64) -> Option<Lit> {
        Some(match (self, ty) {
            // Kernel offset 0 is the rightmost element (yavs-elab `NetGraph::select`).
            (Fmt::Logic(base), _) => {
                let Some(Value::Logic(l)) = vals.first()? else {
                    return None;
                };
                let i = (l.len() as u64).checked_sub(k + 1)?;
                Lit::Enum((l.get(i as usize) as u8).checked_sub(*base)? as usize)
            }
            (Fmt::Bytes(size), _) => {
                let Some(Value::Bytes(b)) = vals.first()? else {
                    return None;
                };
                let at = k as usize * *size as usize;
                let e = b.get(at..at + *size as usize)?;
                // A 4-byte cell is a narrow integer array element (sign-extended) or a
                // non-negative enumeration position.
                let v = match <[u8; 4]>::try_from(e) {
                    Ok(x) => i64::from(i32::from_le_bytes(x)) as u64,
                    Err(_) => {
                        let mut a = [0u8; 8];
                        let n = e.len().min(8);
                        a[..n].copy_from_slice(&e[..n]);
                        u64::from_le_bytes(a)
                    }
                };
                match ty {
                    LitType::Enum(_) => Lit::Enum(v as usize),
                    LitType::Real(_) => Lit::Real(f64::from_bits(v)),
                    LitType::Phys(..) => Lit::Phys(v as i64),
                    _ => Lit::Int(v as i64),
                }
            }
            (Fmt::Array(n, e), LitType::Array { elem, .. }) => Lit::Array(
                (0..*n)
                    .map(|j| e.decode(elem, vals, k * n + j))
                    .collect::<Option<_>>()?,
            ),
            (Fmt::Record(fs), LitType::Record(ts)) => {
                let mut l = 0;
                let mut out = Vec::with_capacity(fs.len());
                for (f, (_, t)) in fs.iter().zip(ts) {
                    out.push(f.decode(t, vals.get(l..)?, k)?);
                    l += f.leaves();
                }
                Lit::Record(out)
            }
            _ => return None,
        })
    }

    /// The (leaf, element) pairs of sub-element `path` (field and element positions) of
    /// element `k`, the leaves counted from `leaf`.
    fn part(&self, path: &[usize], k: u64, leaf: usize, out: &mut Vec<(usize, u64)>) -> Option<()> {
        match (self, path.split_first()) {
            (Fmt::Logic(_) | Fmt::Bytes(_), None) => out.push((leaf, k)),
            (Fmt::Array(n, e), None) => {
                for j in 0..*n {
                    e.part(&[], k * n + j, leaf, out)?;
                }
            }
            (Fmt::Array(n, e), Some((&j, rest))) if (j as u64) < *n => {
                e.part(rest, k * n + j as u64, leaf, out)?
            }
            (Fmt::Record(fs), None) => {
                let mut l = leaf;
                for f in fs {
                    f.part(&[], k, l, out)?;
                    l += f.leaves();
                }
            }
            (Fmt::Record(fs), Some((&i, rest))) => {
                let l = leaf + fs.get(..i)?.iter().map(Fmt::leaves).sum::<usize>();
                fs.get(i)?.part(rest, k, l, out)?
            }
            _ => return None,
        }
        Some(())
    }

    /// Fresh leaf values for `reps` repetitions of this format.
    fn alloc(&self, reps: u64, out: &mut Vec<Value>) {
        match self {
            Fmt::Logic(_) => out.push(Value::Logic(LogicVec::new(reps as usize, StdULogic::U))),
            Fmt::Bytes(size) => out.push(Value::Bytes(vec![0; (reps * u64::from(*size)) as usize])),
            Fmt::Array(n, e) => e.alloc(reps * n, out),
            Fmt::Record(fs) => fs.iter().for_each(|f| f.alloc(reps, out)),
        }
    }

    /// Writes `v` as element `k` into `vals` (the inverse of [`Fmt::decode`]).
    fn encode(&self, v: &Lit, vals: &mut [Value], k: u64) -> Option<()> {
        match (self, v) {
            (Fmt::Logic(base), Lit::Enum(p)) => {
                let Value::Logic(l) = vals.first_mut()? else {
                    return None;
                };
                let i = (l.len() as u64).checked_sub(k + 1)? as usize;
                let code = u8::try_from(*p).ok()?.checked_add(*base)?;
                l.set(i, StdULogic::from_code_pub(code));
            }
            (Fmt::Bytes(size), _) => {
                let Value::Bytes(b) = vals.first_mut()? else {
                    return None;
                };
                let x = match v {
                    Lit::Enum(p) => *p as u64,
                    Lit::Int(x) | Lit::Phys(x) => *x as u64,
                    Lit::Real(x) => x.to_bits(),
                    _ => return None,
                };
                let at = k as usize * *size as usize;
                let n = *size as usize;
                b.get_mut(at..at + n)?
                    .copy_from_slice(x.to_le_bytes().get(..n)?);
            }
            (Fmt::Array(n, e), Lit::Array(xs)) if xs.len() as u64 == *n => {
                for (j, x) in xs.iter().enumerate() {
                    e.encode(x, vals, k * n + j as u64)?;
                }
            }
            (Fmt::Record(fs), Lit::Record(xs)) if xs.len() == fs.len() => {
                let mut l = 0;
                for (f, x) in fs.iter().zip(xs) {
                    f.encode(x, vals.get_mut(l..)?, k)?;
                    l += f.leaves();
                }
            }
            _ => return None,
        }
        Some(())
    }
}

/// A signal or port.
pub(crate) struct Sig {
    pub name: String,
    /// Lower-case path, `:top:u1:s`.
    pub path: String,
    pub ty: String,
    /// The name of its subtype as declared (`std_logic`, where `ty` is the base type).
    pub declared: String,
    /// `signal`, or the mode of a port.
    pub kind: String,
    /// The resolution function of its type, if any.
    pub resolution: Option<String>,
    /// Where it is declared: the file and line.
    pub decl: Option<(String, u32)>,
    /// The type values are read and written in.
    pub lit: LitType,
    /// Per leaf, the kernel signal and the elements that store it.
    leaves: Vec<(SignalId, Range)>,
    fmt: Fmt,
}

impl Sig {
    /// The value, read through `read`; `None` when a leaf is not readable here.
    pub fn value(&self, read: &dyn Fn(SigRef) -> Option<Value>) -> Option<Lit> {
        let vals: Vec<Option<Value>> = (self.leaves.iter())
            .map(|&(s, r)| read(s.slice(r.offset, r.len)))
            .collect();
        self.fmt.decode(&self.lit, &vals, 0)
    }

    /// The value as text (`?` when it is not readable here).
    pub fn show(&self, read: &dyn Fn(SigRef) -> Option<Value>) -> String {
        self.value(read)
            .map_or_else(|| "?".into(), |v| literal_image(&v, &self.lit))
    }

    /// How many kernel parts store it.
    pub fn leaf_count(&self) -> usize {
        self.leaves.len()
    }

    /// The kernel signal and elements of each part.
    pub fn leaf_parts(&self) -> impl Iterator<Item = (SignalId, Range)> + use<'_> {
        self.leaves.iter().copied()
    }

    /// A value of (part of) leaf `l` as VHDL: logic as a literal, others element by element in
    /// the type's images.
    pub fn leaf_image(&self, l: usize, v: &Value) -> String {
        fn leaf<'a>(f: &'a Fmt, t: &'a LitType, l: &mut usize) -> Option<(&'a Fmt, &'a LitType)> {
            match (f, t) {
                (Fmt::Logic(_) | Fmt::Bytes(_), _) if *l == 0 => Some((f, t)),
                (Fmt::Logic(_) | Fmt::Bytes(_), _) => {
                    *l -= 1;
                    None
                }
                (Fmt::Array(_, e), LitType::Array { elem, .. }) => leaf(e, elem, l),
                (Fmt::Record(fs), LitType::Record(ts)) => {
                    fs.iter().zip(ts).find_map(|(f, (_, t))| leaf(f, t, l))
                }
                _ => None,
            }
        }
        match (v, leaf(&self.fmt, &self.lit, &mut { l })) {
            (Value::Logic(x), Some((Fmt::Logic(base), LitType::Enum(_)))) if *base == 0 => {
                if x.len() == 1 {
                    format!("'{x}'")
                } else {
                    format!("\"{x}\"")
                }
            }
            (Value::Logic(x), _) => format!("\"{x}\""),
            (Value::Bytes(b), Some((Fmt::Bytes(size), t))) => {
                let f = Fmt::Bytes(*size);
                let n = b.len() / (*size).max(1) as usize;
                let vals = [Some(v.clone())];
                let one = |k: usize| {
                    f.decode(t, &vals, k as u64)
                        .map_or_else(|| "?".to_string(), |x| literal_image(&x, t))
                };
                match n {
                    1 => one(0),
                    _ => format!("({})", (0..n).map(one).collect::<Vec<_>>().join(", ")),
                }
            }
            _ => "?".to_string(),
        }
    }

    /// The kernel signals that store it.
    pub fn leaves(&self) -> impl Iterator<Item = SignalId> + use<'_> {
        self.leaves.iter().map(|&(s, _)| s)
    }

    /// `text` as a value of the signal's type ([`typed_literal`]).
    pub fn parse(&self, text: &str) -> Result<Lit, String> {
        typed_literal(text, &self.lit).map_err(|hint| {
            let name = crate::console::dotted(&self.path);
            format!(
                "`{text}` is not a value of `{name}`, type {}: {hint}",
                self.declared
            )
        })
    }

    /// The kernel elements of the sub-elements at `paths` (field and element positions; an
    /// empty path is the whole signal), one per element, with their values in `whole` (the
    /// signal's value, none for a release).
    pub fn part(
        &self,
        paths: &[Vec<usize>],
        whole: Option<&Lit>,
    ) -> Option<Vec<(SigRef, Option<Value>)>> {
        let mut vals = Vec::new();
        self.fmt.alloc(1, &mut vals);
        if let Some(v) = whole {
            self.fmt.encode(v, &mut vals, 0)?;
        }
        let mut elems = Vec::new();
        for p in paths {
            self.fmt.part(p, 0, 0, &mut elems)?;
        }
        // ponytail: one kernel call per element; ranges if large parts are forced often.
        elems
            .into_iter()
            .map(|(l, k)| {
                let (id, at) = *self.leaves.get(l)?;
                let id = |i: u32| id.slice(at.offset + i, 1);
                Some(match vals.get(l)? {
                    // Kernel offset 0 is the rightmost element.
                    Value::Logic(lv) => {
                        let i = (lv.len() as u64).checked_sub(k + 1)? as usize;
                        let v = Value::Logic(LogicVec::new(1, lv.get(i)));
                        (id(i as u32), whole.map(|_| v))
                    }
                    Value::Bytes(b) => {
                        let size = b.len() as u64 / self.reps(l)?;
                        let at = (k * size) as usize;
                        let v = Value::Bytes(b.get(at..at + size as usize)?.to_vec());
                        (id(k as u32), whole.map(|_| v))
                    }
                })
            })
            .collect()
    }

    /// The elements of leaf `l` (row major over the arrays around it).
    fn reps(&self, l: usize) -> Option<u64> {
        fn walk(f: &Fmt, reps: u64, l: &mut usize) -> Option<u64> {
            match f {
                Fmt::Logic(_) | Fmt::Bytes(_) if *l == 0 => Some(reps),
                Fmt::Logic(_) | Fmt::Bytes(_) => {
                    *l -= 1;
                    None
                }
                Fmt::Array(n, e) => walk(e, reps * n, l),
                Fmt::Record(fs) => fs.iter().find_map(|f| walk(f, reps, l)),
            }
        }
        walk(&self.fmt, 1, &mut { l })
    }
}

/// A shared variable. What it holds is in `Built`, which changes with every simulation.
pub(crate) struct SharedVar {
    pub name: String,
    /// Lower-case path: `:top:u1:v`, of a package `:lib:pkg:v`. Waves name it the same.
    pub path: String,
    pub region: Option<ScopeId>,
    pub decl: yavs_sem::DeclId,
    pub ty: TyId,
}

/// What a name means, seen from a process or by path.
#[derive(Clone, Copy)]
pub(crate) enum Named {
    /// Index into `Design::sigs`.
    Signal(usize),
    /// Index into `Design::shared`.
    Shared(usize),
}

/// What one process sees.
#[derive(Default)]
pub(crate) struct Proc {
    /// What the debugger's Threads and Call Stack call it (`thread_label`).
    pub label: String,
    /// Path prefixes (`:top:u1:`) of its enclosing regions up to its instance, innermost
    /// first, for name lookup.
    regions: Vec<String>,
    /// Indices into `Design::sigs`.
    pub signals: Vec<usize>,
    /// Indices into `Design::shared`: those of its enclosing regions, then of the packages.
    pub shared: Vec<usize>,
    /// Name and value of each generic of its instance.
    pub generics: Vec<(String, String)>,
    /// The generics that can be read in an expression, with their values.
    pub generic_lits: Vec<(String, LitType, Lit)>,
}

pub(crate) struct Design {
    /// Per statement id.
    pub locs: Vec<Option<Loc>>,
    /// File key, then line: the outermost statements starting on that line.
    lines: HashMap<String, BTreeMap<u32, Vec<StmtId>>>,
    pub sigs: Vec<Sig>,
    /// Lower-case path (`:top:u1:s`) to `sigs` index.
    by_path: HashMap<String, usize>,
    pub shared: Vec<SharedVar>,
    shared_by_path: HashMap<String, usize>,
    /// By process name ([`Elaborated::proc_name`]).
    procs: HashMap<String, Proc>,
    /// The elaborated hierarchy: instances, blocks, generates and packages with signals.
    pub regions: Vec<Region>,
    /// Indices into `regions` of the regions without a parent (the top level, libraries).
    pub roots: Vec<usize>,
}

/// A region of the elaborated hierarchy (component scopes are left out: their instance
/// carries the label, as in path names).
pub(crate) struct Region {
    /// The label with a generate index (`g(0)`), original spelling.
    pub name: String,
    /// Lower-case path, `:top:u1:`.
    pub path: String,
    pub kind: &'static str,
    pub children: Vec<usize>,
    /// Indices into `Design::sigs`.
    pub sigs: Vec<usize>,
    /// Indices into `Design::shared`.
    pub shared: Vec<usize>,
    /// Process names ([`Elaborated::proc_name`]).
    pub procs: Vec<String>,
    /// For an instance, its generics that can be read in an expression.
    pub generic_lits: Vec<(String, LitType, Lit)>,
}

/// The name of the subtype an object is declared with (`std_logic`, not its base type).
pub(crate) fn declared_type(db: &Db, d: yavs_sem::DeclId) -> Option<String> {
    let DeclKind::Object(o) = &db.decls[d].kind else {
        return None;
    };
    let mut cur = Some(o.subtype);
    while let Some(s) = cur {
        if let Some(n) = &db.subtypes[s].name {
            return Some(n.text.to_string());
        }
        cur = db.subtypes[s].parent;
    }
    None
}

fn mode_word(m: yavs_kernel::PortMode) -> &'static str {
    match m {
        yavs_kernel::PortMode::In => "in",
        yavs_kernel::PortMode::Out => "out",
        yavs_kernel::PortMode::Inout => "inout",
        yavs_kernel::PortMode::Buffer => "buffer",
    }
}

/// The name of the resolution function of `shape`, of itself or of its elements.
fn resolution_of(db: &Db, shape: &Shape) -> Option<String> {
    let name = |d: yavs_sem::DeclId| db.decls[d].name.to_string();
    match shape {
        Shape::Scalar { res, .. } => res.map(name),
        Shape::Array { res, elem, .. } => res
            .map(name)
            .or_else(|| resolution_of(db, elem).map(|e| format!("{e} (per element)"))),
        Shape::Record { res, fields, .. } => res.map(name).or_else(|| {
            let parts: Vec<String> = fields
                .iter()
                .filter_map(|(n, f)| Some(format!("{} by {}", n.text, resolution_of(db, f)?)))
                .collect();
            (!parts.is_empty()).then(|| parts.join(", "))
        }),
    }
}

/// A file path as the client sees it: absolute.
pub(crate) fn absolute(p: &str) -> String {
    std::path::absolute(p).map_or_else(|_| p.to_string(), |c| c.display().to_string())
}

/// The key a breakpoint's file is matched by: one file is one key however its path is spelled
/// (`..`, symlinks, `\\?\` on Windows), see [`yavs_diag::files::resolve`]. A path that does not
/// exist (a source map's original, a moved build tree) is resolved as far as it exists. Clients
/// differ in the case of Windows drive letters.
pub(crate) fn file_key(p: &str) -> String {
    let s = yavs_diag::files::resolve(std::path::Path::new(p))
        .display()
        .to_string();
    if cfg!(windows) { s.to_lowercase() } else { s }
}

/// The file and line of a declaration, as the debugger shows them.
pub(crate) fn decl_of(
    db: &Db,
    at: yavs_diag::Span,
    source_map: &[(String, String)],
) -> Option<(String, u32)> {
    let f = db.sources.get(at.file);
    let (line, _) = f.line_col(at.start);
    Some((shown_path(&f.path, source_map), line as u32))
}

/// A source path as the debugger shows it (`--source-map`).
fn shown_path(path: &str, source_map: &[(String, String)]) -> String {
    let p = file_key(path);
    let map = source_map.iter().find(|(from, _)| file_key(from) == p);
    map.map_or_else(|| absolute(path), |(_, to)| absolute(to))
}

impl Design {
    /// `source_map`: source paths as compiled, and as the debugger shows and takes them.
    pub fn new(db: &Db, e: &Elaborated, prog: &Program, source_map: &[(String, String)]) -> Design {
        let mut paths: HashMap<yavs_diag::FileId, String> = HashMap::new();
        let mut locs = Vec::with_capacity(prog.stmts.len());
        for (_, st) in prog.stmts.iter() {
            let f = db.sources.get(st.span.file);
            let path = paths
                .entry(st.span.file)
                .or_insert_with(|| shown_path(&f.path, source_map))
                .clone();
            let (line, col) = f.line_col(st.span.start);
            locs.push(Some(Loc {
                path,
                line: line as u32,
                col: col as u32,
            }));
        }
        let mut lines: HashMap<String, BTreeMap<u32, Vec<StmtId>>> = HashMap::new();
        let mut keys: HashMap<&str, String> = HashMap::new();
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
                .entry(
                    keys.entry(&l.path)
                        .or_insert_with(|| file_key(&l.path))
                        .clone(),
                )
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
            shared: Vec::new(),
            shared_by_path: HashMap::new(),
            procs: HashMap::new(),
            regions: Vec::new(),
            roots: Vec::new(),
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
                let path = format!("{prefix}{}", obj.name.key).to_ascii_lowercase();
                d.sigs.push(Sig {
                    name: obj.name.text.to_string(),
                    path: path.clone(),
                    ty: db.types[obj.shape.ty()].name.text.to_string(),
                    declared: declared_type(db, obj.decl)
                        .unwrap_or_else(|| db.types[obj.shape.ty()].name.text.to_string()),
                    kind: match obj.kind {
                        ObjKind::Port(m) => format!("port, mode {}", mode_word(m)),
                        _ => "signal".to_string(),
                    },
                    resolution: resolution_of(db, &obj.shape),
                    decl: decl_of(db, obj.name.span, source_map),
                    leaves: (obj.leaves.0.0..obj.leaves.1.0)
                        .map(|l| {
                            let l = yavs_elab::LeafId(l);
                            e.nets.kpart(l, Range::new(0, e.nets.leaf(l).ty.len))
                        })
                        .collect(),
                    lit: lit_type(db, &obj.shape),
                    fmt: fmt(db, &obj.shape),
                });
                d.by_path.insert(path, k);
                scope_sigs.entry(s).or_default().push(k);
            }
        }
        let mut scope_shared = d.add_shared(db, e);
        let package_shared: Vec<usize> = (0..d.shared.len())
            .filter(|&k| d.shared[k].region.is_none())
            .collect();
        for p in &e.procs {
            let inst = e.instance_of(p.scope);
            let mut pr = Proc {
                label: e.proc_label(p),
                ..Proc::default()
            };
            let mut cur = e.hier.scope(p.scope).parent;
            while let Some(s) = cur {
                pr.regions.push(e.hier.path_name(s, yavs_elab::Case::Lrm));
                pr.signals
                    .extend(scope_sigs.get(&s).into_iter().flatten().copied());
                pr.shared
                    .extend(scope_shared.get(&s).into_iter().flatten().copied());
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
                    let name = db.decls[decl].name.to_string();
                    let lit = e
                        .value(inst, decl)
                        .and_then(|v| generic_lit(db, db.base(o.subtype), v));
                    if let Some((t, x)) = lit {
                        pr.generic_lits.push((name.clone(), t, x));
                    }
                    pr.generics.push((name, v));
                }
            }
            pr.shared.extend(&package_shared);
            d.procs.insert(e.proc_name(p), pr);
        }
        // Regions: every scope but processes and components, attached to the nearest such
        // ancestor.
        let mut region_of: HashMap<ScopeId, usize> = HashMap::new();
        for (i, sc) in e.hier.scopes.iter().enumerate() {
            let s = ScopeId(i as u32);
            let kind = match sc.kind {
                ScopeKind::Process { .. } | ScopeKind::Component { .. } => continue,
                ScopeKind::Instance { .. } => "instance",
                ScopeKind::Block => "block",
                ScopeKind::ForGenerate { .. } | ScopeKind::Generate { .. } => "generate",
                ScopeKind::Library => "library",
                ScopeKind::Package { .. } => "package",
            };
            let mut name = sc.name.text.to_string();
            if let Some(i) = &sc.index {
                name.push_str(&format!("({i})"));
            }
            region_of.insert(s, d.regions.len());
            d.regions.push(Region {
                name,
                path: e.hier.path_name(s, yavs_elab::Case::Lrm),
                kind,
                children: Vec::new(),
                sigs: scope_sigs.remove(&s).unwrap_or_default(),
                shared: scope_shared.remove(&s).unwrap_or_default(),
                procs: Vec::new(),
                generic_lits: Vec::new(),
            });
        }
        // Scopes come in elaboration order, parents first.
        let up = |mut s: ScopeId| loop {
            match e.hier.scope(s).parent {
                Some(p) if region_of.contains_key(&p) => return Some(region_of[&p]),
                Some(p) => s = p,
                None => return None,
            }
        };
        for (&s, &r) in &region_of {
            match up(s) {
                Some(p) => d.regions[p].children.push(r),
                None => d.roots.push(r),
            }
        }
        for reg in &mut d.regions {
            reg.children.sort_unstable();
        }
        d.roots.sort_unstable();
        for p in &e.procs {
            if let Some(r) = up(p.scope) {
                d.regions[r].procs.push(e.proc_name(p));
            }
        }
        // An instance's generics, for a stop between cycles.
        for p in &e.procs {
            let name = e.proc_name(p);
            if let Some(&r) = region_of.get(&e.instance_of(p.scope))
                && d.regions[r].generic_lits.is_empty()
                && let Some(pr) = d.procs.get(&name)
            {
                d.regions[r].generic_lits = pr.generic_lits.clone();
            }
        }
        d
    }

    /// Fill `shared`, sorted by path; those of each region, by `shared` index.
    fn add_shared(&mut self, db: &Db, e: &Elaborated) -> HashMap<ScopeId, Vec<usize>> {
        let mut all = Vec::new();
        for &(decl, thunk, ty) in e.ir.iter().flat_map(|ir| &ir.var_slots) {
            let name = db.decls[decl].name.to_string();
            let var = |region, prefix: String| SharedVar {
                path: format!("{prefix}{name}").to_ascii_lowercase(),
                name: name.clone(),
                region,
                decl,
                ty,
            };
            if thunk.is_some() {
                // ponytail: one of a package instance nested in a package is left out, as in
                // waves; walk the instance regions too if one is ever wanted.
                let unit = (db.units.iter()).find(|(_, u)| u.scope == db.decls[decl].scope);
                if let Some((_, u)) = unit {
                    let lib = &db.libs[u.lib].name.text;
                    all.push(var(None, format!(":{lib}:{}:", u.name.text)));
                }
            }
            let regions = e.shared_vars.iter().filter(|&&(_, x)| x == decl);
            for &(r, _) in regions.filter(|_| thunk.is_none()) {
                let path = e.hier.path_name(r, yavs_elab::Case::Lrm);
                all.push(var(Some(r), path));
            }
        }
        all.sort_by(|a, b| a.path.cmp(&b.path));
        let mut scopes: HashMap<ScopeId, Vec<usize>> = HashMap::new();
        for (k, v) in all.iter().enumerate() {
            self.shared_by_path.insert(v.path.clone(), k);
            if let Some(r) = v.region {
                scopes.entry(r).or_default().push(k);
            }
        }
        self.shared = all;
        scopes
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

    /// The path a file key (`file_key`, lower case on Windows) is shown with.
    pub fn shown_file(&self, key: &str) -> String {
        let stmt = self
            .lines
            .get(&file_key(key))
            .and_then(|l| l.values().next()?.first());
        stmt.and_then(|&s| self.loc(s))
            .map_or_else(|| key.to_string(), |l| l.path.clone())
    }

    /// The source file `name` means: its path, or the one file whose path ends in it.
    pub fn source_file(&self, name: &str) -> Result<String, String> {
        let key = file_key(name);
        if self.lines.contains_key(&key) {
            return Ok(key);
        }
        let tail = format!("/{}", name.replace('\\', "/"));
        let of = |f: &&String| f.replace('\\', "/").ends_with(&tail);
        let mut found: Vec<&String> = self.lines.keys().filter(of).collect();
        found.sort();
        match found[..] {
            [f] => Ok(f.clone()),
            [] => {
                let files = self.files();
                let help = crate::console::similar(name, "source file", &files);
                let help = help.map_or(String::new(), |h| format!(": {h}"));
                Err(format!(
                    "`{name}` is not a source file of this design{help}"
                ))
            }
            _ => Err(format!(
                "`{name}` could be {}: write more of the path",
                found
                    .iter()
                    .map(|f| f.as_str())
                    .collect::<Vec<_>>()
                    .join(" or ")
            )),
        }
    }

    /// The generics a process sees, or at the top level between cycles those of the top region.
    pub fn generics_at(&self, proc: Option<&str>) -> &[(String, LitType, Lit)] {
        let top = || {
            self.roots
                .first()
                .map(|&r| &self.regions[r].generic_lits[..])
        };
        match proc.and_then(|n| self.proc(n)) {
            Some(p) => &p.generic_lits,
            None => top().unwrap_or_default(),
        }
    }

    pub fn proc(&self, name: &str) -> Option<&Proc> {
        self.procs.get(name)
    }

    /// The source files with statements, as a user would name them on a console.
    pub fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .lines
            .keys()
            .map(|k| self.shown_file(k))
            .filter_map(|p| {
                let p = std::path::Path::new(&p)
                    .file_name()?
                    .to_string_lossy()
                    .into_owned();
                Some(p)
            })
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// The path prefixes (`:top:u1:`) a name is looked up from: those of process `p`, else the top
    /// levels.
    pub fn scope(&self, p: Option<&Proc>) -> Vec<String> {
        match p {
            Some(p) => p.regions.clone(),
            None => self
                .roots
                .iter()
                .map(|&r| self.regions[r].path.clone())
                .collect(),
        }
    }

    /// A signal by simple name seen from process `p`, or by path (`:top:u1:s`, `top.u1.s`): its
    /// index into `sigs`.
    pub fn find(&self, p: Option<&Proc>, name: &str) -> Option<usize> {
        let n = name.trim().to_ascii_lowercase();
        let colons = n.replace('.', ":");
        let path = if n.starts_with(':') {
            colons.clone()
        } else {
            format!(":{colons}")
        };
        // Also a path from where the process stands: `u_reg.q`.
        let local = p
            .into_iter()
            .flat_map(|p| &p.regions)
            .find_map(|r| self.by_path.get(&format!("{r}{colons}")));
        local.or_else(|| self.by_path.get(&path)).copied()
    }

    /// What `name` means: a signal or a shared variable by simple name seen from process `p`
    /// (the innermost region first), by path, or a shared variable of a package by `pkg.v`, or
    /// by `v` if only one package has it.
    pub fn named(&self, p: Option<&Proc>, name: &str) -> Option<Named> {
        let n = name.trim().to_ascii_lowercase();
        let at = |path: &str| {
            let signal = self.by_path.get(path).map(|&k| Named::Signal(k));
            signal.or_else(|| self.shared_by_path.get(path).map(|&k| Named::Shared(k)))
        };
        let colons = n.replace('.', ":");
        let path = if n.starts_with(':') {
            colons.clone()
        } else {
            format!(":{colons}")
        };
        // Also a path from where the process stands: `u_reg.q`.
        let seen =
            (p.into_iter().flat_map(|p| &p.regions)).find_map(|r| at(&format!("{r}{colons}")));
        seen.or_else(|| at(&path)).or_else(|| {
            let tail = format!(":{colons}");
            let of_package = |k: &usize| {
                let v = &self.shared[*k];
                v.region.is_none() && v.path.ends_with(&tail)
            };
            let mut found = (0..self.shared.len()).filter(of_package);
            let k = found.next()?;
            found.next().is_none().then_some(Named::Shared(k))
        })
    }
}

/// An enumeration of character literals only, with `'0'` and `'1'`: `bit`, `std_ulogic`, and the
/// like (`character` has identifiers too).
fn bit_like(t: &EnumType) -> bool {
    let has = |c| t.lits.contains(&EnumLit::Char(c));
    has(b'0') && has(b'1') && t.lits.iter().all(|l| matches!(l, EnumLit::Char(_)))
}

/// A value the user typed, for a target of type `ty`, or what that type takes. One path for force,
/// deposit, Set Value and break on value: the VHDL literal first, so what is a literal of the type
/// keeps its meaning (`1` of an integer stays 1), then the quotes a bit-like target leaves no doubt
/// about (`1` is `'1'`, `0101` is `"0101"`). Rule: `docs/decisions.md` "Values typed in the debugger".
pub(crate) fn typed_literal(text: &str, ty: &LitType) -> Result<Lit, String> {
    let text = text.trim();
    let first = match literal(text.as_bytes(), ty) {
        Ok(v) => return Ok(v),
        Err(e) => e,
    };
    let quoted = match ty {
        LitType::Enum(t) if bit_like(t) && text.len() == 1 => Some(format!("'{text}'")),
        LitType::Array { elem, .. } if !text.is_empty() && !text.contains('"') => match &**elem {
            LitType::Enum(t) if bit_like(t) => Some(format!("\"{text}\"")),
            _ => None,
        },
        _ => None,
    };
    if let Some(v) = quoted.and_then(|q| literal(q.as_bytes(), ty).ok()) {
        return Ok(v);
    }
    Err(takes(text, ty, &first.to_string()))
}

/// What a value of `ty` looks like, for the message about `text`; `why` is the parser's reason,
/// worth reading inside an aggregate.
fn takes(text: &str, ty: &LitType, why: &str) -> String {
    let lit = |l: &EnumLit| match l {
        EnumLit::Char(c) => format!("'{}'", *c as char),
        EnumLit::Ident(n) => String::from_utf8_lossy(n).into_owned(),
    };
    match ty {
        LitType::Enum(t) => {
            let all: Vec<String> = t.lits.iter().map(lit).collect();
            let more = if all.len() > 16 { ", ..." } else { "" };
            let shown = all.iter().take(16).cloned().collect::<Vec<_>>().join(", ");
            let help = crate::console::similar(text, "value", &all)
                .map_or(String::new(), |h| format!(": {h}"));
            format!("expected one of {shown}{more}{help}")
        }
        LitType::Int(b) if *b == StdBounds::I64 => "expected an integer".to_string(),
        LitType::Int(b) => format!("expected an integer from {} to {}", b.low, b.high),
        LitType::Real(_) => "expected a real number".to_string(),
        LitType::Phys(p, _) => {
            let units: Vec<String> = (p.units.iter())
                .map(|u| String::from_utf8_lossy(&u.name).into_owned())
                .collect();
            // A unit that is none of the type's: the closest.
            let word = text.trim_start_matches(|c: char| !c.is_ascii_alphabetic());
            let help = crate::console::similar(word, "unit", &units)
                .filter(|_| !word.is_empty())
                .map_or(String::new(), |h| format!(": {h}"));
            format!(
                "expected a number and a unit, such as 10 {}{help}",
                String::from_utf8_lossy(&p.primary().name)
            )
        }
        LitType::Array { len, elem, .. } => match &**elem {
            LitType::Enum(t) if bit_like(t) => {
                let ex = "01".repeat((*len).min(8) as usize / 2 + 1);
                let ex = &ex[..(*len).min(8) as usize];
                format!(
                    "expected {len} elements, such as {ex} or \"{ex}\", x\"..\" or an aggregate"
                )
            }
            LitType::Enum(_) => {
                format!("expected {len} characters in double quotes, or an aggregate: {why}")
            }
            _ => format!("expected an aggregate of {len} elements: {why}"),
        },
        LitType::Record(fs) => {
            let f = fs.first().map_or("field", |(n, _)| n.as_str());
            format!("expected a record aggregate such as ({f} => ...): {why}")
        }
        LitType::Handle { .. } => "expected `null`".to_string(),
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

fn fmt(db: &Db, shape: &Shape) -> Fmt {
    match shape {
        Shape::Scalar {
            ty,
            elem: Elem::Logic,
            ..
        } => {
            let wk = &db.well_known;
            Fmt::Logic(if Some(*ty) == wk.boolean || Some(*ty) == wk.bit {
                2
            } else {
                0
            })
        }
        Shape::Scalar {
            elem: Elem::Bytes(n),
            ..
        } => Fmt::Bytes(*n),
        Shape::Array { elem, .. } => Fmt::Array(shape.len(), Box::new(fmt(db, elem))),
        Shape::Record { fields, .. } => {
            Fmt::Record(fields.iter().map(|(_, f)| fmt(db, f)).collect())
        }
    }
}

/// The type values of scalar type `ty` within `bounds` are read and written in.
fn scalar_type(db: &Db, ty: yavs_sem::TypeId, bounds: StdBounds) -> LitType {
    match &db.types[ty].kind {
        TypeKind::Enum { lits } => LitType::Enum(EnumType {
            lits: lits
                .iter()
                .map(|&l| match db.decls[l].name {
                    Designator::Char(c) => EnumLit::Char(c as u8),
                    Designator::Ident(i) => EnumLit::ident(&i.text.to_string()),
                    Designator::Operator { text, .. } => EnumLit::ident(&text.to_string()),
                })
                .collect(),
        }),
        TypeKind::Floating | TypeKind::UniversalReal => LitType::Real(RealBounds::REAL),
        TypeKind::Physical { units } => LitType::Phys(
            PhysicalType {
                units: units
                    .iter()
                    .map(|u| Unit {
                        name: to_latin1(&db.decls[u.decl].name.to_string()),
                        value: u.scale,
                    })
                    .collect(),
            },
            bounds,
        ),
        _ => LitType::Int(bounds),
    }
}

/// A generic's value for the debugger: scalars and one-dimensional arrays of enumerations
/// (strings, bit vectors); `None` for the rest.
fn generic_lit(db: &Db, ty: yavs_sem::TypeId, v: &ConstValue) -> Option<(LitType, Lit)> {
    match (&db.types[ty].kind, v) {
        (TypeKind::Array { elem, .. }, ConstValue::Array(a)) if a.bounds.len() == 1 => {
            let Elems::Bytes(b) = &a.elems else {
                return None;
            };
            let (left, dir, _) = a.bounds[0];
            let et = scalar_type(db, db.base(*elem), StdBounds::I64);
            let ascending = dir == Direction::To;
            let len = b.len() as u64;
            Some((
                LitType::Array {
                    left,
                    ascending,
                    len,
                    elem: Box::new(et),
                },
                Lit::Array(b.iter().map(|&p| Lit::Enum(p as usize)).collect()),
            ))
        }
        (TypeKind::Array { .. } | TypeKind::Record { .. }, _) => None,
        (_, ConstValue::Enum(p)) => {
            Some((scalar_type(db, ty, StdBounds::I64), Lit::Enum(*p as usize)))
        }
        (_, ConstValue::Int(n)) => Some((scalar_type(db, ty, StdBounds::I64), Lit::Int(*n))),
        (_, ConstValue::Phys(n)) => Some((scalar_type(db, ty, StdBounds::I64), Lit::Phys(*n))),
        (_, ConstValue::Real(x)) => Some((scalar_type(db, ty, StdBounds::I64), Lit::Real(*x))),
        _ => None,
    }
}

/// The type values of `shape` are read and written in. Arrays of several dimensions nest.
fn lit_type(db: &Db, shape: &Shape) -> LitType {
    match shape {
        Shape::Scalar { ty, range, .. } => {
            let bounds = range.map_or(StdBounds::I64, |r| {
                let (lo, hi) = if r.dir == Direction::To {
                    (r.left, r.right)
                } else {
                    (r.right, r.left)
                };
                StdBounds { low: lo, high: hi }
            });
            scalar_type(db, *ty, bounds)
        }
        Shape::Array { dims, elem, .. } => {
            dims.iter()
                .rev()
                .fold(lit_type(db, elem), |e, d| LitType::Array {
                    left: d.left,
                    ascending: d.dir == Direction::To,
                    len: d.len(),
                    elem: Box::new(e),
                })
        }
        Shape::Record { fields, .. } => LitType::Record(
            fields
                .iter()
                .map(|(n, f)| (n.text.to_string(), lit_type(db, f)))
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

/// A variable's value of IR type `ty` as the debugger reads and writes it; `None` for types
/// it does not show (file). Handles come as `ConstValue::Int` (`ir_decode_shown`).
pub(crate) fn ir_lit(prog: &Program, ty: TyId, v: &ConstValue) -> Option<(LitType, Lit)> {
    Some(match (prog.types.get(ty)?, v) {
        (Ty::Int { lo, hi }, ConstValue::Int(x)) => (
            LitType::Int(StdBounds {
                low: *lo,
                high: *hi,
            }),
            Lit::Int(*x),
        ),
        (Ty::Physical { lo, hi, units }, ConstValue::Phys(x)) => (
            LitType::Phys(
                PhysicalType {
                    units: units
                        .iter()
                        .map(|(n, v)| Unit {
                            name: to_latin1(n),
                            value: *v,
                        })
                        .collect(),
                },
                StdBounds {
                    low: *lo,
                    high: *hi,
                },
            ),
            Lit::Phys(*x),
        ),
        (Ty::Enum { lits, .. }, ConstValue::Enum(p)) => (
            LitType::Enum(EnumType {
                lits: lits
                    .iter()
                    .map(|l| match l.as_bytes() {
                        [b'\'', c, b'\''] => EnumLit::Char(*c),
                        _ => EnumLit::ident(l),
                    })
                    .collect(),
            }),
            Lit::Enum(*p as usize),
        ),
        (Ty::Real, ConstValue::Real(x)) => (LitType::Real(RealBounds::REAL), Lit::Real(*x)),
        (Ty::Access { target }, ConstValue::Int(h)) => (
            LitType::Handle {
                target: target.0,
                protected: false,
            },
            Lit::Handle(*h as u64),
        ),
        (Ty::Protected { state }, ConstValue::Int(h)) => (
            LitType::Handle {
                target: state.0,
                protected: true,
            },
            Lit::Handle(*h as u64),
        ),
        (Ty::Array { elem, .. }, ConstValue::Array(a)) => {
            let n: u64 = a.bounds.iter().map(|b| span(*b)).product();
            let mut xs = Vec::with_capacity(n as usize);
            let mut et = None;
            for k in 0..n {
                let (t, x) = ir_lit(prog, *elem, &elem_at(a, k)?)?;
                et.get_or_insert(t);
                xs.push(x);
            }
            // ponytail: a null array of composites has no element to take the element type
            // from; it shows as an empty aggregate.
            let mut t = et.unwrap_or(LitType::Record(Vec::new()));
            for (d, &(left, dir, right)) in a.bounds.iter().enumerate().rev() {
                let len = span((left, dir, right));
                t = LitType::Array {
                    left,
                    ascending: dir == Direction::To,
                    len,
                    elem: Box::new(t),
                };
                if d > 0 {
                    xs = xs
                        .chunks(len.max(1) as usize)
                        .map(|c| Lit::Array(c.to_vec()))
                        .collect();
                }
            }
            // The outermost dimension: `xs` holds its elements.
            (t, Lit::Array(xs))
        }
        (Ty::Record { fields }, ConstValue::Record(vs)) if vs.len() == fields.len() => {
            let mut ts = Vec::new();
            let mut xs = Vec::new();
            for (f, x) in fields.iter().zip(vs.iter()) {
                let (t, x) = ir_lit(prog, f.ty, x)?;
                ts.push((f.name.clone(), t));
                xs.push(x);
            }
            (LitType::Record(ts), Lit::Record(xs))
        }
        _ => return None,
    })
}

/// The length of one array dimension.
fn span((l, d, r): (i64, Direction, i64)) -> u64 {
    let (lo, hi) = if d == Direction::To { (l, r) } else { (r, l) };
    if hi < lo { 0 } else { hi.abs_diff(lo) + 1 }
}

/// Element `k` (row major) of `a`.
fn elem_at(a: &yavs_sem::stat::ArrayConst, mut k: u64) -> Option<ConstValue> {
    match &a.elems {
        Elems::Bytes(b) => b.get(k as usize).map(|&x| ConstValue::Enum(u32::from(x))),
        Elems::Runs(rs) => {
            for (n, x) in rs.iter() {
                if k < *n {
                    return Some(x.clone());
                }
                k -= n;
            }
            None
        }
    }
}

/// `v` (of the type [`ir_lit`] gave for `like`) as a value shaped like `like`.
pub(crate) fn to_const(v: &Lit, like: &ConstValue) -> Option<ConstValue> {
    Some(match (v, like) {
        (Lit::Int(x), ConstValue::Int(_)) => ConstValue::Int(*x),
        (Lit::Handle(x), ConstValue::Int(_)) => ConstValue::Int(*x as i64),
        (Lit::Phys(x), ConstValue::Phys(_)) => ConstValue::Phys(*x),
        (Lit::Real(x), ConstValue::Real(_)) => ConstValue::Real(*x),
        (Lit::Enum(p), ConstValue::Enum(_)) => ConstValue::Enum(u32::try_from(*p).ok()?),
        (Lit::Array(_), ConstValue::Array(a)) => {
            // Flatten the nested dimensions, row major.
            let mut flat = vec![v];
            for _ in 0..a.bounds.len() {
                let mut next = Vec::new();
                for x in flat {
                    let Lit::Array(xs) = x else {
                        return None;
                    };
                    next.extend(xs);
                }
                flat = next;
            }
            let runs = flat
                .iter()
                .enumerate()
                .map(|(k, x)| Some((1, to_const(x, &elem_at(a, k as u64)?)?)))
                .collect::<Option<Box<[_]>>>()?;
            ConstValue::Array(yavs_sem::stat::ArrayConst {
                bounds: a.bounds.clone(),
                elems: Elems::Runs(runs),
            })
        }
        (Lit::Record(xs), ConstValue::Record(ls)) if xs.len() == ls.len() => ConstValue::Record(
            xs.iter()
                .zip(ls.iter())
                .map(|(x, l)| to_const(x, l))
                .collect::<Option<_>>()?,
        ),
        _ => return None,
    })
}

/// Whether a value of `t` fits sem base type `ty`: the same kind of type, the same enumeration
/// literals, element types and fields (a call's parameter, #364).
pub(crate) fn fits(db: &Db, ty: yavs_sem::TypeId, t: &LitType) -> bool {
    match (&db.types[ty].kind, t) {
        (TypeKind::Integer | TypeKind::UniversalInteger, LitType::Int(_)) => true,
        (TypeKind::Floating | TypeKind::UniversalReal, LitType::Real(_)) => true,
        (TypeKind::Enum { .. }, LitType::Enum(e)) => {
            matches!(scalar_type(db, ty, StdBounds::I64), LitType::Enum(f) if f.lits == e.lits)
        }
        (TypeKind::Physical { .. }, LitType::Phys(p, _)) => {
            matches!(scalar_type(db, ty, StdBounds::I64), LitType::Phys(q, _) if q.units == p.units)
        }
        (TypeKind::Array { indexes, elem }, t) => {
            let mut t = t;
            for _ in 0..indexes.len() {
                let LitType::Array { elem, .. } = t else {
                    return false;
                };
                t = elem;
            }
            fits(db, db.base(*elem), t)
        }
        (TypeKind::Record { elems }, LitType::Record(fs)) => {
            elems.len() == fs.len()
                && elems.iter().zip(fs).all(|(e, (n, t))| {
                    e.name.text.to_string().eq_ignore_ascii_case(n)
                        && fits(db, db.base(e.subtype), t)
                })
        }
        _ => false,
    }
}

/// The type a literal of sem base type `ty` is read in; an array of `len` elements from 0 up
/// (one dimension).
pub(crate) fn literal_type(db: &Db, ty: yavs_sem::TypeId, len: u64) -> Option<LitType> {
    match &db.types[ty].kind {
        TypeKind::Array { indexes, elem } if indexes.len() == 1 => Some(LitType::Array {
            left: 0,
            ascending: true,
            len,
            elem: Box::new(literal_type(db, db.base(*elem), 0)?),
        }),
        TypeKind::Array { .. } | TypeKind::Record { .. } => None,
        TypeKind::Access { .. } | TypeKind::File { .. } | TypeKind::Protected { .. } => None,
        _ => Some(scalar_type(db, ty, StdBounds::I64)),
    }
}

/// `v` (of `t`) as an elaboration-time value of sem base type `ty`, for a call; `None` for what a
/// call cannot take (access values).
pub(crate) fn to_val(db: &Db, ty: yavs_sem::TypeId, t: &LitType, v: &Lit) -> Option<ConstValue> {
    Some(match (&db.types[ty].kind, v) {
        (_, Lit::Int(i)) => ConstValue::Int(*i),
        (_, Lit::Real(x)) => ConstValue::Real(*x),
        (_, Lit::Phys(x)) => ConstValue::Phys(*x),
        (_, Lit::Enum(p)) => ConstValue::Enum(u32::try_from(*p).ok()?),
        (TypeKind::Array { indexes, elem }, Lit::Array(_)) => {
            let (mut bounds, mut flat, mut t) = (Vec::new(), vec![v], t);
            for _ in 0..indexes.len() {
                let LitType::Array {
                    left,
                    ascending,
                    len,
                    elem,
                } = t
                else {
                    return None;
                };
                let last = i64::try_from(*len).ok()? - 1;
                let (dir, right) = match ascending {
                    true => (Direction::To, left.checked_add(last)?),
                    false => (Direction::Downto, left.checked_sub(last)?),
                };
                bounds.push((*left, dir, right));
                let mut next = Vec::new();
                for x in flat {
                    let Lit::Array(xs) = x else {
                        return None;
                    };
                    next.extend(xs);
                }
                (flat, t) = (next, elem);
            }
            let et = db.base(*elem);
            let bytes = (indexes.len() == 1 && matches!(db.types[et].kind, TypeKind::Enum { .. }))
                .then(|| {
                    flat.iter()
                        .map(|x| match x {
                            Lit::Enum(p) => u8::try_from(*p).ok(),
                            _ => None,
                        })
                        .collect::<Option<Box<[u8]>>>()
                })
                .flatten();
            let elems = match bytes {
                Some(b) => Elems::Bytes(b),
                None => Elems::Runs(
                    flat.iter()
                        .map(|x| Some((1, to_val(db, et, t, x)?)))
                        .collect::<Option<_>>()?,
                ),
            };
            ConstValue::Array(yavs_sem::stat::ArrayConst {
                bounds: bounds.into(),
                elems,
            })
        }
        (TypeKind::Record { elems }, Lit::Record(xs)) => {
            let LitType::Record(fs) = t else {
                return None;
            };
            ConstValue::Record(
                elems
                    .iter()
                    .zip(fs.iter().zip(xs))
                    .map(|(e, ((_, t), x))| to_val(db, db.base(e.subtype), t, x))
                    .collect::<Option<_>>()?,
            )
        }
        _ => return None,
    })
}

/// An elaboration-time value of sem base type `ty` as the debugger reads it (a call's result).
pub(crate) fn from_val(db: &Db, ty: yavs_sem::TypeId, v: &ConstValue) -> Option<(LitType, Lit)> {
    Some(match (&db.types[ty].kind, v) {
        (TypeKind::Array { elem, .. }, ConstValue::Array(a)) => {
            let et = db.base(*elem);
            let n: u64 = a.bounds.iter().map(|b| span(*b)).product();
            let mut xs = Vec::with_capacity(n as usize);
            let mut t = None;
            for k in 0..n {
                let (lt, x) = from_val(db, et, &elem_at(a, k)?)?;
                t.get_or_insert(lt);
                xs.push(x);
            }
            let mut t = match t {
                Some(t) => t,
                None => literal_type(db, et, 0)?,
            };
            for (d, &(left, dir, right)) in a.bounds.iter().enumerate().rev() {
                let len = span((left, dir, right));
                t = LitType::Array {
                    left,
                    ascending: dir == Direction::To,
                    len,
                    elem: Box::new(t),
                };
                if d > 0 {
                    xs = xs
                        .chunks(len.max(1) as usize)
                        .map(|c| Lit::Array(c.to_vec()))
                        .collect();
                }
            }
            (t, Lit::Array(xs))
        }
        (TypeKind::Record { elems }, ConstValue::Record(vs)) if vs.len() == elems.len() => {
            let (mut ts, mut xs) = (Vec::new(), Vec::new());
            for (e, v) in elems.iter().zip(vs.iter()) {
                let (t, x) = from_val(db, db.base(e.subtype), v)?;
                ts.push((e.name.text.to_string(), t));
                xs.push(x);
            }
            (LitType::Record(ts), Lit::Record(xs))
        }
        (TypeKind::Array { .. } | TypeKind::Record { .. }, _) => return None,
        _ => generic_lit(db, ty, v)?,
    })
}
