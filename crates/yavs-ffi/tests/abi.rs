//! Hand-written trampoline tests: class mixes, spills, extension and
//! narrowing promises, stack alignment, pointer write-back and errors.
// Every call below passes a callee whose type is exactly the signature,
// except where a test deliberately declares a narrower C type than the
// callee reads (both are 64-bit register images, which is the point).
#![allow(unsafe_code)]

use std::ffi::c_void;

use yavs_ffi::{Arg, CType, FfiError, ForeignFn, MAX_ARGS, Ret, Signature, call};

fn sig(params: &[CType], ret: Option<CType>) -> Signature {
    Signature {
        params: params.to_vec(),
        ret,
    }
}

fn run(f: *const c_void, params: &[CType], ret: Option<CType>, args: &[Arg]) -> Ret {
    unsafe { call(f, &sig(params, ret), args) }.unwrap()
}

extern "C" fn ints12(
    a: i64,
    b: i64,
    c: i64,
    d: i64,
    e: i64,
    f: i64,
    g: i64,
    h: i64,
    i: i64,
    j: i64,
    k: i64,
    l: i64,
) -> i64 {
    [a, b, c, d, e, f, g, h, i, j, k, l]
        .iter()
        .enumerate()
        .map(|(n, v)| v * (n as i64 + 1) * 1000_i64.pow(n as u32 % 3))
        .sum()
}

#[test]
fn ints_only_spill() {
    let args: Vec<Arg> = (1..=12).map(Arg::I64).collect();
    let want = ints12(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12);
    assert_eq!(
        run(
            ints12 as *const c_void,
            &[CType::I64; 12],
            Some(CType::I64),
            &args
        ),
        Ret::I64(want)
    );
}

extern "C" fn floats10(
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
    g: f64,
    h: f64,
    i: f64,
    j: f32,
) -> f64 {
    a + 2.0 * b
        + 4.0 * c
        + 8.0 * d
        + 16.0 * e
        + 32.0 * f
        + 64.0 * g
        + 128.0 * h
        + 256.0 * i
        + 512.0 * f64::from(j)
}

#[test]
fn floats_only_spill_and_f32_on_stack() {
    let mut params = vec![CType::F64; 9];
    params.push(CType::F32);
    let mut args: Vec<Arg> = (0..9).map(|_| Arg::F64(1.0)).collect();
    args.push(Arg::F32(1.0));
    assert_eq!(
        run(floats10 as *const c_void, &params, Some(CType::F64), &args),
        Ret::F64(1023.0)
    );
}

extern "C" fn interleaved(a: i32, b: f32, c: u8, d: f64, e: *mut c_void, f: i16, g: f64) -> f64 {
    f64::from(a)
        + f64::from(b) * 10.0
        + f64::from(c) * 100.0
        + d * 1000.0
        + (e as usize as f64) * 10000.0
        + f64::from(f) * 100000.0
        + g * 1e6
}

#[test]
fn interleaved_classes() {
    use CType::*;
    let r = run(
        interleaved as *const c_void,
        &[I32, F32, U8, F64, Ptr, I16, F64],
        Some(F64),
        &[
            Arg::I32(-1),
            Arg::F32(2.0),
            Arg::U8(3),
            Arg::F64(4.0),
            Arg::Ptr(5 as *mut c_void),
            Arg::I16(-6),
            Arg::F64(7.0),
        ],
    );
    assert_eq!(
        r,
        Ret::F64(-1.0 + 20.0 + 300.0 + 4000.0 + 50000.0 - 600000.0 + 7e6)
    );
}

/// Sums 32 arguments with position weights, in two classes.
macro_rules! wide {
    ($name:ident, $t:ty, $zero:expr, $one:expr) => {
        #[allow(clippy::too_many_arguments)]
        extern "C" fn $name(
            a0: $t,
            a1: $t,
            a2: $t,
            a3: $t,
            a4: $t,
            a5: $t,
            a6: $t,
            a7: $t,
            a8: $t,
            a9: $t,
            a10: $t,
            a11: $t,
            a12: $t,
            a13: $t,
            a14: $t,
            a15: $t,
            a16: $t,
            a17: $t,
            a18: $t,
            a19: $t,
            a20: $t,
            a21: $t,
            a22: $t,
            a23: $t,
            a24: $t,
            a25: $t,
            a26: $t,
            a27: $t,
            a28: $t,
            a29: $t,
            a30: $t,
            a31: $t,
        ) -> $t {
            let v = [
                a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, a16, a17,
                a18, a19, a20, a21, a22, a23, a24, a25, a26, a27, a28, a29, a30, a31,
            ];
            let (mut s, mut w) = ($zero, $zero);
            for x in v {
                w += $one;
                s += x * w;
            }
            s
        }
    };
}
wide!(wide_i64, i64, 0, 1);
wide!(wide_f64, f64, 0.0, 1.0);

#[test]
fn thirty_two_args_each_class() {
    let ints: Vec<Arg> = (1..=32).map(Arg::I64).collect();
    let want: i64 = (1..=32).map(|i| i * i).sum();
    assert_eq!(
        run(
            wide_i64 as *const c_void,
            &[CType::I64; 32],
            Some(CType::I64),
            &ints
        ),
        Ret::I64(want)
    );
    let floats: Vec<Arg> = (1..=32).map(|i| Arg::F64(f64::from(i))).collect();
    assert_eq!(
        run(
            wide_f64 as *const c_void,
            &[CType::F64; 32],
            Some(CType::F64),
            &floats
        ),
        Ret::F64(want as f64)
    );
}

extern "C" fn see_i64(x: i64) -> i64 {
    x
}

extern "C" fn see_u64(x: u64) -> u64 {
    x
}

#[test]
fn integers_reach_the_callee_widened_to_64_bits() {
    let f = see_i64 as *const c_void;
    let r = |ty, a| run(f, &[ty], Some(CType::I64), &[a]);
    assert_eq!(r(CType::I32, Arg::I32(-1)), Ret::I64(-1));
    assert_eq!(r(CType::I32, Arg::I32(i32::MIN)), Ret::I64(i32::MIN.into()));
    assert_eq!(r(CType::I8, Arg::I8(-128)), Ret::I64(-128));
    assert_eq!(r(CType::I16, Arg::I16(-2)), Ret::I64(-2));
    let f = see_u64 as *const c_void;
    let r = |ty, a| run(f, &[ty], Some(CType::I64), &[a]);
    assert_eq!(r(CType::U8, Arg::U8(255)), Ret::I64(255));
    assert_eq!(r(CType::U16, Arg::U16(0xffff)), Ret::I64(0xffff));
    assert_eq!(r(CType::U32, Arg::U32(u32::MAX)), Ret::I64(0xffff_ffff));
}

extern "C" fn garbage() -> u64 {
    0xdead_beef_ffff_ff85
}

extern "C" fn ret_f32() -> f32 {
    -1.5
}

extern "C" fn ret_ptr() -> *mut c_void {
    0x1234_5678 as *mut c_void
}

extern "C" fn ret_void(p: *mut i32) {
    unsafe { *p = 42 };
}

#[test]
fn results_are_narrowed_to_the_declared_type() {
    let f = garbage as *const c_void;
    let r = |ty| run(f, &[], Some(ty), &[]);
    assert_eq!(r(CType::I32), Ret::I32(-123));
    assert_eq!(r(CType::U8), Ret::U8(0x85));
    assert_eq!(r(CType::I8), Ret::I8(-123));
    assert_eq!(r(CType::I16), Ret::I16(-123));
    assert_eq!(r(CType::U16), Ret::U16(0xff85));
    assert_eq!(r(CType::U32), Ret::U32(0xffff_ff85));
    assert_eq!(r(CType::I64), Ret::I64(0xdead_beef_ffff_ff85_u64 as i64));
    assert_eq!(
        run(ret_f32 as *const c_void, &[], Some(CType::F32), &[]),
        Ret::F32(-1.5)
    );
    assert_eq!(
        run(ret_ptr as *const c_void, &[], Some(CType::Ptr), &[]),
        Ret::Ptr(0x1234_5678 as *mut c_void)
    );
}

#[test]
fn out_parameter_written_through_pointer() {
    let mut x = 0i32;
    let r = run(
        ret_void as *const c_void,
        &[CType::Ptr],
        None,
        &[Arg::Ptr(&mut x as *mut i32 as *mut c_void)],
    );
    assert_eq!((r, x), (Ret::Void, 42));
}

#[repr(align(16))]
struct Aligned(#[allow(dead_code)] [u8; 16]);

macro_rules! aligned_check {
    ($name:ident, $($a:ident),*) => {
        #[allow(clippy::too_many_arguments)]
        extern "C" fn $name($($a: i64),*) -> u8 {
            $(std::hint::black_box($a);)*
            let a = Aligned([0; 16]);
            (std::hint::black_box(&a) as *const Aligned as usize % 16 == 0).into()
        }
    };
}
aligned_check!(al0,);
aligned_check!(al7, a, b, c, d, e, f, g);
aligned_check!(al8, a, b, c, d, e, f, g, h);
aligned_check!(al9, a, b, c, d, e, f, g, h, i);
aligned_check!(al10, a, b, c, d, e, f, g, h, i, j);

#[test]
fn stack_is_16_aligned_for_odd_and_even_spills() {
    for (f, n) in [
        (al0 as *const c_void, 0),
        (al7 as *const c_void, 7),
        (al8 as *const c_void, 8),
        (al9 as *const c_void, 9),
        (al10 as *const c_void, 10),
    ] {
        let args = vec![Arg::I64(1); n];
        assert_eq!(
            run(f, &vec![CType::I64; n], Some(CType::U8), &args),
            Ret::U8(1),
            "{n} args"
        );
    }
}

#[test]
fn errors_are_structured_and_nothing_is_called() {
    let s = sig(&[CType::I32], Some(CType::I64));
    assert_eq!(
        unsafe { call(std::ptr::null(), &s, &[Arg::I32(1)]) },
        Err(FfiError::NullFunction)
    );
    let f = see_i64 as *const c_void;
    let big = sig(&[CType::I64; MAX_ARGS + 1], None);
    assert_eq!(
        unsafe { ForeignFn::new(f, big) },
        Err(FfiError::TooManyArgs {
            count: MAX_ARGS + 1,
            max: MAX_ARGS
        })
    );
    let ff = unsafe { ForeignFn::new(f, s) }.unwrap();
    assert_eq!(
        ff.call(&[]),
        Err(FfiError::ArgCount {
            expected: 1,
            found: 0
        })
    );
    let e = ff.call(&[Arg::I64(1)]).unwrap_err();
    assert_eq!(
        e,
        FfiError::ArgType {
            index: 0,
            expected: CType::I32,
            found: CType::I64
        }
    );
    assert_eq!(
        e.to_string(),
        "foreign call argument 1 is int64_t, the signature declares int32_t"
    );
    assert_eq!(ff.call(&[Arg::I32(-7)]), Ok(Ret::I64(-7)));
}
