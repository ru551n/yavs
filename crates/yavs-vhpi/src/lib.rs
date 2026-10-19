//! IEEE 1076-2019 VHPI for yavs: the C ABI (`vhpi_user.h`), the exported
//! `vhpi_*` symbols and plugin loading.
//!
//! There is no separate VHPI library. The simulator executable exports
//! the symbols listed in `src/exports.txt`: on Linux through a linker
//! version script, on Windows through a `.def` file; no import
//! library is built or shipped. `build.rs` generates
//! those files; see `docs/decisions.md` for the reasoning.
//!
//! The object model behind the ABI is a stub (a design root and the
//! tool) until M5 puts the real one on the kernel. Simulation-phase and
//! after-delay callbacks run from a kernel observer (`VhpiObserver`).
//!
//! Unsafe code: a C ABI cannot be written without it, so this crate does
//! not inherit the workspace `forbid(unsafe_code)` (see Cargo.toml). It is
//! confined to `abi` (the exported functions) and `plugin` (dlopen and
//! calling the plugin's entry points); the other modules hold no unsafe.

pub mod sys;

mod abi;
mod callbacks;
mod model;
mod plugin;
mod sim;

pub use model::Control;
pub use plugin::{ForeignLib, LoadError, PluginSpec, bind_foreign, load, load_foreign};
pub use sim::VhpiObserver;

/// Keeps every exported `vhpi_*` symbol linked into the executable. An
/// executable that exports them calls this once at startup; without a
/// reference the linker drops the symbols before the export list applies.
pub fn keep_exports() {
    std::hint::black_box(abi::exported_symbols());
}

/// Sets the name of the (stub) design root that plugins see.
pub fn set_design_root(name: &str) {
    abi::with_state(|s| s.root_name = name.to_owned());
}

/// Sets the resolution limit (`yavs run --resolution`): VHPI times count in
/// this unit. Femtoseconds until set.
pub fn set_resolution(unit: yavs_kernel::Unit) {
    abi::with_state(|s| s.clock.resolution = unit);
}

/// Runs the enabled callbacks of the phase `reason` (a `vhpiCb*` constant
/// without `Rep`) outside a simulation, and returns how many ran. A
/// simulation uses `VhpiObserver` instead.
pub fn run_callbacks(reason: i32) -> usize {
    abi::fire(&abi::with_state(|s| s.cbs.due(reason)))
}

/// Takes the pending `vhpi_control` request, if a plugin made one.
pub fn take_control() -> Option<Control> {
    abi::with_state(|s| s.control.take())
}
