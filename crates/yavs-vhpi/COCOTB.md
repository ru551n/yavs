# What the Python testbench framework needs from yavs

Read from the `master` branch of a widely used Python testbench framework over VHPI at `ae55ced`
(2026-10-15), from a shallow clone kept outside the repository. File names below are relative to that
framework's repository and are named by role ("bootstrap library", "VHPI implementation").

## How the Python testbench framework starts under a VHPI simulator

1. The simulator loads the framework's bootstrap library (a `.so`, or a `.dll` on
   Windows, whose path comes from the framework's config tool via `--lib-entry vhpi <sim>`).
   That library exports `vhpi_startup_routines[] = { bootstrap_entry, NULL }`, so a simulator
   that calls `vhpi_startup_routines` needs no entry name. Only some simulators get
   `lib:bootstrap_entry` (the framework's `lib_entry` table).
2. `bootstrap_entry` reads the `COCOTB_BOOTSTRAP` environment variable (entries separated by `:`
   on Linux and `;` on Windows, each `library[,function]`) and loads libgpi,
   libpython and the PyGPI module in that order. The makefiles and the Python
   runner set it.
3. libgpi loads the implementations listed in `GPI_IMPL`
   (`path/libgpi_impl_<sim>_vhpi.so:gpi_impl_<sim>_vhpi_entry`) with
   `dlopen(RTLD_LAZY | RTLD_GLOBAL)`.
4. The framework's VHPI implementation calls `vhpi_*` directly. It includes its own
   vendored IEEE header (`vhpi_user.h`) plus `vhpi_user_ext.h`, never the
   simulator's header.

## Symbols the Python testbench framework's VHPI layer uses

`vhpi_check_error`, `vhpi_control`, `vhpi_get`, `vhpi_get_phys`,
`vhpi_get_str`, `vhpi_get_time`, `vhpi_get_value`, `vhpi_handle`,
`vhpi_handle_by_index`, `vhpi_handle_by_name`, `vhpi_iterator`,
`vhpi_put_value`, `vhpi_register_cb`, `vhpi_release_handle`, `vhpi_remove_cb`,
`vhpi_scan`. No data symbols (`vhpiFS` and friends are unused). This is the
same list as the framework's `.def` file for the primary reference simulator. yavs exports these and the
rest of the standard set (`src/exports.txt`); all of these work (hierarchy #269,
values #270, puts #271, callbacks #272), and cocotb's own regression passes
(`tools/cocotb`, #122). What is still missing of the standard is in `STANDARD.md`.

Behaviour the Python testbench framework relies on, for M5:
- `vhpi_control(vhpiFinish, vhpiDiagTimeLoc)`: one extra `int` argument
  (`vhpiDiagT` from `vhpi_user_ext.h`). yavs accepts and ignores it.
- `vhpi_register_cb` with `vhpiReturnCb`, and `vhpi_remove_cb` on callbacks
  that already fired or were disabled (rule 9: keep those cheap).
- `vhpi_get_phys(vhpiResolutionLimitP, NULL)` for the time precision.
- `vhpiCaseNameP` with the original spelling. The primary reference simulator returned upper-case names,
  so the framework compares names case-insensitively under a compile-time define for that
  simulator (`compare_names`). yavs keeps the original spelling
  (rule 8), so it can be built without that define.
- Generate labels: the Python testbench framework expects `label(index)` unless built with the define for a commercial simulator
  (`label__index`). yavs uses the `(index)` form (#269).

## Linux

- `yavs` exports the `vhpi_*` symbols from the executable (dynamic list,
  see `docs/decisions.md`). The Python testbench framework's VHPI library is linked with
  `-Wl,--allow-shlib-undefined` and resolves them from the process at load
  time. Nothing to link, no library naming constraint on yavs's side.
- The framework's existing VHPI implementation library for the primary reference simulator
  would already load under yavs: it only adds that simulator's name workarounds. A dedicated
  `gpi_impl_yavs_vhpi` is still the right target (no simulator define).

## Windows

- The Python testbench framework links each VHPI implementation DLL against an import library it
  builds itself from a `.def` file (`lib /def:`
  with MSVC, `dlltool -d` with MinGW; the `WIN32` block of its CMake file).
  The primary reference simulator is the precedent for an executable: its `.def` starts with
  `LIBRARY "<simulator>.exe"`. (A commercial simulator's `.def` names a DLL instead.)
- yavs ships no import library (user decision, 2026-10-18); a plugin brings its own. The framework
  therefore needs a `yavs.def` with `LIBRARY "yavs.exe"` and the export list above, and builds the
  import library from it. yavs's generated `exports.def` plus that line is exactly it.
- The import names `yavs.exe`, so the loader binds to the running
  executable only if it is called `yavs.exe`. A renamed binary breaks VHPI
  plugins; document it next to the install instructions.
- Other precedents (from the README of a Rust VHPI plugin crate, see below): the primary reference simulator ships its
  import library under its install prefix. That crate's shim (the `dynamic` feature) avoids import
  libraries altogether: it resolves every `vhpi_*` at run time with
  `GetProcAddress(GetModuleHandleA(NULL), name)`, i.e. from the export
  table of the main executable. That works with yavs as is and does not
  depend on the executable's name, because yavs.exe really exports the
  symbols.

## Changes to propose upstream (M5)

`tools/cocotb` carries items 3 to 5 locally (the makefile, the runner, the runner and pytest-plugin
table entries) and builds the VHPI library without simulator defines (item 2 on Linux); it runs
cocotb's own regression with them (#122).

1. A `yavsvhpi.def` file: `LIBRARY "yavs.exe"` plus the
   `EXPORTS` list.
2. The CMake file: `add_vhpi_library(YAVS YAVS EXTRA_LIBS ${YAVS_EXTRA_LIBS})`
   with `yavsvhpi` on `WIN32`, and `yavsvhpi` in the two import-library loops.
3. The config module: add `"yavs"` to `supported_sims`; no entry
   function needed.
4. A `Makefile.yavs` simulator makefile, modelled on the one of the primary reference simulator:
   - analyse: `yavs analyze --work $(LIB):$(SIM_BUILD)/$(LIB) $(VHDL_SOURCES_$(LIB))`
     per library, then the top library.
   - run: `GPI_IMPL=$(shell $(PYTHON_BIN) -m cocotb_tools.config --gpi-impl yavs vhpi) $(COCOTB_ENV)`
     `yavs run ... $(COCOTB_TOPLEVEL) --vhpi=$(shell $(PYTHON_BIN) -m cocotb_tools.config --lib-entry vhpi yavs) $(SIM_ARGS)`.
   - VHDL only, like the primary reference simulator: refuse `VERILOG_SOURCES`.
5. The framework's Python runner module: a `Yavs` runner like the one for the primary reference simulator (`_build_command`
   with `yavs analyze`, `_test_command` with `yavs run --vhpi=...`, and
   `GPI_IMPL` set in `_set_env_test`). Add it to the simulator table.
6. The framework's CI matrix entry for yavs on Linux and Windows.

## A Rust VHPI plugin crate as a second conformance client (M5)

The `vhpi` crate family on crates.io (MIT OR Apache-2.0, by the primary reference simulator's
author; read-only clone kept outside the repository, commit `65484e3`) is
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

It is not a dependency yet: every plugin needs values, which the model lacks
until #270, and a Rust plugin has to be its own cdylib crate. The
M5 plan:

1. A test (or `scripts/vhpi-conformance.sh`) that builds the crate's
   plugins from a pinned checkout, runs each with its testbench on the primary reference simulator
   (`sim1 -a ... -e tb -r --load lib.so`) and on yavs (`yavs run tb --vhpi=lib.so`),
   and diffs stdout after normalising the tool name and version. The
   assertion plugins must report all checkpoints passed on both.
2. The `scripts/run_*.sh` marker lists are the pass criteria to copy.
3. On Windows, run the same plugins built with the `dynamic` feature
   (`vhpi-shim`), which exercises yavs.exe's export table directly.
4. Track pass counts per plugin in `docs/STATUS.md` next to the framework's regression.

## Result of cocotb's own regression

Moved here from `docs/vhpi.md` (cocotb is not named in user documents, user 2026-10-28). Runner:
`tools/cocotb/run.py` (needs `uv`, network access for the pinned cocotb release and its test
dependencies, and a C++ compiler; installs into a virtual environment outside the repository).

Result (2026-10-28, #122): **78 of 78 jobs pass**. 43 jobs run VHDL tests (702 tests pass; the 10
failing tests are the ones that two of cocotb's own tests expect to fail); the other 35 are
Verilog-only or error-path cases that skip themselves. The result is the same at `--threads 1` and
`4`, and the `conformance` CI workflow runs it on every push to `main`. Full support is scheduled in
epic #143. An earlier record of 66 of 68 test directories (2026-10-19) had two failures, both in the
local setup of that time and not in yavs.
