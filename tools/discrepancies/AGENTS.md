# AGENTS.md: tools/discrepancies/ (simulator discrepancy runner)

Read the root `AGENTS.md` and `tools/AGENTS.md` first. The catalogue is
`docs/design/simulator-discrepancies.md`; the cases are `tests/discrepancies/`.

## 1. Purpose and boundaries

- `run.py` runs every case on yavs and on the simulators configured by label (`sim1`, `sim2`, ...),
  prints one verdict row per case and revision, and with `--check` fails when yavs differs from
  `expected-yavs` or a recorded verdict in a case's `verdicts` file drifts.
- Python 3 stdlib only.
- Not here: the analysis corpora (`tools/corpus`) and the differential corpus (`tools/difftest`).

## 2. Map

| File | What |
|---|---|
| `run.py` | Runner: `TEMPLATES` (command templates keyed by command name), config loading, verdict classification, `--record`, `--bless-yavs`, `--check` |
| `../sims.local.conf` | Untracked, git-ignored (`tools/simconf.py`): key `discrep = analyse|||elaborate-and-run[|||crash regex]` in each label's section (or `DISCREP_SIM<N>` variables) |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| The label to tool mapping is never committed; command names appear only as `TEMPLATES` keys | Root rule 11 (no other project names in docs) | user, 2026-10-19 |
| A verdict is `REJECT`, `OK`, `OK*`, `CRASH`, `FATAL` or `TIMEOUT`; the observable result of a runnable case is its `report` messages, compared order-insensitively for simulators and exactly for yavs | Simulators order same-cycle reports differently | `docs/design/simulator-discrepancies.md` |
| Runs are sequential by default, at most 2 parallel, each under `timeout 300 systemd-run --user --scope -p MemoryMax=2G` | Root rule 15 | AGENTS.md |

## 4. Invariants and pitfalls

- A licensed simulator template refers to its licence only through an environment variable
  reference; never print or write its value.
- `--bless-yavs` writes `expected-yavs` from yavs's own output: review it against the LRM first.
- A case file must end by itself (no free-running clocks): there is no stop-time option.
- Cache: `~/.cache/yavs-discrep` (`DISCREP_CACHE`).

## 5. How to work here

```sh
YAVS_BIN=$PWD/target/debug/yavs python3 tools/discrepancies/run.py --sims sim1,sim2,sim3,sim4 --check
python3 tools/discrepancies/run.py --filter open-port --bless-yavs
```
