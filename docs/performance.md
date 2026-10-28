# Performance

**In short:** on 41 real third-party testbenches, yavs finishes a repeated (warm) run in about a fifth of the time of an established
open-source VHDL simulator, roughly 5 times faster, and a first (cold) run in a little over half of it, roughly 1.7 times faster. Against an
open-source simulator with its mcode code generator the figures are about 5.5 times (warm) and 2 times (cold), and against a commercial
simulator about 6 times and 2.5 times, on the testbenches it can run. yavs is slower on a few long, compute-heavy testbenches (below).
Every figure is a recorded run with its conditions in "How we measured" at the end, not a promise for your design.

## What the numbers mean

- **Time** is the whole job a user waits for: analysing the libraries and the testbench's own files, elaborating, and running the simulation.
- **Warm** is the second and later run, with yavs's caches filled. **Cold** is the first run, from empty caches and fresh libraries.
- **Ratio** is yavs's time divided by the other simulator's time. Below 1 means yavs is faster: 0.20 is five times faster, 0.50 is twice as fast.
  Across testbenches the ratios are combined with the geometric mean, so a single long testbench does not drown the many short ones.

## Result

Run of 2026-10-28 on commit `1a8edb76`, 41 testbenches (`grlib_leon3mp`, a LEON3 system, is listed but kept out of every mean).

| Compared with | Warm ratio | Cold ratio | Faster on (warm / cold) | Total time, warm: yavs / other |
| --- | ---: | ---: | ---: | ---: |
| **A: an established open-source simulator (compiled code)** | **0.21** | **0.57** | 36 / 34 of 41 | 38 s / 69 s |
| B: an open-source simulator, mcode code generator | 0.18 | 0.48 | 41 / 40 of 41 | 51 s / 237 s |
| C: a commercial simulator (29 of the 41 testbenches, see the notes) | 0.16 | 0.40 | 24 / 22 of 29 | 37 s / 94 s |

With `grlib_leon3mp` included, the ratio against A is 0.22 warm and 0.59 cold (yavs takes about 2.9 times as long on that one testbench).
Against C, only the run itself (elaboration and simulation, without analysing the libraries) can be compared on 40 testbenches: ratio 0.28 warm.
The "total time" column is the sum over the testbenches in the row; the pass against B ran while the host was busier than the pass against A,
so compare ratios, not those sums, between rows.

## Per-test results

Ratios as above, "warm / cold", sorted by the warm ratio against A, slowest first. Seconds are yavs's and A's total for a warm run.
"What it stresses" is the kind of workload the testbench puts on a simulator.

| Testbench | What it stresses | yavs warm s | A warm s | vs A | vs B | vs C |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| `grlib_leon3mp` (outside the mean) | cpu | 40.73 | 14.10 | 2.89 / 2.91 | n/a (1) | n/a (2) |
| `fletcher_bufferwriter_512` | wide-vectors | 4.03 | 2.09 | 1.93 / 2.09 | 0.09 / 0.10 | 1.77 / 2.05 |
| `vai_window_gen_interlock` | rtl-datapath | 7.91 | 5.30 | 1.49 / 1.77 | 0.60 / 0.77 | n/a (5) |
| `fletcher_bufferreader` | records | 1.53 | 1.21 | 1.26 / 1.35 | 0.28 / 0.31 | 1.00 / 1.11 |
| `sc5_fifo_repack` | wide-vectors | 1.48 | 1.18 | 1.25 / 1.77 | 0.79 / 1.08 | 0.81 / 1.09 |
| `poc_stat_histogram` | memory | 0.55 | 0.55 | 1.00 / 1.29 | 0.65 / 0.85 | 0.47 / 0.63 |
| `poc_arith_addw` | numeric | 5.60 | 5.72 | 0.98 / 1.01 | 0.12 / 0.13 | 1.34 / 1.44 |
| `hdlm_lfsr` | numeric | 1.46 | 1.86 | 0.78 / 0.97 | 0.20 / 0.25 | n/a (4) |
| `neorv32_single_min` | cpu | 2.51 | 3.55 | 0.71 / 0.79 | 0.42 / 0.44 | 1.29 / 1.44 |
| `sc5_timestamp` | fsm | 0.43 | 0.62 | 0.69 / 1.00 | 0.14 / 0.19 | 0.26 / 0.37 |
| `poc_cache_cpu` | fsm | 0.61 | 1.01 | 0.60 / 0.72 | 0.61 / 0.75 | 0.52 / 0.60 |
| `uvvm_axistream_vvc` | protected | 2.48 | 4.15 | 0.60 / 0.69 | 0.41 / 0.49 | 0.75 / 0.88 |
| `neorv32_single_cache` | cpu | 2.30 | 4.12 | 0.56 / 0.63 | 0.34 / 0.39 | 1.41 / 1.64 |
| `neorv32_dual_full` | cpu | 1.98 | 4.48 | 0.44 / 0.49 | 0.33 / 0.37 | 1.32 / 1.43 |
| `uvvm_util_rand` | protected | 0.74 | 1.75 | 0.42 / 0.69 | 0.58 / 0.87 | 0.28 / 0.49 |
| `uvvm_sb_slv` | protected | 0.60 | 1.49 | 0.40 / 0.63 | 0.54 / 0.87 | 0.26 / 0.40 |
| `osvvm_axi4_random` | protected | 0.78 | 3.07 | 0.25 / 0.45 | 0.25 / 0.45 | 0.11 / 0.22 |
| `poc_fifo_ic_assembly` | fsm | 0.11 | 0.45 | 0.24 / 0.51 | 0.25 / 0.52 | 0.10 / 0.21 |
| `olo_fix_cordic_rot` | numeric | 0.23 | 1.41 | 0.16 / 0.51 | 0.18 / 0.57 | 0.09 / 0.26 |
| `osvvm_axis_random` | protected | 0.38 | 2.47 | 0.15 / 0.38 | 0.14 / 0.34 | 0.07 / 0.17 |
| `hdlm_axis_bfm` | protected | 0.13 | 1.01 | 0.13 / 0.42 | 0.17 / 0.53 | n/a (4) |
| `hdlm_axi_crossbar` | records | 0.13 | 1.18 | 0.11 / 0.43 | 0.13 / 0.51 | n/a (4) |
| `uvvm_func_cov` | protected | 0.15 | 1.39 | 0.11 / 0.39 | 0.21 / 0.76 | 0.08 / 0.33 |
| `hdlm_axi_fifo` | records | 0.12 | 1.12 | 0.11 / 0.43 | 0.14 / 0.51 | n/a (4) |
| `fletcher_bufferwriter_prim32` | records | 0.08 | 0.76 | 0.11 / 0.24 | 0.27 / 0.62 | 0.07 / 0.17 |
| `vai_pe_array` | numeric | 0.20 | 1.96 | 0.10 / 0.35 | 0.13 / 0.53 | n/a (5) |
| `vai_weight_buffer` | memory | 0.14 | 1.40 | 0.10 / 0.42 | 0.12 / 0.55 | n/a (5) |
| `hdlm_register_file` | records | 0.11 | 1.11 | 0.10 / 0.41 | 0.13 / 0.49 | n/a (4) |
| `hdlm_ring_buffer` | memory | 0.10 | 1.01 | 0.10 / 0.41 | 0.14 / 0.52 | n/a (4) |
| `en_cl_fix_pkg` | numeric | 0.06 | 0.61 | 0.10 / 0.48 | 0.10 / 0.49 | 0.04 / 0.20 |
| `vunit_axi_dma` | records | 0.09 | 0.94 | 0.10 / 0.44 | 0.10 / 0.41 | 0.05 / 0.19 |
| `hdlm_sine_generator` | numeric | 0.10 | 1.07 | 0.09 / 0.38 | 0.15 / 0.53 | n/a (3) |
| `hdlm_fifo_sync` | rtl-datapath | 0.09 | 1.00 | 0.09 / 0.43 | 0.14 / 0.52 | n/a (4) |
| `hdlm_unsigned_divider` | numeric | 0.09 | 1.00 | 0.09 / 0.40 | 0.13 / 0.53 | n/a (4) |
| `poc_uart_rx` | fsm | 0.03 | 0.37 | 0.08 / 0.43 | 0.14 / 0.61 | 0.03 / 0.16 |
| `vunit_logging` | strings-textio | 0.07 | 0.89 | 0.08 / 0.39 | 0.10 / 0.45 | 0.04 / 0.16 |
| `olo_base_cam` | memory | 0.09 | 1.33 | 0.07 / 0.43 | 0.08 / 0.53 | 0.04 / 0.22 |
| `poc_strings` | strings-textio | 0.02 | 0.30 | 0.07 / 0.50 | 0.12 / 0.71 | 0.03 / 0.16 |
| `olo_intf_i2c_master` | bus-tristate | 0.07 | 1.06 | 0.07 / 0.51 | 0.08 / 0.54 | 0.04 / 0.21 |
| `olo_axi_master_full` | records | 0.09 | 1.37 | 0.07 / 0.44 | 0.10 / 0.56 | 0.04 / 0.22 |
| `olo_fix_madd` | numeric | 0.08 | 1.24 | 0.06 / 0.45 | 0.09 / 0.53 | 0.04 / 0.21 |
| `uvvm_methods_log` | protected | 0.01 | 0.60 | 0.02 / 0.46 | 0.02 / 0.41 | 0.01 / 0.19 |

Notes on "n/a" (the other simulator could not be timed on that testbench; yavs ran all of them):
(1) B does not complete this LEON3 design in this setup (exit status 1). (2) C reports 16 compile errors in the GRLIB libraries. (3) C rejects the
`sine_calculator` library file (a user-defined attribute on an entity), so the testbench does not run. (4) Part of the project's libraries
cannot be analysed by C (a vendor-primitive FIFO wrapper and the sine calculator), which makes the analysis step fail; only the
elaboration-plus-run time exists (about 0.3 to 0.9 s). (5) The analysis of the testbench's own files fails under C in this setup (not investigated);
its elaboration-plus-run time exists.

## Where yavs is slower

Against A, yavs was slower warm on 4 of the 41 testbenches (ratios 1.25 to 1.93): the two wide-vector designs `fletcher_bufferwriter_512`
and `sc5_fifo_repack`, one record-heavy testbench, and an RTL datapath with a windowing generator; cold also `poc_stat_histogram` and
`poc_arith_addw` (1.29 and 1.01). Against C the long CPU-bound testbenches (`neorv32_*`, `poc_arith_addw`, `fletcher_bufferwriter_512`) are
slower in yavs, by 1.3 to 1.8 times warm: C runs the simulation itself faster on these, while yavs is far ahead
on start-up and analysis, which is most of a short testbench. Against B yavs was faster on every testbench warm.
The cold ratio is higher than the warm one because analysing the libraries and compiling hot code happens in the first run only.
Among the hand-written RTL designs, clocked designs with very many cheap processes are where fully compiled simulators lead.

## What a run spends its time on

A run goes through analysis, elaboration, initialisation and simulation, and the first three are
cached between runs (see [usage](usage.rst)). Short testbenches are dominated by analysis and start-up,
which is why the cold ratio is higher than the warm one. In a long run, the work is in
processes, so the tier matters:

* `--tier=interp` runs the interpreter alone: no compile cost, the slowest on long compute-heavy
  loops.
* `--tier=auto` (the default in a build with a compiled tier) samples what the interpreter spends
  time in and compiles those functions on one background thread, starting 5 ms into the run; a run
  shorter than that creates no compile thread.
* `--tier=clift` compiles everything before time 0.

The output is the same with every tier. See [usage](usage.rst) for the options and
[install](install.rst) for which build has which tier.

## Measure your own design

* `yavs run --time-report` prints the wall and CPU time of analysis, elaboration, initialisation
  and simulation to standard error.
* `yavs run --profile[=FILE]` prints the busiest processes, instances and subprograms with
  file and line (JSON to FILE). Start here for "where does my long run spend its time".
* Compare `--tier=interp`, `--tier=auto` and, where built in, `--tier=clift`, with
  the cache cold (`--no-cache`, or a fresh work library) and warm.
* Time on an otherwise idle machine, repeat, and compare medians; a short run varies by tens of
  percent.
* `--threads` rarely helps: see [parallel](parallel.md).

## How we measured

- **Tool and tests:** `tools/realbench` runs real testbenches from open-source VHDL projects (hdl-modules, Open Logic, VUnit, OSVVM, UVVM, PoC, NEORV32, GRLIB,
  Fletcher, SatCat5 and others), each in a private directory per run; the 41 are a fixed collection. All simulators get the same analysis grouping
  (consecutive files of one library and standard in one call). The time is the wall time of the whole script: libraries, own files, elaboration, run.
- **yavs:** the build of the release archive, benchmark-built with `tools/build-bench.sh --features clift`, one binary for every cell, at its default `--tier=auto`: the
  interpreter starts the run, hot functions are compiled on a background thread, and the compiled code is cached on disk.
- **Other simulators:** A is compiled-code (native) and has its libraries analysed in the same way; B is run with one elaborate-and-run command and with relaxed
  rules for shared variables (several of the projects need that); C is run as one batch job per testbench. The libraries are analysed once outside the timing, then the
  timed "cold" and "warm" analysis cells repeat the project's analysis calls. None of A, B or C has a compile cache, so for them cold and warm differ only by
  the analysis step. Only one licence seat exists for C, so C was timed one testbench at a time.
- **Repeats:** 5, 3 or 2 timed runs per cell depending on the length of the testbench; yavs and the other simulator are timed on the same core, in turn, and the
  ratio is taken within one pass. For A two passes were taken and the best cell of the two is used.
- **Cores:** one timed process per physical core on the faster chiplet of an AMD Ryzen 9 7950X3D (cores 8 to 15), each timed process pinned.
- **Host and load:** 60 GB RAM, Linux. The host was shared. The run against A was taken first while other jobs ran (load average about 17); the passes against B and C
  were taken while the machine was also in interactive use (load average about 20, with short bursts far higher), which slowed both sides of each pair by a third to a half,
  so absolute seconds are inflated and only the ratios are the comparison. A repeat pass against A under a load of about 25 gave 0.21 warm and 0.57 cold again,
  and passes of a second build of yavs against B and C gave the same ratios within 0.02 (B 0.17 / 0.48, C 0.16 / 0.40).
- **Raw data:** not committed; the full tables and the history of earlier runs are in [`docs/STATUS.md`](https://github.com/ru551n/yavs/blob/main/docs/STATUS.md).
