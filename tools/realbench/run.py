#!/usr/bin/env python3
"""Real-world performance suite for yavs: real testbenches from the external corpora, timed like tools/bench.

  run.py --list [--category C]
  run.py BIN --ref sim1 --cores 6,7 [--tiers interp,auto,auto-cold] [--tests a,b | --category C] [--runs N|auto]
  run.py --compare A B --cores 6,7 [--ref sim1]          (two yavs binaries, interleaved, ratio B/A)
  run.py BIN --ref sim1 --cores 6 --runs 0 --tests a      (probe: one warm-up run per cell, pass/fail and tail)
  run.py --report RESULTS.json                            (markdown from a saved result)
  run.py --self-test

Tests are listed in tests.toml (source, path, top, generics, library setup, category, why). Libraries come
from the analysis-corpus manifests of tools/corpus (built once per tool, cached under --cache), the
test's own extra files are analysed on top, all in a private world directory per timed cell. Cells:
`interp`, `auto`, `llvm` are warm (an untimed run fills the compilation cache of the world, then the timed
runs); `<tier>-cold` copies the pristine world before every timed run (empty compilation cache). The
reference (--ref, label sim1) runs elaboration plus run, analysis excluded. Every timed run is pinned to one
core of --cores (one test at a time per core), under systemd-run MemoryMax=2G and timeout 300, wall, CPU,
peak RSS and (perf) instructions as in tools/bench/run.py --ref, which supplies the timing code.
Python is stdlib only; run with `python3 -I`. See AGENTS.md.
"""
import argparse, concurrent.futures as cf, importlib.util, json, math, os, re, shlex, shutil, statistics, subprocess, sys, threading, time, tomllib
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(REPO / "tools"))
import simconf  # noqa: E402

_spec = importlib.util.spec_from_file_location("bench_run", REPO / "tools/bench/run.py")
B = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(B)

CACHE = Path(os.environ.get("YAVS_REALBENCH_CACHE", "~/.cache/yavs-realbench")).expanduser()
CORPUS = Path(os.environ.get("YAVS_CORPUS", "~/.cache/rsim-corpus")).expanduser()
SPECIAL = {"vhdl-ai-test": "YAVS_BENCH_VHDL_AI_TEST"}  # source id -> environment variable naming its checkout
COLD = "-cold"
RECORDER = (None, None)  # (binary, key) that records the suite drivers for the reference worlds
TIERS_ALL = ("interp", "auto", "llvm")


class Fail(Exception):
    """A named, user-visible failure (a missing checkout, a pin mismatch, a bad test entry)."""


# ---- test list and sources ----

def load_tests(path=HERE / "tests.toml"):
    data = tomllib.loads(Path(path).read_text())
    tests = data["test"]
    seen = set()
    for t in tests:
        t.setdefault("corpus", [])
        t.setdefault("top", "")
        t.setdefault("toplib", "")
        for k in ("name", "category", "why", "source", "path") + (("driver", "pick") if "driver" in t else ("top", "toplib", "corpus")):
            if k not in t:
                raise Fail(f"{path}: test {t.get('name', '?')} lacks `{k}`")
        if t["name"] in seen:
            raise Fail(f"{path}: duplicate test {t['name']}")
        seen.add(t["name"])
        t.setdefault("std", "2008")
        t.setdefault("relaxed", False)
        t.setdefault("generics", [])
        t.setdefault("files", [])
        t.setdefault("stop", "")
        t.setdefault("vunit", False)
        t.setdefault("ok", "")
        t.setdefault("excluded", "")
    return tests


def fetch_pins():
    """source id -> pinned commit, from tools/fetch-corpus.sh (the one place the corpora are pinned)."""
    pins = {}
    for line in (REPO / "tools/fetch-corpus.sh").read_text().splitlines():
        w = line.split()
        if len(w) >= 4 and w[0] in ("fetch", "fetch_with_submodules"):
            pins[w[1]] = w[3]
    return pins


def source_root(sid):
    if sid in SPECIAL:
        if not os.environ.get(SPECIAL[sid]):
            raise Fail(f"source `{sid}`: set ${SPECIAL[sid]} to its checkout")
        return Path(os.environ[SPECIAL[sid]]).expanduser()
    return CORPUS / sid


def check_sources(tests):
    """Every source checkout a selected test uses must exist, at its pinned commit. Returns {id: commit}."""
    pins, used, bad, got = fetch_pins(), {}, [], {}
    for t in tests:
        ids = {t["source"]} | {f["source"] for f in t["files"]}
        for sid in ids:
            used.setdefault(sid, []).append(t["name"])
        for m in t["corpus"]:
            mf = REPO / "tools/corpus" / f"{m}.toml"
            if not mf.exists():
                bad.append(f"{t['name']}: corpus manifest {mf} does not exist")
                continue
            for s in tomllib.loads(mf.read_text()).get("source", []):
                if not (s.get("local") or s.get("env")):
                    used.setdefault(s["id"], []).append(t["name"])
    for sid, names in sorted(used.items()):
        root = source_root(sid)
        if not root.is_dir():
            bad.append(f"source checkout `{sid}` not found at {root} (needed by {', '.join(sorted(set(names))[:4])}; tools/fetch-corpus.sh or YAVS_CORPUS / YAVS_BENCH_VHDL_AI_TEST)")
            continue
        head = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
        got[sid] = head
        if sid in pins and head != pins[sid]:
            bad.append(f"source `{sid}` is at {head[:10]}, pinned {pins[sid][:10]} (tools/fetch-corpus.sh)")
    for t in tests:
        p = source_root(t["source"]) / t["path"]
        if not p.is_file():
            bad.append(f"{t['name']}: {p} does not exist")
    if bad:
        raise Fail("source problems:\n  " + "\n  ".join(bad))
    return got


# ---- libraries, worlds ----

SHIM = """#!/usr/bin/env python3
import json, os, sys
real = os.environ["YAVS_BENCH_REAL"]
if len(sys.argv) > 1 and sys.argv[1] in ("run", "analyze"):
    with open(os.environ["YAVS_BENCH_REC"], "a") as f:
        f.write(json.dumps({"argv": sys.argv[1:], "cwd": os.getcwd()}) + "\\n")
if len(sys.argv) > 1 and sys.argv[1] == "run":
    print(">> Simulation SUCCESS")
    sys.exit(0)
os.execv(real, [real] + sys.argv[1:])
"""


def record_driver(t, Y, key):
    """Run the suite driver of a `driver` test with a recording stand-in for yavs: analysis is real (the libraries are
    kept), a `run` is only written down. Returns (analyse calls, parsed run, library dirs by name)."""
    wd = CACHE / "work" / key / ("rec-" + t["name"])
    rec = wd / "records.jsonl"
    if not rec.exists():
        shutil.rmtree(wd, ignore_errors=True)
        wd.mkdir(parents=True)
        shim = wd / "yavs-shim"
        shim.write_text(SHIM)
        shim.chmod(0o755)
        argv = [sys.executable, "-I", str(REPO / "tools/suites" / t["driver"][0])] + [a.replace("{corpus}", str(CORPUS)) for a in t["driver"][1:]]
        subprocess.run(argv + ["--yavs", str(shim), "--out", str(wd / "out")], cwd=REPO, capture_output=True, text=True,
                       env={**os.environ, "YAVS_BENCH_REAL": Y, "YAVS_BENCH_REC": str(rec)}, timeout=3600)
    if not rec.exists():
        raise Fail(f"{t['name']}: the driver {t['driver'][0]} recorded nothing")
    recs = [json.loads(l) for l in rec.read_text().splitlines()]
    runs = [r for r in recs if r["argv"][0] == "run" and t["pick"] in " ".join(r["argv"])]
    if "nth" in t and len(runs) > t["nth"]:
        runs = [runs[t["nth"]]]  # a testbench the driver runs for several generic sets: the nth
    if len(runs) != 1:
        raise Fail(f"{t['name']}: the driver recorded {len(runs)} runs matching `{t['pick']}`, expected 1 (or give `nth`)")
    libs, gens, flags, top, stop = {}, [], [], None, ""
    for a in runs[0]["argv"][1:]:
        if a.startswith(("--work=", "--map=")):
            n, _, pth = a.split("=", 1)[1].partition(":")
            libs[n] = pth
            if a.startswith("--work="):
                flags.append(n)
        elif a.startswith("--stop-time="):
            stop = a.split("=", 1)[1]
        elif a.startswith("-g"):
            gens.append(a[2:])
        elif a == "--relaxed":
            flags.append(a)
        elif a.startswith("--std="):
            pass
        elif a.startswith("-"):
            raise Fail(f"{t['name']}: recorded run option {a} not handled")
        else:
            top = a
    toplib = next(f for f in flags if not f.startswith("-"))
    return [r for r in recs if r["argv"][0] == "analyze"], dict(top=top, toplib=toplib, relaxed="--relaxed" in flags, generics=gens, stop=stop), libs


def sh(argv, cwd=None, wrap=True, timeout=1800):
    pre = ["timeout", "300", "systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=2G"] if wrap else []
    return subprocess.run(pre + [str(a) for a in argv], cwd=cwd, capture_output=True, text=True, errors="replace", timeout=timeout)


def ensure_base(tool, manifest, Y=None, key=None):
    """The library directories of one corpus manifest built by one tool (cached; builds with tools/corpus/run.py)."""
    d = CACHE / "lib" / (key if tool == "yavs" else tool) / manifest
    log = d.parent / f"{manifest}.log"
    if log.exists() and re.search(r"\[\w+\]: \d+ PASS, \d+ FAIL of \d+ run", log.read_text()[-300:]) and d.is_dir():
        return d
    d.parent.mkdir(parents=True, exist_ok=True)
    argv = [sys.executable, "-I", str(REPO / "tools/corpus/run.py"), manifest, "--out", str(d), "--keep-going", "--expected", "none", "--root", str(CORPUS)]
    argv += ["--cmd", f"{Y} analyze --std={{std}} --work={{lib}}:{{dir}}/{{lib}} {{map}} {{flags}} {{file}}"] if tool == "yavs" else ["--tool", tool]
    p = subprocess.run(argv, capture_output=True, text=True, errors="replace")
    log.write_text(p.stdout + p.stderr)
    if not re.search(r"\[\w+\]: \d+ PASS", p.stdout[-300:]):
        raise Fail(f"building {manifest} with {tool} failed: {(p.stdout + p.stderr)[-300:]}")
    return d


def copy_tree(src, dst):
    subprocess.run(["cp", "-a", "--reflink=auto", str(src), str(dst)], check=True)


def libs_of(w):
    return sorted(p.name for p in Path(w).iterdir() if p.is_dir() and p.name not in ("out",))


def relaxed_flag(tool):
    return ["--relaxed"] if tool == "yavs" else shlex.split(simconf.get(tool, "difftest_relaxed", "--relaxed"))


def analyse_cmd(tool, Y, t, w, lib, file):
    if tool == "yavs":
        return ([Y, "analyze", f"--std={t['std']}", f"--work={lib}:{w}/{lib}"] + [f"--map={l}:{w}/{l}" for l in libs_of(w)]
                + (["--relaxed"] if t["relaxed"] else []) + [file])
    tmpl = simconf.get(tool, "corpus_cmd")
    if not tmpl:
        raise Fail(f"{tool}: key corpus_cmd not configured (tools/sims.local.conf)")
    out = []
    for x in shlex.split(tmpl):
        if x == "{map}":
            continue
        out += (relaxed_flag(tool) if t["relaxed"] else []) if x == "{flags}" else [x.format(std=t["std"], lib=lib, dir=w, file=file)]
    return out


def expand_files(t):
    """The test's extra files as [(lib, path)] in analysis order; glob entries are sorted."""
    out = []
    for f in t["files"]:
        root = source_root(f["source"])
        paths = sorted(p for p in root.glob(f["glob"]) if p.is_file()) if "glob" in f else [root / f["path"]]
        for p in paths:  # lib "@module": the directory after `modules/` (the hdl-modules convention)
            out.append((p.relative_to(root).parts[p.relative_to(root).parts.index("modules") + 1] if f["lib"] == "@module" else f["lib"], p))
    return out


def make_pristine(t, tool, Y, key, dest):
    """A world: the corpus libraries of the test copied together, plus the test's own files analysed on top."""
    dest = Path(dest)
    shutil.rmtree(dest, ignore_errors=True)
    dest.mkdir(parents=True)
    if "driver" in t:
        return _pristine_driver(t, tool, Y, key, dest)
    for m in t["corpus"]:
        base = ensure_base(tool, m, Y, key)
        for lib in libs_of(base):
            if (dest / lib).exists():
                if (dest / lib).is_symlink():
                    continue
                raise Fail(f"{t['name']}: library `{lib}` is in two corpus manifests ({m} and an earlier one)")
            copy_tree(base / lib, dest / lib)
    (dest / "out").mkdir()
    pending = expand_files(t)
    skipped = []
    while pending:  # fixed point: retry what failed until a pass makes no progress (files given in dependency order pass at once)
        rest = [(lib, f) for lib, f in pending
                if (dest / lib).mkdir(exist_ok=True) or sh(analyse_cmd(tool, Y, t, dest, lib, str(f)), cwd=dest).returncode]
        if len(rest) == len(pending):
            skipped = rest
            break
        pending = rest
    if skipped and not t.get("allow_skipped"):
        names = ", ".join(f"{l}:{Path(f).name}" for l, f in skipped[:6])
        p = sh(analyse_cmd(tool, Y, t, dest, skipped[0][0], str(skipped[0][1])), cwd=dest)
        errs = [l for l in (p.stdout + p.stderr).splitlines() if re.search(r"rror", l)] or (p.stdout + p.stderr).strip().splitlines()[-3:]
        raise Fail(f"{t['name']}: {tool} could not analyse {len(skipped)} file(s) ({names}): {' / '.join(errs[:3])[:400]}")
    return dest


def _pristine_driver(t, tool, Y, key, dest):
    """A `driver` test: the libraries the driver built (yavs), or its recorded analysis replayed (reference)."""
    recs, run, libs = record_driver(t, Y or RECORDER[0], key or RECORDER[1])
    run["stop"] = run["stop"] or t["stop"]
    run["generics"] = run["generics"] + [g for g in t["generics"] if g not in run["generics"]]
    t.update(run)
    (dest / "out").mkdir()
    if tool == "yavs":
        for n, pth in libs.items():
            copy_tree(pth, dest / n)
        return dest
    for r in recs:
        std, rel, lib, files = t["std"], False, None, []
        for a in r["argv"][1:]:
            if a.startswith("--std="):
                std = a.split("=", 1)[1]
            elif a.startswith("--work="):
                lib = a.split("=", 1)[1].partition(":")[0]
            elif a == "--relaxed":
                rel = True
            elif a.startswith("--map="):
                pass
            elif a.startswith("-"):
                raise Fail(f"{t['name']}: recorded analyze option {a} not handled")
            else:
                files.append(a if os.path.isabs(a) else str(Path(r["cwd"]) / a))
        (dest / lib).mkdir(exist_ok=True)
        for f in files:
            p = sh(analyse_cmd(tool, Y, dict(t, std=std, relaxed=rel), dest, lib, f), cwd=dest)
            if p.returncode:
                errs = [l for l in (p.stdout + p.stderr).splitlines() if "rror" in l][:2]
                raise Fail(f"{t['name']}: {tool} could not analyse {Path(f).name} (from the recorded driver): {' / '.join(errs)[:300]}")
    return dest


def gen_args(t, w):
    gens = list(t["generics"])
    if t["vunit"]:
        case = t["vunit"] if isinstance(t["vunit"], str) else "_all_"
        tbdir = (source_root(t["source"]) / t["path"]).parent
        gens.append(f"runner_cfg=active python runner : false,enabled_test_cases : {case},seed : 12345,output path : {w}/out/,tb path : {tbdir}/,use_color : false")
    return [f"-g{g}" for g in gens]


def yavs_script(Y, t, w, tier):
    a = [Y, "run", f"--std={t['std']}"] + (["--relaxed"] if t["relaxed"] else []) + [f"--work={t['toplib']}:{w}/{t['toplib']}"]
    a += [f"--map={l}:{w}/{l}" for l in libs_of(w) if l != t["toplib"]]
    a += ([f"--tier={tier}"] if tier != "interp" else ["--tier=interp"]) + ([f"--stop-time={t['stop']}"] if t["stop"] else [])
    a += [t["top"]] + gen_args(t, w)
    return f"cd {shlex.quote(str(w))} && " + " ".join(shlex.quote(x) for x in a)


def ref_script(tool, t, w):
    el, rn = simconf.get(tool, "real_elab"), simconf.get(tool, "real_run")
    if not el or not rn:
        raise Fail(f"{tool}: keys real_elab and real_run not configured (tools/sims.local.conf, see AGENTS.md)")
    rel = " ".join(relaxed_flag(tool)) + " " if t["relaxed"] else ""
    gens = " ".join(shlex.quote(g) for g in gen_args(t, w))
    sub = dict(std=t["std"], lib=t["toplib"], dir=str(w), relaxed=rel, gens=gens, top=t["top"].replace(".", "-", 1),  # a recorded ENTITY.ARCH is ENTITY-ARCH here
               rest=f"--stop-time={t['stop']}" if t["stop"] else "")
    f = lambda s: s.replace("{{", "\0").replace("}}", "\1").format(**sub).replace("\0", "{").replace("\1", "}")
    return f"cd {shlex.quote(str(w))} && {f(el)} || exit 98; {f(rn)}"


# ---- measurement of one test ----

def med(xs):
    xs = [x for x in xs if x is not None]
    return statistics.median(xs) if xs else None


def auto_runs(t):
    return 5 if t < 3 else 3 if t < 40 else 2


class Cell:
    def __init__(self, name, tool, binary=None, tier=None, cold=False):
        self.name, self.tool, self.binary, self.tier, self.cold = name, tool, binary, tier, cold
        self.world = None

    def prepare(self, pristine_of, scratch, t):
        """Untimed: a fresh world for a cold cell, the first one for a warm cell. Returns the script."""
        if self.world is None or self.cold:
            self.world = scratch / self.name.replace("/", "_")
            shutil.rmtree(self.world, ignore_errors=True)
            copy_tree(pristine_of(self), self.world)
        if self.tool == "yavs":
            return yavs_script(self.binary["path"], t, self.world, self.tier)
        return ref_script(self.tool, t, self.world)


def measure_test(t, cells, binaries, refs, cores, runs, instr, scratch):
    """Builds the pristine worlds, then warm-up, interleaved timed rounds and one counted run per cell."""
    try:
        return _measure_test(t, cells, binaries, refs, cores, runs, instr, scratch)
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


def _measure_test(t, cells, binaries, refs, cores, runs, instr, scratch):
    scratch.mkdir(parents=True, exist_ok=True)
    pris = {}
    for b in binaries:
        pris[("yavs", b["label"])] = make_pristine(t, "yavs", b["path"], b["key"], scratch / f"pristine-{b['label']}")
    for r in refs:
        pris[(r, None)] = make_pristine(t, r, None, None, scratch / f"pristine-{r}")
    pristine_of = lambda c: pris[(c.tool, c.binary["label"] if c.binary else None)]
    out = {c.name: [] for c in cells}
    bad = {}
    for c in cells:  # the first run of a cell is untimed and also the pass/fail check
        w = B.timed(c.prepare(pristine_of, scratch, t), str(scratch), cores, ref=c.tool != "yavs")
        text = w["tail"]
        if w["rc"] or (t["ok"] and not re.search(t["ok"], text)):
            bad[c.name] = {"rc": w["rc"], "tail": text}
        out[c.name].append(dict(w, warm=True))
    live = [c for c in cells if c.name not in bad]
    if runs == "auto":
        runs = auto_runs(max([out[c.name][0]["wall"] or 0 for c in live] + [0]) / max(1, 1))
    for rep in range(runs):
        for c in (live if rep % 2 == 0 else live[::-1]):
            out[c.name].append(B.timed(c.prepare(pristine_of, scratch, t), str(scratch), cores, ref=c.tool != "yavs"))
    res = {}
    for c in cells:
        if c.name in bad:
            res[c.name] = {"wall": None, "cpu": None, "rss": None, "ins": None, "ok": 0, "runs": 0, "rcs": [bad[c.name]["rc"]], "tail": bad[c.name]["tail"]}
            continue
        ins = B.timed(c.prepare(pristine_of, scratch, t), str(scratch), cores, instr=instr, ref=c.tool != "yavs")["ins"] if instr and runs else None
        res[c.name] = B.cell(out[c.name] if runs == 0 else out[c.name][1:], ins)
        res[c.name]["warmup_wall"] = out[c.name][0]["wall"]
        if runs == 0:
            res[c.name].update(wall=out[c.name][0]["wall"], cpu=out[c.name][0]["cpu"], rss=out[c.name][0]["rss"])
    return res


def make_cells(binaries, refs, tiers):
    cells = []
    for r in refs:
        cells.append(Cell(r, r))
    for b in binaries:
        for tk in tiers:
            tier = tk.removesuffix(COLD)
            if tier not in TIERS_ALL:
                raise Fail(f"unknown tier `{tk}` (interp, auto, llvm, each also with {COLD})")
            name = tk if len(binaries) == 1 else f"{b['label']}/{tk}"
            cells.append(Cell(name, "yavs", b, tier, tk.endswith(COLD)))
    return cells


# ---- statistics and report ----

def geomean(xs):
    xs = [x for x in xs if x and x > 0]
    return math.exp(sum(map(math.log, xs)) / len(xs)) if xs else None


def ratio(a, b):
    return a / b if a is not None and b else None


def basis(cells_names, refs, compare):
    """Cell name -> the cell it is divided by: the reference (single binary) or binary A (--compare)."""
    out = {}
    for n in cells_names:
        if n in refs:
            continue
        if compare:
            if n.startswith("B/"):
                out[n] = "A/" + n[2:]
        elif refs:
            out[n] = refs[0]
    return out


def summarize(tests, meta):
    """Per cell and metric: geometric mean of the ratio to its basis, overall and per category."""
    names = meta["cells"]
    bas = basis(names, meta["refs"], meta["compare"])
    cats = sorted({r["category"] for r in tests.values()})
    table = {}
    for n, b in bas.items():
        for metric in ("wall", "cpu", "ins"):
            for cat in cats + ["ALL"]:
                xs = []
                for r in tests.values():
                    if cat != "ALL" and r["category"] != cat:
                        continue
                    c, d = r["cells"].get(n), r["cells"].get(b)
                    if c and d:
                        x = ratio(c.get(metric), d.get(metric))
                        if x:
                            xs.append(x)
                table[(n, metric, cat)] = (geomean(xs), len(xs))
    return cats, bas, table


def markdown(res):
    meta, tests = res["meta"], res["tests"]
    cats, bas, table = summarize(tests, meta)
    names = meta["cells"]
    ycells = [n for n in names if n in bas]
    what = "B / A" if meta["compare"] else f"yavs / {meta['refs'][0] if meta['refs'] else '-'}"
    L = [f"Cores {meta['cores']}, runs {meta['runs']}, binaries {', '.join(f'{k} {v[:10]}' for k, v in meta['binaries'].items())}; "
         f"ratio = {what}, below 1 is faster; `n` = tests in the mean.", ""]
    for metric, label in (("wall", "wall time"), ("ins", "instructions"), ("cpu", "CPU time")):
        if not any(table.get((n, metric, "ALL"), (None,))[0] for n in ycells):
            continue
        L += [f"Geometric mean of the {label} ratio, per category:", "", "| category | n | " + " | ".join(ycells) + " |", "|---|---:|" + "---:|" * len(ycells)]
        for cat in ["ALL"] + cats:
            n_ = max((table.get((n, metric, cat), (None, 0))[1] for n in ycells), default=0)
            row = [f"{g:.2f}" if (g := table.get((n, metric, cat), (None, 0))[0]) else "-" for n in ycells]
            L.append(f"| {'**overall**' if cat == 'ALL' else cat} | {n_} | " + " | ".join(row) + " |")
        L.append("")
    L += ["Per test (wall seconds, median; ratio to the basis in brackets; `ins` = instruction ratio):", ""]
    L += ["| test | category | " + " | ".join(names) + " |", "|---|---|" + "---:|" * len(names)]
    for tn, r in tests.items():
        row = []
        for n in names:
            c = r["cells"].get(n)
            if not c or c.get("wall") is None:
                row.append("FAILED" if c else "-")
                continue
            s = f"{c['wall']:.2f}"
            b = bas.get(n)
            if b and r["cells"].get(b) and r["cells"][b].get("wall"):
                s += f" ({c['wall'] / r['cells'][b]['wall']:.2f}"
                x = ratio(c.get("ins"), r["cells"][b].get("ins"))
                s += f", ins {x:.2f})" if x else ")"
            row.append(s)
        L.append(f"| {tn} | {r['category']} | " + " | ".join(row) + " |")
    L.append("")
    L += ["Per test CPU seconds and instructions (G):", "", "| test | " + " | ".join(f"{n} cpu s | {n} G instr" for n in names) + " |", "|---|" + "---:|---:|" * len(names)]
    for tn, r in tests.items():
        cells = [r["cells"].get(n) or {} for n in names]
        L.append(f"| {tn} | " + " | ".join(f"{c['cpu']:.2f} | {c['ins'] / 1e9:.2f}" if c.get("cpu") is not None and c.get("ins") else (f"{c['cpu']:.2f} | -" if c.get("cpu") is not None else "- | -") for c in cells) + " |")
    L.append("")
    fails = [(tn, n, c) for tn, r in tests.items() for n, c in r["cells"].items() if c.get("wall") is None]
    if fails:
        L += ["Cells that failed:", ""] + [f"- {tn} / {n}: rc {c['rcs']}, {c.get('tail', '').strip()[-150:]!r}" for tn, n, c in fails] + [""]
    return "\n".join(L)


# ---- main ----

def self_test():
    assert abs(geomean([1, 4]) - 2) < 1e-9 and geomean([]) is None and ratio(2, 4) == 0.5 and ratio(None, 1) is None
    assert auto_runs(0.2) == 5 and auto_runs(10) == 3 and auto_runs(100) == 2
    tests = load_tests()
    assert len({t["name"] for t in tests}) == len(tests)
    pins = fetch_pins()
    assert "vunit" in pins and len(pins["vunit"]) == 40
    for t in tests:
        assert isinstance(t["generics"], list) and t["category"] and ("driver" in t or (t["top"] and t["corpus"]))
        for m in t["corpus"]:
            assert (REPO / "tools/corpus" / f"{m}.toml").exists(), (t["name"], m)
    global CORPUS
    saved, CORPUS = CORPUS, Path("/nonexistent-corpus")  # a missing checkout must fail by name, not be skipped
    try:
        check_sources(tests[:1])
        raise AssertionError("missing source not reported")
    except Fail as e:
        assert tests[0]["source"] in str(e) and "not found" in str(e), e
    finally:
        CORPUS = saved
    meta = {"cores": "6", "runs": 3, "binaries": {"yavs": "x" * 10}, "refs": ["sim1"], "compare": False, "cells": ["sim1", "interp", "auto"]}
    cell = lambda w, i: {"wall": w, "cpu": w, "ins": i, "rss": 1, "ok": 3, "runs": 3, "rcs": [0] * 3}
    res = {"meta": meta, "tests": {
        "a": {"category": "c1", "cells": {"sim1": cell(1, 10), "interp": cell(2, 40), "auto": cell(3, 20)}},
        "b": {"category": "c2", "cells": {"sim1": cell(4, 10), "interp": cell(2, 10), "auto": {"wall": None, "rcs": [1], "tail": "boom"}}}}}
    cats, bas, table = summarize(res["tests"], meta)
    assert bas == {"interp": "sim1", "auto": "sim1"} and cats == ["c1", "c2"]
    assert abs(table[("interp", "wall", "ALL")][0] - 1.0) < 1e-9 and table[("auto", "wall", "ALL")][1] == 1
    assert abs(table[("interp", "ins", "c1")][0] - 4.0) < 1e-9
    md = markdown(res)
    assert "FAILED" in md and "**overall**" in md and "boom" in md
    meta2 = dict(meta, compare=True, refs=[], cells=["A/interp", "B/interp"])
    assert basis(meta2["cells"], [], True) == {"B/interp": "A/interp"}
    print(f"self-test ok ({len(tests)} tests listed)")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("bin", nargs="?", help="yavs binary")
    ap.add_argument("--compare", nargs=2, metavar=("A", "B"))
    ap.add_argument("--ref", default="", help="reference labels, e.g. sim1")
    ap.add_argument("--tiers", default="interp,auto,auto-cold", help="comma separated: interp, auto, llvm, each also as <tier>-cold")
    ap.add_argument("--tests", default="", help="comma separated test names")
    ap.add_argument("--category", default="", help="only this category")
    ap.add_argument("--runs", default="auto", help="timed runs per cell (default auto: 5, 3 or 2 by test length; 0 = probe)")
    ap.add_argument("--cores", default="", help="cpu list of performance cores, one test at a time per core, e.g. 6,7 (required to time)")
    ap.add_argument("--no-instructions", action="store_true")
    ap.add_argument("--out", default=str(CACHE))
    ap.add_argument("--label", default="")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--include-excluded", action="store_true", help="also run the tests marked `excluded` in tests.toml (known failures on yavs or the reference)")
    ap.add_argument("--prepare", action="store_true", help="build the world of each selected test under <cache>/worlds and print the yavs and reference commands (for perf and debugging)")
    ap.add_argument("--report", metavar="JSON")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    try:
        return run(a)
    except Fail as e:
        print(f"realbench: {e}", file=sys.stderr)
        return 2


def run(a):
    if a.self_test:
        return self_test() or 0
    if a.report:
        print(markdown(json.loads(Path(a.report).read_text())))
        return 0
    tests = load_tests()
    if a.tests:
        want = a.tests.split(",")
        bad = [n for n in want if n not in {t["name"] for t in tests}]
        if bad:
            raise Fail(f"unknown test(s) {bad}; see --list")
        tests = [t for t in tests if t["name"] in want]
    if a.category:
        tests = [t for t in tests if t["category"] == a.category]
    if a.list:
        for t in tests:
            print(f"{t['category']:18} {t['name']:34} {t['source']}/{t['path']}" + (f"   EXCLUDED: {t['excluded']}" if t["excluded"] else ""))
        return 0
    if not a.tests and not a.include_excluded:  # a test named with --tests runs even when excluded
        tests = [t for t in tests if not t["excluded"]]
    bins = a.compare or ([a.bin] if a.bin else None)
    if not bins:
        raise Fail("give a yavs binary, --compare A B, --report, --list or --self-test")
    if not a.cores:
        raise Fail("--cores is required: the performance cores to pin to (e.g. 6,7); never core 0")
    if not B.TIME:
        raise Fail("needs a `time` binary (peak RSS)")
    cores = [c for c in a.cores.split(",") if c]
    refs = [r for r in a.ref.split(",") if r]
    tiers = [x for x in a.tiers.split(",") if x]
    runs = a.runs if a.runs == "auto" else int(a.runs)
    binaries = []
    for lab, p in zip(["A", "B"] if a.compare else ["yavs"], bins):
        p = str(Path(p).expanduser().resolve())
        if not Path(p).is_file():
            raise Fail(f"binary {p} not found")
        binaries.append({"label": lab, "path": p, "key": B.binary_key(p)})
    for r in refs:
        for k in ("corpus_cmd", "real_elab", "real_run"):
            if not simconf.get(r, k):
                raise Fail(f"{r}: key {k} not configured (tools/sims.local.conf, see tools/realbench/AGENTS.md)")
    global RECORDER
    RECORDER = (binaries[0]["path"], binaries[0]["key"])
    pins = check_sources(tests)
    if a.prepare:
        for t in tests:
            for b in binaries:
                w = make_pristine(t, "yavs", b["path"], b["key"], CACHE / "worlds" / f"{t['name']}-{b['label']}")
                print(f"# {t['name']} ({b['label']}): world {w}\n{yavs_script(b['path'], t, w, tiers[0].removesuffix(COLD))}")
            for r in refs:
                w = make_pristine(t, r, None, None, CACHE / "worlds" / f"{t['name']}-{r}")
                print(f"# {t['name']} ({r}): world {w}\n{ref_script(r, t, w)}")
        return 0
    instr = None if a.no_instructions or runs == 0 else B.instr_event(cores[0])
    cells = make_cells(binaries, refs, tiers)
    print(f"{len(tests)} tests, cells {[c.name for c in cells]}, cores {cores}, runs {runs}, load {open('/proc/loadavg').read().split()[:3]}", flush=True)
    for m in sorted({m for t in tests for m in t["corpus"]}):  # libraries first, one build at a time
        for b in binaries:
            ensure_base("yavs", m, b["path"], b["key"])
        for r in refs:
            ensure_base(r, m)
    results, lock = {}, threading.Lock()
    free = list(cores)
    cond = threading.Condition(lock)

    def job(t):
        with cond:
            while not free:
                cond.wait()
            core = free.pop(0)
        try:
            mine = make_cells(binaries, refs, tiers)
            t0 = time.time()
            try:
                cellres = measure_test(t, mine, binaries, refs, core, runs, instr, CACHE / "scratch" / f"{os.getpid()}-{t['name']}")
            except Fail as e:
                cellres = {c.name: {"wall": None, "cpu": None, "rss": None, "ins": None, "ok": 0, "runs": 0, "rcs": [-1], "tail": str(e)} for c in mine}
            with lock:
                results[t["name"]] = {"category": t["category"], "cells": cellres}
                msg = " ".join(f"{n}={c['wall']:.2f}s" if c.get("wall") is not None else f"{n}=FAILED" for n, c in cellres.items())
                print(f"[{core}] {t['name']} ({time.time() - t0:.0f}s): {msg}", flush=True)
        finally:
            with cond:
                free.append(core)
                cond.notify()

    with cf.ThreadPoolExecutor(len(cores)) as ex:
        list(ex.map(job, tests))
    order = [t["name"] for t in tests]
    meta = {"time": time.strftime("%Y-%m-%dT%H:%M:%S"), "cores": a.cores, "runs": runs, "instr_event": instr, "refs": refs, "compare": bool(a.compare),
            "tiers": tiers, "cells": [c.name for c in cells], "binaries": {b["label"]: b["key"] for b in binaries}, "binary_paths": {b["label"]: b["path"] for b in binaries},
            "sources": pins, "host_cpus": os.cpu_count()}
    res = {"meta": meta, "tests": {n: results[n] for n in order}}
    outdir = Path(a.out).expanduser() / "results"
    outdir.mkdir(parents=True, exist_ok=True)
    f = outdir / f"{time.strftime('%Y%m%d-%H%M%S')}{'-' + a.label if a.label else ''}.json"
    f.write_text(json.dumps(res, indent=1))
    f.with_suffix(".md").write_text(markdown(res))
    print(markdown(res))
    print(f"results: {f} and {f.with_suffix('.md')}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
