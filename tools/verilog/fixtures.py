#!/usr/bin/env python3
"""Check the hand-written Verilog fixtures of tests/verilog/ci against `yavs analyze`.

  tools/verilog/fixtures.py [--static] [--yavs CMD] [--dir DIR] [--no-wrap]

--static only checks the manifest against the files (no analyzer needed): every fixture is listed, units are declared in the
file, positions lie inside it. Otherwise each manifest line is one `yavs analyze` call in the fixture directory and the
diagnostics (errors and warnings; notes are ignored) must equal the line's `diag=` list. Format: tests/verilog/README.md.
Exit: 0 all as expected, 1 differences, 2 usage or environment error.
"""
import argparse, re, shlex, sys, tempfile, os
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import vcommon  # noqa: E402

HERE = Path(__file__).resolve().parent
FIXTURES = HERE.parents[1] / "tests/verilog/ci"
EXT = (".v", ".sv")


def load(d):
    rows = []
    for n, ln in enumerate((d / "fixtures.expected").read_text().splitlines(), 1):
        if not ln.strip() or ln.startswith("#"):
            continue
        t = shlex.split(ln)
        r = {"line": n, "status": t[0], "file": t[1], "args": [], "diag": [], "units": [], "later": ""}
        if r["status"] not in ("OK", "ERR"):
            vcommon.die("fixtures.py", f"fixtures.expected:{n}: status must be OK or ERR")
        for kv in t[2:]:
            k, _, v = kv.partition("=")
            if k == "args":
                r["args"] = shlex.split(v)
            elif k == "diag":
                for d_ in v.split(","):
                    m = re.fullmatch(r"([EW](?:\d{4}|\*))(?:@(\d+):(\d+))?", d_)
                    if not m:
                        vcommon.die("fixtures.py", f"fixtures.expected:{n}: bad diag {d_!r}")
                    r["diag"].append((m[1], int(m[2]) if m[2] else None, int(m[3]) if m[3] else None))
            elif k == "units":
                r["units"] = v.split(",")
            elif k == "later":
                r["later"] = v
            else:
                vcommon.die("fixtures.py", f"fixtures.expected:{n}: unknown key {k!r}")
        if (r["status"] == "ERR") != any(c[0] == "E" for c, _, _ in r["diag"]):
            vcommon.die("fixtures.py", f"fixtures.expected:{n}: status {r['status']} disagrees with the diag list")
        rows.append(r)
    return rows


def static(d, rows):
    bad = []
    listed = {r["file"] for r in rows}
    for f in sorted(p.name for p in d.iterdir() if p.suffix in EXT):
        if f not in listed:
            bad.append(f"{f}: not in fixtures.expected")
    for r in rows:
        p = d / r["file"]
        if not p.is_file():
            bad.append(f"fixtures.expected:{r['line']}: {r['file']} does not exist")
            continue
        text = p.read_text()
        for u in r["units"]:
            if not re.search(rf"^\s*(?:module|primitive)\s+{re.escape(u)}\b", text, re.M):
                bad.append(f"{r['file']}: unit {u} is not declared")
        n = text.count("\n") + 1
        for c, ln, col in r["diag"]:
            if ln and not (1 <= ln <= n and 1 <= col <= len(text.split("\n")[ln - 1]) + 1):
                bad.append(f"{r['file']}: position {ln}:{col} is outside the file")
    return bad


def compare(r, rc, diags):
    """Differences between the expected diag list and what the analyzer printed (errors and warnings only)."""
    want, got, bad = list(r["diag"]), [x for x in diags if x[0] in ("error", "warning")], []
    for lvl, code, _, ln, col in list(got):
        i = next((i for i, (c, l, k) in enumerate(want) if (c == code or (c == "W*" and lvl == "warning")) and l in (None, ln) and k in (None, col)), None)
        if i is None:
            bad.append(f"unexpected {code} at {ln}:{col}")
        else:
            want.pop(i)
    bad += [f"missing {c}" + (f" at {l}:{k}" if l else "") for c, l, k in want]
    if (rc != 0) != (r["status"] == "ERR"):
        bad.append(f"exit status {rc} for status {r['status']}")
    return bad


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--static", action="store_true")
    ap.add_argument("--yavs")
    ap.add_argument("--dir", type=Path, default=FIXTURES)
    ap.add_argument("--no-wrap", action="store_true")
    a = ap.parse_args()
    d = a.dir.resolve()
    rows = load(d)
    bad = static(d, rows)
    for b in bad:
        print("DIFF:", b)
    if not a.static:
        yavs = vcommon.find_yavs(a.yavs)
        with tempfile.TemporaryDirectory(prefix="yavs-verilog-fixtures-") as tmp:
            os.environ["YAVS_STD_CACHE"] = os.path.join(tmp, "std")
            for i, r in enumerate(rows):
                rc, diags, text = vcommon.analyze(yavs, os.path.join(tmp, f"w{i}"), "work", r["args"], [r["file"]], d, not a.no_wrap)
                b = compare(r, rc, diags)
                print(("FAIL " if b else "PASS ") + r["file"] + (" " + shlex.join(r["args"]) if r["args"] else ""))
                for x in b:
                    print("   ", x)
                if b:
                    bad.append(r["file"])
                    print("    output:", " | ".join(x[:160] for x in text.strip().splitlines()[:3]))
    print(f"{len(rows)} fixtures, {len(bad)} differences")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
