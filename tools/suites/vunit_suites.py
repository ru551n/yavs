#!/usr/bin/env python3
"""Run the test runner's own VHDL suites (vunit/vhdl/<suite>/run.py) on yavs through vunit-yavs (#112).

    vunit_suites.py [--vhdl DIR] [--yavs BIN] [--out DIR] [-p N] [--std 2008|2019] [--mem M] [--only SUITE,...]

Needs the packages `vunit_hdl` and `vunit-yavs` in the Python running this script. DIR is the
`vunit/vhdl` directory of the installed test runner unless given (a source checkout works too).
Each suite's run script is executed unchanged after the yavs interface is registered, with the
suite's own generics and configurations. Prints pass / fail / skip per suite and exits 1 on any
failure; writes DIR/results.tsv and DIR/<suite>.log.
"""

import argparse
import os
import shutil
import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

SUITES = [
    "check", "com", "data_types", "dictionary", "logging", "path", "random", "run", "string_ops",
    "verification_components",
]  # fmt: skip

# Registers the interface that vunit-yavs provides, then runs the suite's own script as __main__
WRAPPER = """
import runpy, sys
from vunit.sim_if.factory import SIMULATOR_FACTORY
from vunit_yavs import YavsInterface
SIMULATOR_FACTORY.register_simulator(YavsInterface)
script = sys.argv[1]
sys.argv = [script] + sys.argv[2:]
runpy.run_path(script, run_name="__main__")
"""


def default_vhdl_dir():
    """The vunit/vhdl directory of the installed test runner"""
    import vunit  # pylint: disable=import-outside-toplevel

    return Path(vunit.__file__).parent / "vhdl"


def count(xml_file):
    """(passed, failed, skipped) of a JUnit file"""
    root = ET.parse(xml_file).getroot()
    cases = list(root.iter("testcase"))
    failed = sum(1 for c in cases if c.find("failure") is not None or c.find("error") is not None)
    skipped = sum(1 for c in cases if c.find("skipped") is not None)
    return len(cases) - failed - skipped, failed, skipped


def run_suite(vhdl, suite, args, out):
    """Run one suite; returns (passed, failed, skipped, exit status)"""
    xml = out / f"{suite}.xml"
    xml.unlink(missing_ok=True)
    env = dict(os.environ, VUNIT_SIMULATOR="yavs", VUNIT_YAVS_PATH=str(Path(args.yavs).parent))
    env.setdefault("YAVS_STD_CACHE", str(out / "std"))
    env["VUNIT_VHDL_STANDARD"] = args.std
    cmd = [sys.executable, "-c", WRAPPER, "run.py", "-p", str(args.p), "--xunit-xml", str(xml)]
    cmd += ["--output-path", str(out / suite)]
    if shutil.which("systemd-run"):
        cmd = ["systemd-run", "--user", "--scope", "-q", "-p", f"MemoryMax={args.mem}"] + cmd
    with open(out / f"{suite}.log", "wb") as log:
        status = subprocess.run(cmd, cwd=vhdl / suite, env=env, stdout=log, stderr=subprocess.STDOUT, check=False, timeout=3000).returncode
    return (*count(xml), status) if xml.exists() else (0, 0, 0, status)


def main():
    """Entry point"""
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--vhdl", type=Path, default=None, help="the vunit/vhdl directory (default: the installed one)")
    parser.add_argument("--yavs", default=shutil.which("yavs"), help="the yavs executable (default: on PATH)")
    parser.add_argument("--out", type=Path, required=True, help="output directory, outside the repository")
    parser.add_argument("-p", type=int, default=4, help="simulations at once per suite (default 4)")
    parser.add_argument("--std", default="2008", choices=["2008", "2019"], help="VHDL standard (default 2008)")
    parser.add_argument("--mem", default="12G", help="MemoryMax of a suite (default 12G)")
    parser.add_argument("--only", default="", help="comma-separated suites (default: all)")
    args = parser.parse_args()
    if not args.yavs:
        sys.exit("vunit_suites.py: no yavs on PATH; pass --yavs BIN")
    vhdl = args.vhdl or default_vhdl_dir()
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    chosen = [s for s in args.only.split(",") if s] or [s for s in SUITES if (vhdl / s / "run.py").exists()]
    failed = False
    rows = []
    for suite in chosen:
        passed, bad, skipped, status = run_suite(vhdl, suite, args, args.out)
        rows.append((suite, passed, bad, skipped, status))
        failed |= bad > 0 or status != 0 or passed == 0
        print(f"{suite}: pass {passed} fail {bad} skip {skipped} (exit {status})", flush=True)
    (args.out / "results.tsv").write_text(
        "".join("\t".join(map(str, r)) + "\n" for r in [("suite", "pass", "fail", "skip", "exit")] + rows), encoding="utf-8"
    )
    print(f"total: pass {sum(r[1] for r in rows)} fail {sum(r[2] for r in rows)} skip {sum(r[3] for r in rows)}")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
