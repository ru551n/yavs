# yavs-kernel design

The kernel runs the VHDL-2008 simulation cycle (LRM 14.7). It knows nothing about the IR:
processes implement the `Process` trait and values are bytes or bit-plane logic. `SimControl` is
the only thing that drives simulation; the CLI, DAP and VHPI all call it.

## Data layout

- **Ids** are indices in elaboration order: `SignalId`, `ProcId`, `DriverId` (all `u32`). They
  never change during a run, so DAP, VHPI, waves and coverage can store them. Kernel `SignalId`
  order is the elaborator's `LeafId` order.
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
  element. Per-element metadata (`event_cycle`, `active_cycle`, `last_event`, `last_active`)
  are flat arrays over the whole signal (rule 2).
- **Sources**: a signal's source list holds process drivers (creation order) and then out-ports
  (`DriverSlot { range, value, off }`). A driver's current value lives in the signal, its
  projected output waveform in the owning process (`ProcState::waveforms`), so a running process
  mutates only its own state. `off` marks disconnected elements of drivers of guarded signals.
- **Segments**: at build time each signal is split into runs of elements with the same set of
  sources. Resolution runs once per dirty segment: `Resolution::StdLogic` resolves plane-wise
  with `Planes::resolve_block` (an empty source set gives `Z`); `Resolution::Func` gets one byte
  per element (logic as `std_ulogic` codes, a transpose only for user resolution functions).
  A composite resolved as a whole (`Builder::resolved_composite`, #83) keeps its usual leaves,
  logic ones on planes; the leaves form a group whose sources are the same processes and ports
  over each whole leaf, at one port level. The first group leaf popped in the update phase with
  any leaf dirty calls the resolver once with whole composite values (the leaves concatenated in
  leaf order) and writes every leaf's driving value; at initialisation the lowest leaf id does.
- **Transactions** carry `(time, range, Option<value>)`, `None` being a null transaction. LRM
  10.5.2.2 preemption (transport, inertial with rejection limit, the "same value immediately
  before" chain) is decided per element using chunked diffs; a partly preempted old transaction
  is split into its kept runs.
- **Layers** (`Layer { mask, value }` over the whole signal): effective value per element =
  `force in`, else port input (`in`/`inout` formal), else `force out`, else `driving`.
- **Event queue**: one binary heap of `(time, Driver | Timeout(proc, generation) |
  Implicit(i))`. A zero-delay transaction is queued at `now` and matures in the next delta.
  Stale entries are dropped when they reach the top, so they never cause an empty cycle.
- **Observer wake-ups** (`Hook::wake_at(t) -> WakeId`, `Hook::cancel`, #268) live in their own
  ordered map `WakeId (time, registration number) -> observer index`, not in the event queue,
  so a cancel removes the entry at once and an empty map costs one check per cycle. `Tn` is the
  earlier of the queue and the first wake-up, so the simulation is not `Idle` while one is
  pending. A wake-up before `now` is `ErrorKind::DeadlineInPast` (E3005); one at `now` gives the
  next delta (counted against the delta limit). With no observer there are no wake-ups and
  nothing changes.
- **Waiters**: per signal, `(proc, range, generation)`. A process's generation increases on every
  resumption, invalidating its other waiters and its timeout in O(1). Lists are compacted when
  they reach twice their live size.

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
     actuals they feed): resolve dirty ranges, apply queued forces/releases/deposits, then pass
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
     like every BIT and BOOLEAN signal (#185).
   So an uncollapsed port hierarchy of any depth settles in the same delta as its source.
3. Publish value changes on the observer bus (signal id order, then range order), then resume
   processes whose waited range overlaps a changed range.
4. Run resumed non-postponed processes (parallel, below).
5. If no delta follows: `LastKnownDelta` callback (it may queue deposits, which create a delta);
   if still none, run resumed postponed processes and call `EndOfTimeStep`. A postponed process
   that schedules anything at `now` is an `ErrorKind::PostponedDelta`.

The run starts with the `StartOfSim` callback, runs every process once (initialization) and
calls `EndOfInit` before the first cycle (#272).

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

- **Run phase.** The ready processes run on a rayon pool (`Config::threads`, 1 means inline).
  Each gets a `Ctx` holding a shared read-only view (`Shared`: time, delta, cycle number, all
  signals, driver table) and `&mut` to its own `ProcState` behind its own uncontended mutex. A
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
- **Serial processes** (`ProcFlags::serial`, or `Process::serial` returning true) run one at a
  time, in batch order, after the parallel batch. Through M3 every process that touches a
  non-protected shared variable, calls a protected type or touches a disk file is serial (user
  decision; files #82); the kernel needs nothing else for that. `Process::serial` lets the
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
`Ctx` exists. That is where the shared heap collects; process heaps collect when their process
suspends (docs/design/ir.md 1.6). Protected-type locks are never held across a suspension.

## How the IR plugs in

The interpreter and the JIT tier implement:

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
  storage enum and the port/implicit machinery; allocation-free transactions are the next step
  if real designs show it. Measured 2026-10-18 (#103, bench 10000 counters, 8 threads, work
  1000, before the parallel update): update 2.0 s, wake 0.3 s, run 1.4 s, commit 0.5 s. The
  update phase of independent signals is now parallel (see "The cycle"); still serial: the
  queue drain, port and composite-group signals, implicit signals, the wake and the commit
  phase (the commit is small next to the update, and its waiter lists and queue cross
  processes). Parallelising port signals would need level-ordered waves over the port graph.
- Preemption is O(elements times pending transactions) per assignment.
