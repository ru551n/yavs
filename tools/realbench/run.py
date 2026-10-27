#!/usr/bin/env python3
"""Real-world performance suite for yavs: real testbenches from the external corpora, timed like tools/bench.

  run.py --list [--category C]
  run.py BIN --ref sim1 --cores 6,7 [--phases analyze,elab,codegen,run] [--tiers interp,auto,auto-cold] [--tests a,b | --category C] [--runs N|auto]
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
Phases (--phases, default all four), each timed apart for yavs and the reference, with a ratio yavs / reference per test, per category
and overall: `analyze` (the test's whole compile order, consecutive files of one library and standard in ONE CALL by default, for both tools
(#745; --analyze-calls per-file: one call per file); library part and own files apart; cold = fresh libraries, warm =
the same files again into the populated libraries), `elab` (elaboration only, cold and warm cache), `codegen` (yavs `--tier=llvm` minus
`--tier=interp`, both stopped at time 0; cold and warm cache; the reference generates code inside its elaboration), `run` (the --tiers cells).
Python is stdlib only; run with `python3 -I`. See AGENTS.md.
"""
import argparse, concurrent.futures as cf, functools, importlib.util, json, math, os, re, shlex, shutil, statistics, subprocess, sys, tempfile, threading, time, tomllib, types
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
# the shared std/ieee store (#564) is private to this tool, never the user's
os.environ["YAVS_STD_CACHE"] = str(CACHE / "std")
SPECIAL = {"vhdl-ai-test": "YAVS_BENCH_VHDL_AI_TEST"}  # source id -> environment variable naming its checkout
COLD = "-cold"
ANALYZE_CALLS = "batched"  # analysis cells and replays: "batched" (consecutive files of a library and standard per call) or "per-file"
RECORDER = (None, None)  # (binary, key) that records the suite drivers for the reference worlds
TIERS_ALL = ("interp", "auto", "llvm", "clift", "clift-auto", "auto3")


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
        t.setdefault("data", [])
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
        ids = {t["source"]} | {f["source"] for f in t["files"] + t["data"]}
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
    libs, gens, flags, top, stop, severity = {}, [], [], None, "", ""
    for a in runs[0]["argv"][1:]:
        if a.startswith(("--work=", "--map=")):
            n, _, pth = a.split("=", 1)[1].partition(":")
            libs[n] = pth
            if a.startswith("--work="):
                flags.append(n)
        elif a.startswith("--stop-time="):
            stop = a.split("=", 1)[1]
        elif a.startswith("--exit-severity="):
            severity = a.split("=", 1)[1]
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
    return [r for r in recs if r["argv"][0] == "analyze"], dict(top=top, toplib=toplib, relaxed="--relaxed" in flags, generics=gens, stop=stop, severity=severity), libs


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


def analyse_cmd(tool, Y, t, w, lib, file, flags=None, libs=None, std=None):
    """One analysis call. file: one path or a list of paths (one call, in order). flags: the extra arguments (default: --relaxed when the
    test is), libs: the --map libraries (default: the directories of the world)."""
    std = std or t["std"]
    files = [str(f) for f in file] if isinstance(file, (list, tuple)) else [str(file)]
    if flags is None:
        flags = relaxed_flag(tool) if t["relaxed"] else []
    if tool == "yavs":
        return [Y, "analyze", f"--std={std}", f"--work={lib}:{w}/{lib}"] + [f"--map={l}:{w}/{l}" for l in (libs or libs_of(w))] + flags + files
    tmpl = simconf.get(tool, "corpus_cmd")
    if not tmpl:
        raise Fail(f"{tool}: key corpus_cmd not configured (tools/sims.local.conf)")
    out = []
    for x in shlex.split(tmpl):
        if x == "{map}":
            continue
        if x == "{file}":
            out += files
        elif "{file}" in x and len(files) > 1:
            raise Fail(f"{tool}: corpus_cmd needs {{file}} as a word of its own to take several files")
        else:
            out += flags if x == "{flags}" else [x.format(std=std, lib=lib, dir=w, file=files[0])]
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


def copy_corpus_libs(t, tool, Y, key, dest):
    for m in t["corpus"]:
        base = ensure_base(tool, m, Y, key)
        for lib in libs_of(base):
            if (dest / lib).exists():
                if (dest / lib).is_symlink():
                    continue
                raise Fail(f"{t['name']}: library `{lib}` is in two corpus manifests ({m} and an earlier one)")
            copy_tree(base / lib, dest / lib)


def make_pristine(t, tool, Y, key, dest):
    """A world: the corpus libraries of the test copied together, plus the test's own files analysed on top.
    The order in which the own files were analysed (a fixed point, see below) is kept in t["order"][tool]."""
    dest = Path(dest)
    shutil.rmtree(dest, ignore_errors=True)
    dest.mkdir(parents=True)
    if "driver" in t:
        return _pristine_driver(t, tool, Y, key, dest)
    copy_corpus_libs(t, tool, Y, key, dest)
    (dest / "out").mkdir()
    for d in t["data"]:  # files the testbench reads at run time from its working directory (the world)
        shutil.copy(source_root(d["source"]) / d["path"], dest)
    pending = expand_files(t)
    done, skipped = [], []
    while pending:  # fixed point: retry what failed until a pass makes no progress (files given in dependency order pass at once)
        rest = []
        for lib, f in pending:
            (dest / lib).mkdir(exist_ok=True)
            if sh(analyse_cmd(tool, Y, t, dest, lib, str(f)), cwd=dest).returncode:
                rest.append((lib, f))
            else:
                done.append((lib, f))
        if len(rest) == len(pending):
            skipped = rest
            break
        pending = rest
    t.setdefault("order", {})[tool] = done
    if skipped and not t.get("allow_skipped"):
        names = ", ".join(f"{l}:{Path(f).name}" for l, f in skipped[:6])
        p = sh(analyse_cmd(tool, Y, t, dest, skipped[0][0], str(skipped[0][1])), cwd=dest)
        errs = [l for l in (p.stdout + p.stderr).splitlines() if re.search(r"rror", l)] or (p.stdout + p.stderr).strip().splitlines()[-3:]
        raise Fail(f"{t['name']}: {tool} could not analyse {len(skipped)} file(s) ({names}): {' / '.join(errs[:3])[:400]}")
    return dest


def driver_calls(t, Y, key):
    """The analyse calls a `driver` test recorded, one entry per call: (std, relaxed, lib, files); sets the run fields of t."""
    recs, run, libs = record_driver(t, Y or RECORDER[0], key or RECORDER[1])
    run["stop"] = run["stop"] or t["stop"]
    run["generics"] = run["generics"] + [g for g in t["generics"] if g not in run["generics"]]
    t.update(run)
    calls = []
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
        calls.append((std, rel, lib, files))
    return calls, libs


def _pristine_driver(t, tool, Y, key, dest):
    """A `driver` test: the libraries the driver built (yavs), or its recorded analysis replayed (reference)."""
    calls, libs = driver_calls(t, Y, key)
    (dest / "out").mkdir()
    if tool == "yavs":
        for n, pth in libs.items():
            copy_tree(pth, dest / n)
        return dest
    replay_calls(t, tool, Y, calls, dest)
    return dest


def replay_calls(t, tool, Y, calls, dest):
    for std, rel, lib, files in calls:
        (dest / lib).mkdir(exist_ok=True)
        for f in files:  # untimed set-up: one file per call (a recorded call can define a unit twice, which the reference rejects within one call)
            p = sh(analyse_cmd(tool, Y, dict(t, std=std, relaxed=rel), dest, lib, f), cwd=dest)
            if p.returncode:
                errs = [l for l in (p.stdout + p.stderr).splitlines() if "rror" in l][:2]
                raise Fail(f"{t['name']}: {tool} could not analyse {Path(f).name} (from the recorded driver): {' / '.join(errs)[:300]}")


def gen_args(t, w):
    gens = list(t["generics"])
    if t["vunit"]:
        case = t["vunit"] if isinstance(t["vunit"], str) else "_all_"
        tbdir = (source_root(t["source"]) / t["path"]).parent
        gens.append(f"runner_cfg=active python runner : false,enabled_test_cases : {case},seed : 12345,output path : {w}/out/,tb path : {tbdir}/,use_color : false")
    return [f"-g{g}" for g in gens]


def yavs_script(Y, t, w, tier, sub="run", stop=None, extra=()):
    tier, *envs = tier.split("+")  # `tier+VAR=VALUE`: the tier with environment variables set (measurement knobs)
    a = [Y, sub, f"--std={t['std']}"] + (["--relaxed"] if t["relaxed"] else []) + [f"--work={t['toplib']}:{w}/{t['toplib']}"]
    a += [f"--map={l}:{w}/{l}" for l in libs_of(w) if l != t["toplib"]]
    if sub == "run":
        a += [f"--tier={tier}"] + os.environ.get("YAVS_BENCH_TIERFLAGS", "").split()
        stop = stop or t["stop"]
        a += [f"--stop-time={stop}"] if stop else []
        a += [f"--exit-severity={t['severity']}"] if t.get("severity") else []
    a += list(extra) + [t["top"]] + gen_args(t, w)
    pre = "env " + " ".join(shlex.quote(e) for e in envs) + " " if envs else ""
    return f"cd {shlex.quote(str(w))} && " + pre + " ".join(shlex.quote(x) for x in a)


def ref_script(tool, t, w, what="both", extra=""):
    """Elaboration and run of the reference (what="elab": elaboration only; extra: more run options, tools/waveq)."""
    el, rn = simconf.get(tool, "real_elab"), simconf.get(tool, "real_run")
    if not el or not rn:
        raise Fail(f"{tool}: keys real_elab and real_run not configured (tools/sims.local.conf, see AGENTS.md)")
    rel = " ".join(relaxed_flag(tool)) + " " if t["relaxed"] else ""
    gens = " ".join(shlex.quote(g) for g in gen_args(t, w))
    sub = dict(std=t["std"], lib=t["toplib"], dir=str(w), relaxed=rel, gens=gens, top=t["top"].replace(".", "-", 1),  # a recorded ENTITY.ARCH is ENTITY-ARCH here
               rest=" ".join(x for x in (f"--stop-time={t['stop']}" if t["stop"] else "", extra) if x))
    f = lambda s: s.replace("{{", "\0").replace("}}", "\1").format(**sub).replace("\0", "{").replace("\1", "}")
    return f"cd {shlex.quote(str(w))} && {f(el)}" + (f" || exit 98; {f(rn)}" if what == "both" else "")


# ---- the analysis phase: the compile order of a test, consecutive files of a library and standard per call (#745) ----

def pass_keys(tool, manifest, key):
    """`lib source/path` of the manifest files that analysed (PASS) in the cached library build of this tool."""
    log = CACHE / "lib" / (key if tool == "yavs" else tool) / f"{manifest}.log"
    return {l[5:].strip() for l in log.read_text(errors="replace").splitlines() if l.startswith("PASS ")}


def analysis_plan(t, tools):
    """tools: [(tool, binary path, key)]. Per tool {"lib": [item], "own": [item], "libs": [library names]}, item = dict(lib, std, file, flags);
    flags = None (the test's own: --relaxed if it is) or the manifest's flags by tool. The library part is the corpus manifests' files that
    passed on every tool (the others cost different work); the own part is the test's files in the order its world needed; for a `driver`
    test the calls into the testbench's own library are the own part and everything it recorded before is the library part."""
    plan = {}
    if "driver" in t:
        calls, _ = driver_calls(t, tools[0][1], tools[0][2])
        for tool, Y, key in tools:
            items = [dict(lib=lib, std=std, file=f, flags=["--relaxed"] if rel else [], own=lib == t["toplib"]) for std, rel, lib, files in calls for f in files]
            plan[tool] = dict(lib=[i for i in items if not i["own"]], own=[i for i in items if i["own"]])
    else:
        ok = {}
        for tool, Y, key in tools:
            for m in t["corpus"]:
                ks = pass_keys(tool, m, key)
                ok[m] = ks if m not in ok else ok[m] & ks
        libitems = []
        for m in t["corpus"]:
            mf = tomllib.loads((REPO / "tools/corpus" / f"{m}.toml").read_text())
            libitems += [dict(lib=f["lib"], std=f["std"], file=source_root(f["source"]) / f["path"], flags=mf.get("flags", {}), key=f"{f['lib']} {f['source']}/{f['path']}", m=m)
                         for f in mf["file"]]
        libitems = [i for i in libitems if i["key"] in ok[i["m"]]]
        for tool, Y, key in tools:
            plan[tool] = dict(lib=libitems, own=[dict(lib=l, std=t["std"], file=f, flags=None) for l, f in t["order"][tool]])
    for tool in plan:
        plan[tool]["libs"] = sorted({i["lib"] for p in ("lib", "own") for i in plan[tool][p]})
    return plan


def item_flags(tool, t, item):
    fl = item["flags"]
    if fl is None:
        return relaxed_flag(tool) if t["relaxed"] else []
    return fl.get(tool, []) if isinstance(fl, dict) else fl  # a manifest's [flags]: by tool key (the keys are the labels)


def item_cmd(tool, Y, t, w, items, libs):
    """The analysis call of items (a list of plan items of one library, standard and flags, or one item)."""
    items = items if isinstance(items, list) else [items]
    return analyse_cmd(tool, Y, t, w, items[0]["lib"], [i["file"] for i in items], flags=item_flags(tool, t, items[0]), libs=libs, std=items[0]["std"])


UNIT = re.compile(r"^\s*(?:(entity)\s+(\w+)\s+is|(architecture)\s+(\w+)\s+of\s+(\w+)\s+is|(package\s+body)\s+(\w+)\s+is|(package)\s+(\w+)\s+is"
                  r"|(configuration)\s+(\w+)\s+of|(context)\s+(\w+)\s+is)", re.I | re.M)


@functools.lru_cache(maxsize=None)
def units_of(path):
    """The design units a source file defines, as (kind, names...) keys (a scan, not a parse: a false hit only ends a call early)."""
    text = Path(path).read_text(errors="replace")
    return frozenset(tuple(" ".join(g.lower().split()) for g in m.groups() if g) for m in UNIT.finditer(text))


def item_calls(tool, t, items):
    """The items as calls (ANALYZE_CALLS "batched"): consecutive items of one library, standard and flags together; a file that defines a unit
    an earlier file of the call defined starts a new call (the corpus manifests put several projects into one library, and the reference
    rejects a unit replaced within one call). Otherwise one call per item."""
    if ANALYZE_CALLS != "batched":
        return [[i] for i in items]
    calls, seen = [], set()
    for i in items:
        u = units_of(str(i["file"]))
        if calls and (calls[-1][0]["lib"], calls[-1][0]["std"], item_flags(tool, t, calls[-1][0])) == (i["lib"], i["std"], item_flags(tool, t, i)) and not seen & u:
            calls[-1].append(i)
            seen |= u
        else:
            calls.append([i])
            seen = set(u)
    return calls


def analysis_script(tool, Y, t, w, plan, part, path):
    """A bash script of one analysis call per run of files (per file with --analyze-calls per-file); the exit status is 1 if any call failed.
    Returns the command that runs it."""
    L = [f"cd {shlex.quote(str(w))}", "f=0"]
    L += [" ".join(map(shlex.quote, item_cmd(tool, Y, t, w, c, plan["libs"]))) + " >/dev/null 2>&1 || f=$((f+1))" for c in item_calls(tool, t, plan[part])]
    L.append("exit $((f > 0))")
    Path(path).write_text("\n".join(L) + "\n")
    return f"bash {shlex.quote(str(path))}"


def make_libs_world(t, tool, Y, key, plan, dest):
    """The world before the test's own files: the corpus libraries (a `driver` test: its recorded library part replayed, untimed)."""
    shutil.rmtree(dest, ignore_errors=True)
    dest.mkdir(parents=True)
    (dest / "out").mkdir()
    if "driver" in t:
        for c in item_calls(tool, t, plan[tool]["lib"]):
            (dest / c[0]["lib"]).mkdir(exist_ok=True)
            p = sh(item_cmd(tool, Y, t, dest, c, plan[tool]["libs"]), cwd=dest)
            if p.returncode:
                raise Fail(f"{t['name']}: {tool} could not analyse {Path(c[0]['file']).name} or a later file of its call (library part of the recorded driver)")
    else:
        copy_corpus_libs(t, tool, Y, key, dest)
    return dest


# ---- measurement of one test ----

def med(xs):
    xs = [x for x in xs if x is not None]
    return statistics.median(xs) if xs else None


def auto_runs(t):
    return 5 if t < 3 else 3 if t < 40 else 2


PHASES = ("analyze", "elab", "codegen", "run")
COUNT_RUNS = False  # --count-runs: every timed run counts instructions and cycles (perf stat)
SHARED = {}  # library-part analysis cells shared by the tests of one corpus selection: key -> measured cell (measured once)


class Cell:
    """One timed command of a test. mode: run (elaboration and run), elab (elaboration only), stop0 (run to time 0: elaboration, package
    initialisation and, with the llvm tier, code generation), analyze (the compile order, one call per file; part lib or own)."""

    def __init__(self, name, tool, binary=None, tier=None, cold=False, phase="run", mode="run", part=None, shared=None):
        self.name, self.tool, self.binary, self.tier, self.cold = name, tool, binary, tier, cold
        self.phase, self.mode, self.part, self.shared = phase, mode, part, shared
        self.world, self.files = None, None
        # a warm yavs cell needs two untimed runs: the first leaves a note, the second makes the cache entries (analysis cells: the first run is the cold one)
        self.warmups = 2 if tool == "yavs" and not cold and mode != "analyze" else 1

    def label(self):
        return self.binary["label"] if self.binary else None

    def prepare(self, ctx, scratch, t):
        """Untimed: a fresh world for a cold cell, the first one for a warm cell. Returns the script."""
        if self.world is None or self.cold:
            self.world = scratch / self.name.replace("/", "_").replace(":", "_")
            shutil.rmtree(self.world, ignore_errors=True)
            if self.mode == "analyze" and self.part == "lib":
                self.world.mkdir(parents=True)
            else:
                copy_tree(ctx["libs" if self.mode == "analyze" else "pristine"][(self.tool, self.label())], self.world)
        Y = self.binary["path"] if self.binary else None
        if self.mode == "analyze":
            plan = ctx["plan"][self.tool]
            self.files = len(plan[self.part])
            return analysis_script(self.tool, Y, t, self.world, plan, self.part, self.world.parent / (self.world.name + ".sh"))
        if self.tool == "yavs":
            return yavs_script(Y, t, self.world, self.tier, "elab" if self.mode == "elab" else "run", "0fs" if self.mode == "stop0" else None)
        return ref_script(self.tool, t, self.world, "elab" if self.mode == "elab" else "both")


def measure_test(t, cells, derived, binaries, refs, cores, runs, instr, scratch):
    """Builds the pristine worlds, then per phase: warm-up, interleaved timed rounds and one counted run per cell."""
    try:
        return _measure_test(t, cells, derived, binaries, refs, cores, runs, instr, scratch)
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


def _measure_test(t, cells, derived, binaries, refs, cores, runs, instr, scratch):
    scratch.mkdir(parents=True, exist_ok=True)
    ctx = {"pristine": {}, "libs": {}}
    for b in binaries:
        ctx["pristine"][("yavs", b["label"])] = make_pristine(t, "yavs", b["path"], b["key"], scratch / f"pristine-{b['label']}")
    for r in refs:
        ctx["pristine"][(r, None)] = make_pristine(t, r, None, None, scratch / f"pristine-{r}")
    if any(c.mode == "analyze" for c in cells):
        tools = [("yavs", b["path"], b["key"]) for b in binaries] + [(r, None, None) for r in refs]
        ctx["plan"] = analysis_plan(t, tools)  # one plan per tool; the binaries of --compare get the same file lists
        for tool, Y, key in tools:
            lab = [b["label"] for b in binaries if b["path"] == Y][0] if Y else None
            if any(c.mode == "analyze" and c.part == "own" and (c.tool, c.label()) == (tool, lab) for c in cells):
                ctx["libs"][(tool, lab)] = make_libs_world(t, tool, Y, key, ctx["plan"], scratch / f"libs-{tool}-{lab}")
    res = {}
    for phase in PHASES:
        group = [c for c in cells if c.phase == phase]
        todo = []
        for c in group:
            if c.shared and (corpus_key(t), c.name) in SHARED:
                res[c.name] = SHARED[(corpus_key(t), c.name)]
            else:
                todo.append(c)
        if todo:
            res.update(measure_group(t, todo, ctx, cores, runs, instr, scratch))
        for c in todo:
            if c.shared:
                SHARED[(corpus_key(t), c.name)] = res[c.name]
    for name, kind, parts in derived:
        res[name] = derive(kind, [res[p] for p in parts])
    return res


def corpus_key(t):
    return tuple(t["corpus"]) or t["name"]


def ok_run(t, c, w):
    return not w["rc"] and (c.mode != "run" or not t["ok"] or w["ok_match"])


def measure_group(t, cells, ctx, cores, runs, instr, scratch):
    """The cells of one phase, as the run phase always was: an untimed check run, `runs` timed rounds with the cells interleaved
    (order swapped every round), one counted run per cell."""
    out = {c.name: [] for c in cells}
    bad = {}
    for c in cells:
        for _ in range(c.warmups):
            w = B.timed(c.prepare(ctx, scratch, t), str(scratch), cores, ref=c.tool != "yavs", ok=t["ok"] or None)
            if not ok_run(t, c, w):
                bad[c.name] = {"rc": w["rc"], "tail": w["tail"]}
                break
        out[c.name].append(dict(w, warm=True))
    live = [c for c in cells if c.name not in bad]
    if runs == "auto":
        runs = auto_runs(max([out[c.name][0]["wall"] or 0 for c in live] + [0]))
    for rep in range(runs):
        for c in (live if rep % 2 == 0 else live[::-1]):
            out[c.name].append(B.timed(c.prepare(ctx, scratch, t), str(scratch), cores, instr=COUNT_RUNS and instr, ref=c.tool != "yavs"))
    res = {}
    for c in cells:
        if c.name in bad:
            res[c.name] = {"wall": None, "cpu": None, "rss": None, "ins": None, "ok": 0, "runs": 0, "rcs": [bad[c.name]["rc"]], "tail": bad[c.name]["tail"]}
            continue
        ins = None
        if instr and runs:  # the counted run must pass too: a run that stops early counts fewer instructions
            w = B.timed(c.prepare(ctx, scratch, t), str(scratch), cores, instr=instr, ref=c.tool != "yavs", ok=t["ok"] or None)
            if not ok_run(t, c, w):
                res[c.name] = {"wall": None, "cpu": None, "rss": None, "ins": None, "ok": 0, "runs": 0, "rcs": [w["rc"]], "tail": w["tail"]}
                continue
            ins = w["ins"]
        res[c.name] = B.cell(out[c.name] if runs == 0 else out[c.name][1:], ins)
        res[c.name]["warmup_wall"] = out[c.name][0]["wall"]
        if runs == 0:
            res[c.name].update(wall=out[c.name][0]["wall"], cpu=out[c.name][0]["cpu"], rss=out[c.name][0]["rss"])
        if c.mode == "analyze":
            res[c.name]["files"] = c.files
    return res


def derive(kind, parts):
    """sum: the analysis of the library part plus the own files. diff: llvm minus interp up to time 0 (code generation, floored at 0)."""
    if any(p.get("wall") is None for p in parts):
        return {"wall": None, "cpu": None, "rss": None, "ins": None, "ok": 0, "runs": 0, "rcs": [r for p in parts for r in p["rcs"]], "tail": "; ".join(p.get("tail", "") for p in parts if p.get("tail"))}
    f = (lambda xs: None if None in xs else sum(xs)) if kind == "sum" else (lambda xs: None if None in xs else max(xs[0] - xs[1], 0.0))
    r = {k: f([p[k] for p in parts]) for k in ("wall", "cpu", "ins")}
    r.update(rss=max(p["rss"] for p in parts), ok=min(p["ok"] for p in parts), runs=min(p["runs"] for p in parts), rcs=[0])
    if kind == "sum":
        r["files"] = sum(p["files"] for p in parts)
    return r


def make_cells(binaries, refs, tiers, phases, compare=False):
    """(cells to measure, derived cells [(name, sum|diff, parts)], spec). spec: phases {phase: visible cell names}, basis {cell: cell it is
    divided by}, hidden {raw cells that only feed a derived one}."""
    cells, derived, ph, basis, hidden = [], [], {p: [] for p in phases}, {}, []
    ref0 = refs[0] if refs else None
    rname = lambda r: lambda base: r if base == "run" else f"{r}:{base}"
    yname = lambda b: lambda base: base if len(binaries) == 1 else f"{b['label']}/{base}"
    who = [(r, None, rname(r)) for r in refs] + [("yavs", b, yname(b)) for b in binaries]
    for tool, b, nm in who:
        yavs = b is not None
        for p in phases:
            if p == "run":
                for tk in tiers if yavs else ["run"]:
                    if yavs and tk.removesuffix(COLD).split("+")[0] not in TIERS_ALL:
                        raise Fail(f"unknown tier `{tk}` (interp, auto, llvm, clift, clift-auto, auto3, each also with {COLD})")
                    cells.append(Cell(nm(tk), tool, b, tk.removesuffix(COLD) if yavs else None, tk.endswith(COLD)))
                    ph[p].append(nm(tk))
                    if yavs and ref0:
                        basis[nm(tk)] = ref0
            for temp in ("cold", "warm"):
                cold = temp == "cold"
                if p == "elab":
                    cells.append(Cell(nm(f"elab-{temp}"), tool, b, "interp", cold, p, "elab"))
                    ph[p].append(nm(f"elab-{temp}"))
                    if yavs and ref0:
                        basis[nm(f"elab-{temp}")] = f"{ref0}:elab-{temp}"
                elif p == "codegen" and yavs:
                    raw = [nm(f"{tier}0-{temp}") for tier in ("llvm", "interp")]
                    cells += [Cell(n, tool, b, tier, cold, p, "stop0") for n, tier in zip(raw, ("llvm", "interp"))]
                    hidden += raw
                    derived.append((nm(f"codegen-{temp}"), "diff", raw))
                    ph[p].append(nm(f"codegen-{temp}"))
                    if ref0:
                        basis[nm(f"codegen-{temp}")] = f"{ref0}:elab-{temp}"  # the reference generates its code inside the elaboration
                elif p == "analyze":
                    parts = [nm(f"analyze-{temp}-{part}") for part in ("lib", "own")]
                    cells += [Cell(n, tool, b, None, cold, p, "analyze", part, shared=part == "lib") for n, part in zip(parts, ("lib", "own"))]
                    derived.append((nm(f"analyze-{temp}"), "sum", parts))
                    ph[p] += [nm(f"analyze-{temp}")] + parts
                    for n in [nm(f"analyze-{temp}")] + parts:
                        if yavs and ref0:
                            basis[n] = f"{ref0}:" + n.split("/")[-1]
    if compare:  # two binaries: B is divided by A, cell by cell
        basis = {n: "A/" + n[2:] for names in ph.values() for n in names if n.startswith("B/")}
    return cells, derived, dict(phases=ph, basis=basis, hidden=hidden)


# ---- statistics and report ----

def geomean(xs):
    xs = [x for x in xs if x and x > 0]
    return math.exp(sum(map(math.log, xs)) / len(xs)) if xs else None


def ratio(a, b):
    return a / b if a is not None and b else None


def basis(cells_names, refs, compare, explicit=None):
    """Cell name -> the cell it is divided by: the explicit table of a result with phases, else the reference (single binary) or binary A (--compare)."""
    if explicit is not None:
        return {n: b for n, b in explicit.items() if n in cells_names and b in cells_names}
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


def floor(v, metric):
    """`time` reports wall and CPU in steps of 10 ms: a zero is below the resolution, not zero (use the instruction ratio there)."""
    return max(v, 0.005) if v is not None and metric in ("wall", "cpu") else v


def phases_of(meta):
    """{phase: cell names}; a result from before the phases existed has the run phase only."""
    return meta.get("phases") or {"run": meta["cells"]}


def summarize(tests, meta, names=None):
    """Per cell and metric: geometric mean of the ratio to its basis, overall and per category."""
    names = names or meta["cells"]
    bas = {n: b for n, b in basis(meta["cells"], meta["refs"], meta["compare"], meta.get("basis")).items() if n in names}  # the basis may be in another phase
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
                        x = ratio(floor(c.get(metric), metric), floor(d.get(metric), metric))
                        if x:
                            xs.append(x)
                table[(n, metric, cat)] = (geomean(xs), len(xs))
    return cats, bas, table


DETAIL = re.compile(r"-(lib|own)$")  # the library part and the own files of an analysis: shown in their own table


def phase_md(res, phase, names, many):
    """The tables of one phase: geometric means per category (wall, instructions, CPU), run-length view, per test wall and CPU."""
    meta, tests = res["meta"], res["tests"]
    cats, bas, table = summarize(tests, meta, names)
    ycells = [n for n in names if n in bas]
    shown = [n for n in names if not DETAIL.search(n)]
    L = [f"## Phase: {phase}", ""] if many else []
    for metric, label in (("wall", "wall time"), ("ins", "instructions"), ("cpu", "CPU time")):
        if not any(table.get((n, metric, "ALL"), (None,))[0] for n in ycells):
            continue
        L += [f"Geometric mean of the {label} ratio, per category:", "", "| category | n | " + " | ".join(ycells) + " |", "|---|---:|" + "---:|" * len(ycells)]
        for cat in ["ALL"] + cats:
            n_ = max((table.get((n, metric, cat), (None, 0))[1] for n in ycells), default=0)
            row = [f"{g:.2f}" if (g := table.get((n, metric, cat), (None, 0))[0]) else "-" for n in ycells]
            L.append(f"| {'**overall**' if cat == 'ALL' else cat} | {n_} | " + " | ".join(row) + " |")
        L.append("")
    tot = []  # run-length view: the geometric means weigh a 0.1 s test like a 60 s one
    for n in ycells:
        b = bas[n]
        both = [r["cells"] for r in tests.values() if (r["cells"].get(n) or {}).get("wall") is not None and (r["cells"].get(b) or {}).get("wall")]
        long_ = [c for c in both if c[b]["wall"] >= 1.0]
        tot.append((n, ratio(sum(c[n]["wall"] for c in both), sum(c[b]["wall"] for c in both)), len(both),
                    geomean([ratio(floor(c[n]["wall"], "wall"), floor(c[b]["wall"], "wall")) for c in long_]), len(long_)))
    L += ["Run-length view (wall): ratio of the summed times, and geometric mean over the tests whose basis cell takes 1 s or more:", "",
          "| cell | sum ratio | n | geomean, basis >= 1 s | n |", "|---|---:|---:|---:|---:|"]
    L += [f"| {n} | {a:.2f} | {k} | {g:.2f} | {m} |" if a and g else f"| {n} | {a:.2f} | {k} | - | {m} |" if a else f"| {n} | - | {k} | - | {m} |" for n, a, k, g, m in tot] + [""]
    L += ["Per test (wall seconds, median; ratio to the basis in brackets; `ins` = instruction ratio):", ""]
    L += ["| test | category | " + " | ".join(shown) + " |", "|---|---|" + "---:|" * len(shown)]
    for tn, r in tests.items():
        row = []
        for n in shown:
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
    if phase == "analyze":
        L += analysis_md(tests, names, bas)
    L += ["Per test CPU seconds and instructions (G):", "", "| test | " + " | ".join(f"{n} cpu s | {n} G instr" for n in shown) + " |", "|---|" + "---:|---:|" * len(shown)]
    for tn, r in tests.items():
        cells = [r["cells"].get(n) or {} for n in shown]
        L.append(f"| {tn} | " + " | ".join(f"{c['cpu']:.2f} | {c['ins'] / 1e9:.2f}" if c.get("cpu") is not None and c.get("ins") else (f"{c['cpu']:.2f} | -" if c.get("cpu") is not None else "- | -") for c in cells) + " |")
    L.append("")
    return L


def analysis_md(tests, names, bas):
    """Analysis, per file: the first binary's cells against their basis. Files per part, mean wall per file and the ratio of each part."""
    ys = [n for n in names if n in bas and n.endswith("analyze-cold")]
    if not ys:
        return []
    pre = ys[0][:-len("analyze-cold")]
    refp = bas[ys[0]][:-len("analyze-cold")]
    L = [f"Analysis per file ({pre or 'yavs'} against {refp.rstrip(':/') or '-'}; one analysis call per file; ms = mean wall per file; ratio of the library part / of the test's own files):", "",
         "| test | lib files | own files | cold ms/file | cold ms/file basis | cold lib | cold own | warm ms/file | warm ms/file basis | warm lib | warm own |", "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|"]
    for tn, r in tests.items():
        g = lambda p, temp, part="": (r["cells"].get(f"{p}analyze-{temp}{part}") or {})
        ms = lambda c: f"{c['wall'] / c['files'] * 1000:.1f}" if c.get("wall") is not None and c.get("files") else "-"
        rt = lambda temp, part: (lambda x: f"{x:.2f}" if x else "-")(ratio(floor(g(pre, temp, part).get("wall"), "wall"), floor(g(refp, temp, part).get("wall"), "wall")))
        L.append(f"| {tn} | {g(pre, 'cold', '-lib').get('files', '-')} | {g(pre, 'cold', '-own').get('files', '-')} | "
                 f"{ms(g(pre, 'cold'))} | {ms(g(refp, 'cold'))} | {rt('cold', '-lib')} | {rt('cold', '-own')} | "
                 f"{ms(g(pre, 'warm'))} | {ms(g(refp, 'warm'))} | {rt('warm', '-lib')} | {rt('warm', '-own')} |")
    return L + [""]


def markdown(res):
    meta, tests = res["meta"], res["tests"]
    phases = phases_of(meta)
    allnames = [n for names in phases.values() for n in names]
    bas = basis(allnames, meta["refs"], meta["compare"], meta.get("basis"))
    what = "B / A" if meta["compare"] else f"yavs / {meta['refs'][0] if meta['refs'] else '-'}"
    L = [f"Cores {meta['cores']}, runs {meta['runs']}, binaries {', '.join(f'{k} {v[:10]}' for k, v in meta['binaries'].items())}; "
         f"ratio = {what}, below 1 is faster; `n` = tests in the mean.", ""]
    if len(phases) > 1:  # the headline: wall time ratio of every cell, overall and per category
        cats, _, table = summarize(tests, meta, allnames)
        ycells = [n for n in allnames if n in bas]
        L += ["Geometric mean of the wall time ratio per phase and category (rows: cell; `n` = tests in the overall mean):", "",
              "| cell | n | overall | " + " | ".join(cats) + " |", "|---|---:|---:|" + "---:|" * len(cats)]
        for n in ycells:
            g = lambda cat: (f"{v:.2f}" if (v := table.get((n, "wall", cat), (None, 0))[0]) else "-")
            L.append(f"| {n} | {table[(n, 'wall', 'ALL')][1]} | **{g('ALL')}** | " + " | ".join(g(c) for c in cats) + " |")
        L.append("")
    for ph, names in phases.items():
        L += phase_md(res, ph, names, len(phases) > 1)
    if meta["compare"]:  # slowdowns of B: instructions up by more than 5 percent, or wall up by more than 10 percent and 0.05 s
        worse = []
        for tn, r in tests.items():
            for n, b in bas.items():
                c, d = r["cells"].get(n) or {}, r["cells"].get(b) or {}
                wi, ii = ratio(c.get("wall"), d.get("wall")), ratio(c.get("ins"), d.get("ins"))
                if (ii and ii > 1.05) or (wi and wi > 1.10 and c["wall"] - d["wall"] > 0.05):
                    worse.append(f"- {tn} / {n[2:]}: wall " + (f"x{wi:.2f}" if wi else "-") + (f", instructions x{ii:.2f}" if ii else ""))
        L += ["Slowdowns of B (instructions above 1.05, or wall above 1.10 and 0.05 s): " + ("none" if not worse else ""), ""] + worse + ([""] if worse else [])
    hidden = set(meta.get("hidden", []))
    fails = [(tn, n, c) for tn, r in tests.items() for n, c in r["cells"].items() if c.get("wall") is None and n not in hidden]
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
    assert "FAILED" in md and "**overall**" in md and "boom" in md and "## Phase" not in md  # a result from before the phases: one table set
    meta2 = dict(meta, compare=True, refs=[], cells=["A/interp", "B/interp"])
    assert basis(meta2["cells"], [], True) == {"B/interp": "A/interp"}
    cmp_res = {"meta": meta2, "tests": {"a": {"category": "c1", "cells": {"A/interp": cell(1, 10), "B/interp": cell(1.5, 12)}}}}
    assert "a / interp: wall x1.50, instructions x1.20" in markdown(cmp_res)
    # phases: the cells that are made, their basis, the derived cells and the report
    bins = [{"label": "yavs", "path": "/x", "key": "k"}]
    cells, derived, spec = make_cells(bins, ["sim1"], ["interp", "auto-cold"], list(PHASES))
    names = {c.name: c for c in cells}
    assert spec["phases"]["run"] == ["sim1", "interp", "auto-cold"] and spec["basis"]["interp"] == "sim1"
    assert spec["basis"]["analyze-cold-lib"] == "sim1:analyze-cold-lib" and spec["basis"]["elab-warm"] == "sim1:elab-warm"
    assert spec["basis"]["codegen-cold"] == "sim1:elab-cold" and set(spec["hidden"]) == {"llvm0-cold", "interp0-cold", "llvm0-warm", "interp0-warm"}
    assert names["sim1:analyze-cold-lib"].shared and not names["analyze-cold-own"].shared and names["elab-warm"].warmups == 2 and names["elab-cold"].warmups == 1
    assert names["analyze-warm-own"].warmups == 1 and names["llvm0-warm"].mode == "stop0" and names["sim1:elab-cold"].cold
    assert [d[:2] for d in derived if d[0] in ("codegen-warm", "analyze-cold")] == [("analyze-cold", "sum"), ("codegen-warm", "diff")]
    assert make_cells(bins * 2, [], ["interp"], ["run"], True)[2]["basis"] == {}  # (labels A/B come from main)
    two = [dict(bins[0], label="A"), dict(bins[0], label="B")]
    assert make_cells(two, [], ["interp"], ["elab"], True)[2]["basis"] == {"B/elab-cold": "A/elab-cold", "B/elab-warm": "A/elab-warm"}
    c = lambda w, f=None: dict(cell(w, 100 * w), **({"files": f} if f else {}))
    d = derive("sum", [c(1, 10), c(0.5, 5)])
    assert d["wall"] == 1.5 and d["files"] == 15 and d["ins"] == 150
    assert abs(derive("diff", [c(9.7), c(0.05)])["wall"] - 9.65) < 1e-9 and derive("diff", [c(0.01), c(0.05)])["wall"] == 0
    assert derive("sum", [c(1, 1), {"wall": None, "rcs": [1], "tail": "x"}])["wall"] is None
    ph = {"analyze": ["analyze-cold", "analyze-cold-lib", "analyze-cold-own", "sim1:analyze-cold", "sim1:analyze-cold-lib", "sim1:analyze-cold-own"], "codegen": ["codegen-cold"], "elab": ["elab-cold", "sim1:elab-cold"]}
    bs = {"analyze-cold": "sim1:analyze-cold", "analyze-cold-lib": "sim1:analyze-cold-lib", "analyze-cold-own": "sim1:analyze-cold-own", "codegen-cold": "sim1:elab-cold", "elab-cold": "sim1:elab-cold"}
    meta3 = dict(meta, phases=ph, basis=bs, hidden=["llvm0-cold"], cells=[n for v in ph.values() for n in v])
    cs = {"analyze-cold": c(2, 20), "analyze-cold-lib": c(1.5, 15), "analyze-cold-own": c(0.5, 5), "sim1:analyze-cold": c(1, 20), "sim1:analyze-cold-lib": c(0.75, 15),
          "sim1:analyze-cold-own": c(0.25, 5), "codegen-cold": c(3), "elab-cold": c(0.1), "sim1:elab-cold": c(0.3), "llvm0-cold": {"wall": None, "rcs": [1], "tail": "hidden"}}
    md3 = markdown({"meta": meta3, "tests": {"a": {"category": "c1", "cells": cs}}})
    assert "## Phase: analyze" in md3 and "| analyze-cold | 1 | **2.00**" in md3 and "| codegen-cold | 1 | **10.00**" in md3 and "Analysis per file" in md3 and "100.0" in md3
    assert "hidden" not in md3 and "analyze-cold-lib | " in md3
    # the counted run is matched against `ok` like the check run (#794): a pass text beyond the output tail must not fail the cell
    fake = lambda m: lambda script, cwd, cores, env=None, instr=None, ref=True, ok=None: dict(rc=0, wall=1.0, cpu=1.0, rss=1.0, ins=5, tail="", ok_match=None if ok is None else m)
    cl = types.SimpleNamespace(name="x", tool="yavs", mode="run", warmups=1, prepare=lambda *a: "")
    real_timed = B.timed
    try:
        for m, want in ((True, 1), (False, 0)):
            B.timed = fake(m)
            r = measure_group({"ok": "Test passed"}, [cl], None, "0", 1, "instructions:u", None)["x"]
            assert r["ok"] == want and (m or r["ins"] is None), r
    finally:
        B.timed = real_timed
    # analysis calls (#745): consecutive files of one library, standard and flags share a call; --analyze-calls per-file splits them
    it = lambda lib, f, std="2008", fl=None: dict(lib=lib, std=std, file=f, flags=fl)
    items = [it("a", "1"), it("a", "2"), it("b", "3"), it("a", "4"), it("a", "5", "2019"), it("a", "6", "2019", ["--relaxed"])]
    tt = {"relaxed": False, "std": "2008"}
    items = [dict(i, file=REPO / "lib" / "ORDER.md") for i in items]  # no unit definitions: only library, standard and flags split the calls
    assert [len(c) for c in item_calls("yavs", tt, items)] == [2, 1, 1, 1, 1]
    with tempfile.TemporaryDirectory() as d:  # a unit defined twice in a run starts a new call
        fs = []
        for n, src in enumerate(["entity E is end;\narchitecture A of E is begin end;\n", "-- entity E is\npackage P is end;\n", "entity E is end;\n"]):
            (Path(d) / f"{n}.vhd").write_text(src)
            fs.append(dict(lib="a", std="2008", file=Path(d) / f"{n}.vhd", flags=None))
        assert [len(c) for c in item_calls("yavs", tt, fs)] == [2, 1]
    assert analyse_cmd("yavs", "/y", tt, "/w", "a", ["1", "2"], libs=["a"])[-2:] == ["1", "2"]
    global ANALYZE_CALLS
    ANALYZE_CALLS = "per-file"
    assert len(item_calls("yavs", tt, items)) == 6
    ANALYZE_CALLS = "batched"
    print(f"self-test ok ({len(tests)} tests listed)")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("bin", nargs="?", help="yavs binary")
    ap.add_argument("--compare", nargs=2, metavar=("A", "B"))
    ap.add_argument("--ref", default="", help="reference labels, e.g. sim1")
    ap.add_argument("--phases", default=",".join(PHASES), help="comma separated: analyze (the compile order, cold and warm; see --analyze-calls), elab (elaboration only), "
                    "codegen (llvm tier minus interp up to time 0), run (the --tiers cells)")
    ap.add_argument("--analyze-calls", choices=("batched", "per-file"), default="batched",
                    help="analysis cells, both tools: consecutive files of a library and standard in one call (default, #745) or one call per file")
    ap.add_argument("--tiers", default="interp,auto,auto-cold", help="run phase: comma separated: interp, auto, llvm, each also as <tier>-cold")
    ap.add_argument("--tests", default="", help="comma separated test names")
    ap.add_argument("--category", default="", help="only this category")
    ap.add_argument("--runs", default="auto", help="timed runs per cell (default auto: 5, 3 or 2 by test length; 0 = probe)")
    ap.add_argument("--cores", default="", help="cpu list of performance cores, one test at a time per core, e.g. 6,7 (required to time)")
    ap.add_argument("--no-instructions", action="store_true")
    ap.add_argument("--count-runs", action="store_true", help="count instructions and cycles in every timed run (median `cyc`, `ins_med`)")
    ap.add_argument("--out", default=str(CACHE))
    ap.add_argument("--label", default="")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--include-excluded", action="store_true", help="also run the tests marked `excluded` in tests.toml (known failures on yavs or the reference)")
    ap.add_argument("--prepare", action="store_true", help="build the world of each selected test under <cache>/worlds and print the yavs and reference commands (for perf and debugging)")
    ap.add_argument("--report", metavar="JSON")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    global COUNT_RUNS
    COUNT_RUNS = a.count_runs
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
    phases = [x for x in a.phases.split(",") if x]
    if bad := [x for x in phases if x not in PHASES]:
        raise Fail(f"unknown phase(s) {bad}; known: {', '.join(PHASES)}")
    phases = [x for x in PHASES if x in phases]
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
    global RECORDER, ANALYZE_CALLS
    ANALYZE_CALLS = a.analyze_calls
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
    cells, derived, spec = make_cells(binaries, refs, tiers, phases, bool(a.compare))
    print(f"{len(tests)} tests, phases {phases}, cells {[c.name for c in cells]}, cores {cores}, runs {runs}, load {open('/proc/loadavg').read().split()[:3]}", flush=True)
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
            mine, mine_derived, _ = make_cells(binaries, refs, tiers, phases, bool(a.compare))
            t0 = time.time()
            try:
                cellres = measure_test(t, mine, mine_derived, binaries, refs, core, runs, instr, CACHE / "scratch" / f"{os.getpid()}-{t['name']}")
            except Fail as e:
                cellres = {n: {"wall": None, "cpu": None, "rss": None, "ins": None, "ok": 0, "runs": 0, "rcs": [-1], "tail": str(e)} for n in [c.name for c in mine] + [d[0] for d in mine_derived]}
            with lock:
                results[t["name"]] = {"category": t["category"], "cells": cellres}
                msg = " ".join(f"{n}={c['wall']:.2f}s" if c.get("wall") is not None else f"{n}=FAILED" for n, c in cellres.items() if n not in spec["hidden"] and not DETAIL.search(n))
                print(f"[{core}] {t['name']} ({time.time() - t0:.0f}s): {msg}", flush=True)
        finally:
            with cond:
                free.append(core)
                cond.notify()

    with cf.ThreadPoolExecutor(len(cores)) as ex:
        list(ex.map(job, tests))
    order = [t["name"] for t in tests]
    meta = {"time": time.strftime("%Y-%m-%dT%H:%M:%S"), "cores": a.cores, "runs": runs, "instr_event": instr, "refs": refs, "compare": bool(a.compare),
            "tiers": tiers, "phase_list": phases, "phases": spec["phases"], "basis": spec["basis"], "hidden": spec["hidden"],
            "cells": [n for v in spec["phases"].values() for n in v], "binaries": {b["label"]: b["key"] for b in binaries}, "binary_paths": {b["label"]: b["path"] for b in binaries},
            "sources": pins, "host_cpus": os.cpu_count(), "analyze_calls": a.analyze_calls}
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
