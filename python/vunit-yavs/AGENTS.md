# AGENTS.md: python/vunit-yavs/ (the yavs simulator package for VUnit)

Read the root `AGENTS.md` first. This file is for an agent about to change the Python package
`vunit-yavs`.

## 1. Purpose and boundaries

- Everything yavs-specific on the test runner's Python side, in one distribution `vunit-yavs`
  (import name `vunit_yavs`): the simulator interface, `--yavs-debug`, `--yavs-max-threads` and
  `yavs.max_threads`, the jobserver, the version check, `vunit_pkg.toml`. Nothing of it goes into
  the test runner itself or into a crate.
- It drives the CLI contract (`docs/cli-contract.md`); a change of the contract that it relies on
  changes this package in the same commit.
- Not here: the DAP server (`crates/yavs-dap`), the jobserver client inside yavs.

## 2. Map

| Path | What |
|---|---|
| `vunit_yavs/__init__.py` | `YavsInterface` (the `SimulatorInterface`), its arguments, the native library hooks, `check_debug_args`, `write_response_file`, `YavsVersion` |
| `vunit_yavs/jobserver.py` | `lend_order` (the lending priority), `Client` (one fifo), `Jobserver` (accounting, distributor thread, leak warnings) |
| `vunit_yavs/vunit_pkg.toml` | `simulator = "vunit_yavs:YavsInterface"`, shipped as package data |
| `tests/test_interface.py` | Unit tests of the interface, with the package registered in a fresh simulator factory |
| `tests/test_jobserver.py` | Lending order, accounting, reclaim, leaks, a Python child using the fifo |
| `tests/test_fake_e2e.py` | `tests/project/run.py` end to end against `tests/fake_bin/yavs` |
| `tests/test_real_yavs.py` | The same project on a real yavs (`VUNIT_YAVS_PATH` or `PATH`, skipped otherwise), plain, with the jobserver and under `--yavs-debug` with a minimal DAP client |
| `README.md`, `CHANGELOG.md`, `RELEASING.md` | User guide, changes per version, release steps (`.github/workflows/vunit-yavs-release.yml`) |

## 3. Design decisions that bind this module

| Decision | Why |
|---|---|
| A separate distribution in the yavs repository, MPL-2.0 (the interface was written for the test runner under its licence; the file headers keep it) | One place for the Python side; licence of the origin kept (user to confirm) |
| Package arguments carry a `yavs-` prefix (`--yavs-debug`, not `--debug`) | The test runner parses its argv once before packages are added, with prefix matching: `--debug` was taken as another simulator's `--debugger` and failed |
| `--yavs-debug[=PORT]`, default 4711, `0` a free port; the run's address is printed to the terminal | 4711 is the editor extension's attach port; the test runner shows a test's output only on failure |
| `--yavs-debug` is one simulation: refused with `--gui`, `--elaborate` or `-p` above 1 in `from_args`, and a second simulation raises before yavs starts | The interface sees no test list before the run; the second simulation is the earliest point |
| `yavs.max_threads` (sim option) over `--yavs-max-threads` (default) to `--threads=N`, before the raw flags; none under `--yavs-debug` or for `elab` | Raw flags stay the last word; a DAP run is one thread |
| `foreign_language_interface = "VHPIDIRECT"` and `native_library_hooks(library_file)` returning only `run_flags` `--load=<library_file>` (user, 2026-10-29) | A test runner package that loads a native library asks the selected simulator interface how, instead of knowing yavs; the library is needed when the design runs, so `analyze` and the elaborate-only `elab` get nothing |
| The jobserver is opt-in (`--yavs-jobserver`), POSIX only (error on Windows), created only with `-p` above 1 | No change for anyone who does not ask |
| One private fifo per test in a temporary directory, `MAKEFLAGS=-jN --jobserver-auth=fifo:PATH` replacing any outer value | Plain GNU protocol; MAKEFLAGS cannot hold a path with spaces, so not under the output path |
| Accounting: free = N - max(running, 1) - lent; lending breadth first, newest first, per-test cap `max_threads - 1`, at most one token more than in use for a test that left tokens unread at the last tick | Steady-state total N; tokens are not parked in tests that cannot use them |

## 4. Invariants and pitfalls

- Register the package before anything selects the simulator (`add_package` first in `run.py`).
- Tests that register the interface patch a fresh `SimulatorFactory` into the test runner's
  modules: the shared factory refuses a second registration.
- `Jobserver.remove` runs in a `finally`: a test that raises still gives its tokens back.
- The real-binary test under the debugger is an expected failure while yavs reports an internal
  error for the runner's `active_python_runner` under `--dap` (see section 6).

## 5. How to work here

```sh
uv venv ~/.cache/yavs-vunit-venv && VIRTUAL_ENV=~/.cache/yavs-vunit-venv uv pip install -e <vunit with simulator packages> -e python/vunit-yavs pytest
~/.cache/yavs-vunit-venv/bin/python -m pytest python/vunit-yavs/tests
VUNIT_YAVS_PATH=<dir of a yavs binary> ~/.cache/yavs-vunit-venv/bin/python -m pytest python/vunit-yavs/tests/test_real_yavs.py
```

The interface passes `--std-cache=<output path>/std_cache` to every `analyze`, `elab` and `run`, so a
run never writes to the user's cache directory; raw flags come later and win.

## 6. Open issues

- Under `--dap`, the test runner's `active_python_runner` (run_api.vhd:167) is `E9999` "read of
  8 bytes at 0 outside a frame object of 0 bytes" in the interpreter; the same test passes without
  `--dap`.
- The `vunit_hdl` dependency bound is a placeholder until a test runner release has simulator
  packages.
