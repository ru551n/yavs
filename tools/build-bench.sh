#!/usr/bin/env bash
# Build yavs for benchmarking and profiling: the release profile plus 64-byte aligned functions (#576),
# so code layout does not move the cycles of the interpreter loop between builds. Release builds and CI
# artefacts are NOT aligned. Same arguments as `cargo build --release -p yavs`; pass --features llvm etc.
# Put the output in its own target dir (CARGO_TARGET_DIR), RUSTFLAGS differ from a plain release build.
set -euo pipefail
cd "$(dirname "$0")/.."
RUSTFLAGS="${RUSTFLAGS:-} -C llvm-args=-align-all-functions=6" exec cargo build --release -p yavs "$@"
