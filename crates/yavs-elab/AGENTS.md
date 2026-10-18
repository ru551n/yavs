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
| `src/value.rs` | Composite values: aggregates, indexing, slices, fields, concatenation, equality, BIT/BOOLEAN logical operators on `Val`; `encode`/`decode` between `Val` and IR memory (arrays and records of scalars) |
| `src/hier.rs` | `Hierarchy`/`Scope`/`ScopeKind` (path names in `Case::Lrm` and original spelling), `NetGraph` (`select`: the one sub-element map), `Leaf`, `SigObj`, `PortEdge`, `DriverDecl`, `ImplicitKind` |
| `src/sim.rs` | `Elaborated::lower` (yavs-lower), `Elaborated::build(Config) -> SimControl`, `runtime_diagnostic` (a kernel `SimError` as a coded diagnostic with the VHDL call stack as labels), `Ir`, `instance_of` |
| `src/build.rs` | `Elaborated::build_with` (kernel `Builder`, repeatable), `leaf_name` |
| `src/tests.rs` | Unit tests incl. the restart test (two elaborations and two builds give identical results) |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Elaboration is a pure, re-entrant function of (design, generics, configuration); no global state; `build_with` repeatable | In-process restart (GHDL #1184), rule 4 | `docs/plan.md` rule 4 |
| Generic overrides of any type parsed with the normal value parser (`-g`, yavs-std `'value`) | GHDL #1117 | rule 4 |
| External names bound at elaboration | GHDL #548 | rule 4 |
| Uncollapsed ports, each formal its own kernel signal, two port edges per port; component instances get a hidden `Component` scope | Rule 3 (collapsing only as an optional optimisation later) | `docs/plan.md` rule 3, `docs/decisions.md` "Elaboration" |
| One sub-element map: `NetGraph::select`; row-major flattening, logic leaves mirrored (`n - 1 - p`) | Rule 2 | `docs/plan.md` rule 2, `docs/decisions.md` "Elaboration" |
| Logic leaves for `std_ulogic`, `bit`, `boolean` (scalars too); other enums 1 or 4 bytes, integers/physicals/reals 8; BIT/BOOLEAN codes are '0'/'1' (2/3) | Bit-planes everywhere (user, 2026-10-18) | `docs/decisions.md` "Elaboration", "Interpreter: BIT/BOOLEAN codes" |
| Records are one leaf per field (arrays of records: struct of arrays) | Rule 2 storage | `docs/decisions.md` "Elaboration" |
| Unlabeled processes are `_pN` per region | NVC names, difftest paths match | same |
| Ids in deterministic depth-first order; kernel `SignalId` = `LeafId`, `ProcId` = process index | Deterministic commit order | `src/hier.rs` docs |
| Package constant calling a function of its own package before its body is elaborated: `E2114` with both spans | LRM 14.4.2, NVC and Questa | `docs/decisions.md` "Elaboration" |
| Default binding, selected entity names, block configuration of a default-bound instance | Permissive per LRM policy | `docs/decisions.md` "Design units and binding" |
| A user resolution function declared in an entity or architecture runs with that instance's `InstData` (`Ir::res_regions`, its architecture lowered even without processes); declared in a block, generate or subprogram: E9999 | #164: it reads the region's constants and generics | this file |
| Packages with run-time objects (signals, shared variables, files, constants sem did not fold) are lowered first and initialised once, in analysis order, with one `SharedState` for packages and processes; every signal of a loaded package is elaborated and found from any region (`Elaborated::signal`) | #183, LRM 14.4.2; VUnit keeps its state in packages | `docs/decisions.md` "Elaboration" |
| Linkage ports are kernel `in` port edges; mode-view ports take each leaf's mode from `Db::port_mode_tree` (`leaf_modes`) | #75; LRM 6.5.2 | `docs/decisions.md` "Elaboration" |
| Not-yet-supported constructs are `E9999` naming the slice; composite values elab cannot evaluate yet (multi-dimensional aggregates, arrays of composites passed to functions, shifts) `E2106` (`ponytail:` in `eval.rs`) | Fail early | crate docs, #77 |
| Composite elaboration-time values: structural operations on `Val` here, function calls in yavs-interp through `call_values` | One evaluator for subprogram bodies, no expression thunks needing instance data | `docs/decisions.md` "Elaboration", #77 |

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
  code. End-to-end cases go into `tests/corpus` with an NVC reference (`tests/corpus/AGENTS.md`).
- Codes owned: `E2100`-`E2199`, `W2100`-`W2199` (`yavs_elab::codes`: E2101 to E2117, W2105,
  W2108), plus `E9999`.

## 6. Coordination

- Requests to lowering: `crates/yavs-lower/DESIGN.md` "Requests from yavs-elab"; to sem:
  `crates/yavs-sem/CONTRIBUTING.md` "Requests from elab"; to IR/interp:
  `crates/yavs-ir/README.md` "Requests".
- Open issues (snapshot 2026-10-18, `gh issue list -R ru551n/yavs --label crate:elab`): #41
  `GenSelect::Index` evaluated without the generate's scope; #64 wire into `yavs run`; #68
  guarded assignments, force/release, implicit signal attributes; #74 conversion functions and
  non-static actuals in port maps; #75 generic types, subprograms and packages, binding without
  entity aspect; #76 package signals, external names, user resolution, nested resolution; #77
  composite values at elaboration (rest of E2106); #164 resolution functions in blocks; #93 external names; #101 package generics. Older
  unlabeled: #6 remaining elaboration features.
