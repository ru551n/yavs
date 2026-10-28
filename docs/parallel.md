# Parallel simulation

`yavs run --threads N` lets one simulation use N CPU threads (default 1, no thread pool). Only
the evaluation of VHDL processes is spread over the threads, and only when many processes wake at
the same moment. It helps large, flat designs a little; it does not help testbench-style designs
and can slow down mid-size ones. Leave it at 1 unless you simulate a large, flat design, and
measure before you rely on it. Planned: a smarter choice of when to go parallel, and a shared CPU
budget across parallel test runs.

## What it does

| Topic | Behaviour |
|---|---|
| What runs in parallel | Only process evaluation. Parsing, analysis, elaboration and the rest of a simulation step are single-threaded. |
| When a group of processes goes to the threads | A "batch" is the set of processes that wake in one delta cycle. A batch goes to the threads only when its estimated work is worth the hand-over (about 20 microseconds of work that can be split); a smaller one runs on the main thread, as with `--threads 1`. The estimate comes from timing a sample of each process's runs on the main thread. |
| Processes that stay serial | A process that touches files, shared variables or protected objects, or calls something that does, runs on the main thread. In a batch that goes to the threads the others run first, then the serial ones one after the other. |
| Signal updates | Spread over the threads only when at least 256 independent signals change in one delta cycle. |
| Results | Output, reports, waves, coverage and exit status are byte-identical at any thread count, and with `--shuffle` for the same seed. The test corpus is checked at 1 thread, 4 threads and 4 shuffled threads. |
| Checks | Kernel tests compare 1 and 8 threads and several shuffle seeds, and the CI run of the test corpus at 4 threads compares 1 and 4 threads. |
| Compile thread | If a compiled tier is built in, `--tier=auto` compiles hot functions on one more background thread (see [install](install.rst)); it is not counted in `--threads`. |
| Debugging | `--threads` is ignored under `--dap` (warning `W0109`), so a debug session behaves the same every time. |

## Measured speedup

`--threads 4` against 1, interpreter and `--tier=auto`, each run pinned to cores, median of 3, on a
development host shared with other jobs, recorded on 2026-10-21 and 2026-10-22. The figures predate
the single-thread speed work of late October and were not repeated on the current revision: treat
them as the shape of the effect, not as a promise. The designs are small generated RTL designs
(`tools/bench`) and one testbench of a RISC-V soft-core processor.

| Design kind | Speedup |
|---|---|
| Soft-core processor testbench (100 us of simulated time, interpreter, corrected 2026-10-22) | 1.15x to 1.3x at 4 to 8 threads, for about twice the CPU time |
| Flat RTL with large batches | 1.33x (rtl_ff) and 1.20x (rtl_fan) at 4 threads |
| Mid-size batches (32 to 64 processes) | slower: 0.54x to 0.67x (rtl_bus, lfsr64, lfsr64_xor; design study tables), for about 5x the CPU time |
| Testbench-style designs (verification-library tests, scoreboards, bus models) | about 1.0x |

### By estimated work (2026-10-28)

Since 2026-10-28 a batch goes to the threads by its estimated work, not by its number of processes
([#497](https://github.com/ru551n/yavs/issues/497)). Speedup over one thread of the same build,
interpreter, simulation phase, each run pinned to as many cores as threads, median of 5, with the
previous rule in parentheses:

| Design | 2 threads | 4 threads | 8 threads |
|---|---|---|---|
| rtl_fan | 1.14x (1.01x) | 1.20x (1.02x) | 1.11x (0.92x) |
| rtl_ff | 0.92x (0.91x) | 0.96x (0.84x) | 0.92x (0.81x) |
| rtl_lfsr | 0.86x (0.89x) | 0.81x (0.80x) | 0.71x (0.72x) |
| lfsr64, lfsr64_xor, rtl_bus | 0.98x to 1.01x (0.52x to 0.62x) | 0.98x to 1.00x (0.51x to 0.60x) | 0.98x to 1.00x (0.35x to 0.48x) |
| rtl_chain | 0.97x (1.00x) | 1.02x (1.01x) | 0.99x (0.97x) |
| Soft-core processor, two cores | 0.83x (0.65x) | 0.75x (0.63x) | 0.57x (0.53x) |
| Soft-core processor, one minimal core | 1.00x (0.63x) | 0.94x (0.56x) | 0.66x (0.31x) |

The mid-size designs no longer slow down; the batches that still go to the threads lose on the
hand-over, which the next step addresses.

## Planned

* **Split a batch into pieces for the threads.** The batches that go to the threads are cut into
  pieces of the process table that the threads and the main thread take in turn, with the commit
  of the usual clocked process done in the thread
  ([#481](https://github.com/ru551n/yavs/issues/481)).
* **Shared CPU budget across parallel test runs**
  ([#801](https://github.com/ru551n/yavs/issues/801)). Today every yavs process uses exactly the
  threads you give it, and nothing shares the CPU between simulations that a test runner starts side by side (`-p N`), and each simulation with a compiled tier adds one compile thread. Until this lands, do not combine a high `-p` with a high
  `--threads`: the threads of all runs add up.

## Details

* **Why mid-size batches get slower.** Handing a batch to the threads has a fixed cost (66 to 108
  microseconds per batch) and gives up the single-thread fast paths. With 32 to 64 cheap
  processes that cost is larger than the batch itself.
* **Why testbench-style designs do not gain.** They spend 63 to 100 percent of their process time
  in processes that must stay serial and rarely have a batch of 32. Their levers are
  single-thread speed and the shared CPU budget above.
* **Withdrawn soft-core figures.** The first soft-core results (1.75x at 4 threads, 2.3x at 6)
  timed a core that went idle after about 70 instructions
  ([#513](https://github.com/ru551n/yavs/issues/513)) and are withdrawn.
* **How the shared budget would work.** It uses the token-pool protocol that `make -j` and cargo
  use. The test runner owns the pool, one token per CPU slot, and passes it on in the
  `MAKEFLAGS` environment variable. Each simulation owns one implicit token, takes extra tokens
  only when a batch is worth parallelising, returns them when done (also on exit, error and
  interrupt), and never blocks waiting for one. `-p` then becomes a total CPU budget and the long
  tail of a regression uses the idle cores. It only pays off once extra threads speed up long
  simulations, so it follows the work above.
* **Provenance of the figures.** The speedups are from the design study (`docs/design/parallel-kernel.md`
  in the source tree). The CPU-time costs (twice, about 5x), the 1.33x and 1.20x of the flat designs and
  the 66 to 108 microseconds per batch were measured during the audit work and cannot be reproduced from
  the repository: treat them as indicative.
* Full measurements: "Multi-threaded scaling" in `docs/STATUS.md`. This page is updated as the
  parallel work lands.
