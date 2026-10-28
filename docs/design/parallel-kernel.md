# Parallel kernel: where multi-threaded time goes, and a better design

Status: analysis and design (2026-10-21, issue #481 item 3). Nothing here is implemented on `main`. The
prototype measured in section 5 lives on the branch `spike/mt-chunks` (instrumentation plus the run-phase
changes, behind environment variables); it is evidence, not product code.

Status (2026-10-28, #497, #481): steps 1 and 2 (section 7) are on `main`: the inline or pooled choice by estimated
work (4.1), with a threshold that rises when pooled batches do not gain, and the run phase in pieces of the slot array
with the early commit in the worker (4.2; every pooled batch, serial processes after the join, `threads - 1` workers).
Measured in `docs/STATUS.md` "Parallel kernel, step 1" and "step 2". Step 3 (4.5): see `docs/decisions.md`.

Status check (2026-10-28, source at `ac2c94ff`, before step 1): still true. `PAR_BATCH` is a fixed 32, the parallel update cut-off a
fixed 256, the pooled path partitions serial and parallel processes and the kernel thread then locks and commits every
slot as section 2 describes; none of steps 1 to 6 of section 7 is on `main`, and no token pool exists (#801 is an
investigation). The single-thread kernel work since 2026-10-21 (prefetch of the next slot in a big inline batch, fast
update paths, quiet-activation skips) changed the inline path, so the "today" columns of the tables below describe
the pooled path of 2026-10-21 and have not been re-measured. The user-facing summary is `docs/parallel.md`.

Caveat (#513, #521): every neorv32 number in this document (and in `docs/STATUS.md` "Multi-threaded scaling") was taken on a
run in which the core slept in `wfi` after about 70 instructions. The corrected scaling on the real program is in
`docs/STATUS.md` "neorv32 processor testbench against sim1" (Threads): 1.2x to 1.3x at 4 to 8 threads, not 1.8x to 2.3x.

Sources: the kernel (`crates/yavs-kernel/src/kernel.rs`: `cycle`, `update`, `update_independent`, `wake`,
`order`, `run_batch`, `commit`), the process interface (`process.rs`: `Ctx`, `ProcState`), the interpreter's
process and shared state (`crates/yavs-interp/src/lib.rs`: `InterpProcess::run`, `exec`, `SharedState`,
`SharedGc`), the serial classification (`crates/yavs-ir/src/build.rs` `propagate`, `touches_files`;
`Program::is_serial`), the LLVM tier's compile thread (`crates/yavs-llvm/src/auto.rs`), the earlier
scaling measurement (`docs/STATUS.md` "Multi-threaded scaling, measurement", the #481 item 3 comment), the plan
(`docs/plan.md` "Parallel kernel design") and `docs/decisions.md` (kernel rounds 2 to 9, "Process order
within a cycle", "Resolution functions that touch shared state", "Shared heap collection").

## 1. Summary

- **The gap between today's speed-up and the ideal schedule is data movement and bookkeeping on the kernel
  thread, not the thread pool's synchronisation alone.** After a pooled batch the kernel thread walks every
  process slot twice (the live-output scan, which locks each slot, and the commit loop, which has lost the
  round 9 early commit). Each slot was just written by a worker core, so each visit is a cross-core cache
  miss: at 4 threads the live scan alone costs 38 us per batch of 4096 processes and the commit 37 us,
  against 91 us for the processes themselves on an ideal 4-way split. The per-process work does not grow
  at 4 threads (0.8x to 1.2x of the 1-thread time): the processes are not slowed by sharing, the
  hand-over is.
- **Two cheap changes recover most of it** (measured, section 5): skipping the live scan when the batch has
  no serial process (one condition), and a run phase that splits the batch into contiguous pieces of the
  slot array (no per-slot mutex, `split_at_mut`), lets the kernel thread claim pieces too, and does the
  round 9 early commit inside the worker into a per-piece queue buffer. Together with a choice of inline or
  pooled by estimated work instead of by count, every regression of today's pool disappears (lfsr64,
  lfsr64_xor, rtl_bus go from 0.54x to 0.67x to 0.97x to 0.99x) and the gaining designs gain more at 4
  threads: rtl_fan 1.26x to 2.47x, rtl_ff 1.37x to 1.67x, the soft core 1.82x to 1.87x (1.98x with a
  lower threshold), measured in section 5.
- **Testbench-style designs cannot be helped inside a delta.** Even if every process, serial or not, could
  run in parallel with a perfect schedule, the 4-thread ceiling is 1.13x (window_gen), 1.24x (pe_array),
  1.19x (the stream testbench) and 1.0x (the scoreboard testbench): their processes rarely run in the same
  delta, and when they do one of them dominates. Finer conflict classes, per-object locks or speculation
  would buy at most those numbers. Their lever is single-thread speed and running tests side by side
  (the token pool, #481 item 7).
- **Recommended order:** (1) work-based inline or pooled choice plus the live-scan condition (small, no
  risk, removes every regression); (2) the piece-split run phase with in-worker early commit and per-process
  cost estimates (medium, low risk, the main gain); (3) a parallel update for wide signals (element-range
  split) and for many independent signals below today's 256 cut-off (medium, medium risk, 10 to 20 percent
  on rtl_ff, rtl_lfsr); (4) the token pool for test runners. Not recommended: time-parallel or cross-delta
  execution, speculative execution of serial processes, a dedicated testbench/DUT split.

## 2. How a delta runs today

The cycle (`SimControl::cycle`) is strictly phased; every phase except the process run is on the kernel
thread:

1. `phase(NextCycle)` (observers, the shared-heap collector after a serial process ran), `fire_wakes`.
2. **Update** (`update`): matured queue entries are applied to drivers and signals. The common clocked case
   (every entry a driver of one independent signal, in driver order) is applied in place (round 9). Other
   independent signals go through `update_independent`, which uses the pool only from 256 signals on and then
   locks each driver's slot; ports, composite groups, forces and implicit signals are serial
   (`push_up`/`push_down` worklists by port level, `update_implicit`).
3. **Wake** (in `cycle`): per changed signal, the resident wake list (`Waiters::cache`) or `take_woken`;
   `wake` marks `ProcMeta::ready` and appends to `ready`/`ready_several`.
4. **Run** (`run_batch`): the batch is ordered (`order`) or shuffled. Below `PAR_BATCH` (32) processes, or
   at `--threads 1`, the batch runs inline on the kernel thread with `slot_mut` (no lock), and a process that
   waited again and only scheduled drivers is committed right after it ran (the round 9 early commit). From
   32 processes on, with a pool: the batch is partitioned into serial and parallel lists (two vectors),
   `pool.install` runs the parallel list with `par_iter` (each task locks its slot's mutex), then the
   kernel thread locks every parallel slot again to decide whether serial processes may write live output
   (#483), then runs the serial processes one by one.
5. **Commit** (the second half of `run_batch`): in batch order, every process not committed early pushes its
   transactions and timeout into the queue, registers its waiters, commits its effects (reports, `OUTPUT`,
   stop, forces) through the observers. The pooled path never commits early, so this loop visits every slot.
6. `last_delta`: postponed processes, `EndOfTimeStep`.

What makes the parallel run safe: a process reads only the current cycle's signal values (`Shared`, shared
by reference) and writes only its own `ProcState` (waveforms, effect buffer, wait state) and its private
heap. Processes that touch shared state are `serial`: the IR flag `TOUCHES_SHARED` set by a shared-variable
address (`AddrOf` of the shared space), a protected method call (`CallProt`), a shared-heap allocation or a
file native (`touches_files`, including `OUTPUT` writes), propagated to every caller (`propagate`). A serial
process runs with the whole `SharedState` taken out of one `Arc<Mutex<_>>` (`InterpProcess::exec`).

## 3. Where the time goes

### 3.1 Method

Release build with line tables, main at 0d80777f, interpreter tier unless stated. Timed runs pinned:
1 thread on core 4, 4 threads on cores 1,4,5,6, 5 threads on 1,4,5,6,7 (cores 2 and 3 belong to another
timing agent, core 0 is slow). Every timed run waits until its cores were below 15 percent busy for a
second (other agents' unpinned test runs land on these cores); medians of 3. The prototype branch adds
counters behind `YAVS_MT_STATS=<file>`: per batch its size, serial count, run and commit wall time, the
pooled path's sub-phases, and per cycle the update and wake times; with `YAVS_MT_PROC=1` also the time of
every process run (an `Instant` pair per run, about 25 ns, which inflates tiny processes: those numbers are
used only as weights, scaled per batch to the untimed batch time of a separate run). The batch sequence is
deterministic, so the timed and untimed runs line up batch by batch. `perf record --call-graph dwarf`
per thread for the attribution of kernel-thread time.

Designs: the RTL group of `tools/bench` (rtl_ff: 4096 one-bit registers on one clock, rtl_fan: 4000
counters, rtl_lfsr: 512 16-bit registers, lfsr64 and lfsr64_xor: 64 registers, rtl_bus: 32 drivers of
one resolved bus), the soft-core testbench (`neorv32_tb`, two CPU cores, 100 us), and the testbench rows
of the external group (vai_window_gen, vai_pe_array, osvvm_axistream, uvvm_sb_slv_tb).

### 3.2 The phases at one thread

| design | deltas | simulate ms | run % | update % | wake % | commit % | run time in batches of 32+ % | serial share of process time % |
|---|---|---|---|---|---|---|---|---|
| rtl_ff | 6000 | 897 | 81 | 15 | 3 | 1 | 100 | 0 |
| rtl_fan | 10001 | 859 | 92 | 0 | 6 | 2 | 100 | 0 |
| rtl_lfsr | 15000 | 374 | 79 | 19 | 2 | 1 | 100 | 0 |
| lfsr64 | 60000 | 191 | 74 | 22 | 3 | 1 | 97 | 0 |
| lfsr64_xor | 15000 | 73 | 83 | 15 | 2 | 1 | 98 | 0 |
| rtl_bus | 60000 | 187 | 47 | 50 | 2 | 1 | 96 | 0 |
| neorv32_tb | 70059 | 7782 | 94 | 2 | 1 | 2 | 94 | 3 |
| vai_window_gen | 7685 | 321 | 97 | 2 | 1 | 0 | 3 | 90 |
| vai_pe_array | 1494 | 38 | 95 | 4 | 0 | 0 | 0 | 73 |
| osvvm_axistream | 16836 | 307 | 94 | 5 | 1 | 0 | 0 | 98 |
| uvvm_sb_slv_tb | 4 | 956 | 100 | 0 | 0 | 0 | 0 | 100 |

(simulate ms is the sum of the instrumented phases, a few percent above `--time-report` because of the
counters.)

### 3.3 Large batches: rtl_ff, rtl_fan, rtl_lfsr, the soft core

A pooled batch at 4 threads, mean per batch (`YAVS_MT_STATS`, untimed processes):

| design | processes | partition us | `pool.install` us | live scan us | serial rest us | commit us | 1-thread run of the same batch us | ideal 4-way us |
|---|---|---|---|---|---|---|---|---|
| rtl_ff | 4098 | 4.8 | 164.7 | 38.0 | 0.2 | 36.5 | 364 | 91 |
| rtl_fan | 4002 | 4.0 | 63.0 | 34.5 | 0.1 | 35.0 | 158 | 40 |
| rtl_lfsr | 514 | 0.9 | 29.5 | 5.6 | 0.1 | 6.0 | 59 | 15 |
| neorv32_tb | 463 | 1.3 | 220.6 | 1.3 | 14.1 | 25.1 | 508 | 141 |

At 1 thread the commit of the same batch costs 1 to 6 us: the early commit of round 9 commits a clocked
process while its slot is in the cache, and the commit loop then skips it.

Tied to code:

- **Live scan** (`run_batch`, `let live = live_ok && par.iter().all(..)`): after the pool returns, the
  kernel thread locks the mutex of every parallel slot to look at its effects, even when the batch has no
  serial process and the answer is never used. `perf` on rtl_ff at 4 threads puts 26 percent of the kernel
  thread's samples on two atomic instructions (the mutex compare-exchange and the unlock swap) inside
  `run_batch`: the slot's line was last written by a worker, so the locked instruction waits for the
  line to move. 38 us per batch of 4096: about 9 ns per slot over the miss rate of a mostly prefetched walk.
- **Commit loop** (`run_batch`, second half): the pooled path never takes the early commit, so the loop
  reads `st.again`, `sens`, `deadline`, `effects`, `phantoms`, `scheduled` of every slot, again cross-core.
  Commit time grows from 11.5 to 76 ms on rtl_ff and from 19 to 160 ms on rtl_fan.
- **`pool.install` above the ideal** (rtl_ff 165 us against 91, rtl_fan 63 against 40, neorv32 221 against
  141): the per-process work is not inflated (the timed process runs at 4 threads are 0.77x, 0.94x, 1.23x and
  0.92x of their 1-thread times for rtl_ff, rtl_fan, rtl_lfsr, neorv32); what is added is a slot mutex per
  process (again a line that the kernel thread touched last, in `update`'s `apply_driver` and in the commit),
  `par_iter`'s adaptive splitting of 4096 one-process tasks with steals between workers (a process lands on a
  different core from delta to delta, so its interpreter state and slot are cold there), and the kernel
  thread itself, which blocks in `install` instead of running a share (4 workers plus an idle kernel thread
  on 4 cores). For the soft core the imbalance of heterogeneous processes adds to this: a static split by
  count would be worse (ideal model: 2.46x static against 3.03x dynamic at 4 threads).
- **Partition** (`partition` into two vectors per batch): 1 to 5 us, allocation included.
- **Update** stays serial: 15 percent of rtl_ff and 19 percent of rtl_lfsr at one thread, an Amdahl term of
  its own once the run phase scales (section 4.5).

### 3.4 Mid-sized batches: lfsr64, lfsr64_xor, rtl_bus

Batches of 34 to 66 processes worth 4 to 12 us inline. `pool.install` costs 11 us at 4 threads whatever the
work (lfsr64 11.0 us for 1.8 us of ideal 4-way work, rtl_bus 10.8 us for 1.6 us): waking parked workers and
the join. The earlier measurement's `perf` found 34 percent of lfsr64's cycles in the pool's spin and steal
code and 38 percent in the kernel (futex, yield). The count cut-off of 32 sends these batches to the pool;
any cut-off by count does, because the work per process differs by 30x between designs (rtl_fan 40 ns,
lfsr64 100 ns, soft-core processes up to tens of us). rtl_bus also spends half of its time in the update of
one resolved signal with 32 sources: no partition of signals helps it.

### 3.5 Testbench designs: why the time is serial

Per-process times (one thread) and the first cause the IR gives for each serial process (the prototype's
`serial_cause`, `YAVS_MT_CAUSE=1`, path from the process to the instruction):

| design | serial share | heaviest processes (share of process time) | cause |
|---|---|---|---|
| vai_window_gen | 90% | main 41%, monitor 41%, window_hold_check 8% (serial); DUT processes 8% | a protected call in the process (a random generator object local to the process), and the test runner's check and logger procedures (shared protected objects) |
| vai_pe_array | 73% | main 49%, monitor_out 24% (serial); DUT main 23% | the same |
| osvvm_axistream | 98% | receiver, transmit and receive handlers 75% | protected calls into the library's ID, coverage and alert singletons |
| uvvm_sb_slv_tb | 100% | one process runs the whole test | shared variables of the library |
| neorv32_tb | 3% | trace, log and UART receiver processes | file natives (`file_open`, `writeline`) |

What would more parallel execution buy? With the per-process times of every batch, an ideal 4-way list
schedule gives:

| design | as today (serial run after the parallel ones) | serial processes overlapped with the parallel ones | every process parallel, no conflicts at all |
|---|---|---|---|
| vai_window_gen | 1.00x | 1.07x | 1.13x |
| vai_pe_array | 1.00x | 1.10x | 1.24x |
| osvvm_axistream | 1.00x | 1.01x | 1.19x |
| uvvm_sb_slv_tb | 1.00x | 1.00x | 1.00x |

The last column is the ceiling of every finer scheme (per-object locks, conflict classes, speculation with
replay): even a free and perfect one cannot exceed it, because the heavy testbench processes run in
different deltas or alone (a testbench process that computes for 1.2 ms per activation, a scoreboard test
that runs in one process). And the conflicts are real where they matter: the check procedures update shared
counters and the logger state on every call, so main and monitor of window_gen would conflict whenever they
do run together.

Two classification findings, small in these numbers but correct to fix:

- **A protected call on a process-local protected object makes the process serial** (`CallProt` sets
  `TOUCHES_SHARED` unconditionally in `propagate`). Such an object (a random generator variable declared in
  the process) is private: another process can reach a protected object only through a shared variable
  (whose address is `AddrOf` of the shared space, which flags its user) or the shared heap (allocated by a
  flagged function, its handle stored in a shared variable). A `CallProt` whose object is the address of a
  frame or instance slot of the process could therefore stay unflagged; the method body's own flags still
  propagate through the call edge. Here it changes nothing (main and monitor also call the shared check
  procedures), but a testbench whose only protected object is a local generator would become parallel.
- **Writing `OUTPUT` makes a process serial** (the `ponytail` in `touches_files`): `writeline(OUTPUT, ..)`
  goes through `Ctx::output` and is ordered by the commit like a report, so only real file objects need the
  flag. It needs the file argument to be told apart (`OUTPUT` versus a file object); the soft core's log
  process is one such case.

## 4. A better parallel kernel

Hard invariants, unchanged: byte-identical output at any `--threads` and under `--shuffle` (commit order
fixed, execution order free); observers only between phases on the kernel thread (VHPI, DAP, waves,
coverage reports see the same states and callbacks at any thread count); precise GC (process heaps collect
at their own suspension on whatever thread ran the process; the shared heap collects at `NextCycle`, a
serial safepoint); serial processes never run concurrently with each other; `#![forbid(unsafe_code)]` in the
kernel.

### 4.1 Inline or pooled by estimated work

- Keep a cost estimate per process, `cost[p]` in ns, an exponential average of sampled inline runs (see
  below how it is learnt).
- Pool a batch only when its splittable work exceeds a threshold, where the threshold is the measured round trip of the
  pool (calibrated once at start by a few empty pooled batches; about 10 us both for today's pool and for
  the piece-split run phase of the prototype, which uses the same pool) times a safety factor (2): the
  prototype's best fixed value was 20 us of splittable work at 4 threads, 10 us pooled lfsr64 at a loss.
- The splittable work is the sum of the parallel processes' costs minus the largest one (a batch whose
  time is one process cannot be split), and serial processes do not count.
- Costs come from inline runs only: one activation in 16 of each process is timed on its own (rotating
  over the batch), and one batch in 64 that would pool runs inline. Pooled runs measure cold caches and
  would confirm themselves; batch means spread one heavy process over its cheap neighbours (both seen in
  the prototype, section 5). New processes start at a neutral cost (200 ns).
- Determinism: the choice changes only where a process runs, never the commit order, and the inline and the
  pooled paths commit identically (checked by the prototype across every design here).
- Measured (prototype mode 3, section 5): lfsr64, lfsr64_xor and rtl_bus fall back to 1.0x of the one-thread
  time, from 0.55x to 0.67x, and the large-batch designs keep their gain.

### 4.2 The run phase: pieces of the slot array, the kernel thread working, commit in the worker

- **No per-slot lock.** Sort a copy of the batch by process number, cut it into `k * m` pieces (k threads,
  m about 4), and hand each piece the sub-slices of `slots` and `procs` that cover its process range
  (`split_at_mut`, safe Rust). A worker reaches its slots with `get_mut`; the `Mutex<Slot>` can go
  entirely once `update_independent` follows the same scheme (section 4.5) and `Hook` reads slots through
  `&mut` (observers run on the kernel thread between phases).
- **Dynamic pieces, kept local.** Workers claim pieces from an atomic counter (one `fetch_add` and one
  uncontended lock per piece, not per process). Pieces are claimed in order, so with a stable batch (a
  clocked design: the same processes every edge) a worker tends to get the same pieces; for strict locality
  give each worker its own range first and let it steal pieces from the others only when done (the
  heterogeneous soft core needs the steal: static ranges lose 20 percent of its ceiling).
- **The kernel thread runs pieces too** (`ThreadPool::in_place_scope` plus `k - 1` spawned claimers), so
  `--threads N` means N busy threads, not N workers and a blocked kernel thread.
- **Early commit in the worker.** The round 9 early commit (a process that waited again on its resident
  waiters and only scheduled drivers) is done by the worker right after the process ran: `ready` cleared,
  `resumed` set in its `ProcMeta` slice, the transactions appended to the piece's own buffer. The kernel
  thread appends the buffers to the queue after the join. Queue order across processes does not matter: the
  due entries are sorted (`update`), and the in-place path checks driver order and otherwise sorts. The
  commit loop then touches only the processes that need the full commit (it still scans `procs`, a dense
  array; a per-piece list of those processes, concatenated in piece order and sorted into batch order, would
  remove that too).
- **More of the commit in the worker.** Timeouts can go to the piece buffer as well. Waiter registration
  must keep its order (the order of a signal's waiters is the order of first registration, which decides
  `--shuffle` permutations): a worker records `(signal, range, reg)` in its buffer and the kernel thread
  registers them in batch order. Effects (reports, `OUTPUT`, stop, forces) stay with the kernel thread,
  which commits them in batch order through the observers.
- **Serial processes.** Today they run after the parallel ones. They can run on the kernel thread while the
  workers run the parallel pieces (they touch no state a parallel process touches). Live output (#483) is
  then only safe for a serial process that comes before every parallel process of the batch in commit order
  (otherwise an earlier parallel process may still produce a report); others commit at suspension as
  parallel processes do. Gain bounded by the "overlapped" column of section 3.5 and by the soft core's 3
  percent: low priority.
- **Waking workers.** The pool's own spin-then-sleep decides the wake latency; with the work-based choice the
  pool is only used for batches of 15 us or more, where a few microseconds of wake-up are a small share. A
  custom persistent pool with spin-then-park handing borrowed batches to threads needs `unsafe` lifetime
  erasure or `'static` copies of the slot arrays; not worth it until the measured round trip of
  `in_place_scope` is the remaining term.
- **Placement.** Pin nothing by default (the operating system and other programs share the cores); with
  `--threads N` on a hybrid CPU, prefer performance cores for the pool threads when the process may use
  them (an affinity-aware default for `--threads auto`, see 6.4).

### 4.3 What stays serial, and why that is acceptable

The wake scan (1 to 6 percent), observer phases, `commit` of effects, implicit signals, ports and composite
groups. Each is below 6 percent of the cycle in every design measured here. The wake scan could be split
by changed signal with per-signal woken lists concatenated in signal order (the current order), but the
batch is sorted afterwards anyway and the term is small.

### 4.4 Determinism checks to keep

`tests/determinism.rs` at 1, 4 and 8 threads and several shuffle seeds must stay green with the work-based
choice forced both ways (an environment or `Config` knob that forces pooling of every batch of 2 or more
and one that forbids it), so the pooled path is exercised on small test designs. The difftest at
`--threads 4` and the external suites at `--threads 4` cover the rest.

### 4.5 A parallel update

Two cases matter, measured: one wide signal written by thousands of one-element drivers (rtl_ff: 65 us per
rising edge, 15 percent at one thread; rtl_lfsr 14 us per edge, 19 percent) and many independent signals
(the soft core: about 2 percent, not worth it there).

- **Element-range split of one signal.** For an independent signal (no port, group, force, guard or
  resolution function) whose due entries are sole-source quick transactions (the round 9 in-place path),
  the work per element touches only that element's driver slot, planes, code byte and stamps. Split the
  element range into pieces aligned to 64 elements (one plane word), give each piece the sub-slices of the
  signal's per-element arrays, apply the piece's drivers (driver order within a piece is element order
  here), and join the changed runs of the pieces in element order (`canonical_changed` already makes the
  list independent of application order). Needs a split view of `Signal` (codes, stamps, planes,
  `drivers`) and per-piece dirty and changed lists; medium effort, medium risk (every invariant of
  `Signal::codes` and the stamps must hold per piece).
- **Many independent signals:** `update_independent` already runs one signal per task from 256 signals on,
  locking each driver's slot; with the slot mutex gone (4.2) the driver waveforms are reached through the
  slot pieces of the run phase or, simpler, the waveforms of a driver move out of `ProcState` into a
  per-signal driver array (the process then writes a driver it owns through an index, which the signal's
  update reads without touching the process slot). That also removes the remaining cross-core line per
  driver per delta (the worker writes the waveform, the kernel thread reads it).
- Ports and groups: level-synchronous (all signals of one port level in parallel, levels in order) is
  possible but no design measured here spends time there.

### 4.6 Coverage, waves, VHPI, DAP

- Coverage counters are one shared array of relaxed atomic adds (`Inst::Cover`); counters of neighbouring
  instances share cache lines, so `--cov` at N threads will false-share. Per-thread counter shards summed
  at the end keep the totals identical. Not measured.
- Waves and VHPI value-change callbacks run on the kernel thread from the per-signal changed runs (whose
  order is canonical); unchanged.
- The debugger requires `--threads 1` for stops inside processes; unchanged.

## 5. Prototype results

Branch `spike/mt-chunks`. `YAVS_MT_MODE` selects: 0 as `main`; 1 adds the live-scan condition
(`serial.is_empty()` skips it); 2 adds the piece-split run phase with in-worker early commit for batches of
32 or more without serial processes (`YAVS_MT_SPLIT` pieces per thread, claimed from a counter, the kernel
thread claiming too); 3 replaces the count cut-off by splittable work above `YAVS_MT_THR` ns, with
per-process costs learnt as in 4.1 (also for batches with serial processes, which keep the old pooled
path). Batches with serial processes,
initialisation, postponed and shuffled batches take the old paths. Output of every design here is
byte-identical at 1, 4 and 8 threads in every mode (stdout, stderr and exit status hashed), and the kernel
and interpreter tests pass with modes 2 and 3 forced to pool everything.

Simulate wall, seconds, median of 3; speed-up over one thread in parentheses:

| design | 1 thread | 4 threads, `main` | mode 1 (4 threads) | mode 3 (4 threads) | 2 threads, `main` | 2 threads, mode 3 | 5 threads, `main` | 5 threads, mode 3 |
|---|---|---|---|---|---|---|---|---|
| rtl_ff | 0.714 | 0.523 (1.37x) | 0.429 (1.62x) | 0.428 (1.67x) | 0.616 (1.16x) | 0.599 (1.19x) | 0.529 (1.35x) | 0.443 (1.61x) |
| rtl_fan | 0.868 | 0.691 (1.26x) | 0.561 (1.55x) | 0.351 (2.47x) | 0.862 (1.01x) | 0.496 (1.75x) | 0.705 (1.23x) | 0.343 (2.53x) |
| rtl_lfsr | 0.379 | 0.299 (1.27x) | 0.283 (1.33x) | 0.269 (1.41x) | 0.331 (1.15x) | 0.351 (1.08x) | 0.312 (1.21x) | 0.266 (1.42x) |
| lfsr64 | 0.190 | 0.315 (0.60x) | 0.304 (0.63x) | 0.192 (0.99x) | 0.280 (0.68x) | 0.247 (0.77x) | 0.327 (0.58x) | 0.289 (0.66x) |
| lfsr64_xor | 0.072 | 0.108 (0.67x) | 0.104 (0.69x) | 0.074 (0.97x) | 0.102 (0.71x) | 0.086 (0.84x) | 0.116 (0.62x) | 0.095 (0.76x) |
| rtl_bus | 0.181 | 0.334 (0.54x) | 0.329 (0.56x) | 0.185 (0.98x) | 0.292 (0.62x) | 0.183 (0.99x) | 0.357 (0.51x) | 0.183 (0.99x) |
| neorv32_tb | 7.51 | 4.13 (1.82x) | not timed cleanly | 4.02 (1.87x); 3.79 (1.98x) at 10 us | 5.42 (1.39x) | 5.59 (1.34x) | 3.75 (2.00x) | 3.38 (2.22x) |
| vai_window_gen | 0.281 | 0.280 (1.00x) | 0.281 | 0.280 (1.00x) | 0.282 | 0.282 | 0.279 | 0.281 |
| vai_pe_array | 0.025 | 0.025 | | 0.025 | 0.025 | 0.025 | 0.025 | 0.025 |
| osvvm_axistream | 0.256 | 0.258 | 0.262 | 0.258 | 0.259 | 0.261 | 0.258 | 0.257 |
| uvvm_sb_slv_tb | 0.810 | 0.820 | | 0.817 | 0.809 | 0.816 | 0.819 | 0.816 |

Mode 3 at 4 threads is `YAVS_MT_THR=20000` (20 us of splittable work), 2 pieces per thread; the 2- and 5-thread
columns use 10 us, which is too low for the 64-process designs (lfsr64 and lfsr64_xor then pool and lose).
Mode 1 is from an earlier session of the same day with a one-thread time of 0.693 (rtl_ff) and 0.869
(rtl_fan); the neorv32 mode 1 samples were disturbed by another agent's runs.

Reading the table:

- Mode 1 alone (one condition) takes rtl_ff from 1.28x to 1.62x and rtl_fan from 1.12x to 1.55x.
- The piece split with the early commit in the worker (mode 2) is the large step for many small processes:
  rtl_fan 2.47x at 4 threads, 2.53x at 5.
- The work-based choice (mode 3) removes the regressions: lfsr64, lfsr64_xor and rtl_bus run at their
  one-thread speed (they never pool), and nothing that gained loses.

What the prototype went through, since each step is a finding:

- **Static chunks by count** (one piece per thread, assigned in batch order) made the soft core slower
  than `main` (4.56 s against 4.10 s): its processes differ by orders of magnitude in cost, and a static
  split leaves threads idle. Dynamic pieces fixed it (3.7 s).
- **A global average cost** (one number for all processes) sent the soft core's big batches inline
  (8.1 s): unsorted batches and small cheap batches pulled the average down. Per-process costs fixed it.
- **Costs learnt from pooled pieces** (piece time over piece size) are inflated by cold caches and
  self-confirm: lfsr64 kept pooling (0.32 s). Learning only from inline runs, with one batch in 64 that
  would pool run inline to refresh the costs, fixed it.
- **The mean of an inline batch** spread the time of a heavy serial process over the cheap parallel
  ones of the same batch, so testbench batches pooled and lost 12 to 14 percent (window_gen 0.31 s,
  the stream testbench 0.30 s). Timing one activation in 16 of each process individually (rotating, about
  2 ns per activation on average) and estimating the splittable work as the sum of the parallel processes'
  costs minus the largest one fixed it, and also keeps lfsr64_xor (one process is 41 percent of its
  batch) inline.

Where the 4-thread time goes after mode 3 (`YAVS_MT_STATS`): rtl_ff update 111 ms, wake 20, run 279,
commit 15 (pooled batch 135 us against 91 us ideal); rtl_fan update 1, wake 48, run 267, commit 33
(53 us against 40); the soft core update 201, wake 110, run 3333, commit 346 ms (pooled batch 186 us
against 141). The remaining terms, in order: the serial update of rtl_ff and rtl_lfsr (4.5), the soft
core's commit (most of its processes are not of the early-commit kind, so the commit loop still reads
their slots cross-core: move timeouts and waiter registration into the worker's buffer, 4.2), and the
pooled round trip of about 10 us (the reason the 64-process designs stay inline instead of gaining: the
ideal model gives lfsr64 1.47x at 4 threads with a 2 us round trip and a 5 us threshold).

## 6. Coarser parallelism

### 6.1 Across deltas or across time

The next delta's ready set is the result of this delta's update, which needs every transaction of this
delta, so deltas cannot overlap. Across time steps a conservative scheme needs lookahead: a partition of the
design whose outputs reach the others only after a minimum delay. In synchronous RTL every partition is
coupled through the clock and zero-delay signal assignments, so the lookahead is one delta: no gain. It
would help loosely coupled systems (several clock domains joined by delayed links, behavioural models with
explicit delays), which are not what users run here; the rollback machinery of optimistic schemes conflicts
with file output, foreign calls and VHPI. Not recommended.

### 6.2 Testbench and design apart

Running the serial testbench processes concurrently with the parallel design processes is the "overlapped"
column of section 3.5: 1.0x to 1.10x. The heavy share is the testbench itself. Not worth a separate
mechanism beyond 4.2's serial overlap.

### 6.3 Elaboration, analysis, compilation

Outside the current scope; noted for completeness. Analysis of independent design units and libraries can
run in parallel (the library lock already makes concurrent `analyze` processes safe); elaboration of
independent instance subtrees could, but elaboration is a small share of most runs. The LLVM tier compiles
on one background thread (`yavs-tier-up`); at `--threads 1` pinned to one core that thread competes with
the simulation, which is why `--tier=auto` at one pinned core is slower than the interpreter on short
runs.

Measured (one thread, `--tier=auto`, soft core 100 us): on one pinned core 9.77 s against 7.51 s
interpreted; on the set {core 4, efficiency core 8} 8.69 s (a mixed run: the operating system places both
threads, so the simulation may also run on the efficiency core part of the time). Moving the compile
thread to an efficiency core is worth about 11 percent of this run's wall time; with an affinity call
on the compile thread (a small dependency or one system call behind the existing `unsafe` boundary of the
LLVM crate) it would be deterministic. At 4 threads the tier changes little (soft core 4.44 s auto against
4.13 s interpreted with `main`'s pool; rtl_fan 0.36 s with mode 3): the compile thread then shares the
pool's cores.

### 6.4 The token pool (#481 item 7)

The design in the #481 comment stands. Two additions from this study: the kernel should ask for extra
tokens only for batches whose estimated work passes the pooling threshold (4.1), so a test whose batches
never pool never takes a token; and the pool must be resizable between deltas, which the piece split
(4.2) makes trivial (k is read per batch).

## 7. Estimates and plan

Expected speed-ups at 4 threads, from the measurements above (prototype where measured, the ideal-schedule
model with measured sync costs otherwise):

| design | today | after 1 | after 1+2 | after 1+2+3 (parallel update) | ceiling (ideal schedule, update serial) |
|---|---|---|---|---|---|
| rtl_ff | 1.37x | 1.62x (measured, mode 1) | 1.67x (measured) | about 2.0x (update 111 ms to about 35) | 2.56x |
| rtl_fan | 1.26x | 1.55x (measured, mode 1) | 2.47x (measured) | 2.5x (no update term) | 3.22x |
| rtl_lfsr | 1.27x | 1.33x (measured, mode 1) | 1.41x (measured) | about 1.7x | 2.45x |
| lfsr64 | 0.60x | 1.0x (inline) | 0.99x (measured) | 1.0x; 1.3x to 1.5x only with a 2 us round trip | 2.13x |
| lfsr64_xor | 0.67x | 1.0x | 0.97x (measured) | 1.0x | 1.57x |
| rtl_bus | 0.54x | 1.0x | 0.98x (measured) | 1.0x (one resolved signal) | 1.38x |
| neorv32_tb | 1.82x | about 1.85x | 1.87x to 1.98x (measured) | about 2.2x with timeouts and waiters committed in the worker | 3.03x |
| vai_window_gen | 1.00x | 1.00x | 1.00x (measured) | 1.00x | 1.02x (1.13x with no conflicts at all) |
| vai_pe_array | 1.00x | 1.00x | 1.00x (measured) | 1.00x | 1.03x (1.24x) |
| osvvm_axistream | 1.00x | 1.00x | 1.00x (measured) | 1.00x | 1.00x (1.19x) |
| uvvm_sb_slv_tb | 1.00x | 1.00x | 1.00x (measured) | 1.00x | 1.00x |

| step | what | effort | risk |
|---|---|---|---|
| 1 | work-based inline or pooled choice (per-process cost averages, calibrated threshold), live scan only with serial processes | small (one function, a cost array) | low: commit path unchanged |
| 2 | piece-split run phase without slot mutex, kernel thread works, early commit and timeouts in the worker, steal between ranges | medium (`run_batch` pooled path, `Slot` without `Mutex`) | low to medium: determinism tests with forced pooling |
| 3 | element-range parallel update of wide independent signals; driver waveforms out of `ProcState` | medium to large | medium: `Signal` invariants per piece |
| 4 | classification: frame-local protected objects and `OUTPUT`-only writers are not serial | small (IR flag propagation) | medium: soundness argument must hold for every way a protected object can be reached |
| 5 | token pool, `--threads auto` with core-type awareness | medium (runner and simulator) | low |
| 6 | serial processes overlapped with the parallel pieces | small | low, gain small |

Order: 1, 2, then 4 and 5 independently, 3 when a user design shows update-bound behaviour at N threads,
6 last.

## 8. Reproducing

```sh
git switch --detach spike/mt-chunks   # in a worktree
cargo build --release -p yavs --features llvm
YAVS_MT_STATS=/path/stats YAVS_MT_PROC=1 YAVS_MT_CAUSE=1 target/release/yavs run --threads 1 ...
YAVS_MT_MODE=3 YAVS_MT_SPLIT=4 YAVS_MT_THR=15000 taskset -c 1,4,5,6 target/release/yavs run --threads 4 ...
```

Statistics file lines: `C update_ns wake_ns rest_ns` per cycle; `B kind n n_serial serial_ns parallel_ns
max_ns greedy2 greedy4 greedy8 chunk2 chunk4 chunk8 overlap2 overlap4 overlap8 run_ns commit_ns pooled
partition_ns install_ns live_ns all4` per batch; `P pid serial runs ns name` per process.
