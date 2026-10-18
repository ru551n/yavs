# vhdl-ideas-2019: NVC validation

nvc 1.23-devel (1.22.0.r66.gef5084a94) (Using LLVM 21.1.8)

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/nvc-vhdl-ideas-2019` with `tools/corpus/run.py vhdl-ideas-2019 --tool nvc --bless` (one file per call, `--std=2019`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 19; accepted by NVC: 18; rejected by NVC: 1. The manifest lists only files the reference tools accept (NVC, Questa as second witness); the rest are under Excluded.
- Flags needed: none.
- Licenses: vhdl-ideas: MIT.

## Excluded by validation (25 files; 25 direct, 0 cascades of a missing unit in a later round)

- 9 x uses a design unit or library that is missing: its own file is excluded or it is a vendor library (cascade) (e.g. `bfm/axi4mm_bfm.vhdl`)
- 4 x rejected by NVC and Questa: NVC: ** Error: unexpected generic while parsing protected type declarative item, expecting one of attribute, use, function (e.g. `poc/cpu.vhdl`)
- 1 x rejected by NVC and Questa: NVC: ** Error: no visible declaration for CONTROLLER; questa: ** Error: (10): (vcom-1136) Unknown identifier "_". (e.g. `bfm/i2c_bfm.vhdl`)
- 1 x rejected by NVC and Questa: NVC: ** Error: subprogram BURST_LENGTHS [return BURST_LENGTHS_T] called before its body has been elaborated; questa: ** Er (e.g. `interfaces/axi/axi4mm.vhdl`)
- 1 x rejected by NVC and Questa: NVC: ** Error: declaration of AXIS_T is hidden; questa: ** Error: (35): near "_": (vcom-1576) expecting NEW. (e.g. `interfaces/axi/axi4s.vhdl`)
- 1 x rejected by NVC and Questa: NVC: ** Error: subprogram body is not allowed in package specification; questa: ** Error: (5): Subprogram body 'log' not a (e.g. `poc/log.vhdl`)
- 1 x rejected by NVC and Questa: NVC: ** Error: subprogram RAND declaration has 15 generics but body has 14; questa: ** Error: (17): near "_": (vcom-1576) (e.g. `random/rand.vhdl`)
- 1 x rejected by NVC and Questa: NVC: ** Error: shared variable ARR must have protected type; questa: ** Error: (vcom-11) Could not find std.reflection. Ma (e.g. `sim/mirror.vhdl`)
- 1 x rejected by NVC and Questa: NVC: ** Error: subtype of generic "_" does not match type "_" [anonymous, anonymous return BOOLEAN] in specification; ques (e.g. `synth/range.vhdl`)
- 1 x rejected by NVC and Questa: NVC: ** Error: type A does not have a range record; questa: ** Error: (3): (vcom-1136) Unknown identifier "_". (e.g. `util/base64.vhdl`)
- 1 x rejected by NVC and Questa: NVC: ** Error: type mark of return identifier must denote an unconstrained type; questa: ** Error: (6): (vcom-1136) Unknow (e.g. `util/bigint.vhdl`)
- 1 x rejected by NVC and Questa: NVC: ** Error: object prefix of attribute RANGE must be an array; questa: ** Error: (3): near "_": (vcom-1576) expecting ' (e.g. `util/generic_demux.vhdl`)
- 1 x rejected by NVC and Questa: NVC: ** Error: type (an anonymous type) does not have a range record; questa: ** Error: (5): near "_": (vcom-1576) expecti (e.g. `util/range.vhdl`)
- 1 x rejected by NVC and Questa: NVC: ** Error: type mark of return identifier must denote an unconstrained type; questa: ** Error: (vcom-11) Could not fin (e.g. `util/util.vhdl`)

## Kept although NVC rejects them (GHDL and Questa accept)

- `vhdl-ideas/sim/frequency.vhdl`

## Tool disagreements

- kept interfaces/axi/axi4-body.vhdl: nvc=ok questa=FAIL (questa: ** Error: (vcom-11) Could not find std.reflection. Make sure the top design unit is compiled into the correct library or that the correct library is being referenced.)
- kept interfaces/ethernet/caui.vhdl: nvc=ok questa=FAIL (questa: ** Error (suppressible): caui.vhdl(1): (vcom-1491) Empty source files.)
- kept interfaces/ethernet/mii.vhdl: nvc=ok questa=FAIL (questa: ** Error (suppressible): mii.vhdl(1): (vcom-1491) Empty source files.)
- kept interfaces/ethernet/qsfp.vhdl: nvc=ok questa=FAIL (questa: ** Error (suppressible): qsfp.vhdl(1): (vcom-1491) Empty source files.)
- kept interfaces/ethernet/rgmii.vhdl: nvc=ok questa=FAIL (questa: ** Error (suppressible): rgmii.vhdl(1): (vcom-1491) Empty source files.)
- kept interfaces/ethernet/rmii.vhdl: nvc=ok questa=FAIL (questa: ** Error (suppressible): rmii.vhdl(1): (vcom-1491) Empty source files.)
- kept interfaces/ethernet/sfp.vhdl: nvc=ok questa=FAIL (questa: ** Error (suppressible): sfp.vhdl(1): (vcom-1491) Empty source files.)
- kept interfaces/ethernet/sgmii.vhdl: nvc=ok questa=FAIL (questa: ** Error (suppressible): sgmii.vhdl(1): (vcom-1491) Empty source files.)
- kept interfaces/ethernet/xaui.vhdl: nvc=ok questa=FAIL (questa: ** Error (suppressible): xaui.vhdl(1): (vcom-1491) Empty source files.)
- kept interfaces/io.vhdl: nvc=ok questa=FAIL (questa: ** Error: io.vhdl(13): near "type": (vcom-1576) expecting NEW.)
- kept interfaces/adi/ad9361.vhdl: nvc=ok questa=FAIL (questa: ** Error: (vcom-11) Could not find work.io. Make sure the top design unit is compiled into the correct library or that the correct library is being referenced.)
- kept interfaces/pcie.vhdl: nvc=ok questa=FAIL (questa: ** Error: (vcom-11) Could not find work.io. Make sure the top design unit is compiled into the correct library or that the correct library is being referenced.)
- kept poc/rangetest.vhdl: nvc=ok questa=FAIL (questa: ** Error: rangetest.vhdl(11): near "is": (vcom-1576) expecting ';' or ')'.)
- kept sim/frequency.vhdl: nvc=FAIL questa=ok (NVC: ** Error: homograph of WORK.FREQUENCY.GENERATE_CLOCK [STD_LOGIC, FREQUENCY, NATURAL] already declared in this region)
- kept util/generic_mux.vhdl: nvc=ok questa=FAIL (questa: ** Error: generic_mux.vhdl(3): near "is": (vcom-1576) expecting ';' or ')'.)
- kept poc/hdmi_switch.vhdl: nvc=ok questa=FAIL (questa: ** Error: (vcom-11) Could not find work.generic_mux. Make sure the top design unit is compiled into the correct library or that the correct library is being referenced.)
- kept util/memory.vhdl: nvc=ok questa=FAIL (questa: ** Error: memory.vhdl(4): Prefix of attribute "range" must be appropriate for an array object or must denote an array subtype.)
