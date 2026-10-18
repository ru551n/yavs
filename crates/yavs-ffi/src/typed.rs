//! C signatures derived from Rust `extern "C" fn` pointer types, so a
//! Rust callee can be bound without unsafe code (`ForeignFn::from_fn`).

use std::ffi::c_void;

use crate::{CType, Signature};

/// A Rust type that is one C argument type.
pub trait CArgType {
    const CTYPE: CType;
}

/// A Rust type that is a C result type; `()` is `void`.
pub trait CRetType {
    const CTYPE: Option<CType>;
}

macro_rules! arg_types {
    ($($t:ty => $c:ident),*) => {$(
        impl CArgType for $t {
            const CTYPE: CType = CType::$c;
        }
        impl CRetType for $t {
            const CTYPE: Option<CType> = Some(CType::$c);
        }
    )*};
}

arg_types!(i8 => I8, i16 => I16, i32 => I32, i64 => I64, u8 => U8, u16 => U16, u32 => U32,
    f32 => F32, f64 => F64, *mut c_void => Ptr);

impl CRetType for () {
    const CTYPE: Option<CType> = None;
}

/// A safe `extern "C" fn` pointer with C argument and result types.
pub trait CFnPtr: Copy {
    fn signature() -> Signature;
    fn addr(self) -> *const c_void;
}

macro_rules! fn_ptrs {
    ($(($($a:ident),*)),*) => {$(
        impl<R: CRetType, $($a: CArgType),*> CFnPtr for extern "C" fn($($a),*) -> R {
            fn signature() -> Signature {
                Signature {
                    params: vec![$($a::CTYPE),*],
                    ret: R::CTYPE,
                }
            }
            fn addr(self) -> *const c_void {
                self as *const c_void
            }
        }
    )*};
}

fn_ptrs!(
    (),
    (A),
    (A, B),
    (A, B, C),
    (A, B, C, D),
    (A, B, C, D, E),
    (A, B, C, D, E, F),
    (A, B, C, D, E, F, G),
    (A, B, C, D, E, F, G, H)
);

#[cfg(test)]
mod tests {
    use crate::{Arg, ForeignFn, Ret};

    extern "C" fn mix(a: i32, b: f64, c: u8) -> i64 {
        i64::from(a) * 100 + b as i64 * 10 + i64::from(c)
    }

    #[test]
    fn typed_binding() {
        let f = ForeignFn::from_fn(mix as extern "C" fn(i32, f64, u8) -> i64);
        assert_eq!(f.signature().params.len(), 3);
        let r = f.call(&[Arg::I32(7), Arg::F64(2.0), Arg::U8(3)]).unwrap();
        assert_eq!(r, Ret::I64(723));
    }
}
