//! Clock-edge idioms in the HIR (#632): the one place that says which expressions are an edge
//! test. The elaborator uses it for edge triggers, `yavs-lower` for the IR edge test.
//!
//! The forms: `rising_edge (x)` / `falling_edge (x)` (the IEEE 1164 functions of `std_ulogic`,
//! and the predefined ones of BIT and BOOLEAN), and `x'event and x = '1'` / `'0'` (either order,
//! BOOLEAN also `x'event and x` and `x'event and not x`; `not x'stable` is made `x'event` by
//! the analyser). Only the leaf is recognised here, never an `and` chain around it.

use crate::db::Db;
use crate::hir::{Actual, Arg, AttrPrefix, ExprKind, PredefAttr};
use crate::ids::ExprId;
use crate::stat::ConstValue;

/// A recognised edge test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdgeShape {
    /// Rising (to '1') or falling (to '0').
    pub rising: bool,
    /// `x'event and x = lit`: an event that ends on the level. Not legacy: the IEEE function
    /// (or the predefined one of BIT and BOOLEAN), which also needs the old value to be '0' or
    /// 'L' (the level, for BIT and BOOLEAN).
    pub legacy: bool,
    /// The signal name `x` (of the `'event` or the call).
    pub sig: ExprId,
    /// For the legacy form, the other mention of the signal (the one compared).
    pub other: Option<ExprId>,
}

impl Db {
    /// `x = '1'` / `'1' = x` of the predefined `=`: the signal name and the position of the
    /// enumeration literal in `std_ulogic` numbering (BIT and BOOLEAN are stored with its
    /// codes: '0' and FALSE are 2, '1' and TRUE are 3).
    pub fn level_of(&self, e: ExprId) -> Option<(ExprId, u32)> {
        self.level_cmp(e).filter(|c| c.2).map(|(x, p, _)| (x, p))
    }

    /// [`Db::level_of`] also for `/=`: the signal name, the position, and whether the
    /// comparison is `=`.
    pub fn level_cmp(&self, e: ExprId) -> Option<(ExprId, u32, bool)> {
        let ExprKind::Call {
            callee,
            args,
            method_of: None,
            implicit_cond: false,
        } = &self.exprs[e].kind
        else {
            return None;
        };
        let decl = &self.decls[*callee];
        let [
            Arg {
                actual: Actual::Expr(l),
                ..
            },
            Arg {
                actual: Actual::Expr(r),
                ..
            },
        ] = &args[..]
        else {
            return None;
        };
        let eq = match decl.name.key().as_str() {
            "\"=\"" if decl.implicit => true,
            "\"/=\"" if decl.implicit => false,
            _ => return None,
        };
        let pos = |x: ExprId| match self.exprs[x].kind {
            ExprKind::Const(c) => match self.consts[c] {
                ConstValue::Enum(p) => Some(p),
                _ => None,
            },
            _ => None,
        };
        let bit = |x: ExprId| {
            let t = Some(self.base(self.exprs[x].subtype));
            t == self.well_known.bit || t == self.well_known.boolean
        };
        match (pos(*l), pos(*r)) {
            (None, Some(p)) => Some((*l, if bit(*r) { p + 2 } else { p }, eq)),
            (Some(p), None) => Some((*r, if bit(*l) { p + 2 } else { p }, eq)),
            _ => None,
        }
    }

    /// The edge test `e` is, if it is one of the forms in the module docs.
    pub fn edge_shape(&self, e: ExprId) -> Option<EdgeShape> {
        let ExprKind::Call {
            callee,
            args,
            method_of: None,
            implicit_cond: false,
        } = &self.exprs[e].kind
        else {
            return None;
        };
        let decl = &self.decls[*callee];
        let name = decl.name.key();
        let (Some(Arg {
            actual: Actual::Expr(first),
            ..
        }),) = (args.first(),)
        else {
            return None;
        };
        let call = |rising| {
            Some(EdgeShape {
                rising,
                legacy: false,
                sig: *first,
                other: None,
            })
        };
        match (name.as_str(), args.len()) {
            ("rising_edge" | "falling_edge", 1) => {
                let rising = name.as_str() == "rising_edge";
                if decl.implicit {
                    return call(rising);
                }
                let owner = self.scopes[decl.scope].owner?;
                (self.decls[owner].name.key().as_str() == "std_logic_1164").then(|| call(rising))?
            }
            ("\"and\"", 2) if decl.implicit => {
                let boolean = Some(self.base(self.exprs[e].subtype)) == self.well_known.boolean;
                let Some(Arg {
                    actual: Actual::Expr(second),
                    ..
                }) = args.get(1)
                else {
                    return None;
                };
                if boolean {
                    self.legacy_of(*first, *second)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// `x'event` with `x = '1'` (rising) or `x = '0'`, in either order.
    fn legacy_of(&self, a: ExprId, b: ExprId) -> Option<EdgeShape> {
        let event = |e: ExprId| match &self.exprs[e].kind {
            ExprKind::Attr {
                attr: PredefAttr::Event,
                prefix: AttrPrefix::Expr(x),
                ..
            } => Some(*x),
            _ => None,
        };
        let (x, (l, pos)) = match (event(a), event(b)) {
            (Some(x), None) => (x, self.level_operand(b)?),
            (None, Some(x)) => (x, self.level_operand(a)?),
            _ => return None,
        };
        // std_ulogic positions: '0' is 2, '1' is 3.
        matches!(pos, 2 | 3).then_some(EdgeShape {
            rising: pos == 3,
            legacy: true,
            sig: x,
            other: Some(l),
        })
    }

    /// A comparison with a level, or a BOOLEAN name (`x`: level 3, `not x`: level 2).
    fn level_operand(&self, e: ExprId) -> Option<(ExprId, u32)> {
        if let Some(l) = self.level_of(e) {
            return Some(l);
        }
        let boolean = |x: ExprId| Some(self.base(self.exprs[x].subtype)) == self.well_known.boolean;
        if !boolean(e) {
            return None;
        }
        match &self.exprs[e].kind {
            ExprKind::Obj(_) | ExprKind::Index { .. } | ExprKind::Field { .. } => Some((e, 3)),
            ExprKind::Call {
                callee,
                args,
                method_of: None,
                implicit_cond: false,
            } if self.decls[*callee].implicit
                && self.decls[*callee].name.key().as_str() == "\"not\"" =>
            {
                match &args[..] {
                    [
                        Arg {
                            actual: Actual::Expr(x),
                            ..
                        },
                    ] if matches!(
                        self.exprs[*x].kind,
                        ExprKind::Obj(_) | ExprKind::Index { .. } | ExprKind::Field { .. }
                    ) && boolean(*x) =>
                    {
                        Some((*x, 2))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Whether `a` and `b` are the same signal name: the same object, field selections and
    /// indices that are the same constants or objects (reading them twice gives one value).
    pub fn same_name(&self, a: ExprId, b: ExprId) -> bool {
        if a == b {
            return true;
        }
        let (x, y) = (&self.exprs[a], &self.exprs[b]);
        if let (Some(p), Some(q)) = (x.value, y.value) {
            return self.consts[p] == self.consts[q];
        }
        match (&x.kind, &y.kind) {
            (ExprKind::Obj(p), ExprKind::Obj(q)) => p == q,
            (ExprKind::Const(p), ExprKind::Const(q)) => self.consts[*p] == self.consts[*q],
            (ExprKind::Field { prefix: p, elem: m }, ExprKind::Field { prefix: q, elem: n }) => {
                m == n && self.same_name(*p, *q)
            }
            (
                ExprKind::Index {
                    prefix: p,
                    indices: i,
                },
                ExprKind::Index {
                    prefix: q,
                    indices: j,
                },
            ) => {
                i.len() == j.len()
                    && self.same_name(*p, *q)
                    && i.iter().zip(j.iter()).all(|(&m, &n)| self.same_name(m, n))
            }
            _ => false,
        }
    }
}
