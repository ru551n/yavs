# vhdl-tool-checks-2008: reference simulator validation

Reference simulator sim1 1.23-devel (1.22.0.r66.gef5084a94)

sim1 is the primary reference simulator, sim2 the second and sim3 the third; the roles are fixed by the corpus tooling.

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/sim1-vhdl-tool-checks-2008` with `tools/corpus/run.py vhdl-tool-checks-2008 --tool sim1 --bless` (one file per call, `--std=2008`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 1; accepted by sim1: 1; rejected by sim1: 0. The manifest lists only files the reference tools accept (sim1 and sim2, sim3 as tiebreaker); the rest are under Excluded.
- Flags needed: none.
- Licenses: vhdl-tool-checks: Apache-2.0.
