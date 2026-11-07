#!/usr/bin/env python3
"""The ivtest regression suite (ivtest, GPL-2.0, fetched by tools/fetch-corpus.sh) against `yavs analyze`.

  tools/verilog/ivtest.py [check|bless|table|list] [--root DIR] [--yavs CMD] [-j N] [--no-wrap]

Every test of the regress lists is analyzed with the flags of its list line (-D, -I and the extra source files; -g selects the language
generation), and every other .v file of ivltests and contrib once alone. The manifest ivtest.expected has one line per run:
`OUTCOME LIST TYPE GEN FILES [-I DIR | -D NAME]...`, FILES being comma separated paths relative to the root. OUTCOME is PASS or the
sorted error codes. Tests of the lists vlg, ivl1, ivl2, v10, v11, v12, synth and msys2 are 1364 or, with -g2005-sv, -g2009 or -g2012 (and
in the lists sv and fsv), SystemVerilog (GEN `sv`); vams is Verilog-AMS (GEN `ams`). vlog95, vhdl and the vpi lists repeat or leave Verilog and are not read.
Every run adds `-I .` (the suite runs its tests from its root). TYPE is the list's expectation: normal, CO, CN, RE (compile, so analysis passes), CE (a compile error), EF, NI, TE (no expectation).

`table` prints, per list and type, how analysis and the expectation agree, from the manifest alone. A test that disagrees must carry a
reason in ivtest.triage (`TYPE:FIRSTFILE CLASS note`), otherwise `check` fails: the manifest says what yavs does, the triage says why that is fine.
Exit: 0 as expected, 1 differences, 2 usage.
"""
import argparse, collections, concurrent.futures as cf, os, re, shlex, sys, tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import vcommon  # noqa: E402

HERE = Path(__file__).resolve().parent
LISTS = ("vlg", "ivl1", "ivl2", "v10", "v11", "v12", "fsv", "synth", "msys2", "sv", "vams")
OK_TYPES = ("normal", "CO", "CN", "RE")
SV_FLAGS = ("-g2005-sv", "-g2009", "-g2012")
SV_CODES = {"E4002", "E4003"}
# Why a disagreement is acceptable: sem = a rule M0 does not check (binding, types, widths, elaboration), ext = an extension or a
# rule outside 1364-2005, ctx = needs a define, an include or a build step the list line does not give, src = the source is
# invalid and yavs is right, gap = a rule of the standard that yavs misses and should check, opt = the expectation is for a tool switch (the synthesis checks), bug = a front-end bug still to fix.
CLASSES = ("sem", "ext", "ctx", "src", "gap", "bug", "opt")


def rd(p):
    return p.read_text(errors="replace")


def root_of(arg):
    cands = [arg, os.environ.get("YAVS_CORPUS"), "~/.cache/rsim-corpus", "~/.cache/rsim-corpora"]
    return next((Path(c).expanduser() / "ivtest" for c in cands if c and (Path(c).expanduser() / "ivtest/ivltests").is_dir()), None)


def parse_list(root, name):
    """Entries (list, test, type, gen, files, args) of regress-NAME.list."""
    text = rd(root / f"regress-{name}.list")
    text = re.sub(r"\\[ \t]*\n[ \t]*", "", text)
    out = []
    for ln in text.splitlines():
        if not ln.strip() or ln.lstrip().startswith("#"):
            continue
        t = ln.split("#")[0].split()
        if len(t) < 3:
            continue
        test, spec, d = t[0], t[1], t[2]
        typ, *flags = [x for x in spec.split(",") if x]
        if typ == "nornal":
            typ = "normal"  # a typo in regress-sv.list
        gen = "sv" if name in ("sv", "fsv") or any(f in SV_FLAGS for f in flags) else "ams" if "-gverilog-ams" in flags else "v"
        files = [f"{d}/{test}.v"] + [f.removeprefix("./") for f in flags if f.endswith(".v")]
        args = []
        for f in flags:
            if f.startswith("-D"):
                args += ["-D", f[2:]]
            elif f.startswith("-I"):
                args += ["-I", f[2:].removeprefix("./")]
        out.append((name, test, typ, gen, files, args))
    return out


def enumerate_runs(root):
    runs, seen = [], set()
    for name in LISTS:
        for e in parse_list(root, name):
            runs.append(e)
            seen.update(e[4])
    for d in ("ivltests", "contrib"):
        for p in sorted((root / d).glob("*.v")):
            r = f"{d}/{p.name}"
            if r not in seen:
                runs.append(("-", p.stem, "-", "-", [r], []))
    return runs


def line(e, outcome):
    lst, test, typ, gen, files, args = e
    return shlex.join([outcome, lst, typ, gen, ",".join(files)] + args)


def read_manifest(path):
    rows = []
    for ln in path.read_text().splitlines():
        if not ln.strip() or ln.startswith("#"):
            continue
        t = shlex.split(ln)
        rows.append((t[0], t[1], t[2], t[3], t[4].split(","), t[5:]))
    return rows


def verdict(typ, gen, outcome):
    """agree, agree-sv (SystemVerilog recognized), disagree, or none (no expectation)."""
    if typ in ("EF", "NI", "TE", "-"):
        return "none"
    codes = set(outcome.split(","))
    if typ == "CE":
        return "agree" if outcome != "PASS" else "disagree"
    if outcome == "PASS":
        return "agree"
    if gen == "sv":
        return "agree-sv" if codes <= SV_CODES else "sv-syntax"  # sv-syntax: SystemVerilog reported as a plain syntax error
    return "agree-ams" if gen == "ams" else "disagree"


def tkey(typ, files, args):
    return f"{typ}:{files[0]}" + ("+" if len(files) > 1 or args else "")


def triage_of():
    p = HERE / "ivtest.triage"
    t = {}
    if p.exists():
        for ln in p.read_text().splitlines():
            if ln.strip() and not ln.startswith("#"):
                k, c, *_ = ln.split(None, 2)
                if c not in CLASSES:
                    vcommon.die("ivtest.py", f"ivtest.triage: unknown class {c!r}")
                t[k] = c
    return t


def table(rows, triage):
    cnt = collections.defaultdict(collections.Counter)
    for outcome, lst, typ, gen, files, args in rows:
        v = verdict(typ, gen, outcome)
        if v == "disagree":
            v = "disagree:" + triage.get(tkey(typ, files, args), "?")
        cnt[(lst, typ, gen)][v] += 1
    cols = sorted({v for c in cnt.values() for v in c})
    print(f"{'list':<7}{'type':<7}{'gen':<4}{'total':>6}  " + " ".join(f"{c:>13}" for c in cols))
    tot = collections.Counter()
    for k in sorted(cnt):
        print(f"{k[0]:<7}{k[1]:<7}{k[2]:<4}{sum(cnt[k].values()):>6}  " + " ".join(f"{cnt[k][c]:>13}" for c in cols))
        tot.update(cnt[k])
    print(f"{'all':<18}{sum(tot.values()):>6}  " + " ".join(f"{tot[c]:>13}" for c in cols))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("action", nargs="?", default="check", choices=["check", "bless", "table", "list"])
    ap.add_argument("--root")
    ap.add_argument("--yavs")
    ap.add_argument("-j", type=int, default=4)
    ap.add_argument("--no-wrap", action="store_true")
    a = ap.parse_args()
    manifest, triage = HERE / "ivtest.expected", triage_of()
    if a.action == "table":
        table(read_manifest(manifest), triage)
        return 0
    root = root_of(a.root)
    if root is None:
        print("ivtest: skipped, the ivtest suite was not found; run tools/fetch-corpus.sh (or pass --root with the corpus cache)")
        return 0
    runs = enumerate_runs(root)
    if a.action == "list":
        print(f"ivtest: root {root}, {len(runs)} runs, {len({(tuple(e[4]), tuple(e[5])) for e in runs})} distinct")
        return 0
    yavs = vcommon.find_yavs(a.yavs)
    keys = list(dict.fromkeys((tuple(e[4]), tuple(e[5])) for e in runs))
    got = {}
    with tempfile.TemporaryDirectory(prefix="yavs-ivtest-") as tmp:
        os.environ["YAVS_STD_CACHE"] = os.path.join(tmp, "std")

        def work(item):
            n, (files, args) = item
            # the suite runs its tests from its root, so `include "ivltests/x.v"` resolves there
            rc, diags, text = vcommon.analyze(yavs, os.path.join(tmp, f"w{n}"), "work", ["-I", "."] + list(args), [root / f for f in files], root, not a.no_wrap)
            errs = sorted({c for lvl, c, *_ in diags if lvl == "error"})
            return (files, args), ",".join(errs) if errs else "PASS" if rc == 0 else f"EXIT{rc}"

        with cf.ThreadPoolExecutor(a.j) as pool:
            got = dict(pool.map(work, enumerate(keys)))
    outs = [got[(tuple(e[4]), tuple(e[5]))] for e in runs]
    if a.action == "bless":
        manifest.write_text("# tools/verilog/ivtest.py: outcome per ivtest run, `OUTCOME LIST TYPE GEN FILES [-I DIR | -D NAME]...`; paths only, no test content.\n"
                            + "".join(line(e, o) + "\n" for e, o in zip(runs, outs)))
        print(f"wrote {manifest}")
        table(read_manifest(manifest), triage)
        return 0
    exp = read_manifest(manifest) if manifest.exists() else vcommon.die("ivtest.py", f"{manifest} missing; create it with `bless`")
    bad = []
    if [(e[1], e[2], e[3], e[4], e[5]) for e in exp] != [(e[0], e[2], e[3], e[4], e[5]) for e in runs]:
        bad.append(f"the manifest lists other runs than the suite holds ({len(exp)} against {len(runs)}); run `bless` and review the diff")
    else:
        for x, e, o in zip(exp, runs, outs):
            if x[0] != o:
                bad.append(f"{e[0]} {e[1]} {','.join(e[4])}: got {o}, expected {x[0]}")
            elif verdict(e[2], e[3], o) == "disagree" and triage.get(tkey(e[2], e[4], e[5])) is None:
                bad.append(f"{e[0]} {e[1]} {e[4][0]}: {o} against the expectation {e[2]} and no entry in ivtest.triage")
    print(f"{len(runs)} runs ({len(keys)} distinct); " + ", ".join(f"{k} {v}" for k, v in sorted(collections.Counter(outs).items(), key=lambda kv: -kv[1])[:8]))
    for b in bad[:60]:
        print("DIFF:", b)
    if len(bad) > 60:
        print(f"... and {len(bad) - 60} more differences")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
