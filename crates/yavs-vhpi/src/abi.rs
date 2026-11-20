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

use std::cell::Cell;
use std::ffi::{CStr, c_char, c_int, c_long, c_void};
use std::io::Write;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::callbacks::{Callback, RegisterError};
use crate::model::{Control, Fault, INT_PROPS, Iter, Object, Reset, STR_PROPS, State};
use crate::sys::*;
use crate::value::{self, Encoded, Given, Member};
use yavs_kernel::Hook;

static STATE: Mutex<Option<State>> = Mutex::new(None);

thread_local! {
    /// The kernel hook of the observer call that is running plugin callbacks on this thread.
    static HOOK: Cell<Option<NonNull<Hook<'static>>>> = const { Cell::new(None) };
}

/// Runs `f` (which calls plugins) with `hook` available to the ABI through [`with_hook`].
pub(crate) fn hooked<R>(hook: &mut Hook<'_>, f: impl FnOnce() -> R) -> R {
    let p = NonNull::from(hook).cast::<Hook<'static>>();
    let prev = HOOK.with(|c| c.replace(Some(p)));
    let r = f();
    HOOK.with(|c| c.set(prev));
    r
}

/// Calls `f` with the hook of the running observer call; `None` outside one.
pub(crate) fn with_hook<R>(f: impl FnOnce(&mut Hook<'_>) -> R) -> Option<R> {
    let p = HOOK.with(Cell::get)?;
    // SAFETY: `hooked` set the pointer from a `&mut Hook` that outlives its call, and the
    // caller does not use that borrow until `hooked` returns; the lifetime is erased only
    // for the duration of the call, and calls on this thread do not overlap.
    Some(f(unsafe { &mut *p.cast::<Hook<'_>>().as_ptr() }))
}

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
    let r = f(state);
    drop(guard);
    if PLI_ERROR_DUE.load(Ordering::Relaxed) {
        pli_error();
    }
    r
}

/// An error was recorded and a `vhpiCbPLIError` callback is enabled. Plugin code cannot run
/// under the lock, so `with_state` runs them as it leaves (#832).
pub(crate) static PLI_ERROR_DUE: AtomicBool = AtomicBool::new(false);

/// Runs the `vhpiCbPLIError` callbacks: they read the error with `vhpi_check_error`, as does
/// the plugin code whose call failed afterwards.
fn pli_error() {
    PLI_ERROR_DUE.store(false, Ordering::Relaxed);
    let (ids, error) = {
        let mut guard = lock();
        let s = guard.get_or_insert_with(State::default);
        s.in_pli_error = true;
        (s.cbs.due(vhpiCbPLIError), s.error.clone())
    };
    for id in ids {
        let data = {
            let mut guard = lock();
            let s = guard.get_or_insert_with(State::default);
            s.error.clone_from(&error);
            let now = s.clock.now;
            s.cbs.fire(id, now)
        };
        if let Some(rtn) = data.and_then(|d| d.cb_rtn.map(|r| (r, d))) {
            // SAFETY: the plugin registered `rtn` for exactly this call.
            unsafe { (rtn.0)(&rtn.1) };
        }
    }
    let mut guard = lock();
    let s = guard.get_or_insert_with(State::default);
    s.in_pli_error = false;
    s.error = error;
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
    let text = plugin_text(bytes);
    let mut out = std::io::stdout().lock();
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// A plugin's text for the terminal, in UTF-8 as VHDL reports are (#984). Strings of VHPI values are
/// ISO-8859-1, so text that is not UTF-8 is that; a C plugin's UTF-8 text stays as written.
fn plugin_text(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.into(),
        Err(_) => bytes
            .iter()
            .map(|&b| char::from(b))
            .collect::<String>()
            .into(),
    }
}

/// `vhpi_assert` after formatting: prints like a VHDL report.
///
/// # Safety
/// `msg` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yavs_vhpi_assert(severity: c_int, msg: *const c_char) -> c_int {
    // SAFETY: the shim passes the string it just formatted.
    let msg = unsafe { CStr::from_ptr(msg) };
    let msg = plugin_text(msg.to_bytes());
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
            vhpiReset => {
                if let Some(why) = s.reset_refused.clone() {
                    s.fail(
                        Fault::Unsupported,
                        format!("vhpi_control(vhpiReset): {why}"),
                    );
                    return 1;
                }
                if !matches!(
                    s.tool_phase.0,
                    vhpiInitializationPhase | vhpiSimulationPhase
                ) || s.reset != Reset::None
                {
                    s.fail(
                        Fault::Misuse,
                        "vhpi_control(vhpiReset): only while a simulation runs",
                    );
                    return 1;
                }
                Control::Reset
            }
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

fn phys_of(v: i64) -> vhpiPhysT {
    vhpiPhysT {
        high: (v >> 32) as i32,
        low: v as u32,
    }
}

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

/// `vhpiRandomSeedP` of the tool. Never a made-up or truncated number: a plugin could replay it.
fn random_seed(s: &mut State) -> vhpiIntT {
    let why = match s.shuffle_seed {
        None => "this run has no seed, it runs without --shuffle".to_string(),
        Some(seed) => match vhpiIntT::try_from(seed) {
            Ok(seed) => return seed,
            Err(_) => format!(
                "--shuffle seed {seed} does not fit vhpiIntT; a seed up to {} does",
                vhpiIntT::MAX
            ),
        },
    };
    s.fail(Fault::NotFound, format!("vhpi_get: vhpiRandomSeedP: {why}"));
    vhpiUndefined
}

#[unsafe(no_mangle)]
pub extern "C" fn vhpi_get(prop: vhpiIntPropertyT, h: vhpiHandleT) -> vhpiIntT {
    with_state(|s| {
        // The tool's phase needs no handle (one reference takes only NULL, #832).
        if h.is_null() && prop == vhpiPhaseP {
            return s.tool_phase.0;
        }
        if h.is_null() && prop == vhpiRandomSeedP {
            return random_seed(s);
        }
        let Some(obj) = get_obj(s, h, "vhpi_get") else {
            return vhpiUndefined;
        };
        if prop == vhpiRandomSeedP && matches!(obj, Object::Tool) {
            return random_seed(s);
        }
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

/// Floating bounds of a range constraint.
#[unsafe(no_mangle)]
pub extern "C" fn vhpi_get_real(prop: vhpiRealPropertyT, h: vhpiHandleT) -> vhpiRealT {
    with_state(|s| {
        let Some(obj) = get_obj(s, h, "vhpi_get_real") else {
            return 0.0;
        };
        match &obj {
            Object::Type(o) => s.design.types.real_property(*o, prop),
            _ => None,
        }
        .unwrap_or_else(|| {
            let known = matches!(prop, vhpiFloatLeftBoundP | vhpiFloatRightBoundP);
            no_property(s, "vhpi_get_real", prop, &obj, known);
            0.0
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn vhpi_get_phys(prop: vhpiPhysPropertyT, h: vhpiHandleT) -> vhpiPhysT {
    with_state(|s| {
        if prop == vhpiResolutionLimitP
            && (h.is_null() || matches!(s.handles.get(h), Some(Object::Tool)))
        {
            return phys(s.clock.resolution.fs());
        }
        if prop == vhpiTimeP
            && let Some(Object::Transaction(t)) = s.handles.get(h)
        {
            let t = t.time;
            let v = s.clock.vhpi_time(t);
            return vhpiPhysT {
                high: v.high,
                low: v.low,
            };
        }
        let known = matches!(
            prop,
            vhpiPhysLeftBoundP | vhpiPhysRightBoundP | vhpiPhysPositionP
        );
        if known && let Some(obj) = get_obj(s, h, "vhpi_get_phys") {
            if let Object::Type(o) = obj
                && let Some(v) = s.design.types.phys_property(o, prop)
            {
                return phys_of(v);
            }
            no_property(s, "vhpi_get_phys", prop, &obj, true);
            return vhpiPhysT::default();
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
        let watch = if matches!(
            data.reason,
            vhpiCbValueChange | vhpiCbSensitivity | vhpiCbForce | vhpiCbRelease | vhpiCbTransaction
        ) {
            match s.handles.get(data.obj).cloned() {
                Some(o @ Object::NetDriver(_)) if data.reason == vhpiCbTransaction => {
                    Some((o, Box::default()))
                }
                Some(o) => s.watch(&o).map(|w| (o, w)),
                None => None,
            }
        } else {
            None
        };
        match s.cbs.register(data, flags, delay, watch, &s.clock) {
            Ok(id) => {
                // `data.obj` in the callback is the callback's own handle: a plugin may release
                // the one it passed as soon as this returns (#273).
                if let Some(o) = s.cbs.get(id).and_then(|cb| cb.obj.clone()) {
                    let own = s.handles.alloc(o);
                    s.cbs.set_trigger(id, own);
                }
                if flags & vhpiReturnCb != 0 {
                    s.handles.alloc(Object::Callback(id))
                } else {
                    std::ptr::null_mut()
                }
            }
            Err(RegisterError::Misuse(why)) => {
                s.fail(Fault::Misuse, format!("vhpi_register_cb: {why}"));
                std::ptr::null_mut()
            }
            Err(RegisterError::Unsupported) => {
                s.fail(
                    Fault::Unsupported,
                    format!(
                        "vhpi_register_cb: {}: {NOT_IMPLEMENTED}",
                        crate::callbacks::refused(data.reason)
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
        if let Some(own) = s.cbs.remove(id) {
            s.handles.release(own);
        }
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
        let Some((mut data, mut time)) = with_state(|s| {
            let data = s.cbs.fire(id, s.clock.now)?;
            // A value-change callback: the object's value in the format the plugin asked for.
            let obj = s.cbs.get(id).and_then(|cb| cb.obj.clone());
            // SAFETY: the plugin registered `value` as NULL or a writable `vhpiValueT` (with
            // its buffer) that lives as long as the callback.
            if let (Some(obj), Some(v)) = (obj, unsafe { data.value.as_mut() }) {
                get_value(s, &obj, v);
            }
            Some((data, s.clock.vhpi_now()))
        }) else {
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

/// Calls `f(var, data)` for a variable of a protected type and returns what it returns. Plugin
/// code never runs alongside a process that holds the object: observers run between phases,
/// foreign calls one at a time, and a process touching a protected object is serial.
///
/// # Safety
/// `f` is NULL or a function taking a handle and a pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vhpi_protected_call(
    var: vhpiHandleT,
    f: vhpiUserFctT,
    data: *mut c_void,
) -> c_int {
    let func = "vhpi_protected_call";
    let ok = with_state(|s| {
        let protected = match get_obj(s, var, func) {
            Some(Object::Node(n)) => s.design.protected.contains(&n),
            Some(_) => false,
            None => return false,
        };
        if !protected {
            s.fail(
                Fault::Misuse,
                format!("{func}: the handle is not a variable of a protected type"),
            );
        } else if f.is_none() {
            s.fail(Fault::Misuse, format!("{func}: NULL function"));
        }
        protected && f.is_some()
    });
    let Some(f) = f.filter(|_| ok) else {
        return 1;
    };
    // SAFETY: the standard's `userFct` takes the handle and the data; `vhpi_user.h` declares it
    // without a prototype, and no state lock is held across the call.
    unsafe {
        let f: unsafe extern "C" fn(vhpiHandleT, *mut c_void) -> c_int = std::mem::transmute(f);
        f(var, data)
    }
}

/// Reads the value of `expr` in `value.format` (`vhpiObjTypeVal`: the object's own format,
/// written back). Returns 0, the buffer size in bytes the value needs when `value.bufSize` is
/// smaller (or `value.value.ptr` is NULL), or -1 on error.
///
/// # Safety
/// `value` is NULL or points to a writable `vhpiValueT` whose buffer pointer, when the format
/// needs one, has `bufSize` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vhpi_get_value(expr: vhpiHandleT, value: *mut vhpiValueT) -> c_int {
    // SAFETY: NULL or writable per the contract.
    let Some(v) = (unsafe { value.as_mut() }) else {
        with_state(|s| s.fail(Fault::Misuse, "vhpi_get_value: NULL value_p"));
        return -1;
    };
    with_state(|s| match get_obj(s, expr, "vhpi_get_value") {
        Some(obj) => get_value(s, &obj, v),
        None => -1,
    })
}

/// `vhpi_get_value` of `obj` into `v`: 0, -1 on error, or the buffer size needed.
fn get_value(s: &mut State, obj: &Object, v: &mut vhpiValueT) -> c_int {
    // An access parameter of a foreign call or an access variable: the designated object's
    // data, or NULL.
    let access = match s.access_var(obj) {
        Some(Ok(p)) => Some(p),
        Some(Err((fault, m))) => {
            s.fail(fault, m);
            return -1;
        }
        None => s.foreign_access(obj),
    };
    if let Some(p) = access {
        if !matches!(v.format, vhpiPtrVal | vhpiObjTypeVal) {
            s.fail(
                Fault::Misuse,
                "vhpi_get_value: an access value has only a vhpiPtrVal value",
            );
            return -1;
        }
        v.format = vhpiPtrVal;
        v.numElems = 1;
        v.value.ptr = p as usize as *mut std::ffi::c_void;
        return 0;
    }
    // A composite has the one format that holds its elements; any scalar or array has it too.
    // Only when asked for: a client of `vhpiObjTypeVal` cannot decode a tool-defined layout (#273).
    if matches!(obj, Object::Node(_) | Object::Elem { .. }) && v.format == vhpiRawDataVal {
        let res = s.clock.resolution.fs();
        let (mut bytes, mut n) = (Vec::new(), 0);
        if let Err((fault, m)) = s.raw_read(obj, res, &mut bytes, &mut n) {
            s.fail(fault, m);
            return -1;
        }
        v.format = vhpiRawDataVal;
        return put_encoded(v, Encoded::Buffer { bytes, n });
    }
    let read = match s.read(obj, |r| with_hook(|h| h.value(r))) {
        Ok(r) => r,
        Err((fault, m)) => {
            s.fail(fault, m);
            return -1;
        }
    };
    if v.format == vhpiObjTypeVal {
        v.format = read.natural();
    }
    let encoded = match read.encode(v.format, s.clock.resolution.fs()) {
        Ok(e) => e,
        Err(value::Error::Unsupported(m)) => {
            s.fail(Fault::Unsupported, format!("{m}: {NOT_IMPLEMENTED}"));
            return -1;
        }
        Err(value::Error::Mismatch(m)) => {
            s.fail(Fault::Misuse, format!("vhpi_get_value: {m}"));
            return -1;
        }
    };
    put_encoded(v, encoded)
}

/// Writes `encoded` into the plugin's `v`: 0, or the buffer size needed when its buffer is
/// NULL or too small (`numElems` is set either way).
fn put_encoded(v: &mut vhpiValueT, encoded: Encoded) -> c_int {
    match encoded {
        Encoded::Scalar(u) => {
            v.value = u;
            v.numElems = 1;
            0
        }
        Encoded::Buffer { bytes, n } => {
            v.numElems = i32::try_from(n).unwrap_or(i32::MAX);
            // SAFETY: reading the pointer member of the union; every member is plain data.
            let ptr = unsafe { v.value.ptr };
            if ptr.is_null() || v.bufSize < bytes.len() {
                return c_int::try_from(bytes.len()).unwrap_or(c_int::MAX);
            }
            // SAFETY: the plugin's buffer has `bufSize` writable bytes.
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.cast::<u8>(), bytes.len()) };
            0
        }
    }
}

/// Writes a signal, port or sub-element (#271): deposits and forces through the kernel,
/// applied when the calling callback returns (see `crates/yavs-vhpi/AGENTS.md`).
///
/// # Safety
/// `value` is NULL or points to a `vhpiValueT` whose buffer (for string and vector formats)
/// is readable: a NUL-terminated string, or `numElems` elements.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vhpi_put_value(
    obj: vhpiHandleT,
    value: *mut vhpiValueT,
    mode: vhpiPutValueModeT,
) -> c_int {
    // SAFETY: NULL or readable per the contract.
    let v = unsafe { value.as_ref() };
    with_state(|s| {
        let Some(o) = get_obj(s, obj, "vhpi_put_value") else {
            return 1;
        };
        let res = s.clock.resolution.fs();
        // SAFETY: the contract above.
        let decode = |ty: &value::Scalar, n| unsafe { decode(v, ty, n, res, "vhpi_put_value") };
        let put = if matches!(o, Object::Call(_) | Object::Param { .. }) {
            s.foreign_put(&o, mode, v.map(|v| v.numElems), decode)
        } else if let Some(v) = v.filter(|v| v.format == vhpiRawDataVal) {
            // SAFETY: the plugin's buffer has `bufSize` readable bytes.
            let bytes = unsafe {
                let p = v.value.ptr;
                if p.is_null() {
                    &[][..]
                } else {
                    std::slice::from_raw_parts(p.cast::<u8>(), v.bufSize)
                }
            };
            s.put_raw(&o, mode, bytes)
        } else {
            s.put(&o, mode, decode)
        };
        match put {
            Ok(()) => 0,
            Err((fault, m)) => {
                let m = if fault == Fault::Unsupported {
                    format!("{m}: {NOT_IMPLEMENTED}")
                } else {
                    m
                };
                s.fail(fault, m);
                1
            }
        }
    })
}

/// Decodes the plugin's value `v` into `n` elements (`None`: a scalar) of type `ty`.
///
/// # Safety
/// `v` is NULL or its buffer is readable as `vhpi_put_value` says.
unsafe fn decode(
    v: Option<&vhpiValueT>,
    ty: &value::Scalar,
    n: Option<usize>,
    res: u64,
    func: &str,
) -> Result<value::Read, value::Error> {
    let Some(v) = v else {
        return Err(value::Error::Mismatch("NULL value_p".into()));
    };
    let mut format = v.format;
    if format == vhpiObjTypeVal {
        format = value::natural_format(ty, n.is_some());
    }
    // SAFETY: the plugin's value, read as its format says (the contract above).
    let given = unsafe { given(v, format) }.ok_or_else(|| {
        value::Error::Unsupported(format!("{func}: format {}", value::format_name(format)))
    })?;
    value::Read::decode(ty.clone(), n, format, given, res)
}

/// The value a plugin passes in `format`; `None` for a format `vhpi_put_value` does not read.
///
/// # Safety
/// The buffer of a string or vector format is readable as `vhpi_put_value` says.
unsafe fn given(v: &vhpiValueT, format: vhpiFormatT) -> Option<Given<'_>> {
    if value::is_text(format) {
        // SAFETY: reading the pointer member; the plugin passes a NUL-terminated string.
        let p = unsafe { v.value.str_ };
        if p.is_null() {
            return Some(Given::Text(&[]));
        }
        // SAFETY: as above.
        return Some(Given::Text(unsafe { CStr::from_ptr(p.cast()) }.to_bytes()));
    }
    if let Some(size) = value::vector_elem_size(format) {
        // SAFETY: reading the pointer member; the buffer holds `numElems` elements.
        let p = unsafe { v.value.ptr };
        let n = usize::try_from(v.numElems).ok()?;
        if p.is_null() || n == 0 {
            return Some(Given::Vector(&[]));
        }
        // SAFETY: as above.
        return Some(Given::Vector(unsafe {
            std::slice::from_raw_parts(p.cast::<u8>(), n * size)
        }));
    }
    scalar_member(format, v.value).map(Given::Scalar)
}

/// The union member scalar format `format` names.
fn scalar_member(format: vhpiFormatT, u: vhpiValueU) -> Option<Member> {
    // SAFETY: every union member is plain data; the format names the one the plugin set.
    let m = unsafe {
        match format {
            vhpiEnumVal | vhpiLogicVal => Member::Int(i64::from(u.enumv)),
            vhpiSmallEnumVal => Member::Int(i64::from(u.smallenumv)),
            vhpiIntVal => Member::Int(i64::from(u.intg)),
            vhpiLongIntVal => Member::Int(u.longintg),
            vhpiRealVal => Member::Real(u.real),
            vhpiCharVal => Member::Char(u.ch),
            vhpiTimeVal => Member::Time((i64::from(u.time.high) << 32) | i64::from(u.time.low)),
            vhpiPhysVal => Member::Phys((i64::from(u.phys.high) << 32) | i64::from(u.phys.low)),
            vhpiSmallPhysVal => Member::Phys(i64::from(u.smallphys)),
            _ => return None,
        }
    };
    Some(m)
}

/// Schedules a transaction on a driver: `value` (one value, `num_values` 1) after `delay`
/// (resolution units) with `mode`, an inertial one rejecting pulses shorter than
/// `pulse_rej` (NULL: the delay). 0, or 1 on error.
///
/// # Safety
/// `value` is NULL or readable as for `vhpi_put_value`; `delay` and `pulse_rej` are NULL or
/// readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vhpi_schedule_transaction(
    driver: vhpiHandleT,
    value: *mut vhpiValueT,
    num_values: u32,
    delay: *mut vhpiTimeT,
    mode: vhpiDelayModeT,
    pulse_rej: *mut vhpiTimeT,
) -> c_int {
    let func = "vhpi_schedule_transaction";
    // SAFETY: NULL or readable per the contract.
    let (v, delay, rej) = unsafe { (value.as_ref(), delay.as_ref(), pulse_rej.as_ref()) };
    with_state(|s| {
        let Some(o) = get_obj(s, driver, func) else {
            return 1;
        };
        let res = s.clock.resolution.fs();
        let time = |t: &vhpiTimeT| {
            let units = (i64::from(t.high) << 32) | i64::from(t.low);
            u64::try_from(units)
                .ok()
                .and_then(|u| u.checked_mul(res))
                .and_then(yavs_kernel::Time::fs)
                .ok_or_else(|| format!("{func}: a delay of {units} is negative or overflows"))
        };
        let parsed = (|| {
            if num_values != 1 {
                return Err(format!("{func}: numValues is {num_values}, not 1"));
            }
            let d = time(delay.ok_or_else(|| format!("{func}: NULL delay"))?)?;
            let m = match mode {
                vhpiTransport => yavs_kernel::DelayMode::Transport,
                vhpiInertial => yavs_kernel::DelayMode::Inertial {
                    reject: rej.map(time).transpose()?,
                },
                _ => return Err(format!("{func}: bad delay mode {mode}")),
            };
            Ok((d, m))
        })();
        let (d, m) = match parsed {
            Ok(x) => x,
            Err(e) => {
                s.fail(Fault::Misuse, e);
                return 1;
            }
        };
        // SAFETY: the contract above.
        let decode = |ty: &value::Scalar, n| unsafe { decode(v, ty, n, res, func) };
        match s.foreign_schedule(&o, d, m, decode) {
            Ok(()) => 0,
            Err((fault, msg)) => {
                let msg = if fault == Fault::Unsupported {
                    format!("{msg}: {NOT_IMPLEMENTED}")
                } else {
                    msg
                };
                s.fail(fault, msg);
                1
            }
        }
    })
}

/// Converts `input` into the format `output` names, as a value of the type `input`'s format
/// stands for (`value::format_type`): 0, -1 on error, or the buffer size needed.
///
/// # Safety
/// `input` and `output` are NULL or valid; `input`'s buffer is readable as for
/// `vhpi_put_value`, `output`'s writable as for `vhpi_get_value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vhpi_format_value(
    input: *const vhpiValueT,
    output: *mut vhpiValueT,
) -> c_int {
    // SAFETY: NULL or valid per the contract.
    let (Some(i), Some(o)) = (unsafe { input.as_ref() }, unsafe { output.as_mut() }) else {
        with_state(|s| s.fail(Fault::Misuse, "vhpi_format_value: NULL value"));
        return -1;
    };
    with_state(|s| {
        let res = s.clock.resolution.fs();
        let name = value::format_name(i.format);
        // SAFETY: the contract above.
        let converted = unsafe { given(i, i.format) }
            .and_then(|g| Some((value::format_type(i.format, &g)?, g)))
            .ok_or_else(|| value::Error::Unsupported(format!("vhpi_format_value: from {name}")))
            .and_then(|((ty, n), g)| value::Read::decode(ty, n, i.format, g, res))
            .and_then(|r| {
                let f = if o.format == vhpiObjTypeVal {
                    r.natural()
                } else {
                    o.format
                };
                o.format = f;
                r.convert_for(f)?.encode(f, res)
            });
        match converted {
            Ok(e) => put_encoded(o, e),
            Err(value::Error::Unsupported(m)) => {
                s.fail(Fault::Unsupported, format!("{m}: {NOT_IMPLEMENTED}"));
                -1
            }
            Err(value::Error::Mismatch(m)) => {
                s.fail(Fault::Misuse, format!("vhpi_format_value: {m}"));
                -1
            }
        }
    })
}

/// The time of the next scheduled activity, in units of the resolution limit, into `time`:
/// 0, or `vhpiNoActivity` when nothing is scheduled. The kernel knows it in the
/// end-of-time-step phase only; elsewhere 1 and `W0903` (#828).
///
/// # Safety
/// `time` is NULL or points to a writable `vhpiTimeT`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vhpi_get_next_time(time: *mut vhpiTimeT) -> c_int {
    let next = with_hook(|h| h.next_time()).flatten();
    with_state(|s| {
        let Some(next) = next else {
            s.fail(
                Fault::Misuse,
                "vhpi_get_next_time: the next time is known in the end-of-time-step phase (vhpiCbEndOfTimeStep)",
            );
            return 1;
        };
        // After-delay callbacks registered in this call reach the kernel after it returns.
        let pending = s
            .cbs
            .to_wake
            .iter()
            .filter_map(|&id| s.cbs.due_at(id))
            .min();
        let Some(t) = next.into_iter().chain(pending).min() else {
            return vhpiNoActivity;
        };
        // SAFETY: NULL or writable per the contract.
        if let Some(p) = unsafe { time.as_mut() } {
            *p = s.clock.vhpi_time(t);
        }
        0
    })
}

/// Creates a process (`vhpiProcessStmtK`) or a driver (`vhpiDriverK`) of a foreign
/// architecture's model while it is elaborated (see `foreign.rs`); NULL on error.
#[unsafe(no_mangle)]
pub extern "C" fn vhpi_create(
    kind: vhpiClassKindT,
    h1: vhpiHandleT,
    h2: vhpiHandleT,
) -> vhpiHandleT {
    let func = "vhpi_create";
    with_state(|s| {
        let mut obj = |h: vhpiHandleT| {
            if h.is_null() {
                return Ok(None);
            }
            get_obj(s, h, func).map(Some).ok_or(())
        };
        let (Ok(a), Ok(b)) = (obj(h1), obj(h2)) else {
            return std::ptr::null_mut();
        };
        match s.create(kind, a, b) {
            Ok(o) => s.handles.alloc(o),
            Err((fault, m)) => {
                let m = if fault == Fault::Unsupported {
                    format!("{m}: {NOT_IMPLEMENTED}")
                } else {
                    m
                };
                s.fail(fault, m);
                std::ptr::null_mut()
            }
        }
    })
}

/// Registers a foreign model (`vhpiFuncF`, `vhpiProcF`, `vhpiArchF`, `vhpiLibF`, `vhpiAppF`)
/// and returns its `vhpiForeignfK` handle, NULL on error. See `foreign.rs`.
///
/// # Safety
/// `data` is NULL or points to a `vhpiForeignDataT` whose names are NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vhpi_register_foreignf(data: *mut vhpiForeignDataT) -> vhpiHandleT {
    // SAFETY: NULL or valid per the contract; the struct is copied.
    let Some(d) = (unsafe { data.as_ref() }).copied() else {
        with_state(|s| s.fail(Fault::Misuse, "vhpi_register_foreignf: NULL data"));
        return std::ptr::null_mut();
    };
    with_state(|s| match s.register_foreign(&d) {
        Ok(i) => s.handles.alloc(Object::Foreign(i)),
        Err(m) => {
            s.fail(Fault::Misuse, format!("vhpi_register_foreignf: {m}"));
            std::ptr::null_mut()
        }
    })
}

/// Fills `data` from the registration behind `h`. Returns 0, or 1 on error.
///
/// # Safety
/// `data` is NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vhpi_get_foreignf_info(
    h: vhpiHandleT,
    data: *mut vhpiForeignDataT,
) -> c_int {
    // SAFETY: NULL or writable per the contract.
    let Some(out) = (unsafe { data.as_mut() }) else {
        with_state(|s| s.fail(Fault::Misuse, "vhpi_get_foreignf_info: NULL data"));
        return 1;
    };
    with_state(|s| {
        // A registration, or an instance of a foreign architecture (its model's registration).
        let i = match get_obj(s, h, "vhpi_get_foreignf_info") {
            Some(Object::Foreign(i)) => Some(i),
            Some(Object::Node(n)) => s.foreign_arch(n),
            Some(_) => None,
            None => return 1,
        };
        match i.and_then(|i| s.foreign_info(i, out)) {
            Some(()) => 0,
            None => {
                s.fail(
                    Fault::Misuse,
                    "vhpi_get_foreignf_info: not a foreign registration or foreign architecture",
                );
                1
            }
        }
    })
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
