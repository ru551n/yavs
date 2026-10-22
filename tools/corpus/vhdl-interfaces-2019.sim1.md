# vhdl-interfaces-2019: reference simulator validation

Reference simulator sim1 1.23-devel (1.22.0.r66.gef5084a94)

sim1 is the primary reference simulator, sim2 the second and sim3 the third; the roles are fixed by the corpus tooling.

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/sim1-vhdl-interfaces-2019` with `tools/corpus/run.py vhdl-interfaces-2019 --tool sim1 --bless` (one file per call, `--std=2019`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 34; accepted by sim1: 34; rejected by sim1: 0. The manifest lists only files the reference tools accept (sim1, sim3 as second witness); the rest are under Excluded.
- Flags needed: none.
- Licenses: vhdl-interfaces: Apache-2.0.

## Tool disagreements

- kept PoC/CSE.vhdl: sim1=ok sim3=FAIL (sim3: ** Error: CSE.vhdl(30): near "is": (sim3-1576) expecting ';' or ')'.)
