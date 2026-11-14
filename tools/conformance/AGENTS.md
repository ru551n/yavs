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
- Python stdlib only (`python3 -I`), except that the `osvvm` step runs `suites/pro.py`, which needs pyEDAA.OSVVM in that Python (`python3 -m pip install -r tools/suites/requirements.txt`; `conformance.yml` installs it).

## 2. Map

| Path | What |
|---|---|
| `run.py` | Steps (`step_*` methods of `Run`), static facts (discrepancy catalogue, recorded sim1 corpus lists, CI platforms from `ci.yml` and `tools/ci-plan.py`, known limits), `render()`, `--check` |
| `../../docs/conformance-local.json` | Records of runs that CI cannot repeat: `--local` rewrites `refdiff` and `vhpi`; `vunit-own` and `cocotb` are hand-kept copies of `docs/STATUS.md` numbers (date, commit, command, note); `openlogic` is hand-kept too (Open Logic's own `sim/run.py` run with yavs, 6867 tests: re-run it and edit the record by hand) |
| `../../.github/workflows/conformance.yml` | CI, nightly (not per push): build, `run.py --check`, upload the report artifact; a red nightly opens an issue |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| CI regenerates the steps that need only yavs and the fetched corpora; everything else is a dated record (`date`, `commit`, `command`) in `conformance-local.json`, never silently carried over | Honest provenance: a number says where and when it was produced | #780 |
| The reference simulators appear as sim1/sim2 in the report; no product names | Repo rule 11 | root `AGENTS.md` |
| The committed report is compared with the regenerated one ignoring the `Generated:` line. In the nightly run on `main` a difference is a warning, in a run by hand and on a release tag it fails the run (RELEASING.md checks the release commit by hand before tagging) | Ten agents push to `main`: failing every push for a changed count would block them; a release must carry a current report | #780 |
| The report is user-facing: it opens with a short plain-language summary and one table (suite, what it is, passed / total), terms explained once; everything else (per-suite detail, where and when each result was produced, gated lists, discrepancies, commands) follows under "Details", "How to reproduce" and "Known limits". No internal jargon (no agent names, tool option names or rule numbers); CI rows name no commit (only the `Generated:` line does, so `--check` can compare) | Maintainer instruction | #780, 2026-10-28 |
| Records of external tools that are not user-facing (`INTERNAL_RECORDS` in `run.py`: cocotb) stay in `conformance-local.json` and are left out of `conformance.md` | Maintainer: cocotb is an external tool and is not named in user documents | user, 2026-10-28 |
| The platform table is the full-scope plan of `tools/ci-plan.py` (nightly, by hand, release tag) for each job whose `ci.yml` matrix reads a `plan` output; no match fails the run instead of printing an empty table | `ci.yml` stopped having a literal `os: [..]` list in #888 and the old regex then returned nothing, silently | #1018 |
| Nothing is published outside the repository (no Pages) | Needs the maintainer's decision | #780 |
| Corpus results come from `tools/corpus/run.py` against the gated `.expected` lists; sim1's numbers are the recorded `.sim1.expected` lists, not a rerun | Same files, no reference needed in CI | #780 |

## 4. Invariants and pitfalls

- The site shows only the summary: `docs/conf.py` cuts `conformance.md` at the `## Details` heading and fails the docs build without it. Keep that heading.
- Exit statuses of the underlying tools are results, not errors (a failing suite is a number in the report), except `realbench`, whose failure stops the run.
- The report text must be deterministic for a given commit: no timestamps outside the `Generated:` line, no timings, sorted keys.
- A `--local` run needs a clean tree to record a meaningful commit (`-dirty` is appended otherwise); commit `docs/conformance-local.json` with the regenerated report.
- The `Generated:` line says `-dirty` when `git status --porcelain` lists anything outside `docs/conformance*`, untracked files included. CI keeps its corpus cache in `.corpus-cache`, which `.gitignore` lists for that reason: any other untracked directory in a CI job would make the committed line say `-dirty`. To refresh the line from an artifact, run `--report-only` in a clean tree.
- Every tool it calls keeps its scratch under `--out`, not the user's shared `~/.cache/yavs/std`.

## 5. How to work here

The CI parts need no local run: download the `conformance-report` artifact of a green `conformance` run and run `run.py --report-only --out <dir>` on it to refresh the committed report (the local records stay). The workflow runs nightly on `main`, on release tags and by hand, not on a push (`gh workflow run conformance.yml --ref main` makes a run of the current head).

```sh
tools/fetch-corpus.sh
cargo build --release -p yavs        # --features clift for the auto and clift tiers
tools/conformance/run.py --yavs target/release/yavs --out ~/.cache/yavs-conformance [--local --compiled]
tools/conformance/run.py --check --yavs target/release/yavs --out ~/.cache/yavs-conformance
```
