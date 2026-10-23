#!/usr/bin/env bash
# Re-fetch the IEEE VASG packages at the pinned commit, verify hashes, and
# regenerate (or with --check, compare against) lib/std and lib/ieee.
#   tools/update-ieee-libs.sh          # check: fail if the vendored tree differs
#   tools/update-ieee-libs.sh --write  # overwrite lib/std and lib/ieee
set -euo pipefail
URL=https://opensource.ieee.org/vasg/Packages.git
PIN=dc691c8cf8a7365dd1d60895818631320996e983   # branch release, after tag 1076-2019
here=$(cd "$(dirname "$0")/.." && pwd)
mode=${1:---check}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/yavs-ieee.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
git clone -q "$URL" "$tmp/up"
git -C "$tmp/up" checkout -q "$PIN"
test "$(git -C "$tmp/up" rev-parse HEAD)" = "$PIN"
# 1. upstream files must match the pinned hashes
(cd "$tmp/up" && sha256sum -c --quiet "$here/lib/UPSTREAM.sha256")
# 2. derive the tree
python3 -I "$here/tools/derive_libs.py" "$tmp/up" "$tmp/out"
# 3. compare or write
if [ "$mode" = --write ]; then
  # The VITAL and Synopsys packages come from other sources (lib/SOURCES.md): keep them.
  mkdir "$tmp/vital"
  for r in 2008 2019; do mkdir "$tmp/vital/$r"; cp "$here"/lib/ieee/$r/vital_* "$here"/lib/ieee/$r/std_logic_{arith,unsigned,signed,misc}* "$tmp/vital/$r/"; done
  # ieee/1993 is vendored (1993-era packages, Synopsys and VITAL copies) except the four derived
  # numeric_std/numeric_bit files.
  cp -r "$here/lib/ieee/1993" "$tmp/vital/1993"
  rm -rf "$here/lib/std" "$here/lib/ieee"
  cp -r "$tmp/out/std" "$tmp/out/ieee" "$here/lib/"
  for r in 2008 2019; do cp "$tmp/vital/$r"/* "$here/lib/ieee/$r/"; done
  for f in "$tmp"/vital/1993/*; do [ -e "$here/lib/ieee/1993/$(basename "$f")" ] || cp "$f" "$here/lib/ieee/1993/"; done
  (cd "$here/lib" && find std ieee -type f | LC_ALL=C sort | xargs sha256sum > SHA256SUMS)
else
  diff -r "$tmp/out/std" "$here/lib/std"
  diff -r -x 1993 -x 'vital_*' -x 'std_logic_arith*' -x 'std_logic_unsigned*' -x 'std_logic_signed*' -x 'std_logic_misc*' "$tmp/out/ieee" "$here/lib/ieee"
  for f in "$tmp"/out/ieee/1993/*; do diff "$f" "$here/lib/ieee/1993/$(basename "$f")"; done
fi
(cd "$here/lib" && sha256sum -c --quiet SHA256SUMS)
echo "ok: lib/std and lib/ieee match $PIN"
