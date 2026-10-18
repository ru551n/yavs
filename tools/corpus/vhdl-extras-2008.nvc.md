# vhdl-extras-2008: NVC validation

nvc 1.23-devel (1.22.0.r66.gef5084a94) (Using LLVM 21.1.8)

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/nvc-vhdl-extras-2008` with `tools/corpus/run.py vhdl-extras-2008 --tool nvc --bless` (one file per call, `--std=2008`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 25; accepted by NVC: 25; rejected by NVC: 0. The manifest lists only files the reference tools accept (NVC and GHDL, Questa as tiebreaker); the rest are under Excluded.
- Flags needed: none.
- Licenses: vhdl-extras: MIT.

## Excluded by validation (53 files; 53 direct, 0 cascades of a missing unit in a later round)

- 50 x uses a design unit or library that is missing: its own file is excluded or it is a vendor library (cascade) (e.g. `rtl/extras/bcd_conversion.vhdl`)
- 1 x rejected by the reference tools: NVC: ** Error: homograph of TO_GRAY [STD_LOGIC_VECTOR return STD_LOGIC_VECTOR] already declared in this region; ghdl: (e.g. `rtl/extras/gray_code.vhdl`)
- 1 x rejected by the reference tools: NVC: ** Error: homograph of CHANGE_SIZE [STD_LOGIC_VECTOR, POSITIVE, RESIZE_METHOD, STD_ULOGIC return STD_LOGIC_VECTO (e.g. `rtl/extras/sizing.vhdl`)
- 1 x NVC and GHDL disagree (nvc=ok, ghdl=FAIL), Questa rejects: ghdl: error: interface declaration expected (e.g. `rtl/extras/strings_maps.vhdl`)

## Tool disagreements

- disagree rtl/extras/pipelining.vhdl: nvc=ok ghdl=FAIL questa=ok (ghdl: pipelining.vhdl:314:13:error: attribute spec not in the same immediate scope as port "Sig_out")
- disagree rtl/extras/arithmetic.vhdl: nvc=ok ghdl=FAIL questa=ok (ghdl: arithmetic.vhdl:100:12:error: unit "pipelining" not found in library "extras")
- disagree rtl/extras/cordic.vhdl: nvc=ok ghdl=FAIL questa=ok (ghdl: cordic.vhdl:551:12:error: unit "pipelining" not found in library "extras")
- disagree rtl/extras_2008/pipelining_2008.vhdl: nvc=ok ghdl=FAIL questa=ok (ghdl: pipelining_2008.vhdl:410:13:error: attribute spec not in the same immediate scope as port "Sig_out")
