# rsim-lib-2019: NVC validation

nvc 1.23-devel (1.22.0.r66.gef5084a94) (Using LLVM 21.1.8)

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/nvc-rsim-lib-2019` with `tools/corpus/run.py rsim-lib-2019 --tool nvc --bless` (one file per call, `--std=2019`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 28; accepted: 26; rejected: 2.
- Flags needed: none.
- `standard.vhdl` is analyzed with `--bootstrap` (2019: from a copy with `DIRECTION` renamed `RANGE_DIRECTION` and moved behind `BOOLEAN`, as `tools/check-nvc.sh` does).

## Rejected files

- `std rsim/lib/std/2019/env.vhdl`: `formal parameter CALL_PATH with access type must have class VARIABLE`; `formal parameter CALL_PATH with type containing an access type must have class VARIABLE`; `formal parameter DIR with type containing an access type must have class VARIABLE`
- `std rsim/lib/std/2019/reflection.vhdl`: `invalid use of incomplete type ARRAY_SUBTYPE_MIRROR`; `invalid use of incomplete type ENUMERATION_SUBTYPE_MIRROR`; `invalid use of incomplete type FLOATING_SUBTYPE_MIRROR`; `invalid use of incomplete type INTEGER_SUBTYPE_MIRROR`; `invalid use of incomplete type PHYSICAL_SUBTYPE_MIRROR`; `invalid use of incomplete type RECORD_SUBTYPE_MIRROR`; `invalid use of incomplete type SUBTYPE_MIRROR`; `invalid use of incomplete type VALUE_MIRROR`

Both are NVC limitations (access-type `in` parameters in `std.env`, an incomplete-type use in `std.reflection`), not defects of the sources; rsim's list expects them to pass.
