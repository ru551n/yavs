# AGENTS.md: tools/difftest/ (differential simulation harness)

Read the root `AGENTS.md` and `tools/AGENTS.md` first. This file is for an agent about to change
the harness or run it. `tools/difftest/README.md` is the full contract (test header, canonical
form, ignore set, templates, placeholders); this file only summarises it. The test files
themselves are described in `tests/corpus/AGENTS.md`.

## 1. Purpose and boundaries

- Runs every `tests/corpus/**/*.vhd` testbench on a candidate simulator (`nvc`, `ghdl`, `yavs` or
  any bash template), normalises its output to the canonical event form and compares it with the
  checked-in NVC reference `<name>.expected`. CI needs no NVC. The M2 check: every group equal to
  NVC at 1 thread, 4 threads and 4 threads shuffled (#65).
- With `{threads}` in the template, `--threads 4` also requires the 1- and 4-thread outputs to be
  identical (root rule 3).
- Not here: the analysis corpora (`tools/corpus`), the in-tree Rust corpus test
  (`crates/yavs-lower/tests/corpus.rs`, which runs the same files without the CLI).
- Python 3 stdlib only.

## 2. Map

| File | What |
|---|---|
| `difftest.py` | Harness: templates `NVC`, `GHDL`, `YAVS`, `normalise()`, `yavs_fatal()` (yavs runtime diagnostics to `fatal` events), ignore masks, `--bless`, `--json`, `--threads`, `--shuffle`, `--filter`, `--jobs` (max 8), `--list`, `--no-sandbox` |
| `report.sh` | Build `yavs`, run at 1, 4 and 4 shuffled threads, write `tests/corpus/RESULTS.md` (scratch `~/.cache/rsim-difftest2`, removed afterwards) |
| `report.py` | `--json` results to `RESULTS.md`, failures grouped with an owner guess per code/message |
| `README.md` | The contract |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| NVC is the reference; `ref=hand` marks tests where NVC is wrong (hand-edited `.expected` to the LRM result) | LRM policy: the LRM wins where clear | README "Test header", `docs/plan.md` "Verification" |
| yavs is compared strictly (no ignored fields by default); a test may opt into `ignore=order` with a comment | Same-cycle report order is unspecified by the LRM | README "Ignore set", `docs/decisions.md` |
| Exit codes 97 (analysis) and 98 (elaboration) are reserved for the harness | Compile failures reported as such | README "Plugging yavs in" |
| yavs reports in NVC's format on stdout; runtime errors are rustc-style diagnostics on stderr (`error[E3xxx]`, `= note: at T, delta D`) | One canonical form | README; `docs/cli-contract.md` 5 |
| Every run `timeout 300 systemd-run --user --scope -p MemoryMax=2G`, at most 8 at once; work dirs under `~/.cache/yavs-difftest` (`DIFFTEST_CACHE`) | Root rule 15 | README |
| `--bless` refuses an `.expected` whose exit status differs from the header's `exit=` | A broken test cannot be blessed by accident | README "Test header" |

Decisions about the harness go in this section and README in the same commit (root `AGENTS.md`
rule 24).

## 4. Invariants and pitfalls

- `report.sh` uses `|| true` on the runs on purpose (failures become the report); a check of yours
  must never do that (root rule 13).
- `YAVS_BIN` defaults to `yavs` on `PATH`; `report.sh` uses `${CARGO_TARGET_DIR:-target}/debug/yavs`.
  With a shared `CARGO_TARGET_DIR` that can be another worktree's binary (#7).
- Changing `normalise()` changes every comparison: run the NVC candidate against its own
  `.expected` (must stay green) and the yavs candidate before and after.
- Process order inside one cycle differs from NVC in places (#11); default assert message path
  (#12).

## 5. How to work here

```sh
python3 tools/difftest/difftest.py                          # NVC against its own .expected (needs NVC)
python3 tools/difftest/difftest.py --candidate yavs --threads 4 --filter 'report/|env/' -v
YAVS_BIN=$PWD/target/debug/yavs python3 tools/difftest/difftest.py --candidate yavs
python3 tools/difftest/difftest.py --bless --filter 'mygroup/'   # new references from NVC
tools/difftest/report.sh                                    # tests/corpus/RESULTS.md
```

## 6. Coordination

- Open issues (snapshot 2026-10-18): #65 reach the M2 check; #107 `--threads N` determinism across
  the difftest; #11 report order within a cycle; #12 default assert message path.
