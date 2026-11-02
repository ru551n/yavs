# AGENTS.md: tools/realbench/ (real-world performance suite)

Read the root `AGENTS.md`, `tools/AGENTS.md` and `tools/bench/AGENTS.md` (section 7, the timing method) first. This file is
for an agent that tunes performance or measures a performance change on real testbenches.

## 1. Purpose and boundaries

- The designs in `tools/bench/designs` were written for the harness, and tuning on them alone risks overfitting. This suite times
  about 30 testbenches taken from real projects (hdl-modules, Open Logic, the test runner's own examples, PoC, fletcher,
  en_cl_fix, neorv32, the two verification libraries, and the maintainer's own project), chosen for diversity: clocked RTL, FSMs,
  resolved buses, records, `numeric_std` and fixed point, memories, a CPU running software, protected types and scoreboards,
  string and textio code, run lengths from 0.1 s to about 30 s on the reference simulator.
- It measures; it never judges a change and never edits a binary. It reuses the timing code of `tools/bench/run.py`
  (`timed`, `cell`, `instr_event`: pinning, `systemd-run` cap, `timeout 300`, memory guard, medians, `perf stat` instructions).
- Sources are read in place from the corpus cache (`tools/fetch-corpus.sh`) and the maintainer's project checkout; nothing is copied
  into the repository (root rule 7). Python is stdlib only (`python3 -I`). Scratch and results are under `~/.cache/yavs-realbench`
  (`YAVS_REALBENCH_CACHE`).

## 2. Map

| Path | What |
|---|---|
| `run.py` | The runner: `--list`, `BIN --ref sim1 --cores 6,7 [--phases analyze,elab,codegen,run]`, `--compare A B`, `--report JSON`, `--self-test` |
| `tests.toml` | One `[[test]]` per testbench: source, path, top, library, generics, why it is in the set, category (field list in its header) |
| `programs.py` | Builds the program images of the CPU workloads (section 7) from pinned sources into `<cache>/images`; also a command: `programs.py PROGRAM key=value` |
| `window.py` | Short-window A/B on Cranelift against sim1: minutes, not hours (section 8) |
| `perfguard.py` | Guard run by hand: window instructions of a commit against a stored reference, pass or fail per test (section 8a) |
| `cpu/` | The harness's own VHDL and C for the CPU workloads: the Microwatt console, JTAG-socket and testbench replacements and its CoreMark port (source id `@harness` in `tests.toml`) |

`YAVS_BENCH_TIERFLAGS` (extra arguments after `--tier=` of every yavs run cell, e.g. `--tier-heat=1,0,0` to compile every function at its first call) is not part of the cell name: set it for both binaries of a `--compare`.

Categories (`--category`): `rtl-datapath`, `fsm`, `numeric`, `memory`, `records`, `wide-vectors`, `bus-tristate`, `strings-textio`,
`protected`, `cpu`, and `cpubench` (the long CPU workloads of section 7, opt-in). Run `run.py --list` for the current selection.

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Libraries come from the analysis-corpus manifests (`tools/corpus/*.toml`), built once per tool and cached (`lib/<binary hash or label>/<manifest>`); a test copies them into its own world directory and analyses its extra files on top. A `driver` test (OSVVM `.pro` scripts, UVVM maintenance scripts) takes its libraries and run command from a recorded run of `tools/suites` (a recording stand-in for yavs that analyses for real and writes a `run` down); the reference replays the recorded analyse calls | Same libraries, flags and generics as the corpus runners and suite drivers, no second build-description reader | `tools/corpus/AGENTS.md`, `tools/suites/AGENTS.md` |
| Every timed cell has a private world: `interp`, `auto` and `llvm` are warm (an untimed run fills the compilation cache of the world, the timed runs reuse it); `<tier>-cold` copies the pristine world before every timed run (empty compilation cache, includes the dependency libraries). Analysis is never timed; the reference time is elaboration plus run | "Cold and warm cache in separate work dirs" (maintainer, 2026-10-21); `tools/bench/AGENTS.md` section 7 | #505 |
| The shared std/ieee unit store (`YAVS_STD_CACHE`, #564) is set by the tool to `<cache>/std` (the cache directory of the run, `YAVS_REALBENCH_CACHE`), never the user's: "cold" means a fresh library with a store that is warm after the tool's first call (the steady state of an installed tool, comparable to the reference's installed precompiled `ieee`). The `std` directory stays between runs of one cache, so the first cold cell of a new cache or a new binary (the keys include the build) pays the std/ieee analysis once; to measure a cold machine, delete `<cache>/std` before the run | User decision 2026-10-26 on #564 | #564 |
| Timed runs: `--cores` is required (performance cores only, never core 0), one test at a time per core (all cells of a test on the same core, interleaved, order swapped every round), median of `--runs` (default `auto`: 5, 3 or 2 by test length), one extra counted `perf stat` run per cell | Same method as `run.py --ref`, root rule 19 | #387 |
| Four phases are timed separately (`--phases`, default all): `analyze` (the whole compile order of the test; consecutive files of one library and standard go in ONE CALL by default, for yavs and for the reference alike, because integrations batch since #745; `--analyze-calls per-file` gives the old one-call-per-file cells), `elab` (`yavs elab` against the reference's elaboration step), `codegen` (`yavs run --tier=llvm --stop-time=0fs` minus the same with `--tier=interp`, both ending at time 0: the time spent generating machine code before the simulation starts; the reference generates its code inside its elaboration, so the basis is its elaboration cell), `run` (the cells of `--tiers`, as before). Each of `analyze`, `elab`, `codegen` has a `cold` cell (pristine world copied before every timed run: no compilation cache, libraries analysed from scratch) and a `warm` cell (cache present, unchanged sources) | Where start-up time goes, per phase, is the other half of the speed users see (maintainer, 2026-10-22). One call per run of files is how integrations call the analysis (maintainer, 2026-10-25) | #505, #745 |
| Analysis cells run the compile order as one bash script of one tool call per run of consecutive files of one library, standard and flag set (`item_calls`; one call per file with `--analyze-calls per-file`; the reference takes the same grouping through `{file}` as a word of its own in `corpus_cmd`, its multi-file form; output discarded), timed as a whole; `cold` = fresh empty libraries, `warm` = the same files analysed again into the populated libraries (`yavs analyze` has no compilation cache, so the two are expected to be equal for it). The order is the corpus manifests' (only files that passed on every tool, so both do the same work) followed by the test's own files in the order its world needed; a `driver` test uses the files of its recorded calls in recorded order (regrouped like every other list: the grouping of the recorded calls is not kept), the calls into the testbench's own library are the own part. A file that defines a design unit an earlier file of the same call defined starts a new call, for both tools (`item_calls`, a scan of the unit headers): the corpus manifests put several projects into one library and the reference rejects a unit replaced within one call. The untimed setup (`make_pristine`'s fixed point that finds the order of the own files) stays one file per call: it needs each file's own result. The library part (VUnit, OSVVM, UVVM, the project's libraries) and the own files are timed in separate scripts (`-lib`, `-own`; the total is their sum) and the library part is measured once per corpus selection and shared by the tests that use it | Per file mean and the split between library and design are what the maintainer asked for | maintainer, 2026-10-22 |
| A warm yavs cell of `elab`, `codegen` and `run` has two untimed runs before the timed ones: the first run of a closure only leaves a note in the cache, the second makes the entries, the third is the steady state | Otherwise the first timed run is not warm | `docs/cli-contract.md` 3 |
| Passing is exit status 0 (VUnit tests run all test cases, or one named case, with `seed : 12345`). A test that fails on either tool is shown as FAILED in its cell and left out of the means; a cell that cannot be built stops that test, with the reason, not the run | Failing tests must be visible, not silently skipped | maintainer, 2026-10-21 |
| A missing source checkout, a source at another commit than the pin in `tools/fetch-corpus.sh`, a missing test file or manifest stops the run before anything is timed, naming it. The maintainer's own project is read in place from the checkout named by `$YAVS_BENCH_VHDL_AI_TEST` (required for its tests) without a pin; its commit is recorded in the result | Fail early and clearly | root rule 1 |
| The instruction count of a cell is one extra run, counted in user space only (`instructions:u`), and that run must pass like the timed ones | A cold cell's kernel instructions varied from 16 M to 566 M for the same user count of 0.85 G (#788), and a counted run that stopped early counted fewer instructions without a word | #788 |
| Ratios are yavs / reference (single binary) or B / A (`--compare`) of wall, CPU and instructions; the means are geometric, overall and per category, over the tests that passed in both cells | Ratios of very different run lengths | maintainer, 2026-10-21 |
| `--count-runs` counts instructions and cycles (`perf stat`) in every timed run: the cell gets their medians (`ins_med`, `cyc`) next to the counted run's `ins`; every cell keeps its run walls and the 1-minute load at each run's start (`walls`, `loads`) | A verdict on a loaded host needs cycles and the spread per cell, and the load each run saw | #481, user, 2026-10-26 |
| The reference needs the keys `corpus_cmd`, `real_elab` and `real_run` in the untracked `tools/sims.local.conf` (`real_elab`/`real_run`: `{std} {lib} {dir} {relaxed} {gens} {top} {rest}` placeholders; `{lib}:{dir}/{lib}` is the work library and `-L {dir}` style search option for all the others; relaxed is applied at analysis only) | Tracked files know labels only (root rule 11) | `tools/AGENTS.md` section 2a |

## 4. Invariants and pitfalls

- Time only binaries from `tools/build-bench.sh` (64-byte aligned functions, #576); a plain release build shifts the cycles of single tests by up to 30 percent with identical instructions when unrelated code moves. Use the same build for both sides of `--compare`.
- Never put a tool name, path or licence value into `tests.toml`, the results or this file; labels only.
- `tests.toml` changes invalidate earlier result files: say so in the commit. Pick generics as the project's own test setup does
  (its Python or script), not at random; the defaults of a testbench are often invalid combinations.
- A testbench that runs several VUnit test cases in one simulation (`vunit = true`) can fail where the runner would have run
  them one per simulation (state kept between cases); name one case then (`vunit = "test_name"`).
- Testbenches that never finish need `stop` (`--stop-time`); the same flag is passed to both tools.
- Results with a loaded host are noise: check the load average printed at the start and repeat on quiet cores (9 to 15, siblings idle), with `--count-runs`. Wall and counted cycles decide a verdict, instructions only explain it: under `--tier=auto` compiled code can retire fewer instructions and take more cycles (front-end misses), so an instruction ratio would hide exactly that loss. An interpreter change has its own rule: instructions drop and the cycle geomean is not worse.
- `data` files of a test (the GRLIB template reads `prom.srec` and `ram.srec` from its working directory) are copied into the world; the run starts in the world directory.
- `grlib_leon3mp` runs at `std = "1993"` with the `grlib-1993` manifest (GRLIB is VHDL-93 code, decision #629; see `tools/corpus/AGENTS.md` and `docs/STATUS.md` "GRLIB"). Its `stop` is just before the template's own end-of-test failure assertion, so the run ends with status 0; the pass check is the text `Test passed` of the on-chip system test, searched in the whole output (`ok` is matched by `bench/run.py timed`, not in its 200 character tail).
- Excluded testbenches (they fail on yavs or the reference) are listed with the reason in `docs/STATUS.md`
  ("Real-world performance suite, baseline 2026-10-21") and each yavs failure has an issue.
- Do not read the `codegen` ratio as a code generation ratio against the reference: it divides by the reference's whole elaboration (which contains its code generation). Compare yavs `elab-X` + `codegen-X` with the reference `elab-X` for that.
- Scratch worlds are removed after each test; a killed run may leave `~/.cache/yavs-realbench/scratch/*`.

## 5. How to work here

```sh
tools/build-bench.sh --features clift     # release + 64-byte aligned functions (#576), never a plain release build; the compiled tier users get
# --features llvm instead, in a target dir of its own, for the LLVM yardstick column and for the `codegen` phase, which needs `--tier=llvm`
cp ~/.cache/yavs-agents/sims.local.conf tools/sims.local.conf       # untracked
python3 -I tools/realbench/run.py --list
python3 -I tools/realbench/run.py --self-test
# the phases of one test, probe (one run per cell): analyze, elab, run (`codegen` runs `--tier=llvm`: only on a build with the `llvm` feature)
python3 -I tools/realbench/run.py target/release/yavs --ref sim1 --cores 6 --runs 1 --phases analyze,elab,run --tests hdlm_fifo_sync
# baseline: all tests, interp and auto (cold and warm), two performance cores, against the reference
python3 -I tools/realbench/run.py target/release/yavs --ref sim1 --cores 6,7 --tiers interp,auto,auto-cold --phases analyze,elab,run --label baseline
# a probe: one untimed run per cell, pass or fail and the tail of the output
python3 -I tools/realbench/run.py target/release/yavs --ref sim1 --cores 6 --runs 0 --tests hdlm_fifo_sync
# regression check of a tuning change: two binaries interleaved on the same worlds
python3 -I tools/realbench/run.py --compare /path/to/before target/release/yavs --cores 6,7 --tiers interp,auto
python3 -I tools/realbench/run.py --report ~/.cache/yavs-realbench/results/<file>.json     # the markdown again
```

Results (raw samples reduced to medians, plus the markdown) go to `~/.cache/yavs-realbench/results/<time>-<label>.json` and `.md`.

## 6. Coordination

- The baselines and the worst tests are in `docs/STATUS.md` ("Real-world performance suite, baseline 2026-10-21" for run, "Real-world suite: analysis, elaboration and code generation vs sim1, 2026-10-22" for the other phases). Slow spots are `perf` issues.

## 6a. Bounds a performance change is held to (baseline `a95d9ad1`, 2026-11-01, Cranelift bench build)

Set from the best-of-two-passes baseline plus 2 percent (peak RSS 10 percent), so each one can fail. They replace the
numbers of head-to-head 8 (0.206 and 0.571), which the baseline did not reproduce for a reason not found (`docs/STATUS.md`,
"baseline for long CPU tests and large SoC models"). They are read by hand from the report of `run.py` (there is no automatic check).

| what | measured | bound |
|---|---:|---:|
| geomean total warm yavs / sim1, 41 tests without `grlib_leon3mp`, `--tier=auto` | 0.213 | at most 0.217 |
| same, cold | 0.589 | at most 0.601 |
| geomean peak RSS yavs / sim1, 42 tests | 0.382 | at most 0.42 |
| `grlib_leon3mp` peak RSS | 502 MB | at most 552 MB |
| `tools/bench/run.py --ref sim1 --tiers interp,auto`: geomean auto / sim1 of the 10 h2h designs without `xor_sig` | 0.562 | at most 0.573 |
| `tools/bench/run.py --ref sim1 --tiers interp,auto --tiny`: sum of yavs / sum of sim1 on the common tests | 2.42 s / 23.48 s | at most 0.105 |

A miss is a finding to chase on a quiet sitting, not a verdict: compare the two builds interleaved on the same cores before blaming the change,
and name the load. The head-to-head geomean row covers the 10 h2h designs that existed on 2026-11-01; `rec_pair` (#1052) was added after the bound was set and is judged on its own (`tools/bench/AGENTS.md`). Not covered: whole suites with many testbenches in one library (`d789ead2`, #1029), waves and coverage on, more than one thread.

## 7. CPU workloads: CoreMark on NEORV32 and Microwatt (category `cpubench`, #924)

Two processors running a fixed program, to measure the simulator on long clocked runs of real CPUs. Primary number: simulated clock cycles per
wall-clock second (`stop` over `clock`, divided by the wall time of the whole run cell, elaboration and compilation included); next to it the
wall time (cold and warm cache), the elaboration alone (`elab-cold`, `elab-warm`), CPU time, peak RSS and the host load of the timed runs.
The rate against the reference (cycles per second ratio, above 1 is faster) is the last column. Fair comparison (root rule 23): only the compiled
tier `auto` (cold and warm) is compared with the reference; `interp` (opt-in with `--with-interp`) has its own before/after.

One command for a clean measurement (needs 4 quiet performance cores, `9,10,11,12` (rule 19: timed runs on 9-15, SMT siblings idle): the 4-thread cell runs on all four, the other cells on the first;
tests run one at a time; check `uptime` first and re-run if the load is above the cores in use):

```sh
tools/fetch-corpus.sh                                   # neorv32 (with sw/), neorv32-coremark, eembc-coremark, microwatt
tools/build-bench.sh --features clift
export YAVS_RISCV_PREFIX=/path/to/bin/riscv-none-elf-   # or a RISC-V GCC on PATH
python3 -I tools/realbench/run.py target/release/yavs --ref sim1 --cores 9,10,11,12 --cpubench --prepare   # builds the images (and the ziglang venv), once
python3 -I tools/realbench/run.py target/release/yavs --ref sim1 --cores 9,10,11,12 --cpubench
# YAVS_PPC_ZIG=/path/to/zig or YAVS_PPC_PREFIX=powerpc64le-linux-gnu- override the pinned POWER compiler
```

Duration: sim1 needs about 2 minutes for NEORV32 (3.6 M cycles at about 25 k cycles/s) and a bit more than a minute (0.6 M cycles at about 8.7 k cycles/s) for Microwatt; per test yavs cells run one untimed check (two for warm cells), 3 timed runs and one counted run, for 3 cells (`auto`, `auto-cold`, `auto@4`) plus sim1 and the elaboration cells, so expect about 2 hours at the 2026-10-31 speeds (per test a cell costs 6 runs: NEORV32 3 minutes each on yavs, Microwatt 2 to 3.5 minutes; 3 yavs cells plus sim1 and the elaboration cells; `--with-interp` adds a fourth cell) for both tests. `--with-interp` adds the interpreter cell, the slowest. A run that exceeds `--timeout` is reported as failed (rc 124) and the numbers of that cell are lost, so raise it rather than trusting a partial result.

`--cpubench` is shorthand for `--category cpubench --phases elab,run --tiers auto,auto-cold,auto@4 --runs 3 --count-runs --timeout 900
--label cpubench`; any of them can be given to override, and `--with-interp` adds the interpreter tier (opt-in: the longest cell, never compared with the reference). A tier `auto@4` is `--tier=auto --threads=4` on the first 4 `--cores`. A test whose
cross compiler is missing is left out with the reason (printed and in the report); naming it with `--tests` stops instead. `--prepare` builds the images and the
worlds and prints the commands (the first `--prepare` also makes `<cache>/venv`), `programs.py` alone builds an image.

| Decision | Why |
|---|---|
| Programs are built from pinned sources fetched into the corpus cache (`tools/fetch-corpus.sh`), never vendored; the image is stored per pins, parameters and compiler under `<cache>/images/<program>-<key>` and reused | Root rule 7; a build is a few seconds but the toolchains are not always present |
| Nothing is installed outside the realbench cache. The POWER compiler is zig's bundled clang and lld from the PyPI `ziglang` wheel, pinned with hashes in `requirements.txt` and installed by `programs.py` into the harness's own virtual environment `<cache>/venv` (`python3 -m venv` plus `python3 -m pip --python`, network access to PyPI once); `YAVS_PPC_ZIG` (a zig binary) or `YAVS_PPC_PREFIX` (a POWER GCC) override it. The RISC-V compiler is found through `YAVS_RISCV_PREFIX` or PATH and otherwise the test stops naming what is needed | Maintainer decision 2026-10-31 (#924); root rule 9 |
| NEORV32: the CoreMark port (separate repository) is built against the pinned NEORV32 `sw/` with `-DUART0_SIM_MODE` (console text goes to standard output in simulation, not through the baud-timed UART) and `-DITERATIONS=N`; the generated `neorv32_imem_image.vhd` is analysed on top of the corpus library together with the units that depend on it (`neorv32_imem`, `neorv32_top`, the testbench). Edits made on the build copy: the port's unconditional `ITERATIONS` define and the makefile's flag line | The imem image is a VHDL package; the sources of the processor are not touched |
| Microwatt: its simulation uses C helpers (`sim_console`, `sim_bram_helpers`, `sim_jtag_socket`, `glibc_random_helpers`) written for the foreign interface of sim2, which passes strings as bounds-carrying fat pointers. Tried through the foreign interface of sim1 and yavs (`--load`): `from_string` dereferences that fat pointer and crashes (sim1: SIGSEGV at elaboration of the memory), yavs follows the other convention (docs/vhpi.md: an unconstrained array is a pointer followed by its length), so the helpers cannot be used unchanged. The core's debug trace is silenced, see the next row. `cpu/microwatt/` replaces `sim_console.vhdl` (lines of standard output, no input) and `sim_jtag_socket.vhdl` (never a message) and adds `microwatt_bench_tb.vhdl` (the SoC of `core_tb` with the memory file as a generic); the memory is Microwatt's own `fpga/main_bram.vhdl` (hex file read with textio) in place of `sim_bram.vhdl`, and `nonrandom.vhdl` replaces the libc `random` helper. No Microwatt logic is changed | Foreign C helpers are not part of what is measured |
| Microwatt is measured quiet only: the core reports a line per unit per cycle (about 2 KB per clock cycle, 89 `report` statements in 15 files), which dominated the run and hid the CPU model. `programs.py` removes the plain `report` statements (not the assertions, so the test's own end-of-test failure stays) from copies of the files under the image directory (`mw/`, source id `@image` in `tests.toml`); per-file counts are pinned in `MW_REPORTS` and a different count stops the build. No logic changes, cycle counts and console are identical to the noisy form (the end of test at 6139105 ns on both tools). The noisy form is not measured | Maintainer decision 2026-10-31 (#924) |
| Microwatt CoreMark is built from the EEMBC sources with the port in `cpu/microwatt/coremark` (barebones port, integer only, timebase) and Microwatt's own `head.S` and `lib/console.c`; the barebones `ee_printf.c` is used with its empty `uart_send_char` renamed away on the build copy | The project has no CoreMark port |
| `quiet = true`: standard output and error of the run go to files in the world (`stdout.txt`, `stderr.txt`, same for every tool), the console is read from the first and both are deleted. yavs prints reports on standard output and a pipe would carry all of it into the harness (NEORV32 warns, the quiet Microwatt form reports little). The console is the standard output without report records (`** Note:` and similar lines with their indented continuation lines), because the reference prints those on standard error. | A failed run shows the last 300 characters of both files |
| `console = true`: the console of every run cell (the untimed check run and every timed run) must be line-for-line equal to the reference's (the first reference cell, else the first yavs cell) and match every regex of `expect` (the validation line, the iteration count, the last line of the program); otherwise the cell has no time and is listed under "Cells that failed" with the first differing line | A speed number of a run that computed something else is worthless |
| NEORV32 runs at `CLOCK_FREQUENCY=100000` (a 10 us period, `clock = "10us"`): CoreMark calls a run invalid below 10 s of its own timer, which at 100 MHz is a billion cycles. The port's timer reads the cycle counter over the clock frequency; at 100 kHz a few million cycles read as more than 10 s and the program prints its validation line. The Microwatt port scales its timebase to 10 kHz for the same reason. Cycles are unaffected | A valid CoreMark result is an output check that tells a wrong computation from a right one |
| `stop` is calibrated just after the program prints its last line (the processor loops afterwards, the simulation does not end by itself), so the cycles of a test are `stop` over `clock` with no long idle tail. After changing `iterations`, recalibrate: run with a generous `stop` and shorten it until `expect` (which includes the last line) still matches, then add about 1 percent | Cycles per second needs a cycle count that is the work done |

Licences of the programs (nothing is committed from them): NEORV32 BSD-3-Clause, the NEORV32 CoreMark port and EEMBC CoreMark Apache-2.0,
Microwatt CC-BY-4.0 with IBM's additional-rights notice (its `fpga/` files BSD-3-Clause). `cpu/microwatt/coremark` is derived from the EEMBC
barebones port (Apache-2.0, header says so).

## 8. Short-window A/B (`window.py`): can this idea be killed in minutes?

All performance work has Cranelift as the base (maintainer rule, tracker #1046). The tool builds a checkout with Cranelift only, runs `--tier=auto`, and compares against sim1. There is no LLVM cell unless you pass an LLVM binary by hand.

Each test runs a fixed simulated-time window after start-up under `perf stat` (cycles, instructions, L1i, iTLB and branch misses), all cells of a test on one core of 9 to 15. A window is the counters of a run to T1 minus those of a run to T0, so start-up cancels. It needs no change to the simulator. Whole-run test checks do not apply at a window's stop: pass is an exit status of 0 or the test's own `exit_code`, and the console must equal the first variant's (against a reference: the same set of lines).

```sh
export YAVS_REALBENCH_CACHE=~/.cache/yavs-<tag>/rb YAVS_BENCH_VHDL_AI_TEST=/path/to/vhdl-ai-test
cp ~/.cache/yavs-agents/sims.local.conf tools/sims.local.conf                # untracked
python3 -I tools/realbench/window.py ../before ../after --bar cycles:0.97   # two checkouts, built with Cranelift
python3 -I tools/realbench/window.py A/yavs B/yavs:auto+YAVS_SOME_KNOB=1 --tests grlib_leon3mp   # two binaries (tools/build-bench.sh), a knob
python3 -I tools/realbench/window.py A/yavs B/yavs --rounds 6                # rounds for a cycle verdict: paired ratios with a 95 percent band
python3 -I tools/realbench/window.py A/yavs B/yavs --full --tests microwatt_coremark   # the whole run
python3 -I tools/realbench/window.py --self-test
```

With no variant it builds this checkout. A checkout's build stays in `~/.cache/yavs-window-build-<hash>`: delete it when your task ends. `--bar COUNTER:RATIO` is the largest passing ratio to the first variant, per test and on the geomean; repeat it for several counters. Ratios are to the first variant, plus `cyc/sim1` and `ins/sim1`. Counters are the simulation thread (`--no-inherit`, as in the baseline); compile threads are not in them, so a change that compiles more must also run with `--inherit` or wall. `--fe` counts front-end dispatch slots (Zen 4 `de_no_dispatch_per_slot.no_ops_from_frontend`) in place of branch misses, in the `branch` columns, headed `fe/k`. `--window T0,T1` replaces the window to calibrate. `--no-sim1` leaves out sim1. With `--rounds N` the ratios are taken per round (both variants of a round ran minutes apart, so the host's slow drift is shared) and shown as the geomean with a 95 percent band (`cyc band`, t-quantile of the rounds' spread); the geomean row is the per-round geomean over tests with its own band. A bar is `pass` when the top of the band is at or under it, `KILL` when the bottom is over it, otherwise `undecidable`; cycles, L1i, iTLB and branch misses are `undecidable` below 3 rounds, instructions decide in one. To see the noise of a binary alone, pass it twice (`A/yavs A/yavs`): that is an A/A, and its band is the smallest difference that run can resolve. `--json` keeps the per-round values of each variant (`rounds`, `pairs`) next to the medians.

Total for two variants plus sim1, all nine tests, one round: **541 s** (setup 14 s, next to another timed job on CCD1, 2026-11-01). GRLIB and Microwatt are most of it (193 and 225 s); the other seven take 108 s. Each yavs variant runs two warm-ups to T1, then T1 and T0.

**Whole-run guard, not a window:** `uvvm_axistream_vvc` (2.5 s, 27 G instructions) is the test that stopped compiling `Kind::Frame` subprograms (#1046: a Frame callee sends every calling process through the interpreter's loop for compiled code, +4 to 5 percent instructions and cycles). It has no window and is not in `WINDOWS` (that would make the `perfguard.py` reference stale and needs the validation below); every change to what `auto` compiles is checked on it with `--tests uvvm_axistream_vvc --full --inherit --rounds 9` (warm; its cold cell is `run.py --compare A B --tiers auto-cold`). Bar of #1046 steps: instructions within 0.5 percent and cycles inside their band. Its compile outcome varies from run to run (#673), so read rounds, not one run.

### Validation, 2026-11-01, e8289fd1, Cranelift cells only

Tolerances fixed before looking: instructions 3 percent, cycles 5, misses per 1000 instructions 25. Each cell: window, then whole run of the same binary. Baseline is `--tier=auto` of a95d9ad1 (whole-run instructions to sim1 within 5.4 percent of it on all nine; L1i per 1000 up to 45 percent off it on the small tests, where it moves between runs, GRLIB and Microwatt within 3). Whole-run L1i, iTLB and branch columns are auto.

| test | window T0 to T1 | instr auto/interp | cycles auto/interp | instr auto/sim1, window : rest of run | L1i/k | iTLB/k | br/k | A/A: instr, cycles |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| grlib_leon3mp | 300 to 700 us | 0.943 : 0.951 | 1.160 : 1.195 | 2.59 : 2.65 | 1.63 : 1.76 | 0.477 : 0.387 | 4.48 : 4.42 | 1.002, 0.942 |
| vai_window_gen_interlock | 64 to 512 us | 0.291 : 0.292 | 0.325 : 0.320 | 2.06 : 2.02 | 0.69 : 0.64 | 0.085 : 0.088 | 1.26 : 1.15 | 1.024, 0.983 |
| fletcher_bufferwriter_512 | 8 to 256 us | 0.923 : 0.935 | 1.041 : 1.003 | 1.94 : 1.96 | 0.96 : 1.07 | 0.253 : 0.017 | 1.27 : 1.35 | 1.000, 1.033 |
| sc5_fifo_repack | 20 to 400 us | 0.715 : 0.727 | 0.671 : 0.685 | 2.34 : 2.32 | 0.65 : 0.44 | 0.017 : 0.005 | 0.69 : 0.68 | 1.003, 1.014 |
| sc5_timestamp | 100 to 2500 us | 0.998 : 1.004 | 1.004 : 1.006 | 1.61 : 1.63 | 0.42 : 0.32 | 0.000 : 0.000 | 0.12 : 0.19 | 1.008, 1.055 |
| fletcher_bufferreader | 16 to 512 us | 0.998 : 0.996 | 0.996 : 1.024 | 1.30 : 1.33 | 1.16 : 1.70 | 0.023 : 0.006 | 1.22 : 1.32 | 0.997, 0.985 |
| hdlm_lfsr | 128 us to 4 ms | 0.379 : 0.383 | 0.389 : 0.389 | 1.50 : 1.50 | 1.62 : 1.74 | 0.000 : 0.001 | 0.18 : 0.20 | **1.122**, 1.095 |
| neorv32_single_cache | 40 to 280 us | 1.015 : 1.017 | 1.043 : 1.058 | 1.10 : 1.10 | 0.60 : 0.79 | 0.006 : 0.098 | 2.01 : 2.28 | 1.000, 1.020 |
| microwatt_coremark | 300 to 800 us | 0.959 : 0.938 | 1.109 : 1.153 | 2.17 : 2.13 | 1.23 : 1.38 | 0.380 : 0.465 | 4.78 : 5.43 | 1.000, 1.029 |

Verdict per test:

| test | instructions | cycles | front-end misses |
|---|---|---|---|
| grlib_leon3mp | holds | holds; A/A 5.8 percent, needs rounds | L1i, iTLB, branch hold |
| vai_window_gen_interlock | holds; A/A 2.4 percent | holds | L1i, branch hold; iTLB moves 0.05 to 0.2 between runs |
| fletcher_bufferwriter_512 | holds | holds | L1i, branch hold; iTLB does not |
| sc5_fifo_repack | holds | holds | branch holds; L1i and iTLB do not |
| sc5_timestamp | holds | holds; A/A 5.5 percent, needs rounds | none: counts too small |
| fletcher_bufferreader | holds | holds | branch holds; L1i is unstable (1.16 to 1.75 in windows); iTLB does not |
| hdlm_lfsr | the window stands in for the run, but one run cannot decide: `auto`'s compile outcome varies (#673), identical runs differ by 12 percent (6 in another sitting), whole runs too (22.90 and 24.36 G) | same | L1i, branch hold; iTLB has no signal |
| neorv32_single_cache | holds | holds | L1i (just), branch hold; iTLB does not |
| microwatt_coremark | holds, near the tolerance | **does not hold reliably**: 1.07, 1.09, 1.11 against 1.153, under-reporting the loss; later windows (800 to 1300 us, 1500 to 2000 us) gave 1.08, 1.06 | L1i, iTLB, branch within tolerance, all below the whole run |

Lengthened after failing: grlib_leon3mp from 100 to 300 us (instructions 0.995 against 0.951, cycles 1.30 against 1.20), vai_window_gen_interlock from 16 to 256 us (0.322 against 0.292), fletcher_bufferwriter_512 from 8 to 128 us (cycles), microwatt_coremark from 100 to 300 us (0.988 against 0.938). None dropped. Early windows read `auto` worse than its whole run; the likely reason is that tiering has not settled, not traced.

What does not hold, and how to use it:

- **Against sim1 a window gives the run rate, not the whole-run ratio.** sim1's start-up is in its whole run: `neorv32_single_cache` is 1.10 in the window and 0.54 whole, GRLIB 2.59 and 2.10. The window matches the rest of the run after T0 on instructions (within 2.3 percent on all nine). Cycles to sim1 match less well: GRLIB 4.40 against 4.96, neorv32_single_cache 1.29 against 1.17, 6 percent on two others. Publish whole-run ratios from `run.py`, never these.
- **One round of cycles cannot decide a bar near noise, and neither can a median of a few rounds (#1055).** Cycles move 20 percent and more between identical runs on this host at load 18 to 25 (GRLIB window of the same binary, instructions equal to 0.5 percent: 147 to 269 G cycles; the run to T0 alone 101 to 177 G), because other jobs share CCD1's L3 and memory. That noise is in time, not in the order: the run to T0 jumps while its instructions stay flat. Until 2026-11-01 the ratio was the median of one variant's rounds over the median of the other's, and with 3 rounds the two medians took different slices of that drift: the same binary against itself read 0.83 on GRLIB (issue #1055), 0.82 again on the first three rounds of a later six-round A/A and 1.08 on the last three. The slice depended on the sitting, so a first variant that read slower in four sittings is what that estimator gives by chance; no order effect reproduces. Measured 2026-11-01 on `0301b104` (Cranelift bench build, one core of CCD1, load 18 to 25) the first-in-round versus second-in-round ratio is 0.925 [0.76, 1.13] on GRLIB, 1.009 [0.99, 1.03] on sc5_fifo_repack and 1.032 [0.99, 1.07] on neorv32_single_cache over 8 rounds each: the bands hold 1.0, no order bias is visible. Warm-up and cache state do not explain it: round 0 is not slower than later rounds, and a run right after a sim1 run is not slower than after a yavs run (0.98 against 0.99 of its round, 4 runs each), so no per-run warm-up was added. Of 30 overlapping 3-round slices of A/A data (GRLIB, sc5_fifo_repack, neorv32_single_cache) the old estimator passed `cycles:0.97` in 8; the paired band passes none (one KILL, 29 `undecidable`).
- **What a cycle difference can be trusted at, 8 rounds, same sitting:** half-width of the band GRLIB 19.5 percent (its A/A moved 0.62 to 1.20 over 3-round slices; a 3 percent bar needs hundreds of rounds there, so decide GRLIB on instructions, misses per 1000 and, for a large change, cycles at 25 percent or more), sc5_fifo_repack 2.1, neorv32_single_cache 4.3, geomean of the three 6.5. A quiet host narrows the band; the table prints it, so read it instead of this list. Instructions between two copies of one binary differ by up to 0.5 percent on GRLIB (band 1.001 to 1.010), so a 1 percent instruction bar there is at the noise.
- **Microwatt cycles need `--full`** (about 4.5 minutes per variant, plus two warm-ups).
- **iTLB ratios mean something only on GRLIB and Microwatt.** Elsewhere the counts are tiny and identical runs differ by up to 6.5 times.
- Ratios of two cells are from one sitting on one core; the whole-run cells ran in another sitting, CCD1 not quiet. A warm-up reaches T1 only, so the cache holds what ran by then, not what a whole run would compile.

## 8a. Performance guard (`perfguard.py`, #1052): did a commit move the window instructions?

A by-hand check, nothing is scheduled and nothing is posted. It builds a commit with Cranelift only, runs the nine windows of section 8 at `--tier=auto` (`window.py` with one variant and `--no-sim1`, so sim1 is not needed), and compares the instructions of each window (simulation thread) with a reference stored in `~/.cache/yavs-perfguard/reference.json`. Instructions only: cycles need rounds (section 8).

```sh
export YAVS_BENCH_VHDL_AI_TEST=/path/to/vhdl-ai-test
python3 -I tools/realbench/perfguard.py check            # origin/main; or a commit: ... check <commit>   (the one command)
python3 -I tools/realbench/perfguard.py reference [commit]   # the only command that writes the reference
python3 -I tools/realbench/perfguard.py --self-test
```

- Exit 0: every test inside its bound. Exit 1: a test moved past its bound, in either direction (less work can be a test that quietly stopped doing it); the table and the text an issue should carry are printed. Exit 2: it could not run (tool, `perf`, source checkout, reference or build missing, a window failed); it never passes by default.
- Bounds: 3 percent of the reference per test. `hdlm_lfsr` has 15 and `vai_window_gen_interlock` 20, because the compile outcome of `auto` depends on timing (#673). Measured 2026-11-01 by checking the commit the reference was taken from, twice, on a host with load 23 to 43: those two moved by 1.075 and 1.0 (hdlm_lfsr) and 1.070 and 1.135 (vai_window_gen_interlock), the other seven by under 1.1 percent (GRLIB 0.6, Microwatt 0.7). So at 3 percent seven tests can fail only for a real reason; the two noisy ones catch only large moves, and a smaller move there is found by `window.py` with rounds on a quiet host.
- A build stays in `~/.cache/yavs-window-build-<hash>` and the checkout in `~/.cache/yavs-perfguard/tree` (detached worktree of this repository, moved to each commit): delete both when done. The libraries of the tests are cached in `~/.cache/yavs-perfguard/rb` (`YAVS_REALBENCH_CACHE` overrides).
- A reference taken with other windows than `WINDOWS` in `window.py` is refused as stale. Refresh it after a change of a window, of `tests.toml` or of a pinned source, and after an intended speed-up (a move down past the bound also fails).
- Time: about 6 minutes for the nine windows after the build (362 s and 457 s measured), one timing core of 9 to 15 held for the run.
