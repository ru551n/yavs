# AGENTS.md: yavs (the CLI binary)

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs`.

## 1. Purpose and boundaries

- The `yavs` executable: `parse`, `analyze`, `elab`, `run`, `cov`. Parses options (clap), expands `@file`
  response files, maps libraries, drives the stages in order and stops at the first failed one,
  renders diagnostics, maps outcomes to exit codes, prints reports in the reference simulator's format.
- Does not: analyze, elaborate or simulate itself. All semantics live in the library crates; the
  CLI only wires them. External test runners depend on its behaviour through `docs/cli-contract.md`.
- Depends on: `yavs-diag`, `yavs-syntax`, `yavs-lib`, `yavs-sem`, `yavs-std`, `yavs-elab`, `yavs-ir`,
  `yavs-interp`, `yavs-kernel`, `yavs-wave`, `yavs-cache`, `yavs-dap` (`--dap`), `yavs-cov` (`--cov`, `cov`), `yavs-vhpi` (`--load` and `--vhpi`; its exports are linked into this binary by `build.rs`, #266), `clap`;
  optionally `yavs-llvm` (feature `llvm`) and `yavs-clift` (feature `clift`; it also enables `yavs-llvm` without LLVM for the shared policy code). There are no default features. Nothing depends on this crate.

## 2. Map

| File | Responsibility |
|---|---|
| `src/main.rs` | `Cli`/`Command` (clap), global `--color`, `--message-format`, `--encoding` (`Top`); `parse_hot` (the command line of `analyze`, `elab` and `run` parsed from a command that holds only that subcommand; help, errors and the other subcommands go through `Cli::parse_from`, so their text is the whole command's); `parse` subcommand; `@file` expansion before clap |
| `src/analyze.rs` | `yavs analyze` (`Args`), `expand_response_files`, `Out` (counts errors/warnings, `-Werror`, "aborting due to" line, exit code), batch notes |
| `src/cov.rs` | `yavs cov merge`, `yavs cov report` (`--exclude`, `--uncovered`, `--tests`, `--html`, `-W`; E0110, E0111, E0112, E0113) and `yavs cov rank` |
| `src/gui.rs` | `run --gui`: starts `yavs-gui --yavs <this exe> -- <run args>` (next to the exe, else PATH; E0122), exit status passed through |
| `src/cache.rs` | The compilation cache in front of analysis (`analyze_closure`: key from options and closure files, `yavs_cache::Store` of the work library, `Db::from_image` or analysis from source plus `yavs_elab::load_bodies`, then `settle` and `to_image`, so the image holds the package bodies elaboration needs; `Stages` gives the later stages the same store for their own entries, keyed by the analysis key, `Db::set_stages`: the lowered run-time packages, kind `low`, and for the library prefixes of the closure (`Level`, `StageCache::prefixes`, `prefix_get`/`prefix_put`/`prefix_claim`, `prefix_db`) the entries every closure of the prefix shares, kinds `lib`, `libopt`, `libinit` (`docs/decisions.md` "Library stages"); the image and `low` entries are made on the second analysis of a closure, the first leaves a note, and none at all for a closure under a prefix whose `lib` entry exists (`library_made`; `prefix_release` trims the heap after the entry was made); a closure that misses continues from the longest prefix image `pre` (`bounds`, `prefix_store`; made by the second different closure with that prefix, `PREFIX_SIGHTINGS`, one maker at a time), see `docs/decisions.md` "Prefix images"), `yavs cache clean` (E0123) |
| `src/stdout.rs` | Standard output of `run`: `write` (the `Printer` observer), `buffer` (block-buffered when stdout is not a terminal and no plugin or foreign code is loaded), `flush` (called before every stderr write and the final status lines); the signal thread writes out what waits every 100 ms (`TICK`; one thread also serves Ctrl-C and SIGUSR1, `yavs_wave::interrupt`, #562); the `Printer` is also the kernel's `LiveSink` (reports of serially running processes arrive as made, #483); a failed write stops the run (`on_broken`, exit 141, #484); without `--wave` or `--profile` the first Ctrl-C/SIGTERM flushes and exits 130 |
| `src/jobserver.rs` | The token pool of `MAKEFLAGS` (`connect`, read at the start of `run` with `--threads` above 1 and no `--dap`; `Pool` implements `yavs_kernel::Tokens`; `close` gives the tokens back before an exit: `exit_after_signal`, the broken-pipe exit); E0128 when it cannot be used (#801, `docs/parallel.md`) |
| `src/timing.rs` | `--time-report`: phase marks (`mark`, one atomic load when off), `report` to stderr; CPU from `/proc/self/stat` (10 ms steps, no new dependency) |
| `src/counters.rs` | `YAVS_COUNTERS=1`: the work counters of a run as `counters: NAME=VALUE` lines on stderr (kernel `Stats`, `ops`, call cache, wave variables, `llvm_*`, `cb_*`; `YAVS_COUNTERS=kinds|pc` adds `cb_<entry>.<Kind>` lines, the calls compiled code makes into the host by instruction kind); hidden test aid |
| `src/elab.rs` (`YAVS_LEAN_EVENTS`) | Hidden test aid: `YAVS_LEAN_EVENTS=off` keeps the event history of every wide signal (`Elaborated::lean` cleared); otherwise a signal of more than 64 elements that no `'last_event`/`'last_value` can ask the history of single elements of keeps only each block's last event (#519), and every signal keeps its history when coverage, a debugger or a VHPI plugin observes the run |
| `src/elab.rs` (`YAVS_CELLS`) | Hidden test aid: `YAVS_CELLS=off` makes a wide signal that processes wait on parts of list the exact runs of its changed elements (`Config::cells` false); otherwise it lists the changes per cell between the waited ranges (#519); value-change observers get exact runs either way |
| `src/elab.rs` (`YAVS_POOL_ALL`) | Hidden test aid: `YAVS_POOL_ALL=1` with `--threads` above 1 hands every batch of two or more processes to the thread pool, except at initialisation and postponed ones (`Config::pool_all`) instead of choosing by estimated work, so the differential corpus exercises the pooled path (#497) |
| `src/elab.rs` (`YAVS_IDLE_SKIP`) | Hidden test aid: `YAVS_IDLE_SKIP=off` runs every activation of a clocked process, `check` runs the ones the kernel would skip and fails when one does something else (`Config::idle_skip`, `idle_check`); unset or `on` skips |
| `src/profile.rs` | `run --profile[=FILE]`: `Sampler` (a thread at 1 kHz tallying `Profile::sample`, `Runtime::sampler` and `Runtime::line_sampler`), the stderr tables and the JSON file; `watch_interpreter` installs a tier with no compiled code so that a `--tier=interp` run names its subprograms and lines; the JSON file write error is E0124 |
| `src/explain.rs` | `--explain` table (all codes but those of `yavs-sem`) and the every-code-has-a-text test |
| `src/elab.rs` | `yavs elab` (`Common`) and `yavs run` (`RunArgs`): closure from manifests, design revision, `Db::analyze_closure`, `yavs_elab::find_top`/`elaborate`/`lower`/`build`, kernel `Config`, `Printer` observer, `--stop-time` parsing, end-of-run exit code, `bind_foreign` (`--load`, E0106/E0107), `use_tier` (picks `use_llvm_tier`, `use_clift` or `use_auto3` by `RunArgs::tier()`; `--tier-max-size`, `--verbose`, E0120/E0121/E0127; `StopTierUp` ends tier-up when `run` returns) |
| `tests/cli.rs` | Exit code and stderr snapshots for `analyze`/`elab`/`run` (`tests/cli/`), library locking, concurrent batches |
| `tests/cache_prefix.rs` | The library stages: testbenches of one library share its lowered form, package state and optimised functions and print what a run without the cache prints (sessions or not, one thread and four); editing a package of the prefix; a body that follows the prefix is not reused (and a changed body is seen); parallel first runs; damaged entries |
| `tests/units.rs` | The analysed-unit cache: cached analysis reports what source does (and `YAVS_UNIT_CHECK`), an edited dependency invalidates its dependents, a file analysed again is verified without loading its units (only when its closure reports nothing and the manifest record is the entry's, #581), damaged entries, `run` never writes entries, read-only libraries |
| `tests/std_share.rs` | The shared std/ieee units (`YAVS_STD_CACHE`): a fresh library reads them, six concurrent cold analyses agree, damaged entries are rebuilt, an unusable store is `E0126`, a user's own `ieee` is never shared |
| `tests/cache.rs` | The compilation cache: warm run identical to cold (also `--threads 4`, `-Werror`), editing a dependency invalidates, damaged entries, read-only library, `cache clean`, parallel cold runs; the same for the lowered packages (`low` entries: waveform bytes, edit, damaged entry, parallel runs; the same for `init` entries, the package state after initialisation) |
| `tests/jobserver.rs` | Token pool (Linux), both forms (`fifo:PATH` and inherited `R,W`): tokens taken at most `--threads` - 1, all back after the end, a failure and Ctrl-C (with and without `--wave`), an empty pool pools no batch, stdout identical at 0, 1, 3, 8 tokens and without a pool; closed descriptors, a path that is no pipe and an unknown form are E0128 (exit 2, before analysis); `--threads=1` and `--dap` do not read `MAKEFLAGS` |
| `tests/perf.rs` | Work-counter tests: designs in `tests/perf/` against ceilings and floors of counters that do not depend on the host (root `AGENTS.md` section 4, "Work-counter tests") |
| `tests/elabcache.rs` | The elaboration-time call cache (`ecall`): warm run prints what a cold one does (reports replayed), an edited package constant or body never gives an old result, an impure function is run every time |
| `tests/coverage.rs` | `run --cov` counts of `tests/cli/cov_run.vhd` and toggles of `tests/cli/cov_toggle.vhd` (`--cov-kind`, W0115), `--threads 4` identity, `cov merge`, per-test attribution (`--cov-test`, default names, `cov rank`, `--tests`), `cov report` and Cobertura, E0111, exclusions (pragmas, report rows round trip, moved/stale/hit warnings) |
| `tests/foreign.rs` | `--load`: a C library built at test time (`cc`, target from `build.rs`), calls, copy-back, E0106/E0107 |
| `tests/diag.rs` | Parse diagnostic snapshots (`tests/diag/*.vhd` vs `.stderr`, `-- yavs: ARGS` header) |
| `tests/lib_parse.rs` | Every file in `lib/` parses (2008 and 2019) |
| `tests/dap.rs` | `--dap`: scripted DAP sessions over stdio (`tests/dap/*.script` and `.expected`), pause, TCP |
| `tests/corpus.rs` | External parse corpora vs `tests/parse-corpus/` lists (`--features corpus`, `YAVS_CORPUS`) |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Stop at the first failed stage; never exit 0 after an error | Error motto | `docs/plan.md` "Error-handling motto", `docs/cli-contract.md` 0 |
| Exit codes: analyze 0/1 (file error)/2 (usage)/3 (ICE); run 0, 1 (failure, report at/above `--exit-severity`, error reported), 2 (usage, analysis or elab error), 3 (internal) | The test runner reads only zero/non-zero, humans need the class | `docs/cli-contract.md` 4, 5 |
| `--exit-severity` default `failure`; without the option an `error` report still makes the run exit 1 at the end, unless `ClearVhdlAssert` cleared the std.env assert counts (#191); given explicitly, only reports at or above the level count (#498) | Like the reference simulators (user, 2026-10-18, 2026-10-21) | `docs/decisions.md` "Runtime: default exit severity" |
| `std`/`ieee` are provided by yavs; mapping them is `E0102` (except `analyze --bootstrap`) | One std/ieee per design | `docs/cli-contract.md` 3 |
| Design revision = newest standard in the closure unless `--std` is given | Mixed 2008/2019 projects (user, 2026-10-18) | `docs/plan.md` "Test-runner integration", `docs/design/sem.md` 0.1 |
| `--threads` defaults to 1 | Each yavs under a test runner's parallel option (`-p N`) | `docs/plan.md` "Test-runner integration" |
| `@file` response files from the start | Test runners on Windows | `docs/plan.md` "Test-runner integration" |
| `--encoding=auto` default | User decision | `docs/decisions.md` "Parser", source encoding |
| Reports on stdout as `** Note: <fs>fs+<delta>: msg` then `    Process <path>` | tools/difftest parses the reference simulator's format | `docs/decisions.md` "Runtime: default exit severity", `docs/cli-contract.md` 5 |
| Runtime errors are diagnostics on stderr (`E3001`-`E3010`, statement span, `called from here` per frame, time/delta/process note) via `Elaborated::runtime_diagnostic` | Same format as every other error | `docs/cli-contract.md` 5 |
| A runtime error while package or instance data is initialised (`Elaborated::build`) is exit 2 with the runtime diagnostic; an internal one is exit 3 | It is elaboration, and test runner users need the VHDL location | `docs/cli-contract.md` 5 |
| Foreign symbols are bound after lowering, before `build`: `--load` libraries in order, then the executable; missing is warning W0108, E0107 only at the call (user decision 2026-10-18) | Warn early, fail at call | `docs/cli-contract.md` 5, `docs/decisions.md` "Foreign subprograms and `--load`" |
| The executable is a position independent executable on every platform, `-no-pie` is not linked (user decision 2026-10-21, reverting #426) | Hardening and packaging; the about 7% of a short run that `-no-pie` saved is given up (`docs/decisions.md` "Floor of a run: symbol table size, pruning on demand, executable layout") | `build.rs` |
| `--profile` sets an installed `Profile` (every process is wrapped once at kernel build: one relaxed store before and after `Process::run`), starts the sampler just before `run_until` and prints after it; stdout and the exit status are unchanged; `--dap` with it is `E0125`; the first Ctrl-C/SIGTERM stops the run between cycles and the profile is printed, exit 130 (a second signal exits at once) (#587); with `--tier=interp` a tier without compiled code is installed so the interpreter publishes its function (about the cost of the `--tier=auto` watch loop); processes and subprograms are separate tables (paired in the JSON only when one thread runs one process), a subprogram inlined by the optimiser counts as its caller; lines are the earliest statement of the unoptimised function, not a statement profile (#580) | Free when off, no new hot-path code; a statement-level profile would need a store in the interpreter loop | `docs/cli-contract.md` 5 |
| `--cov` writes the database after the run whatever its outcome (none after an elaboration failure); a write failure makes the exit status at least 1 | Coverage of failing tests is still wanted | `docs/cli-contract.md` 5, `docs/decisions.md` "Code coverage" |
| Unimplemented flags and combinations (`--dap` with `--wave`, `--vhpi` or `--cov`) are `E9999` "not implemented yet" naming the milestone, exit 2 | No silent ignore | root `AGENTS.md` rule 1 |
| `--dap` hands the elaborated design to `yavs_dap::serve` after `--load` binding and the resolution check; `--threads` above 1 is warning `W0109` | Stops need one thread | `docs/cli-contract.md` 5, `docs/decisions.md` "DAP server" |
| `yavs run` writes reports and `OUTPUT` text in blocks (64 KiB, or every 100 ms) when stdout is a pipe or file and no `--vhpi`/`--load`/foreign subprogram is present; a terminal or any foreign code keeps line-by-line writes; every stderr write flushes first | A write per `report` line was a system call each (4x the CPU of a report-heavy run into a pipe); the order against stderr and against C code stays as before (#428) | `docs/cli-contract.md` 5, `src/stdout.rs` |
| `--time-report` marks phases with `timing::mark` at fixed places (after analysis, after lowering, before `run_until`, after it) and prints at the end of `run`; CPU time is read from `/proc` (10 ms steps) because a safe cross-platform CPU clock needs a new dependency | Zero cost off; one shared benchmark harness (`tools/bench`) needs phase times (#387) | `docs/cli-contract.md` 5 |
| `--tier` default `auto` in a build with a compiled tier (feature `llvm` or `clift`) unless `--dap` is given, else `interp` with no message (`RunArgs::tier()`; a build with `clift` and without `llvm` turns `auto` into `clift-auto`, user, 2026-10-24; an explicit tier whose feature is missing is E0120, `auto3` needs both features; user, 2026-10-22; it was `interp` from the predecoded interpreter, #431, until then). Under `--dap` no compiled tier is ever installed, whatever `--tier` says: the DAP branch of `run` returns before `use_tier` but first calls `use_tier(None, ..)`, which checks the features only, so a missing one is E0120 there as elsewhere (#835); `--tier=llvm` sets the LLVM tier on the runtime after package initialisation, `--tier=auto` sets up tier-up there (feature `llvm`; with `clift` alone it is `clift-auto`; neither: `E0120`) and the signal thread starts its compile thread 5 ms into the run (`StopTierUp::launcher`, `Handlers::after`; no compile thread in a shorter run, #426); `--tier-max-size` (default 50000, user 2026-10-19) for both; hidden `--tier-heat` (the `Heat` threshold of `auto`, for tests); at the end `StopTierUp::end` exits at once (after `Runtime::close_files`) while a function compiles, unless `--load`/`--vhpi` libraries need their exit handlers; `--verbose` notes go to stderr | Default build needs no LLVM; elaboration stays interpreted | `docs/cli-contract.md` 5, `docs/decisions.md` "LLVM tier: side exits" |
| `run --gui` only starts the `yavs-gui` process (never links it) on the run arguments minus `run` and `--gui`; it conflicts with `--dap` | The GUI is EUPL-1.2 and drives `yavs run --dap=stdio` itself (user, 2026-10-19) | `docs/cli-contract.md` 6, `docs/decisions.md` "GUI (M10)" |
| `elab`/`run` read the analysis of the closure from `<work dir>/yavs-cache/` when the key and the source bytes match, else analyze and store it (error-free only); `--no-cache` skips both; `analyze` never touches it; custom `$YAVS_STDLIB` disables it. The analysed units (`cache.rs` `Units`, `yavs-cache/units/` of each library; the built-in std/ieee in the store shared by all libraries, `shared_std_dir`, `YAVS_STD_CACHE`, #564) are written by `analyze` only and read by all three; `analyze --no-cache` skips them; `YAVS_UNIT_CHECK=1` makes `analyze` compare with an analysis without them | Short test runs spent most of their CPU analyzing (#405); the cache is an optimisation that can only miss (user, 2026-10-19) | `docs/decisions.md` "Compilation cache", `docs/cli-contract.md` 3 |
| `yavs analyze` holds one exclusive library session for the whole batch; elab/run only read | Rule 14 locking | `docs/plan.md` rule 14, `docs/cli-contract.md` 3 |
| `analyze` records each file with `Session::record_deferred` and writes the index once with `Session::flush` after the loop, also after a failing file and a panic (the files before it stay recorded) | The index was rewritten whole after every file: quadratic in the files of a call and a third of the cold cost of a 250-file call (#747) | `docs/decisions.md` "Batched analysis: shared work paid once", `docs/cli-contract.md` 4 |

Module-local decisions go in this section in the same commit; CLI or test-runner-facing behaviour goes
into `docs/cli-contract.md` (root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- `docs/cli-contract.md` is a contract with external test runners. Change it and
  the code together, deliberately; `tests/cli.rs` pins the observable part.
- A panic in `analyze` is caught and reported as an `E9999` ICE with the file name (exit 3); `elab`
  and `run` are wrapped by `guard` in `main.rs` (no file name). The panic hook is silenced in `main`.
- `-W` forms (`Out::set_warn` in `analyze.rs`): `error`, `error=CODE`, `no-CODE`, `CODE`; the last
  option for a code wins, an unknown or non-warning code is `E0103`. Warnings are filtered in
  `Out::emit`, so every stage that goes through `Out` obeys them.
- `yavs --explain CODE` is `src/explain.rs`: its table covers every code except `yavs-sem`'s
  (`yavs_sem::diag::EXPLAIN`). A test scans all `crates/*/src` for code literals, so a new code
  fails `cargo test -p yavs` until it has a text (#136).
- `--ieee-warnings=on|off`: `off` calls `yavs_interp::suppress_ieee_warnings` with the `ieee` tree of the standard library before elaboration; the interpreter drops warning reports of natives and of statements in files under it (#836).
- std/ieee come from `yavs_sem::default_stdlib()`: `$YAVS_STDLIB`, else the copy built into the
  binary (test `standalone_binary` in `tests/cli.rs` runs a lone copy of the executable).
- Tests run the built binary (`env!("CARGO_BIN_EXE_yavs")`); scratch dirs come from
  `tests/common` `tempdir` (system temp dir, removed on drop, panics included). Never a fixed or pid-named
  directory that a failing test would leave behind.

## 5. How to work here

```sh
CARGO_BUILD_JOBS=6 cargo test -p yavs                    # cli, diag snapshots, lib/ parse
YAVS_BLESS=1 cargo test -p yavs                          # rewrite tests/diag and tests/cli expected files; review the diff
tools/fetch-corpus.sh && YAVS_CORPUS=~/.cache/rsim-corpus cargo test -p yavs --features corpus
cargo run -p yavs -- analyze --std=2008 --work=work:$HOME/.cache/<dir>/work f.vhd
cargo run -p yavs -- run --work=work:$HOME/.cache/<dir>/work top --threads 4
```

- New CLI behaviour: add a `Case` to `tests/cli.rs` with its source in `tests/cli/` and bless the
  `.stderr`. A parse diagnostic: add `tests/diag/NAME.vhd` and bless.
- Env vars: `YAVS_STDLIB` (std/ieee tree), `YAVS_STD_CACHE` (shared std/ieee unit store or `off`; the flag `--std-cache=DIR|off` of `analyze`/`elab`/`run`/`cache clean` wins; `cargo test` sets it to `target/yavs-std-cache` through `.cargo/config.toml`), `YAVS_BLESS=1`, `YAVS_CORPUS`, `NO_COLOR`.
- Codes owned: `E0100`-`E0199`/`W0100`-`W0199` (in use: E0100 unreadable file, E0101 response
  file, E0102 std/ieee mapped, E0103 `-W` form, E0104 no top level, E0105 bad `--stop-time`, E0106 `--load` library
  cannot be loaded, E0107 foreign subprogram cannot be bound or called, W0108 foreign symbol not bound, W0109
  `--threads` ignored with `--dap`, E0110 coverage file cannot be written, E0111 coverage database
  cannot be read, E0112 exclusion file, W0110-W0114 exclusions and pragmas, W0115 signal over
  `--cov-toggle-max`, W0116 signal handlers cannot be installed, W0117 the compile thread of `--tier=auto` cannot be started, E0120 a `--tier` whose cargo feature (`llvm`, `clift` or both) is not built in, E0121 a compiled tier cannot
  compile the design, E0122 `yavs-gui` not found or not startable, E0123 `yavs cache clean` cannot remove an entry, E0124 `--profile=FILE` cannot be written, E0125 `--profile` with `--dap`, E0126 the shared std/ieee cache directory unusable, E0127 the LLVM library cannot be loaded, E0128 the token pool in `MAKEFLAGS` cannot be used).

## 6. Coordination

- The difftest (`tools/difftest`) and the corpus report (`tools/corpus/report.sh`) drive this
  binary; output format changes break their parsers.
- Open issues (snapshot 2026-10-18, `gh issue list -R ru551n/yavs --label crate:cli`): #64 wire
  lowering, interpreter and kernel into `yavs run`; #110 test runner support;
  #135 native Windows build; #22 bootstrap mode.
