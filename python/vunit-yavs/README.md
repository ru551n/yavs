# vunit-yavs

The yavs simulator for VUnit, as a VUnit package. Add it in the run script before anything else,
then select yavs with `VUNIT_SIMULATOR=yavs`:

```python
from vunit import VUnit

vu = VUnit.from_argv()
vu.add_package("vunit-yavs", allow_setup=True)  # first: it provides the simulator
vu.add_vhdl_builtins()
vu.add_library("lib").add_source_files("*.vhd")
vu.main()
```

```sh
pip install vunit-yavs
VUNIT_SIMULATOR=yavs python run.py
```

`yavs` is found on `PATH`, or in the directory named by `VUNIT_YAVS_PATH`. It needs VUnit 5.0.0.dev16
or later (`pip install vunit-yavs` installs it, a pre-release) and yavs 0.11.0 or later.

## Options

| Sim or compile option | Passed to |
|---|---|
| `yavs.global_flags` | `yavs analyze`, `run` and `elab` |
| `yavs.a_flags` | `yavs analyze` |
| `yavs.elab_flags` | `yavs run` and `elab` |
| `yavs.sim_flags` | `yavs run` |
| `yavs.max_threads` (integer, at least 1) | `yavs run --threads=N`, the most threads of one simulation |
| `enable_coverage` (VUnit's option) | `yavs run --cov=<test output>/yavs/coverage.ycov --cov-test=<test name>` (coverage, below) |
| `yavs.cov_kind` (list of strings) | `yavs run --cov-kind=LIST`: `statement`, `branch`, `toggle`, `condition`, `expression` |
| `yavs.init_files.after_load` | Console command files sourced after each `vunit_load` |
| `yavs.init_files.before_run` | Console command files sourced before each `vunit_run` |
| `yavs.init_file.gui` | A console command file sourced by `vunit_user_init` and at the start of `--yavs-interactive` |

| Command line | What it does |
|---|---|
| `--yavs-debug[=PORT]` | Runs the selected test under `yavs run --dap=PORT` (default 4711, `0` picks a free port) and prints where to attach; write the port as `--yavs-debug=PORT` |
| `--yavs-interactive` | Runs the selected test at the yavs console: a `yavs>` prompt at time 0 (below) |
| `--yavs-surfer[=HOST:PORT]` | Shows the waves of the one selected test in Surfer 0.6.0 or later while it runs (`yavs run --surfer`); strictly opt-in. Combines with `--yavs-debug` and `--yavs-interactive`, where the cursor follows the stops |
| `--yavs-max-threads N` | `yavs.max_threads` for every test that does not set it (environment variable `VUNIT_YAVS_MAX_THREADS` for run scripts with a command line of their own) |
| `--yavs-lint` | After compiling, runs `yavs lint` on every library of the project that is not external; a finding of severity error fails the compile step |
| `--yavs-cov` | Collects coverage in every simulation, as `enable_coverage` does for the tests it is set for |
| `--yavs-cov-html DIR` | `results.merge_coverage()` also writes the HTML coverage report into DIR |
| `--yavs-cov-fail-under PCT` | `results.merge_coverage()` prints the coverage report and the run exits with status 1 when the total is below PCT percent |
| `--yavs-jobserver` | Lends the CPU slots of `-p N` that no test uses to the running simulations through a token server (below) |

Compiling: consecutive files of one library, standard and flags go to `yavs analyze` in one call. With a VUnit that has
`--batch-compile` and `--parallel-compile`, those flags work with yavs too (several files per call, several calls at once).

Without `yavs.max_threads` or `--yavs-max-threads` no thread count is passed, and yavs runs with
its default, unless the token server runs (`--threads=min(N, 8)` for `-p N`, below). An explicit `--threads` in `yavs.sim_flags` comes after the cap and wins.

## Coverage

```python
vu.set_sim_option("enable_coverage", True)      # or run with --yavs-cov
vu.set_sim_option("yavs.cov_kind", ["statement", "branch"])   # optional; default statement, branch, toggle
vu.main(post_run=lambda results: results.merge_coverage("coverage"))
```

Each simulation writes a yavs coverage database named after its test. `results.merge_coverage("coverage")`
merges them into `coverage.ycov` (`yavs cov merge`); with `--yavs-cov-html DIR` or
`--yavs-cov-fail-under PCT`, or arguments in `merge_coverage("coverage", args=["--cobertura", "c.xml"])`, it
then runs `yavs cov report` on the merged database. A total below the threshold makes the run exit with status 1
after VUnit has written its reports (a JUnit file included). For a CI gate:

```sh
VUNIT_SIMULATOR=yavs python run.py --yavs-cov --yavs-cov-html cov-html --yavs-cov-fail-under 85
```

Coverage cannot be combined with `--yavs-debug`. Reports, exclusions and the numbers are in the yavs
documentation page on coverage. The coverage options need the yavs release that has `yavs cov report --fail-under`.

## Native libraries of VUnit packages

A VUnit package that loads a native library can ask the simulator interface how: `YavsInterface`
has `foreign_language_interface = "VHPIDIRECT"` (the VHDL calls the library through foreign
subprograms with a `"VHPIDIRECT ..."` attribute) and `native_library_hooks(library_file)`, which
returns the simulator hooks that pass `--load=<library_file>` to every `yavs run`. A package registers them with
`context.register_simulator_hooks("yavs", **YavsInterface.native_library_hooks(library))`. The repository's
`tests/project_foreign` is a package that builds a C library, with Python embedded where the host allows it, and a
testbench that calls Python from VHDL. A VHDL string reaches the library as one byte per character, and yavs reads a
UTF-8 source file as Latin-1 characters: give `--encoding=latin1` to `yavs.a_flags` and (as a sim option) to
`yavs.global_flags` when the library needs the UTF-8 bytes of an accented literal.

## Debugging one test

```sh
VUNIT_SIMULATOR=yavs python run.py lib.tb.test_name --yavs-debug
```

yavs waits for a Debug Adapter Protocol client on `127.0.0.1:4711` before the simulation starts;
press F5 or run "yavs: Attach" in VS Code with the yavs extension: no launch configuration is needed. Set breakpoints
in your own files: with location or check preprocessing on, the debugger maps the rewritten copies to them. When the
session ends, the test is reported as usual.

`--yavs-debug` runs one simulation. A second simulation of the run (a second test, or the same
test in another configuration) is an error before yavs starts, naming both tests. `--yavs-debug`
together with `--elaborate` or `-p` above 1 is an error at the start. A bare
`--yavs-debug` reads a following test name as its port: put test names first, or write
`--yavs-debug=4711`.

## Token server

With `-p N --yavs-jobserver` (Linux and Windows) the run script starts a token server on
127.0.0.1 and lends the CPU slots of `-p N` that no test uses to the running simulations. Every
simulation runs with `--threads=N` (at most 8 unless `yavs.max_threads` or `--yavs-max-threads` says more) and gets
`YAVS_TOKENS=<port>:<secret>`, a secret of its own; yavs runs its extra threads only on tokens the
server lends, and asks for them only while it has work for them. One CPU slot per running test is
free, the other `N - k` (for `k` running tests) are lent: breadth first (every test gets one extra
before any gets a second) and newest test first within a round, at most `yavs.max_threads - 1` per
test. A token in use is never taken back, so a test that starts while every token is out runs at
once: the total is `N` in the steady state, not at every instant. A simulation gives its tokens
back when it stops using them and when it ends in any way, also when it is killed. The tail of a
regression, a few long tests on a machine of idle slots, gets faster on designs whose processes
run in parallel well. If the server cannot start (a blocked local socket) the run warns and every
simulation uses one thread.

## Licence

Mozilla Public License 2.0 (https://mozilla.org/MPL/2.0/), see the header of each file.

## The console

```sh
VUNIT_SIMULATOR=yavs python run.py lib.tb.test_name --yavs-interactive
```

The test stops at time 0 at a `yavs>` prompt. All commands of the yavs console work (`run`, `step`,
`examine`, `force`, `break`, `restart`, `source`, `help`, `quit`), and this package adds VUnit's own,
which behave as in VUnit's other simulator interfaces. They are also in the Debug Console under
`--yavs-debug`.

| Command | What it does | Returns |
|---|---|---|
| `vunit_help` | Lists these commands | |
| `vunit_load [flags]` | Loads the test at time 0, with extra `yavs run` flags if given, and sources `yavs.init_files.after_load` | `false`, `true` on failure |
| `vunit_user_init` | Sources `yavs.init_file.gui` | `0`, `true` on failure |
| `vunit_run` | Sources `yavs.init_files.before_run`, runs, and prints `Test Run Failed!` unless the test suite finished (so also when a breakpoint stops the run) | `false`, `true` on failure |
| `vunit_compile` | Runs `run.py --compile` with your arguments, without `--clean` | `false`, `true` on failure |
| `vunit_restart` | `vunit_compile`, and if that worked: the new design at time 0, breakpoints kept, and `vunit_run` | as `vunit_run` |

The yavs `restart` goes back to time 0 with the design as loaded; `vunit_restart` compiles first. Init files are
files of console commands; `$vunit_tb_path` and `$vunit_tb_name` are replaced as VUnit defines them. When you `quit`,
the test is reported as usual. A session that compiled at `vunit_compile` and failed keeps the old design loaded.
