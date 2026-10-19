//! The exported `vhpi_*` entry points. Every function here is part of the
//! C ABI listed in `exports.txt`. The variadic ones (`vhpi_printf`,
//! `vhpi_vprintf`, `vhpi_assert`, `vhpi_control`) live in `shim.c` and
//! call the `yavs_vhpi_*` helpers below.
//!
//! VHPI has no context argument, so the model is one process-wide
//! `State` behind a mutex. The lock is never held while plugin code runs
//! (callbacks), because callbacks call back into VHPI.

// VHPI names are matched as written in the C standard.
#![allow(non_upper_case_globals)]

use std::ffi::{CStr, c_char, c_int, c_long, c_void};
use std::io::Write;
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::callbacks::{Callback, RegisterError};
use crate::model::{Control, Fault, INT_PROPS, Iter, Object, STR_PROPS, State};
use crate::sys::*;

static STATE: Mutex<Option<State>> = Mutex::new(None);

// SAFETY: the raw pointers in a callback's `vhpiCbDataT` (user data, the
// trigger object) belong to the plugin. yavs never dereferences them, it
// only hands them back to the plugin, and only under the state mutex.
unsafe impl Send for Callback {}

fn lock() -> MutexGuard<'static, Option<State>> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs `f` on the state. Clears the previous error first, as each VHPI
/// call does, so `vhpi_check_error` reports only the latest call.
pub(crate) fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = lock();
    let state = guard.get_or_insert_with(State::default);
    state.error = None;
    f(state)
}

const NOT_IMPLEMENTED: &str = "not implemented in yavs yet";

fn not_implemented(name: &str) {
    with_state(|s| s.fail(Fault::Unsupported, format!("{name}: {NOT_IMPLEMENTED}")));
}

fn get_obj(s: &mut State, h: vhpiHandleT, func: &str) -> Option<Object> {
    let obj = s.handles.get(h).cloned();
    if obj.is_none() {
        s.fail(Fault::Misuse, format!("{func}: invalid or released handle"));
    }
    obj
}

/// The object of an optional reference handle: `Ok(None)` for NULL, `Err` (reported) for a
/// stale one.
fn get_ref(s: &mut State, h: vhpiHandleT, func: &str) -> Result<Option<Object>, ()> {
    if h.is_null() {
        return Ok(None);
    }
    get_obj(s, h, func).map(Some).ok_or(())
}

// ---- Called from shim.c ----------------------------------------------

/// Writes formatted plugin output to stdout. Returns 0 or -1.
///
/// # Safety
/// `buf` points to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yavs_vhpi_write(buf: *const c_char, len: usize) -> c_int {
    // SAFETY: the caller passes a buffer of `len` bytes.
    let bytes = unsafe { std::slice::from_raw_parts(buf.cast::<u8>(), len) };
    with_state(|_| ());
    let mut out = std::io::stdout().lock();
    match out.write_all(bytes).and_then(|()| out.flush()) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// `vhpi_assert` after formatting: prints like a VHDL report.
///
/// # Safety
/// `msg` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yavs_vhpi_assert(severity: c_int, msg: *const c_char) -> c_int {
    // SAFETY: the shim passes the string it just formatted.
    let msg = unsafe { CStr::from_ptr(msg) }.to_string_lossy();
    let level = match severity {
        vhpiNote => "Note",
        vhpiWarning => "Warning",
        vhpiError => "Error",
        vhpiFailure => "Failure",
        _ => {
            with_state(|s| {
                s.fail(
                    Fault::Misuse,
                    format!("vhpi_assert: bad severity {severity}"),
                )
            });
            return 1;
        }
    };
    with_state(|_| ());
    eprintln!("VHPI assertion {level}: {msg}");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn yavs_vhpi_control(command: c_int) -> c_int {
    with_state(|s| {
        let request = match command {
            vhpiStop => Control::Stop,
            vhpiFinish => Control::Finish,
            _ => {
                s.fail(
                    Fault::Unsupported,
                    format!("vhpi_control: command {command} not supported"),
                );
                return 1;
            }
        };
        s.control = Some(request);
        0
    })
}

// Declared so `exported_symbols` can take their addresses.
unsafe extern "C" {
    fn vhpi_printf(fmt: *const c_char, ...) -> c_int;
    fn vhpi_vprintf(fmt: *const c_char, args: *mut c_void) -> c_int;
    fn vhpi_assert(severity: vhpiSeverityT, fmt: *mut c_char, ...) -> c_int;
    fn vhpi_control(command: vhpiSimControlT, ...) -> c_int;
}

// ---- Data symbols ------------------------------------------------------

const fn phys(fs: u64) -> vhpiPhysT {
    vhpiPhysT {
        high: (fs >> 32) as i32,
        low: fs as u32,
    }
}

#[unsafe(no_mangle)]
pub static vhpiFS: vhpiPhysT = phys(1);
#[unsafe(no_mangle)]
pub static vhpiPS: vhpiPhysT = phys(1_000);
#[unsafe(no_mangle)]
pub static vhpiNS: vhpiPhysT = phys(1_000_000);
#[unsafe(no_mangle)]
pub static vhpiUS: vhpiPhysT = phys(1_000_000_000);
#[unsafe(no_mangle)]
pub static vhpiMS: vhpiPhysT = phys(1_000_000_000_000);
#[unsafe(no_mangle)]
pub static vhpiS: vhpiPhysT = phys(1_000_000_000_000_000);
#[unsafe(no_mangle)]
pub static vhpiMN: vhpiPhysT = phys(60_000_000_000_000_000);
#[unsafe(no_mangle)]
pub static vhpiHR: vhpiPhysT = phys(3_600_000_000_000_000_000);

// ---- Handles -------------------------------------------------------------

/// # Safety
/// `name` is NULL or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vhpi_handle_by_name(
    name: *const c_char,
    scope: vhpiHandleT,
) -> vhpiHandleT {
    if name.is_null() {
        with_state(|s| s.fail(Fault::Misuse, "vhpi_handle_by_name: NULL name"));
        return std::ptr::null_mut();
    }
    // SAFETY: non-NULL and NUL-terminated per the contract.
    let name = unsafe { CStr::from_ptr(name) };
    with_state(|s| {
        let Ok(scope) = get_ref(s, scope, "vhpi_handle_by_name") else {
            return std::ptr::null_mut();
        };
        match s.lookup(name, scope.as_ref()) {
            Some(obj) => s.handles.alloc(obj),
            None => {
                s.fail(
                    Fault::NotFound,
                    format!(
                        "vhpi_handle_by_name: no object named {}",
                        name.to_string_lossy()
                    ),
                );
                std::ptr::null_mut()
            }
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn vhpi_handle(kind: vhpiOneToOneT, reference: vhpiHandleT) -> vhpiHandleT {
    with_state(|s| {
        let Ok(r) = get_ref(s, reference, "vhpi_handle") else {
            return std::ptr::null_mut();
        };
        match s.one_to_one(kind, r.as_ref()) {
            Ok(obj) => s.handles.alloc(obj),
            Err(unsupported) => {
                let what = r.as_ref().map_or(0, |o| s.kind(o));
                let (fault, why) = if unsupported {
                    (Fault::Unsupported, NOT_IMPLEMENTED)
                } else {
                    (Fault::NotFound, "no such object")
                };
                s.fail(
                    fault,
                    format!("vhpi_handle: relation {kind} of kind {what}: {why}"),
                );
                std::ptr::null_mut()
            }
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn vhpi_release_handle(h: vhpiHandleT) -> c_int {
    with_state(|s| {
        let obj = s.handles.get(h).cloned();
        if s.handles.release(h) {
            if let Some(Object::Callback(id)) = obj {
                s.cbs.handle_released(id);
            }
            0
        } else {
            s.fail(
                Fault::Misuse,
                "vhpi_release_handle: invalid or released handle",
            );
            1
        }
    })
}

/// 1 when both handles are live and refer to the same object.
#[unsafe(no_mangle)]
pub extern "C" fn vhpi_compare_handles(a: vhpiHandleT, b: vhpiHandleT) -> c_int {
    with_state(|s| {
        match (
            get_obj(s, a, "vhpi_compare_handles"),
            get_obj(s, b, "vhpi_compare_handles"),
        ) {
            // Iterators are equal only to themselves.
            (Some(Object::Iter(_)), _) | (_, Some(Object::Iter(_))) => c_int::from(a == b),
            (Some(x), Some(y)) => c_int::from(x == y),
            _ => 0,
        }
    })
}

// ---- Properties -------------------------------------------------------

/// A property `obj` does not have: one yavs implements does not apply to this object (only
/// reported through `vhpi_check_error`), or one it does not implement yet.
fn no_property(s: &mut State, func: &str, prop: i32, obj: &Object, implemented: bool) {
    let kind = s.kind(obj);
    if implemented {
        s.fail(
            Fault::NotFound,
            format!("{func}: property {prop} does not apply to kind {kind}"),
        );
    } else {
        s.fail(
            Fault::Unsupported,
            format!("{func}: property {prop} of kind {kind}: {NOT_IMPLEMENTED}"),
        );
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn vhpi_get(prop: vhpiIntPropertyT, h: vhpiHandleT) -> vhpiIntT {
    with_state(|s| {
        let Some(obj) = get_obj(s, h, "vhpi_get") else {
            return vhpiUndefined;
        };
        s.int_property(&obj, prop).unwrap_or_else(|| {
            no_property(s, "vhpi_get", prop, &obj, INT_PROPS.contains(&prop));
            vhpiUndefined
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn vhpi_get_str(prop: vhpiStrPropertyT, h: vhpiHandleT) -> *const vhpiCharT {
    with_state(|s| {
        let Some(obj) = get_obj(s, h, "vhpi_get_str") else {
            return std::ptr::null();
        };
        match s.str_property(&obj, prop) {
            Some(v) => s.intern(v),
            None => {
                no_property(s, "vhpi_get_str", prop, &obj, STR_PROPS.contains(&prop));
                std::ptr::null()
            }
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn vhpi_get_real(_prop: vhpiRealPropertyT, _h: vhpiHandleT) -> vhpiRealT {
    not_implemented("vhpi_get_real");
    0.0
}

#[unsafe(no_mangle)]
pub extern "C" fn vhpi_get_phys(prop: vhpiPhysPropertyT, h: vhpiHandleT) -> vhpiPhysT {
    with_state(|s| {
        if prop == vhpiResolutionLimitP && h.is_null() {
            return phys(s.clock.resolution.fs());
        }
        s.fail(
            Fault::Unsupported,
            format!("vhpi_get_phys: property {prop}: {NOT_IMPLEMENTED}"),
        );
        vhpiPhysT::default()
    })
}

// ---- Iteration ----------------------------------------------------------

fn iterate(s: &mut State, rel: vhpiOneToManyT, reference: vhpiHandleT, func: &str) -> Option<Iter> {
    let r = get_ref(s, reference, func).ok()?;
    match s.iterate(rel, r.as_ref()) {
        Ok(it) => it,
        Err(unsupported) => {
            let what = r.as_ref().map_or(0, |o| s.kind(o));
            let (fault, why) = if unsupported {
                (Fault::Unsupported, NOT_IMPLEMENTED)
            } else {
                (Fault::Misuse, "not an object with sub-elements")
            };
            s.fail(
                fault,
                format!("{func}: relation {rel} of kind {what}: {why}"),
            );
            None
        }
    }
}

/// NULL when there is nothing to iterate.
#[unsafe(no_mangle)]
pub extern "C" fn vhpi_iterator(rel: vhpiOneToManyT, reference: vhpiHandleT) -> vhpiHandleT {
    with_state(|s| match iterate(s, rel, reference, "vhpi_iterator") {
        Some(it) => s.handles.alloc(Object::Iter(it)),
        None => std::ptr::null_mut(),
    })
}

/// The next object, or NULL at the end, which also releases the iterator.
#[unsafe(no_mangle)]
pub extern "C" fn vhpi_scan(iterator: vhpiHandleT) -> vhpiHandleT {
    with_state(|s| {
        let next = match s.handles.get_mut(iterator) {
            Some(Object::Iter(it)) => it.scan(),
            Some(_) => {
                s.fail(Fault::Misuse, "vhpi_scan: not an iterator handle");
                return std::ptr::null_mut();
            }
            None => {
                s.fail(Fault::Misuse, "vhpi_scan: invalid or released handle");
                return std::ptr::null_mut();
            }
        };
        match next {
            Some(obj) => s.handles.alloc(obj),
            None => {
                s.handles.release(iterator);
                std::ptr::null_mut()
            }
        }
    })
}

/// Object `index` (from 0) of the iteration `rel` of `parent`: for `vhpiIndexedNames` the
/// element at that row-major position from the left, for `vhpiSelectedNames` that field.
#[unsafe(no_mangle)]
pub extern "C" fn vhpi_handle_by_index(
    rel: vhpiOneToManyT,
    parent: vhpiHandleT,
    index: i32,
) -> vhpiHandleT {
    with_state(|s| {
        let func = "vhpi_handle_by_index";
        let it = iterate(s, rel, parent, func);
        let found = u64::try_from(index)
            .ok()
            .and_then(|i| it.and_then(|it| it.nth(i)));
        match found {
            Some(obj) => s.handles.alloc(obj),
            None => {
                if s.error.is_none() {
                    s.fail(
                        Fault::NotFound,
                        format!("{func}: no object at index {index}"),
                    );
                }
                std::ptr::null_mut()
            }
        }
    })
}

// ---- Callbacks --------------------------------------------------------

/// Registers a callback for a simulation phase or after a delay (see `callbacks.rs` for
/// the reasons).
///
/// # Safety
/// `cb` is NULL or points to a valid `vhpiCbDataT` whose `time` is NULL or readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vhpi_register_cb(cb: *mut vhpiCbDataT, flags: i32) -> vhpiHandleT {
    // SAFETY: NULL or valid per the contract; the struct is copied.
    let Some(data) = (unsafe { cb.as_ref() }).copied() else {
        with_state(|s| s.fail(Fault::Misuse, "vhpi_register_cb: NULL callback data"));
        return std::ptr::null_mut();
    };
    // SAFETY: NULL or readable per the contract; copied.
    let delay = unsafe { data.time.as_ref() }.copied();
    with_state(|s| {
        if data.cb_rtn.is_none() {
            s.fail(Fault::Misuse, "vhpi_register_cb: NULL cb_rtn");
            return std::ptr::null_mut();
        }
        match s.cbs.register(data, flags, delay, &s.clock) {
            Ok(id) if flags & vhpiReturnCb != 0 => s.handles.alloc(Object::Callback(id)),
            Ok(_) => std::ptr::null_mut(),
            Err(RegisterError::Misuse(why)) => {
                s.fail(Fault::Misuse, format!("vhpi_register_cb: {why}"));
                std::ptr::null_mut()
            }
            Err(RegisterError::Unsupported) => {
                s.fail(
                    Fault::Unsupported,
                    format!(
                        "vhpi_register_cb: reason {}: {NOT_IMPLEMENTED}",
                        data.reason
                    ),
                );
                std::ptr::null_mut()
            }
        }
    })
}

/// Runs `f` on the callback behind `h`. Matured callbacks count: removing one is legal.
fn with_callback(h: vhpiHandleT, func: &str, f: impl FnOnce(&mut State, u64) -> c_int) -> c_int {
    with_state(|s| match get_obj(s, h, func) {
        Some(Object::Callback(id)) => f(s, id),
        Some(_) => {
            s.fail(Fault::Misuse, format!("{func}: not a callback handle"));
            1
        }
        None => 1,
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn vhpi_remove_cb(h: vhpiHandleT) -> c_int {
    with_callback(h, "vhpi_remove_cb", |s, id| {
        s.cbs.remove(id);
        s.handles.release(h);
        0
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn vhpi_disable_cb(h: vhpiHandleT) -> c_int {
    with_callback(h, "vhpi_disable_cb", |s, id| {
        s.cbs.set_enabled(id, false);
        0
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn vhpi_enable_cb(h: vhpiHandleT) -> c_int {
    with_callback(h, "vhpi_enable_cb", |s, id| {
        s.cbs.set_enabled(id, true);
        0
    })
}

/// # Safety
/// `out` is NULL or points to writable `vhpiCbDataT`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vhpi_get_cb_info(h: vhpiHandleT, out: *mut vhpiCbDataT) -> c_int {
    let mut data = None;
    let rc = with_callback(h, "vhpi_get_cb_info", |s, id| {
        data = s.cbs.get(id).map(|cb| cb.data);
        0
    });
    if let Some(d) = data {
        // SAFETY: NULL or writable per the contract.
        match unsafe { out.as_mut() } {
            Some(out) => *out = d,
            None => {
                with_state(|s| s.fail(Fault::Misuse, "vhpi_get_cb_info: NULL cb_data_p"));
                return 1;
            }
        }
    }
    rc
}

/// Calls callbacks `ids` in order, each only if it is still enabled when its turn comes,
/// without holding the state lock. The `time` field points at the current time. Returns
/// how many ran.
pub(crate) fn fire(ids: &[u64]) -> usize {
    let mut ran = 0;
    for &id in ids {
        let Some((mut data, mut time)) =
            with_state(|s| Some((s.cbs.fire(id, s.clock.now)?, s.clock.vhpi_now())))
        else {
            continue;
        };
        data.time = &mut time;
        if let Some(rtn) = data.cb_rtn {
            // SAFETY: the plugin registered `rtn` for exactly this call.
            unsafe { rtn(&data) };
            ran += 1;
        }
    }
    ran
}

// ---- Time, errors, utilities -----------------------------------------

/// # Safety
/// Both pointers are NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vhpi_get_time(time: *mut vhpiTimeT, cycles: *mut c_long) {
    let (now, delta) = with_state(|s| (s.clock.vhpi_now(), s.clock.delta));
    // SAFETY: NULL or writable per the contract.
    unsafe {
        if let Some(t) = time.as_mut() {
            *t = now;
        }
        if let Some(c) = cycles.as_mut() {
            *c = delta as c_long; // at most the delta limit
        }
    }
}

/// Copies the error of the previous VHPI call into `info`. Returns 1 when
/// there was one. The strings stay valid until the next VHPI call.
///
/// # Safety
/// `info` is NULL or points to writable `vhpiErrorInfoT`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vhpi_check_error(info: *mut vhpiErrorInfoT) -> c_int {
    let guard = lock();
    let Some(err) = guard.as_ref().and_then(|s| s.error.as_ref()) else {
        return 0;
    };
    // SAFETY: NULL or writable per the contract.
    if let Some(info) = unsafe { info.as_mut() } {
        *info = vhpiErrorInfoT {
            severity: err.severity,
            message: err.message.as_ptr().cast_mut(),
            str_: std::ptr::null_mut(),
            file: std::ptr::null_mut(),
            line: -1,
        };
    }
    1
}

/// ISO 8859-1 graphic characters are printable.
#[unsafe(no_mangle)]
pub extern "C" fn vhpi_is_printable(ch: c_char) -> c_int {
    with_state(|_| ());
    // c_char is i8 on x86_64 and Windows, u8 on aarch64 Linux.
    #[allow(clippy::unnecessary_cast)]
    let ch = ch as u8;
    c_int::from((0x20..=0x7e).contains(&ch) || ch >= 0xa0)
}

// ---- Not implemented yet (M5) -----------------------------------------
// Present so plugins link and load; each fails cleanly and reports through
// vhpi_check_error.

#[unsafe(no_mangle)]
pub extern "C" fn vhpi_protected_call(
    _var: vhpiHandleT,
    _f: vhpiUserFctT,
    _data: *mut c_void,
) -> c_int {
    not_implemented("vhpi_protected_call");
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn vhpi_get_value(_expr: vhpiHandleT, _value: *mut vhpiValueT) -> c_int {
    not_implemented("vhpi_get_value");
    -1
}

#[unsafe(no_mangle)]
pub extern "C" fn vhpi_put_value(
    _obj: vhpiHandleT,
    _value: *mut vhpiValueT,
    _mode: vhpiPutValueModeT,
) -> c_int {
    not_implemented("vhpi_put_value");
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn vhpi_schedule_transaction(
    _driver: vhpiHandleT,
    _value: *mut vhpiValueT,
    _num_values: u32,
    _delay: *mut vhpiTimeT,
    _mode: vhpiDelayModeT,
    _pulse_rej: *mut vhpiTimeT,
) -> c_int {
    not_implemented("vhpi_schedule_transaction");
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn vhpi_format_value(_input: *const vhpiValueT, _output: *mut vhpiValueT) -> c_int {
    not_implemented("vhpi_format_value");
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn vhpi_get_next_time(_time: *mut vhpiTimeT) -> c_int {
    not_implemented("vhpi_get_next_time");
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn vhpi_create(
    _kind: vhpiClassKindT,
    _h1: vhpiHandleT,
    _h2: vhpiHandleT,
) -> vhpiHandleT {
    not_implemented("vhpi_create");
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn vhpi_register_foreignf(_data: *mut vhpiForeignDataT) -> vhpiHandleT {
    not_implemented("vhpi_register_foreignf");
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn vhpi_get_foreignf_info(_h: vhpiHandleT, _data: *mut vhpiForeignDataT) -> c_int {
    not_implemented("vhpi_get_foreignf_info");
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn vhpi_get_data(_id: i32, _loc: *mut c_void, _n: usize) -> usize {
    not_implemented("vhpi_get_data");
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn vhpi_put_data(_id: i32, _loc: *mut c_void, _n: usize) -> usize {
    not_implemented("vhpi_put_data");
    0
}

include!(concat!(env!("OUT_DIR"), "/exports.rs"));
