# vhdl-compliance-2008: reference simulator validation

Reference simulator sim1 1.23-devel (1.22.0.r66.gef5084a94)

sim1 is the primary reference simulator, sim2 the second and sim3 the third; the roles are fixed by the corpus tooling.

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/sim1-vhdl-compliance-2008` with `tools/corpus/run.py vhdl-compliance-2008 --tool sim1 --bless` (one file per call, `--std=2008`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 171; accepted by sim1: 171; rejected by sim1: 0. The manifest lists only files the reference tools accept (sim1 and sim2, sim3 as tiebreaker); the rest are under Excluded.
- Flags needed: none.
- The first 143 entries are the vunit-2008 library files the test benches need (vunit_lib, OSVVM); the project's own files follow.
- Licenses: vhdl-compliance: Apache-2.0.

## Excluded by validation (1 files; 1 direct, 0 cascades of a missing unit in a later round)

- 1 x sim1 and sim2 disagree (sim1=ok, sim2=FAIL), sim3 rejects: sim2: error: matching a formal generic package with an explicit generic map aspect is not s (e.g. `vhdl_2008/tb_generic_packages_on_entity.vhd`)
