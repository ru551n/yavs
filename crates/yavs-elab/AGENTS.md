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
| `src/lib.rs` | `find_top` (`ENTITY`, `ENTITY.ARCH`, `CONFIGURATION`), `elaborate(db, top, overrides, cfg)`, `ElabConfig` (`max_depth` 1000), `Top`, `ProcInst` (`lsps` in kernel offsets), `Elaborated` (`hier`, `nets`, `procs`, `model_vals`, `sigs`, `warnings`, `ir`; `signal`, `value`), `codes` |
| `src/walk.rs` | The elaborator: instances, generics, generates, blocks, binding (configuration declaration, then specification, then default binding), ports, processes, depth-first in statement order |
| `src/eval.rs` | Elaboration-time values: `Model`, `Val`, `elem_of`; `Db::fold_with` for globally static expressions, interpreter calls of user functions (`call_values`); `-g` overrides via the yavs-std `'value` parser |
| `src/value.rs` | Composite values: aggregates (any number of dimensions), indexing, slices, fields, concatenation, equality, BIT/BOOLEAN logical operators on `Val`; other predefined operations and `TO_STRING` through their yavs-interp natives (`implicit_call`); `encode`/`decode` between `Val` and IR memory (any composite, `Ty::layout`) |
| `src/pkgcall.rs` | Elaboration-time calls with package state (#77): `Session` (packages lowered and initialised, calls logged), `Call`, `replay` at build, `init_packages`, `run` |
| `src/hier.rs` | `Hierarchy`/`Scope`/`ScopeKind` (path names in `Case::Lrm` and original spelling), `NetGraph` (`select`: the one sub-element map; `copies`: equivalent processes of external-name actuals), `Leaf`, `SigObj`, `PortEdge`, `DriverDecl`, `ImplicitKind` |
| `src/sim.rs` | `Elaborated::lower` (yavs-lower), `Elaborated::build(Config) -> SimControl`, `runtime_diagnostic` (a kernel `SimError` as a coded diagnostic with the VHDL call stack as labels), `Ir`, `instance_of` |
| `src/build.rs` | `Elaborated::build_with` (kernel `Builder`, repeatable), `leaf_name` |
| `src/whole.rs` | Kernel resolver of a composite resolved as a whole: kernel leaves to and from the IR layout of the composite (#193) |
| `src/tests.rs` | Unit tests incl. the restart test (two elaborations and two builds give identical results) |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Elaboration is a pure, re-entrant function of (design, generics, configuration); no global state; `build_with` repeatable | In-process restart, rule 4 | `docs/plan.md` rule 4 |
| Generic overrides of any type parsed with the normal value parser (`-g`, yavs-std `'value`) | A recurring bug class in other simulators | rule 4 |
| External names bound at elaboration | A recurring bug class in other simulators | rule 4 |
| Uncollapsed ports, each formal its own kernel signal, two port edges per port; component instances get a hidden `Component` scope | Rule 3 (collapsing only as an optional optimisation later) | `docs/plan.md` rule 3, `docs/decisions.md` "Elaboration" |
| One sub-element map: `NetGraph::select`; row-major flattening, logic leaves mirrored (`n - 1 - p`) | Rule 2 | `docs/plan.md` rule 2, `docs/decisions.md` "Elaboration" |
| Logic leaves for `std_ulogic`, `bit`, `boolean` (scalars too); other enums 1 or 4 bytes, integers/physicals/reals 8; BIT/BOOLEAN codes are '0'/'1' (2/3) | Bit-planes everywhere (user, 2026-10-18) | `docs/decisions.md` "Elaboration", "Interpreter: BIT/BOOLEAN codes" |
| Records are one leaf per field (arrays of records: struct of arrays) | Rule 2 storage | `docs/decisions.md` "Elaboration" |
| Unlabeled processes are `_pN` per region | The reference simulator names, difftest paths match | same |
| Ids in deterministic depth-first order; kernel `SignalId` = `LeafId`, `ProcId` = process index | Deterministic commit order | `src/hier.rs` docs |
| Package constant calling a function of its own package before its body is elaborated: `E2114` with both spans | LRM 14.4.2, the reference simulators | `docs/decisions.md` "Elaboration" |
| Default binding, selected entity names, block configuration of a default-bound instance | Permissive per LRM policy | `docs/decisions.md` "Design units and binding" |
| A user resolution function declared in an entity or architecture runs with that instance's `InstData` (`Ir::res_regions`, its architecture lowered even without processes); declared in a block, generate or subprogram: E9999 | #164: it reads the region's constants and generics | this file |
| A composite subtype resolved as a whole (`Shape` `res`, `LeafRes::Whole`): kernel `resolved_composite` over the object's leaves, every driving process drives every leaf; nested in another composite or with resolved subelements: E9999 | #193, LRM 4.6 and 14.7.2 | `docs/decisions.md` "Elaboration" |
| Packages with run-time objects (signals, shared variables, files, constants sem did not fold) are lowered first and initialised once, in analysis order, with one `SharedState` for packages and processes; every signal of a loaded package is elaborated and found from any region (`Elaborated::signal`) | #183, LRM 14.4.2; the test runner keeps its state in packages | `docs/decisions.md` "Elaboration" |
| An external name as the actual of an `in` port is an expression actual: no port edge, the formal is driven by a kernel copy process `formal <= actual` (`NetGraph::copies`, built after the IR processes), one delta behind | #186, LRM 6.5.6.3, the reference simulator | `docs/decisions.md` "Elaboration", `docs/design/lrm-checks.md` L49 |
| A non-static expression actual of an `in` port (or part) is driven by its equivalent IR process `formal <= expr` in the instantiating region (`ProcInst::actual`, lowered by `Lowerer::port_actual`), one delta behind; a static expression part goes into the formal's initial value (`Model::part_init`) | #74, LRM 6.5.6.3 | `docs/decisions.md` "Elaboration" |
| An alias of an external signal binds at its declaration, in elaboration order (`E2117` before the target exists); `sigs[(region, alias)]` is the bound target, a process's alias keyed by the process's region | LRM 8.7, 14.4.1, #19 | `docs/decisions.md` "Elaboration" |
| Names through an object alias are the aliased name; `LspStep::Rebase` and alias prefixes map indices by element position (`Model::rebase`) | LRM 6.6.2, #203 | `docs/decisions.md` "Lowering" |
| A signal whose subtype leaves bounds open takes them from its evaluated initial value (`Model::shape_init`, element and record element bounds included); a package array signal's descriptor is written into package data after the package's initialisation; a signal with records gets its whole type's descriptor (`write_desc`, #215) | #187, LRM 6.4.2.3 (2019) | `docs/decisions.md` "Elaboration" |
| An entity with generic types, subprograms or packages: `Elab::instantiate` asks yavs-sem for the clone of entity and architecture for those actuals (`Db::instantiate_entity`, one per distinct map) and elaborates the clone, its generic constants as usual; a missing actual (top level) is E2103 | Shapes and code depend on the actual types; rule 4 identity by mangling (#75) | `docs/design/sem.md` 5.3 |
| Linkage ports are kernel `in` port edges; mode-view ports take each leaf's mode from `Db::port_mode_tree` (`leaf_modes`) | #75; LRM 6.5.2 | `docs/decisions.md` "Elaboration" |
| Not-yet-supported constructs are `E9999` naming the slice; expressions elab cannot evaluate yet `E2106` | Fail early | crate docs, #77 |
| Predefined operations elab does not evaluate structurally (shifts, array ordering, matching operators, `TO_STRING`, ...) run their yavs-interp native (`yavs_interp::call_native`, `Lowerer::implicit_native`) on the marshalled operands | One implementation of each operation, the simulation's | #220 |
| A constant with an impure initial value elab cannot evaluate yet keeps its run-time thunk with `W2118`; its calls run after the elaboration-time calls | Only for what elab cannot evaluate at all (`'PATH_NAME`, say); everything marshallable runs in declaration order | #220, `docs/decisions.md` "Elaboration" |
| Composite elaboration-time values: structural operations on `Val` here, function calls in yavs-interp through `call_values` | One evaluator for subprogram bodies, no expression thunks needing instance data | `docs/decisions.md` "Elaboration", #77 |
| Functions using package objects run in a session against the initialised packages; impure constants are evaluated in declaration order; package constants with impure initial values are read from package data; `build` replays the logged calls (`Elaborated::elab_calls`) before any instance data | LRM 14.4: side effects once and in order; rule 4 restart | `docs/decisions.md` "Elaboration", #77 |

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
- Codes owned: `E2100`-`E2199`, `W2100`-`W2199` (`yavs_elab::codes`: E2101 to E2117, W2105,
  W2108, W2118), plus `E9999`.

## 6. Coordination

- Requests to lowering: `crates/yavs-lower/DESIGN.md` "Requests from yavs-elab"; to sem:
  `crates/yavs-sem/CONTRIBUTING.md` "Requests from elab"; to IR/interp:
  `crates/yavs-ir/README.md` "Requests".
- Open issues (snapshot 2026-10-18, `gh issue list -R ru551n/yavs --label crate:elab`): #41
  `GenSelect::Index` evaluated without the generate's scope; #64 wire into `yavs run`; #68
  guarded assignments, force/release, implicit signal attributes; #74 conversion functions and
  non-static actuals in port maps; #76 package signals, external names, user resolution, nested resolution; #220
  the rest of E2106 (null multi-dimensional aggregates, `'PATH_NAME` in impure constants); #164 resolution functions in blocks; #93 external names; #101 package generics. Older
  unlabeled: #6 remaining elaboration features.
