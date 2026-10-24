//! Whole record signals as one instruction (#677): the leaf table of `Inst::SigGather` and
//! `Inst::SigScatter` (`yavs_ir::gather`) of a record signal whose leaves are scalars or arrays
//! of scalars below records. Where the leaves lie in the value is left to the instructions, which
//! take the descriptor of the signal.

use yavs_diag::Span;
use yavs_ir::gather::{self, LeafKind, TabLeaf};
use yavs_ir::{ConstId, Ty};
use yavs_sem::DeclId;

use crate::func::Fx;
use crate::{Obj, R};

impl Fx<'_, '_> {
    /// The leaf table of record signal `d`; `None` when it has a shape the instructions do not
    /// take (the caller reads and drives leaf by leaf).
    pub(crate) fn rec_table(&mut self, d: DeclId, span: Span) -> R<Option<ConstId>> {
        if let Some(t) = self.cx.rec_tables.get(&d) {
            return Ok(*t);
        }
        let t = self.rec_table_new(d, span)?;
        self.cx.rec_tables.insert(d, t);
        Ok(t)
    }

    fn rec_table_new(&mut self, d: DeclId, span: Span) -> R<Option<ConstId>> {
        let db = self.db;
        if !matches!(self.cx.objs.get(&d), Some(Obj::RecSignal(None))) {
            return Ok(None);
        }
        let Some(st) = crate::ty::obj_subtype(db, d) else {
            return Ok(None);
        };
        let tid = self.ty(db.base(st), span)?;
        let leaves = self.cx.rec_leaves.get(&d).cloned().unwrap_or_default();
        if gather::shape(self.fb.program().types(), tid) != Some(leaves.len()) {
            return Ok(None);
        }
        let mut table = Vec::with_capacity(leaves.len());
        for l in &leaves {
            // The type of the leaf: the fields on its path.
            let mut t = tid;
            for &i in &l.path {
                let Some(Ty::Record { fields }) = self.fb.program().types().get(t) else {
                    return Ok(None);
                };
                let Some(f) = fields.get(i as usize) else {
                    return Ok(None);
                };
                t = f.ty;
            }
            let array = matches!(self.fb.program().types().get(t), Some(Ty::Array { .. }));
            let (logic, bias, sw, ew) = self.leaf_facts(l, span)?;
            let kind = match (logic, array) {
                (true, false) => LeafKind::LogicScalar,
                (true, true) => LeafKind::LogicVec,
                (false, false) => LeafKind::Scalar,
                (false, true) => LeafKind::Bytes,
            };
            table.push(TabLeaf {
                sig: l.sig,
                kind,
                bias: if kind == LeafKind::LogicScalar {
                    bias as u8
                } else {
                    0
                },
                sw: sw.bytes() as u8,
                ew: ew.bytes() as u8,
            });
        }
        Ok(Some(self.fb.program().konst(gather::encode(tid, &table))))
    }
}
