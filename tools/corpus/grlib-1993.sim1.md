# grlib-1993: reference simulator validation

Reference simulator sim1 1.23-devel (1.22.0.r58.gb96f33ba4)

sim1 is the primary reference simulator, sim2 the second and sim3 the third; the roles are fixed by the corpus tooling.

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/sim1-grlib-1993` with `tools/corpus/run.py grlib-1993 --tool sim1 --bless` (one file per call, `--std=1993`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 454; accepted by sim1: 454; rejected by sim1: 0. The manifest lists only files the reference tools accept (sim1, sim3 as second witness); the rest are under Excluded.
- Flags needed: --relaxed.
- Licenses: grlib: GPL-2.0-only.

GRLIB is VHDL-93 code; this is the manifest at VHDL-1993, where `std_logic_vector` is a type of its own and the paired overloads of `stdio` and `testlib` are legal. The sim2 flags are GRLIB's own (`-fexplicit`, the synopsys library switch).

## Excluded by validation (2 files; 2 direct, 0 cascades of a missing unit in a later round)

- 1 x rejected by the reference tools: sim1: ** Error: no visible declaration for CUSTOMOUT; sim2: error: no declaration for "_" (e.g. `lib/gaisler/axi/axi4mux.vhd`)
- 1 x uses a design unit or library that is missing: its own file is excluded or it is a vendor library (cascade) (e.g. `lib/gaisler/sim/axi4mem.vhd`)
