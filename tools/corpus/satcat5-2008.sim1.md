# satcat5-2008: reference simulator validation

Reference simulator sim1 1.23-devel (1.22.0.r66.gef5084a94)

sim1 is the primary reference simulator, sim2 the second and sim3 the third; the roles are fixed by the corpus tooling.

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/sim1-satcat5-2008` with `tools/corpus/run.py satcat5-2008 --tool sim1 --bless` (one file per call, `--std=2008`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 246; accepted by sim1: 246; rejected by sim1: 0. The manifest lists only files the reference tools accept (sim1 and sim2, sim3 as tiebreaker); the rest are under Excluded.
- Flags needed: none.
- Licenses: satcat5: CERN-OHL-W-2.0.

## Excluded by validation (70 files; 70 direct, 0 cascades of a missing unit in a later round)

- 65 x uses a design unit or library that is missing: its own file is excluded or it is a vendor library (cascade) (e.g. `sim/vhdl/cfgbus_hdlc_tb.vhd`)
- 1 x rejected by the reference tools: sim1: ** Error: shared variable RCVD_PKTS must have protected type; sim2: error: type of a shared variable must be a p (e.g. `sim/vhdl/mac_log_validate.vhd`)
- 1 x rejected by the reference tools: sim1: ** Error: shared variable FIFO_DATA must have protected type; sim2: error: type of a shared variable must be a p (e.g. `sim/vhdl/port_test_common.vhd`)
- 1 x rejected by the reference tools: sim1: ** Error: shared variable RS_SEED1 must have protected type; sim2: error: type of a shared variable must be a pr (e.g. `sim/vhdl/router_sim_tools.vhd`)
- 1 x rejected by the reference tools: sim1: ** Error: shared variable AUX_STATS must have protected type; sim2: error: type of a shared variable must be a p (e.g. `sim/vhdl/sgmii_data_sync_tb.vhd`)
- 1 x rejected by the reference tools: sim1: ** Error: shared variable TEST_FIXED must have protected type; sim2: error: type of a shared variable must be a (e.g. `sim/vhdl/tcam_cache_tb.vhd`)

## Tool disagreements

- disagree src/vhdl/common/mac_log_core.vhd: sim1=ok sim2=FAIL sim3=ok (sim2: mac_log_core.vhd:147:11:error: attribute spec not in the same immediate scope as port "port_data")
- disagree src/vhdl/common/mac_log_cfgbus.vhd: sim1=ok sim2=FAIL sim3=ok (sim2: mac_log_cfgbus.vhd:60:21:error: unit "mac_log_core" not found in library "work")
- disagree src/vhdl/common/mac_log_uart.vhd: sim1=ok sim2=FAIL sim3=ok (sim2: mac_log_uart.vhd:60:21:error: unit "mac_log_core" not found in library "work")
- disagree src/vhdl/common/router2_core.vhd: sim1=ok sim2=FAIL sim3=ok (sim2: router2_core.vhd:500:32:error: unit "mac_log_cfgbus" not found in library "work")
- disagree src/vhdl/common/switch_core.vhd: sim1=ok sim2=FAIL sim3=ok (sim2: switch_core.vhd:593:32:error: unit "mac_log_cfgbus" not found in library "work")
- disagree sim/vhdl/switch_core_tb.vhd: sim1=ok sim2=FAIL sim3=ok (sim2: switch_core_tb.vhd:174:19:error: unit "switch_core" not found in library "work")
