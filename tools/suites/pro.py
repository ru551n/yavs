#!/usr/bin/env python3
"""Run a verification library's own regression (its .pro build scripts) on yavs.

    pro.py [--std 2008|2019] [--root DIR] [--out DIR] [--yavs BIN] [--jobs N]
           [--filter SUBSTR] [--list] [--analyze-only]

Reads OsvvmLibraries.pro (the library build) and RunAllTests.pro (the tests) from the corpus
checkout `osvvm-libraries` (default ~/.cache/rsim-corpus/osvvm-libraries) with a small Tcl subset.
Only the commands those scripts use are known; any other command is an error naming file and line.
Library, testbench and test files are analyzed in script order into per-library directories under
`--out`; each test is then simulated with `yavs run` under timeout and systemd-run (MemoryMax=4G).
Pass = exit status 0 and the library's own summary line with zero errors. Results go to
`OUT/results.tsv`, logs to `OUT/run/<n>/log.txt`. Python stdlib only; nothing is copied into the repo.
"""
import argparse, os, re, subprocess, sys, time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

SKIP_DIRS = {"CoSim": "needs a native co-simulation host", "CoSimPCIe": "needs a native co-simulation host"}
# Commands that only affect reporting or the vendor tool; accepted and ignored on purpose.
NOOP = {"SetCoverageSimulateEnable", "SetCoverageAnalyzeEnable", "SetExtendedSimulateOptions",
        "SetExtendedAnalyzeOptions", "SetExtendedOptimizeOptions", "SetLogSignals", "SetInteractive"}


class ProError(Exception):
    pass


def words(s):
    """Split a Tcl command into words; {..} and [..] keep their delimiters."""
    out, i, n = [], 0, len(s)
    while i < n:
        c = s[i]
        if c in " \t\n":
            i += 1
            continue
        j = i
        if c in "{[":
            close, depth = ("}" if c == "{" else "]"), 0
            while j < n:
                if s[j] == c: depth += 1
                elif s[j] == close:
                    depth -= 1
                    if depth == 0: break
                j += 1
            j += 1
        elif c == '"':
            j = s.index('"', i + 1) + 1
        else:
            while j < n and s[j] not in " \t\n":
                j += 1
        out.append(s[i:j])
        i = j
    return out


def commands(text):
    """Yield (line number, command) with comments dropped; braces and brackets continue a command."""
    buf, depth, start = "", 0, 0
    for no, raw in enumerate(text.splitlines(), 1):
        if not buf and raw.lstrip().startswith("#"):
            continue
        if not buf:
            raw = re.sub(r"\s*;\s*#.*$", "", raw)
            start = no
        buf += ("\n" if buf else "") + raw
        depth = buf.count("{") - buf.count("}") + buf.count("[") - buf.count("]")
        if depth <= 0 and not buf.rstrip().endswith("\\"):
            if buf.strip():
                yield start, buf.strip()
            buf = ""
    if buf.strip():
        yield start, buf.strip()


class Plan:
    """Interpret the scripts into a list of analyze and test actions."""

    def __init__(self, root, std, out):
        self.root, self.std, self.out = root, std, out
        self.cwd, self.lib, self.where = root, "work", ""
        self.suite, self.suite_no, self.testname, self.generics = None, 0, None, []
        self.phase = "build"
        self.libdirs = {}      # library name -> directory (latest declaration)
        self.actions = []      # ("analyze"|"test", dict)
        self.skipped = []      # (what, reason)
        self.vars = {"ToolName": "yavs", "ToolVendor": "yavs", "VhdlVersion": int(std),
                     "ToolSupportsGenericPackages": True, "Supports2019Interface": True,
                     "Supports2019ImpureFunctions": True, "Supports2019FilePath": True,
                     "Supports2019AssertApi": True, "Supports2019Integer64Bits": True,
                     "Supports2019Generics": True, "FunctionalCoverageIntegratedInSimulator": "default",
                     "UseMemoryPkg01": False, "ClockResetVersion": "2026.09", "OsvvmVersion": "2026.09",
                     "OsvvmDevDerivePackageHeaders": False, "OsvvmDevDeriveArchitectures": False}

    def err(self, msg):
        raise ProError(f"{self.where}: {msg}")

    def subst(self, w):
        w = w.strip('"')
        if w.startswith("[") and w.endswith("]"):
            c = words(w[1:-1])
            if c[0] == "FindOsvvmScriptSettingsPkg":
                return self.script_settings()
            if c[0] == "FindSettingsPkgBody":
                return f"{self.subst(c[1])}_default.vhd"
            if c[0] == "NoNullRangeWarning":
                return None
            if c[0] == "CreateTestCaseCommonPkg":
                return self.common_pkg(*[self.subst(x) for x in c[1:]])
            if c[0] == "generic":
                self.generics.append((c[1], self.subst(c[2])))
                return None
            self.err(f"unknown command substitution [{c[0]}]")
        def var(m):
            if m.group(1) not in self.vars:
                self.err(f"unknown variable ::osvvm::{m.group(1)}")
            return str(self.vars[m.group(1)])
        return re.sub(r"\$\{?::osvvm::(\w+)\}?", var, w)

    def script_settings(self):
        """What the scripts generate for OsvvmScriptSettingsPkg's body: the default one with the
        scripts' release as OSVVM_REVISION and OSVVM_SETTINGS_REVISION (the settings that
        follow it, such as the transcript layout, are those of the validated results)."""
        text = (self.cwd / "OsvvmScriptSettingsPkg_default.vhd").read_text()
        for c in ("OSVVM_REVISION", "OSVVM_SETTINGS_REVISION"):
            text, n = re.subn(rf'({c}\s*:\s*string\s*:=\s*)"[^"]*"', rf'\g<1>"{self.vars["OsvvmVersion"]}"', text)
            if n != 1:
                self.err(f"no {c} in OsvvmScriptSettingsPkg_default.vhd")
        f = self.out / "gen" / "OsvvmScriptSettingsPkg_generated.vhd"
        f.parent.mkdir(parents=True, exist_ok=True)
        f.write_text(text)
        return str(f)

    def common_pkg(self, name="TestCaseCommonPkg", validated="../ValidatedResults"):
        """What the library's CreateTestCaseCommonPkg writes (the 2008 form), into OUT/gen."""
        d = self.out / "gen" / f"{self.suite_no}_{self.suite}"
        f = d / f"{name}.vhd"
        d.mkdir(parents=True, exist_ok=True)
        f.write_text(f'''-- generated by pro.py
library osvvm ;
context osvvm.OsvvmContext ;
package {name} is
  constant PATH_TO_TEST_SRC            : string := "{self.cwd}/" ;
  constant PATH_TO_VALIDATED_RESULTS   : string  := PATH_TO_TEST_SRC & "{validated}/" ;
  constant CHECK_TRANSCRIPT            : boolean := PATH_TO_TEST_SRC'length > 0 ;
  constant BUILD_NAME                  : string  := "yavs" ;
  constant TEST_SUITE_NAME             : string  := "{self.suite}" ;
  constant PATH_TO_RESULTS             : string := OSVVM_BASE_DIRECTORY & BUILD_NAME & "/results/" & TEST_SUITE_NAME & "/" ;
  constant RESULTS_DIR             : string := PATH_TO_RESULTS ;
  constant VALIDATED_RESULTS_DIR   : string := PATH_TO_VALIDATED_RESULTS ;
end package {name} ;
''')
        return str(f)

    def cond(self, expr):
        if re.fullmatch(r"!\[file exists \$CURR_DIR/bmp_logs\]", expr.strip()):
            return False  # the directory is made per test run
        def var(m):
            if m.group(1) not in self.vars:
                self.err(f"unknown variable ::osvvm::{m.group(1)}")
            return repr(self.vars[m.group(1)])
        e = re.sub(r"\[DirectoryExists\s+(\S+?)\]", lambda m: repr((self.cwd / m.group(1)).is_dir()), expr)
        e = re.sub(r"\[string compare\s+(\S+)\s+(\S+)\]",
                   lambda m: f"(({m.group(1)}>{m.group(2)})-({m.group(1)}<{m.group(2)}))", e)
        e = re.sub(r"\$\{?::osvvm::(\w+)\}?", var, e)
        e = e.replace(" ne ", " != ").replace(" eq ", " == ").replace("&&", " and ").replace("||", " or ")
        e = re.sub(r"\b(true|false)\b", lambda m: m.group(1).capitalize(), e)
        e = re.sub(r'"(\d+(?:\.\d+)?)"', lambda m: repr(m.group(1)), e)
        if re.search(r"\[|\$", e):
            self.err(f"unsupported condition {expr!r}")
        return bool(eval(e, {"__builtins__": {}}, {}))

    def run_file(self, path):
        text = path.read_text(encoding="utf-8", errors="replace")
        self.run_text(text, path)

    def run_text(self, text, path):
        for no, cmd in commands(text):
            self.where = f"{path.relative_to(self.root)}:{no}"
            self.command(words(cmd), path)

    def command(self, w, path):
        name, a = w[0], w[1:]
        if name == "library":
            self.lib = self.subst(a[0])
            if self.phase == "tests" and self.lib.lower() not in self.build_libs:
                d = self.out / "tb" / f"{self.suite_no}_{self.suite or 'none'}" / self.lib
            else:
                d = self.out / "libs" / self.lib
            self.libdirs[self.lib.lower()] = d
        elif name == "analyze":
            f = self.subst(a[0])
            if f is None:
                self.err("analyze without a file")
            self.generics = []
            self.actions.append(("analyze", dict(lib=self.lib, file=(self.cwd / f).resolve(), where=self.where,
                                                 suite=self.suite_no, libs=dict(self.libdirs))))
        elif name == "include":
            self.include(self.subst(a[0]))
        elif name == "ChangeWorkingDirectory":
            self.cwd = (self.cwd / self.subst(a[0])).resolve()
        elif name == "TestSuite":
            self.suite, self.suite_no = self.subst(a[0]), self.suite_no + 1
        elif name in ("TestName", "TestCase"):
            self.testname = self.subst(a[0])
        elif name == "generic":
            self.generics.append((a[0], self.subst(a[1])))
        elif name in ("RunTest", "simulate"):
            self.test(name, a)
        elif name == "set" and a[0] == "::osvvm::CurrentWorkingDirectory":
            m = re.fullmatch(r"\[file join \$::osvvm::CurrentWorkingDirectory (\w+)\]", a[1])
            if not m:
                self.err(f"unsupported set value {a[1]}")
            self.cwd = self.cwd / m.group(1)
        elif name == "set" and a[0] == "CURR_DIR" and a[1] == "[pwd]":
            pass  # the tests run in a private directory that has bmp_logs (see run_test)
        elif name in NOOP:
            pass
        elif name == "if":
            i = 1
            while i < len(a) + 1:
                if w[i] == "else":
                    self.run_text(w[i + 1][1:-1], path); break
                if w[i] == "elseif":
                    i += 1
                if self.cond(w[i][1:-1]):
                    self.run_text(w[i + 1][1:-1], path); break
                i += 2
                if i >= len(w) or w[i] not in ("elseif", "else"):
                    break
        else:
            self.err(f"unknown command {name!r}")

    def include(self, p):
        t = (self.cwd / p).resolve()
        if t.name in SKIP_DIRS:
            self.skipped.append((str(t.relative_to(self.root)), SKIP_DIRS[t.name]))
            return
        if t.is_dir():
            for n in (t.name + ".pro", "build.pro"):
                if (t / n).is_file():
                    t = t / n
                    break
            else:
                self.err(f"no .pro script in {p}")
        saved, where = self.cwd, self.where
        self.cwd = t.parent
        self.run_file(t)
        self.cwd, self.where = saved, where

    def test(self, kind, a):
        gens_before = len(self.generics)
        args = [self.subst(x) for x in a]
        args = [x for x in args if x is not None]
        if kind == "RunTest":
            f = args[0]
            sim = args[1] if len(args) > 1 else Path(f).stem
            self.actions.append(("analyze", dict(lib=self.lib, file=(self.cwd / f).resolve(), where=self.where,
                                                 suite=self.suite_no, libs=dict(self.libdirs))))
        else:
            sim = args[0]
        if self.suite is None:
            self.suite, self.suite_no = self.lib, self.suite_no + 1
        self.actions.append(("test", dict(name=self.testname or sim, sim=sim, lib=self.lib, where=self.where,
                                          suite=self.suite, suite_no=self.suite_no, generics=list(self.generics),
                                          libs=dict(self.libdirs))))
        self.generics, self.testname = [], None


def build_plan(root, std, out):
    p = Plan(root, std, out)
    p.build_libs = set()
    p.run_file(root / "OsvvmLibraries.pro")
    p.build_libs = set(p.libdirs)
    p.phase = "tests"
    p.cwd, p.suite, p.suite_no = root, None, 0
    p.run_file(root / "RunAllTests.pro")
    return p


def sandbox(cmd, log, cwd, timeout=300):
    """Run cmd under timeout and systemd-run; return (status, seconds). Output appended to log."""
    full = ["timeout", str(timeout)] + ([] if NO_WRAP else ["systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=4G"]) + cmd
    t = time.time()
    with open(log, "ab") as f:
        r = subprocess.run(full, stdout=f, stderr=subprocess.STDOUT, cwd=cwd)
    return r.returncode, time.time() - t


NO_WRAP = False  # --no-wrap: CI runners have no systemd user session
SUMMARY = re.compile(r"^%%.*\bDONE\s+PASSED\b", re.M)
FAILSUM = re.compile(r"^%%.*\bDONE\s+FAILED\b", re.M)


def verdict(status, text):
    """Pass = exit 0 and the library's summary line reports zero errors."""
    if status == 124:
        return "TIMEOUT"
    if status == 0 and SUMMARY.search(text) and not FAILSUM.search(text):
        return "PASS"
    if status == 0:
        return "NOSUMMARY"
    return "FAIL"


def first_cause(text):
    """First error diagnostic, else the first failed check line, else the last output line."""
    for pat in (r"^(error|fatal)", r"^%%.*(ERROR|FAILED|FAILURE)", r"panicked|Killed"):
        for line in text.splitlines():
            if re.search(pat, line):
                return line.strip()[:200]
    lines = [l for l in text.splitlines() if l.strip()]
    return "no diagnostic: " + (lines[-1].strip()[:150] if lines else "no output")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--std", default="2008")
    ap.add_argument("--root", default=os.path.expanduser("~/.cache/rsim-corpus/osvvm-libraries"))
    ap.add_argument("--out", required=True)
    ap.add_argument("--yavs", default="yavs")
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--filter", default="")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--analyze-only", action="store_true")
    ap.add_argument("--no-wrap", action="store_true", help="run without systemd-run (CI)")
    o = ap.parse_args()
    global NO_WRAP
    NO_WRAP = o.no_wrap
    if o.jobs > 4:
        sys.exit("pro.py: at most 4 parallel runs")
    out, root = Path(o.out).resolve(), Path(o.root).resolve()
    # the shared std/ieee store (#564) is private to this tool, never the user's
    os.environ["YAVS_STD_CACHE"] = str(out / "std-cache")
    try:
        plan = build_plan(root, o.std, out)
    except ProError as e:
        sys.exit(f"pro.py: {e}")
    tests = [d for k, d in plan.actions if k == "test"]
    print(f"{sum(1 for k, _ in plan.actions if k == 'analyze')} analyze, {len(tests)} tests; skipped: {plan.skipped}")
    if o.list:
        for t in tests:
            print(t["suite"], t["sim"], t["generics"])
        return
    out.mkdir(parents=True, exist_ok=True)
    (out / "run").mkdir(exist_ok=True)
    # phase 1: analyze everything in script order; consecutive files of one library with the same
    # library mapping go in one call (one analysis session sees the units of the files before it)
    failed, nan = {}, 0
    batches = []
    for k, d in plan.actions:
        if k != "analyze":
            continue
        key = (d["lib"].lower(), tuple(sorted(d["libs"].items())), d["suite"])
        if batches and batches[-1][0] == key:
            batches[-1][1].append(d)
        else:
            batches.append((key, [d]))
    log = out / "analyze.log"

    def analyze(group):
        d = group[0]
        dirs = dict(d["libs"])
        cmd = [o.yavs, "analyze", f"--std={o.std}", f"--work={d['lib']}:{dirs[d['lib'].lower()]}"]
        cmd += [f"--map={n}:{p}" for n, p in dirs.items()]
        with open(log, "ab") as f:
            for x in group:
                f.write(f"== {x['where']} {x['file'].name}\n".encode())
        return sandbox(cmd + [str(x["file"]) for x in group], log, out)[0]

    for _, group in batches:
        nan += len(group)
        if analyze(group) == 0:
            continue
        for x in group:  # a failing batch stops at its first error: find the files that fail
            if len(group) > 1 and analyze([x]) == 0:
                continue
            failed.setdefault(x["suite"], []).append(f"{x['where']} {x['file'].name}")
    print(f"analyzed {nan} files; failures: {sum(len(v) for v in failed.values())}")
    if o.analyze_only:
        for v in failed.values():
            print(*v, sep="\n")
        return

    def one(item):
        n, t = item
        rd = out / "run" / str(n)
        rd.mkdir(parents=True, exist_ok=True)
        log = rd / "log.txt"
        log.write_bytes(b"")
        (rd / "yavs" / "results" / t["suite"]).mkdir(parents=True, exist_ok=True)
        (rd / "bmp_logs").mkdir(exist_ok=True)
        cmd = [o.yavs, "run", f"--std={o.std}", f"--work={t['lib']}:{t['libs'][t['lib'].lower()]}"]
        cmd += [f"--map={a}:{b}" for a, b in t["libs"].items()] + [t["sim"]] + [f"-g{a}={b}" for a, b in t["generics"]]
        status, secs = sandbox(cmd, log, rd)
        text = log.read_text(errors="replace")
        v = verdict(status, text)
        return (n, t["suite"], t["name"], v, status, f"{secs:.1f}", first_cause(text) if v != "PASS" else "")

    items = [(n, t) for n, t in enumerate(tests) if o.filter in f"{t['suite']}/{t['name']}"]
    with ThreadPoolExecutor(o.jobs) as ex:
        rows = list(ex.map(one, items))
    with open(out / "results.tsv", "w") as f:
        for r in rows:
            f.write("\t".join(map(str, r)) + "\n")
    from collections import Counter
    print(Counter(r[3] for r in rows))
    causes = Counter(re.sub(r"`[^`]*`|:[\w.:()]+", "`X`", r[6]) for r in rows if r[3] != "PASS")
    for k, v in causes.most_common():
        print(f"{v:5} {k}")
    for suite in sorted({r[1] for r in rows}):
        c = Counter(r[3] for r in rows if r[1] == suite)
        print(f"{suite:32} " + " ".join(f"{k}={v}" for k, v in sorted(c.items())))
    sys.exit(0 if all(r[3] == "PASS" for r in rows) else 1)


if __name__ == "__main__":
    main()
