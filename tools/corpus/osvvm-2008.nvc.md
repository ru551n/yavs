# osvvm-2008: NVC validation

nvc 1.23-devel (1.22.0.r66.gef5084a94) (Using LLVM 21.1.8)

Every manifest entry analyzed in order into `~/.cache/rsim-corpora/nvc-osvvm-2008` with `tools/corpus/run.py osvvm-2008 --tool nvc --bless` (one file per call, `--std=2008`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).

- Files: 174; accepted: 174; rejected: 0.
- Flags needed: none.

No rejected files. Result list: `osvvm-2008.nvc.expected`.
