# vhdl-compliance-2008: NVC validation

nvc 1.23-devel (1.22.0.r66.gef5084a94) (Using LLVM 21.1.8)

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/nvc-vhdl-compliance-2008` with `tools/corpus/run.py vhdl-compliance-2008 --tool nvc --bless` (one file per call, `--std=2008`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 171; accepted by NVC: 171; rejected by NVC: 0. The manifest lists only files the reference tools accept (NVC and GHDL, Questa as tiebreaker); the rest are under Excluded.
- Flags needed: none.
- The first 143 entries are the vunit-2008 library files the test benches need (vunit_lib, OSVVM); the project's own files follow.
- Licenses: vhdl-compliance: Apache-2.0.

## Excluded by validation (1 files; 1 direct, 0 cascades of a missing unit in a later round)

- 1 x NVC and GHDL disagree (nvc=ok, ghdl=FAIL), Questa rejects: ghdl: error: matching a formal generic package with an explicit generic map aspect is not s (e.g. `vhdl_2008/tb_generic_packages_on_entity.vhd`)
