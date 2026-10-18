# AGENTS.md: yavs-interp

Read the root `AGENTS.md` first. This file is for an agent about to change
`crates/yavs-interp`. The design is `docs/design/ir.md` section 3 (dispatch, kernel `Process`,
diagnostics and stack traces, debug hooks, thread safety, elaboration-time evaluation) and
section 9; the interpreter API and the Requests log are in `crates/yavs-ir/README.md`.

## 1. Purpose and boundaries

- Executes `yavs-ir` programs: `InterpProcess` implements `yavs_kernel::Process` (heap frame
  stack, waits inside procedures), `call`/`call_shared` run code without a kernel (elaboration
  time), natives (std.standard, textio, files, env, implicit operations, `'image`/`'value`,
  `std.reflection`), VHPIDIRECT foreign calls with marshalling, generational heaps with
  precise-root GC, protected objects, and debug hooks in the style of commercial simulators.
- **The interpreter is the semantic reference** (rule 5): the JIT tier must match it exactly.
- Does not: lower HIR, build the hierarchy, schedule processes (kernel), or format values itself
  where `yavs-std` has the function (it marshals and calls `yavs-std`).
- Depends on: `yavs-ir`, `yavs-kernel`, `yavs-logic`, `yavs-std`, `yavs-ffi`, `yavs-diag`.
  Called by: `yavs-elab` (builds processes, elaboration-time calls), `yavs-lower` (tests).

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | `Runtime::new(Arc<Program>, Bindings)` (fails early on unknown natives/unbound foreigns), `Bindings` (foreigns, `revision`, `resolution`, `sources`), `Limits`, `InstData`/`InstSig`, `ProcDriver`, `SharedState`, `InterpProcess` (`new`, `with_debug`), `proc_flags`, `call`, `call_shared`, `call_values` (composite arguments and results as data and descriptor bytes, `CallVal`), `call_native` (a native on such values, elaboration-time predefined operations, #220), `resolve` |
| `src/exec.rs` | `Machine`: instruction dispatch, checked arithmetic via `yavs-std`, checks with reference-simulator-compatible texts, signal ops mapped 1:1 to `Ctx`, faults (`rt_err`, `ice`) |
| `src/heap.rs` | `Heap`: generational handles `{tag: u8, index: u24, generation: u32}` in a u64 (private `PRIVATE`, shared `SHARED`), mark and sweep from explicit roots and pinned slots, quarantine of deallocated slots (`recycle`) |
| `src/native.rs` | Native dispatch keyed by `lib/NATIVE.md` symbol |
| `src/implicit.rs` | `yavs.implicit.<family>.<type>[.<form>]` operations |
| `src/textio.rs` | std.textio and TEXT file natives, `FILE_INPUT`/`FILE_OUTPUT`, std.env glue |
| `src/binfile.rs` | `read`/`write` on files of scalars and arrays of scalars (on-disk format: decisions.md "Binary files") |
| `src/env.rs` | 2019 std.env: call paths (`Sources`), the VHDL assert state (`Asserts`, cycle-deterministic), time_record |
| `src/foreign.rs` | VHPIDIRECT marshalling (scalars, arrays, records, access, copy-back) |
| `src/reflect.rs` | `std.reflection`: mirror objects and the 123 mirror methods; a test checks every NATIVE.md reflection row is implemented |
| `src/debug.rs` | `DebugSession`, `Debugger`, `LineBp`/`HitCond`, `SignalBreak`, `FrameView`/`LocalView`, `ProcState`, `Stop`/`StopReason`, `Resume`, `format_scalar` |
| `tests/sim.rs`, `checks.rs`, `debug.rs`, `foreign.rs`, `textio.rs` | End-to-end designs at 1 and 4 threads, check texts, debug sessions, marshalling, textio/file/env; helpers in `tests/common/mod.rs` |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Interpreter is the semantic reference; `--threads 1`/`N` identical | Rules 3 and 5 | `docs/plan.md` rule 5; root `AGENTS.md` rule 3 |
| A signal reference of several parts is mapped part by part onto `Ctx` in one step: reads concatenate, `'event`/`'active` any part, `'last_event` the most recent, drives split each waveform value and use each part's own driver slot; the parts' table lives in temporary memory | #71, LRM 16.2.4; no kernel change, same delta | ir.md 1.8 |
| Memory model: untyped u64 registers, 8 address spaces, every data-dependent access bounds-checked; out-of-object is an ICE, freed object a runtime error | Fail early, no UB | `docs/decisions.md` "IR and interpreter (M2)" |
| Temporaries live until the next statement of their frame | Unconstrained results survive return without copy | same |
| Natives by NATIVE.md symbol; undefined symbol fails `Runtime::new`, defined but unimplemented is a runtime error naming it | Fail early | same; `lib/NATIVE.md` |
| Heap: generational handles, precise roots (`Func::roots`), mark and sweep at suspensions; skip collection rather than be unsound | Rule 6 | `docs/plan.md` rule 6, ir.md 1.6 |
| Deallocated heap slots are quarantined until a suspension finds no address to them in the frames and temporaries; such addresses also keep objects alive in a collection | Addresses have no generation (#81) | `docs/decisions.md` "IR and interpreter" |
| Disk files written directly by the owning process, every caller of a file native serial; `STD_OUTPUT` through `Ctx::output` | Write/close/reopen in one delta; `--threads` independent; ordered with `report` (orchestrator, 2026-10-18; serial: #82) | `docs/decisions.md` "Interpreter: BIT/BOOLEAN codes and file writes", "Kernel round 3" |
| BIT/BOOLEAN elements of logic arrays use std_ulogic codes '0'/'1' (2/3); scalar BIT/BOOLEAN are positions 0/1 | Shared plane ops | same; `src/implicit.rs` docs |
| Foreign arrays as C-layout copies left to right, logic elements one code byte, records as aligned structs, access as pointer; copy-back range-checked | The reference simulator convention plus yavs additions | `docs/decisions.md` "IR and interpreter" |
| Debugging like the reference simulators: conditional line breakpoints with hit counts, step in/over/out, global step, signal breakpoints, process table; stops inside a process need `--threads 1` | User requirement | `docs/decisions.md` "IR and interpreter", ir.md 3.4 |
| Reflection: TIME mirror values in resolution-limit units; unit indices 1-based; element/literal indices 0-based; names upper case; case-insensitive lookups (user, 2026-10-18) | The test runner `record_dict_pkg`, the reference simulator | `docs/decisions.md` "Reflection: ..." (two sections) |
| textio READ/WRITE move the line's bytes to the new object (`Heap::take`, `alloc_from`) instead of copying; a READ drops the consumed prefix through the object's front offset, compacted once it is the larger half. The handle, deallocation and renumbering `1 to len` stay as before | N reads or writes on one line are O(N) (#196) | `src/textio.rs` `line_edit`, `src/heap.rs` |
| Assert state: each process sees the state at the start of its cycle plus its own changes; changes merge in process order at the next cycle | Rule 3 with parallel processes (#99) | `docs/decisions.md` "IR and interpreter" |
| Check and overflow texts match the reference simulator (`value 101 outside of PCT range 0 to 100 for variable P`, ...) | Strict difftest | `crates/yavs-ir/README.md` Requests answer 4 |
| `now` without a kernel (elaboration-time calls, #77) returns 0 fs | LRM 14.7.5.2: simulation starts at 0; the external test runner's factories read it during elaboration | `docs/decisions.md` "Elaboration" |
| Runtime errors stop at once (bound, overflow, null, delta limit); a report at the stop severity ends the process | Error motto; exit severity (user, 2026-10-18) | `docs/plan.md` "Error-handling motto", `docs/decisions.md` "Runtime: default exit severity" |

Module-local decisions go in this section and ir.md section 9 in the same commit (root
`AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- **Determinism:** nothing may depend on thread scheduling; shared heap and shared variables are
  only touched by serial processes (kernel `ProcFlags::serial`, `proc_flags`, and
  `InterpProcess`'s `Process::serial`, which the kernel ORs in so no builder can drop it).
- **Checked i64 everywhere** through `yavs_std::arith`; never wrap, clamp or saturate.
- **Generational handles:** a stale access value is a clean runtime error. Heap *addresses*
  carry no generation: deallocated slots wait in quarantine until a suspension finds no address
  to them (`Machine::pinned`), so never hand out a freed slot any other way; the GC follows
  descriptors (`heap::handles_in`).
- A runtime error's `SimError::trace` is the VHDL stack as `StmtId`s, innermost first; keep it
  correct when adding call paths (natives calling back into IR).
- `reflect.rs` has a test that NATIVE.md reflection rows and the implementation agree: add both.
- `std.env.finish`/`stop` must end the calling process at once (#4).
- Tests build programs by hand (`tests/common/mod.rs`); keep them independent of `yavs-lower`.

## 5. How to work here

```sh
CARGO_BUILD_JOBS=6 cargo test -p yavs-interp
cargo test -p yavs-ir -p yavs-interp -p yavs-lower -p yavs-elab   # after IR or API changes
python3 tools/difftest/difftest.py --candidate yavs              # end to end against reference-simulator refs
```

- A new native: row in `lib/NATIVE.md` (see `lib/AGENTS.md`), implementation in `native.rs` /
  `implicit.rs` / `textio.rs` / `env.rs` / `reflect.rs`, a test in `tests/`.
- No codes of its own: runtime errors are kernel `SimError`s with `ErrorKind::Runtime` (`E3009`)
  or `Internal` (`E9999`); the message text is what the user sees, so keep it reference-simulator-compatible.

## 6. Coordination

- Requests from lowering and elaboration are logged with answers in `crates/yavs-ir/README.md`
  "Requests".
- Open issues (snapshot 2026-10-18, `gh issue list -R ru551n/yavs --label crate:interp`): #77 fold
  calls and aggregates at elaboration (E2106); #79 unimplemented natives and `Elab*`; #97
  reflection runtime. Older unlabeled: #4
  `finish`/`stop` end the process at once; #9 array signals and
  constants in instance data; #10 `FToIRange` text.
