# fletcher-2008: NVC validation

nvc 1.23-devel (1.22.0.r66.gef5084a94) (Using LLVM 21.1.8)

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/nvc-fletcher-2008` with `tools/corpus/run.py fletcher-2008 --tool nvc --bless` (one file per call, `--std=2008`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 101; accepted by NVC: 101; rejected by NVC: 0. The manifest lists only files the reference tools accept (NVC and GHDL, Questa as tiebreaker); the rest are under Excluded.
- Flags needed: none.
- Licenses: fletcher: Apache-2.0; vhlib: Apache-2.0.

## Excluded by validation (10 files; 1 direct, 9 cascades of a missing unit in a later round)

- 1 x NVC and GHDL disagree (nvc=FAIL, ghdl=ok), Questa rejects: NVC: ** Error: parameters of protected type methods cannot be of an access type or a compos (e.g. `sim/SimDataComms_pkg.sim.08.vhd`)
