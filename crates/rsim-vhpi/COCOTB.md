# What cocotb needs from rsim

Read from cocotb `master` at `ae55ced` (2026-10-15), shallow clone in
`~/.cache/rsim-vhpi/cocotb`. Paths below are relative to the cocotb repo.

## How cocotb starts under a VHPI simulator

1. The simulator loads `libcocotb_bootstrap.so` (`cocotb_bootstrap.dll` on
   Windows, from `cocotb-config --lib-entry vhpi <sim>`). That library exports
   `vhpi_startup_routines[] = { cocotb_bootstrap_entry, NULL }`
   (`src/cocotb/share/lib/gpi/bootstrapper.cpp`), so a simulator that calls
   `vhpi_startup_routines` needs no entry name. Only Riviera/Active-HDL get
   `lib:cocotb_bootstrap_entry` (see `lib_entry` in `src/cocotb_tools/config.py`).
2. `cocotb_bootstrap_entry` reads `COCOTB_BOOTSTRAP` (entries separated by `:`
   on Linux and `;` on Windows, each `library[,function]`) and loads libgpi,
   libpython and the PyGPI module in that order. The makefiles and the Python
   runner set it.
3. libgpi loads the implementations listed in `GPI_IMPL`
   (`path/libgpi_impl_<sim>_vhpi.so:gpi_impl_<sim>_vhpi_entry`) with
   `dlopen(RTLD_LAZY | RTLD_GLOBAL)` (`src/cocotb/share/lib/gpi/dynload.cpp`).
4. The VHPI implementation (`src/cocotb/share/lib/gpi/vhpi/*.cpp`) calls
   `vhpi_*` directly. It includes cocotb's vendored IEEE header
   (`src/cocotb/_vendor/vhpi/vhpi_user.h`) plus `vhpi_user_ext.h`, never the
   simulator's header.

## Symbols cocotb's VHPI layer uses

`vhpi_check_error`, `vhpi_control`, `vhpi_get`, `vhpi_get_phys`,
`vhpi_get_str`, `vhpi_get_time`, `vhpi_get_value`, `vhpi_handle`,
`vhpi_handle_by_index`, `vhpi_handle_by_name`, `vhpi_iterator`,
`vhpi_put_value`, `vhpi_register_cb`, `vhpi_release_handle`, `vhpi_remove_cb`,
`vhpi_scan`. No data symbols (`vhpiFS` and friends are unused). This is the
same list as `src/cocotb/share/def/nvcvhpi.def`. rsim exports these and the
rest of the standard set (`src/exports.txt`); only the stub subset works
before M5.

Behaviour cocotb relies on, for M5:
- `vhpi_control(vhpiFinish, vhpiDiagTimeLoc)`: one extra `int` argument
  (`vhpiDiagT` from `vhpi_user_ext.h`). rsim accepts and ignores it.
- `vhpi_register_cb` with `vhpiReturnCb`, and `vhpi_remove_cb` on callbacks
  that already fired or were disabled (rule 9: keep those cheap).
- `vhpi_get_phys(vhpiResolutionLimitP, NULL)` for the time precision.
- `vhpiCaseNameP` with the original spelling. NVC returned upper-case names,
  so cocotb compares names case-insensitively under `#ifdef NVC`
  (`compare_names` in `VhpiImpl.cpp`). rsim keeps the original spelling
  (rule 8), so it can be built without that define.
- Generate labels: cocotb expects `label(index)` unless built with `ALDEC`
  (`label__index`). rsim should use the `(index)` form.

## Linux

- `rsim` exports the `vhpi_*` symbols from the executable (dynamic list,
  see `docs/decisions.md`). cocotb's VHPI library is linked with
  `-Wl,--allow-shlib-undefined` and resolves them from the process at load
  time. Nothing to link, no library naming constraint on rsim's side.
- The existing `libgpi_impl_nvc_vhpi.so` would already load under rsim: it
  only adds the NVC name workarounds. A dedicated `gpi_impl_rsim_vhpi` is
  still the right target (no `NVC` define).

## Windows

- cocotb links each VHPI implementation DLL against an import library it
  builds itself from a `.def` file in `src/cocotb/share/def/` (`lib /def:`
  with MSVC, `dlltool -d` with MinGW; `CMakeLists.txt`, the `WIN32` block).
  NVC is the precedent for an executable: `nvcvhpi.def` starts with
  `LIBRARY "nvc.exe"`. (Riviera's `aldec.def` names a DLL, `aldecpli.dll`.)
- So cocotb does not need rsim's shipped `rsim.lib`; it needs an `rsim.def`
  with `LIBRARY "rsim.exe"` and the export list above. Our generated
  `exports.def` plus that line is exactly it. `rsim.lib` serves plugins that
  are not built by cocotb.
- The import names `rsim.exe`, so the loader binds to the running
  executable only if it is called `rsim.exe`. A renamed binary breaks VHPI
  plugins; document it next to the install instructions.
- Other precedents (from the rust-vhpi README, see below): NVC ships its
  import library as `$PREFIX/lib/nvc/libnvcimp.a`, the same idea as our
  `rsim.lib`. rust-vhpi's `vhpi-shim` (the `dynamic` feature) avoids import
  libraries altogether: it resolves every `vhpi_*` at run time with
  `GetProcAddress(GetModuleHandleA(NULL), name)`, i.e. from the export
  table of the main executable. That works with rsim as is and does not
  depend on the executable's name, because rsim.exe really exports the
  symbols.

## Changes to propose upstream (M5)

1. `src/cocotb/share/def/rsimvhpi.def`: `LIBRARY "rsim.exe"` plus the
   `EXPORTS` list.
2. `CMakeLists.txt`: `add_vhpi_library(RSIM RSIM EXTRA_LIBS ${RSIM_EXTRA_LIBS})`
   with `rsimvhpi` on `WIN32`, and `rsimvhpi` in the two import-library loops.
3. `src/cocotb_tools/config.py`: add `"rsim"` to `supported_sims`; no entry
   function needed.
4. `src/cocotb_tools/makefiles/simulators/Makefile.rsim`, modelled on
   `Makefile.nvc`:
   - analyse: `rsim analyze --work $(LIB):$(SIM_BUILD)/$(LIB) $(VHDL_SOURCES_$(LIB))`
     per library, then the top library.
   - run: `GPI_IMPL=$(shell $(PYTHON_BIN) -m cocotb_tools.config --gpi-impl rsim vhpi) $(COCOTB_ENV)`
     `rsim run ... $(COCOTB_TOPLEVEL) --vhpi=$(shell $(PYTHON_BIN) -m cocotb_tools.config --lib-entry vhpi rsim) $(SIM_ARGS)`.
   - VHDL only, like NVC: refuse `VERILOG_SOURCES`.
5. `src/cocotb_tools/runner.py`: an `Rsim` runner like `Nvc` (`_build_command`
   with `rsim analyze`, `_test_command` with `rsim run --vhpi=...`, and
   `GPI_IMPL` set in `_set_env_test`). Add it to the simulator table.
6. cocotb's CI matrix entry for rsim on Linux and Windows.

## rust-vhpi as a second conformance client (M5)

[rust-vhpi](https://crates.io/crates/vhpi) (MIT OR Apache-2.0, by NVC's
author; read-only clone in `~/.cache/rust-vhpi/repo`, commit `65484e3`) is
the plugin side of VHPI in Rust: `vhpi-sys` (bindings to the IEEE header we
vendor), `vhpi` 0.6.0 (safe wrapper), `vhpi-shim`, and cdylib plugins with
testbenches:

| Plugin | Testbench | Checks |
|---|---|---|
| `dumper` | any (`test_examples/tb_*.vhdl`, `mixed/`) | walks the hierarchy, prints objects, types and values |
| `tests/test_simple` | `tb_simple.vhdl` | 6 assertion checkpoints |
| `tests/stringindexing` | `tb_string.vhdl` | string and array indexing |
| `tests/cb_toggle` | `tb_cb_toggle.vhdl` | disabling and re-enabling callbacks |
| `tests/foreignf` | `tb_foreignf.vhdl` | foreign subprograms through `vhpi_register_foreignf` |

It is not a dependency yet: every plugin needs iterators and values, which
the stub model lacks, and a Rust plugin has to be its own cdylib crate. The
M5 plan:

1. A test (or `scripts/vhpi-conformance.sh`) that builds the rust-vhpi
   plugins from a pinned checkout, runs each with its testbench on NVC
   (`nvc -a ... -e tb -r --load lib.so`) and on rsim (`rsim run tb --vhpi=lib.so`),
   and diffs stdout after normalising the tool name and version. The
   assertion plugins must report all checkpoints passed on both.
2. The `scripts/run_*.sh` marker lists are the pass criteria to copy.
3. On Windows, run the same plugins built with the `dynamic` feature
   (`vhpi-shim`), which exercises rsim.exe's export table directly.
4. Track pass counts per plugin in `STATUS.md` next to cocotb's regression.
