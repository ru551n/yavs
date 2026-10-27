Waveform equivalence: run locally on 2026-10-27 at 05aa8453fb (yavs build 5dc3cb5d28), references sim1, sim2.
Reproduce: `python3 -I tools/waveq/run.py BIN`

| design | A | B | signals | value changes | equal | different | equal as slice | other shape | only A | only B | differences | A compiled code |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| rtl_ff | yavs-interp | sim1 | 3 | 604 | 3 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_ff | yavs-auto | sim1 | 3 | 604 | 3 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_ff | yavs-llvm | sim1 | 3 | 604 | 3 | 0 | 0 | 0 | 0 | 0 | none | 285 IR instructions |
| rtl_ff | sim2 | sim1 | 3 | 604 | 3 | 0 | 0 | 0 | 0 | 0 | none |  |
| rtl_lfsr | yavs-interp | sim1 | 66 | 33067 | 66 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_lfsr | yavs-auto | sim1 | 66 | 33067 | 66 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_lfsr | yavs-llvm | sim1 | 66 | 33067 | 66 | 0 | 0 | 0 | 0 | 0 | none | 357 IR instructions |
| rtl_lfsr | sim2 | sim1 | 66 | 33067 | 66 | 0 | 0 | 0 | 0 | 0 | none |  |
| rtl_chain | yavs-interp | sim1 | 4 | 605 | 4 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_chain | yavs-auto | sim1 | 4 | 605 | 4 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_chain | yavs-llvm | sim1 | 4 | 605 | 4 | 0 | 0 | 0 | 0 | 0 | none | 261 IR instructions |
| rtl_chain | sim2 | sim1 | 4 | 605 | 4 | 0 | 0 | 0 | 0 | 0 | none |  |
| rtl_fan | yavs-interp | sim1 | 4 | 405 | 4 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_fan | yavs-auto | sim1 | 4 | 405 | 4 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_fan | yavs-llvm | sim1 | 4 | 405 | 4 | 0 | 0 | 0 | 0 | 0 | none | 137 IR instructions |
| rtl_fan | sim2 | sim1 | 4 | 405 | 4 | 0 | 0 | 0 | 0 | 0 | none |  |
| rtl_bus | yavs-interp | sim1 | 4 | 2005 | 4 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_bus | yavs-auto | sim1 | 4 | 2005 | 4 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_bus | yavs-llvm | sim1 | 4 | 2005 | 4 | 0 | 0 | 0 | 0 | 0 | none | 261 IR instructions |
| rtl_bus | sim2 | sim1 | 4 | 2005 | 4 | 0 | 0 | 0 | 0 | 0 | none |  |
| rtl_ram | yavs-interp | sim1 | 16390 | 30385 | 16390 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_ram | yavs-auto | sim1 | 16390 | 30385 | 16390 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_ram | yavs-llvm | sim1 | 16390 | 30385 | 16390 | 0 | 0 | 0 | 0 | 0 | none | 675 IR instructions |
| rtl_ram | sim2 | sim1 | 16390 | 30385 | 16390 | 0 | 0 | 0 | 0 | 0 | none |  |
| lfsr64_xor | yavs-interp | sim1 | 67 | 33567 | 67 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| lfsr64_xor | yavs-auto | sim1 | 67 | 33567 | 67 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| lfsr64_xor | yavs-llvm | sim1 | 67 | 33567 | 67 | 0 | 0 | 0 | 0 | 0 | none | 382 IR instructions |
| lfsr64_xor | sim2 | sim1 | 67 | 33567 | 67 | 0 | 0 | 0 | 0 | 0 | none |  |
| lfsr64 | yavs-interp | sim1 | 67 | 33567 | 67 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| lfsr64 | yavs-auto | sim1 | 67 | 33567 | 67 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| lfsr64 | yavs-llvm | sim1 | 67 | 33567 | 67 | 0 | 0 | 0 | 0 | 0 | none | 309 IR instructions |
| lfsr64 | sim2 | sim1 | 67 | 33567 | 67 | 0 | 0 | 0 | 0 | 0 | none |  |
| neorv32_single_min | yavs-interp | sim1 | 13466 | 15877072 | 13466 | 0 | 133 | 0 | 1045 | 0 | none | 0 |
| neorv32_single_min | yavs-auto | sim1 | 13466 | 15877072 | 13466 | 0 | 133 | 0 | 1045 | 0 | none | 0 |
| neorv32_single_min | yavs-llvm | sim1 | 13466 | 15877072 | 13466 | 0 | 133 | 0 | 1045 | 0 | none | 128233 IR instructions |
| grlib_leon3mp | yavs-interp | sim1 | 13897 | 33197020 | 13897 | 0 | 230 | 0 | 3843 | 0 | none | 0 |
| grlib_leon3mp | yavs-auto | sim1 | 13897 | 33197020 | 13897 | 0 | 230 | 0 | 3843 | 0 | none | 4 functions |
| grlib_leon3mp | yavs-llvm | sim1 | 13897 | 33197020 | 13897 | 0 | 230 | 0 | 3843 | 0 | none | 226142 IR instructions |
| uvvm_sb_slv | yavs-interp | sim1 | 3 | 3 | 3 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| uvvm_sb_slv | yavs-auto | sim1 | 3 | 3 | 3 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| uvvm_sb_slv | yavs-llvm | sim1 | 3 | 3 | 3 | 0 | 0 | 0 | 0 | 0 | none | 42834 IR instructions |
| uvvm_util_rand | yavs-interp | sim1 | 3 | 3 | 3 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| uvvm_util_rand | yavs-auto | sim1 | 3 | 3 | 3 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| uvvm_util_rand | yavs-llvm | sim1 | 3 | 3 | 3 | 0 | 0 | 0 | 0 | 0 | none | 84159 IR instructions |
| uvvm_axistream_vvc | yavs-interp | sim1 | 154 | 785584 | 150 | 4 | 0 | 0 | 20 | 1 | reference process-order-in-cycle: 4 | 0 |
| uvvm_axistream_vvc | yavs-auto | sim1 | 154 | 785584 | 150 | 4 | 0 | 0 | 20 | 1 | reference process-order-in-cycle: 4 | 0 |
| uvvm_axistream_vvc | yavs-llvm | sim1 | 154 | 785584 | 150 | 4 | 0 | 0 | 20 | 1 | reference process-order-in-cycle: 4 | 69265 IR instructions |
| uvvm_methods_log | yavs-interp | sim1 | 3 | 3 | 3 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| uvvm_methods_log | yavs-auto | sim1 | 3 | 3 | 3 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| uvvm_methods_log | yavs-llvm | sim1 | 3 | 3 | 3 | 0 | 0 | 0 | 0 | 0 | none | 14064 IR instructions |
| uvvm_func_cov | yavs-interp | sim1 | 4 | 4 | 4 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| uvvm_func_cov | yavs-auto | sim1 | 4 | 4 | 4 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| uvvm_func_cov | yavs-llvm | sim1 | 4 | 4 | 4 | 0 | 0 | 0 | 0 | 0 | none | 45937 IR instructions |
| osvvm_axis_random | yavs-interp | sim1 | 223 | 109554 | 202 | 21 | 0 | 0 | 2 | 0 | reference process-order-in-cycle: 21 | 0 |
| osvvm_axis_random | yavs-auto | sim1 | 223 | 109554 | 202 | 21 | 0 | 0 | 2 | 0 | reference process-order-in-cycle: 21 | 0 |
| osvvm_axis_random | yavs-llvm | sim1 | 223 | 109554 | 202 | 21 | 0 | 0 | 2 | 0 | reference process-order-in-cycle: 21 | 55734 IR instructions |
| osvvm_axi4_random | yavs-interp | sim1 | 531 | 198238 | 491 | 40 | 0 | 0 | 0 | 0 | reference process-order-in-cycle: 40 | 0 |
| osvvm_axi4_random | yavs-auto | sim1 | 531 | 198238 | 491 | 40 | 0 | 0 | 0 | 0 | reference process-order-in-cycle: 40 | 0 |
| osvvm_axi4_random | yavs-llvm | sim1 | 531 | 198238 | 491 | 40 | 0 | 0 | 0 | 0 | reference process-order-in-cycle: 40 | 69313 IR instructions |

Not configured (no waveform keys for the kind of design in tools/sims.local.conf): sim2 for 9 designs

Differences explained:

- reference process-order-in-cycle: the verification components take IDs from a shared counter in the order their processes run in one cycle; the LRM leaves that order open (14.7.5.3) and the references differ among themselves (docs/design/simulator-discrepancies.md); every bus signal is equal
