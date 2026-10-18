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
- **Transactions** carry `(time, range, Option<value>)`, `None` being a null transaction. LRM
  10.5.2.2 preemption (transport, inertial with rejection limit, the "same value immediately
  before" chain) is decided per element using chunked diffs; a partly preempted old transaction
  is split into its kept runs.
- **Layers** (`Layer { mask, value }` over the whole signal): effective value per element =
  `force in`, else port input (`in`/`inout` formal), else `force out`, else `driving`.
- **Event queue**: one binary heap of `(time, Driver | Timeout(proc, generation) |
  Implicit(i))`. A zero-delay transaction is queued at `now` and matures in the next delta.
  Stale entries are dropped when they reach the top, so they never cause an empty cycle.
- **Waiters**: per signal, `(proc, range, generation)`. A process's generation increases on every
  resumption, invalidating its other waiters and its timeout in O(1). Lists are compacted when
  they reach twice their live size.

## The cycle

`SimControl::cycle` follows 14.7.5.3:

1. Advance to `Tn` (new time step: delta 0, `NextTimeStep` callback) or count a delta. More than
   `max_deltas` deltas at one time is an `ErrorKind::DeltaLimit` naming the processes of the last
   delta.
2. **Update phase** (`update`), all within this cycle:
   - drain due queue entries: matured transactions into driver values (null ones disconnect),
     marking elements active; timeouts resume processes;
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
     They are kernel state, so none of them costs an extra delta or a process.
   So an uncollapsed port hierarchy of any depth settles in the same delta as its source.
3. Publish value changes on the observer bus (signal id order, then range order), then resume
   processes whose waited range overlaps a changed range.
4. Run resumed non-postponed processes (parallel, below).
5. If no delta follows: `LastKnownDelta` callback (it may queue deposits, which create a delta);
   if still none, run resumed postponed processes and call `EndOfTimeStep`. A postponed process
   that schedules anything at `now` is an `ErrorKind::PostponedDelta`.

Initialization (`settle` at build) computes driving values bottom-up and effective values
top-down through ports without events; then every process runs once at 0 fs delta 0, after
`StartOfSim`. The first cycle at 0 fs is delta 1.

**Ports** (`Builder::port(formal, actual, mode, PortConv)`) relate a formal range to an actual
range: `in` feeds the formal's port layer, `out` and `buffer` add the formal as a source of the
actual (the formal's effective value is its own driving value, LRM 14.7.3.3 b), `inout` does
both. Conversions run in the update phase and must return the full target range; a failure is
`ErrorKind::Conversion`. Port levels come from the association graph; a cycle is a build error.

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
- **Commit phase** (one thread). Processes are visited in elaboration order (`ProcId` order):
  their new transactions and timeouts go into the queue, their sensitivity into waiter lists, and
  their buffered effects are committed: `report`, `OUTPUT` text, `stop`/`finish`, force/release,
  and user-file streams (K6: `open_stream` hands the kernel a writer, `output` and
  `close_stream` are applied in order; ids are `(process + 1) << 32 | n`, independent of
  scheduling; streams are flushed at end of simulation; an I/O error stops the simulation with
  the process attached).
- **Serial processes** (`ProcFlags::serial`) run one at a time, in order, after the parallel
  batch. Through M3 every process that touches a non-protected shared variable or calls a
  protected type is serial (user decision); the kernel needs nothing else for that.
- **Shuffle** (`Config::shuffle`) permutes the execution order with a seeded splitmix64. Commit
  order stays fixed, so only designs whose processes share state change their output. Tests
  check that a design of 1000+ processes, with ports, implicit signals and a logic bus, gives
  identical traces for 1 and 8 threads and several seeds.

**GC and heap safepoints.** The commit phase is the only point at which no process runs and no
`Ctx` exists. That is where the shared heap collects; process heaps collect when their process
suspends (docs/design/ir.md 1.6). Protected-type locks are never held across a suspension.

## How the IR plugs in

The interpreter and the LLVM tier implement:

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
  `force`/`release` to `Ctx::force/release`, textio to `Ctx::output(StreamId::OUTPUT, ..)` and
  user files to `open_stream`/`output`/`close_stream`.
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

- With trivial processes the serial update and commit phases dominate (see STATUS.md); the
  scalar byte path is 10 to 15% slower than the first byte-only kernel because of the storage
  enum and the port/implicit machinery. Parallel per-signal update and allocation-free
  transactions are the next steps if real designs show it.
- Preemption is O(elements times pending transactions) per assignment.
- A process that writes a user file and reads it back before the end of the delta does not see
  its own buffered writes (M3: the interpreter can flush its stream through a serial path).
- Records resolved as a whole composite are one byte-kind leaf whose element is the whole record;
  a logic field inside such a leaf is bytes, not planes.
- Kernel errors carry no diagnostic code yet; yavs-diag assigns codes per `ErrorKind`.
