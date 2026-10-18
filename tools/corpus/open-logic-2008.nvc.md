# open-logic-2008: NVC validation

nvc 1.23-devel (1.22.0.r66.gef5084a94) (Using LLVM 21.1.8)

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/nvc-open-logic-2008` with `tools/corpus/run.py open-logic-2008 --tool nvc --bless` (one file per call, `--std=2008`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 386; accepted by NVC: 386; rejected by NVC: 0. The manifest lists only files the reference tools accept (NVC and GHDL, Questa as tiebreaker); the rest are under Excluded.
- Flags needed: --relaxed.
- The first 143 entries are the vunit-2008 library files the test benches need (vunit_lib, OSVVM); the project's own files follow.
- Licenses: open-logic: LGPL-2.1 with the PSI HDL library FPGA exception; en_cl_fix: MIT.

## Excluded by validation (1 files; 1 direct, 0 cascades of a missing unit in a later round)

- 1 x uses a design unit or library that is missing: its own file is excluded or it is a vendor library (cascade) (e.g. `test/fix/olo_fix_pkg_writer/olo_fix_pkg_writer_tb.vhd`)
