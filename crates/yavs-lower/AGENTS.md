# AGENTS.md: yavs-lower

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-lower`.
The design, the module ownership table, the HIR to IR mapping table, the contract with
yavs-interp and yavs-elab and the Requests are in `crates/yavs-lower/DESIGN.md`; read it.

## 1. Purpose and boundaries

- Lowers yavs-sem's typed HIR to one `yavs_ir::Program`: one `Func` per process, subprogram body
  and elaboration body **per design unit** (shared by every instance, ir.md 2), plus a `UnitIr`
  per architecture (instance values, constants, signals, processes, driver slots) for the
  elaborator. Subprograms are lowered on first reference, so unused std/ieee bodies are not.
- Does not: own IR types (it only uses `yavs_ir::ProgramBuilder`/`FuncBuilder` and the
  verifier), analyze (input must be an error-free `Db`), fold generics or build the hierarchy
  (`yavs-elab`), or execute (`yavs-interp`).
- Depends on: `yavs-diag`, `yavs-syntax`, `yavs-sem`, `yavs-ir`, `yavs-kernel` (dev: `yavs-elab`,
  `yavs-interp`, `yavs-logic`). Called by: `yavs-elab` (`Elaborated::lower`).

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | `Lowerer` (`new`, `architecture`, `subprogram`, `subprograms_of`, `finish -> Lowered`), `LowerOptions` (`coverage`), `UnitIr`, `InstValue::write_value`, `InstConst`, `SignalIr`, `ProcessIr`, `Cx` (`error`, `unsupported`, `ice`, `cover`), `Obj`, `Space`, `top_architecture`, codes |
| `src/func.rs` | `Fx`: frames, parameters, declarations, subprograms, processes, architectures |
| `src/expr.rs` | Scalar expressions, calls, implicit operations, attributes, names, 1-D arrays |
| `src/stmt.rs` | Sequential statements, waits, signal assignments, case |
| `src/ty.rs` | HIR type to IR `Ty`, widths, `signal_elem` |
| `src/composite.rs`, `access.rs`, `protected.rs`, `file.rs`, `force.rs` | Records and composite arrays, access types, protected types, files/textio, force/release (lower2) |
| `src/reflect.rs` | `'reflect` and mirror lowering |
| `tests/corpus.rs` | `tests/corpus` groups through sem, elab (lowering), verifier, interpreter and kernel vs the reference simulator `.expected`; `PASS` list must match exactly |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| One code per design unit; generics, signals and non-locally-static constants are instance data (`Space::Inst`), never folded into code | Shared code, rule 4 identity | ir.md 2, DESIGN.md "Shape" |
| Objects live in frame slots; registers are per-statement temporaries; nothing live across `Suspend` | Simple verifier, re-entrant waits | DESIGN.md "Registers, slots, temporaries", ir.md 1.2 |
| Composite temporaries are `Alloc`ed and live to the frame's next statement | Results survive return without copies | `docs/decisions.md` "IR and interpreter (M2)" |
| Every array subtype gets a runtime descriptor (pool constant when static, `MakeDope` otherwise), one format | Rule 1 | `docs/plan.md` rule 1, DESIGN.md "Values and descriptors" |
| Logic arrays are bit-planes; element `i` from the left is plane element `n - 1 - i`; BIT/BOOLEAN use std_ulogic codes | Shared plane ops; one sub-element map | `docs/decisions.md` "Elaboration", "Interpreter: BIT/BOOLEAN codes" |
| Checked arithmetic (`BinRange`, `Check{Range}`), reference-simulator-compatible check texts | Rule 7, strict difftest | `docs/plan.md` rule 7, `crates/yavs-ir/README.md` Requests |
| Compact constants (`ConstData` runs/logic) | Rule 12 | `docs/plan.md` rule 12 |
| Every instruction carries the current `StmtId`; a `Stmt{id}` marker opens each statement | Debug hooks, coverage, traces | `docs/plan.md` "Designed for the GUI", DESIGN.md "Shape" |
| Coverage counters only with `LowerOptions::coverage` | Default code has no counters | DESIGN.md "Shape", rule 10 |
| Unlowered constructs are `E2001` naming the construct; broken HIR invariants `E9999`; any error means no program | Error motto | crate docs, DESIGN.md |
| `force` default mode `out` for out/inout/buffer ports, `in` otherwise | LRM 10.5.2.1 | DESIGN.md "Composites" |
| Instance data per region (instance, block, generate iteration); values written by the elaborator (`write_value`, `write_string`), thunks only for unfoldable scalar constants | Generate parameters, generics per instance, shared code | `docs/decisions.md` "Lowering" |
| One driver slot per `ProcessInfo::drivers` entry, in order | yavs-elab maps slots to LSP ranges | `docs/decisions.md` "Lowering" |
| Guarded blocks: GUARD's default is the condition; a `_guard` process drives GUARD | LRM 11.2, 11.6 | `docs/decisions.md` "Lowering" |
| Matching case compares with `?=` on BIT/STD_ULOGIC elements | LRM 10.9, 9.2.3 | `docs/decisions.md` "Lowering" |

Module-local decisions go in this section and DESIGN.md in the same commit (root `AGENTS.md`
rule 24).

## 4. Invariants and pitfalls

- Lowering a unit with errors is an ICE: callers must stop after sem errors.
- Every function goes through the yavs-ir verifier; a verifier error is an ICE of this crate,
  never "fixed" by relaxing the verifier.
- The calling and native conventions (fat pair `[data, dope]`, cells per parameter, signal
  parameter = two cells) are shared with `yavs-interp`: change both, note it in DESIGN.md and
  `crates/yavs-ir/README.md`.
- Kernel offsets of non-static indices must use the same rule as `yavs_elab::NetGraph::select`.
- `tests/corpus.rs` writes its library to a fixed `temp_dir()/yavs-lower-work`, which collides
  between worktrees and parallel runs (#160); until fixed, do not run two of them at once.
- A shared `CARGO_TARGET_DIR` can link another worktree's crates into the test binary (#7).

## 5. How to work here

```sh
CARGO_BUILD_JOBS=6 cargo test -p yavs-lower                     # unit tests and the corpus PASS list
cargo test -p yavs-lower --test corpus -- --nocapture           # print the non-passing files with their diff
python3 tools/difftest/difftest.py --candidate yavs             # full differential run (tools/difftest)
tools/difftest/report.sh                                        # rewrite tests/corpus/RESULTS.md
```

- A newly passing corpus file goes into `PASS` in `tests/corpus.rs` in the same commit; refresh
  `tests/corpus/RESULTS.md` with `tools/difftest/report.sh`.
- Codes owned: `E2000`-`E2099` (in use: `E2001` not lowered yet), plus `E9999`.

## 6. Coordination

- Owners per module: DESIGN.md "Modules and owners" (lower: core; lower2: composites). Replace an
  `E2001` arm in another owner's file with a call into your module in a small commit.
- Requests from yavs-elab and their answers: DESIGN.md "Requests from yavs-elab"; requests to the
  IR/interpreter: `crates/yavs-ir/README.md` "Requests".
- Open issues (snapshot 2026-10-18, `gh issue list -R ru551n/yavs --label crate:lower`): #64 wire
  lowering into `yavs run`; #67 array signals, sub-elements, aggregate targets, signal out
  parameters; #68 guarded assignments, null waveforms, force/release, implicit signal
  attributes; #69 matching case, case over arrays, non-static choices; #70 expression gaps; #71
  foreign, generic and instantiated subprograms, open actuals; #72 remaining attributes; #89 composite arrays and aggregates; #93 external names;
  #101 package generics; #160 fixed scratch dir in `tests/corpus.rs`. Older unlabeled: #5, #8,
  #9, #13 to #17.
