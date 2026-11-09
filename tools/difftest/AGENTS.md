# AGENTS.md: tools/difftest/ (differential simulation harness)

Read the root `AGENTS.md` and `tools/AGENTS.md` first. This file is for an agent about to change
the harness or run it. `tools/difftest/README.md` is the full contract (test header, canonical
form, ignore set, templates, placeholders); this file only summarises it. The test files
themselves are described in `tests/corpus/AGENTS.md`.

## 1. Purpose and boundaries

- Runs every `tests/corpus/**/*.vhd` testbench on a candidate simulator (`sim1`, `sim2`, `yavs` or
  any bash template), normalises its output to the canonical event form and compares it with the
  checked-in reference-simulator `<name>.expected`. CI needs no reference simulator. The M2 check: every group equal to
  the reference simulator at 1 thread, 4 threads and 4 threads shuffled (#65).
- With `{threads}` in the template, `--threads 4` also requires the 1- and 4-thread outputs to be
  identical (root rule 3).
- Not here: the analysis corpora (`tools/corpus`), the in-tree Rust corpus test
  (`crates/yavs-lower/tests/corpus.rs`, which runs the same files without the CLI).
- Python 3 stdlib only.

## 2. Map

| File | What |
|---|---|
| `difftest.py` | Harness: templates `sim1`, `sim2`, `YAVS`, `YAVS_AUTO` (`--tier=auto`, compared with the same `.expected`: rule 5), `YAVS_CLIFT`, `YAVS_CLIFT_AUTO` (`--tier=clift`, `clift-auto`, likewise; the LLVM templates `yavs-llvm`, `yavs-auto3`, `yavs-auto3-eager`, `yavs-mixed` went with the LLVM tier, 2026-11-08), `YAVS_CLIFT_CHECK` (`yavs-clift-check`: every process with one suspension point gets a copy from its third activation, every skip of a drive in generated code, and of a record drive against its image from the first scatter on, is held against the kernel's full test), `YAVS_CLIFT_COUNT1` (`yavs-clift-count1`: the count rule of `--tier=auto` at a count of 1 with the run waiting for what it asks, so hold, gate, copies and native-frame code run on every test), `YAVS_CLIFT_CHEAP`, `YAVS_CLIFT_CHEAPALT` (`yavs-clift-cheap`, `yavs-clift-cheapalt`: `--tier=clift` with every block, or every second block, lowered as code that never ran, `YAVS_CLIFT_CHEAP=all|alt`), `YAVS_CLIFT_LISTALL` (`yavs-clift-listall`: `yavs-clift-count1` with every block of every compiled body on the never-run list, `YAVS_CLIFT_LIST=all`, and the trip at one stepped instruction per IR instruction, so functions are made again in full during the run), `YAVS_CLIFT_LISTREC` (`yavs-clift-listrec`: a first pass at the default policy in its own directory writes the never-run list from the blocks the interpreter entered, `YAVS_CLIFT_LIST=rec`, keeping only its counters; the second compiles every function at its first call with the list and the trip at one stepped instruction per IR instruction), `YAVS_INTERP` (`--tier=interp`; plain `yavs` uses the default tier of the build), `normalise()`, `yavs_fatal()` (yavs runtime diagnostics to `fatal` events), ignore masks, `--bless`, `--json`, `--threads`, `--shuffle`, `--filter`, `--exclude` (the complement; gc-stress builds skip `composite/aggregate_fill`, #852), `--jobs` (max 8), `--list`, `--no-sandbox` |
| `external.py` | The reference simulators' own self-checking regression tests, run in place from the corpus cache on yavs and the reference simulator, each mismatch classified (README "External regression tests"); `--oracle --check` against `external-*.pass` without a reference simulator (CI) |
| `external-sim1.pass`, `external-sim2.pass` | Tests that pass both ways; CI gates them |
| `reach.py --counters A,B [--candidate T ...] [--filter RE]` | The reach check of a perf or optimiser change: the corpus on the given templates with `YAVS_COUNTERS=1`, the named counters summed; exit 1 when one is zero over the whole set, so a new path that no test runs is seen before it lands. Output is compared by `difftest.py`, not here |
| `report.sh` | Build `yavs`, run at 1, 4 and 4 shuffled threads, write `tests/corpus/RESULTS.md` (scratch `~/.cache/rsim-difftest2.<pid>`, removed afterwards) |
| `report.py` | `--json` results to `RESULTS.md`, failures grouped with an owner guess per code/message |
| `README.md` | The contract |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| The reference simulator is the reference; `ref=hand` marks tests where the reference simulator is wrong (hand-edited `.expected` to the LRM result) | LRM policy: the LRM wins where clear | README "Test header", `docs/plan.md` "Verification" |
| yavs is compared strictly (no ignored fields by default); a test may opt into `ignore=order` with a comment | Same-cycle report order is unspecified by the LRM | README "Ignore set", `docs/decisions.md` |
| Header `relaxed`: each candidate's relaxing switch (`{relaxed}`), yavs also `-Wno-W1904` (and `-Wno-W1302` for 1993 homograph pairs); analysis output only when analysis fails | Tests of constructs only `--relaxed` accepts, compared without analysis warnings | README "Test header", #285 |
| Exit codes 97 (analysis) and 98 (elaboration) are reserved for the harness | Compile failures reported as such | README "Plugging yavs in" |
| yavs reports in the reference simulator's format on stdout; runtime errors are diagnostics on stderr (`error[E3xxx]`, `= note: at T, delta D`) | One canonical form | README; `docs/cli-contract.md` 5 |
| yavs writes a process path dotted, the reference simulators with colons: `normalise()` turns a yavs path into the reference form (`lrm_path`), on the path field only, so everything else compares as printed | Dotted output is the maintainer's choice; no comparison is weakened | #975, `docs/decisions.md` "Dotted paths" |
| Every run `timeout 300 systemd-run --user --scope -p MemoryMax=2G`, at most 8 at once; work dirs under `~/.cache/yavs-difftest/run-<pid>` (unique per process; `DIFFTEST_CACHE` overrides); a run removes its passing cases and itself on exit, keeps failing cases and prints the path | Root rule 15; the shared directory grew to 47 GB | README, `test_cleanup.py` |
| `--bless` refuses an `.expected` whose exit status differs from the header's `exit=` | A broken test cannot be blessed by accident | README "Test header" |
| `external.py` compares per run with the classes `pass` < `order` < `fatalmsg` < `delta-path` < `output`, `outcome` when the pass/fail verdict differs; the pass lists hold the tests that pass the suite's own verdict (`--oracle --write-pass`; some differ from the reference output in the comparison), and CI checks only that | CI has no reference simulator; the classes rank causes | #354 |

Decisions about the harness go in this section and README in the same commit (root `AGENTS.md`
rule 24).

## 4. Invariants and pitfalls

- `report.sh` uses `|| true` on the runs on purpose (failures become the report); a check of yours
  must never do that (root rule 13).
- `YAVS_BIN` defaults to `yavs` on `PATH`; `report.sh` uses `${CARGO_TARGET_DIR:-target}/debug/yavs`.
  With a shared `CARGO_TARGET_DIR` that can be another worktree's binary (#7).
- Changing `normalise()` changes every comparison: run the reference simulator candidate against its own
  `.expected` (must stay green) and the yavs candidate before and after.
- `external.py` reads the reference suites in place: never copy their files into the repository
  (root rule 7); its pass lists hold test names only.
- Process order inside one cycle differs from the reference simulator in places (#11); default assert message path
  (#12).

## 5. How to work here

```sh
python3 tools/difftest/difftest.py                          # reference simulator against its own .expected (needs it)
python3 tools/difftest/difftest.py --candidate yavs --threads 4 --filter 'report/|env/' -v
YAVS_BIN=$PWD/target/debug/yavs python3 tools/difftest/difftest.py --candidate yavs
YAVS_BIN=<yavs built with --features clift> python3 tools/difftest/difftest.py --candidate yavs-clift
python3 tools/difftest/difftest.py --bless --filter 'mygroup/'   # new references from the reference simulator
tools/difftest/report.sh                                    # tests/corpus/RESULTS.md
```

## 6. Coordination

- Open issues (snapshot 2026-10-18): #65 reach the M2 check; #107 `--threads N` determinism across
  the difftest; #11 report order within a cycle; #12 default assert message path.
