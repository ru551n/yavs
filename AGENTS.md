# AGENTS.md: working on yavs

yavs is a production VHDL simulator in Rust: VHDL-2008 and 2019, an IR interpreter plus an
optional LLVM tier, a parallel kernel, VHPI for cocotb, VUnit support, and later a DAP server and a
GUI. The name stands for "Yet Another VHDL Simulator". This file is for AI agents (and humans) changing the code. Read it
fully before your first edit. It is short on purpose: the details live in the documents it points
to, and those documents win over your memory.

## 1. Read before you work

| Always | Why |
|---|---|
| `docs/plan.md` | Binding plan: decisions, the 15 design rules, milestones, error motto, LRM policy |
| `docs/decisions.md` | Decisions made since the plan, by area; append yours here |
| `STATUS.md` | What exists and works, per crate and milestone |
| `release_notes/news.d/README.md` | Release note fragments (towncrier): one file per user-visible change (rule 25) |
| `<module>/AGENTS.md` | Per-module guide (every crate, `lib/`, `tools/`, `tests/`, `fuzz/`): boundaries, map, binding decisions, pitfalls, commands, open issues (rule 23) |

| When you touch | Also read |
|---|---|
| parser (`yavs-syntax`) | `crates/yavs-syntax/AGENTS.md`, `docs/decisions.md` "Parser", `docs/design/lrm-checks.md` P1/P2 |
| semantic analysis (`yavs-sem`) | `crates/yavs-sem/AGENTS.md`, `docs/design/sem.md`, `docs/design/lrm-checks.md`, `crates/yavs-sem/CONTRIBUTING.md` |
| IR, interpreter, lowering, elaboration | `crates/yavs-{ir,interp,lower,elab}/AGENTS.md`, `docs/design/ir.md`, `crates/yavs-ir/README.md`, `crates/yavs-lower/DESIGN.md`, `crates/yavs-kernel/DESIGN.md` |
| kernel (`yavs-kernel`) | `crates/yavs-kernel/AGENTS.md`, `crates/yavs-kernel/DESIGN.md` |
| CLI, libraries, VUnit integration | `crates/yavs/AGENTS.md`, `crates/yavs-lib/AGENTS.md`, `docs/cli-contract.md` (VUnit depends on it: change it deliberately) |
| diagnostics, logic, std natives | `crates/yavs-{diag,logic,std}/AGENTS.md` |
| FFI, VHPI, waves, LLVM | `crates/yavs-{ffi,vhpi,wave,llvm}/AGENTS.md` |
| `lib/` (std/ieee sources) | `lib/AGENTS.md`, `lib/README.md`, `lib/SOURCES.md`, `lib/NATIVE.md` |
| tests and tools | `tests/AGENTS.md` (and `tests/{diag,corpus,parse-corpus}/AGENTS.md`), `tools/AGENTS.md` (and `tools/{corpus,difftest}/AGENTS.md`), `fuzz/AGENTS.md` |

## 2. Repository map

| Path | What |
|---|---|
| `crates/yavs-diag` | Diagnostics model and rustc-style renderer; code registry in its crate docs |
| `crates/yavs-syntax` | Lexer, conditional analysis, parser, owned AST (2008 and 2019) |
| `crates/yavs-sem` | Semantic analysis to typed HIR (`hir.rs`); module owners in its `CONTRIBUTING.md` |
| `crates/yavs-lib` | Library manifests, `_YAVS_LIB` locking, unit closure |
| `crates/yavs-logic` | `std_ulogic` and packed logic vectors (bit-planes are the default) |
| `crates/yavs-std` | Native `std.standard`/`textio`/`env` semantics, checked arithmetic |
| `crates/yavs-kernel` | LRM 14.7 cycle, drivers, ports, parallel evaluation, `SimControl`, event bus |
| `crates/yavs-ir`, `yavs-interp`, `yavs-lower`, `yavs-elab` | M2 middle end (see `docs/design/ir.md`) |
| `crates/yavs-ffi` | Foreign-call trampolines per ABI (no libffi) |
| `crates/yavs-vhpi` | VHPI C ABI and plugin loading; vendored IEEE `vhpi_user.h` |
| `crates/yavs-wave` | FST waveform sink |
| `crates/yavs-llvm` | LLVM 21 ORC JIT tier, behind the `llvm` feature (off by default) |
| `crates/yavs` | The `yavs` binary |
| `lib/` | Vendored IEEE std/ieee sources (2008, 2019), patched only via `tools/derive_libs.py` |
| `tests/diag/` | Rendered-diagnostic snapshots; `tests/diag/sem/` negative suite with `-- expect:` headers |
| `tests/corpus/` | Differential testbenches with NVC reference outputs (`tools/difftest`) |
| `tests/parse-corpus/` | Expected parse results for external corpora |
| `tools/corpus/` | Analysis corpus manifests (VUnit, OSVVM, UVVM, hdl-modules, lib, external libraries), `run.py`, `RESULTS.md` |
| `fuzz/` | cargo-fuzz targets (own workspace) |

## 3. Rules (mandatory)

### Correctness and behaviour
1. **Fail early, fail clearly.** Every problem is a `yavs_diag::Diagnostic` in the rustc format:
   code, primary span, labels, notes, help. No stage runs after a failed one. No silent
   fallbacks: never `unwrap_or(default)` on anything that comes from the design, the user or the
   environment. A broken invariant is an internal-compiler-error diagnostic ("this is a bug in
   yavs"), never a bare panic. Not-yet-implemented features report an explicit "not implemented"
   diagnostic naming the milestone or slice, never a guess.
2. **LRM policy.** Where the LRM is clear, follow it and fail early. Where it is unclear or silent,
   be permissive: accept what any major simulator (NVC, GHDL, Questa, Riviera) accepts, pick the
   semantics that lets the most designs elaborate, and warn only when simulators give different
   results. Record each such choice in `docs/decisions.md` or `docs/design/lrm-checks.md`.
3. **The interpreter is the semantic reference.** The LLVM tier must produce identical results.
   Runs with `--threads 1` and `--threads N` must produce byte-identical output.
4. **Respect the design rules** in `docs/plan.md` (runtime constraints on every composite, one
   sub-element signal map, checked i64 everywhere, original identifier spelling, generational
   handles, and the rest). If you think one is wrong, say so in your report; don't route around it.
5. **Every diagnostic has a stable code** from the registry (`yavs-diag` crate docs, `sem.md` 8.1).
   New codes are added to the registry in the same commit.

### Licensing and sources
6. **Never copy code** from NVC or GHDL (GPL) or rust_hdl (MPL). Reading them for behaviour is
   fine. Write everything fresh from the LRM and our design docs.
7. **External corpora are inputs, never repository content.** Fetch them with
   `tools/fetch-corpus.sh` into a cache directory. Never commit their files.
8. **Clean-room areas stay clean.** Do not reproduce third-party package sources (e.g. the
   Synopsys packages, which are deferred) from memory.
9. **Dependencies:** a crate that cargo fetches and builds by itself is fine. Anything that needs a
   manual install must be approved by the user first. Approved so far: LLVM 21 (optional feature),
   a C compiler at build time, NVC/GHDL/Questa as dev-only references, qemu-user and the aarch64
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
15. **Simulator reference runs** (NVC, GHDL, Questa) are wrapped as
    `timeout 300 systemd-run --user --scope -p MemoryMax=2G ...`, at most 8 at once, with work
    directories under `~/.cache/<your-dir>`, never inside the repository.
16. **No testbench may run longer than 5 minutes.** Shrink the scenario, never weaken assertions.

### Git and collaboration
17. **Current phase: push straight to `main`** (no branches, no PRs). CI
    (`.github/workflows/ci.yml`) runs the rule 13 checks on Linux x86_64, Linux aarch64 and
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
    - LRM interpretations: `docs/design/lrm-checks.md` (policy, rule 2: a clear LRM wins; where it
      is unclear, be permissive and match the existing simulators);
    - CLI or VUnit-facing behaviour: `docs/cli-contract.md`.
25. **Release notes: one towncrier fragment per user-visible change** (user, 2026-10-18), in the
    same commit: `release_notes/news.d/<issue>.<type>.md` (`+<name>.<type>.md` without an
    issue), type `breaking`, `feature`, `bugfix`, `doc` or `misc`, one or two sentences on what
    a user can do now: features, not numbers. The user documentation (README, `docs/*.rst`,
    release notes) describes yavs as a simulator on its own: never mention other projects there
    (VUnit, NVC, GHDL, cocotb, rustc, ...), except in licence notes (user, 2026-10-18). Never write `release_notes/<version>.md` by hand
    outside a release. The user documentation (Sphinx, `docs/conf.py`, `docs/*.rst`, published
    on Read the Docs) shows the fragments under "Unreleased"; build it with
    `sphinx-build -W -b html docs docs/_build/html` after `pip install -r docs/requirements.txt`.

    Each entry gives the decision, a one-line rationale, and who decided it (`user, YYYY-MM-DD`
    for user rulings). Decisions that need the user get a GitHub issue labelled `decision`, and
    are recorded once answered. A decision that exists only in a commit message, an agent report
    or a chat is not recorded.

## 4. Common commands

```sh
cargo test --workspace                               # unit, snapshot and lib/ tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
YAVS_BLESS=1 cargo test -p yavs                      # update rendered snapshots, then review the diff
tools/fetch-corpus.sh                                # external corpora into ~/.cache/rsim-corpus
YAVS_CORPUS=~/.cache/rsim-corpus cargo test -p yavs --features corpus   # parse corpora
tools/corpus/report.sh                               # analyze all corpora, write tools/corpus/RESULTS.md
python3 tools/difftest/difftest.py --candidate nvc   # differential corpus (see tools/difftest/README.md)
cargo test -p yavs-llvm --features llvm              # LLVM tier (needs LLVM 21)
cargo run -p yavs -- analyze --std=2008 --work=work:out/work file.vhd
```

AArch64 under qemu and Windows under Wine: see the commands in `STATUS.md` ("ffi").

## 5. When you finish a task

- Update `STATUS.md` for what you changed, and `docs/decisions.md` for any decision you made.
- Add a release note fragment in `release_notes/news.d/` for every user-visible change (rule 25),
  and update the user documentation in `docs/*.rst` when the change shows there.
- Report: commits pushed, what works now (with test and corpus numbers), what is left, and
  every decision the user should confirm. Say plainly what you did not verify.
