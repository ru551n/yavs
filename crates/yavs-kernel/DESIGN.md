# yavs-kernel design

The kernel runs the VHDL-2008 simulation cycle (LRM 14.7). It knows nothing about the IR:
processes implement the `Process` trait and values are bytes or bit-plane logic. `SimControl` is
the only thing that drives simulation; the CLI, DAP and VHPI all call it.

## Data layout

- **Ids** are indices in elaboration order: `SignalId`, `ProcId`, `DriverId` (all `u32`). They
  never change during a run, so DAP, VHPI, waves and coverage can store them. Kernel `SignalId`
  order is the order of the elaborator's first leaf of each signal (`NetGraph::loc`, #686).
- **Signal types** (`value.rs`): `SignalType { elem: Elem, len }`. `Elem::Logic` signals store
  `std_ulogic` in yavs-logic's bit-plane layout (four `u64` planes per 64 elements, the same
  `Block` as `yavs_logic::Planes`). `Elem::Bytes(n)` signals store `n` opaque bytes per element
  (integers, enums, reals, record fields once elab splits records into leaves). Values cross the
  API as `Value`/`ValueRef` (`Bytes` or `Logic(LogicVec)`, where `LogicVec = Logic<Planes>`).
  `len == 0` is a valid null array (K5); null ranges are valid everywhere and never overlap
  anything.
- **Storage** (`Buf`): every buffer of a signal (effective `cur`, `driving`, `last_value`,
  driver values, transaction values, force and port layers) is a `Buf` of the signal's kind.
  Element-addressed operations (copy a range, masked select, diff, resolve) work on chunks of 64
  elements with shifts and masks, so a sub-range at any offset is never transposed element by
  element. Per-element metadata (event cycle, active cycle, last event time, last active time)
  are flat arrays over the whole signal (rule 2), the four in one buffer; a signal of more than
  64 elements also keeps the active cycle and time per block of 64 (`blk_active`), written when
  a marked range covers the whole block, and the activity of an element is the later of the two;
  the events its plane-word update finds are records per block (cycle, time, elements whose last
  event it is) instead of element stamps (#519). Small collections
  (a `Buf` of up to 16 bytes or one logic block, one waiter, one segment, one pending
  transaction, a signal's scratch range lists) are stored inline (`SmallVec`): a scalar signal
  and its driver cost no allocation of their own (#421).
- **Sources**: a signal's source list holds process drivers (creation order) and then out-ports
  (`DriverSlot { range, value, off }`). A driver's current value lives in the signal, its
  projected output waveform in the owning process (`ProcState::waveforms`), so a running process
  mutates only its own state. `off` marks disconnected elements of drivers of guarded signals.
- **Segments**: at build time each signal is split into runs of elements with the same set of
  sources. Resolution runs once per dirty segment: `Resolution::StdLogic` resolves plane-wise
  with `Planes::resolve_block` (an empty source set gives `Z`); `Resolution::Func` gets one byte
  per element (logic as `std_ulogic` codes, a transpose only for user resolution functions).
  A composite resolved as a whole (`Builder::resolved_composite`, #83) keeps its usual leaves,
  logic ones on planes; parts of leaves (whole leaves, or slices for an element of an array of
  resolved records, #333) form a group whose sources are the same processes and ports, each
  covering its part, at one port level. A signal can hold several groups, which cover it. The
  first part popped in the update phase with any part dirty calls the resolver once per cycle
  with whole composite values (the parts concatenated in order) and writes every part's
  driving value; at initialisation the lowest leaf id does. A group without sources keeps
  its driving value without calling the resolver (LRM 14.7.3.2 a).
- **Quick transaction**: the zero-delay assignment of one `std_ulogic` element to a driver with
  nothing pending (`Ctx::assign_code`) is `Waveform::quick` (time, element, code), not a list
  entry with a value buffer. It matures in the next update (`Signal::apply_driver` writes the
  code into the driver); everything that schedules, preempts or inspects `Waveform::pending` goes
  through `Waveform::spill` or `idle` first. A zero-delay assignment of at most 64 logic elements
  to an idle driver (`Ctx::assign`, `Ctx::assign_blocks`) is `Waveform::vquick` (time, first
  element, length, the four plane words), likewise spilled on demand. On an element run whose
  driver is its only source, of a signal with no layer, group, guard or resolution function,
  `Signal::apply_driver` writes the plane words into the driver and into the signal's driving
  value at once and lists the run in `Signal::fast_dirty`; `finish_signal` then only updates the
  effective value of those runs. Any other vector assignment of a cycle appends its one
  transaction directly (`Ctx::assign`). A delayed assignment (`after T`, transport or inertial
  without a rejection limit) of either shape to an idle driver is the same `quick` or `vquick`
  with its time `now + T`: with nothing pending there is nothing to preempt. It is queued at that
  time and matures in the update of that time step. A direct driver takes it only when the run
  has no direct entry for the driver (those commit at the end of the run, before it).
- **Byte quick transaction**: one element of at most 8 bytes of a byte signal (`Ctx::assign_bytes`) assigned to
  an idle driver is `Waveform::bquick` (time, element, size, the bytes). When the driver is the element's only
  source and the signal has no layer, group, guard or resolution function, `Signal::apply_bquick` writes the
  driver, driving and effective values, the stamps and `'last_value` when it is applied (`finish_bquick`), as
  `finish_quick` does for a logic element; otherwise the element goes to the dirty ranges.
- **Direct transactions**: a driver that is the only source of its elements of a logic signal
  with no port, composite group, guard or resolution function (std_logic resolution of one
  source is the identity) is a direct driver (`DriverInfo::direct`, set by `SimControl::new`).
  A zero-delay assignment of one element, or of up to 64, that changes nothing (the signal allows skipping, the driver
  has no entry of this run, the signal and the driving value have the value) is not an entry: it only
  keeps the run's delta (`SKIPPED` in `ProcState::phantoms`, #720). A driver whose skip tests miss 8 times in a row makes
  its next 248 entries without the test (`ProcState::same`): an entry of an unchanged value commits and finishes with no event.
  A zero-delay assignment of at most 64 elements to it, with nothing pending on its waveform, is
  an entry in the process's `ProcState::out` (signal, source slot, element, code) or, for more
  elements, `outv` (first element, length, plane words; the entries of one driver are in one of
  the two, so their order is kept), not a waveform entry, a `scheduled` time or a queue entry; a later assignment to the same driver in
  the run is another entry (the last wins, as zero-delay preemption would give), and any
  assignment that needs the waveform first moves the driver's entries there (`Ctx::spill_out`).
  The commit (`commit_out`; for processes committed in the batch loop, after the batch, since
  the signals are shared until then) writes the driver value and the signal's driving value at
  once: the update phase is over and no process reads either before the next one. The elements
  wait in a per-signal bit mask (`Signal::pend`); the next update finishes them a plane word at a
  time (`finish_pend`: activity, then the XOR of `cur` and the driving value gives the events; the
  words in element order only for a signal whose changed runs are listed or that takes the dirty ranges),
  one element through its code byte. A signal with a force layer, or a force or deposit in that
  cycle, takes the elements to its dirty ranges and the general path instead. Activity stamps
  are not written for a signal whose activity nothing reads (`Signal::inactive`:
  `Config::skip_unchanged`, and no implicit signal has it as its prefix). A one-element entry
  whose code the driver and the signal already have writes nothing on such a signal, only the
  delta happens (`empty_delta`); the first one-element entry of a signal in a cycle waits in
  `Signal::last_commit` (`pend_one`), not in the bit mask. The commit writes only the driving
  value, which is the value of the driver as its only source (`Signal::lag`); the driver values
  are brought up to date (`sync_drivers`) before a force, release or deposit of the signal is
  applied or the general path computes the driving value from them, and a deposit makes the
  signal `exact` (its commits then write the driver value too). When one element changes and
  one resident process without an edge trigger waits for it, `finish_direct` wakes it at once.
- **Direct byte transactions**: the same for one element of at most 8 bytes of a byte signal (an enumeration,
  an integer, a boolean, a real, a time) with no port, group, guard or resolution function whose driver is the
  element's only source (`DriverInfo::bdirect`). A zero-delay assignment to an idle driver is an `OutB` entry
  (signal, slot, element, size, bytes) in `ProcState::outb`; the commit (`Signal::commit_bytes`) writes the
  driving value (the driver value too after a deposit; `lag` as above) and puts the element in `pend_one` or the
  bit mask; the update finishes it (`finish_pend_bytes`: the driving bytes against the current ones, an event if
  they differ, the activity stamps unless `inactive`), or takes it to the dirty ranges on a signal with a layer.
  Unchanged assignments, spilling (`Ctx::spill_out`) and `'driving_value` are as for logic.
- **Waiting again**: a process whose wait is exactly the one its resident waiter already covers
  (`Ctx::wait_on_again`) leaves its sensitivity list empty and sets `ProcState::again`; the commit keeps
  the registered waiter. If the process names more in the same wait, the commit rebuilds the list as the
  previous wait's ranges followed by the new ones and registers it as any wait.
- **Transactions** carry `(time, range, Option<value>)`, `None` being a null transaction. LRM
  10.5.2.2 preemption (transport, inertial with rejection limit, the "same value immediately
  before" chain) is decided per element using chunked diffs; a partly preempted old transaction
  is split into its kept runs.
- **Layers** (`Layer { mask, value }` over the whole signal): effective value per element =
  `force in`, else port input (`in`/`inout` formal), else `force out`, else `driving`.
- **Event queue**: one binary heap of `(time, Driver | Timeout(proc, generation) |
  Implicit(i))`. A zero-delay transaction is queued at `now` and matures in the next delta;
  entries at `now` go to a plain list instead of the heap, merged with the heap's due entries
  in the same `(time, entry)` order at the drain. Stale entries are dropped when they reach
  the top, so they never cause an empty cycle.
- **Transaction values** are recycled: a driver's waveform keeps a few buffers of applied
  transactions (a value covering the whole driver is swapped into the driver, not copied), and
  the next assignment refills one, so a steady stream of assignments allocates nothing. An
  assignment to elements with nothing pending skips the preemption scan.
- **Observer wake-ups** (`Hook::wake_at(t) -> WakeId`, `Hook::cancel`, #268) live in their own
  ordered map `WakeId (time, registration number) -> observer index`, not in the event queue,
  so a cancel removes the entry at once and an empty map costs one check per cycle. `Tn` is the
  earlier of the queue and the first wake-up, so the simulation is not `Idle` while one is
  pending. A wake-up before `now` is `ErrorKind::DeadlineInPast` (E3005); one at `now` gives the
  next delta (counted against the delta limit). With no observer there are no wake-ups and
  nothing changes.
- **Next activity time** (`Hook::next_time`, #828): in the end-of-time-step phase the kernel has
  just computed `Tn` (`next_time`, stale queue entries dropped) and hands it to the hook, so a
  VHPI plugin can ask for the next active time; wake-ups scheduled from the current call are
  added. Only when an observer says `Observer::wants_next_time` (VHPI does, waves do not). Other phases get `None`: the queue may still change in them.
- **Waiters**: per signal, `(proc, range, generation)`. A process's generation increases on every
  resumption, invalidating its other waiters and its timeout in O(1). Each signal buckets its
  waiters by chunk of 8 elements, 64 for a signal over 4096 elements (a waiter over at most 4 chunks
  sits in each of them, a wider or null one in a wide list; a signal of one chunk has the one list;
  the lists are made when the first process waits), so an event looks only at the waiters of the chunks it touches
  and the wide list: one process per element of a wide vector costs O(1) per event, not
  O(processes). Waiters carry their registration number and wake in that order, each once.
  Lists are compacted when they reach twice their live size. A process that waits for the
  same signals as in its previous wait, without a timeout, gets resident waiters
  (`ProcMeta::perm`, an epoch): they stay in their lists when they wake the process, so a
  clocked process is not removed and added again each cycle; a different wait, or one with a
  timeout, ends the epoch and the stale waiters go at the next scan of their list. Resident
  waiters keep their first registration number, so processes of one signal wake in the order
  they first waited, not in the order they last ran. A signal of 9 to 1024 elements has a list per
  element (one process per element of a vector scans one waiter per event). When every waiter of a
  one-list signal is resident and waits for the whole signal (a clock and its processes), the
  signal keeps the wake list itself (`Waiters::cache`, with the few distinct trigger ids in it):
  an event wakes the list in order without scanning it. A waiter added, or a process that leaves
  its resident wait (`run_batch` drops the lists of its old signals), drops the list, and the
  next scan builds it again. The list is split once into the processes that can run as one batch
  in its order (one suspension point, not postponed, ascending) and the others: when the signal is
  the only one that changed and nothing else woke a process, the first are the batch without a
  wake each, the others wake as usual.

## The cycle

`SimControl::cycle` follows 14.7.5.3:

1. Advance to `Tn` (new time step: delta 0, `NextTimeStep` callback) or count a delta. More than
   `max_deltas` deltas at one time is an `ErrorKind::DeltaLimit` naming the processes of the last
   delta. Then the `NextCycle` callback (every cycle) and the observer wake-ups due now fire (`Observer::wake`, only to the observer that
   scheduled each, in `WakeId` order); forces and deposits they queue apply in this cycle's
   update, and wake-ups they schedule at `now` fire in the next delta.
2. **Update phase** (`update`), all within this cycle:
   - drain due queue entries: matured transactions into driver values (null ones disconnect),
     marking elements active; timeouts resume processes;
   - **independent signals** (no port edge, no composite group, no force or deposit this
     cycle) touch nothing but themselves, so the drain only collects their matured drivers;
     they are then updated in parallel, one signal per task (with at least 256 of them and
     `threads > 1`): each applies its drivers' transactions in queue order, resolves and
     updates its effective value. A resolution error is taken from the lowest signal id, so
     the result is the same at any thread count. Everything else goes through the worklists
     below, serially;
   - **driving values bottom-up** over a worklist ordered by port level (formals before the
     actuals they feed): resolve dirty ranges, apply queued forces/releases/deposits (a release recomputes the
     released range's driving value from the sources, dropping an earlier deposit, #310), then pass
     the formal's driving value (after `force out`, through `to_actual` conversion) into the
     actual's port source and mark it active and dirty (K1);
   - **effective values top-down** (actuals before formals): update effective values and record
     events, then pass the actual's effective value (through `to_formal`) into the port layer of
     each `in`/`inout` formal whose actual elements were active;
   - **implicit signals** (K2) in creation order: `'transaction` toggles on activity,
     `'stable(T)`/`'quiet(T)` drop to FALSE on an event/activity and rise again at `now + T`
     (queue entry; `T = 0` means the next delta), `'delayed(T)` is a transport-delayed copy.
     `'transaction`, `'stable` and `'quiet` of a composite whose scalars are in several
     signals have one source per signal (`Builder::implicit_source`), any of them counting.
     They are kernel state, so none of them costs an extra delta or a process. `'transaction`
     (BIT) and `'stable`/`'quiet` (BOOLEAN) are one logic element with the codes of '0'/'1',
     like every BIT and BOOLEAN signal (#185). Their update is skipped when no source had
     activity in the cycle and none is due (`implicit_srcs`), which lets the fast update paths
     run in a design that has implicit signals (#720).
   So an uncollapsed port hierarchy of any depth settles in the same delta as its source.
3. Publish value changes on the observer bus (signal id order, then range order), then resume
   processes whose waited range overlaps a changed range. A process with an edge trigger
   (`Builder::edge_trigger`: its body is one `if rising_edge(s)` or `falling_edge(s)` on an
   element of a signal, or one `wait until` of that, and does nothing without the edge) stays
   suspended, its waiter in place, when the event does not make the edge true: running it
   would only evaluate the condition and suspend again
   (`Config::edge_triggers`, off when activations are observable). The trigger may also be
   `s'event and s = '1'` (`legacy`) and carry an asynchronous reset level
   (`if r = c then ... elsif <edge> then ...`): the process then also runs when `r` holds `c`.
   `Config::skip_unchanged` (R2b): an assignment that changes nothing on a signal whose
   activity nothing can observe (`Builder::mark_skippable`) is not queued; the cycle it would
   have caused still runs (`empty_delta`), and a later assignment of the run to the same driver
   queues the skipped ones first (`Ctx::flush_phantoms`) unless it is skipped too (the fast paths
   take a repeat that changes nothing, #720). See `docs/decisions.md` "Skipped
   unchanged assignments".
4. Run resumed non-postponed processes (parallel, below), between the `StartOfProcesses` and
   `EndOfProcesses` callbacks (every cycle, #317; at `EndOfProcesses` what the processes
   scheduled is not applied yet). `EndOfProcesses` also fires once after initialization's
   process run, before `EndOfInit` (majority of the reference simulators).
5. If no delta follows: `LastKnownDelta` callback (it may queue deposits, which create a delta);
   if still none, run resumed postponed processes (after the `StartOfPostponed` callback, only
   when there are any) and call `EndOfTimeStep`. A postponed process
   that schedules anything at `now` is an `ErrorKind::PostponedDelta`.

The run starts with the `StartOfSim` callback, runs every process once (initialization) and
calls `EndOfInit` before the first cycle (#272). When no delta follows at time 0, step 5 runs
right after `EndOfInit`, so `LastKnownDelta` and `EndOfTimeStep` happen at time 0 as in both
references with VHPI (a deposit queued there makes a delta at time 0, #347).

Initialization (`settle` at build) computes driving values bottom-up and effective values
top-down through ports without events; no signal is active then, so `'transaction` keeps its
initial '0' until the first transaction, `'stable`/`'quiet` start TRUE and `'delayed` at the
prefix's initial value (LRM 14.7.5.2, `docs/design/lrm-checks.md` L67, #241); then every process runs once at 0 fs delta 0, after
`StartOfSim`. The first cycle at 0 fs is delta 1.

**Ports** (`Builder::port(formal, actual, mode, PortConv)`) relate a formal range to an actual
range: `in` feeds the formal's port layer, `out` and `buffer` add the formal as a source of the
actual (the formal's effective value is its own driving value, LRM 14.7.3.3 b), `inout` does
both. Conversions run in the update phase and must return the full target range; a failure is
`ErrorKind::Conversion`. Port levels come from the association graph; a cycle is a build error.
`Builder::port_parts` relates a formal and an actual of several ranges each, possibly in
different signals (a record port with a conversion function, #74): the converter gets one value
per source part and returns one per target part. Every formal part is deeper than every actual
part; each edge runs its conversion when the worklist reaches the part it reaches last
(`PortEdge::up_at`, `down_at`), so it runs once per cycle with every part current, in the same
update phase (docs/design/ir.md 4.1). Without a conversion both sides must be one range.

**Guarded signals** (`Builder::guard(sig, Bus | Register, disconnect_time)`, resolved only):
`Ctx::disconnect` schedules a null transaction (after the given delay or the disconnection
time). With all sources of an element disconnected, a bus resolves the empty set and a register
keeps its value. `Ctx::driving(d)` is `'driving`; `Ctx::driving_value(d)` (K3) is the driver's
current value and an error while it is disconnected.

## Parallel evaluation and the commit protocol

Within a delta the kernel has two phases that never overlap:

- **Run phase.** The ready processes run on `Config::threads` threads: a rayon pool of one
  fewer and the kernel thread (1 means inline, no pool). A batch goes to the pool only when the
  estimated time of its parallel processes, less the largest one, passes a threshold that
  starts at `POOL_WORK_NS` (20 us) and rises when pooled batches turn out no faster than their
  estimate (`learn_pooled`); a smaller one runs inline, the hand-off costing more than it saves
  (#497). The estimate of a process is an average of timed inline activations (one in
  `COST_EVERY` of each process, at a position that moves from batch to batch; a process not yet
  timed counts 0), and one in `PROBE_EVERY` of the batches that would pool runs inline to
  refresh them: a pooled run would measure cold caches. Initialisation and postponed batches
  run inline; `Config::pool_all` pools every other batch of two or more (tests). A pooled batch
  is cut into pieces of consecutive process numbers, each with its own sub-slices of the slot,
  meta and idle arrays (`split_at_mut`, no lock); the workers and the kernel thread claim
  pieces in turn, run the parallel processes and commit the usual clocked ones into the
  piece's buffers, which the kernel thread applies after the join; the serial processes then
  run on the kernel thread in batch order (#481). Where a batch runs changes no output.
  Each gets a `Ctx` holding a shared read-only view (`Shared`: time, delta, cycle number, all
  signals, driver table) and `&mut` to its own `ProcState` (a per-process slot; its mutex is
  locked only by the parallel update of independent signals). A
  process can only read current-cycle values and write its own waveforms, its own wait state and
  its own effect buffer. No data races are possible without `unsafe`, and the crate forbids
  `unsafe`.
- **Commit phase** (one thread). Processes are visited in the batch order below: their new
  transactions and timeouts go into the queue, their sensitivity into waiter lists, and their
  buffered effects are committed: `report`, textio `OUTPUT` text, `stop`/`finish`,
  force/release. User files are not kernel state (K6 was dropped, #82): the interpreter reads
  and writes disk files directly, so a process that writes, closes and reopens a file in one
  delta reads what it wrote, and every caller of a file operation is serial.
- **Batch order** (`SimControl::order`, #252, user 2026-10-19). At initialisation:
  elaboration (`ProcId`) order. After that: the processes with one suspension point first, in
  elaboration order; then those with `ProcFlags::several_waits` (or `Process::several_waits`:
  more than one wait statement, or a wait in a called procedure; the interpreter takes it from
  `yavs_ir::Program::single_suspend`) in reverse elaboration order, those resumed by a signal
  before those resumed by a timeout (`ProcMeta::by_timeout`, the reason of the resumption that
  made the process ready). Postponed processes use the same order. It matches the majority of
  the reference simulators per case (`docs/decisions.md` "Process order within a cycle").
  Within the `several_waits` group the concurrent procedure calls (`ProcFlags::concurrent_call`
  or `Process::concurrent_call`; the interpreter takes it from the IR `IMPLICIT` flag of the
  entry function together with `!single_suspend`) come first, then the other processes; each
  sub-group in reverse elaboration order (#281, user 2026-10-19). A process that a signal
  resumes after it was resumed before (`ProcMeta::resumed`, not a concurrent call) runs after
  the concurrent calls, deeper instances first (`ProcMeta::depth`), then in elaboration order
  (#417, #462, #469, `docs/decisions.md` "Process order within a cycle").
- **Serial processes** (`ProcFlags::serial`, or `Process::serial` returning true) run one at a
  time, in batch order, after the parallel batch. Through M3 every process that touches a
  non-protected shared variable, calls a protected type or touches a disk file is serial (user
  decision; files #82; non-protected shared variables under `--relaxed`, #285); the kernel needs
  nothing else for that. `Process::serial` lets the
  process implementation (the interpreter, from the IR's `TOUCHES_SHARED`) say so itself, so
  no builder in between can drop it (#107).
- **Shuffle** (`Config::shuffle`) permutes the execution order with a seeded splitmix64. Commit
  order stays the batch order, so only designs whose processes share state change their output. Tests
  check that a design of 1000+ processes, with ports, implicit signals and a logic bus, gives
  identical traces for 1 and 8 threads and several seeds.

**Observers** run on the kernel thread, one at a time in subscription order, only between the
phases above (never during the run phase), so everything they do (reads, forces, deposits,
wake-ups and their numbering) is the same at any `--threads`; `tests/determinism.rs` checks
traces with wake-ups at 1, 4 and 8 threads.

**GC and heap safepoints.** The commit phase is the only point at which no process runs and no
`Ctx` exists. The shared heap collects between cycles, from an observer at `Phase::NextCycle`
(`yavs_interp::SharedGc`, #303); process heaps collect when their process suspends
(docs/design/ir.md 1.6). Deallocated slots are also recycled inside a long activation, by the
process itself between instructions (#355). Protected-type locks are never held across a suspension.

## How the IR plugs in

The interpreter implements it; compiled code (`yavs-llvm`, `yavs-clift`) runs inside the interpreter's process object, not as separate `Process` implementations:

```rust
pub trait Process: Send {
    fn run(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError>;
}
```

`run` resumes the saved frame and executes until the next `wait`. M2 provides:

- A process object owning its frame that is `Send`.
- Signal reads: `Ctx::value` (byte signals) or `Ctx::logic` (logic signals) with a `SigRef`
  (leaf signal plus element range); attributes `event/active/last_event/last_value/
  last_active`, `driving`/`driving_value(DriverId)`.
- Signal assignments: `Ctx::assign(driver, target range, waveform, delay mode)` with `ValueRef`
  values; `Ctx::disconnect` for `null` elements. The elaborator creates one `DriverId` per
  longest static prefix a process drives (`Builder::driver`) and passes them to
  `Builder::process`.
- `wait on S until C for T`: `wait_on` each signal, `wait_for(T)` the first time, then on
  resumption evaluate `C`; if false and not `timed_out()`, `wait_on` again and
  `wait_at(original deadline)`.
- `report`/`assert` to `Ctx::report`, `std.env.stop/finish` to `Ctx::stop/finish`, VHDL-2008
  `force`/`release` to `Ctx::force/release`, textio `OUTPUT` to `Ctx::output`; disk files are
  the interpreter's own (its callers are serial).
- Runtime errors as `Err(ctx.error(ErrorKind::Runtime(..)))` with the VHDL call stack in `trace`.
- A plain concurrent copy (`target <= source;` of signal names) needs no process code: the
  elaborator builds the kernel's own `CopyProcess` for it (#529).
- A clock generator `x <= not x after T;` on one scalar element needs none either: the elaborator
  evaluates `T` and builds the kernel's own `ToggleProcess` (#440).
- A one-operator logic gate (`y <= a and b;`, `y <= not a;` on logic signal parts of one length)
  needs none either: the kernel's own `GateProcess` reads the operands and applies the IEEE 1164
  tables (#779). So do literal operands, `=`/`/=` as a BOOLEAN gate, and a conditional assignment
  with one condition (`y <= a when s = '1' else b;`): `MuxProcess` (#786). Expression trees of those operators, compound conditions and chains of arms are a small program the kernel evaluates itself: `ProgProcess` (#788).
- Elaboration: leaves as `Builder::signal`/`resolved_signal` with `SignalType::logic` for
  `std_ulogic` arrays and `SignalType::bytes` otherwise; uncollapsed ports as `Builder::port`
  with `Converter` closures that call the IR conversion function; `'stable/'quiet/'delayed/
  'transaction` as `Builder::*_signal`; guarded signals with `Builder::guard`; flags `postponed`
  and `serial`.

## Error handling

Every failure is a `SimError` (a box around `ErrorInfo { kind, time, delta, process, trace }`,
so `Result` stays small on hot paths) and latches: later calls return the same error. Bad ids and
element ranges are `BadRef`, never an index panic. Time arithmetic is checked. A panic inside
process code, and a panic anywhere in the kernel (a broken invariant), is caught and becomes an
`Internal` error with exit status 101; other errors have exit status 1.

## Open questions

- The scalar byte path is 10 to 15% slower than the first byte-only kernel because of the
  storage enum and the port/implicit machinery (transactions are allocation-free since #385). Measured 2026-10-18 (#103, bench 10000 counters, 8 threads, work
  1000, before the parallel update): update 2.0 s, wake 0.3 s, run 1.4 s, commit 0.5 s. The
  update phase of independent signals is now parallel (see "The cycle"); still serial: the
  queue drain, port and composite-group signals, implicit signals, the wake and the commit
  phase (the commit is small next to the update, and its waiter lists and queue cross
  processes). Parallelising port signals would need level-ordered waves over the port graph.
- Preemption is O(elements times pending transactions) per assignment.
