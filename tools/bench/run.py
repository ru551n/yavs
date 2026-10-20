#!/usr/bin/env python3
"""Shared benchmark harness for yavs: phase timings, A/B comparison, JSON results.

  run.py --list
  run.py BIN [--bench a,b | --group internal|external|all] [--runs N]
  run.py --compare A B [--runs N] [--sims sim1,sim2] [--fail-on-regression]

Per benchmark and binary it reports the median CPU seconds of each phase (analyze, elaborate,
init, simulate: from `yavs run --time-report`), the process CPU total (exact, from rusage) and
the wall total. With --compare the two binaries run interleaved (rep-major, order alternating
every rep) and the table shows B/A ratios: below 1 is faster, `!` marks a slowdown and `+` a
speedup beyond --threshold (default 10%) and 0.05 s. Reference simulators (--sims) time the
internal designs only, as one number (elaborate + run), labels only: see AGENTS.md.
Every run writes a JSON file (raw samples and medians) under --out/results.
Stdlib only. Run with `python3 -I`.
"""
import argparse, hashlib, importlib.util, json, os, re, resource, shutil, statistics, subprocess, sys, time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
CACHE = Path(os.environ.get("YAVS_BENCH_CACHE", "~/.cache/yavs-bench")).expanduser()
CORPUS = Path(os.environ.get("YAVS_CORPUS", "~/.cache/rsim-corpus")).expanduser()
RUN_FLAGS = os.environ.get("YAVS_BENCH_RUN_FLAGS", "").split()  # extra `yavs run` flags, e.g. "--wave={out}"
OUT = CACHE / "bench-out.fst"
PHASES = ["analyze", "elaborate", "init", "simulate"]
STD = "2008"
INTERNAL = ["compute_loop", "ff_rtl", "resolved_bus", "delta_chain", "textio_heavy", "protected_scoreboard", "calls", "strings_heavy", "access_heavy", "report_heavy", "file_io", "ieee_heavy", "elab_heavy"]
SHIM = """#!/usr/bin/env python3
import json, os, sys
if len(sys.argv) > 1 and sys.argv[1] == "run":
    with open(os.environ["YAVS_BENCH_REC"], "a") as f:
        f.write(json.dumps({"argv": sys.argv[1:], "cwd": os.getcwd()}) + "\\n")
    print(">> Simulation SUCCESS")
    sys.exit(0)
os.execv(os.environ["YAVS_BENCH_REAL"], [os.environ["YAVS_BENCH_REAL"]] + sys.argv[1:])
"""
TR = re.compile(r"^time-report: (\w+) wall=([\d.]+) cpu=([\d.]+|n/a)$", re.M)
GC = re.compile(r"^time-report: gc collections=(\d+)$", re.M)


class Skip(Exception):
    pass


def sh(argv, cwd, env=None, timeout=1800, check=True):
    p = subprocess.run(argv, cwd=cwd, capture_output=True, text=True, errors="replace", timeout=timeout,
                       env={**os.environ, **(env or {})})
    if check and p.returncode:
        raise RuntimeError(f"{' '.join(map(str, argv[:4]))}... exit {p.returncode}: {(p.stdout + p.stderr)[-400:]}")
    return p


def mkdir(p):
    Path(p).mkdir(parents=True, exist_ok=True)
    return Path(p)


def corpus(name):
    d = CORPUS / name
    if not d.is_dir():
        raise Skip(f"{d} not found (tools/fetch-corpus.sh, or set YAVS_CORPUS)")
    return d


# ---- preparation: every benchmark becomes a list of commands [(argv with "{Y}" first, cwd)] ----

def yavs_libs(Y, key, root):
    """The external test-runner libraries of the analysis corpus (tools/corpus), built by binary Y."""
    out = mkdir(CACHE / "work" / key / "vunit")
    if not (out / "vunit_lib").is_dir():
        corpus("vunit")
        cmd = f"{Y} analyze --std={{std}} --work={{lib}}:{{dir}}/{{lib}} {{map}} {{flags}} {{file}}"
        sh([sys.executable, "-I", str(REPO / "tools/corpus/run.py"), "vunit-tests-2008", "--out", str(out), "--keep-going",
            "--no-wrap", "--root", str(CORPUS), "--cmd", cmd, "--expected", "none"], REPO, check=False)
        if not (out / "vunit_lib").is_dir():
            raise Skip("building the vunit libraries failed")
    return out


def prep_internal(name, Y, key):
    wd = mkdir(CACHE / "work" / key / name)
    sh([Y, "analyze", f"--std={STD}", f"--work=work:{wd}/lib", str(HERE / "designs" / f"{name}.vhd")], wd)
    return [([ "{Y}", "run", f"--std={STD}", f"--work=work:{wd}/lib", f"bench_{name}"], str(wd))]


def prep_vunit_vc(Y, key):
    libs = yavs_libs(Y, key, None)
    tests = corpus("vunit") / "vunit/vhdl/verification_components/test"
    vc = mkdir(CACHE / "work" / key / "vc")
    if not (vc / "vunit_lib").is_dir():  # the testbenches use `work.` for the library they live in
        shutil.copytree(libs / "vunit_lib", vc / "vunit_lib")
        for tb in sorted(tests.glob("tb_*.vhd")):
            sh([Y, "analyze", f"--std={STD}", "--relaxed", f"--work=vunit_lib:{vc}/vunit_lib", f"--map=osvvm:{libs}/osvvm", str(tb)],
               vc, check=False)
    cmds = []
    for line in (HERE / "vc_tests.tsv").read_text().splitlines():
        tb, case = line.split("\t")
        cfg = f"active python runner : false,enabled_test_cases : {case},output path : {vc}/,tb path : {tests}/,use_color : false"
        cmds.append((["{Y}", "run", f"--std={STD}", "--relaxed", f"--work=vunit_lib:{vc}/vunit_lib", f"--map=osvvm:{libs}/osvvm",
                      tb, f"-grunner_cfg={cfg}"], str(vc)))
    return cmds


def record(Y, key, tag, driver):
    """Run a suite driver with a recording stand-in for yavs: analysis is real, a `run` is only written down."""
    wd = mkdir(CACHE / "work" / key / tag)
    shim, rec = wd / "yavs-shim", wd / "records.jsonl"
    if not rec.exists():
        shim.write_text(SHIM)
        shim.chmod(0o755)
        env = {"YAVS_BENCH_REAL": str(Path(Y).resolve()), "YAVS_BENCH_REC": str(rec)}
        sh([sys.executable, "-I", str(REPO / "tools/suites" / driver[0])] + list(driver[1:]) + ["--yavs", str(shim), "--out", str(wd / "out")],
           REPO, env=env, timeout=3600, check=False)  # a driver reports the stand-in's runs as failures
    if not rec.exists():
        raise Skip(f"the {driver[0]} driver recorded no run")
    return [json.loads(l) for l in rec.read_text().splitlines()]


def prep_uvvm(comp, ent, Y, key, stop=None):
    root = corpus("uvvm")
    recs = record(Y, key, f"uvvm_{comp}", ("uvvm.py", "--only", comp, "-j", "2", "--no-wrap", "--root", str(CORPUS)))
    r = next((r for r in recs if ent in r["argv"]), None)
    if r is None:
        raise Skip(f"no recorded run of {ent} in {root}")
    # a bounded slice of a run that takes far over 5 minutes: --stop-time ends it with exit 0
    return [(["{Y}", r["argv"][0]] + ([f"--stop-time={stop}"] if stop else []) + r["argv"][1:], r["cwd"])]


def prep_osvvm(test, Y, key):
    corpus("osvvm-libraries")
    recs = record(Y, key, "osvvm", ("pro.py", "--jobs", "2", "--root", str(CORPUS / "osvvm-libraries"), "--filter", test))
    if len(recs) != 1:
        raise Skip(f"expected one recorded run for {test}, found {len(recs)}")
    return [(["{Y}"] + recs[0]["argv"], recs[0]["cwd"])]


VAI_TBS = ["tb_cnn_accel_pe_array", "tb_cnn_accel_window_gen"]


def prep_vai(tb, Y, key):
    root = os.environ.get("YAVS_BENCH_VHDL_AI_TEST")
    if not root:
        raise Skip("YAVS_BENCH_VHDL_AI_TEST is not set")
    root = Path(root).expanduser()
    libs = yavs_libs(Y, key, None)
    wd = mkdir(CACHE / "work" / key / "vai")
    marker = wd / "done"
    srcs = []  # (library, file): the module and its testbenches that need no Python, and the hdl-modules libraries
    for d in sorted((root / "hdl-modules/modules").glob("*")):
        srcs += [(d.name, f) for f in sorted((d / "src").rglob("*.vhd"))]
    mod = root / "modules/cnn_accel"
    srcs += [("cnn_accel", f) for d in ("regs_src", "src") for f in sorted((mod / d).glob("*.vhd"))]
    srcs += [("cnn_accel", f) for f in sorted((mod / "test").glob("*.vhd")) if not re.search(r"python_(pkg|context|execute|call)", f.read_text(errors="replace"), re.I)]
    names = sorted({l for l, _ in srcs})
    maps = [f"--map={n}:{wd}/{n}" for n in names] + [f"--map=vunit_lib:{libs}/vunit_lib", f"--map=osvvm:{libs}/osvvm"]
    if not marker.exists():
        pending = srcs  # fixed point: retry what failed until a pass makes no progress
        while pending:
            rest = [(l, f) for l, f in pending if sh([Y, "analyze", f"--std={STD}", "--relaxed", f"--work={l}:{wd}/{l}"]
                                                     + [m for m in maps if not m.startswith(f"--map={l}:")] + [str(f)], wd, check=False).returncode]
            if len(rest) == len(pending):
                break
            pending = rest
        marker.write_text("\n".join(f"{l} {f}" for l, f in pending))
    cfg = f"active python runner : false,seed : 12345,output path : {wd}/,tb path : {mod}/test/,use_color : false"
    return [(["{Y}", "run", f"--std={STD}", "--relaxed", f"--work=cnn_accel:{wd}/cnn_accel"] + maps + [tb, f"-grunner_cfg={cfg}"], str(wd))]


def registry():
    r = {n: ("internal", lambda Y, k, n=n: prep_internal(n, Y, k)) for n in INTERNAL}
    r["vunit_vc30"] = ("external", prep_vunit_vc)
    r["uvvm_axistream_vvc_tb"] = ("external", lambda Y, k: prep_uvvm("bitvis_vip_axistream", "axistream_vvc_tb", Y, k, "10us"))
    r["uvvm_util_rand_tb"] = ("external", lambda Y, k: prep_uvvm("uvvm_util", "rand_tb", Y, k))
    r["uvvm_util_queue_tb"] = ("external", lambda Y, k: prep_uvvm("uvvm_util", "generic_queue_tb", Y, k))
    r["uvvm_sb_slv_tb"] = ("external", lambda Y, k: prep_uvvm("bitvis_vip_scoreboard", "generic_sb_slv_tb", Y, k))
    r["osvvm_axistream"] = ("external", lambda Y, k: prep_osvvm("AxiStream/TbStream_AxiSendGetRandomParam2", Y, k))
    for tb in VAI_TBS:
        r["vai_" + tb.removeprefix("tb_cnn_accel_")] = ("external", lambda Y, k, tb=tb: prep_vai(tb, Y, k))
    return r


# ---- measurement ----

def binary_key(Y):
    return hashlib.sha256(Path(Y).read_bytes()).hexdigest()[:10]


def has_time_report(Y):
    return "--time-report" in sh([Y, "run", "--help"], None, check=False).stdout


TIME = shutil.which("time") or ("/usr/bin/time" if os.path.exists("/usr/bin/time") else None)
PEAK = re.compile(r"^peak-kb (\d+)\n?", re.M)


def run_ru(argv, cwd):
    """subprocess.run(capture_output) with the child's peak RSS in KiB (0 when `time` is missing).
    A wait4 rusage of our own child is floored by this Python's RSS at fork, so `time -f %M` measures."""
    pre = ["timeout", "300"]
    if TIME:
        pre += [TIME, "-f", "peak-kb %M"]
    p = subprocess.run(pre + argv, cwd=cwd, capture_output=True, text=True, errors="replace")
    m = PEAK.findall(p.stderr)
    p.stderr = PEAK.sub("", p.stderr)
    return p, int(m[-1]) if m else 0


def measure(cmds, Y, tr):
    """One iteration of a benchmark: its commands in order; sums of wall, cpu and phases."""
    wall = cpu = rss = fst = gcs = 0.0
    ph = {}
    rc, tail = 0, ""
    for argv, cwd in cmds:
        argv = [Y if a == "{Y}" else a for a in argv]
        if tr:
            argv.insert(2, "--time-report")
        argv[2:2] = [f.replace("{out}", str(OUT)) for f in RUN_FLAGS]
        c0 = resource.getrusage(resource.RUSAGE_CHILDREN)
        t0 = time.perf_counter()
        try:
            p, ru = run_ru(argv, cwd)
        except OSError as e:
            return {"rc": 127, "error": str(e)}
        wall += time.perf_counter() - t0
        c1 = resource.getrusage(resource.RUSAGE_CHILDREN)
        cpu += (c1.ru_utime + c1.ru_stime) - (c0.ru_utime + c0.ru_stime)
        rss = max(rss, ru / 1024)
        if RUN_FLAGS and OUT.exists():
            fst += OUT.stat().st_size
            OUT.unlink()
        if p.returncode and not rc:
            rc, tail = p.returncode, (p.stderr or p.stdout).strip()[-300:]
        gcs += sum(int(x) for x in GC.findall(p.stderr))
        for n, w, c in TR.findall(p.stderr):
            if n != "total":
                d = ph.setdefault(n, {"wall": 0.0, "cpu": 0.0})
                d["wall"] += float(w)
                d["cpu"] += float(c) if c != "n/a" else 0.0
    return {"rc": rc, "wall": wall, "cpu": cpu, "rss": rss, "fst_bytes": fst, "gc": gcs, "phases": ph, "error": tail}


def measure_sim(tpl, sub):
    cmd = ["timeout", "300", "systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=2G", "bash", "-c", _sub(tpl[1], sub)]
    c0 = resource.getrusage(resource.RUSAGE_CHILDREN)
    t0 = time.perf_counter()
    p = subprocess.run(cmd, cwd=sub["work"], capture_output=True, text=True, errors="replace")
    c1 = resource.getrusage(resource.RUSAGE_CHILDREN)
    return p.returncode, time.perf_counter() - t0, (c1.ru_utime + c1.ru_stime) - (c0.ru_utime + c0.ru_stime)


def med(xs):
    return statistics.median(xs) if xs else None


def summarize(samples):
    ok = [s for s in samples if s["rc"] == 0]
    out = {"ok": len(ok), "runs": len(samples), "rc": next((s["rc"] for s in samples if s["rc"]), 0),
           "error": next((s.get("error", "") for s in samples if s["rc"]), ""),
           "rss": med([s.get("rss", 0) for s in ok]), "gc": med([s.get("gc", 0) for s in ok]), "fst_bytes": med([s.get("fst_bytes", 0) for s in ok]), "cpu": med([s["cpu"] for s in ok]), "wall": med([s["wall"] for s in ok]), "phases": {}}
    for p in PHASES:
        v = [s["phases"][p] for s in ok if p in s["phases"]]
        if v:
            out["phases"][p] = {"cpu": med([x["cpu"] for x in v]), "wall": med([x["wall"] for x in v])}
    return out


def fmt(x):
    return "-" if x is None else f"{x:.2f}"


def flag(a, b, thr):
    if a is None or b is None or a <= 0:
        return ""
    r = b / a
    if abs(b - a) < 0.05:
        return ""
    return " !" if r > 1 + thr else " +" if r < 1 - thr else ""


def report(names, labels, summ, thr, sims, simsumm):
    reg = False
    cmp = len(labels) == 2
    for n in names:
        s = [summ[l].get(n) for l in labels]
        if any(x is None for x in s):
            continue
        for l, x in zip(labels, s):
            if x["rc"]:
                print(f"\n{n} [{l}] exit status {x['rc']} in {x['runs'] - x['ok']} of {x['runs']} runs: {x['error']}")
        print(f"\n{n}  ({s[0]['ok']}/{s[0]['runs']} ok" + (f", B {s[1]['ok']}/{s[1]['runs']}" if cmp else "") + ")")
        print(f"  {'phase':10} " + " ".join(f"{l + ' cpu':>10}" for l in labels) + (f" {'B/A':>6}" if cmp else "") + "   wall: " + " ".join(f"{l:>8}" for l in labels))
        rows = [(p, [x["phases"].get(p, {}).get("cpu") for x in s], [x["phases"].get(p, {}).get("wall") for x in s]) for p in PHASES]
        rows.append(("total", [x["cpu"] for x in s], [x["wall"] for x in s]))
        rows.append(("peak MiB", [x.get("rss") for x in s], [None] * len(s)))
        if any(x.get("gc") for x in s):
            rows.append(("collections", [x["gc"] for x in s], [None] * len(s)))
        if any(x.get("fst_bytes") for x in s):
            rows.append(("FST MiB", [x["fst_bytes"] / 1048576 for x in s], [None] * len(s)))
        for p, c, w in rows:
            ratio = f" {c[1] / c[0]:6.2f}{flag(c[0], c[1], thr)}" if cmp and c[0] and c[1] is not None else (" " * 7 if cmp else "")
            print(f"  {p:10} " + " ".join(f"{fmt(v):>10}" for v in c) + ratio + "         " + " ".join(f"{fmt(v):>8}" for v in w))
            if p == "total" and cmp and c[0] and c[1] and c[1] / c[0] > 1 + thr and c[1] - c[0] >= 0.05:
                reg = True
        for lab in sims:
            t = simsumm.get(lab, {}).get(n)
            if t:
                print(f"  {lab:10} total cpu {t:.2f} (elaborate + run)   A/{lab} = {s[0]['cpu'] / t:.2f}")
    if cmp:
        rs = [summ[labels[1]][n]["cpu"] / summ[labels[0]][n]["cpu"] for n in names
              if n in summ[labels[0]] and n in summ[labels[1]] and summ[labels[0]][n]["cpu"] and summ[labels[1]][n]["cpu"]]
        if rs:
            g = statistics.geometric_mean(rs)
            print(f"\ngeometric mean of total CPU ratio B/A over {len(rs)} benchmarks: {g:.3f}")
    return reg


def load_sims(labels):
    spec = importlib.util.spec_from_file_location("discrep_run", REPO / "tools/discrepancies/run.py")
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    for l in labels:  # BENCH_SIM1 etc. win over DISCREP_SIM1 etc.
        if os.environ.get("BENCH_" + l.upper()):
            os.environ["DISCREP_" + l.upper()] = os.environ["BENCH_" + l.upper()]
    sims = m.load_sims(labels)
    sims.pop("yavs", None)
    return sims


def main():
    reg = registry()
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("bin", nargs="?", help="yavs binary (single-binary report)")
    ap.add_argument("--compare", nargs=2, metavar=("A", "B"))
    ap.add_argument("--bench", help="comma separated benchmark names")
    ap.add_argument("--group", default="all", choices=["internal", "external", "all"])
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--threshold", type=float, default=0.10)
    ap.add_argument("--sims", default="", help="reference simulator labels, e.g. sim1,sim2 (internal designs only)")
    ap.add_argument("--out", default=str(CACHE))
    ap.add_argument("--label", default="", help="tag stored in the JSON file name")
    ap.add_argument("--fail-on-regression", action="store_true")
    ap.add_argument("--list", action="store_true")
    a = ap.parse_args()
    if a.list:
        for n, (g, _) in reg.items():
            print(f"{g:9} {n}")
        return 0
    bins = a.compare or ([a.bin] if a.bin else None)
    if not bins:
        ap.error("give a yavs binary or --compare A B")
    labels = ["A", "B"] if a.compare else ["A"]
    bins = [str(Path(b).expanduser().resolve()) for b in bins]
    names = a.bench.split(",") if a.bench else [n for n, (g, _) in reg.items() if a.group in ("all", g)]
    unknown = [n for n in names if n not in reg]
    if unknown:
        ap.error(f"unknown benchmark(s) {unknown}; see --list")
    keys = [binary_key(b) for b in bins]
    trs = [has_time_report(b) for b in bins]
    cmds, skipped = {}, {}
    for n in names:
        for l, b, k in zip(labels, bins, keys):
            try:
                cmds[(l, n)] = reg[n][1](b, k)
            except Skip as e:
                skipped[n] = str(e)
            except Exception as e:
                skipped[n] = f"prepare failed: {e}"
    names = [n for n in names if n not in skipped]
    for n, why in skipped.items():
        print(f"skipped {n}: {why}", file=sys.stderr)
    samples = {(l, n): [] for l in labels for n in names}
    for rep in range(a.runs):
        order = list(zip(labels, bins, trs))
        if rep % 2:
            order.reverse()  # alternate A/B order so a drifting host load hits both
        for n in names:
            for l, b, tr in order:
                samples[(l, n)].append(measure(cmds[(l, n)], b, tr))
            print(f"rep {rep + 1}/{a.runs} {n}", file=sys.stderr)
    summ = {l: {n: summarize(samples[(l, n)]) for n in names} for l in labels}
    sims = load_sims([s for s in a.sims.split(",") if s]) if a.sims else {}
    simsumm = {}
    for lab, tpl in sims.items():
        simsumm[lab] = {}
        for n in names:
            if n not in INTERNAL:
                continue
            work = mkdir(CACHE / "work" / "sims" / lab / n)
            sub = dict(file=str(HERE / "designs" / f"{n}.vhd"), top=f"bench_{n}", std=STD, std2=STD[2:], work=str(work), res="fs")
            try:
                subprocess.run(["bash", "-c", _sub(tpl[0], sub)],
                               cwd=work, capture_output=True, timeout=300)
                runs = [measure_sim(tpl, sub) for _ in range(a.runs)]
            except Exception:
                continue
            good = [c for rc, w, c in runs if rc == 0]
            if good:
                simsumm[lab][n] = med(good)
    reg_flag = report(names, labels, summ, a.threshold, list(sims), simsumm)
    meta = {"time": time.strftime("%Y-%m-%dT%H:%M:%S"), "runs": a.runs, "host_cpus": os.cpu_count(),
            "binaries": {l: {"path": b, "sha256_10": k, "time_report": t} for l, b, k, t in zip(labels, bins, keys, trs)},
            "skipped": skipped}
    res = mkdir(Path(a.out).expanduser() / "results") / f"{time.strftime('%Y%m%d-%H%M%S')}{'-' + a.label if a.label else ''}.json"
    res.write_text(json.dumps({"meta": meta, "summary": summ, "sims": simsumm,
                               "samples": {f"{l}/{n}": v for (l, n), v in samples.items()}}, indent=1))
    print(f"\nresults: {res}")
    return 1 if (reg_flag and a.fail_on_regression) else 0


def _sub(snippet, sub):
    for k, v in sub.items():
        snippet = snippet.replace("{" + k + "}", v)
    return snippet


if __name__ == "__main__":
    sys.exit(main())
