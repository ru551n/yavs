#!/usr/bin/env python3
"""Replay gate of cycle mode: run recorded `yavs run` command lines (the runs of a test runner such as
VUnit) normal and with `--cycle` using one binary, each mode in its own copy of the run's output
directory, and compare exit status, the simulation end time (the `stop`/`finish` events), the console in
difftest's canonical form (without the reports and asserts of cycle-based processes, found through a
`--cycle-report` run as cyclegate.py does, and without the cycle summary line) and the text files the run
writes. Stdlib only. See README.md.

Command file: one JSON object per line: {"name", "cwd", "dir", "argv", "env"}. `argv[0]` is replaced by the
binary, `argv[1]` is `run`; `dir` is the run's output directory (copied per mode, its path replaced in every
argument and environment value); `cwd` is the working directory of the process; `env` is optional.
`--record OUT_DIR...` makes the file from the output directories of a VUnit run with yavs
(`<out>/test_output/<test>/yavs/args.rsp`), on standard output. The libraries of `--work=`/`--map=` are
replayed from private copies made once per invocation (without their `yavs-cache/`), so the recorded
libraries are never written; `--libs DIR` keeps the copies and the std store in DIR, and a later replay with
the same DIR runs on them as they were left. `--warm` runs each cycle run with `YAVS_CYCLE_STATS` (its lines taken
out before the compare), counts the cache hits in the summary and fails a run with members that was not warm.

Buckets: identical; documented (equal once the delta of report stamps and the reports and asserts of
cycle-based processes are taken out, or the normal run ended on a failure of such a process); differs
(a wrong result: the exit status, the end time, the console or a written file; fails the gate); skipped
(the normal run does not finish or fails to start). Each test shows how many processes joined the region.
"""
import argparse, atexit, concurrent.futures as cf, difflib, json, os, random, re, shlex, shutil, subprocess, sys, time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import cyclegate as G  # noqa: E402
import difftest as D  # noqa: E402

MODES = ("normal", "cycle", "report")
MASKS = []  # regular expressions (--mask) whose matches are taken out of the console and of the files, in both modes


def masked(text):
    for m in MASKS:
        text = m.sub("<masked>", text)
    return text


# ---------------------------------------------------------------- recording
def record(outs, cwd, env):
    """Entries of every test of the VUnit output directories `outs`."""
    entries = []
    for o in outs:
        for rsp in sorted(Path(o).resolve().glob("test_output/*/*/args.rsp")):
            d = rsp.parent.parent
            args = rsp.read_text(encoding="utf-8").split("\n")[:-1]  # one argument per line, verbatim
            entries.append({"name": re.sub(r"_[0-9a-f]{40}$", "", d.name), "cwd": str(cwd), "dir": str(d),
                            "argv": ["yavs", "run", *args], "env": env})
    return entries


def load(path, filt):
    out = []
    for n, l in enumerate(Path(path).read_text().splitlines(), 1):
        if not l.strip():
            continue
        e = json.loads(l)
        for k in ("name", "cwd", "argv"):
            if k not in e:
                sys.exit(f"cyclereplay: {path}:{n}: no key {k!r}")
        if e["argv"][:2][-1:] != ["run"] or any(a.startswith("--cycle") for a in e["argv"]):
            sys.exit(f"cyclereplay: {path}:{n}: argv must be `yavs run ...` without a --cycle option")
        if re.search(filt, e["name"]):
            out.append(e)
    names = [e["name"] for e in out]
    dup = sorted({x for x in names if names.count(x) > 1})
    if dup:
        sys.exit(f"cyclereplay: duplicate test names (the scratch directories would collide): {dup[:3]}")
    return out


LIB_ARG = re.compile(r"^(--(?:work|map)=[^:]+:)(.+)$")


def private_libs(todo, root, std, keep=False):
    """Point every `--work=`/`--map=` library outside a run's own directory at a copy under `root` (one per
    library, without its `yavs-cache/`), and a `--std-cache=DIR` at `std`: runs write their caches there,
    never into the recorded libraries. With `keep` the copies of an earlier replay listed in
    `root/libs.json` are used as they are (with the caches the runs left in them); the new copies are returned."""
    man = root / "libs.json"
    if not keep:
        shutil.rmtree(root, ignore_errors=True)  # left by a replay that was killed
    copies = {k: Path(v) for k, v in json.loads(man.read_text()).items()} if keep and man.is_file() else {}
    old = {k: v for k, v in copies.items() if v.is_dir()}
    copies = dict(old)
    for e in todo:
        own = e.get("dir", "").rstrip("/")
        for i, x in enumerate(e["argv"]):
            if x.startswith("--std-cache=") and x != "--std-cache=off":
                e["argv"][i] = f"--std-cache={std}"
                continue
            if x in ("--work", "--map", "--std-cache"):
                sys.exit(f"cyclereplay: {e['name']}: `{x} LIB:DIR` as two arguments is not supported: record it as `{x}=LIB:DIR`")
            m = LIB_ARG.match(x)
            if not m or (own and (m.group(2) + "/").startswith(own + "/")):
                continue
            src = m.group(2).rstrip("/")
            if not Path(src).is_dir():
                continue  # a mapped library the run may not use: nothing to write into
            if src not in copies:
                copies[src] = dst = root / f"{len(copies)}_{Path(src).name}"
                shutil.rmtree(dst, ignore_errors=True)  # a kept copy whose entry was dropped
                shutil.copytree(src, dst, symlinks=True, ignore=shutil.ignore_patterns("yavs-cache"))
            e["argv"][i] = m.group(1) + str(copies[src])
    if keep:
        root.mkdir(parents=True, exist_ok=True)
        man.write_text(json.dumps({k: str(v) for k, v in copies.items()}))
    return {k: v for k, v in copies.items() if k not in old}


# ---------------------------------------------------------------- comparison
def sev(l):
    return l.split("\t")[1]


IEEE_MSG = re.compile(r"^[A-Z][A-Z0-9_]*\.\S.*(metavalue|null) detected|^[A-Z][A-Z0-9_]*\.\S.*non logical")


def member_paths(rep):
    """The LRM paths of every cycle-based process of a region report (its last section)."""
    out, on = set(), False
    for l in rep:
        if re.match(r"^  \S", l):
            on = bool(re.match(r"^  cycle-based, \d+ process", l))
        elif on:
            m = re.match(r"^    (.*?) \(\S+:\d+\)  \S", l)
            if m:
                out.update({D.lrm_path(m.group(1)), D.lrm_path(re.sub(r" <= \.\.\.$", "", m.group(1)))})
    return out


def raw_names(rep):
    """(held, members): the process names of a region report as the console prints them, with and without
    their `(file:line)`: the processes holding asserts and reports that are not evaluated, and every cycle-based one."""
    held, members, sec = set(), set(), ""
    for l in rep:
        if re.match(r"^  \S", l):
            sec = "held" if " not evaluated in cycle mode, held by " in l else "members" if re.match(r"^  cycle-based, \d+ process", l) else ""
        elif sec == "held" and (m := re.match(r"^\s+\d+  (.*?)(?: \((\S+:\d+)\))?$", l)):
            held.update({m.group(1), l.strip().partition("  ")[2]})
        elif sec == "members" and (m := re.match(r"^    (.*?) \((\S+:\d+)\)  \S", l)):
            members.update({m.group(1), f"{m.group(1)} ({m.group(2)})"})
    return held, members


RECORD = re.compile(r"^\*\* (\w+): \S+: (.*)$")


def strip_records(text, held, members):
    """Without the records of a console text that cycle mode does not give: notes and warnings of processes
    holding asserts and reports that are not evaluated (any message), library warnings of any cycle-based process.
    A record is its `** Severity: time: message` line and the `    Process NAME` line after it. This finds a
    process the canonical path field does not name (an unlabelled one prints `NAME <= ... (file:line)`)."""
    out, ls, i = [], text.splitlines(keepends=True), 0
    while i < len(ls):
        m = RECORD.match(ls[i])
        if m and i + 1 < len(ls) and ls[i + 1].startswith("    Process "):
            who = ls[i + 1][len("    Process "):].strip()
            if (m.group(1) in ("Note", "Warning") and who in held) or (who in members and IEEE_MSG.match(m.group(2))):
                i += 2
                continue
        out.append(ls[i])
        i += 1
    return "".join(out)


def drop_ieee(lines, members):
    """Without the library warnings of cycle-based processes (A6: none are given at any severity)."""
    keep = []
    for l in lines:
        f = l.split("\t")
        if not (f[0] == "diag" and len(f) == 6 and f[4] in members and IEEE_MSG.match(f[5].replace("\\n", " "))):
            keep.append(l)
    return keep


def strip(lines, lost, drop_delta=True):
    """Canonical lines without the reports of lost processes (never a stop or finish), deltas masked."""
    out = []
    for l in lines:
        f = l.split("\t")
        if f[0] == "exit":
            continue  # compared on its own
        if f[0] == "diag" and len(f) == 6:
            if f[1] not in ("stop", "finish") and lost(f[4]):
                continue
            if drop_delta:
                f[3] = "-"
        out.append("\t".join(f))
    return out


def ends(lines):
    """The simulation end time of canonical lines: the times of the stop and finish events; when there
    is none, the time of the last report (`last report`)."""
    st = [l.split("\t")[2] for l in lines if l.startswith("diag\t") and sev(l) in ("stop", "finish")]
    if st:
        return st
    t = [l.split("\t")[2] for l in lines if l.startswith("diag\t")]
    return [f"last report {t[-1]}"] if t else ["no report"]


def same_text(a, b, ra="", rb=""):
    """None when file a (normal) and b (cycle) are equal, a `?` in a matching any character (a metavalue
    digit, read as 0 in cycle mode), the run directories ra and rb (in the texts) taken as one; else where they differ."""
    xa, xb = a.read_bytes(), b.read_bytes()
    if ra and rb:
        xa, xb = xa.replace(ra.encode(), b"<dir>"), xb.replace(rb.encode(), b"<dir>")
    if MASKS:
        xa, xb = (masked(x.decode(errors="replace")).encode() for x in (xa, xb))
    if xa == xb:
        return None
    la, lb = xa.decode(errors="replace").splitlines(), xb.decode(errors="replace").splitlines()
    for i, (x, y) in enumerate(zip(la + [None] * len(lb), lb + [None] * len(la)), 1):
        if x != y and (x is None or y is None or len(x) != len(y) or any(c != d and c != "?" for c, d in zip(x, y))):
            return f"line {i}: {x!r} against {y!r}"
    return None


def compare(nrc, nl, nfiles, crc, cl, cfiles, lost, members=frozenset(), roots=("", "")):
    """(bucket, [what differs]) of the two runs: exit status, canonical console lines, written files
    ({relative name: path}). `lost(path)` says whether a report of a process at that path may be missing;
    `members` are the paths of all cycle-based processes, whose library warnings are always missing."""
    same = nl == cl
    nl, cl = drop_ieee(nl, members), drop_ieee(cl, members)
    why = []
    # a lost error, failure or fatal of a cycle-based process may change the exit status (A12)
    lost_err = any(l.startswith("diag\t") and sev(l) in ("error", "failure", "fatal") and lost(l.split("\t")[4]) for l in nl)
    if nrc != crc and not lost_err:
        why.append(f"exit status {nrc} normal, {crc} cycle")
    ns, cs = strip(nl, lost), strip(cl, lost)
    cut = False
    if ns != cs:
        r, how = G.classify(nl, cl, lost)  # a lost failure ended the normal run: the rest is not comparable
        cut = r == "documented" and "a lost failure stopped the normal run" in how
        if not cut:
            d = list(difflib.unified_diff(ns, cs, "normal", "cycle", lineterm="", n=0))
            first = next((x for x in d[2:] if x[:1] in "+-"), "")
            why.append("console differs, first line: " + first[:300])
    if not cut:
        if ends(nl) != ends(cl):
            why.append(f"end time {ends(nl)} normal, {ends(cl)} cycle")
        if set(nfiles) != set(cfiles):
            why.append(f"files written: {sorted(set(nfiles) ^ set(cfiles))[:4]} in one mode only")
        why += [f"file {k}: {w}" for k in sorted(set(nfiles) & set(cfiles)) if (w := same_text(nfiles[k], cfiles[k], *roots))]
    if why:
        return "differs", why
    if same:
        return "identical", []
    return "documented", ["delta counts, or reports and asserts of cycle-based processes"
                          + (" (the normal run ended on one: not compared after it)" if cut else "")]


# ---------------------------------------------------------------- running
STATS = re.compile(r"^cycle stats: (?:(analysis|region) from the cache( checked: the same)?$|(\d+) functions in \d+ ms, (\d+) from the cache$)")


def cache_hits(err):
    """(stderr without its `cycle stats:` lines, {"analysis", "region": hits, "checked": hits compared with a
    new compile (`YAVS_CYCLE_CACHE_CHECK`), "funs": functions made, "fhits": of them from the cache}) of a run
    with `YAVS_CYCLE_STATS`."""
    hits, keep = {"analysis": 0, "region": 0, "checked": 0, "funs": 0, "fhits": 0}, []
    for l in err.splitlines(keepends=True):
        if not l.startswith("cycle stats: "):
            keep.append(l)
        elif m := STATS.match(l.rstrip("\n")):
            if m.group(1):
                hits["checked" if m.group(2) else m.group(1)] += 1
            else:
                hits["funs"] += int(m.group(3))
                hits["fhits"] += int(m.group(4))
    return "".join(keep), hits


def not_warm(h):
    """Why a cycle run with members was not warm (None: it was): every function from the cache, or the region
    from it (also when the cache check compiled it again and found it the same). No stats line is no proof."""
    if h.get("region") or (h.get("checked") and h.get("funs")) or (h.get("funs") and h["fhits"] == h["funs"]):
        return None
    if not h.get("funs"):
        return "not warm: the cycle run printed no cache counts"
    return f"not warm: the cycle run compiled {h['funs'] - h['fhits']} of {h['funs']} functions"


def snapshot(d):
    return {p: (p.stat().st_size, p.stat().st_mtime_ns) for p in d.rglob("*") if p.is_file()} if d.is_dir() else {}


def one_run(e, mode, base, a):
    """Run entry e in `mode` in a copy of its directory; (exit status, stdout, stderr, {written file: path})."""
    work = base / mode
    shutil.rmtree(work, ignore_errors=True)
    if e.get("dir") and Path(e["dir"]).is_dir():
        shutil.copytree(e["dir"], work, symlinks=True)
    else:
        work.mkdir(parents=True)
    before = snapshot(work)
    sub = [(k, v) for k, v in [s.split("=", 1) for s in a.subst]] + ([(e["dir"].rstrip("/"), str(work))] if e.get("dir") else [])

    def fix(s):
        for k, v in sub:
            s = s.replace(k, v)
        return s

    flag = {"normal": [], "cycle": ["--cycle"], "report": ["--cycle-report"]}[mode]
    argv = [a.bin, "run", *flag, *[fix(x) for x in e["argv"][2:]]]
    env = {**os.environ, **{k: fix(v) for k, v in e.get("env", {}).items()}, **dict(x.split("=", 1) for x in a.env),
           "YAVS_STD_CACHE": str(a.std)}
    if mode == "cycle" and a.warm:
        env["YAVS_CYCLE_STATS"] = "1"  # the cache hits; its lines are taken out below
    cmd = shlex.split(a.prefix) + [shutil.which("timeout"), str(a.timeout)]
    if not a.no_sandbox:
        cmd += ["systemd-run", "--user", "--scope", "-q", "-p", f"MemoryMax={a.memory}"]
    p = subprocess.run(cmd + argv, capture_output=True, text=True, errors="replace", cwd=e["cwd"], env=env, stdin=subprocess.DEVNULL)
    files = {str(f.relative_to(work)): f for f, s in snapshot(work).items() if before.get(f) != s}
    (base / f"{mode}.out").write_text(p.stdout)
    (base / f"{mode}.err").write_text(p.stderr)
    clean = lambda t: masked(t.replace(str(work), "<dir>"))
    err, hits = cache_hits(p.stderr) if mode == "cycle" and a.warm else (p.stderr, {})
    a.hits[e["name"]] = hits if mode == "cycle" else a.hits.get(e["name"], {})
    return p.returncode, clean(p.stdout), clean(err), files


def one(e, a):
    if a.budget and time.time() > a.deadline:
        return {"name": e["name"], "bucket": "skipped", "why": ["not started: the time budget is spent"], "members": 0, "procs": 0, "end": ""}
    base = D.CACHE / "replay" / re.sub(r"\W+", "_", e["name"])[-120:]
    res = {"name": e["name"], "bucket": "", "why": [], "members": 0, "procs": 0, "end": ""}
    try:
        (nrc, nout, nerr, nf) = one_run(e, "normal", base, a)
        if nrc in (124, 97, 98) or (nrc and not nout.strip() and not nerr.strip()):
            res.update(bucket="skipped", why=[f"the normal run gives status {nrc}" + (" (timeout)" if nrc == 124 else "")])
        else:
            (crc, cout, cerr, cf_) = one_run(e, "cycle", base, a)
            rerr = one_run(e, "report", base, a)[2]
            cerr, _ = G.split_report(cerr)
            rep = G.split_report(rerr)[1]
            held, dropped = G.held_asserts(rep)
            res["members"], res["procs"] = G.count_members(rep)
            lost = lambda path: dropped > 0 and path != "-" and path in held
            hr, mr = raw_names(rep)
            nl = D.normalise(nrc, strip_records(nout, hr, mr), strip_records(nerr, hr, mr), yavs=True)
            cl = D.normalise(crc, strip_records(cout, hr, mr), strip_records(cerr, hr, mr), yavs=True)
            res["end"] = ends(nl)[-1] if ends(nl) == ends(cl) else f"{ends(nl)[-1]} normal, {ends(cl)[-1]} cycle"
            res["bucket"], res["why"] = compare(nrc, nl, nf, crc, cl, cf_, lost, member_paths(rep), (str(base / "normal"), str(base / "cycle")))
            if res["bucket"] == "identical" and (nout, nerr) != tuple(strip_records(t, hr, mr) for t in (nout, nerr)):
                res.update(bucket="documented", why=["reports and library warnings of cycle-based processes"])
            h = res["hits"] = a.hits.get(e["name"], {})
            if a.warm and res["bucket"] != "differs" and res["members"] and (w := not_warm(h)):
                res.update(bucket="differs", why=[w])
            if crc == 124:  # nothing to compare: its output stops where the time ran out
                res.update(bucket="timeout", why=[f"the cycle run timed out after {a.timeout} s; the normal run did not"])
        if res["bucket"] in ("differs", "timeout"):
            res["dir"] = str(base)
        else:
            D.drop(base)
        return res
    except Exception as ex:  # report, never hide
        return {**res, "bucket": "differs", "why": [f"harness error: {ex!r}"], "dir": str(base)}


# ---------------------------------------------------------------- selftest
def selftest():
    import tempfile
    n = ["exit\t0", "diag\tnote\t5\t1\t:tb:p\thi", "diag\twarning\t9\t1\t:tb:m\tmember", "out\tdone", "diag\tstop\t90\t3\t-\t0"]
    lost = lambda p: p == ":tb:m"
    cmp = lambda nl, cl, **k: compare(k.get("nrc", 0), nl, k.get("nf", {}), k.get("crc", 0), cl, k.get("cf", {}), k.get("lost", lost),
                                      k.get("members", frozenset()))[0]
    assert cmp(n, n) == "identical"
    assert cmp(n, [l.replace("\t1\t", "\t2\t") for l in n]) == "documented"  # other delta counts
    assert cmp(n, n[:2] + n[3:]) == "documented"  # a member's report is gone
    assert cmp(n, n[:1] + n[2:]) == "differs"  # a testbench report is gone
    assert cmp(n, n[:4] + ["diag\tstop\t80\t3\t-\t0"]) == "differs"  # stopped one clock early
    assert cmp(n, n[:3] + ["out\tother"] + n[4:]) == "differs"
    assert cmp(n, n, crc=1) == "differs"
    assert cmp(n[:3] + n[4:], n[:3] + n[4:], lost=lambda p: True) == "identical"
    # the stop event is never forgiven, even when its path (`-`) is one of a member's
    assert cmp(n, n[:4] + ["diag\tstop\t80\t3\t-\t0"], lost=lambda p: True) == "differs"
    w = "diag\twarning\t0\t1\t:tb:d:r\tNUMERIC_STD.\">\": metavalue detected, returning FALSE"
    assert cmp(n + [w], n, members={":tb:d:r"}) == "documented"  # a library warning of a cycle-based process
    assert cmp(n + [w], n) == "differs" and cmp(n + [w], n, members={":tb:x"}) == "differs"
    assert member_paths(["cycle mode: 1 of 2 processes", "  cycle-based, 2 processes:", "    tb.d.r (a.vhd:3)  clocked", "    tb.q <= ... (a.vhd:4)  combinational", "  next"]) == {":tb:d:r", D.lrm_path("tb.q <= ..."), ":tb:q"}
    rep = ["cycle mode: 2 of 5 processes run cycle-based", "  2 asserts and reports not evaluated in cycle mode, held by 2 cycle-based processes:",
           "       1  tb.q <= ... (a.vhd:3)", "       1  tb.p", "  cycle-based, 3 processes:", "    tb.q <= ... (a.vhd:3)  combinational",
           "    tb.p (a.vhd:9)  clocked", "    tb.r (a.vhd:12)  clocked", "  subset"]
    held, mem = raw_names(rep)
    assert held == {"tb.q <= ...", "tb.q <= ... (a.vhd:3)", "tb.p"} and "tb.r" in mem and "tb.r (a.vhd:12)" in mem, (held, mem)
    txt = ("** Note: 5ns+1: kept\n    Process tb.o\n** Note: 5ns+1: lost\n    Process tb.q <= ... (a.vhd:3)\n"
           "** Warning: 5ns+1: NUMERIC_STD.TO_INTEGER: metavalue detected, returning 0\n    Process tb.r (a.vhd:12)\n"
           "** Warning: 5ns+1: NUMERIC_STD.TO_INTEGER: metavalue detected, returning 0\n    Process tb.o\n"
           "** Failure: 5ns+1: lost?\n    Process tb.p\nplain\n")
    assert strip_records(txt, held, mem) == ("** Note: 5ns+1: kept\n    Process tb.o\n** Warning: 5ns+1: NUMERIC_STD.TO_INTEGER: metavalue detected, "
                                              "returning 0\n    Process tb.o\n** Failure: 5ns+1: lost?\n    Process tb.p\nplain\n")
    t = ["out\tx", "diag\tnote\t7\t1\t-\tlast"]
    assert ends(t) == ["last report 7"] and ends(n) == ["90"] and ends(["exit\t0"]) == ["no report"]
    f = ["exit\t1", "diag\tfailure\t9\t1\t:tb:m\tbad"]
    assert cmp(f, ["exit\t0", "out\tmore", "diag\tstop\t20\t1\t-\t0"], nrc=1) == "documented"  # its stop is lost
    assert cmp(f, ["exit\t0", "diag\tstop\t20\t1\t-\t0"], nrc=1, lost=lambda p: False) == "differs"
    with tempfile.TemporaryDirectory(dir=D.CACHE.parent if D.CACHE.parent.is_dir() else None) as d:
        d = Path(d)
        (d / "a").write_text("x 0x????0001\ny\n")
        (d / "b").write_text("x 0x00000001\ny\n")
        (d / "c").write_text("x 0x00000002\ny\n")
        assert same_text(d / "a", d / "b") is None and same_text(d / "a", d / "c").startswith("line 1")
        assert cmp(n, n, nf={"f": d / "a"}, cf={"f": d / "c"}) == "differs"
        assert cmp(n, n, nf={"f": d / "a"}, cf={"f": d / "b"}) == "identical"
        assert cmp(n, n, nf={"f": d / "a"}, cf={}) == "differs"
        (d / "p").write_text("at /x/normal/f\n")
        (d / "q").write_text("at /x/cycle/f\n")
        MASKS.append(re.compile(r"tmp_\w{4}"))
        (d / "m1").write_text("tmp_ab12 x\n")
        (d / "m2").write_text("tmp_cd34 x\n")
        assert same_text(d / "m1", d / "m2") is None and masked("a tmp_zz99") == "a <masked>"
        MASKS.clear()
        assert same_text(d / "m1", d / "m2") is not None
        assert same_text(d / "p", d / "q") is not None and same_text(d / "p", d / "q", "/x/normal", "/x/cycle") is None
        t = d / "o" / "test_output" / ("t_" + "0" * 40) / "yavs"
        t.mkdir(parents=True)
        (t / "args.rsp").write_text("--work=w:/x\n-gp=a b\n")
        e = record([d / "o"], "/cwd", {})
        assert e[0]["name"] == "t" and e[0]["argv"] == ["yavs", "run", "--work=w:/x", "-gp=a b"] and e[0]["dir"].endswith("0" * 40)
        (d / "L" / "yavs-cache").mkdir(parents=True)
        (d / "L" / "u").write_text("unit")
        e = [{"name": "t", "dir": str(t), "argv": ["yavs", "run", f"--work=w:{d}/L", f"--map=v:{d}/L/", f"--map=o:{t}/lib",
                                                    f"--std-cache={d}/S", "--std-cache=off"]}]
        private_libs(e, d / "priv", d / "std")
        assert e[0]["argv"][2:] == [f"--work=w:{d}/priv/0_L", f"--map=v:{d}/priv/0_L", f"--map=o:{t}/lib", f"--std-cache={d}/std",
                                    "--std-cache=off"], e
        assert (d / "priv" / "0_L" / "u").is_file() and not (d / "priv" / "0_L" / "yavs-cache").exists()
        (d / "priv" / "0_L" / "yavs-cache").mkdir()  # a run's cache; a kept replay finds it again
        e2 = [{"name": "t", "argv": ["yavs", "run", f"--work=w:{d}/L", f"--map=n:{t}"]}]
        assert private_libs(e2, d / "keep", d / "std", keep=True).keys() == {f"{d}/L", str(t)}
        (d / "keep" / "0_L" / "yavs-cache").mkdir()
        e3 = [{"name": "t", "argv": ["yavs", "run", f"--work=w:{d}/L", f"--map=n:{t}"]}]
        assert private_libs(e3, d / "keep", d / "std", keep=True) == {} and e3[0]["argv"] == e2[0]["argv"]
        assert (d / "keep" / "0_L" / "yavs-cache").is_dir()
        private_libs(e3, d / "keep", d / "std")  # not kept: copied afresh
        assert not (d / "keep" / "0_L" / "yavs-cache").exists()
    err = ("x\ncycle stats: analysis from the cache\ncycle stats: region from the cache\ncycle stats: f: 3 operations, 9 us\n"
           "cycle stats: 4 functions in 2 ms, 3 from the cache\ny\n")
    assert cache_hits(err) == ("x\ny\n", {"analysis": 1, "region": 1, "checked": 0, "funs": 4, "fhits": 3}), cache_hits(err)
    assert not_warm(cache_hits(err)[1]) is None  # the region from the cache
    cold = cache_hits("cycle stats: 4 functions in 2 ms, 3 from the cache\n")[1]
    assert not_warm(cold) == "not warm: the cycle run compiled 1 of 4 functions"
    assert not_warm(cache_hits("cycle stats: 4 functions in 2 ms, 4 from the cache\n")[1]) is None
    assert not_warm(cache_hits("x\n")[1]) == "not warm: the cycle run printed no cache counts"
    chk = cache_hits("cycle stats: region from the cache checked: the same\ncycle stats: 4 functions in 2 ms, 0 from the cache\n")[1]
    assert chk["checked"] == 1 and not_warm(chk) is None, chk
    print("selftest ok")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("commands", nargs="*", help="command file (JSON lines); with --record the VUnit output directories")
    ap.add_argument("--bin", default=os.environ.get("YAVS_BIN", ""), help="yavs binary (default $YAVS_BIN)")
    ap.add_argument("--record", action="store_true", help="print the command file of the given VUnit output directories")
    ap.add_argument("--cwd", default=os.getcwd(), help="with --record: the directory the test runner ran in (default: here)")
    ap.add_argument("--env", action="append", default=[], metavar="K=V", help="environment of every run (with --record: stored in the file)")
    ap.add_argument("--subst", action="append", default=[], metavar="OLD=NEW", help="replace OLD by NEW in every argument and environment value")
    ap.add_argument("--mask", action="append", default=[], metavar="REGEX",
                    help="text that differs between runs by itself (a temporary directory a test makes): taken out of the console and the files")
    ap.add_argument("--filter", default="", help="regex on the test name")
    ap.add_argument("--jobs", type=int, default=2, help=f"tests at once, two to three runs each (default 2, max {D.MAX_JOBS})")
    ap.add_argument("--shuffle", type=int, metavar="SEED", help="run the tests in the order of this random seed (with --budget: a sample, not a prefix)")
    ap.add_argument("--timeout", type=int, default=900, help="seconds per run")
    ap.add_argument("--budget", type=int, default=0, help="seconds after which no further test is started (those are skipped and counted)")
    ap.add_argument("--memory", default="8G", help="MemoryMax of each run")
    ap.add_argument("--prefix", default="", help="command put before every run, e.g. 'tools/claim core --'")
    ap.add_argument("--no-sandbox", action="store_true", help="skip systemd-run")
    ap.add_argument("--libs", type=Path, metavar="DIR",
                    help="keep the private copies of the libraries and the std store in DIR; a later replay with the same DIR uses them as left (warm)")
    ap.add_argument("--warm", action="store_true",
                    help="count the cache hits; a cycle run with members that is not warm differs (with the --libs of an earlier replay)")
    ap.add_argument("--json", type=Path, help="write one JSON result per test to this file")
    ap.add_argument("--selftest", action="store_true")
    ap.add_argument("-v", "--verbose", action="store_true", help="one line per test")
    a = ap.parse_args()
    if a.selftest:
        return selftest()
    if a.record:
        for e in record(a.commands, a.cwd, dict(x.split("=", 1) for x in a.env)):
            print(json.dumps(e))
        return
    if len(a.commands) != 1:
        sys.exit("cyclereplay: give one command file (made with --record)")
    if not a.bin or not Path(a.bin).is_file():
        sys.exit(f"cyclereplay: no yavs binary {a.bin!r}: pass --bin or set YAVS_BIN")
    MASKS.extend(re.compile(m) for m in a.mask)
    a.bin = str(Path(a.bin).resolve())
    a.deadline = time.time() + a.budget
    todo = load(a.commands[0], a.filter)
    if not todo:
        sys.exit("cyclereplay: no test selected")
    if a.shuffle is not None:
        random.Random(a.shuffle).shuffle(todo)
    D.guard(D.CACHE)
    libs = a.libs.resolve() if a.libs else D.CACHE / "libs"
    if not a.libs:
        atexit.register(shutil.rmtree, libs, ignore_errors=True)  # runs before D.cleanup
    a.std, a.hits = libs / "std", {}
    new = private_libs(todo, libs, a.std, keep=bool(a.libs))
    kept = len(json.loads((libs / "libs.json").read_text())) - len(new) if a.libs else 0
    print(f"cycle replay: {len(new)} libraries copied" + (f" into {libs}, {kept} kept from an earlier replay" if a.libs else ""), flush=True)
    res = []
    with cf.ThreadPoolExecutor(max(1, min(a.jobs, D.MAX_JOBS))) as ex:
        try:
            for r in ex.map(lambda e: one(e, a), todo):
                res.append(r)
                if r["why"][:1] == ["not started: the time budget is spent"]:
                    continue
                if a.verbose or r["bucket"] in ("differs", "skipped", "timeout"):
                    print(f"{r['bucket'].upper():10} {r['name']}  [{r['members']}/{r['procs']} in the region] end {r['end']}"
                          + (f"\n    " + "\n    ".join(r["why"]) if r["why"] and r["bucket"] != "documented" else ""), flush=True)
                if r["bucket"] == "differs":
                    print(f"    outputs kept in {r['dir']}", flush=True)
        except BaseException:
            ex.shutdown(wait=True, cancel_futures=True)
            raise
    if a.json:
        a.json.write_text("\n".join(json.dumps(r) for r in res) + "\n")
    c = {b: sum(r["bucket"] == b for r in res) for b in ("identical", "documented", "differs", "timeout", "skipped")}
    ran = [r for r in res if r["bucket"] not in ("skipped", "timeout")]
    budget = sum(r["why"][:1] == ["not started: the time budget is spent"] for r in res)
    with_m = sum(r["members"] > 0 for r in ran)
    print(f"cycle replay: {len(res)} tests: {c['identical']} identical, {c['documented']} documented losses only, "
          f"{c['differs']} differ, {c['timeout']} cycle runs timed out (not compared), {c['skipped'] - budget} skipped, {budget} not started (budget); {with_m} of {len(ran)} compared tests have at least one process "
          f"in the region ({sum(r['members'] for r in ran)} of {sum(r['procs'] for r in ran)} processes)")
    hs = [r.get("hits", {}) for r in ran if r["members"]]
    if a.warm:
        print(f"cycle replay: cache of the {len(hs)} cycle runs with members: {sum(bool(h.get('region')) for h in hs)} took the region from it, "
              f"{sum(bool(h.get('analysis')) for h in hs)} the analysis, {sum(bool(h.get('checked')) for h in hs)} checked them, "
              f"{sum(h.get('fhits', 0) for h in hs)} of {sum(h.get('funs', 0) for h in hs)} functions")
    if not ran:
        print("cyclereplay: nothing was compared")
    sys.exit(1 if c["differs"] or not ran else 0)


if __name__ == "__main__":
    main()
