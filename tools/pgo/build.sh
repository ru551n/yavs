#!/usr/bin/env bash
# Profile-guided release build of yavs for the host target: instrumented build, training runs
# (tools/pgo/train.py), profile merge, final build with the profile. See tools/pgo/AGENTS.md.
#
#   tools/pgo/build.sh OUT_BINARY
#
# Env: RUSTFLAGS      flags both builds share (for example `-C target-cpu=x86-64-v3`)
#      PGO_DIR        profile, scratch and both target directories (default target/pgo)
#      PGO_BUILD      the build command (default `cargo build --release --locked -p yavs --features clift`;
#                     `tools/build-bench.sh --features clift` for a build to benchmark)
#      PGO_BIN_REL    binary below a target directory (default release/yavs, release/yavs.exe on Windows)
#      PGO_FINAL_BUILD, PGO_FINAL_BIN_REL   the same for the final build when it differs from the instrumented one
#                     (a release that links against an old glibc builds the final binary with cargo-zigbuild and a --target)
#      PGO_JOBS       training cases in parallel (default 2)
set -euo pipefail
cd "$(dirname "$0")/../.."
out=${1:?usage: tools/pgo/build.sh OUT_BINARY}
dir=$(mkdir -p "${PGO_DIR:-target/pgo}" && cd "${PGO_DIR:-target/pgo}" && pwd)
build=${PGO_BUILD:-cargo build --release --locked -p yavs --features clift}
exe=
case "$(uname -s)" in MINGW* | MSYS* | CYGWIN*) exe=.exe ;; esac
rel=${PGO_BIN_REL:-release/yavs$exe}
final_build=${PGO_FINAL_BUILD:-$build}
final_rel=${PGO_FINAL_BIN_REL:-$rel}
shared=${RUSTFLAGS:-}

host=$(rustc -vV | sed -n 's/^host: //p')
profdata="$(rustc --print sysroot)/lib/rustlib/$host/bin/llvm-profdata$exe"
[[ -x $profdata ]] || { echo "pgo: $profdata is missing: run \`rustup component add llvm-tools\`" >&2; exit 1; }

secs() { date +%s; }
t0=$(secs)

echo "pgo: 1/4 instrumented build"
t=$(secs)
CARGO_TARGET_DIR=$dir/target-instr RUSTFLAGS="$shared -C profile-generate=$dir/raw" $build
echo "pgo: instrumented build $(($(secs) - t)) s"

echo "pgo: 2/4 training"
t=$(secs)
python3 -I tools/pgo/train.py "$dir/target-instr/$rel" --profile-dir "$dir/raw" --scratch "$dir/scratch" --jobs "${PGO_JOBS:-2}"
echo "pgo: training $(($(secs) - t)) s"

echo "pgo: 3/4 merge"
"$profdata" merge -o "$dir/merged.profdata" "$dir"/raw/*.profraw
sha256sum "$dir/merged.profdata" 2> /dev/null || shasum -a 256 "$dir/merged.profdata"

echo "pgo: 4/4 final build"
t=$(secs)
# A profile that does not match the sources makes the compiler warn: that is an error here, never a silent no-op.
CARGO_TARGET_DIR=$dir/target-use RUSTFLAGS="$shared -C profile-use=$dir/merged.profdata" $final_build 2>&1 | tee "$dir/final-build.log"
if grep -Eiq 'hash mismatch|profile data|malformed|no profile' "$dir/final-build.log"; then
  echo "pgo: the compiler complains about the profile, see $dir/final-build.log" >&2
  exit 1
fi
echo "pgo: final build $(($(secs) - t)) s"
cp "$dir/target-use/$final_rel" "$out"
echo "pgo: $out, total $(($(secs) - t0)) s"
