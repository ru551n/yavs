# grlib-2008: reference simulator validation

Reference simulator sim1 1.23-devel (1.22.0.r58.gb96f33ba4)

sim1 is the primary reference simulator, sim2 the second and sim3 the third; the roles are fixed by the corpus tooling.

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/sim1-grlib-2008` with `tools/corpus/run.py grlib-2008 --tool sim1 --bless` (one file per call, `--std=2008`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 410; accepted by sim1: 410; rejected by sim1: 0. The manifest lists only files the reference tools accept (sim1 and sim2, sim3 as tiebreaker); the rest are under Excluded.
- Flags needed: --relaxed.
- Licenses: grlib: GPL-2.0-only.

GRLIB is VHDL-93 code (its scripts pass the 93 flags to every tool; `stdio.vhd` and `testlib.vhd` are marked `vhdlstd=93` in its file lists). Under 2008 `std_logic_vector` is a subtype of `std_ulogic_vector`, so the paired overloads of those two packages are homographs and every reference rejects them; the files that use them (the processor core, the bus controller, the simulation models) drop out as cascades. What is listed is the rest.

## Excluded by validation (46 files; 46 direct, 0 cascades of a missing unit in a later round)

- 43 x uses a design unit or library that is missing: its own file is excluded or it is a vendor library (cascade) (e.g. `lib/cypress/ssram/cy7c1354b.vhd`)
- 1 x rejected by the reference tools: sim1: ** Error: no visible declaration for CUSTOMOUT; sim2: error: no declaration for "_" (e.g. `lib/gaisler/axi/axi4mux.vhd`)
- 1 x rejected by the reference tools: sim1: ** Error: homograph of HREAD [LINE, STD_LOGIC_VECTOR, BOOLEAN] already declared in this region; sim2: error: re (e.g. `lib/grlib/stdlib/stdio.vhd`)
- 1 x rejected by the reference tools: sim1: ** Error: homograph of U_AND [STD_LOGIC_VECTOR return STD_LOGIC] already declared in this region; sim2: error: (e.g. `lib/gsi/ssram/functions.vhd`)
