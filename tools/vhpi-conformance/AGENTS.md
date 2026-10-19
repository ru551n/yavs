# AGENTS.md: tools/vhpi-conformance/ (VHPI conformance trace diff)

Read the root `AGENTS.md` and `tools/AGENTS.md` first. This file is for an agent about to run or
extend the VHPI conformance run (#273).

## 1. Purpose and boundaries

- Small C plugins (`plugins/*.c`, shared helpers in `plugins/common.h`) exercise the VHPI calls in
  a fixed order on small designs (`designs/*.vhd`) and print a canonical trace. `run.py` runs each
  plugin on yavs and on the primary reference simulator, keeps the trace lines and the exit
  status, and diffs them. Nothing needs installing: gcc and the reference simulator only.
- Not here: the end-to-end plugin tests (`crates/yavs/tests/vhpi_run.rs`) and the Python
  testbench framework regression (`crates/yavs-vhpi/COCOTB.md`).

## 2. Map

| Path | What |
|---|---|
| `run.py` | Builds each plugin with the vendored `vhpi_user.h`, runs both simulators (`RUNS` table: plugin to designs), writes traces, diffs and stderr to `--out` (default `~/.cache/yavs-vhpi-conformance`), prints one PASS/DIFF line per pair |
| `plugins/walk.c` | Hierarchy walk: regions, declarations, instances, parent navigation |
| `plugins/props.c` | Name lookup, kinds, integer and string properties, type handles, indexing, resolution |
| `plugins/values.c` | Every signal read in every value format |
| `plugins/cbs.c` | Callback reasons: registration, firing order, time and delta |
| `plugins/put.c` | `vhpi_put_value` deposit, force, release, bad arguments, read back at several phases |
| `designs/hier.vhd`, `designs/drive.vhd` | The designs |

## 3. Design decisions that bind this module

| Decision | Why |
|---|---|
| Trace lines start with `T `; only those are compared, plus `# exit N` | The simulators print their own messages differently |
| Each trace line is one `vhpi_printf` call (`L()` in `common.h`) | A reference simulator wraps every call into a message of its own |
| Error text is never printed, only whether an error was pending | Messages differ by design |
| Times are printed in fs (callback time times the resolution) | Resolution defaults differ |
| A zero diff is the goal, but not every difference is a yavs bug (for example the reference simulator reports upper-case names); judge each hunk, and file an issue for real ones | VHPI leaves details to the tool |

## 4. Pitfalls

- Plugin output of the reference simulator comes as `** Note: <time>: T ...` on stderr; `run.py`
  strips the prefix.
- The reference simulator needs a library directory ending in `.08` and the `work:` form.
- A plugin that registers repeating callbacks without finishing relies on the design ending; keep
  designs finite (the run is killed after 120 s).

## 5. How to work here

```sh
YAVS_BIN=path/to/yavs tools/vhpi-conformance/run.py [--only walk,put] [--show]
```

Exit status 1 when any trace differs. Add a plugin: write `plugins/<name>.c`, add it to `RUNS`.
