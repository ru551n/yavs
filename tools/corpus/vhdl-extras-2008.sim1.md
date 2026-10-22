# vhdl-extras-2008: reference simulator validation

Reference simulator sim1 1.23-devel (1.22.0.r66.gef5084a94)

sim1 is the primary reference simulator, sim2 the second and sim3 the third; the roles are fixed by the corpus tooling.

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/sim1-vhdl-extras-2008` with `tools/corpus/run.py vhdl-extras-2008 --tool sim1 --bless` (one file per call, `--std=2008`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 25; accepted by sim1: 25; rejected by sim1: 0. The manifest lists only files the reference tools accept (sim1 and sim2, sim3 as tiebreaker); the rest are under Excluded.
- Flags needed: none.
- Licenses: vhdl-extras: MIT.

## Excluded by validation (53 files; 53 direct, 0 cascades of a missing unit in a later round)

- 50 x uses a design unit or library that is missing: its own file is excluded or it is a vendor library (cascade) (e.g. `rtl/extras/bcd_conversion.vhdl`)
- 1 x rejected by the reference tools: sim1: ** Error: homograph of TO_GRAY [STD_LOGIC_VECTOR return STD_LOGIC_VECTOR] already declared in this region; sim2: (e.g. `rtl/extras/gray_code.vhdl`)
- 1 x rejected by the reference tools: sim1: ** Error: homograph of CHANGE_SIZE [STD_LOGIC_VECTOR, POSITIVE, RESIZE_METHOD, STD_ULOGIC return STD_LOGIC_VECTO (e.g. `rtl/extras/sizing.vhdl`)
- 1 x sim1 and sim2 disagree (sim1=ok, sim2=FAIL), sim3 rejects: sim2: error: interface declaration expected (e.g. `rtl/extras/strings_maps.vhdl`)

## Tool disagreements

- disagree rtl/extras/pipelining.vhdl: sim1=ok sim2=FAIL sim3=ok (sim2: pipelining.vhdl:314:13:error: attribute spec not in the same immediate scope as port "Sig_out")
- disagree rtl/extras/arithmetic.vhdl: sim1=ok sim2=FAIL sim3=ok (sim2: arithmetic.vhdl:100:12:error: unit "pipelining" not found in library "extras")
- disagree rtl/extras/cordic.vhdl: sim1=ok sim2=FAIL sim3=ok (sim2: cordic.vhdl:551:12:error: unit "pipelining" not found in library "extras")
- disagree rtl/extras_2008/pipelining_2008.vhdl: sim1=ok sim2=FAIL sim3=ok (sim2: pipelining_2008.vhdl:410:13:error: attribute spec not in the same immediate scope as port "Sig_out")
