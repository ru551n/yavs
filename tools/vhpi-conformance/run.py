#!/usr/bin/env python3
"""Install-free VHPI conformance run: each C plugin in plugins/ runs on each of its designs in
designs/ under yavs and under the primary reference simulator (`nvc` on PATH); the canonical
traces (the plugin's "T " lines plus the exit status) are diffed.

    tools/vhpi-conformance/run.py [--only PLUGIN[,..]] [--show] [--out DIR]

Environment: YAVS_BIN (default `yavs` on PATH). Exit status 1 if any trace differs.
Python 3 stdlib only.
"""
import argparse
import difflib
import os
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
INCLUDE = HERE.parent.parent / "crates" / "yavs-vhpi" / "include"
# plugin -> [(design file stem, top entity)]
RUNS = {
    "walk": [("hier", "hier")],
    "props": [("hier", "hier")],
    "values": [("hier", "hier")],
    "cbs": [("hier", "hier")],
    "put": [("drive", "drive")],
}
SANDBOX = ["timeout", "120", "systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=2G"]


def sh(cmd, cwd):
    p = subprocess.run(SANDBOX + cmd, cwd=cwd, capture_output=True, text=True, errors="replace")
    return p.returncode, p.stdout, p.stderr


NOTE = re.compile(r"^\*\* Note: (\S+: )?")  # a reference simulator prints plugin output as notes


def trace(out, err, rc):
    lines = [NOTE.sub("", l).rstrip() for l in (out + err).splitlines()]
    return [l for l in lines if l.startswith("T ")] + [f"# exit {rc}"]


def run_yavs(so, design, top, work):
    yavs = os.environ.get("YAVS_BIN", "yavs")
    f = str(HERE / "designs" / f"{design}.vhd")
    w = f"work:{work}"
    for step in (["analyze", "--std=2008", f"--work={w}", f], ["elab", "--std=2008", f"--work={w}", top]):
        rc, o, e = sh([yavs] + step, work.parent)
        if rc:
            return None, f"{step[0]} failed: {e.strip().splitlines()[:1]}"
    rc, o, e = sh([yavs, "run", "--std=2008", f"--work={w}", f"--vhpi={so}", top], work.parent)
    return trace(o, e, rc), e


def run_ref(so, design, top, work):
    f = str(HERE / "designs" / f"{design}.vhd")
    base = ["nvc", "--std=2008", f"--work=work:{work}"]
    for step in (["-a", f], ["-e", top]):
        rc, o, e = sh(base + step, work.parent)
        if rc:
            return None, f"{step[0]} failed: {e.strip().splitlines()[:1]}"
    rc, o, e = sh(base + [f"--load={so}", "-r", top], work.parent)
    return trace(o, e, rc), e


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("--only", help="comma separated plugin names")
    ap.add_argument("--show", action="store_true", help="print every diff")
    ap.add_argument("--out", default=os.path.expanduser("~/.cache/yavs-vhpi-conformance"))
    a = ap.parse_args()
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    names = a.only.split(",") if a.only else list(RUNS)
    bad = 0
    for name in names:
        so = out / f"{name}.so"
        cc = subprocess.run(["gcc", "-shared", "-fPIC", "-O0", "-Wall", "-Wno-unused-function", "-I", str(INCLUDE), "-I", str(HERE / "plugins"),
                             str(HERE / "plugins" / f"{name}.c"), "-o", str(so)], capture_output=True, text=True)
        if cc.returncode:
            sys.exit(f"{name}: plugin build failed\n{cc.stderr}")
        for design, top in RUNS[name]:
            tag = f"{name}.{design}"
            res = {}
            for sim, fn in (("sim1", run_ref), ("yavs", run_yavs)):
                work = out / f"{tag}.{sim}.{'08' if sim == 'sim1' else 'work'}"
                tr, err = fn(so, design, top, work)
                (out / f"{tag}.{sim}.err").write_text(err if isinstance(err, str) else "")
                if tr is None:
                    tr = [f"# {err}"]
                (out / f"{tag}.{sim}.trace").write_text("\n".join(tr) + "\n")
                res[sim] = tr
            ref, got = res["sim1"], res["yavs"]
            sm = difflib.SequenceMatcher(None, ref, got, autojunk=False)
            same = sum(b.size for b in sm.get_matching_blocks())
            diff = list(difflib.unified_diff(ref, got, "sim1", "yavs", lineterm="", n=1))
            (out / f"{tag}.diff").write_text("\n".join(diff) + "\n")
            ok = ref == got
            bad += not ok
            print(f"{'PASS' if ok else 'DIFF'} {tag}: {same}/{len(ref)} reference lines matched, yavs {len(got)} lines")
            if a.show and diff:
                print("\n".join(diff))
    print(f"traces and diffs in {out}")
    sys.exit(1 if bad else 0)


main()
