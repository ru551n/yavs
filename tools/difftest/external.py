#!/usr/bin/env python3
"""Run the reference simulators' own self-checking regression tests on yavs (run time, not
analysis). The tests are read in place from the corpus cache (tools/fetch-corpus.sh), never
copied into the repo. A suite is named by the label of its simulator: sim1 (a test list,
test/regress/testlist.txt) and sim2 (testsuite/gna/*/testsuite.sh, simple scripts only). See README.md "External regression tests". The reference
commands come from tools/sims.local.conf (keys ext_analyse, ext_elab, ext_run, see tools/AGENTS.md);
--oracle needs none.

    external.py <suite>                     # differential: reference simulator and yavs, classify
    external.py <suite> --filter 'issue1'   # a subset
    external.py <suite> --oracle --check tools/difftest/external-<suite>.pass   # CI: no reference needed
"""
import argparse, concurrent.futures as cf, json, os, re, shlex, shutil, subprocess, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import difftest as dt  # noqa: E402
import simconf  # noqa: E402

CORPUS = Path(os.environ.get("YAVS_CORPUS") or Path.home() / ".cache" / "rsim-corpus")
CACHE = Path(os.environ.get("DIFFTEST_CACHE") or Path.home() / ".cache" / "yavs-difftest" / f"ext-{os.getpid()}")
# the shared std/ieee store (#564) is private to this tool, never the user's
os.environ["YAVS_STD_CACHE"] = str(CACHE / "std")


class Skip(Exception):
    """A test the harness does not model (shell scripts, VHPI, waves, mixed language, ...)."""


# A test: {name, dir (directory copied as the working directory, or None), std, relaxed, psl,
# gold, env, steps}. Steps are ("a", [files], [script arguments]), ("clean",) and
# ("run", top, [run options], expect_fail).
def list_tests():
    base = CORPUS / "nvc" / "test" / "regress"
    for line in (base / "testlist.txt").read_text().splitlines():
        line = line.split("#")[0].split()
        if not line:
            continue
        name, opts = line[0], line[1].split(",")
        t = {"name": name, "dir": None, "std": "1993", "relaxed": False, "psl": False, "gold": None, "env": {}}
        run, fail = [], False
        try:
            for o in opts:
                k, _, v = o.partition("=")
                if o in ("normal", "slow", "!windows", "!freebsd", "wave", "gtkw", "dump-arrays") or k in ("O", "H", "dump-arrays") or re.fullmatch(r"O\d", o):
                    pass  # waves and optimisation levels do not change the outcome
                elif o in ("2000", "2002", "2008", "2019"):
                    t["std"] = o
                elif o == "gold":
                    t["gold"] = base / "gold" / f"{name}.txt"
                elif o == "fail":
                    fail = True
                elif k == "stop":
                    run.append(f"--stop-time={v}")
                elif k == "exit":
                    run.append(f"--exit-severity={v}")
                elif o == "relaxed":
                    t["relaxed"] = True
                elif o == "psl":
                    t["psl"] = True
                elif o.startswith("g") and v and "." not in k:  # hierarchical generics are a reference option
                    run.append(f"-{k}={v}")
                elif o.startswith("$") and v:
                    t["env"][k[1:]] = v
                else:
                    raise Skip(o)  # shell, tcl, vhpi, verilog, mixed, cover, seed, relax=, work=, define, shuffle, plusargs
        except Skip as e:
            yield {"name": name, "skip": str(e)}
            continue
        t["steps"] = [("a", [str(base / f"{name}.vhd")], []), ("run", name, run, fail)]
        yield t


STD_FLAGS = {"--std=87": None, "--std=93": "1993", "--std=93c": "1993", "--std=00": "2000",
            "--std=02": "2002", "--std=08": "2008", "--std=19": "2019"}


def script_flags(t, toks):
    for f in toks:
        if f in STD_FLAGS:
            if STD_FLAGS[f] is None:
                raise Skip(f)
            t["std"] = STD_FLAGS[f]
            if f == "--std=93c":  # the relaxed 1993 mode of the second suite
                t["relaxed"] = True
        elif f in ("-frelaxed", "-frelaxed-rules"):
            t["relaxed"] = True
        elif f == "-fpsl":
            t["psl"] = True
        else:
            raise Skip(f)


def script_dir_test(d):
    t = {"name": d.name, "dir": d, "std": "1993", "relaxed": False, "psl": False, "gold": None, "env": {}, "steps": []}
    script = d / "testsuite.sh"
    if not script.exists():
        raise Skip("no testsuite.sh")
    std_flags, flags = [], []
    for raw in script.read_text().splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or line in (". ../../testenv.sh", "clean", "exit 0") or line.startswith("echo "):
            if line == "clean":
                t["steps"].append(("clean",))
            continue
        try:
            w = shlex.split(line)
        except ValueError:
            raise Skip("unparsed line")
        if w[0] == "export" and len(w) == 2:
            w = w[1:]
        if len(w) == 1 and w[0].startswith(("GHDL_STD_FLAGS=", "GHDL_FLAGS=")):
            k, _, v = w[0].partition("=")
            (std_flags if k == "GHDL_STD_FLAGS" else flags)[:] = v.split()
        elif w[0] == "analyze_failure":
            pass  # negative analysis tests belong to the analysis corpora; a failed analysis changes no library
        elif w[0] == "analyze":
            files = [f for f in w[1:] if not f.startswith("-")]
            script_flags(t, std_flags + flags + [f for f in w[1:] if f.startswith("-")])
            t["steps"].append(("a", files, w[1:]))
        elif w[0] in ("elab_simulate", "elab_simulate_failure") and len(w) >= 2:
            script_flags(t, std_flags + flags)
            opts = w[2:]
            if any(not (o.startswith("--stop-time=") or (o.startswith("-g") and "=" in o)) for o in opts):
                raise Skip(" ".join(opts))
            t["steps"].append(("run", w[1], opts, w[0] == "elab_simulate_failure"))
        else:
            raise Skip(w[0])
    if not any(s[0] == "run" for s in t["steps"]):
        raise Skip("no simulation")
    t["std_flags"] = std_flags + flags
    return t


def script_dir_tests():
    for d in sorted((CORPUS / "ghdl" / "testsuite" / "gna").iterdir()):
        if d.is_dir():
            try:
                yield script_dir_test(d)
            except Skip as e:
                yield {"name": d.name, "skip": str(e)}


SUITES = {"sim1": list_tests, "sim2": script_dir_tests}  # a suite is named by the label of its simulator


# ---------------------------------------------------------------- scripts
def refcmd(sim, key, t, required=True, **sub):
    """A reference command of tools/sims.local.conf for this test, placeholders filled."""
    lab = sim
    tpl = simconf.get(lab, key)
    if tpl is None:
        if not required:
            return ""
        sys.exit(f"{lab}: key {key} not configured (tools/sims.local.conf)")
    rel = simconf.get(lab, "difftest_relaxed", "--relaxed")
    sub = dict(sub, std=t["std"], relaxed=rel + " " if t["relaxed"] else "",
               psl=simconf.get(lab, "ext_psl", "") + " " if t["psl"] else "",
               std_flags=" ".join(shlex.quote(x) for x in t.get("std_flags", [])))
    for k, v in sub.items():
        tpl = tpl.replace("{" + k + "}", v)
    return tpl


def script(sim, t):
    """bash script running the test's steps: analysis failure exits 97, elaboration 98; run i
    writes r<i>.out, r<i>.err and r<i>.rc and the script goes on (expected failures)."""
    q = shlex.quote
    std, rel = t["std"], t["relaxed"]
    if sim == "yavs" and std < "2008":
        # yavs runs 2008 and 2019 only; code of an earlier standard is read as 2008, relaxed
        # for what only the earlier standards allow (non-protected shared variables).
        std, rel = "2008", True
    lines, i = ["set -u", "mkdir -p lib"], 0
    yavs = '"${YAVS_BIN:-yavs}"'
    for s in t["steps"]:
        if s[0] == "clean":
            lines.append("rm -rf lib && mkdir lib")
        elif s[0] == "a":
            files = " ".join(q(f) if not any(c in f for c in "*?[") else f for f in s[1])
            if sim != "yavs":
                cmd = refcmd(sim, "ext_analyse", t, files=files, args=" ".join(map(q, s[2])))
            else:
                cmd = f"{yavs} analyze --std={std} --work=work:lib {'--relaxed ' if rel else ''}{'--psl ' if t['psl'] else ''}{files}"
            lines.append(f"o=$({cmd} 2>&1) || {{ printf '%s\\n' \"$o\" >&2; exit 97; }}")
        else:
            _, top, opts, _ = s
            o = " ".join(map(q, opts))
            if sim != "yavs":
                gens = " ".join(q(x) for x in opts if x.startswith("-g"))
                rest = " ".join(q(x) for x in opts if not x.startswith("-g"))
                elab = refcmd(sim, "ext_elab", t, gens=gens, top=top, required=False)
                if elab:
                    lines.append(f"{elab} >/dev/null 2>e.txt || {{ cat e.txt >&2; exit 98; }}")
                run = refcmd(sim, "ext_run", t, gens=gens, rest=rest, opts=o, top=top)
            else:
                flags = f"--std={std} --work=work:lib {'--relaxed ' if rel else ''}{'--psl ' if t['psl'] else ''}"
                rest = " ".join(q(x) for x in opts if not x.startswith("-g"))
                gens = " ".join(q(x) for x in opts if x.startswith("-g"))
                lines.append(f"{yavs} elab {flags}{gens} {top} >/dev/null 2>e.txt || {{ cat e.txt >&2; exit 98; }}")
                run = f"{yavs} run {flags}{rest} --threads=${{YAVS_THREADS:-1}} ${{YAVS_RUN_FLAGS:-}} {top} {gens}"
            lines.append(f"{run} >r{i}.out 2>r{i}.err; echo $? >r{i}.rc")
            i += 1
    return "\n".join(lines) + "\n"


def execute(sim, t, work, sandbox, threads=1):
    shutil.rmtree(work, ignore_errors=True)
    if t["dir"]:
        shutil.copytree(t["dir"], work)
    else:
        work.mkdir(parents=True)
    (work / "run.sh").write_text(script(sim, t))
    cmd = ["timeout", str(dt.TIMEOUT)]
    if sandbox:
        cmd += ["systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=2G"]
    env = {**os.environ, **t["env"], "YAVS_THREADS": str(threads)}
    p = subprocess.run(cmd + ["bash", "run.sh"], capture_output=True, text=True, errors="replace", cwd=work, env=env)
    runs = []
    for i in range(sum(s[0] == "run" for s in t["steps"])):
        rc = work / f"r{i}.rc"
        if not rc.exists():
            break
        rd = lambda f: (work / f).read_text(errors="replace")
        runs.append((int(rd(f"r{i}.rc").strip() or 124), rd(f"r{i}.out"), rd(f"r{i}.err")))
    return p.returncode, p.stderr, runs


WARN = re.compile(r"^warning(\[\w+\])?: ")


def drop_warnings(err):
    """yavs repeats analysis and elaboration warnings when it runs; the references print theirs
    while analysing, which the harness does not compare."""
    keep, skip = [], False
    for line in err.splitlines():
        skip = bool(WARN.match(line)) or (skip and line.strip() != "")
        if not skip and not (keep and keep[-1] == "" and line == ""):
            keep.append(line)
    return "\n".join(keep)


def errexit_ok(norm):
    """yavs exits 1 after `error` reports like the first reference; the second one exits 0."""
    return norm[0] == "exit\t1" and not any(l.startswith(("diag\tfailure\t", "diag\tfatal\t")) for l in norm)


def meets(t, runs, yavs):
    """The suite's own verdict: every run fails exactly when the test expects it to; for the
    reference simulator the gold lines (first suite) also appear in order in its output (yavs
    prints times in its own units; the differential comparison covers the text)."""
    expect = [s[3] for s in t["steps"] if s[0] == "run"]
    if len(runs) != len(expect):
        return False
    for (rc, out, err), fail in zip(runs, expect):
        failed = rc != 0 and not (t["dir"] and errexit_ok(dt.normalise(rc, out, err, yavs)))
        if failed != fail or rc == 124:
            return False
    if t["gold"] and not yavs:  # gold text is in the reference's own format (time units)
        text = "".join(o + e for _, o, e in runs).splitlines()
        it = iter(text)
        for g in t["gold"].read_text(errors="replace").splitlines():
            if g and not any(g in l for l in it):
                return False
    return True


LEVELS = [("pass", ""), ("order", "order"), ("fatalmsg", "order,fatalmsg"), ("delta-path", "order,fatalmsg,delta,path")]


def fatal_path(lines):
    """`fatalmsg` here also covers the process path of a runtime error (yavs prints it in a
    note), and in a failed run the exit status and its error text (the run fails in both)."""
    if lines and lines[0] != "exit\t0":  # a failed run: its unparsed error text is wording too
        lines = ["exit\t1"] + [l for l in lines[1:] if not l.startswith(("err\t", "diag\tfatal\t"))]
    return [re.sub(r"^(diag\tfatal\t[^\t]*\t[^\t]*\t)[^\t]*", r"\1-", l) for l in lines]


def compare(ref, got, base):
    """Smallest ignore set under which yavs equals the reference, run by run."""
    for cls, extra in LEVELS:
        ign = set(base) | set(extra.split(",")) - {""}
        m = (lambda l: fatal_path(dt.mask(l, ign))) if "fatalmsg" in ign else (lambda l: dt.mask(l, ign))
        if all(m(a) == m(b) for a, b in zip(ref, got)):
            return cls
    return "output"


# Errors the LRM states that yavs reports when it analyses or elaborates, where the reference
# simulators (and so the suites) have the run fail: a test that expects a failing last run
# is met by these (not by an unsupported feature or an internal error).
EARLY_OK = re.compile(r"\[(E1405|E1812|E2103|E2107|E2110|E2111|E2113|E2117)\]")


def one(t, args):
    if "skip" in t:
        return {"name": t["name"], "class": "skip", "msg": t["skip"]}
    res = {"name": t["name"]}
    default_ignore = simconf.get(args.suite, "difftest_ignore", "")
    work = CACHE / args.suite / t["name"]
    if not args.oracle:
        rc, err, runs = execute(args.suite, t, work / "ref", not args.no_sandbox)
        if rc in (97, 98) or rc == 124 or not meets(t, runs, False):
            return {**res, "class": "ref-bad", "msg": f"reference rc={rc} {err[-300:]}"}
        ref = [dt.normalise(*r) for r in runs]
    outs = []
    for th in args.threads:
        rc, err, runs = execute("yavs", t, work / f"yavs-t{th}", not args.no_sandbox, th)
        if rc == 124 or any(r[0] == 124 for r in runs):
            return {**res, "class": "timeout", "msg": ""}
        if rc in (97, 98):
            if EARLY_OK.search(err) and [s[3] for s in t["steps"] if s[0] == "run"][-1:] == [True]:
                return {**res, "class": "pass" if args.oracle else "early", "oracle": True, "msg": err[:2000]}
            return {**res, "class": "analysis" if rc == 97 else "elab", "msg": err[:2000]}
        got = [dt.normalise(rc, o, drop_warnings(e), yavs=True) for rc, o, e in runs]
        if "errexit" in default_ignore:  # exit 1 after `error` reports vs 0 (README "Ignore set", errexit)
            got = [["exit\t0"] + g[1:] if errexit_ok(g) else g for g in got]
        outs.append((runs, got))
    runs, got = outs[0]
    if any(g != outs[0][1] for _, g in outs[1:]):
        return {**res, "class": "threads", "msg": "output differs between thread counts"}
    if args.oracle:
        return {**res, "class": "pass" if meets(t, runs, True) else "oracle", "msg": "\n".join(l for g in got for l in g)[:2000]}
    if [g[0] == "exit\t0" for g in got] != [r[0] == "exit\t0" for r in ref]:
        cls = "outcome"  # one passes a run that the other fails
    else:
        cls = compare(ref, got, default_ignore.split(",") + ["wallclock"])  # the suites print the time of day (std.env tests)
    res["oracle"] = meets(t, runs, True)  # the pass lists also need the suite's own verdict
    msg = "" if cls == "pass" else "\n".join(f"ref: {a}\nyavs: {b}" for r, g in zip(ref, got) for a, b in zip(r + [""] * 99, g) if a != b)[:3000]
    return {**res, "class": cls, "msg": msg}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("suite", choices=SUITES)
    ap.add_argument("--filter", default="", help="regex on the test name")
    ap.add_argument("--jobs", type=int, default=2, help=f"tests at once (max {dt.MAX_JOBS}); each runs one simulator at a time")
    ap.add_argument("--threads", type=lambda s: [1] + [int(x) for x in s.split(",") if x and int(x) != 1], default=[1],
                    help="yavs thread counts, e.g. 4 (runs 1 and 4, outputs must be identical)")
    ap.add_argument("--oracle", action="store_true", help="no reference simulator: judge yavs by the suite's own verdict (exit status)")
    ap.add_argument("--check", type=Path, help="run only the tests in this pass list; fail if any does not pass")
    ap.add_argument("--write-pass", type=Path, help="write the names of the tests that pass (and meet the suite's own verdict) to this file")
    ap.add_argument("--json", type=Path, help="write one JSON result per test")
    ap.add_argument("--no-sandbox", action="store_true", help="skip systemd-run (CI, debugging)")
    ap.add_argument("-v", "--verbose", action="store_true")
    args = ap.parse_args()
    tests = [t for t in SUITES[args.suite]() if re.search(args.filter, t["name"])]
    if args.check:
        want = set(args.check.read_text().split())
        missing = want - {t["name"] for t in tests}
        if missing:
            sys.exit(f"{args.check}: not in the suite: {' '.join(sorted(missing))}")
        tests = [t for t in tests if t["name"] in want]
    counts, results = {}, []
    with cf.ThreadPoolExecutor(max(1, min(args.jobs, dt.MAX_JOBS))) as ex:
        for r in ex.map(lambda t: one(t, args), tests):
            results.append(r)
            counts[r["class"]] = counts.get(r["class"], 0) + 1
            if args.verbose or (r["class"] not in ("pass", "skip", "ref-bad")):
                print(f"{r['class'].upper()} {r['name']}" + ("" if r["class"] in ("pass", "skip") else "\n" + "\n".join("    " + l for l in r["msg"].splitlines()[:20])))
    if args.json:
        args.json.write_text("".join(json.dumps(r) + "\n" for r in results))
    if args.write_pass:
        args.write_pass.write_text("".join(r["name"] + "\n" for r in sorted(results, key=lambda r: r["name"]) if r["class"] in ("pass", "early") and r.get("oracle", True)))
    print(f"{args.suite}: " + ", ".join(f"{k} {v}" for k, v in sorted(counts.items(), key=lambda kv: -kv[1])))
    if args.check and counts.get("pass", 0) != len(tests):
        sys.exit(1)


if __name__ == "__main__":
    main()
