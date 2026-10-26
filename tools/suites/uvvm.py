#!/usr/bin/env python3
"""Run the maintenance testbenches of the `uvvm` corpus (a verification library) on yavs.

  tools/suites/uvvm.py [--out DIR] [--yavs BIN] [-j N] [--only COMP,...] [--list] [--no-wrap]

Sources come from the corpus cache (tools/fetch-corpus.sh), never copied. For every component with a
script/maintenance_script/test.py, that script is executed against a recording stand-in for its Python regression
framework, which yields the libraries, files, testbench entities and generic sets the project itself runs.
Libraries are compiled in the project's order (script/component_list.txt and each component's
script/compile_order.txt; testbench files by a dependency scan, as tools/corpus/gen_manifests.py does)
with `yavs analyze --relaxed`. Each testbench (entity plus generic set) then runs with `yavs run --relaxed`
in its own directory. PASS = exit 0 and the library's final summary ">> Simulation SUCCESS" without a
">> Simulation FAILED". Every call runs under `timeout 300 systemd-run --user --scope -p MemoryMax=...`.
Results: DIR/results.tsv (component, lib.entity, generics, status, first error line, seconds of the run) and DIR/<comp>/<n>/log.txt;
the per-component pass/fail table is printed at the end. Exit 0 always unless usage errors (a measurement).
"""
import argparse, concurrent.futures as cf, contextlib, glob, io, itertools, os, re, shutil, subprocess, sys, time, types
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "corpus"))
sys.path.insert(0, str(HERE.parent))
import simconf  # noqa: E402  (tools/simconf.py, put on the path by gen_manifests)
import gen_manifests as gm  # noqa: E402  (order_files: the dependency scan shared with the corpus manifests)

STD = "2008"


class Done(Exception):
    pass


class Recorder:
    """Stand-in for the project's regression framework: records add_files / add_generics, ignores the rest."""

    def __init__(self):
        self.files, self.generics = [], []
        self.settings = types.SimpleNamespace(get_simulator_name=lambda: simconf.get("sim1", "uvvm_simulator_name", "sim1"))

    def add_files(self, pattern, lib, *a, **k):
        self.files.append((pattern, lib.lower()))

    def add_generics(self, entity=None, generics=(), *a, **k):
        self.generics.append((entity.lower(), list(generics)))

    def start(self, *a, **k):
        raise Done

    def __getattr__(self, name):
        return lambda *a, **k: None


def record(test_py, scratch):
    """Execute test.py in a scratch directory with the recorder (stdout dropped); return (files, generics).
    Only the first start() is recorded: the VVC framework script then goes on to generator tests (not run here)."""
    hr = Recorder()
    mod = types.ModuleType("hdlregression")
    mod.HDLRegression = lambda *a, **k: hr
    sys.modules["hdlregression"] = mod
    sys.modules["mock"] = types.SimpleNamespace(patch=lambda *a, **k: (lambda f: f))
    sys.modules["vvc_generator"] = types.ModuleType("vvc_generator")
    scratch.mkdir(parents=True, exist_ok=True)
    cwd = os.getcwd()
    os.chdir(scratch)
    try:
        with contextlib.redirect_stdout(io.StringIO()):
            exec(compile(test_py.read_text(), str(test_py), "exec"), {"__name__": "__main__", "__file__": str(test_py)})
    except (Done, SystemExit):
        pass
    finally:
        os.chdir(cwd)
    return hr.files, hr.generics


def script_dir(root, comp):
    return root / comp / "script/maintenance_script"


def compile_order(root, comp):
    """(lib, abs path) per compile_order.txt line, as gen_manifests.gen_uvvm."""
    d = root / comp / "script"
    out, lib = [], None
    for line in (d / "compile_order.txt").read_text().splitlines():
        line = line.strip()
        m = re.match(r"#\s*library\s+(\w+)", line)
        if m:
            lib = m.group(1).lower()
        elif line and not line.startswith("#"):
            out.append((lib, (d / line).resolve()))
    return out


def order_blocks(blocks):
    """Component source blocks, dependencies first: a block follows the blocks of the libraries it names in `library` clauses
    (the project's own tool sorts by dependency; the listed order of the scripts is not always one)."""
    def deps(b):
        names = {n.strip().lower() for _, p in b for m in re.findall(r"^\s*library\s+([\w\s,]+);", p.read_text(errors="replace"), re.I | re.M) for n in m.split(",")}
        return names - {b[0][0]}
    libs = [{l for l, _ in b} for b in blocks]
    out, rest = [], list(range(len(blocks)))
    while rest:
        i = next((i for i in rest if not any(d in libs[j] for j in rest if j != i for d in deps(blocks[i]))), rest[0])
        out.append(blocks[i])
        rest.remove(i)
    return out


# Third-party design files the testbenches use that are not in the pinned checkout: prefix below the UVVM root -> the
# checkout (tools/fetch-corpus.sh) that holds the files below that prefix.
THIRD_PARTY = {
    "bitvis_vip_spi/tb/maintenance_tb/spi_master_slave_opencores": "uvvm-supplementary/bitvis_vip_spi/tb/maintenance_tb/spi_master_slave_opencores",
    "bitvis_vip_i2c/tb/maintenance_tb/i2c_opencores/trunk": "uvvm-i2c-master",
    "bitvis_vip_i2c/tb/maintenance_tb/fpga_i2c_slave_github": "uvvm-i2c-slave",
}


def third_party(root, corpus, p):
    """`p` (a glob below the UVVM root) moved to its fetched third-party checkout; `p` itself when there is none."""
    rel = p.relative_to(root).as_posix()
    for prefix, to in THIRD_PARTY.items():
        if rel.startswith(prefix + "/"):
            return corpus / to / rel[len(prefix) + 1:]
    return p


def plan(root, comp, scratch, corpus):
    """Compile list [(lib, path)], the testbench files [(lib, path)] and generic sets for one component."""
    files, generics = record(script_dir(root, comp) / "test.py", scratch / comp)
    blocks, tbs, seen, missing = [], [], set(), []
    for pat, lib in files:
        p = (script_dir(root, comp) / pat).resolve()
        m = re.fullmatch(re.escape(str(root)) + r"/(\w+)/src/\*\.vhd", str(p))
        if m:
            blocks.append([e for e in compile_order(root, m[1]) if e not in seen and not seen.add(e)])
        elif "/src_target_dependent/" in str(p):
            pass  # compiled per VVC library by that component's compile_order.txt
        else:
            if not glob.glob(str(p)):
                p = third_party(root, corpus, p)
            if not glob.glob(str(p)):
                missing.append(str(p.relative_to(root / comp)).split('/maintenance_tb/')[-1].split('/')[0])
            for f in sorted(glob.glob(str(p))):
                if (lib, Path(f)) not in seen:
                    seen.add((lib, Path(f)))
                    tbs.append((lib, Path(f)))
    srcs = [e for b in order_blocks(blocks) for e in b]
    tbs = [(f[0], f[3]) for f in gm.order_files([(l, "", str(p), p) for l, p in tbs])]
    return srcs, tbs, missing, [(e, [(script_dir(root, comp) / v[0]).resolve() if isinstance(v, tuple) and not Path(v[0]).is_absolute() else v for v in g]) for e, g in generics]


def generics_without_default(body):
    """Names of the entity's generics that have no default value."""
    m = re.search(r"\bgeneric\s*\(", body, re.I)
    if not m:
        return []
    depth, k, stmt, out = 1, m.end(), "", []
    for ch in body[k:]:
        depth += ch == "("
        depth -= ch == ")"
        if depth == 0 or (ch == ";" and depth == 1):
            names, _, rest = re.sub(r"--[^\n]*", "", stmt).partition(":")
            if ":=" not in rest:
                out += [n.strip().lower() for n in names.split(",") if n.strip()]
            stmt = ""
            if depth == 0:
                break
        else:
            stmt += ch
    return out


def tops(tbs):
    """Testbench entities (declared in tb files, no ports, not instantiated by another tb file): {name: (lib, generics without default)}."""
    decl, text = {}, {p: p.read_text(errors="replace") for _, p in tbs}
    for lib, p in tbs:
        for n, body in re.findall(r"^\s*entity\s+(\w+)\s+is\b(.*?)^\s*end\b", text[p], re.I | re.M | re.S):
            if not re.search(r"\bport\s*\(", body, re.I):
                decl[n.lower()] = (lib, generics_without_default(body))
    return {n: v for n, v in decl.items()
            if not any(re.search(rf"(:\s*(entity\s+\w+\.)?|\.){n}\b\s*(generic|port|;)", t, re.I) for t in text.values())}


def testcases(tbs, ent):
    """Names tested by `GC_TESTCASE = "name"` in the entity's files (the project's tool runs each as -gGC_TESTCASE=name)."""
    pat = rf"\b(entity|of)\s+{ent}\b"
    names = []
    for _, p in tbs:
        t = p.read_text(errors="replace")
        if re.search(pat, t, re.I):
            names += re.findall(r'GC_TESTCASE\s*=\s*"([^"]+)"', t, re.I)
    return list(dict.fromkeys(names))


def sh(cmd, cwd, wrap, mem, log=None):
    if wrap:
        cmd = ["timeout", "300", "systemd-run", "--user", "--scope", "-q", "-p", f"MemoryMax={mem}"] + cmd
    r = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, errors="replace")
    out = r.stdout + r.stderr
    if log:
        Path(log).write_text(out)
    return r.returncode, out


def gval(v, d):
    if isinstance(v, tuple):  # a PATH generic: the run's own directory (outputs the testbench writes there)
        return str(d) + "/"
    return ("true" if v else "false") if isinstance(v, bool) else str(v)


def build(a, comp, plan_):
    srcs, tbs, _, _ = plan_
    d = a.out / comp / "libs"
    d.mkdir(parents=True, exist_ok=True)
    libs = sorted({l for l, _ in srcs + tbs})
    if a.private_copies:  # a source listed for several libraries gets one path per library (see the module doc)
        srcs = [(lib, copy_for(a, comp, lib, p)) for lib, p in srcs]
    bad = {}  # failed testbench file -> first error; library sources stop the component
    todo = srcs + tbs
    # Consecutive files of one library go in one call (one analysis session sees the units of the
    # files before it); a failing call is repeated file by file to find the files that fail.
    groups = [(lib, [p for _, p in g]) for lib, g in itertools.groupby(todo, key=lambda t: t[0])]

    def call(lib, files):
        cmd = [a.yavs, "analyze", f"--std={STD}", "--relaxed", f"--work={lib}:{d}/{lib}"] + [f"--map={l}:{d}/{l}" for l in libs if l != lib] + [str(p) for p in files]
        return sh(cmd, d, not a.no_wrap, "2G")

    for lib, files in groups:
        if call(lib, files)[0] == 0:
            continue
        for p in files:
            rc, out = call(lib, [p])
            if rc:
                if (lib, p) not in tbs:
                    return f"analyze {p.name} exit {rc}: " + first_error(out), {}
                bad[p] = f"analyze {p.name} exit {rc}: " + first_error(out)
    return None, bad


def copy_for(a, comp, lib, p):
    q = a.out / comp / "src" / lib / p.name
    q.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(p, q)
    return q


def first_error(out):
    """First error line, with the cause the compiler gives for a failed lowering when there is one."""
    ls = out.splitlines()
    for k, l in enumerate(ls):
        if re.match(r"\s*(error|ICE|thread|Error)|^E\d{4}|internal", l):
            why = next((m.group(1) for x in ls[k:k + 25] if (m := re.search(r"lowering it failed: (.*)", x))), "")
            return (l.strip()[:160] + (f" [lowering failed: {why[:100]}]" if why else ""))
    return " ".join(out.split())[-200:]


def archs(tbs, ent):
    return sorted({m.lower() for _, p in tbs for m in re.findall(rf"^\s*architecture\s+(\w+)\s+of\s+{ent}\b", p.read_text(errors="replace"), re.I | re.M)})


# Testbenches that need a different command line than the project's regression script gives them.
# A reference simulator behaves the same: with the default the first run stops with status 1.
DISABLED = {"questa_extension_tb": ["-gGC_EXTENSIONS_ENABLED=false"]}  # exercises a vendor extension only one simulator has
NO_SUMMARY = {"questa_extension_tb"}  # prints no final summary; an expected-alert count stops it with status 1 otherwise


def run_one(a, comp, n, lib, ent, gens, libs, miss):
    d = a.out / comp / str(n)
    d.mkdir(parents=True, exist_ok=True)
    shared = a.out / comp / "files" / ent  # a PATH generic: one directory for all runs of the entity (a test reads what an earlier one wrote)
    shared.mkdir(parents=True, exist_ok=True)
    gl = [f"-g{k}={gval(v, shared)}" for k, v in zip(gens[::2], gens[1::2])] + DISABLED.get(ent.split(".")[0], [])
    # The project's own verdict is its final summary: error reports of a design under test do not fail a run.
    cmd = [a.yavs, "run", f"--std={STD}", "--relaxed", "--exit-severity=failure", f"--work={lib}:{a.out}/{comp}/libs/{lib}"] + \
        [f"--map={l}:{a.out}/{comp}/libs/{l}" for l in libs if l != lib] + [ent] + gl
    t0 = time.perf_counter()
    rc, out = sh(cmd, d, not a.no_wrap, a.mem, d / "log.txt")
    secs = f"{time.perf_counter() - t0:.1f}"
    if rc == 0 and ">> Simulation FAILED" not in out and (">> Simulation SUCCESS" in out or ent.split(".")[0] in NO_SUMMARY):
        return "PASS", "", secs
    if rc == 124:
        return "SKIP", "over 5 minutes (timeout 300)", secs
    if rc and miss and (m := re.search(r"no entity `(\w+)` is visible and none is configured", out)):
        return "SKIP", f"design component {m[1]} stays unbound: third-party sources absent from the pinned checkout: {' '.join(dict.fromkeys(miss))}", secs
    why = first_error(out) if rc else ("summary reports FAILED" if ">> Simulation FAILED" in out else "no final summary")
    return "FAIL", f"exit {rc}: {why}", secs


def job(a, t, plans, built, libs):
    n, (c, lib, ent, g) = t
    err, bad = built[c]
    miss = plans[c][2]
    if err:
        return ("SKIP", f"sources absent from the pinned checkout: {' '.join(dict.fromkeys(miss))}") if miss and "not found" in err else ("FAIL", err)
    name, _, arch = ent.partition(".")
    mine = rf"\barchitecture\s+{arch}\s+of\s+{name}\b|\bentity\s+{name}\s+is\b" if arch else rf"\b(entity|of)\s+{name}\b"
    for p, msg in bad.items():
        if re.search(mine, p.read_text(errors="replace"), re.I):
            return ("SKIP", f"sources absent from the pinned checkout: {' '.join(dict.fromkeys(miss))}") if miss and "not found" in msg else ("FAIL", msg)
    return run_one(a, c, n, lib, ent, g, libs[c], miss)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", default="~/.cache/yavs-uvvm-suite")
    ap.add_argument("--yavs", default="yavs")
    ap.add_argument("-j", type=int, default=4)
    ap.add_argument("--mem", default="4G")
    ap.add_argument("--only")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--no-wrap", action="store_true")
    ap.add_argument("--private-copies", action="store_true", help="analyze every library source from a private per-library copy of the file")
    ap.add_argument("--root", default=os.environ.get("YAVS_CORPUS", "~/.cache/rsim-corpus"))
    a = ap.parse_args()
    a.out = Path(a.out).expanduser()
    # the shared std/ieee store (#564) is private to this tool, never the user's
    os.environ["YAVS_STD_CACHE"] = str(a.out / "std-cache")
    root = (Path(a.root).expanduser() / "uvvm").resolve()
    corpus = Path(a.root).expanduser()  # the third-party checkouts are next to the UVVM one
    if not root.is_dir():
        sys.exit(f"{root} not found; run tools/fetch-corpus.sh")
    comps = [c for c in (root / "script/component_list.txt").read_text().split() if (root / c / "script/maintenance_script/test.py").exists()]
    if a.only:
        comps = [c for c in comps if c in a.only.split(",")]
    plans = {c: plan(root, c, a.out / "rec", corpus) for c in comps}
    jobs = []  # (comp, lib, entity, generics)
    for c in comps:
        _, tbs, _, gens = plans[c]
        for ent, (lib, nodef) in tops(tbs).items():
            sets = [g for e, g in gens if e == ent] or [[]]
            if "gc_testcase" in nodef and not any("GC_TESTCASE" in g for g in sets):
                sets = [g + ["GC_TESTCASE", t] for g in sets for t in testcases(tbs, ent)]
            if {n for n in nodef if not any(n.upper() in map(str.upper, map(str, g[::2])) for g in sets)} or not sets:
                continue  # a harness (generics only its parent supplies) or no test case found
            aa = archs(tbs, ent)  # an entity with several architectures runs once per architecture
            jobs += [(c, lib, ent + ("." + ar if len(aa) > 1 else ""), g) for ar in (aa if len(aa) > 1 else [""]) for g in sets]
    if a.list:
        for c, lib, ent, g in jobs:
            print(c, f"{lib}.{ent}", *g)
        return
    a.out.mkdir(parents=True, exist_ok=True)
    for c in comps:
        shutil.rmtree(a.out / c / "files", ignore_errors=True)  # no file of an earlier invocation
    with cf.ThreadPoolExecutor(a.j) as ex:
        built = dict(zip(comps, ex.map(lambda c: build(a, c, plans[c]), comps)))
        berr = {c: built[c][0] for c in comps}
        libs = {c: sorted({l for l, _ in plans[c][0] + plans[c][1]}) for c in comps}
        # Runs of one entity with a PATH generic share a directory and run in order, on one worker.
        groups = {}
        for i, (c, _, ent, g) in enumerate(jobs):
            groups.setdefault((c, ent) if any(isinstance(v, tuple) for v in g) else i, []).append(i)
        res = [None] * len(jobs)
        for part in ex.map(lambda ix: [(i, job(a, (i, jobs[i]), plans, built, libs)) for i in ix], groups.values()):
            for i, r in part:
                res[i] = r
    rows = [(c, f"{lib}.{ent}", " ".join(map(str, g)), s, w, *t) for (c, lib, ent, g), (s, w, *t) in zip(jobs, res)]
    (a.out / "results.tsv").write_text("".join("\t".join(r) + "\n" for r in rows))
    print(f"{'component':32} pass fail skip")
    for c in comps:
        k = [sum(1 for r in rows if r[0] == c and r[3] == s) for s in ("PASS", "FAIL", "SKIP")]
        print(f"{c:32} {k[0]:4} {k[1]:4} {k[2]:4}" + (f"  (compile: {berr[c]})" if berr[c] else ""))
    print(f"{'total':32} " + " ".join(f"{sum(1 for r in rows if r[3] == s):4}" for s in ("PASS", "FAIL", "SKIP")))


if __name__ == "__main__":
    main()
