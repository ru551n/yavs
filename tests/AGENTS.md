# AGENTS.md: tests/ (shared test inputs)

Read the root `AGENTS.md` first. This directory holds test **inputs and expected outputs** shared
by several crates; the test code lives in the crates. Each subdirectory has its own guide.

| Path | What | Run by | Guide |
|---|---|---|---|
| `diag/*.vhd`, `*.stderr` | Parse-diagnostic snapshots | `crates/yavs/tests/diag.rs` | `tests/diag/AGENTS.md` |
| `diag/sem/<area>/*.vhd` | Negative semantic suite with `-- expect:` headers | `crates/yavs-sem/tests/diag_sem.rs` | `tests/diag/AGENTS.md` |
| `cli/` | `yavs analyze/elab/run` sources and expected stderr (`$TMP` placeholder) | `crates/yavs/tests/cli.rs` (cases listed in the test) | `crates/yavs/AGENTS.md` |
| `corpus/<group>/*.vhd`, `*.expected` | Differential testbenches with reference simulator outputs, `RESULTS.md` | `tools/difftest`, `crates/yavs-lower/tests/corpus.rs` | `tests/corpus/AGENTS.md` |
| `discrepancies/<id>/` | One reproducer per simulator discrepancy, with expected yavs output and recorded verdicts | `tools/discrepancies/run.py` | `tests/discrepancies/AGENTS.md` |
| `parse-corpus/*.txt` | Expected parse results for external corpora | `crates/yavs/tests/corpus.rs` (`--features corpus`) | `tests/parse-corpus/AGENTS.md` |

## Rules that bind every test input here

- Every behaviour gets a test; every error a negative test with its code and position (root
  rule 14). Expected outputs change only by blessing (`YAVS_BLESS=1`, `--bless`,
  `YAVS_SEM_XFAIL_WRITE=1`) followed by reading the diff; explain list changes in the commit.
- Written fresh: never copy the reference simulators or rust_hdl tests (root rule 6). External corpora are never
  committed (rule 7); only result lists are.
- No testbench longer than 5 minutes (rule 16); reference runs per rule 15.
- Test scratch files go to `tempfile` directories that remove themselves when dropped (also on a panic; `crates/yavs/tests/common` has `tempdir`), never fixed or pid-named directories under `/tmp`
  (see #160 for the one offender).
- Before pushing, `git diff --stat origin/main..HEAD`: a blessed snapshot you did not mean to
  change is a regression or a bad rebase (root rule 17).

Decisions about test conventions go in the matching subdirectory guide in the same commit (root
`AGENTS.md` rule 24).
