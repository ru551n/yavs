//! Elaboration-time values: the evaluator, subtype shapes, initial values and `-g` overrides.
//!
//! Globally static expressions are folded with sem's static folder (`Db::fold_with`) over
//! generic and constant values; aggregates (any number of dimensions), indexing, slices,
//! fields, concatenation, composite equality and logical operators on BIT and BOOLEAN arrays
//! are evaluated in `crate::value`, the other predefined operations on composites and
//! `TO_STRING` by their yavs-interp natives (`yavs_interp::call_native`, #220); calls of user
//! functions run in yavs-interp after lowering the function alone (ir.md 3.6), with any
//! composite marshalled in the IR layout through `yavs_interp::call_values` (#77, #220);
//! functions that use package objects run against the elaborated packages (`crate::pkgcall`).

use std::collections::HashMap;

use yavs_diag::{Diagnostic, Span};
use yavs_kernel::{Elem, LogicVec, Value};
use yavs_sem::decl::{DeclKind, ObjClass, ObjRole};
use yavs_sem::hir::{AttrPrefix, Direction, ExprKind, PredefAttr, RangeC};
use yavs_sem::scope::ScopeKind as SemScope;
use yavs_sem::stat::{ArrayConst, Elems};
use yavs_sem::types::{Constraint, Resolution, TypeKind};
use yavs_sem::{Db, DeclId, ExprId, SubtypeId, TypeId};

use crate::codes;
use crate::hier::{Bounds, Hierarchy, NetGraph, ObjId, ScopeId, Shape};

pub type Val = yavs_sem::stat::ConstValue;

/// The values of one scope's generics, generate parameters and constants: a few entries,
/// kept sorted by declaration (a hash map of two entries is five times the size).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScopeVals(Vec<(DeclId, Val)>);

impl ScopeVals {
    pub fn get(&self, d: &DeclId) -> Option<&Val> {
        let i = self.0.binary_search_by_key(d, |e| e.0).ok()?;
        Some(&self.0[i].1)
    }

    pub fn contains_key(&self, d: &DeclId) -> bool {
        self.get(d).is_some()
    }

    pub fn insert(&mut self, d: DeclId, v: Val) {
        // Declarations are usually set in increasing order: append.
        match self.0.last() {
            Some(l) if l.0 >= d => match self.0.binary_search_by_key(&d, |e| e.0) {
                Ok(i) => self.0[i].1 = v,
                Err(i) => self.0.insert(i, (d, v)),
            },
            _ => {
                // Exact room while small: most scopes hold one to three values.
                if self.0.len() == self.0.capacity() {
                    self.0
                        .reserve_exact(if self.0.len() < 8 { 1 } else { self.0.len() });
                }
                self.0.push((d, v))
            }
        }
    }

    pub fn values(&self) -> impl Iterator<Item = &Val> {
        self.0.iter().map(|e| &e.1)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&DeclId, &Val)> {
        self.0.iter().map(|e| (&e.0, &e.1))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// What evaluation needs besides the HIR: values per elaborated scope and signal objects.
#[derive(Default)]
pub struct Model {
    pub hier: Hierarchy,
    pub nets: NetGraph,
    /// Generic, generate parameter and constant values, per scope.
    pub(crate) vals: Vec<ScopeVals>,
    /// Signal objects by (scope, declaration).
    pub(crate) sigs: HashMap<(ScopeId, DeclId), ObjId>,
    /// Package constants.
    pub(crate) pkg: HashMap<DeclId, Val>,
    /// Shared variables by (region, declaration), and the object of each variable port
    /// (`Elaborated::var_ports`).
    pub(crate) shared_vars: std::collections::HashSet<(ScopeId, DeclId)>,
    pub(crate) var_ports: std::collections::BTreeMap<(ScopeId, DeclId), crate::VarObj>,
    /// `Elaborated::var_shapes`.
    pub(crate) var_shapes: HashMap<(Option<ScopeId>, DeclId), Shape>,
    /// User functions lowered for elaboration-time calls (ir.md 3.6): `None` when lowering
    /// failed (the reason is reported at the call).
    pub(crate) funcs: HashMap<DeclId, Result<Lowered1, crate::pkgcall::Fail>>,
    /// Results of pure calls from earlier elaborations (`callcache`, #539).
    pub(crate) calls: crate::callcache::CallCache,
    /// Package state for calls of functions that use package objects (#77).
    pub(crate) session: crate::pkgcall::Session,
    /// Package scopes by package unit, library scopes by library.
    pub(crate) pkg_scopes: HashMap<yavs_sem::UnitId, ScopeId>,
    pub(crate) lib_scopes: HashMap<yavs_sem::LibId, ScopeId>,
    /// Conversions of port associations, indexed by `PortEdge::to_formal`/`to_actual`.
    pub convs: Vec<ConvFn>,
    /// The declarative regions (yavs-sem scopes) of each elaborated scope, for external
    /// constant and variable names (#93).
    pub(crate) regions: HashMap<ScopeId, Vec<yavs_sem::ScopeId>>,
    /// Aliases of external shared variables, by (region, alias): the region and declaration
    /// of the variable (#93).
    pub(crate) ext_vars: HashMap<(ScopeId, DeclId), (ScopeId, DeclId)>,
    /// Aliases of external constants elaboration cannot evaluate (a run-time thunk computes
    /// the target), by (region, alias): the region and declaration of the constant (#255).
    pub(crate) ext_consts: HashMap<(ScopeId, DeclId), (ScopeId, DeclId)>,
    /// Aliases of the external names in package subprograms (#255): bound, with their
    /// package's scope, or why not (the subprogram fails when it is called).
    pub(crate) pkg_ext: HashMap<DeclId, ScopeId>,
    pub(crate) ext_unbound: HashMap<DeclId, String>,
    /// While a generic actual is evaluated in the parent: the instance whose generics are
    /// being bound. The actual's context subtype (`(others => ..)` for a generic of subtype
    /// `bit_vector(W - 1 downto 0)`) names the instance's earlier generics (LRM 6.5.6.2).
    pub(crate) actual_of: Option<ScopeId>,
    /// While the initial value of a signal is evaluated: a signal of the instance reads as
    /// its initial value (an `in` port's is its actual's, see `walk::Elab::edges`).
    pub(crate) sig_init: bool,
    /// The ranges of types and subtypes with a bound that is not globally static
    /// (`Db::dynamic_range`), evaluated once at their declaration in an elaborated scope, by
    /// left bound (#612); those of packages by left bound, read from the package data.
    pub(crate) ranges: HashMap<(ScopeId, ExprId), Bounds>,
    pub(crate) pkg_ranges: HashMap<ExprId, Bounds>,
    /// The package type or subtype declaring each such range of a run-time package, with the
    /// position of its left bound among the bounds the package evaluated (`pkg_range`).
    pub(crate) pkg_range_owners: Option<HashMap<ExprId, (DeclId, i64, Direction)>>,
}

/// How a scalar crosses the kernel: its element and its class.
#[derive(Clone, Copy, Debug)]
pub struct Enc {
    pub elem: Elem,
    /// BIT/BOOLEAN stored as std_ulogic codes 2/3.
    pub bit01: bool,
    pub class: Class,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Int,
    Real,
    Enum,
}

/// A conversion of a port association (LRM 6.5.7.1): a function of one value, run in
/// yavs-interp, or a type conversion. Each side is one kernel leaf (a scalar or an array of
/// scalars) or, for a function, a composite of several leaves.
#[derive(Clone)]
pub struct ConvFn {
    /// The function and the IR type of its result.
    pub(crate) func: Option<(
        std::sync::Arc<yavs_interp::Runtime>,
        yavs_ir::FuncId,
        Option<yavs_ir::TyId>,
    )>,
    pub from: Side,
    pub to: Side,
}

/// One end of a conversion: a scalar, an array leaf with its bounds as an IR descriptor, or
/// a composite of several leaves (a record, an array of composites) with the kernel length
/// and element of each leaf part, in leaf order, and how they map to IR memory.
#[derive(Clone)]
pub enum Side {
    Scalar(Enc),
    Array {
        elem: Elem,
        len: u64,
        desc: Vec<u8>,
    },
    Composite {
        map: std::sync::Arc<crate::whole::Map>,
        parts: Vec<(Elem, u64)>,
    },
    /// The leaves of a composite in a type conversion: each leaf's scalar and length.
    Leaves(Vec<(Enc, u64)>),
}

impl ConvFn {
    /// Convert kernel values: one per part of the source side, one per part of the target.
    pub fn apply(&self, vs: &[Value]) -> Result<Vec<Value>, String> {
        let one = || match vs {
            [v] => Ok(v),
            _ => Err(format!("a port conversion got {} values for one", vs.len())),
        };
        // A type conversion of composites: leaf by leaf, element by element (LRM 9.3.6).
        if let (Side::Leaves(a), Side::Leaves(b)) = (&self.from, &self.to) {
            if vs.len() != a.len() {
                return Err("a port conversion got the wrong number of parts".into());
            }
            return vs
                .iter()
                .zip(a.iter().zip(b))
                .map(|(v, ((ea, _), (eb, _)))| match v {
                    _ if ea.elem == eb.elem && ea.class == eb.class => Ok(v.clone()),
                    // Integers and reals, 8 bytes or 4 for a narrow integer element.
                    Value::Bytes(d)
                        if let (Elem::Bytes(wa @ (4 | 8)), Elem::Bytes(wb @ (4 | 8))) =
                            (ea.elem, eb.elem) =>
                    {
                        let mut out = Vec::with_capacity(d.len() / wa as usize * wb as usize);
                        for c in d.chunks(wa as usize) {
                            let x = conv_cell(ea.class, eb.class, cell(c))?;
                            if wb == 4 && i32::try_from(x as i64).is_err() {
                                return Err(format!("{} is outside the integer range", x as i64));
                            }
                            out.extend_from_slice(&x.to_le_bytes()[..wb as usize]);
                        }
                        Ok(Value::Bytes(out))
                    }
                    _ => Err("this type conversion of a port is not implemented yet".into()),
                })
                .collect();
        }
        let arg = match &self.from {
            Side::Leaves(_) => return Err("a type conversion of composites to a scalar".into()),
            Side::Scalar(e) => yavs_interp::CallVal::Cell(decode(e, one()?)?),
            Side::Array { desc, .. } => yavs_interp::CallVal::Composite {
                data: value_bytes(one()?),
                desc: desc.clone(),
            },
            Side::Composite { map, parts } => {
                if vs.len() != parts.len() {
                    return Err("a port conversion got the wrong number of parts".into());
                }
                let mut codes = Vec::new();
                for v in vs {
                    match v {
                        Value::Bytes(b) => codes.extend_from_slice(b),
                        Value::Logic(l) => codes.extend((0..l.len()).map(|i| l.get(i) as u8)),
                    }
                }
                yavs_interp::CallVal::Composite {
                    data: map.pack(&codes)?,
                    desc: map.desc.clone(),
                }
            }
        };
        let r = match &self.func {
            Some((rt, f, ret)) => {
                let ret = match self.to {
                    Side::Array { .. } | Side::Composite { .. } | Side::Leaves(_) => *ret,
                    Side::Scalar(_) => None,
                };
                yavs_interp::call_values(rt.clone(), *f, Default::default(), &[arg], ret)
                    .map_err(|e| e.to_string())?
                    .ok_or("the conversion function returned no value")?
            }
            None => match (&self.from, &self.to, arg) {
                (Side::Scalar(a), Side::Scalar(b), yavs_interp::CallVal::Cell(c)) => {
                    yavs_interp::CallVal::Cell(conv_cell(a.class, b.class, c)?)
                }
                _ => return Err("this type conversion of a port is not implemented yet".into()),
            },
        };
        match (&self.to, r) {
            (Side::Scalar(e), yavs_interp::CallVal::Cell(c)) => Ok(vec![encode_cell(e, c)]),
            (Side::Composite { map, parts }, yavs_interp::CallVal::Composite { data, desc }) => {
                let rt = match &self.func {
                    Some((rt, _, Some(ret))) => (rt, *ret),
                    _ => return Err("a composite conversion without a function".into()),
                };
                let codes = map.unpack(&rt.0.prog, rt.1, &data, &desc)?;
                let mut at = 0;
                let mut out = Vec::with_capacity(parts.len());
                for &(elem, n) in parts {
                    let w = match elem {
                        Elem::Logic => 1,
                        Elem::Bytes(w) => u64::from(w),
                    };
                    let b = codes
                        .get(at..at + (n * w) as usize)
                        .ok_or("conversion result of the wrong size")?;
                    at += (n * w) as usize;
                    out.push(match elem {
                        Elem::Logic => Value::Logic(LogicVec::from_fn(n as usize, |i| {
                            yavs_logic::StdULogic::from_code_pub(b[i])
                        })),
                        Elem::Bytes(_) => Value::Bytes(b.to_vec()),
                    });
                }
                Ok(out)
            }
            (Side::Array { elem, len, .. }, yavs_interp::CallVal::Composite { data, desc }) => {
                let n: u64 = desc
                    .as_chunks::<24>()
                    .0
                    .iter()
                    .map(|d| {
                        let w = |k: usize| i64::from_le_bytes(std::array::from_fn(|j| d[k + j]));
                        crate::hier::Bounds {
                            left: w(0),
                            dir: if w(16) == 0 {
                                Direction::To
                            } else {
                                Direction::Downto
                            },
                            right: w(8),
                        }
                        .len()
                    })
                    .product();
                if n != *len {
                    return Err(format!(
                        "the conversion result has {n} elements, the port {len}"
                    ));
                }
                Ok(vec![match elem {
                    Elem::Logic => {
                        let blocks: Vec<yavs_logic::Block> = data
                            .as_chunks::<32>()
                            .0
                            .iter()
                            .map(|c| {
                                std::array::from_fn(|k| {
                                    u64::from_le_bytes(std::array::from_fn(|j| c[k * 8 + j]))
                                })
                            })
                            .collect();
                        if blocks.len() != (n as usize).div_ceil(64) {
                            return Err("conversion result of the wrong size".into());
                        }
                        Value::Logic(LogicVec::from_repr(yavs_logic::Planes::from_blocks(
                            n as usize, blocks,
                        )))
                    }
                    Elem::Bytes(w) if data.len() as u64 == n * u64::from(*w) => Value::Bytes(data),
                    Elem::Bytes(_) => return Err("conversion result of the wrong size".into()),
                }])
            }
            _ => Err("the conversion result does not fit the port".into()),
        }
    }
}

/// A little-endian cell of up to 8 bytes; a 4-byte cell (a narrow integer element) is
/// sign-extended.
fn cell(b: &[u8]) -> u64 {
    if let Ok(x) = <[u8; 4]>::try_from(b) {
        return i64::from(i32::from_le_bytes(x)) as u64;
    }
    let mut w = [0u8; 8];
    let n = b.len().min(8);
    w[..n].copy_from_slice(&b[..n]);
    u64::from_le_bytes(w)
}

/// A scalar cell of class `a` as one of class `b` (a type conversion, LRM 9.3.6).
fn conv_cell(a: Class, b: Class, c: u64) -> Result<u64, String> {
    Ok(match (a, b) {
        (Class::Int, Class::Real) => (c as i64 as f64).to_bits(),
        (Class::Real, Class::Int) => {
            let r = f64::from_bits(c).round();
            if !(-9.2e18..9.2e18).contains(&r) {
                return Err(format!("{r} is outside the integer range"));
            }
            r as i64 as u64
        }
        _ => c,
    })
}

/// The type conversion end of a composite of shape `s`: each leaf's scalar and length.
fn leaves_of(db: &Db, s: &Shape) -> Option<Side> {
    s.leaves()
        .into_iter()
        .map(|(_, l, n)| match l {
            Shape::Scalar { ty, .. } => enc_of(db, *ty).map(|e| (e, n)),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
        .map(Side::Leaves)
}

/// A kernel value as IR memory: the bytes, or the bit-plane blocks of a logic value.
fn value_bytes(v: &Value) -> Vec<u8> {
    match v {
        Value::Bytes(b) => b.clone(),
        Value::Logic(l) => l
            .repr()
            .blocks()
            .iter()
            .flat_map(|b| b.iter().flat_map(|w| w.to_le_bytes()))
            .collect(),
    }
}

fn decode(e: &Enc, v: &Value) -> Result<u64, String> {
    match (e.elem, v) {
        (Elem::Logic, Value::Logic(l)) if l.len() == 1 => {
            let code = l.get(0) as u64;
            Ok(if e.bit01 {
                code.saturating_sub(2)
            } else {
                code
            })
        }
        (Elem::Bytes(n), Value::Bytes(b)) if b.len() == n as usize => Ok(cell(b)),
        _ => Err("a port conversion got a value of the wrong shape".into()),
    }
}

fn encode_cell(e: &Enc, c: u64) -> Value {
    match e.elem {
        Elem::Logic => {
            let code = if e.bit01 { c as u8 + 2 } else { c as u8 };
            Value::Logic(LogicVec::from_fn(1, |_| {
                yavs_logic::StdULogic::from_code_pub(code)
            }))
        }
        Elem::Bytes(n) => Value::Bytes(c.to_le_bytes()[..n as usize].to_vec()),
    }
}

/// The conversion end of a signal of shape `s`: a scalar or an array of scalars.
fn side_of(db: &Db, s: &Shape) -> Option<Side> {
    match s {
        Shape::Scalar { ty, .. } => enc_of(db, *ty).map(Side::Scalar),
        Shape::Array { dims, elem, .. } => {
            let Shape::Scalar { elem: e, .. } = **elem else {
                return None;
            };
            let desc = dims
                .iter()
                .flat_map(|b| {
                    [b.left, b.right, i64::from(b.dir == Direction::Downto)]
                        .into_iter()
                        .flat_map(|x| x.to_le_bytes())
                })
                .collect();
            Some(Side::Array {
                elem: e,
                len: s.len(),
                desc,
            })
        }
        Shape::Record { .. } => None,
    }
}

/// The kernel encoding of scalar type `t`.
pub(crate) fn enc_of(db: &Db, t: TypeId) -> Option<Enc> {
    let class = match &db.types[t].kind {
        TypeKind::Enum { .. } => Class::Enum,
        TypeKind::Floating | TypeKind::UniversalReal => Class::Real,
        k if k.is_scalar() => Class::Int,
        _ => return None,
    };
    let elem = elem_of(db, t);
    Some(Enc {
        elem,
        bit01: elem == Elem::Logic && !is_std_ulogic(db, t),
        class,
    })
}

/// A conversion `f` (a function or a type) from a signal of shape `from` to one of `to`.
pub(crate) fn conv_fn(db: &Db, f: DeclId, from: &Shape, to: &Shape, at: Span) -> R<ConvFn> {
    let (fe, te) = (side_of(db, from), side_of(db, to));
    let is_fn = matches!(db.decls[f].kind, DeclKind::Subprogram(_));
    let mut pty = None;
    if !is_fn && !matches!((&fe, &te), (Some(Side::Scalar(_)), Some(Side::Scalar(_)))) {
        // Closely related composites (LRM 9.3.6): the same leaves, element by element.
        let (Some(Side::Leaves(a)), Some(Side::Leaves(b))) =
            (leaves_of(db, from), leaves_of(db, to))
        else {
            return Err(err(
                codes::INTERNAL,
                at,
                "a type conversion of a scalar and a composite",
                "here",
            ));
        };
        let na: Vec<u64> = a.iter().map(|x| x.1).collect();
        let nb: Vec<u64> = b.iter().map(|x| x.1).collect();
        if na != nb {
            return Err(err(
                codes::PORT_MISMATCH,
                at,
                format!(
                    "the type conversion gives {} elements, the target has {}",
                    na.iter().sum::<u64>(),
                    nb.iter().sum::<u64>()
                ),
                "conversion",
            ));
        }
        return Ok(ConvFn {
            func: None,
            from: Side::Leaves(a),
            to: Side::Leaves(b),
        });
    }
    let func = match &db.decls[f].kind {
        DeclKind::Subprogram(_) => {
            let l = lower_one(db, f, false).map_err(|e| match e {
                crate::pkgcall::Fail::Lower(why) => not_callable(db, f, at, &why),
                crate::pkgcall::Fail::Unsupported(w) => err(codes::INTERNAL, at, w, "here"),
                crate::pkgcall::Fail::Runtime(w) => err(codes::INTERNAL, at, w, "here"),
            })?;
            pty = l.sig.params.first().copied();
            Some((l.rt, l.sig.func, l.sig.ret))
        }
        _ => None,
    };
    // A composite side maps its kernel leaves to the IR value of the function's parameter or
    // result (whole.rs, as for a composite resolved as a whole).
    let composite = |side: Option<Side>, ty: Option<yavs_ir::TyId>, shape: &Shape| -> R<Side> {
        if let Some(s) = side {
            return Ok(s);
        }
        let fail = |m: String| err(codes::INTERNAL, at, m, "conversion");
        let (Some((rt, _, _)), Some(ty)) = (&func, ty) else {
            return Err(fail("composite conversion without a typed function".into()));
        };
        let map = crate::whole::Map::new(&rt.prog, ty, shape).map_err(fail)?;
        let parts = shape
            .leaves()
            .into_iter()
            .map(|(_, leaf, n)| match leaf {
                Shape::Scalar { elem, .. } => (*elem, n),
                _ => (Elem::Bytes(0), 0),
            })
            .collect();
        Ok(Side::Composite {
            map: std::sync::Arc::new(map),
            parts,
        })
    };
    let rty = func.as_ref().and_then(|f| f.2);
    Ok(ConvFn {
        from: composite(fe, pty, from)?,
        to: composite(te, rty, to)?,
        func,
    })
}

/// A function lowered on its own into a program, for `yavs_interp::call`.
pub(crate) struct Lowered1 {
    rt: std::sync::Arc<yavs_interp::Runtime>,
    sig: crate::pkgcall::Sig,
    /// The constants and generics of enclosing regions it reads, and the bytes of instance
    /// data they sit in (`Lowerer::capture_free_objects`).
    free: Vec<yavs_lower::FreeObj>,
    inst_size: u32,
}

pub(crate) type R<T> = Result<T, Box<Diagnostic>>;

pub(crate) fn err(
    code: &'static str,
    span: Span,
    msg: impl Into<String>,
    label: &str,
) -> Box<Diagnostic> {
    Box::new(Diagnostic::error(code, msg).with_primary(span, label))
}

impl Model {
    /// The signal object `d` names, seen from `scope`.
    pub fn signal(&self, scope: ScopeId, d: DeclId) -> Option<ObjId> {
        let mut cur = Some(scope);
        while let Some(s) = cur {
            if let Some(&o) = self.sigs.get(&(s, d)) {
                return Some(o);
            }
            cur = self.hier.scope(s).parent;
        }
        None
    }

    pub(crate) fn set(&mut self, scope: ScopeId, d: DeclId, v: Val) {
        let i = scope.index();
        if self.vals.len() <= i {
            self.vals.resize_with(i + 1, ScopeVals::default);
        }
        self.vals[i].insert(d, v);
    }

    fn get(&self, scope: Option<ScopeId>, d: DeclId) -> Option<&Val> {
        for start in [scope, self.actual_of] {
            let mut cur = start;
            while let Some(s) = cur {
                if let Some(v) = self.vals.get(s.index()).and_then(|m| m.get(&d)) {
                    return Some(v);
                }
                cur = self.hier.scope(s).parent;
            }
        }
        self.pkg.get(&d)
    }

    /// Evaluate `e` in `scope` (`None`: package level).
    pub fn eval(&mut self, db: &Db, e: ExprId, scope: Option<ScopeId>) -> R<Val> {
        let ex = &db.exprs[e];
        if let Some(c) = ex.value {
            return Ok(db.consts[c].clone());
        }
        match &ex.kind {
            ExprKind::Const(c) => return Ok(db.consts[*c].clone()),
            ExprKind::Obj(d) => return self.value_of(db, *d, scope, ex.span),
            ExprKind::External(x) => return self.external_value(db, *x, scope),
            // `'path_name`/`'instance_name` of a design unit names the instance being
            // elaborated (LRM 16.2.5), so a constant or generic actual built from it has its
            // value now (#314), the same text as the run-time one (`Elaborated::name_of`).
            // ponytail: other named entities stay E2106 here (their declaring region is not
            // known before every object is registered); handle them when a design needs it.
            ExprKind::Attr {
                attr: attr @ (PredefAttr::PathName | PredefAttr::InstanceName),
                prefix: AttrPrefix::Entity(d),
                ..
            } if let (Some(s), DeclKind::Unit(_) | DeclKind::Label(_)) =
                (scope, &db.decls[*d].kind) =>
            {
                // An instance label is defined in the generic map of its own
                // instantiation (LRM 16.2.5): it names the instance whose generics
                // are being bound (`actual_of`), not the parent the actual is
                // evaluated in. Another label's name is not known yet (#619).
                let i = if matches!(db.decls[*d].kind, DeclKind::Label(_)) {
                    match self.actual_of {
                        Some(i) if db.decls[*d].name.key() == self.hier.scope(i).name.key => i,
                        _ => {
                            return Err(err(
                                codes::NOT_EVALUATED,
                                ex.span,
                                "the name of another instance is not known during elaboration yet",
                                "here",
                            ));
                        }
                    }
                } else {
                    self.hier.instance_of(s)
                };
                let text = if *attr == PredefAttr::InstanceName {
                    self.hier.instance_name(i, db, crate::Case::Lrm)
                } else {
                    self.hier.path_name(i, crate::Case::Lrm)
                };
                return Ok(Val::Array(ArrayConst {
                    bounds: [(1, Direction::To, text.len() as i64)].into(),
                    elems: Elems::Bytes(text.into_bytes().into()),
                }));
            }
            ExprKind::Attr { attr, prefix, args }
                if matches!(
                    attr,
                    PredefAttr::Left
                        | PredefAttr::Right
                        | PredefAttr::High
                        | PredefAttr::Low
                        | PredefAttr::Length
                        | PredefAttr::Ascending
                ) && !matches!(prefix, AttrPrefix::Subtype(st) if matches!(
                    db.types[db.base(*st)].kind,
                    TypeKind::Floating | TypeKind::UniversalReal
                )) =>
            {
                let dim = match args.first() {
                    Some(a) => int(&self.eval(db, *a, scope)?, db.exprs[*a].span)? - 1,
                    None => 0,
                };
                // A scalar subtype's range may name an object (`integer range p'range`, the
                // index subtype of an aggregate's range choice, #311): evaluated here, since
                // folding knows no signal bounds.
                let b = match prefix {
                    AttrPrefix::Subtype(st) if db.types[db.base(*st)].kind.is_scalar() => {
                        self.scalar_bounds(db, *st, scope, ex.span)?
                    }
                    _ => self.prefix_bounds(db, prefix, dim as u32, scope, ex.span)?,
                };
                let (lo, hi) = b.low_high();
                let wrap = |v: i64| match db.types[db.base(ex.subtype)].kind {
                    TypeKind::Enum { .. } => Val::Enum(v as u32),
                    TypeKind::Physical { .. } => Val::Phys(v),
                    _ => Val::Int(v),
                };
                return Ok(match attr {
                    PredefAttr::Left => wrap(b.left),
                    PredefAttr::Right => wrap(b.right),
                    PredefAttr::Low => wrap(lo),
                    PredefAttr::High => wrap(hi),
                    PredefAttr::Length => Val::Int(b.len() as i64),
                    _ => Val::Enum(u32::from(b.dir == Direction::To)),
                });
            }
            _ => {}
        }
        // `now` is 0 fs during elaboration, as without a kernel (decisions.md
        // "Elaboration-time calls"): a constant that bounds a subtype may read it (#620).
        if let ExprKind::Call { callee, .. } = &ex.kind
            && let Some(sp) = db.decls[*callee].kind.subprogram()
            && let yavs_sem::decl::Body::Native(sym) = &sp.body
            && sym.as_str().starts_with("yavs.standard.now.")
        {
            return Ok(Val::Phys(0));
        }
        if let ExprKind::Call {
            callee,
            args,
            method_of: None,
            ..
        } = &ex.kind
            && let Some(sp) = db.decls[*callee].kind.subprogram()
            && matches!(sp.body, yavs_sem::decl::Body::Hir { .. })
        {
            return self.call(db, e, *callee, args, scope);
        }
        if let Some(v) = self.composite(db, e, scope)? {
            return Ok(v);
        }
        if let Some(v) = self.attr_text(db, e, scope)? {
            return Ok(v);
        }
        let mut inner: Option<Box<Diagnostic>> = None;
        let r = db.fold_with(e, &mut |c| match self.eval(db, c, scope) {
            Ok(v) => Some(v),
            Err(d) => {
                inner.get_or_insert(d);
                None
            }
        });
        match r {
            Ok(Some(v)) => Ok(v),
            Ok(None) => Err(inner.unwrap_or_else(|| {
                Box::new(
                    Diagnostic::error(
                        codes::NOT_EVALUATED,
                        "this expression cannot be evaluated during elaboration yet",
                    )
                    .with_primary(ex.span, "needs the IR interpreter")
                    .with_note(
                        "yavs folds globally static expressions over generics and constants, \
                         evaluates composite values and runs function calls in yavs-interp; \
                         this kind of expression is not evaluated during elaboration yet",
                    ),
                )
            })),
            Err(why) => Err(err(
                codes::EVAL_FAILED,
                ex.span,
                format!("evaluation fails during elaboration: {why}"),
                why,
            )),
        }
    }

    /// A call of a user function during elaboration: scalar arguments and result, run in
    /// yavs-interp after lowering the function (and what it calls) with yavs-lower.
    fn call(
        &mut self,
        db: &Db,
        e: ExprId,
        callee: DeclId,
        args: &[yavs_sem::hir::Arg],
        scope: Option<ScopeId>,
    ) -> R<Val> {
        let ex = &db.exprs[e];
        let Some(sp) = db.decls[callee].kind.subprogram() else {
            return Err(err(
                codes::INTERNAL,
                ex.span,
                "internal error: not a subprogram",
                "here",
            ));
        };
        let not_scalar = |what: &str| {
            Box::new(
                Diagnostic::error(
                    codes::NOT_EVALUATED,
                    format!("this call cannot be evaluated during elaboration yet: {what}"),
                )
                .with_primary(ex.span, "called during elaboration")
                .with_note(
                    "elaboration-time calls take constant `in` parameters of any type but \
                     access and file types",
                ),
            )
        };
        let mut vals = Vec::with_capacity(sp.params.len());
        for (i, p) in sp.params.iter().enumerate() {
            if p.mode != yavs_sem::hir::Mode::In || p.class != ObjClass::Constant {
                return Err(not_scalar("a parameter that is not an `in` constant"));
            }
            let a = args.iter().find(|a| a.formal as usize == i);
            vals.push(match a.map(|a| &a.actual) {
                Some(yavs_sem::hir::Actual::Expr(x)) => self.eval(db, *x, scope)?,
                Some(yavs_sem::hir::Actual::Default) | None => match p.default {
                    Some(d) => self.eval(db, d, scope)?,
                    None => return Err(not_scalar("a missing argument")),
                },
                Some(_) => return Err(not_scalar("an individual or converted argument")),
            });
        }
        if sp.ret.is_none() {
            return Err(not_scalar("not a function"));
        }
        // A named return value (2019, 4.2.1): a value of the call's result subtype goes
        // last, for its bounds (yavs-lower passes the descriptor the same way).
        if let yavs_sem::decl::Body::Hir { scope: ps, .. } = &sp.body
            && db.ret_names.contains_key(ps)
        {
            if db.types[db.base(ex.subtype)].kind.is_scalar() {
                // A scalar: its range only (`pkgcall::run` passes it as a descriptor).
                let b = self.scalar_bounds(db, ex.subtype, scope, ex.span)?;
                vals.push(Val::Array(ArrayConst {
                    bounds: Box::new([(b.left, b.dir, b.right)]),
                    elems: Elems::Runs(Box::new([])),
                }));
            } else {
                let Some(shape) = self.shape(db, ex.subtype, scope, ex.span)? else {
                    return Err(not_scalar("a named return value of this subtype"));
                };
                vals.push(shape_value(&shape));
            }
        }
        let failed = |f: crate::pkgcall::Fail| match f {
            crate::pkgcall::Fail::Unsupported(w) => not_scalar(w),
            crate::pkgcall::Fail::Runtime(msg) => err(
                codes::EVAL_FAILED,
                ex.span,
                format!("evaluation fails during elaboration: {msg}"),
                "in this call",
            ),
            crate::pkgcall::Fail::Lower(why) => not_callable(db, callee, ex.span, &why),
        };
        let key = crate::callcache::cacheable(db, callee)
            .then(|| crate::callcache::key(callee, &vals))
            .flatten();
        if let Some(k) = &key
            && let Some(hit) = self.calls.get(db, k)
        {
            yavs_interp::replay_reports(&hit.reports);
            crate::callcache::count(true);
            return Ok(hit.val);
        }
        if key.is_some() {
            crate::callcache::count(false);
        }
        match self
            .funcs
            .entry(callee)
            .or_insert_with(|| lower_one(db, callee, true))
        {
            Ok(l) => {
                yavs_interp::Runtime::promote_hot(&mut l.rt);
                let (rt, sig, free, size) =
                    (l.rt.clone(), l.sig.clone(), l.free.clone(), l.inst_size);
                let inst = self.free_data(db, &rt, &free, size, scope, ex.span)?;
                // A call that only computed, from nothing but its arguments, is kept (#539).
                let Some(k) = key.filter(|_| free.is_empty()) else {
                    return crate::pkgcall::run(&rt, &sig, &vals, None, inst).map_err(failed);
                };
                // The traced call may restart with functions optimised, which needs the runtime
                // to itself (`pkgcall::run_restarting`).
                drop(rt);
                let Some(Ok(l)) = self.funcs.get_mut(&callee) else {
                    return Err(err(
                        codes::INTERNAL,
                        ex.span,
                        "internal error: lowered call lost",
                        "here",
                    ));
                };
                let (r, trace) = crate::pkgcall::run_traced(&mut l.rt, &sig, &vals, inst);
                let v = r.map_err(failed)?;
                if trace.tainted.is_none() {
                    let hit = crate::callcache::Hit {
                        val: v.clone(),
                        reports: trace.reports,
                    };
                    self.calls.put(db, k, &hit);
                }
                Ok(v)
            }
            // A function that uses package objects (shared variables, say) runs against the
            // elaborated packages, in elaboration order (LRM 14.4); one that reads shared
            // variables of its region runs there too, with their objects.
            Err(_) => match self.session_call(db, callee, vals, scope, ex.span, key)? {
                Ok(v) => Ok(v),
                Err(f) => Err(failed(f)),
            },
        }
    }

    /// Call `callee` in the session (package state, shared variables of regions): the objects
    /// of its region that it reads are given as the values they have in `scope`; a shared
    /// variable's object is created the first time a call needs it.
    fn session_call(
        &mut self,
        db: &Db,
        callee: DeclId,
        vals: Vec<Val>,
        scope: Option<ScopeId>,
        span: Span,
        key: Option<Vec<u8>>,
    ) -> R<Result<Val, crate::pkgcall::Fail>> {
        let needs = match self.session.needs(db, callee) {
            Ok(n) => n,
            Err(f) => return Ok(Err(f)),
        };
        if needs.free.is_empty() {
            // A pure function of a package reads nothing but its arguments and the package
            // constants, which are the closure's: its result is kept like a lowered call's (#539).
            let Some(k) = key else {
                return Ok(self.session.call(db, callee, vals, None));
            };
            let mut trace = yavs_interp::CallTrace::default();
            let r = self.session.call(db, callee, vals, Some(&mut trace));
            if let Ok(v) = &r
                && trace.tainted.is_none()
                && self.session.state_is_pure()
            {
                let hit = crate::callcache::Hit {
                    val: v.clone(),
                    reports: trace.reports,
                };
                self.calls.put(db, k, &hit);
            }
            return Ok(r);
        }
        let items = match self.fill_items(db, &needs, scope, span) {
            Ok(Ok(items)) => items,
            Ok(Err(f)) => {
                self.session.abandon();
                return Ok(Err(f));
            }
            Err(d) => {
                self.session.abandon();
                return Err(d);
            }
        };
        Ok(self.session.call_in(db, callee, vals, items))
    }

    /// What a session call reads of the regions around it (`needs`): the values the
    /// constants and generics have in `scope`, and the shared variables, created now when they
    /// do not exist yet.
    fn fill_items(
        &mut self,
        db: &Db,
        needs: &crate::pkgcall::Needs,
        scope: Option<ScopeId>,
        span: Span,
    ) -> R<Result<Vec<(DeclId, crate::pkgcall::FillItem)>, crate::pkgcall::Fail>> {
        use crate::pkgcall::FillItem;
        let mut items = Vec::new();
        for f in &needs.free {
            if !f.var {
                let v = self.value_of(db, f.decl, scope, span)?;
                items.push((f.decl, FillItem::Val(v)));
                continue;
            }
            let name = &db.decls[f.decl].name;
            let Some(region) = self.var_region(f.decl, scope) else {
                return Err(err(
                    codes::NOT_EVALUATED,
                    span,
                    format!("shared variable `{name}` does not exist yet where this is called"),
                    "called during elaboration",
                ));
            };
            let key = (region, f.decl);
            if self.session.var(key).is_none() {
                let n = match self.session.init_needs(db, f.decl) {
                    Ok(n) => n,
                    Err(e) => return Ok(Err(e)),
                };
                let titems = match self.fill_items(db, &n, Some(region), span)? {
                    Ok(i) => i,
                    Err(e) => return Ok(Err(e)),
                };
                if let Err(e) = self.session.create_var(db, key, titems) {
                    return Ok(Err(e));
                }
            }
            items.push((f.decl, FillItem::Var(key)));
        }
        Ok(Ok(items))
    }

    /// The elaborated region, `scope` or one around it, that declares shared variable `decl`.
    fn var_region(&self, decl: DeclId, scope: Option<ScopeId>) -> Option<ScopeId> {
        let mut cur = scope;
        while let Some(r) = cur {
            if self.shared_vars.contains(&(r, decl)) {
                return Some(r);
            }
            cur = self.hier.scope(r).parent;
        }
        None
    }

    /// The instance data a function called in `scope` reads its enclosing regions' constants
    /// from: their values as they are in `scope` (`None` when it reads none).
    fn free_data(
        &mut self,
        db: &Db,
        rt: &std::sync::Arc<yavs_interp::Runtime>,
        free: &[yavs_lower::FreeObj],
        size: u32,
        scope: Option<ScopeId>,
        span: Span,
    ) -> R<Option<yavs_interp::InstData>> {
        if free.is_empty() {
            return Ok(None);
        }
        let mut inst = yavs_interp::InstData {
            data: vec![0; size as usize],
            ..Default::default()
        };
        for f in free {
            let v = self.value_of(db, f.decl, scope, span)?;
            let name = &db.decls[f.decl].name;
            let off = f.off as usize;
            match crate::value::encode(&rt.prog, f.ty, &v) {
                Some(yavs_interp::CallVal::Cell(c)) => {
                    let n = rt
                        .prog
                        .types
                        .get(f.ty)
                        .and_then(|t| t.scalar_size())
                        .unwrap_or(8) as usize;
                    inst.data[off..off + n].copy_from_slice(&c.to_le_bytes()[..n]);
                }
                Some(yavs_interp::CallVal::Composite { data, desc }) => {
                    yavs_lower::UnitIr::write_composite(f.off, &data, &desc, &mut inst.data);
                }
                None => {
                    return Err(Box::new(
                        Diagnostic::error(
                            codes::NOT_EVALUATED,
                            format!("`{name}` cannot be passed to a call during elaboration yet"),
                        )
                        .with_primary(span, "called during elaboration"),
                    ));
                }
            }
        }
        Ok(Some(inst))
    }

    pub(crate) fn value_of(
        &mut self,
        db: &Db,
        d: DeclId,
        scope: Option<ScopeId>,
        span: Span,
    ) -> R<Val> {
        if let Some(v) = self.get(scope, d) {
            return Ok(v.clone());
        }
        let decl = &db.decls[d];
        if let DeclKind::Alias(yavs_sem::decl::Alias::Object { target, .. }) = &decl.kind {
            return self.eval(db, *target, scope);
        }
        let DeclKind::Object(o) = &decl.kind else {
            return Err(err(
                codes::NOT_EVALUATED,
                span,
                format!("`{}` has no value during elaboration", decl.name),
                "not a constant",
            ));
        };
        match (o.class, &o.role, o.default) {
            (ObjClass::Constant, ObjRole::Plain, Some(init)) => {
                let in_pkg = matches!(
                    db.scopes[decl.scope].kind,
                    SemScope::Package | SemScope::PackageBody
                );
                if in_pkg {
                    Self::check_body_elaborated(db, init, decl.scope)?;
                    // An impure initial value ran when its package was elaborated: read
                    // the value from the package data, never call it again (#77).
                    let v = if calls_impure(db, init) {
                        self.session.package_const(db, d).map_err(|f| match f {
                            crate::pkgcall::Fail::Runtime(m) => err(
                                codes::EVAL_FAILED,
                                span,
                                format!("evaluation fails during elaboration: {m}"),
                                "while reading this package constant",
                            ),
                            crate::pkgcall::Fail::Lower(m) => err(
                                codes::NOT_EVALUATED,
                                span,
                                format!(
                                    "`{}` cannot be read during elaboration yet: {m}",
                                    decl.name
                                ),
                                "read during elaboration",
                            ),
                            crate::pkgcall::Fail::Unsupported(w) => err(
                                codes::NOT_EVALUATED,
                                span,
                                format!(
                                    "`{}` cannot be read during elaboration yet: {w}",
                                    decl.name
                                ),
                                "read during elaboration",
                            ),
                        })?
                    } else {
                        self.eval(db, init, None)?
                    };
                    let v = self.as_subtype(db, o.subtype, v, None, decl.span)?;
                    self.pkg.insert(d, v.clone());
                    Ok(v)
                } else {
                    let v = self.eval(db, init, scope)?;
                    let v = self.as_subtype(db, o.subtype, v, scope, decl.span)?;
                    if let Some(s) = scope {
                        self.set(s, d, v.clone());
                    }
                    Ok(v)
                }
            }
            // A deferred constant (LRM 4.8): the value of the full declaration in the package
            // body, which is elaborated before the design that uses it (LRM 14.4.2).
            (ObjClass::Constant, ObjRole::Deferred, _) => {
                let Some(full) = yavs_lower::deferred_completion(db, d) else {
                    return Err(err(
                        codes::NOT_EVALUATED,
                        span,
                        format!("deferred constant `{}` has no full declaration", decl.name),
                        "used here during elaboration",
                    ));
                };
                let v = self.value_of(db, full, None, span)?;
                self.pkg.insert(d, v.clone());
                Ok(v)
            }
            (ObjClass::Constant, ObjRole::Generic | ObjRole::GenerateParam, _) => Err(Box::new(
                Diagnostic::error(
                    codes::INTERNAL,
                    format!("internal error: generic `{}` has no value", decl.name),
                )
                .with_primary(span, "while elaborating this")
                .with_note("this is a bug in yavs"),
            )),
            (ObjClass::Signal, _, _) if self.sig_init => {
                let Some(v) = scope.and_then(|s| self.signal_init(db, d, s)) else {
                    return Err(err(
                        codes::NOT_EVALUATED,
                        span,
                        format!("the initial value of `{}` cannot be read yet", decl.name),
                        "an array of composites or an unelaborated signal",
                    ));
                };
                Ok(v)
            }
            _ => Err(err(
                codes::NOT_EVALUATED,
                span,
                format!(
                    "`{}` is not a constant: it has no value during elaboration",
                    decl.name
                ),
                "needs a globally static value",
            )),
        }
    }

    /// The initial value of signal `d` of the instance `scope` from its leaves; `None` for
    /// an array of composites (its leaves are not one run of elements).
    fn signal_init(&self, db: &Db, d: DeclId, scope: ScopeId) -> Option<Val> {
        let obj = self.nets.obj(self.signal(scope, d)?);
        let mut next = obj.leaves.0.index();
        init_val(db, &obj.shape, &self.nets, &mut next)
    }

    /// Array value `v` of a constant of subtype `st`: with the bounds of `st` when it is
    /// constrained, not those of the initial value (LRM 6.4.2.2, 5.3.2.2).
    pub(crate) fn as_subtype(
        &mut self,
        db: &Db,
        st: SubtypeId,
        v: Val,
        scope: Option<ScopeId>,
        span: Span,
    ) -> R<Val> {
        if !matches!(v, Val::Array(_) | Val::Record(_)) {
            return Ok(v);
        }
        // A record whose fields have no shape (an access or file field) keeps its bounds.
        let shape = match (&v, self.shape(db, st, scope, span)) {
            (_, Ok(Some(sh))) => sh,
            (Val::Record(_), _) | (_, Ok(None)) => return Ok(v),
            (_, Err(e)) => return Err(e),
        };
        match rebound(&v, &shape) {
            Some(v) => Ok(v),
            None => Err(err(
                codes::NOT_EVALUATED,
                span,
                "the initial value does not match the length of the constant's subtype",
                "bound error",
            )),
        }
    }

    /// LRM 14.4.2: a package constant whose initial value calls a subprogram of the same
    /// package, whose body (in the package body) is not elaborated yet. sim1 and sim3
    /// report it; fail early with both spans.
    pub(crate) fn check_body_elaborated(db: &Db, e: ExprId, pkg: yavs_sem::ScopeId) -> R<()> {
        let Some((at, callee)) = find_call(db, e, &mut |c| {
            let cd = &db.decls[c];
            let user = cd.kind.subprogram().is_some_and(|s| {
                !matches!(
                    s.body,
                    yavs_sem::decl::Body::Implicit(_) | yavs_sem::decl::Body::Native(_)
                )
            });
            user && cd.scope == pkg && db.scopes[pkg].kind == SemScope::Package
        }) else {
            return Ok(());
        };
        let cd = &db.decls[callee];
        Err(Box::new(
            Diagnostic::error(
                codes::BODY_NOT_ELABORATED,
                format!(
                    "subprogram `{}` is called before its body is elaborated",
                    cd.name
                ),
            )
            .with_primary(db.exprs[at].span, "called while its package is elaborated")
            .with_secondary(cd.span, "declared here; its body is in the package body")
            .with_note("the package declaration is elaborated before its body (LRM 14.4.2)")
            .with_help(
                "move the constant into the package body, or the function into another package",
            ),
        ))
    }

    /// Bounds of dimension `dim` of an attribute prefix (`s'range`, `t'length`).
    fn prefix_bounds(
        &mut self,
        db: &Db,
        p: &AttrPrefix,
        dim: u32,
        scope: Option<ScopeId>,
        span: Span,
    ) -> R<Bounds> {
        let shape = match p {
            AttrPrefix::Subtype(st) => {
                if db.types[db.base(*st)].kind.is_scalar() {
                    return self.scalar_bounds(db, *st, scope, span);
                }
                self.shape(db, *st, scope, span)?
            }
            AttrPrefix::Expr(e) => self.expr_shape(db, *e, scope)?,
            AttrPrefix::Entity(_) => None,
        };
        match shape {
            Some(Shape::Array { dims, .. }) if (dim as usize) < dims.len() => {
                Ok(dims[dim as usize])
            }
            _ => Err(err(
                codes::NOT_EVALUATED,
                span,
                "the bounds of this prefix are not known during elaboration",
                "not a constrained array",
            )),
        }
    }

    /// The shape of a signal or of a record element of one (`p.data`, `b.resp.user`): a
    /// signal has no value during elaboration, but its bounds are known (a port's from its
    /// actual, a package signal's from its constrained subtype). `None` when `e` names no
    /// signal.
    fn sig_shape(&mut self, db: &Db, e: ExprId, scope: Option<ScopeId>) -> R<Option<Shape>> {
        match &db.exprs[e].kind {
            ExprKind::Obj(d) => {
                if let Some(s) = scope
                    && let Some(o) = self.signal(s, *d)
                {
                    return Ok(Some(self.nets.obj(o).shape.clone()));
                }
                if let DeclKind::Object(o) = &db.decls[*d].kind
                    && o.class == ObjClass::Signal
                    && matches!(
                        db.scopes[db.decls[*d].scope].kind,
                        SemScope::Package | SemScope::PackageBody
                    )
                {
                    return self.shape(db, o.subtype, None, db.exprs[e].span);
                }
                Ok(None)
            }
            ExprKind::Field { prefix, elem } => Ok(match self.sig_shape(db, *prefix, scope)? {
                Some(Shape::Record { mut fields, .. }) if (*elem as usize) < fields.len() => {
                    Some(fields.swap_remove(*elem as usize).1)
                }
                _ => None,
            }),
            // An element of an array of records (`a(0).data`): every element has the same
            // shape, but the index must still be in range.
            ExprKind::Index { prefix, indices } => {
                let Some(Shape::Array { dims, elem, .. }) = self.sig_shape(db, *prefix, scope)?
                else {
                    return Ok(None);
                };
                if dims.len() != indices.len() {
                    return Ok(None);
                }
                for (&i, b) in indices.iter().zip(&dims) {
                    let span = db.exprs[i].span;
                    let v = int(&self.eval(db, i, scope)?, span)?;
                    if b.pos(v).is_none() {
                        return Err(err(
                            codes::EVAL_FAILED,
                            span,
                            format!("index {v} is out of the bounds of the prefix"),
                            "out of range",
                        ));
                    }
                }
                Ok(Some(*elem))
            }
            _ => Ok(None),
        }
    }

    /// The shape of an object expression: a signal (or a record element of one), or a
    /// constant's value.
    fn expr_shape(&mut self, db: &Db, e: ExprId, scope: Option<ScopeId>) -> R<Option<Shape>> {
        if let Some(sh) = self.sig_shape(db, e, scope)? {
            return Ok(Some(sh));
        }
        let span = db.exprs[e].span;
        // An object of a constrained subtype has its bounds, whether or not it has a value now:
        // `d'range` of a parameter `d : word` in `s(d'range) <= d` (#522).
        let st = db.exprs[e].subtype;
        if db.constrainedness(st) == yavs_sem::types::Constrainedness::Full
            && !matches!(
                db.subtypes[st].constraint,
                yavs_sem::types::Constraint::FromObject { .. }
            )
            && let Ok(Some(sh @ Shape::Array { .. })) = self.shape(db, st, scope, span)
        {
            return Ok(Some(sh));
        }
        match self.eval(db, e, scope)? {
            Val::Array(ArrayConst { bounds, .. }) => Ok(Some(Shape::Array {
                ty: db.base(db.exprs[e].subtype),
                dims: bounds
                    .iter()
                    .map(|&(left, dir, right)| Bounds { left, dir, right })
                    .collect(),
                // Only the bounds are used (attribute prefixes).
                elem: Box::new(Shape::Scalar {
                    ty: db.base(db.exprs[e].subtype),
                    sub: db.exprs[e].subtype,
                    range: None,
                    elem: Elem::Bytes(1),
                    left: Val::Int(0),
                    res: None,
                }),
                res: None,
            })),
            _ => Err(err(
                codes::NOT_EVALUATED,
                span,
                "not an array",
                "bounds needed here",
            )),
        }
    }

    /// Evaluate a range constraint.
    pub fn range(&mut self, db: &Db, r: &RangeC, scope: Option<ScopeId>, span: Span) -> R<Bounds> {
        if let Some(left) = db.dynamic_range(r) {
            let mut cur = scope;
            while let Some(s) = cur {
                if let Some(&b) = self.ranges.get(&(s, left)) {
                    return Ok(b);
                }
                cur = self.hier.scope(s).parent;
            }
            if let Some(b) = self.pkg_range(db, left, span)? {
                return Ok(b);
            }
        }
        match r {
            RangeC::Explicit { left, dir, right } => Ok(Bounds {
                left: int(&self.eval(db, *left, scope)?, db.exprs[*left].span)?,
                dir: *dir,
                right: int(&self.eval(db, *right, scope)?, db.exprs[*right].span)?,
            }),
            RangeC::Attr {
                prefix,
                reverse,
                dim,
            } => {
                let b = self.prefix_bounds(db, prefix, dim.saturating_sub(1), scope, span)?;
                Ok(if *reverse {
                    Bounds {
                        left: b.right,
                        dir: match b.dir {
                            Direction::To => Direction::Downto,
                            Direction::Downto => Direction::To,
                        },
                        right: b.left,
                    }
                } else {
                    b
                })
            }
        }
    }

    /// The bounds of range `left` of a type or subtype of a run-time package, as its package
    /// evaluated them (the elaboration of the package, #612); `None` for another range.
    fn pkg_range(&mut self, db: &Db, left: ExprId, span: Span) -> R<Option<Bounds>> {
        if let Some(&b) = self.pkg_ranges.get(&left) {
            return Ok(Some(b));
        }
        let owners = self.pkg_range_owners.get_or_insert_with(|| {
            let mut m = HashMap::new();
            for u in crate::sim::run_time_packages(db) {
                for s in db.package_regions(u).unwrap_or_default() {
                    for d in db.decls_with_instances(s) {
                        let rs = db.declared_ranges(d).into_iter();
                        for (k, r) in rs.filter(|r| db.dynamic_range(r).is_some()).enumerate() {
                            if let RangeC::Explicit { left, dir, .. } = *r {
                                m.insert(left, (d, 2 * k as i64, dir));
                            }
                        }
                    }
                }
            }
            m
        });
        let Some(&(d, k, dir)) = owners.get(&left) else {
            return Ok(None);
        };
        let mut bound = |k: i64| {
            let v = self.session.package_range(db, d, k).map_err(|f| match f {
                crate::pkgcall::Fail::Runtime(m) => err(
                    codes::EVAL_FAILED,
                    span,
                    format!("evaluation fails during elaboration: {m}"),
                    "while reading the bounds of this package subtype",
                ),
                crate::pkgcall::Fail::Lower(m) => err(
                    codes::NOT_EVALUATED,
                    span,
                    format!("the bounds cannot be read during elaboration yet: {m}"),
                    "read during elaboration",
                ),
                crate::pkgcall::Fail::Unsupported(w) => err(
                    codes::NOT_EVALUATED,
                    span,
                    format!("the bounds cannot be read during elaboration yet: {w}"),
                    "read during elaboration",
                ),
            })?;
            int(&v, span)
        };
        let b = Bounds {
            left: bound(k)?,
            dir,
            right: bound(k + 1)?,
        };
        self.pkg_ranges.insert(left, b);
        Ok(Some(b))
    }

    pub(crate) fn scalar_bounds(
        &mut self,
        db: &Db,
        st: SubtypeId,
        scope: Option<ScopeId>,
        span: Span,
    ) -> R<Bounds> {
        match db.scalar_range(st) {
            Some(r) => self.range(db, &r, scope, span),
            None => match &db.types[db.base(st)].kind {
                TypeKind::Enum { lits } => Ok(Bounds {
                    left: 0,
                    dir: Direction::To,
                    right: lits.len() as i64 - 1,
                }),
                _ => Err(err(
                    codes::NOT_EVALUATED,
                    span,
                    "the range of this subtype is not known",
                    "here",
                )),
            },
        }
    }

    /// LRM 5.3.2.2: a non-null index range lies inside its index subtype `ix` (#637).
    fn check_index_range(
        &mut self,
        db: &Db,
        b: Bounds,
        ix: SubtypeId,
        scope: Option<ScopeId>,
        span: Span,
    ) -> R<()> {
        if b.is_empty() {
            return Ok(());
        }
        let ib = self.scalar_bounds(db, ix, scope, span)?;
        if ib.pos(b.left).is_some() && ib.pos(b.right).is_some() {
            return Ok(());
        }
        let dir = |d| match d {
            Direction::To => "to",
            Direction::Downto => "downto",
        };
        Err(err(
            codes::EVAL_FAILED,
            span,
            format!(
                "index range {} {} {} is outside index subtype `{}` ({} {} {})",
                b.left,
                dir(b.dir),
                b.right,
                db.subtype_name(ix),
                ib.left,
                dir(ib.dir),
                ib.right
            ),
            "index constraint out of range",
        ))
    }

    /// The fully constrained shape of subtype `st`; `None` when it is not fully constrained.
    pub fn shape(
        &mut self,
        db: &Db,
        st: SubtypeId,
        scope: Option<ScopeId>,
        span: Span,
    ) -> R<Option<Shape>> {
        self.shape_init(db, st, None, scope, span)
    }

    /// The shape of an object of subtype `st` with initial value `init`: bounds the subtype
    /// leaves open, element bounds included, come from the value (2019 signals, LRM 6.4.2.3).
    pub fn shape_init(
        &mut self,
        db: &Db,
        st: SubtypeId,
        init: Option<&Val>,
        scope: Option<ScopeId>,
        span: Span,
    ) -> R<Option<Shape>> {
        let c = db.subtypes[st].constraint.clone();
        self.shape_c(db, st, &c, init, scope, span)
    }

    fn shape_c(
        &mut self,
        db: &Db,
        st: SubtypeId,
        c: &Constraint,
        init: Option<&Val>,
        scope: Option<ScopeId>,
        span: Span,
    ) -> R<Option<Shape>> {
        // `x'subtype`, `x'element` of an object whose bounds are open in its declared subtype:
        // the object's own shape (a signal's, else its value's), `elem_depth` levels down.
        if let Constraint::FromObject { expr, elem_depth } = *c {
            let mut sh = match self.sig_shape(db, expr, scope)? {
                Some(sh) => Some(sh),
                None => {
                    let v = self.eval(db, expr, scope)?;
                    self.shape_init(db, db.exprs[expr].subtype, Some(&v), scope, span)?
                }
            };
            for _ in 0..elem_depth {
                sh = match sh {
                    Some(Shape::Array { elem, .. }) => Some(*elem),
                    _ => None,
                };
            }
            return Ok(sh);
        }
        let ty = db.base(st);
        match &db.types[ty].kind {
            k if k.is_scalar() => {
                let mut range = None;
                let left = match k {
                    TypeKind::Floating | TypeKind::UniversalReal => match db.scalar_range(st) {
                        Some(RangeC::Explicit { left, .. }) => self.eval(db, left, scope)?,
                        _ => Val::Real(-f64::MAX),
                    },
                    _ => {
                        let b = self.scalar_bounds(db, st, scope, span)?;
                        range = Some(b);
                        match k {
                            TypeKind::Enum { .. } => Val::Enum(b.left as u32),
                            TypeKind::Physical { .. } => Val::Phys(b.left),
                            _ => Val::Int(b.left),
                        }
                    }
                };
                let res = match &db.subtypes[st].resolution {
                    Some(Resolution::Function(f)) => Some(*f),
                    _ => None,
                };
                Ok(Some(Shape::Scalar {
                    ty,
                    sub: st,
                    range,
                    elem: elem_of(db, ty),
                    left,
                    res,
                }))
            }
            TypeKind::Array { elem, indexes } => {
                let (elem, (dims, ec)) = (
                    *elem,
                    match c {
                        Constraint::Array { dims, elem } => (dims.clone(), (**elem).clone()),
                        _ => (None, Constraint::None),
                    },
                );
                let arr = match init {
                    Some(Val::Array(a)) => Some(a),
                    _ => None,
                };
                let ds = match (dims, arr) {
                    (Some(dims), _) => {
                        let mut ds = Vec::with_capacity(dims.len());
                        for (d, &ix) in dims.iter().zip(indexes.iter()) {
                            let b = self.range(db, d, scope, span)?;
                            self.check_index_range(db, b, ix, scope, span)?;
                            ds.push(b);
                        }
                        ds
                    }
                    (None, Some(a)) => a
                        .bounds
                        .iter()
                        .map(|&(left, dir, right)| Bounds { left, dir, right })
                        .collect(),
                    (None, None) => return Ok(None),
                };
                // Every element has the element subtype's bounds: the first one gives them.
                let ev = arr.and_then(|a| match &a.elems {
                    Elems::Runs(r) => r.first().map(|(_, v)| v),
                    Elems::Bytes(_) => None,
                });
                let es = if matches!(ec, Constraint::None) {
                    self.shape_init(db, elem, ev, scope, span)?
                } else {
                    self.shape_c(db, elem, &ec, ev, scope, span)?
                };
                let Some(mut es) = es else { return Ok(None) };
                if let Some(Resolution::Element(r)) = &db.subtypes[st].resolution
                    && !apply_res(&mut es, r)
                {
                    return Err(err(
                        codes::INTERNAL,
                        span,
                        "internal error: resolution indication does not fit the subtype",
                        "this is a bug in yavs",
                    ));
                }
                Ok(Some(Shape::Array {
                    ty,
                    dims: ds,
                    elem: Box::new(es),
                    res: whole_res(db, st),
                }))
            }
            TypeKind::Record { elems } => {
                let mut fields = Vec::with_capacity(elems.len());
                for (i, f) in elems.iter().enumerate() {
                    let fc = match c {
                        Constraint::Record { elems } => elems
                            .iter()
                            .find(|(k, _)| *k as usize == i)
                            .map(|(_, c)| c.clone()),
                        _ => None,
                    };
                    let fv = match init {
                        Some(Val::Record(vs)) => vs.get(i),
                        _ => None,
                    };
                    let s = match fc {
                        Some(fc) => self.shape_c(db, f.subtype, &fc, fv, scope, span)?,
                        None => self.shape_init(db, f.subtype, fv, scope, span)?,
                    };
                    let Some(s) = s else { return Ok(None) };
                    fields.push((f.name, s));
                }
                let mut sh = Shape::Record {
                    ty,
                    fields,
                    res: whole_res(db, st),
                };
                if let Some(r @ Resolution::Record(_)) = &db.subtypes[st].resolution
                    && !apply_res(&mut sh, r)
                {
                    return Err(err(
                        codes::INTERNAL,
                        span,
                        "internal error: resolution indication does not fit the subtype",
                        "this is a bug in yavs",
                    ));
                }
                Ok(Some(sh))
            }
            _ => Err(err(
                codes::NOT_IMPLEMENTED,
                span,
                "signals of this type are not implemented yet",
                "here",
            )),
        }
    }
}

/// `v` under the bounds of `shape` (LRM 5.3.2.2): the arrays at the top and in the fields of
/// records; `None` when a length differs.
fn rebound(v: &Val, shape: &Shape) -> Option<Val> {
    match (v, shape) {
        (Val::Array(a), Shape::Array { dims, .. }) => {
            let want: Vec<_> = dims.iter().map(|b| (b.left, b.dir, b.right)).collect();
            a.with_bounds(&want).map(Val::Array)
        }
        (Val::Record(vs), Shape::Record { fields, .. }) if vs.len() == fields.len() => vs
            .iter()
            .zip(fields)
            .map(|(v, (_, s))| rebound(v, s))
            .collect::<Option<Box<[Val]>>>()
            .map(Val::Record),
        _ => Some(v.clone()),
    }
}

/// `f` could not be lowered for a call during elaboration.
fn not_callable(db: &Db, f: DeclId, at: Span, why: &str) -> Box<Diagnostic> {
    Box::new(
        Diagnostic::error(
            codes::NOT_EVALUATED,
            format!(
                "`{}` cannot be called during elaboration yet",
                db.decls[f].name
            ),
        )
        .with_primary(at, "called during elaboration")
        .with_secondary(db.decls[f].span, "declared here")
        .with_note(format!("lowering it failed: {why}")),
    )
}

/// Lower `f` (and what it calls) into its own program, for elaboration-time calls.
///
/// With `capture`, the constants and generics of the regions around `f` that it reads are
/// listed in the result, to be given their values in each call.
fn lower_one(db: &Db, f: DeclId, capture: bool) -> Result<Lowered1, crate::pkgcall::Fail> {
    let mut l = yavs_lower::Lowerer::new(db, yavs_lower::LowerOptions::default());
    if capture {
        l.capture_free_objects(false);
    }
    let lowered = crate::pkgcall::sig(&mut l, db, f).and_then(|s| {
        let (free, size) = l.free_objects();
        l.finish().map(|lw| (s, lw, free, size))
    });
    let (sig, lw, free, inst_size) = lowered.map_err(|ds| {
        crate::pkgcall::Fail::Lower(ds.into_iter().next().map(|d| d.message).unwrap_or_default())
    })?;
    let rt = yavs_interp::Runtime::new(std::sync::Arc::new(lw.program), Default::default())
        .map_err(|e| crate::pkgcall::Fail::Runtime(format!("internal error: {e}")))?;
    Ok(Lowered1 {
        rt,
        sig,
        free,
        inst_size,
    })
}

/// Apply an element resolution indication (LRM 6.3) to the element shape `s`:
/// `(f) t` resolves each element with `f` (a composite element as a whole), `((f)) t` the
/// elements of nested arrays, `(a f, b g) r` per record element. `false` when it does not fit.
fn apply_res(s: &mut Shape, r: &Resolution) -> bool {
    match (r, s) {
        (Resolution::Function(f), s) => {
            set_res(s, *f);
            true
        }
        (Resolution::Element(inner), Shape::Array { elem, .. }) => apply_res(elem, inner),
        (Resolution::Record(fs), Shape::Record { fields, .. }) => fs.iter().all(|(i, r)| {
            fields
                .get_mut(*i as usize)
                .is_some_and(|(_, f)| apply_res(f, r))
        }),
        _ => false,
    }
}

/// The resolution function of composite subtype `st` resolved as a whole (LRM 4.6).
fn whole_res(db: &Db, st: SubtypeId) -> Option<DeclId> {
    match db.subtypes[st].resolution {
        Some(Resolution::Function(f)) => Some(f),
        _ => None,
    }
}

fn set_res(s: &mut Shape, f: DeclId) {
    // A function resolving a composite element resolves it as a whole (LRM 6.3, 4.6).
    match s {
        Shape::Scalar { res, .. } | Shape::Array { res, .. } | Shape::Record { res, .. } => {
            *res = Some(f)
        }
    }
}

pub(crate) fn int(v: &Val, span: Span) -> R<i64> {
    v.as_int().ok_or_else(|| {
        err(
            codes::INTERNAL,
            span,
            "internal error: expected a discrete value",
            "this is a bug in yavs",
        )
    })
}

/// The kernel element of a scalar type: bit-plane logic for the `LogicKind` enums
/// (std_ulogic, bit, boolean; ir.md 1.3), else bytes as laid out in an array
/// (`Ty::elem_size`: an integer type within 32 bits is 4 bytes, scalar signals included).
pub fn elem_of(db: &Db, t: TypeId) -> Elem {
    match &db.types[t].kind {
        TypeKind::Enum { .. } if is_logic(db, t) => Elem::Logic,
        TypeKind::Enum { lits } if lits.len() <= 256 => Elem::Bytes(1),
        TypeKind::Enum { .. } => Elem::Bytes(4),
        _ if yavs_lower::narrow_elem(db, t) => Elem::Bytes(4),
        _ => Elem::Bytes(8),
    }
}

fn is_logic(db: &Db, t: TypeId) -> bool {
    let wk = &db.well_known;
    if Some(t) == wk.bit || Some(t) == wk.boolean {
        return true;
    }
    is_std_ulogic(db, t)
}

pub(crate) fn is_std_ulogic(db: &Db, t: TypeId) -> bool {
    let td = &db.types[t];
    // The name first: the declaration is in a page of the cached analysis that a run reads
    // only for this (#426).
    if td.name.key.as_str() != "std_ulogic" {
        return false;
    }
    let owner = db.scopes[db.decls[td.decl].scope].owner;
    owner.is_some_and(|o| db.decls[o].name.key().as_str() == "std_logic_1164")
}

// ---------------------------------------------------------------------------
// Values of leaves.

/// The scalar values of `v` (of shape `shape`) in row-major order, `None` meaning
/// the scalar's default (`'left`).
fn flatten(
    shape: &Shape,
    v: Option<&Val>,
    out: &mut Vec<(Val, Vec<u32>)>,
    path: &mut Vec<u32>,
) -> Result<(), String> {
    match shape {
        Shape::Scalar { left, .. } => {
            out.push((v.cloned().unwrap_or_else(|| left.clone()), path.clone()));
            Ok(())
        }
        Shape::Array { elem, .. } => {
            let n = shape.len();
            match v {
                None => {
                    for _ in 0..n {
                        flatten(elem, None, out, path)?;
                    }
                }
                Some(Val::Array(a)) => {
                    let have: u64 = a
                        .bounds
                        .iter()
                        .map(|&(l, d, r)| {
                            Bounds {
                                left: l,
                                dir: d,
                                right: r,
                            }
                            .len()
                        })
                        .product();
                    if have != n {
                        return Err(format!("value has {have} elements, the subtype {n}"));
                    }
                    match &a.elems {
                        Elems::Bytes(b) => {
                            for &x in b.iter() {
                                flatten(elem, Some(&Val::Enum(u32::from(x))), out, path)?;
                            }
                        }
                        Elems::Runs(rs) => {
                            for (k, x) in rs.iter() {
                                for _ in 0..*k {
                                    flatten(elem, Some(x), out, path)?;
                                }
                            }
                        }
                    }
                }
                Some(_) => return Err("expected an array value".into()),
            }
            Ok(())
        }
        Shape::Record { fields, .. } => {
            let vs = match v {
                None => None,
                Some(Val::Record(vs)) if vs.len() == fields.len() => Some(vs),
                Some(_) => return Err("expected a record value".into()),
            };
            for (i, (_, f)) in fields.iter().enumerate() {
                path.push(i as u32);
                flatten(f, vs.map(|vs| &vs[i]), out, path)?;
                path.pop();
            }
            Ok(())
        }
    }
}

/// Initial kernel values of each leaf of `shape`, in leaf order.
pub fn leaf_values(db: &Db, shape: &Shape, v: Option<&Val>) -> Result<Vec<Value>, String> {
    let mut flat = Vec::with_capacity(shape.len() as usize);
    flatten(shape, v, &mut flat, &mut Vec::new())?;
    let mut out = Vec::with_capacity(1);
    for (path, leaf, n) in shape.leaves() {
        let Shape::Scalar {
            elem,
            ty,
            sub,
            range,
            ..
        } = leaf
        else {
            continue;
        };
        let xs: Vec<&Val> = flat
            .iter()
            .filter(|(_, p)| *p == path)
            .map(|(v, _)| v)
            .collect();
        debug_assert_eq!(xs.len() as u64, n);
        // Every scalar of the value lies in its (element) subtype (LRM 6.4.2.3, 9.3.3.3, #821).
        if v.is_some()
            && let Some(b) = range
            && let Some(x) = xs
                .iter()
                .filter_map(|x| x.as_int())
                .find(|&x| b.pos(x).is_none())
        {
            let (lo, hi) = b.low_high();
            return Err(format!(
                "value {x} is outside subtype `{}` ({lo} to {hi})",
                db.subtype_name(*sub)
            ));
        }
        out.push(encode(*elem, !is_std_ulogic(db, *ty), &xs));
    }
    Ok(out)
}

/// Kernel bytes or planes of scalar values in row-major order. `bit01`: a BIT or BOOLEAN
/// leaf, whose positions 0/1 are stored as the std_ulogic codes of '0'/'1' (2/3,
/// docs/decisions.md).
pub fn encode(elem: Elem, bit01: bool, xs: &[&Val]) -> Value {
    match elem {
        Elem::Logic => {
            let n = xs.len();
            let off = if bit01 { 2 } else { 0 };
            Value::Logic(LogicVec::from_fn(n, |i| {
                let code = xs[n - 1 - i].as_int().unwrap_or(0) as u8 + off;
                yavs_logic::StdULogic::from_code_pub(code)
            }))
        }
        Elem::Bytes(w) => {
            let mut b = Vec::with_capacity(xs.len() * w as usize);
            for x in xs {
                match (x, w) {
                    (Val::Real(r), _) => b.extend_from_slice(&r.to_le_bytes()),
                    (x, 1) => b.push(x.as_int().unwrap_or(0) as u8),
                    (x, 4) => b.extend_from_slice(&(x.as_int().unwrap_or(0) as u32).to_le_bytes()),
                    (x, _) => b.extend_from_slice(&x.as_int().unwrap_or(0).to_le_bytes()),
                }
            }
            Value::Bytes(b)
        }
    }
}

// ---------------------------------------------------------------------------
// `-g NAME=VALUE` (plan rule 4, sim2 #1117): the text is a value of the generic's type,
// parsed with the `'value` parser of yavs-std.

pub fn parse_override(
    m: &mut Model,
    db: &Db,
    g: DeclId,
    text: &str,
    scope: ScopeId,
) -> Result<Val, String> {
    use yavs_std::types as st;
    use yavs_std::value as v;
    let DeclKind::Object(o) = &db.decls[g].kind else {
        return Err("only generic constants can be overridden".into());
    };
    let sub = o.subtype;
    let ty = db.base(sub);
    let bytes = st::to_latin1(text);
    let span = db.decls[g].span;
    let bounds = |m: &mut Model| {
        m.scalar_bounds(db, sub, Some(scope), span)
            .map_err(|d| d.message.clone())
    };
    match &db.types[ty].kind {
        TypeKind::Integer | TypeKind::UniversalInteger => {
            let (lo, hi) = bounds(m)?.low_high();
            v::integer_value(&bytes, st::Bounds { low: lo, high: hi })
                .map(Val::Int)
                .map_err(|e| e.to_string())
        }
        TypeKind::Floating | TypeKind::UniversalReal => v::real_value(&bytes, st::RealBounds::REAL)
            .map(Val::Real)
            .map_err(|e| e.to_string()),
        TypeKind::Physical { units } => {
            let pt = st::PhysicalType {
                units: units
                    .iter()
                    .map(|u| st::Unit {
                        name: st::to_latin1(&db.decls[u.decl].name.to_string()),
                        value: u.scale,
                    })
                    .collect(),
            };
            let (lo, hi) = bounds(m)?.low_high();
            v::physical_value(&bytes, &pt, st::Bounds { low: lo, high: hi })
                .map(Val::Phys)
                .map_err(|e| e.to_string())
        }
        TypeKind::Enum { lits } => {
            let et = enum_type(db, lits);
            let p = v::enum_value(&bytes, &et).map_err(|e| e.to_string())?;
            let b = bounds(m)?;
            if b.pos(p as i64).is_none() {
                return Err(format!("`{text}` is outside the subtype of the generic"));
            }
            Ok(Val::Enum(p as u32))
        }
        TypeKind::Array { indexes, elem } if indexes.len() == 1 => {
            let et =
                match &db.types[db.base(*elem)].kind {
                    TypeKind::Enum { lits } => enum_type(db, lits),
                    _ => return Err(
                        "only arrays of characters or other enumeration literals can be overridden"
                            .into(),
                    ),
                };
            // A string is the raw text; an array of other enums is one character literal
            // per element (`-gV=0101`).
            let mut pos = Vec::with_capacity(bytes.len());
            for &c in &bytes {
                let p = et
                    .lits
                    .iter()
                    .position(|l| *l == st::EnumLit::Char(c))
                    .ok_or_else(|| {
                        format!("'{}' is not a literal of the element type", c as char)
                    })?;
                pos.push(p as u8);
            }
            let n = pos.len() as i64;
            let b = match m
                .shape(db, sub, Some(scope), span)
                .map_err(|d| d.message.clone())?
            {
                Some(Shape::Array { dims, .. }) => {
                    if dims[0].len() != n as u64 {
                        return Err(format!(
                            "the generic has {} elements, the value {n}",
                            dims[0].len()
                        ));
                    }
                    dims[0]
                }
                _ => {
                    // Unconstrained: the index subtype's 'left, ascending (LRM 9.3.3.3 like a string literal).
                    let ib = m
                        .scalar_bounds(db, indexes[0], Some(scope), span)
                        .map_err(|d| d.message.clone())?;
                    Bounds {
                        left: ib.left,
                        dir: ib.dir,
                        right: match ib.dir {
                            Direction::To => ib.left + n - 1,
                            Direction::Downto => ib.left - n + 1,
                        },
                    }
                }
            };
            Ok(Val::Array(ArrayConst {
                bounds: Box::new([(b.left, b.dir, b.right)]),
                elems: Elems::Bytes(pos.into()),
            }))
        }
        _ => Err(format!(
            "generics of type `{}` cannot be set from the command line",
            db.types[ty].name.text
        )),
    }
}

fn enum_type(db: &Db, lits: &[DeclId]) -> yavs_std::types::EnumType {
    use yavs_sem::decl::Designator;
    yavs_std::types::EnumType {
        lits: lits
            .iter()
            .map(|&l| match db.decls[l].name {
                Designator::Char(c) => yavs_std::types::EnumLit::Char(c as u8),
                d => yavs_std::types::EnumLit::Ident(yavs_std::types::to_latin1(&d.to_string())),
            })
            .collect(),
    }
}

/// The first call in expression `e` (outermost first) whose callee satisfies `p`: the call
/// expression and the callee.
pub(crate) fn find_call(
    db: &Db,
    e: ExprId,
    p: &mut dyn FnMut(DeclId) -> bool,
) -> Option<(ExprId, DeclId)> {
    let mut stack = vec![e];
    while let Some(e) = stack.pop() {
        match &db.exprs[e].kind {
            ExprKind::Call { callee, args, .. } => {
                if p(*callee) {
                    return Some((e, *callee));
                }
                for a in args.iter() {
                    if let yavs_sem::hir::Actual::Expr(x) = a.actual {
                        stack.push(x);
                    }
                }
            }
            ExprKind::Index { prefix, indices } => {
                stack.push(*prefix);
                stack.extend(indices.iter().copied());
            }
            ExprKind::Field { prefix, .. }
            | ExprKind::Deref { prefix }
            | ExprKind::Slice { prefix, .. } => stack.push(*prefix),
            ExprKind::Qualified { operand, .. }
            | ExprKind::Conv { operand, .. }
            | ExprKind::ImplicitConv { operand } => stack.push(*operand),
            ExprKind::Aggregate { assocs, .. } => {
                stack.extend(assocs.iter().map(|a| a.value));
            }
            ExprKind::Conditional { alts } => {
                for (v, c) in alts.iter() {
                    stack.push(*v);
                    stack.extend(*c);
                }
            }
            _ => {}
        }
    }
    None
}

/// Whether expression `e` calls an impure function with a VHDL body (natives such as `now`
/// change no package state).
pub(crate) fn calls_impure(db: &Db, e: ExprId) -> bool {
    find_call(db, e, &mut |c| {
        db.decls[c].kind.subprogram().is_some_and(|s| {
            matches!(
                s.kind,
                yavs_sem::decl::SubprogramKind::Function { pure: false }
            ) && matches!(s.body, yavs_sem::decl::Body::Hir { .. })
        })
    })
    .is_some()
}

/// Whether expression `e` names a signal or port.
pub(crate) fn reads_signal(db: &Db, e: ExprId) -> bool {
    let mut found = false;
    db.walk_expr(e, &mut |x| {
        if let ExprKind::Obj(d) = db.exprs[x].kind {
            found |=
                matches!(&db.decls[d].kind, DeclKind::Object(o) if o.class == ObjClass::Signal);
        }
    });
    found
}

/// The value of `shape` from the initial values of the leaves from `*next` on (the inverse
/// of [`leaf_values`]).
fn init_val(db: &Db, shape: &Shape, nets: &NetGraph, next: &mut usize) -> Option<Val> {
    let scalar = |sh: &Shape, init: &Value, i: usize, n: usize| {
        let Shape::Scalar { ty, left, .. } = sh else {
            return None;
        };
        Some(match init {
            Value::Logic(l) => {
                let off = if is_std_ulogic(db, *ty) { 0 } else { 2 };
                Val::Enum(u32::from(l.get(n - 1 - i) as u8) - off)
            }
            Value::Bytes(b) => {
                let w = b.len() / n;
                let c = &b[i * w..(i + 1) * w];
                match (left, w) {
                    (Val::Real(_), _) => Val::Real(f64::from_le_bytes(c.try_into().ok()?)),
                    (Val::Enum(_), 1) => Val::Enum(u32::from(c[0])),
                    (Val::Enum(_), _) => Val::Enum(u32::from_le_bytes(c.try_into().ok()?)),
                    (v, 4) => {
                        let x = i64::from(i32::from_le_bytes(c.try_into().ok()?));
                        if matches!(v, Val::Phys(_)) {
                            Val::Phys(x)
                        } else {
                            Val::Int(x)
                        }
                    }
                    (v, _) => {
                        let x = i64::from_le_bytes(c.try_into().ok()?);
                        if matches!(v, Val::Phys(_)) {
                            Val::Phys(x)
                        } else {
                            Val::Int(x)
                        }
                    }
                }
            }
        })
    };
    match shape {
        Shape::Scalar { .. } => {
            let init = &nets.leaves.get(*next)?.init;
            *next += 1;
            scalar(shape, init, 0, 1)
        }
        Shape::Array { dims, elem, .. } if matches!(**elem, Shape::Scalar { .. }) => {
            let init = &nets.leaves.get(*next)?.init;
            *next += 1;
            let n = shape.len() as usize;
            let vals: Vec<Val> = (0..n)
                .map(|i| scalar(elem, init, i, n))
                .collect::<Option<_>>()?;
            let bytes =
                dims.len() == 1 && vals.iter().all(|v| matches!(v, Val::Enum(x) if *x < 256));
            let elems = if bytes {
                Elems::Bytes(vals.iter().map(|v| v.as_int().unwrap_or(0) as u8).collect())
            } else {
                Elems::Runs(vals.into_iter().map(|v| (1, v)).collect())
            };
            Some(Val::Array(ArrayConst {
                bounds: dims.iter().map(|b| (b.left, b.dir, b.right)).collect(),
                elems,
            }))
        }
        Shape::Record { fields, .. } => fields
            .iter()
            .map(|(_, f)| init_val(db, f, nets, next))
            .collect::<Option<Box<[_]>>>()
            .map(Val::Record),
        Shape::Array { .. } => None,
    }
}

/// A value of `shape`, each scalar its subtype's `'left`.
fn shape_value(shape: &Shape) -> Val {
    match shape {
        Shape::Scalar { left, .. } => left.clone(),
        Shape::Array { dims, elem, .. } => Val::Array(ArrayConst {
            bounds: dims.iter().map(|b| (b.left, b.dir, b.right)).collect(),
            elems: Elems::Runs(Box::new([(
                dims.iter().map(|b| b.len()).product(),
                shape_value(elem),
            )])),
        }),
        Shape::Record { fields, .. } => {
            Val::Record(fields.iter().map(|(_, f)| shape_value(f)).collect())
        }
    }
}
