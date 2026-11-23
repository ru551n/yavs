#!/usr/bin/env python3
"""Waveforms of cycle mode against the normal mode (A52): each design of cyclegate.py's `cycle` and
`bench` sets runs normal and `--cycle` (`--contract` as in cyclegate.py) with `--wave`, and the
comparer `crates/yavs-wave/examples/waveq.rs` compares the two files signal by signal at the end of
each time step (the files keep no delta cycles). A design differs when a value differs or a name is
recorded on one side only, except a memory cycle mode cannot export (W0802 names it). Stdlib only.

  python3 -I tools/difftest/cyclewave.py --comparer PATH [--contract exact|default] [--only cycle|bench]
  (the yavs binary: $YAVS_BIN; the comparer: cargo build --release -p yavs-wave --example waveq)

Under `--contract two-state` a difference is listed but fails only for a crash or a name recorded on one
side: two-state values give other values by design (docs/cycle-mode.rst "Waveforms").
"""
import argparse, json, re, shutil, subprocess, sys, concurrent.futures as cf
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import cyclegate as G  # noqa: E402

D = G.D
# Reviewed differences under the exact contract: (test, contract) -> (signals, cause).
CHOICE = ("a clocked member with a choice keeps a register where only the second image changed it "
          "(docs/cycle-mode.rst \"Two-state values\"): `sums` shows 0 at the first edge where the normal mode has 'U'")
KNOWN = {
    ("bench/lfsr64.vhd", "exact"): ({"bench_lfsr64.sums"}, CHOICE),
    ("bench/lfsr64_xor.vhd", "exact"): ({"bench_lfsr64_xor.sums"}, CHOICE),
}
HELD = re.compile(r"warning\[W0802\]: `([^`]+)` is not recorded in the waveform")


def one(item, a):
    name, vhd, hdr, (tn, tc) = item
    base = D.CACHE / "wave" / re.sub(r"\W+", "_", name)
    res = {"name": name, "bucket": "same", "why": ""}
    try:
        runs = []
        for kind, t in (("normal", tn), ("cycle", tc)):
            t = t.replace(G.CYC, "--wave=w.fst " + G.CYC)
            runs.append(D.run(t, vhd, hdr, base / kind, 1, "", not a.no_sandbox))
        (nrc, nout, _), (crc, cout, cerr) = runs
        if G.CRASH.search(cerr) or (crc == 124 and nrc != 124):
            return {**res, "bucket": "differs", "why": "cycle mode crashed or timed out: " + cerr[-400:]}
        nf, cf_ = base / "normal" / "w.fst", base / "cycle" / "w.fst"
        if not nf.is_file() or not cf_.is_file():
            return {**res, "bucket": "skipped", "why": f"no wave file (normal exit {nrc}, cycle exit {crc})"}
        out = base / "waveq.json"
        p = subprocess.run([a.comparer, str(nf), str(cf_), "--json", str(out)], capture_output=True, text=True)
        if p.returncode == 2 or not out.is_file():
            return {**res, "bucket": "differs", "why": "comparer: " + p.stderr[-400:]}
        r = json.loads(out.read_text())
        held = {h.lower() for h in HELD.findall(cerr)}
        # A memory left out: its name and every element below it.
        only_a = [n for n in r["only_a"] if not any(n.lower() == h or n.lower().startswith(h + ".") or
                                                     n.lower().startswith(h + "[") for h in held)]
        why = []
        if only_a or r["only_b"]:
            why.append(f"names on one side only: {(only_a + r['only_b'])[:5]}")
        if r["different"] or r["shape"]:
            why.append(f"{r['different']} of {r['signals']} signals differ: {[d.get('name') for d in r['diffs'][:5]]}")
        names, cause = KNOWN.get((name, a.contract), (set(), ""))
        if why and not only_a and not r["only_b"] and not r["shape"] and \
                {d.get("name") for d in r["diffs"]} <= names and len(r["diffs"]) == r["different"]:
            return {**res, "bucket": "known", "why": cause}
        if why:
            bucket = "differs" if a.contract == "exact" or only_a or r["only_b"] else "contract"
            return {**res, "bucket": bucket, "why": "; ".join(why), "dir": str(base)}
        res["signals"] = r["signals"]
        shutil.rmtree(base, ignore_errors=True)
        return res
    except Exception as e:  # noqa: BLE001 (reported as a difference, never lost)
        return {**res, "bucket": "differs", "why": f"{type(e).__name__}: {e}"}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--comparer", required=True, help="the built waveq example")
    ap.add_argument("--only", choices=("all", "cycle", "bench"), default="all")
    ap.add_argument("--filter", default="", help="regex on the test name")
    ap.add_argument("--contract", choices=("exact", "two-state"), default="exact")
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--no-sandbox", action="store_true")
    ap.add_argument("-v", "--verbose", action="store_true")
    a = ap.parse_args()
    if not Path(a.comparer).is_file():
        sys.exit(f"cyclewave: no comparer {a.comparer!r}: cargo build --release -p yavs-wave --example waveq")
    G.exact(a.contract == "exact")
    err = D.candidate_error("yavs")
    if err:
        sys.exit(err)
    sets = ("cycle", "bench") if a.only == "all" else (a.only,)
    todo = [i for s in sets for i in G.items(s) if re.search(a.filter, i[0])]
    D.guard(D.CACHE)
    counts = {}
    with cf.ThreadPoolExecutor(max(1, min(a.jobs, D.MAX_JOBS))) as ex:
        for r in ex.map(lambda i: one(i, a), todo):
            counts[r["bucket"]] = counts.get(r["bucket"], 0) + 1
            if a.verbose or r["bucket"] != "same":
                print(f"{r['bucket'].upper():9} {r['name']}  {r['why']}", flush=True)
    shutil.rmtree(D.CACHE / "gen", ignore_errors=True)
    print("cycle waves: " + ", ".join(f"{v} {k}" for k, v in sorted(counts.items())))
    return 1 if counts.get("differs") else 0


if __name__ == "__main__":
    sys.exit(main())
