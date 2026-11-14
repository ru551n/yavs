# VHPI: coverage of the standard and call-by-call behaviour

Moved from `docs/vhpi.md` (2026-10-28, maintainer: the site page keeps only the "At a glance"
table and how to load a plugin). Keep this file current as VHPI work lands; the user page links it.

## Coverage of the standard

The audit of yavs against the VHPI of IEEE 1076-2019 (`vhpi_user.h`, every class, relation,
property, format, mode, reason and function), 2026-10-28. "Partial" names what is missing; each
gap is tracked in an issue or in an item of the VHPI epic [#143](https://github.com/ru551n/yavs/issues/143), except the Windows second client (see its row).

| Area | Status | What works | Missing (issue) |
|---|---|---|---|
| Functions | partial | all 34 exported; every one works except the two on the right; `vhpi_get_next_time` in the end-of-time-step phase | `vhpi_get_data`, `vhpi_put_data` return 0 with an error: no save and restart (not started; tracked in [#143](https://github.com/ru551n/yavs/issues/143), item "VHPI save and restart") |
| Plugin loading, tool object | supported | startup table or `LIB:ENTRY`, Linux and Windows export tables, `vhpiArgcP`, `vhpiArgvs`, `vhpiNameP`, `vhpiToolVersionP`, `vhpiCapabilitiesP` (hierarchy, static access, connectivity, foreign models, advanced foreign models, run-time debug, reset unless the options refuse it), `vhpiPhaseP` with the tool handle or NULL | |
| Region classes | partial | root and component instances, blocks, `for` and `if`/`case` generates, processes, equivalent processes of concurrent statements, sequential statements (kind, place, label), package instances, entities, architectures, packages | |
| Declaration classes | partial | generics, ports, signals, constants, variables, shared variables, user attribute specifications and declarations of declarations, units and labels, aliases, files, implicit signals, components, configuration specifications, subprograms and their parameters and bodies, package bodies, libraries, use clauses | attribute names and other expressions; a configuration declaration's block configurations |
| Types | supported | type and subtype declarations of every scalar, array and record kind, constraints, literals, record elements, units, access, file and protected types (kind and name) | |
| One-to-one relations | partial | `vhpiRootInst`, `vhpiTool`, `vhpiUpperRegion`, `vhpiImmRegion`, `vhpiDesignUnit`, `vhpiPrimaryUnit`, `vhpiPrefix`, `vhpiParent`, `vhpiType`, `vhpiSubtype`, `vhpiBaseType`, `vhpiElemType`, `vhpiAttrDecl`, `vhpiSimNet`, `vhpiResolFunc`, `vhpiSignal` and `vhpiEqProcessStmt` of a driver, `vhpiFormal` and `vhpiActual` of a port association, `vhpiAliasedName`, `vhpiCompDecl`, `vhpiSubpBody`, `vhpiSubpDecl`, `vhpiReturnType`, `vhpiLibraryDecl` | expressions, stack frames, `vhpiConfigDecl` |
| One-to-many relations | partial | the region, declaration, element, type, parameter, driver (foreign models) and argument iterators of the hierarchy table below, `vhpiAttrSpecs` of a declaration, `vhpiDrivers`, `vhpiDrivenSigs`, `vhpiLocalContributors`, `vhpiOptimizedContributors`, `vhpiLocalLoads`, `vhpiOptimizedLoads`, `vhpiBasicSignals`, `vhpiTransactions`, `vhpiAliasDecls`, `vhpiSigAttrs`, `vhpiAttrDecls`, `vhpiConfigSpecs`, `vhpiSensitivities`, `vhpiEqProcessStmts`, `vhpiSeqStmts`, `vhpiParamDecls` of a subprogram, `vhpiLibraryDecls`, `vhpiDesignUnits`, `vhpiUseClauses` | expressions and branches; `vhpiGenerateStmts` deliberately not (both references refuse it) |
| Properties | partial | names, kind, file and line, mode, size, composite and scalar flags, type properties, `vhpiNumPortsP`, `vhpiNumGensP`, `vhpiLabelNameP`, `vhpiGenerateIndexP`, `vhpiIsSharedP`, `vhpiStartLineNoP`, `vhpiResolutionLimitP`, `vhpiIsResolvedP`, `vhpiIsBasicP`, `vhpiSigKindP`, `vhpiIsForcedP`, `vhpiTimeP`, `vhpiEndLineNoP`, `vhpiCompNameP`, `vhpiCompInstNameP`, `vhpiCompInstKindP`, `vhpiNumParamsP`, `vhpiIsPureP`, `vhpiPredefAttrP`, `vhpiAttrKindP`, `vhpiLibLogicalNameP`, `vhpiLibPhysicalNameP`, `vhpiStaticnessP`, `vhpiIsPassiveP`, `vhpiIsPostponedP`, `vhpiIsGuardedP`, `vhpiIsSeqStmtP` | |
| Value formats | supported | every format on scalars, arrays of any dimension, record fields; `vhpiRawDataVal` also on whole records and arrays of composites; `vhpiPtrVal` on access parameters of foreign calls | no value for protected objects and no `vhpiPtrVecVal` object exists (no array of access parameters) |
| Put modes | supported | `vhpiDepositPropagate`, `vhpiForcePropagate`, `vhpiRelease`, `vhpiDeposit` on variables, `vhpiSizeConstraint` on foreign results, `vhpi_schedule_transaction` on foreign drivers (inertial with pulse rejection, transport) | `vhpiDeposit` and `vhpiForce` on signals refused on purpose (two of three references refuse them) |
| Callback reasons | partial | the simulation-cycle reasons, after delay, value change, `vhpiCbTimeOut`, `vhpiCbRepTimeOut`, `vhpiCbSensitivity` on a signal, transaction (signal or driver), force, release, tool and elaboration phases, `vhpiCbPLIError`, `vhpiCbSigInterrupt`, interactive, reset | statement, subprogram call, suspend and resume (they need hooks in running code); analysis (`yavs run` does not analyse), quiescence (no meaning known without the standard's text), save and restart ([#143](https://github.com/ru551n/yavs/issues/143)): refused at registration |
| Foreign models | supported | `vhpiFuncF`, `vhpiProcF`, `vhpiArchF`, `vhpiLibF`, `vhpiAppF`, `vhpi_create` of processes and drivers | file parameters, `vhpiForeignfs` (W0904, foreign models only ever take scalars, arrays, records, access and signals) |
| Simulation control | supported | `vhpi_control(vhpiStop)`, `vhpi_control(vhpiFinish)`, `vhpi_control(vhpiReset)` (refused with options that report on a whole run) | |
| Errors and handles | supported | `vhpi_check_error` after every call, `vhpi_assert`, `vhpi_printf`, generational handles, `vhpi_compare_handles`, `vhpi_release_handle`, iterators released when exhausted | |
| Conformance evidence | partial | a Python testbench framework's own regression (CI: the whole of it on Linux, the jobs that need no make on Windows, workflow `cocotb-windows`), C plugin traces diffed against a reference simulator (`tools/vhpi-conformance`), the plugin tests of `crates/yavs/tests/vhpi_run.rs` | on Windows, the Rust `vhpi` crate's plugins as a second client (Linux: `tools/vhpi-conformance/rust_plugins.py`; [#273](https://github.com/ru551n/yavs/issues/273) is closed with the Linux part done; Windows has no open item) |

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
`vhpi_get_value` reads their values like constants, and `vhpiAttrDecl` gives the attribute declaration (`vhpiAttrDeclK`).
Writing them or a value-change callback on them is `W0903`.

Types: `vhpiType` (and `vhpiSubtype`), `vhpiBaseType` and `vhpiElemType` give kind, name,
constraints (integer, enumeration, floating and physical ranges with bounds, direction, null
flag), enumeration literals, record elements, physical units and dimensions.

### Connectivity

From the signal, port or sub-element: `vhpiDrivers`, `vhpiLocalContributors` (its drivers and the
port associations feeding it), `vhpiOptimizedContributors` (every driver reaching it through
ports), `vhpiLocalLoads` (the associations it feeds), `vhpiOptimizedLoads` (the ports it reaches),
`vhpiSimNet` (the outermost actual), `vhpiBasicSignals`, `vhpiResolFunc`, `vhpiIsResolvedP`,
`vhpiSigKindP`, `vhpiIsBasicP`, `vhpiIsForcedP`. From a process: `vhpiDrivenSigs`. A driver
(`vhpiDriverK`) has `vhpiSignal`, `vhpiEqProcessStmt`, `vhpiSizeP`, its value and
`vhpiTransactions`; a transaction (`vhpiTransactionK`) has `vhpiTimeP`, `vhpiIsNullP` and its
value, as pending when the iterator was made. A port association (`vhpiInPortK`, `vhpiOutPortK`)
has `vhpiFormal` and `vhpiActual`. Processes that read a signal are not listed as loads.

### Values

`vhpi_get_value` reads signals, ports, generics and constants, process variables and shared
variables (not of a protected type), and their record fields and array elements, from a
callback. Formats: `vhpiObjTypeVal` (the object's own format), `vhpiBinStrVal`, `vhpiOctStrVal`,
`vhpiDecStrVal`, `vhpiHexStrVal`, `vhpiStrVal`, `vhpiCharVal`, `vhpiEnumVal`,
`vhpiSmallEnumVal`, `vhpiLogicVal`, `vhpiIntVal`, `vhpiLongIntVal`, `vhpiRealVal`,
`vhpiTimeVal`, `vhpiPhysVal`, and the vector forms of the enumeration, logic, integer, real,
time and physical formats. A buffer that is too small makes the call return the size needed. A
format that does not fit is warning `W0903` and returns -1. A record or an array of composites
has `vhpiRawDataVal`, only when asked for (`vhpiObjTypeVal` on one is an error, as in the reference): its scalars in order, without
padding, native byte order: enumerations of up to 256 literals 1 byte (the position), longer ones
4, integers 4 or 8 as the kernel holds them, reals, times (in the resolution limit) and other
physical values (base units) 8. `vhpiSmallPhysVal` and `vhpiSmallPhysVecVal` are base units in 32
bits. A variable of a protected type has no value (warning `W0903`); `vhpi_protected_call` calls
the function with the handle and data and returns its result.

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
- `vhpiCbTransaction` on signals, ports, their sub-elements and drivers: once in every cycle
  with a transaction, also one that changes nothing, right after the update.
- `vhpiCbForce` and `vhpiCbRelease` on signals, ports and their sub-elements: for a VHDL force
  or release and for `vhpi_put_value` alike, once per operation.
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
The input format decides the kind of value (a binary string is a logic vector). Across kinds: an
integer to a real, a real to an integer (rounded) or decimal text, a logic array to an integer
(unsigned, no metavalues). Any other conversion to another kind is warning `W0903`. The output follows the buffer protocol of
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
- `vhpi_printf` and `vhpi_assert` write UTF-8: text that is not valid UTF-8 is taken as ISO-8859-1,
  the encoding of VHPI string values.
- Property 1100 (`vhpiRandomSeedP`) is an extension of one reference, not in the standard's header:
  `vhpi_get` on it with the tool handle or NULL is the seed of `--shuffle`. Without `--shuffle`
  (no seed is in use), or with a seed above 2147483647 (it does not fit `vhpiIntT`), it returns -1
  with a message for `vhpi_check_error` and no warning; other objects are an error too.
- `vhpi_control` with `vhpiStop` or `vhpiFinish` ends the run with status 0 (an extra argument
  after the command is accepted and ignored). `vhpiReset`: `docs/vhpi.md`, "Resetting a
  simulation". Other commands are refused.
- `+NAME[=VALUE]` arguments after the top level are available through `vhpiArgcP` and the
  `vhpiArgvs` iterator; the first argument is the program name `yavs`.
- Time counts in units of `--resolution` (`vhpi_get_time`, `vhpiResolutionLimitP`).
- Plugin calls are serialized, so a plugin needs no locking whatever `--threads` says.
- Under `--dap=stdio` a plugin's prints go to standard error, because standard output carries the
  debugger protocol. `--vhpi` and `--dap` work together.
- A call that yavs does not implement fails with a "not implemented" message that the plugin can
  read with `vhpi_check_error`, and yavs prints warning `W0904` once per message. A misused call
  (bad handle, property or argument) is warning `W0903`.
