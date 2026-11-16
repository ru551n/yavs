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
"""
import argparse, atexit, shutil, zlib, concurrent.futures as cf, difflib, json, re, sys
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
    "constelem.vhd": [("cycle_constelem", "", lambda s, w=w: pats(s, w)) for w in (1, 64, 65)],
    "export.vhd": [("cycle_export", "", None)],
    "fsmrec.vhd": [("cycle_fsmrec", "", None)],
    "handshake.vhd": [("cycle_handshake", "", None)],
    "history.vhd": [("cycle_history", "", None)],
    "inputs.vhd": [("cycle_inputs", "", None)],
    "inreset.vhd": [("cycle_inreset", "", None)],
    "lazy.vhd": [("cycle_lazy", "", None)],
    "intarr.vhd": [("cycle_intarr", "", None)],
    "ints.vhd": [("cycle_ints", "", None)],
    "numeric.vhd": [("cycle_ops2", "", lambda s, w=w: W.sub(str(w), s)) for w in (8, 63, 65)],
    "partial.vhd": [("cycle_partial", "", None)],
    "ports.vhd": [("cycle_ports", "", None)],
    "quiet.vhd": [("cycle_quiet", "", None)],
    "recreset.vhd": [("cycle_recreset", "", None)],
    "recarr.vhd": [("cycle_recarr", "", lambda s, w=w: W.sub(str(w), s)) for w in (1, 64, 65)],
    "reclit.vhd": [("cycle_reclit", "", lambda s, w=w: W.sub(str(w), s)) for w in (1, 64, 65)],
    "reset.vhd": [("cycle_reset", "", None), ("cycle_reset", "-gCOMB=true", None)],
    "shapes.vhd": [("cycle_shapes", "", None)],
    "skip.vhd": [("cycle_skip", "", None)],
    "ram.vhd": [("cycle_ram", "", lambda s, w=w: W.sub(str(w), s)) for w in (1, 64, 65)],
    "slices.vhd": [("cycle_slices", "", lambda s, w=w: W.sub(str(w), s)) for w in (1, 64, 65)],
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


def load_expected():
    """name -> (crc of the diff, cause) of cyclegate.expected: differences someone reviewed."""
    out = {}
    for l in (HERE / "cyclegate.expected").read_text().splitlines():
        if l.strip() and not l.startswith("#"):
            name, crc, cause = l.split("\t", 2)
            out[name] = (crc, cause)
    return out


def known(r, expected):
    """A difference that is listed with the same diff is known; a listed one that changed is not."""
    if r["bucket"] == "differs" and r["name"] in expected:
        crc, cause = expected[r["name"]]
        if crc == r.get("crc"):
            r.update(bucket="known", why=cause)
        else:
            r["why"] = f"the listed difference changed (checksum {crc} before, {r.get('crc')} now): review it\n" + r["why"]


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
        for kind, t in (("normal", tn), ("cycle", tc), ("report", tr)):
            if kind == "cycle" and jobs_args.warm:
                # A cold run first, then the run compared in a fresh directory with its library: the
                # region's code from the compilation cache (files the test writes start afresh).
                cold = D.normalise(*D.run(t, vhd, hdr, base / "cold", 1, "", not jobs_args.no_sandbox), yavs=True)
                shutil.rmtree(base / kind, ignore_errors=True)
                if (base / "cold" / "lib").is_dir():  # (none when the analysis failed)
                    shutil.copytree(base / "cold" / "lib", base / kind / "lib")
                res["cached"] = any((base / kind / "lib" / "yavs-cache" / "cycle").glob("cycle-*.img"))
            rc, out, err = D.run(t, vhd, hdr, base / kind, 1, "", not jobs_args.no_sandbox,
                                 fresh=not (kind == "cycle" and jobs_args.warm))
            if kind == "cycle" and jobs_args.warm:
                warm = D.normalise(rc, out, err, yavs=True)
                if warm != cold:
                    res.update(bucket="differs", why="the warm run (region code from the cache) differs from the cold run\n" + first_diff(cold, warm))
                    res["dir"] = str(base)
                    return res
            outs.append((rc, out, err))
        (nrc, nout, nerr), (crc, cout, cerr), (_, _, rerr) = outs
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
                nl = D.normalise(nrc, nout, nerr, yavs=True)
                cl = D.normalise(crc, cout, cerr, yavs=True)
                lost = lambda path: dropped > 0 and (path in members or (path == "-" and "-" in members))
                res["bucket"], res["why"] = classify(nl, cl, lost)
        if res["bucket"] == "differs":
            res["crc"] = "%08x" % zlib.crc32("\n".join(difflib.unified_diff(nl, cl, lineterm="", n=1)).encode())
            res["dir"] = str(base)
        else:
            D.drop(base)
        return res
    except Exception as e:  # report, never hide
        return {**res, "bucket": "differs", "why": f"harness error: {e!r}"}


def selftest():
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
    rep = "cycle mode: 2 of 5 processes run cycle-based, on the rising edge of clk\n  3 processes stay in the event kernel:\n    E4119 a latch (3 processes):\n      tb.l (a.vhd:9)  E4119 it is a latch (a.vhd:9)\n  3 asserts and reports not evaluated in cycle mode, held by 2 cycle-based processes:\n       2  tb.dut.p\n       1  tb.q (a.vhd:3)\n  subset: 2\n  cycle-based, 2 processes:\n    tb.dut.p  clocked\nreal\n"
    e, r = split_report(rep)
    assert (e, held_asserts(r), count_members(r)) == ("real", ({":tb:dut:p", ":tb:q", "-"}, 3), (2, 5)), (e, held_asserts(r))
    e, r = split_report("cycle mode: 2 of 5 processes run cycle-based, 3 in the event kernel (see --cycle-report)\nreal\n")
    assert (e, held_asserts(r), count_members(r)) == ("real", (set(), 0), (2, 5)), e
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
    ap.add_argument("-v", "--verbose", action="store_true", help="one line per test with its membership")
    a = ap.parse_args()
    if a.selftest:
        return selftest()
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
                if a.verbose or r["bucket"] in ("differs", "known") or "not compared" in r["why"]:
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
    for r in res:
        if r["name"] in expected and r["bucket"] in ("identical", "documented"):
            print(f"note: {r['name']} no longer differs: remove it from cyclegate.expected")
    c = {b: sum(r["bucket"] == b for r in res) for b in ("identical", "documented", "known", "differs", "refused", "skipped")}
    ran = [r for r in res if r["bucket"] in ("identical", "documented", "known", "differs")]
    with_m = sum(r["members"] > 0 for r in ran)
    print(f"cycle gate: {len(res)} tests: {c['identical']} identical, {c['documented']} documented losses only, "
          f"{c['known']} known differences, {c['differs']} differ otherwise, {c['refused']} refused, {c['skipped']} skipped; "
          f"{with_m} of {len(ran)} compared tests have at least one process in the region "
          f"({sum(r['members'] for r in ran)} of {sum(r['procs'] for r in ran)} processes)"
          + (f"; warm: {sum(bool(r.get('cached')) for r in res)} tests ran with region code in the cache" if a.warm else ""))
    sys.exit(1 if c["differs"] else 0)


if __name__ == "__main__":
    main()
