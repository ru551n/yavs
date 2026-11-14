# AGENTS.md: python/vunit-yavs/ (the yavs simulator package for VUnit)

Read the root `AGENTS.md` first. This file is for an agent about to change the Python package
`vunit-yavs`.

## 1. Purpose and boundaries

- Everything yavs-specific on the test runner's Python side, in one distribution `vunit-yavs`
  (import name `vunit_yavs`): the simulator interface, `--yavs-debug`, `--yavs-max-threads` and
  `yavs.max_threads`, coverage (`enable_coverage`, `--yavs-cov*`, `merge_coverage`), `--yavs-lint` (`compile_project`), the token server, the version check, `vunit_pkg.toml`. Nothing of it goes into
  the test runner itself or into a crate.
- It drives the CLI contract (`docs/cli-contract.md`); a change of the contract that it relies on
  changes this package in the same commit.
- Not here: the DAP server (`crates/yavs-dap`), the token client inside yavs (`crates/yavs/src/tokens.rs`).

## 2. Map

| Path | What |
|---|---|
| `vunit_yavs/__init__.py` | `YavsInterface` (the `SimulatorInterface`), its arguments, `CoverageArgs` and `merge_coverage`, the native library hooks, `check_debug_args`, `write_response_file`, `YavsVersion` |
| `vunit_yavs/console.py` | The console hook of yavs (`Hook`) and the `vunit_*` commands (`VunitCommands`); `source_map_flags` in `__init__.py` gives the preprocessed files' `--source-map` |
| `vunit_yavs/tokens.py` | `lend_order` (the lending priority), `Client` (one test: secret, cap), `Connection`, `TokenServer` (127.0.0.1 listener, selector thread, accounting) |
| `vunit_yavs/vunit_pkg.toml` | `simulator = "vunit_yavs:YavsInterface"`, shipped as package data |
| `tests/test_interface.py` | Unit tests of the interface, with the package registered in a fresh simulator factory |
| `tests/test_interface.py` `TestCompileSchedule` | The compile schedule on VUnit's real `Project`, a recording `check_output`: dependency order under `-j`, one call per library at a time, batches, failures, `--keep-compiling`, incremental, the name limit; runs on Windows too |
| `tests/test_tokens.py` | Lending order, the protocol over real sockets: answers per message, caps, tokens back on close, kill and test end, bad secrets and lines |
| `tests/test_fake_e2e.py` | `tests/project/run.py` end to end against `tests/fake_bin/yavs` |
| `tests/test_docs.py` | `docs/vunit-yavs.rst` names exactly the options of the package; the debugger tutorial (`docs/debugger-tutorial.rst`) names files and lines that hold the text it says, and its `launch.json` matches |
| `tests/test_real_yavs.py` | The run scripts of the user guide (`docs/examples/vunit`, and the debugger tutorial's `docs/examples/fifo-debug`), and the same project on a real yavs (`VUNIT_YAVS_PATH` or `PATH`, skipped otherwise), plain, with the token server, a real yavs on a lent token, and under `--yavs-debug` with a minimal DAP client; `tests/project/run_vendor.py` on the vendor libraries of a real install, added with `add_library` and `add_source_files` (skipped without `XILINX_VIVADO`) |
| `README.md`, `CHANGELOG.md`, `RELEASING.md` | User guide, changes per version, a pointer to the release steps (root `RELEASING.md`: one release with yavs, one version) |

## 3. Design decisions that bind this module

| Decision | Why |
|---|---|
| A separate distribution in the yavs repository, MPL-2.0 (the interface was written for the test runner under its licence; the file headers keep it) | One place for the Python side; licence of the origin kept (user to confirm) |
| Package arguments carry a `yavs-` prefix (`--yavs-debug`, not `--debug`); the one exception is `-j`/`--jobs` (user, 2026-11-14), checked free in VUnit 5.0.0.dev16 and its master of 2026-11-06 (`test_jobs_argument`) | The test runner parses its argv once before packages are added, with prefix matching: `--debug` was taken as another simulator's `--debugger` and failed |
| `--yavs-debug[=PORT]`, default 4711, `0` a free port; the run's address is printed to the terminal | 4711 is the editor extension's attach port; the test runner shows a test's output only on failure |
| A wrong command line or setup stops through `refuse`: `error: ...` on stderr and status 1, with no traceback; VUnit's main turns the exit into status 1 and prints any other exception with its traceback. A refusal inside `simulate` prints the same and returns False, so the test fails and the others report | Cause and fix are all a user needs (#998) |
| `--yavs-debug` is one simulation: refused with `--elaborate`, `--yavs-cov` or `-p` above 1 in `from_args`, and a second simulation fails its test before yavs starts | The interface sees no test list before the run; the second simulation is the earliest point |
| `yavs.max_threads` (sim option) over `--yavs-max-threads` (default) to `--threads=N`, before the raw flags; none under `--yavs-debug` or for `elab` | Raw flags stay the last word; a DAP run is one thread |
| `yavs.tier_compile` (sim option, `"wait"` or `"background"`) to `run --tier-compile=MODE` after the thread count and before the raw flags; nothing when unset (yavs's default `wait`), nothing for `elab` (user, 2026-11-07, #1123) | Lets a suite on a machine with idle cores take the old background compile thread back per test |
| `foreign_language_interface = "VHPIDIRECT"` and `native_library_hooks(library_file, args=None)` returning `elab_flags` `--load=<library_file>[:args=<layout>]`, which `yavs elab` and `yavs run` both take (user, 2026-10-29; `args`, #928; `elab` too, maintainer 2026-10-31, #943) | A test runner package that loads a native library asks the selected simulator interface how, instead of knowing yavs; a signal's initial value may call the library while the design is elaborated, so `elab` (the elaborate-only run) needs it as much as `run`; `analyze` gets nothing (it was `run_flags` only until #943) |
| Coverage: `enable_coverage` (VUnit's option) or `--yavs-cov` adds the kinds (`--cov-kind=` from `yavs.cov_kind` or `--yavs-cov-kind`, else `--cov=LETTERS` from `--yavs-cov=LETTERS`; none on by default, so no kind is an error before yavs starts, and the letters and `--yavs-cov-kind` together are refused in `from_args`) and `--cov-file=<test output>/yavs/coverage.ycov --cov-test=NAME` before the raw flags; `merge_coverage` merges to `<file>.ycov` and runs `yavs cov report` when there are `args` or any of `--yavs-cov-html`, `--yavs-cov-fail-under`, `--yavs-cov-exclude`, `--yavs-cov-config`; VUnit's `Results.merge_coverage` forwards only `file_name` and `args`, so everything for the report from a run script goes in `args`; this package never replaces a method of VUnit or reads its private fields (maintainer, 2026-11-01, #1020); a failing report registers an `os._exit(1)` exit hook | `post_run` runs before the JUnit file is written, so raising there would lose it; `--cov` with `--dap` is not implemented, so the combination is refused before yavs starts (#877) |
| The `vunit_*` console commands keep VUnit's names, wording, arguments and return values, from its other simulator interfaces (help text says yavs for the other simulator's name); they live here, never in yavs (user, 2026-10-30, #904). `vunit_restart` compiles through `run.py --compile` and then `reload`s, since only VUnit knows the preprocessing, files and options. Init files are console command files whose `$vunit_tb_path` and `$vunit_tb_name` this package substitutes, line by line through `exec` | The maintainer's split; keeps variables out of the yavs console |
| Every `run` except `--yavs-interactive` gets `--exit-on-stdin-close` (#940) | VUnit's `Process` keeps a pipe to the child's stdin and never writes to it, so the pipe closes when the runner dies by any means; a process group kill or a signal cannot cover SIGKILL, and polling the parent needs unsafe system calls. Needs a `Process` that keeps stdin open (it does today) |
| `--yavs-interactive` runs one test on the terminal: the child inherits stdin and stdout, not `Process.consume_output`; refused with `--yavs-debug`, `--elaborate`, `-p` above 1, and a second simulation raises | The terminal is yavs's; one session |
| `compile_source_files` schedules by the project's dependency graph (`_Compile`: `create_dependency_graph`, `get_direct_dependencies`, `get_dependent`, VUnit's compile order): a call takes a ready file and the later files of its key (library, standard, flags) whose dependencies are done or in the call, up to `MAX_BATCH_CHARS` of names, always on; `-j N` runs up to N calls in worker threads, one at a time per library group (`_library_groups`: libraries that read each other, #1150; only for a yavs up to `YAVS_LOCK_CYCLE_LAST_VERSION` (0.19.1) or of unknown version, `yavs_version` from `check_version`); a failed call of several files is repeated one file per call; printing, `project.update` and the skip and stop rules of VUnit's loop stay in the main thread. A runner that passes `batch` or `num_threads` gets the whole compile (user, 2026-11-14) | `docs/decisions.md` "Batch and parallel compile in vunit-yavs"; yavs holds a library's lock for a whole call, so a second call into it would only wait |
| `--yavs-surfer[=HOST:PORT]` adds `--wave=<test output>/yavs/wave.fst --surfer...`; one simulation, refused with `--elaborate` or `-p` above 1 | The file belongs to the test; a viewer for each of many tests helps nobody |
| The token server is opt-in (`--yavs-jobserver`, name kept from 0.10), created only with `-p` above 1 (`-p 0` is resolved to the CPU count the way the test runner does, #1141); Linux and Windows (user, 2026-10-29: one cross-platform mechanism, the fifo jobserver removed) | No change for anyone who does not ask |
| A localhost TCP server at a free port, one random secret per test, `YAVS_TOKENS=<port>:<secret>`; protocol in `docs/cli-contract.md` "Token server" | Same code on every platform; the secret names the test (its cap) and keeps other local processes out |
| With the server, every run gets `--threads=auto` (`max_threads` when set, a number or `auto`); with `auto` a connection's cap is `min(N, AUTO_MAX_THREADS) - 1`, 8 being yavs's own cap of `auto` (agent, 2026-11-07, #884, replacing `--threads=min(N, 4)`, the maintainer's stopgap of 2026-10-31 until `auto` existed; for the maintainer to confirm) | Idle slots speed up the tail without further options; a test whose threads do not gain asks for no tokens after its trial, so the cap no longer has to keep them from processor tests |
| Accounting per connection: free = N - max(running, 1) - lent; lent up to each connection's `want`, breadth first, newest first, per-test cap `max_threads - 1`; never taken back, returned by a lower `want` or a closed connection | Steady-state total N; no token parked where it cannot be used, no reclaim tick |

## 4. Invariants and pitfalls

- Register the package before anything selects the simulator (`add_package` first in `run.py`).
- Tests that register the interface patch a fresh `SimulatorFactory` into the test runner's
  modules: the shared factory refuses a second registration.
- `TokenServer.remove` runs in a `finally`: a test that raises still gives its tokens back (its connections are shut down).
- Only the selector thread registers, unregisters and closes sockets; other threads shut a connection down and the selector thread closes it.

## 5. How to work here

```sh
uv venv ~/.cache/yavs-vunit-venv && VIRTUAL_ENV=~/.cache/yavs-vunit-venv uv pip install -e <vunit with simulator packages> -e python/vunit-yavs pytest
~/.cache/yavs-vunit-venv/bin/python -m pytest python/vunit-yavs/tests
VUNIT_YAVS_PATH=<dir of a yavs binary> ~/.cache/yavs-vunit-venv/bin/python -m pytest python/vunit-yavs/tests/test_real_yavs.py
```

The interface passes `--std-cache=<output path>/std_cache` to every `analyze`, `elab` and `run`, so a
run never writes to the user's cache directory; raw flags come later and win.

## 6. Open issues

- The `vunit_hdl` dependency bound is a placeholder until a test runner release has simulator
  packages.
