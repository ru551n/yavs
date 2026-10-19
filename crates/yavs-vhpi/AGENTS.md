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
- **The object model is a stub** (a design root, tool info) until M5 puts the real one on
  `SimControl` and the event bus (#121). Simulation-phase and after-delay callbacks are real
  (#272): `VhpiObserver` (`sim.rs`) is a kernel `Observer`.
- Does not: drive simulation itself (only through `SimControl`, plan "Designed for the GUI"),
  or ship a separate VHPI DLL. `yavs run --vhpi` (#267, `crates/yavs/src/elab.rs`) sets the
  resolution limit (`set_resolution`) and the root (`set_design_root`), loads plugins with
  `load` and subscribes `VhpiObserver`, which runs the callbacks and turns `vhpi_control` into
  `Hook::end(Finish|Stop(0))`.
- Also loads `--load` libraries and binds VHPIDIRECT symbols (`load_foreign`, `bind_foreign`,
  #95), since it already owns `libloading` and the plugin path rules.
- Depends on: `yavs-diag`, `yavs-ffi`, `yavs-kernel` (`Observer`, `Hook`, `Time`), `libloading`; build: `cc`. Called by: the `yavs` CLI
  (`--load`, `--vhpi`) and its spike binary `src/bin/yavs-vhpi-host.rs`.

## 2. Map

| File | Responsibility |
|---|---|
| `include/vhpi_user.h` | Official IEEE header, vendored unchanged (Apache-2.0) |
| `src/sys.rs` | `#[repr(C)]` types and one constant per enumerator (no bindgen), no unsafe |
| `src/abi.rs` | Exported `extern "C"` VHPI functions, process-wide state behind a mutex (`with_state`, `fire`); unsafe confined here and in `plugin.rs` |
| `src/model.rs` | Safe object model stub, handles, `Control` (`vhpi_control` requests), `W0903`/`W0904` reporting |
| `src/callbacks.rs` | Callback registry: registration, enable/disable/remove, per-reason sets of enabled callbacks, after-delay wake-up bookkeeping, `Clock` (time in resolution units) |
| `src/sim.rs` | `VhpiObserver`: phase callbacks at the matching `Phase`, after-delay callbacks on `Observer::wake`, kernel requests (wake-ups, cancels, end) after each batch |
| `src/plugin.rs` | `PluginSpec`, `load`, `LoadError` (`E0901` cannot load, `E0902` no entry point); `ForeignLib`, `load_foreign`, `bind_foreign` (`--load`: libraries in order, then the executable; never unloaded) |
| `src/shim.c` | `vhpi_printf`, `vhpi_vprintf`, `vhpi_assert`, `vhpi_control` (C variadics) |
| `src/exports.txt` | The one list of exported symbols |
| `build.rs` | Generates the Linux version script, the Windows `.def`, the test import library `yavs-vhpi-host.lib` (no `yavs.lib`) and a Rust table referencing every symbol; exports `DEP_YAVS_VHPI_VERSION_SCRIPT`, `_DEF`, `_IMPORT_LIB_DIR` |
| `src/lib.rs` | `set_design_root`, `set_resolution`, `run_callbacks` (outside a simulation, the spike host), `take_control` |
| `tests/vhpi.rs`, `tests/plugin/plugin.c` | ABI vs C header (`abi_matches_c_header`, all ~610 constants, sizes, offsets), C plugin loaded both ways, missing file/entry/symbol errors |
| `tests/plugin/sched.c` | Callback scenarios (one entry each) run by `crates/yavs/tests/vhpi_run.rs` through `yavs run --vhpi` |

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
side #268, the callbacks #272). Each `Rep` reason maps like its one-shot form and stays until
removed. There is no read-write or read-only synch reason in IEEE VHPI (`vhpi_user.h`): Python
testbench frameworks use `vhpiCbRepLastKnownDeltaCycle` for their read-write phase and
`vhpiCbRepEndOfTimeStep` for their read-only phase, as the reference simulator implements them.

| VHPI reason | Kernel |
|---|---|
| `vhpiCbStartOfSimulation`, `vhpiCbStartOfInitialization` | `Phase::StartOfSim`, in that order |
| `vhpiCbEndOfInitialization` | `Phase::EndOfInit` (every process ran once, before the first cycle) |
| `vhpiCbEndOfSimulation` | `Phase::EndOfSim` |
| `vhpiCbNextTimeStep` | `Phase::NextTimeStep` |
| `vhpiCbStartOfNextCycle` | `Phase::NextCycle` (every cycle, before its wake-ups and update) |
| `vhpiCbAfterDelay` | `Hook::wake_at(now + delay)`, then `Observer::wake` with that `WakeId`; removing the callback is `Hook::cancel`. A delay of 0 is the next delta. A disabled one is skipped when due (a one-shot then matures) |
| `vhpiCbLastKnownDeltaCycle` (read-write) | `Phase::LastKnownDelta` (puts queued here create one more delta) |
| `vhpiCbEndOfTimeStep` (read-only) | `Phase::EndOfTimeStep` (after postponed processes; no puts) |
| `vhpiCbValueChange` (#269, not yet) | `Observer::value_change`; register it in `callbacks.rs` keyed by signal, dispatch in `sim.rs` |

Callback rules (#272, #123):

- Only enabled phase callbacks are in the per-reason sets that a phase looks at, so disabled
  callbacks cost nothing per cycle, however many there are (100000 disabled ones: no measurable
  change over 200000 cycles). Disable and enable move a callback out of and into its set.
- A phase runs a snapshot of its set in registration order: a callback registered from inside a
  callback for the same phase fires at the next occurrence, not the current one; one disabled
  or removed by an earlier callback of the batch does not run.
- A one-shot callback matures when it runs; it is dropped then unless a `vhpiReturnCb` handle
  refers to it, and dropped when that handle is released. Removing a matured callback is legal
  (returns 0).
- VHPI times are in units of the resolution limit (`set_resolution`, from `--resolution`);
  `vhpi_get_phys(vhpiResolutionLimitP)` returns it, `vhpi_get_time` gives the delta as `cycles`,
  and the `time` field passed to every callback points at the current time.
- Registering or removing an after-delay callback outside an observer call (at load time)
  queues the kernel request until the next observer call (`StartOfSim`).

## 4. Invariants and pitfalls

- Adding an export means editing `src/exports.txt` only; `build.rs` derives every other list.
  A symbol missing from the Rust table is dropped by the linker from the rlib.
- `abi_matches_c_header` needs a C compiler at test time (approved dependency); it cannot run
  under wine.
- The Windows MSVC path (`/DEF:`, `lib.exe`, `cl`) runs in the `windows-latest` CI job:
  `crates/yavs/tests/vhpi_run.rs` builds a throwaway `yavs.lib` from the `.def` (`/NAME:yavs.exe`),
  links the plugin DLL against it and loads it with `yavs run --vhpi` (#124). MinGW was checked under
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
  approval); #269 to #271 object model, values, puts (value-change
  callbacks of #272 wait for #269). #123 (disabled callbacks stay cheap) is done.
