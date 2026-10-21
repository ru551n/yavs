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
| A process that runs serially (inline batch: always at `--threads 1`; the serial phase of a pooled batch when no parallel process wrote or failed) writes its reports and `OUTPUT` text at once to the `LiveSink` an observer offers (`Observer::live_sink`), in commit order; later processes of a batch that failed stop running; shuffled runs and parallel processes keep commit-at-suspension. The offering observer is skipped for those effects at commit, the others still get them then (#483) | Output of a long non-waiting process must appear while it runs and survive a kill; bytes unchanged | `SimControl::run_batch`, `Ctx::report` |
| Ready processes run in parallel (rayon); they read only current-cycle values and write their own driver queue and effect buffer | No data races without `unsafe` | `docs/plan.md` "Parallel kernel design", DESIGN.md "Parallel evaluation" |
| Effects (report, assert, textio `OUTPUT`, `stop`/`finish`, force) are committed in one fixed batch order: elaboration order at initialisation; then one-suspension-point processes in elaboration order, then `several_waits` ones in reverse (concurrent procedure calls first, #281), signal-resumed before timeout-resumed; a process a signal resumes after earlier resumptions (`ProcMeta::resumed`) runs after the concurrent calls, deeper instances first (`ProcMeta::depth`), then in elaboration order (#417, #462) | `--threads 1` and `N` byte-identical; the majority of the reference simulators per case (#252, user 2026-10-19) | `docs/plan.md` "Parallel kernel design"; root `AGENTS.md` rule 3; DESIGN.md "Parallel evaluation"; `docs/decisions.md` "Process order within a cycle" |
| The update phase of independent signals (no port, no composite group, no force or deposit this cycle) runs in parallel, one signal per task, errors taken in signal id order; ports, groups and the commit phase stay serial (#103) | Signals cross no other state; ports cross signals in level order, and the commit is small next to the update | DESIGN.md "The cycle" |
| Processes touching non-protected shared variables, protected types or disk files are `serial` through M3 (user decision; files: #82); serial is `ProcFlags::serial` or `Process::serial` | No locks needed yet; the process knows what it touches | `docs/decisions.md` "Kernel round 2" (Protected types), "Kernel round 3", DESIGN.md |
| No kernel user-file streams (K6 dropped): the interpreter reads and writes disk files directly | Write, close, reopen-for-read in one delta; serial callers keep it deterministic | `docs/decisions.md` "Kernel round 3" |
| `--shuffle` permutes execution order with a seeded splitmix64, commit order fixed | Expose order-dependent testbenches | `docs/plan.md`, DESIGN.md |
| `SimControl` is the only way to drive simulation (CLI, DAP, VHPI) | GUI-ready design | `docs/plan.md` "Designed for the GUI" |
| Per-element flat event arrays over whole signals, no per-scalar objects | Rule 2 | `docs/plan.md` rule 2, `docs/decisions.md` "Kernel crate and process interface" |
| Logic signals stored as bit-planes, everything else as bytes (K7); BIT/BOOLEAN implicit signals too ('0'/'1' codes, #185) | User, 2026-10-18 | `docs/decisions.md` "Kernel round 2", "Packed logic vectors", "Kernel round 3" |
| A composite resolved as a whole keeps its per-field leaves (logic on planes) and is resolved as one group, not stored as one byte leaf (#83) | Bit-planes everywhere (user, 2026-10-18) | `docs/decisions.md` "Kernel round 3", DESIGN.md "Segments" |
| `'delayed` starts at its source's initial effective value, taken at initialisation after the port values are settled (`settle`), not the declared default the builder saw | A port's initial value is its actual's (LRM 14.7.5.2); three of the four references (catalogue `delayed-in-port-init`, #413) | DESIGN.md "The cycle" |
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
| `Config::skip_unchanged`: a zero-delay assignment of the value a sole-source, unobserved signal already has is not queued, but its delta still happens (`empty_delta`); later assignments of the run to the driver put it back first | Most RTL assignments change nothing; delta numbers in reports must not move | `docs/decisions.md` "Skipped unchanged assignments", DESIGN.md "The cycle" |
| A resident process that waited again (`Ctx::wait_on_again`) and only scheduled drivers is committed in the inline batch loop right after it ran (`ProcMeta::ready` cleared, the commit loop skips it); `ProcState::timed_out`/`known_edge` are set when the process runs (`resumption`), not in `wake`; `update` applies the due entries of one independent signal in place when they are one signal in driver order; processes with several suspension points wake into `ready_several`, the others into `ready` with `ready_mono` telling `order` to skip | The same work as before with the slot still in the cache and without the job lists, sorts and second passes of the general path; `ready`-order and queue-order effects are removed by the sorted queue and the order key (#440) | `src/kernel.rs` `run_batch`, `update`, `wake`; `docs/decisions.md` "Kernel round 9" |
| Process slots are `Mutex<Slot>` but the kernel thread reaches them with `get_mut` (no lock) everywhere but inside a parallel batch and in `update_independent` | An uncontended lock and unlock was 14% of a 4000-process single-clock design at `--threads 1` | `src/kernel.rs` `slot_mut` |
| `Observer::wants_phase` and `wants_value_changes` (default yes) tell the kernel which callbacks an observer reads; `SimControl::subscribe` caches the union and `phase`/the value-change loop do nothing for a callback nobody wants | A plain run has two observers (report printer, shared-heap collector) that read almost nothing; the collector's mutex was locked five times per delta (#440) | `src/kernel.rs` `phase_mask`, `watch_changes` |
| A scalar `std_ulogic` assignment has its own entry (`Ctx::assign_code`, one-element value kept in `ProcState::one`), the edge of a logic element one (`Ctx::edge`), and the reference check is `inline(always)` into the interpreter's reads | A `SigRef` built just before a call was passed through memory and its 16-byte load waited on the narrower stores (store forwarding); the list and value allocations of an assignment cost more than the kernel work (#440) | `src/process.rs` |
| `Ctx::assign_code` takes a fast path (`assign_code_fast`) for a zero-delay one-element assignment to a driver with nothing pending and nothing skipped: the transaction is appended without a waveform list, a value buffer or the preemption scan; anything else (errors included) runs the general path | The general path took ~1250 instructions behind each assignment of a clocked process (target, ownership, skip test, buffer refill, schedule layers); with nothing pending there is nothing to preempt (#440) | `src/process.rs` |
| A process that waits for the same signals twice in a row without a timeout has resident waiters (`ProcMeta::perm`, `Waiter::perm`): `take_woken` keeps them, the commit registers nothing for such a wait. Wake order within one signal is the order of first registration | Removing and re-adding a waiter per activation (list retain, push, realloc) was ~110 instructions of every clocked process activation (#440); the batch order is sorted anyway, only `--shuffle` permutations of resumed processes change | `src/kernel.rs` `run_batch`, `src/signal.rs` |
| `Observer::phase_after_serial(phase)`: the kernel calls such an observer's `phase` only in a cycle after which a serial process ran (`SimControl::serial_ran`); each observer has its own phase masks (`obs_phases`). The shared-heap collector uses it for `NextCycle` | Only serial processes allocate in the shared heap, and the collector's mutex plus the hook made up a few percent of a delta cycle with one tiny process (#440) | `src/kernel.rs` `phase`, `yavs-interp` `SharedGc` |
| The wake scan asks an interned trigger id (`Waiter::tid`) instead of the process table; a one-signal update takes no job list; per-process state (`waveforms`, `sens`, `scheduled`) is inline | Each saved a dependent load or an allocation per activation or per delta (#440) | `src/kernel.rs`, `src/process.rs` |
| A process a signal resumed for its edge trigger (no reset, not the legacy form) has `ProcState::known_edge` for that activation: `Ctx::edge` of that element and direction answers true without reading the signal. Timeouts and the initial run clear it; with `Config::edge_triggers` off it is never set | The kernel only wakes such a process when the edge holds, so the interpreter's `rising_edge` test (~160 instructions) repeated the kernel's answer (#440) | `src/kernel.rs` `wake`, `src/process.rs` `edge` |
| Dirty ranges that continue the last one are merged (`Signal::push_dirty`), and `compute_driving` starts at the last segment or its neighbour (`seg_hint`) | One process per element of a vector made one range, one segment search, one diff and one event per element (#440) | `src/signal.rs` |
| A signal whose waiters are all resident and wait for the whole signal keeps its wake list (`Waiters::cache`: registration, process and trigger id per waiter); `cycle` wakes from it without a scan, holding back the waiters of a trigger that does not hold. A waiter added, or a process that leaves its resident wait (`run_batch`, over its previous sensitivity), drops it. Signals of 9 to 1024 elements have a waiter list per element | The scan of thousands of resident waiters per clock event (range test, liveness, hold, push) was a few percent of a clocked design and the 8-waiter chunk scan a tenth of a delta of a chain of one-process stages (#440) | `src/signal.rs` `Waiters`, `src/kernel.rs` `cycle`, `run_batch` |
| A zero-delay one-element logic assignment to an idle driver is a `Waveform::quick` (no `Txn`, no value buffer); code that reads or edits `Waveform::pending` must call `spill`/`idle`. A zero-delay assignment of a vector to an idle driver pushes its one `Txn` directly | The transaction list entry (80 bytes), its buffer and the schedule layers were a quarter of the kernel cost of an assignment (#440) | `src/signal.rs` `Waveform`, `src/process.rs` `assign_code_fast`, `assign` |
| A zero-delay assignment of at most 64 logic elements to an idle driver is a `Waveform::vquick` (plane words, no `Txn`, no value buffer; `spill`/`idle` as for `quick`); on a sole source of a plain signal `apply_driver` writes the driving value too and lists the run in `Signal::fast_dirty` (only the effective value is left to update). `Ctx::assign_blocks` takes the plane blocks as they are in memory (the layout of `Ctx::read_into`) so the caller builds no value. The per-element time stamps hold the time plus one (0: never), so a fresh array is all zeros | The `Txn`, its buffer, `apply_txn`, the schedule layers and the per-segment `compute_driving` were most of the kernel cost of a 16-bit register assignment; the interpreter built a heap `LogicVec` per assignment; 32 bytes of stamps per element were filled at elaboration of a large memory (#440) | `src/signal.rs` `Waveform`, `apply_driver`, `src/process.rs` `assign_planes_fast`, `assign_blocks` |
| `Ctx::wait_on_again(signal, range)`: a wait on exactly the range the last wait registered, with no timeout and nothing else named, leaves `ProcState::sens` empty and sets `ProcState::again`; `run_batch` takes it for the wait it already registered (`ProcMeta::perm`) and, when the process names more after it (another signal, a timeout), rebuilds `sens` as `sens_prev` followed by the rest; a process that only waited again and scheduled drivers skips the rest of the commit. `Ctx::assign_code_now` is `assign_code` for one element at zero delay without the error paths (false: use `assign_code`) | The registration of a static sensitivity after every activation was 5% of a clocked design (`SmallVec` push, compare with `sens_prev`, the waiter bookkeeping) | `src/process.rs` `wait_on_again`, `src/kernel.rs` `run_batch`; `docs/decisions.md` "Compiled clocked RTL processes, round 2" |
| `finish_signal` finishes one dirty logic element with one source by comparing codes (`Signal::finish_one`); `compute_driving` gathers a run of one-element, one-driver segments into plane words (`gather_ones`); `update_effective` stamps and lists runs of changed elements; `push_dirty` also merges a range that ends where the last starts | One process per element of a vector touched each element four times per cycle with generic block code (#440) | `src/signal.rs` |
| `SimControl::run_until` reuses the `Tn` that `last_delta` computed when another delta follows (`known_next`); the buffers of a batch and the worklists of `update` are kept between cycles | `next_time` and a list allocation per delta were a few percent of a chain of one-process stages (#440) | `src/kernel.rs` |
| A logic signal of at most 2^20 elements keeps one byte per element, the `std_ulogic` code of its current value (`Signal::codes`); `Signal::cur_code`, `mirrored_code`, `Ctx::elem_code`, `first_code`, `edge` and the unchanged-assignment test read it. Every write of `cur` must refresh it: `update_effective` (`refresh_codes` of the span that changed), `finish_one`/`finish_quick` (one byte), `sync_codes` after `cur` is replaced as a whole. Debug builds compare each read with the planes | A read of one element out of the planes was ~30 instructions (two storage dispatches, four shifts, two bounds checks), a few percent of a clocked design (#440) | `src/signal.rs` |
| A quick transaction on an element whose driver is its only source (`DriverSlot::shared` false) is finished when it is applied (`Signal::finish_quick`: driving value, effective value, event, last value) for the first four per signal and cycle, when the signal has no group, guard, layer or resolution function to wait for; later ones go through the dirty ranges. `finish_signal` then sorts and joins the runs (`canonical_changed`) so the list does not depend on the order the drivers applied in. `Signal::end_apply` resets the count | A chain of one-process stages has one such transaction per delta; the dirty range, segment lookup and driver value read-back were ~100 instructions of it (#440) | `src/signal.rs` `apply_driver`, `src/kernel.rs` `finish_signal` |
| The runs of elements that changed in a cycle (`Signal::changed`) are listed only for a signal with a waiter or when an observer wants value changes (`Signal::watched`, set by the commit that registers a waiter and by `subscribe`); otherwise `Signal::nruns` counts them for `Stats::events`. Anything new that reads `changed` must be satisfied by that | A vector written by hundreds of processes that nothing waits on made thousands of runs per cycle (#440) | `src/signal.rs` `note_run` |
| GC safepoint: the commit phase (no `Ctx` exists) | A deadlock class seen in other simulators | DESIGN.md "Parallel evaluation" |
| Small per-signal and per-driver data is stored inline (`SmallVec`, `union` feature): a `Buf` of up to 16 bytes or one logic block, the four per-element stamp arrays in one buffer, one waiter, one segment, one pending transaction, the scratch range lists; a signal of one chunk has one waiter list and a signal nothing waits on has none; the scratch list of one assignment is a local; a process name is stored once (`Hook::process_name`); `Builder::process_boxed` takes an already boxed process | A scalar signal cost about 25 allocations (#421); fewer allocations are less memory and elaboration time | `DESIGN.md` "Data layout", `docs/decisions.md` "Memory of designs with many instances" |
| `update_effective` of a range on a block boundary of a plain logic signal works on whole plane words (`Signal::update_planes`: changed mask by XOR, masked blend of `last_value` and `cur`, then stamps, runs and codes); other ranges, bytes and layered signals take the element helpers. `Buf::write_to` of at most 64 logic elements reads one block | The per-chunk `diff_chunk` and two `select`s were a third of the update of a 16-bit register (#440) | `src/signal.rs` `update_planes`, `src/value.rs`; `docs/decisions.md` "Kernel round 10" |
| One element of plane storage is written with a masked blend of the code's four plane masks (`put_plane_code`, a 16-entry table); `finish_quick` takes the old code from `Signal::codes` and writes the driving value, `cur` and `last_value` planes directly | Four shift-and-merge sequences per write, four writes per finished quick transaction (#440) | `src/value.rs`, `src/signal.rs` `finish_quick` |
| A delta whose due entries are all drivers of one independent signal from `queue.next_delta` (no force, deposit, implicit signal or later-time entry) is updated by a short path in `update` (`one_independent`; quick transactions through `Signal::apply_quick`); `changed_sigs` is walked in place; one changed element whose list holds one resident waiter, with an empty wide list, wakes it directly (`Waiters::sole_resident`). Anything else takes the general path | The work-list take and give-back, the independent list and the `take_woken` call and closures were a fifth of the kernel cost of a delta of a chain of one-process stages (#440) | `src/kernel.rs` `update`, `cycle`; `src/signal.rs` |
| The inline batch (`run_batch` without the pool) has one unwind guard for the whole batch and keeps the index of the running process, so a panic is still the internal error of the process that panicked (`process_panic`); a batch whose processes were all committed right after they ran skips the commit loop | A `catch_unwind` closure per process and a second walk of the batch per delta (#440) | `src/kernel.rs` `run_batch` |

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
