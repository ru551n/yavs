# AGENTS.md: tools/bench/ (shared performance benchmarks)

Read the root `AGENTS.md` and `tools/AGENTS.md` first. This file is for an agent that measures a
performance change. Everyone measures with this one harness, so numbers are comparable (#387).

## 1. Purpose and boundaries

- A fixed benchmark set, per-phase timings, an interleaved A/B comparison of two yavs binaries and a
  JSON record of every run. It measures; it never judges a change by itself and never edits a binary.
- External benchmarks are referenced by path or environment variable and never copied into the repository
  (root rule 7). Python is stdlib only (`python3 -I`). Scratch and results are under
  `~/.cache/yavs-bench` (`YAVS_BENCH_CACHE`), never in the repository.

## 2. Map

| Path | What |
|---|---|
| `run.py` | The harness: `--list`, `BIN` (one binary), `--compare A B`, `--bench a,b`, `--group internal\|h2h\|external\|all`, `--runs N` (default 3), `--sims sim1,sim2`, `--threshold`, `--fail-on-regression`, `--label`, `--out`; head-to-head mode `--ref sim1,sim2 [--cores L] [--tiers interp,auto] [--instructions] [--tiny]` (section 7) |
| `tiny.py` | Head-to-head on the tiny-test corpus `tests/corpus`: per-test wall time of one run, yavs against the references (section 7) |
| `designs/*.vhd` | Checked-in designs written for this harness; top level `bench_<name>`, default generics (the head-to-head designs carry the parameters their results are quoted at) |
| `vc_tests.tsv` | The fixed 30 test cases (`testbench<TAB>test case`) of benchmark `vunit_vc30` |

Benchmarks (`run.py --list`):

| Name | Group | What |
|---|---|---|
| `compute_loop`, `ff_rtl`, `resolved_bus`, `delta_chain`, `textio_heavy`, `protected_scoreboard` | internal | Sequential compute, many flip-flops, a resolved bus, a deep delta chain, text I/O, a protected-type scoreboard: each 1 to 4 s |
| `protected_queue` | internal | A byte queue on pointer tables behind protected types with thin subprograms in front, the way a verification library keeps its queues (#493): protected method calls, copies of table records, access tables |
| `calls` | internal | 3 million calls of one-line functions (#431): frame entry and exit |
| `strings_heavy`, `access_heavy`, `report_heavy`, `file_io` | internal | Message strings (`image`, `to_string`, concatenation, search), access types as queues and buffers use them, many `report`/`assert` lines, text and binary file I/O (#428) |
| `ieee_heavy` | internal | `numeric_std` and `std_logic_1164` operations on vectors: conversions, arithmetic, comparison, resize, shifts, logical operators, slices (#428) |
| `elab_heavy` | internal | 20000 small entity instances: elaboration and initialisation cost |
| `vunit_vc30` | external | 30 fixed test cases of the external test runner's verification components, one `yavs run` each (summed per iteration) |
| `uvvm_axistream_vvc_tb` | external | The AXI-stream verification component testbench, first generic set, capped with `--stop-time=10us` (it runs far over 5 minutes in full) |
| `uvvm_util_rand_tb` | external | The utility library's random testbench (`rand_basic`) |
| `uvvm_util_queue_tb`, `uvvm_sb_slv_tb` | external | The utility library's generic queue testbench and the scoreboard library's `generic_sb_slv_tb` (the queue and scoreboard maintenance testbenches) |
| `osvvm_axistream` | external | One AXI-stream testbench (`TbStream_AxiSendGetRandomParam2`) of the other verification library's `.pro` scripts |
| `vai_pe_array`, `vai_window_gen` | external | Two testbenches of a local, read-only testbench repository (the same test cases and generics as the realbench tests `vai_pe_array` and `vai_window_gen_interlock`; `VAI_TBS` in `run.py`); needs `YAVS_BENCH_VHDL_AI_TEST=<checkout>`; skipped when unset, or when the testbench does not analyse against the checkout (the reason names the marker file listing the files left over). The file lists follow realbench's: when the checkout moves, change both |

Head-to-head designs (group `h2h`, run by `--ref`; written for yavs, each has a header comment). The old names are the
ones used in `docs/STATUS.md` (performance sections of 2026-10-20 and 2026-10-21) and in issues:

| Name | Old name, parameters | What it stresses |
|---|---|---|
| `rtl_ff` | `ff`, 4096 registers x 2000 cycles | Many one-bit clocked processes, scalar signal updates |
| `rtl_lfsr` | `lfsr`, 512 x 16-bit x 5000 | Vector slice and concatenation, element-wise assignment to an array signal |
| `rtl_chain` | `chain`, 1000 stages x 2000 | A long chain of concurrent assignments: delta cycles, one event per stage |
| `rtl_fan` | `fan`, 4000 processes x 5000 | Wake-up and suspend of many near-idle processes on one clock |
| `rtl_bus` | `bus` (top `bus_tb`), 32 drivers x 20000 | Resolved vector bus, driver bookkeeping |
| `rtl_ram` | `ram`, 16384 deep x 20000 | Indexed signal assignment into a large array signal |
| `lfsr64_xor` | row 2a, `rtl`, `-gNREG=64 -gCYCLES=5000` | 64 LFSR processes plus a 64-way XOR reduction each cycle |
| `lfsr64` | row 2b, `rtl2`, `-gNREG=64 -gCYCLES=20000` | The same without the reduction: process wake-up cost |
| `compute_h2h` | row 1, `compute`, `-gN=20000000` | One sequential integer/real loop, no signals |
| `xor_sig` | `xr`; generics now N=10000000 (was 100000), SIG | One operator on unsigned against a signal or a variable (micro-benchmark) |
| `xor_types` | `xr2`; generics now N=10000000 (was 100000), K | One operator on unsigned, std_logic_vector, bit_vector or `+` (micro-benchmark, K 0 to 3) |

Other rows of the 2026-10-21 head-to-head tables map to existing benchmarks: row 3 `uvvm_sb_slv_tb`, `osvvm_axistream`,
`uvvm_util_rand_tb`, `uvvm_axistream_vvc_tb`, `strings_heavy`, `report_heavy`, `textio_heavy`, `protected_scoreboard`,
`elab_heavy` (row f). Row 4 (`TbStream_AxiSendGetRandomParamAsync2`) is not registered. The external rows run on yavs only
(`--ref` times the internal and h2h designs).

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Phase times come from `yavs run --time-report` (analyze, elaborate, init, simulate); a binary without the flag is still timed in total | One definition of the phases, zero cost off | `docs/cli-contract.md` 5, #387 |
| CPU totals are exact (`rusage` of the child), phase CPU is from `--time-report` (10 ms steps); the report shows the median over runs | CPU time is stable on a shared host, wall time is not | #387 |
| `--compare` runs rep-major (all benchmarks, then the next rep) and swaps the A/B order every rep | Host load drifts; it must hit both binaries alike | #387 |
| A regression is a total CPU ratio above `1 + --threshold` (default 10%) and at least 0.05 s; with an identical binary on a busy host expect noise up to about 5% | Do not chase noise | #387 |
| Each binary gets its own library directories (`work/<sha256 of the binary>/...`); preparation (analysis, recording) is cached | A and B never read each other's libraries | #387 |
| The suite drivers of `tools/suites` are reused for the external libraries: they run with a recording stand-in for yavs that really analyzes but only writes down each `run` command; the harness replays the recorded command with `--time-report` | Same libraries, files and generics as the drivers, no second Tcl or test-list reader | #387, `tools/suites/AGENTS.md` |
| Reference simulators (`--sims`) time the internal designs only, as one number (elaborate + run, analysis excluded), and are shown by label only. Labels map to commands through `BENCH_SIM1` etc. (else key `bench`, then `DISCREP_SIM1` etc., then key `discrep` of the untracked `tools/sims.local.conf`); never write a tool name, path or licence value into the repository or the JSON beyond the label | Root rule 3 and 11; reuses `tools/discrepancies/run.py` templates | #387 |
| Every command runs under `timeout 300`; reference simulators also under `systemd-run --user --scope -p MemoryMax=2G`. One process at a time | Root rules 15 and 16 | root `AGENTS.md` |
| Peak RSS (`rss`, MiB, shown as `peak MiB`) is the child's own, measured with `time -f %M` inside the `timeout` wrapper. Without a `time` binary it is 0 | A wait4 rusage of our own child would be floored by the harness RSS at fork | #409 |
| `--ref` (head-to-head) times one yavs binary and the references on the same designs: analysis once per tool and not timed, one untimed warm-up run per cell, then `--runs` rounds with the tools interleaved (order alternating every round), median wall, CPU (user plus system of the process tree) and peak RSS (largest process, `time -f %M`), ratios yavs/reference per design. Every timed run is `taskset`-pinned to `--cores` (default: the performance cores of the host, the same set for every tool), under `systemd-run -p MemoryMax=2G`, `timeout 300` and a 6 GB free-memory guard; references also get an unlimited stack. `--instructions` adds one extra `perf stat` run per cell, with the user-space instruction counter of the core type the cores belong to (kernel instructions of a short run vary by hundreds of millions with the page cache; #788); `timed` counts the cycles with the instructions (`cyc`; medians per cell with realbench `--count-runs`), and every cell keeps its run walls and the 1-minute load at each run's start (`walls`, `loads`, #481) | Same method as the `docs/STATUS.md` head-to-head sections; root rules 15 to 19 | #387 |
| Reference templates in `--ref` and `tiny.py` are the `--sims` ones (`load_sims` of `tools/discrepancies/run.py`): a label maps to an `analyse\|\|\|elaborate-and-run` template through `BENCH_<LABEL>`, key `bench`, `DISCREP_<LABEL>` or key `discrep` of the untracked `tools/sims.local.conf`. Several labels may name the same tool run in different ways (an extra label with its own template). No tool name, path or licence value in the repository | Root rules 3 and 11 | #387 |
| `tiny.py` runs the whole process once under `taskset` and a 2 GiB cap (it re-executes itself under `systemd-run`), times each run as wall clock of `bash -c` minus the median of three `true` runs under the same wrapper timed right before it (per cell, so the subtracted value follows the host load; a single number from the start biased two invocations by up to 1 ms on a loaded host and made them incomparable, #569; the spread is printed at the end and the raw values are in the JSON `ov`; two invocations are comparable only if their spreads are alike, otherwise compare with `run.py --compare`), and drops a tool from a test when its analysis fails or its warm-up run crashes, times out or fails to elaborate. Its reference templates (key `bench`) may use the environment variables `STOP_FLAG` (`--stop-time=T` or empty), `RELAXED` and `PSL` (`1` or empty) | Tests take milliseconds: per-run wrappers would dominate | #387 |
| A `--tier=auto` run pinned to one core (`taskset -c N`) has its compile thread on that core too (it keeps the mask it was started with, #681): `auto` and `auto-cold` walls include the compile CPU time, as they did not before 2026-10-24, when the thread moved to every other CPU of the host (the cores of other pinned runs). Do not compare `auto` walls across that change; instructions are unaffected | The thread no longer steals the cores other timed runs are pinned to | #681 |
| `tools/bench/run.py` sets `YAVS_STD_CACHE` to `<cache>/std` (`YAVS_BENCH_CACHE`), never the user's store (#564). Its cold cells (`auto-cold`, fresh library or work dir) therefore see a warm shared std/ieee store after the first call of a binary: the agreed cold basis, comparable to the reference's installed precompiled `ieee`. To measure a cold machine, delete `<cache>/std` first | User decision 2026-10-26 on #564 | #564 |
| The JSON file holds raw samples (`wall`, `cpu`, `rss`, `phases`, `rc`, `error`) and medians per benchmark and binary, plus the binary hash and host CPU count | Tracking over time | #387 |

## 4. Invariants and pitfalls

- Build both binaries with `tools/build-bench.sh` (release plus 64-byte aligned functions, #576; same features for both, own `CARGO_TARGET_DIR`); a plain release build moves some designs by 3 to 33 percent in cycles with identical instructions when unrelated code moves. Release builds and CI artefacts stay unaligned and have no line tables (#600); the script turns line tables on (`CARGO_PROFILE_RELEASE_DEBUG=line-tables-only` is its default) so `perf` and `gdb` show lines, without changing code or timing.
  Other agents load the host: for a decision, repeat with `--runs 5` and look at CPU, not wall.
- The external benchmarks need the corpora (`tools/fetch-corpus.sh`, `YAVS_CORPUS`); the first run per binary
  analyzes the libraries (minutes) and caches them. A benchmark that cannot be prepared is skipped with the
  reason on stderr; a benchmark that exits non-zero is reported with its first error and left out of the medians.
- The designs must stay deterministic and finish by themselves. Changing a design or `vc_tests.tsv` invalidates
  earlier JSON files: say so in the commit.
- Do not add a benchmark that needs a manual install.
- `perf record` writes an empty file ("increase perf_event_mlock_kb") when parallel sessions have used up the per-user buffer budget: add `-m 64`. Layers of one run: `YAVS_PERFMAP=1 YAVS_LLVM_PERFMAP=1 perf record -m 64 -e instructions:u -c 4000000 ...` then `perf report --no-children --sort comm,sym`; with `--tier=auto` the compile thread is the command `yavs-tier-up`, compiled code `f<id>_n_<name>`.

## 5. How to work here

```sh
tools/build-bench.sh                                                          # release + aligned functions (#576)
python3 -I tools/bench/run.py --list
# where does one long run spend its time (VHDL level)? yavs run --profile (see docs/running-faster.rst)
python3 -I tools/bench/run.py target/release/yavs --group internal                      # one binary
python3 -I tools/bench/run.py --compare /path/to/yavs-before target/release/yavs --runs 5   # A/B, all benchmarks
python3 -I tools/bench/run.py --compare A B --bench elab_heavy,vunit_vc30 --fail-on-regression
BENCH_SIM1=<template-name> python3 -I tools/bench/run.py --compare A B --group internal --sims sim1
```

- Extra `yavs run` flags for every benchmark: `YAVS_BENCH_RUN_FLAGS="--wave={out} --wave-variables"` (`{out}` is a scratch
  file the harness deletes after each run; the report then shows the FST size). Every run also reports the peak RSS
  (`wait4`, per child; the maximum over the commands of a benchmark).
- Extra `yavs run` flags for every benchmark: `YAVS_BENCH_RUN_FLAGS="--wave={out} --wave-variables"` (`{out}` is a scratch
  file the harness deletes after each run; the report then shows its size as `FST MiB`).
- No diagnostic codes.

## 7. Head-to-head against reference simulators

Method (the same for every number quoted in `docs/STATUS.md`):

- Binaries under test come from `tools/build-bench.sh` (`RUSTFLAGS="-C llvm-args=-align-all-functions=6" cargo build --release -p yavs`; Cargo profiles cannot set `llvm-args`, so it is a flag, not a profile). Check with `nm -n --defined-only BIN | awk '$2~/[tT]/' | head`: function starts are multiples of 0x40. A release binary is not comparable with an aligned one.
- Pin every timed run, of every tool, to the performance cores of the host (`--cores`, default read from the CPU topology;
  on a hybrid host pass a subset when other agents use some of them). Run builds, analysis and test suites elsewhere
  (`taskset -c <efficiency cores> cargo ...`) so they do not disturb the timings. Say which cores were used.
- Analysis is excluded: libraries are analysed once per tool before timing (the untimed preparation batches consecutive files of a library in one call like the integrations, #745: `analyse_files`, a failing call repeated file by file); the timed part is elaborate + initialise + run
  (references that need a separate elaboration command are summed).
- Interleave the tools inside each round and swap the order every round; take the median of `--runs` (use 5 for a decision).
- Report wall, CPU and peak RSS. When the host is loaded, wall time is noise: also pass `--instructions` and quote cycles
  or instructions (`perf stat` counters are per core type: a run pinned to efficiency cores counts the other event).
- Cells below 0.1 s carry up to +-15%.

Set up the references once (untracked, never committed): `tools/sims.local.conf` (`tools/simconf.py`), section `[label]`, key `bench` or `discrep` =
`analyse-snippet|||run-snippet` (placeholders `{file} {top} {std} {std2} {work} {res}`),
or the same through `BENCH_<LABEL>` environment variables. Two labels may use one tool in two ways.

```sh
# designs (group h2h) with both yavs tiers, the instruction counts, then the tiny-test corpus: one command
python3 -I tools/bench/run.py target/release/yavs --ref sim1,sim2 --tiers interp,auto --instructions --runs 5 --tiny
python3 -I tools/bench/run.py target/release/yavs --ref sim1 --bench rtl_ff,lfsr64 --cores 2,3   # a few designs
python3 -I tools/bench/tiny.py target/release/yavs --ref sim1,sim2 --filter delta/                  # a part of the corpus
```

`--tiers` entries may carry environment variables, `clift+YAVS_CLIFT_NATIVE=0` (the tier with `env VAR=VALUE` in front, for measurement knobs; the column is named with the suffix). `--tiers auto` and `llvm` need a yavs built with `--features llvm`; a tier that fails is shown as FAILED in its row. Results
(raw samples and medians) go to `<cache>/results/<time>-ref*.json` and `-tiny*.json`.

## 6. Coordination

- Real testbenches of real projects (about 30, diverse; baseline against the reference, A/B between binaries): `tools/realbench/AGENTS.md`. Use it besides these designs when tuning, so a choice is not fitted to them.

- #387 (this harness). Slow spots it found are filed as `perf` issues; the baseline is in `docs/STATUS.md` ("Performance").
