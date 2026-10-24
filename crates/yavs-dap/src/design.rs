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

    /// The kernel signals that store it.
    pub fn leaves(&self) -> impl Iterator<Item = SignalId> + use<'_> {
        self.leaves.iter().map(|&(s, _)| s)
    }

    /// `text` as a value of the signal's type (`yavs_std::value::literal`).
    pub fn parse(&self, text: &str) -> Result<Lit, String> {
        literal(text.as_bytes(), &self.lit).map_err(|e| e.to_string())
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
    /// Process names ([`Elaborated::proc_name`]).
    pub procs: Vec<String>,
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
                procs: Vec::new(),
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

    /// A signal by simple name seen from process `p`, or by path (`:top:u1:s`, `top.u1.s`): its
    /// index into `sigs`.
    pub fn find(&self, p: Option<&Proc>, name: &str) -> Option<usize> {
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
        local.or_else(|| self.by_path.get(&path)).copied()
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
            match &db.types[*ty].kind {
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
/// it does not show (access, file, protected).
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
