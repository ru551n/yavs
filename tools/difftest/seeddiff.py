#!/usr/bin/env python3
"""Compare, normal mode against `--cycle`, the values at the end of every time step of the signals driven by
the processes outside the region whose result may depend on the deltas their inputs change in (state,
latches, incomplete sensitivity lists, or a process the checker could not show free of them: the seeds of
the run-count rule). Console equality is too weak for those: Microwatt's `dcache_request` gave a stale
`req_nc` from 10.3 us on, and the console lost one line at the very end.

  seeddiff.py --bin YAVS --cwd DIR [--out DIR] [--show N] [--seeds LIST] -- <yavs run arguments without --cycle>

Steps: `--cycle-report` with YAVS_CYCLE_SEEDS writes the leaf names; both modes run with YAVS_TRACE (one
line `time name value` per change at the end of a time step); the traces are compared time step by time
step. A difference only where the normal mode holds a metavalue (`U X Z W L H -`) is counted apart: cycle mode
reads metavalues as 0 or 1, and logic that follows such a value may give another value (Microwatt's
`early_rd_valid` is `'U'` at 0 fs in the normal mode, `'1'` in cycle mode). Any other difference is one, and the
exit status is 1. Cycle mode runs under the exact rules (`--cycle` alone, YAVS_CYCLE_EXACT_CLOCKS=1)
unless `--contract two-state` asks for `--cycle --two-state`, whose copied clocks and metavalues may give other values
by design (docs/cycle-mode.rst "The contract in short"). Stdlib only.
"""
import argparse, os, subprocess, sys, tempfile
from pathlib import Path

META = set("UXZWLH-")


def two_state(n, c):
    """Whether cycle value c differs from n only where n holds a metavalue (logic string literals of one
    length): the boundary reads it as 0 or 1, and what follows from that value may differ too."""
    if len(n) != len(c) or not n.startswith('"'):
        return False
    return all(x == y or x in META for x, y in zip(n, c))


def steps(path):
    """(time, {name: value}) per time step of a trace, in order."""
    t0, cur = None, {}
    with open(path, errors="replace") as f:
        for line in f:
            t, name, v = line.rstrip("\n").split(" ", 2)
            t = int(t)
            if name == "-":
                continue
            if t != t0 and cur:
                yield t0, cur
                cur = {}
            t0 = t
            cur[name] = v
    if cur:
        yield t0, cur


def end_time(path):
    """The time the run of a trace ended (its `- end` line), else that of its last line."""
    t = 0
    with open(path, errors="replace") as f:
        for line in f:
            t = int(line.split(" ", 1)[0])
    return t


def compare(n_path, c_path, show, until=None):
    """Counts of (two-state, differing) and the first `show` differences, for steps before `until`."""
    vn, vc, shown = {}, {}, []
    two = diff = 0
    a, b = steps(n_path), steps(c_path)
    x, y = next(a, None), next(b, None)
    while x or y:
        t = min(s[0] for s in (x, y) if s)
        if until is not None and t >= until:
            break
        changed = set()
        if x and x[0] == t:
            vn.update(x[1]); changed |= x[1].keys(); x = next(a, None)
        if y and y[0] == t:
            vc.update(y[1]); changed |= y[1].keys(); y = next(b, None)
        for k in sorted(changed):
            n, c = vn.get(k), vc.get(k)
            if n == c:
                continue
            if n is not None and c is not None and two_state(n, c):
                two += 1
                continue
            diff += 1
            if len(shown) < show:
                shown.append(f"{t} fs {k}: normal {n} cycle {c}")
    return two, diff, shown


def selftest():
    assert two_state('"0U1"', '"001"') and two_state('"U"', '"1"')
    assert not two_state('"01"', '"00"') and not two_state("[1, 2]", "[1, 3]")
    print("selftest ok")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bin", default=os.environ.get("YAVS_BIN"))
    ap.add_argument("--cwd", default=".")
    ap.add_argument("--out", help="keep the list and traces here (default: a temporary directory)")
    ap.add_argument("--show", type=int, default=10)
    ap.add_argument("--seeds", help="trace the signals of this list (YAVS_CYCLE_SEEDS of another binary, say "
                    "the one before a change that lets processes join) instead of asking this one")
    ap.add_argument("--contract", choices=("exact", "two-state"), default="exact",
                    help="exact (default): --cycle alone and YAVS_CYCLE_EXACT_CLOCKS=1; two-state: --cycle --two-state")
    ap.add_argument("--selftest", action="store_true")
    ap.add_argument("args", nargs=argparse.REMAINDER)
    o = ap.parse_args()
    two = [] if o.contract == "exact" else ["--two-state"]
    if not two:
        os.environ["YAVS_CYCLE_EXACT_CLOCKS"] = "1"
    else:
        os.environ.pop("YAVS_CYCLE_EXACT_CLOCKS", None)
    if o.selftest:
        return selftest()
    if not o.bin:
        ap.error("give --bin or YAVS_BIN")
    args = o.args[1:] if o.args[:1] == ["--"] else o.args
    if not args or args[0] != "run":
        ap.error("give the yavs command after `--`, starting with `run`")
    out = Path(o.out or tempfile.mkdtemp(prefix="seeddiff-")).resolve()
    out.mkdir(parents=True, exist_ok=True)
    lst = out / "seeds.txt"
    if o.seeds:
        lst.write_text(Path(o.seeds).read_text())
    else:
        env = dict(os.environ, YAVS_CYCLE_SEEDS=str(lst))
        r = subprocess.run([o.bin, *args[:1], "--cycle-report", *two, *args[1:]], cwd=o.cwd, env=env,
                           stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
        if not lst.exists():
            sys.exit(f"seeddiff: --cycle-report wrote no list (exit {r.returncode}): {r.stderr[-2000:]}")
    names = lst.read_text().split()
    print(f"seeddiff: {len(names)} signals driven by seed processes")
    if not names:
        return 0
    rcs = {}
    for mode, extra in (("normal", []), ("cycle", ["--cycle", *two])):
        env = dict(os.environ, YAVS_TRACE=str(out / f"{mode}.trace"), YAVS_TRACE_SIGNALS=str(lst))
        with open(out / f"{mode}.out", "w") as fo, open(out / f"{mode}.err", "w") as fe:
            rcs[mode] = subprocess.run([o.bin, *args[:1], *extra, *args[1:]], cwd=o.cwd, env=env,
                                       stdout=fo, stderr=fe).returncode
    print(f"seeddiff: exit status normal {rcs['normal']}, cycle {rcs['cycle']}")
    # A run that ended early (Microwatt's RTL failure, which cycle mode drops) is compared up to the
    # time step it ended in, which it may not have finished.
    until = None
    if rcs["normal"] != rcs["cycle"]:
        until = min(end_time(out / f"{m}.trace") for m in ("normal", "cycle"))
        print(f"seeddiff: the exit statuses differ: compared before {until} fs")
    two, diff, shown = compare(out / "normal.trace", out / "cycle.trace", o.show, until)
    print(f"seeddiff: {diff} differing values, {two} metavalues read two-state")
    for s in shown:
        print("  " + s)
    print(f"seeddiff: traces in {out}")
    return 1 if diff else 0


if __name__ == "__main__":
    sys.exit(main())
