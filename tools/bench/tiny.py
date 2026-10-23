#!/usr/bin/env python3
"""Head-to-head on the tiny-test corpus (tests/corpus): per-test wall time of one run, yavs against references.

  tiny.py BIN --ref sim1,sim2 [--runs 5] [--cores 0-7] [--tiers interp,auto] [--filter SUBSTR] [--out DIR]

Per test: analysis once per tool (not timed), one untimed warm-up run, then --runs timed runs with the tools
interleaved (order alternating every rep); the wrapper's own cost is subtracted per cell: three runs of `true`
under the same wrapper are timed right before each timed run, so the subtracted value follows the host load (#569).
The whole process runs once under `taskset` (every tool on the same cores) and a 2 GiB memory cap. Tests a tool
cannot analyse, elaborate or finish are left out for that tool; the summary also gives the set all tools ran.

Reference labels map to commands exactly as for `run.py --ref` (BENCH_<LABEL>, DISCREP_<LABEL> or the untracked
tools/sims.local.conf, key `bench` before `discrep`). For this corpus the template must use the test
header's environment variables (below). Placeholders:
{file} {top} {std} {std2} {work} {res}; environment variables of the test header: STOP_FLAG
(`--stop-time=T` or empty), RELAXED and PSL (`1` or empty).
Stdlib only. Run with `python3 -I`.
"""
import argparse, importlib.util, json, os, shlex, shutil, statistics as st, subprocess, sys, time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    m = importlib.util.module_from_spec(spec)
    sys.modules[name] = m
    spec.loader.exec_module(m)
    return m


bench = load("bench_run", HERE / "run.py")
dt = load("difftest", REPO / "tools/difftest/difftest.py")
BAD_RC = {97, 98, 124, 134, 137, 139}


def tm(script, cwd, env=None, to=120):
    t0 = time.perf_counter_ns()
    try:
        rc = subprocess.run(["timeout", str(to), "bash", "-c", script], cwd=cwd, capture_output=True, env={**os.environ, **(env or {})}).returncode
    except OSError:
        rc = 999
    return (time.perf_counter_ns() - t0) / 1e9, rc


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("bin")
    ap.add_argument("--ref", required=True)
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--cores", default="")
    ap.add_argument("--tiers", default="interp")
    ap.add_argument("--filter", default="")
    ap.add_argument("--out", default=str(bench.CACHE))
    ap.add_argument("--label", default="")
    a = ap.parse_args()
    cores = a.cores or bench.default_cores()
    if not os.environ.get("YAVS_TINY_SCOPED"):  # pin and cap the whole run once: the children inherit both
        os.environ["YAVS_TINY_SCOPED"] = "1"
        os.execvp("systemd-run", ["systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=2G", "taskset", "-c", cores,
                                  sys.executable, "-I"] + sys.argv)
    Y = str(Path(a.bin).expanduser().resolve())
    refs = bench.ref_templates([s for s in a.ref.split(",") if s])
    tiers = [s for s in a.tiers.split(",") if s]
    tools = ["yavs-" + t for t in tiers] + list(refs)
    work = bench.mkdir(bench.CACHE / "work" / "tiny")
    print(f"pinned to cpus {cores}; median of {a.runs}; load {open('/proc/loadavg').read().split()[:3]}", file=sys.stderr)
    plans, ok = {}, {}
    for t in sorted((REPO / "tests/corpus").rglob("*.vhd")):
        key = str(t.relative_to(REPO / "tests/corpus"))
        if a.filter not in key:
            continue
        h = dt.parse_header(t)
        d = work / key.replace("/", "__").removesuffix(".vhd")
        shutil.rmtree(d, ignore_errors=True)
        std, top = h["std"], h["top"]
        rel, psl, stop = h["relaxed"] is not None, h["psl"] is not None, h["stop"]
        ya = [f"--std={std}", f"--work=work:{d}/y"] + (["--relaxed", "-Wno-W1904"] if rel else []) + (["--psl"] if psl else [])
        yr = ya + ([f"--stop-time={stop}"] if stop else []) + ([f"--resolution={h['res']}"] if h["res"] else [])
        q = lambda l: " ".join(shlex.quote(x) for x in l)
        scripts, ana = {}, {}
        ana["yavs"] = q([Y, "analyze"] + ya + [str(t)])
        for tier in tiers:
            scripts["yavs-" + tier] = q([Y, "run", f"--tier={tier}"] + yr + [top])
        env = {"STOP_FLAG": f"--stop-time={stop}" if stop else "", "RELAXED": "1" if rel else "", "PSL": "1" if psl else ""}
        for lab, tpl in refs.items():
            sub = dict(file=shlex.quote(str(t)), top=top, std=std, std2={"1993": "93", "2002": "02", "2008": "08", "2019": "19"}[std],
                       work=f"{d}/{lab}", res=h["res"] or "fs")
            ana[lab] = bench._sub(tpl[0], sub)
            scripts[lab] = bench._sub(tpl[1], sub)
        d.mkdir(parents=True)
        good = {}
        for tool, s in ana.items():
            rc = tm(s, d, env)[1]
            for k in ([tool] if tool != "yavs" else ["yavs-" + x for x in tiers]):
                good[k] = rc == 0
        plans[key] = (scripts, d, env)
        ok[key] = good
    print(f"analysed {len(plans)} tests", file=sys.stderr, flush=True)
    data, ovs = {}, []
    for key, (scripts, d, env) in plans.items():
        for tool in tools:  # warm-up, and drop what crashes or times out
            if ok[key][tool]:
                ok[key][tool] = tm(scripts[tool], d, env)[1] not in BAD_RC
    for rep in range(a.runs):
        for key, (scripts, d, env) in plans.items():
            order = [t for t in tools if ok[key][t]]
            for tool in (order if rep % 2 == 0 else order[::-1]):
                ov = st.median([tm("true", "/")[0] for _ in range(3)])  # interleaved baseline, not one number from the start
                ovs.append(ov)
                data.setdefault(key, {}).setdefault(tool, []).append(tm(scripts[tool], d, env)[0] - ov)
        print(f"rep {rep + 1}/{a.runs}", file=sys.stderr, flush=True)
    q = st.quantiles(ovs, n=10) if len(ovs) > 1 else ovs * 9
    print(f"wrapper overhead per cell: median {st.median(ovs) * 1000:.2f} ms, p10 {q[0] * 1000:.2f}, p90 {q[-1] * 1000:.2f}, max {max(ovs, default=0) * 1000:.2f} ms", file=sys.stderr)
    summary(data, ok, tools, refs, [t for t in tools if t.startswith("yavs")])
    out = bench.mkdir(Path(a.out).expanduser() / "results") / f"{time.strftime('%Y%m%d-%H%M%S')}-tiny{'-' + a.label if a.label else ''}.json"
    out.write_text(json.dumps({"ov": ovs, "cores": cores, "runs": a.runs, "ok": ok, "data": data}))
    print(f"\nresults: {out}")


def summary(data, ok, tools, refs, yv):
    keys = list(ok)
    m = lambda k, t: st.median(data[k][t])
    common = [k for k in keys if all(ok[k][t] and t in data.get(k, {}) for t in tools)]
    for lab, ks in (("runnable per tool", None), (f"common set ({len(common)} tests)", common)):
        print(f"\n{lab}")
        for t in tools:
            kk = ks if ks is not None else [k for k in keys if ok[k][t] and t in data.get(k, {})]
            if not kk:
                continue
            pt = sorted(m(k, t) for k in kk)
            reps = len(data[kk[0]][t])
            tot = st.median([sum(data[k][t][i] for k in kk) for i in range(reps)])
            print(f"  {t:14} n={len(kk):4} total {tot:7.2f} s  median/test {st.median(pt) * 1000:7.1f} ms  p90 {pt[int(.9 * len(pt))] * 1000:7.1f} ms  unrunnable {sum(not ok[k][t] for k in keys)}")
    for y in yv:
        for r in refs:
            if common:
                print(f"  {y} faster than {r}: {sum(m(k, y) < m(k, r) for k in common)}/{len(common)}")
    if common and yv and refs:
        r0 = next(iter(refs))
        worst = sorted(common, key=lambda k: -m(k, yv[0]) / max(m(k, r0), 1e-4))[:10]
        print(f"\nworst ratios {yv[0]} / {r0}")
        for k in worst:
            print(f"  {k}: " + "  ".join(f"{t} {m(k, t) * 1000:.0f} ms" for t in tools) + f"  ratio {m(k, yv[0]) / max(m(k, r0), 1e-4):.1f}")


if __name__ == "__main__":
    main()
