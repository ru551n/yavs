#!/usr/bin/env python3
"""Run every tests/discrepancies case on yavs and on configured reference simulators.

Usage:
  run.py [--sims sim1,sim2,yavs] [--filter SUBSTR] [--check] [--record] [--bless-yavs]
         [--no-sandbox] [-v] [--print-config]

Simulator labels (sim1, sim2, ...) map to commands through the untracked file
tools/sims.local.conf (key `discrep` in the label's section, see tools/simconf.py) or the
environment variables DISCREP_SIM1, DISCREP_SIM2, ... The value is a bash pair
`analyse|||elaborate-and-run` with an optional third part, a regex for the reference's internal
error banner. The label to tool mapping is never committed.
yavs is built in: binary from YAVS_BIN or `yavs` on PATH.

Case layout: tests/discrepancies/<id>/{case.vhd, case.conf, expected-yavs, verdicts}.
case.conf: `top = NAME`, `std = 2008 2019`, `mode = run|analyze`.
Output of a run is the list of `report`/`assert` messages (`severity: text`), in order.
expected-yavs: per std a section `== 2008` followed by the expected message lines, or the single
word REJECT. Verdicts per simulator: REJECT (analysis or elaboration error), CRASH, TIMEOUT,
FATAL (nonzero exit at run time), OK (same messages as yavs's expected output), OK* (other
messages: see -v). `verdicts` holds `label std verdict` lines recorded earlier; --check flags drift.
"""
import argparse, os, re, subprocess, sys, shutil
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
CASES = os.path.normpath(os.path.join(HERE, "..", "..", "tests", "discrepancies"))
sys.path.insert(0, os.path.join(HERE, ".."))
import simconf  # noqa: E402

CACHE = os.path.expanduser(os.environ.get("DISCREP_CACHE", "~/.cache/yavs-discrep"))
# the shared std/ieee store (#564) is private to this tool, never the user's
os.environ["YAVS_STD_CACHE"] = os.path.join(CACHE, "std")

# (analyse, elab_and_run) bash snippets; placeholders {file} {top} {std} {work} {res} (the resolution limit of the case, default fs). Exit 97 is an
# analysis error, 98 an elaboration error.
TEMPLATES = {
    "yavs": ('"${YAVS_BIN:-yavs}" analyze --std={std} --work=work:{work} {file}',
             '"${YAVS_BIN:-yavs}" elab --std={std} --work=work:{work} {top} || exit 98; '
             '"${YAVS_BIN:-yavs}" run --std={std} --work=work:{work} --resolution={res} {top}; '
             'rc=$?; [ $rc = 2 ] && exit 98; exit $rc'),
}
MSG = [  # report/assert lines of the known output formats
    re.compile(r"^\*\* (Note|Warning|Error|Failure): (?:\d+\w*\+\d+: )?(?:\S+:\d+ )?(.*)$"),  # yavs-style
    re.compile(r"\((?:report|assertion) (note|warning|error|failure)\): (.*)$"),
    re.compile(r"^# \*\* (Note|Warning|Error|Failure): (.*)$"),
    re.compile(r"^# EXECUTION:: (NOTE|WARNING|ERROR|FAILURE)\s*: (.*)$"),
]
CRASH = re.compile(r"internal error|Segmentation|Caught signal|stack dump|PANIC|Assertion failed|GNAT BUG|thread .* panicked", re.I)


def load_sims(names):
    """Return the command pairs by label and the extra crash pattern (or None) by label.
    A reference that reports internal errors with its own banner and a nonzero exit gives the
    banner regex as third part; otherwise they would be read as ordinary failures."""
    sims, extra = {"yavs": TEMPLATES["yavs"]}, {}
    for n in names:
        if n == "yavs":
            continue
        v = os.environ.get("DISCREP_" + n.upper(), simconf.get(n, "discrep"))
        if v is None:
            print(f"{n}: not configured (see --print-config), skipped", file=sys.stderr)
            continue
        parts = v.split("|||")
        if len(parts) < 2:
            sys.exit(f"{n}: the discrep value needs 'analyse|||elaborate-and-run'")
        sims[n] = (parts[0], parts[1])
        extra[n] = re.compile(parts[2]) if len(parts) > 2 and parts[2] else None
    return sims, extra


def parse_conf(path):
    c = {"top": "top", "std": "2008", "mode": "run", "res": "fs"}
    for line in open(path):
        if "=" in line:
            k, v = line.split("=", 1)
            c[k.strip()] = v.strip()
    c["std"] = c["std"].split()
    return c


def messages(text):
    out = []
    for line in text.splitlines():
        for rx in MSG:
            m = rx.search(line.rstrip())
            if m and not re.search(r"\([a-z]{3,4}-\d+\)", line):
                out.append(f"{m.group(1).lower()}: {m.group(2).strip()}")
                break
    return out


def run_one(sim, tpl, case, conf, std, sandbox, extra_crash=None):
    work = os.path.join(CACHE, str(os.getpid()), sim, os.path.basename(case), std)
    os.makedirs(os.path.dirname(work), exist_ok=True)
    f = os.path.join(case, "case.vhd")
    sub = dict(file=f, top=conf["top"], std=std, std2=std[2:], work=work, res=conf["res"])

    def sh(snippet):
        for k, v in sub.items():
            snippet = snippet.replace("{" + k + "}", v)
        cmd = ["bash", "-c", snippet]
        if sandbox:
            cmd = ["timeout", "300", "systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=2G"] + cmd
        p = subprocess.run(cmd, capture_output=True, text=True, errors="replace", cwd=os.path.dirname(work))
        return p.returncode, p.stdout + "\n" + p.stderr

    def crashed(out):
        return CRASH.search(out) or (extra_crash and extra_crash.search(out))

    rc, out = sh(tpl[0])
    if crashed(out) and rc:
        return "CRASH", []
    if rc:
        return ("TIMEOUT" if rc == 124 else "REJECT"), []
    if conf["mode"] == "analyze":
        return "OK", []
    rc, out = sh(tpl[1])
    if rc == 98:
        return "REJECT", []
    if rc == 124:
        return "TIMEOUT", []
    if crashed(out) or rc in (139, 134):
        return "CRASH", []
    if rc:
        return "FATAL", messages(out)
    return "OK", messages(out)


def read_sections(path):
    secs, cur = {}, None
    if not os.path.exists(path):
        return secs
    for line in open(path):
        line = line.rstrip("\n")
        if line.startswith("== "):
            cur = line[3:].strip()
            secs[cur] = []
        elif cur and line:
            secs[cur].append(line)
    return secs


def read_verdicts(path):
    d = {}
    if os.path.exists(path):
        for line in open(path):
            p = line.split()
            if len(p) == 3:
                d[(p[0], p[1])] = p[2]
    return d


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--sims", default="sim1,sim2,sim3,sim4")
    ap.add_argument("--filter", default="")
    ap.add_argument("--check", action="store_true", help="exit 1 if yavs differs from expected-yavs or a verdict drifts")
    ap.add_argument("--record", action="store_true", help="write the observed verdicts of the given sims to verdicts")
    ap.add_argument("--bless-yavs", action="store_true", help="write expected-yavs from yavs's output")
    ap.add_argument("--no-sandbox", action="store_true")
    ap.add_argument("--jobs", type=int, default=1, help="parallel runs, at most 2")
    ap.add_argument("--print-config", action="store_true")
    ap.add_argument("-v", action="store_true")
    a = ap.parse_args()
    if a.print_config:
        print("# tools/sims.local.conf (untracked), one section per label:\n[sim1]\n"
              "discrep = <analyse bash>|||<elaborate-and-run bash>[|||<crash regex>]\n"
              "# placeholders {file} {top} {std} {std2} {work} {res}; exit 97 = analysis error, 98 = elaboration error\n"
              "# or DISCREP_SIM1=<same value> in the environment")
        return 0
    sims, extra = load_sims([s for s in a.sims.split(",") if s])
    sims.setdefault("yavs", TEMPLATES["yavs"])
    cases = sorted(d for d in os.listdir(CASES) if os.path.exists(os.path.join(CASES, d, "case.conf")) and a.filter in d)
    labels = ["yavs"] + [s for s in sims if s != "yavs"]
    jobs = [(c, std) for c in cases for std in parse_conf(os.path.join(CASES, c, "case.conf"))["std"]]
    sandbox = not a.no_sandbox
    res = {}

    def work(job, sim):
        c, std = job
        d = os.path.join(CASES, c)
        return (c, std, sim), run_one(sim, sims[sim], d, parse_conf(os.path.join(d, "case.conf")), std, sandbox,
                                      extra.get(sim))

    with ThreadPoolExecutor(max(1, min(a.jobs, 2))) as ex:
        futs = [ex.submit(work, j, s) for j in jobs for s in labels]
        for f in futs:
            k, v = f.result()
            res[k] = v
    bad = []
    rows = []
    newverd = {}
    for c in cases:
        d = os.path.join(CASES, c)
        exp = read_sections(os.path.join(d, "expected-yavs"))
        old = read_verdicts(os.path.join(d, "verdicts"))
        cells = []
        for std in parse_conf(os.path.join(d, "case.conf"))["std"]:
            yv, ym = res[(c, std, "yavs")]
            got = ["REJECT"] if yv == "REJECT" else (ym if yv == "OK" else [yv])
            if a.bless_yavs:
                newverd.setdefault(c, {})[std] = got
            elif exp.get(std) != got:
                bad.append(f"{c} {std}: yavs {got} != expected {exp.get(std)}")
            row = [f"{c}", std, "yavs:" + ("ok" if exp.get(std) == got else "DIFF")]
            for s in labels[1:]:
                v, m = res[(c, std, s)]
                if v == "OK" and exp.get(std) != ["REJECT"] and sorted(m) != sorted(exp.get(std, [])):
                    v = "OK*"
                    if a.v:
                        print(f"-- {c} {std} {s} messages: {m}", file=sys.stderr)
                if exp.get(std) == ["REJECT"] and v == "OK" and False:
                    pass
                prev = old.get((s, std))
                if prev and prev != v:
                    bad.append(f"{c} {std} {s}: recorded {prev}, now {v}")
                    v += "!"
                row.append(f"{s}:{v}")
                newverd.setdefault(("v", c), []).append(f"{s} {std} {v.rstrip('!')}")
            cells.append(row)
        rows += cells
    for r in rows:
        print("  ".join(r))
    if a.bless_yavs:
        for c, secs in ((k, v) for k, v in newverd.items() if isinstance(k, str)):
            with open(os.path.join(CASES, c, "expected-yavs"), "w") as fh:
                for std, lines in secs.items():
                    fh.write(f"== {std}\n" + "".join(l + "\n" for l in lines))
    if a.record:
        for k, lines in newverd.items():
            if isinstance(k, tuple):
                p = os.path.join(CASES, k[1], "verdicts")
                keep = [l for l in (open(p).read().splitlines() if os.path.exists(p) else [])
                        if l.split()[0] not in {x.split()[0] for x in lines}]
                open(p, "w").write("".join(l + "\n" for l in keep + lines))
    shutil.rmtree(os.path.join(CACHE, str(os.getpid())), ignore_errors=True)
    for b in bad:
        print("CHECK:", b, file=sys.stderr)
    return 1 if (bad and a.check) else 0


if __name__ == "__main__":
    sys.exit(main())
