# satcat5-2008: NVC validation

nvc 1.23-devel (1.22.0.r66.gef5084a94) (Using LLVM 21.1.8)

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/nvc-satcat5-2008` with `tools/corpus/run.py satcat5-2008 --tool nvc --bless` (one file per call, `--std=2008`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 246; accepted by NVC: 246; rejected by NVC: 0. The manifest lists only files the reference tools accept (NVC and GHDL, Questa as tiebreaker); the rest are under Excluded.
- Flags needed: none.
- Licenses: satcat5: CERN-OHL-W-2.0.

## Excluded by validation (70 files; 70 direct, 0 cascades of a missing unit in a later round)

- 65 x uses a design unit or library that is missing: its own file is excluded or it is a vendor library (cascade) (e.g. `sim/vhdl/cfgbus_hdlc_tb.vhd`)
- 1 x rejected by the reference tools: NVC: ** Error: shared variable RCVD_PKTS must have protected type; ghdl: error: type of a shared variable must be a p (e.g. `sim/vhdl/mac_log_validate.vhd`)
- 1 x rejected by the reference tools: NVC: ** Error: shared variable FIFO_DATA must have protected type; ghdl: error: type of a shared variable must be a p (e.g. `sim/vhdl/port_test_common.vhd`)
- 1 x rejected by the reference tools: NVC: ** Error: shared variable RS_SEED1 must have protected type; ghdl: error: type of a shared variable must be a pr (e.g. `sim/vhdl/router_sim_tools.vhd`)
- 1 x rejected by the reference tools: NVC: ** Error: shared variable AUX_STATS must have protected type; ghdl: error: type of a shared variable must be a p (e.g. `sim/vhdl/sgmii_data_sync_tb.vhd`)
- 1 x rejected by the reference tools: NVC: ** Error: shared variable TEST_FIXED must have protected type; ghdl: error: type of a shared variable must be a (e.g. `sim/vhdl/tcam_cache_tb.vhd`)

## Tool disagreements

- disagree src/vhdl/common/mac_log_core.vhd: nvc=ok ghdl=FAIL questa=ok (ghdl: mac_log_core.vhd:147:11:error: attribute spec not in the same immediate scope as port "port_data")
- disagree src/vhdl/common/mac_log_cfgbus.vhd: nvc=ok ghdl=FAIL questa=ok (ghdl: mac_log_cfgbus.vhd:60:21:error: unit "mac_log_core" not found in library "work")
- disagree src/vhdl/common/mac_log_uart.vhd: nvc=ok ghdl=FAIL questa=ok (ghdl: mac_log_uart.vhd:60:21:error: unit "mac_log_core" not found in library "work")
- disagree src/vhdl/common/router2_core.vhd: nvc=ok ghdl=FAIL questa=ok (ghdl: router2_core.vhd:500:32:error: unit "mac_log_cfgbus" not found in library "work")
- disagree src/vhdl/common/switch_core.vhd: nvc=ok ghdl=FAIL questa=ok (ghdl: switch_core.vhd:593:32:error: unit "mac_log_cfgbus" not found in library "work")
- disagree sim/vhdl/switch_core_tb.vhd: nvc=ok ghdl=FAIL questa=ok (ghdl: switch_core_tb.vhd:174:19:error: unit "switch_core" not found in library "work")
