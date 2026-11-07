#!/usr/bin/env python3
"""Self-test of the Verilog test tools' own paths: the static check, a failing analyzer, and the skip notices.

  python3 -I tools/verilog/selftest.py

Needs no vendor install and no yavs binary. Exit 0 when every path behaves as described.
"""
import os, subprocess, sys, tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent


def run(cmd, env=None, expect=0, want_text=()):
    p = subprocess.run(cmd, capture_output=True, text=True, env={**os.environ, **(env or {})})
    ok = p.returncode == expect and all(w in p.stdout for w in want_text)
    print(("ok   " if ok else "FAIL ") + " ".join(cmd[2:])[:150] + (f"  [{env}]" if env else ""))
    if not ok:
        print(f"     exit {p.returncode}, wanted {expect}; wanted text {want_text}\n" + p.stdout[-1500:] + p.stderr[-500:])
    return ok


def main():
    py = [sys.executable, "-I"]
    fx, co = py + [str(HERE / "fixtures.py")], py + [str(HERE / "corpus.py")]
    ok = run(fx + ["--static"])
    # An analyzer that prints nothing is not the expected one: every fixture differs.
    ok &= run(fx + ["--yavs", "true", "--no-wrap"], expect=1, want_text=["FAIL"])
    with tempfile.TemporaryDirectory(prefix="yavs-verilog-selftest-") as tmp:
        ok &= run(co + ["vendor", "--root", str(Path(tmp) / "none")], env={"HOME": tmp, "YAVS_VENDOR_DATA": ""}, want_text=["skipped"])
        ok &= run(co + ["cells", "--root", str(Path(tmp) / "none")], env={"HOME": tmp, "YAVS_CORPUS": ""}, want_text=["skipped"])
    print("selftest", "passed" if ok else "FAILED")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
