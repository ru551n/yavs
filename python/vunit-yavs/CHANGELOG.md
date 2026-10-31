# Changelog

## Unreleased

- Consecutive files of one library, standard and flags are analyzed in one `yavs analyze` call, which makes a first
  compile faster; a call that fails is repeated file by file, so failures are reported as before.
- `--yavs-surfer[=HOST:PORT]` shows the waves of one test in Surfer while it runs.
- Coverage: `supports_coverage()` is true. `enable_coverage` or `--yavs-cov` collects a yavs coverage database per
  simulation, `yavs.cov_kind` chooses the kinds, and `results.merge_coverage(file, args)` merges them into
  `file.ycov` and, with `--yavs-cov-html DIR`, `--yavs-cov-fail-under PCT` or `args`, runs `yavs cov report`; a
  total below the threshold makes the run exit with status 1 after VUnit's own reports are written.
- `--yavs-interactive` runs one test at the yavs console. `vunit_help`, `vunit_load`, `vunit_user_init`, `vunit_run`,
  `vunit_compile` and `vunit_restart` work there and in the Debug Console under `--yavs-debug`, with the sim options
  `yavs.init_files.after_load`, `yavs.init_files.before_run` and `yavs.init_file.gui`. Needs a yavs with the console
  hook.

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
