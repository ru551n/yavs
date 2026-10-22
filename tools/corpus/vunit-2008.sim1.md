# vunit-2008: reference simulator validation

Reference simulator sim1 1.23-devel (1.22.0.r66.gef5084a94)

sim1 is the primary reference simulator, sim2 the second and sim3 the third; the roles are fixed by the corpus tooling.

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/sim1-vunit-2008` with `tools/corpus/run.py vunit-2008 --tool sim1 --bless` (one file per call, `--std=2008`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 143; accepted: 143; rejected: 0.
- Flags needed: none.

No rejected files. Result list: `vunit-2008.sim1.expected`.
