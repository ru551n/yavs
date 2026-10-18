# rsim-lib-2008: NVC validation

nvc 1.23-devel (1.22.0.r66.gef5084a94) (Using LLVM 21.1.8)

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/nvc-rsim-lib-2008` with `tools/corpus/run.py rsim-lib-2008 --tool nvc --bless` (one file per call, `--std=2008`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 27; accepted: 27; rejected: 0.
- Flags needed: none.
- `standard.vhdl` is analyzed with `--bootstrap` (2019: from a copy with `DIRECTION` renamed `RANGE_DIRECTION` and moved behind `BOOLEAN`, as `tools/check-nvc.sh` does).

No rejected files. Result list: `rsim-lib-2008.nvc.expected`.
