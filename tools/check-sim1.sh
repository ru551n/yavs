#!/usr/bin/env bash
# Analyze lib/ in the order of lib/ORDER.md with the first reference simulator (sim1), one scratch
# library per standard under ~/.cache/rsim-libs. Prints every diagnostic and exits non-zero if any
# file fails. usage: tools/check-sim1.sh [2008|2019 ...]
# The command is key check_cmd of section [sim1] in tools/sims.local.conf (untracked), a shell
# command using $std, $lib, $w (scratch dir), $extra (flags) and $path; key check_bootstrap holds the
# flag for analysing STD.STANDARD itself.
set -uo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
cmd=$(python3 -I "$here/tools/simconf.py" sim1 check_cmd) || exit 2
boot=$(python3 -I "$here/tools/simconf.py" sim1 check_bootstrap "")
scratch=${YAVS_LIBS_SCRATCH:-$HOME/.cache/rsim-libs/work}
fail=0
for std in "${@:-2008 2019}"; do
  for std in $std; do
    w=$scratch/sim1-$std
    rm -rf "$w"; mkdir -p "$w"
    sed -n "/^\`\`\`order-$std\$/,/^\`\`\`\$/p" "$here/lib/ORDER.md" | grep -v '^```' |
    while read -r lib file; do
      extra=""; path="$here/lib/$file"
      if [ "$file" = "std/$std/standard.vhdl" ]; then
        extra="$boot"
        if [ "$std" = 2019 ]; then
          # sim1 hardcodes its own bootstrap: it wants RANGE_DIRECTION declared behind
          # BOOLEAN (a sim1 limitation, the LRM text is valid). Analyze a copy with the type
          # moved behind BOOLEAN instead.
          path="$w/standard-fixed.vhdl"
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
      export std lib w extra path
      out=$(timeout 300 systemd-run --user --scope -q -p MemoryMax=2G \
        bash -c "$cmd" _ 2>&1)
      rc=$?
      if [ $rc -ne 0 ]; then echo "FAIL $std $file (rc=$rc)"; echo "$out"; echo x >> "$w/failed"
      elif [ -n "$out" ]; then echo "WARN $std $file"; echo "$out"
      else echo "ok   $std $file"; fi
    done
    [ -e "$w/failed" ] && fail=1
  done
done
exit $fail
