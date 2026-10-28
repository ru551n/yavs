# VHPI: coverage of the standard and call-by-call behaviour

Moved from `docs/vhpi.md` (2026-10-28, maintainer: the site page keeps only the "At a glance"
table and how to load a plugin). Keep this file current as VHPI work lands; the user page links it.

## Coverage of the standard

The audit of yavs against the VHPI of IEEE 1076-2019 (`vhpi_user.h`, every class, relation,
property, format, mode, reason and function), 2026-10-28. "Partial" names what is missing; each
gap has an issue.

| Area | Status | What works | Missing (issue) |
|---|---|---|---|
| Functions | partial | all 34 exported; every one works except the three on the right; `vhpi_get_next_time` in the end-of-time-step phase | `vhpi_get_data`, `vhpi_put_data` ([#832](https://github.com/ru551n/yavs/issues/832)), `vhpi_protected_call` ([#833](https://github.com/ru551n/yavs/issues/833)) |
| Plugin loading, tool object | supported | startup table or `LIB:ENTRY`, Linux and Windows export tables, `vhpiArgcP`, `vhpiArgvs`, `vhpiNameP`, `vhpiToolVersionP`, `vhpiCapabilitiesP` (hierarchy, static access, foreign models, advanced foreign models, run-time debug) | `vhpiPhaseP` ([#832](https://github.com/ru551n/yavs/issues/832)) |
| Region classes | partial | root and component instances, blocks, `for` and `if`/`case` generates, processes, package instances, entities, architectures, packages | equivalent processes of concurrent statements, statements ([#829](https://github.com/ru551n/yavs/issues/829)); components, configurations, subprograms, libraries ([#831](https://github.com/ru551n/yavs/issues/831)) |
| Declaration classes | partial | generics, ports, signals, constants, variables, shared variables, user attribute specifications and declarations of declarations | aliases, files, implicit signals, attribute names ([#831](https://github.com/ru551n/yavs/issues/831)) |
| Types | supported | type and subtype declarations of every scalar, array and record kind, constraints, literals, record elements, units | access, file and protected types as objects ([#831](https://github.com/ru551n/yavs/issues/831), [#833](https://github.com/ru551n/yavs/issues/833)) |
| One-to-one relations | partial | `vhpiRootInst`, `vhpiTool`, `vhpiUpperRegion`, `vhpiImmRegion`, `vhpiDesignUnit`, `vhpiPrimaryUnit`, `vhpiPrefix`, `vhpiParent`, `vhpiType`, `vhpiSubtype`, `vhpiBaseType`, `vhpiElemType`, `vhpiAttrDecl` | expressions, statements, stack frames ([#829](https://github.com/ru551n/yavs/issues/829)), connectivity ([#830](https://github.com/ru551n/yavs/issues/830)), aliases and configurations ([#831](https://github.com/ru551n/yavs/issues/831)) |
| One-to-many relations | partial | the region, declaration, element, type, parameter, driver (foreign models) and argument iterators of the hierarchy table below, `vhpiAttrSpecs` of a declaration | `vhpiSensitivitys`, `vhpiSeqStmts` ([#829](https://github.com/ru551n/yavs/issues/829)), drivers, loads and contributors of signals ([#830](https://github.com/ru551n/yavs/issues/830)), `vhpiAliasDecls`, `vhpiSigAttrs`, `vhpiDesignUnits` ([#831](https://github.com/ru551n/yavs/issues/831)); `vhpiGenerateStmts` deliberately not (both references refuse it) |
| Properties | partial | names, kind, file and line, mode, size, composite and scalar flags, type properties, `vhpiNumPortsP`, `vhpiNumGensP`, `vhpiLabelNameP`, `vhpiGenerateIndexP`, `vhpiIsSharedP`, `vhpiStartLineNoP`, `vhpiResolutionLimitP` | staticness, passive, postponed, guarded ([#829](https://github.com/ru551n/yavs/issues/829)), resolved, signal kind, forced ([#830](https://github.com/ru551n/yavs/issues/830)), library and component names, end line ([#831](https://github.com/ru551n/yavs/issues/831)) |
| Value formats | partial | every format except the four on the right, on scalars, arrays of any dimension, record fields | records as a whole, `vhpiRawDataVal`, `vhpiSmallPhysVal`, `vhpiSmallPhysVecVal`, `vhpiPtrVecVal` ([#833](https://github.com/ru551n/yavs/issues/833)) |
| Put modes | supported | `vhpiDepositPropagate`, `vhpiForcePropagate`, `vhpiRelease`, `vhpiDeposit` on variables, `vhpiSizeConstraint` on foreign results, `vhpi_schedule_transaction` on foreign drivers (inertial with pulse rejection, transport) | `vhpiDeposit` and `vhpiForce` on signals refused on purpose (two of three references refuse them) |
| Callback reasons | partial | the simulation-cycle reasons, after delay, value change, `vhpiCbTimeOut`, `vhpiCbRepTimeOut`, `vhpiCbSensitivity` on a signal | transaction, force, release ([#830](https://github.com/ru551n/yavs/issues/830)); statement, subprogram call, suspend and resume ([#829](https://github.com/ru551n/yavs/issues/829)); tool, analysis and elaboration phases, quiescence, interrupt, errors, interactive, save, restart and reset ([#832](https://github.com/ru551n/yavs/issues/832)) |
| Foreign models | supported | `vhpiFuncF`, `vhpiProcF`, `vhpiArchF`, `vhpiLibF`, `vhpiAppF`, `vhpi_create` of processes and drivers | file parameters, `vhpiForeignfs` ([#831](https://github.com/ru551n/yavs/issues/831)) |
| Simulation control | partial | `vhpi_control(vhpiStop)`, `vhpi_control(vhpiFinish)` | `vhpiReset` ([#832](https://github.com/ru551n/yavs/issues/832)) |
| Errors and handles | supported | `vhpi_check_error` after every call, `vhpi_assert`, `vhpi_printf`, generational handles, `vhpi_compare_handles`, `vhpi_release_handle`, iterators released when exhausted | |
| Conformance evidence | partial | a Python testbench framework's own regression (CI, Linux; Windows not tried yet: [#849](https://github.com/ru551n/yavs/issues/849)), C plugin traces diffed against a reference simulator (`tools/vhpi-conformance`), the plugin tests of `crates/yavs/tests/vhpi_run.rs` | the Rust `vhpi` crate's plugins as a second client ([#273](https://github.com/ru551n/yavs/issues/273)) |

## Details

### Hierarchy and navigation

The hierarchy is a snapshot of the elaborated design taken before the plugins load: root and
component instances, blocks, `for` iterations (`G(0)`), `if` and `case` generates, process
statements, package instances (`@lib:pkg`), generics, ports (with mode), signals, constants
(generate parameters too) and variables. Record fields and array elements of any dimension are
made on demand (`vhpiSelectedNameK`, `vhpiIndexedNameK`).

| Call | Supported |
|---|---|
| `vhpi_handle` | root, tool, upper and immediate region, design and primary unit, prefix, `vhpiAttrDecl` of an attribute specification |
| `vhpi_handle_by_name` | absolute and relative names, case-insensitive, `.field` and `(index)` selections |
| `vhpi_handle_by_index`, `vhpi_iterator`, `vhpi_scan` | `vhpiInternalRegions`, `vhpiBlockStmts`, `vhpiCompInstStmts`, `vhpiStmts`, `vhpiDecls`, `vhpiSigDecls`, `vhpiPortDecls`, `vhpiGenericDecls`, `vhpiConstDecls`, `vhpiVarDecls`, `vhpiPackInsts`, `vhpiIndexedNames`, `vhpiSelectedNames`, `vhpiParamDecls`, `vhpiDrivers`, `vhpiAttrSpecs` (of a declaration) |
| `vhpi_get`, `vhpi_get_str`, `vhpi_get_real`, `vhpi_get_phys` | name, `vhpiCaseNameP` (original spelling), full name, kind, file, line, mode, `vhpiDefNameP`, size, composite and scalar flags, type properties; other properties report "not implemented" through `vhpi_check_error` |
| `vhpi_compare_handles`, `vhpi_release_handle`, `vhpi_check_error` | yes |

User attributes: `vhpiAttrSpecs` of a declaration lists its attribute specifications
(`vhpiAttrSpecK`, named after the attribute, full name `:top:s'mark`, also found by that name);
`vhpi_get_value` reads their values like constants (a value that depends on a generic is not
read yet: `W0904`, #831), and `vhpiAttrDecl` gives the attribute declaration (`vhpiAttrDeclK`).
Writing them or a value-change callback on them is `W0903`.

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
  for signals (a decision: they are warning `W0903`). Writes take effect in a new delta cycle at the same
  time.
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
- The foreign-model reasons: `vhpiCbTimeOut` and `vhpiCbRepTimeOut` behave as after-delay
  callbacks (the repeating one every period until removed), `vhpiCbSensitivity` on a signal
  as a value-change callback.

Any other reason is refused with a "not implemented" diagnostic.

A value-change callback registered while a cycle's changes are being delivered (for example one a
framework re-arms from inside its own callback) is also shown the changes of that same cycle, once.
This is a deliberate choice of yavs and differs from other simulators that implement VHPI: a
framework that waits repeatedly for the first of several triggers that fire together then does not
lose the later triggers.

### Converting values

`vhpi_format_value` converts a value from one format to another without an object: integers
(and the enumeration formats) to and from decimal, binary, octal and hex strings, bit, octal and
hex strings to and from logic vectors, reals, times and physical values, characters and strings.
The input format decides the kind of value (a binary string is a logic vector); a conversion to
another kind (an integer to a real) is warning `W0903`. The output follows the buffer protocol of
`vhpi_get_value`.

### Next activity time

`vhpi_get_next_time` gives the time of the next scheduled activity in units of the resolution
limit (0), or returns `vhpiNoActivity` when nothing is scheduled. It answers in the
end-of-time-step phase (`vhpiCbEndOfTimeStep`, the read-only phase), where the kernel has just
computed it; after-delay callbacks registered in the same callback count. Elsewhere it returns 1
with warning `W0903`.

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
- Forcing a variable, force and release of constants or generics, and the non-propagating put modes on
  signals (deliberately refused).
- The element subtype of a resolved array subtype reports `STD_ULOGIC` for `std_logic_vector`.

## Loading a plugin (full rules)

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
  `vhpiArgvs` iterator; the first argument is the program name `yavs`.
- Time counts in units of `--resolution` (`vhpi_get_time`, `vhpiResolutionLimitP`).
- Plugin calls are serialized, so a plugin needs no locking whatever `--threads` says.
- Under `--dap=stdio` a plugin's prints go to standard error, because standard output carries the
  debugger protocol. `--vhpi` and `--dap` work together.
- A call that yavs does not implement fails with a "not implemented" message that the plugin can
  read with `vhpi_check_error`, and yavs prints warning `W0904` once per message. A misused call
  (bad handle, property or argument) is warning `W0903`.
