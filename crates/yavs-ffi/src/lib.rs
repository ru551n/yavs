//! Calls C functions with a signature known only at run time, for
//! VHPIDIRECT `foreign` subprograms, without libffi.
//!
//! Classification is plain Rust ([`abi`]): each argument gets an integer
//! register, a float register or a stack slot per the target ABI. A small
//! per-target asm trampoline ([`sys`]) loads the registers, copies the
//! stack slots below a 16-byte aligned stack pointer, calls the function
//! and stores both result registers. Supported targets: x86_64 Linux
//! (SysV), x86_64 Windows (Win64) and aarch64 Linux (AAPCS64). Any other
//! target is a compile-time error.
//!
//! [`vhdl`] maps VHDL parameter and result types to C types the way sim1
//! does, so C code written for sim1's VHPIDIRECT works unchanged.
//!
//! Unsafe code is denied crate-wide and allowed only in `sys`.

use std::ffi::c_void;
use std::fmt;

mod abi;
mod sys;
pub mod vhdl;

pub use sys::{ForeignFn, call, testing};
pub use typed::{CArgType, CFnPtr, CRetType};

mod typed;

/// Most arguments one call may take. More is an error, not a crash.
pub const MAX_ARGS: usize = 64;

/// The C types a foreign call can pass or return.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum CType {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    F32,
    F64,
    Ptr,
}

impl CType {
    pub(crate) fn is_float(self) -> bool {
        matches!(self, CType::F32 | CType::F64)
    }
}

impl fmt::Display for CType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CType::I8 => "int8_t",
            CType::I16 => "int16_t",
            CType::I32 => "int32_t",
            CType::I64 => "int64_t",
            CType::U8 => "uint8_t",
            CType::U16 => "uint16_t",
            CType::U32 => "uint32_t",
            CType::F32 => "float",
            CType::F64 => "double",
            CType::Ptr => "void *",
        })
    }
}

/// A C function type: parameter types and the result (`None` is `void`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Signature {
    pub params: Vec<CType>,
    pub ret: Option<CType>,
}

/// One argument value. Integers are widened to 64 bits (signed types
/// sign-extended, unsigned zero-extended) before they reach a register or
/// stack slot, which every supported ABI accepts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Arg {
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    U8(u8),
    U16(u16),
    U32(u32),
    F32(f32),
    F64(f64),
    Ptr(*mut c_void),
}

impl Arg {
    pub fn ctype(&self) -> CType {
        match self {
            Arg::I8(_) => CType::I8,
            Arg::I16(_) => CType::I16,
            Arg::I32(_) => CType::I32,
            Arg::I64(_) => CType::I64,
            Arg::U8(_) => CType::U8,
            Arg::U16(_) => CType::U16,
            Arg::U32(_) => CType::U32,
            Arg::F32(_) => CType::F32,
            Arg::F64(_) => CType::F64,
            Arg::Ptr(_) => CType::Ptr,
        }
    }

    /// The 64-bit register or slot image of the value.
    pub(crate) fn bits(&self) -> u64 {
        match *self {
            Arg::I8(v) => v as i64 as u64,
            Arg::I16(v) => v as i64 as u64,
            Arg::I32(v) => v as i64 as u64,
            Arg::I64(v) => v as u64,
            Arg::U8(v) => v.into(),
            Arg::U16(v) => v.into(),
            Arg::U32(v) => v.into(),
            Arg::F32(v) => v.to_bits().into(),
            Arg::F64(v) => v.to_bits(),
            Arg::Ptr(v) => v as usize as u64,
        }
    }
}

/// A call result, narrowed from the result register to the declared type
/// (the callee may leave garbage in the upper bits).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ret {
    Void,
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    U8(u8),
    U16(u16),
    U32(u32),
    F32(f32),
    F64(f64),
    Ptr(*mut c_void),
}

impl Ret {
    pub(crate) fn decode(ty: Option<CType>, int: u64, float: u64) -> Ret {
        let Some(ty) = ty else { return Ret::Void };
        match ty {
            CType::I8 => Ret::I8(int as i8),
            CType::I16 => Ret::I16(int as i16),
            CType::I32 => Ret::I32(int as i32),
            CType::I64 => Ret::I64(int as i64),
            CType::U8 => Ret::U8(int as u8),
            CType::U16 => Ret::U16(int as u16),
            CType::U32 => Ret::U32(int as u32),
            CType::F32 => Ret::F32(f32::from_bits(float as u32)),
            CType::F64 => Ret::F64(f64::from_bits(float)),
            CType::Ptr => Ret::Ptr(int as usize as *mut c_void),
        }
    }
}

/// Why a foreign call or a VHDL to C mapping was refused. Nothing is
/// called when an error is returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FfiError {
    /// The function pointer is null.
    NullFunction,
    /// The signature has more parameters than [`MAX_ARGS`].
    TooManyArgs { count: usize, max: usize },
    /// The call passes a different number of arguments than declared.
    ArgCount { expected: usize, found: usize },
    /// Argument `index` (0-based) has the wrong C type.
    ArgType {
        index: usize,
        expected: CType,
        found: CType,
    },
    /// A VHDL type or mode that VHPIDIRECT cannot pass.
    Unsupported { what: String },
}

impl fmt::Display for FfiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FfiError::NullFunction => write!(f, "foreign function pointer is null"),
            FfiError::TooManyArgs { count, max } => write!(
                f,
                "foreign function takes {count} C arguments, at most {max} are supported"
            ),
            FfiError::ArgCount { expected, found } => write!(
                f,
                "foreign call passes {found} arguments, the signature declares {expected}"
            ),
            FfiError::ArgType {
                index,
                expected,
                found,
            } => write!(
                f,
                "foreign call argument {} is {found}, the signature declares {expected}",
                index + 1
            ),
            FfiError::Unsupported { what } => {
                write!(f, "cannot pass to a VHPIDIRECT subprogram: {what}")
            }
        }
    }
}

impl std::error::Error for FfiError {}
