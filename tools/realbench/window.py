#!/usr/bin/env python3
"""Short-window A/B on Cranelift against sim1: each test runs a fixed simulated-time window after start-up under perf stat, on one core.

  window.py [A [B ...]] [--bar cycles:0.97] [--tests a,b] [--rounds N] [--window T0,T1] [--full] [--inherit] [--no-sim1] [--core N] [--json F]
  window.py --self-test

A variant is CHECKOUT_OR_BINARY[:TIER] (default tier auto; TIER takes `+VAR=VALUE`). A checkout is built with Cranelift only; with no
variant, this checkout. sim1 is always added unless --no-sim1. Ratios are against the first variant and against sim1. A window is the
counters of a run to T1 minus those of a run to T0. Ratios to the first variant are paired per round, with a 95 percent band; a bar inside the
band is `undecidable`. See AGENTS.md section 8."""
import argparse, fcntl, hashlib, importlib.util, json, math, os, re, shlex, shutil, subprocess, sys, time
from pathlib import Path

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("realbench_run", HERE / "run.py")
R = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(R)
B = R.B
Fail = R.Fail

# test -> (T0, T1). Validated against whole runs: AGENTS.md section 8 has the table and what does not hold.
WINDOWS = {
    "grlib_leon3mp": ("300us", "700us"),
    "vai_window_gen_interlock": ("64us", "512us"),
    "fletcher_bufferwriter_512": ("8us", "256us"),
    "sc5_fifo_repack": ("20us", "400us"),
    "sc5_timestamp": ("100us", "2500us"),
    "fletcher_bufferreader": ("16us", "512us"),
    "hdlm_lfsr": ("128us", "4ms"),
    "neorv32_single_cache": ("40us", "280us"),
    "microwatt_coremark": ("300us", "800us"),
}
FE_EVENT = "de_no_dispatch_per_slot.no_ops_from_frontend:u"  # Zen 4: dispatch slots with no op from the front end (`--fe`)
EVENTS = {"cycles": "cycles:u", "instructions": "instructions:u", "l1i": "L1-icache-load-misses:u", "itlb": "iTLB-load-misses:u", "branch": "branch-misses:u"}
METRICS = tuple(EVENTS)
REF = "sim1"
TIMED_CORES = range(9, 16)  # root rule: CCD1 without core 8, siblings idle
SETUP_CPUS = set(range(8)) | set(range(16, 24))  # CCD0: builds and everything untimed


def parse_variant(spec, refs=("sim1", "sim2")):
    """(label placeholder, binary or None, tier): `sim1` is a reference, anything else PATH[:TIER]."""
    if spec in refs:
        return {"tool": spec, "path": None, "tier": None, "name": spec}
    path, _, tier = spec.partition(":")
    tier = tier or "auto"
    p = Path(path).expanduser().resolve()
    return {"tool": "yavs", "path": str(p), "tier": tier, "name": f"{p.name}:{tier}"}


def build(checkout):
    """A checkout's Cranelift bench build, in a target directory of its own; returns the binary."""
    root = Path(checkout).resolve()
    if not (root / "tools/build-bench.sh").exists():
        raise Fail(f"{root} is a directory but not a yavs checkout (no tools/build-bench.sh)")
    target = Path("~/.cache").expanduser() / f"yavs-window-build-{hashlib.sha256(str(root).encode()).hexdigest()[:12]}"
    print(f"building {root} (Cranelift) in {target}", flush=True)
    env = {**os.environ, "CARGO_TARGET_DIR": str(target)}
    cmd = [str(R.REPO / "tools/claim"), "build", "--", "systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=10G", "taskset", "-c", "0-5,16-21", "tools/build-bench.sh", "--features", "clift"]
    if subprocess.run(cmd, env=env, cwd=root).returncode:
        raise Fail(f"the Cranelift build of {root} failed; the compiler's output is above")
    return str(target / "release/yavs")


def parse_bars(specs):
    bars = {}
    for spec in specs:
        m, _, v = spec.partition(":")
        if m not in METRICS:
            raise Fail(f"--bar {spec}: unknown counter `{m}`; known: {', '.join(METRICS)}. Write it as cycles:0.97 (ratio to the first variant must be at most 0.97)")
        try:
            bars[m] = float(v)
        except ValueError:
            raise Fail(f"--bar {spec}: `{v}` is not a ratio; write it as cycles:0.97") from None
    return bars


# Two-sided 95 percent t quantiles by degrees of freedom: a handful of rounds gives a band that is honest about how little it knows.
T95 = {1: 12.71, 2: 4.30, 3: 3.18, 4: 2.78, 5: 2.57, 6: 2.45, 7: 2.36, 8: 2.31, 9: 2.26, 10: 2.23, 11: 2.20, 12: 2.18, 13: 2.16, 14: 2.14, 15: 2.13}
MIN_ROUNDS = 3  # fewest rounds that decide a counter other than instructions (AGENTS.md section 8: instructions are good to under 1 percent)


def paired(qs):
    """(ratio, low, high, n) from per-round ratios to the first variant: the geomean, and its 95 percent band from the spread of the
    rounds' logs. Rounds are paired (both variants ran seconds apart, so the host's drift cancels); one round has no band."""
    qs = [q for q in qs if q and q > 0]
    if not qs:
        return None
    logs = list(map(math.log, qs))
    mu = sum(logs) / len(logs)
    if len(logs) < 2:
        return (math.exp(mu), math.exp(mu), math.exp(mu), 1)
    sd = math.sqrt(sum((x - mu) ** 2 for x in logs) / (len(logs) - 1))
    half = T95.get(len(logs) - 1, 1.96) * sd / math.sqrt(len(logs))
    return (math.exp(mu), math.exp(mu - half), math.exp(mu + half), len(logs))


def verdict(ratios, bars):
    """`pass`, `KILL counters` or `undecidable counters`. ratios: counter -> (ratio, low, high, rounds) or None; a bar is the largest ratio to the
    first variant that passes. Pass needs the top of the band at or under the bar, KILL the bottom over it; a bar inside the band cannot be
    decided, and neither can a counter other than instructions from fewer than MIN_ROUNDS rounds."""
    kill, unsure = [], []
    for m, bar in bars.items():
        r = ratios.get(m)
        if r is None:
            kill.append(m)  # a counter that was not counted cannot pass
        elif m != "instructions" and r[3] < MIN_ROUNDS:
            unsure.append(m)
        elif r[1] > bar:
            kill.append(m)
        elif r[2] > bar:
            unsure.append(m)
    return "KILL " + ",".join(kill) if kill else "undecidable " + ",".join(unsure) if unsure else "pass"


def parse_perf(text):
    """The counters of one `perf stat -x,` output; a counter perf could not count is None."""
    back = {ev: m for m, ev in EVENTS.items()}
    got = {m: None for m in METRICS}
    for line in text.splitlines():
        f = line.split(",")
        if len(f) > 2 and f[2] in back:
            got[back[f[2]]] = float(f[0]) if f[0].replace(".", "").isdigit() else None
    return got


def subtract(t1, t0):
    return {k: None if t1.get(k) is None or t0.get(k) is None else t1[k] - t0[k] for k in METRICS + ("wall",)}


def geomean(xs):
    xs = [x for x in xs if x and x > 0]
    return math.exp(sum(map(math.log, xs)) / len(xs)) if xs else None


def perf_prefix(pf, inherit):
    return f"perf stat -x, -o {shlex.quote(str(pf))} -e {','.join(EVENTS.values())} " + ("" if inherit else "--no-inherit ") + "-- "


def run_once(v, t, world, stop, core, inherit, timeout, count=True):
    """One pinned run to `stop` (None: the test's own end). Returns the counters, wall, status and console."""
    tq = dict(t, quiet=True, stop=stop if stop is not None else t["stop"])
    pf = world.parent / (world.name + ".perf")
    wrap = perf_prefix(pf, inherit) if count else ""
    if v["tool"] == "yavs":  # perf execs yavs itself: --no-inherit then counts the simulation thread, not a shell
        script = R.yavs_script(v["path"], tq, world, v["tier"], stop=tq["stop"] or None, wrap=wrap)
    else:  # elaboration and run are two processes: always inherited
        script = "ulimit -s unlimited; " + (perf_prefix(pf, True) if count else "") + "bash -c " + shlex.quote(R.ref_script(v["tool"], tq, world))
    B.wait_mem()
    t0 = time.time()
    p = subprocess.run(["timeout", str(timeout), "systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=6G", "taskset", "-c", str(core), "bash", "-c", script],
                       cwd=world, capture_output=True, text=True, errors="replace")
    res = {"wall": time.time() - t0, "rc": p.returncode}
    res.update(parse_perf(pf.read_text()) if count and pf.exists() else {})
    fo, fe = world / R.OUT, world / R.ERR
    res["tail"] = "".join(f.read_bytes()[-300:].decode(errors="replace") for f in (fo, fe) if f.exists())[-300:] + p.stderr[-200:]
    res["console"] = R.console_of(open(fo, errors="replace")) if fo.exists() else ""
    res["all"] = (fo.read_text(errors="replace") if fo.exists() else "") + (fe.read_text(errors="replace") if fe.exists() else "")
    for f in (fo, fe, pf):
        f.unlink(missing_ok=True)
    return res


def measure(variants, t, stops, core, a, scratch):
    """Worlds, warm-ups, then rounds (order swapped every round). stops: (T0, T1), or (None, None) for the whole run.
    Ratios to the first variant are taken per round (`pairs`), never from medians over rounds: host noise is slow, so the two variants of
    one round share it and it cancels, while the median of each variant over a few rounds takes a different slice of it (issue #1055)."""
    full = stops[0] is None
    t0, t1 = stops
    for i, v in enumerate(variants):
        v["world"] = scratch / f"{t['name']}-{i}"
        R.copy_tree(v["pristine"], v["world"])
        if v["tool"] == "yavs":  # the first run leaves a note, the second makes the cache entries; the elaboration cache warms the interpreter too
            for _ in range(2):
                r = run_once(v, t, v["world"], t1, core, a.inherit, a.timeout, count=False)
                check(r, t, full, v, "warm-up")
    samples = [[] for _ in variants]
    consoles = {}
    for rnd in range(a.rounds):
        order = list(enumerate(variants))
        for i, v in (order if rnd % 2 == 0 else order[::-1]):
            top = run_once(v, t, v["world"], t1, core, a.inherit, a.timeout)
            check(top, t, full, v, "run")
            consoles[i] = top["console"]
            if full:
                samples[i].append(top)
            else:
                low = run_once(v, t, v["world"], t0, core, a.inherit, a.timeout)
                check(low, t, full, v, "run to T0")
                d = subtract(top, low)
                d["t0"] = {k: low[k] for k in METRICS}
                samples[i].append(d)
    out = []
    for i, _ in enumerate(variants):
        row = {k: R.med([s.get(k) for s in samples[i]]) for k in METRICS + ("wall",)}
        row["t0"] = samples[i][-1].get("t0")
        row["rounds"] = [{k: x.get(k) for k in METRICS} for x in samples[i]]  # one entry per round, in round order
        row["pairs"] = None if i == 0 else {m: [x[m] / b[m] if x[m] is not None and b[m] else None for x, b in zip(samples[i], samples[0])] for m in METRICS}
        row["console"] = None if i == 0 else console_differs(consoles[0], consoles[i], "yavs" != variants[i]["tool"])
        out.append(row)
    return out


def console_differs(a, b, reference):
    """Against a reference only the set of lines counts: the order of processes at time 0 is open in the LRM (GRLIB's start-up banners)."""
    if reference:
        a, b = ("\n".join(sorted(x.splitlines())) for x in (a, b))
    return R.console_diff(a, b)


def check(r, t, full, v, what):
    # a window stops early: a test with an expected failure status may already carry it from an earlier assertion (Microwatt's debug log), or not yet
    want = [t["exit_code"]] if full and t["exit_code"] else [0, t["exit_code"]]
    if r["rc"] not in want:
        raise Fail(f"{t['name']}, {v['name']}: {what} exited with status {r['rc']}, expected {' or '.join(map(str, sorted(set(want))))}: {r['tail'].strip()[-300:]}")
    if full and t["ok"] and not re.search(t["ok"], r["all"]):
        raise Fail(f"{t['name']}, {v['name']}: {what} lacks /{t['ok']}/ in its output")


def rates(row):
    ins = row["instructions"]
    if not ins or ins <= 0:
        return {}
    return {"ipc": ins / row["cycles"] if row["cycles"] else None, "l1i": 1e3 * row["l1i"] / ins if row["l1i"] is not None else None,
            "itlb": 1e3 * row["itlb"] / ins if row["itlb"] is not None else None, "branch": 1e3 * row["branch"] / ins if row["branch"] is not None else None}


def fmt(x, spec=".3f"):
    return "-" if x is None else format(x, spec)


def band(r):
    """The half-width of a ratio's 95 percent band, for the table; `1 round` when there is no band."""
    return "-" if r is None else "1 round" if r[3] < 2 else f"+-{50 * (r[2] - r[1]) / r[0]:.1f}%"


def table(names, results, bars):
    """The aligned table: one line per test and variant (ratios to the first variant and to sim1), then the geomean of each variant's ratios."""
    head = ["test", "variant", "cycles G", "instr G", "IPC", "L1i/k", "iTLB/k", BRLABEL, "wall s", "cycles", "instr", "L1i", "iTLB", "branch", "cyc band", "cyc/sim1", "ins/sim1", "bar"]
    rows = [head]
    ratios = {n: [] for n in names[1:]}
    to_sim1 = {n: {"cycles": [], "instructions": []} for n in names if n != REF}
    notes = []
    for test, r in results.items():
        if "error" in r:
            rows.append([test, "FAILED"] + [""] * (len(head) - 2))
            notes.append(f"{test}: {r['error']}")
            continue
        base = r["rows"][0]
        ref = r["rows"][names.index(REF)] if REF in names else None
        for n, row in zip(names, r["rows"]):
            rr = rates(row)
            pr = row.get("pairs") or {m: [row[m] / base[m] if row[m] is not None and base[m] else None] for m in METRICS}  # no rounds recorded: one pair
            q = {m: paired(pr[m]) for m in METRICS}
            is_base = n == names[0]
            sr = {m: row[m] / ref[m] if ref and row[m] is not None and ref[m] else None for m in ("cycles", "instructions")} if n != REF else {}
            for m, x in sr.items():
                to_sim1[n][m].append(x)
            if not is_base:
                ratios[n].append(pr)
                if row["console"]:
                    notes.append(f"{test}, {n}: CONSOLE DIFFERS from {names[0]}, {row['console']}. Check this first (wrong-result rule)")
            if row.get("t0") and row["instructions"] and row["instructions"] < row["t0"]["instructions"]:
                notes.append(f"{test}, {n}: the window is smaller than the run to its start, so subtraction noise is large; check that T1 is before the end of the test")
            rows.append([test, n, fmt(row["cycles"] and row["cycles"] / 1e9, ".2f"), fmt(row["instructions"] and row["instructions"] / 1e9, ".2f"), fmt(rr.get("ipc"), ".2f"),
                         fmt(rr.get("l1i"), ".2f"), fmt(rr.get("itlb")), fmt(rr.get("branch"), ".2f"), fmt(row["wall"], ".1f")]
                        + (["base"] + [""] * 5 if is_base else [fmt(q[m] and q[m][0]) for m in METRICS] + [band(q["cycles"])])
                        + [fmt(sr.get("cycles")), fmt(sr.get("instructions")), verdict(q, bars) if bars and not is_base and n != REF else ""])
    for n in names[1:]:
        g = {m: paired([geomean(c) for c in zip(*[q[m] for q in ratios[n]])]) if ratios[n] else None for m in METRICS}  # per round across tests, then the band
        ts = to_sim1.get(n, {})
        rows.append(["geomean", n] + [""] * 7 + [fmt(g[m] and g[m][0]) for m in METRICS] + [band(g["cycles"])] + [fmt(geomean(ts.get(m, []))) for m in ("cycles", "instructions")] + [verdict(g, bars) if bars and n != REF else ""])
    if REF in names and names[0] != REF:
        rows.append(["geomean", names[0]] + [""] * 13 + [fmt(geomean(to_sim1[names[0]]["cycles"])), fmt(geomean(to_sim1[names[0]]["instructions"])), ""])
    widths = [max(len(r[i]) for r in rows) for i in range(len(head))]
    text = "\n".join("  ".join(c.ljust(w) if i < 2 or i == len(head) - 1 else c.rjust(w) for i, (c, w) in enumerate(zip(r, widths))).rstrip() for r in rows)
    if "undecidable" in text:
        notes.append("undecidable: the bar lies inside the 95 percent band of the paired per-round ratios, or a counter other than instructions has fewer than "
                     f"{MIN_ROUNDS} rounds. The band narrows with the square root of the rounds; two copies of one binary (A A) show the band of a difference of nothing")
    return text + ("\n\n" + "\n".join(notes) if notes else "")


def take_core(want):
    """A free timing core under the lock `tools/claim core` uses; held until this process ends."""
    locks = Path(os.environ.get("YAVS_LOCK_DIR", "~/.cache/yavs-agents/locks")).expanduser()
    locks.mkdir(parents=True, exist_ok=True)
    for c in [want] if want is not None else TIMED_CORES:
        fd = os.open(locks / f"core-{c}", os.O_CREAT | os.O_RDWR, 0o644)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            return c, fd
        except OSError:
            os.close(fd)
    raise Fail(f"no free timing core in {[want] if want is not None else list(TIMED_CORES)}: another run holds it (tools/claim core); try again")


def self_test():
    tests = {t["name"] for t in R.load_tests()}
    assert not (bad := set(WINDOWS) - tests), f"window of an unknown test: {bad}"
    v = parse_variant("/x/yavs:auto+A=1")
    assert (v["tool"], v["tier"]) == ("yavs", "auto+A=1") and parse_variant("sim1")["path"] is None and parse_variant("/x/y")["tier"] == "auto"
    p = parse_perf("# started\n\n100,,cycles:u,1,100,,\n50,,instructions:u,1,100,,\n<not counted>,,iTLB-load-misses:u,0,0,,\n")
    assert p["cycles"] == 100 and p["instructions"] == 50 and p["itlb"] is None and p["l1i"] is None
    d = subtract(dict(p, wall=3.0), dict(p, cycles=40, wall=1.0))
    assert d["cycles"] == 60 and d["wall"] == 2.0 and d["itlb"] is None
    one = {"cycles": paired([0.96]), "instructions": paired([0.96])}
    assert verdict(one, parse_bars(["instructions:0.97"])) == "pass" and verdict(one, parse_bars(["instructions:0.95"])) == "KILL instructions"  # instructions decide in one round
    assert verdict(one, parse_bars(["cycles:0.97"])) == "undecidable cycles"  # one round of cycles never passes a bar
    assert verdict({}, {"cycles": 1.0}).startswith("KILL")  # a counter that was not counted cannot pass
    assert verdict({"cycles": paired([0.5] * 4)}, parse_bars(["cycles:0.9"])) == "pass" and verdict({"cycles": paired([1.2, 1.21, 1.19])}, parse_bars(["cycles:0.9"])) == "KILL cycles"
    # same binary twice on GRLIB, six rounds, ratios of the window cycles of the second to the first (tools/realbench/AGENTS.md section 8): the
    # old median ratio read 0.82 on the first three rounds; the paired band holds 1.0 and cannot pass a 3 percent bar
    aa = [196 / 183, 183 / 236, 190 / 232, 225 / 218, 236 / 209, 169 / 160]
    r = paired(aa)
    assert abs(r[0] - 0.97) < 0.01 and r[1] < 1.0 < r[2], r
    assert verdict({"cycles": r}, parse_bars(["cycles:0.97"])) == "undecidable cycles"
    try:
        parse_bars(["cycle:0.9"])
        raise AssertionError("a bad bar passed")
    except Fail as e:
        assert "cycle" in str(e)
    assert abs(geomean([2, 8]) - 4) < 1e-9
    row = dict(cycles=2e9, instructions=4e9, l1i=4e6, itlb=None, branch=8e6, wall=1.0, t0=dict(instructions=1e9), console=None)
    txt = table(["a", "b", REF], {"t": {"rows": [row, dict(row, cycles=1e9, instructions=3e9, console="line 1: x"), dict(row, cycles=4e9)]}}, {"instructions": 0.9})
    assert "0.500" in txt and "0.750" in txt and "pass" in txt and "CONSOLE DIFFERS" in txt and "1 round" in txt, txt
    assert sum("KILL" in l for l in txt.splitlines() if REF + " " in l) == 0, txt  # a bar judges a variant, never the reference
    print("window self-test: ok")


BRLABEL = "br/k"


def main():
    global BRLABEL
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("variants", nargs="*", help="CHECKOUT_OR_BINARY[:TIER], default this checkout's Cranelift build at tier auto; ratios are against the first and against sim1")
    ap.add_argument("--bar", action="append", default=[], help="kill bar, COUNTER:RATIO, largest passing ratio to the first variant; counters " + ", ".join(METRICS))
    ap.add_argument("--tests", default="", help="comma separated; default all of WINDOWS")
    ap.add_argument("--rounds", type=int, default=1, help="rounds per variant; ratios are paired per round, geomean with a 95 percent band. 1 decides instructions, cycles and misses need " + str(MIN_ROUNDS) + " or more and are `undecidable` when the bar is inside the band")
    ap.add_argument("--window", default="", help="T0,T1: replace the window of every selected test (calibration)")
    ap.add_argument("--full", action="store_true", help="the whole run of each test instead of its window (how windows are validated)")
    ap.add_argument("--inherit", action="store_true", help="count all threads of yavs (compile threads too), not the simulation thread")
    ap.add_argument("--no-sim1", action="store_true", help="leave out the sim1 cell (saves its runs; one variant then prints counters only, as perfguard.py uses it)")
    ap.add_argument("--core", type=int, help=f"timing core, default a free one of {TIMED_CORES.start}-{TIMED_CORES.stop - 1}")
    ap.add_argument("--timeout", type=int, default=1500, help="seconds before a run is killed and its test reported failed")
    ap.add_argument("--json", help="also write the results to this file")
    ap.add_argument("--fe", action="store_true", help="count front-end dispatch slots (Zen 4 de_no_dispatch_per_slot.no_ops_from_frontend) in place of branch misses; the `branch` and `br/k` columns then hold slots")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.fe:
        EVENTS["branch"] = FE_EVENT
        BRLABEL = "fe/k"
    if a.self_test:
        return self_test() or 0
    specs = a.variants or [str(R.REPO)]
    if not a.no_sim1 and REF not in specs:
        specs = specs + [REF]
    if a.rounds < 1:
        raise Fail("--rounds must be at least 1")
    bars = parse_bars(a.bar)
    want = a.tests.split(",") if a.tests else list(WINDOWS)
    all_tests = {t["name"]: t for t in R.load_tests()}
    for n in want:
        if n not in all_tests or (n not in WINDOWS and not a.window and not a.full):
            raise Fail(f"{n}: no such window; known: {', '.join(WINDOWS)}")
    tests = [all_tests[n] for n in want]
    variants = [parse_variant(v) for v in specs]
    for v in variants:
        if v["path"] and Path(v["path"]).is_dir():
            v["path"] = build(v["path"])
    names = [v["name"] for v in variants]
    names = [f"{n}#{names[:i].count(n) + 1}" if names.count(n) > 1 else n for i, n in enumerate(names)]  # the same variant twice is the noise check
    for v in variants:
        if v["tool"] == "yavs":
            if not Path(v["path"]).is_file():
                raise Fail(f"binary {v['path']} not found; build it with tools/build-bench.sh")
            v["key"] = B.binary_key(v["path"])
        elif not R.simconf.get(v["tool"], "real_elab"):
            raise Fail(f"{v['tool']}: key real_elab not configured (tools/sims.local.conf, see this directory's AGENTS.md)")
    os.sched_setaffinity(0, SETUP_CPUS & os.sched_getaffinity(0) or os.sched_getaffinity(0))  # setup stays off the timing cores
    begun = time.time()
    R.check_sources(tests)
    scratch = R.CACHE / "scratch" / f"window-{os.getpid()}"
    scratch.mkdir(parents=True, exist_ok=True)
    results = {}
    try:
        ready = []
        for t in tests:  # libraries and worlds first: a test that cannot be set up is named before anything is timed
            try:
                if t["image"]:
                    t["image_dir"] = str(R.P.build(t["image"]))
                pv = [dict(v) for v in variants]
                for i, v in enumerate(pv):
                    for m in t["corpus"]:
                        R.ensure_base(v["tool"], m, v.get("path"), v.get("key"))
                    v["pristine"] = R.make_pristine(t, v["tool"], v.get("path"), v.get("key"), scratch / f"pristine-{t['name']}-{i}")
                ready.append((t, pv))
            except (Fail, R.P.Fail) as e:
                results[t["name"]] = {"error": str(e)}
        core, _lock = take_core(a.core)
        setup = time.time() - begun
        print(f"{len(ready)} tests, setup {setup:.0f} s, core {core}, {'whole runs' if a.full else 'windows'}, {'all threads' if a.inherit else 'simulation thread'}, load {os.getloadavg()[0]:.1f}", flush=True)
        for t, pv in ready:
            stops = (None, None) if a.full else tuple(a.window.split(",")) if a.window else WINDOWS[t["name"]]
            t0 = time.time()
            try:
                results[t["name"]] = {"rows": measure(pv, t, stops, core, a, scratch)}
                print(f"{t['name']}: {time.time() - t0:.0f} s", flush=True)
            except Fail as e:
                results[t["name"]] = {"error": str(e)}
                print(f"{t['name']}: FAILED", flush=True)
            for v in pv:
                shutil.rmtree(v["world"], ignore_errors=True)
                shutil.rmtree(v["pristine"], ignore_errors=True)
    finally:
        shutil.rmtree(scratch, ignore_errors=True)
    print()
    print(table(names, results, bars))
    print(f"\ntotal {time.time() - begun:.0f} s, setup {setup:.0f} s")
    if a.json:
        Path(a.json).write_text(json.dumps({"variants": names, "results": results}, indent=1))
    return 1 if any("error" in r for r in results.values()) else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Fail as e:
        sys.exit(f"window: {e}")
