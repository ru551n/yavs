# yavs-lib-2019: reference simulator validation

Reference simulator sim1 1.23-devel (1.22.0.r66.gef5084a94)

sim1 is the primary reference simulator, sim2 the second and sim3 the third; the roles are fixed by the corpus tooling.

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/sim1-yavs-lib-2019` with `tools/corpus/run.py yavs-lib-2019 --tool sim1 --bless` (one file per call, `--std=2019`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 28; accepted: 26; rejected: 2.
- Flags needed: none.
- `standard.vhdl` is analyzed with `--bootstrap` (2019: from a copy with `RANGE_DIRECTION` moved behind `BOOLEAN`, as `tools/check-sim1.sh` does).

## Rejected files

- `std yavs/lib/std/2019/env.vhdl`: `formal parameter CALL_PATH with access type must have class VARIABLE`; `formal parameter CALL_PATH with type containing an access type must have class VARIABLE`; `formal parameter DIR with type containing an access type must have class VARIABLE`
- `std yavs/lib/std/2019/reflection.vhdl`: `invalid use of incomplete type ARRAY_SUBTYPE_MIRROR`; `invalid use of incomplete type ENUMERATION_SUBTYPE_MIRROR`; `invalid use of incomplete type FLOATING_SUBTYPE_MIRROR`; `invalid use of incomplete type INTEGER_SUBTYPE_MIRROR`; `invalid use of incomplete type PHYSICAL_SUBTYPE_MIRROR`; `invalid use of incomplete type RECORD_SUBTYPE_MIRROR`; `invalid use of incomplete type SUBTYPE_MIRROR`; `invalid use of incomplete type VALUE_MIRROR`

Both are sim1 limitations (access-type `in` parameters in `std.env`, an incomplete-type use in `std.reflection`), not defects of the sources; yavs's list expects them to pass.
