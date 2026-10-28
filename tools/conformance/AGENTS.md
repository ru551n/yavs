# AGENTS.md: tools/conformance/ (conformance report)

Read the root `AGENTS.md` and `tools/AGENTS.md` first. This file is for an agent about to change the
report generator or regenerate the report (#780).

## 1. Purpose and boundaries

- `run.py` runs the evidence suites that exist elsewhere in `tools/` (difftest, discrepancies, the
  reference simulators' own tests, analysis corpora, UVVM, OSVVM and VESTs suites, real designs, and with
  `--local` the reference-simulator baselines and the VHPI trace diff) and renders `docs/conformance.md`
  and `docs/conformance.json`. It is a thin driver: it parses the output of those tools and builds no
  test of its own.
- Not here: waveform equivalence on real designs (agent trust2: the section includes
  `tools/waveq/RESULTS.md` when that file exists, a placeholder before); the owner of `tools/waveq` refreshes the report with `run.py --waveq` after updating `RESULTS.md` and `results.json` (no CI steps needed), performance numbers (`tools/bench`,
  `tools/realbench`).
- Python stdlib only (`python3 -I`).

## 2. Map

| Path | What |
|---|---|
| `run.py` | Steps (`step_*` methods of `Run`), static facts (discrepancy catalogue, recorded sim1 corpus lists, CI platforms from `ci.yml`, known limits), `render()`, `--check` |
| `../../docs/conformance-local.json` | Records of runs that CI cannot repeat: `--local` rewrites `refdiff` and `vhpi`; `vunit-own` and `cocotb` are hand-kept copies of `docs/STATUS.md` numbers (date, commit, command, note); `openlogic` is hand-kept too (Open Logic's own `sim/run.py` run with yavs, 6867 tests: re-run it and edit the record by hand) |
| `../../.github/workflows/conformance.yml` | CI: build, `run.py --check`, upload the report artifact |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| CI regenerates the steps that need only yavs and the fetched corpora; everything else is a dated record (`date`, `commit`, `command`) in `conformance-local.json`, never silently carried over | Honest provenance: a number says where and when it was produced | #780 |
| The reference simulators appear as sim1/sim2 in the report; no product names | Repo rule 11 | root `AGENTS.md` |
| The committed report is compared with the regenerated one ignoring the `Generated:` line. On `main` a difference is a warning, on a release tag it fails the run | Ten agents push to `main`: failing every push for a changed count would block them; a release must carry a current report | #780 |
| The report is user-facing: it opens with a short plain-language summary and one table (suite, what it is, passed / total), terms explained once; everything else (per-suite detail, where and when each result was produced, gated lists, discrepancies, commands) follows under "Details", "How to reproduce" and "Known limits". No internal jargon (no agent names, tool option names or rule numbers); CI rows name no commit (only the `Generated:` line does, so `--check` can compare) | Maintainer instruction | #780, 2026-10-28 |
| Records of external tools that are not user-facing (`INTERNAL_RECORDS` in `run.py`: cocotb) stay in `conformance-local.json` and are left out of `conformance.md` | Maintainer: cocotb is an external tool and is not named in user documents | user, 2026-10-28 |
| Nothing is published outside the repository (no Pages) | Needs the maintainer's decision | #780 |
| Corpus results come from `tools/corpus/run.py` against the gated `.expected` lists; sim1's numbers are the recorded `.sim1.expected` lists, not a rerun | Same files, no reference needed in CI | #780 |

## 4. Invariants and pitfalls

- The site shows only the summary: `docs/conf.py` cuts `conformance.md` at the `## Details` heading and fails the docs build without it. Keep that heading.
- Exit statuses of the underlying tools are results, not errors (a failing suite is a number in the report), except `realbench`, whose failure stops the run.
- The report text must be deterministic for a given commit: no timestamps outside the `Generated:` line, no timings, sorted keys.
- A `--local` run needs a clean tree to record a meaningful commit (`-dirty` is appended otherwise); commit `docs/conformance-local.json` with the regenerated report.
- Every tool it calls keeps its scratch under `--out`, not the user's shared `~/.cache/yavs/std`.

## 5. How to work here

The CI parts need no local run: download the `conformance-report` artifact of a green `conformance` run and run `run.py --report-only --out <dir>` on it to refresh the committed report (the local records stay).

```sh
tools/fetch-corpus.sh
cargo build --release -p yavs        # --features llvm for the auto and llvm tiers
tools/conformance/run.py --yavs target/release/yavs --out ~/.cache/yavs-conformance [--local --llvm]
tools/conformance/run.py --check --yavs target/release/yavs --out ~/.cache/yavs-conformance
```
