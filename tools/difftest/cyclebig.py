#!/usr/bin/env python3
"""Big designs under cycle mode: run realbench worlds (a CPU running its program) normal and with
`--cycle`, with one binary, for the test's bounded simulated time, and compare exit status, the console
(standard output without report records) and the text files the run writes in its working directory
(the CPU's instruction trace, UART logs). A `?` in the normal mode's file (a hex digit of a metavalue)
matches any character: cycle mode reads metavalues as 0. Reports are counted, not compared: cycle
mode does not evaluate reports, asserts and library warnings of member processes. Fails when the run
compared nothing (no member, or no output at all). Worlds are made by tools/realbench/run.py --prepare;
a test without a world is skipped with the command that makes it. Stdlib only. See README.md.
"""
import argparse, itertools, os, re, shutil, subprocess, sys, time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "realbench"))
import run as R  # noqa: E402  (tools/realbench/run.py: tests.toml, the yavs command of a world, console_of)

DEFAULT = "neorv32_dual_full"  # microwatt_coremark_short is opt-in: its normal run takes minutes
MEMBERS = re.compile(r"(\d+) of (\d+) processes")


def same_lines(a, b):
    """None when the text files a (normal) and b (cycle) are equal, a `?` of a matching anything; else where."""
    with open(a, errors="replace") as fa, open(b, errors="replace") as fb:
        for i, (x, y) in enumerate(itertools.zip_longest(fa, fb), 1):
            if x is None or y is None:
                return f"line {i}: one file ends earlier"
            if x != y and (len(x) != len(y) or any(c != d and c != "?" for c, d in zip(x, y))):
                return f"line {i}: {x.rstrip()!r} against {y.rstrip()!r}"
    return None


def selftest(d):
    d.mkdir(parents=True, exist_ok=True)
    def f(name, text):
        (d / name).write_text(text)
        return d / name

    n = f("n", "a 0x????0001\nb\n")
    assert same_lines(n, f("c", "a 0x00000001\nb\n")) is None
    assert same_lines(n, f("x", "a 0x00000002\nb\n")).startswith("line 1")
    assert same_lines(n, f("s", "a 0x00000001\n")).startswith("line 2")
    assert same_lines(f("e", ""), f("e2", "")) is None
    shutil.rmtree(d)
    print("selftest ok")


def one_mode(Y, t, world, d, cycle, tier, timeout):
    """Run one mode in a copy of the world; (exit status, members, reports, {name: text file}, last lines of stderr)."""
    shutil.copytree(world, d, symlinks=True)
    start = time.time()
    script = R.yavs_script(Y, t, d, tier, extra=["--cycle"] if cycle else [])
    with open(d.parent / f"{d.name}.out", "w") as fo, open(d.parent / f"{d.name}.err", "w") as fe:
        rc = subprocess.run(["timeout", str(timeout), "systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=8G",
                             "bash", "-c", script], stdout=fo, stderr=fe).returncode
    out = d / R.OUT if t.get("quiet") else d.parent / f"{d.name}.out"
    err = d / R.ERR if t.get("quiet") else d.parent / f"{d.name}.err"
    con = d.parent / f"{d.name}.console"
    with open(out, errors="replace") as f:
        con.write_text(R.console_of(f))
    reports = sum(1 for l in open(out, errors="replace") if R.REPORT.match(l))
    m = MEMBERS.search(err.read_text(errors="replace")) if cycle else None
    files = {p.name: p for p in d.iterdir() if p.is_file() and p.stat().st_mtime >= start and p.name not in (R.OUT, R.ERR)}
    tail = [l for l in err.read_text(errors="replace").splitlines() if l.strip()][-3:]
    return rc, m and m.group(0), reports, {"console": con, **files}, tail


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("bin", nargs="?", default=os.environ.get("YAVS_BIN", ""), help="yavs binary (default $YAVS_BIN)")
    ap.add_argument("--tests", default=DEFAULT, help=f"comma list of realbench tests (default {DEFAULT}; also microwatt_coremark_short)")
    ap.add_argument("--label", default="yavs", help="world suffix: <cache>/worlds/<test>-<label> (run.py --prepare names it after the binary's label)")
    ap.add_argument("--tier", default="auto")
    ap.add_argument("--timeout", type=int, default=900, help="seconds per run")
    ap.add_argument("--keep", action="store_true", help="keep the scratch directory")
    ap.add_argument("--selftest", action="store_true")
    a = ap.parse_args()
    scratch = Path(os.environ.get("CYCLEBIG_CACHE", "~/.cache/yavs-cyclebig")).expanduser() / f"run-{os.getpid()}"
    if a.selftest:
        return selftest(scratch)
    if not a.bin or not Path(a.bin).is_file():
        sys.exit(f"cyclebig: no yavs binary {a.bin!r}: pass it or set YAVS_BIN")
    tests = {t["name"]: t for t in R.load_tests()}
    bad = ran = 0
    for name in a.tests.split(","):
        if name not in tests:
            sys.exit(f"cyclebig: no realbench test {name!r}")
        t, world = tests[name], R.CACHE / "worlds" / f"{name}-{a.label}"
        if not world.is_dir():
            print(f"SKIPPED {name}: no world at {world}; make it with\n"
                  f"    YAVS_REALBENCH_CACHE={R.CACHE} python3 -I tools/realbench/run.py {a.bin} --cores 6 --tests {name} --prepare")
            continue
        ran += 1
        (rn, _, nrep, nf, ntail), (rc, mem, crep, cf, ctail) = (one_mode(a.bin, t, world, scratch / name / k, k == "cycle", a.tier, a.timeout)
                                                  for k in ("normal", "cycle"))
        why = []
        if rn != t["exit_code"]:
            why.append(f"the normal run failed (exit {rn}): " + " / ".join(ntail))
        if rn != rc:
            why.append(f"exit status {rn} normal, {rc} cycle: " + " / ".join(ctail))
        if set(nf) != set(cf):
            why.append(f"files written: {sorted(nf)} normal, {sorted(cf)} cycle")
        why += [f"{k}: {d}" for k in sorted(set(nf) & set(cf)) if (d := same_lines(nf[k], cf[k]))]
        lines = {k: sum(1 for l in open(p, errors="replace") if l.strip()) for k, p in nf.items()}
        if not mem or mem.startswith("0 "):
            why.append("no process joined the region: nothing was compared")
        elif not any(lines.values()):
            why.append("the normal run wrote nothing to compare")
        bad += bool(why)
        print(f"{'DIFFERS' if why else 'SAME':8}{name}  [{mem or 'no region'}], {a.tier}, stop {t['stop'] or 'none'}, exit {rn}; "
              + ", ".join(f"{k} {n} lines" for k, n in sorted(lines.items()))
              + f"; reports {nrep} normal, {crep} cycle (not compared)", flush=True)
        for w in why:
            print(f"    {w}")
    if not a.keep and not bad:
        shutil.rmtree(scratch, ignore_errors=True)
    elif scratch.exists():
        print(f"outputs kept in {scratch}")
    if not ran:
        print("cyclebig: no world found, nothing compared")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
