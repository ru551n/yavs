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
| `src/ieee.rs` | `ieee::parse`/`native_symbol`: which mangled `ieee` subprograms have a native (`yavs.ieee.<key>`); `yavs-lower` emits the call, `yavs-interp` `ieee.rs` implements it |
| `src/opt.rs` | The optimiser, run per function by `Program::func` on first use (folding, sharing, forwarding, promotion of scalar frame slots to registers, dead definitions); `YAVS_NO_IROPT=1` turns it off; `opt::keep_variables` keeps variables in their slots for a debugger |
| `src/opt/range.rs`, `src/opt/live.rs` | Interval analysis of integer registers (drops provable checks, decided compares and branches, threads branches to single-`CondBr` blocks) and liveness-based removal of dead definitions of multiply-defined registers, both for subprograms that cannot suspend |
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
| A signal reference is two registers: one range of one instance signal, or (`SigParts`) a table of parts with a driver slot each; sliced only with `SigSlice` | Individually associated signal formals across signals (#71) without changing the calling convention | ir.md 1.8, `docs/decisions.md` "Multi-part signal references and port converters" |
| Logic arrays are bit-planes; BIT/BOOLEAN elements use the std_ulogic codes of '0'/'1' | Shared plane ops (orchestrator, 2026-10-18) | `docs/decisions.md` "Interpreter: BIT/BOOLEAN codes" |
| Constant aggregates are compact (`ConstData` run-length / `others`) | Rule 12 | `docs/plan.md` rule 12 |
| Natives named by `lib/NATIVE.md` symbol; `'image` is `yavs.attr.image` | One symbol table | `docs/decisions.md` "IR and interpreter", `lib/NATIVE.md` |
| Foreign calls without libffi through `yavs-ffi`; arrays as C-layout copies, records as aligned structs | ir.md 1.7 | `docs/decisions.md` "IR and interpreter" (Foreign arrays) |
| Verifier checks register liveness across suspension within a block only | Cross-block needs dataflow (JIT tier) | `docs/decisions.md` "IR and interpreter" |
| The optimiser runs lazily per function on the first `Program::func`; the unoptimised code stays in `Program::funcs`. A function's version is fixed at its first use, because frames find their function by id and a pc must keep its meaning | #427 |
| The optimiser never removes a store, a `Stmt` or a `Cover`, never lets a register live across a suspension, forgets memory facts at every `Stmt` (a debugger may write a named local) and drops a check only when it provably passes | Stepping, coverage, DAP, identical runtime errors | #427, ir.md 9 |
| A frame slot becomes a register (`Opt::promote`) only in a `Function` or `Procedure` that cannot suspend and has no nested subprogram, only if every access is a direct `Load` or `Store` of the same width and register class, its address never escapes (`escapes`), no root map covers it (except a handle or address root whose every access is exactly a `Handle` or `Ptr` register, which the collector finds as live registers) and no `Sig`-wide access overlaps it; a slot inside a named local only while `opt::keep_variables` was not called (the CLI calls it for `--dap`, `DebugSession::new` for library users, before any function is optimised). Registers that some path reads before writing get a zero (or the parameter's value) at the entry. Blocks no branch reaches (and every block of a function that may suspend counts as reached) do not count as definitions and are emptied at the end. Last: `Stmt` markers whose statement may not allocate temporary memory are dropped in subprograms without a debugger (`may_allocate` lists the instructions that never do; a new instruction is assumed to allocate), equal single-definition constants and preloaded addresses are merged, and registers are renumbered densely (an instruction operand the instruction does not read, such as the second operand of `Bool Not`, is pointed at a register that exists: the interpreter reads the cell) | Loops loaded and stored the same slots on every pass (a quarter of the executed instructions in the verification-library testbenches); a debugger reads and writes variables in their slots | #444 |
| The storage of a local composite variable of a subprogram that cannot suspend is `Inst::NewFrame` (pinned temporary memory, not a heap object): once per call, no handles in the type, the handle only dereferenced or stored into the variable's handle slot, no debugger | A heap object per call cost an allocation, a copy and collection work; a result pointing into it lives as long as any temporary result (#444) | `docs/decisions.md` "Local composite variables live in the frame's temporary memory" |
| Integer registers get intervals (`opt/range.rs`) and a check is dropped, a compare or branch decided, only when the interval proves it; arithmetic overflow is a runtime error, which bounds every result by `i64`. Definitions of registers defined once are never removed by liveness (`opt/live.rs`): frames start with their constant or address | Concatenations and counters were checked and selected on lengths that cannot be negative (#444) | `docs/decisions.md` "Interval analysis, jump threading and dead register definitions" |
| Array elements of an integer type within 32 bits are 4 bytes (`Ty::elem_size`), scalars 8; `Width::B4` loads sign-extend | Memory and cache of integer arrays (R12, #415) | `docs/decisions.md` "Narrow integer storage" |
| Time resolution is a runtime setting (`Bindings::resolution`), never baked into IR | Rule 7 | `docs/plan.md` rule 7 |
| Coverage items identified by hierarchical path plus span hash (`CovInfo`: statement, `CovKind::Statement` or `Branch(arm)`; path and hash are made by yavs-cov); instrumentation is ordinary IR | Rule 10 | `docs/plan.md` rule 10 |

Module-local decisions go in this section (and README "Stable"/"Changes") and ir.md section 9 in
the same commit (root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- **Breaking the IR breaks three crates.** `README.md` "Stable" lists what producers rely on;
  record any change under "Changes" (an exhaustive `match` on `ForeignArg` or `Inst` needs new
  arms in `yavs-interp` and `yavs-lower`).
- **The IR is serialised (`Program`, `ProgramBuilder`; the compilation cache stores lowered packages, #407).** A new type reachable from them derives `serde::Serialize`/`Deserialize`; derived caches (`Func::opt`, `Program::layouts`) are `#[serde(skip)]` and rebuilt on use. Big byte strings go through the `bytes` module of `program.rs` (a sequence of numbers decodes at a byte per call).
- Verifier errors are ICEs of the producer: never relax the verifier to make lowering pass
  without an ir.md note.
- The descriptor format and the fat pair `[data, dope]` for unconstrained results are shared with
  `yavs-lower`, `yavs-interp` and `yavs-elab`; change all of them together.
- Element order inside logic leaves: row-major position `p` is kernel offset `n - 1 - p`
  (`docs/decisions.md` "Elaboration"); lowering of non-static indices must match
  `NetGraph::select`.
- Heap addresses carry no generation; the interpreter quarantines deallocated slots instead
  (#81, `docs/decisions.md`).
- Precise roots (plan rule 6, #422): a register holding an access or protected handle is
  `RegTy::Handle` (the verifier wants one for `New`, `Deref`, `CallProt`; an `I64` operand
  takes it too), a frame slot holding a handle or a heap address is in `Func::roots`
  (`RootKind::Handle`, `Composite`, `Addr`), and a temporary that holds handles or heap
  addresses is recorded by `Inst::TempRoot`. An emitter that keeps a handle anywhere else
  makes the collection inside an activation free a live object. The optimiser (`opt.rs`)
  never replaces a `Handle` register by another register (`Opt::single` takes `I64` and
  `Ptr` only); replacing one by an `I64` would hide a root.

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
