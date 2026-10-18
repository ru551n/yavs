#!/usr/bin/env bash
# Regenerate tests/corpus/RESULTS.md: build yavs, run the corpus at 1 thread, 4 threads and
# 4 threads shuffled, then summarise. Run outputs go to ~/.cache/rsim-difftest2.<pid> (removed afterwards).
set -euo pipefail
cd "$(dirname "$0")/../.."
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-6} DIFFTEST_CACHE=${DIFFTEST_CACHE:-$HOME/.cache/rsim-difftest2.$$}
cargo build -p yavs
export YAVS_BIN=${CARGO_TARGET_DIR:-$PWD/target}/debug/yavs
mkdir -p "$HOME/.cache"
tmp=$(mktemp -d "$HOME/.cache/rsim-report.XXXXXX")
d="python3 tools/difftest/difftest.py --candidate yavs --jobs 8"
$d --json "$tmp/t1.json" >/dev/null || true
$d --threads 4 --json "$tmp/t4.json" >/dev/null || true
$d --threads 4 --shuffle 1 --json "$tmp/t4s.json" >/dev/null || true
python3 tools/difftest/report.py tests/corpus/RESULTS.md \
  "1 thread=$tmp/t1.json" "4 threads (also equal to 1)=$tmp/t4.json" "4 threads, shuffled=$tmp/t4s.json"
rm -rf "$tmp" "$DIFFTEST_CACHE"
