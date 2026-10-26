#!/usr/bin/env python3
"""Run an analysis corpus manifest through a simulator and compare with the checked-in expected list.

  tools/corpus/run.py MANIFEST.toml [--tool yavs|sim1] [--cmd TEMPLATE] [--keep-going] [--bless]

MANIFEST is a path or a bare name (vunit-2008 -> tools/corpus/vunit-2008.toml). Files are analyzed in manifest
order. For yavs, consecutive files of one library and standard go in one call by default (#745: one analysis
session, as integrations call it); a failing call is repeated file by file, so every file still gets its own
exact result and output (a call that fails while every file passes alone is a DIFF of its own). --per-file (and every reference label) makes one call per file. The result of every file (PASS = exit 0) is compared with <name>.expected (yavs) or
<name>.<suffix>.expected (a reference label), one `PASS|FAIL <lib> <source>/<path>` line per manifest entry in order. Any
difference fails, until the list is updated (--bless rewrites it from this run, which then covers every file).
Without --keep-going the run stops at the first difference (without an expected list: the first FAIL).

Command template words: {std} {lib} {dir} {file}; {map} (one --map=LIB:DIR/LIB per library but std/ieee unless the flags have --bootstrap) and
{flags} (the manifest's [flags] for --tool) expand to several arguments when they are a word of their own; so does {file}
(the files of a call, one for a call per file).
Default templates:
  yavs: yavs analyze --std={std} --work={lib}:{dir}/{lib} {map} {flags} {file}
A reference label (sim1, ...) takes its template, expected-file suffix and optional
standard.vhdl fixup from tools/sims.local.conf (keys corpus_cmd, corpus_suffix, corpus_fixup).
Every call runs under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G` (--no-wrap to disable).
A source with `env = "VAR"` (vendor primitive libraries, unisim-2008) is read in place from $VAR and the
whole manifest is skipped (exit 0) when VAR is unset. --extra-map maps a library built by an earlier run.
Sources are looked up as <root>/<source id>, roots: --root, $YAVS_CORPUS, ~/.cache/rsim-corpus,
~/.cache/rsim-corpora (fetch them with tools/fetch-corpus.sh). Nothing is copied into the repo.
Exit: 0 all as expected, 1 differences, 2 usage/environment error.
"""
import argparse, os, shlex, shutil, subprocess, sys, tomllib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import simconf  # noqa: E402

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
TEMPLATES = {
    "yavs": "yavs analyze --std={std} --work={lib}:{dir}/{lib} {map} {flags} {file}",
}
WRAP = ["timeout", "300", "systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=2G"]


def std_fixup(f, path, out):
    """The first reference bootstraps STD.STANDARD itself: --bootstrap, and for 2019 a copy with
    RANGE_DIRECTION moved behind BOOLEAN (as tools/check-sim1.sh does). Returns (extra args, path)."""
    if f["path"].rsplit("/", 1)[-1] != "standard.vhdl":
        return [], path
    if f["std"] == "2019":
        s = path.read_text(encoding="latin-1").split("\n")
        i = next(k for k, l in enumerate(s) if l.startswith("  type RANGE_DIRECTION is ("))
        j = next(k for k, l in enumerate(s) if l.startswith("  type BOOLEAN is"))
        t = "\n".join(s[:i] + s[j:j + 1] + s[i:j] + s[j + 1:])
        path = out / "standard-fixed.vhdl"
        path.write_text(t, encoding="latin-1")
    return ["--bootstrap"], path


def die(msg):
    print("run.py:", msg, file=sys.stderr)
    sys.exit(2)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("manifest")
    ap.add_argument("--tool", default="yavs", help="yavs or a reference label (sim1, ...)")
    ap.add_argument("--cmd")
    ap.add_argument("--root", action="append", default=[])
    ap.add_argument("--out", help="library directory (default ~/.cache/yavs-corpus-out/<tool>-<name>-<pid>, unique per run)")
    ap.add_argument("--expected", help="expected-results list ('none' to skip the comparison)")
    ap.add_argument("--extra-map", action="append", default=[], metavar="NAME:PATH", help="another library for {map} (e.g. unisim:DIR/unisim from a vendor run)")
    ap.add_argument("--keep-going", action="store_true")
    ap.add_argument("--bless", action="store_true", help="write the expected list from this (complete) run")
    ap.add_argument("--log", help="write the full output of every failed file here")
    ap.add_argument("--no-wrap", action="store_true")
    ap.add_argument("--per-file", action="store_true", help="one call per file (the default for a reference label)")
    a = ap.parse_args()
    ref = a.tool != "yavs"
    if ref and not (a.cmd or simconf.get(a.tool, "corpus_cmd")):
        die(f"{a.tool}: key corpus_cmd not configured (tools/sims.local.conf)")
    suffix = simconf.get(a.tool, "corpus_suffix", a.tool) if ref else ""
    fkey = a.tool  # the manifest [flags] key is the label (sim1, ...) or "yavs"
    fixup = ref and simconf.get(a.tool, "corpus_fixup")

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
        if s.get("env"):  # vendor tree read in place, never fetched or committed
            if not os.environ.get(s["env"]):
                print(f"{name}: skipped, ${s['env']} is not set")
                sys.exit(0)
            base[s["id"]] = Path(os.environ[s["env"]]).expanduser()
            continue
        d = next((r / s["id"] for r in roots if (r / s["id"]).is_dir()), None)
        if d is None:
            die(f"source {s['id']} not found under {[str(r) for r in roots]}; run tools/fetch-corpus.sh")
        base[s["id"]] = d
    out = Path(a.out or f"~/.cache/yavs-corpus-out/{a.tool}-{name}-{os.getpid()}").expanduser()
    # the shared std/ieee store (#564) is private to this tool, never the user's
    os.environ["YAVS_STD_CACHE"] = str(out.resolve().parent / "std")
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)

    exp_path = None if a.expected == "none" else Path(a.expected or HERE / (name + ("." + suffix if ref else "") + ".expected"))
    expected = None
    if exp_path and exp_path.exists() and not a.bless:
        expected = [ln.split(None, 1) for ln in exp_path.read_text().splitlines() if ln.strip() and not ln.startswith("#")]
    elif exp_path and not a.bless:
        print(f"note: {exp_path} missing; stopping at the first FAIL (create it with --bless)")

    tmpl = shlex.split(a.cmd or (simconf.get(a.tool, "corpus_cmd") if ref else TEMPLATES[a.tool]))
    flags = m.get("flags", {}).get(fkey, [])
    libs = sorted({f["lib"] for f in m["file"]} - (set() if "--bootstrap" in flags else {"std", "ieee"}))
    keep = a.keep_going or a.bless
    results, bad, stopped = [], 0, False
    log = open(a.log, "w") if a.log else None

    batch = not ref and not a.per_file and "{file}" in tmpl  # a template with {file} inside a longer word cannot take several files
    prep = []  # per manifest entry: (path, extra args)
    for f in m["file"]:
        extra, path = std_fixup(f, base[f["source"]] / f["path"], out) if fixup else ([], base[f["source"]] / f["path"])
        prep.append((path, extra))
        (out / f["lib"]).parent.mkdir(parents=True, exist_ok=True)

    def gkey(j):  # entries with the same key can share a call
        return m["file"][j]["lib"], m["file"][j]["std"], tuple(prep[j][1])

    def call(ids):
        """One tool call for the entries ids (same library, standard and extra args): (exit status, output)."""
        f0, extra = m["file"][ids[0]], prep[ids[0]][1]
        argv = []
        for t in tmpl:
            if t == "{map}":
                argv += [f"--map={l}:{out}/{l}" for l in libs] + [f"--map={x}" for x in a.extra_map]
            elif t == "{flags}":
                argv += flags + (extra if fixup else [])
            elif t == "{file}":
                argv += [str(prep[i][0]) for i in ids]
            else:
                argv.append(t.format(std=f0["std"], lib=f0["lib"], dir=out, file=prep[ids[0]][0]))
        try:
            p = subprocess.run(argv if a.no_wrap else WRAP + argv, capture_output=True, text=True, errors="replace")
        except OSError as e:
            die(f"cannot run {argv[0]}: {e}")
        return p.returncode, p.stdout + p.stderr

    result = {}  # entry -> (exit status, output), filled a group at a time
    for i, f in enumerate(m["file"]):
        key = f"{f['lib']} {f['source']}/{f['path']}"
        if i not in result:
            ids = [i]
            while batch and i + len(ids) < len(m["file"]) and gkey(i + len(ids)) == gkey(i):
                ids.append(i + len(ids))
            rc, text = call(ids)
            if rc == 0:
                result.update({j: (0, text) for j in ids})
            else:  # a failing call stops at its first failing file: repeat file by file for the exact result of each
                result.update({j: (call([j]) if len(ids) > 1 else (rc, text)) for j in ids})
                if len(ids) > 1 and all(result[j][0] == 0 for j in ids):  # a failure of the session itself (state kept between files, a cap, an ICE)
                    bad += 1
                    print(f"DIFF: the call of entries {ids[0] + 1} to {ids[-1] + 1} failed (exit {rc}) but every file passes alone:\n   " + "\n   ".join(text.strip().splitlines()[:6]))
        rc, text = result.pop(i)
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
