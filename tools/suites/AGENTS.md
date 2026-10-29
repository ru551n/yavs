# AGENTS.md: tools/suites/ (external library testbench suites)

Read the root `AGENTS.md` and `tools/AGENTS.md` first. This file is for an agent about to run or
change a driver that simulates an external library's own testbenches on yavs.

## 1. Purpose and boundaries

- Measurement drivers: compile an external library from the corpus cache and run its own
  testbenches with `yavs run`, one pass/fail line per testbench and generic set. They report; they
  never fix simulator bugs (each failure root cause is a GitHub issue with a minimal reproducer).
- Sources come from `~/.cache/rsim-corpus` (`tools/fetch-corpus.sh`), never copied into the
  repository. Output goes under `~/.cache/<your-dir>` (`--out`).
- Not here: the analysis corpora (`tools/corpus`) and the differential harness (`tools/difftest`).

## 2. Map

| File | What |
|---|---|
| `pro.py [--std 2008\|2019] [--out DIR] [--yavs BIN] [--jobs N] [--filter SUBSTR] [--list] [--analyze-only] [--no-wrap]` | The `osvvm-libraries` corpus (#276): the library's `.pro` build and test scripts read with a Tcl subset (an unknown command, variable or condition is an error naming file and line); `CoSim` and `CoSimPCIe` are skipped (native host). All files are analyzed first in script order (a private library directory per test suite; consecutive files of one library with the same mapping in one `yavs analyze` call, repeated file by file when a call fails), then the tests run (4 at most). Pass = exit 0 and a `DONE  PASSED` line. Writes `DIR/results.tsv`, `DIR/run/<n>/log.txt`, `DIR/analyze.log`; the generated test-case package goes to `DIR/gen` |
| `vests.py [--root DIR] [--out DIR] [--yavs BIN] [-j N] [--only GROUP,...] [--no-wrap]` | The `vests` corpus: the VESTs VHDL-1993 suite (billowitch, ashenden, clifton-labs) at `--std=93`, by the suite's own `.exp` lists and expectations (exact verdict rules in its docstring). `OVERRIDES` holds each test whose expectation is wrong (PASS) or that cannot be judged (SKIP) with its reason, `ISSUES` the yavs issue of each failing test. Writes `DIR/results.tsv` (group, test, PASS/FAIL/SKIP, detail, issue) and `DIR/<group>/<n>/log.txt`; about 40 s at `-j 8` |
| `uvvm.py [--out DIR] [--yavs BIN] [-j N] [--mem M] [--only COMP,...] [--list] [--private-copies] [--no-wrap]` | The `uvvm` corpus: every component's maintenance testbenches. Consecutive files of one library go in one `yavs analyze` call (repeated file by file when a call fails). Prints pass / fail / skip per component, writes `DIR/results.tsv` and `DIR/<component>/<n>/log.txt` |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| The project's own regression script of each component (`script/maintenance_script/test.py`) is executed against a recording stand-in for its Python framework; only its `add_files` and `add_generics` calls are used | Gets the exact libraries, files, testbench entities and generic sets (including loops over configurations) without reimplementing the scripts | #277 |
| Library sources compile in the order of `script/component_list.txt` and each component's `script/compile_order.txt` (reusing the `tools/corpus` conventions), blocks reordered so a block follows the libraries it names; testbench files by the dependency scan of `tools/corpus/gen_manifests.py` | The listed order is not always a dependency order | #277 |
| A testbench entity with several architectures runs once per architecture; an entity with a `GC_TESTCASE` generic and no default runs once per `GC_TESTCASE = "name"` it tests | What the project's tool does | #277 |
| Pass = exit 0, a `>> Simulation SUCCESS` line and no `>> Simulation FAILED` line in the output | The library's own final summary | #277 |
| `--private-copies` analyzes every library source from a per-library copy of the file | The same source file compiled into several libraries was a yavs defect (#290, fixed); the option is no longer needed | #277, #290 |
| Third-party design files a testbench needs are fetched at pinned commits (`uvvm-supplementary`, `uvvm-i2c-master`, `uvvm-i2c-slave`, `THIRD_PARTY` in `uvvm.py` maps the testbench paths to them). A testbench whose files are still absent (vendor simulation libraries) is skipped with the missing directories named, and so is a run that fails with a design component left unbound | Not a yavs result | #277 |
| Every run passes `--exit-severity=failure`: the library's verdict is its final summary, and a design under test may report errors by design (a sim1 run exits 1 on an `error` report too). `func_cov_tb` runs share one directory per entity and run in the listed order (a test reads the files an earlier one wrote). `questa_extension_tb` runs with `GC_EXTENSIONS_ENABLED=false` and passes on exit 0 (it prints no summary; sim1 behaves the same, with the default it stops with status 1) | The project's own tool runs them that way | #277 |
| `pro.py` generates the body of `OsvvmScriptSettingsPkg` from the default one with the scripts' release (`OsvvmVersion`) as `OSVVM_REVISION` and `OSVVM_SETTINGS_REVISION`, as the scripts' own generator does | The settings revision selects the transcript layout the tests compare against the validated results (58 false failures with the default body) | #276 |
| `vests.py` judges each test by the suite's own list and expectation. A test is overridden only with a recorded reason: PASS where the language reference makes the expectation wrong and yavs's result is right; SKIP (not counted) for files that are not legal VHDL-1993, files every tool (yavs, sim1, sim2) rejects under the suite's own build list, and results the language reference leaves to the implementation. An `analyzer_failure` test in the hand-checked `LATE` list also passes when yavs stops it at elaboration or run time with the error the test is about, and the report shows that count apart. In every must-fail group a first error E1999, E2001, E9999 or a crash is no rejection | The suite has wrong expectations and broken build lists; the language reference leaves the detection time of value errors open (sim2 detects them at run time too); a not-implemented message or a crash says nothing about the test | #780, 2026-10-28 |
| Each yavs call is `timeout 300 systemd-run --user --scope -p MemoryMax=...` (2G for analysis, `--mem` for runs, default 4G); `-j` calls at once (default 4) | Root rules 15 and 16: a testbench over 5 minutes is reported as skipped, never shortened | root `AGENTS.md` |

## 4. Invariants and pitfalls

- The recorder executes corpus scripts (in a scratch directory, stdout dropped); a script that
  starts anything else prints its error on stderr, harmlessly.
- Generated-VIP tests of the VVC framework script (the generator output compiled and simulated)
  are not run: only the part up to its first `start()` is recorded.
- Run only with the host's simulation budget in mind: at most 4 yavs calls at once.

## 5. How to work here

```sh
tools/fetch-corpus.sh
cargo build --release -p yavs
python3 -I tools/suites/uvvm.py --list
python3 -I tools/suites/uvvm.py --yavs target/release/yavs --out ~/.cache/yavs-uvvm-suite --private-copies
```

- No diagnostic codes.

## 6. Coordination

- #277 (`uvvm.py`), #276 (`pro.py`), #780 (`vests.py`; its failures: #807 to #824), #115 (parent). Root causes found: #285 to #290, #291 to #304, fixed #391 to #393, open: #460 (see `docs/STATUS.md`).
