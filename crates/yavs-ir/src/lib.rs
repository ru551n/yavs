//! The yavs register IR (docs/design/ir.md section 1).
//!
//! A [`Program`] holds every function (processes, subprograms, elaboration bodies, thunks),
//! the runtime type descriptions, the compact constant pool and the global statement table.
//! It is built once with [`ProgramBuilder`], checked by the linear-time verifier, and is
//! immutable and `Send + Sync` afterwards. All ids are 32-bit (rule 5).
//!
//! Every instruction carries a [`StmtId`] (`Func::loc`), which is the kernel's
//! `yavs_kernel::StmtId`, so a runtime error's trace resolves to spans without another table.

mod build;
mod dump;
pub mod ieee;
mod inst;
pub mod opt;
mod program;
mod ty;
mod verify;

pub use build::{FuncBuilder, ProgramBuilder};
pub use inst::{
    AddrSpace, AttrKind, BoolOp, CheckKind, CmpOp, DriveMode, ElabOp, FOp, FUnOp, ForceKind, Inst,
    IntOp, IntUnOp, LNumOp, LOp, PhysOp, Reg, RegTy, Regs, Role, SIG_PART_BYTES, Width,
};
pub use program::{
    Block, CaseTable, CheckMsg, ConstData, CovInfo, CovKind, Dir, ForeignArg, ForeignMode,
    ForeignRef, Func, FuncFlags, FuncKind, Local, Mode, NativeRef, Param, ParamClass, Program,
    RootKind, StmtInfo, StmtKind, VhpiIndex, VhpiModel, VhpiParam,
};
pub use ty::{DOPE_DIM_BYTES, Field, Layout, LayoutError, LogicKind, Refl, ReflKind, Ty, dim_len};
pub use verify::VerifyError;
pub use yavs_kernel::StmtId;

use std::fmt;
use std::marker::PhantomData;

/// A 32-bit id newtype.
pub trait Id: Copy {
    fn from_index(i: u32) -> Self;
    fn index(self) -> usize;
}

macro_rules! ids {
    ($($name:ident $prefix:literal),* $(,)?) => {$(
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u32);
        impl Id for $name {
            fn from_index(i: u32) -> Self {
                $name(i)
            }
            fn index(self) -> usize {
                self.0 as usize
            }
        }
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "{}"), self.0)
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Debug::fmt(self, f)
            }
        }
    )*};
}

ids! {
    FuncId "f", BlockId "b", TyId "t", ConstId "c", NativeId "n", ForeignId "x",
    CheckMsgId "m", CaseTableId "case", CovItem "cov", ReflId "r",
}

impl Id for StmtId {
    fn from_index(i: u32) -> Self {
        StmtId(i)
    }
    fn index(self) -> usize {
        self.0 as usize
    }
}

/// A vector indexed by a 32-bit id. Access is checked (`get`); there is no panicking index.
#[derive(Clone, PartialEq)]
pub struct IdVec<I, T> {
    items: Vec<T>,
    _id: PhantomData<fn(I) -> I>,
}

impl<I, T> Default for IdVec<I, T> {
    fn default() -> Self {
        IdVec {
            items: Vec::new(),
            _id: PhantomData,
        }
    }
}

impl<I: Id, T> IdVec<I, T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append, returning the new id. More than `u32::MAX` items is a broken invariant of the
    /// producer; the id saturates and the verifier reports the overflow.
    pub fn push(&mut self, v: T) -> I {
        let i = u32::try_from(self.items.len()).unwrap_or(u32::MAX);
        self.items.push(v);
        I::from_index(i)
    }

    pub fn get(&self, i: I) -> Option<&T> {
        self.items.get(i.index())
    }

    pub fn get_mut(&mut self, i: I) -> Option<&mut T> {
        self.items.get_mut(i.index())
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (I, &T)> {
        self.items
            .iter()
            .enumerate()
            .map(|(i, v)| (I::from_index(i as u32), v))
    }

    pub fn values(&self) -> &[T] {
        &self.items
    }
}

impl<I, T: fmt::Debug> fmt::Debug for IdVec<I, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.items.fmt(f)
    }
}
