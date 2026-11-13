# Parallel simulation

The user guide has the page for users: [Threads and the token server](https://yavs.readthedocs.io/en/latest/threads.html).
This document holds the behaviour in detail and the measurements behind it, for developers.

`yavs run --threads N` lets one simulation use N CPU threads (default 1, no thread pool). Only
the evaluation of VHDL processes is spread over the threads, and only when many processes wake at
the same moment. Large, flat RTL designs, where thousands of processes wake on the same clock edge,
run up to about 2.4 times faster on 4 threads and 3.25 times on 8 (whole run, measured 2026-10-28). Testbench-style designs and
processor models run as fast as on one thread: yavs then keeps the work on one thread. Leave it at
1 for regressions that already run tests side by side, and measure before you rely on it, or use
`--threads=auto`, which uses more threads only while they make the run faster (below). Under a
token server named in `YAVS_TOKENS`, several simulations share one CPU budget (see "Thread budget
across parallel test runs" below).

## What it does

| Topic | Behaviour |
|---|---|
| What runs in parallel | Only process evaluation. Parsing, analysis, elaboration and the rest of a simulation step are single-threaded. |
| When a group of processes goes to the threads | A "batch" is the set of processes that wake in one delta cycle. A batch goes to the threads only when its estimated work is worth the hand-over (from about 20 microseconds of work that can be split); a smaller one runs on the main thread, as with `--threads 1`. The estimate comes from timing a sample of each process's runs on the main thread, and the threshold rises for a design whose batches turn out no faster on the threads (processes with much state, such as a processor's pipeline). |
| How a batch is shared | The batch is cut into pieces of neighbouring processes that the worker threads and the main thread take in turn; `--threads N` means N busy threads (N - 1 workers and the main thread). A clocked process that only drives signals is committed by the thread that ran it. |
| Processes that stay serial | A process that touches files, shared variables or protected objects, or calls something that does, runs on the main thread. In a batch that goes to the threads the others run first, then the serial ones one after the other, in the same order as on one thread. |
| Signal updates | Spread over the threads only when at least 256 independent signals change in one delta cycle. |
| Results | Output, reports, waves, coverage and exit status are byte-identical at any thread count, and with `--shuffle` for the same seed. The test corpus is checked at 1 thread, 4 threads and 4 shuffled threads. |
| Checks | Kernel tests compare 1 and 8 threads and several shuffle seeds, and the CI run of the test corpus at 4 threads compares 1 and 4 threads. |
| Compile thread | If a compiled tier is built in, `--tier=auto` compiles hot functions (see [installation](https://yavs.readthedocs.io/en/latest/getting-started.html#installation)). By default (`--tier-compile=wait`) the simulation waits while one compiles: it stops at the next delta cycle or at the interpreter's periodic look inside a long process run. A tier-up thread still prepares functions alongside the run, so a run can use somewhat more than its `--threads` cores for short periods; `--tier=interp` starts no such thread. `--tier-compile=background` compiles on one more thread alongside the run, not counted in `--threads`, at the lowest OS priority (nice 19 on Linux, the lowest thread priority on Windows), so the desktop and other simulations come first on a busy host. |
| Debugging | `--threads` is ignored under `--dap` (warning `W0109`), so a debug session behaves the same every time. |

## Measured speedup

Measured 2026-10-28 on `main`: speedup over one thread of the same build, whole run (simulation phase in
parentheses), interpreter, median of 5 rounds that interleave thread counts, each run pinned to as many cores as
threads. The designs are small RTL benchmarks (`tools/bench`) and the testbench of a RISC-V soft-core processor.

| Design | 2 threads | 4 threads | 8 threads |
|---|---|---|---|
| rtl_fan: 4000 small processes on one clock | 1.61x (1.70x) | 2.39x (2.63x) | 3.25x (3.89x) |
| rtl_ff: 4096 one-bit registers | 1.33x (1.37x) | 1.93x (2.10x) | 2.64x (3.04x) |
| rtl_lfsr: 512 16-bit registers | 1.33x (1.41x) | 1.69x (1.86x) | 2.07x (2.41x) |
| 64 registers, a resolved bus, a chain of one-process stages | 0.95x to 1.07x | 0.96x to 1.02x | 0.95x to 1.01x |
| Soft-core processor, one or two cores | 0.94x to 1.00x | 0.96x to 0.98x | 0.97x to 0.98x |

Before 2026-10-28 the mid-size designs ran at 0.35x to 0.6x on several threads and the soft core at 0.6x to 0.75x;
flat RTL gained at most 1.6x.

Further work on the signal kernel is tracked in [#884](https://github.com/ru551n/yavs/issues/884).

## Thread budget across parallel test runs

When yavs runs with `--threads N` above 1 and the environment variable `YAVS_TOKENS` names a
token server (`<port>:<secret>`: a server on 127.0.0.1 that the test runner starts, the same on
Linux and Windows), the threads come from that server
([#801](https://github.com/ru551n/yavs/issues/801)):

* `--threads N` is the cap. Each simulation owns one implicit token, its main thread, and can use
  at most N - 1 more lent by the server. The tokens decide how many of the N threads run at a time.
  The worker threads are started when the run is lent its first token (#879), so a simulation that
  never gets one has no more threads than a run with `--threads 1`.
* Tokens are asked for only when a group of processes is worth handing to the threads, never by
  waiting: while none is lent, the group runs on the main thread, as with `--threads 1`.
* With `--threads=auto` (what the test runner package passes under its server since 2026-11-07) the
  cap is 8, and tokens are asked for only while the threads are on: a design that does not gain gives
  them back after its trial (a fraction of a second) and asks again only at the next trial. Its
  cost under the server (the 2 percent of the processor model below, #882) was not measured again.
* Tokens are held while groups keep going to the threads and given back after 64 groups in a row
  that stayed on the main thread. The connection to the server closes when the simulation ends in
  any way (normally, with an error, on Ctrl-C, killed or crashed), and that gives every token back.
* Output, reports, waves, coverage and exit status do not depend on the tokens: they are the
  same at any token count, as at any thread count.
* Without `YAVS_TOKENS`, or with `--threads 1` (the default), it is not read and `--threads`
  behaves as before. `--dap` runs on one thread and ignores it.
* A `YAVS_TOKENS` yavs cannot use (not `<port>:<secret>`, or a secret the server does not accept,
  often a value left over from a test runner that ended) is error `E0128` before analysis (exit 2).
  Unset it to run without a token server. A server that cannot be reached, because nothing listens,
  it does not answer or the connection breaks during the run, is warning `W0118`: the run goes on
  with one thread and gives the same output.
* Without a token server, nothing shares the CPU between simulations run side by side: do not
  combine a high `-p` with a high `--threads`, the threads of all runs add up.

The protocol is in `docs/cli-contract.md` ("Token server") in the source tree.

A test runner that runs `-p N` simulations side by side can hand each of them `--threads N` and
lend the slots no test uses as tokens: one more each time one of its N slots runs out of tests. The long tests at the end of a regression then use the cores that are
free. This pays only where more threads speed a simulation up: flat RTL designs (see the measured
speedup above). Processors and testbench-style designs keep their batches on one thread and gain
nothing from extra tokens. A compiled tier's compile thread (`--tier=auto`) is not counted.

## Token server, measured

Measured 2026-10-29 on a regression of 24 testbenches from a downstream FPGA project (2 to 4 per
module, 0.1 s to 3.7 minutes each: AXI and Ethernet blocks, a soft-core processor, a CNN
accelerator, board-level system benches), `python run.py -p 8` with and without
`--yavs-jobserver`, yavs 0.11.0, the whole run pinned to 8 cores. Median of 3 runs that alternate
the two modes, after one untimed warm-up run of each. All 24 tests pass in both modes. The
"tail" is the time from the first moment, once the last test has started, that fewer than 8
simulations run, to the end of the last one.

| | without | with `--yavs-jobserver` |
|---|---|---|
| Total wall | 225.7 s | 234.4 s (+3.9 %) |
| Tail | 194.8 s | 201.4 s (+3.4 %) |

The token server gained nothing here, and cost about 4 percent. The run is decided by one test,
a soft-core processor benchmark that takes 222 s alone on one thread; it keeps all its work on
one thread (CPU time equals wall time), so the 7 other cores that sat idle during the tail were
never used. It ran 4 percent longer with the server (230 s against 222 s), probably because
other tests borrowed threads while all 8 slots were busy. Of the 24 tests only three used more
than one thread on average (a CNN-accelerator system bench 1.8, a board boot bench 1.8, a
firmware bench 1.3); they ended between 4.6 s earlier and 4.7 s later than without the server
and none of them is on the critical path. The rest ran at 0.9 to 1.1 threads. The server pays
only when the long tail is a flat RTL design (see "Measured speedup"); a regression whose long
tests are processors or testbenches does not gain, and with the server the processors cost a
little more than without.

A single large test run alone at `-p 8`, so that it can borrow up to 7 tokens (same host and method,
median of 3 interleaved runs after an untimed warm-up of each mode, simulation wall time and CPU time of the
`yavs run` process):

| Test | Mode | Wall | CPU | Average threads | Change in wall |
|---|---|---|---|---|---|
| CNN accelerator system bench | without | 47.7 s | 48.9 s | 1.02 | |
| | with `--yavs-jobserver` | 36.0 s | 77.4 s | 2.18 | 1.33x faster |
| Soft-core processor benchmark | without | 219.9 s | 221.2 s | 1.01 | |
| | with `--yavs-jobserver` | 224.7 s | 226.1 s | 1.01 | 2.2 % slower |

The accelerator bench pools: it gains a third with 2.2 threads on average, and the extra threads cost
58 percent more CPU time for that third. The processor model never uses a token, and the token mode alone
costs it 2 percent even with seven cores free ([#882](https://github.com/ru551n/yavs/issues/882)).

## `--threads=auto`

The kernel decides while the run goes (`crates/yavs-kernel/src/kernel/auto.rs`; the rules are in
`docs/cli-contract.md`, the reasons in `docs/decisions.md` "`--threads=auto`"). Three paths:

* **Static:** fewer than 8 processes that may run in parallel: one thread, no trial, no cache lookup.
* **Trial:** the threads start off, a trial starts 10 ms into the run, and windows of 25 ms (5 ms when
  no batch wants the threads) are judged by what the pooled batches saved against their estimate
  inline, calibrated by timed probe batches; losing windows turn the threads off (the code of
  `--threads=1`), a pause of 1 s doubling to 16 s (0.1 s once a window won) brings the next trial.
  Now and then a window runs on half the threads and the faster count is kept.
* **Memory:** what the last run of the same command line found (a `runner_cfg` generic counted without its
  seed and output path) is kept in the work library's cache:
  after a run that gained the threads start on, after one that did not they start off with the first
  trial after 4 s.

Measured 2026-11-07 on `main` plus this change, bench builds with Cranelift, 4 cores and their
hardware threads (CPUs 4-7 and 20-23, the 8 CPUs of the run; `auto` may use all of them), median of 5
rounds interleaving the cells, after two untimed runs of each. The host was shared and loaded (load
7 to 26; builds and tests ran on the other cores of the same CCD), so wall spreads are 2 to 36
percent; instructions and cycles (user, all threads) are the steadier measure for designs that do not
gain. Speedup over one thread of the baseline build, whole run; "memory" is a run after one of the
same command line, "trial" one with nothing remembered.

| Design | Tier | Best fixed | `auto`, memory | `auto`, trial |
|---|---|---|---|---|
| rtl_fan | default (`auto`) | 2.24x (8) | 2.03x, 91 % | 2.03x, 91 % |
| rtl_ff | default | 1.85x (8) | 1.72x, 93 % | 1.72x, 92 % |
| rtl_lfsr | default | 1.63x (4) | 1.54x, 95 % | 1.13x, 76 % |
| rtl_fan | interpreter | 2.70x (8) | 2.49x, 92 % | 2.50x, 92 % |
| rtl_ff | interpreter | 2.25x (8) | 2.08x, 92 % | 1.96x, 88 % |
| rtl_lfsr | interpreter | 1.81x (4) | 1.74x, 96 % | 1.66x, 97 % |
| CNN accelerator system bench (4 cores of 2 slots, 27 s) | default | 1.36x (4) | 1.30x, 96 % | 1.33x, 98 % |

The percentages are `auto`'s speedup over the best fixed one. Counted as a share of the gain
(speedup less 1) instead, the memory path reaches 83 to 92 percent and the trial 78 to 94 percent,
`rtl_lfsr` under the default tier 27 percent.

The CNN bench is one long test of the downstream project (the 4-core configuration of its top-level
bench, a network of a few layers through the weight read port; not the prefetch sweep the tracker
names, which runs 6 to 14 s), median of 3 rounds; its output was identical in every run.

The trial of `rtl_lfsr` under the default tier loses because once its processes are compiled the
estimated work of a batch (512 registers, about 15 us) is below the pool threshold (20 us), so no
batch wants the threads; `--threads=4` keeps pooling on costs it timed while the processes were
interpreted. A lower threshold under `auto` (5 us) only reached 1.19x and made the trial of
`lfsr64_xor` 9 percent slower in cycles; it was not kept.

The nine designs that do not gain (`rtl_chain`, `rtl_bus`, `rtl_ram`, `lfsr64`, `lfsr64_xor`,
`compute_h2h`, `xor_sig`, `xor_types`, `rec_pair`), `auto` against `--threads=1`: instructions 0.992
to 1.009, user cycles 0.954 to 1.009, on both tiers and both paths (the largest instruction
differences are under the default tier, whose compiles vary from run to run by that much at any
thread count). `--threads=4` and `--threads=8` cost the same designs up to 2 percent of their
instructions and 5 percent of their cycles.

The real-world performance suite (`tools/realbench`, 38 of its 40 tests; the two OSVVM driver tests did
not run on this host for either binary), `--threads=auto` against `--threads=1` of the baseline, both on
the same 8 CPUs, interpreter and default tier, median of 3 runs after the warm-up runs, so the memory
path. By path (a probe run of every cell): 9 tests static, 11 that end before their first trial or start
off from the memory, 18 judged and turned off, 2 that gain with the interpreter and are turned off
compiled:

| Test | Tier | `--threads=4` | `--threads=8` | `auto` |
|---|---|---|---|---|
| sc5_fifo_repack (FIFO repacker, wide vectors) | interpreter | 1.70x | 1.38x | 1.66x, 97 % |
| grlib_leon3mp (LEON3 system) | interpreter | 1.12x | 1.04x | 1.11x, 99 % |

Every other test: wall time 0.92 to 1.02 of `--threads=1` for the tests of 0.1 s or more (the shorter
ones are within the 10 ms steps of the harness's clock) and instructions 0.947 to 1.013 (both ends
under the default tier, whose compiles vary). User cycles were above 1.01 on 12 cells, 9 of them tests of
0.02 to 0.14 s whose cycles move by 2 to 6 percent from run to run with identical instructions; the
others are `vai_window_gen_interlock` interpreted (24 s, 1.017, instructions 1.0000), `uvvm_axistream_vvc`
compiled (1.021, wall spread 41 percent) and `sc5_fifo_repack` compiled (1.059 with wall 1.000: its
trials pool and gain nothing compiled, costing CPU time on the other threads, not wall time).

Tiny runs (`tests/corpus/scalar/array_attrs` and two designs of 2 and 66 processes, 1.5 to 4 ms,
`perf stat -r 30`): instructions within 0.05 percent of `--threads=1` on each path and cycles within
the spread (0.1 to 0.3 percent); no thread more than `--threads=1` is started. Before the trial was
delayed and the CPU count and the cache lookup were made lazy, the same runs paid 2 percent of their
instructions and 8 percent of their CPU time.

## Round 2 (2026-11-07)

Three changes (`docs/decisions.md` "Parallel kernel, step 3, and `--threads=auto` round 2"): the kernel thread waits
for the workers of a pooled batch spinning instead of sleeping in the scope (its wake-up was 3 to 5 microseconds of
every pooled batch); a batch on half the threads teaches the pool threshold; and a trial of `--threads=auto` starts on
costs timed for every process, compares all against half the threads by pooled time over the inline time of its probe
batches (half kept unless all are 5 percent faster), and keeps a design that gains from losing its threshold to a few
slow batches.

Measured on the same host and method as above (bench builds with Cranelift, CPUs 4-7 and 20-23, a shared host: other
jobs ran on these CPUs for parts of the sitting), 7 rounds, the ratio of each round's pair (cell and one thread of
`main`) taken per round, speedup over one thread of `main`, whole run. "First run": nothing remembered (a command line
of its own); "remembered": the run after one of the same command line.

| Design | Tier | Best fixed, `main` | Best fixed, now | `auto` first run, `main` | `auto` first run, now | `auto` remembered, now |
|---|---|---|---|---|---|---|
| rtl_fan | interpreter | 2.78x (8) | 2.89x (8) | 2.24x, 80 % | 2.69x, 93 % | 2.82x, 97 % |
| rtl_ff | interpreter | 2.26x (8) | 2.29x (8) | 1.87x, 83 % | 2.07x, 90 % | 2.18x, 95 % |
| rtl_lfsr | interpreter | 1.70x (8) | 1.88x (8) | 1.60x, 94 % | 1.77x, 94 % | 1.86x, 99 % |
| rtl_fan | default | 2.15x (8) | 2.23x (8) | 1.83x, 85 % | 2.11x, 95 % | 2.27x, 102 % |
| rtl_ff | default | 1.86x (8) | 1.90x (8) | 1.56x, 84 % | 1.69x, 89 % | 1.81x, 95 % |
| rtl_lfsr | default | 1.53x (4) | 1.81x (4) | 1.37x, 90 % | 1.45x, 80 % | 1.69x, 93 % |

The percentages are of the same build's best fixed speedup. A first run has a command line of its own, and the
compiled tier keeps a profile per command line in the cache too, so under the default tier a fresh command line is
slower at one thread already (8 to 21 percent more instructions on the realbench tests below): the default-tier first
runs above are against remembered fixed runs and read low. Against fixed runs on fresh command lines (7 rounds, ratio
to one thread of `main` on a fresh command line): rtl_fan 2.25x of 2.47x (91 percent; `main` 80), rtl_ff 1.78x of
1.90x (94; `main` 88), rtl_lfsr 1.38x of 1.62x (85; `main` 80). The interpreter keeps no such profile. At 2 threads the new build gains 1.30x to 1.64x (`main`
1.27x to 1.54x). Not met: the first run of `rtl_lfsr` under the default tier (85 percent against fresh fixed runs, 80 against
remembered ones); it gained in absolute terms while its best fixed speedup rose more. Its runs last
0.15 to 0.2 s, so the off start (10 ms), the calibration and one window on the slower thread count weigh; `--threads=8`
of compiled `rtl_lfsr` still loses its threads part of the run in some runs (3369 to 3493 of 4920 batches pooled), the
raised threshold that `auto` now undoes.

Two tests of the real-world suite that gain with the interpreter (the same CPUs, 5 rounds for `sc5_fifo_repack`, 3 for
`grlib_leon3mp`; other jobs ran on these CPUs through most of the `grlib_leon3mp` runs, 15 to 100 s of CPU time per run,
so its bands are wide):

| Test | Tier | `--threads=4`, `main` | `--threads=4`, now | `auto` first run, now | `auto` remembered, now |
|---|---|---|---|---|---|
| sc5_fifo_repack | interpreter | 1.42x | 1.95x | 1.79x, 92 % | 1.81x, 93 % |
| sc5_fifo_repack | default | 1.00x | 1.02x | 1.04x (fresh command lines) | 1.01x |
| grlib_leon3mp | interpreter | 1.09x | 1.17x | not resolved (band 0.49 to 2.2) | 1.13x, 97 % |
| grlib_leon3mp | default | 0.92x | 1.04x | 1.09x (band 0.58 to 1.46) | 1.10x (band 0.57 to 1.44) |

The CNN accelerator system bench of the earlier table (default tier, 5 rounds, a quieter hour): `--threads=4` 1.34x on
`main`, 1.41x now; `auto` 1.37x on the first run and 1.37x remembered, 97 percent; output identical in every run.

`sc5_fifo_repack` under the default tier: against `--threads=1` on a fresh command line the first run of `auto` is
0.96 in wall and 1.03 in instructions (3 rounds); against a remembered `--threads=1` it read 1.12 (`main` 1.17), which is
the compiled tier's profile, not the threads. The same holds for the default-tier first runs of `neorv32_single_cache`
and `poc_ooo_buffer` (equal walls to `--threads=1` on a fresh command line).

The designs that do not gain (`lfsr64`, `lfsr64_xor`, `rtl_chain`, `rtl_bus`, `rtl_ram`, `compute_h2h`, `xor_sig`,
`rec_pair`), 5 rounds, both tiers: `auto` instructions 0.997 to 1.002 of `--threads=1` on the first run and the
remembered one, except the first run of `rec_pair` under the default tier at 1.060 (as on `main`, 1.060: the fresh
command line of the compiled tier, see above); with a token
server of 8 slots (`YAVS_TOKENS`, one client) 1.000 to 1.008; one thread of the new build against `main` 1.000 to 1.002.
`--threads=4` still costs those designs 0 to 2.8 percent of their instructions (#858: the estimate of every batch
and the timed sample; not changed). Wall times of all of them are within the spread of the runs (bands of 5 to 30
percent on this host).

## Round 3 (2026-11-07)

Three changes (`docs/decisions.md` "`--tier=auto` under `--threads`" and "`--threads=auto` round 3"): the compiled
tier's simulation CPU is the kernel thread's (the process's had counted the pool workers spinning, so a threaded
`rtl_lfsr` read six times its wall time and its warm runs took the warm path that one thread never takes); under
`auto` the kernel looks at the clock about once a millisecond instead of every 256 delta cycles; a batch too small to
pool at the dearest cost known is not estimated.

Measured with bench builds (Cranelift) on CPUs 8-15 and 24-31 (8 cores and their second hardware threads, `auto`
at most 8 threads), first runs on fresh command lines, 7 to 10 rounds, paired ratios per round. The host was busy
throughout (other jobs ran on these CPUs: 1 to 7 s of their CPU time per run), so bands are
wide.

| Design | Tier | Best fixed (8) | `auto` first run before | `auto` first run now |
|---|---|---|---|---|
| rtl_fan | default | 2.61x | 2.26x, 87 % | 2.36x, 90 % |
| rtl_ff | default | 1.94x | 1.65x, 85 % | 1.72x, 89 % |
| rtl_lfsr | default | 1.58x | 1.36x, 86 % | 1.48x, 94 % |
| rtl_fan | interpreter | | 88 % | 92 % |
| rtl_ff | interpreter | | 90 % | 96 % |
| rtl_lfsr | interpreter | | 90 % | 94 % |

`sc5_fifo_repack` (default tier) gained 1.04x to 1.08x with `auto` on 8 and on 4 cores, so the round 2 reading of a
first run slower than one thread did not reproduce. The fixed `--threads=8` raise trap of compiled `rtl_lfsr` did not
reproduce in 37 runs on 8 or 4 cores (all 4920 batches pooled). The designs that never pool: instructions over one
thread unchanged within 0.001 except `rtl_ram` (1.012 to 1.004); `lfsr64` keeps 2 percent, its batches are estimated
just under the threshold.

## Round 4 (2026-11-08)

Two changes to the choice of where a batch runs (`docs/decisions.md` "`--threads=auto` round 4"): the batch last
estimated below the threshold is not estimated again for its next 15 batches, and the estimate of a batch that pools
stops once it passes the threshold, the threads adding up the rest while they run it.

Measured with one PGO bench build (Cranelift) whose switch turns each change off, every cell its own copy of the binary,
plus an A/A cell, on CPUs 12-15 and 28-31, 8 rounds, paired ratios per round, interpreter and default tier, 4 and 8
threads. Instructions of all threads against both changes off:

| Design | Pools | Instructions | Wall |
|---|---|---|---|
| rtl_fan | yes | 0.93 to 0.97 | 0.82 to 0.995 (both cells in all four sittings), 0.95 to 1.09 with the stop off |
| rtl_ff | yes | 0.96 to 0.97 | within the A/A |
| rtl_lfsr | yes | 0.97 to 0.99 | within the A/A |
| lfsr64 | no | 0.96 to 0.98 | within the A/A |
| rtl_bus | no | 0.99 | within the A/A |

The A/A cell's own walls moved 0.87 to 1.14 on this busy host, so only `rtl_fan` is resolved in wall time. Over one
thread (interpreter, nothing pooled): `lfsr64` at `--threads=4` 1.021 to 1.002 of the instructions, `rtl_bus` at
`--threads=8` 1.008 to 1.001.

An earlier first trial of `--threads=auto` (2 or 5 ms into the run instead of 10) was measured and not changed: on
first runs (fresh command lines, interpreter) `rtl_lfsr` cut to 16 us of simulated time read 0.90 of the wall time at
2 ms and the full designs 0.97 to 1.04 (A/A 0.98 to 1.04); `lfsr64`, which does not gain, read 1.06 (A/A 1.02) and its
short cuts were within their wide A/A. The default-tier sitting was too loaded to read.

## Details

* **Why mid-size batches no longer slow down.** Handing a batch to the threads has a fixed cost of
  a few microseconds and moves the processes' data between cores. A batch of a few dozen cheap
  processes costs less than that on one thread, so it stays there.
* **Why processors and testbench-style designs do not gain.** A processor's pipeline processes
  carry much state that another core's caches lack, so their batches run no faster on several
  threads, and yavs learns that and keeps them on one thread. Testbench-style designs spend most
  of their process time in processes that must stay serial (files, shared variables, protected
  objects). Their levers are single-thread speed, and a token server gives them no extra speed.
* **What it costs when nothing goes to the threads.** About 1 to 2 percent more instructions; the
  design runs at about 0.95x to 1.0x of one thread.
* Full measurements: "Parallel kernel, step 1" and "step 2" in `docs/STATUS.md`; the design is
  `docs/design/parallel-kernel.md` in the source tree.
