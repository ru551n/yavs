# json-for-vhdl-2008: NVC validation

nvc 1.23-devel (1.22.0.r66.gef5084a94) (Using LLVM 21.1.8)

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/nvc-json-for-vhdl-2008` with `tools/corpus/run.py json-for-vhdl-2008 --tool nvc --bless` (one file per call, `--std=2008`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 149; accepted by NVC: 149; rejected by NVC: 0. The manifest lists only files the reference tools accept (NVC and GHDL, Questa as tiebreaker); the rest are under Excluded.
- Flags needed: none.
- The first 143 entries are the vunit-2008 library files the test benches need (vunit_lib, OSVVM); the project's own files follow.
- Licenses: json-for-vhdl: Apache-2.0.

## Excluded by validation (3 files; 3 direct, 0 cascades of a missing unit in a later round)

- 2 x uses a design unit or library that is missing: its own file is excluded or it is a vendor library (cascade) (e.g. `examples/Boards_VUnit.vhdl`)
- 1 x rejected by the reference tools: NVC: ** Error: no visible declaration for JSONLOADFILE; ghdl: error: no declaration for "_" (e.g. `tests/JSONTestSuite/TopLevel.vhdl`)
