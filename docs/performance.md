# Performance: yavs against an established open-source VHDL simulator

This sheet compares the wall-clock time of yavs with that of an established open-source VHDL simulator (called "the reference" below) on
real testbenches of third-party VHDL projects. It holds the current head-to-head run; the full history, the breakdowns and the
per-phase analysis are in the head-to-head sections of [`docs/STATUS.md`](STATUS.md).

## Result

Run of 2026-10-27 on commit `5b2beb73` (head-to-head 7). Ratio = yavs time / reference time, geometric mean over 41 testbenches
(`grlib_leon3mp` is reported but kept out of the mean). Below 1 means yavs is faster.

| Basis | Warm cache | Cold cache |
| --- | ---: | ---: |
| **Full total: analysis of libraries and own files + elaboration + run** | **0.22** | **0.59** |
| Own files + elaboration + run | 0.22 | 0.49 |
| Elaboration + run only | 0.22 | 0.48 |
| Instructions executed in the run phase | 0.30 | 0.59 |

Summed over the 41 testbenches the full total is 38 s (yavs) against 70 s (reference) warm and 57 s against 70 s cold. yavs is faster on 36 of 41 warm and
on 33 of 41 cold. With `grlib_leon3mp` (a LEON3 system, 2.7 times slower than the reference) the geometric means are 0.23 warm and 0.61 cold.
Ten small hand-written RTL designs (`tools/bench/designs`, group `h2h`) give 0.50 warm and 0.61 cold; `rtl_ff`, `rtl_lfsr` and `lfsr64` are still slower than the reference.

## Method

- **What is timed:** the full total, that is analysis of the project's libraries and of the testbench's own files, elaboration and the run, as the wall time of the whole script.
  The two tools get the same analysis grouping: consecutive files of one library and standard are analysed in one call.
- **Tier:** `--tier=auto` (the interpreter starts the run and hot functions are compiled on a background thread; the compiled-code cache is on disk).
  The reference always runs compiled code.
- **Cold and warm:** cold starts with fresh libraries and an empty compiled-code cache; warm repeats the same files with the caches of the cold run.
- **Repeats:** 5 timed runs per cell, interleaved between the tools, two complete passes, the best cell of the two passes. The two passes agree (warm 0.213 and 0.214, cold 0.587 and 0.582).
- **Cores:** one timed process per physical core, on three claimed cores of the faster chiplet (cores 9 to 11); the cells of the two tools are interleaved.
  Three long testbenches were timed as controls at the start and at the end of the run to show host drift.
- **Build:** the benchmark build of yavs with the LLVM code generator (`tools/build-bench.sh --features llvm`), one binary for every cell.
- **Tool:** `tools/realbench`; the designs come from `tools/bench`. Raw results are not committed.

## Host

AMD Ryzen 9 7950X3D (16 cores, 32 threads), 60 GB RAM, Linux. The host was shared with other jobs (load average 5 to 10, no quiet window), so absolute seconds
of the late cells carry drift for both tools; the ratios are the comparison. Controls (run phase, reference / yavs seconds), start then end: `fletcher_bufferwriter_512`
1.85 / 4.03 then 1.91 / 3.93; `poc_arith_addw` 5.23 / 5.19 then 6.01 / 5.85; `vai_window_gen_interlock` 5.88 / 7.53 then 7.81 / 9.26.

## Per-test results

Ratio yavs / reference of the full total, and the seconds behind it (full total, warm unless stated). Sorted by the warm ratio, slowest first.
Category is the kind of workload the testbench stresses.

| Testbench | Category | Ratio warm | Ratio cold | yavs warm s | Reference warm s | yavs cold s |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| `grlib_leon3mp` (outside the mean) | cpu | 2.67 | 2.84 | 38.22 | 14.32 | 40.71 |
| `fletcher_bufferwriter_512` | wide-vectors | 1.98 | 2.15 | 3.99 | 2.02 | 4.33 |
| `fletcher_bufferreader` | records | 1.36 | 1.43 | 1.61 | 1.18 | 1.67 |
| `vai_window_gen_interlock` | rtl-datapath | 1.29 | 1.98 | 8.07 | 6.24 | 12.38 |
| `sc5_fifo_repack` | wide-vectors | 1.12 | 1.97 | 1.29 | 1.15 | 2.27 |
| `poc_stat_histogram` | memory | 1.04 | 2.00 | 0.55 | 0.53 | 1.06 |
| `poc_arith_addw` | numeric | 0.95 | 1.12 | 5.21 | 5.51 | 6.16 |
| `hdlm_lfsr` | numeric | 0.84 | 1.07 | 1.54 | 1.83 | 1.93 |
| `sc5_timestamp` | fsm | 0.72 | 1.02 | 0.41 | 0.57 | 0.58 |
| `neorv32_single_min` | cpu | 0.70 | 0.80 | 2.62 | 3.72 | 2.98 |
| `poc_cache_cpu` | fsm | 0.65 | 0.74 | 0.64 | 0.98 | 0.73 |
| `neorv32_single_cache` | cpu | 0.55 | 0.65 | 2.30 | 4.16 | 2.72 |
| `uvvm_axistream_vvc` | protected | 0.52 | 0.61 | 2.53 | 4.83 | 2.97 |
| `uvvm_sb_slv` | protected | 0.44 | 0.62 | 0.68 | 1.53 | 0.96 |
| `uvvm_util_rand` | protected | 0.44 | 0.70 | 0.79 | 1.79 | 1.24 |
| `neorv32_dual_full` | cpu | 0.44 | 0.53 | 2.02 | 4.61 | 2.46 |
| `poc_fifo_ic_assembly` | fsm | 0.26 | 0.50 | 0.11 | 0.42 | 0.21 |
| `osvvm_axi4_random` | protected | 0.25 | 0.46 | 0.69 | 2.78 | 1.27 |
| `olo_fix_cordic_rot` | numeric | 0.16 | 0.50 | 0.23 | 1.48 | 0.73 |
| `osvvm_axis_random` | protected | 0.15 | 0.39 | 0.39 | 2.52 | 0.97 |
| `hdlm_axis_bfm` | protected | 0.14 | 0.42 | 0.13 | 0.95 | 0.39 |
| `hdlm_axi_crossbar` | records | 0.12 | 0.43 | 0.14 | 1.13 | 0.47 |
| `fletcher_bufferwriter_prim32` | records | 0.12 | 0.24 | 0.09 | 0.76 | 0.18 |
| `hdlm_ring_buffer` | memory | 0.12 | 0.40 | 0.12 | 1.02 | 0.40 |
| `hdlm_unsigned_divider` | numeric | 0.12 | 0.41 | 0.11 | 0.94 | 0.38 |
| `hdlm_sine_generator` | numeric | 0.12 | 0.38 | 0.12 | 1.04 | 0.39 |
| `hdlm_axi_fifo` | records | 0.12 | 0.42 | 0.13 | 1.13 | 0.46 |
| `uvvm_func_cov` | protected | 0.11 | 0.40 | 0.15 | 1.33 | 0.53 |
| `hdlm_register_file` | records | 0.11 | 0.42 | 0.12 | 1.10 | 0.45 |
| `vunit_axi_dma` | records | 0.11 | 0.44 | 0.10 | 0.95 | 0.42 |
| `en_cl_fix_pkg` | numeric | 0.10 | 0.47 | 0.06 | 0.58 | 0.27 |
| `hdlm_fifo_sync` | rtl-datapath | 0.10 | 0.43 | 0.10 | 0.99 | 0.41 |
| `vai_pe_array` | numeric | 0.09 | 0.38 | 0.18 | 1.98 | 0.75 |
| `vai_weight_buffer` | memory | 0.09 | 0.46 | 0.13 | 1.43 | 0.61 |
| `vunit_logging` | strings-textio | 0.09 | 0.39 | 0.08 | 0.90 | 0.35 |
| `poc_uart_rx` | fsm | 0.08 | 0.42 | 0.03 | 0.36 | 0.15 |
| `olo_axi_master_full` | records | 0.08 | 0.44 | 0.11 | 1.41 | 0.61 |
| `olo_intf_i2c_master` | bus-tristate | 0.07 | 0.52 | 0.08 | 1.09 | 0.56 |
| `olo_base_cam` | memory | 0.07 | 0.43 | 0.10 | 1.37 | 0.58 |
| `olo_fix_madd` | numeric | 0.07 | 0.44 | 0.09 | 1.26 | 0.55 |
| `poc_strings` | strings-textio | 0.07 | 0.48 | 0.02 | 0.29 | 0.14 |
| `uvvm_methods_log` | protected | 0.02 | 0.45 | 0.01 | 0.56 | 0.25 |
