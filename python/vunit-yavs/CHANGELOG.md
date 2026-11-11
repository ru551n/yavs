# Changelog

## 0.19.1 (2026-11-11)

- Needs yavs 0.19.1 or later. All changes: the yavs 0.19.1 release notes.

## 0.19.0 (2026-11-10)

- Needs yavs 0.19.0 or later. All changes: the yavs 0.19.0 release notes.

## 0.18.0 (2026-11-09)

- Needs yavs 0.18.0 or later. All changes: the yavs 0.18.0 release notes.

## 0.17.0 (2026-11-08)

- Needs yavs 0.17.0 or later. All changes: the yavs 0.17.0 release notes.

## 0.16.1 (2026-11-06)

- Needs yavs 0.16.1 or later. All changes: the yavs 0.16.1 release notes.

## 0.16.0 (2026-11-06)

- Needs yavs 0.16.0 or later. All changes: the yavs 0.16.0 release notes.

## 0.15.1 (2026-11-05)

- Needs yavs 0.15.1 or later. All changes: the yavs 0.15.1 release notes.

## 0.15.0 (2026-11-04)

- Needs yavs 0.15.0 or later. All changes: the yavs 0.15.0 release notes.

## 0.14.0 (2026-11-04)

- Needs yavs 0.14.0 or later. All changes: the yavs 0.14.0 release notes.

## 0.13.0 (2026-11-02)

- Needs yavs 0.13.0 or later. All changes: the yavs 0.13.0 release notes.

## 0.12.1 (2026-11-02)

- Needs yavs 0.12.1 or later. All changes: the yavs 0.12.1 release notes.

## 0.12.0 (2026-11-01)

- Breaking: no coverage kind is on by default. `--yavs-cov` becomes `--yavs-cov=LETTERS` (`s` statement, `b` branch,
  `c` condition, `e` expression, `t` toggle, `f` fsm, `a` assert, `m` combination; for example `--yavs-cov=sbt`), and
  `--yavs-cov-kind` or `yavs.cov_kind` name kinds by word. A simulation with coverage on and no kind fails before yavs
  starts, with a message that names the flag, the option and the letters. The database goes to yavs by `--cov-file`
  instead of `--cov=FILE`. Needs a yavs with `--cov=LETTERS` and `--cov-file`.
- `--yavs-lint` checks the configuration once before any library: an unknown key, rule or rule group in
  `yavs-lint.yaml` or the waiver file is error E0134, told once, and fails the compile step. Needs a yavs
  that rejects unknown keys.
- `--yavs-cov-exclude FILE` (repeatable) and `--yavs-cov-config FILE` apply exclusion files and a coverage configuration to
  the report; a run script passes the same through `results.merge_coverage(file, args=["--exclude", f, "--config", g])`. Needs a yavs with repeatable `--exclude` and `--config`.
- `--yavs-cov-cobertura`, `--yavs-cov-json`, `--yavs-cov-lcov` and `--yavs-cov-sonarqube` write the coverage exports. Needs a yavs with those exports.
- `--yavs-cov-embed-sources` and the sim option `yavs.cov_embed_sources` store the source text in the coverage database. Needs a yavs with `--cov-embed-sources`.
- `--yavs-cov-below PCT` limits the report's table to the instances below PCT percent, worst first, and `--yavs-cov-detailed` lists what never ran under each. In a script they go through `merge_coverage(args=[...])`. Needs a yavs with `yavs cov report --below` and `--detailed`.
- A `yavs cov merge` that fails in `merge_coverage`, for example on a database another yavs version wrote, shows yavs's error and exits with status 1 after VUnit's reports, not a Python traceback.
- `--yavs-cov-fail-under` takes a percentage per kind (`statement:90,branch:80`), with the kinds checked before any simulation runs. Needs a yavs with per-kind `--fail-under`.
- `yavs.cov_kind` takes `fsm` and `assert`, and `--yavs-cov-kind LIST` gives the kinds on the command line for tests
  without that option. Needs a yavs with state machine coverage.
- A simulation ends with its test runner: yavs gets `--exit-on-stdin-close` and ends when the pipe the runner keeps to
  its standard input closes, so killing only `python run.py` (SIGKILL included) no longer leaves yavs waiting for a
  debugger on the port. Needs a yavs with that option.
- `native_library_hooks` passes `--load` to the elaborate-only step (`--elaborate`) as well as to the run, because a foreign
  function may be called while the design is elaborated; it returns `elab_flags` instead of `run_flags`.
- `yavs.cov_kind` takes `combination`, and `--yavs-cov-directives apply|ignore` passes `--coverage-directives` to
  `yavs analyze`, `elab` and `run`: the `-- yavs coverage exclude` comments of the sources are checked when compiling
  (a malformed one fails the compile step) unless `ignore` is given. Needs a yavs with combination coverage.
- `native_library_hooks(library_file, args="fat-pointer")` loads a native library in that argument layout.
- `VUNIT_YAVS_MAX_THREADS` sets the thread cap for run scripts that parse their own command line (no `--yavs-max-threads`).
- The token server: a simulation uses at most 4 threads unless `yavs.max_threads` or `--yavs-max-threads` says more (many
  idle threads at a large `-p`), and a server that cannot start is a warning, every simulation then runs on one thread.
- Tests of a native library and of Python embedded in it, called from VHDL through foreign subprograms
  (`tests/project_foreign`); the README says how a package registers the hooks.
- A runner's `--parallel-compile` and `--batch-compile` work with yavs (`supports_concurrent_compiles_into_library`,
  `supports_batch_compile`); the file names of a batch go on the command line. Without those flags the package batches
  consecutive files as before.
- Consecutive files of one library, standard and flags are analyzed in one `yavs analyze` call, which makes a first
  compile faster; a call that fails is repeated file by file, so failures are reported as before.
- `--yavs-surfer[=HOST:PORT]` shows the waves of one test in Surfer while it runs.
- Coverage: `supports_coverage()` is true. `enable_coverage` or `--yavs-cov` collects a yavs coverage database per
  simulation, `yavs.cov_kind` chooses the kinds, and `results.merge_coverage(file, args)` merges them into
  `file.ycov` and, with `--yavs-cov-html DIR`, `--yavs-cov-fail-under PCT` or `args`, runs `yavs cov report`; a
  total below the threshold makes the run exit with status 1 after VUnit's own reports are written.
- `--yavs-lint` runs `yavs lint` on the libraries of the project after compiling; a finding of severity error fails
  the compile step.
- `--yavs-interactive` runs one test at the yavs console. `vunit_help`, `vunit_load`, `vunit_user_init`, `vunit_run`,
  `vunit_compile` and `vunit_restart` work there and in the Debug Console under `--yavs-debug`, with the sim options
  `yavs.init_files.after_load`, `yavs.init_files.before_run` and `yavs.init_file.gui`. Needs a yavs with the console
  hook.
- `-g/--gui` is refused at the start with a message that points to `--yavs-debug`: yavs has no
  graphical debugger and no `yavs run --gui` any more.
- Needs yavs 0.12.0 or later. All changes: the yavs 0.12.0 release notes.

## 0.11.0 (2026-10-29)

- `--yavs-jobserver` runs a token server on a localhost socket instead of a fifo jobserver per test, on Linux and
  Windows: each simulation gets `YAVS_TOKENS` and, unless `yavs.max_threads` says otherwise, `--threads=N` for `-p N`,
  so the idle slots of `-p N` speed up the tests still running. Needs a yavs with the token server client; the
  `MAKEFLAGS` jobserver is gone from both sides.
- Needs yavs 0.11.0 or later.

## 0.10.1 (2026-10-29)

- Native library hooks for VUnit packages that load a native library: `YavsInterface` has
  `foreign_language_interface = "VHPIDIRECT"` and `native_library_hooks(library_file)`, which
  passes `--load=<library_file>` to `yavs run`.

## 0.10.0 (2026-10-29)

- First release. Needs VUnit 5.0.0.dev16 or later and yavs 0.10.0 or later.
- The yavs simulator interface as a VUnit package: `vu.add_package("vunit-yavs", allow_setup=True)`
  and `VUNIT_SIMULATOR=yavs`.
- `--yavs-debug[=PORT]` runs one test under the yavs debugger for an editor to attach to.
- `yavs.max_threads` and `--yavs-max-threads` cap the threads of each simulation.
- `--yavs-jobserver` lends the idle CPU slots of `-p N` to the running simulations through a
  jobserver per test.
- The shared std/ieee units are kept in `std_cache` under the simulator output directory
  (`--std-cache` on every `analyze`, `elab` and `run`); a `--std-cache` in the `yavs.*` flags wins.
