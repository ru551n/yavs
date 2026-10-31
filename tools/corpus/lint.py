#!/usr/bin/env python3
"""
yavs lint over the libraries a corpus run built, default rules only: no internal error anywhere, and
the findings are exactly those of tools/corpus/lint.expected (every definite finding on the corpora
is confirmed real; a new one is a false finding or a reason to add it to the list).

  tools/corpus/lint.py OUT [--bless]

OUT holds one directory per manifest, each with one per library (tools/corpus/run.py --out OUT/<name>).
`yavs` comes from PATH.
"""

import argparse
import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path

HERE = Path(__file__).parent
EXPECTED = HERE / "lint.expected"
ROOTS = [os.environ.get("YAVS_CORPUS", ""), "~/.cache/rsim-corpus", "~/.cache/rsim-corpora"]


def relative(path):
    """A source path without the corpus root"""
    for root in ROOTS:
        root = str(Path(root).expanduser()) if root else ""
        if root and path.startswith(root + "/"):
            return path[len(root) + 1 :]
    return path


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out", type=Path)
    ap.add_argument("--bless", action="store_true")
    a = ap.parse_args()
    found, internal = set(), []
    for manifest in sorted(p for p in a.out.iterdir() if p.is_dir() and (HERE / f"{p.name}.toml").exists()):
        flags = tomllib.loads((HERE / f"{manifest.name}.toml").read_text()).get("flags", {}).get("yavs", "")
        flags = flags.split() if isinstance(flags, str) else flags
        if "--bootstrap" in flags:
            continue
        keep = [f for f in flags if f in ("--relaxed", "--psl") or f.startswith("-D")]
        libs = sorted(p.name for p in manifest.iterdir() if (p / "yavs-lib.toml").exists())
        for lib in libs:
            maps = [f"--map={x}:{manifest / x}" for x in libs if x != lib]
            cmd = ["yavs", "lint", "--message-format=short", "--color=never", f"--work={lib}:{manifest / lib}"]
            r = subprocess.run(cmd + maps + keep, capture_output=True, text=True, check=False)
            if r.returncode == 3 or "E9999" in r.stderr:
                internal.append(f"{manifest.name} {lib}: {r.stderr[-400:]}")
            for m in re.finditer(r"^(.+?):(\d+):\d+: \w+\[(lint_\d+)\]", r.stderr, re.M):
                found.add(f"{manifest.name} {m[3]} {relative(m[1])}:{m[2]}")
    for i in internal:
        print("INTERNAL", i)
    if a.bless:
        EXPECTED.write_text("".join(f"{x}\n" for x in sorted(found)))
    want = set(EXPECTED.read_text().splitlines())
    for x in sorted(found - want):
        print("NEW", x)
    for x in sorted(want - found):
        print("GONE", x)
    sys.exit(1 if internal or found != want else 0)


if __name__ == "__main__":
    main()
