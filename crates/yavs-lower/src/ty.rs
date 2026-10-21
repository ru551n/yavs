//! HIR types to IR type descriptions (ir.md 1.3).

use yavs_diag::Span;
use yavs_ir::{LogicKind, Program, ProgramBuilder, Ty, TyId, Width};
use yavs_kernel::Elem;
use yavs_sem::decl::Designator;
use yavs_sem::stat::{ConstValue, Elems};
use yavs_sem::types::TypeKind;
use yavs_sem::{Db, SubtypeId, TypeId};

use crate::{Cx, R};

/// The IR type of `t`, lowered once.
pub(crate) fn lower(
    db: &Db,
    cx: &mut Cx,
    pb: &mut ProgramBuilder,
    t: TypeId,
    span: Span,
) -> R<TyId> {
    if let Some(&id) = cx.tys.get(&t) {
        return Ok(id);
    }
    let td = &db.types[t];
    let ty = match &td.kind {
        TypeKind::Enum { lits } => Ty::Enum {
            lits: lits.iter().map(|&d| lit_name(db, d)).collect(),
            logic: logic_kind(db, t),
        },
        TypeKind::Integer | TypeKind::UniversalInteger => {
            let (lo, hi) = int_bounds(db, td.base_subtype).unwrap_or((i64::MIN, i64::MAX));
            Ty::Int { lo, hi }
        }
        TypeKind::Physical { units } => {
            let (lo, hi) = int_bounds(db, td.base_subtype).unwrap_or((i64::MIN, i64::MAX));
            let units = units
                .iter()
                .map(|u| (db.decls[u.decl].name.to_string(), u.scale))
                .collect();
            Ty::Physical { lo, hi, units }
        }
        TypeKind::Floating | TypeKind::UniversalReal => Ty::Real,
        TypeKind::Array { indexes, elem } => {
            let elem_t = lower(db, cx, pb, db.base(*elem), span)?;
            if !matches!(pb.types().get(elem_t), Some(t) if t.scalar_size().is_some()) {
                return crate::composite::array_ty(db, cx, pb, t, *elem, elem_t, span);
            }
            Ty::Array {
                elem: elem_t,
                elem_desc: None,
                dims: indexes.len() as u8,
            }
        }
        TypeKind::Record { .. }
        | TypeKind::Access { .. }
        | TypeKind::File { .. }
        | TypeKind::Protected { .. }
        | TypeKind::Incomplete { .. } => return crate::composite::lower_ty(db, cx, pb, t, span),
        _ => return Err(cx.unsupported(span, "generic and incomplete types")),
    };
    let id = pb.ty(ty);
    cx.tys.insert(t, id);
    Ok(id)
}

/// `'image` spelling of an enumeration literal: identifiers as written, characters quoted.
fn lit_name(db: &Db, d: yavs_sem::DeclId) -> String {
    match db.decls[d].name {
        Designator::Char(c) => format!("'{c}'"),
        n => n.to_string(),
    }
}

/// BIT, BOOLEAN and STD_ULOGIC (and their arrays) are stored as yavs-logic codes and planes.
fn logic_kind(db: &Db, t: TypeId) -> Option<LogicKind> {
    let wk = &db.well_known;
    if Some(t) == wk.bit {
        return Some(LogicKind::Bit);
    }
    if Some(t) == wk.boolean {
        return Some(LogicKind::Boolean);
    }
    // ponytail: recognised by name and shape; a `std_ulogic` declared outside ieee is treated
    // the same, which is harmless since layout is all that changes.
    let td = &db.types[t];
    match &td.kind {
        TypeKind::Enum { lits } if lits.len() == 9 && td.name.key.as_str() == "std_ulogic" => {
            Some(LogicKind::StdULogic)
        }
        _ => None,
    }
}

fn int_bounds(db: &Db, st: SubtypeId) -> Option<(i64, i64)> {
    let (l, d, r) = db.static_bounds(st)?;
    let (l, r) = (l.as_int()?, r.as_int()?);
    Some(match d {
        yavs_sem::types::Direction::To => (l, r),
        yavs_sem::types::Direction::Downto => (r, l),
    })
}

/// Memory width of a scalar of IR type `ty`.
pub(crate) fn width(ty: &Ty) -> Width {
    match ty.scalar_size() {
        Some(1) => Width::B1,
        Some(4) => Width::B4,
        _ => Width::B8,
    }
}

/// Whether elements of sem type `t` in an array are narrow integers (4 bytes in memory and in
/// the kernel, `Ty::elem_size`): the same rule as the IR type [`lower`] gives `t`.
pub fn narrow_elem(db: &Db, t: TypeId) -> bool {
    matches!(
        db.types[t].kind,
        TypeKind::Integer | TypeKind::UniversalInteger
    ) && int_bounds(db, db.types[t].base_subtype)
        .is_some_and(|(lo, hi)| Ty::Int { lo, hi }.elem_size() == Some(4))
}

/// Memory width of an array element of IR type `ty` ([`Ty::elem_size`]: integers within 32
/// bits are 4 bytes there, sign-extended on load).
pub(crate) fn elem_width(ty: &Ty) -> Width {
    match ty.elem_size() {
        Some(1) => Width::B1,
        Some(4) => Width::B4,
        _ => Width::B8,
    }
}

/// The kernel element representation of a signal of IR type `ty`: logic leaves (BIT, BOOLEAN,
/// STD_ULOGIC, scalar or array) are bit-planes, everything else its bytes as an array element
/// (`Ty::elem_size`: a narrow integer is 4 bytes in the kernel, scalar signals included). The
/// elaborator uses this to create kernel signals the IR's `Drive` and `SigRead` agree with.
pub fn signal_elem(prog: &Program, ty: TyId) -> Option<Elem> {
    if Ty::logic_leaf(&prog.types, ty).is_some() {
        return Some(Elem::Logic);
    }
    let mut t = prog.types.get(ty)?;
    while let Ty::Array { elem, .. } = t {
        t = prog.types.get(*elem)?;
    }
    Some(Elem::Bytes(t.elem_size()? as u32))
}

/// A folded scalar as one register cell.
pub(crate) fn cell(v: &ConstValue) -> Option<u64> {
    match v {
        ConstValue::Int(i) | ConstValue::Phys(i) => Some(*i as u64),
        ConstValue::Enum(p) => Some(u64::from(*p)),
        ConstValue::Real(f) => Some(f.to_bits()),
        _ => None,
    }
}

pub(crate) fn is_real(db: &Db, t: TypeId) -> bool {
    matches!(
        db.types[t].kind,
        TypeKind::Floating | TypeKind::UniversalReal
    )
}

pub(crate) fn is_array(db: &Db, t: TypeId) -> bool {
    matches!(db.types[t].kind, TypeKind::Array { .. })
}

/// Whether subtype `st` or a subtype it is built on has an index constraint.
pub(crate) fn index_constrained(db: &Db, st: SubtypeId) -> bool {
    std::iter::successors(Some(st), |&s| db.subtypes[s].parent).any(|s| {
        matches!(
            db.subtypes[s].constraint,
            yavs_sem::types::Constraint::Array { dims: Some(_), .. }
        )
    })
}

/// Element subtype and dimension count of an array type.
pub(crate) fn array_of(db: &Db, t: TypeId) -> Option<(SubtypeId, u8)> {
    match &db.types[t].kind {
        TypeKind::Array { indexes, elem } => Some((*elem, indexes.len() as u8)),
        _ => None,
    }
}

/// Name of a subtype in runtime check messages. Upper case, as sim1 prints it, so the
/// differential corpus compares exactly (difftest is strict for yavs).
pub(crate) fn subtype_name(db: &Db, st: SubtypeId) -> String {
    let s = &db.subtypes[st];
    match s.name {
        Some(n) => n.text.as_str().to_ascii_uppercase(),
        None => db.types[s.base].name.text.as_str().to_ascii_uppercase(),
    }
}

/// An object as check messages name it: `variable P`, `signal S`, `parameter X`.
pub(crate) fn obj_desc(db: &Db, d: yavs_sem::DeclId) -> String {
    use yavs_sem::decl::{DeclKind, ObjClass, ObjRole};
    let name = db.decls[d].name.to_string().to_ascii_uppercase();
    let class = match &db.decls[d].kind {
        DeclKind::Object(o) if o.role == ObjRole::Param => "parameter",
        DeclKind::Object(o) => match o.class {
            ObjClass::Constant => "constant",
            ObjClass::Signal => "signal",
            ObjClass::Variable | ObjClass::SharedVariable => "variable",
            ObjClass::File => "file",
        },
        _ => return name,
    };
    format!("{class} {name}")
}

/// Elements (IR layout) and descriptor bytes of a folded array value of IR type `ty`.
pub fn encode_array(prog: &Program, ty: TyId, v: &ConstValue) -> Option<(Vec<u8>, Vec<u8>)> {
    let ConstValue::Array(a) = v else {
        return None;
    };
    let Ty::Array { elem, .. } = prog.types.get(ty)? else {
        return None;
    };
    let mut cells: Vec<u64> = Vec::new();
    match &a.elems {
        Elems::Bytes(b) => cells.extend(b.iter().map(|&x| u64::from(x))),
        Elems::Runs(runs) => {
            for (n, c) in runs.iter() {
                let x = cell(c)?;
                cells.extend(std::iter::repeat_n(x, usize::try_from(*n).ok()?));
            }
        }
    }
    let mut dope = Vec::new();
    for &(l, d, r) in a.bounds.iter() {
        let d = i64::from(d == yavs_sem::types::Direction::Downto);
        for x in [l, r, d] {
            dope.extend_from_slice(&x.to_le_bytes());
        }
    }
    let elems = match Ty::logic_leaf(&prog.types, ty) {
        Some(k) => {
            let bias = if k == LogicKind::StdULogic { 0 } else { 2 };
            let n = cells.len();
            let mut blocks = vec![[0u64; 4]; n.div_ceil(64)];
            for (p, &c) in cells.iter().enumerate() {
                let i = n - 1 - p; // element 0 is the rightmost
                let code = c + bias;
                for (plane, w) in blocks[i / 64].iter_mut().enumerate() {
                    *w |= (code >> plane & 1) << (i % 64);
                }
            }
            blocks
                .iter()
                .flat_map(|b| b.iter().flat_map(|w| w.to_le_bytes()))
                .collect()
        }
        None => {
            let w = prog.types.get(*elem)?.elem_size()? as usize;
            cells
                .iter()
                .flat_map(|c| c.to_le_bytes()[..w].to_vec())
                .collect()
        }
    };
    Some((elems, dope))
}

/// The subtype of an object or object alias (an alias without a subtype has its target's).
pub(crate) fn obj_subtype(db: &Db, d: yavs_sem::DeclId) -> Option<SubtypeId> {
    use yavs_sem::decl::{Alias, DeclKind};
    match &db.decls[d].kind {
        DeclKind::Object(o) => Some(o.subtype),
        DeclKind::Alias(Alias::Object { target, subtype }) => {
            Some(subtype.unwrap_or(db.exprs[*target].subtype))
        }
        _ => None,
    }
}

/// Whether `t` is a record or an array (of arrays) of records: a signal of it has one kernel
/// signal per field.
pub(crate) fn has_record(db: &Db, t: TypeId) -> bool {
    match &db.types[t].kind {
        TypeKind::Record { .. } => true,
        TypeKind::Array { elem, .. } => has_record(db, db.base(*elem)),
        _ => false,
    }
}

/// The leaves of a signal of subtype `st` (docs/decisions.md "Elaboration": one kernel signal
/// per record field, recursively; an array of records one per field, a struct of arrays):
/// (field path, leaf subtype: the field's scalar or array of scalars).
pub(crate) fn leaves(db: &Db, st: SubtypeId) -> Vec<(Vec<u32>, SubtypeId)> {
    let t = db.base(st);
    match &db.types[t].kind {
        TypeKind::Record { elems } => {
            let mut out = Vec::new();
            for (i, e) in elems.iter().enumerate() {
                for (mut p, s) in leaves(db, e.subtype) {
                    p.insert(0, i as u32);
                    out.push((p, s));
                }
            }
            out
        }
        TypeKind::Array { elem, .. } if has_record(db, db.base(*elem)) => leaves(db, *elem),
        _ => vec![(Vec::new(), st)],
    }
}
