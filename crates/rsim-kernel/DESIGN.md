# rsim-kernel design

The kernel runs the VHDL-2008 simulation cycle (LRM 14.7). It knows nothing about the IR:
processes implement the `Process` trait and values are opaque bytes. `SimControl` is the only
thing that drives simulation; the CLI, DAP and VHPI all call it.

## Data layout

- **Ids** are indices in elaboration order: `SignalId`, `ProcId`, `DriverId` (all `u32`). They
  never change during a run, so DAP, VHPI, waves and coverage can store them.
- **Signal** (`signal.rs`): one contiguous array of `len` elements of `elem_size` bytes
  (`SignalType`). Per signal: the effective value `cur`, the driving value `driving` before
  forces, `last_value`, and per-element arrays `event_cycle`, `active_cycle`, `last_event`,
  `last_active`. These are flat arrays over the whole signal, not objects per scalar (rule 2), and
  every operation works on a `Range` (offset, length) of elements.
- **Drivers**: a driver covers a fixed `Range` of one signal. Its current value lives in the signal
  (`Signal::drivers`), its projected output waveform lives in the owning process
  (`ProcState::waveforms`), so a running process mutates only its own state.
- **Segments**: at build time each signal is split into runs of elements with the same set of
  drivers. Resolution runs once per dirty segment, with one source slice per driver. A
  `std_logic_vector` resolver therefore sees 2 slices of 32 elements, not 32 calls.
- **Transactions** carry `(time, range, value)`. LRM 10.5.2.2 preemption (transport, inertial
  with rejection limit, the "same value immediately before" chain) is decided per element; an old
  transaction that is partly preempted is split into its kept runs. So `s(3) <= ...` only affects
  element 3, and partial inertial matches behave as the LRM says per scalar.
- **Forces** (`force in`/`force out`) are masks plus values per signal, allocated on first use.
  Effective value per element = effective force, else driving force, else `driving`.
- **Event queue**: one binary heap of `(time, Driver(id) | Timeout(proc, generation))`. A
  zero-delay transaction is queued at `now` and matures in the next delta. Stale entries
  (preempted transactions, timeouts of processes that resumed on an event) are dropped when they
  reach the top, so they never cause an empty cycle.
- **Waiters**: per signal, `(proc, range, generation)`. A process's generation increases on every
  resumption, which invalidates all its other waiters and its timeout in O(1). Lists are compacted
  when they reach twice their live size, so rarely changing signals do not leak entries.

## The cycle

`SimControl::cycle` follows 14.7.5.3:

1. Advance to `Tn` (new time step: delta 0, `NextTimeStep` callback) or count a delta. More than
   `max_deltas` deltas at one time is an `ErrorKind::DeltaLimit` naming the processes of the last
   delta.
2. Drain due queue entries: copy matured transactions into driver values, mark elements active,
   resume timed-out processes. Recompute driving values of dirty ranges (resolution), apply
   queued forces, releases and deposits, then update effective values and record events.
3. Toggle `'transaction` signals of active signals.
4. Publish value changes on the observer bus (signal id order, then range order), then resume
   processes whose waited range overlaps a changed range.
5. Run resumed non-postponed processes (parallel, below).
6. If no delta follows: `LastKnownDelta` callback (it may queue deposits, which create a delta);
   if still none, run resumed postponed processes and call `EndOfTimeStep`. A postponed process
   that schedules anything at `now` is an `ErrorKind::PostponedDelta`.

Initialization runs every process once at 0 fs delta 0, after `StartOfSim`. The first cycle at
0 fs is delta 1.

## Parallel evaluation and the commit protocol

Within a delta the kernel has two phases that never overlap:

- **Run phase.** The ready processes run on a rayon pool (`Config::threads`, 1 means inline).
  Each gets a `Ctx` holding a shared read-only view (`Shared`: time, delta, cycle number, all
  signals, driver table) and `&mut` to its own `ProcState` behind its own uncontended mutex. A
  process can only read current-cycle values (signals change only in the update phase) and write
  its own waveforms, its own wait state and its own effect buffer. No data races are possible
  without `unsafe`, and the crate forbids `unsafe`.
- **Commit phase** (one thread). Processes are visited in elaboration order (`ProcId` order):
  their new transactions and timeouts go into the queue, their sensitivity into waiter lists, and
  their buffered effects (`report`, output, `stop`/`finish`, force/release) are committed. So
  reports, queue order and every later decision are the same for any thread count or execution
  order. The first error in elaboration order stops the simulation.
- **Serial processes** (`ProcFlags::serial`, for non-protected shared variables) run one at a
  time, in order, after the parallel batch.
- **Shuffle** (`Config::shuffle`) permutes the execution order with a seeded splitmix64. Commit
  order stays fixed, so only designs whose processes share state change their output. Tests check
  that 1000 processes give identical traces for 1 and 8 threads and several seeds.

**GC and heap safepoints.** The commit phase is the only point at which no process runs and no
`Ctx` exists. That is where the future heap (rsim-rt) collects: the roots are each process's
saved frame (precise, rule 6), the queued effects and the signal values that hold access values
(none, as signals cannot have access types). Processes must not keep heap handles outside their
frame across a suspension. Protected-type locks are taken and released inside a run, so no lock
is held at a safepoint, which avoids the GC/lock deadlock NVC had (#1649).

## How the IR plugs in

The interpreter and the LLVM tier implement:

```rust
pub trait Process: Send {
    fn run(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError>;
}
```

`run` resumes the saved frame and executes until the next `wait`. M2 must provide:

- A process object owning its frame (registers, locals, PC) that is `Send`.
- Lowering of signal reads and attributes to `Ctx::value/event/active/last_event/last_value/
  last_active` with the `SigRef` (signal id plus element range of the longest static prefix).
- Lowering of signal assignments to `Ctx::assign(driver, target range, waveform, delay mode)`;
  the elaborator creates one `DriverId` per longest static prefix the process drives
  (`Builder::driver`) and passes them to `Builder::process`.
- Lowering of `wait on S until C for T` to `wait_on` for each signal, `wait_for(T)` the first
  time, then on resumption: evaluate `C`; if false and not `timed_out()`, `wait_on` again and
  `wait_at(original deadline)`. Static sensitivity lists are a `wait_on` before each return.
- `report`/`assert` to `Ctx::report`, `std.env.stop/finish` to `Ctx::stop/finish`, VHDL-2008
  `force`/`release` to `Ctx::force/release`, textio output to `Ctx::output`.
- Runtime errors (bound check, overflow, null access) as `Err(ctx.error(ErrorKind::Runtime(..)))`
  with the VHDL call stack in `SimError::trace` as `StmtId`s. The kernel adds nothing to such an
  error and stops at once.
- Values encoded as bytes in the layout rsim-logic and rsim-rt define; resolution functions as a
  `Resolver` closure that calls the interpreter on a segment.
- Elaboration flags: `postponed`, and `serial` for processes touching non-protected shared
  variables.

## Error handling

Every failure is a `SimError { kind, time, delta, process, trace }` and latches: later calls
return the same error. Bad ids and element ranges are `BadRef`, never an index panic. Time
arithmetic is checked (`TimeOverflow`); deadlines in the past are an error, not clamped. A panic
inside process code is caught and becomes an `Internal` error of that process (exit status 101);
other errors have exit status 1. `EndReason` covers normal ends (idle, `stop`, `finish`, severity
at or above `Config::stop_severity`, interrupt).

## Open questions

- Null transactions, guarded signals and disconnection (`after` on `null`) are not modelled.
- Ports: effective-value propagation through ports and conversion functions will come from
  elaboration as implicit processes (rule 3); until then effective = driving unless forced.
- `'stable`, `'quiet` and `'delayed` are left to elaboration as implicit processes; only
  `'transaction` is native.
- The commit and update phases are serial; with trivial processes they dominate (see the bench
  in STATUS.md). Parallel per-signal update and allocation-free transactions are the next steps
  if profiles show it.
- Preemption is O(elements times pending transactions) per assignment.
- Buffered file writes: a process that writes a file and reads it back in the same run needs
  file effects to apply immediately for that process only (M3).
- Kernel errors carry no diagnostic code yet; rsim-diag assigns codes per `ErrorKind`.
