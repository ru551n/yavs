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

| Command line | What it does |
|---|---|
| `-g/--gui` | Opens the run in the graphical debugger (`yavs run --gui`) |
| `--yavs-debug[=PORT]` | Runs the selected test under `yavs run --dap=PORT` (default 4711, `0` picks a free port) and prints where to attach; write the port as `--yavs-debug=PORT` |
| `--yavs-max-threads N` | `yavs.max_threads` for every test that does not set it |
| `--yavs-jobserver` | Lends the CPU slots of `-p N` that no test uses to the running simulations through a token server (below) |

Without `yavs.max_threads` or `--yavs-max-threads` no thread count is passed, and yavs runs with
its default, unless the token server runs (`--threads=N` for `-p N`, below). An explicit `--threads` in `yavs.sim_flags` comes after the cap and wins.

## Native libraries of VUnit packages

A VUnit package that loads a native library can ask the simulator interface how: `YavsInterface`
has `foreign_language_interface = "VHPIDIRECT"` (the VHDL calls the library through foreign
subprograms with a `"VHPIDIRECT ..."` attribute) and `native_library_hooks(library_file)`, which
returns the simulator hooks that pass `--load=<library_file>` to every `yavs run`.

## Debugging one test

```sh
VUNIT_SIMULATOR=yavs python run.py lib.tb.test_name --yavs-debug
```

yavs waits for a Debug Adapter Protocol client on `127.0.0.1:4711` before the simulation starts;
attach the editor (the yavs extension's "Attach to yavs" configuration uses that port). When the
session ends, the test is reported as usual.

`--yavs-debug` runs one simulation. A second simulation of the run (a second test, or the same
test in another configuration) is an error before yavs starts, naming both tests. `--yavs-debug`
together with `--gui`, `--elaborate` or `-p` above 1 is an error at the start. A bare
`--yavs-debug` reads a following test name as its port: put test names first, or write
`--yavs-debug=4711`.

## Token server

With `-p N --yavs-jobserver` (Linux and Windows) the run script starts a token server on
127.0.0.1 and lends the CPU slots of `-p N` that no test uses to the running simulations. Every
simulation runs with `--threads=N` (`yavs.max_threads` or `--yavs-max-threads` caps it) and gets
`YAVS_TOKENS=<port>:<secret>`, a secret of its own; yavs runs its extra threads only on tokens the
server lends, and asks for them only while it has work for them. One CPU slot per running test is
free, the other `N - k` (for `k` running tests) are lent: breadth first (every test gets one extra
before any gets a second) and newest test first within a round, at most `yavs.max_threads - 1` per
test. A token in use is never taken back, so a test that starts while every token is out runs at
once: the total is `N` in the steady state, not at every instant. A simulation gives its tokens
back when it stops using them and when it ends in any way, also when it is killed. The tail of a
regression, a few long tests on a machine of idle slots, gets faster on designs whose processes
run in parallel well.

## Licence

Mozilla Public License 2.0 (https://mozilla.org/MPL/2.0/), see the header of each file.
