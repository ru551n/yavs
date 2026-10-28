# VHPI support

VHPI is the standard C interface (IEEE 1076-2019) that lets a plugin or a test framework look
into a running VHDL simulation, read and write signals, and react to simulation events. yavs
implements it: plugins written in C, C++ or Rust can inspect the design hierarchy, read and
drive values, register callbacks and supply foreign models. cocotb 2.1.0's own regression
passes in full (78 of 78 jobs, 702 VHDL tests). Full cocotb support is scheduled in epic
[#143](https://github.com/ru551n/yavs/issues/143), and the scope is full VHPI per the standard,
not only what cocotb needs.

## At a glance

| Area | Supported | Not supported yet |
|---|---|---|
| Loading | `--vhpi=LIB[:ENTRY]`, several plugins, `+args` | |
| Hierarchy | instances, blocks, generates, processes, packages, generics, ports, signals, constants, variables, record fields, array elements | aliases, files, equivalent processes of concurrent statements, variables of subprograms |
| Lookup | by name, by index, iterators, parent/prefix handles | |
| Reading values | all common formats, from callbacks | whole records, protected objects, raw data |
| Writing values | signals and ports (deposit or force with propagation), variables, driver transactions | forcing variables, constants, generics, non-propagating modes on signals |
| Callbacks | start/end of simulation and initialization, time step, delta cycle, process and postponed phases, after delay, value change | any other reason (for example sensitivity) |
| Foreign models | functions, procedures, architectures, library and application hooks | file parameters |
| Misc calls | `vhpi_control` stop and finish | `vhpi_get_data`, `vhpi_put_data`, `vhpi_format_value`, `vhpi_get_next_time`, `vhpi_protected_call` (exported so plugins link, but they report "not implemented") |

## cocotb

cocotb's own simulator regression runs on yavs with `tools/cocotb/run.py` (VHDL over VHPI, the
way cocotb's CI runs it for a simulator). It needs `uv`, network access for the pinned cocotb 2.1.0
and its test dependencies, and a C++ compiler; everything is installed into a virtual environment
outside the repository.

```
cargo build --release -p yavs
python3 -I tools/cocotb/run.py --yavs target/release/yavs [--run-args='--threads=4']
```

Result (2026-10-28, issue [#122](https://github.com/ru551n/yavs/issues/122)): **78 of 78 jobs pass**.
43 jobs run VHDL tests (702 tests pass; the 10 failing tests are the ones that cocotb's own
`test_failure` and `test_max_failures` expect to fail); the other 35 are Verilog-only or
error-path cases that skip themselves. The result is the same at `--threads 1` and `4`, and the
`conformance` CI workflow runs it on every push to `main`.

An earlier record of 66 of 68 test directories (2026-10-19) had two failures, both in the local
setup of that time and not in yavs: the time resolution and the plusargs were not passed on, and
cocotb's Python runner had no yavs entry.

## Details

### Loading a plugin

```
yavs run --vhpi=LIB[:ENTRY] [--vhpi=...] TOP
```

- Plugins load in command-line order, after elaboration and before the simulation starts.
- Without `ENTRY`, every routine in the library's NULL-terminated `vhpi_startup_routines` array
  runs. With `ENTRY`, that `void (*)(void)` function runs instead. The text after the last `:`
  counts as an entry only when it is a C identifier, so a Windows path such as `C:\x\lib.dll`
  stays a path.
- A name with a directory part must name an existing file. A bare name goes through the
  operating system's library search path.
- A plugin that cannot be loaded is error `E0901` (with the loader's own message); one with no
  entry is `E0902`. Both end the run with exit code 2 before time 0.
- Linux: the `yavs` executable exports the whole `vhpi_*` set, so a plugin leaves those symbols
  unresolved and links nothing. It is opened with `RTLD_NOW | RTLD_GLOBAL`, so a symbol yavs
  does not export fails at load, and libraries the plugin loads later can see its symbols.
- Windows: the `.exe` exports the same set. yavs ships no import library; a plugin brings its
  own (built from the generated export list) or resolves the names at run time from the main
  module. An import library made for `yavs.exe` binds only to an executable of that name.
- `vhpi_control` with `vhpiStop` or `vhpiFinish` ends the run with status 0 (an extra argument
  after the command is accepted and ignored). Other commands are refused.
- `+NAME[=VALUE]` arguments after the top level are available through `vhpiArgcP` and the
  `vhpiArgvs` iterator.
- Time counts in units of `--resolution` (`vhpi_get_time`, `vhpiResolutionLimitP`).
- Plugin calls are serialized, so a plugin needs no locking whatever `--threads` says. Under
  `--dap=stdio` a plugin's prints go to standard error.

### Hierarchy and navigation

The hierarchy is a snapshot of the elaborated design taken before the plugins load: root and
component instances, blocks, `for` iterations (`G(0)`), `if` and `case` generates, process
statements, package instances (`@lib:pkg`), generics, ports (with mode), signals, constants
(generate parameters too) and variables. Record fields and array elements of any dimension are
made on demand (`vhpiSelectedNameK`, `vhpiIndexedNameK`).

| Call | Supported |
|---|---|
| `vhpi_handle` | root, tool, upper and immediate region, design and primary unit, prefix |
| `vhpi_handle_by_name` | absolute and relative names, case-insensitive, `.field` and `(index)` selections |
| `vhpi_handle_by_index`, `vhpi_iterator`, `vhpi_scan` | `vhpiInternalRegions`, `vhpiBlockStmts`, `vhpiCompInstStmts`, `vhpiStmts`, `vhpiDecls`, `vhpiSigDecls`, `vhpiPortDecls`, `vhpiGenericDecls`, `vhpiConstDecls`, `vhpiVarDecls`, `vhpiPackInsts`, `vhpiIndexedNames`, `vhpiSelectedNames`, `vhpiParamDecls`, `vhpiDrivers` |
| `vhpi_get`, `vhpi_get_str`, `vhpi_get_real`, `vhpi_get_phys` | name, `vhpiCaseNameP` (original spelling), full name, kind, file, line, mode, `vhpiDefNameP`, size, composite and scalar flags, type properties; other properties report "not implemented" through `vhpi_check_error` |
| `vhpi_compare_handles`, `vhpi_release_handle`, `vhpi_check_error` | yes |

Types: `vhpiType` (and `vhpiSubtype`), `vhpiBaseType` and `vhpiElemType` give kind, name,
constraints (integer, enumeration, floating and physical ranges with bounds, direction, null
flag), enumeration literals, record elements, physical units and dimensions.

### Values

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
  for signals. Writes take effect in a new delta cycle at the same time.
- Writes are refused in the read-only phases (start of postponed, end of time step, end of
  simulation). Constants, generics, whole records and `vhpiSizeConstraint` give `W0903`.
- Variables take `vhpiDeposit` or `vhpiDepositPropagate`, written at once; forcing or releasing
  a variable gives `W0903`. A process variable exists once the process has run.
- `vhpi_schedule_transaction` works on the drivers of foreign-model parameters and ports, with
  inertial or transport delay.

### Callbacks

`vhpi_register_cb`, `vhpi_remove_cb`, `vhpi_enable_cb`, `vhpi_disable_cb` and
`vhpi_get_cb_info` are implemented. A disabled callback costs nothing while it waits. Accepted
reasons, each also in its `Rep` (repeating) form where the standard has one:

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

### Foreign models

A subprogram or architecture whose `foreign` attribute is `"VHPI library model"` runs the model
a plugin registered with `vhpi_register_foreignf`: `vhpiFuncF`, `vhpiProcF` (scalars, arrays of
scalars, record, access and signal parameters; array results), `vhpiArchF` (processes and
drivers made with `vhpi_create`; an unregistered one is error `E0905`), `vhpiLibF` and
`vhpiAppF`. File parameters are not supported.

### Other gaps

- Sub-elements of variables whose bounds are only known at run time.
- Values: `vhpiRawDataVal`, `vhpiSmallPhys*` formats; `vhpiPtrVal` only for `in` access
  parameters of foreign subprograms.
- Force and release of constants or generics.
- The element subtype of a resolved array subtype reports `STD_ULOGIC` for `std_logic_vector`.

For per-area implementation notes see the "VHPI plugins" section of the user guide.
