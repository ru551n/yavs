#!/usr/bin/env python3
"""Check of the design under test that `--cycle` and `--two-state` find without PATH (A64): run recorded
`yavs run` command lines once per testbench (by work library and top), with `--print-hierarchy` and
analysis only (`--cycle --two-state --cycle-report`: plain `--cycle` takes the whole design), and compare the scope yavs names on its "design under test:" line
with an oracle used for this check only: the instances of the top-level unit labelled like a design under
test (`dut`, `uut`, `i_dut`, `u_dut`, ...) or whose entity is the testbench's name without its `tb`/`test`
prefix or suffix. The rule itself reads no names.

Verdicts: match; dut+more (the oracle's instances and other RTL instances, such as a memory model with
no testbench process); inside-dut (instances inside the oracle's); none (nothing found); other; and
no-oracle:{none,top,one,many} for a testbench the oracle cannot judge. With `--modes` each testbench also
runs under `--cycle-strict --two-state`, `--cycle --two-state --no-asserts` and normal `--two-state`
(`--stop-time=1fs`): the
scope must be the same in all of them (a different one fails the check).

Command file: one JSON object per line: {"name", "cwd", "argv"} as cyclereplay.py records them. Every run
gets `--no-cache` (nothing is written into the recorded libraries) and `--std-cache=DIR`.

    python3 -I tools/difftest/scopecheck.py BIN CMDS.jsonl... --std DIR --out OUT.jsonl [--jobs 2] [--modes]
"""
import argparse, concurrent.futures as cf, json, os, re, subprocess, sys
from collections import Counter

LABEL = re.compile(r"^(i_|u_|inst_|u\d*_?)?(dut|uut)(_.*|\d*)$", re.I)
DUT = re.compile(r"^design under test: (none found.*?|(.*?)) \(the scope of ", re.M)


def found(err):
    """The paths of the design under test line, [] for none, None without the line."""
    m = DUT.search(err)
    if not m:
        return None
    return [] if m.group(1).startswith("none found") else sorted(p.lower() for p in m.group(2).split(", "))


def hierarchy(out):
    """(instance path, entity) of the `--print-hierarchy` lines, generate indices kept."""
    insts = []
    for line in out.splitlines():
        last = line.rsplit(".", 1)[-1]
        if "@" in last and " (" not in line:
            segs = [re.sub(r"@.*$", "", x) for x in line.split(".")]
            segs[0] = re.sub(r"\(.*\)$", "", segs[0])
            insts.append((".".join(segs).lower(), line.rsplit("@", 1)[1].split("(")[0].lower()))
    return insts


def oracle(top, insts):
    tb = re.sub(r"^(tb_?|test_?)|(_?tb|_?test|_?testbench)$", "", top.split(".")[-1], flags=re.I).lower()
    root = insts[0][0].split(".")[0] if insts else ""
    kids = [(p, e) for p, e in insts if p.count(".") == 1 and p.startswith(root + ".")]
    return sorted({p for p, e in kids if LABEL.match(p.rsplit(".", 1)[-1]) or e == tb})


def verdict(dut, got, root):
    if got is None:
        return "no-line"
    if not dut:
        return "no-oracle:" + ("none" if not got else "top" if got == [root] else "one" if len(got) == 1 else "many")
    if got == dut:
        return "match"
    if not got:
        return "none"
    if set(dut) <= set(got):
        return "dut+more"
    if all(any(g.startswith(d + ".") for d in dut) for g in got):
        return "inside-dut"
    return "other"


# Plain `--cycle` takes the whole design (A76): the runs that find the design under test have `--two-state`.
MODES = {"report": ["--cycle", "--two-state", "--cycle-report"],
         "strict": ["--cycle-strict", "--two-state", "--cycle-report"],
         "noasserts": ["--cycle", "--two-state", "--no-asserts", "--cycle-report"],
         "normal-two": ["--two-state", "--stop-time=1fs"]}


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("bin")
    ap.add_argument("cmds", nargs="+")
    ap.add_argument("--std", required=True, help="the std/ieee store of these runs")
    ap.add_argument("--out", required=True)
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--modes", action="store_true", help="also check that every mode finds the same scope")
    a = ap.parse_args()
    seen, todo = set(), []
    for f in a.cmds:
        for line in open(f):
            r = json.loads(line)
            argv = r["argv"][2:]
            work = next((x for x in argv if x.startswith("--work=")), "")
            pos = [x for x in argv if not x.startswith("-")]
            key = (work, pos[-1] if pos else "")
            if key not in seen:
                seen.add(key)
                todo.append(r)
    gone = [r["name"] for r in todo if r.get("cwd") and not os.path.isdir(r["cwd"])]
    if gone:
        sys.exit(f"scopecheck: {len(gone)} testbenches have no working directory any more, such as {gone[0]}")

    def run(r, flags):
        argv = [f"--std-cache={a.std}" if x.startswith("--std-cache=") else x for x in r["argv"][2:]]
        cmd = ["timeout", "300", "systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=6G", a.bin, "run",
               "--no-cache", "--print-hierarchy", *flags, *argv]
        env = dict(os.environ, YAVS_STD_CACHE=a.std)
        return subprocess.run(cmd, capture_output=True, text=True, errors="replace", cwd=r.get("cwd") or None,
                              env=env, stdin=subprocess.DEVNULL)

    def one(r):
        p = run(r, MODES["report"])
        top = next((x for x in reversed(r["argv"][2:]) if not x.startswith("-")), "")
        insts = hierarchy(p.stdout)
        root = insts[0][0].split(".")[0] if insts else ""
        got = found(p.stderr)
        res = {"name": r["name"], "top": top, "rc": p.returncode, "found": got, "oracle": oracle(top, insts)}
        res["verdict"] = verdict(res["oracle"], got, root)
        if p.returncode:
            res["err"] = p.stderr[-2000:]
        if a.modes:
            res["modes"] = {m: found(run(r, MODES[m]).stderr) for m in ("strict", "noasserts", "normal-two")}
        return res

    stats, bad = Counter(), []
    with cf.ThreadPoolExecutor(a.jobs) as ex, open(a.out, "w") as out:
        for res in ex.map(one, todo):
            out.write(json.dumps(res) + "\n")
            out.flush()
            stats[res["verdict"] if res["rc"] == 0 else "failed"] += 1
            if res["rc"] or any(v != res["found"] for v in res.get("modes", {}).values()):
                bad.append(res["name"])
    judged = sum(n for k, n in stats.items() if not k.startswith(("no-oracle", "failed", "no-line")))
    print(f"{len(todo)} testbenches, {judged} with an oracle: {stats['match']} match ({100 * stats['match'] / max(judged, 1):.0f}%)")
    for k, n in sorted(stats.items()):
        print(f"  {k}: {n}")
    for n in bad:
        print(f"FAILED (run failed or a mode found another scope): {n}")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
