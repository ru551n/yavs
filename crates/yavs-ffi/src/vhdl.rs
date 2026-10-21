//! VHDL to C type mapping for VHPIDIRECT subprograms.
//!
//! Follows sim1's convention so C code written for sim1 works unchanged
//! (reference: the sim1 user manual, `sim1(1)`, section "VHPIDIRECT" under
//! foreign subprograms, as shipped with sim1 1.x):
//!
//! - integers: the smallest signed C integer type holding the VHDL range
//!   (`integer` is `int32_t`, `integer range 1 to 5` is `int8_t`);
//! - enumerations: the smallest unsigned type holding the literal count
//!   (`boolean`, `bit`, `character`, `std_ulogic` are `uint8_t`);
//! - `real`: `double`; physical types: `int64_t` in resolution units;
//! - constrained arrays: a pointer to the element data;
//! - unconstrained arrays: the element data pointer followed by one
//!   `int64_t` length argument per dimension (bounds and direction are not
//!   passed; the VHDL side passes them explicitly if needed);
//! - scalar `out`/`inout` parameters: a pointer to the C-typed value.
//!
//! yavs additions beyond sim1: records and access values are passed as a
//! pointer (records to a C struct with natural alignment).

use crate::{CType, FfiError, Signature};

/// The shape of a VHDL parameter or result type, as far as C sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VhdlType {
    /// An integer type or subtype with this range.
    Integer {
        low: i64,
        high: i64,
    },
    /// An enumeration type with this many literals.
    Enum {
        literals: u64,
    },
    Physical,
    Real,
    Access,
    ConstrainedArray,
    UnconstrainedArray {
        dims: u32,
    },
    Record,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    In,
    Out,
    InOut,
}

fn scalar(ty: VhdlType) -> Result<CType, FfiError> {
    Ok(match ty {
        VhdlType::Integer { low, high } => {
            let fits = |min: i64, max: i64| low >= min && high <= max;
            if fits(i8::MIN.into(), i8::MAX.into()) {
                CType::I8
            } else if fits(i16::MIN.into(), i16::MAX.into()) {
                CType::I16
            } else if fits(i32::MIN.into(), i32::MAX.into()) {
                CType::I32
            } else {
                CType::I64
            }
        }
        VhdlType::Enum { literals: 0 } => {
            return Err(FfiError::Unsupported {
                what: "an enumeration type without literals".into(),
            });
        }
        VhdlType::Enum { literals } if literals <= 1 << 8 => CType::U8,
        VhdlType::Enum { literals } if literals <= 1 << 16 => CType::U16,
        VhdlType::Enum { literals } if literals <= 1 << 32 => CType::U32,
        VhdlType::Enum { literals } => {
            return Err(FfiError::Unsupported {
                what: format!("an enumeration type with {literals} literals"),
            });
        }
        VhdlType::Physical => CType::I64,
        VhdlType::Real => CType::F64,
        VhdlType::Access => CType::Ptr,
        VhdlType::Record => {
            return Err(FfiError::Unsupported {
                what: "a record result (only scalar results are supported)".into(),
            });
        }
        VhdlType::ConstrainedArray | VhdlType::UnconstrainedArray { .. } => {
            return Err(FfiError::Unsupported {
                what: "an array result (only scalar results are supported)".into(),
            });
        }
    })
}

/// The C arguments one VHDL parameter becomes, in order.
pub fn lower_param(ty: VhdlType, mode: Mode) -> Result<Vec<CType>, FfiError> {
    Ok(match ty {
        VhdlType::UnconstrainedArray { dims: 0 } => {
            return Err(FfiError::Unsupported {
                what: "an array type without dimensions".into(),
            });
        }
        VhdlType::UnconstrainedArray { dims } => {
            let mut v = vec![CType::Ptr];
            v.extend(std::iter::repeat_n(CType::I64, dims as usize));
            v
        }
        VhdlType::ConstrainedArray | VhdlType::Record => vec![CType::Ptr],
        _ if mode != Mode::In => {
            scalar(ty)?;
            vec![CType::Ptr]
        }
        _ => vec![scalar(ty)?],
    })
}

/// The C result type of a VHDL function (`None` for a procedure).
pub fn lower_result(ty: Option<VhdlType>) -> Result<Option<CType>, FfiError> {
    ty.map(scalar).transpose()
}

/// The C signature of a whole VHPIDIRECT subprogram.
pub fn lower(params: &[(VhdlType, Mode)], result: Option<VhdlType>) -> Result<Signature, FfiError> {
    let mut c = Vec::new();
    for &(ty, mode) in params {
        c.extend(lower_param(ty, mode)?);
    }
    Ok(Signature {
        params: c,
        ret: lower_result(result)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use CType::*;
    use Mode::*;
    use VhdlType::*;

    const INTEGER: VhdlType = Integer {
        low: i32::MIN as i64,
        high: i32::MAX as i64,
    };
    const STD_LOGIC: VhdlType = Enum { literals: 9 };

    #[test]
    fn sim1_manual_examples() {
        // int32_t my_func(int32_t x, const uint8_t *y, int64_t y_len, uint8_t z);
        let s = lower(
            &[
                (INTEGER, In),
                (UnconstrainedArray { dims: 1 }, In),
                (STD_LOGIC, In),
            ],
            Some(INTEGER),
        )
        .unwrap();
        assert_eq!(s.params, [I32, Ptr, I64, U8]);
        assert_eq!(s.ret, Some(I32));
        // void my_proc(int32_t *x, uint8_t *y, int64_t y_len, uint8_t z);
        let s = lower(
            &[
                (INTEGER, Out),
                (UnconstrainedArray { dims: 1 }, Out),
                (STD_LOGIC, In),
            ],
            None,
        )
        .unwrap();
        assert_eq!(s.params, [Ptr, Ptr, I64, U8]);
        assert_eq!(s.ret, None);
        assert_eq!(lower_param(Integer { low: 1, high: 5 }, In).unwrap(), [I8]);
        assert_eq!(lower_param(Real, In).unwrap(), [F64]);
        assert_eq!(lower_param(ConstrainedArray, In).unwrap(), [Ptr]);
    }

    #[test]
    fn widths_and_composites() {
        assert_eq!(
            lower_param(
                Integer {
                    low: 0,
                    high: 40000
                },
                In
            )
            .unwrap(),
            [I32]
        );
        assert_eq!(
            lower_param(
                Integer {
                    low: -300,
                    high: 300
                },
                In
            )
            .unwrap(),
            [I16]
        );
        assert_eq!(
            lower_param(
                Integer {
                    low: 0,
                    high: 1 << 40
                },
                In
            )
            .unwrap(),
            [I64]
        );
        assert_eq!(lower_param(Enum { literals: 256 }, In).unwrap(), [U8]);
        assert_eq!(lower_param(Enum { literals: 257 }, In).unwrap(), [U16]);
        assert_eq!(lower_param(Physical, InOut).unwrap(), [Ptr]);
        assert_eq!(lower_param(Record, In).unwrap(), [Ptr]);
        assert_eq!(lower_param(Access, In).unwrap(), [Ptr]);
        assert_eq!(
            lower_param(UnconstrainedArray { dims: 2 }, In).unwrap(),
            [Ptr, I64, I64]
        );
        assert_eq!(lower_result(Some(Physical)).unwrap(), Some(I64));
        assert!(matches!(
            lower_result(Some(Record)),
            Err(FfiError::Unsupported { .. })
        ));
        assert!(lower_param(Enum { literals: 0 }, Out).is_err());
        assert!(lower_param(UnconstrainedArray { dims: 0 }, In).is_err());
    }
}
