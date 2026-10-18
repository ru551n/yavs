#!/usr/bin/env bash
# Analyze lib/ in the order of lib/ORDER.md with NVC, one scratch library per
# standard under ~/.cache/rsim-libs. Prints every NVC diagnostic and exits
# non-zero if any file fails. usage: tools/check-nvc.sh [2008|2019 ...]
set -uo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
scratch=${YAVS_LIBS_SCRATCH:-$HOME/.cache/rsim-libs/work}
fail=0
for std in "${@:-2008 2019}"; do
  for std in $std; do
    w=$scratch/nvc-$std
    rm -rf "$w"; mkdir -p "$w"
    sed -n "/^\`\`\`order-$std\$/,/^\`\`\`\$/p" "$here/lib/ORDER.md" | grep -v '^```' |
    while read -r lib file; do
      extra=""; path="$here/lib/$file"
      if [ "$file" = "std/$std/standard.vhdl" ]; then
        extra="--bootstrap"
        if [ "$std" = 2019 ]; then
          # NVC hardcodes its own bootstrap: it wants RANGE_DIRECTION declared behind
          # BOOLEAN (NVC limitation, the LRM text is valid). Analyze a copy with the type
          # moved behind BOOLEAN instead.
          path="$w/standard-nvc.vhdl"
          python3 -I - "$here/lib/$file" "$path" <<'PY'
import sys
s = open(sys.argv[1], encoding="latin-1", newline="").read().split("\n")
i = next(k for k, l in enumerate(s) if l.startswith("  type RANGE_DIRECTION is ("))
j = next(k for k, l in enumerate(s) if l.startswith("  type BOOLEAN is"))
out = "\n".join(s[:i] + s[j:j + 1] + s[i:j] + s[j + 1:])
open(sys.argv[2], "w", encoding="latin-1", newline="").write(out)
PY
        fi
      fi
      out=$(timeout 300 systemd-run --user --scope -q -p MemoryMax=2G \
        nvc --std="$std" --work="$lib:$w/$lib" -a $extra "$path" 2>&1)
      rc=$?
      if [ $rc -ne 0 ]; then echo "FAIL $std $file (rc=$rc)"; echo "$out"; echo x >> "$w/failed"
      elif [ -n "$out" ]; then echo "WARN $std $file"; echo "$out"
      else echo "ok   $std $file"; fi
    done
    [ -e "$w/failed" ] && fail=1
  done
done
exit $fail
