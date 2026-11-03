#!/usr/bin/env python3
"""exitmap.py FILE...: sum the lines `YAVS_CLIFT_EXITMAP` wrote: bytes of machine code by what they
are (`Why` in crates/yavs-clift/src/lower.rs) and, for each way out, by IR kind."""
import sys
from collections import Counter

by, kinds, mode, refused = Counter(), Counter(), Counter(), Counter()
names = []
for path in sys.argv[1:]:
    for line in open(path):
        m, _fid, why, kind, n = line.split()
        n = int(n)
        if m == "x":
            refused[why] += 1
            names.append(f"{why} {kind} ({n} IR instructions)")
            continue
        by[why] += n
        mode[m, why] += n
        if why != "body":
            kinds[why, kind] += n
total = sum(by.values()) or 1
print(f"{'':>10} {'bytes':>12} {'share':>7} {'frame':>12} {'native':>12}")
for why, n in by.most_common():
    print(f"{why:>10} {n:>12} {100 * n / total:6.1f}% {mode['f', why]:>12} {mode['n', why]:>12}")
print(f"{'all':>10} {total:>12}")
print()
for (why, kind), n in kinds.most_common(40):
    print(f"{why:>10} {kind:<16} {n:>12} {100 * n / total:6.2f}%")
if refused:
    print()
    print("no native variant:", dict(refused))
    for n in names[:40]:
        print(" ", n)
