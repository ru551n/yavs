//! 2019 mode views and `'converse` (sem.md 6.7). Analysis: slice S7.

use crate::ids::{DeclId, SubtypeId, ViewId};

pub use crate::hir::Mode;

pub struct ViewDef {
    pub decl: DeclId,
    pub record: SubtypeId,
    /// One per record element, in element order.
    pub elems: Box<[ElemMode]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElemMode {
    Simple(Mode),
    View {
        view: ViewId,
        converse: bool,
        array: bool,
    },
}
