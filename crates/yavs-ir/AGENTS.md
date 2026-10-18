# AGENTS.md: yavs-ir

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-ir`.
The design is `docs/design/ir.md` section 1 (and 2, 9); what is stable vs still moving, the
calling and native conventions, and the Requests log are in `crates/yavs-ir/README.md`.

## 1. Purpose and boundaries

- The register IR: `Program` (functions: processes, subprograms, elaboration bodies, thunks),
  runtime type descriptions (`Ty`, descriptor-based layouts, bit-plane logic arrays), the compact
  constant pool, the global statement table (`StmtInfo`), natives by `lib/NATIVE.md` symbol,
  foreign references, check messages, case tables, coverage items, reflection descriptions.
  Built once by `ProgramBuilder`/`FuncBuilder`, verified, then immutable and `Send + Sync`.
- Does not: execute (`yavs-interp`), lower HIR (`yavs-lower`), elaborate (`yavs-elab`), or JIT
  (`yavs-llvm`, M6). It has no VHDL front-end knowledge.
- Depends on: `yavs-diag`, `yavs-kernel` (`StmtId`), `yavs-ffi` (foreign signatures). Called by:
  `yavs-lower` and `yavs-elab` (producers), `yavs-interp` (consumer); later `yavs-llvm`.

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | `Id` trait, 32-bit id newtypes (`FuncId`, `BlockId`, `TyId`, `ConstId`, `NativeId`, `ForeignId`, `CheckMsgId`, `CaseTableId`, `CovItem`), `IdVec`; re-exports |
| `src/program.rs` | `Program`, `Func` (`kind`, parent for static links, params, locals, root map, register types, blocks, `loc` = one `StmtId` per instruction), `FuncFlags` (`MAY_SUSPEND`, `TOUCHES_SHARED`), `StmtInfo`, `ConstData`, `NativeRef`, `ForeignRef`/`ForeignArg`, `CheckMsg`, `CaseTable`, `CovInfo` |
| `src/inst.rs` | `Inst` (all instructions incl. `BinRange`, `FToIRange`, `FCheck`, `CallProt`, `Elab*`), `Reg`, `RegTy`, `AddrSpace`, op enums |
| `src/ty.rs` | `Ty`, `Layout`/`LayoutError`, `LogicKind`, `Field`, descriptor format (`DOPE_DIM_BYTES` = 24: `[left, right, dir]` i64 per dimension), `dim_len`, `Refl`/`ReflKind` |
| `src/build.rs` | `ProgramBuilder` (types, constants, natives, `check_msg_for`, `reflect`), `FuncBuilder`; `finish()` propagates flags, numbers resume points, runs the verifier |
| `src/verify.rs` | One linear walk; `VerifyError` (an ICE of the producer) |
| `src/dump.rs` | Text dump (`Program`'s `Display`, `dump_func`) |
| `tests/verify.rs` | Verifier and layout tests |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| 32-bit ids, register reuse, linear-time passes; a unit too big for LLVM stays interpreted | Rule 5 | `docs/plan.md` rule 5, ir.md 1.1 to 1.2 |
| Every instruction carries a `StmtId` (span, statement id) | Stepping, coverage, DAP, stack traces | `docs/plan.md` "Designed for the GUI", ir.md 1.1 |
| Constraints are runtime descriptors on every composite; static ones are pool constants | Rule 1 | `docs/plan.md` rule 1, ir.md 1.3 |
| Registers are untyped u64 cells; addresses `[space:3 | index:29 | offset:32]` over eight spaces; out-of-object access is an ICE | Memory model | `docs/decisions.md` "IR and interpreter (M2)", ir.md 1.4 |
| Logic arrays are bit-planes; BIT/BOOLEAN elements use the std_ulogic codes of '0'/'1' | Shared plane ops (orchestrator, 2026-10-18) | `docs/decisions.md` "Interpreter: BIT/BOOLEAN codes" |
| Constant aggregates are compact (`ConstData` run-length / `others`) | Rule 12 | `docs/plan.md` rule 12 |
| Natives named by `lib/NATIVE.md` symbol; `'image` is `yavs.attr.image` | One symbol table | `docs/decisions.md` "IR and interpreter", `lib/NATIVE.md` |
| Foreign calls without libffi through `yavs-ffi`; arrays as C-layout copies, records as aligned structs | ir.md 1.7 | `docs/decisions.md` "IR and interpreter" (Foreign arrays) |
| Verifier checks register liveness across suspension within a block only | Cross-block needs dataflow (JIT tier) | `docs/decisions.md` "IR and interpreter" |
| Time resolution is a runtime setting (`Bindings::resolution`), never baked into IR | Rule 7 | `docs/plan.md` rule 7 |
| Coverage items identified by hierarchical path plus span hash (`CovInfo`); instrumentation is ordinary IR | Rule 10 | `docs/plan.md` rule 10 |

Module-local decisions go in this section (and README "Stable"/"Changes") and ir.md section 9 in
the same commit (root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- **Breaking the IR breaks three crates.** `README.md` "Stable" lists what producers rely on;
  record any change under "Changes" (an exhaustive `match` on `ForeignArg` or `Inst` needs new
  arms in `yavs-interp` and `yavs-lower`).
- Verifier errors are ICEs of the producer: never relax the verifier to make lowering pass
  without an ir.md note.
- The descriptor format and the fat pair `[data, dope]` for unconstrained results are shared with
  `yavs-lower`, `yavs-interp` and `yavs-elab`; change all of them together.
- Element order inside logic leaves: row-major position `p` is kernel offset `n - 1 - p`
  (`docs/decisions.md` "Elaboration"); lowering of non-static indices must match
  `NetGraph::select`.
- Heap addresses carry no generation; the interpreter quarantines deallocated slots instead
  (#81, `docs/decisions.md`).

## 5. How to work here

```sh
CARGO_BUILD_JOBS=6 cargo test -p yavs-ir -p yavs-interp     # IR tests plus end-to-end designs on the interpreter
cargo test -p yavs-lower -p yavs-elab                       # producers
```

- New instruction: add it to `inst.rs`, the verifier, the dump, the interpreter (`yavs-interp`
  `exec.rs`) and a test in both crates; note it in README "Changes".
- No diagnostic codes: verifier failures surface as `E9999` ICEs of the producer.

## 6. Coordination

- Requests to IR/interp go in `crates/yavs-ir/README.md` "Requests" with a dated answer.
- Open issues (snapshot 2026-10-18, `gh issue list -R ru551n/yavs --label crate:ir`): #79
  unimplemented natives and `Elab*` instructions.
