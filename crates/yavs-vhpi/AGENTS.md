# AGENTS.md: yavs-vhpi

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-vhpi`.
The ABI and export decisions are in `docs/decisions.md` "VHPI ABI and symbol export"; what
the Python testbench framework needs (startup, symbol list, Linux and Windows linking, upstream changes) is in
`crates/yavs-vhpi/COCOTB.md`.

## 1. Purpose and boundaries

- The IEEE 1076-2019 VHPI C ABI: the vendored official `include/vhpi_user.h`, hand-written
  `#[repr(C)]` types and constants (`sys.rs`), the exported `vhpi_*` functions (`abi.rs`), the
  variadic shim (`shim.c`), plugin loading via `vhpi_startup_routines` or `LIB:ENTRY`
  (`plugin.rs`), and export files for the executable (`build.rs`; the `yavs` CLI passes them to its link, #266).
- **The object model is a stub** (a design root, tool info, start/end-of-simulation callbacks)
  until M5 puts the real one on `SimControl` and the event bus (#121).
- Does not: drive simulation itself (only through `SimControl`, plan "Designed for the GUI"),
  or ship a separate VHPI DLL. `yavs run --vhpi` (#267, `crates/yavs/src/elab.rs`) loads plugins
  with `load`, sets the root with `set_design_root`, and a kernel `Observer` there calls
  `run_callbacks` at `Phase::StartOfSim`/`EndOfSim` and turns `take_control` into
  `Hook::end(Finish|Stop(0))`.
- Also loads `--load` libraries and binds VHPIDIRECT symbols (`load_foreign`, `bind_foreign`,
  #95), since it already owns `libloading` and the plugin path rules.
- Depends on: `yavs-diag`, `yavs-ffi`, `libloading`; build: `cc`. Called by: the `yavs` CLI
  (`--load`, `--vhpi`) and its spike binary `src/bin/yavs-vhpi-host.rs`.

## 2. Map

| File | Responsibility |
|---|---|
| `include/vhpi_user.h` | Official IEEE header, vendored unchanged (Apache-2.0) |
| `src/sys.rs` | `#[repr(C)]` types and one constant per enumerator (no bindgen), no unsafe |
| `src/abi.rs` | Exported `extern "C"` VHPI functions, process-wide state behind a mutex (`with_state`, `fire`); unsafe confined here and in `plugin.rs` |
| `src/model.rs` | Safe object model stub, handles, `Control` (`vhpi_control` requests), `W0903`/`W0904` reporting |
| `src/plugin.rs` | `PluginSpec`, `load`, `LoadError` (`E0901` cannot load, `E0902` no entry point); `ForeignLib`, `load_foreign`, `bind_foreign` (`--load`: libraries in order, then the executable; never unloaded) |
| `src/shim.c` | `vhpi_printf`, `vhpi_vprintf`, `vhpi_assert`, `vhpi_control` (C variadics) |
| `src/exports.txt` | The one list of exported symbols |
| `build.rs` | Generates the Linux version script, the Windows `.def`, the test import library `yavs-vhpi-host.lib` (no `yavs.lib`) and a Rust table referencing every symbol; exports `DEP_YAVS_VHPI_VERSION_SCRIPT`, `_DEF`, `_IMPORT_LIB_DIR` |
| `src/lib.rs` | `set_design_root`, `run_callbacks`, `take_control` |
| `tests/vhpi.rs`, `tests/plugin/plugin.c` | ABI vs C header (`abi_matches_c_header`, all ~610 constants, sizes, offsets), C plugin loaded both ways, missing file/entry/symbol errors |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| VHPI is mandatory (Python testbench frameworks) and uses no separate DLL: the executable exports `vhpi_*`; no import library is built or shipped, plugins bring their own (user, 2026-10-18) | Plugins build against the exports themselves | `docs/plan.md` "Decisions" / "Platforms" |
| Header is the official IEEE file, unchanged; Rust side hand-written, checked against it by a test | Names and layouts are fixed by the standard; no libclang on Windows | `docs/decisions.md` "VHPI ABI and symbol export" |
| Linux exports `src/exports.txt` via `-rdynamic` plus `-Wl,--version-script` (cargo-zigbuild rejects `--dynamic-list`; zig exports more than the list); plugins loaded `RTLD_NOW | RTLD_GLOBAL` | Missing symbols fail at load, naming them | same |
| Windows: `.def` only; no `yavs.lib` is built or shipped | Same symbol set as Linux | same; COCOTB.md "Windows" |
| One process-wide state behind a mutex never held while plugin code runs | VHPI has no context argument | same |
| Handles are `(index, generation)` packed into the pointer value; never NULL, never dereferenced; 64-bit only | Rule 9 | `docs/plan.md` rule 9 |
| Disabled callbacks that are never removed must stay cheap | Python testbench frameworks never remove them | rule 9, #123 |
| Records, arrays, generics and generates are native in the object model | Not a Verilog-shaped shim | rule 9 |
| Plugin load failures are hard errors before simulation (`E0901`, `E0902`); misuse `W0903` every time; unimplemented calls `W0904` once per message; failed lookups only via `vhpi_check_error` | Fail early without breaking Python testbench framework probing | `docs/decisions.md` "VHPI ABI" |
| `unsafe` allowed crate-wide in `Cargo.toml` but confined to `abi.rs`, `plugin.rs`, `shim.c` | Root `AGENTS.md` rule 11 | `Cargo.toml`, crate docs |
| Logic values cross the boundary as std_ulogic codes (the `vhpiEnumT` value); BIT/BOOLEAN translated at the boundary | One encoding | `crates/yavs-logic/LAYOUT.md`, `docs/decisions.md` "Interpreter: BIT/BOOLEAN codes" |

Module-local decisions go in this section in the same commit (root `AGENTS.md` rule 24).

Callback reasons map to kernel observer calls (`crates/yavs-kernel/DESIGN.md` "The cycle"; kernel
side #268, the callbacks themselves #272):

| VHPI reason | Kernel |
|---|---|
| `vhpiCbStartOfSimulation`, `vhpiCbEndOfSimulation` | `Phase::StartOfSim`, `Phase::EndOfSim` |
| `vhpiCbNextTimeStep` | `Phase::NextTimeStep` |
| `vhpiCbAfterDelay` | `Hook::wake_at(now + delay)`, then `Observer::wake` with that `WakeId`; removing the callback is `Hook::cancel` |
| `vhpiCbReadWriteSynch` | `Observer::wake` of a `Hook::wake_at(now)`: the next delta at this time, before its update, so `vhpi_put_value` applies in that delta |
| `vhpiCbLastKnownDeltaCycle` | `Phase::LastKnownDelta` (puts queued here create one more delta) |
| `vhpiCbReadOnlySynch`, `vhpiCbEndOfTimeStep` | `Phase::EndOfTimeStep` (after postponed processes; no puts) |
| `vhpiCbValueChange` | `Observer::value_change` |

## 4. Invariants and pitfalls

- Adding an export means editing `src/exports.txt` only; `build.rs` derives every other list.
  A symbol missing from the Rust table is dropped by the linker from the rlib.
- `abi_matches_c_header` needs a C compiler at test time (approved dependency); it cannot run
  under wine.
- The Windows MSVC path (`/DEF:`, `lib.exe`, `cl`) has never run (#124); MinGW was checked under
  wine.
- The CLI must call `keep_exports()` (done first in its `main`); otherwise the linker drops the
  symbols before the export list applies. `crates/yavs/tests/vhpi_exports.rs` checks the
  built binary's `.dynsym` against `vhpi_user.h` (Linux only; no Windows export-table check yet).
- Never hold the state mutex while calling into a plugin (callbacks re-enter the ABI).

## 5. How to work here

```sh
CARGO_BUILD_JOBS=6 cargo test -p yavs-vhpi           # header/ABI check, plugin loading (needs cc)
cargo run -p yavs-vhpi --bin yavs-vhpi-host -- <plugin>[:entry]
```

- Codes owned: `E0900`-`E0999`, `W0900`-`W0999` (E0901, E0902, W0903, W0904 in use).
- Env vars set by `build.rs` for tests: `YAVS_VHPI_TARGET`, `YAVS_VHPI_HOST`,
  `YAVS_VHPI_IMPORT_LIB`; downstream builds read `DEP_YAVS_VHPI_*`.

## 6. Coordination

- The Python testbench framework itself is not installed; installing it needs the user's approval (#122).
- Open issues (snapshot 2026-10-18, `gh issue list -R ru551n/yavs --label crate:vhpi`): #121 real
  object model over `SimControl` and the event bus; #122 Python testbench framework regression (install needs
  approval); #123 disabled callbacks stay cheap; #124 Windows MSVC path never run.
