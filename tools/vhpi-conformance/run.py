#!/usr/bin/env python3
"""Install-free VHPI conformance run: each C plugin in plugins/ runs on each of its designs in
designs/ under yavs and under the primary reference simulator (`nvc` on PATH); the canonical
traces (the plugin's "T " lines plus the exit status) are diffed.

    tools/vhpi-conformance/run.py [--only PLUGIN[,..]] [--show] [--out DIR]

A second reference simulator (sim4, also implements VHPI) is used when SIM4_BIN names its bin
directory (the one with vlib, vcom and vsim; the simulator's licence variable must be set). The
output is then a three-way comparison per plugin and design: how many yavs trace lines match sim1,
sim4, both or neither. A line that matches neither is a candidate yavs gap; a line where the two
references differ from each other is a reference quirk.

SIM3_BIN (bin directory of the third reference, licence in LM_LICENSE_FILE) adds a run of it, which
needs full design access (vopt) for puts; its trace is written next to the others and its match
count with yavs is printed.

Environment: YAVS_BIN (default `yavs` on PATH), SIM4_BIN and SIM3_BIN (optional). Exit status 1 if any yavs
trace differs from sim1.
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
    "nonprop": [("nonprop", "nonprop")],
    "intvec": [("intvec", "intvec")],
    "strput": [("strput", "strput")],
    "vecedge": [("vecedge", "vecedge")],
    "tport": [("tport", "tport")],
    "vars": [("vars", "vars")],
    "fresult": [("fresult", f"fr_{c}") for c in ("rev", "iota", "word", "nosize", "fixed")],
    "farch": [("farch", "farch")],
    "fparam": [("fparam", "fp_rec"), ("fparam", "fp_acc"), ("fsig", "fp_sig"), ("fsigin", "fp_sigin")],
}
# Designs with shared variables of non-protected types: analysed with the relaxed option.
RELAXED = {"vars"}
# Plugins that register foreign models; their designs name the plugin's file in the library word.
FOREIGN = {"fresult", "farch", "fparam"}
SANDBOX = ["timeout", "120", "systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=2G"]


def sh(cmd, cwd):
    p = subprocess.run(SANDBOX + cmd, cwd=cwd, capture_output=True, text=True, errors="replace")
    return p.returncode, p.stdout, p.stderr


NOTE = re.compile(r"^(\*\* Note: (\S+: )?|# VHPI: |# )")  # reference simulators print plugin output as notes


def trace(out, err, rc):
    lines = [NOTE.sub("", l).rstrip() for l in (out + err).splitlines()]
    return [l for l in lines if l.startswith("T ")] + [f"# exit {rc}"]


def run_yavs(so, design, top, work):
    yavs = os.environ.get("YAVS_BIN", "yavs")
    f = str(HERE / "designs" / f"{design}.vhd")
    w = f"work:{work}"
    rx = ["--relaxed"] if design in RELAXED else []
    for step in (["analyze", "--std=2008", f"--work={w}"] + rx + [f], ["elab", "--std=2008", f"--work={w}"] + rx + [top]):
        rc, o, e = sh([yavs] + step, work.parent)
        if rc:
            return None, f"{step[0]} failed: {e.strip().splitlines()[:1]}"
    rc, o, e = sh([yavs, "run", "--std=2008", f"--work={w}"] + rx + [f"--vhpi={so}", top], work.parent)
    return trace(o, e, rc), e


def run_ref(so, design, top, work):
    f = str(HERE / "designs" / f"{design}.vhd")
    base = ["nvc", "--std=2008", f"--work=work:{work}"]
    rx = ["--relaxed"] if design in RELAXED else []
    for step in (["-a"] + rx + [f], ["-e", top]):
        rc, o, e = sh(base + step, work.parent)
        if rc:
            return None, f"{step[0]} failed: {e.strip().splitlines()[:1]}"
    rc, o, e = sh(base + [f"--load={so}", "-r", top], work.parent)
    return trace(o, e, rc), e


def run_sim4(so, design, top, work):
    bindir = os.environ["SIM4_BIN"]
    f = str(HERE / "designs" / f"{design}.vhd")
    work.mkdir(exist_ok=True)  # the run directory; the library is its `work`
    # A foreign model's library word names a shared library this simulator loads from here.
    # It loads it from there, running its startup routines, so it is not loaded a second time.
    (work / Path(so).name).write_bytes(Path(so).read_bytes())
    load = [] if Path(so).stem in FOREIGN else ["-loadvhpi", f"{so}:conf_boot"]
    (work / "run.do").write_text("run -all\nquit\n")
    for step in ([f"{bindir}/vlib", "work"], [f"{bindir}/vcom", "-2008", f]):
        rc, o, e = sh(step, work)
        if rc or re.search(r"[1-9]\d* Errors", o):
            return None, f"{step[0].split('/')[-1]} failed: {o.strip().splitlines()[-1:]}"
    rc, o, e = sh([f"{bindir}/vsim", "-c"] + load + [top, "-do", "run.do"], work)
    return trace(o, e, rc), o + e


def run_sim3(so, design, top, work):
    """Third reference: needs full design access (vopt) and loads the plugin by its startup table."""
    bindir = os.environ["SIM3_BIN"]
    f = str(HERE / "designs" / f"{design}.vhd")
    work.mkdir(exist_ok=True)
    (work / "run.do").write_text("run -all\nquit\n")
    env_lic = os.environ.get("LM_LICENSE_FILE", "")
    pre = ["env", f"SALT_LICENSE_SERVER={env_lic}"]
    for step in ([f"{bindir}/vlib", "work"], [f"{bindir}/vcom", "-2008", f],
                 [f"{bindir}/vopt", "-access=rw+/.", top, "-o", "opt"]):
        rc, o, e = sh(pre + step, work)
        if rc or re.search(r"[1-9]\d* Errors", o):
            return None, f"{step[0].split('/')[-1]} failed: {o.strip().splitlines()[-1:]}"
    rc, o, e = sh(pre + [f"{bindir}/vsim", "-c", "-vhpi", str(so), "opt", "-do", "run.do"], work)
    return trace(o, e, rc), o + e


def fold(l):
    """Case-folded line with the library separator unified: names are upper case in one reference."""
    return l.lower().replace("work:", "work.")


def matched(a, b):
    """Indices of a that are in the longest common subsequence with b."""
    idx = set()
    for blk in difflib.SequenceMatcher(None, a, b, autojunk=False).get_matching_blocks():
        idx.update(range(blk.a, blk.a + blk.size))
    return idx


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
            tag = f"{name}.{design}" if top == design else f"{name}.{top}"
            res = {}
            sims = [("sim1", run_ref), ("yavs", run_yavs)] + ([("sim4", run_sim4)] if "SIM4_BIN" in os.environ else []) \
                + ([("sim3", run_sim3)] if "SIM3_BIN" in os.environ else [])
            for sim, fn in sims:
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
            if "sim4" in res:
                g, r1, r4 = (list(map(fold, x)) for x in (got, ref, res["sim4"]))
                m1, m4 = matched(g, r1), matched(g, r4)
                kinds = ["neither", "sim1", "sim4", "both"]
                lab = [kinds[(i in m1) + 2 * (i in m4)] for i in range(len(got))]
                (out / f"{tag}.three").write_text("".join(f"{k:8} {l}\n" for k, l in zip(lab, got)))
                cnt = {k: lab.count(k) for k in kinds}
                agree = len(r4) - len(matched(r4, r1))
                print(f"  three-way ({len(got)} yavs lines): both {cnt['both']}, sim1 only {cnt['sim1']}, sim4 only {cnt['sim4']}, neither {cnt['neither']}; case-folded; sim4 lines not in sim1: {agree}")
            if "sim3" in res:
                m3 = matched(list(map(fold, got)), list(map(fold, res["sim3"])))
                print(f"  sim3: {len(m3)}/{len(got)} yavs lines matched; sim3 trace {len(res['sim3'])} lines")
            if a.show and diff:
                print("\n".join(diff))
    print(f"traces and diffs in {out}")
    sys.exit(1 if bad else 0)


main()
