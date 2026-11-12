# Performance

This page gives the measured speed of yavs against other simulators, and how it was measured.

**In short**, as measured on 2026-11-01 at commit `a95d9ad1` (and re-checked at `cb1a1295`): on 41 real third-party testbenches, yavs finishes a repeated (warm) run in about a fifth of the time of an established
open-source VHDL simulator, roughly 4.7 times faster, and a first (cold) run in about six tenths of it, roughly 1.7 times faster. yavs is slower on a few
testbenches, and clearly slower on long runs of whole processors (below). These figures are slightly worse than the previous measurement
(0.206 warm and 0.571 cold on 2026-10-28), and we have not found out why. The comparisons with the other two simulators are from that earlier run
and were not repeated. Every figure is a recorded run with its conditions in "How we measured" at the end, not a promise for your design.

**Since then the simulator changed a lot** (the performance work merged on 2026-11-04, released in 0.15.0), and it was measured on its own, against the previous release: see "The performance work of 0.15.0". The short version: long simulations of whole processors
take about half the processor cycles of the previous release, a run of everyday testbenches is about 9 percent cheaper warm, and short first (cold) runs got more expensive: 25 of 46 tests cost more cold, up to twice as much on one.

**The tables in "Result" and "Where yavs is slower", and the comparisons with other simulators in them, are older than the performance work merged on 2026-11-04.** They were not repeated on the merged code and say nothing about it. The only comparison with another simulator on the merged code is the short one in "The performance work of 0.15.0", in processor cycles and for one simulator.

```{contents} On this page
:local:
```

## What the numbers mean

- **Time** is the whole job a user waits for: analysing the libraries and the testbench's own files, elaborating, and running the simulation.
- **Warm** is the second and later run, with yavs's caches filled. **Cold** is the first run, from empty caches and fresh libraries.
- **Ratio** is yavs's time divided by the other simulator's time. Below 1 means yavs is faster: 0.20 is five times faster, 0.50 is twice as fast.
  Across testbenches the ratios are combined with the geometric mean, so a single long testbench does not drown the many short ones.

## Result

Row A is the run of 2026-11-01 on commit `a95d9ad1`, 41 testbenches (`grlib_leon3mp`, a LEON3 system, is listed but kept out of every mean). Rows B and C
are the run of 2026-10-28 on commit `1a8edb76`; they were not repeated, so they do not include the changes since.

| Compared with | Warm ratio | Cold ratio | Faster on (warm / cold) | Total time, warm: yavs / other |
| --- | ---: | ---: | ---: | ---: |
| **A: an established open-source simulator (compiled code)** | **0.213** | **0.589** | 37 / 35 of 41 | 37 s / 72 s |
| B: an open-source simulator, mcode code generator (2026-10-28) | 0.18 | 0.48 | 41 / 40 of 41 | 51 s / 237 s |
| C: a commercial simulator (2026-10-28; 29 of the 41 testbenches; it could not analyse or run the others) | 0.16 | 0.40 | 24 / 22 of 29 | 37 s / 94 s |

With `grlib_leon3mp` included, the ratio against A is 0.227 warm and 0.614 cold (yavs takes about 3.1 times as long on that one testbench warm, 3.3 times cold).
Without the libraries' analysis (own files, elaboration and run) the ratio against A is 0.209 warm and 0.491 cold. The executed instructions are 0.29 of A's.
Against C, only the run itself (elaboration and simulation, without analysing the libraries) can be compared on 40 testbenches: ratio 0.28 warm.
The "total time" column is the sum over the testbenches in the row; the pass against B ran while the host was busier than the pass against A,
so compare ratios, not those sums, between rows.

**These figures are slightly worse than the previous head-to-head against A** (0.206 warm and 0.571 cold on 2026-10-28, with 0.219 and 0.594 including `grlib_leon3mp`).
We do not know why. The hosts' load differed, about 60 commits lie between the two runs, and no quiet run of both builds side by side exists, so the difference is
neither shown to be a slowdown nor shown to be noise. There was no fully quiet sitting at all: other jobs ran on the same host throughout, most of the time on
the other half of the cores. Re-checking at `cb1a1295` (a later commit) changed instructions by at most 0.6 percent and cycles by at most 3 percent, with no direction.

## The performance work of 0.15.0

The performance work merged on 2026-11-04 (commit `b5663d516`, what 0.15.0 is built from) was measured against the previous release, 0.14.0, plus three commits that do not touch the simulator (`4ef7ec895`). It is a different measurement from the tables above: whole runs of 46 tests at `--tier=auto`, counted in processor cycles and instructions of the whole process, not in wall time, with the previous release as the baseline. The 46 are a LEON3 system (`grlib_leon3mp`), Microwatt, 39 of the testbenches of the real-world suite and five PoC testbenches (these five ran with the older PoC checkout, see "What these numbers do not cover").
The output and exit status were identical to the baseline in every run of every test.

Cycles of the new code divided by cycles of the previous release; below 1 is faster. Each measurement ran twice at once, on the two halves of the CPU (separate sets of cores); the figure in brackets is the other half. The control row compares a byte copy of the new binary with itself and shows how far two identical runs differ.

| | Warm | Cold | Instructions, warm / cold |
| --- | ---: | ---: | ---: |
| Microwatt, a long run | 0.51 (0.48) | 0.50 (0.49) | 0.44 / 0.45 |
| `grlib_leon3mp`, a long run | 0.50 (0.47) | 0.65 (0.60) | 0.51 / 0.61 |
| The 20 tests of the 46 that take over 1 G cycles | 0.79 (0.77) | not computed | 0.77 |
| All 46 tests, geometric mean | 0.91 (0.91) | **1.03** (1.02) | 0.90 / 1.00 |
| Control: the same binary against itself, 46 tests | 1.00 | 1.00 | 1.00 |

So long simulations run in about half the cycles, and so do the two big designs when the compiled code is already stored; the first run of `grlib_leon3mp` gets about two thirds of that gain, because it also has to compile. Across all 46 tests, which are mostly short, the gain warm is 9 percent, and cold the mean is **3 percent worse**.

### Where it is slower

Cold means a first run, with the compiled code still to be made. Of the 46 tests, 25 took clearly more cycles cold than before (against 10 that took fewer), where "clearly" means outside the control's range. The worst, new code divided by previous release in cycles (the other half of the CPU in brackets; peak memory is for the cold run):

| Testbench | Cold | Peak memory, cold |
| --- | ---: | ---: |
| `uvvm_axistream_vvc` | 1.96 (1.84) | 2.1 times |
| `neorv32_dual_full` | 1.43 (1.36) | 1.3 times |
| `poc_cache_cpu` | 1.35 (1.29) | 1.3 times |
| `osvvm_axi4_random` | 1.35 (1.31) | 1.7 times |
| `uvvm_func_cov` | 1.27 (1.29) | |
| `neorv32_single_cache` | 1.23 (1.17) | |

Most of the short tests (a first run of one to a few seconds) also cost 3 to 10 percent more cold. `--tier=auto` now also decides by how often code runs when it chooses what to compile, and that policy is what costs the first run: on `neorv32_dual_full` the same binary with `YAVS_AUTO_POLICY=sampled`, the earlier policy, used 0.67 of the cycles of the default in an earlier sitting. It pays on long runs and does not for a run of a few seconds. `YAVS_AUTO_POLICY=sampled` goes back to the earlier policy ([Environment](environment.rst)). We did not measure how much of the cold loss shows in wall time on a host with free cores: in these runs every process was pinned to one core, so the compile thread and the simulation shared it, which leans toward a larger cold loss than a free core would show.

Warm, 11 tests gained beyond the control (15 on the other half of the CPU) and one lost (two on the other half); the rest were not different from the control. `osvvm_axi4_random` read slowest warm (1.48 and 1.34, instructions 1.26 to 1.31, peak memory 2.1 times); its own warm runs fell into two groups of instructions, so the cycle figure has a wide band (19 and 12 percent) and is undecided, but the instructions point to a real loss. `hdlm_axis_bfm` (1.12 and 1.13) and `poc_fifo_ic_assembly` (1.06 and 1.08) were slightly slower warm on both halves.

### What the profile and the x86-64-v3 build add

The measurements above are of the plain Cranelift bench builds, not of the release archives, which are profile-guided. On top of the new code, a build with the profile and for x86-64-v3 (the plain x86_64 archives) took 0.90 of the new bench build's cycles warm across the 46 tests (0.89 on the other half), 0.88 on `grlib_leon3mp` and 0.87 on Microwatt (0.91 on the other half for both; the Microwatt band on the first half, 9.5 percent, reaches the control's). The x86-64-v3 build alone changed nothing beyond the control (0.99 across the 46). It was not measured cold. The profile-guided build in this comparison was built with a different `--target` setting than the two it was compared with, so some of the difference may be code layout.

### Against the established simulator A, on the merged code

A was timed apart from the squares above, on two cores of one half of the CPU, with one other job running; the new code ran in squares with three or four. Cycles of the whole process: for A, elaboration and run; for the new code, the whole run. One to a few runs each. Below 1 means yavs uses fewer cycles.

| | Warm | Cold | Wall time, warm: yavs / A |
| --- | ---: | ---: | ---: |
| Microwatt, a long run | 1.78 | 1.78 | 125 s / 71 s |
| `grlib_leon3mp`, a long run | 1.49 | 2.12 | 18 s / 13 s |
| 46 tests, geometric mean | 0.27 | not measured | |
| The 13 of the 46 where A takes at least a second, geometric mean | 0.92 | not measured | |

yavs is still slower than A on the two long processor runs: 1.5 to 1.8 times the cycles warm, 2.1 times on the first run of `grlib_leon3mp`. For `grlib_leon3mp` this agrees with the older measurement above (3.1 warm, 3.3 cold in wall time): the previous release would be at about 3.0 warm (1.49 divided by 0.50) and 3.3 cold (2.12 divided by 0.65) in these numbers. The Microwatt run is not the CoreMark run above, so the two are not comparable. On the tests where A takes at least a second yavs is about level; the 0.27 of the whole set comes from the many short tests, where yavs's start-up and cache are the advantage. It is a figure in cycles and is not comparable to the 0.213 of wall time in the table above.

## Long runs of whole processors

Two processors running a fixed CoreMark program for a long time, yavs warm (`--tier=auto`) divided by the same open-source simulator A, run including elaboration, as measured before the merge of 2026-11-04:

| Testbench | Ratio (warm) | Note |
| --- | ---: | --- |
| NEORV32 CoreMark | 1.45 | measured while other jobs ran on the neighbouring cores: it may read a few percent high |
| Microwatt CoreMark | 3.76 | measured on a quiet half of the CPU; on a busy one the same run gave up to 4.93, and its time varied between 271 s and 413 s |

Both were slower in yavs than in A, and they were the clearest place where yavs lost: Microwatt and `grlib_leon3mp` (3.4 times) ran about twice the instructions A did, at lower
instructions per cycle. On Microwatt and `grlib_leon3mp` the default tier was slower than yavs's own interpreter (by 14 and 26 percent in wall time); on NEORV32 it was level.
Elaboration of NEORV32 is faster than A's, that of Microwatt slower.

## Where yavs is slower

Against A (2026-11-01), yavs was slower warm on 4 of the 41 testbenches, on total time including analysis: `fletcher_bufferwriter_512` 1.81, `vai_window_gen_interlock` 1.48,
`sc5_fifo_repack` 1.26 and `fletcher_bufferreader` 1.12 (and `grlib_leon3mp`, outside the 41, 3.10). Cold it was also slower on `poc_stat_histogram` (1.29) and
`sc5_timestamp` (1.09), and on those four (1.97, 1.81, 1.76, 1.21; `grlib_leon3mp` 3.32). In all of them the simulation run is what costs, not analysis or elaboration, where yavs is ahead.
On `hdlm_lfsr` and `sc5_timestamp` the run itself is also slower than A's (1.35 s against 1.11 s, and 0.41 s against 0.30 s), though the total is not.
The short `neorv32_*` testbenches look fast (ratios 0.4 to 0.7) because of start-up, not simulation speed: see the CoreMark rows above for the rate.
In the 2026-10-28 run, against C the long CPU-bound testbenches (`neorv32_*`, `poc_arith_addw`, `fletcher_bufferwriter_512`) are
slower in yavs, by 1.3 to 1.8 times warm: C runs the simulation itself faster on these, while yavs is far ahead
on start-up and analysis, which is most of a short testbench. Against B (2026-10-28) yavs was faster on every testbench warm.
The cold ratio is higher than the warm one because analysing the libraries and compiling hot code happens in the first run only.
Among the hand-written RTL designs, clocked designs with very many cheap processes are where fully compiled simulators lead.

## What these numbers do not cover

- **Large suites with many testbenches in one library.** A change after these runs (`d789ead2`, the compile cache keeps up to 4 GiB per library directory) made warm runs of such suites
  faster, in particular a full Open Logic run. The figures above are one testbench per run and were taken before it, so they say nothing about it. Measurements of whole suites
  are in [issue 1029](https://github.com/ru551n/yavs/issues/1029); they were not repeated for this page. A suite more than about three times over the limit is slower warm than cold, by 7 to 8 percent in instructions at six times
  ([issue 1034](https://github.com/ru551n/yavs/issues/1034)).
- Waves and coverage on, other tiers than `--tier=auto`, and more than one thread were not measured here.
- **The 0.15.0 measurement counts cycles, not wall time on a free host.** Every process was pinned to one core, so the compile thread shared it with the simulation. Wall time on a host with several free cores, which is what most users have, was not measured, and neither were Microwatt cold with more than three pairs, a set of designs the work was not tuned on, or the tests under waves.
- Five of the 46 tests are PoC testbenches that ran with the older PoC checkout (see "PoC testbenches" below), because the list of the measured build no longer includes them. `uvvm_rand_perf` and the CoreMark runs of NEORV32 and Microwatt were not part of it.
- The comparisons with B and C are of 2026-10-28.
- **PoC testbenches.** The 41 testbenches of the 2026-10-28 and 2026-11-01 runs include six from the PoC Library v1.2 (`poc_arith_addw`, `poc_stat_histogram`, `poc_cache_cpu`, `poc_strings`,
  `poc_uart_rx`, `poc_fifo_ic_assembly`), and the figures above, including the `poc_*` ratios named in "Where yavs is slower", are from those. The suite now uses PoC v3.0.0
  ([issue 1060](https://github.com/ru551n/yavs/issues/1060)), whose old-style testbenches no longer exist, and has four OSVVM-based PoC testbenches instead (`poc_arith_addw`, now a different
  testbench, `poc_arith_scaler`, `poc_ooo_buffer`, `poc_arith_counter_bcd`). The means and ratios on this page were not recomputed for that suite.

## Which build these numbers are for

The figures in "The performance work of 0.15.0" are of plain bench builds (release, aligned functions, Cranelift for compiled code), built for the baseline x86-64 CPU, not of the release archives, which are profile-guided and, for the plain x86_64 ones, built for x86-64-v3. What the profile and x86-64-v3 add on top of those is given there (about 0.90 of the cycles warm across 46 tests). The tables of the profile gain below are wall-clock times on the GitHub runners.

Since 2026-11-12 every release build, bench builds included, is optimised as one program (link-time optimisation, one codegen unit; `docs/decisions.md` "Release profile"). The figures on this page from before that date are of the earlier release profile.

The release archives are profile-guided from 0.14.0 (see [Getting started](getting-started.rst)). Every comparison with other simulators on this page was made with a plain release build, so it says nothing about what the profile adds, and we have not repeated those comparisons with a profile-guided build.

What the profile adds was measured on 2026-11-04 on the GitHub runners that build the archives. Each archive was built with and without a profile from the same source and flags, and ten benchmark designs that the training never ran took turns on all arms, with a second plain binary as a control for the machine's noise. Three workflow runs of seven rounds each; each cell gives the lowest and highest geometric mean of the three runs, profile-guided divided by plain, so below 1 is faster. Times are wall-clock. Only the aarch64 runner has an instruction counter.

| Archive | Run, warm | Run, cold | Analysis | Control, warm | Instructions, warm |
| --- | ---: | ---: | ---: | ---: | ---: |
| Linux x86_64 | 0.924 to 0.938 | 0.922 to 0.937 | 1.001 to 1.003 | 0.998 to 1.007 | no counter |
| Linux x86_64 `-compat` | 0.931 to 0.940 | 0.929 to 0.939 | 0.993 to 1.003 | 1.001 to 1.004 | no counter |
| Linux aarch64 | 0.899 to 0.903 | 0.893 to 0.897 | 0.991 to 1.009 | 1.001 to 1.002 | 0.946 to 0.951 |
| Windows x86_64 | 0.903 to 0.910 | 0.904 to 0.916 | 0.978 to 0.983 | 1.005 to 1.008 | no counter |
| Windows x86_64 `-compat` | 0.904 to 0.933 | 0.914 to 0.930 | 0.972 to 0.975 | 0.993 to 1.005 | no counter |

What this does not say:

- **The gain differs a lot per design.** Text I/O gained 14 to 25 percent on every archive, a long chain of signals 2 to 25 percent, while a design that spends its run in compiled code (`compute_h2h`) gained nothing on any.
- **Two designs are slower on one archive.** On the Linux `-compat` archive `rtl_bus` took 4 to 6 percent longer warm and 1 to 2 percent longer cold, in all three runs, and `ieee_heavy` up to 2 percent longer. We do not know why: on our own host `rtl_bus` executes about the same number of instructions. The same design on Windows `-compat` read 0.98 to 1.05, and on the archives for x86-64-v3 and aarch64 it gained 7 to 11 percent. No other cell on any archive stayed above 1 in all three runs.
- **Analysis of a small file does not get faster on Linux:** it is mostly start-up. On aarch64 it executes 3 percent fewer instructions and takes the same time.
- **Elaboration of very wide port connections can lose.** Before the training had a case for it, a design with an out port on each of 20000 elements of one vector took 2.3 times the instructions of a plain build. A training case cured that, but we did not find out why it happened, so another path the training does not run could do the same. That design is left out of the table.
- **The runners differ.** The Linux x86_64 runners were AMD EPYC 7763, 9V45 and 9V74 in these runs, and different runs of one archive landed on different CPUs. These are not figures for your machine.

On our own host (AMD Ryzen 9 7950X3D, shared and loaded), a binary built the same way for x86-64-v3 executed 0.944 of the instructions of the plain build in the run of 34 real testbenches (highest 1.026), 0.967 in elaboration (highest 1.053) and 0.992 in analysis of their own files (highest 1.020). That is one pass per binary, and the machine was too loaded for its times to count.

## Measure your own design

`--time-report` and `--profile` show where a run spends its time, and the tiers and the cache
decide how much of it is compiling and analysing: see [Running faster](running-faster.rst). Time on
an otherwise idle machine, repeat, and compare medians; a short run varies by tens of percent.

## How we measured

The testbenches come from open-source VHDL projects (verification libraries, FPGA libraries, processor
designs), each run as its project's own scripts would: analysing the libraries and the testbench's files,
elaborating and running. yavs is a plain release build at its default `--tier=auto`, built for the baseline x86-64 CPU with aligned functions (for the 2026-11-04 measurement of 0.15.0, a Cranelift-only build of that kind): not the x86-64-v3 build of the x86_64 release archives from 2026-11-03 on and not a profile-guided one, which were not measured here. Every pair of timings ran on
the same CPU core, in turn, on a shared Linux host (AMD Ryzen 9 7950X3D), repeated two to five times; the 2026-11-01 run took the best of two passes for each measurement,
with a median host load of 10 to 14. The
other simulators are not named; all of them run compiled or generated code. The 2026-11-04 measurement of 0.15.0 was different: whole runs, warm and cold, in a Latin square with a barrier (every core runs a different binary at any moment), each process on one core, two squares at once on the two halves of the CPU, with a byte copy of the new binary as control; load median 6.8, all of it from the measurement itself. The per-testbench tables, the
method in full and earlier runs are in
[`docs/STATUS.md`](https://github.com/ru551n/yavs/blob/main/docs/STATUS.md) ("Real-world performance suite").

## See also

- [Running faster](running-faster.rst): what you can change for your own design.
- [Threads and the token server](threads.rst): when more threads help.
- [Conformance](conformance.rst): the conformance sheet.
