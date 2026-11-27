#!/usr/bin/env python3
"""Big designs under cycle mode: run realbench worlds (a CPU running its program) normal and with
`--cycle`, with one binary, for one bounded simulated time (`GATE`), and compare exit status, the console
(standard output without report records) and the text files the run writes in its working directory
(the CPU's instruction trace, UART logs). A `?` in the normal mode's file (a hex digit of a metavalue)
matches any character: cycle mode reads metavalues as 0. Reports are counted, not compared: member
processes give no library warnings, and NEORV32's normal mode prints tens of thousands. The region's members
must be the count `GATE` holds, so a change of membership is seen, and the normal console must hold the
test's `expect` lines, so two equally short runs cannot pass. Fails when the run compared nothing (no
member, or no output at all) and when a world is missing (the command that makes it is printed). Worlds
are made by tools/realbench/run.py --prepare. Stdlib only. See README.md.

`--contract exact` (the default) runs cycle mode under the exact rules (`--cycle` alone,
YAVS_CYCLE_EXACT_CLOCKS=1), which `GATE` holds. `--contract two-state` runs `--cycle --two-state` against a normal `--two-state` run: the member
count is shown, not checked, and a line of a file that differs only where the normal mode has a
metavalue and cycle mode a digit (`cyclegate.meta_pair`) is `CONTRACT`; every other difference fails.
"""
import argparse, itertools, os, re, shutil, subprocess, sys, time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "realbench"))
import run as R  # noqa: E402  (tools/realbench/run.py: tests.toml, the yavs command of a world, console_of)
sys.path.insert(0, str(HERE))
import cyclegate as G  # noqa: E402  (the contract classes)

DEFAULT = "neorv32_dual_full,microwatt_coremark_short"
MEMBERS = re.compile(r"(\d+) of (\d+) processes")
# Per test: the stop time of both runs (None: the test's own), the exit status of each mode (None: the
# test's), and the members of the region. Microwatt ends at 1368.555 us with its RTL's `end of test`
# failure in both modes (cycle mode evaluates the asserts of its members, A52): exit 1, stop 1369 us.
# A new member count is a decision, made in the commit that changes it. NEORV32 3822 -> 3795: the
# registers with an asynchronous reset in the fan-in of run-count seeds stay out (the reset
# sequencer making `rstn_sys`, the NEOTRNG ring-oscillator flip-flops). 3795 -> 3801: the testbench's
# `std_ulogic(...)` copies of its pulled-up lines (`twi_sda_i`, `onewire_i`, `smc_di` and three more) join.
# Microwatt 281 -> 258: the cross-delta rule follows paths through processes outside the region, so
# the logic from registers of the DTM's `jtag_clk` to registers of `clk` stays out (DTM, debug master,
# arbiter, the inputs of the block RAM and of the caches). 258 -> 233 (exact contract, A52): a member with
# an assert or report that reads a signal the region does not drive alone stays out under
# no `--two-state` (E4140): `wb_x_state` reads `soc_reset`, and 26 core processes with asserts
# (caches, TLBs, register file, execute, FPU, MMU, log buffer) then read what processes outside drive.
# 233 -> 258 (nine-valued region, A66): that rule is gone, the members' notes see the nine values.
GATE = {
    "neorv32_dual_full": dict(stop=None, normal=None, cycle=None, members="3801 of 3910"),
    "microwatt_coremark_short": dict(stop="1369us", normal=1, cycle=1, members="351 of 418"),
}


def same_lines(a, b, meta=None):
    """None when the text files a (normal) and b (cycle) are equal, a `?` of a matching anything; else where.
    With a list `meta` (the two-state contract) a line the metavalue class explains (`cyclegate.meta_pair`)
    matches too, and is counted in it."""
    with open(a, errors="replace") as fa, open(b, errors="replace") as fb:
        for i, (x, y) in enumerate(itertools.zip_longest(fa, fb), 1):
            if x is None or y is None:
                return f"line {i}: one file ends earlier"
            if x != y and (len(x) != len(y) or any(c != d and c != "?" for c, d in zip(x, y))):
                if meta is not None and G.meta_pair(x, y):
                    meta.append(i)
                    continue
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
    m, u = [], f("u", "a 0xUUUU0001\nb\n")
    assert same_lines(u, f("c2", "a 0x00000001\nb\n"), m) is None and m == [1]
    assert same_lines(u, f("c3", "a 0x00000001\nb\n")).startswith("line 1")  # exact: no metavalue class
    assert same_lines(u, f("c4", "a 0x00000002\nb\n"), []).startswith("line 1")
    shutil.rmtree(d)
    print("selftest ok")


EXTRA = ["--cycle"]  # the cycle flags; `--contract two-state` adds "--two-state" (to the normal run too)
CRASH = re.compile(r"E9999|this is a bug in yavs|panicked")


def one_mode(Y, t, world, d, cycle, tier, timeout, stop):
    """Run one mode in a copy of the world; (exit status, members, reports, {name: text file}, last lines of stderr)."""
    shutil.copytree(world, d, symlinks=True)
    start = time.time()
    # The two-state contract's reference is the normal mode with `--two-state` (as cyclegate's).
    script = R.yavs_script(Y, t, d, tier, stop=stop, extra=EXTRA if cycle else [x for x in EXTRA if x != "--cycle"])
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
    text = err.read_text(errors="replace")
    tail = [l for l in text.splitlines() if l.strip()][-3:]
    if CRASH.search(text):
        tail = ["crashed: " + CRASH.search(text).group(0)] + tail
    return rc, m and m.group(0), reports, {"console": con, **files}, tail


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("bin", nargs="?", default=os.environ.get("YAVS_BIN", ""), help="yavs binary (default $YAVS_BIN)")
    ap.add_argument("--tests", default=DEFAULT, help=f"comma list of the realbench tests of GATE (default {DEFAULT})")
    ap.add_argument("--label", default="yavs", help="world suffix: <cache>/worlds/<test>-<label> (run.py --prepare names it after the binary's label)")
    ap.add_argument("--tier", default="auto")
    ap.add_argument("--timeout", type=int, default=900, help="seconds per run")
    ap.add_argument("--keep", action="store_true", help="keep the scratch directory")
    ap.add_argument("--contract", choices=("exact", "two-state"), default="exact",
                    help="exact (default): --cycle alone and YAVS_CYCLE_EXACT_CLOCKS=1, members checked against GATE; "
                         "two-state: --cycle --two-state, lines that differ only by metavalues read two-state are CONTRACT, any other difference fails")
    ap.add_argument("--selftest", action="store_true")
    a = ap.parse_args()
    if a.contract == "exact":
        os.environ["YAVS_CYCLE_EXACT_CLOCKS"] = "1"
    else:
        EXTRA.append("--two-state")
        os.environ.pop("YAVS_CYCLE_EXACT_CLOCKS", None)
    scratch = Path(os.environ.get("CYCLEBIG_CACHE", "~/.cache/yavs-cyclebig")).expanduser() / f"run-{os.getpid()}"
    if a.selftest:
        return selftest(scratch)
    if not a.bin or not Path(a.bin).is_file():
        sys.exit(f"cyclebig: no yavs binary {a.bin!r}: pass it or set YAVS_BIN")
    tests = {t["name"]: t for t in R.load_tests()}
    bad = ran = 0
    for name in a.tests.split(","):
        if name not in tests or name not in GATE:
            sys.exit(f"cyclebig: no realbench test {name!r} in GATE (one of {', '.join(GATE)})")
        t, world, g = tests[name], R.CACHE / "worlds" / f"{name}-{a.label}", GATE[name]
        if not world.is_dir():
            bad += 1
            print(f"MISSING {name}: no world at {world}; make it with\n"
                  f"    YAVS_REALBENCH_CACHE={R.CACHE} python3 -I tools/realbench/run.py {a.bin} --cores 6 --tests {name} --prepare")
            continue
        ran += 1
        began = time.time()
        (rn, _, nrep, nf, ntail), (rc, mem, crep, cf, ctail) = (one_mode(a.bin, t, world, scratch / name / k, k == "cycle", a.tier, a.timeout, g["stop"])
                                                  for k in ("normal", "cycle"))
        want_n = t["exit_code"] if g["normal"] is None else g["normal"]
        want_c = want_n if g["cycle"] is None else g["cycle"]
        why = []
        if rn != want_n:
            why.append(f"the normal run ended with exit status {rn}, not {want_n}: " + " / ".join(ntail))
        if rc != want_c:
            why.append(f"the cycle run ended with exit status {rc}, not {want_c}: " + " / ".join(ctail))
        if mem and mem != f"{g['members']} processes" and a.contract == "exact":
            why.append(f"members: {mem}, the gate holds {g['members']} (a change of membership: explain it and update GATE)")
        con = nf["console"].read_text(errors="replace")
        why += [f"the normal console lacks /{e}/" for e in t.get("expect", []) if not re.search(e, con, re.M)]
        if set(nf) != set(cf):
            why.append(f"files written: {sorted(nf)} normal, {sorted(cf)} cycle")
        meta = [] if a.contract == "two-state" else None
        why += [f"{k}: {d}" for k in sorted(set(nf) & set(cf)) if (d := same_lines(nf[k], cf[k], meta))]
        crashed = any(l.startswith("crashed: ") for l in ctail)
        if crashed:
            why.append("the cycle run crashed: " + " / ".join(ctail))
        contract = not why and bool(meta)
        diffs = [f"{len(meta)} lines differ only where the normal mode has a metavalue (read as 0 or 1)"] if contract else []
        lines = {k: sum(1 for l in open(p, errors="replace") if l.strip()) for k, p in nf.items()}
        if not mem or mem.startswith("0 "):
            why.append("no process joined the region: nothing was compared")
        elif not any(lines.values()):
            why.append("the normal run wrote nothing to compare")
        bad += bool(why)
        print(f"{'DIFFERS' if why else 'CONTRACT' if contract else 'SAME':8}{name}  [{mem or 'no region'}], {a.tier}, stop {g['stop'] or t['stop'] or 'none'}, "
              + f"exit {rn} normal, {rc} cycle, {time.time() - began:.0f} s; "
              + ", ".join(f"{k} {n} lines" for k, n in sorted(lines.items()))
              + f"; reports {nrep} normal, {crep} cycle (not compared)", flush=True)
        for w in why + (diffs if contract else []):
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
