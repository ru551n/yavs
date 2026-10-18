//! Staticness classification and the static folder (sem.md section 4).

use crate::types::Direction;

/// 9.4. Ordered: `None < Globally < Locally`, so `min` combines operands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Staticness {
    None,
    Globally,
    Locally,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConstValue {
    Int(i64),
    Real(f64),
    /// Enumeration position.
    Enum(u32),
    /// In base units.
    Phys(i64),
    Array(ArrayConst),
    Record(Box<[ConstValue]>),
    Null,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArrayConst {
    /// One per dimension.
    pub bounds: Box<[(i64, Direction, i64)]>,
    pub elems: Elems,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Elems {
    /// 1-D arrays of enums with at most 256 literals: strings, bit and std_ulogic vectors.
    Bytes(Box<[u8]>),
    /// Run-length (rule 12): `(others => (others => '0'))` of 64K entries is one run.
    Runs(Box<[(u64, ConstValue)]>),
}

impl ConstValue {
    pub fn as_int(&self) -> Option<i64> {
        match *self {
            ConstValue::Int(v) | ConstValue::Phys(v) => Some(v),
            ConstValue::Enum(p) => Some(i64::from(p)),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Folding (4.2).

use crate::attr::PredefAttr;
use crate::db::Db;
use crate::decl::Body;
use crate::hir::{Actual, AttrPrefix, ExprKind};
use crate::ids::ExprId;
use crate::implicit::Family;
use crate::types::TypeKind;

/// Why a value could not be folded.
enum Fold {
    /// Not foldable by sem (user calls, non-static operands, division by zero, ...).
    No,
    /// Checked i64 arithmetic overflowed (rule 7).
    Overflow,
}

fn bool_val(b: bool) -> ConstValue {
    ConstValue::Enum(u32::from(b))
}

impl Db {
    /// Fold expression `id` in place when sem can (4.2); overflow is recorded for
    /// the complete context (`E1401` or `W1401`, Q5). The tree stays either way.
    pub(crate) fn fold_into(&mut self, id: ExprId) {
        match self.fold(id) {
            Ok(v) => {
                let c = self.add_const(v);
                self.exprs[id].value = Some(c);
            }
            Err(Fold::Overflow) => self.overflow.push(id),
            Err(Fold::No) => {}
        }
    }

    fn arg_value(&self, a: &Actual, default: Option<ExprId>) -> Result<ConstValue, Fold> {
        let e = match a {
            Actual::Expr(e) => Some(*e),
            Actual::Default => default,
            _ => None,
        };
        e.and_then(|e| self.value(e).cloned()).ok_or(Fold::No)
    }

    fn fold(&self, id: ExprId) -> Result<ConstValue, Fold> {
        let e = &self.exprs[id];
        let result_kind = &self.types[self.base(e.subtype)].kind;
        match &e.kind {
            ExprKind::Call { callee, args, .. } => {
                let Some(s) = self.decls[*callee].kind.subprogram() else {
                    return Err(Fold::No);
                };
                let Body::Implicit(op) = s.body else {
                    return Err(Fold::No);
                };
                let mut vals = Vec::with_capacity(args.len());
                for a in args.iter() {
                    vals.push(self.arg_value(
                        &a.actual,
                        s.params.get(a.formal as usize).and_then(|p| p.default),
                    )?);
                }
                fold_family(op.family, &vals, result_kind)
            }
            ExprKind::Attr {
                attr,
                prefix: AttrPrefix::Subtype(st),
                args,
            } => {
                let vals: Vec<ConstValue> = args
                    .iter()
                    .map(|a| self.value(*a).cloned().ok_or(Fold::No))
                    .collect::<Result<_, _>>()?;
                match (attr, vals.as_slice()) {
                    (
                        PredefAttr::Left | PredefAttr::Right | PredefAttr::High | PredefAttr::Low,
                        [],
                    ) => self.bound(*st, *attr).ok_or(Fold::No),
                    (PredefAttr::Ascending, []) => {
                        let (_, dir, _) = self.static_bounds(*st).ok_or(Fold::No)?;
                        Ok(bool_val(dir == crate::types::Direction::To))
                    }
                    (PredefAttr::Pos, [v]) => Ok(ConstValue::Int(v.as_int().ok_or(Fold::No)?)),
                    (PredefAttr::Val, [v]) => {
                        let n = v.as_int().ok_or(Fold::No)?;
                        match result_kind {
                            TypeKind::Enum { lits } => {
                                let p = u32::try_from(n)
                                    .ok()
                                    .filter(|&p| (p as usize) < lits.len())
                                    .ok_or(Fold::No)?;
                                Ok(ConstValue::Enum(p))
                            }
                            TypeKind::Integer => Ok(ConstValue::Int(n)),
                            TypeKind::Physical { .. } => Ok(ConstValue::Phys(n)),
                            _ => Err(Fold::No),
                        }
                    }
                    (PredefAttr::Succ | PredefAttr::Pred, [v]) => {
                        let d = if *attr == PredefAttr::Succ { 1 } else { -1 };
                        match v {
                            ConstValue::Enum(p) => {
                                let q = i64::from(*p) + d;
                                match result_kind {
                                    TypeKind::Enum { lits }
                                        if q >= 0 && (q as usize) < lits.len() =>
                                    {
                                        Ok(ConstValue::Enum(q as u32))
                                    }
                                    _ => Err(Fold::No),
                                }
                            }
                            ConstValue::Int(n) => {
                                n.checked_add(d).map(ConstValue::Int).ok_or(Fold::Overflow)
                            }
                            ConstValue::Phys(n) => {
                                n.checked_add(d).map(ConstValue::Phys).ok_or(Fold::Overflow)
                            }
                            _ => Err(Fold::No),
                        }
                    }
                    _ => Err(Fold::No),
                }
            }
            ExprKind::Conv { operand, .. }
            | ExprKind::ImplicitConv { operand }
            | ExprKind::Qualified { operand, .. } => {
                let v = self.value(*operand).cloned().ok_or(Fold::No)?;
                match (v, result_kind) {
                    (ConstValue::Int(n), TypeKind::Floating | TypeKind::UniversalReal) => {
                        Ok(ConstValue::Real(n as f64))
                    }
                    (ConstValue::Real(r), TypeKind::Integer | TypeKind::UniversalInteger) => {
                        // 9.3.6: round to nearest, half away from zero.
                        let r = r.round();
                        if fits_i64(r) {
                            Ok(ConstValue::Int(r as i64))
                        } else {
                            Err(Fold::Overflow)
                        }
                    }
                    (v, _) => Ok(v),
                }
            }
            _ => Err(Fold::No),
        }
    }
}

/// Scalar implicit operations (4.2): checked i64 integers and physicals, f64 reals.
fn fold_family(f: Family, v: &[ConstValue], result: &TypeKind) -> Result<ConstValue, Fold> {
    use ConstValue::*;
    use Family as F;
    let phys = matches!(result, TypeKind::Physical { .. });
    let wrap = |n: i64| if phys { Phys(n) } else { Int(n) };
    let ovf = |o: Option<i64>| o.ok_or(Fold::Overflow);
    match (f, v) {
        (F::Pos, [a]) => Ok(a.clone()),
        (F::Neg, [Int(a)]) => Ok(Int(ovf(a.checked_neg())?)),
        (F::Neg, [Phys(a)]) => Ok(Phys(ovf(a.checked_neg())?)),
        (F::Neg, [Real(a)]) => Ok(Real(-a)),
        (F::Abs, [Int(a)]) => Ok(Int(ovf(a.checked_abs())?)),
        (F::Abs, [Phys(a)]) => Ok(Phys(ovf(a.checked_abs())?)),
        (F::Abs, [Real(a)]) => Ok(Real(a.abs())),
        (F::Not, [Enum(a)]) => Ok(Enum(1 - (*a).min(1))),
        (F::Add, [a, b]) => match (a, b) {
            (Real(x), Real(y)) => Ok(Real(x + y)),
            (x, y) => Ok(wrap(ovf(x
                .as_int()
                .zip(y.as_int())
                .ok_or(Fold::No)
                .map(|(x, y)| x.checked_add(y))?)?)),
        },
        (F::Sub, [a, b]) => match (a, b) {
            (Real(x), Real(y)) => Ok(Real(x - y)),
            (x, y) => Ok(wrap(ovf(x
                .as_int()
                .zip(y.as_int())
                .ok_or(Fold::No)
                .map(|(x, y)| x.checked_sub(y))?)?)),
        },
        (F::Mul, [a, b]) => match (a, b) {
            (Real(x), Real(y)) => Ok(Real(x * y)),
            (Phys(p), Real(r)) | (Real(r), Phys(p)) => round_phys(*p as f64 * r),
            (Real(x), Int(y)) | (Int(y), Real(x)) => Ok(Real(x * *y as f64)),
            (x, y) => Ok(wrap(ovf(x
                .as_int()
                .zip(y.as_int())
                .ok_or(Fold::No)
                .map(|(x, y)| x.checked_mul(y))?)?)),
        },
        (F::Div, [a, b]) => match (a, b) {
            (Real(x), Real(y)) => Ok(Real(x / y)),
            (Real(x), Int(y)) => Ok(Real(x / *y as f64)),
            (Phys(p), Real(r)) => round_phys(*p as f64 / r),
            (Phys(x), Phys(y)) => ovf(x.checked_div(*y)).map(Int).map_err(|_| Fold::No),
            (x, y) => {
                let (x, y) = x.as_int().zip(y.as_int()).ok_or(Fold::No)?;
                if y == 0 {
                    return Err(Fold::No);
                }
                Ok(wrap(ovf(x.checked_div(y))?))
            }
        },
        (F::Rem | F::Mod, [a, b]) => {
            let (x, y) = a.as_int().zip(b.as_int()).ok_or(Fold::No)?;
            if y == 0 {
                return Err(Fold::No);
            }
            let r = ovf(x.checked_rem(y))?;
            let r = if f == F::Mod && r != 0 && ((r < 0) != (y < 0)) {
                r + y
            } else {
                r
            };
            Ok(wrap(r))
        }
        (F::Exp, [a, Int(n)]) => match a {
            Int(x) => {
                let n = u32::try_from(*n).map_err(|_| Fold::No)?;
                Ok(Int(ovf(x.checked_pow(n))?))
            }
            Real(x) => {
                // Repeated multiplication, the order the runtime uses (4.2, rule 5).
                let mut r = 1.0;
                for _ in 0..n.unsigned_abs() {
                    r *= x;
                }
                Ok(Real(if *n < 0 { 1.0 / r } else { r }))
            }
            _ => Err(Fold::No),
        },
        (F::Eq | F::Neq | F::Lt | F::Le | F::Gt | F::Ge, [a, b]) => {
            let ord = match (a, b) {
                (Real(x), Real(y)) => x.partial_cmp(y).ok_or(Fold::No)?,
                (x, y) => {
                    let (x, y) = x.as_int().zip(y.as_int()).ok_or(Fold::No)?;
                    x.cmp(&y)
                }
            };
            use std::cmp::Ordering::*;
            Ok(bool_val(match f {
                F::Eq => ord == Equal,
                F::Neq => ord != Equal,
                F::Lt => ord == Less,
                F::Le => ord != Greater,
                F::Gt => ord == Greater,
                _ => ord != Less,
            }))
        }
        (F::Minimum | F::Maximum, [a, b]) => {
            let less = match (a, b) {
                (Real(x), Real(y)) => x < y,
                (x, y) => x.as_int() < y.as_int(),
            };
            Ok(if less == (f == F::Minimum) {
                a.clone()
            } else {
                b.clone()
            })
        }
        (F::And | F::Or | F::Nand | F::Nor | F::Xor | F::Xnor, [Enum(x), Enum(y)]) => {
            let (x, y) = (*x != 0, *y != 0);
            Ok(bool_val(match f {
                F::And => x && y,
                F::Or => x || y,
                F::Nand => !(x && y),
                F::Nor => !(x || y),
                F::Xor => x != y,
                _ => x == y,
            }))
        }
        _ => Err(Fold::No),
    }
}

fn round_phys(v: f64) -> Result<ConstValue, Fold> {
    let r = v.round();
    if fits_i64(r) {
        Ok(ConstValue::Phys(r as i64))
    } else {
        Err(Fold::Overflow)
    }
}

/// Whether a rounded f64 is representable as i64 (`2**63` itself is not).
fn fits_i64(r: f64) -> bool {
    const LIMIT: f64 = 9_223_372_036_854_775_808.0;
    (-LIMIT..LIMIT).contains(&r)
}
