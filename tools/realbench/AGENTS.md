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

`YAVS_BENCH_TIERFLAGS` (extra arguments after `--tier=` of every yavs run cell, e.g. `--tier-heat=1,0,0` to compile every function at its first call) is not part of the cell name: set it for both binaries of a `--compare`.

Categories (`--category`): `rtl-datapath`, `fsm`, `numeric`, `memory`, `records`, `wide-vectors`, `bus-tristate`, `strings-textio`,
`protected`, `cpu`. Run `run.py --list` for the current selection.

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
