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

The [guide](https://yavs.readthedocs.io/en/latest/vunit.html) goes from installing to running, debugging and
coverage, and the [debugger tutorial](https://yavs.readthedocs.io/en/latest/debugger-tutorial.html) debugs a failing test with `--yavs-debug`, `--yavs-surfer` and `--yavs-interactive`. `yavs` is found on `PATH`, or in the directory named by `VUNIT_YAVS_PATH`. It needs VUnit 5.0.0.dev16
or later (`pip install vunit-yavs` installs it, a pre-release) and yavs 0.19.1 or later.

## Options

| Sim or compile option | Passed to |
|---|---|
| `yavs.global_flags` | `yavs analyze`, `run` and `elab` |
| `yavs.a_flags` | `yavs analyze` |
| `yavs.elab_flags` | `yavs run` and `elab` |
| `yavs.sim_flags` | `yavs run`, such as `--assert-summary`, `--activity`, `--hang-report`, `--wave-include`: they pass through to the run |
| `yavs.max_threads` (integer, at least 1, or `"auto"`) | `yavs run --threads=N`, the most threads of one simulation; `"auto"` is `--threads=auto`, more threads only while they make the run faster |
| `yavs.tier_compile` (`"wait"` or `"background"`) | `yavs run --tier-compile=MODE`: `wait` (yavs's default) makes a simulation wait while a function compiles, `background` compiles alongside it on a thread at the lowest system priority |
| `enable_coverage` (VUnit's option) | `yavs run --cov-file=<test output>/yavs/coverage.ycov --cov-test=<test name>` with the kinds below (coverage, below) |
| `yavs.cov_kind` (list of strings) | `yavs run --cov-kind=LIST`: `statement`, `branch`, `toggle`, `condition`, `assert`, `expression`, `combination`, `fsm`. No kind is on by default |
| `yavs.init_files.after_load` | Console command files sourced after each `vunit_load` |
| `yavs.init_files.before_run` | Console command files sourced before each `vunit_run` |
| `yavs.init_file.gui` | A console command file sourced by `vunit_user_init` and at the start of `--yavs-interactive` |

| Command line | What it does |
|---|---|
| `--yavs-debug[=PORT]` | Runs the selected test under `yavs run --dap=PORT` (default 4711, `0` picks a free port) and prints where to attach; write the port as `--yavs-debug=PORT` |
| `--yavs-interactive` | Runs the selected test at the yavs console: a `yavs>` prompt at time 0 (below). Not with `--yavs-debug`: one of them drives the run |
| `--yavs-surfer[=HOST:PORT]` | Shows the waves of the one selected test in Surfer 0.6.0 or later while it runs (`yavs run --surfer`); strictly opt-in. Combines with `--yavs-debug` and `--yavs-interactive`, where the cursor follows the stops |
| `--yavs-max-threads N` | `yavs.max_threads` (a number or `auto`) for every test that does not set it (environment variable `VUNIT_YAVS_MAX_THREADS` for run scripts with a command line of their own) |
| `--yavs-lint` | After compiling, runs `yavs lint` on every library of the project that is not external; a finding of severity error fails the compile step. To leave out VUnit's own libraries, put `exclude: ['**/vunit/vhdl/**']` in `yavs-lint.yaml`. `file_list` in that file is not read: the project says which files a library has. Unknown keys in that file are warned about once, for the first library |
| `--yavs-cov=LETTERS` | Collects coverage in every simulation, as `enable_coverage` does for the tests it is set for, of the kinds the letters name: `s` statement, `b` branch, `c` condition, `e` expression, `t` toggle, `f` fsm, `a` assert, `m` combination (`--yavs-cov=sbt`). No kind is on by default; coverage without a kind fails the simulation, naming this flag, `--yavs-cov-kind` and `yavs.cov_kind`. Write the letters with `=` |
| `--yavs-cov-directives MODE` | `apply` or `ignore`: what `yavs analyze`, `elab` and `run` do with the `-- yavs coverage exclude` comments in the sources (`--coverage-directives`); without it yavs's default, `apply`, which checks them when compiling |
| `--yavs-cov-html DIR` | `results.merge_coverage()` also writes the HTML coverage report into DIR |
| `--yavs-cov-fail-under PCT` | `results.merge_coverage()` prints the coverage report and the run exits with status 1 when the total is below PCT percent; per kind with `statement:90,branch:80` |
| `--yavs-cov-cobertura FILE`, `--yavs-cov-json FILE`, `--yavs-cov-lcov FILE`, `--yavs-cov-sonarqube FILE` | `results.merge_coverage()` also writes that export; in a script `merge_coverage("coverage", args=["--lcov", "c.info"])` and so on |
| `--yavs-cov-embed-sources` | Passes `--cov-embed-sources`, as the sim option `yavs.cov_embed_sources` does for the tests it is set for: the HTML report then needs no checkout, and the source text travels with the database |
| `--yavs-cov-below PCT` | The report of `results.merge_coverage()` limits its table of instances to those below PCT percent, worst first |
| `--yavs-cov-detailed` | That report also lists what never ran, under each instance. In a script both go through `merge_coverage("coverage", args=["--below=100", "--detailed"])` |
| `--yavs-cov-exclude FILE` | `results.merge_coverage()` reports without the items this exclusion file lists (`yavs cov report --exclude`); repeatable. A run script passes the same in `merge_coverage("coverage", args=["--exclude", file])` |
| `--yavs-cov-config FILE` | `results.merge_coverage()` reads exclusion files, root and kinds from this file (`yavs cov report --config`). A run script passes `args=["--config", file]` |
| `--yavs-jobserver` | Lends the CPU slots of `-p N` that no test uses to the running simulations through a token server (below) |

Compiling: consecutive files of one library, standard and flags go to `yavs analyze` in one call. With a VUnit that has
`--batch-compile` and `--parallel-compile` (5.0.0.dev16 does not have them), those flags work with yavs too (several files per call, several calls at once).

Without `yavs.max_threads` or `--yavs-max-threads` no thread count is passed, and yavs runs with
its default, unless the token server runs (`--threads=auto`, below). An explicit `--threads` in `yavs.sim_flags` comes after the cap and wins.

## Coverage

```python
vu.set_sim_option("enable_coverage", True)      # or run with --yavs-cov=sb
vu.set_sim_option("yavs.cov_kind", ["statement", "branch"])   # the kinds; none is on by default
vu.main(post_run=lambda results: results.merge_coverage("coverage"))
```

Each simulation writes a yavs coverage database named after its test (`yavs run --cov-file`). The kinds come from the `yavs.cov_kind` sim option, else `--yavs-cov-kind`, else the letters of `--yavs-cov=LETTERS`; the letters and `--yavs-cov-kind` are not given together. `results.merge_coverage("coverage")`
merges them into `coverage.ycov` (`yavs cov merge`); with `--yavs-cov-html DIR` or
`--yavs-cov-fail-under PCT`, or arguments in `merge_coverage("coverage", args=["--cobertura", "c.xml"])`, it
then runs `yavs cov report` on the merged database. A total below the threshold makes the run exit with status 1
after VUnit has written its reports (a JUnit file included). For a CI gate:

```sh
VUNIT_SIMULATOR=yavs python run.py --yavs-cov=sbt --yavs-cov-html cov-html --yavs-cov-fail-under 85
```

Coverage cannot be combined with `--yavs-debug`. Reports, exclusions and the numbers are in the yavs
documentation page on coverage. The coverage options need the yavs release that has `yavs cov report --fail-under`.

## Native libraries of VUnit packages

A VUnit package that loads a native library can ask the simulator interface how: `YavsInterface`
has `foreign_language_interface = "VHPIDIRECT"` (the VHDL calls the library through foreign
subprograms with a `"VHPIDIRECT ..."` attribute) and `native_library_hooks(library_file)`, which
returns the simulator hooks that pass `--load=<library_file>` to every `yavs elab` and `yavs run` (so `--elaborate` works for a design that calls the library while it is elaborated); `native_library_hooks(library_file, args="fat-pointer")`
selects that argument layout for the library (a `run.py` without a package gives `--load=LIB:args=fat-pointer` in `yavs.elab_flags`). A package registers them with
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

Every simulation (not `--yavs-interactive`) is started with `--exit-on-stdin-close`: it ends when the pipe the
runner keeps to its standard input closes, so killing only `python run.py`, even with SIGKILL, does not leave yavs
waiting for a debugger on the port. The waves and coverage files of such a run are not finished.

## Token server

With `-p N --yavs-jobserver` (Linux and Windows; `-p 0`, one job per CPU, counts as the CPU count) the run script starts a token server on
127.0.0.1 and lends the CPU slots of `-p N` that no test uses to the running simulations. Every
simulation runs with `--threads=auto` (unless `yavs.max_threads` or `--yavs-max-threads` says otherwise) and gets
`YAVS_TOKENS=<port>:<secret>`, a secret of its own; yavs runs its extra threads only on tokens the
server lends, and asks for them only while it has work for them. One CPU slot per running test is
free, the other `N - k` (for `k` running tests) are lent: breadth first (every test gets one extra
before any gets a second) and newest test first within a round, at most `yavs.max_threads - 1` per
test (`min(N, 8) - 1` with `auto`). With `auto` a simulation whose threads do not make it faster asks for no
tokens after its first fraction of a second. A token in use is never taken back, so a test that starts while every token is out runs at
once: the total is `N` in the steady state, not at every instant. A simulation gives its tokens
back when it stops using them and when it ends in any way, also when it is killed. The tail of a
regression, a few long tests on a machine of idle slots, gets faster on designs whose processes
run in parallel well. If the server cannot start (a blocked local socket) the run warns and every
simulation uses one thread.

## The console

```sh
VUNIT_SIMULATOR=yavs python run.py lib.tb.test_name --yavs-interactive
```

The test stops at time 0 at a `yavs>` prompt. All commands of the yavs console work (`run`, `step`,
`examine`, `force`, `break`, `restart`, `source`, `help`, `quit`), and this package adds VUnit's own,
which behave as in VUnit's other simulator interfaces. They are also in the Debug Console under
`--yavs-debug`.

| Command | What it does | Fails (an error at the console, the end of a script) when |
|---|---|---|
| `vunit_help` | Lists these commands | |
| `vunit_load [flags]` | Loads the test at time 0, with extra `yavs run` flags if given, and sources `yavs.init_files.after_load` | The design does not elaborate, a flag is unknown or an init file fails |
| `vunit_user_init` | Sources `yavs.init_file.gui` | The file fails |
| `vunit_run` | Sources `yavs.init_files.before_run`, runs, and prints `Test Run Passed!` | The test suite did not finish, also when a breakpoint stops the run: `Test Run Failed!` |
| `vunit_compile` | Runs `run.py --compile` with your arguments, without `--clean` | The compile fails |
| `vunit_restart` | `vunit_compile`, and if that worked: the new design at time 0, breakpoints kept, and `vunit_run` | As `vunit_compile`, then `vunit_run` |

The yavs `restart` goes back to time 0 with the design as loaded; `vunit_restart` compiles first. Init files are
files of console commands; `$vunit_tb_path` and `$vunit_tb_name` are replaced as VUnit defines them. When you `quit`,
the test is reported as usual. A session that compiled at `vunit_compile` and failed keeps the old design loaded.

## Licence

Mozilla Public License 2.0 (https://mozilla.org/MPL/2.0/), see the header of each file.
