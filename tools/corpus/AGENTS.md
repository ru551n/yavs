# AGENTS.md: tools/corpus/ (analysis corpora)

Read the root `AGENTS.md` and `tools/AGENTS.md` first. This file is for an agent about to change
an analysis corpus manifest, an expected list or the runner. The full usage is in the docstrings
of `run.py`, `gen_manifests.py` and `gen_external.py`.

## 1. Purpose and boundaries

- Measures `yavs analyze` on real libraries (M1 check: zero diagnostics on the library corpora):
  one TOML manifest per corpus and standard (external test runner and its tests, verification libraries, and third-party
  libraries and cores, named as in the `<name>-<std>.toml` files, plus `yavs-lib` = our own `lib/`), each with expected
  PASS/FAIL lists for yavs and for the reference simulator.
- Not here: parse-only corpora (`tests/parse-corpus/`, run by `cargo test -p yavs --features
  corpus`) and simulation (`tools/difftest`). Sources come from the corpus cache, never copied.

## 2. Map

| File | What |
|---|---|
| `<name>.toml` | Generated compile manifest: `name`, `std`, ordered files with library, `exclude` with reasons, `[flags]` per tool. Do not edit by hand |
| `<name>.expected` | yavs expected list, one `PASS|FAIL <lib> <source>/<path>` per entry in order (PASS everywhere except the known yavs bugs; CI fails on any difference) |
| `<name>.sim1.expected`, `<name>.sim1.md` | The reference simulator results and the validation note (tool version, accepted/rejected, flags) |
| `rejects/<name>.json` | Files the reference simulators rejected during validation, with the first error line |
| `run.py MANIFEST [--tool yavs|sim1] [--cmd T] [--keep-going] [--bless] [--expected none] [--log F]` | Analyze in manifest order (yavs: consecutive files of a library and standard per call, `--per-file` for one call per file) and compare with the expected list; exit 0 as expected, 1 differences, 2 usage |
| `gen_manifests.py [--root DIR] [NAME...]`, `--validate NAME...` | Generate manifests from each project's own build description (its compile-order scripts and project files, and `lib/ORDER.md`) |
| `gen_external.py` | Specs for the external library corpora and their validation with the reference simulator plus the second reference simulator (2008) or the third reference simulator (2019) |
| `unisim-2008.toml`, `gen_vendor.py` | Vendor primitive libraries (`unisim`, `unimacro`) read in place from `$YAVS_VENDOR_UNISIM` (the directory holding `unisims/` and `unimacro/`); `run.py` skips the manifest (exit 0) when unset, so CI skips it. No expected list: results depend on the installed version. `gen_vendor.py` regenerates it from the vendor's analyze-order lists. Files are never copied or committed (rule 7). `run.py --extra-map unisim:DIR/unisim` maps the built library into another run (the 6 hdl-modules files that need it) |
| `report.sh [YAVS_BINARY]` | Run every manifest (4 in parallel, no comparison) and regenerate `RESULTS.md`, grouping failures by first diagnostic |
| `RESULTS.md` | Generated; do not edit by hand |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Manifests are generated from the projects' own build descriptions and validated against reference tools; files the references reject are excluded with the reason | yavs must analyze what the reference simulators accept (LRM policy) | `gen_external.py` docstring, `docs/plan.md` "LRM interpretation policy" |
| Where the reference simulators disagree, the third reference simulator decides | Tie-breaker | `gen_external.py` |
| The third reference simulator is found on `PATH` and run in the caller's environment; its licence comes from `LM_LICENSE_FILE`, `SALT_LICENSE_FILE` or `SALT_LICENSE_SERVER` (never written into the repo; the simulator itself needs `SALT_LICENSE_SERVER`, which `gen_external.py` fills from the other two when unset, and so should any script that runs it), and a licence failure aborts with a message naming those variables | Availability is "the tool runs", not one specific variable | `gen_external.py` |
| A corpus the reference simulators analyze only with their relaxed flag runs yavs with `--relaxed` too (uvvm, open-logic: unprotected shared variables, #52) | Same input contract as the references | `[flags]` in the manifest, `gen_external.py`, `gen_manifests.py` |
| yavs: consecutive files of one library and standard in one `yavs analyze` call, in manifest order; a call that fails is repeated file by file, so every file keeps its exact result and output, and a call that fails while every file passes alone is a `DIFF` (a session-only failure, a memory cap; `--per-file` and every reference label: one call per file) | Integrations batch since #745 (user, 2026-10-25), so the corpus gate exercises the session path; per-file failures stay exact. All 34 manifests give identical results either way | `run.py`, `docs/decisions.md` "Batched analysis calls", "Semantic analysis" (failed file hides its units) |
| Each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G` | Root rule 15 | `run.py` |
| Default `--out` is `~/.cache/yavs-corpus-out/<tool>-<name>-<pid>`, unique per run (pass `--out` to choose; old dirs are not removed) | Parallel agents sharing one library directory produced bogus diffs (#202) | `run.py` |
| Expected lists change only with `--bless` and a reviewed diff explained in the commit | No silent regressions | `run.py` |
| `grlib-2008` (GRLIB 2026.3, the Gaisler IP library) takes its order from GRLIB's own script generator: `gen_external.grlib_files` runs the make target named by key `grlib_target` of sim1 in `tools/sims.local.conf` (GRLIB's Tcl `bin/scriptgen` writes the ordered per-file list, only into its working directory) for `designs/leon3mp` with `TECHLIBS=inferred` in a temporary copy of the template directory (needs `make` and `tclsh`) and keeps the `.vhd` entries under `lib/` with the library they name; the template's own files (config, ROM, top, testbench) are not in the manifest, they are the own part of the realbench test. The manifest is compiled as VHDL-2008, GRLIB itself is VHDL-93 (its scripts pass the 93 flags; `stdio.vhd` and `testlib.vhd` are tagged `vhdlstd=93`): their `std_logic_vector`/`std_ulogic_vector` overload pairs are homographs under 2008, the references reject them, and 46 files that use them drop out as cascades (processor core, bus controller, simulation models), see `grlib-2008.sim1.md`. Where sim1 and sim2 disagree and no sim3 is configured, the file is dropped. `grlib-1993` is the same list compiled at `--std=93` (GRLIB's own standard, the sim2 flags are GRLIB's `-fexplicit` and synopsys switch): 454 files, only `axi4mux.vhd` and its cascade `axi4mem.vhd` (errors in GRLIB itself) are excluded; `grlib_leon3mp` of `tools/realbench` runs on it | GRLIB's own lists are the only complete order (it has no regex-scannable layout); 93 code cannot be analysed unmodified by a 2008-only tool, and the corpus never patches sources (root rule 7) | `gen_external.py`, #604 |
| `vests-1993` (the VESTs suite for 1076-1993, GPL-2.0, University of Cincinnati) is the compliant directories of the billowitch, ashenden and clifton-labs groups, all in library `work`, order by the regex dependency scan, validated against sim1 and sim2 at 1993 (sim2 as `93c`); there is no third reference at 1993, so a file the two disagree on is dropped. The non-compliant and disputed directories are not part of the list. `yavs-lib-1993` is `lib/ORDER.md` order-1993 and has no sim1 list: its bootstrap rejects the 64-bit `TIME` range of the vendored `standard.vhdl` at 1993 | A 1993 language test suite for the 93 mode; the compliant tests are the ones whose result is "accepted" | `gen_external.py`, #629 |

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
python3 tools/corpus/run.py osvvm-2019 --tool sim1             # reference simulator
python3 tools/corpus/gen_manifests.py vunit-2008              # regenerate a manifest
tools/corpus/report.sh                                        # RESULTS.md
```

## 6. Coordination

- Open issues (snapshot 2026-10-18): #62 refresh `RESULTS.md`; corpus false
  positives and bugs #59, #61, #157 (#55 to #58, #60 closed as stale 2026-10-18); #115 external verification library suites; #112 external test runner suites.

CI (`corpus` job in `.github/workflows/ci.yml`, Linux x86_64) runs every manifest with `run.py
--no-wrap --keep-going` and fails on any difference, so a regression and an unexpected pass both
turn it red; after fixing a bug `--bless` the corpus and review the diff. Whole job: about 4 minutes
plus the fetch (cached by the hash of `tools/fetch-corpus.sh`).
