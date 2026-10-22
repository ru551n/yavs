# vhdl-ideas-2019: reference simulator validation

Reference simulator sim1 1.23-devel (1.22.0.r66.gef5084a94)

sim1 is the primary reference simulator, sim2 the second and sim3 the third; the roles are fixed by the corpus tooling.

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/sim1-vhdl-ideas-2019` with `tools/corpus/run.py vhdl-ideas-2019 --tool sim1 --bless` (one file per call, `--std=2019`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 19; accepted by sim1: 18; rejected by sim1: 1. The manifest lists only files the reference tools accept (sim1, sim3 as second witness); the rest are under Excluded.
- Flags needed: none.
- Licenses: vhdl-ideas: MIT.

## Excluded by validation (25 files; 25 direct, 0 cascades of a missing unit in a later round)

- 9 x uses a design unit or library that is missing: its own file is excluded or it is a vendor library (cascade) (e.g. `bfm/axi4mm_bfm.vhdl`)
- 4 x rejected by sim1 and sim3: sim1: ** Error: unexpected generic while parsing protected type declarative item, expecting one of attribute, use, function (e.g. `poc/cpu.vhdl`)
- 1 x rejected by sim1 and sim3: sim1: ** Error: no visible declaration for CONTROLLER; sim3: ** Error: (10): (sim3-1136) Unknown identifier "_". (e.g. `bfm/i2c_bfm.vhdl`)
- 1 x rejected by sim1 and sim3: sim1: ** Error: subprogram BURST_LENGTHS [return BURST_LENGTHS_T] called before its body has been elaborated; sim3: ** Er (e.g. `interfaces/axi/axi4mm.vhdl`)
- 1 x rejected by sim1 and sim3: sim1: ** Error: declaration of AXIS_T is hidden; sim3: ** Error: (35): near "_": (sim3-1576) expecting NEW. (e.g. `interfaces/axi/axi4s.vhdl`)
- 1 x rejected by sim1 and sim3: sim1: ** Error: subprogram body is not allowed in package specification; sim3: ** Error: (5): Subprogram body 'log' not a (e.g. `poc/log.vhdl`)
- 1 x rejected by sim1 and sim3: sim1: ** Error: subprogram RAND declaration has 15 generics but body has 14; sim3: ** Error: (17): near "_": (sim3-1576) (e.g. `random/rand.vhdl`)
- 1 x rejected by sim1 and sim3: sim1: ** Error: shared variable ARR must have protected type; sim3: ** Error: (sim3-11) Could not find std.reflection. Ma (e.g. `sim/mirror.vhdl`)
- 1 x rejected by sim1 and sim3: sim1: ** Error: subtype of generic "_" does not match type "_" [anonymous, anonymous return BOOLEAN] in specification; ques (e.g. `synth/range.vhdl`)
- 1 x rejected by sim1 and sim3: sim1: ** Error: type A does not have a range record; sim3: ** Error: (3): (sim3-1136) Unknown identifier "_". (e.g. `util/base64.vhdl`)
- 1 x rejected by sim1 and sim3: sim1: ** Error: type mark of return identifier must denote an unconstrained type; sim3: ** Error: (6): (sim3-1136) Unknow (e.g. `util/bigint.vhdl`)
- 1 x rejected by sim1 and sim3: sim1: ** Error: object prefix of attribute RANGE must be an array; sim3: ** Error: (3): near "_": (sim3-1576) expecting ' (e.g. `util/generic_demux.vhdl`)
- 1 x rejected by sim1 and sim3: sim1: ** Error: type (an anonymous type) does not have a range record; sim3: ** Error: (5): near "_": (sim3-1576) expecti (e.g. `util/range.vhdl`)
- 1 x rejected by sim1 and sim3: sim1: ** Error: type mark of return identifier must denote an unconstrained type; sim3: ** Error: (sim3-11) Could not fin (e.g. `util/util.vhdl`)

## Kept although sim1 rejects them (sim2 and sim3 accept)

- `vhdl-ideas/sim/frequency.vhdl`

## Tool disagreements

- kept interfaces/axi/axi4-body.vhdl: sim1=ok sim3=FAIL (sim3: ** Error: (sim3-11) Could not find std.reflection. Make sure the top design unit is compiled into the correct library or that the correct library is being referenced.)
- kept interfaces/ethernet/caui.vhdl: sim1=ok sim3=FAIL (sim3: ** Error (suppressible): caui.vhdl(1): (sim3-1491) Empty source files.)
- kept interfaces/ethernet/mii.vhdl: sim1=ok sim3=FAIL (sim3: ** Error (suppressible): mii.vhdl(1): (sim3-1491) Empty source files.)
- kept interfaces/ethernet/qsfp.vhdl: sim1=ok sim3=FAIL (sim3: ** Error (suppressible): qsfp.vhdl(1): (sim3-1491) Empty source files.)
- kept interfaces/ethernet/rgmii.vhdl: sim1=ok sim3=FAIL (sim3: ** Error (suppressible): rgmii.vhdl(1): (sim3-1491) Empty source files.)
- kept interfaces/ethernet/rmii.vhdl: sim1=ok sim3=FAIL (sim3: ** Error (suppressible): rmii.vhdl(1): (sim3-1491) Empty source files.)
- kept interfaces/ethernet/sfp.vhdl: sim1=ok sim3=FAIL (sim3: ** Error (suppressible): sfp.vhdl(1): (sim3-1491) Empty source files.)
- kept interfaces/ethernet/sgmii.vhdl: sim1=ok sim3=FAIL (sim3: ** Error (suppressible): sgmii.vhdl(1): (sim3-1491) Empty source files.)
- kept interfaces/ethernet/xaui.vhdl: sim1=ok sim3=FAIL (sim3: ** Error (suppressible): xaui.vhdl(1): (sim3-1491) Empty source files.)
- kept interfaces/io.vhdl: sim1=ok sim3=FAIL (sim3: ** Error: io.vhdl(13): near "type": (sim3-1576) expecting NEW.)
- kept interfaces/adi/ad9361.vhdl: sim1=ok sim3=FAIL (sim3: ** Error: (sim3-11) Could not find work.io. Make sure the top design unit is compiled into the correct library or that the correct library is being referenced.)
- kept interfaces/pcie.vhdl: sim1=ok sim3=FAIL (sim3: ** Error: (sim3-11) Could not find work.io. Make sure the top design unit is compiled into the correct library or that the correct library is being referenced.)
- kept poc/rangetest.vhdl: sim1=ok sim3=FAIL (sim3: ** Error: rangetest.vhdl(11): near "is": (sim3-1576) expecting ';' or ')'.)
- kept sim/frequency.vhdl: sim1=FAIL sim3=ok (sim1: ** Error: homograph of WORK.FREQUENCY.GENERATE_CLOCK [STD_LOGIC, FREQUENCY, NATURAL] already declared in this region)
- kept util/generic_mux.vhdl: sim1=ok sim3=FAIL (sim3: ** Error: generic_mux.vhdl(3): near "is": (sim3-1576) expecting ';' or ')'.)
- kept poc/hdmi_switch.vhdl: sim1=ok sim3=FAIL (sim3: ** Error: (sim3-11) Could not find work.generic_mux. Make sure the top design unit is compiled into the correct library or that the correct library is being referenced.)
- kept util/memory.vhdl: sim1=ok sim3=FAIL (sim3: ** Error: memory.vhdl(4): Prefix of attribute "range" must be appropriate for an array object or must denote an array subtype.)
