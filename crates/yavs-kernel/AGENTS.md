# AGENTS.md: yavs-kernel

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-kernel`.
The design is `crates/yavs-kernel/DESIGN.md` (data layout, the cycle, the commit protocol, how
the IR plugs in, error handling, open questions); read it before editing.

## 1. Purpose and boundaries

- The LRM 14.7 simulation cycle: time with runtime resolution, signals and drivers (transport and
  inertial preemption per element), resolution, uncollapsed ports with conversions, implicit
  signals, guarded signals and null transactions, force/release/deposit, postponed processes,
  delta limit, **parallel process evaluation with a deterministic commit**, and `SimControl`
  (run/step/interrupt, observers on an event bus, phase callbacks).
- Does not: know the IR, VHDL types or the heap. Processes are opaque `Process` trait objects
  (`yavs-interp`, later `yavs-llvm`); values are bytes or bit-plane logic described by
  `SignalType`. It does not print anything (the CLI's `Observer` prints reports).
- Depends on: `yavs-logic`, `rayon`. Called by: `yavs-interp` (implements `Process`, uses `Ctx`),
  `yavs-ir` (types like `StmtId`), `yavs-lower`, `yavs-elab` (`Builder`), `yavs` (`Config`,
  `SimControl`, `Observer`; runtime errors are rendered by `yavs_elab::Elaborated::runtime_diagnostic`). Planned subscribers: `yavs-wave`, `yavs-vhpi`, DAP.

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | Re-exports (the public API is exactly this list) |
| `src/builder.rs` | `Builder` (signals, resolved signals, ports with `Converter`/`PortConv`, drivers, implicit signals, guards, processes with `ProcFlags`: `postponed`, `serial`, `several_waits`), `PortMode` |
| `src/kernel.rs` | `SimControl` (`run_until`, step, `subscribe`, `now`, `delta`), `Config` (`threads`, `shuffle`, `max_deltas` 10000, `resolution`, `stop_severity`), the cycle, run and commit phases, `Observer`/`Hook`/`Phase`, `Hook::with_process` (a suspended process, VHPI variables), `Hook::assign` (a transaction on a driver, VHPI foreign architectures), wake-ups (`WakeId`, `Hook::wake_at`/`cancel`, `Observer::wake`), `Report`, `RunStatus`/`EndReason`, `Interrupt`, `Stats` |
| `src/process.rs` | `Process` trait (`run`, `serial`, `several_waits`, `as_any`), `Ctx` (reads, `read_into` without allocation, `assign`, `disconnect`, waits, `report`, `output`, `stop`/`finish`, `force`/`release`, `deposit` for VHPI foreign calls), `ProcId`, `Severity` |
| `src/signal.rs` | Per-element event arrays, drivers, transactions, preemption, resolution (`Resolver`, `Resolution`), `Range`, `SigRef`, `DelayMode`, `ForceMode`, `GuardKind` |
| `src/value.rs` | `SignalType` (`Elem::Bytes(n)` or `Elem::Logic`, `len`), `Value`, `ValueRef`, `LogicVec` |
| `src/time.rs` | `Time` (checked fs arithmetic), `Unit`, `TimeError` |
| `src/error.rs` | `SimError` (boxed `ErrorInfo { kind, time, delta, process, trace }`), `ErrorKind` (`code()`: `E3001`-`E3010`, `Internal` = `E9999`), `StmtId`, `exit_status()` |
| `tests/scenarios.rs`, `tests/kernel_asks.rs`, `tests/determinism.rs` | Cycle scenarios; K1 to K7 asks of ir.md; 1 vs 8 threads and shuffle seeds on 1000+ processes |
| `examples/bench.rs` | `cargo run --release -p yavs-kernel --example bench -- 10000 <work>` |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| A `report` in a resolution function or port conversion function during the run is captured (`callout`, a thread-local the interpreter pushes to) and committed after the update phase as a `Report` with `process: None` and a `context`, in port-conversion, group, then signal order; at initialisation it prints `(init)` as before. A `failure` there ends the run like any other (#372) | The functions run outside any process, so the cycle's time, delta and severity handling must come from the kernel; the order does not depend on `--threads` | `src/callout.rs`, `SimControl::commit_callouts` |
| A force or release that changes a signal's effective value is an event even when a later force of the same cycle takes it back (`Signal::force_ev`); forcing the value it already has is none (#382) | The four reference simulators agree on the round trip, split on the same value (catalogue `force-twice`) | `apply_op`, `Signal::force_events` |
| Ready processes run in parallel (rayon); they read only current-cycle values and write their own driver queue and effect buffer | No data races without `unsafe` | `docs/plan.md` "Parallel kernel design", DESIGN.md "Parallel evaluation" |
| Effects (report, assert, textio `OUTPUT`, `stop`/`finish`, force) are committed in one fixed batch order: elaboration order at initialisation; then one-suspension-point processes in elaboration order, then `several_waits` ones in reverse (concurrent procedure calls first, #281), signal-resumed before timeout-resumed | `--threads 1` and `N` byte-identical; the majority of the reference simulators per case (#252, user 2026-10-19) | `docs/plan.md` "Parallel kernel design"; root `AGENTS.md` rule 3; DESIGN.md "Parallel evaluation"; `docs/decisions.md` "Process order within a cycle" |
| The update phase of independent signals (no port, no composite group, no force or deposit this cycle) runs in parallel, one signal per task, errors taken in signal id order; ports, groups and the commit phase stay serial (#103) | Signals cross no other state; ports cross signals in level order, and the commit is small next to the update | DESIGN.md "The cycle" |
| Processes touching non-protected shared variables, protected types or disk files are `serial` through M3 (user decision; files: #82); serial is `ProcFlags::serial` or `Process::serial` | No locks needed yet; the process knows what it touches | `docs/decisions.md` "Kernel round 2" (Protected types), "Kernel round 3", DESIGN.md |
| No kernel user-file streams (K6 dropped): the interpreter reads and writes disk files directly | Write, close, reopen-for-read in one delta; serial callers keep it deterministic | `docs/decisions.md` "Kernel round 3" |
| `--shuffle` permutes execution order with a seeded splitmix64, commit order fixed | Expose order-dependent testbenches | `docs/plan.md`, DESIGN.md |
| `SimControl` is the only way to drive simulation (CLI, DAP, VHPI) | GUI-ready design | `docs/plan.md` "Designed for the GUI" |
| Per-element flat event arrays over whole signals, no per-scalar objects | Rule 2 | `docs/plan.md` rule 2, `docs/decisions.md` "Kernel crate and process interface" |
| Logic signals stored as bit-planes, everything else as bytes (K7); BIT/BOOLEAN implicit signals too ('0'/'1' codes, #185) | User, 2026-10-18 | `docs/decisions.md` "Kernel round 2", "Packed logic vectors", "Kernel round 3" |
| A composite resolved as a whole keeps its per-field leaves (logic on planes) and is resolved as one group, not stored as one byte leaf (#83) | Bit-planes everywhere (user, 2026-10-18) | `docs/decisions.md` "Kernel round 3", DESIGN.md "Segments" |
| `'transaction`, `'stable` and `'quiet` may have several sources (`Builder::implicit_source`): one implicit signal for a record prefix; `'delayed` has one | LRM 16.2.3 over all scalar subelements, #215 | DESIGN.md "The cycle" |
| Uncollapsed ports, conversions in the update phase without an extra delta (K1) | Rule 3 | `docs/plan.md` rule 3, `docs/decisions.md` "Kernel round 2" |
| Port edges are lists of parts on each side (`Builder::port_parts`); a converter maps all source parts to all target parts once per cycle, at the part the level-ordered worklist reaches last | Record ports with conversion functions in the same delta, deterministic (#74) | DESIGN.md "The cycle", ir.md 4.1, `docs/decisions.md` "Multi-part signal references and port converters" |
| Forces and deposits queued and applied next cycle | VHDL-2008 force semantics | `docs/decisions.md` "Kernel crate and process interface" |
| Delta numbering: init is delta 0 at 0 fs; default delta limit 10000; delays truncated to the resolution | Clear, reference-simulator-compatible report times | same |
| Exit status: 101 internal (also a caught panic in process code), 1 otherwise; `Idle` is terminal like `stop`/`finish` | Error motto | same; DESIGN.md "Error handling" |
| `SimError` is boxed and latches | Hot-path `Result` one word | `docs/decisions.md` "Kernel round 2" |
| Time is checked i64 fs with a runtime resolution | Rule 7 | `docs/plan.md` rule 7 |
| Observer wake-ups are a separate ordered map, not queue entries; `Tn` includes the first one, so no `Idle` end while one is pending; past is E3005 (#268) | Cheap cancel and nothing to pay with no observer (plan rule 9); VHPI after-delay callbacks and DAP run-to-time | DESIGN.md "Data layout", "The cycle" |
| Observers run serially on the kernel thread between phases | Callbacks and wake-up numbering identical at any thread count | DESIGN.md "Parallel evaluation" |
| Observers reach a process only through `Hook::with_process`, and its state only through `Process::as_any` (the kernel does not know the process tier); every process is suspended then (#309) | VHPI reads and deposits variables; waves can sample them the same way | `docs/decisions.md` "VHPI ABI and symbol export" |
| A process with an edge trigger (`Builder::edge_trigger`) is not resumed by an event that does not make its edge true; `Config::edge_triggers` turns it off (coverage, debugger, foreign callbacks) | Half the activations of a single-clock design do nothing visible (#385, `docs/design/performance.md` R2d) | DESIGN.md "The cycle" |
| GC safepoint: the commit phase (no `Ctx` exists) | A deadlock class seen in other simulators | DESIGN.md "Parallel evaluation" |

Module-local decisions go in this section and DESIGN.md in the same commit (root `AGENTS.md`
rule 24).

## 4. Invariants and pitfalls

- **Determinism across threads** is the check that matters: anything that depends on scheduling
  (hash-map order, thread ids, timing) in the commit phase breaks it. `tests/determinism.rs` must
  stay green at 1 and 8 threads with several seeds.
- `#![forbid(unsafe_code)]` via workspace lints: keep it that way.
- Bad ids or ranges are `ErrorKind::BadRef`, never an index panic; a panic is still caught and
  becomes `Internal` (exit 101, the CLI maps it to 3).
- User files are not kernel state: the interpreter writes disk files directly and their users
  are serial (`Process::serial`); only textio `OUTPUT` goes through `Ctx::output`, ordered with
  `report`. A process that is serial in the IR but built without the flag runs in parallel and
  breaks determinism silently, so `Builder::process` ORs in `Process::serial` (#107).
- A composite resolved as a whole is its usual leaves joined by `Builder::resolved_composite`
  (#83): logic leaves stay on planes, and the resolver sees each source as the concatenation of
  its leaves (leaf order, one code byte per logic element). The parts may be leaf slices (#333:
  one element of an array of resolved records), a signal may be in several groups, which must
  cover it. The parts must share their sources (same owners in the same order, each covering
  the part) and port level; `build` refuses anything else, guarded composites included.
- A new `ErrorKind` needs its own code in `ErrorKind::code` (registry range `E3000`-`E3099`) and
  an entry in the yavs-diag registry table if it opens a new sub-range.

## 5. How to work here

```sh
CARGO_BUILD_JOBS=6 cargo test -p yavs-kernel
cargo test -p yavs-interp -p yavs-elab               # main users; run after API changes
cargo run --release -p yavs-kernel --example bench -- 10000 0
python3 tools/difftest/difftest.py --candidate yavs  # end to end, threads 1 and 4 (tools/difftest)
```

- Add a scenario test with hand-written processes (`tests/common/mod.rs` helpers); a
  thread-sensitive change also gets a case in `tests/determinism.rs`.
- Codes owned: `E3000`-`E3099` (`ErrorKind::code`: E3001 time overflow, E3002 delta limit, E3003
  postponed delta, E3004 invalid assignment, E3005 deadline in the past (wait or wake-up), E3006 bad reference,
  E3007 resolution, E3008 conversion, E3009 VHDL runtime error, E3010 usage), plus `E9999`.

## 6. Coordination

- Kernel API changes ripple into `yavs-interp` (process), `yavs-elab` (builder) and the CLI;
  `docs/design/ir.md` sections 3.2 and 6 describe the seam.
- Open issues (snapshot 2026-10-18, `gh issue list -R ru551n/yavs --label crate:kernel`): none
  on the cycle's threading (#103 done).
  Questions on report order vs the reference simulator: #11, #12.
