# AGENTS.md: tools/corpus/ (analysis corpora)

Read the root `AGENTS.md` and `tools/AGENTS.md` first. This file is for an agent about to change
an analysis corpus manifest, an expected list or the runner. The full usage is in the docstrings
of `run.py`, `gen_manifests.py` and `gen_external.py`.

## 1. Purpose and boundaries

- Measures `yavs analyze` on real libraries (M1 check: zero diagnostics on the library corpora):
  one TOML manifest per corpus and standard (VUnit, VUnit tests, OSVVM, UVVM, hdl-modules,
  open-logic, en_cl_fix, neorv32, satcat5, surf, vhlib, json-for-vhdl, fletcher, VHDL compliance,
  tool checks, ideas, interfaces, extras, and `yavs-lib` = our own `lib/`), each with expected
  PASS/FAIL lists for yavs and for NVC.
- Not here: parse-only corpora (`tests/parse-corpus/`, run by `cargo test -p yavs --features
  corpus`) and simulation (`tools/difftest`). Sources come from the corpus cache, never copied.

## 2. Map

| File | What |
|---|---|
| `<name>.toml` | Generated compile manifest: `name`, `std`, ordered files with library, `exclude` with reasons, `[flags]` per tool. Do not edit by hand |
| `<name>.expected` | yavs expected list, one `PASS|FAIL <lib> <source>/<path>` per entry in order (initially derived from NVC's) |
| `<name>.nvc.expected`, `<name>.nvc.md` | NVC results and the validation note (tool version, accepted/rejected, flags) |
| `rejects/<name>.json` | Files NVC/GHDL/Questa rejected during validation, with the first error line |
| `run.py MANIFEST [--tool yavs|nvc] [--cmd T] [--keep-going] [--bless] [--expected none] [--log F]` | Analyze one file per call in manifest order and compare with the expected list; exit 0 as expected, 1 differences, 2 usage |
| `gen_manifests.py [--root DIR] [NAME...]`, `--validate NAME...` | Generate manifests from each project's own build description (VUnit `builtins.py`, OSVVM `.pro`, UVVM compile orders, `lib/ORDER.md`) |
| `gen_external.py` | Specs for the external library corpora and their validation with NVC plus GHDL (2008) or Questa (2019) |
| `report.sh [YAVS_BINARY]` | Run every manifest (4 in parallel, no comparison) and regenerate `RESULTS.md`, grouping failures by first diagnostic |
| `RESULTS.md` | Generated; do not edit by hand |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Manifests are generated from the projects' own build descriptions and validated against reference tools; files the references reject are excluded with the reason | yavs must analyze what NVC/GHDL/Questa accept (LRM policy) | `gen_external.py` docstring, `docs/plan.md` "LRM interpretation policy" |
| Where NVC and GHDL disagree, Questa decides | Tie-breaker | `gen_external.py` |
| One file per `yavs analyze` call, in manifest order | Matches how VUnit compiles; per-file failures are exact | `run.py`, `docs/decisions.md` "Semantic analysis" (failed file hides its units) |
| Each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G` | Root rule 15 | `run.py` |
| Expected lists change only with `--bless` and a reviewed diff explained in the commit | No silent regressions | `run.py` |

Decisions about the corpora go in this section in the same commit (root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- A `FAIL` in an expected list is a known yavs bug: each should have an issue (`gh issue list -R
  ru551n/yavs --label false-positive`). Turning `FAIL` into `PASS` is the goal; the reverse needs
  a reason in the commit.
- `report.sh` builds a release binary and uses about 4 parallel analyses; check memory and disk
  first. Its scratch dir is `mktemp` under `$TMPDIR` or `~/.cache`.
- `std`/`ieee` are mapped only for `yavs-lib-*` (with `--bootstrap`); other manifests must not
  map them (E0102).

## 5. How to work here

```sh
tools/fetch-corpus.sh                                         # sources into ~/.cache/rsim-corpus
python3 tools/corpus/run.py vunit-2008 --keep-going           # compare with vunit-2008.expected
python3 tools/corpus/run.py vunit-2008 --bless                # accept, then review git diff
python3 tools/corpus/run.py osvvm-2019 --tool nvc             # NVC reference
python3 tools/corpus/gen_manifests.py vunit-2008              # regenerate a manifest
tools/corpus/report.sh                                        # RESULTS.md
```

## 6. Coordination

- Open issues (snapshot 2026-10-18): #62 refresh `RESULTS.md`; #63 gate in CI; corpus false
  positives and bugs #55 to #61, #157; #115 OSVVM and UVVM own suites; #112 VUnit test suites.
