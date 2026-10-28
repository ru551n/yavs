# Changelog

## 0.9.0 (unreleased)

- The yavs simulator interface as a VUnit package: `vu.add_package("vunit-yavs", allow_setup=True)`
  and `VUNIT_SIMULATOR=yavs`.
- `--yavs-debug[=PORT]` runs one test under the yavs debugger for an editor to attach to.
- `yavs.max_threads` and `--yavs-max-threads` cap the threads of each simulation.
- `--yavs-jobserver` lends the idle CPU slots of `-p N` to the running simulations through a
  jobserver per test.
- The shared std/ieee units are kept in `std_cache` under the simulator output directory
  (`--std-cache` on every `analyze`, `elab` and `run`); a `--std-cache` in the `yavs.*` flags wins.
