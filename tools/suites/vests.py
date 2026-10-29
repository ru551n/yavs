#!/usr/bin/env python3
"""Run the VESTs VHDL-1993 suite (the `vests` corpus) on yavs at --std=93, judged by the suite's own lists.

  tools/suites/vests.py [--root DIR] [--out DIR] [--yavs BIN] [-j N] [--only GROUP,...] [--no-wrap]

The test lists are the suite's own `.exp` files (clifton-labs: every .vhd/.vhdl below it, as its list file loops);
what each command expects:
  run_compliant_test F      analyse F and run its top (its last configuration, else its last entity that no other
                            unit of the file instantiates) with --exit-severity=failure. Pass: no FAILED verdict line
                            ("FAILED TEST", "TEST FAILED"), the PASSED line when the file has one, and exit 0, or
                            exit 1 from a report whose message is the pass line (some tests end the run that way)
  build_compliant_test F    analyse F into the current library (LIBRARY=lib, else work); create_lib and delete_lib
                            are followed in order, so the group runs sequentially. Pass: analysis succeeds
  run_non_compliant_test F  analyse and run as above. Pass: some step fails. In `analyzer_failure` the suite expects
                            the analysis to fail; the tests in LATE also pass when a yavs error at elaboration or run
                            time stops them before their verdict line: each was checked to be the error the test is
                            about (values out of range, lengths, delays, non-static actuals), whose detection time the
                            language reference leaves open; the report gives the strict count of analysis failures too.
                            In every must-fail group a first error E1999 (not analysed yet), E2001 (not lowered yet), E9999 or a crash is no
                            rejection: the test fails
The billowitch `disputed` directory has no list and is not run. Tests that read or write the shared
`iofile.NN` files run one after the other in one directory seeded with the suite's copies, in list order.
OVERRIDES holds the tests whose expectation is wrong or cannot be judged, each with its reason; ISSUES the yavs
issue of each failing test. Every yavs call runs under `timeout 120 systemd-run --user --scope -p MemoryMax=2G`
(--no-wrap drops the systemd-run scope, not the timeout). Results: DIR/results.tsv (group, test, PASS|FAIL|SKIP, detail, issue); a table of pass,
total (without SKIP) and skip per group is printed. Exit 0 always (a measurement).
"""
import argparse, concurrent.futures as cf, os, re, shutil, subprocess
from pathlib import Path

GROUPS = {  # name: (directory under vhdl-93, list file, kind)
    "billowitch/compliant": ("billowitch/compliant", "compliant.exp", "run"),
    "billowitch/analyzer_failure": ("billowitch/non_compliant/analyzer_failure", "non_compliant.exp", "analysis-fails"),
    "billowitch/simulator_failure": ("billowitch/non_compliant/simulator_failure", "non_compliant.exp", "fails"),
    "ashenden/compliant": ("ashenden/compliant", "compliant.exp", "build"),
    "ashenden/non_compliant": ("ashenden/non_compliant", "non_compliant.exp", "fails"),
    "clifton-labs/compliant": ("clifton-labs/compliant", "compliant.exp", "run"),
}
# Tests whose own expectation the language reference contradicts while yavs's result is right (PASS), and tests that
# cannot be judged (SKIP, not counted): files that are not legal VHDL-1993, files every tool rejects under the suite's
# own build list, results the reference leaves to the implementation. Section numbers are of IEEE 1076-1993.
_ILLEGAL = "not counted: the file is not legal VHDL-1993 (yavs rejects it, as it must): "
_BROKEN = "not counted: every tool rejects it under the suite's own build list: "
_COMMENTED = "the illegal lines of the book's example are commented out; what is left is legal"
_FILEFMT = "not counted: reads a file of a non-TEXT type, whose format is implementation-defined (3.4.1); the input is text"
OVERRIDES = {
    "tc217.vhd": ("PASS", "its PASSED condition uses the 1987 CHARACTER'RIGHT (127); CHARACTER has 256 values (14.2), so only the FAILED assertion decides and it stays silent"),
    "tc2572.vhd": ("PASS", "the range of REAL is implementation-defined with a minimum (3.1.4); the 64-bit REAL holds 2.12E80"),
    "ch_03_ch_03_09.vhd": ("PASS", _COMMENTED),
    "ch_04_ch_04_03.vhd": ("PASS", _COMMENTED),
    "ch_04_ch_04_09.vhd": ("PASS", _COMMENTED),
    "functional/attributes/signal/simple-event-attribute.vhdl": ("PASS", "the test expects an event at 36 ns, but the signal changes at 10, 20, 30 and 40 ns (14.1 'EVENT)"),
    "ch_18_ch_18_07.vhd": ("SKIP", _ILLEGAL + "a formal of a file type must be of class file (2.1.1), the file uses a variable"),
    "ch_16_fg_16_06.vhd": ("SKIP", _ILLEGAL + "a formal signal parameter declared with bus (2008 4.2.2.3, #808)"),
    "functional/functions/integer-type-overload.vhdl": ("SKIP", _ILLEGAL + "the initial value 3 is outside integer_t1 (4.3.1.3)"),
    "functional/generics/entity-generic-defines-port-type.vhdl": ("SKIP", _ILLEGAL + "an entity of library work named without the prefix work. is not visible (10.4, 11.2)"),
    "functional/std_logic_1164/test_std_logic_type.vhdl": ("SKIP", _ILLEGAL + "'LEFT of a scalar object is not a 1993 attribute (14.1)"),
    "functional/textio/write/record_write.vhd": ("SKIP", _ILLEGAL + "STRING(0 to 10) has the bound 0 outside POSITIVE (3.2.1.1)"),
    "functional/variable/integer-variable-persist-across-activations.vhdl": ("SKIP", _ILLEGAL + "a process with a sensitivity list contains a wait statement (9.2)"),
    **{t: ("SKIP", _ILLEGAL + "a deferred constant is allowed in a package declaration only (4.3.1.1)") for t in
       ("functional/subtypes/modified-character-subtype.vhdl", "functional/subtypes/modified-integer-subtype.vhdl", "functional/subtypes/simple-enumeration-subtype.vhdl")},
    **{t: ("SKIP", _ILLEGAL + "the bounds of a type definition must be locally static (3.1.2)") for t in
       ("functional/types/array-types/subprogram-dynamic-type.vhdl", "functional/types/integer-types/subprogram-dynamic-type.vhdl")},
    **{t: ("SKIP", _BROKEN + "undeclared names or ports, or syntax errors") for t in
       ("ch_04_tb_04_01.vhd", "ch_13_fg_13_20.vhd", "ch_17_ch_17_06.vhd", "ch_18_fg_18_09.vhd", "ch_19_srvr.vhd")},
    "ch_17_fg_17_14.vhd": ("SKIP", "not counted: every tool stops at a unit the suite never builds for it (stimulus_types), before the error the test is about"),
    "ch_19_random-b.vhd": ("SKIP", _BROKEN + "a pre-standard math package (library math)"),
    "ch_13_ch_13_01.vhd": ("SKIP", _BROKEN + "an architecture named before it is analysed, and a port binding one reference rejects"),
    **{t: ("SKIP", _BROKEN + "it uses a unit that the list builds into another library, never builds, or builds from a broken file") for t in
       ("ap_a_ap_a_03.vhd", "ch_13_fg_13_05.vhd", "ch_13_fg_13_07.vhd", "ch_13_fg_13_08.vhd", "ch_13_fg_13_09.vhd", "ch_13_fg_13_22.vhd", "ch_14_fg_14_11.vhd",
        "ch_14_fg_14_12.vhd", "ch_15_dlxr.vhd", "ch_15_dlxstsv.vhd", "ch_15_dlxtstr.vhd", "ch_19_ds-qn.vhd", "ch_19_srvr-b.vhd", "ch_19_tb-frk.vhd",
        "ch_19_tb-jnsth.vhd", "ch_19_tb-jn.vhd", "ch_19_tb-qs.vhd", "ch_19_tb-snk.vhd", "ch_19_tb-src.vhd", "ch_20_ch_20_01.vhd", "ch_20_ch_20_02.vhd")},
    **{t: ("SKIP", _FILEFMT) for t in
       ("functional/textio/read/character_array_read.vhdl", "functional/textio/read/character_read.vhdl", "functional/textio/read/simple-read.vhdl")},
}

# analyzer_failure tests that yavs rejects at elaboration or run time with the error the test is about (checked one by
# one); they count as passes because the language reference leaves open when such value errors are detected.
LATE = set("tc310 tc342 tc738 tc764 tc1068 tc1124 tc1127 tc1136 tc1141 tc1142 tc1143 tc1144 tc1146 tc1217 tc1333 tc1334 tc1345 tc1346 "
           "tc1416 tc1726 tc2255 tc2523 tc2536 tc2548".split())

# Failing tests by the yavs issue that tracks their cause (the report groups failures by these).
ISSUES = {
    816: "tc1781", 817: "tc3153 tc3156 tc3159",
    819: "ch_11_fg_11_09 ch_11_fg_11_10",
    820: "ch_13_fg_13_24",
}
ISSUE_OF = {t: n for n, ts in ISSUES.items() for t in ts.split()}


class Ctx:
    def __init__(self, a):
        self.a = a

    def call(self, args, cwd, log):
        cmd = [self.a.yavs, *args]
        cmd = ["timeout", "120"] + ([] if self.a.no_wrap else ["systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=2G"]) + cmd
        r = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, errors="replace")
        out = r.stdout + r.stderr
        with open(log, "a") as f:
            f.write(f"$ {' '.join(map(str, cmd))}\n{out}[exit {r.returncode}]\n")
        return r.returncode, out


def top_of(src):
    """The last configuration of the file, else its last entity that no other unit of the file instantiates."""
    text = re.sub(r"--.*", "", src.read_text(errors="replace"))
    cfgs = re.findall(r"^\s*configuration\s+(\w+)\s+of\b", text, re.I | re.M)
    if cfgs:
        return cfgs[-1]
    ents = re.findall(r"^\s*entity\s+(\w+)\s+is\b", text, re.I | re.M)
    used = lambda e: re.search(rf"^(?!\s*(entity|end|architecture)\b).*\b{e}\b", text, re.I | re.M)
    return next((e for e in reversed(ents) if not used(e)), ents[-1] if ents else None)


def first_error(out):
    out = re.sub(r"\x1b\[[0-9;]*m", "", out)
    return next((l.strip() for l in out.splitlines() if re.match(r"\s*(error|internal error|thread|\*\*)", l) or "FAILED" in l), out.strip()[-200:]).replace("\t", " ")[:200]


def simulate(c, f, cwd, libs=(), work="work"):
    """Analyse and run f in cwd; return (step that failed or None, output). The step is analysis, run or verdict."""
    log = cwd / "log.txt"
    maps = [f"--map={l}:{cwd / l}" for l in libs if l != work]
    rc, out = c.call(["analyze", "--std=93", f"--work={work}:{cwd / work}", *maps, f], cwd, log)
    if rc:
        return "analysis", out
    top = top_of(f)
    rc, out = c.call(["run", "--std=93", f"--work={work}:{cwd / work}", *maps, "--exit-severity=failure", top], cwd, log)
    if rc == 124:
        return "timeout", out
    if re.search(r"FAILED TEST|TEST FAILED", out):
        return "verdict", out
    if rc and not ("PASSED" in out and not re.search(r"^error\[|panicked|overflowed", out, re.M)):  # a pass line at severity failure ends a run too
        return "run", out
    if re.search(r"PASSED", f.read_text(errors="replace")) and "PASSED" not in out:
        return "verdict", "no PASSED line\n" + out
    return None, out


NOT_A_REJECTION = r"^error\[E(1999|2001|9999)\]|panicked|overflowed"  # first error: not implemented, internal error, crash


def judge(kind, step, out, test):
    why = f"{step}: {first_error(out)}" if step else ""
    if kind == "run":
        return ("FAIL", why) if step else ("PASS", "")
    if step and re.search(NOT_A_REJECTION, first_error(out)):
        return ("FAIL", f"no real rejection, {why}")
    if kind == "analysis-fails":
        if step == "analysis" or (step == "run" and Path(test).stem in LATE):
            return ("PASS", why if step == "analysis" else f"rejected at run: {first_error(out)}")
        return ("FAIL", "analysis accepted" + (f", {why}" if step else ""))
    return ("PASS", why) if step else ("FAIL", "every step passed")


def lines(path):
    for l in path.read_text(errors="replace").splitlines():
        if m := re.match(r"\s*(run_compliant_test|run_non_compliant_test|build_compliant_test|create_lib|delete_lib)\s+(\S+)(.*)", l):
            yield m[1], m[2], m[3]


def tests_of(root, group):
    d, exp, kind = GROUPS[group]
    d = root / d
    if group == "clifton-labs/compliant":  # its list file is a loop over every .vhd/.vhdl below it
        return [("run_compliant_test", str(p.relative_to(d)), "") for p in sorted(d.rglob("*.vhd*"))]
    return list(lines(d / exp))


def run_group(c, root, group, out):
    d, _, kind = GROUPS[group]
    d = root / d
    gdir = out / group.replace("/", "-")
    shutil.rmtree(gdir, ignore_errors=True)
    gdir.mkdir(parents=True)
    tests = tests_of(root, group)
    rows = []
    if kind == "build":  # sequential: libraries persist between tests until delete_lib
        libs = {"work"}
        for n, (cmd, arg, rest) in enumerate(tests):
            if cmd == "create_lib":
                libs.add(arg.lower())
            elif cmd == "delete_lib":
                shutil.rmtree(gdir / arg.lower(), ignore_errors=True)
            else:
                m = re.search(r"LIBRARY=(\w+)", rest)
                lib = (m[1] if m else "work").lower()
                libs.add(lib)
                rc, o = c.call(["analyze", "--std=93", f"--work={lib}:{gdir / lib}", *[f"--map={l}:{gdir / l}" for l in sorted(libs) if l != lib], d / arg], gdir, gdir / "log.txt")
                rows.append((group, arg, *(("FAIL", f"analysis: {first_error(o)}") if rc else ("PASS", ""))))
        return rows

    def one(n, t, cwd=None):
        cwd = cwd or gdir / str(n)
        cwd.mkdir(exist_ok=True)
        f = d / t
        for inp in f.parent.glob(f.name.rsplit(".", 1)[0] + "*.in"):  # clifton-labs input files (INPUT=)
            shutil.copy(inp, cwd)
        step, o = simulate(c, f, cwd)
        return (group, t, *judge(kind, step, o, t))

    names = [t for cmd, t, _ in tests if cmd.startswith("run_")]
    io = [t for t in names if "iofile" in (d / t).read_text(errors="replace")]
    with cf.ThreadPoolExecutor(c.a.jobs) as ex:
        futs = {t: ex.submit(one, n, t) for n, t in enumerate(names) if t not in io}
        done = {}
        if io:  # one after the other in one seeded directory, list order
            shutil.copytree(d / "iofiles", gdir / "io")
            done = {t: one(0, t, gdir / "io") for t in io}
        return [done[t] if t in done else futs[t].result() for t in names]


def apply_overrides(rows):
    return [(g, t, *((o[0], f"override: {o[1]}") if (o := OVERRIDES.get(t)) and (s == "FAIL" or o[0] == "SKIP") else (s, w))) for g, t, s, w in rows]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--root", default=os.path.join(os.environ.get("YAVS_CORPUS") or os.path.expanduser("~/.cache/rsim-corpus"), "vests"))
    ap.add_argument("--out", default="~/.cache/yavs-vests-suite")
    ap.add_argument("--yavs", default="yavs")
    ap.add_argument("-j", "--jobs", type=int, default=4)
    ap.add_argument("--only", help="comma list of groups: " + ", ".join(GROUPS))
    ap.add_argument("--no-wrap", action="store_true", help="no timeout/systemd-run around yavs (CI)")
    a = ap.parse_args()
    a.out = Path(a.out).expanduser().resolve()
    a.yavs = str(Path(a.yavs).resolve()) if "/" in a.yavs else a.yavs
    root = Path(a.root).expanduser() / "vhdl-93"
    if not root.is_dir():
        raise SystemExit(f"{root} is missing: run tools/fetch-corpus.sh (or set YAVS_CORPUS)")
    c = Ctx(a)
    rows = []
    for g in (a.only.split(",") if a.only else GROUPS):
        rows += run_group(c, root, g, a.out)
    rows = [(*r, str(ISSUE_OF.get(Path(r[1]).stem, "")) if r[2] == "FAIL" else "") for r in apply_overrides(rows)]
    (a.out / "results.tsv").write_text("".join("\t".join(r) + "\n" for r in rows))
    print(f"{'group':30} pass total skip")
    for g in [*dict.fromkeys(r[0] for r in rows), None]:
        k = [r[2] for r in rows if g in (None, r[0])]
        print(f"{g or 'total':30} {k.count('PASS'):4} {len(k) - k.count('SKIP'):5} {k.count('SKIP'):4}")


if __name__ == "__main__":
    main()
