# AGENTS.md: tools/

Read the root `AGENTS.md` first. This file is for an agent about to change or run a script under
`tools/`. Subdirectories have their own guides: `tools/corpus/AGENTS.md` (analysis corpora) and
`tools/difftest/AGENTS.md` (differential simulation corpus), `tools/conformance/AGENTS.md`, `tools/suites/AGENTS.md`, `tools/bench/AGENTS.md` and `tools/realbench/AGENTS.md`, `tools/stepcount/AGENTS.md`, `tools/waveq/AGENTS.md`, `tools/cocotb/AGENTS.md`.

## 1. Purpose and boundaries

- Developer and CI scripts around the Rust workspace: fetching external corpora, deriving and
  checking the vendored IEEE libraries, generating `lib/NATIVE.md`, reference simulator runs, the
  analysis corpus runner and the differential test harness.
- Not part of any crate; nothing here is built by cargo. Python is stdlib only (run with
  `python3 -I`), shell is bash or POSIX sh.
- Inputs are external checkouts in a cache directory, never repository content (root rule 7).

## 2. Map

| Path | What |
|---|---|
| `fetch-corpus.sh [DIR]` | Fetch the external corpora (test-runner and verification libraries, third-party VHDL libraries, the reference simulators' own tests and a conformance suite; the list is in the script) at pinned commits into `DIR` (default `~/.cache/rsim-corpus`), sparse where possible |
| `derive_libs.py <packages-checkout> <outdir>` | Derive `lib/std` and `lib/ieee` from IEEE VASG Packages: byte-for-byte upstream plus the edits listed in `lib/SOURCES.md` |
| `update-ieee-libs.sh [--write]` | Re-fetch the pinned VASG commit, verify `lib/UPSTREAM.sha256`, derive, compare (default) or rewrite |
| `gen_native_md.py` | Generate `lib/NATIVE.md` (`python3 -I tools/gen_native_md.py > lib/NATIVE.md`) |
| `check-sim1.sh [2008|2019]` | Analyze `lib/` with the reference simulator in `lib/ORDER.md` order (scratch `~/.cache/rsim-libs/work`, `YAVS_LIBS_SCRATCH`) |
| `conformance/` | The conformance report generator: runs the evidence suites, writes `docs/conformance.md` and `.json` (`tools/conformance/AGENTS.md`) |
| `release/build-libllvm.sh PREFIX` | Not used by the release since 2026-10-28 (kept for the later LLVM removal decision). Builds the self-contained libLLVM (shared, C API, host target, no extra libraries) of the Linux release archives inside a manylinux2014 container, so glibc 2.17 holds; checks its glibc symbols and needed libraries (`docs/decisions.md` "Release archives with and without the compiled tier", "One archive per platform") |
| `claim <take|release|list|stale|core|build>` | Issue claims on GitHub for multi-agent work, plus host-local core and build locks (root `AGENTS.md` section 3a; `claim selftest` checks the comment parser) |
| `suites/` | Drivers that simulate an external library's own testbenches on yavs (`tools/suites/AGENTS.md`) |
| `corpus/` | Analysis corpus manifests, runner, report (`tools/corpus/AGENTS.md`) |
| `difftest/` | Differential simulation harness (`tools/difftest/AGENTS.md`) |
| `cocotb/` | cocotb's own regression over VHPI: venv setup, the yavs makefile and Python runner for cocotb (`tools/cocotb/AGENTS.md`) |
| `vhpi-conformance/` | VHPI plugin traces on yavs and the reference simulator, diffed (`tools/vhpi-conformance/AGENTS.md`) |
| `bench/` | Shared performance benchmarks: phase timings, A/B comparison, JSON results (`tools/bench/AGENTS.md`) |
| `realbench/` | Real-world performance suite: about 30 testbenches of real projects, timed against the reference and between two binaries (`tools/realbench/AGENTS.md`) |
| `stepcount/` | Exact instruction counts per function and per source line for a window of a run (ptrace single-stepper in C plus a Python report; `tools/stepcount/AGENTS.md`) |
| `waveq/` | Waveform equivalence: real designs on yavs and the reference simulators, compared signal by signal (`tools/waveq/AGENTS.md`) |
| `discrepancies/` | Simulator discrepancy runner and verdict table (`tools/discrepancies/AGENTS.md`, catalogue `docs/design/simulator-discrepancies.md`) |

## 2a. Reference simulator configuration

Tracked files know the reference simulators only by label (sim1 to sim4). Everything that names a
product (executables, command templates, output markers, a few output-file prefixes) is in the untracked,
git-ignored `tools/sims.local.conf`, an INI file with a section per label, read by `tools/simconf.py`
(`YAVS_SIMS_CONF` overrides the path; `SIMCONF_<KEY>_<LABEL>` overrides a key). Keys by tool:
`discrep`, `bench` (discrepancies, bench, tiny: `analyse|||run[|||crash regex]`); `difftest`,
`difftest_relaxed`, `difftest_ignore`, `ext_analyse`, `ext_elab`, `ext_run`, `ext_psl` (difftest, external.py);
`corpus_cmd`, `corpus_suffix`, `corpus_fixup`, `gen_cmd`, `gen_setup`, `gen_licence`,
`gen_version`, `grlib_target` (sim1 only: the make target in GRLIB's script generator that writes the ordered per-file list, used by `tools/corpus/gen_external.py` for `grlib-2008`), `osvvm_tool_name`, `osvvm_tool_vendor`, `uvvm_simulator_name` (tools/corpus, tools/suites);
`real_elab`, `real_run` (realbench: elaboration and run templates with `{std} {lib} {dir} {gens} {top} {rest}`; analysis uses `corpus_cmd`, whose `{file}` must be a word of its own so that realbench can pass several files in one call, #745); `check_cmd`, `check_bootstrap` (check-sim1.sh); `waveq`, `waveq_flags` (tools/waveq: one design with a waveform, run options that write `{wave}`); `vhpi_analyse`, `vhpi_run` (vhpi-conformance);
`negtest_cmd`, `negtest_err`, `negtest_warn` (tests/diag/sem/check_ref.py); `golden_cmd`, `golden_name`
(crates/yavs-std/golden/gen.py). The `YAVS_SIM1_CMD` variable names the sim1 command of the ignored
cross-check test in `yavs-sem`. Never commit the file or put a tool name, path or licence value in a tracked file.

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| External corpora are fetched at pinned commits into a cache, never committed | Licences, repo size | root `AGENTS.md` rule 7, `docs/plan.md` "External test inputs" |
| A tool that runs yavs sets `YAVS_STD_CACHE` to a directory in its own cache (never the user's shared std/ieee store); a new tool does the same | A harness must not write the user's store; the store stays warm after a tool's first call, the agreed cold basis (user, 2026-10-26) | `docs/decisions.md` "Shared std/ieee units", #564 |
| Every reference simulator run is `timeout 300 systemd-run --user --scope -p MemoryMax=2G`, at most 8 at once | Shared host (OOM history) | root `AGENTS.md` rule 15, `docs/plan.md` "Working rules" |
| Scratch output under `~/.cache/rsim-*` (old name kept on purpose) | `/tmp` is a small shared tmpfs; directories exist on disk | root `AGENTS.md` rule 20, `docs/decisions.md` "Project name" |
| `lib/` changes only through `derive_libs.py` | Reproducible from upstream, hashes stay valid | `lib/SOURCES.md`, `lib/AGENTS.md` |
| The reference simulators are dev-only references | Approved dependencies | root `AGENTS.md` rule 9 |
| Exit statuses always propagate; never pipe a check into `grep`/`tail` | Error motto | root `AGENTS.md` rule 13 |

Decisions about the tooling go in this section (or the subdirectory guide) in the same commit
(root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- Pinned commits in `fetch-corpus.sh` and the expected lists in `tests/parse-corpus/` and
  `tools/corpus/*.expected` move together: bump a commit, re-run, review the list diff.
- `update-ieee-libs.sh` contacts opensource.ieee.org (a read-only fetch); it runs the check even
  with unknown arguments.
- Disk is shared and often near full (`df -h ~`): the corpora take several GB in
  `~/.cache/rsim-corpus`.

## 5. How to work here

```sh
tools/fetch-corpus.sh                                  # ~/.cache/rsim-corpus
YAVS_CORPUS=~/.cache/rsim-corpus cargo test -p yavs --features corpus
tools/update-ieee-libs.sh                              # lib/ still equals upstream plus edits
tools/check-sim1.sh 2019
```

- No diagnostic codes.

## 6. Coordination

- Open issues (snapshot 2026-10-18, `gh issue list -R ru551n/yavs --label tools`): #62 re-run
  `report.sh` and refresh `tools/corpus/RESULTS.md`; #63 gate analysis corpora in CI with
  expected-failure lists; #65 difftest: all `tests/corpus` groups equal to the reference simulator at 1, 4 and 4
  shuffled threads. Infra: #87 periodic fuzz and parse-corpus gate, #134 CI.
