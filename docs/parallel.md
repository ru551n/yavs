# Parallel simulation

A practical sheet on what `--threads` does, what it does not do, and what is planned. Short
version: leave `--threads` at its default of 1 unless you simulate a large, flat design, and
measure before you rely on it.

## How `--threads N` works today

`yavs run --threads N` (default 1) starts a pool of N worker threads. What runs on it:

* **Only process evaluation is parallel.** Parsing, analysis, elaboration and everything outside the
  run phase of a delta cycle are single-threaded.
* **Batches are pooled from a size cut-off.** The processes that wake in one delta cycle form a
  batch. A batch of 32 or more processes goes to the pool; a smaller batch runs on the kernel
  thread, exactly as with `--threads 1`. The cut-off counts processes, not the work they do
  (`PAR_BATCH` in `crates/yavs-kernel/src/kernel.rs`).
* **Serial processes stay on the kernel thread.** A process that touches files, or calls
  something that does, is classified serial. In a pooled batch the parallel processes run first
  on the pool and the serial ones then run one after the other on the kernel thread.
* **The update phase is parallel only above 256 independent signals.** Signals that are updated
  in one delta cycle are split over the pool only when there are at least 256 of them; fewer are
  updated one after the other.
* **The output does not depend on N.** Output, reports, waves, coverage and the exit status are
  byte-identical at any thread count, and with `--shuffle` the same for the same seed. The corpus
  is checked at 1 thread, 4 threads and 4 shuffled threads.
* **`--dap` runs on one thread.** `--threads` is ignored under `--dap` with warning `W0109`, so
  a debug session behaves the same every time.

With `--threads 1` there is no pool at all.

## Thread budget across parallel test runs

**Planned, not built.** Today every yavs process uses exactly the threads you give it. When a
test runner such as VUnit runs several simulations side by side (`-p N`), nothing shares the CPU
between them, so you cannot hand the cores that go idle at the end of a regression to the tests
that are still running. Tracked in
[#801](https://github.com/ru551n/yavs/issues/801).

The plan is a jobserver-style token pool, the protocol that `make -j` and cargo use:

* The test runner owns the pool, with one token per CPU slot, and passes it on in `MAKEFLAGS`.
* Each simulation owns one implicit token. It takes extra tokens only when a batch is worth
  parallelising, returns them when it is done with them (and on exit, error and interrupt), and
  never blocks waiting for one.
* `-p` then becomes a total CPU budget, and the long tail of a run uses the cores that are free.

This only pays off once extra threads speed up long simulations, so it follows the kernel work
below. Until then, do not combine a high `-p` with a high `--threads`: the threads of all runs
add up.

## Current performance

Measured on the interpreter and `--tier=auto`, `--threads 4` against 1, each run pinned to
performance cores, median of 3 (2026-10-21, details in `docs/STATUS.md` "Multi-threaded
scaling"). These figures are from before the single-thread kernel work of late October and are
being re-measured.

| Design kind | Speedup with threads |
|---|---|
| Soft-core processor testbench (corrected 2026-10-22) | 1.2x to 1.3x at 4 to 8 threads, for twice the CPU time |
| Flat RTL with large batches | gains a little: 1.33x (rtl_ff) and 1.20x (rtl_fan) at 4 threads |
| Mid-size batches (32 to 64 processes) | slower: 0.5x to 0.65x (rtl_bus, lfsr64, lfsr64_xor) for about 5x the CPU |
| Testbench-style designs (VUnit, OSVVM and UVVM tests) | about 1.0x |

Why:

* A pooled batch costs a fixed hand-over (66 to 108 us per batch) and loses the single-thread
  fast paths. With 32 to 64 cheap processes that is more than the batch itself, which is the
  regression above ([#497](https://github.com/ru551n/yavs/issues/497)).
* Testbench-style designs spend 63 to 100 percent of their process time in serial processes and
  rarely have a batch of 32, so they cannot gain from more threads inside one simulation.
  Their levers are single-thread speed and the thread budget above.
* The first soft-core figures (1.75x at 4 threads, 2.3x at 6) timed a core that was asleep after
  about 70 instructions (#513) and are withdrawn.

Work to remove the regressions (a work-based choice between inline and pooled, instead of the
count of 32) and to split the run phase into pieces handled in the workers is in progress
(#481 item 3, #497). The expected effect from a prototype is no regression on mid-size batches
and a larger gain on flat RTL; it is not on `main` yet.

This page is updated as the parallel-kernel work lands.
