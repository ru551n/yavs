# AGENTS.md: yavs-elab

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-elab`.
The design is `docs/design/ir.md` section 4 (steps 1 to 10) and 3.6 (elaboration-time
evaluation); decisions in `docs/decisions.md` "Elaboration (yavs-elab, M2)".

## 1. Purpose and boundaries

- Turns an analyzed design (yavs-sem HIR) into a hierarchy of scopes, a signal net graph (leaves,
  drivers, sources, uncollapsed port edges, resolution) and a list of process instances; folds
  generics and constants; expands generates; binds components and configurations; checks
  multiple sources; then lowers the architectures that own processes (`yavs-lower`) and builds the
  kernel model with one interpreter process per process instance.
- Does not: analyze (needs an error-free `Db`), define IR (`yavs-ir`), execute processes
  (`yavs-interp`, `yavs-kernel`), or print (the CLI renders `warnings` and errors).
- Depends on: `yavs-diag`, `yavs-syntax`, `yavs-sem`, `yavs-lib`, `yavs-std`, `yavs-logic`,
  `yavs-ir`, `yavs-lower`, `yavs-interp`, `yavs-kernel`. Called by: `yavs` (`elab`, `run`),
  `yavs-lower` tests (dev).

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | `find_top` (`ENTITY`, `ENTITY.ARCH`, `CONFIGURATION`), `elaborate(db, top, overrides, cfg)`, `ElabConfig` (`max_depth` 1000, `collapse_ports`), `Top`, `ProcInst` (`lsps` in kernel offsets), `Elaborated` (`hier`, `nets`, `procs`, `model_vals`, `sigs`, `var_shapes` (elaborated shapes of process and shared variables for the waveform, #321), `regions` (the yavs-sem regions of each scope, read by VHPI), `var_ports`, `warnings`, `ir`; `signal`, `value`, `var_port`), `VarObj`, `codes` |
| `src/walk.rs` (`load_bodies` is public: the CLI runs it before the analysis image is taken, #407) | The elaborator: instances, generics, generates, blocks, binding (configuration declaration, then specification, then default binding), ports, processes, depth-first in statement order; a VHPI foreign architecture (`'FOREIGN "VHPI library model"`) as its ports and entity only (`foreign_archs`, #330) |
| `src/eval.rs` | Elaboration-time values: `Model`, `Val`, `elem_of`; `Db::fold_with` for globally static expressions, interpreter calls of user functions (`call_values`); `-g` overrides via the yavs-std `'value` parser |
| `src/value.rs` | Composite values: aggregates (any number of dimensions), indexing, slices, fields, concatenation, equality, BIT/BOOLEAN logical operators on `Val`; other predefined operations and `TO_STRING` through their yavs-interp natives (`implicit_call`); `encode`/`decode` between `Val` and IR memory (any composite, `Ty::layout`; public as `ir_encode`/`ir_decode` for VHPI variables) |
| `src/pkgcall.rs` | Elaboration-time calls with package state (#77): `Session` (packages lowered and initialised, calls logged; a new function is added to the program, `add`, not lowered again with the packages, #383), `Call`, `replay` at build, `init_packages`, `run` |
| `src/hier.rs` | `Hierarchy`/`Scope`/`ScopeKind` (path names in `Case::Lrm` and original spelling), `NetGraph` (`select`: the one sub-element map), `Leaf`, `SigObj`, `PortEdge`, `DriverDecl`, `Whole` (`NetGraph::wholes`, `whole_of`), `ImplicitKind` |
| `src/sim.rs` | `Elaborated::lower` and `lower_with` (yavs-lower; `LowerOptions` re-exported for coverage), `cov_rows` (coverage counter rows: row 0 without instance data, region `r` at `r + 1`, `InstData::cov_base`), `Elaborated::build(Config) -> SimControl` (`build_full_once` builds a run that builds once: the elaboration start state is moved into the runtime, not copied, #409; `build_full` also returns `Built`: runtime, shared state and the slot of each non-protected shared variable, #309; `build_debug` attaches every process to a debug session, named by `proc_name`), `runtime_diagnostic` (a kernel `SimError` as a coded diagnostic with the VHDL call stack as labels), `Ir`, `instance_of` |
| `src/collapse.rs` | `in_ports`: `in` ports whose actual is whole leaves become aliases of the actual (R4, #413), after `bind_late`; renumbers leaves and every `LeafId` in nets and processes |
| `src/build.rs` | `Elaborated::build_with` (kernel `Builder`, repeatable; the processes of VHPI foreign architectures, `ext_procs`, own their drivers and run no code, #330), `leaf_name` |
| `src/whole.rs` | Kernel resolver of a composite resolved as a whole: kernel leaves to and from the IR layout of the composite (#193) |
| `src/tests.rs` | Unit tests incl. the restart test (two elaborations and two builds give identical results) |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| `'left`/`'right`/`'low`/`'high`/`'ascending` of a non-real scalar subtype evaluate its range in the instance (`scalar_bounds`), never by folding | A range may name a port (`(p'range => '0')`, #311) | LRM 16.2 |
| `x'subtype`/`x'element` with open bounds (`Constraint::FromObject`) has the object's shape (`Model::shape_c`): a signal's elaborated shape, else its value's | The subtype is the object's (#315) | LRM 16.2 |
| `'range`/`'length`/... of a signal or of a record element of one (`p.data`, `b.resp.user`, `a(0).data`) take its bounds without a value (`Model::sig_shape`): a port's from its actual, a package signal's from its constrained subtype; an index into an array of records is still range-checked (E2107) | The bounds are known, the signal has no value during elaboration (#294, #304) | LRM 16.2 |
| Elaboration is a pure, re-entrant function of (design, generics, configuration); no global state; `build_with` repeatable | In-process restart, rule 4 | `docs/plan.md` rule 4 |
| Generic overrides of any type parsed with the normal value parser (`-g`, yavs-std `'value`) | A recurring bug class in other simulators | rule 4 |
| External names bound at elaboration | A recurring bug class in other simulators | rule 4 |
| Uncollapsed ports, each formal its own kernel signal, two port edges per port; component instances get a hidden `Component` scope | Rule 3 | `docs/plan.md` rule 3, `docs/decisions.md` "Elaboration" |
| Except `in` ports whose actual is whole leaves of the same type, no conversion, not forced or released by any VHDL statement, no driver, guard, user resolution or whole-composite resolution (`collapse.rs`): the formal object takes the actual's leaf range, so an object's leaves may be another object's (`Leaf::obj` is the actual), and anything reading a leaf's `path` of an object must go by position in `shape.leaves()`; off with `ElabConfig::collapse_ports` (`--no-collapse-ports`, `--dap`, `--vhpi`) | R4, #413, LRM 14.7.3.3, 10.5.2.1 | `docs/decisions.md` "Elaboration" |
| One sub-element map: `NetGraph::select`; row-major flattening, logic leaves mirrored (`n - 1 - p`) | Rule 2 | `docs/plan.md` rule 2, `docs/decisions.md` "Elaboration" |
| Logic leaves for `std_ulogic`, `bit`, `boolean` (scalars too); other enums 1 or 4 bytes, integers/physicals/reals 8; BIT/BOOLEAN codes are '0'/'1' (2/3) | Bit-planes everywhere (user, 2026-10-18) | `docs/decisions.md` "Elaboration", "Interpreter: BIT/BOOLEAN codes" |
| Records are one leaf per field (arrays of records: struct of arrays) | Rule 2 storage | `docs/decisions.md` "Elaboration" |
| Unlabeled processes are `_pN` per region | The reference simulator names, difftest paths match | same |
| Ids in deterministic depth-first order; kernel `SignalId` = `LeafId`, `ProcId` = process index | Deterministic commit order | `src/hier.rs` docs |
| Package constant calling a function of its own package before its body is elaborated: `E2114` with both spans | LRM 14.4.2, the reference simulators | `docs/decisions.md` "Elaboration" |
| Default binding, selected entity names, block configuration of a default-bound instance | Permissive per LRM policy | `docs/decisions.md` "Design units and binding" |
| A user resolution function declared in an entity or architecture runs with that instance's `InstData` (`Ir::res_regions`, its architecture lowered even without processes); declared in a block, generate or subprogram: E9999 | #164: it reads the region's constants and generics | this file |
| A user resolution function of a leaf (`LeafRes::Func`) runs once per element of the leaf; on `std_ulogic`, `bit` and `boolean` leaves its argument is a logic array of bit-planes and its result a position (`yavs_interp::resolve`'s `logic`) | #293, LRM 4.6; bit-planes everywhere | `docs/decisions.md` "Elaboration" |
| A composite subtype resolved as a whole (`Shape` `res`, `LeafRes::Whole`): the outermost one on each path is a `Whole` (`NetGraph::wholes`), one instance per element of the arrays above it, each a kernel `resolved_composite` over its leaf slices; a process driving part of an instance drives all of that instance; resolution below it is not used | #193, #333, LRM 4.6, 14.7.2, 14.7.3.1 | `docs/decisions.md` "Elaboration" |
| Packages with run-time objects (signals, shared variables, files, constants sem did not fold) are lowered first and initialised once, in analysis order, with one `SharedState` for packages and processes; every signal of a loaded package is elaborated and found from any region (`Elaborated::signal`) | #183, LRM 14.4.2; the test runner keeps its state in packages | `docs/decisions.md` "Elaboration" |
| An `inertial` actual is an expression actual even for a signal name, on a whole formal (`A::Expr`'s flag) and on a part of an individual association (`A::Parts`, the HIR keeps the flag per part) | #233, #261, LRM 6.5.6.3 | `docs/design/lrm-checks.md` L80 |
| An external name as the actual of an `in` port is a signal name actual: an ordinary port edge, same delta | #186, LRM 8.7, user 2026-10-19 (majority of the reference simulators) | `docs/decisions.md` "Elaboration", `docs/design/lrm-checks.md` L49 |
| A non-static expression actual of an `in` port (or part) is driven by its equivalent IR process `formal <= expr` in the instantiating region (`ProcInst::actual`, lowered by `Lowerer::port_actual`), one delta behind, one driver per leaf of the formal; in a binding indication the region is the component's; a scalar formal (or a scalar part, #263, its subtype taken from the elaborated port: `scalar_part`) is range-checked, with bounds from the instance's generics evaluated in the instance (`PortActual::bounds`); a static expression part goes into the formal's initial value (`Model::part_init`); an unconstrained formal takes a static value's bounds, else the expression needs a globally static subtype (E2111) | #74, LRM 6.5.6.3 | `docs/decisions.md` "Elaboration" |
| An alias of an external signal binds at its declaration, in elaboration order (`E2117` before the target exists); `sigs[(region, alias)]` is the bound target, a process's alias keyed by the process's region | LRM 8.7, 14.4.1, #19 | `docs/decisions.md` "Elaboration" |
| External names used directly in processes, concurrent statements and expression port actuals get implicit aliases (`Db::alias_external_uses`) bound after the whole hierarchy (`Elab::bind_late`, with the drivers of processes driving them); constants bind to the target's value (`Model::set`), shared variables to their region and declaration (`Elaborated::ext_vars`, cell copied in `region_data`); lookup by name in the region's sem scopes (`Model::regions`) | LRM 8.7, L71, #93: the reference simulators accept a process before the instance it names | `docs/decisions.md` "Elaboration" |
| External names in subprograms (`ExtOwner::Sub`) bind with the region declaring the subprogram: per elaborated region late (`Elab::decls` queues them), or for a package once per design in `bind_late` (`Elaborated::pkg_ext`), a failure there recorded in `Elaborated::ext_unbound` (the call fails); `build` writes a package's external constants and variables into the shared variables area; an external constant bound to a run-time-only target (`Elaborated::ext_consts`) copies the target's thunk result (`target_const`); package paths are package objects (`Db::bind_package_paths`); a package declaration naming the hierarchy is `E2117` (`package_externals`) | LRM 8.7, 14.4.1, L86, #255 | `docs/decisions.md` "Elaboration" |
| Names through an object alias are the aliased name; `LspStep::Rebase` and alias prefixes map indices by element position (`Model::rebase`) | LRM 6.6.2, #203 | `docs/decisions.md` "Lowering" |
| A signal whose subtype leaves bounds open takes them from its evaluated initial value (`Model::shape_init`, element and record element bounds included); a package array signal's descriptor is written into package data after the package's initialisation; a signal with records gets its whole type's descriptor (`write_desc`, #215) | #187, LRM 6.4.2.3 (2019) | `docs/decisions.md` "Elaboration" |
| An entity with generic types, subprograms or packages: `Elab::instantiate` asks yavs-sem for the clone of entity and architecture for those actuals (`Db::instantiate_entity`, one per distinct map) and elaborates the clone, its generic constants as usual; a missing actual (top level) is E2103 | Shapes and code depend on the actual types; rule 4 identity by mangling (#75) | `docs/design/sem.md` 5.3 |
| A component with generic types, subprograms or packages: `Elab::component` elaborates the clone (`Db::instantiate_component`), its objects also under the original's generic constants and ports (the binding names those), and `through_component` turns entity actuals naming the component's formals into the component's actuals; blocks arrive instantiated from sem | #222 | `docs/decisions.md` "Generics" |
| `'stable`, `'quiet` and `'transaction` of a prefix over several leaves are one implicit leaf with the other leaves as extra sources (`Leaf::implicit_more`, kernel `implicit_source`); `'delayed` of one is one delayed leaf per leaf of the prefix | LRM 16.2.3: an event or activity on any scalar subelement counts, #215 | `docs/decisions.md` "Lowering" |
| Implicit signals named in a process (sem: the process's scope) are registered under the region enclosing the process; those of a package body's subprograms are package signals of the package's scope, created after the packages' own signals | LRM 16.2.4, #240 | `docs/decisions.md` "Lowering" |
| An unconstrained formal with a conversion takes its bounds from the conversion's subtype: the actual part's result subtype or type mark (modes `in`, `inout`), else the formal part's parameter subtype or type mark | #262, LRM 5.3.2.2 | `docs/design/lrm-checks.md` L87 |
| A converted association with several leaves on a side (record port, array of composites) is one `ConvPort` (kernel `port_parts`), the function's argument and result packed and unpacked by `whole::Map`; a type conversion of such a port maps leaf to leaf (`Side::Leaves`); an `inout` port converted on the actual side only is E1313 in sem (the formal side only is accepted: no `to_formal` converter, the kernel passes the actual's parts through one to one) | #74, LRM 6.5.7.1: the conversion is part of the port, in the same delta | ir.md 4.1 |
| Package instances: a design unit instance is a run-time package (`run_time_packages`, `Db::package_regions`) whose signals live in its own package scope; objects of an instance declared in a region are elaborated with that region's (`Db::decls_with_instances`); generic packages have none | LRM 4.9, 14.4.2 (#101) | `docs/decisions.md` "Elaboration" |
| A shared variable is one object per elaborated declaring region (instance, block, generate alternative, `for` iteration): its thunk runs once, keyed by that region (`Elaborated::shared_vars`), and nested regions' instance data hold the same handle | LRM 14.4, 6.4.2.4, #190, #249 | `docs/decisions.md` "Lowering" |
| A 2019 variable port denotes the shared variable at the end of its chain of actuals (`Elaborated::var_ports`, `VarObj`); building writes the object's handle into the formal's cell (`UnitIr::var_ports`) from the declaring region's instance data, built on demand (`region_data`), or a package thunk; a top-level one is `E2119` | #188, LRM 6.5.2, 6.5.6.3 | `docs/decisions.md` "Elaboration" |
| `NetGraph::select`: a `Field` step after an array of records (no index) selects that element of every array element (drivers of mode-view signal parameters of arrays) | #254; Lsp has no "every element" step | `docs/decisions.md` "Lowering" |
| Packages declared in declarative parts add their objects to the declaring region (`Db::decls_with_instances`); `load_bodies` clones instance bodies until no new instance appears, since a cloned body can declare instances (#34) | LRM 4.7, 4.9 | `docs/decisions.md` "Elaboration" |
| An unconstrained port associated individually is shaped from its parts (`parts_shape`): an array per dimension from the lowest to the highest index named, in the index subtype's direction; a record element by element, an element associated as a whole taking its actual's bounds (a signal name's, else the expression's subtype's or static value's, else E2111), an unconstrained element left unassociated E2111; arrays with unconstrained elements and partially constrained records are E9999 | #288, LRM 6.5.7.1 | `docs/decisions.md` "Elaboration" |
| Linkage ports are kernel `in` port edges; mode-view ports take each leaf's mode from `Db::port_mode_tree` (`leaf_modes`) | #75; LRM 6.5.2 | `docs/decisions.md` "Elaboration" |
| Not-yet-supported constructs are `E9999` naming the slice; expressions elab cannot evaluate yet `E2106` | Fail early | crate docs, #77 |
| Predefined operations elab does not evaluate structurally (shifts, array ordering, matching operators, `TO_STRING`, ...) run their yavs-interp native (`yavs_interp::call_native`, `Lowerer::implicit_native`) on the marshalled operands | One implementation of each operation, the simulation's | #220 |
| A constant with an impure initial value elab cannot evaluate yet keeps its run-time thunk with `W2118`; its calls run after the elaboration-time calls | Only for what elab cannot evaluate at all (`'PATH_NAME` of an object, say; a design unit's is evaluated, #314); everything marshallable runs in declaration order | #220, `docs/decisions.md` "Elaboration" |
| Composite elaboration-time values: structural operations on `Val` here, function calls in yavs-interp through `call_values` | One evaluator for subprogram bodies, no expression thunks needing instance data | `docs/decisions.md` "Elaboration", #77 |
| The session's program grows: a function called for the first time is lowered into it (`Lowerer::resume`, `snapshot`) and the runtime moves to it with the package state kept (`Runtime::extend`); only `undo`, a failed lowering or call, or other run-time packages lower everything, initialise the packages and replay the log. The simulation's program goes on from the session's lowering and starts from a copy of the package state elaboration left (`Session::finish`, `pkgcall::Start`, `Ir::start`): no package initialisation and no replay at build; it falls back to both with coverage counters, other run-time packages, a subprogram with external names lowered during elaboration (`Lowerer::externals_early`), a file opened or a call failed during elaboration | Initialising the packages again per new function, and once more with the replay for the simulation, was most of elaboration on library testbenches (#383) | `docs/decisions.md` "Elaboration" |
| Functions using package objects run in a session against the initialised packages; impure constants are evaluated in declaration order; package constants with impure initial values are read from package data; `build` replays the logged calls (`Elaborated::elab_calls`) before any instance data | LRM 14.4: side effects once and in order; rule 4 restart | `docs/decisions.md` "Elaboration", #77 |
| A process's declarative part is not evaluated by elab (`Elab::decls` of a process scope registers only its implicit signals and aliases): the lowered process elaborates it at start | LRM 14.5: elaborated exactly once; #251 | `docs/decisions.md` "Elaboration" |

Module-local decisions go in this section and ir.md section 4 or `docs/decisions.md`
"Elaboration" in the same commit (root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- **Determinism:** elaboration order is the kernel's commit order. Never iterate a `HashMap` to
  create scopes, leaves, drivers or processes.
- **Fail early:** any error-severity diagnostic means no `Elaborated`; never carry a guessed
  value (e.g. a default for an unevaluable generic) into the kernel.
- Offsets of non-static indices computed by lowering must agree with `NetGraph::select`.
- `ProcInst::lsps` must match `ProcessInfo::drivers` of the HIR statement one to one, in order
  (the lowering's driver slots).
- Restart test: anything cached across `elaborate` calls breaks rule 4.
- The CLI passes `-g` values verbatim; this crate parses them.

## 5. How to work here

```sh
CARGO_BUILD_JOBS=6 cargo test -p yavs-elab
cargo test -p yavs-lower --test corpus            # corpus through elab, lowering, interp, kernel
cargo run -p yavs -- elab --print-hierarchy --work=work:$HOME/.cache/<dir>/work top
python3 tools/difftest/difftest.py --candidate yavs
```

- New behaviour: a unit test in `src/tests.rs` (inline VHDL), and a negative case for each new
  code. End-to-end cases go into `tests/corpus` with the reference simulator output (`tests/corpus/AGENTS.md`).
- Codes owned: `E2100`-`E2199`, `W2100`-`W2199` (`yavs_elab::codes`: E2101 to E2117, E2119, E2120, W2105,
  W2108, W2118), plus `E9999`.

## 6. Coordination

- Requests to lowering: `crates/yavs-lower/DESIGN.md` "Requests from yavs-elab"; to sem:
  `crates/yavs-sem/CONTRIBUTING.md` "Requests from elab"; to IR/interp:
  `crates/yavs-ir/README.md` "Requests".
- Open issues (snapshot 2026-10-18, `gh issue list -R ru551n/yavs --label crate:elab`): #41
  `GenSelect::Index` evaluated without the generate's scope; #64 wire into `yavs run`; #68
  guarded assignments, force/release, implicit signal attributes; #74 conversion functions and
  non-static actuals in port maps; #76 package signals, external names, user resolution; #220
  the rest of E2106 (null multi-dimensional aggregates, `'PATH_NAME` of named entities other than design units); #164 resolution functions in blocks; #93 external names. Older
  unlabeled: #6 remaining elaboration features.
