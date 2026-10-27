#!/usr/bin/env python3
"""Waveform equivalence of yavs against the reference simulators on real designs (#781).

  run.py BIN [--designs a,b | --group h2h|real|all] [--refs sim1,sim2] [--tiers interp,auto] [--jobs 2]
  run.py --report RESULTS.json
  run.py --self-test

Runs every design on yavs (one run per tier) and on the reference simulators, each dumping its
signals to a waveform file, then compares the files signal by signal with the waveq comparer
(crates/yavs-wave/examples/waveq.rs, built on demand). Pairs: each yavs tier against the first
reference that ran the design, and every further reference against that one (the calibration:
where the references disagree with each other, the difference is theirs). Results (JSON and
markdown) go to --out. Python is stdlib only; run with `python3 -I`. See AGENTS.md.
"""
import argparse, concurrent.futures as cf, fnmatch, importlib.util, json, os, re, shlex, shutil, subprocess, sys, time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(REPO / "tools"))
import simconf  # noqa: E402

OUT = Path(os.environ.get("YAVS_WAVEQ_CACHE", "~/.cache/yavs-waveq")).expanduser()
# the shared std/ieee store (#564) is private to this tool (realbench sets its own for its designs)
os.environ["YAVS_STD_CACHE"] = str(OUT / "std")

# h2h designs (tools/bench/designs/<name>.vhd, top bench_<name>) with generics that keep the waves small;
# compute_h2h and xor_types have no signals, xor_sig is rejected by the references (#782)
H2H = {
    "rtl_ff": ["N=256", "CYC=200"], "rtl_lfsr": ["N=64", "CYC=500"], "rtl_chain": ["N=100", "CYC=200"],
    "rtl_fan": ["N=200", "CYC=200"], "rtl_bus": ["CYC=500"], "rtl_ram": ["CYC=2000"],
    "lfsr64_xor": ["CYCLES=500"], "lfsr64": ["CYCLES=500"],
}
# realbench tests (tools/realbench/tests.toml): name -> stop time for the wave run (None: the test's own) and comparer skip globs
REAL = {
    "neorv32_single_min": (None, []),
    "grlib_leon3mp": (None, []),
    "uvvm_sb_slv": (None, []), "uvvm_util_rand": (None, []), "uvvm_axistream_vvc": (None, []),
    "uvvm_methods_log": (None, []), "uvvm_func_cov": (None, []),
    "osvvm_axis_random": (None, []), "osvvm_axi4_random": (None, []),
}
YAVS_MEM, REF_MEM = "6G", "2G"


def cap(mem):
    return ["timeout", "300", "systemd-run", "--user", "--scope", "-q", "-p", f"MemoryMax={mem}"]


def bash(script, cwd, mem):
    """(exit status, output tail, compiled code of a yavs run: functions --tier=auto compiled (its --verbose notes), IR
    instructions --tier=llvm compiled inline (counter), else None)."""
    p = subprocess.run(cap(mem) + ["bash", "-c", script], cwd=cwd, capture_output=True, text=True, errors="replace", env=os.environ | {"YAVS_COUNTERS": "1"})
    out = p.stdout + p.stderr
    m = re.search(r"llvm_inline_insts=(\d+)", out)
    auto = len(re.findall(r"compiled by --tier=auto", out))
    code = f"{auto} functions" if auto else f"{m[1]} IR instructions" if m and m[1] != "0" else "0" if m else None
    return p.returncode, out[-1500:], code


def fill(tmpl, **kw):
    return tmpl.replace("{{", "\0").replace("}}", "\1").format(**kw).replace("\0", "{").replace("\1", "}")


def realbench():
    spec = importlib.util.spec_from_file_location("realbench_run", REPO / "tools/realbench/run.py")
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


# ---- one design on one tool: (status, waveform file or None, output tail) ----

def run_h2h(name, tool, Y, tier, d):
    f = REPO / "tools/bench/designs" / f"{name}.vhd"
    top, gens = f"bench_{name}", H2H[name]
    work = d / f"work-{tool}"
    shutil.rmtree(work, ignore_errors=True)
    if tool == "yavs":
        wave = d / f"yavs-{tier}.fst"
        s = (f"{shlex.quote(Y)} analyze --std=2008 --work=work:{work}/work {f} && "
             f"{shlex.quote(Y)} run --std=2008 --work=work:{work}/work --tier={tier} --verbose --wave={wave} {top} " + " ".join(f"-g{g}" for g in gens))
        mem = YAVS_MEM
    else:
        tmpl = simconf.get(tool, "waveq")
        if not tmpl:
            return "not configured", None, f"{tool}: key waveq not set in tools/sims.local.conf", None
        wave = d / f"{tool}.wave"
        an, rn = tmpl.split("|||")
        kw = dict(std="2008", std2="08", work=str(work), file=str(f), top=top, gens=" ".join(f"-g{g}" for g in gens), stop_flag="", wave=str(wave))
        s = f"mkdir -p {work} && {fill(an, **kw)} && {fill(rn, **kw)}"
        mem = REF_MEM
    rc, tail, frames = bash(s, d, mem)
    return ("ok" if rc == 0 else f"exit {rc}"), (wave if wave.exists() else None), tail, frames


def real_test(R, name):
    t = dict(next(x for x in R.load_tests() if x["name"] == name))
    t["stop"] = REAL[name][0] or t["stop"]
    return t


def run_real(R, t, tool, Y, key, tier, d, world):
    """t: the test, one copy per tool (a driver test's world setup fills in its run fields)."""
    if tool == "yavs":
        wave = d / f"yavs-{tier}.fst"
        if not world.exists():
            R.make_pristine(t, "yavs", Y, key, world)
        s = R.yavs_script(Y, t, world, tier, extra=[f"--wave={wave}", "--verbose"])
        mem = YAVS_MEM
    else:
        flags = simconf.get(tool, "waveq_flags")
        if not flags or not simconf.get(tool, "real_run"):
            return "not configured", None, f"{tool}: keys waveq_flags and real_run not set in tools/sims.local.conf", None
        wave = d / f"{tool}.wave"
        R.make_pristine(t, tool, None, None, world)
        s = R.ref_script(tool, t, world, extra=fill(flags, wave=str(wave)))
        mem = REF_MEM
    rc, tail, frames = bash(s, world, mem)
    return ("ok" if rc == 0 else f"exit {rc}"), (wave if wave.exists() else None), tail, frames


def compare(cmp, a, b, d, skip, until):
    j = d / f"cmp-{a.stem}-{b.stem}.json"
    argv = [cmp, str(a), str(b), "--json", str(j)] + [x for g in skip for x in ("--skip", g)] + (["--until", str(until)] if until else [])
    p = subprocess.run(cap("6G") + argv, capture_output=True, text=True, errors="replace")
    if p.returncode not in (0, 1) or not j.exists():
        return {"error": f"comparer exit {p.returncode}: {(p.stdout + p.stderr)[-400:]}"}
    return json.loads(j.read_text())


STOP_RE = re.compile(r"^(\d+)\s*(fs|ps|ns|us|ms)$")


def fs_of(stop):
    m = STOP_RE.match(stop or "")
    return int(m[1]) * 10 ** {"fs": 0, "ps": 3, "ns": 6, "us": 9, "ms": 12}[m[2]] if m else None


def design(name, Y, key, tiers, refs, cmp, R):
    kind = "h2h" if name in H2H else "real"
    d = OUT / "runs" / name
    shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    runs = {}
    worlds = []
    tests = {}
    try:
        for tool in [("yavs", t) for t in tiers] + [(r, None) for r in refs]:
            label = f"yavs-{tool[1]}" if tool[0] == "yavs" else tool[0]
            t0 = time.time()
            if kind == "h2h":
                st, wave, tail, frames = run_h2h(name, tool[0], Y, tool[1], d)
            else:
                world = R.CACHE / "worlds" / f"waveq-{os.getpid()}-{name}-{'yavs' if tool[0] == 'yavs' else tool[0]}"
                worlds.append(world)
                try:
                    t = tests.setdefault(tool[0], real_test(R, name))
                    st, wave, tail, frames = run_real(R, t, tool[0], Y, key, tool[1], d, world)
                except R.Fail as e:
                    st, wave, tail, frames = "setup failed", None, str(e), None
            runs[label] = {"status": st, "wave": str(wave) if wave else None, "seconds": round(time.time() - t0, 1),
                           "bytes": wave.stat().st_size if wave else 0, "tail": tail if st != "ok" else "", "compiled": frames}
            print(f"  {name} {label}: {st} ({runs[label]['seconds']}s, {runs[label]['bytes'] >> 20} MB)", flush=True)
    finally:
        for w in worlds:
            shutil.rmtree(w, ignore_errors=True)
    skip = REAL.get(name, (None, []))[1]
    until = fs_of(tests["yavs"]["stop"]) if "yavs" in tests else None
    base = next((r for r in refs if runs.get(r, {}).get("wave")), None)
    pairs = []
    if base:
        pairs = [(f"yavs-{t}", base) for t in tiers] + [(r, base) for r in refs if r != base and runs.get(r, {}).get("wave")]
    out = {"kind": kind, "runs": runs, "base": base, "pairs": []}
    for a, b in pairs:
        if not runs[a]["wave"]:
            out["pairs"].append({"a": a, "b": b, "error": f"{a}: {runs[a]['status']}"})
            continue
        res = compare(cmp, Path(runs[a]["wave"]), Path(runs[b]["wave"]), d, skip, until)
        res.update(a=a, b=b)
        out["pairs"].append(res)
    for w in d.glob("work-*"):
        shutil.rmtree(w, ignore_errors=True)
    return out


# Differences explained so far: (design, glob over the lower-case signal name, verdict, reference). A verdict is
# `bug` (a yavs issue), `reference` (the references differ, entry of docs/design/simulator-discrepancies.md) or `noise`
# (the dump format; fixed in the comparer where it can be). A difference no entry matches is `unexplained`.
# Each entry holds only for a first difference within [from, to] fs (to None: the end), so a later regression elsewhere
# in the design stays unexplained.
KNOWN = [
    ("osvvm_*", "*.id", 0, 0, "reference", "process-order-in-cycle"),  # NewID order of the VC processes at their first resumption
    ("osvvm_*", "*fifoid", 0, 0, "reference", "process-order-in-cycle"),
    ("osvvm_*", "*.intfrommodel", 0, 20_000_000, "reference", "process-order-in-cycle"),  # the ID a VC hands back at its start
    ("uvvm_axistream_vvc", "*entry_num_in_vvc_activity_register", 0, 0, "reference", "process-order-in-cycle"),
]

WHY = {
    "process-order-in-cycle": "the verification components take IDs from a shared counter in the order their processes run in one "
                              "cycle; the LRM leaves that order open (14.7.5.3) and the references differ among themselves "
                              "(docs/design/simulator-discrepancies.md); every bus signal is equal",
}


def verdict(name, signal, t):
    s = signal.lower()
    return next(((v, r) for d, g, lo, hi, v, r in KNOWN if fnmatch.fnmatchcase(name, d) and fnmatch.fnmatchcase(s, g) and lo <= t and (hi is None or t <= hi)),
                ("unexplained", ""))


def compact(res):
    """The result in the form the conformance report reads (tools/waveq/AGENTS.md section 3): one row per design and pair."""
    rows = []
    for n, r in res["designs"].items():
        for p in r["pairs"] or [{"a": "", "b": "", "error": "no reference waveform: " + ", ".join(f"{k} {v['status']}" for k, v in r["runs"].items())}]:
            row = {"design": n, "kind": r["kind"], "a": p["a"], "b": p["b"], "compiled": r["runs"].get(p["a"], {}).get("compiled")}
            if "error" in p:
                rows.append(row | {"error": p["error"]})
                continue
            diffs = [{"signal": d["name"], "time_fs": d["time_fs"], **dict(zip(("verdict", "ref"), verdict(n, d["name"], d["time_fs"])))} for d in p["diffs"]]
            counts = {}
            for d in diffs:
                k = f"{d['verdict']} {d['ref']}".strip()
                counts[k] = counts.get(k, 0) + 1
            rows.append(row | {k: p[k] for k in ("signals", "changes", "equal", "different", "sliced", "until_fs")}
                        | {"other_shape": len(p["shape"]), "only_a": len(p["only_a"]), "only_b": len(p["only_b"]), "verdicts": counts, "diffs": diffs[:50]})
    status = {n: {k: v["status"] for k, v in r["runs"].items()} for n, r in res["designs"].items()}
    return {"run": res["meta"], "status": status, "rows": rows}


def markdown(c):
    m = c["run"]
    s = [f"Waveform equivalence: run locally on {m['date']} at {m['commit']} (yavs build {m['yavs_key']}), references {', '.join(m['refs'])}.",
         f"Reproduce: `{m['command']}`", "",
         "| design | A | B | signals | value changes | equal | different | equal as slice | other shape | only A | only B | differences | A compiled code |",
         "|---|---|---|---|---|---|---|---|---|---|---|---|---|"]
    for r in c["rows"]:
        if "error" in r:
            s.append(f"| {r['design']} | {r['a']} | {r['b']} | {r['error'][:160]} | | | | | | | | | |")
            continue
        v = ", ".join(f"{k}: {n}" for k, n in sorted(r["verdicts"].items())) or "none"
        s.append(f"| {r['design']} | {r['a']} | {r['b']} | {r['signals']} | {r['changes']} | {r['equal']} | {r['different']} | "
                 f"{r['sliced']} | {r['other_shape']} | {r['only_a']} | {r['only_b']} | {v} | {'' if r.get('compiled') is None else r['compiled']} |")
    bad = [f"{n} {k}: {st}" for n, runs in c["status"].items() for k, st in runs.items() if st not in ("ok", "not configured")]
    if bad:
        s += ["", "Runs that did not end with status 0 (compared up to where their waveform ends): " + "; ".join(bad)]
    nc = {}
    for runs in c["status"].values():
        for k, st in runs.items():
            nc[k] = nc.get(k, 0) + (st == "not configured")
    if any(nc.values()):
        s += ["", "Not configured (no waveform keys for the kind of design in tools/sims.local.conf): " + ", ".join(f"{k} for {n} designs" for k, n in nc.items() if n)]
    seen = sorted({k for r in c["rows"] for k in r.get("verdicts", {}) if not k.startswith("unexplained")})
    if seen:
        s += ["", "Differences explained:", ""] + [f"- {k}: {WHY.get(k.split(' ', 1)[-1], 'see KNOWN in run.py')}" for k in seen]
    un = [(r, d) for r in c["rows"] for d in r.get("diffs", []) if d["verdict"] == "unexplained"]
    if un:
        s += ["", "Unexplained differences (first time per signal):", ""] + [f"- {r['design']} {r['a']} vs {r['b']}: `{d['signal']}` at {d['time_fs']} fs" for r, d in un[:100]]
    return "\n".join(s) + "\n"


def self_test():
    assert fs_of("100us") == 100 * 10 ** 9 and fs_of("1060500ns") == 1060500 * 10 ** 6 and fs_of(None) is None
    assert fill("a {wave} {{x}}", wave="w.fst") == "a w.fst {x}"
    assert verdict("osvvm_axi4_random", "TbAxi4.Manager_1.ArrDelayCovID(1).ID", 0) == ("reference", "process-order-in-cycle")
    assert verdict("osvvm_axi4_random", "TbAxi4.Manager_1.ArrDelayCovID(1).ID", 5) == ("unexplained", "")
    assert verdict("uvvm_axistream_vvc", "tb.axistream_if_m.tdata", 1000) == ("unexplained", "")
    assert verdict("uvvm_axistream_vvc", "tb.axistream_if_m.tdata", 266_872_500_000) == ("unexplained", "")
    assert verdict("rtl_ff", "x.id", 0) == ("unexplained", "")
    pair = {"a": "yavs-interp", "b": "sim1", "signals": 3, "changes": 9, "equal": 2, "different": 1, "sliced": 0, "until_fs": 5, "shape": [],
            "only_a": ["x"], "only_b": [], "diffs": [{"name": "t.x", "time_fs": 5, "a": [], "b": []}]}
    c = compact({"meta": {"date": "2026-01-01", "commit": "abc", "yavs_key": "k", "refs": ["sim1"], "command": "c"},
                 "designs": {"d": {"kind": "h2h", "runs": {"sim1": {"status": "ok"}}, "pairs": [pair]}}})
    assert c["rows"][0]["verdicts"] == {"unexplained": 1} and c["rows"][0]["only_a"] == 1, c
    md = markdown(c)
    assert "| d | yavs-interp | sim1 | 3 | 9 | 2 | 1 | 0 | 0 | 1 | 0 | unexplained: 1 |  |" in md and "`t.x` at 5 fs" in md, md
    assert "run locally on 2026-01-01 at abc" in md and "Differences explained" not in md
    print("self-test ok")


def comparer():
    """Build the comparer (release) and return its path."""
    target = Path(os.environ.get("CARGO_TARGET_DIR", REPO / "target"))
    p = subprocess.run(["cargo", "build", "--release", "-q", "-p", "yavs-wave", "--example", "waveq"], cwd=REPO)
    if p.returncode:
        sys.exit("waveq: building the comparer failed")
    return str(target / "release/examples/waveq")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("bin", nargs="?", help="yavs binary (a build with the llvm feature for --tiers auto)")
    ap.add_argument("--designs", default="", help="comma separated design names")
    ap.add_argument("--group", default="all", choices=["h2h", "real", "all"])
    ap.add_argument("--refs", default="sim1,sim2")
    ap.add_argument("--tiers", default="interp,auto,llvm", help="yavs tiers (auto compiles only hot code after 5 ms; llvm compiles everything first)")
    ap.add_argument("--jobs", type=int, default=2, help="designs at once (at most 2: heavy runs, AGENTS.md)")
    ap.add_argument("--comparer", default="", help="path of a built waveq comparer (default: build it)")
    ap.add_argument("--keep", action="store_true", help="keep the waveform files")
    ap.add_argument("--report", metavar="JSON")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if a.report:
        print(markdown(json.loads(Path(a.report).read_text())), end="")
        return 0
    if not a.bin or not Path(a.bin).is_file():
        sys.exit("waveq: give a yavs binary")
    names = [x for x in a.designs.split(",") if x] or [n for n in [*H2H, *REAL] if a.group == "all" or (n in H2H) == (a.group == "h2h")]
    if bad := [n for n in names if n not in H2H and n not in REAL]:
        sys.exit(f"waveq: unknown design(s) {bad}; known: {', '.join([*H2H, *REAL])}")
    Y = str(Path(a.bin).resolve())
    refs = [r for r in a.refs.split(",") if r]
    tiers = [t for t in a.tiers.split(",") if t]
    cmp = a.comparer or comparer()
    R = realbench() if any(n in REAL for n in names) else None
    key = R.B.binary_key(Y) if R else Y
    if R:
        R.RECORDER = (Y, key)
    commit = subprocess.run(["git", "rev-parse", "--short=10", "HEAD"], cwd=REPO, capture_output=True, text=True).stdout.strip()
    res = {"meta": {"date": time.strftime("%Y-%m-%d"), "commit": commit, "yavs_key": key if R else "", "refs": refs, "tiers": tiers,
                    "command": " ".join(["python3 -I tools/waveq/run.py BIN"] + [x for x in sys.argv[2:] if x != a.comparer and x != "--comparer"])}, "designs": {}}
    with cf.ThreadPoolExecutor(max(1, min(a.jobs, 2))) as ex:
        futs = {n: ex.submit(design, n, Y, key, tiers, refs, cmp, R) for n in names}
        for n in names:
            res["designs"][n] = futs[n].result()
            if not a.keep:
                for r in res["designs"][n]["runs"].values():
                    if r["wave"]:
                        Path(r["wave"]).unlink(missing_ok=True)
    OUT.mkdir(parents=True, exist_ok=True)
    f = OUT / f"{time.strftime('%Y%m%d-%H%M%S')}.json"
    f.with_suffix(".full.json").write_text(json.dumps(res, indent=1))
    c = compact(res)
    f.write_text(json.dumps(c, indent=1) + "\n")
    f.with_suffix(".md").write_text(markdown(c))
    print(markdown(c), end="")
    print(f"results: {f} and {f.with_suffix('.md')} (per signal: {f.with_suffix('.full.json')} and {OUT / 'runs'})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
