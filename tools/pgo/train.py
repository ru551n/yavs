#!/usr/bin/env python3
"""Run the training workloads of a profile-guided build with an instrumented yavs.

  python3 -I tools/pgo/train.py INSTRUMENTED_YAVS --profile-dir DIR [--jobs N] [--scratch DIR] [--only a,b]

Every case is a self-checking testbench `tools/pgo/train/NAME.vhd` with a header line
`-- pgo: top=NAME_tb std=2008 stop=300us [relaxed] [weight=N] [analyze=N] [gen=NAME=VALUE]`. A case runs the paths users run:
analysis into fresh libraries and again into populated ones, `elab` cold and warm cache, `run` at the `auto` tier
(cold cache, then warm), compiled eagerly, at the interpreter tier, with 4 threads and with a waveform. Then every `tests/corpus` testbench is analysed and run twice (`auto`, and every function compiled) (broad language coverage: the analysis and elaboration paths). The first case runs alone against an
empty std/ieee store, the rest in parallel. Every run must exit 0: a failing run stops the build with its command and
output (a crashed instrumented run writes no profile, so a silent skip would train on less than it seems).
Standard library only; the profile goes to DIR/p-<signature>.profraw (runs of one binary merge into one file).
"""
import argparse
import concurrent.futures
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
HEADER = re.compile(r"^--\s*pgo:\s*(.*)$")


class Case:
    def __init__(self, path):
        self.path = path
        self.name = path.stem
        self.top = self.name + "_tb"
        self.std = "2008"
        self.stop = None
        self.relaxed = False
        self.weight = 1
        self.analyze = 1
        self.gens = []
        for line in path.read_text(encoding="utf-8").splitlines()[:15]:
            m = HEADER.match(line)
            if not m:
                continue
            for tok in re.sub(r"(\d+) +([fpnum]?s)\b", r"\1\2", m.group(1)).split():
                k, _, v = tok.partition("=")
                if k == "top":
                    self.top = v
                elif k == "std":
                    self.std = {"93": "93", "1993": "93"}.get(v, v)
                elif k == "stop":
                    self.stop = v
                elif k == "relaxed":
                    self.relaxed = True
                elif k == "weight":
                    self.weight = int(v)
                elif k == "analyze":
                    self.analyze = int(v)
                elif k == "gen":
                    self.gens.append(v)
                else:
                    sys.exit(f"train: {path}: unknown header key {k!r}")
            break
        else:
            sys.exit(f"train: {path}: no `-- pgo:` header in the first 15 lines")
        if not self.stop:
            sys.exit(f"train: {path}: the header needs stop=TIME")


def split_time(t):
    m = re.fullmatch(r"(\d+)\s*([a-z]+)", t)
    if not m:
        sys.exit(f"train: bad time {t!r}")
    return int(m.group(1)), m.group(2)


def run_ok(cmd, cwd, env, timeout, log):
    t0 = time.time()
    try:
        r = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        sys.exit(f"train: timeout after {timeout} s: {' '.join(cmd)}")
    out = (r.stdout + r.stderr).decode("utf-8", "replace")
    log.append((time.time() - t0, " ".join(cmd[1:3])))
    if r.returncode != 0:
        sys.exit(
            f"train: exit status {r.returncode}: {' '.join(cmd)}\n  cwd {cwd}\n"
            "  A training run must pass. Last output:\n" + out[-1500:]
        )
    return out


def train_case(c, yavs, scratch, env, timeout):
    d = scratch / c.name
    shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    log = []
    std = ["--std=" + c.std] + (["--relaxed"] if c.relaxed else [])
    work = ["--work=work:" + str(d / "work")]
    gens = [f"-g{g}" for g in c.gens]
    src = str(c.path)
    y = str(yavs)

    def ok(*args):
        return run_ok([y, *args], d, env, timeout, log)

    ok("parse", "--std=" + c.std, src)
    for i in range(c.analyze):  # cold libraries (a source-heavy case repeats the analysis and the uncached elaboration)
        w = ["--work=work:" + str(d / f"cold{i}")]
        ok("analyze", *std, *w, src)
        ok("elab", *std, *w, "--no-cache", *gens, c.top)
    ok("analyze", *std, *work, src)  # cold libraries
    ok("analyze", *std, *work, src)  # again, into the populated library
    ok("elab", *std, *work, *gens, c.top)  # cold compilation cache
    ok("elab", *std, *work, *gens, c.top)  # warm
    run = ["run", *std, *work, *gens, "--stop-time=" + c.stop, c.top]
    ok(*run, "--tier=auto")  # first run of a closure: leaves a note
    ok(*run, "--tier=auto")  # makes the cache entries
    for _ in range(c.weight):
        ok(*run, "--tier=auto")  # steady state
    ok(*run, "--tier=auto", "--threads=4")
    ok(*run, "--tier=clift")  # every function compiled before time 0
    ok(*run, "--tier=clift-auto", "--tier-heat=0,0,0")  # compiled at the first call, cold then from the cache
    ok(*run, "--tier=clift-auto", "--tier-heat=0,0,0")
    ok(*run, "--tier=auto", "--tier-max-size=300")  # big functions stay interpreted, as in big designs
    ok(*run, "--tier=auto", "--tier-max-size=300")
    for _ in range(c.weight):
        ok(*run, "--tier=interp")
    n, unit = split_time(c.stop)
    short = f"--stop-time={max(1, n // 10)}{unit}"
    ok("run", *std, *work, *gens, short, "--wave=" + str(d / "w.fst"), "--tier=auto", c.top)
    return c.name, log


def corpus_run(path, yavs, scratch, env, timeout, root):
    """One tests/corpus testbench: analyse, then run at `auto`; returns an error text or None (exit status other than its header's)."""
    hdr = {}
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines()[:15]:
        if line.startswith("-- difftest:"):
            for tok in line[len("-- difftest:"):].split():
                k, _, v = tok.partition("=")
                hdr[k] = v
            break
    if "top" not in hdr:
        return None
    d = scratch / "corpus" / str(path.relative_to(root)).replace(os.sep, "_")
    d.mkdir(parents=True)
    std = ["--std=" + {"1993": "93", "93": "93"}.get(hdr.get("std", "2008"), hdr.get("std", "2008"))]
    std += (["--relaxed"] if "relaxed" in hdr else []) + (["--psl"] if "psl" in hdr else [])
    work = ["--work=work:" + str(d / "work")]
    e = dict(env, YAVS_SPIN_LIMIT=hdr.get("spin", env.get("YAVS_SPIN_LIMIT", "1000000")))
    run = [str(yavs), "run", *std, *work, *(["--stop-time=" + hdr["stop"]] if "stop" in hdr else []),
           *(["--resolution=" + hdr["res"]] if "res" in hdr else [])]
    want_exit = int(hdr.get("exit", 0))
    steps = [([str(yavs), "analyze", *std, *work, str(path)], 0),
             ([*run, "--tier=auto", hdr["top"]], want_exit),
             ([*run, "--tier=clift", hdr["top"]], want_exit)]
    for i, (cmd, want) in enumerate(steps):
        try:
            (d / f"r{i}").mkdir()  # a testbench that writes a file starts from none every time
            r = subprocess.run(cmd, cwd=d / f"r{i}", env=e, capture_output=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            return f"{path.name}: timeout"
        if r.returncode != want:
            return f"{path.name}: {cmd[1]} exit status {r.returncode}, expected {want}"
    shutil.rmtree(d, ignore_errors=True)
    return None


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("yavs", help="instrumented yavs binary")
    ap.add_argument("--profile-dir", required=True)
    ap.add_argument("--scratch", help="work directory (default: PROFILE_DIR/../train-scratch)")
    ap.add_argument("--jobs", type=int, default=2, help="cases in parallel (default 2)")
    ap.add_argument("--only", help="comma separated case names")
    ap.add_argument("--no-corpus", action="store_true", help="skip the sweep over tests/corpus")
    ap.add_argument("--timeout", type=int, default=600, help="seconds per run")
    a = ap.parse_args()

    yavs = Path(a.yavs).resolve()
    if not yavs.is_file():
        sys.exit(f"train: {yavs} is not a file; build the instrumented binary first (tools/pgo/build.sh)")
    prof = Path(a.profile_dir).resolve()
    shutil.rmtree(prof, ignore_errors=True)
    prof.mkdir(parents=True)
    scratch = Path(a.scratch).resolve() if a.scratch else prof.parent / "train-scratch"
    shutil.rmtree(scratch, ignore_errors=True)
    scratch.mkdir(parents=True)

    cases = sorted((Case(p) for p in (HERE / "train").glob("*.vhd")), key=lambda c: c.name)
    if a.only:
        want = a.only.split(",")
        cases = [c for c in cases if c.name in want]
    if not cases:
        sys.exit("train: no cases found in tools/pgo/train")

    env = dict(os.environ)
    env["LLVM_PROFILE_FILE"] = str(prof / "p-%m.profraw")
    env["YAVS_STD_CACHE"] = str(scratch / "std")  # empty: the first case also trains the std/ieee analysis
    env["NO_COLOR"] = "1"

    t0 = time.time()
    runs = 0
    done = []
    first, rest = cases[0], cases[1:]
    done.append(train_case(first, yavs, scratch, env, a.timeout))
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, a.jobs)) as ex:
        for name, log in ex.map(lambda c: train_case(c, yavs, scratch, env, a.timeout), rest):
            done.append((name, log))
    root = HERE.parent.parent / "tests" / "corpus"
    sweep = sorted(root.rglob("*.vhd")) if root.is_dir() and not a.no_corpus else []
    t1 = time.time()
    bad = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, a.jobs)) as ex:
        bad = [r for r in ex.map(lambda p: corpus_run(p, yavs, scratch, env, a.timeout, root), sweep) if r]
    for b in bad[:20]:
        print("train: corpus run differs from its header:", b)
    if sweep:
        print(f"train: corpus sweep {len(sweep)} files, {len(bad)} not as their header says, {time.time() - t1:.0f} s")
    for name, log in done:
        runs += len(log)
        print(f"train: {name:20} {len(log):3} runs {sum(t for t, _ in log):7.1f} s")
    raw = list(prof.glob("*.profraw"))
    if not raw or any(p.stat().st_size == 0 for p in raw):
        sys.exit(f"train: no usable profile in {prof}: is {yavs} built with -C profile-generate?")
    print(f"train: {len(cases)} cases, {runs} runs, {time.time() - t0:.0f} s, {len(raw)} profile file(s)")


if __name__ == "__main__":
    main()
