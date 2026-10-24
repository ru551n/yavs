//! Record signals as one instruction (#677): the leaf table of `Inst::SigGather` and
//! `Inst::SigScatter` (`yavs_ir::gather`) of a record signal, or of a part of one that is a record
//! or an array of records: a whole signal, a field, an element or a slice of an array of
//! records. The leaves are scalars or arrays of scalars below records and arrays of records.
//! Where the leaves lie in the value is left to the instructions, which take the descriptor of
//! the part.

use yavs_diag::Span;
use yavs_ir::gather::{self, LeafKind, TabLeaf};
use yavs_ir::{ConstId, Ty, TyId};
use yavs_sem::DeclId;

use crate::func::Fx;
use crate::{Obj, R};

impl Fx<'_, '_> {
    /// The leaf table of record signal `d` as a whole; `None` when it has a shape the
    /// instructions do not take (the caller reads and drives leaf by leaf).
    pub(crate) fn rec_table(&mut self, d: DeclId, span: Span) -> R<Option<ConstId>> {
        let Some(st) = crate::ty::obj_subtype(self.db, d) else {
            return Ok(None);
        };
        let tid = self.ty(self.db.base(st), span)?;
        self.table_for(d, &[], tid, span)
    }

    /// Whether the part that `steps` select has a value a part can be: any value when no index or
    /// slice selected it, else a record or an array of records (an array of arrays of records
    /// would need the element counts of every level in `Part`, #677).
    pub(crate) fn part_value_ok(&mut self, steps: &[crate::expr::Step<'_>], tid: TyId) -> bool {
        use crate::expr::Step;
        if !steps
            .iter()
            .any(|s| matches!(s, Step::Index(_) | Step::Slice(_)))
        {
            return true;
        }
        let types = self.fb.program().types();
        match types.get(tid) {
            Some(Ty::Record { .. }) => true,
            Some(Ty::Array { elem, .. }) => matches!(types.get(*elem), Some(Ty::Record { .. })),
            _ => false,
        }
    }

    /// The leaves below field path `path` of `d` form a value of type `tid`: their table.
    pub(crate) fn table_for(
        &mut self,
        d: DeclId,
        path: &[u32],
        tid: TyId,
        span: Span,
    ) -> R<Option<ConstId>> {
        let key = (d, path.to_vec(), tid);
        if let Some(t) = self.cx.rec_tables.get(&key) {
            return Ok(*t);
        }
        let t = self.table_new(d, path, tid, span)?;
        self.cx.rec_tables.insert(key, t);
        Ok(t)
    }

    fn table_new(&mut self, d: DeclId, path: &[u32], tid: TyId, span: Span) -> R<Option<ConstId>> {
        if !matches!(self.cx.objs.get(&d), Some(Obj::RecSignal(None))) {
            return Ok(None);
        }
        let leaves = self.leaves_under(d, path);
        if gather::shape(self.fb.program().types(), tid) != Some(leaves.len()) {
            return Ok(None);
        }
        // The leaves' paths run from the signal; the type's own start at the type of the
        // value, which is that of the signal's field `path`.
        let Some(root) = crate::ty::obj_subtype(self.db, d) else {
            return Ok(None);
        };
        let root = self.ty(self.db.base(root), span)?;
        let mut table = Vec::with_capacity(leaves.len());
        for l in &leaves {
            // The type of the leaf: the fields on its path, through the arrays of records.
            let mut t = root;
            for &i in &l.path {
                let types = self.fb.program().types();
                while let Some(Ty::Array { elem, .. }) = types.get(t) {
                    t = *elem;
                }
                let Some(Ty::Record { fields }) = types.get(t) else {
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
