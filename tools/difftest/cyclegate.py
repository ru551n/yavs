#!/usr/bin/env python3
"""Correctness gate of cycle mode: run the difftest corpus, the designs of
crates/yavs/tests/cycle and the bench RTL designs twice with the same yavs binary,
normal and `--cycle`, and compare console output and exit status; a third run with `--cycle-report`
(analysis only) gives the members of each test. Stdlib only.

Buckets: identical; documented (differs only by what cycle mode drops by design: the delta of
a report stamp, reports and asserts of member processes, delta cycles a monitor no longer
sees); known (differs, and cyclegate.expected lists the test with the checksum of its diff and
the reviewed cause); differs (anything else: a wrong result in cycle mode, or a known
difference that changed; the bucket that fails the gate); refused (E41xx under --cycle);
skipped (the normal mode does not run it). Each test also reports how many processes joined
the region, so a pass is not vacuous. See README.md.

The gate compares exactly (`--contract exact`, the default): cycle runs take `--cycle-exact-meta`
and YAVS_CYCLE_EXACT_CLOCKS=1, which keep the metavalue rules and copied clocks of the normal
mode. `--contract default` runs plain `--cycle` (two-state metavalues, a copied clock is its
source: docs/cycle-mode.rst "The contract in short") and also reports the region under the exact
rules: where the two reports agree the test must pass as under the exact gate; where they differ
a difference of the output is bucket `contract` (listed, not failing) and only a crash fails it. A
failed check (E3009) is compared there by its code and source place only (no values in its message).
"""
import argparse, atexit, os, shutil, zlib, concurrent.futures as cf, difflib, json, re, sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import difftest as D  # noqa: E402

ROOT = D.ROOT
CYCLE_DIR = ROOT / "crates" / "yavs" / "tests" / "cycle"
BENCH_DIR = ROOT / "tools" / "bench" / "designs"
CYC = "$YAVS_RUN_FLAGS"  # the one place `yavs run` takes its extra flags in D.YAVS
NORMAL = D.YAVS
CYCLE = D.YAVS.replace(CYC, "--cycle " + CYC)
EXACT = "--cycle-exact-meta"  # with YAVS_CYCLE_EXACT_CLOCKS=1: the exact rules (`exact`)
CRASH = re.compile(r"E9999|this is a bug in yavs|panicked")


def exact(on):
    """Run every cycle run under the exact rules (`--contract exact`) or the default contract."""
    global CYCLE
    CYCLE = D.YAVS.replace(CYC, "--cycle " + (EXACT + " " if on else "") + CYC)
    if on:
        os.environ["YAVS_CYCLE_EXACT_CLOCKS"] = "1"
    else:
        os.environ.pop("YAVS_CYCLE_EXACT_CLOCKS", None)

# crates/yavs/tests/cycle/*.vhd: file -> [(top, extra run flags, text edit)]. numeric.vhd, calls.vhd,
# recarr.vhd, slices.vhd, ram.vhd and reclit.vhd have the width token W, constelem.vhd also string
# literals of that width; subset.vhd is a checker test (never run by its test) and stays out.
W = re.compile(r"(?<![\w])W(?![\w])")


def pats(s, w):
    """`s` at width `w` with the string literals PAT1 to PAT3 filled in as the cargo test does."""
    s = W.sub(str(w), s)
    for k in (1, 2, 3):
        s = s.replace(f"PAT{k}", '"' + "".join("1" if (i * i + i * k + k) % 5 < 2 else "0" for i in range(w)) + '"')
    return s


CYCLE_TESTS = {
    "calls.vhd": [("cycle_calls", "", lambda s, w=w: W.sub(str(w), s)) for w in (1, 64, 65)],
    "clocks.vhd": [("cycle_clocks", "", None)],
    "cmpcount.vhd": [("cycle_cmpcount", "", None)],
    "consts.vhd": [("cycle_consts", "", None)],
    "constcall.vhd": [("cycle_constcall", "", None)],
    "constelem.vhd": [("cycle_constelem", "", lambda s, w=w: pats(s, w)) for w in (1, 64, 65)],
    "elempart.vhd": [("cycle_elempart", "", lambda s, w=w: W.sub(str(w), s)) for w in (64, 65, 128)],
    "export.vhd": [("cycle_export", "", None)],
    "fsmrec.vhd": [("cycle_fsmrec", "", None)],
    "handshake.vhd": [("cycle_handshake", "", None)],
    "history.vhd": [("cycle_history", "", None)],
    "initunset.vhd": [("initunset", f, None) for f in ("--init-unset=0", "--init-unset=random:5")],
    "inputs.vhd": [("cycle_inputs", "", None)],
    "inreset.vhd": [("cycle_inreset", "", None)],
    "lazy.vhd": [("cycle_lazy", "", None)],
    "lazyrst.vhd": [("cycle_lazyrst", "", None)],
    "intelem1.vhd": [("cycle_intelem1", "", None)],
    "intarr.vhd": [("cycle_intarr", "", None)],
    "ints.vhd": [("cycle_ints", "", None)],
    "joinif.vhd": [("cycle_joinif", f"-gW={w}", None) for w in (1, 64, 65)],
    "numeric.vhd": [("cycle_ops2", "", lambda s, w=w: W.sub(str(w), s)) for w in (8, 63, 65)],
    "partial.vhd": [("cycle_partial", "", None)],
    "ports.vhd": [("cycle_ports", "", None)],
    "quiet.vhd": [("cycle_quiet", "", None)],
    "recreset.vhd": [("cycle_recreset", "", None)],
    "recarr.vhd": [("cycle_recarr", "", lambda s, w=w: W.sub(str(w), s)) for w in (1, 64, 65)],
    "reclit.vhd": [("cycle_reclit", "", lambda s, w=w: W.sub(str(w), s)) for w in (1, 64, 65)],
    "reset.vhd": [("cycle_reset", "", None), ("cycle_reset", "-gCOMB=true", None)],
    "pulse.vhd": [("cycle_pulse", "", None)],
    "runreset.vhd": [("cycle_runreset", "", None)],
    "selfassign.vhd": [("cycle_selfassign", f"-gW={w}", None) for w in (1, 64, 65)],
    "shapes.vhd": [("cycle_shapes", "", None)],
    "shiftfill.vhd": [("cycle_shiftfill", f"-gW={w}", None) for w in (1, 64, 65)],
    "skip.vhd": [("cycle_skip", "", None)],
    "ram.vhd": [("cycle_ram", "", lambda s, w=w: W.sub(str(w), s)) for w in (1, 64, 65)],
    "slices.vhd": [("cycle_slices", "", lambda s, w=w: W.sub(str(w), s)) for w in (1, 64, 65)],
    "shiftop.vhd": [("cycle_shiftop", "", lambda s, w=w: W.sub(str(w), s)) for w in (1, 64, 65)],
    "qualrec.vhd": [("cycle_qualrec", "", lambda s, w=w: W.sub(str(w), s)) for w in (1, 64, 65)],
    "elabcall.vhd": [("cycle_elabcall", "", lambda s, w=w: W.sub(str(w), s)) for w in (1, 64, 65)],
    "waitfilter.vhd": [("cycle_waitfilter", "", lambda s, w=w: W.sub(str(w), s)) for w in (1, 64, 65)],
}


def header(top, **kw):
    return {"top": top, "std": "2008", "stop": "", "exit": 0, "ref": "sim1", "ignore": "", "res": "",
            "relaxed": None, "psl": None, "spin": "", **kw}


def items(only):
    """(name, file, header, template pair) of every design of the gate."""
    out = []
    if only in ("all", "corpus"):
        for p in sorted(D.CORPUS.rglob("*.vhd")):
            h = D.parse_header(p)
            if h["relaxed"] is not None:
                h["relaxed_flag"] = D.RELAXED["yavs"]
            out.append((str(p.relative_to(D.CORPUS)), p, h, (NORMAL, CYCLE)))
    if only in ("all", "cycle"):
        for f, tops in CYCLE_TESTS.items():
            for i, (top, flags, edit) in enumerate(tops):
                p = CYCLE_DIR / f
                if edit:
                    p = D.CACHE / "gen" / f"{p.stem}_{i}.vhd"
                    p.parent.mkdir(parents=True, exist_ok=True)
                    p.write_text(edit((CYCLE_DIR / f).read_text()))
                add = f" {flags}" if flags else ""
                out.append((f"cycle/{f}:{top}{add}" + (f"#{i}" if edit else ""), p, header(top),
                            (NORMAL.replace(CYC, CYC + add), CYCLE.replace(CYC, CYC + add))))
    if only in ("all", "bench"):
        for p in sorted(BENCH_DIR.glob("*.vhd")):
            out.append((f"bench/{p.name}", p, header(f"bench_{p.stem}", res="fs"), (NORMAL, CYCLE)))
    return out


# ---------------------------------------------------------------- the region report
def split_report(err):
    """(stderr without the region summary or report, its lines). The summary line of `--cycle` and
    the report of `--cycle-report` both start with "cycle mode: N of M processes"."""
    lines = err.splitlines()
    rep, i = [], len(lines)
    for k, l in enumerate(lines):  # it comes after the warnings of elaboration
        if re.match(r"cycle mode: \d+ of \d+ processes", l):
            i = k + 1
            rep = [l]
            while i < len(lines) and lines[i].startswith("  "):
                rep.append(lines[i])
                i += 1
            break
    lines = lines[: i - len(rep)] + lines[i:]
    # W4101 (asserts and reports cycle mode drops) is cycle mode's own: its block up to the blank line.
    plain = [re.sub(r"\x1b\[[0-9;]*m", "", l) for l in lines]  # a recorded run may have colour
    if (w := next((k for k, l in enumerate(plain) if l.startswith("warning[W4101]")), None)) is not None:
        end = next((k for k in range(w, len(lines)) if not plain[k].strip()), len(lines))
        lines = lines[:w] + lines[end + 1 :]
    return "\n".join(l for l in lines if not l.startswith("cycle mode: ")), rep  # not the metavalue summary


def held_asserts(rep):
    """(members, dropped) of a report: the LRM paths of the processes that hold asserts and reports
    compiled to nothing ('-' for an unlabelled one), and the number of those asserts and reports."""
    members, dropped, on = set(), 0, False
    for l in rep[1:]:
        if re.match(r"^  \S", l):
            on = " not evaluated in cycle mode, held by " in l
        elif on:
            n, _, name = l.strip().partition("  ")
            bare = re.sub(r" \(\S+:\d+\)$", "", name)  # a labelled process shows its place too
            members.update({D.lrm_path(bare), "-"} if bare != name else {D.lrm_path(name)})
            dropped += int(n)
    return members, dropped


def count_members(rep):
    m = rep and re.match(r"cycle mode: (\d+) of (\d+) processes", rep[0])
    return (int(m.group(1)), int(m.group(2))) if m else (0, 0)


# ---------------------------------------------------------------- classification
def stamp(lines):
    """Drop the delta of every report stamp (documented loss: cycle mode has other delta counts)."""
    out = []
    for l in lines:
        f = l.split("\t")
        if f[0] == "diag" and len(f) == 6:
            f[3] = "-"
        out.append("\t".join(f))
    return out


def settled(lines, lost):
    """What a testbench sees when cycle mode has fewer delta cycles: of the reports of one process
    at one time only the last delta's, the other lines as they are. Lost member reports go."""
    last, rest = {}, []
    for l in lines:
        f = l.split("\t")
        if f[0] == "diag" and len(f) == 6:
            if lost(f[4]):
                continue
            k = (f[2], f[4])
            if k not in last or last[k][0] != f[3]:
                last[k] = (f[3], [])
            last[k][1].append(f[1] + "\t" + f[5])
        else:
            rest.append(l)
    return rest, {k: v[1] for k, v in last.items()}


def classify(nl, cl, lost):
    """nl, cl: canonical lines of normal and cycle mode; lost(path) says whether a report of a
    process at that path may be missing. Returns (bucket, why): identical, documented (the
    differences are what the mode drops by design, shown by the lines themselves) or differs."""
    if nl == cl:
        return "identical", ""
    ns, cs = stamp(nl), stamp(cl)
    if ns == cs:
        return "documented", "delta counts of report stamps"
    ops = [o for o in difflib.SequenceMatcher(None, ns, cs, autojunk=False).get_opcodes() if o[0] != "equal"]
    sev = lambda l: l.split("\t")[1]
    # A member's lost report is a deletion of diag lines only. A lost failure stops the normal run:
    # what follows it is not comparable. A lost error or failure may change the exit status (line 0).
    lostd = lambda i: ns[i].startswith("diag\t") and lost(ns[i].split("\t")[4])  # a member's report
    gone = [i for tag, i1, i2, _, _ in ops if tag in ("delete", "replace") for i in range(i1, i2) if lostd(i)]
    stop = min((i for i in gone if sev(ns[i]) in ("failure", "fatal")), default=len(ns))
    bad = any(sev(ns[i]) in ("error", "failure", "fatal") for i in gone)
    ok = lambda i: i > stop or (i == 0 and bad) or lostd(i)  # lines cycle mode may lack
    for tag, i1, i2, j1, j2 in ops:
        if tag == "insert" and i1 >= stop:  # after a lost stop the run goes on
            continue
        if tag in ("delete", "replace") and all(ok(i) for i in range(i1, i2)) and (tag == "delete" or i2 > stop or j2 <= 1):
            continue
        break
    else:
        return "documented", "reports and asserts of member processes" + (" (a lost failure stopped the normal run: the rest is not compared)" if stop < len(ns) else "")
    if settled(nl, lost) == settled(cl, lambda p: False):
        return "documented", "intermediate delta cycles are not visible (the settled values are equal)"
    return "differs", "the output differs\n" + first_diff(ns, cs)


# ---------------------------------------------------------------- contract classes (README "Contract classes")
IEEE_MSG = re.compile(r"^[A-Z][A-Z0-9_]*\.\S.*(metavalue|null) detected|^[A-Z][A-Z0-9_]*\.\S.*non logical")
LIB_META = re.compile(r"^[A-Z][A-Z0-9_]*\.\S.*metavalue detected")
RECORD = re.compile(r"^\*\* (\w+): \S+: (.*)$")
DIGITS = set("0123456789ABCDEF")


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


def drop_ieee(lines, members):
    """Without the library warnings of cycle-based processes (A6, A33: none are given at any severity)."""
    keep = []
    for l in lines:
        f = l.split("\t")
        if not (f[0] == "diag" and len(f) == 6 and f[4] in members and IEEE_MSG.match(f[5].replace("\\n", " "))):
            keep.append(l)
    return keep


def read_down(path):
    """The names of YAVS_CYCLE_DOWNSTREAM's file (processes outside that read what the region exports),
    as the console prints them, with and without their `(file:line)`."""
    return read_down_text(Path(path).read_text() if Path(path).is_file() else "")


def read_down_text(text):
    names = text.splitlines()
    return set(names) | {re.sub(r" \(\S+:\d+\)$", "", n) for n in names}


def drop_down(text, down):
    """(text without the library metavalue warnings of the processes in `down`, the records taken out).
    A record is its `** Severity: time: message` line and the `    Process NAME` line after it."""
    out, gone, ls, i = [], [], text.splitlines(keepends=True), 0
    while i < len(ls):
        m = RECORD.match(ls[i])
        if m and m.group(1) in ("Note", "Warning") and i + 1 < len(ls) and ls[i + 1].startswith("    Process ") \
                and LIB_META.match(m.group(2)) and ls[i + 1][len("    Process "):].strip() in down:
            gone.append(ls[i] + ls[i + 1])
            i += 2
            continue
        out.append(ls[i])
        i += 1
    return "".join(out), gone


def meta_pair(n, c):
    """Whether line c (cycle mode) is line n (normal mode) with its metavalues read two-state: of one
    length, and where they differ n holds 'U', 'X', 'Z', 'W' or '-' and c a digit (a bit, or a hex or
    octal digit of the value computed from it), or n 'H' and c '1', or n 'L' and c '0'. Never the reverse.
    A '-' counts only as a logic value: `'-'`, or in a run of logic values holding a letter one (not the
    sign of a number)."""
    def dash(i):
        if n[i - 1:i + 2] == "'-'":
            return True
        j, k = i, i
        while j > 0 and n[j - 1] in "UXZWLH01-":
            j -= 1
        while k < len(n) - 1 and n[k + 1] in "UXZWLH01-":
            k += 1
        return bool(re.search("[UXZWLH]", n[j:k + 1]))
    return len(n) == len(c) and all(a == b or (a in "UXZW" and b in DIGITS) or (a == "-" and b in DIGITS and dash(i))
                                    or (a, b) in (("H", "1"), ("L", "0")) for i, (a, b) in enumerate(zip(n, c)))


def meta_align(nl, cl):
    """(cl with every line meta_pair explains replaced by its normal line, how many). Lines are paired by
    their text with digits and metavalues masked, in order."""
    shape = lambda l: re.sub(r"[0-9A-FUXZWHL-]", "#", l)
    out, k = list(cl), 0
    sm = difflib.SequenceMatcher(None, [shape(x) for x in nl], [shape(x) for x in cl], autojunk=False)
    for tag, i1, i2, j1, j2 in sm.get_opcodes():
        if tag == "equal":
            for i, j in zip(range(i1, i2), range(j1, j2)):
                if nl[i] != cl[j] and meta_pair(nl[i], cl[j]):
                    out[j], k = nl[i], k + 1
    return out, k


def meta_evidence(cerr, rep):
    """Whether the region read a metavalue two-state: the run's count line, a metavalue literal of a member
    or an input nothing drives in its report. Without it no line is explained as a metavalue."""
    return bool(re.search(r"^cycle mode: \d+ metavalue element", cerr, re.M)) or any(
        "with a metavalue read as 0" in l or "driven by nothing" in l or "the region reads without a driver" in l for l in rep)


def lib_class(gone_n, gone_c):
    """The class of library warnings of processes outside taken out, when the two runs' differ."""
    return ["library metavalue warnings of processes outside that read the region"] if sorted(gone_n) != sorted(gone_c) else []


def contract_lines(nrc, nout, nerr, crc, cout, cerr, raw_cerr, rep, down, default):
    """(normal lines, cycle lines, classes used): the canonical lines of both runs without the library
    warnings of members (A33) and, where the runs differ in them, those of processes outside reading
    the region (`down`); under the default contract with the cycle line of a metavalue read two-state
    replaced by the normal one (`meta_pair`) when the run shows it read one (`meta_evidence`)."""
    nout, a = drop_down(nout, down)
    nerr, b = drop_down(nerr, down)
    cout, c = drop_down(cout, down)
    cerr, d = drop_down(cerr, down)
    classes = lib_class(a + b, c + d)
    mem = member_paths(rep)
    nl = drop_ieee(D.normalise(nrc, nout, nerr, yavs=True), mem)
    cl = drop_ieee(D.normalise(crc, cout, cerr, yavs=True), mem)
    if default and nl != cl and meta_evidence(raw_cerr, rep):
        cl, k = meta_align(nl, cl)
        if k:
            classes.append("metavalues read as 0 or 1")
    return nl, cl, classes


def contract_bucket(bucket, why, classes, default):
    """A test the classes explained: `contract` under the default contract, `documented` under the exact
    one (only the library warning class applies there); else as it was."""
    if bucket in ("identical", "documented") and classes:
        return ("contract" if default else "documented"), "explained by " + "; ".join(classes) + (f" ({why})" if why else "")
    return bucket, why


def load_expected():
    """name -> {crc of the diff: cause} of cyclegate.expected: differences someone reviewed."""
    out = {}
    for l in (HERE / "cyclegate.expected").read_text().splitlines():
        if l.strip() and not l.startswith("#"):
            name, crc, cause = l.split("\t", 2)
            for c in crc.split(","):  # several per setting (a line each per contract where the causes differ)
                out.setdefault(name, {})[c] = cause
    return out


def known(r, expected):
    """A difference that is listed with the same diff is known; a listed one that changed is not."""
    if r["bucket"] == "differs" and r["name"] in expected:
        crcs = expected[r["name"]]
        if r.get("crc") in crcs:
            r.update(bucket="known", why=crcs[r["crc"]])
        else:
            r["why"] = f"the listed difference changed (checksum {', '.join(crcs)} before, {r.get('crc')} now): review it\n" + r["why"]


def first_diff(a, b):
    d = list(difflib.unified_diff(a, b, "normal", "cycle", lineterm="", n=1))
    return "\n".join(d[:30])


# ---------------------------------------------------------------- one test
def one(item, jobs_args):
    name, vhd, hdr, (tn, tc) = item
    base = D.CACHE / "gate" / re.sub(r"\W+", "_", name)
    res = {"name": name, "bucket": "", "why": "", "members": 0, "procs": 0}
    try:
        outs = []
        tr = tc.replace("--cycle ", "--cycle-report ", 1)  # the members: analysis only
        runs = [("normal", tn), ("cycle", tc), ("report", "export YAVS_CYCLE_DOWNSTREAM=down.txt; " + tr)]
        for kind, t in runs:
            if kind == "cycle" and jobs_args.warm:
                # A cold run first, then the run compared in a fresh directory with its library: the
                # region's code from the compilation cache (files the test writes start afresh).
                cold = D.normalise(*D.run(t, vhd, hdr, base / "cold", 1, "", not jobs_args.no_sandbox), yavs=True)
                shutil.rmtree(base / kind, ignore_errors=True)
                if (base / "cold" / "lib").is_dir():  # (none when the analysis failed)
                    shutil.copytree(base / "cold" / "lib", base / kind / "lib")
                store = base / kind / "lib" / "yavs-cache" / "cycle"
                res["cached"] = any(store.glob("cycle-*.img"))
                analyses = len(list(store.glob("cyc3-*.img")))
            rc, out, err = D.run(t, vhd, hdr, base / kind, 1, "", not jobs_args.no_sandbox,
                                 fresh=not (kind == "cycle" and jobs_args.warm))
            if kind == "cycle" and jobs_args.warm:
                # The analysis came from the cache when the cold run kept one and the warm run kept no other.
                res["analysis"] = analyses > 0 and len(list(store.glob("cyc3-*.img"))) == analyses
                warm = D.normalise(rc, out, err, yavs=True)
                if warm != cold:
                    res.update(bucket="differs", why="the warm run (region code from the cache) differs from the cold run\n" + first_diff(cold, warm))
                    res["dir"] = str(base)
                    return res
            outs.append((rc, out, err))
        (nrc, nout, nerr), (crc, cout, cerr), (_, _, rerr) = outs[:3]
        if CRASH.search(cerr + rerr) or (crc == 124 and nrc != 124):
            res.update(bucket="differs", why="cycle mode crashed or timed out\n" + "\n".join(cerr.splitlines()[-20:]))
            res["dir"] = str(base)
            return res
        if nrc in (97, 98, 124) and hdr["exit"] != nrc:
            res.update(bucket="skipped", why={97: "analysis fails", 98: "elaboration fails", 124: "timeout"}[nrc] + " in the normal mode")
        else:
            cerr, _ = split_report(cerr)
            rep = split_report(rerr)[1]
            members, dropped = held_asserts(rep)
            res["members"], res["procs"] = count_members(rep)
            if crc != 0 and nrc == 0 and re.search(r"E41\d\d", cerr) and "E9999" not in cerr:
                res.update(bucket="refused", why=re.search(r"E41\d\d", cerr).group(0))
            else:
                default = jobs_args.contract == "default"
                if default:  # a failed check by code and place (A34)
                    nerr, cerr = D.code_loc(nerr), D.code_loc(cerr)
                nl, cl, classes = contract_lines(nrc, nout, nerr, crc, cout, cerr, outs[1][2], rep,
                                                 read_down(base / "report" / "down.txt"), default)
                lost = lambda path: dropped > 0 and (path in members or (path == "-" and "-" in members))
                res["bucket"], res["why"] = contract_bucket(*classify(nl, cl, lost), classes, default)
                res["classes"] = classes
        if res["bucket"] == "differs":
            res["crc"] = "%08x" % zlib.crc32("\n".join(difflib.unified_diff(nl, cl, lineterm="", n=1)).encode())
            res["dir"] = str(base)
        else:
            D.drop(base)
        return res
    except Exception as e:  # report, never hide
        return {**res, "bucket": "differs", "why": f"harness error: {e!r}"}


def selftest():
    e = "error[E3009]: value 8 outside of INTEGER range 0 to 7\n  --> /a/b/t.vhd:12:5\n   |"
    assert D.code_loc(e).splitlines()[:2] == ["error[E3009]: E3009 at t.vhd:12:5", "  --> /a/b/t.vhd:12:5"]
    fat = lambda e: D.normalise(1, "", D.code_loc(e + "\n   = note: at 5 ns, delta 1, in process t.p"), yavs=True)
    assert fat(e) == fat(e.replace("value 8 outside", "value outside").replace("/a/b/", "/c/"))
    assert fat(e) != fat(e.replace(":12:5", ":13:5")) and fat(e) != fat(e.replace("E3009", "E3010"))
    n = ["exit\t0", "diag\tnote\t5\t1\t:tb:p\thi", "diag\tnote\t9\t1\t:tb:m\tmember", "out\tdone"]
    lost = lambda p: p == ":tb:m"
    assert classify(n, n, lost)[0] == "identical"
    assert classify(n, [l.replace("\t1\t", "\t2\t") for l in n], lost)[0] == "documented"
    assert classify(n, n[:2] + n[3:], lost)[0] == "documented"  # a member's report is gone
    assert classify(n, n[:1] + n[2:], lost)[0] == "differs"  # a testbench report is gone
    assert classify(n, n[:3] + ["out\tother"], lost)[0] == "differs"
    d = ["exit\t0", "diag\tnote\t5\t1\t:tb:p\ta", "diag\tnote\t5\t2\t:tb:p\tb", "out\tdone"]
    assert classify(d, d[:1] + d[2:], lost)[0] == "documented"  # the first of two deltas is not seen
    f = ["exit\t1", "diag\tfailure\t9\t1\t:tb:m\tbad"]
    assert classify(f, ["exit\t0", "out\tmore"], lost)[0] == "documented"  # its stop is lost
    assert classify(f, ["exit\t0"], lambda p: False)[0] == "differs"
    e = ["exit\t1", "diag\terror\t5\t1\t:tb:m\tbad", "out\tdone"]
    assert classify(e, ["exit\t0", "out\tdone"], lost)[0] == "documented"  # a lost error changes the exit status
    assert classify(e, ["exit\t0", "out\tdone"], lambda p: False)[0] == "differs"
    rep = "cycle mode: 2 of 5 processes run cycle-based, on the rising edge of clk\n  3 processes stay in the event kernel:\n    E4119 a latch (3 processes):\n      tb.l (a.vhd:9)  E4119 it is a latch (a.vhd:9)\n  3 asserts and reports not evaluated in cycle mode, held by 2 cycle-based processes:\n       2  tb.dut.p\n       1  tb.q (a.vhd:3)\n  each assert and report not evaluated, by severity (2 error, 1 note):\n    error    a.vhd:4  tb.dut.p\n  subset: 2\n  cycle-based, 2 processes:\n    tb.dut.p  clocked\nreal\n"
    e, r = split_report(rep)
    assert (e, held_asserts(r), count_members(r)) == ("real", ({":tb:dut:p", ":tb:q", "-"}, 3), (2, 5)), (e, held_asserts(r))
    e, r = split_report("cycle mode: 2 of 5 processes run cycle-based, 3 in the event kernel (see --cycle-report)\nreal\n")
    assert (e, held_asserts(r), count_members(r)) == ("real", (set(), 0), (2, 5)), e
    w = "warning[W4101]: 1 assert or report of severity failure is not evaluated in cycle mode\n |\n = note: a.vhd:4: severity failure, in tb.dut.p\n = help: these do not stop\n\n"
    e, r = split_report("cycle mode: 2 of 5 processes run cycle-based\n" + w + "real\n")
    assert (e, count_members(r)) == ("real", (2, 5)), e
    c = "\x1b[1m\x1b[33mwarning[W4101]\x1b[0m\x1b[1m: 2 asserts\x1b[0m\n \x1b[1m\x1b[34m|\x1b[0m\n = note: x\n\x1b[0m\n"
    assert split_report("cycle mode: 2 of 5 processes run cycle-based\n" + c + "real\n")[0] == "real"
    exact(True)
    assert EXACT in CYCLE and os.environ.get("YAVS_CYCLE_EXACT_CLOCKS") == "1"
    exact(False)
    assert EXACT not in CYCLE and "YAVS_CYCLE_EXACT_CLOCKS" not in os.environ
    # The contract classes: each explains only its own shape, one way.
    assert meta_pair("q=ZZ x='U' h='H'", "q=3F x='0' h='1'") and meta_pair("v=0X", "v=0A")
    assert not meta_pair("q=3F", "q=ZZ") and not meta_pair("q=10", "q=11") and not meta_pair("q=Z", "q=00")
    assert not meta_pair("h='H'", "h='0'") and not meta_pair("t=5", "t=6")
    assert not meta_pair("q=-7", "q=17") and not meta_pair("i=-1 j=2", "i=01 j=2")  # a sign is no metavalue
    assert meta_pair("v='-' w=U-01", "v='0' w=0101") and not meta_pair("w=0-01", "w=0101")
    n = ["exit\t0", "out\t1 q=ZZ r=05", "out\t2 q=ZZ r=06"]
    assert meta_align(n, ["exit\t0", "out\t1 q=00 r=05", "out\t2 q=1F r=06"]) == (n, 2)
    bad = ["exit\t0", "out\t1 q=00 r=05", "out\t2 q=00 r=07"]  # a wrong bit where the normal mode has none
    assert meta_align(n, bad)[0] != n and classify(n, meta_align(n, bad)[0], lambda p: False)[0] == "differs"
    assert meta_evidence("cycle mode: 3 metavalue elements read as 0 (3 at initialisation)", [])
    assert not meta_evidence("cycle mode: 2 of 5 processes run cycle-based", ["  cycle-based, 2 processes:"])
    w = "** Warning: 0fs+1: NUMERIC_STD.TO_INTEGER: metavalue detected, returning 0\n    Process tb.u.rd (a.vhd:9)\n"
    t = "** Note: 5ns+0: done\n    Process tb.p\n"
    assert drop_down(w + t, read_down_text("tb.u.rd (a.vhd:9)\n")) == (t, [w])
    assert drop_down(w + t, read_down_text("tb.other (a.vhd:3)\n"))[0] == w + t  # it reads nothing the region exports
    assert drop_down(t, {"tb.p"})[0] == t  # only library metavalue warnings
    assert lib_class([w], []) and not lib_class([w], [w])
    assert contract_bucket("identical", "", ["x"], True)[0] == "contract"
    assert contract_bucket("identical", "", ["x"], False)[0] == "documented"
    assert contract_bucket("differs", "y", ["x"], True)[0] == "differs"
    import cyclefuzz as F  # the fuzzer's copied-clock variant
    t = ("architecture a of fz_top is\nbegin\n  dk1 <= clk0 when G_B else clk1;\n  signal x : std_ulogic;\n"
         "  process (dk1) begin if rising_edge(dk1) then q <= d; end if; end process;\n"
         "  u : entity work.fz_s port map (clk0 => dk1, dk1 => dk1);\nend architecture;\n"
         "architecture a of fz_tb is\nbegin\n  dut : entity work.fz_top\n    generic map (G_K => 1, G_B => true)\n"
         "    port map (dk1 => dk1);\n      write(l, string'(\" dk1=\"));\nend architecture;\n")
    v = F.copy_variant(t)
    assert "rising_edge(clk0)" in v and "(clk0 => clk0, dk1 => clk0)" in v and "dk1 <= clk0 when" in v, v
    assert "port map (dk1 => dk1)" in v and '" dk1="' in v
    two = ("library ieee;\nentity fz_s is end;\narchitecture a of fz_s is\nbegin\n  dk2 <= clk0 when G_B else clk1;\n"
           "  process (dk2) begin end process;\nend architecture;\narchitecture a of fz_top is\nbegin\n"
           "  u1 : entity work.fz_s generic map (G_B => true);\n  u2 : entity work.fz_s generic map (G_B => false);\n"
           "end architecture;\n")
    v = F.copy_variant(two)  # instances of both values: a copy of the entity for the false ones
    assert "entity work.fz_s_f generic map (G_B => false)" in v and "process (clk1)" in v and "process (clk0)" in v, v
    assert F.copy_variant(t.replace("dk1 <= clk0 when G_B else clk1", "dk1 <= clk0 and e")) is None
    print("selftest ok")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--only", choices=("all", "corpus", "cycle", "bench"), default="all")
    ap.add_argument("--filter", default="", help="regex on the test name")
    ap.add_argument("--jobs", type=int, default=4, help=f"parallel tests, two runs each (max {D.MAX_JOBS})")
    ap.add_argument("--no-sandbox", action="store_true")
    ap.add_argument("--json", type=Path, help="write one JSON result per test to this file")
    ap.add_argument("--selftest", action="store_true")
    ap.add_argument("--warm", action="store_true",
                    help="run each cycle run first in a library of its own, then the compared run on a copy of that "
                         "library, with the region's code from the compilation cache, and fail when the two differ")
    ap.add_argument("--contract", choices=("exact", "default"), default="exact",
                    help="exact (default): cycle runs keep the normal mode's metavalue rules and copied clocks "
                         "(--cycle-exact-meta, YAVS_CYCLE_EXACT_CLOCKS=1); default: plain --cycle, a difference the "
                         "contract classes explain fully is bucket contract (README), any other fails")
    ap.add_argument("-v", "--verbose", action="store_true", help="one line per test with its membership")
    a = ap.parse_args()
    if a.selftest:
        return selftest()
    exact(a.contract == "exact")
    err = D.candidate_error("yavs")
    if err:
        sys.exit(err)
    todo = [i for i in items(a.only) if re.search(a.filter, i[0])]
    D.guard(D.CACHE)
    atexit.register(shutil.rmtree, D.CACHE / "gen", ignore_errors=True)  # runs before D.cleanup
    expected = load_expected()
    res = []
    with cf.ThreadPoolExecutor(max(1, min(a.jobs, D.MAX_JOBS))) as ex:
        try:
            for r in ex.map(lambda i: one(i, a), todo):
                res.append(r)
                known(r, expected)
                if a.verbose or r["bucket"] in ("differs", "known", "contract") or "not compared" in r["why"]:
                    print(f"{r['bucket'].upper():10} {r['name']}  [{r['members']}/{r['procs']} in the region]"
                          + (f" {r['why'].splitlines()[0]}" if r["why"] else ""), flush=True)
                if r["bucket"] == "known":
                    D.drop(Path(r["dir"]))
                if r["bucket"] == "differs":
                    print(f"    to list it after a review: {r['name']}\t{r.get('crc', '?')}\t<cause>")
                    print("\n".join("    " + l for l in r["why"].splitlines()[1:]), flush=True)
        except BaseException:
            ex.shutdown(wait=True, cancel_futures=True)
            raise
    if a.json:
        a.json.write_text("\n".join(json.dumps(r) for r in res) + "\n")
    mine = lambda cause: cause.startswith("default contract") == (a.contract == "default")  # the entries of this contract
    for r in res:
        if any(mine(c) for c in expected.get(r["name"], {}).values()) and r["bucket"] in ("identical", "documented", "contract"):
            print(f"note: {r['name']} no longer differs: remove its entry for this contract from cyclegate.expected")
    c = {b: sum(r["bucket"] == b for r in res) for b in ("identical", "documented", "known", "contract", "differs", "refused", "skipped")}
    ran = [r for r in res if r["bucket"] in ("identical", "documented", "known", "contract", "differs")]
    with_m = sum(r["members"] > 0 for r in ran)
    print(f"cycle gate: {len(res)} tests: {c['identical']} identical, {c['documented']} documented losses only, "
          f"{c['known']} known differences, "
          + (f"{c['contract']} differ by the default contract, " if a.contract == "default" else "")
          + f"{c['differs']} differ otherwise, {c['refused']} refused, {c['skipped']} skipped; "
          f"{with_m} of {len(ran)} compared tests have at least one process in the region "
          f"({sum(r['members'] for r in ran)} of {sum(r['procs'] for r in ran)} processes)"
          + (f"; warm: {sum(bool(r.get('cached')) for r in res)} tests ran with region code in the cache, "
             f"{sum(bool(r.get('analysis')) for r in res)} with the analysis from it" if a.warm else ""))
    sys.exit(1 if c["differs"] else 0)


if __name__ == "__main__":
    main()
