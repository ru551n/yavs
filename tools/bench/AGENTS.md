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
| `run.py` | The harness: `--list`, `BIN` (one binary), `--compare A B`, `--bench a,b`, `--group internal\|external\|all`, `--runs N` (default 3), `--sims sim1,sim2`, `--threshold`, `--fail-on-regression`, `--label`, `--out` |
| `designs/*.vhd` | Checked-in designs written for this harness; top level `bench_<name>`, default generics |
| `vc_tests.tsv` | The fixed 30 test cases (`testbench<TAB>test case`) of benchmark `vunit_vc30` |

Benchmarks (`run.py --list`):

| Name | Group | What |
|---|---|---|
| `compute_loop`, `ff_rtl`, `resolved_bus`, `delta_chain`, `textio_heavy`, `protected_scoreboard` | internal | Sequential compute, many flip-flops, a resolved bus, a deep delta chain, text I/O, a protected-type scoreboard: each 1 to 4 s |
| `elab_heavy` | internal | 20000 small entity instances: elaboration and initialisation cost |
| `vunit_vc30` | external | 30 fixed test cases of the external test runner's verification components, one `yavs run` each (summed per iteration) |
| `uvvm_axistream_vvc_tb` | external | The AXI-stream verification component testbench, first generic set, capped with `--stop-time=10us` (it runs far over 5 minutes in full) |
| `uvvm_util_rand_tb` | external | The utility library's random testbench (`rand_basic`) |
| `osvvm_axistream` | external | One AXI-stream testbench (`TbStream_AxiSendGetRandomParam2`) of the other verification library's `.pro` scripts |
| `vai_pe_array`, `vai_window_gen` | external | Two long testbenches of a local, read-only testbench repository; needs `YAVS_BENCH_VHDL_AI_TEST=<checkout>`; skipped when unset |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Phase times come from `yavs run --time-report` (analyze, elaborate, init, simulate); a binary without the flag is still timed in total | One definition of the phases, zero cost off | `docs/cli-contract.md` 5, #387 |
| CPU totals are exact (`rusage` of the child), phase CPU is from `--time-report` (10 ms steps); the report shows the median over runs | CPU time is stable on a shared host, wall time is not | #387 |
| `--compare` runs rep-major (all benchmarks, then the next rep) and swaps the A/B order every rep | Host load drifts; it must hit both binaries alike | #387 |
| A regression is a total CPU ratio above `1 + --threshold` (default 10%) and at least 0.05 s; with an identical binary on a busy host expect noise up to about 5% | Do not chase noise | #387 |
| Each binary gets its own library directories (`work/<sha256 of the binary>/...`); preparation (analysis, recording) is cached | A and B never read each other's libraries | #387 |
| The suite drivers of `tools/suites` are reused for the external libraries: they run with a recording stand-in for yavs that really analyzes but only writes down each `run` command; the harness replays the recorded command with `--time-report` | Same libraries, files and generics as the drivers, no second Tcl or test-list reader | #387, `tools/suites/AGENTS.md` |
| Reference simulators (`--sims`) time the internal designs only, as one number (elaborate + run, analysis excluded), and are shown by label only. Labels map to commands through `BENCH_SIM1` etc. (else `DISCREP_SIM1` etc., else the untracked `tools/discrepancies/sims.local.conf`); never write a tool name, path or licence value into the repository or the JSON beyond the label | Root rule 3 and 11; reuses `tools/discrepancies/run.py` templates | #387 |
| Every command runs under `timeout 300`; reference simulators also under `systemd-run --user --scope -p MemoryMax=2G`. One process at a time | Root rules 15 and 16 | root `AGENTS.md` |
| Peak RSS (`rss`, MiB, shown as `peak MiB`) is the child's own, measured with `time -f %M` inside the `timeout` wrapper. Without a `time` binary it is 0 | A wait4 rusage of our own child would be floored by the harness RSS at fork | #409 |
| The JSON file holds raw samples (`wall`, `cpu`, `rss`, `phases`, `rc`, `error`) and medians per benchmark and binary, plus the binary hash and host CPU count | Tracking over time | #387 |

## 4. Invariants and pitfalls

- Build both binaries from `--release` with the same features; compare binaries built the same way.
  Other agents load the host: for a decision, repeat with `--runs 5` and look at CPU, not wall.
- The external benchmarks need the corpora (`tools/fetch-corpus.sh`, `YAVS_CORPUS`); the first run per binary
  analyzes the libraries (minutes) and caches them. A benchmark that cannot be prepared is skipped with the
  reason on stderr; a benchmark that exits non-zero is reported with its first error and left out of the medians.
- The designs must stay deterministic and finish by themselves. Changing a design or `vc_tests.tsv` invalidates
  earlier JSON files: say so in the commit.
- Do not add a benchmark that needs a manual install.

## 5. How to work here

```sh
cargo build --release -p yavs
python3 -I tools/bench/run.py --list
python3 -I tools/bench/run.py target/release/yavs --group internal                      # one binary
python3 -I tools/bench/run.py --compare /path/to/yavs-before target/release/yavs --runs 5   # A/B, all benchmarks
python3 -I tools/bench/run.py --compare A B --bench elab_heavy,vunit_vc30 --fail-on-regression
BENCH_SIM1=<template-name> python3 -I tools/bench/run.py --compare A B --group internal --sims sim1
```

- No diagnostic codes.

## 6. Coordination

- #387 (this harness). Slow spots it found are filed as `perf` issues; the baseline is in `STATUS.md` ("Performance").
