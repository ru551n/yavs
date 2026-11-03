# AGENTS.md: tools/pgo/ (profile-guided release build)

Read the root `AGENTS.md` and `tools/AGENTS.md` first. This file is for an agent that changes the training set or the build
script, or measures a profile-guided binary.

## 1. Purpose and boundaries

- `build.sh OUT` builds yavs twice with rustc's own profile flags and trains in between: an instrumented build
  (`-C profile-generate`), the training runs (`train.py`), `llvm-profdata merge`, and the final build (`-C profile-use`). No
  crate changes; the profile is regenerated in every build, never checked in (a checked-in profile goes stale with every source
  change). `llvm-profdata` comes from rustup's `llvm-tools` component (`rustup component add llvm-tools`, rustc's own LLVM, so the
  profile format matches); the yavs LLVM tier is not involved.
- `train.py` runs the cases in `train/*.vhd` with the instrumented binary and fails on any run that does not exit 0.
- Everyday perf comparisons (root rule on bench builds, `tools/build-bench.sh`, `tools/bench`, `tools/realbench`) stay on
  non-profile-guided builds. Say which kind of build a number is from. A profile-guided build moves every baseline, and a change
  to the runtime shifts the profile it was trained with.
- The release workflow does not run this script (maintainer's decision, `docs/decisions.md` "Profile-guided builds").

## 2. Map

| Path | What |
|---|---|
| `build.sh OUT` | The four steps above; env `RUSTFLAGS` (shared by both builds), `PGO_DIR`, `PGO_BUILD`, `PGO_BIN_REL`, `PGO_JOBS`; prints the time of each step and the sha256 of the merged profile; fails when the compiler complains about the profile |
| `train.py BIN --profile-dir DIR` | The training driver (stdlib Python, runs on every platform the build runs on) |
| `train/*.vhd` | The training cases: original, self-checking testbenches, header `-- pgo: top=NAME_tb std=2008 stop=TIME [relaxed] [weight=N] [gen=NAME=VALUE]` |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| The training set is original VHDL in the repository, written from a description of what the real designs do at run time, never copied from them | A public release workflow must be able to run it: one of the designs measured is in a private repository, the processor core needs a POWER cross compiler for its program image, and a profile trained on GPL or share-alike sources is a licence question for the maintainer | user, 2026-11-03 |
| Each case runs analysis cold and again, `elab` cold and warm, `run` at `auto` (cold cache, then warm), eagerly compiled (`--tier=clift`, `--tier-heat=0,0,0`), 4 threads, the interpreter, and a waveform | Small tests are mostly start-up (analysis, elaboration, cache load), and the compiled-code runtime is only trained when functions are compiled; a training that misses a path treats it as cold and loses on it | measured, `docs/STATUS.md` "Profile-guided build" |
| The first case runs alone against an empty std/ieee store, the rest in parallel | The std/ieee analysis is trained once, like the first run of an installed tool | |
| Runs of one instrumented binary write one merged `.profraw` (`%m`) | One file per process is 14 MB each | |
| Held-out judgement is on designs the training never saw, worst per class, instructions (exact) and cycles (indicative on a shared host) | The training must not be judged on itself | user, 2026-11-03 |
| `rtl_bus` and the other `tools/bench/designs` are never training cases | They are the designs where yavs leads: a held-out check | user, 2026-11-03 |

## 4. Invariants and pitfalls

- Training cases name no project, simulator or tool (root rule 25), have no em dashes, finish before their `stop` time, and print
  `PASS` (a `report` at note severity) after every check.
- The instrumented and the final build must use the same `RUSTFLAGS` apart from the profile flag, and the same source: a profile
  of another commit makes the compiler warn about mismatches (`build.sh` stops on that).
- A run killed by a timeout or a crash writes no profile: `train.py` stops on any non-zero exit, so a silent loss is not possible.
- Never time an instrumented binary; never compare a profile-guided binary with a plain release build of another flag set (use the
  same `RUSTFLAGS`, and for bench comparisons `PGO_BUILD='tools/build-bench.sh --features clift'` against `tools/build-bench.sh`).

## 5. How to work here

```sh
rustup component add llvm-tools
tools/pgo/build.sh target/yavs-pgo                                  # release flavour
RUSTFLAGS='-C target-cpu=x86-64-v3' tools/pgo/build.sh target/yavs-pgo-v3
PGO_BUILD='tools/build-bench.sh --features clift' PGO_DIR=~/.cache/yavs-t-pgo tools/pgo/build.sh ~/.cache/yavs-pgo-bench   # to benchmark
python3 -I tools/pgo/train.py INSTRUMENTED --profile-dir DIR --only rec_pipe                         # one case
```

Where a run is not covered: `llvm-profdata show --all-functions --counts merged.profdata` lists the counters of every function;
a function with a large count under the real designs and none under the cases is a path to write a case for.

## 6. Other targets (not built or run here)

- **Linux AArch64:** the same script on a native AArch64 runner (the instrumented binary has to run; the toolchain has the profiler runtime for the target, and `llvm-profdata` is the one of that host's toolchain). The profile is per target and never reused across targets. Not run: this host has no emulator or cross linker.
- **Windows (MSVC):** the script is bash and Python only, and handles `.exe`; it needs `llvm-tools` for the MSVC host and a bash on the runner (the release workflow already uses `shell: bash`). Not run, and the profiler runtime for the MSVC target was not checked.
- **A release with an old glibc:** instrumented build with plain cargo (it runs on the runner), final build with cargo-zigbuild and `--target x86_64-unknown-linux-gnu.2.17` through `PGO_FINAL_BUILD` and `PGO_FINAL_BIN_REL`. Checked here: a zigbuild final build with a profile from a native instrumented build gives no profile warning, needs glibc 2.17 at most and runs.
