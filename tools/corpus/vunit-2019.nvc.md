# vunit-2019: NVC validation

nvc 1.23-devel (1.22.0.r66.gef5084a94) (Using LLVM 21.1.8)

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/nvc-vunit-2019` with `tools/corpus/run.py vunit-2019 --tool nvc --bless` (one file per call, `--std=2019`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 143; accepted: 143; rejected: 0.
- Flags needed: none.

No rejected files. Result list: `vunit-2019.nvc.expected`.
