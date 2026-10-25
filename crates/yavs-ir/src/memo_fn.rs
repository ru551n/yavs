//! Which function calls the interpreter may answer from a memo (#701).
//!
//! A call of a pure function with plain data arguments gives the same result whenever the
//! arguments hold the same bytes, so the interpreter keeps the results of the calls it ran
//! and answers a repeat from them (`yavs-interp` `pmemo.rs`). This module decides, from the
//! code as lowered and the types, which functions qualify: [`Program::memo_sig`].
//!
//! A function qualifies when
//! - it is declared `pure`, not nested, cannot suspend, touches no shared state, counts no
//!   coverage and has no signal parameter;
//! - every parameter is an `in` scalar of a plain type or an `in` array of plain scalars (the
//!   bytes of its descriptor and elements are the key), and the result is one of those;
//! - its code (and that of every function it calls) reads nothing but its parameters, its own
//!   frame, constants and the package constants: no signal, instance data, enclosing frame,
//!   shared variable, clock, file, foreign or indirect call, and only natives that compute
//!   from their arguments;
//! - it is worth it: a loop, a call of another function or a body of some size. A body of a
//!   few instructions runs faster than the key is built.
//!
//! A `Report` is allowed: the interpreter notes that one executed during a call and stores
//! nothing for it, so a call that reports is run again every time.

use crate::{
    AddrSpace, Func, FuncFlags, FuncId, FuncKind, Inst, Mode, ParamClass, Program, RegTy, Ty, TyId,
    Width,
};

/// One argument or the result of a memoised function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoArg {
    /// One cell holding the value.
    Scalar,
    /// An array of plain scalars: the data address and the descriptor address (two cells for a
    /// parameter, a fat pair `[data, dope]` for a result).
    Array(MemoArr),
}

/// The shape of an array of plain scalars: what the bytes of a value are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoArr {
    pub dims: u8,
    /// Bit-planes (`ceil(n / 64)` blocks of 32 bytes), else `elem` bytes per element.
    pub logic: bool,
    pub elem: u8,
}

impl MemoArr {
    /// Bytes of the descriptor.
    pub fn desc_bytes(&self) -> usize {
        usize::from(self.dims) * crate::DOPE_DIM_BYTES as usize
    }

    /// Bytes of the elements of a value with the descriptor `desc` (`desc_bytes` long); `None`
    /// when the array is too large to count.
    pub fn data_bytes(&self, desc: &[u8]) -> Option<u64> {
        let mut n: u64 = 1;
        // (`DOPE_DIM_BYTES` is 24: three words a dimension.)
        for d in desc.as_chunks::<24>().0 {
            let word = |i: usize| i64::from_le_bytes(std::array::from_fn(|k| d[i * 8 + k]));
            n = n.checked_mul(crate::dim_len(word(0), word(1), word(2) == 1)?)?;
        }
        if self.logic {
            n.div_ceil(64).checked_mul(32)
        } else {
            n.checked_mul(u64::from(self.elem))
        }
    }
}

/// What a call of a function can be memoised on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoSig {
    pub args: Vec<MemoArg>,
    pub ret: MemoArg,
    /// Cells of the argument list of a call.
    pub cells: u32,
}

/// A scalar of a plain type: an integer, physical, enumeration or real value in one cell.
fn plain_scalar(prog: &Program, t: TyId) -> bool {
    matches!(
        prog.types.get(t),
        Some(Ty::Int { .. } | Ty::Physical { .. } | Ty::Enum { .. } | Ty::Real)
    )
}

/// An array of plain scalars of type `t`.
pub(crate) fn plain_array(prog: &Program, t: TyId) -> Option<MemoArr> {
    let Some(Ty::Array { elem, dims, .. }) = prog.types.get(t) else {
        return None;
    };
    if !plain_scalar(prog, *elem) {
        return None;
    }
    let logic = Ty::logic_leaf(&prog.types, t).is_some();
    let elem = prog.types.get(*elem)?.elem_size()?;
    Some(MemoArr {
        dims: *dims,
        logic,
        elem: u8::try_from(elem).ok()?,
    })
}

/// An instruction a function may execute and still be memoised (`calls` collects the functions
/// it calls).
fn inst_ok(prog: &Program, i: &Inst, calls: &mut Vec<FuncId>) -> bool {
    match *i {
        Inst::AddrOf { space, .. } => matches!(
            space,
            AddrSpace::Frame | AddrSpace::Const(_) | AddrSpace::Pkg(_)
        ),
        Inst::Call { func, .. } => {
            calls.push(func);
            true
        }
        Inst::CallNative { native, .. } => prog
            .natives
            .get(native)
            .is_some_and(|n| crate::native_is_pure(&n.symbol)),
        Inst::New { shared, .. } | Inst::NewFill { shared, .. } => !shared,
        Inst::Const { .. }
        | Inst::ConstF { .. }
        | Inst::Move { .. }
        | Inst::Select { .. }
        | Inst::CopyConst { .. }
        | Inst::Alloc { .. }
        | Inst::TempRoot { .. }
        | Inst::Load { .. }
        | Inst::Store { .. }
        | Inst::Copy { .. }
        | Inst::Fill { .. }
        | Inst::Pos { .. }
        | Inst::SlicePos { .. }
        | Inst::Index { .. }
        | Inst::Field { .. }
        | Inst::Bin { .. }
        | Inst::Un { .. }
        | Inst::FBin { .. }
        | Inst::FUn { .. }
        | Inst::Phys { .. }
        | Inst::IToF { .. }
        | Inst::FToI { .. }
        | Inst::FToIRange { .. }
        | Inst::Cmp { .. }
        | Inst::FCmp { .. }
        | Inst::Bool { .. }
        | Inst::LogicOp { .. }
        | Inst::LNum { .. }
        | Inst::LEq { .. }
        | Inst::MemEq { .. }
        | Inst::LGet { .. }
        | Inst::LSet { .. }
        | Inst::LFill { .. }
        | Inst::LCopy { .. }
        | Inst::Cat { .. }
        | Inst::Check { .. }
        | Inst::FCheck { .. }
        | Inst::BinRange { .. }
        | Inst::MakeDope { .. }
        | Inst::DopeLen { .. }
        | Inst::LayoutOf { .. }
        | Inst::SubtypeCheck { .. }
        | Inst::Br { .. }
        | Inst::CondBr { .. }
        | Inst::Case { .. }
        | Inst::Ret { .. }
        | Inst::Unreachable { .. }
        | Inst::NewFrame { .. }
        | Inst::Free { .. }
        | Inst::Deref { .. }
        | Inst::Report { .. }
        | Inst::Stmt => true,
        _ => false,
    }
}

/// A function body that computes from its arguments only, and the functions it calls.
fn body_ok(prog: &Program, f: &Func, calls: &mut Vec<FuncId>) -> bool {
    f.kind == FuncKind::Function
        && f.flags.contains(FuncFlags::PURE)
        && !f.flags.contains(FuncFlags::MAY_SUSPEND)
        && !f.flags.contains(FuncFlags::TOUCHES_SHARED)
        && !f.flags.contains(FuncFlags::TOUCHES_FILES)
        && !f.flags.contains(FuncFlags::HAS_COVER)
        && f.parent.is_none()
        && f.defined
        && !f.reg_ty.contains(&RegTy::Sig)
        && f.params.iter().all(|p| p.class == ParamClass::Value)
        && f.full().code.iter().all(|i| inst_ok(prog, i, calls))
}

/// Whether `f` and everything it calls compute from their arguments only. A call back to a
/// function being examined is taken as fine: the answer holds if every function of the cycle
/// passes. `ok` holds the functions found fine in this examination.
fn clean(prog: &Program, id: FuncId, ok: &mut Vec<FuncId>) -> bool {
    if ok.contains(&id) {
        return true;
    }
    let Some(f) = prog.funcs.get(id) else {
        return false;
    };
    ok.push(id);
    let mut calls = Vec::new();
    if body_ok(prog, f, &mut calls) && calls.into_iter().all(|c| clean(prog, c, ok)) {
        return true;
    }
    // A failure anywhere fails the examination: the callers' `ok` entries rested on this one.
    false
}

/// Whether a function is expensive enough that remembering its calls pays: it loops, calls
/// another function, or has a body of some size.
fn worth(f: &Func) -> bool {
    const SIZE: usize = 120;
    let code = &f.full().code;
    code.len() >= SIZE
        || code.iter().any(|i| matches!(i, Inst::Call { .. }))
        || f.blocks.iter().any(|(b, blk)| {
            code[blk.start as usize..blk.end as usize]
                .last()
                .is_some_and(|i| match *i {
                    Inst::Br { to } => to.0 <= b.0,
                    Inst::CondBr { t, f, .. } => t.0 <= b.0 || f.0 <= b.0,
                    _ => false,
                })
        })
}

impl Program {
    /// What the calls of function `id` can be memoised on, if they can (see the module).
    pub fn memo_sig(&self, id: FuncId) -> Option<&MemoSig> {
        let raw = self.funcs.get(id)?;
        raw.memo
            .get_or_init(|| self.work_out_sig(id, raw))
            .as_deref()
    }

    fn work_out_sig(&self, id: FuncId, f: &Func) -> Option<Box<MemoSig>> {
        // The header decides most functions without decoding a body.
        if f.kind != FuncKind::Function
            || !f.flags.contains(FuncFlags::PURE)
            || f.flags.contains(FuncFlags::MAY_SUSPEND)
            || f.flags.contains(FuncFlags::TOUCHES_SHARED)
            || f.flags.contains(FuncFlags::TOUCHES_FILES)
        {
            return None;
        }
        let ret = match f.ret_ty? {
            t if plain_scalar(self, t) => MemoArg::Scalar,
            t => MemoArg::Array(plain_array(self, t)?),
        };
        let mut args = Vec::new();
        let mut cells = 0;
        let mut ps = f.params.iter().peekable();
        while let Some(p) = ps.next() {
            if p.mode != Mode::In || p.width != Width::B8 {
                return None;
            }
            if let Some(t) = p.arr {
                // The descriptor cell follows the data cell.
                let d = ps.next()?;
                if d.slot != p.slot + 8 || d.mode != Mode::In || d.width != Width::B8 {
                    return None;
                }
                args.push(MemoArg::Array(plain_array(self, t)?));
                cells += 2;
            } else if p.ty.is_some_and(|t| plain_scalar(self, t)) {
                args.push(MemoArg::Scalar);
                cells += 1;
            } else {
                return None;
            }
        }
        if !worth(f) || !clean(self, id, &mut Vec::new()) {
            return None;
        }
        Some(Box::new(MemoSig { args, ret, cells }))
    }
}

/// Whether the native `symbol` only computes from its arguments and the heap: package
/// initialisation that calls any other (files, the environment, clocks, assert state,
/// reflection) is not stored (`SharedState::tainted`, #407), and a function that calls one is
/// not memoised. A symbol not named here is not pure.
pub fn native_is_pure(symbol: &str) -> bool {
    if symbol.starts_with("yavs.ieee.") || symbol.starts_with("yavs.attr.") {
        return true;
    }
    if let Some(r) = symbol.strip_prefix("yavs.implicit.") {
        let family = r.split('.').next().unwrap_or(r);
        return !(family.starts_with("file_")
            || matches!(family, "read" | "write" | "flush" | "endfile"));
    }
    if let Some(r) = symbol.strip_prefix("yavs.textio.") {
        let op = r.split('.').next().unwrap_or(r);
        return !matches!(op, "readline" | "writeline" | "tee");
    }
    if let Some(r) = symbol.strip_prefix("yavs.env.") {
        return r.starts_with("to_string.")
            || r.starts_with("add.")
            || r.starts_with("sub.")
            || r.starts_with("tool_")
            || matches!(
                r.split('.').next().unwrap_or(r),
                "vhdl_version" | "resolution_limit"
            );
    }
    false
}
