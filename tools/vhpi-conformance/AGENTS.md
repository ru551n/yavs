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
| `run.py` | Builds each plugin with the vendored `vhpi_user.h`, runs yavs and the reference simulators (`RUNS` table: plugin to designs), writes traces, diffs, three-way tables (`*.three`) and stderr to `--out` (default `~/.cache/yavs-vhpi-conformance`), prints one PASS/DIFF line per pair, plus a three-way count when a second reference is set |
| `plugins/walk.c` | Hierarchy walk: regions, declarations, instances, parent navigation |
| `plugins/props.c` | Name lookup, kinds, integer and string properties, type handles, indexing, resolution |
| `plugins/values.c` | Every signal read in every value format |
| `plugins/cbs.c` | Callback reasons: registration, firing order, time and delta |
| `plugins/nonprop.c`, `designs/nonprop.vhd` | The non-propagating put modes: accepted or not, the value, value-change callbacks and process events |
| `plugins/intvec.c`, `designs/intvec.vhd` | An integer put into vectors: which bits each simulator takes, negative values, values that do not fit |
| `plugins/strput.c`, `designs/strput.vhd` | `vhpiStrVal` puts shorter and longer than a string of either direction: accepted or not, which elements change to what |
| `plugins/vecedge.c`, `designs/vecedge.vhd` | Value-change callbacks on a vector with several changed runs per cycle: a repeating one, one re-armed from inside itself, one registered mid-cycle on another signal (yavs deliberately also fires that one for the same cycle, one extra `w` line against the references, #496) |
| `plugins/lateedge.c`, `designs/lateedge.vhd` | A value-change callback registered from the callback of another signal that changes in the same cycle, re-armed from inside itself, with the second signal reported after the first and before it (yavs deliberately fires both, two extra lines against the references, #496) |
| `plugins/tport.c`, `designs/tport.vhd` | Read-write and read-only callbacks at time 0 of a design with no delta there, a put from the read-write one, value changes through a transport delay on an integer port |
| `plugins/put.c` | `vhpi_put_value` deposit, force, release, bad arguments, read back at several phases |
| `plugins/fresult.c`, `designs/fresult.vhd` | Foreign functions with array results (#330), one entity per case: sized with `vhpiSizeConstraint` or not, a string, a constrained result; the size before and after, the bounds |
| `plugins/fparam.c`, `designs/fparam.vhd`, `fsig.vhd`, `fsigin.vhd` | Foreign subprograms with record (fields read and written), access (`vhpiPtrVal`, `vhpiDerefObj`) and signal parameters (read, a deposit, transactions on the parameter and on its driver); signal ones in their own files, as one reference refuses `out` signal parameters at analysis |
| `plugins/farch.c`, `designs/farch.vhd` | A foreign architecture: when `elabf` and `execf` run, `vhpi_create` of processes and drivers (which handles, which order, which phase), transactions from a value-change callback (inertial with a rejected pulse, transport) |
| `plugins/libapp.c`, `designs/libapp.vhd` | Library and application foreign models (#357): whether and when their `elabf` and `execf` run, with a foreign function of the same library in the design |
| `plugins/vars.c`, `designs/vars.vhd` | Variables read and deposited before their process ran, and a shared variable no process uses (#342) |
| `designs/hier.vhd`, `designs/drive.vhd` | The designs |

## 3. Design decisions that bind this module

| Decision | Why |
|---|---|
| Trace lines start with `T `; only those are compared, plus `# exit N` | The simulators print their own messages differently |
| Each trace line is one `vhpi_printf` call (`L()` in `common.h`) | A reference simulator wraps every call into a message of its own |
| Error text is never printed, only whether an error was pending | Messages differ by design |
| Times are printed in fs (callback time times the resolution) | Resolution defaults differ |
| A zero diff is the goal, but not every difference is a yavs bug (for example the reference simulator reports upper-case names); judge each hunk, and file an issue for real ones | VHPI leaves details to the tool |

| A third reference (`--refs sim3`, licence in `LM_LICENSE_FILE`) is run too: `vopt -access=rw+/.` for the design, the plugin loaded by its startup table (no function name), trace lines are prefixed `# `; its match count with yavs is printed, no table | Puts need full design access; sim3 refuses most callback reasons and some puts, so it is a vote, not a template |
| A second reference (`--refs sim4`, configured by keys `vhpi_analyse` and `vhpi_run` in `tools/sims.local.conf`; its licence variable must be set) gives a three-way table: each yavs line is `both` (matches the two references), `sim1`, `sim4` or `neither`, after case folding | A line only yavs has is a candidate gap; where the references differ from each other it is a reference quirk |
| The plugins load the second reference through the exported `conf_boot` function (`PLUGIN_INIT` defines it) and look signals up in the first region below the root when the root is a wrapper | That reference has a dummy root above the design and loads by function name |

| A design with several cases names one top per case in `RUNS` (`(design, top)` pairs); the trace files are then named after the top | A case that crashes a reference must not hide the others |
| Plugins that register foreign models are listed in `FOREIGN`: their designs name the plugin's file in the `'FOREIGN` library word, and the second reference loads the plugin from the run directory through that word (running its startup routines), not with its load option | That reference loads a foreign model's library itself; loading the plugin twice registers everything twice |

## 4. Pitfalls

- Plugin output of the reference simulator comes as `** Note: <time>: T ...` on stderr; `run.py`
  strips the prefix.
- The reference simulator needs a library directory ending in `.08` and the `work:` form.
- The `.three` tables align by longest common subsequence: a missing line shifts the rest, so
  read the traces side by side before trusting a `neither` run. Many `neither` lines are yavs
  being more complete than both references (properties they report as -1).
- A plugin that registers repeating callbacks without finishing relies on the design ending; keep
  designs finite (the run is killed after 120 s).

## 5. How to work here

```sh
YAVS_BIN=path/to/yavs tools/vhpi-conformance/run.py [--only walk,put] [--refs sim4] [--show]
```

Exit status 1 when any trace differs. Add a plugin: write `plugins/<name>.c`, add it to `RUNS`.
