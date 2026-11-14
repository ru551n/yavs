# AGENTS.md: tools/pgo/ (profile-guided build of the release archives)

Read the root `AGENTS.md` and `tools/AGENTS.md` first. This file is for an agent that changes the training set or the build
script, or measures a profile-guided binary.

## 1. Purpose and boundaries

- `build.sh OUT` builds yavs twice with rustc's own profile flags and trains in between: an instrumented build
  (`-C profile-generate`), the training runs (`train.py`), `llvm-profdata merge`, and the final build (`-C profile-use`). No
  crate changes; the profile is regenerated in every build, never checked in (a checked-in profile goes stale with every source
  change). `llvm-profdata` comes from rustup's `llvm-tools` component (`rustup component add llvm-tools`, rustc's own LLVM, so the
  profile format matches).
- `train.py` runs the cases in `train/*.vhd` and then every `tests/corpus` testbench (analysed, run at `auto` and with every function compiled) with the instrumented binary and fails on any run that does not exit 0.
- Perf comparisons of runtime changes run on profile-guided bench builds (`tools/build-bench.sh --pgo OUT`, after an A/A of
  two such builds), those of generated-code changes on plain bench builds by exact counts (maintainer, 2026-11-06;
  `tools/bench/AGENTS.md` section 7). Say which kind of build a number is from. A profile-guided build moves every baseline,
  and a change to the runtime shifts the profile it was trained with.
- Both builds use `[profile.release]`: fat LTO and one codegen unit since 2026-11-12 (`docs/decisions.md` "Release profile"), so each build ends in one long optimisation step of the whole program.
- `.github/workflows/release.yml` runs this script for every archive (maintainer, 2026-11-03; `docs/decisions.md` "Profile-guided release archives"). `gain.py` is run with `pgo_gain=true` on a dispatch, never on a tag.

## 2. Map

| Path | What |
|---|---|
| `build.sh OUT` | The four steps above; env `RUSTFLAGS` (shared by both builds), `PGO_DIR`, `PGO_BUILD`, `PGO_BIN_REL`, `PGO_INSTR_RUSTFLAGS` (the instrumented build only), `PGO_JOBS`; prints the time of each step and the sha256 of the merged profile |
| `check.py LOG SHOW INSTR FINAL`, `--selftest` | The final build used the profile: no mismatch warning, no function missing from the profile, a profile of more than a thousand functions, the profile runtime in the instrumented binary and not in the final one. Called by `build.sh` |
| `train.py BIN --profile-dir DIR`, `--check` | The training driver (stdlib Python, runs on every platform the build runs on); `--check` runs each case once on any build, in the `corpus` job of `ci.yml` |
| `gain.py PLAIN PGO`, `--selftest` | Profile-guided against plain on `tools/bench/designs`: paired wall-clock runs with a plain/plain control, instructions where `perf` has a counter |
| `train/*.vhd` | The training cases: original, self-checking testbenches, header `-- pgo: top=NAME_tb std=2008 stop=TIME [relaxed] [weight=N] [analyze=N] [gen=NAME=VALUE]` |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| The training set is original VHDL in the repository, written from a description of what the real designs do at run time, never copied from them | A public release workflow must be able to run it: one of the designs measured is in a private repository, the processor core needs a POWER cross compiler for its program image, and a profile trained on GPL or share-alike sources is a licence question for the maintainer | user, 2026-11-03 |
| Each case runs analysis cold and again, `elab` cold and warm, `run` at `auto` (cold cache, then warm), eagerly compiled (`--tier=clift`, `--tier-heat=0,0,0`), 4 threads, the interpreter, and a waveform | Small tests are mostly start-up (analysis, elaboration, cache load), and the compiled-code runtime is only trained when functions are compiled; a training that misses a path treats it as cold and loses on it | measured, `docs/STATUS.md` "Profile-guided build" |
| The first case runs alone against an empty std/ieee store, the rest in parallel | The std/ieee analysis is trained once, like the first run of an installed tool | |
| Runs of one instrumented binary write one merged `.profraw` (`%m`) | One file per process is 14 MB each | |
| Held-out judgement is on designs the training never saw, worst per class, instructions (exact) and cycles (indicative on a shared host). A wall figure counts only when repeated runs agree | The training must not be judged on itself | user, 2026-11-03 |
| `rtl_bus` and the other `tools/bench/designs` are never training cases | They are the designs where yavs leads: a held-out check | user, 2026-11-03 |
| Every archive trains on its own runner with its own `RUSTFLAGS` | v3 and baseline are different code, and a profile is per target | user, 2026-11-03 |
| Both builds run one command; Linux adds `-C linker=cc` to the instrumented build through `PGO_INSTR_RUSTFLAGS` | Section 4. Linking the instrumented build with zig fails (`__llvm_profile_runtime: unrecognized file extension`). With the extra flag `-C metadata` stays equal and the later `-C linker` is the one used | measured, 2026-11-03 |
| No function may be missing from the profile, no threshold | A build that fits has 0 such warnings in all five release jobs, a build that does not had tens of thousands | measured, 2026-11-03 |
| The release build job has no rust-cache | Cargo does not see a profile's content, so a cached dependency would keep an older profile; `build.sh` deletes its final target directory for the same reason | |

## 4. Invariants and pitfalls

- Training cases name no project, simulator or tool (root rule 25), have no em dashes, finish before their `stop` time, and print
  `PASS` (a `report` at note severity) after every check.
- The instrumented and the final build must use the same `RUSTFLAGS` apart from the profile flag, and the same source: a profile
  of another commit makes the compiler warn about mismatches (`check.py` stops on that).
- **A profile that matches nothing is silent.** Profile names carry the crate's `-C metadata` hash, and cargo mixes `--target` and a
  configured linker into it. A plain `cargo build` against a `--target` build, or `cargo build --target T` against `cargo zigbuild --target T.2.17`
  (it sets the linker), shares no name. Without `-pgo-warn-missing-function` the compiler says nothing, and the result is **slower than a
  plain build**: measured 3 to 5 percent in runs on a Linux x86_64 runner. The earlier check of a zigbuild final build (no warning, so it
  seemed fine) was this case. `check.py` counts those warnings for that reason. Measured: adding flags to `RUSTFLAGS` leaves `-C metadata` as it is.
- **A path the training barely ran can lose a lot, and why is not established.** On `elab_heavy` (one vector signal with an out port on each of
  20000 elements) the loop that counts overlapping sources in `collapse::ports` ran scalar in the profile-guided binary and vectorised in the plain
  one: 2.3 times the instructions on x86-64-v3, 1.6 on the baseline. Measured to change nothing: `-pgso=false`,
  `-loop-vectorize-with-block-frequency=false`, `-vectorizer-min-trip-count=2`. Vectoriser remarks did not name a reason for that loop. A low
  trip count in the profile is a guess, not a finding. Measured to cure it: `train/element_ports.vhd`. That is one case for one failure, so the same
  loss may exist on paths no case runs. Look for it with held-out instruction counts and do not add a case per failure: find the cause first.
- `elab_heavy` is not held out any more: `element_ports` was written after reading the first 20 lines of it and profiling it, so `gain.py` leaves it out.

## 5. How to work here

```sh
rustup component add llvm-tools
tools/pgo/build.sh target/yavs-pgo                                  # release flavour
RUSTFLAGS='-C target-cpu=x86-64-v3' tools/pgo/build.sh target/yavs-pgo-v3
CARGO_TARGET_DIR=~/.cache/yavs-t-pgo tools/build-bench.sh --pgo ~/.cache/yavs-pgo-bench --features clift --locked   # to benchmark (PGO_DIR defaults to $CARGO_TARGET_DIR/pgo)
python3 -I tools/pgo/train.py INSTRUMENTED --profile-dir DIR --only rec_pipe                         # one case
python3 -I tools/pgo/train.py target/release/yavs --check --jobs 4                                   # every case passes, on any build
python3 -I tools/pgo/gain.py PLAIN PGO                                                               # the gain, on held-out designs
```

Where a run is not covered: `llvm-profdata show --all-functions --counts merged.profdata` lists the counters of every function;
a function with a large count under the real designs and none under the cases is a path no case runs.

## 6. Targets

All three run in `release.yml`: Linux x86_64 and aarch64 through cargo-zigbuild against glibc 2.17, Windows x86_64 through plain cargo with `--target` (Git Bash: `build.sh` turns its paths with `cygpath -m`, because the native rustc cannot read an MSYS path). `llvm-profdata` is the one of the host's toolchain. The profile is per target and never reused across targets. Other targets are untried. The default command of `build.sh` (no `PGO_BUILD`) has been run on Linux x86_64 only; the other two run it through the release workflow's `PGO_BUILD`.

`gain.py` on a GitHub runner: the Linux x86_64 and Windows runners have no instruction counter, so their figures are paired timings; the Linux aarch64 runner counts instructions.
