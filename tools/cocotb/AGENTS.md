# AGENTS.md: tools/cocotb/ (cocotb's own regression on yavs)

Read the root `AGENTS.md` and `tools/AGENTS.md` first. This file is for an agent about to run or
change the cocotb regression (#122, epic #143).

## 1. Purpose and boundaries

- `run.py` installs a pinned cocotb into a venv and runs cocotb's own simulator regression with
  `SIM=yavs TOPLEVEL_LANG=vhdl` over VHPI: what cocotb's noxfile runs for a simulator (the
  `tests/test_cases` directories through make, the examples through make and pytest,
  `pytest -k simulator_required` and the pytest plugin's tests), with the noxfile's environment
  (`PYTHONWARNINGS=error,...`, the debug switches).
- `Makefile.yavs` and `yavs_runner.py` are the yavs integration for cocotb's makefiles and its
  Python runner and pytest plugin: the change cocotb itself would need, kept here until it is
  upstream. Setup copies them into the venv and appends the registration to cocotb's runner table
  and the pytest plugin's simulator table.
- Not here: the VHPI implementation (`crates/yavs-vhpi`), the C trace diff against the reference
  simulators (`tools/vhpi-conformance`).

## 2. Map

| Path | What |
|---|---|
| `run.py` | Setup (venv, clone, VHPI library, integration files), the job list, `GATES`, `EXPECTED_FAIL`, the summary and `--json` |
| `Makefile.yavs` | The simulator makefile: `yavs analyze` per library, `yavs run --vhpi=... --resolution=...` with plusargs after the top level |
| `yavs_runner.py` | `Yavs(Runner)` for `cocotb_tools.runner.get_runner("yavs")` |

## 3. Design decisions that bind this module

| Decision | Why |
|---|---|
| cocotb 2.1.0 pinned three ways: the wheel version, the clone's commit `SHA` (its tests and examples are not in the sdist), and the test dependencies (`TEST_DEPS`, Python `PYTHON`) | Reproducible numbers; a bump is one commit that updates all of them and re-checks `GATES` |
| The VHPI library is cocotb's own source (`share/lib/gpi/vhpi`) compiled from the clone without any simulator define, as `libcocotbvhpi_yavs`, against the wheel's `libgpi` (old libstdc++ string ABI of the manylinux wheel) | No simulator-specific workaround in the path; the library cocotb would ship for yavs |
| `GATES` add `yavs` to name lists in the scratch copy of the tests where cocotb leaves out a simulator it does not know (VHDL libraries, top-level library, waves, timescale, IPython, the recursive object count three references agree on, the generics of an example) | Those tests then run instead of skipping; it is the upstream change, not a test workaround. A gate whose text is gone after a bump stops the run |
| A job passes when it exits 0: cocotb's own verdict, which includes its expected failures (`test_failure`, `test_max_failures`) | No second judgement on top of the suite's |
| Jobs that run no VHDL test (Verilog only, error-path cases) pass by skipping; the summary counts how many jobs ran tests | Honest totals |
| `--run-args` reaches the makefile and the runner through a wrapper of the binary | One knob for `--threads` and `--tier` in both paths |
| Setup needs `uv`, network for pip and the clone, and a C++ compiler; the venv lives outside the repository (default `~/.cache/yavs-cocotb`) | User approval for the install (#122, 2026-10-19 and 2026-10-28) |

## 4. Invariants and pitfalls

- Run with `python3 -I`; the tool itself is stdlib only, the venv has cocotb.
- `--out` is wiped at the start of a run; the tool refuses a directory it did not create.
- cocotb's `test_cases/test_array` releases the handle of a pseudo-region of a two-dimensional array
  that borrows its parent's handle (a cocotb bug): yavs reports `W0903` for the second release.
  It is noise in the log, not a failure.
- `pytest` and `pytest_plugin` share the out directory; their JUnit files have fixed names.

## 5. How to work here

```sh
cargo build --release -p yavs
python3 -I tools/cocotb/run.py --yavs target/release/yavs [--dir DIR] [--jobs 6] [--run-args='--threads=4'] [--json F] [job ...]
```

Logs per job are in `--out` (`test_cases-test_array.log`). CI runs it in the `conformance`
workflow; `tools/conformance/run.py --local` records it in `docs/conformance-local.json`.
