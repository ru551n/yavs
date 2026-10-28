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
| When a group of processes goes to the threads | A "batch" is the set of processes that wake in one delta cycle. A batch of 32 or more processes goes to the threads; a smaller one runs on the main thread, as with `--threads 1`. The rule counts processes, not the work they do. |
| Processes that touch files | Run one after the other on the main thread, after the others in the batch. |
| Signal updates | Spread over the threads only when at least 256 independent signals change in one delta cycle. |
| Results | Output, reports, waves, coverage and exit status are byte-identical at any thread count, and with `--shuffle` for the same seed. The test corpus is checked at 1 thread, 4 threads and 4 shuffled threads. |
| Debugging | `--threads` is ignored under `--dap` (warning `W0109`), so a debug session behaves the same every time. |

## Measured speedup

`--threads 4` against 1, interpreter and `--tier=auto`, each run pinned to performance cores,
median of 3 (2026-10-21). These figures predate the single-thread speed work of late October and
are being re-measured.

| Design kind | Speedup |
|---|---|
| Soft-core processor testbench (corrected 2026-10-22) | 1.2x to 1.3x at 4 to 8 threads, for twice the CPU time |
| Flat RTL with large batches | 1.33x (rtl_ff) and 1.20x (rtl_fan) at 4 threads |
| Mid-size batches (32 to 64 processes) | slower: 0.5x to 0.65x (rtl_bus, lfsr64, lfsr64_xor), for about 5x the CPU time |
| Testbench-style designs (VUnit, OSVVM and UVVM tests) | about 1.0x |

## Planned

* **Better choice of parallel or not.** Decide by how much work a batch holds, not by how many
  processes it has, and split the work of a batch into pieces for the threads. In progress
  ([#497](https://github.com/ru551n/yavs/issues/497),
  [#481](https://github.com/ru551n/yavs/issues/481)). A prototype shows no slowdown on mid-size
  batches and a larger gain on flat RTL; it is not on `main` yet.
* **Shared CPU budget across parallel test runs**
  ([#801](https://github.com/ru551n/yavs/issues/801)). Today every yavs process uses exactly the
  threads you give it, and nothing shares the CPU between simulations that a test runner such as
  VUnit starts side by side (`-p N`). Until this lands, do not combine a high `-p` with a high
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
* Full measurements: "Multi-threaded scaling" in `docs/STATUS.md`. This page is updated as the
  parallel work lands.
