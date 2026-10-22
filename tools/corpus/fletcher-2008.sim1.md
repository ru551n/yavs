# fletcher-2008: reference simulator validation

Reference simulator sim1 1.23-devel (1.22.0.r66.gef5084a94)

sim1 is the primary reference simulator, sim2 the second and sim3 the third; the roles are fixed by the corpus tooling.

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/sim1-fletcher-2008` with `tools/corpus/run.py fletcher-2008 --tool sim1 --bless` (one file per call, `--std=2008`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 101; accepted by sim1: 101; rejected by sim1: 0. The manifest lists only files the reference tools accept (sim1 and sim2, sim3 as tiebreaker); the rest are under Excluded.
- Flags needed: none.
- Licenses: fletcher: Apache-2.0; vhlib: Apache-2.0.

## Excluded by validation (10 files; 1 direct, 9 cascades of a missing unit in a later round)

- 1 x sim1 and sim2 disagree (sim1=FAIL, sim2=ok), sim3 rejects: sim1: ** Error: parameters of protected type methods cannot be of an access type or a compos (e.g. `sim/SimDataComms_pkg.sim.08.vhd`)
