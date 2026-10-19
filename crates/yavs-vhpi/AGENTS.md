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
- The object model: the design hierarchy (`design.rs`, #269), a read-only snapshot of the
  elaborated design with no simulation state, and the tool. `vhpi_get_value` (#270,
  `value.rs`) reads signals through the kernel `Hook` of the running observer call and
  generics and constants from their elaborated value; `vhpi_put_value` (#271) queues kernel
  deposits and forces. Simulation-phase, after-delay and value-change callbacks are real
  (#272): `VhpiObserver` (`sim.rs`) is a kernel `Observer`.
- Does not: drive simulation itself (only through `SimControl`, plan "Designed for the GUI"),
  or ship a separate VHPI DLL. `yavs run --vhpi` (#267, `crates/yavs/src/elab.rs`) sets the
  resolution limit (`set_resolution`) and the design (`set_design(&db, &elaborated)`), loads
  plugins with `load` and subscribes `VhpiObserver`, which runs the callbacks and turns
  `vhpi_control` into `Hook::end(Finish|Stop(0))`.
- Also loads `--load` libraries and binds VHPIDIRECT symbols (`load_foreign`, `bind_foreign`,
  #95), since it already owns `libloading` and the plugin path rules.
- Depends on: `yavs-diag`, `yavs-elab`, `yavs-sem` (the design snapshot), `yavs-ffi`,
  `yavs-kernel` (`Observer`, `Hook`, `Time`), `libloading`; build: `cc`. Called by: the `yavs` CLI
  (`--load`, `--vhpi`) and its spike binary `src/bin/yavs-vhpi-host.rs`.

## 2. Map

| File | Responsibility |
|---|---|
| `include/vhpi_user.h` | Official IEEE header, vendored unchanged (Apache-2.0) |
| `src/sys.rs` | `#[repr(C)]` types and one constant per enumerator (no bindgen), no unsafe |
| `src/abi.rs` | Exported `extern "C"` VHPI functions, process-wide state behind a mutex (`with_state`, `fire`); unsafe confined here and in `plugin.rs` |
| `src/design.rs` | The design hierarchy snapshot: `Design::build(db, elaborated)`, `Node` (regions, declarations, design units; `Origin` links each to the elaborated design for the value layer), `Layout` (sub-element structure, each scalar with its value type), `lookup` by name, `vhpi_name` |
| `src/value.rs` | `Scalar` (a scalar's value type), `Read` (a value as positions, integers or reals, from a kernel `Value` or an elaborated `ConstValue`), `natural` (the `vhpiObjTypeVal` format) and `encode` (every format); `decode` and `to_kernel` for puts (`Given` is the plugin's value, read out by `abi.rs`); no unsafe |
| `src/types.rs` | Types as VHPI objects: `Types` (interned `Ty`: kind, name, base, structure, ranges), `TypeObj` (type, range, literal, record element, unit), their properties, relations and iterators; built by `design.rs` (`object_ty`, `base_ty`) |
| `src/model.rs` | `Object` (tool, callback, node, sub-element path, iterator, type object), handles, `State` (properties, one-to-one relations, iterators), `Control` (`vhpi_control` requests), `W0903`/`W0904` reporting; `at` (where an object lives: kernel leaf and range, or elaborated value), `read`, `put` (queues a `Put`), `watch` (the kernel elements of a value-change callback) |
| `src/callbacks.rs` | Callback registry: registration, enable/disable/remove, per-reason sets of enabled callbacks, after-delay wake-up bookkeeping, value-change callbacks keyed by kernel signal (`watched`, `changed`, `WATCHING`), `Clock` (time in resolution units) |
| `src/sim.rs` | `VhpiObserver`: phase callbacks at the matching `Phase`, after-delay callbacks on `Observer::wake`, value-change callbacks on `Observer::value_change`, kernel requests (puts, wake-ups, cancels, end) after each batch |
| `src/plugin.rs` | `PluginSpec`, `load`, `LoadError` (`E0901` cannot load, `E0902` no entry point); `ForeignLib`, `load_foreign`, `bind_foreign` (`--load`: libraries in order, then the executable; never unloaded) |
| `src/foreign.rs` | Foreign models (#274): registrations (`vhpi_register_foreignf`, `vhpi_get_foreignf_info`), the calls of `'FOREIGN "VHPI library model"` subprograms as `Object::Call`/`Object::Param`, their reads and writes, `foreign_model` (the `yavs_interp::ForeignModel` the CLI binds); holds the two calls into plugin code (`elabf`, `execf`) |
| `src/shim.c` | `vhpi_printf`, `vhpi_vprintf`, `vhpi_assert`, `vhpi_control` (C variadics) |
| `src/exports.txt` | The one list of exported symbols |
| `build.rs` | Generates the Linux version script, the Windows `.def`, the test import library `yavs-vhpi-host.lib` (no `yavs.lib`) and a Rust table referencing every symbol; exports `DEP_YAVS_VHPI_VERSION_SCRIPT`, `_DEF`, `_IMPORT_LIB_DIR` |
| `src/lib.rs` | `set_design`, `set_design_root` (a root-only design for the spike host), `set_resolution`, `run_callbacks` (outside a simulation, the spike host), `take_control` |
| `tests/vhpi.rs`, `tests/plugin/plugin.c` | ABI vs C header (`abi_matches_c_header`, all ~610 constants, sizes, offsets), C plugin loaded both ways, missing file/entry/symbol errors |
| `tests/plugin/sched.c` | Callback scenarios (one entry each) run by `crates/yavs/tests/vhpi_run.rs` through `yavs run --vhpi` |
| `tests/plugin/put.c`, `tests/put/` | Puts and value changes: a counter clocked and reset only from VHPI, force/release/deposit on a driven signal (`expected.txt`), misuse (`errors.txt`); run like `hier.c` |
| `tests/plugin/values.c`, `tests/values/` | `vhpi_get_value` in every format over a design with every kind of object; run like `hier.c` |
| `tests/plugin/foreign.c`, `tests/foreign/` | Foreign function and procedure models and the design that calls them (`expected.txt` is the plugin's output); the `foreign_extras` entry (queries and misuse) is yavs only |
| `tests/plugin/hier.c`, `tests/hier/` | Hierarchy dump plugin, its design and expected output; run through `yavs run --vhpi` by `crates/yavs/tests/vhpi_run.rs` (`YAVS_BLESS=1` rewrites the dump) |

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
| The hierarchy is a snapshot built once from `Elaborated` and the `Db` before plugins load (`set_design`), owned, with no simulation state; each node keeps its `Origin` (scope, signal object, declaration, unit) for the value layer | The model must outlive the borrow of the design and be `Send`; plugins walk it at startup | #269 |
| Regions: root instance `vhpiRootInstK`, entity and component instances `vhpiCompInstStmtK` (the hidden component scope is skipped), blocks, `for` iterations `vhpiForGenerateK` named `label(index)`, `if` and `case` generates `vhpiIfGenerateK` (VHPI has no case-generate class), process statements; package instances `vhpiPackInstK` named `lib:pkg` (full name `@lib:pkg`), from `vhpiPackInsts` with a NULL or root reference, for every analyzed non-generic package and package instantiation. One reference simulator reports an `if` generate as a block and lists no `case` generate | The VHPI class model; `label(index)` is what Python testbench frameworks expect | #269 |
| Statement iterators (#316): `vhpiBlockStmts` lists a region's child blocks, `vhpiCompInstStmts` its child instances, `vhpiStmts` every child region (instances, blocks, generates, processes); none of them is an error, an empty one is a NULL iterator (a process has none). `vhpiGenerateStmts` stays `W0904`: both references with VHPI refuse it at the root | Both references list blocks and instances this way; one lists `vhpiStmts` (without processes, which it does not model) | #316 |
| Declarations: generics `vhpiGenericDeclK`, ports `vhpiPortDeclK` (with `vhpiModeP`; a mode view is `vhpiInoutMode`), signals, constants (a `for` generate's parameter is a constant of its iteration), variables and shared variables `vhpiVarDeclK`, in declaration order. Not yet listed: files, aliases, implicit signals, equivalent processes of concurrent statements | What the issue and the framework need first | #269 |
| Names: `vhpiCaseNameP` is the original spelling (rule 8), `vhpiNameP` upper-cases basic identifiers only (extended identifiers and character literals as written); full names `:Top:G(0):U:S` with `:` before regions and declarations, `.F` and `(i,j)` for sub-elements, enumeration indexes by literal (`C(Idle)`); `vhpiDefNameP` of an instance is `LIB.ENTITY(ARCH)`; a design unit's name is its simple name | Rule 8; the reference simulator agrees on every name it reports | #269 |
| `vhpi_handle_by_name`: a name starting with `:` or `@` is absolute; otherwise relative to the scope handle, or with NULL to the top (`top:u1:s`); case-insensitive except extended identifiers; blanks ignored outside extended identifiers; the longest prefix naming a declaration, then `.field` and `(index)` selections. A `.` that follows a region (no layout) is a hierarchy separator like `:` (`:TOP.CHILD.SIG`, `top.g(1).u.reg`; `Design::dots_to_colons`); a `.` after a declaration is a field selection. Both reference simulators that implement VHPI accept `.` between regions (checked with `tools/vhpi-conformance`, props plugin) | Python testbench frameworks probe both forms; extended identifiers are case-sensitive in VHDL | #269 |
| `vhpiIsUnconstrainedP` is 0 on every range handle and on scalar, record and constrained array types, 1 on an unconstrained array type (objects stay undefined, as in the references); `vhpiStrValP` of an enumeration literal is the literal as written (`'0'`, `IDLE`), as in both references | The Python testbench framework tests `!vhpi_get(vhpiIsUnconstrainedP, c)` per constraint and passes the literal string to `strncmp` | #332 |
| Sub-elements are not stored: an `Object::Elem` is a declaration plus a path of field indices and row-major element positions into its `Layout`; iterators over array elements are lazy. `vhpi_handle_by_index(vhpiIndexedNames, a, i)` is the element at row-major position `i` from the left | Arrays can be large; handles stay small | #269, rule 9 |
| Array bounds come from the elaborated shape (signals, ports) or the elaborated value or a constraint folded with the region's generics (generics, constants); a variable whose bounds are known only at run time has an open layout: composite, no size, and iterating its elements is `W0904` until #270 | Elaboration does not evaluate process declarative parts | #269 |
| `vhpi_compare_handles` compares objects (an iterator only equals itself); `vhpi_scan` releases the iterator when it returns NULL; an empty iteration is a NULL iterator | VHPI semantics | #269 |
| A property or relation yavs implements that does not apply to an object only sets `vhpi_check_error`; one it does not implement is `W0904` | Frameworks probe properties routinely | #269 |
| Plugin load failures are hard errors before simulation (`E0901`, `E0902`); misuse `W0903` every time; unimplemented calls `W0904` once per message; failed lookups only via `vhpi_check_error` | Fail early without breaking Python testbench framework probing | `docs/decisions.md` "VHPI ABI" |
| `unsafe` allowed crate-wide in `Cargo.toml` but confined to `abi.rs`, `plugin.rs`, `shim.c` and the plugin calls in `foreign.rs` | Root `AGENTS.md` rule 11 | `Cargo.toml`, crate docs |
| Logic values cross the boundary as std_ulogic codes (the `vhpiEnumT` value); BIT/BOOLEAN translated at the boundary | One encoding | `crates/yavs-logic/LAYOUT.md`, `docs/decisions.md` "Interpreter: BIT/BOOLEAN codes" |
| The tool's arguments (`State::argv`, set by `set_argv`): the program name, then the `+plusargs` of `yavs run`; read by `vhpiArgcP` and the `vhpiArgvs` iterator (`vhpi_scan` releases an exhausted iterator) | Plugins need the plusargs (#298) | `docs/cli-contract.md` 5 |
| `vhpi_get_value` reads signals only inside an observer call: `sim.rs` runs plugin callbacks inside `abi::hooked`, which puts the kernel `Hook` in a thread-local slot that `abi::with_hook` borrows (also the way in for puts, #271). Outside a callback a signal read is `W0903` | VHPI has no context argument; the observer holds the only kernel access while callbacks run | #270 |
| Value formats: enumeration values are positions (std_ulogic `vhpiU`..`vhpiDontCare`; BIT and BOOLEAN, kernel codes 2 and 3, become 0 and 1, as `vhpibit0`/`vhpiFalse`). `vhpiObjTypeVal` gives `vhpiLogicVal`/`vhpiLogicVecVal` for std_ulogic and BIT, `vhpiCharVal`/`vhpiStrVal` for CHARACTER, `vhpiSmallEnumVal`/`vhpiSmallEnumVecVal` for other enumerations of up to 256 literals (`vhpiEnumVal` beyond), `vhpiIntVal`, `vhpiRealVal`, `vhpiTimeVal` and `vhpiPhysVal` and their vector forms, and writes `numElems`; multi-dimensional arrays are flattened row major. Times count in the resolution limit, other physical values in base units | What the reference simulator with VHPI reports for every object it reads correctly | #270 |
| Permissive where the reference simulator is: enumeration formats and `vhpiIntVal` apply to every enumeration value, `vhpiEnumVal` also to a non-negative integer (both references with VHPI, #318), `vhpiStrVal` to an enumeration value (its literal: `TRUE`, `H`) and to arrays of character literals, `vhpiCharVal` to a character literal. Beyond it: binary/octal/hex strings of logic values (a group of bits that is not all `0`/`1` shows its common metavalue or `X`; decimal of a value with a metavalue is an error), radix strings of integer, enumeration and physical scalars (signed). A format that does not fit the value (a scalar format on an array, `vhpiIntVal` out of 32 bits) is `W0903` and -1, never a converted value; formats yavs does not produce (`vhpiPtrVal`, `vhpiRawDataVal`, `vhpiSmallPhys*`) and records or arrays of composites as a whole are `W0904` | No wrong values; the reference simulator returns zeros for several of these | #270 |
| An object's `vhpiType` is its type declaration when its subtype is the type's first subtype with the declaration's own bounds, a named subtype (`STD_LOGIC`, `NATURAL`) when the bounds are the subtype's own, else an anonymous subtype (`vhpiIsAnonymousP`) with the elaborated bounds, named like its base type as the reference simulator does (`STD_ULOGIC_VECTOR` for `std_logic_vector(7 downto 0)`). Equal types are one interned object, so handles compare equal. Every scalar type has its range as a constraint (enumerations and REAL too, where the reference simulator lists none); `vhpiBaseType` of a type declaration is itself | Python testbench frameworks branch on the base type's kind and name and size arrays from `vhpiConstraints` | #270 |
| Buffer protocol: a string or vector format returns the size it needs in bytes (strings with their NUL) when `bufSize` is smaller or the buffer pointer is NULL, with `numElems` already set | IEEE `vhpi_get_value` | #270 |

| `vhpi_put_value` (#271) checks the object, the phase and the value at the call (misuse `W0903`: constants, generics, bad formats, lengths or literals, `vhpiSizeConstraint`, a write in a read-only phase; `W0904`: variables (#309), records and arrays of composites as a whole), then queues a `Put` in the state; `sim.rs` hands the queue to `Hook::deposit`/`force`/`release` after each callback batch (puts made at load time at `StartOfSim`). So puts follow the kernel's rules: from an after-delay, next-time-step or start-of-cycle callback in this cycle, from a value-change or read-write callback in the next delta. A queue rather than `with_hook` at the call, so load-time puts take the same path | One path for every caller; deterministic at any thread count | #271 |
| Put modes: `vhpiDeposit[Propagate]` is a kernel deposit (driving value until a source next updates the elements), `vhpiForce[Propagate]` a force with the force statement's default mode (`out` for ports of mode out, inout, buffer; `in` otherwise), `vhpiRelease` the matching release. The non-propagating modes act like the propagating ones (the kernel changes no value without a cycle; the reference simulator refuses them). A release recomputes the driving value from the sources, so a release after a deposit returns to the drivers' value as both references do (#310; no sources: the deposit stays) | Permissive; one kernel API | #271 |
| Put formats: every format `vhpi_get_value` produces, decoded into the object's element type (`Read::decode`); binary strings of logic values are one literal per element (case-insensitive), octal, hex and decimal strings of logic values a number of that many bits; `vhpiObjTypeVal` means the natural format; enumeration positions are checked against the literals. ponytail: no range check of integer subtypes (a value outside a subtype's range is stored); add it with type objects | Mirrors the reads | #271 |
| `vhpiCbValueChange` (#272) on a signal, port or sub-element: registered with the object's kernel elements (`State::watch`, every leaf of a record), kept per kernel signal in `Callbacks::watched` while enabled; `sim.rs` dispatches it from `Observer::value_change` at most once per cycle (time, delta), in the kernel's signal and range order, then registration order, with `value` filled in the format it asks for (`vhpiObjTypeVal` becomes the natural one); it repeats until removed. A static `WATCHING` flag skips the state lock per event while none is enabled. On anything else it is `W0903` | #123: cheap when unused; one event bus | #272 |

| Foreign models (`'FOREIGN "VHPI library model"`, #274): the CLI binds a subprogram to the registration with the same library and model words (exact match) after the plugins load; `elabf` runs once per bound subprogram (its `obj` is the `vhpiForeignfK` registration handle, `vhpi_get_foreignf_info` fills a `vhpiForeignDataT`); `execf` runs per call, `obj` is the call (kind `vhpiFuncDeclK` or `vhpiProcDeclK`, as the reference simulator reports it; subprogram declarations are not in the object model otherwise) and, for a function, also the result object. Parameters are `vhpiConstParamDeclK` (mode in) or `vhpiVarParamDeclK`; `vhpi_put_value` takes `vhpiDeposit` and `vhpiDepositPropagate` on `out`/`inout` parameters and the result (other modes and `in` parameters are `W0903`); a function that sets no result is a run-time error, never a default. Scalars and arrays of scalars (the VHPIDIRECT shapes minus records and access); times count in the resolution limit like everywhere. Calls are serialized by one mutex so plugin code never runs concurrently, at any `--threads`. `vhpiArchF`, `vhpiLibF`, `vhpiAppF` are stored and queryable but never run. A registration needs both names; a function or procedure needs `execf`; a duplicate (kind, library, model) is `W0903` and NULL | The reference simulator that has foreign models supports only subprograms declared in architectures and entities, cannot return a `time` and rejects `vhpiLogicVecVal` writes to `inout` arrays; yavs also binds package subprograms | #274 |

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
| `vhpiCbStartOfProcesses` | `Phase::StartOfProcesses` (every cycle, before its processes run, #317) |
| `vhpiCbEndOfProcesses` | `Phase::EndOfProcesses` (every cycle, after its processes ran; what they scheduled is not applied yet; puts here make the next delta, #317) |
| `vhpiCbLastKnownDeltaCycle` (read-write) | `Phase::LastKnownDelta` (puts queued here create one more delta) |
| `vhpiCbStartOfPostponed` | `Phase::StartOfPostponed` (before the postponed processes, only when some run; no puts, #317) |
| `vhpiCbEndOfTimeStep` (read-only) | `Phase::EndOfTimeStep` (after postponed processes; no puts) |
| `vhpiCbValueChange` | `Observer::value_change`, once per cycle per callback (see the decisions above) |

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
- Open issues (snapshot 2026-10-19, `gh issue list -R ru551n/yavs --label crate:vhpi`): #121 real
  object model over `SimControl` and the event bus (hierarchy done, #269); #122 Python testbench
  framework regression (install needs approval); #309 variables' values. #123, #270, #271,
  #272, #274, #310 are done; foreign architectures (`vhpiArchF`) wait for `vhpi_create` and
  `vhpi_schedule_transaction`.
