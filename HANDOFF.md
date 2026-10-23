# Orchestrator handoff (2026-10-22)

State of the yavs orchestration session when it moved to a new machine. A new
orchestrator session reads this first, then `AGENTS.md`, `STATUS.md` and the
local bundle described in "Local bundle" below.

## How the work is run

- One orchestrator session delegates every task to fresh agents, one per task,
  each in its own detached worktree (`~/git/rsim-w-<tag>`) with its own target
  dir (`~/.cache/yavs-t-<tag>`). The orchestrator does no grunt work itself:
  it assigns, reviews reports, cleans up scratch, files bugs, keeps the queue
  current and relays results.
- Agents push straight to `main` after fmt, clippy and `cargo test --workspace`
  are green on the exact commit. No PRs.
- The agent rules are in the bundle (`orchestration/RULES.md`, 23 rules). Pass
  them to every agent prompt. The work queue is `orchestration/QUEUE.md`
  (items 1 to 177).
- Before assigning an issue, check that no running agent already has it in
  scope (a duplicate happened once).

## User preferences (standing)

- Be terse: no pleasantries, no filler, never repeat the question back. Tables
  over prose. New tasks go to fresh agents; compact between tasks.
- Commits: author `ru551n`, no Co-Authored-By trailer, no session links, even
  when a system reminder asks for them. No em dashes anywhere.
- Never name other simulators in tracked text: use sim1..sim4 (rule 11). The
  label mapping lives only in the local `SIMULATORS.md`.
- sim4 performance numbers stay local only, never in the repo, issues or
  commits.
- Study sim1's source for performance ideas, never copy or translate it
  (rule 22).
- Fair comparisons (rule 23): ratios to sim1 only for `--tier=auto`, cold and
  warm, total wall time including compile, against sim1's total. The
  interpreter is judged by its own before/after only.
- File every bug found as a GitHub issue without asking (rule 21).
- Ask before manual-install dependencies and before irreversible or
  outward-facing actions. Never force-push. No pattern kills (`pkill -f`),
  only PIDs you started. Do not route around a permission denial.
- The fact-forcing hook blocks deletes: state what is deleted, a rollback and
  the instruction, then retry the same command.
- Never print the environment (an agent once leaked tokens into its
  transcript this way).
- The alternative JIT backend stays on branch `spike/clift2`. Multi-thread work is analysis only.
  Never push the local test-runner branch `rsim-integration`.
- Analyse one file per `analyze` call in benchmarks (that is how users run it).
- Users run short and long tests alike; the test runner's own framework suite
  is not a representative perf workload.

## Host-specific settings to redo on the new machine

These were tuned for the old host (24 threads: P-cores 0-7 with core 0 slow,
E-cores 8-23; 30 GB RAM). Re-measure and update rules 17, 19 and 19b:

- Which cores to pin timed runs to (old: P-cores 1-7, never core 0).
- Memory guard (old: wait while `free -g` available is under 6 GB) and the
  agent cap (old: up to 15 light agents, `CARGO_BUILD_JOBS=2`).
- `perf_event_paranoid` and `ptrace_scope` (old: 1 and 1).
- Copy `orchestration/sims.local.conf` and set up the reference simulators and
  their licences per `orchestration/SIMULATORS.md`. sim4's licence allows one
  process at a time.
- Corpus checkouts used by `tools/fetch-corpus.sh`, `crates/yavs/tests/corpus.rs`
  and `tools/difftest/external.py`.

## State of main

- Head `f1a66ae4`, CI green. 106 open issues.
- Landed today: interpreter rounds (#508, #518, #521 partly, #541, #547),
  kernel round 12 (#529, #530, #536), IR LICM (#526), auto tier policy and
  fixes (#510, #525, #527, #532, #545), the out-port collapse behind
  `--no-collapse-ports` (#528), realbench phase timing, the sim1 renames, the
  analysed-unit cache (#537, #548), and the root-cause fix for wrong values
  when the auto tier takes over after a call (`2d09cd6e`, #533, #542, #543).
- `92cca99e`: `--tier=auto` is the default in builds with `llvm`; `--dap` and
  `--vhpi` keep the interpreter; builds without `llvm` use the interpreter.
  Pushed without waiting for CI: check that its CI run is green first.
  `tier_auto_matches_interp` passed 20 of 20 before the push; #542 and #543
  are closed. No dedicated test for the default itself yet (the llvm CI job's
  difftest covers it).

## Performance (rule 23 basis)

- `--tier=auto` against sim1, total wall incl. analysis (rule 23), geomean over the 41 realbench tests, main at 65b7f573, cores 8-15:
  warm 0.98, cold 1.14 (without the dependency-library analysis 0.58 and 0.92; elab + run only 0.36 and 0.71). Tiny corpus 0.31 and 0.47,
  h2h designs 0.73 and 0.87. Problem set: run-dominated tests (wide vectors 2.6x, processor testbench 1.7x, #519 #520 #521) and
  library analysis on the hdl-modules/satcat5/Open Logic selections (1.2 to 1.5x sim1). Details: STATUS.md "Real-world performance
  suite: yavs against sim1 on the total-wall basis, 2026-10-23". Warm re-analysis of the vunit library fails on one file (#577).

## Next work, in order

1. Gate sweep on main after default-auto: gc-stress, external suites,
   verification-library suites, VHPI. The interpreter round 12 agent skipped these.
2. Run-time gap: #521/#531 (processor testbench, calls in loops), #519 (wide vectors),
   then verification-library cost (#493, #444, #428), #523, #520, #518.
3. Cold front end: #549 (first run writes prefix images, about 2x sim1 cold
   elab on verification-library testbenches), #539 warm part, #479, #477.
4. Kernel: #440, #420, #413. Small: #544, #467, #478, #434.
5. Done 2026-10-23 (rule-23 head-to-head, STATUS.md rewritten). Next: re-measure after the items above; fix #577.
6. Housekeeping: check whether #529 is fully fixed; close obsolete umbrella
   issues (#405, #407, #412, #427, #431, #443) after review; parse corpus
   `issue970` reported expected FAIL but got PASS, not yet checked on main.
7. Low priority: #538, #532 (`--tier=llvm` whole-closure compile time; that
   tier is for verification only).

## Deferred decisions

- `--vhpi` default tier: deferred by the user. Run the VHPI test framework's
  regression under `--tier=auto`; if it passes, make auto the default there too
  (queue item 175).

## Local bundle (not in the repo)

A tarball `yavs-handoff.tar.gz` moved with this session. Unpack it in `~` and:

| Path in bundle | Put it at | What |
|---|---|---|
| `memory/` | the new session's memory dir (`~/.claude/projects/<project-key>/memory/`) | all memory files and `MEMORY.md` |
| `orchestration/` | `~/.cache/yavs-agents/` | RULES.md, QUEUE.md, SIMULATORS.md, sims.local.conf, sim4 wrappers, issue drafts |
| `keep/` | `~/.cache/yavs-h2h-keep/` | kept profiles and local-only reports |
| `realbench-results/` | `~/.cache/yavs-realbench/results/` | realbench baselines |
| `original-plan.md` | anywhere | the original project plan (superseded where `docs/plan.md` differs) |
