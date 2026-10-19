#!/usr/bin/env python3
"""Run an analysis corpus manifest through a simulator and compare with the checked-in expected list.

  tools/corpus/run.py MANIFEST.toml [--tool yavs|nvc] [--cmd TEMPLATE] [--keep-going] [--bless]

MANIFEST is a path or a bare name (vunit-2008 -> tools/corpus/vunit-2008.toml). Files are analyzed one per
call, in manifest order. The result of every file (PASS = exit 0) is compared with <name>.expected (yavs) or
<name>.nvc.expected (nvc), one `PASS|FAIL <lib> <source>/<path>` line per manifest entry in order. Any
difference fails, until the list is updated (--bless rewrites it from this run, which then covers every file).
Without --keep-going the run stops at the first difference (without an expected list: the first FAIL).

Command template words: {std} {lib} {dir} {file}; {map} (one --map=LIB:DIR/LIB per library but std/ieee unless the flags have --bootstrap) and
{flags} (the manifest's [flags] for --tool) expand to several arguments when they are a word of their own.
Default templates:
  yavs: yavs analyze --std={std} --work={lib}:{dir}/{lib} {map} {flags} {file}
  nvc:  nvc --std={std} --work={lib}:{dir}/{lib} -L {dir} -a {flags} {file}
Every call runs under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G` (--no-wrap to disable).
Sources are looked up as <root>/<source id>, roots: --root, $YAVS_CORPUS, ~/.cache/rsim-corpus,
~/.cache/rsim-corpora (fetch them with tools/fetch-corpus.sh). Nothing is copied into the repo.
Exit: 0 all as expected, 1 differences, 2 usage/environment error.
"""
import argparse, os, shlex, shutil, subprocess, sys, tomllib
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
TEMPLATES = {
    "yavs": "yavs analyze --std={std} --work={lib}:{dir}/{lib} {map} {flags} {file}",
    "nvc": "nvc --std={std} --work={lib}:{dir}/{lib} -L {dir} -a {flags} {file}",
}
WRAP = ["timeout", "300", "systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=2G"]


def nvc_fixup(f, path, out):
    """NVC bootstraps STD.STANDARD itself: --bootstrap, and for 2019 a copy with RANGE_DIRECTION
    moved behind BOOLEAN (as tools/check-nvc.sh does). Returns (extra args, path)."""
    if f["path"].rsplit("/", 1)[-1] != "standard.vhdl":
        return [], path
    if f["std"] == "2019":
        s = path.read_text(encoding="latin-1").split("\n")
        i = next(k for k, l in enumerate(s) if l.startswith("  type RANGE_DIRECTION is ("))
        j = next(k for k, l in enumerate(s) if l.startswith("  type BOOLEAN is"))
        t = "\n".join(s[:i] + s[j:j + 1] + s[i:j] + s[j + 1:])
        path = out / "standard-nvc.vhdl"
        path.write_text(t, encoding="latin-1")
    return ["--bootstrap"], path


def die(msg):
    print("run.py:", msg, file=sys.stderr)
    sys.exit(2)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("manifest")
    ap.add_argument("--tool", choices=sorted(TEMPLATES), default="yavs")
    ap.add_argument("--cmd")
    ap.add_argument("--root", action="append", default=[])
    ap.add_argument("--out", help="library directory (default ~/.cache/yavs-corpus-out/<tool>-<name>-<pid>, unique per run)")
    ap.add_argument("--expected", help="expected-results list ('none' to skip the comparison)")
    ap.add_argument("--keep-going", action="store_true")
    ap.add_argument("--bless", action="store_true", help="write the expected list from this (complete) run")
    ap.add_argument("--log", help="write the full output of every failed file here")
    ap.add_argument("--no-wrap", action="store_true")
    a = ap.parse_args()

    mp = Path(a.manifest)
    if not mp.exists():
        mp = HERE / (a.manifest + ".toml")
    m = tomllib.loads(mp.read_text())
    name = m["name"]
    roots = [Path(r).expanduser() for r in a.root + [os.environ.get("YAVS_CORPUS", ""), "~/.cache/rsim-corpus", "~/.cache/rsim-corpora"] if r]
    base = {}
    for s in m["source"]:
        if s.get("local"):
            base[s["id"]] = REPO
            continue
        d = next((r / s["id"] for r in roots if (r / s["id"]).is_dir()), None)
        if d is None:
            die(f"source {s['id']} not found under {[str(r) for r in roots]}; run tools/fetch-corpus.sh")
        base[s["id"]] = d
    out = Path(a.out or f"~/.cache/yavs-corpus-out/{a.tool}-{name}-{os.getpid()}").expanduser()
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)

    exp_path = None if a.expected == "none" else Path(a.expected or HERE / (name + (".nvc" if a.tool == "nvc" else "") + ".expected"))
    expected = None
    if exp_path and exp_path.exists() and not a.bless:
        expected = [ln.split(None, 1) for ln in exp_path.read_text().splitlines() if ln.strip() and not ln.startswith("#")]
    elif exp_path and not a.bless:
        print(f"note: {exp_path} missing; stopping at the first FAIL (create it with --bless)")

    tmpl = shlex.split(a.cmd or TEMPLATES[a.tool])
    flags = m.get("flags", {}).get(a.tool, [])
    libs = sorted({f["lib"] for f in m["file"]} - (set() if "--bootstrap" in flags else {"std", "ieee"}))
    keep = a.keep_going or a.bless
    results, bad, stopped = [], 0, False
    log = open(a.log, "w") if a.log else None

    for i, f in enumerate(m["file"]):
        key = f"{f['lib']} {f['source']}/{f['path']}"
        path = base[f["source"]] / f["path"]
        extra = []
        if a.tool == "nvc":
            extra, path = nvc_fixup(f, path, out)
        argv = []
        for t in tmpl:
            if t == "{map}":
                argv += [f"--map={l}:{out}/{l}" for l in libs]
            elif t == "{flags}":
                argv += flags + (extra if a.tool == "nvc" else [])
            else:
                argv.append(t.format(std=f["std"], lib=f["lib"], dir=out, file=path))
        (out / f["lib"]).parent.mkdir(parents=True, exist_ok=True)
        try:
            p = subprocess.run(argv if a.no_wrap else WRAP + argv, capture_output=True, text=True, errors="replace")
            rc, text = p.returncode, p.stdout + p.stderr
        except OSError as e:
            die(f"cannot run {argv[0]}: {e}")
        res = "PASS" if rc == 0 else "FAIL"
        results.append((res, key))
        diff = None
        if expected is not None:
            if i >= len(expected) or expected[i][1] != key or expected[i][0] != res:
                diff = expected[i] if i < len(expected) else None
        elif res == "FAIL" and not a.bless:
            diff = ("?", "", key)
        print(f"{res} {key}" + (f"   (exit {rc})" if rc else ""))
        if res == "FAIL":
            if log:
                log.write(f"=== {key} (exit {rc})\n{text}\n")
            if diff is not None or a.bless or a.keep_going:
                print("   " + "\n   ".join(text.strip().splitlines()[:6]))
        if diff is not None:
            bad += 1
            want = f"expected {diff[0]} {diff[1]}" if diff[0] != "?" else "no expected list"
            print(f"DIFF at entry {i + 1}: got {res}; {want}")
            if not keep:
                stopped = True
                break

    if a.bless:
        exp_path.write_text("# tools/corpus/run.py expected results for %s (%s); one line per manifest entry, in order\n" % (name, a.tool)
                            + "".join(f"{r} {k}\n" for r, k in results))
        print(f"wrote {exp_path}")
    elif expected is not None and not stopped and len(expected) > len(results):
        bad += 1
        print(f"DIFF: expected list has {len(expected)} entries, manifest {len(results)}")
    n_pass = sum(r == "PASS" for r, _ in results)
    print(f"{name} [{a.tool}]: {n_pass} PASS, {len(results) - n_pass} FAIL of {len(results)} run"
          + (f", stopped early ({len(m['file']) - len(results)} not run)" if stopped else "")
          + f"; {bad} differ from the expected list")
    sys.exit(1 if bad else 0)


main()
