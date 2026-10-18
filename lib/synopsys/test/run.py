#!/usr/bin/env python3
"""Differential test: original ieee Synopsys packages vs lib/synopsys, on NVC and GHDL.

  run.py [--sims nvc,ghdl] [--stds 08,93] [--only orig|mine|both] [--jobs 8] [tb-name-substring ...]

Testbenches are tb/*.vhd.  The token `syna` is the library the packages are taken
from: `ieee` for the simulator's originals, `synr` for lib/synopsys.
Output (stdout+stderr) is normalised (package source locations and call-stack lines
dropped) and compared byte for byte between orig and mine for the same simulator.
"""
import argparse, os, re, shutil, subprocess, sys, difflib
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.dirname(HERE)
CACHE = os.path.expanduser("~/.cache/rsim-synopsys")
BUILD = os.path.join(CACHE, "build")
GUARD = ["systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=4G", "timeout", "300"]

def files_for(std):
    d = SRC if std == "08" else os.path.join(SRC, "v93")
    fs = ["std_logic_arith.vhd", "std_logic_unsigned.vhd", "std_logic_signed.vhd"]
    out = [os.path.join(SRC, f) for f in fs]
    misc = os.path.join(d, "std_logic_misc.vhd")
    return out + [misc]

def sh(cmd, cwd):
    p = subprocess.run(GUARD + cmd, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace")
    return p.returncode, p.stdout

def norm(sim, text):
    out = []
    for l in text.splitlines():
        if sim == "nvc" and l.startswith("   "): continue
        l = re.sub(r"^\S*std_logic_(arith|unsigned|signed|misc)\.vhdl?:\d+:\d+:", "PKG:", l)
        out.append(l)
    return "\n".join(out) + "\n"

def setup(sim, std, var, tbs):
    d = os.path.join(BUILD, f"{sim}-{std}-{var}")
    shutil.rmtree(d, ignore_errors=True); os.makedirs(d)
    if var == "mine":
        fl = files_for(std)
        if sim == "nvc":
            rc, o = sh(["nvc", f"--std={'1993' if std=='93' else '2008'}", "--work=synr", "-a"] + fl, d)
        else:
            rc, o = sh(["ghdl", "-a", f"--std={std}", "--work=synr"] + fl, d)
        if rc != 0:
            print(f"ANALYSIS FAILED {sim} {std} mine\n{o}"); return None
    return d

def run_tb(sim, std, var, d, tb):
    lib = "ieee" if var == "orig" else "synr"
    src = open(os.path.join(HERE, "tb", tb + ".vhd")).read().replace("syna", lib)
    root = d; d = os.path.join(root, tb); os.makedirs(d, exist_ok=True)
    f = os.path.join(d, tb + ".vhd"); open(f, "w").write(src)
    if sim == "nvc":
        s = "1993" if std == "93" else "2008"
        rc, o = sh(["nvc", f"--std={s}", "-L", root, "-a", f, "-e", tb, "-r"], d)
    else:
        extra = ["-fsynopsys"] if var == "orig" else []
        extra += ["-P" + root]
        rc, o = sh(["ghdl", "-a", f"--std={std}"] + extra + [f], d)
        if rc == 0:
            rc, o2 = sh(["ghdl", "--elab-run", f"--std={std}"] + extra + [tb], d)
            o += o2
    open(os.path.join(root, tb + ".raw"), "w").write(o)
    n = norm(sim, o)
    open(os.path.join(root, tb + ".out"), "w").write(n)
    return rc, n

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--sims", default="nvc,ghdl"); ap.add_argument("--stds", default="08,93")
    ap.add_argument("--only", default="both"); ap.add_argument("--jobs", type=int, default=8)
    ap.add_argument("pat", nargs="*")
    a = ap.parse_args()
    tbs = sorted(f[:-4] for f in os.listdir(os.path.join(HERE, "tb")) if f.endswith(".vhd"))
    if a.pat: tbs = [t for t in tbs if any(p in t for p in a.pat)]
    vars_ = ["orig", "mine"] if a.only == "both" else [a.only]
    jobs = []; dirs = {}
    for sim in a.sims.split(","):
        for std in a.stds.split(","):
            for var in vars_:
                d = setup(sim, std, var, tbs)
                if d is None: sys.exit(1)
                dirs[(sim, std, var)] = d
                for tb in tbs: jobs.append((sim, std, var, d, tb))
    res = {}
    with ThreadPoolExecutor(a.jobs) as ex:
        for j, r in zip(jobs, ex.map(lambda j: run_tb(*j), jobs)): res[(j[0], j[1], j[2], j[4])] = r
    bad = 0; tot = 0
    for sim in a.sims.split(","):
        for std in a.stds.split(","):
            for tb in tbs:
                tot += 1
                if a.only != "both":
                    rc, n = res[(sim, std, a.only, tb)]
                    print(f"{sim:5} {std} {tb:28} {a.only} rc={rc} lines={n.count(chr(10))}")
                    continue
                (rc0, n0), (rc1, n1) = res[(sim, std, "orig", tb)], res[(sim, std, "mine", tb)]
                ok = rc0 == rc1 and n0 == n1
                print(f"{'PASS' if ok else 'FAIL'} {sim:5} {std} {tb:28} rc={rc0}/{rc1} lines={n0.count(chr(10))}/{n1.count(chr(10))}")
                if not ok:
                    bad += 1
                    dl = list(difflib.unified_diff(n0.splitlines(), n1.splitlines(), "orig", "mine", lineterm="", n=0))
                    print("\n".join(dl[:12]))
    print(f"{tot - bad}/{tot} passed")
    sys.exit(1 if bad else 0)
main()
