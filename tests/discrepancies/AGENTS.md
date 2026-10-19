# AGENTS.md: tests/discrepancies/

Read the root `AGENTS.md` and `tests/AGENTS.md` first. One directory per catalogued simulator
discrepancy (`docs/design/simulator-discrepancies.md`), run by `tools/discrepancies/run.py`.

| File | What |
|---|---|
| `<id>/case.vhd` | Minimal reproducer, one file; the result is in `report` messages (prefix `RES`) |
| `<id>/case.conf` | `top`, `std` (one or more revisions), `mode = run` or `analyze`, optional `res` (resolution limit, default `fs`; simulators that cannot select one run at their default and are not recorded for that case) |
| `<id>/expected-yavs` | Per revision (`== 2008`): yavs's expected messages, or `REJECT` |
| `<id>/verdicts` | Recorded verdicts per simulator label and revision (`sim1 2008 REJECT`) |

Rules: write fresh reproducers (some were reduced from the differential corpus or the negative
sem suite); no simulator or project names in files (use `sim1`, `sim2`, ...); a new case needs a
row in the catalogue in the same commit; the case must end by itself.
