# Changelog

## Unreleased

- `--yavs-jobserver` runs a token server on a localhost socket instead of a fifo jobserver per test, on Linux and
  Windows: each simulation gets `YAVS_TOKENS` and, unless `yavs.max_threads` says otherwise, `--threads=N` for `-p N`,
  so the idle slots of `-p N` speed up the tests still running. Needs a yavs with the token server client; the
  `MAKEFLAGS` jobserver is gone from both sides.

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
