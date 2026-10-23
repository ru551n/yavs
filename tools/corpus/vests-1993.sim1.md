# vests-1993: reference simulator validation

Reference simulator sim1 1.23-devel (1.22.0.r58.gb96f33ba4)

sim1 is the primary reference simulator, sim2 the second and sim3 the third; the roles are fixed by the corpus tooling.

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/sim1-vests-1993` with `tools/corpus/run.py vests-1993 --tool sim1 --bless` (one file per call, `--std=1993`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 1939; accepted by sim1: 1939; rejected by sim1: 0. The manifest lists only files the reference tools accept (sim1, sim3 as second witness); the rest are under Excluded.
- Flags needed: none.
- Licenses: vests: GPL-2.0-only.

The tests are small self-checking benches written for the 1993 standard; the non-compliant and disputed directories are not part of this list. Many are standalone, the Ashenden ones share packages and entities.

## Excluded by validation (146 files; 146 direct, 0 cascades of a missing unit in a later round)

- 96 x uses a design unit or library that is missing: its own file is excluded or it is a vendor library (cascade) (e.g. `vhdl-93/ashenden/compliant/ap_a_ap_a_03.vhd`)
- 3 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: object F1 has no attribute ATTR1 (e.g. `vhdl-93/billowitch/compliant/tc3060.vhd`)
- 3 x Reference simulator sim1 and sim2 disagree (sim1=ok, sim2=FAIL), no third reference at 1993: sim2: error: not static choice exclude others choice (e.g. `vhdl-93/billowitch/compliant/tc517.vhd`)
- 2 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Fatal: /home/sebbe/.cache/rsim-corpus2/val-vests-1993/sim1/work/WORK (e.g. `vhdl-93/ashenden/compliant/ch_05_tb_05_02.vhd`)
- 2 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: signature in generic formal designator is not supported in VH (e.g. `vhdl-93/ashenden/compliant/ch_20_ch_20_03.vhd`)
- 2 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Fatal: missing class_of for T_GROUP (e.g. `vhdl-93/ashenden/compliant/ch_20_ch_20_11.vhd`)
- 2 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: no possible overload of P1 has formal B (e.g. `vhdl-93/billowitch/compliant/tc146.vhd`)
- 1 x rejected by the reference tools: sim1: ** Error: missing actual for port RESET of mode IN without a default expression; sim2: error: port "_" of mode (e.g. `vhdl-93/ashenden/compliant/ch_03_tb_03_04.vhd`)
- 1 x rejected by the reference tools: sim1: ** Error: range direction of slice DOWNTO does not match prefix TO; sim2: error: direction of the range mismatc (e.g. `vhdl-93/ashenden/compliant/ch_04_ch_04_08.vhd`)
- 1 x rejected by the reference tools: sim1: ** Error: no visible declaration for RISING_EDGE; sim2: error: no declaration for "_" (e.g. `vhdl-93/ashenden/compliant/ch_06_reg-b.vhd`)
- 1 x rejected by the reference tools: sim1: ** Error: no visible declaration for ALU_FUNC; sim2: error: no declaration for "_" (e.g. `vhdl-93/ashenden/compliant/ch_10_alu.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: case expression must have locally static subtype (e.g. `vhdl-93/ashenden/compliant/ch_13_fg_13_14.vhd`)
- 1 x rejected by the reference tools: sim1: ** Error: cannot find architecture STRUCTURE of entity WORK.COMPUTER_SYSTEM; sim2: error: no architecture "_" (e.g. `vhdl-93/ashenden/compliant/ch_13_fg_13_15.vhd`)
- 1 x rejected by the reference tools: sim1: ** Error: WORK.REG has no port named RESET_N; sim2: error: no declaration for "_" (e.g. `vhdl-93/ashenden/compliant/ch_13_fg_13_20.vhd`)
- 1 x rejected by the reference tools: sim1: ** Error: no visible declaration for NAND3; sim2: error: no declaration for "_" (e.g. `vhdl-93/ashenden/compliant/ch_13_fg_13_26.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: range of generate statement must be static (e.g. `vhdl-93/ashenden/compliant/ch_14_fg_14_05.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: no visible declaration for BUS_MONITOR_PKG (e.g. `vhdl-93/ashenden/compliant/ch_14_fg_14_10.vhd`)
- 1 x rejected by the reference tools: sim1: ** Error: cannot find architecture RTL of entity WORK.DLX; sim2: error: no architecture "_" (e.g. `vhdl-93/ashenden/compliant/ch_15_dlxr.vhd`)
- 1 x rejected by the reference tools: sim1: ** Error: no visible declaration for TO_BIT; sim2: error: no declaration for "_" (e.g. `vhdl-93/ashenden/compliant/ch_15_mux2-b.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: unexpected all while parsing disconnection specification, exp (e.g. `vhdl-93/ashenden/compliant/ch_16_ch_16_05.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: unexpected others while parsing disconnection specification, (e.g. `vhdl-93/ashenden/compliant/ch_16_ch_16_06.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: formal signal parameter declaration may not include the reser (e.g. `vhdl-93/ashenden/compliant/ch_16_fg_16_06.vhd`)
- 1 x rejected by the reference tools: sim1: ** Error: no visible declaration for VALUE_PTR; sim2: error: no declaration for "_" (e.g. `vhdl-93/ashenden/compliant/ch_17_ch_17_06.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: formal parameter F with file type must have class FILE (e.g. `vhdl-93/ashenden/compliant/ch_18_ch_18_07.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: object THE_FPU has no attribute CELL_POSITION (e.g. `vhdl-93/ashenden/compliant/ch_20_ch_20_08.vhd`)
- 1 x rejected by the reference tools: sim1: ** Fatal: tree kind T_ARCH does not have item I_FLAGS; sim2: error: FOREIGN architectures are not yet handled (e.g. `vhdl-93/ashenden/compliant/ch_20_ch_20_10.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=ok, sim2=FAIL), no third reference at 1993: sim2: error: value of 'FOREIGN attribute does not begin with VHPIDIRECT (e.g. `vhdl-93/ashenden/compliant/ch_20_fg_20_19.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: port Q0 with mode BUFFER cannot be associated with formal por (e.g. `vhdl-93/ashenden/compliant/ch_21_fg_21_01.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: value 1 is already covered (e.g. `vhdl-93/billowitch/compliant/tc135.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: sorry, this form of named parameter is not supported (e.g. `vhdl-93/billowitch/compliant/tc136.vhd`)
- 1 x rejected by the reference tools: sim1: ** Error: type of string literal cannot be determined from the surrounding context; sim2: error: operator "_" i (e.g. `vhdl-93/billowitch/compliant/tc2369.vhd`)
- 1 x rejected by the reference tools: sim1: ** Error: missing choices for elements C128 to 'ÿ' of CHAR_ARR with index type CHARACTER; sim2: error: no choic (e.g. `vhdl-93/billowitch/compliant/tc2439.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=ok, sim2=FAIL), no third reference at 1993: sim2: error: missing body for function "_" declared at line 32:13 (e.g. `vhdl-93/billowitch/compliant/tc2874.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: object ONE has no attribute ATTR1 (e.g. `vhdl-93/billowitch/compliant/tc3101.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: value 730 outside of UDF_INT range -500 to 500 for parameter (e.g. `vhdl-93/billowitch/compliant/tc523.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: value 730 outside of INT range -500 to 500 for parameter R (e.g. `vhdl-93/billowitch/compliant/tc524.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=ok, sim2=FAIL), no third reference at 1993: sim2: error: type "_" is not visible here (e.g. `vhdl-93/billowitch/compliant/tc539.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: range direction of slice TO does not match prefix DOWNTO (e.g. `vhdl-93/billowitch/compliant/tc711.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: unexpected use while parsing block configuration, expecting o (e.g. `vhdl-93/billowitch/compliant/tc886.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: type mark cannot be the prefix of a selected name (e.g. `vhdl-93/billowitch/compliant/tc913.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: no visible declaration for A (e.g. `vhdl-93/billowitch/compliant/tc922.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: WORK is not a library or package (e.g. `vhdl-93/billowitch/compliant/tc927.vhd`)
- 1 x Reference simulator sim1 and sim2 disagree (sim1=FAIL, sim2=ok), no third reference at 1993: sim1: ** Error: no visible declaration for C (e.g. `vhdl-93/billowitch/compliant/tc933.vhd`)
