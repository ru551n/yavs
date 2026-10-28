# AGENTS.md: working on yavs

yavs is a production VHDL simulator in Rust: VHDL-1993, 2008 and 2019, an IR interpreter plus optional compiled tiers
(LLVM or Cranelift), a parallel kernel, VHPI (partial) for Python testbench frameworks, external test runner support, a DAP
server (with a VS Code extension) and an early graphical debugger. The name stands for "Yet Another VHDL Simulator". This file is for AI agents (and humans) changing the code. Read it
fully before your first edit. It is short on purpose: the details live in the documents it points
to, and those documents win over your memory.

## 1. Read before you work

| Always | Why |
|---|---|
| `docs/plan.md` | Binding plan: decisions, the 15 design rules, milestones, error motto, LRM policy |
| `docs/decisions.md` | Decisions made since the plan, by area; append yours here |
| `docs/STATUS.md` | What exists and works, per crate and milestone |
| `release_notes/news.d/README.md` | Release note fragments (towncrier): one file per user-visible change (rule 25) |
| `<module>/AGENTS.md` | Per-module guide (every crate, `lib/`, `tools/`, `tests/`, `fuzz/`): boundaries, map, binding decisions, pitfalls, commands, open issues (rule 23) |

| When you touch | Also read |
|---|---|
| parser (`yavs-syntax`) | `crates/yavs-syntax/AGENTS.md`, `docs/decisions.md` "Parser", `docs/design/lrm-checks.md` P1/P2 |
| semantic analysis (`yavs-sem`) | `crates/yavs-sem/AGENTS.md`, `docs/design/sem.md`, `docs/design/lrm-checks.md`, `crates/yavs-sem/CONTRIBUTING.md` |
| IR, interpreter, lowering, elaboration | `crates/yavs-{ir,interp,lower,elab}/AGENTS.md`, `docs/design/ir.md`, `crates/yavs-ir/README.md`, `crates/yavs-lower/DESIGN.md`, `crates/yavs-kernel/DESIGN.md` |
| kernel (`yavs-kernel`) | `crates/yavs-kernel/AGENTS.md`, `crates/yavs-kernel/DESIGN.md` |
| CLI, libraries, test-runner integration | `crates/yavs/AGENTS.md`, `crates/yavs-lib/AGENTS.md`, `docs/cli-contract.md` (external test runners depend on it: change it deliberately) |
| diagnostics, logic, std natives | `crates/yavs-{diag,logic,std}/AGENTS.md` |
| FFI, VHPI, waves, DAP, coverage, LLVM | `crates/yavs-{ffi,vhpi,wave,dap,cov,llvm}/AGENTS.md` |
| `lib/` (std/ieee sources) | `lib/AGENTS.md`, `lib/README.md`, `lib/SOURCES.md`, `lib/NATIVE.md` |
| the GUI (`gui/`) | `gui/AGENTS.md`, `docs/design/gui.md` |
| tests and tools | `tests/AGENTS.md` (and `tests/{diag,corpus,parse-corpus,discrepancies}/AGENTS.md`), `tools/AGENTS.md` (and `tools/{corpus,difftest,discrepancies}/AGENTS.md`), `fuzz/AGENTS.md` |

## 2. Repository map

| Path | What |
|---|---|
| `crates/yavs-diag` | Diagnostics model and renderer (codes, snippets, labels, notes, help); code registry in its crate docs |
| `crates/yavs-syntax` | Lexer, conditional analysis, parser, owned AST (2008 and 2019) |
| `crates/yavs-sem` | Semantic analysis to typed HIR (`hir.rs`); module owners in its `CONTRIBUTING.md` |
| `crates/yavs-lib` | Library manifests, `_YAVS_LIB` locking, unit closure |
| `crates/yavs-cache` | The on-disk store of the compilation cache (keys, atomic writes, validation, pruning); VHDL-agnostic |
| `crates/yavs-prefetch` | A cache prefetch hint (`lines`): the one `unsafe` call the kernel's batch loop needs, so `yavs-kernel` stays `forbid(unsafe_code)` |
| `crates/yavs-logic` | `std_ulogic` and packed logic vectors (bit-planes are the default) |
| `crates/yavs-std` | Native `std.standard`/`textio`/`env` semantics, checked arithmetic |
| `crates/yavs-kernel` | LRM 14.7 cycle, drivers, ports, parallel evaluation, `SimControl`, event bus |
| `crates/yavs-ir`, `yavs-interp`, `yavs-lower`, `yavs-elab` | M2 middle end (see `docs/design/ir.md`) |
| `crates/yavs-ffi` | Foreign-call trampolines per ABI (no libffi) |
| `crates/yavs-vhpi` | VHPI C ABI and plugin loading; vendored IEEE `vhpi_user.h` |
| `crates/yavs-wave` | FST waveform sink |
| `crates/yavs-dap` | Debug Adapter Protocol server (`yavs run --dap`) |
| `crates/yavs-cov` | Code coverage: counters to a mergeable database, text and Cobertura reports |
| `crates/yavs-llvm` | LLVM 21 ORC JIT tier, behind the `llvm` feature (not a default feature; not shipped in the release archives, build it from source) |
| `crates/yavs-clift` | Cranelift tier (#481), behind the `clift` feature (off by default; the compiled tier of the release archives); shares `yavs-llvm`'s `rt` module |
| `crates/yavs` | The `yavs` binary |
| `lib/` | Vendored IEEE std/ieee sources (2008, 2019), patched only via `tools/derive_libs.py` |
| `tests/diag/` | Rendered-diagnostic snapshots; `tests/diag/sem/` negative suite with `-- expect:` headers |
| `tests/corpus/` | Differential testbenches with reference-simulator outputs (`tools/difftest`) |
| `tests/parse-corpus/` | Expected parse results for external corpora |
| `tests/discrepancies/` | Reproducers of every known simulator discrepancy; catalogue `docs/design/simulator-discrepancies.md`, runner `tools/discrepancies/run.py` |
| `tools/corpus/` | Analysis corpus manifests (external libraries, `lib`), `run.py`, `RESULTS.md` |
| `fuzz/` | cargo-fuzz targets (own workspace) |
| `editors/vscode/` | VS Code extension contributing the `yavs` debugger (plain JS) |
| `gui/` | `yavs-gui`, the graphical debugger: its own workspace, EUPL-1.2, DAP client only (`gui/AGENTS.md`) |
| `python/vunit-yavs/` | The Python test runner package `vunit-yavs`: simulator interface, `--yavs-debug`, thread cap, jobserver, PyPI release (`python/vunit-yavs/AGENTS.md`) |

## 3. Rules (mandatory)

### Correctness and behaviour
1. **Fail early, fail clearly.** Every problem is a `yavs_diag::Diagnostic` with a code, primary span, labels, notes and help.
   No stage runs after a failed one. No silent
   fallbacks: never `unwrap_or(default)` on anything that comes from the design, the user or the
   environment. A broken invariant is an internal-compiler-error diagnostic ("this is a bug in
   yavs"), never a bare panic. Not-yet-implemented features report an explicit "not implemented"
   diagnostic naming the milestone or slice, never a guess.
2. **LRM policy.** Where the LRM is clear, follow it and fail early. Where it is unclear or silent,
   be permissive: accept what the other major simulators accept, pick the
   semantics that lets the most designs elaborate, and warn only when simulators give different
   results. Record each such choice in `docs/decisions.md` or `docs/design/lrm-checks.md`.
3. **The interpreter is the semantic reference.** The JIT tier must produce identical results.
   Runs with `--threads 1` and `--threads N` must produce byte-identical output.
4. **Respect the design rules** in `docs/plan.md` (runtime constraints on every composite, one
   sub-element signal map, checked i64 everywhere, original identifier spelling, generational
   handles, and the rest). If you think one is wrong, say so in your report; don't route around it.
5. **Every diagnostic has a stable code** from the registry (`yavs-diag` crate docs, `sem.md` 8.1).
   New codes are added to the registry in the same commit.

### Licensing and sources
6. **Never copy code** from sim1 or sim2 (GPL) or rust_hdl (MPL). Reading them for behaviour is
   fine. Write everything fresh from the LRM and our design docs.
7. **External corpora are inputs, never repository content.** Fetch them with
   `tools/fetch-corpus.sh` into a cache directory. Never commit their files.
8. **Third-party package sources are vendored, never rewritten.** The Synopsys and VITAL packages
   are the original files, byte for byte (`lib/SOURCES.md`); do not reproduce or edit third-party
   package text from memory.
9. **Dependencies:** a crate that cargo fetches and builds by itself is fine. Anything that needs a
   manual install must be approved by the user first. Approved so far: LLVM 21 (optional feature),
   a C compiler at build time, reference simulators as dev-only references, qemu-user and the aarch64
   cross linker for testing.

### Code
10. **Lean code.** No speculative abstractions, no unused crates, features or config. Match the
    surrounding code's naming, comment density and idiom. Mark deliberate shortcuts with a
    `ponytail:` comment naming the ceiling and the upgrade path.
11. **`unsafe` only where unavoidable** (FFI, VHPI, LLVM). Those crates opt out of the workspace
    `forbid` and confine `unsafe` to one module with a comment.
12. **Ownership.** In shared crates (notably `yavs-sem`) claim your module in its `CONTRIBUTING.md`
    before editing. Changes in someone else's module must be small, additive, in their own commit,
    or a request in that file's Requests section. Never fork another crate's types.

### Tests and checks
13. **Green before every push, on the exact commit you push:**
    `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
    `cargo test --workspace`. Never pipe a check through `grep`/`tail` in a way that hides its exit
    status. These checks run whenever the pushed range (`origin/main..HEAD`) touches `crates/`,
    regardless of what you think you changed.
14. **Every behaviour gets a test.** Errors get a negative test with the code and position.
    Rendered output changes are snapshot changes that show up in review (`YAVS_BLESS=1` to update,
    then inspect the diff).
15. **Simulator reference runs** are wrapped as
    `timeout 300 systemd-run --user --scope -p MemoryMax=2G ...`, at most 8 at once, with work
    directories under `~/.cache/<your-dir>`, never inside the repository.
16. **No testbench may run longer than 5 minutes.** Shrink the scenario, never weaken assertions.

### Git and collaboration
17. **Current phase: push straight to `main`** (no branches, no PRs). CI
    (`.github/workflows/ci.yml`) runs the rule 13 checks (plus the analysis corpus gate, Linux x86_64 only) on Linux x86_64, Linux aarch64 and
    Windows x86_64 for every push: watch your run (`gh run watch`) and fix a red one. Rebase on
    `origin/main` first; push only with no rebase in progress and a clean tree. If the push is
    rejected, rebase again and re-run the checks. **Before every push run
    `git diff --stat origin/main..HEAD`** (or `git show --stat HEAD`) and confirm every path is
    one you meant to change: any diff outside your area means a bad rebase (commit 219eabe, a
    "tools only" commit, silently reverted another agent's Rust work through a conflicted rebase
    and skipped the checks; fixed in 6a21cb8).
18. **Commit messages:** imperative subject and a body that says why. No `Co-Authored-By`
    trailer (user rule), and commit as the GitHub noreply address
    `1427336+ru551n@users.noreply.github.com` (set by `git config user.email` in this repo). No
    session links. No em dashes anywhere (code,
    comments, docs, commits): use a colon, full stop or comma.
19. **Worktrees:** parallel agents each use their own git worktree. Never edit another agent's
    worktree. Never use bare `git stash` (the stash is shared across worktrees).

### Resources
20. **Disk and memory are shared.** Set `CARGO_BUILD_JOBS=6`. Put large outputs under
    `~/.cache/<your-dir>`, not `/tmp` (a small shared tmpfs). Run `cargo clean` in your worktree
    when you finish (unless told you share a target directory). Check `df -h ~` before long runs.
21. **Ask before anything irreversible or outward-facing**: deleting data you did not create,
    force-pushing, publishing, contacting external services beyond read-only fetches.

### Issues
22. **Work is tracked as GitHub issues** on `ru551n/yavs`. Before starting, find the
    issue for your task (`gh issue list -R ru551n/yavs --search ...`); if none exists, create one
    with the labels described in its template (milestone `M<n>`, crate `crate:<name>`, kind).
    Reference it in your commits, and close it with `Fixes #N` in the commit that completes it
    (or `gh issue close N --comment "<commit>: <what was done>"` if work was spread over several
    commits). Things you find but don't fix become new issues, not just notes in a report.

### Documentation and decisions
23. **Before working in a module, read its `AGENTS.md`; when your change makes it stale, update it
    in the same commit.** Each module directory also has a one-line `CLAUDE.md` (`@AGENTS.md`).
24. **Every design decision is stored in the applicable place, in the same commit as the change
    that makes it:**
    - project-wide or cross-crate decisions: `docs/decisions.md` (and `docs/plan.md` if it changes
      the plan);
    - decisions local to one module: that module's `AGENTS.md` "Design decisions" section (plus its
      design doc section if one exists, e.g. `docs/design/sem.md`, `ir.md`, a crate `DESIGN.md`);
    - simulator disagreements: a row and a reproducer in `docs/design/simulator-discrepancies.md`
      and `tests/discrepancies/` (the reading itself stays in `lrm-checks.md`);
    - LRM interpretations: `docs/design/lrm-checks.md` (policy, rule 2: a clear LRM wins; where it
      is unclear, be permissive and match the other simulators);
    - CLI or test-runner-facing behaviour: `docs/cli-contract.md`.
25. **Release notes: one towncrier fragment per user-visible change** (user, 2026-10-18), in the
    same commit: `release_notes/news.d/<issue>.<type>.md` (`+<name>.<type>.md` without an
    issue), type `breaking`, `feature`, `bugfix`, `doc` or `misc`, one or two sentences on what
    a user can do now: features, not numbers. The user documentation (README, `docs/*.rst`,
    release notes) describes yavs as a simulator on its own: never mention other projects there
    (test runners, simulators, libraries, frameworks), and the same holds for every document in the
    repository (developer docs, `AGENTS.md` files, `docs/STATUS.md`, tool and test docs), except licence notes
    (user, 2026-10-18, widened the same day). Never write `release_notes/<version>.md` by hand
    outside a release. The user documentation (Sphinx, `docs/conf.py`, `docs/*.rst`, published
    on Read the Docs) shows the fragments under "Unreleased"; build it with
    `sphinx-build -W -b html docs docs/_build/html` after `pip install -r docs/requirements.txt`.

    Each entry gives the decision, a one-line rationale, and who decided it (`user, YYYY-MM-DD`
    for user rulings). Decisions that need the user get a GitHub issue labelled `decision`, and
    are recorded once answered. A decision that exists only in a commit message, an agent report
    or a chat is not recorded.

## 3a. Working alongside other agents

Several agents, possibly from different tools or machines, work on this repo at once. GitHub is
the shared source of truth; `tools/claim` (needs `gh`) is the only protocol. `who` is
`<harness>/<agent>`, e.g. `cc-host1/kern23`.

- One issue per task. Before starting: `tools/claim take <issue> <who>` (exit 0 = yours; non-zero =
  someone else holds it or won the race, pick another issue). Never start on a claimed issue.
- While working: `tools/claim list` shows active claims; `tools/claim stale [hours]` lists claims
  older than 24 h by default, for a human to clear.
- Before every push: `git fetch && git rebase origin/main`, then the CI gates (rule 13) green on
  that exact commit. Push only fast-forward to `main` (`git push origin HEAD:main`), never force.
- When done or abandoned: `tools/claim release <issue> <who> [note]`.
- Comments posted through the maintainer's account are prefixed "Written by an agent (<who>)."
  by the tool.
- Host-local only (these locks do not coordinate other machines): `tools/claim core [-- cmd...]`
  picks a free timing core 8-15 and runs the command pinned there; `tools/claim build -- cmd...`
  runs a heavy build in one of 4 host-wide slots and refuses below 50 GB free disk.

## 4. Common commands

```sh
cargo test --workspace                               # unit, snapshot and lib/ tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
YAVS_BLESS=1 cargo test -p yavs                      # update rendered snapshots, then review the diff
tools/fetch-corpus.sh                                # external corpora into ~/.cache/rsim-corpus
YAVS_CORPUS=~/.cache/rsim-corpus cargo test -p yavs --features corpus   # parse corpora
tools/corpus/report.sh                               # analyze all corpora, write tools/corpus/RESULTS.md
python3 tools/difftest/difftest.py --candidate sim1   # differential corpus (see tools/difftest/README.md)
python3 tools/discrepancies/run.py --check           # simulator discrepancy catalogue (docs/design/simulator-discrepancies.md)
cargo test -p yavs-llvm --features llvm              # LLVM tier (needs LLVM 21)
cargo run -p yavs -- analyze --std=2008 --work=work:out/work file.vhd
yavs run --profile[=out.json] --work=work:W top      # first tool for "where does a long run spend its time": busiest processes, instances, subprograms with file:line (stderr; JSON with FILE)
```

For a performance question about a long run, start with `--profile` (VHDL-level: which process, which line), then `perf` on a
`tools/build-bench.sh` build (machine-level) for the cause inside that process.

AArch64 under qemu and Windows under Wine: see the commands in `docs/STATUS.md` ("ffi").

### Work-counter tests (perf regressions that do not depend on the host)

`crates/yavs/tests/perf.rs` runs about 25 small designs (`crates/yavs/tests/perf/*.vhd`, each
asserting its own result) with `YAVS_COUNTERS=1 --tier=interp --threads 1` and checks counters of
work against bounds: kernel (`cycles`, `process_runs`, `wakes`, `events`, update paths,
quick and direct commits), interpreter `ops` executed, elaboration call cache hits, analysed-unit
cache hits, recorded wave variables, and under `--features llvm` (the CI `llvm` job) the
callbacks of compiled code and the instructions left to the interpreter. A bound is a ceiling
(`Max`) or, where fewer is the regression, a floor (`Min`). `ops` and `cb_*` are counted in debug
builds and with `--features op-census` only (a release test run skips them).

```sh
cargo test -p yavs --test perf                                   # check
YAVS_PERF_PRINT=1 cargo test -p yavs --test perf -- --nocapture  # every counter of every design
cargo test -p yavs --features llvm --test perf                   # plus the LLVM counters
```

- A failure names the counter, design, value and bound. First find the change that made the
  design do more work; if the new value is intended (a trade, a feature), edit the bound in the
  `case!` line of that design to the new value plus about 3 percent and say why in the commit
  message. When your change lowers a counter, lower its bound to the new value plus the margin, so
  the next regression is seen. A new perf feature gets a design and bounds in the same commit:
  pick the counter of the slow path it avoids, not only totals.
- Counters are exact and equal on every host and platform: no timing in these tests. Keep a design
  to a few hundred cycles (debug builds run it) and make it print a checksum.
- Rule of thumb for new counters: a plain increment on a path that is already slow, or one add per
  cycle. Nothing per op or per element on a fast path in release builds; `ops` and `cb_*` use
  `cfg(any(debug_assertions, feature = "op-census"))` for that reason.

## 5. When you finish a task

- Update `docs/STATUS.md` for what you changed, and `docs/decisions.md` for any decision you made.
- Add a release note fragment in `release_notes/news.d/` for every user-visible change (rule 25),
  and update the user documentation in `docs/*.rst` when the change shows there. The site is a
  short user guide (`docs/README.md`): full behaviour detail goes in `docs/cli-contract.md`.
- Report: commits pushed, what works now (with test and corpus numbers), what is left, and
  every decision the user should confirm. Say plainly what you did not verify.
