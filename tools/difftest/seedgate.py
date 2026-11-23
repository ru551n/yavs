#!/usr/bin/env python3
"""Determinism gate of `--init-unset` and `--init-x` (user decision A68): run the designs of the cycle gate
(the difftest corpus, crates/yavs/tests/cycle, the bench RTL designs) with one `random` seed in every tier, at one
and four threads, with `--shuffle`, from a cold and a warm cache, normal and `--cycle`, and compare the output of
each mode's runs. The warm cycle run comes after a run with another seed in the same library: it must take the
analysis from the cache (no other analysis stored) and give the cold run's output. Stdlib only.

  seedgate.py [--seed N] [--two-state] [--only all|corpus|cycle|bench] [--filter RE] [--jobs N] [--tiers ...]

`--two-state` adds the option to every run, normal and cycle mode alike.

Across modes (normal, `--cycle`, `--cycle-strict`, `--two-state`, `--cycle --two-state`, A68(2)): two modes that
give the same output with `--init-unset=0 --init-x=0` and with `=1` must also give the same output with the seed.
Pairs that differ at a fixed value already differ by contract (docs/cycle-mode.rst) and are counted, not compared.
The cross-mode runs are left out under `--two-state`, which they cover.
"""
import argparse, re, shutil, sys, concurrent.futures as cf
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import difftest as D  # noqa: E402
import cyclegate as G  # noqa: E402


TWO = False  # --two-state: every run two-state


def flags(seed):
    return f"--init-unset=random:{seed} --init-x=random:{seed}" + (" --two-state" if TWO else "")


def one(item, a):
    """(name, bucket, why, {(mode, mode): [compared, apart at a fixed value]}): `same`, `differs` (a run of the mode
    gave other output, the warm cycle run stored another analysis, or two modes the seed alone set apart) or
    `skipped` (the plain normal run fails before simulating)."""
    name, vhd, hdr, (tn, tc) = item
    base = D.CACHE / "seed" / re.sub(r"\W+", "_", name)
    f = flags(a.seed)

    def go(t, d, threads=1, shuffle="", fresh=True, seed_flags=f):
        rc, out, err = D.run(t.replace(G.CYC, G.CYC + " " + seed_flags), vhd, hdr, base / d, threads, shuffle,
                             not a.no_sandbox, fresh=fresh)
        return D.normalise(rc, out, err, yavs=True), rc

    first, rc = go(tn.replace("--threads=", "--tier=interp --threads="), "n")
    if rc in (97, 98):
        return name, "skipped", "analysis or elaboration fails", {}
    runs = {"normal interp": first}
    for t in a.tiers:
        runs[f"normal {t}"] = go(tn.replace("--threads=", f"--tier={t} --threads="), "n")[0]
    runs["normal 4 threads"] = go(tn, "n", threads=4)[0]
    runs["normal --shuffle"] = go(tn, "n", shuffle="1")[0]
    cyc = {"cycle": go(tc, "c")[0], "cycle 4 threads": go(tc, "c", threads=4)[0],
           "cycle --shuffle": go(tc, "c", shuffle="1")[0]}
    # Another seed fills the cache, then this one runs warm on its library, in a fresh directory (files
    # the test writes start afresh).
    go(tc, "w0", seed_flags=flags(a.seed + 1))
    shutil.rmtree(base / "w", ignore_errors=True)
    if (base / "w0" / "lib").is_dir():
        shutil.copytree(base / "w0" / "lib", base / "w" / "lib")
    store = base / "w" / "lib" / "yavs-cache" / "cycle"
    before = sorted(p.name for p in store.glob("cyc3-*.img"))
    cyc["cycle warm after another seed"] = go(tc, "w", fresh=False)[0]
    if sorted(p.name for p in store.glob("cyc3-*.img")) != before:
        return name, "differs", "the warm cycle run with a new seed stored another analysis", {}
    for group in (runs, cyc):
        (k0, v0), *rest = group.items()
        for k, v in rest:
            if v != v0:
                return name, "differs", f"{k} against {k0}: {G.first_diff(v0, v)}", {}
    pairs = {}  # (mode, mode) -> [compared, apart at a fixed value]
    if not TWO:
        two = lambda t: t.replace(G.CYC, "--two-state " + G.CYC)
        ex = "export YAVS_CYCLE_EXACT_CLOCKS=1; "  # a copied clock as in the normal mode, as the exact cycle gate
        modes = {"normal": tn, "--cycle": ex + tc, "--cycle-strict": ex + tc.replace("--cycle ", "--cycle-strict ", 1),
                 "--two-state": two(tn), "--cycle --two-state": two(tc)}
        out = {s: {m: cross(go(t, f"x{i}", seed_flags=s)) for i, (m, t) in enumerate(modes.items())}
               for s in (f, FIXED[0], FIXED[1])}
        names = list(modes)
        for i, m1 in enumerate(names):
            for m2 in names[i + 1:]:
                apart = any(out[s][m1] != out[s][m2] for s in FIXED)
                if not apart and out[f][m1] != out[f][m2]:
                    return name, "differs", f"{m2} against {m1} with the seed: {G.first_diff(out[f][m1], out[f][m2])}", pairs
                pairs.setdefault((m1, m2), [0, 0])[apart] += 1
    if not a.keep:
        shutil.rmtree(base, ignore_errors=True)
    return name, "same", "", pairs


FIXED = ("--init-unset=0 --init-x=0", "--init-unset=1 --init-x=1")


def cross(r):
    """A run's canonical output without the line cycle mode adds (how many processes joined)."""
    return [l for l in r[0] if not l.startswith("err\tcycle mode: ")]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--seed", type=int, default=11)
    ap.add_argument("--only", choices=("all", "corpus", "cycle", "bench"), default="all")
    ap.add_argument("--filter", default="", help="regex on the test name")
    ap.add_argument("--jobs", type=int, default=4, help=f"parallel tests (max {D.MAX_JOBS})")
    ap.add_argument("--tiers", default="auto,clift", help="tiers besides interp (a build without clift: none)")
    ap.add_argument("--no-sandbox", action="store_true")
    ap.add_argument("--keep", action="store_true", help="keep the run directories")
    ap.add_argument("--two-state", action="store_true", help="add --two-state to every run")
    a = ap.parse_args()
    global TWO
    TWO = a.two_state
    a.tiers = [t for t in a.tiers.split(",") if t]
    items = [i for i in G.items(a.only) if re.search(a.filter, i[0])]
    counts, pairs = {}, {}
    with cf.ThreadPoolExecutor(min(a.jobs, D.MAX_JOBS)) as ex:
        for name, bucket, why, pp in ex.map(lambda i: one(i, a), items):
            counts[bucket] = counts.get(bucket, 0) + 1
            for k, (n, m) in pp.items():
                pairs.setdefault(k, [0, 0])
                pairs[k][0] += n
                pairs[k][1] += m
            if bucket == "differs":
                print(f"DIFFERS {name}: {why}", flush=True)
    for (m1, m2), (n, m) in pairs.items():
        print(f"across modes: {m1} against {m2}: {n} tests compared, {m} differ at a fixed value")
    print(f"seedgate: seed {a.seed}, {len(items)} tests: " + ", ".join(f"{v} {k}" for k, v in sorted(counts.items())))
    sys.exit(1 if counts.get("differs") else 0)


if __name__ == "__main__":
    main()
