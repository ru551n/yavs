#!/usr/bin/env python3
"""Exact user-instruction counts per function and per source line for a window of a run.

  python3 -I tools/stepcount/stepcount.py [--from SYM[:SKIP]] [--to SYM[:N]] [--max N] [--per N]
        [--top N] [--lines N] [--in SYM] BIN [ARGS...]
  python3 -I tools/stepcount/stepcount.py --self-test

Builds step.c (a ptrace single-stepper) with the system C compiler into the cache, runs BIN under it and
reports the instructions executed in the window per function (self, without callees) and, with --lines,
per source line (addr2line; needs line tables: tools/build-bench.sh keeps them). Read
tools/stepcount/AGENTS.md section 2 for the method and its limits.
"""
import argparse
import bisect
import collections
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
CACHE = os.path.expanduser(os.environ.get("YAVS_STEPCOUNT_CACHE", "~/.cache/yavs-stepcount"))
HASH = re.compile(r"::h[0-9a-f]{16}$")


def die(msg):
    sys.exit(f"stepcount: {msg}")


def stepper():
    """Path of the compiled stepper, rebuilt when step.c is newer."""
    os.makedirs(CACHE, exist_ok=True)
    exe, src = os.path.join(CACHE, "step"), os.path.join(HERE, "step.c")
    if not os.path.exists(exe) or os.path.getmtime(exe) < os.path.getmtime(src):
        cc = os.environ.get("CC", "cc")
        r = subprocess.run([cc, "-O2", "-Wall", "-o", exe, src], capture_output=True, text=True)
        if r.returncode:
            die(f"{cc} failed on step.c:\n{r.stderr}")
    return exe


def symbols(binary):
    """Sorted [(address, size, name)] of the defined symbols with sizes, names demangled, hash removed."""
    raw = subprocess.run(["nm", "-n", "-S", "--defined-only", binary], capture_output=True, text=True)
    dem = subprocess.run(["nm", "-n", "-S", "-C", "--defined-only", binary], capture_output=True, text=True)
    if raw.returncode or dem.returncode:
        die(f"nm failed on {binary}: {raw.stderr or dem.stderr}")
    out = []
    for a, b in zip(raw.stdout.splitlines(), dem.stdout.splitlines()):
        f = a.split(None, 3)
        if len(f) < 4 or f[2] not in "tTwW":
            continue
        out.append((int(f[0], 16), int(f[1], 16), HASH.sub("", b.split(None, 3)[3])))
    return out


def split_count(spec, default):
    """`NAME:COUNT` into (NAME, COUNT); a name may contain `::`."""
    name, _, n = spec.rpartition(":")
    return (name, int(n)) if name and not name.endswith(":") and n.isdigit() else (spec, default)


def find(syms, spec, what):
    """Address of the one symbol whose name contains SPEC (0xADDR: that address); an error otherwise."""
    if spec.startswith("0x"):
        return int(spec, 16)
    hits = sorted({(a, n) for a, _, n in syms if spec in n})
    exact = [h for h in hits if h[1] == spec]
    hits = exact or hits
    if len(hits) != 1:
        lines = "\n".join(f"  {a:x} {n}" for a, n in hits[:10])
        die(f"--{what} {spec!r} matches {len(hits)} symbols, need exactly one (use a longer name or 0xADDRESS):\n{lines}")
    return hits[0][0]


LIBRARY = re.compile(r"^/rustc/|/\.cargo/registry/")


def lines_of(binary, pcs, inner=False):
    """{pc: (file:line, function)} from one addr2line call. The innermost inlined frame that is not
    library code (the standard library and registry crates), so a line is yours; `inner`: the
    innermost frame whatever it is."""
    if not pcs:
        return {}
    with tempfile.NamedTemporaryFile("w", suffix=".pcs") as f:
        f.write("\n".join(f"{p:x}" for p in pcs) + "\n")
        f.flush()
        with open(f.name) as stdin:
            r = subprocess.run(["addr2line", "-e", binary, "-f", "-C", "-a", "-i"], stdin=stdin,
                               capture_output=True, text=True)
    if r.returncode:
        die(f"addr2line failed: {r.stderr}")
    frames, cur = collections.defaultdict(list), None
    rows = r.stdout.splitlines()
    i = 0
    while i < len(rows):
        if re.fullmatch(r"0x[0-9a-f]+", rows[i]):
            cur = int(rows[i], 16)
            i += 1
        elif i + 1 < len(rows):
            frames[cur].append((HASH.sub("", rows[i]), rows[i + 1].split(" (discriminator")[0]))
            i += 2
        else:
            break
    res = {}
    for pc, fr in frames.items():
        own = [x for x in fr if not LIBRARY.match(x[1])]
        fn, loc = (own or fr)[0] if not inner else fr[0]
        res[pc] = (loc, fn)
    return res


def run(binary, args, frm, to, mx, per, top, nlines, within, inner):
    binary = os.path.realpath(binary)
    syms = symbols(binary)
    starts = [s[0] for s in syms]
    cmd = [stepper()]
    if frm:
        spec, skip = split_count(frm, 0)
        cmd += ["-f", f"{find(syms, spec, 'from'):x}:{skip}"]
    if to:
        spec, n = split_count(to, 1)
        cmd += ["-t", f"{find(syms, spec, 'to'):x}:{n}"]
    if mx:
        cmd += ["-n", str(mx)]
    with tempfile.TemporaryDirectory(prefix="stepcount", dir=CACHE) as d:
        pcfile = os.path.join(d, "pcs")
        r = subprocess.run(cmd + ["-o", pcfile, "--", binary, *args], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
        if r.returncode:
            die(r.stderr.strip())
        pcs = [(int(a, 16), int(n)) for a, n in (ln.split() for ln in open(pcfile))]
    total = sum(n for _, n in pcs)
    for ln in r.stderr.strip().splitlines():
        print(ln)
    byfunc = collections.Counter()
    for pc, n in pcs:
        i = bisect.bisect_right(starts, pc) - 1
        ok = i >= 0 and pc < syms[i][0] + max(syms[i][1], 1)
        byfunc[syms[i][2] if ok else "(no symbol)"] += n
    div = per or 1
    unit = f" per {per}" if per else ""
    print(f"\nfunctions (self instructions{unit}), {total / div:.1f} in the program file:")
    for name, n in byfunc.most_common(top):
        print(f"{n / div:12.1f} {100 * n / total:5.1f}%  {name}")
    if nlines:
        info = lines_of(binary, [pc for pc, _ in pcs], inner)
        byline = collections.Counter()
        for pc, n in pcs:
            loc, fn = info.get(pc, ("??:0", "??"))
            if within is None or within in fn:
                byline[(loc, fn)] += n
        print(f"\nlines (instructions{unit}){' in ' + within if within else ''}:")
        for (loc, fn), n in byline.most_common(nlines):
            print(f"{n / div:12.1f} {100 * n / total:5.1f}%  {loc}  {fn}")


SELF_TEST_C = r"""
__attribute__((noinline)) void work(void) {
    __asm__ volatile("mov $1000, %%ecx\n1: dec %%ecx\njnz 1b" ::: "ecx");
}
__attribute__((noinline)) void after(void) { __asm__ volatile(""); }
int main(void) { work(); after(); return 0; }
"""


def self_test():
    """A program whose `work` runs 1 + 2 * 1000 loop instructions plus its ret, built as PIE and not."""
    with tempfile.TemporaryDirectory(prefix="stepcount-test", dir=_cache()) as d:
        src = os.path.join(d, "t.c")
        open(src, "w").write(SELF_TEST_C)
        for pie in ("-pie", "-no-pie"):
            exe, pcfile = os.path.join(d, "t" + pie), os.path.join(d, "pcs")
            r = subprocess.run([os.environ.get("CC", "cc"), "-O1", "-g", "-fcf-protection=none", pie, "-o", exe, src],
                               capture_output=True, text=True)
            if r.returncode:
                die(r.stderr)
            syms = symbols(exe)
            starts = [s[0] for s in syms]
            r = subprocess.run([stepper(), "-f", f"{find(syms, 'work', 'from'):x}", "-t", f"{find(syms, 'after', 'to'):x}",
                                "-o", pcfile, "--", exe], capture_output=True, text=True)
            if r.returncode:
                die(r.stderr)
            counts = {int(a, 16): int(n) for a, n in (ln.split() for ln in open(pcfile))}
            work = sum(n for a, n in counts.items() if syms[bisect.bisect_right(starts, a) - 1][2] == "work")
            assert work == 2002, f"{pie}: work executed {work} instructions, expected 2002"
            info = lines_of(exe, list(counts))
            assert any("t.c" in loc for loc, _ in info.values()), f"{pie}: addr2line found no line of t.c"
    print("stepcount self-test ok")


def _cache():
    os.makedirs(CACHE, exist_ok=True)
    return CACHE


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--from", dest="frm", metavar="SYM[:SKIP]", help="open the window at the (SKIP+1)-th entry to SYM (a name part, or 0xADDRESS)")
    ap.add_argument("--to", metavar="SYM[:N]", help="close it at the N-th entry to SYM after it opened (default 1)")
    ap.add_argument("--max", type=int, help="close it after this many instructions")
    ap.add_argument("--per", type=int, help="divide the counts by this (edges, cycles, activations)")
    ap.add_argument("--top", type=int, default=25, help="functions to show (default 25)")
    ap.add_argument("--lines", type=int, metavar="N", help="also the N busiest source lines")
    ap.add_argument("--inner", action="store_true", help="lines of the innermost inlined frame, library code included")
    ap.add_argument("--in", dest="within", metavar="SYM", help="lines of functions containing SYM only")
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("binary", nargs="?")
    ap.add_argument("args", nargs=argparse.REMAINDER)
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.binary:
        ap.error("BIN is required")
    run(a.binary, a.args, a.frm, a.to, a.max, a.per, a.top, a.lines, a.within, a.inner)


if __name__ == "__main__":
    main()
