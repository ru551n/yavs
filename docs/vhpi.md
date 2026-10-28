# VHPI support

yavs implements the VHPI C interface of IEEE 1076-2019 (`vhpi_user.h`, vendored unchanged) so a
plugin written in C, C++ or Rust, or a test framework such as cocotb, can inspect and drive a
running simulation. This page lists what works today. The user-level description is the "VHPI
plugins" section of `usage.rst`; the details per area are in `crates/yavs-vhpi/AGENTS.md`.

Work on full cocotb support is scheduled: epic
[#143](https://github.com/ru551n/yavs/issues/143). The scope is full VHPI support per the
standard, not only what cocotb needs.

## Loading a plugin

```
yavs run --vhpi=LIB[:ENTRY] [--vhpi=...] TOP
```

- `--vhpi` is repeatable; plugins load in command-line order, after elaboration and before the
  simulation starts.
- Without `ENTRY`, every routine in the library's NULL-terminated `vhpi_startup_routines` array
  runs. With `ENTRY`, that `void (*)(void)` function runs instead. The text after the last `:`
  counts as an entry only when it is a C identifier, so a Windows path such as `C:\x\lib.dll`
  stays a path.
- A name with a directory part must name an existing file. A bare name goes through the
  operating system's library search path.
- A plugin that cannot be loaded is error `E0901` (with the loader's own message), one with no
  entry is `E0902`; both end the run with exit code 2 before time 0.
- Linux: the `yavs` executable exports the whole `vhpi_*` set, so a plugin leaves those symbols
  unresolved and links nothing. It is opened with `RTLD_NOW | RTLD_GLOBAL`, so a symbol yavs
  does not export fails at load, and libraries the plugin loads later can see its symbols.
- Windows: the `.exe` exports the same set. yavs ships no import library; a plugin brings its
  own (built from the generated export list) or resolves the names at run time from the main
  module. An import library made for `yavs.exe` binds only to an executable of that name.
- `vhpi_control` with `vhpiStop` or `vhpiFinish` ends the run with status 0 (an extra
  argument after the command is accepted and ignored). Other commands are refused.
- `+NAME[=VALUE]` arguments after the top level are available through `vhpiArgcP` and the
  `vhpiArgvs` iterator.
- Time counts in units of `--resolution` (`vhpi_get_time`, `vhpiResolutionLimitP`).
- Plugin calls are serialized, so a plugin needs no locking whatever `--threads` says.
  Under `--dap=stdio` a plugin's prints go to standard error.

## Object model and navigation

The hierarchy is a snapshot of the elaborated design taken before the plugins load. It has the
root and component instances, blocks, `for` iterations (`G(0)`), `if` and `case` generates,
process statements, package instances (`@lib:pkg`), generics, ports (with mode), signals,
constants (generate parameters too) and variables. Record fields and array elements of any
dimension are made on demand (`vhpiSelectedNameK`, `vhpiIndexedNameK`).

| Call | Supported |
|---|---|
| `vhpi_handle` | root, tool, upper and immediate region, design and primary unit, prefix |
| `vhpi_handle_by_name` | absolute and relative names, case-insensitive, `.field` and `(index)` selections |
| `vhpi_handle_by_index`, `vhpi_iterator`, `vhpi_scan` | `vhpiInternalRegions`, `vhpiBlockStmts`, `vhpiCompInstStmts`, `vhpiStmts`, `vhpiDecls`, `vhpiSigDecls`, `vhpiPortDecls`, `vhpiGenericDecls`, `vhpiConstDecls`, `vhpiVarDecls`, `vhpiPackInsts`, `vhpiIndexedNames`, `vhpiSelectedNames`, `vhpiParamDecls`, `vhpiDrivers` |
| `vhpi_get`, `vhpi_get_str`, `vhpi_get_real`, `vhpi_get_phys` | name, `vhpiCaseNameP` (original spelling), full name, kind, file, line, mode, `vhpiDefNameP`, size, composite and scalar flags, type properties below; other properties report "not implemented" through `vhpi_check_error` |
| `vhpi_compare_handles`, `vhpi_release_handle`, `vhpi_check_error` | yes |

Types: `vhpiType` (and `vhpiSubtype`), `vhpiBaseType` and `vhpiElemType` give kind, name,
constraints (integer, enumeration, floating and physical ranges with bounds, direction, null
flag), enumeration literals, record elements, physical units and dimensions.

## Values

`vhpi_get_value` reads signals, ports, generics and constants, process variables and shared
variables (not of a protected type), and their record fields and array elements, from a
callback. Formats: `vhpiObjTypeVal` (the object's own format), `vhpiBinStrVal`, `vhpiOctStrVal`,
`vhpiDecStrVal`, `vhpiHexStrVal`, `vhpiStrVal`, `vhpiCharVal`, `vhpiEnumVal`,
`vhpiSmallEnumVal`, `vhpiLogicVal`, `vhpiIntVal`, `vhpiLongIntVal`, `vhpiRealVal`,
`vhpiTimeVal`, `vhpiPhysVal`, and the vector forms of the enumeration, logic, integer, real,
time and physical formats. A buffer that is too small makes the call return the size needed. A
format that does not fit is warning `W0903` and returns -1.

`vhpi_put_value` writes signals, ports and their fields and elements in the same formats, plus
an integer into a logic vector and `vhpiStrVal` text (padded or cut to the string's length):

- `vhpiDepositPropagate` sets the value until a driver next updates it; `vhpiForcePropagate`
  holds it until `vhpiRelease`. The non-propagating `vhpiDeposit` and `vhpiForce` are refused
  for signals. Writes take effect in a new delta at the same time.
- Writes are refused in the read-only phases (start of postponed, end of time step, end of
  simulation). Constants, generics, whole records and `vhpiSizeConstraint` are `W0903`.
- Variables take `vhpiDeposit` or `vhpiDepositPropagate`, written at once; forcing or releasing
  a variable is `W0903`. A process variable exists once the process has run.
- `vhpi_schedule_transaction` works on the drivers of foreign-model parameters and ports
  (see below), with inertial or transport delay.

## Callbacks

`vhpi_register_cb`, `vhpi_remove_cb`, `vhpi_enable_cb`, `vhpi_disable_cb` and
`vhpi_get_cb_info` are implemented. A disabled callback costs nothing while it waits. These
reasons are accepted, each also in its `Rep` form where the standard has one:

- `vhpiCbStartOfSimulation`, `vhpiCbEndOfSimulation`
- `vhpiCbStartOfInitialization`, `vhpiCbEndOfInitialization`
- `vhpiCbNextTimeStep`, `vhpiCbStartOfNextCycle`
- `vhpiCbStartOfProcesses`, `vhpiCbEndOfProcesses`, `vhpiCbStartOfPostponed`
- `vhpiCbLastKnownDeltaCycle` (read-write phase)
- `vhpiCbEndOfTimeStep` (read-only phase)
- `vhpiCbAfterDelay`
- `vhpiCbValueChange` on signals, ports and their sub-elements: runs once in every cycle in
  which any element changed, with the value in the format the `value` field asks for.

Any other reason (for example `vhpiCbSensitivity`) is refused with a "not implemented"
diagnostic.

## Foreign models

A subprogram or architecture whose `foreign` attribute is `"VHPI library model"` runs the model
a plugin registered with `vhpi_register_foreignf`: `vhpiFuncF`, `vhpiProcF` (scalars, arrays of
scalars, record, access and signal parameters; array results), `vhpiArchF` (processes and
drivers made with `vhpi_create`; an unregistered one is `E0905`), `vhpiLibF` and `vhpiAppF`.
File parameters are not supported.

## cocotb

cocotb 2.1.0's own regression (`tests/test_cases`, 68 directories, `SIM=yavs
TOPLEVEL_LANG=vhdl`, the VHPI library cocotb ships for another simulator, a local
`Makefile.yavs` adapted from that simulator's) was last recorded at **66 of 68 directories
passing** (2026-10-19, tracked in issue #122). The two that fail, per that record:

1. `test_integers`: not a yavs problem. cocotb's own pytest runner has a fixed list of
   simulators and no yavs entry, so it stops before reaching yavs.
2. `test_first_on_coincident_triggers`, test `test_repeated_first_no_missed_edges`: it expects a
   callback registered in the same time step as a change to fire for that change. cocotb marks
   this test as an expected failure for every simulator with VHPI that it knows; yavs behaves
   like them, but is not yet in that list.

No VHPI blocker was left in that run. A later run (2026-10-22, interpreter and compiled tier
alike, identical per-test results) saw non-zero exits in `test_cocotb`
(`test_timer_with_units`: the local makefile did not pass `--resolution`),
`test_plusargs` (the local makefile put `+args` before the top unit) and `test_integers`. The
two makefile problems are in the local integration, not in yavs. Running cocotb's regression
with yavs as a first-class cocotb simulator (a runner entry, a yavs-specific VHPI library) is
part of the scheduled work.

## Not supported yet

- `vhpi_get_data`, `vhpi_put_data`, `vhpi_format_value`, `vhpi_get_next_time`,
  `vhpi_protected_call`: exported so plugins link, but report "not implemented".
- `vhpi_control` commands other than stop and finish; callback reasons not listed above.
- Equivalent processes of concurrent statements, aliases, files and `vhpiGenerateIndexP` in the
  hierarchy; sub-elements of variables whose bounds are only known at run time; variables of
  subprograms.
- Values: records as a whole, protected objects, `vhpiRawDataVal`, `vhpiSmallPhys*` formats;
  `vhpiPtrVal` only for `in` access parameters of foreign subprograms.
- Forcing a variable, force and release of constants or generics, and the non-propagating put
  modes on signals (deliberately refused).
- The element subtype of a resolved array subtype reports `STD_ULOGIC` for `std_logic_vector`.
