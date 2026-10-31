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
- Results with a loaded host are noise: check the load average printed at the start and use the instruction ratio when wall differs.
- `data` files of a test (the GRLIB template reads `prom.srec` and `ram.srec` from its working directory) are copied into the world; the run starts in the world directory.
- `grlib_leon3mp` runs at `std = "1993"` with the `grlib-1993` manifest (GRLIB is VHDL-93 code, decision #629; see `tools/corpus/AGENTS.md` and `docs/STATUS.md` "GRLIB"). Its `stop` is just before the template's own end-of-test failure assertion, so the run ends with status 0; the pass check is the text `Test passed` of the on-chip system test, searched in the whole output (`ok` is matched by `bench/run.py timed`, not in its 200 character tail).
- Excluded testbenches (they fail on yavs or the reference) are listed with the reason in `docs/STATUS.md`
  ("Real-world performance suite, baseline 2026-10-21") and each yavs failure has an issue.
- Do not read the `codegen` ratio as a code generation ratio against the reference: it divides by the reference's whole elaboration (which contains its code generation). Compare yavs `elab-X` + `codegen-X` with the reference `elab-X` for that.
- Scratch worlds are removed after each test; a killed run may leave `~/.cache/yavs-realbench/scratch/*`.

## 5. How to work here

```sh
tools/build-bench.sh --features llvm      # release + 64-byte aligned functions (#576), never a plain release build
cp ~/.cache/yavs-agents/sims.local.conf tools/sims.local.conf       # untracked
python3 -I tools/realbench/run.py --list
python3 -I tools/realbench/run.py --self-test
# the phases of one test, probe (one run per cell): analyze, elab, codegen, run
python3 -I tools/realbench/run.py target/release/yavs --ref sim1 --cores 6 --runs 1 --phases analyze,elab,codegen --tests hdlm_fifo_sync
# baseline: all tests, interp and auto (cold and warm), two performance cores, against the reference
python3 -I tools/realbench/run.py target/release/yavs --ref sim1 --cores 6,7 --tiers interp,auto,auto-cold --label baseline
# a probe: one untimed run per cell, pass or fail and the tail of the output
python3 -I tools/realbench/run.py target/release/yavs --ref sim1 --cores 6 --runs 0 --tests hdlm_fifo_sync
# regression check of a tuning change: two binaries interleaved on the same worlds
python3 -I tools/realbench/run.py --compare /path/to/before target/release/yavs --cores 6,7 --tiers interp,auto
python3 -I tools/realbench/run.py --report ~/.cache/yavs-realbench/results/<file>.json     # the markdown again
```

Results (raw samples reduced to medians, plus the markdown) go to `~/.cache/yavs-realbench/results/<time>-<label>.json` and `.md`.

## 6. Coordination

- The baselines and the worst tests are in `docs/STATUS.md` ("Real-world performance suite, baseline 2026-10-21" for run, "Real-world suite: analysis, elaboration and code generation vs sim1, 2026-10-22" for the other phases). Slow spots are `perf` issues.

## 7. CPU workloads: CoreMark on NEORV32 and Microwatt (category `cpubench`, #924)

Two processors running a fixed program, to measure the simulator on long clocked runs of real CPUs. Primary number: simulated clock cycles per
wall-clock second (`stop` over `clock`, divided by the wall time of the whole run cell, elaboration and compilation included); next to it the
wall time (cold and warm cache), the elaboration alone (`elab-cold`, `elab-warm`), CPU time, peak RSS and the host load of the timed runs.
The rate against the reference (cycles per second ratio, above 1 is faster) is the last column. Fair comparison (root rule 23): only the compiled
tier `auto` (cold and warm) is compared with the reference; `interp` has its own before/after.

One command for a clean measurement (needs 4 quiet performance cores, `9,10,11,12` (rule 19: timed runs on 9-15, SMT siblings idle): the 4-thread cell runs on all four, the other cells on the first;
tests run one at a time; check `uptime` first and re-run if the load is above the cores in use):

```sh
tools/fetch-corpus.sh                                   # neorv32 (with sw/), neorv32-coremark, eembc-coremark, microwatt
tools/build-bench.sh --features llvm
export YAVS_RISCV_PREFIX=/path/to/bin/riscv-none-elf-   # or a RISC-V GCC on PATH
export YAVS_PPC_ZIG=/path/to/zig                         # or YAVS_PPC_PREFIX=powerpc64le-linux-gnu-, or that GCC on PATH
python3 -I tools/realbench/run.py target/release/yavs --ref sim1 --cores 9,10,11,12 --cpubench
```

Duration: sim1 needs about 2 minutes per test (NEORV32 3.6 M cycles at about 25 k cycles/s, Microwatt 0.6 M cycles at about 6 k cycles/s on a quiet core); yavs cells run per test one untimed check (two for warm cells), 3 timed runs and one counted run, for 5 cells, so expect 3 to 5 hours for both tests. `interp` is the slowest cell; a run that exceeds `--timeout` is reported as failed (rc 124) and the numbers of that cell are lost, so raise it rather than trusting a partial result.

`--cpubench` is shorthand for `--category cpubench --phases elab,run --tiers interp,auto,auto-cold,auto@4 --runs 3 --count-runs --timeout 900
--label cpubench`; any of them can be given to override. A tier `auto@4` is `--tier=auto --threads=4` on the first 4 `--cores`. A test whose
cross compiler is missing is left out with the reason (printed and in the report); naming it with `--tests` stops instead. `--prepare` builds the
worlds and prints the commands, `programs.py` alone builds an image.

| Decision | Why |
|---|---|
| Programs are built from pinned sources fetched into the corpus cache (`tools/fetch-corpus.sh`), never vendored; the image is stored per pins, parameters and compiler under `<cache>/images/<program>-<key>` and reused | Root rule 7; a build is a few seconds but the toolchains are not always present |
| No toolchain is installed by the harness: `programs.py` finds one through `YAVS_RISCV_PREFIX` / `YAVS_PPC_PREFIX` / `YAVS_PPC_ZIG` or PATH and otherwise stops naming what is needed | Root rule 9 |
| NEORV32: the CoreMark port (separate repository) is built against the pinned NEORV32 `sw/` with `-DUART0_SIM_MODE` (console text goes to standard output in simulation, not through the baud-timed UART) and `-DITERATIONS=N`; the generated `neorv32_imem_image.vhd` is analysed on top of the corpus library together with the units that depend on it (`neorv32_imem`, `neorv32_top`, the testbench). Edits made on the build copy: the port's unconditional `ITERATIONS` define and the makefile's flag line | The imem image is a VHDL package; the sources of the processor are not touched |
| Microwatt: its simulation uses C helpers (`sim_console`, `sim_bram_helpers`, `sim_jtag_socket`, `glibc_random_helpers`) written for the foreign interface of sim2, which passes strings as bounds-carrying fat pointers. Tried through the foreign interface of sim1 and yavs (`--load`): `from_string` dereferences that fat pointer and crashes (sim1: SIGSEGV at elaboration of the memory), yavs follows the other convention (docs/vhpi.md: an unconstrained array is a pointer followed by its length), so the helpers cannot be used unchanged. `cpu/microwatt/` replaces `sim_console.vhdl` (lines of standard output, no input) and `sim_jtag_socket.vhdl` (never a message) and adds `microwatt_bench_tb.vhdl` (the SoC of `core_tb` with the memory file as a generic); the memory is Microwatt's own `fpga/main_bram.vhdl` (hex file read with textio) in place of `sim_bram.vhdl`, and `nonrandom.vhdl` replaces the libc `random` helper. No Microwatt logic is changed | Foreign C helpers are not part of what is measured |
| Microwatt CoreMark is built from the EEMBC sources with the port in `cpu/microwatt/coremark` (barebones port, integer only, timebase) and Microwatt's own `head.S` and `lib/console.c`; the barebones `ee_printf.c` is used with its empty `uart_send_char` renamed away on the build copy | The project has no CoreMark port |
| `quiet = true`: standard output and error of the run go to files in the world (`stdout.txt`, `stderr.txt`, same for every tool), the console is read from the first and both are deleted. Microwatt reports a line per unit per cycle (about 2 KB per clock cycle), and yavs prints reports on standard output; through a pipe that would be gigabytes in the harness. The console is the standard output without report records (`** Note:` and similar lines with their indented continuation lines), because the reference prints those on standard error. The simulators still do the formatting and writing, so it stays part of the measurement | A failed run shows the last 300 characters of both files |
| `console = true`: the console of every run cell (the untimed check run and every timed run) must be line-for-line equal to the reference's (the first reference cell, else the first yavs cell) and match every regex of `expect` (the validation line, the iteration count, the last line of the program); otherwise the cell has no time and is listed under "Cells that failed" with the first differing line | A speed number of a run that computed something else is worthless |
| NEORV32 runs at `CLOCK_FREQUENCY=100000` (a 10 us period, `clock = "10us"`): CoreMark calls a run invalid below 10 s of its own timer, which at 100 MHz is a billion cycles. The port's timer reads the cycle counter over the clock frequency; at 100 kHz a few million cycles read as more than 10 s and the program prints its validation line. The Microwatt port scales its timebase to 10 kHz for the same reason. Cycles are unaffected | A valid CoreMark result is an output check that tells a wrong computation from a right one |
| `stop` is calibrated just after the program prints its last line (the processor loops afterwards, the simulation does not end by itself), so the cycles of a test are `stop` over `clock` with no long idle tail. After changing `iterations`, recalibrate: run with a generous `stop` and shorten it until `expect` (which includes the last line) still matches, then add about 1 percent | Cycles per second needs a cycle count that is the work done |

Licences of the programs (nothing is committed from them): NEORV32 BSD-3-Clause, the NEORV32 CoreMark port and EEMBC CoreMark Apache-2.0,
Microwatt CC-BY-4.0 with IBM's additional-rights notice (its `fpga/` files BSD-3-Clause). `cpu/microwatt/coremark` is derived from the EEMBC
barebones port (Apache-2.0, header says so).
