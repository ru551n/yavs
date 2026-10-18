# vhdl-tool-checks-2019: NVC validation

nvc 1.23-devel (1.22.0.r66.gef5084a94) (Using LLVM 21.1.8)

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/nvc-vhdl-tool-checks-2019` with `tools/corpus/run.py vhdl-tool-checks-2019 --tool nvc --bless` (one file per call, `--std=2019`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 1; accepted by NVC: 1; rejected by NVC: 0. The manifest lists only files the reference tools accept (NVC, Questa as second witness); the rest are under Excluded.
- Flags needed: none.
- Licenses: vhdl-tool-checks: Apache-2.0.

## Tool disagreements

- kept Ranges.2019.vhdl: nvc=ok questa=FAIL (questa: ** Error: Ranges.2019.vhdl(10): near "9223372036854775807": (vcom-119) Integer value exceeds INTEGER'high.)
