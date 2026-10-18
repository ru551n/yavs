# vunit-tests-2019: NVC validation

nvc 1.23-devel (1.22.0.r66.gef5084a94) (Using LLVM 21.1.8)

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/nvc-vunit-tests-2019` with `tools/corpus/run.py vunit-tests-2019 --tool nvc --bless` (one file per call, `--std=2019`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 218; accepted by NVC: 218; rejected by NVC: 0. The manifest lists only files the reference tools accept (NVC, Questa as second witness); the rest are under Excluded.
- Flags needed: none.
- The first 143 entries are the vunit-2019 library files the test benches need (vunit_lib, OSVVM); the project's own files follow.
- Licenses: vunit: MPL-2.0.

## Excluded by validation (40 files; 40 direct, 0 cascades of a missing unit in a later round)

- 40 x uses a design unit or library that is missing: its own file is excluded or it is a vendor library (cascade) (e.g. `examples/vhdl/uart/src/test/tb_uart_rx.vhd`)
