#!/usr/bin/env python3
"""Differential test harness for yavs: run corpus testbenches on a candidate
simulator and diff the normalised output against checked-in NVC reference
outputs (`.expected`). Stdlib only. See README.md."""
import shutil, argparse, zlib, concurrent.futures as cf, difflib, os, re, shlex, subprocess, sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
CORPUS = ROOT / "tests" / "corpus"
# Work directory: unique per process unless DIFFTEST_CACHE is set, so parallel runs never collide.
CACHE = Path(os.environ.get("DIFFTEST_CACHE") or Path.home() / ".cache" / "yavs-difftest" / f"run-{os.getpid()}")
MAX_JOBS = 8
TIMEOUT = 300
RC_ANALYSE, RC_ELAB = 97, 98

# Candidate command templates (run with bash -c). Placeholders: {file} {top}
# {std} {std2} {stop} {stop_flag} {res_flag} {relaxed} {work} {threads} {shuffle}.
# {relaxed} is the candidate's own switch for the `relaxed` header (empty without it).
# Analysis output is shown only when analysis fails: its warnings (those of `relaxed` tests)
# are not simulation output.
NVC = ("o=$(nvc --std={std} --work={work} -a --psl {relaxed} {file} 2>&1) || {{ printf '%s\\n' \"$o\" >&2; exit 97; }}; "
       "nvc --std={std} --work={work} -e {top} || exit 98; "
       "nvc --std={std} --work={work} -r {top} {stop_flag}")
GHDL = ("mkdir -p {work} && o=$(ghdl -a -fpsl --std={std2} --workdir={work} {relaxed} {file} 2>&1) || {{ printf '%s\\n' \"$o\" >&2; exit 97; }}; "
        "ghdl -e -fpsl --std={std2} --workdir={work} {relaxed} {top} || exit 98; "
        "ghdl -r -fpsl --std={std2} --workdir={work} {top} {stop_flag}")
# analyze fails -> 97; `yavs elab` fails (usage, elaboration, "not implemented") -> 98. `yavs run`
# then passes its status through: `stop(2)` exits 2, the same code as an elaboration error.
# --shuffle is a plain switch in yavs, so {shuffle_on} replaces the seeded {shuffle_flag}.
YAVS = ('R=${{YAVS_BIN:-yavs}}; o=$("$R" analyze --std={std} --work=work:{work} {relaxed} {file} 2>&1) || {{ printf \'%s\\n\' "$o" >&2; exit 97; }}; '
        '"$R" elab --std={std} --work=work:{work} {relaxed} {top} || exit 98; '
        '"$R" run --std={std} --work=work:{work} {relaxed} {stop_flag} {res_flag} --threads={threads} {shuffle_on} {top}')
TEMPLATES = {"nvc": NVC, "ghdl": GHDL, "yavs": YAVS}
# yavs repeats analysis warnings when it loads the units: W1904 (a shared variable of a
# non-protected type) is the warning `relaxed` tests are about.
RELAXED = {"nvc": "--relaxed", "ghdl": "-frelaxed", "yavs": "--relaxed -Wno-W1904"}
# Fields masked by default per builtin candidate (reference prints them all).
DEFAULT_IGNORE = {"nvc": "", "ghdl": "delta,path,fatalmsg,order,errexit", "yavs": ""}

UNITS = {"fs": 1, "ps": 10**3, "ns": 10**6, "us": 10**9, "ms": 10**12, "sec": 10**15, "s": 10**15}


def to_fs(num, unit):
    return str(int(num) * UNITS[unit])


def esc(s):
    return s.replace("\\", "\\\\").replace("\n", "\\n").replace("\t", "\\t")


# ---------------------------------------------------------------- headers
HDR = re.compile(r"^--\s*difftest:\s*(.*)$")


def parse_header(path):
    h = {"top": None, "std": "2008", "stop": "", "exit": 0, "ref": "nvc", "ignore": "", "res": "",
         "relaxed": None}
    for line in path.read_text().splitlines()[:15]:
        m = HDR.match(line)
        if m:
            for kv in m.group(1).split():
                k, _, v = kv.partition("=")
                if k not in h:
                    raise SystemExit(f"{path}: unknown header key {k!r}")
                h[k] = int(v) if k == "exit" else v
                if k == "relaxed" and v:
                    raise SystemExit(f"{path}: header key 'relaxed' takes no value")
    if not h["top"]:
        raise SystemExit(f"{path}: missing '-- difftest: top=NAME' header")
    return h


# ---------------------------------------------------------------- running
def run(template, vhd, hdr, work, threads, shuffle, sandbox=True):
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True)
    std = hdr["std"]
    script = template.format(
        file=shlex.quote(str(vhd)), top=hdr["top"], std=std,
        std2={"1993": "93", "2002": "02", "2008": "08", "2019": "19"}[std],
        stop=hdr["stop"], stop_flag=f"--stop-time={hdr['stop']}" if hdr["stop"] else "",
        res_flag=f"--resolution={hdr['res']}" if hdr["res"] else "", relaxed=hdr.get("relaxed_flag", ""),
        work=shlex.quote(str(work / "lib")), threads=threads, shuffle=shuffle or "",
        shuffle_flag=f"--shuffle={shuffle}" if shuffle else "",
        shuffle_on="--shuffle" if shuffle else "")
    cmd = ["timeout", str(TIMEOUT)]
    if sandbox:
        cmd += ["systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=2G"]
    cmd += ["bash", "-c", script]
    p = subprocess.run(cmd, capture_output=True, text=True, errors="replace", cwd=work)
    (work / "stdout.txt").write_text(p.stdout)
    (work / "stderr.txt").write_text(p.stderr)
    return p.returncode, p.stdout, p.stderr


# ---------------------------------------------------------------- normalise
NVC_HDR = re.compile(r"^\*\* (Note|Warning|Error|Failure|Fatal): (\d+)([a-z]+)\+(\d+): (.*)$")
NVC_TRACE = re.compile(r"^\s+(Process|Procedure|Function|Protected|Instance|Package)\b\s*(\S*)")
# Source snippets, and the "Label L at file:line" line of a PSL directive with a report.
NVC_SNIP = re.compile(r"^\s*(>|\||\d+ \||Label \S+ at \S+:\d+$)")
NVC_CTL = re.compile(r"^(FINISH|STOP) called(?: with status (\d+))?$")
GHDL_HDR = re.compile(r"^\S+?:\d+:\d+:@(\d+)([a-z]+):\((?:psl )?(?:report|assertion|assumption|cover) (\w+)\): (.*)$")
GHDL_CTL = re.compile(r"^simulation (finished|stopped) @(\d+)([a-z]+)(?: with status (\d+))?$")
GHDL_ERR = re.compile(r"^ghdl:error: (.*)$")
YAVS_RTE = re.compile(r"^error\[(?:E3\d{3}|E9999)\]: (.*)$")
YAVS_AT = re.compile(r"^\s*= note: at (\d+) ([a-z]+), delta (\d+)")


def yavs_fatal(out, err):
    """yavs prints a runtime error as a rustc-style diagnostic on stderr (code E3xxx or E9999,
    a `= note: at T, delta D` line): turn it into NVC's `** Fatal` line at the end of stdout,
    where its other output is, and drop the block and the "aborting" summary."""
    lines, keep, fatal = err.splitlines(), [], []
    i = 0
    while i < len(lines):
        m = YAVS_RTE.match(lines[i])
        j = i + 1
        while m and j < len(lines) and not lines[j].startswith("error"):
            j += 1
        at = [YAVS_AT.match(l) for l in lines[i:j]] if m else []
        at = next((a for a in at if a), None)
        if at:
            fatal.append(f"** Fatal: {at.group(1)}{at.group(2)}+{at.group(3)}: {m.group(1)}")
            i = j
            continue
        if not (fatal and lines[i].startswith("error: aborting due to")):
            keep.append(lines[i])
        i += 1
    return "\n".join(out.splitlines() + fatal), "\n".join(keep)


def norm_msg(m):
    return "Assertion violation" if m == "Assertion violation." else m  # NVC adds a full stop


def normalise(rc, out, err, yavs=False):
    """Return canonical lines: exit, diag events, stdout lines.
    Event line: diag TAB sev TAB time_fs TAB delta TAB path TAB msg ('-' = absent)."""
    diags, outl = [], []
    cur = None
    if yavs:
        out, err = yavs_fatal(out, err)
    swallow = False

    def flush():
        nonlocal cur
        if cur:
            if cur[4] == "Assertion violation":
                cur[3] = "-"  # NVC prints a source snippet, not the process, for the default message
            cur[4] = esc(cur[4])
            diags.append("\t".join(["diag"] + cur))
        cur = None

    def ctl(kind, status, t, d):
        diags.append("\t".join(["diag", kind.lower(), t, d, "-", status]))

    for stream, text in (("out", out), ("err", err)):  # GHDL: reports on stdout, errors on stderr
        for line in text.splitlines():
            if swallow and stream == "err" and (NVC_TRACE.match(line) or NVC_SNIP.match(line)):
                continue
            swallow = False
            m = NVC_HDR.match(line)
            if m:
                flush()
                sev, n, u, d, msg = m.groups()
                t = to_fs(n, u)
                c = NVC_CTL.match(msg)
                if c:
                    ctl(c.group(1), c.group(2) or "0", t, d)
                    swallow = True
                    continue
                cur = [sev.lower(), t, d, "-", norm_msg(msg)]
                continue
            m = GHDL_HDR.match(line)
            if m:
                flush()
                n, u, sev, msg = m.groups()
                cur = [sev, to_fs(n, u), "-", "-", norm_msg(msg)]
                continue
            m = GHDL_CTL.match(line)
            if m:
                flush()
                ctl({"finished": "finish", "stopped": "stop"}[m.group(1)], m.group(4) or "0",
                    to_fs(m.group(2), m.group(3)), "-")
                continue
            if line.startswith("ghdl:info: "):
                continue  # e.g. "simulation stopped by --stop-time"
            m = GHDL_ERR.match(line)
            if m:
                flush()
                if m.group(1) not in ("assertion failed", "simulation failed"):
                    diags.append("\t".join(["diag", "fatal", "-", "-", "-", esc(m.group(1))]))
                continue
            if line.startswith("  instance: ") or line.startswith("  from: "):
                continue
            if cur and stream == "out" and yavs:
                # yavs prints NVC's format on stdout (cli-contract), the process trace last.
                t = NVC_TRACE.match(line)
                if t:
                    if t.group(1) == "Process" and cur[3] == "-":
                        cur[3] = t.group(2)
                    flush()
                else:
                    cur[4] += "\n" + line
                continue
            if cur and stream == "err":
                t = NVC_TRACE.match(line)
                if t:
                    if t.group(1) == "Process" and cur[3] == "-":
                        cur[3] = t.group(2)
                elif not NVC_SNIP.match(line):
                    cur[4] += "\n" + line
                continue
            flush()
            if stream == "out":
                outl.append("out\t" + esc(line))
            elif line.strip():
                outl.append("err\t" + esc(line))
        flush()
    # NVC fatal runtime errors carry their text in the header: message kept
    return ["exit\t%d" % rc] + diags + outl


def mask(lines, ignore):
    if "errexit" in ignore and lines[0] == "exit\t1" and not any(
            l.startswith(("diag\tfailure\t", "diag\tfatal\t")) for l in lines):
        lines = ["exit\t0"] + lines[1:]  # NVC exits 1 after `error` severity reports, GHDL 0
    res, keys = [], []
    for l in lines:
        f = l.split("\t")
        if f[0] == "diag" and len(f) == 6:
            if "delta" in ignore: f[3] = "-"
            if "path" in ignore: f[4] = "-"
            if "fatalmsg" in ignore and f[1] == "fatal": f[2] = f[3] = f[5] = "-"
            if "time" in ignore: f[2] = "-"
        if f[0] == "err" and "stderr" in ignore:
            continue
        keys.append((f[2], f[3]) if f[0] == "diag" and len(f) == 6 else None)
        res.append("\t".join(f))
    if "order" in ignore:  # same-time(+delta) events: process order is unspecified by the LRM
        out, i = [], 0
        while i < len(res):
            j = i
            while j < len(res) and keys[j] is not None and keys[j] == keys[i]:
                j += 1
            j = max(j, i + 1)
            # With paths, a stable sort by process keeps each process's own order; without
            # them (GHDL) only the text is left to sort by.
            by = (lambda l: l) if "path" in ignore else (lambda l: l.split("\t")[4])
            out += sorted(res[i:j], key=by) if keys[i] is not None else res[i:j]
            i = j
        res = out
    return res


# ---------------------------------------------------------------- per test
def expected_path(vhd):
    return vhd.with_suffix(".expected")


def one(vhd, args, template, ignore):
    rel = vhd.relative_to(CORPUS)
    hdr = parse_header(vhd)
    base = CACHE / args.candidate_id / rel.with_suffix("")
    res = {"name": str(rel), "ok": True, "msg": ""}
    hand = hdr["ref"] == "hand"  # .expected maintained by hand: NVC is known wrong here
    ignore = ignore | set(hdr["ignore"].split(",")) - {""}  # per-test additions
    if hdr["relaxed"] is not None:  # a bash template gets the yavs switch
        hdr["relaxed_flag"] = RELAXED.get(args.candidate, "--relaxed")
    if hand and args.candidate == "nvc":
        return {**res, "msg": "skipped, hand-maintained .expected"}
    try:
        outs = []
        thread_list = args.threads if "{threads}" in template or "{threads}" in args.candidate else [1]
        for th in thread_list:
            rc, out, err = run(template, vhd, hdr, base / f"t{th}", th, args.shuffle, not args.no_sandbox)
            if rc == 124:
                return {**res, "ok": False, "msg": f"timeout after {TIMEOUT}s"}
            if rc in (RC_ANALYSE, RC_ELAB) and hdr["exit"] != rc:
                return {**res, "ok": False, "msg": f"{'analysis' if rc == RC_ANALYSE else 'elaboration'} failed:\n" + (err or out)[:1500],
                        "stage": "analysis" if rc == RC_ANALYSE else "elab", "raw": err or out}
            outs.append(normalise(rc, out, err, yavs="YAVS_BIN" in template))
        if args.bless:
            if hand:
                return {**res, "msg": "skipped, hand-maintained .expected"}
            if outs[0][0] != "exit\t%d" % hdr["exit"]:
                return {**res, "ok": False, "msg": f"bless refused: exit {outs[0][0]} but header says exit={hdr['exit']}"}
            expected_path(vhd).write_text("\n".join(outs[0]) + "\n")
            return {**res, "msg": "blessed"}
        ep = expected_path(vhd)
        if not ep.exists():
            return {**res, "ok": False, "msg": "no .expected (run with --bless using --candidate nvc)"}
        want = mask(ep.read_text().splitlines(), ignore)
        problems = []
        for th, o in zip(thread_list, outs):
            got = mask(o, ignore)
            if got != want:
                problems.append(f"threads={th} differs from expected:\n" + "\n".join(
                    difflib.unified_diff(want, got, "expected", "got", lineterm="", n=1)))
            if o != outs[0]:
                problems.append(f"threads={th} differs from threads={thread_list[0]}:\n" + "\n".join(
                    difflib.unified_diff(outs[0], o, f"t{thread_list[0]}", f"t{th}", lineterm="", n=1)))
        if problems:
            res.update(ok=False, msg="\n".join(problems))
        return res
    except Exception as e:  # report, never hide
        return {**res, "ok": False, "msg": f"harness error: {e!r}"}


def main():
    global CORPUS
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--candidate", default="nvc", help="nvc | ghdl | yavs | a bash command template (see README)")
    ap.add_argument("--bless", action="store_true", help="regenerate .expected from NVC (candidate must be nvc)")
    ap.add_argument("--jobs", type=int, default=8, help=f"parallel simulations (max {MAX_JOBS})")
    ap.add_argument("--filter", default="", help="regex on the corpus-relative path")
    ap.add_argument("--threads", type=lambda s: [1] + [int(x) for x in s.split(",") if x and int(x) != 1],
                    default=[1], help="candidate thread counts to compare, e.g. 4 (runs 1 and 4); needs {threads} in the template")
    ap.add_argument("--shuffle", default="", help="seed passed to the candidate as {shuffle}/{shuffle_flag}")
    ap.add_argument("--ignore", default=None, help="comma list of: delta,path,fatalmsg,order,errexit,time,stderr (default depends on candidate)")
    ap.add_argument("--corpus", type=Path, default=CORPUS)
    ap.add_argument("--no-sandbox", action="store_true", help="skip systemd-run (debugging only)")
    ap.add_argument("--json", type=Path, help="write one JSON result per test (stage, message) to this file")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("-v", "--verbose", action="store_true")
    args = ap.parse_args()
    CORPUS = args.corpus.resolve()
    if args.bless and args.candidate != "nvc":
        sys.exit("--bless requires --candidate nvc")
    template = TEMPLATES.get(args.candidate, args.candidate)
    args.candidate_id = args.candidate if args.candidate in TEMPLATES else "custom-%x" % zlib.crc32(args.candidate.encode())
    ignore = set((DEFAULT_IGNORE.get(args.candidate, "") if args.ignore is None else args.ignore).split(","))
    tests = sorted(p for p in CORPUS.rglob("*.vhd") if re.search(args.filter, str(p.relative_to(CORPUS))))
    if args.list:
        print("\n".join(str(p.relative_to(CORPUS)) for p in tests)); return
    jobs = max(1, min(args.jobs, MAX_JOBS))
    fails, results = 0, []
    with cf.ThreadPoolExecutor(jobs) as ex:
        for r in ex.map(lambda p: one(p, args, template, ignore), tests):
            results.append(r)
            if r["ok"]:
                if args.verbose or args.bless:
                    print(f"PASS {r['name']}" + (f" ({r['msg']})" if r["msg"] else ""))
            else:
                fails += 1
                print(f"FAIL {r['name']}\n" + "\n".join("    " + l for l in r["msg"].splitlines()[:40]))
    if args.json:
        import json
        args.json.write_text("\n".join(json.dumps(r) for r in results) + "\n")
    print(f"{len(tests) - fails}/{len(tests)} passed (candidate={args.candidate}, ignore={','.join(sorted(ignore - {''})) or 'none'})")
    sys.exit(1 if fails else 0)


if __name__ == "__main__":
    main()
