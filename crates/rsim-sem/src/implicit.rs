//! Implicitly declared operations (sem.md 2.10, `lib/NATIVE.md` section 2).
//!
//! Every predefined operation is a real `DeclKind::Subprogram` entity, created
//! in the same region right after its type, with `implicit = true`, so
//! visibility, hiding, aliases, `use p."+"` and overload resolution need no
//! special cases.

use crate::ids::TypeId;

/// The native link of an implicit op is `rsim.implicit.<family>.<type mangled>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImplicitOp {
    pub family: Family,
    pub ty: TypeId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Eq,
    Neq,
    Lt,
    Le,
    Gt,
    Ge,
    Minimum,
    Maximum,
    ToString,
    Add,
    Sub,
    Mul,
    Div,
    Neg,
    Pos,
    Abs,
    Exp,
    Rem,
    Mod,
    And,
    Or,
    Nand,
    Nor,
    Xor,
    Xnor,
    Not,
    /// Matching relational operators `?=` etc. and the condition operator `??`.
    MEq,
    MNeq,
    MLt,
    MLe,
    MGt,
    MGe,
    Cond,
    Cat,
    Sll,
    Srl,
    Sla,
    Sra,
    Rol,
    Ror,
    ToOString,
    ToHString,
    Deallocate,
    FileOpen,
    FileClose,
    Read,
    Write,
    Endfile,
    Flush,
    FileState,
    FileMode,
    FilePosition,
    FileSize,
    FileSeek,
    FileTruncate,
    FileRewind,
    RisingEdge,
    FallingEdge,
}

impl Family {
    /// The family part of the native symbol.
    pub fn as_str(self) -> &'static str {
        use Family::*;
        match self {
            Eq => "eq",
            Neq => "neq",
            Lt => "lt",
            Le => "le",
            Gt => "gt",
            Ge => "ge",
            Minimum => "minimum",
            Maximum => "maximum",
            ToString => "to_string",
            Add => "add",
            Sub => "sub",
            Mul => "mul",
            Div => "div",
            Neg => "neg",
            Pos => "pos",
            Abs => "abs",
            Exp => "exp",
            Rem => "rem",
            Mod => "mod",
            And => "and",
            Or => "or",
            Nand => "nand",
            Nor => "nor",
            Xor => "xor",
            Xnor => "xnor",
            Not => "not",
            MEq => "meq",
            MNeq => "mneq",
            MLt => "mlt",
            MLe => "mle",
            MGt => "mgt",
            MGe => "mge",
            Cond => "cond",
            Cat => "cat",
            Sll => "sll",
            Srl => "srl",
            Sla => "sla",
            Sra => "sra",
            Rol => "rol",
            Ror => "ror",
            ToOString => "to_ostring",
            ToHString => "to_hstring",
            Deallocate => "deallocate",
            FileOpen => "file_open",
            FileClose => "file_close",
            Read => "read",
            Write => "write",
            Endfile => "endfile",
            Flush => "flush",
            FileState => "file_state",
            FileMode => "file_mode",
            FilePosition => "file_position",
            FileSize => "file_size",
            FileSeek => "file_seek",
            FileTruncate => "file_truncate",
            FileRewind => "file_rewind",
            RisingEdge => "rising_edge",
            FallingEdge => "falling_edge",
        }
    }
}

// ---------------------------------------------------------------------------
// Hooks called by `types.rs` (M1 lead); the bodies are this module's owner's.

use crate::db::Db;
use crate::ids::ScopeId;

impl Db {
    /// Create the implicitly declared operations of `ty` in `scope`, right after its
    /// declaration (2.10). Operations whose parameter or result types are not
    /// declared yet (inside `std.standard`: BOOLEAN, INTEGER, STRING, ...) are
    /// queued in `pending_implicit` and created by [`Db::flush_pending_implicit`].
    pub(crate) fn declare_implicit_ops(&mut self, ty: TypeId, scope: ScopeId) {
        self.pending_implicit.push((ty, scope));
        self.flush_pending_implicit();
    }

    /// Retry queued implicit operations; called after each `std.standard` type or subtype.
    // ponytail: stub until the implicit-ops owner lands the families; until then the
    // queue only drains outside std.standard (where no op is created at all).
    pub(crate) fn flush_pending_implicit(&mut self) {
        if self.well_known.standard.is_none() || self.well_known.string.is_some() {
            self.pending_implicit.clear();
        }
    }
}
