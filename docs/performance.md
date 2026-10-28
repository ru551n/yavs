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
| C: a commercial simulator (29 of the 41 testbenches; it could not analyse or run the others) | 0.16 | 0.40 | 24 / 22 of 29 | 37 s / 94 s |

With `grlib_leon3mp` included, the ratio against A is 0.22 warm and 0.59 cold (yavs takes about 2.9 times as long on that one testbench).
Against C, only the run itself (elaboration and simulation, without analysing the libraries) can be compared on 40 testbenches: ratio 0.28 warm.
The "total time" column is the sum over the testbenches in the row; the pass against B ran while the host was busier than the pass against A,
so compare ratios, not those sums, between rows.

## Where yavs is slower

Against A, yavs was slower warm on 4 of the 41 testbenches (ratios 1.25 to 1.93): the two wide-vector designs `fletcher_bufferwriter_512`
and `sc5_fifo_repack`, one record-heavy testbench, and an RTL datapath with a windowing generator; cold also `poc_stat_histogram` and
`poc_arith_addw` (1.29 and 1.01). Against C the long CPU-bound testbenches (`neorv32_*`, `poc_arith_addw`, `fletcher_bufferwriter_512`) are
slower in yavs, by 1.3 to 1.8 times warm: C runs the simulation itself faster on these, while yavs is far ahead
on start-up and analysis, which is most of a short testbench. Against B yavs was faster on every testbench warm.
The cold ratio is higher than the warm one because analysing the libraries and compiling hot code happens in the first run only.
Among the hand-written RTL designs, clocked designs with very many cheap processes are where fully compiled simulators lead.

## Measure your own design

`--time-report` and `--profile` show where a run spends its time, and the tiers and the cache
decide how much of it is compiling and analysing: see [Running faster](running-faster.rst). Time on
an otherwise idle machine, repeat, and compare medians; a short run varies by tens of percent.

## How we measured

The testbenches come from open-source VHDL projects (verification libraries, FPGA libraries, processor
designs), each run as its project's own scripts would: analysing the libraries and the testbench's files,
elaborating and running. yavs is the release build at its default `--tier=auto`. Every pair of timings ran on
the same CPU core, in turn, on a shared Linux host (AMD Ryzen 9 7950X3D), repeated two to five times. The
other simulators are not named; all of them run compiled or generated code. The per-testbench tables, the
method in full and earlier runs are in
[`docs/STATUS.md`](https://github.com/ru551n/yavs/blob/main/docs/STATUS.md) ("Real-world performance suite").
