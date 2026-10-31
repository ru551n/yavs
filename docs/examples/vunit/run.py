"""The counter of the project walkthrough, tested under VUnit with yavs as the simulator"""

import os
import sys
from pathlib import Path

os.environ.setdefault("VUNIT_SIMULATOR", "yavs")

from vunit import VUnit  # pylint: disable=wrong-import-position

ROOT = Path(__file__).parent

vu = VUnit.from_argv()
vu.add_package("vunit-yavs", allow_setup=True)  # first: it provides the simulator
vu.add_vhdl_builtins()

vu.add_library("rtl").add_source_files(ROOT.parent / "project" / "rtl" / "*.vhd")
tb = vu.add_library("tb")
tb.add_source_files(ROOT / "tb_counter.vhd")

# One simulation per configuration, each with its own value of the generic
tb_counter = tb.test_bench("tb_counter")
for width in (3, 5):
    tb_counter.add_config(name=f"width_{width}", generics={"WIDTH": width})


def merge_coverage(results):
    """merge_coverage is an error when no simulation collected coverage"""
    if "--yavs-cov" in sys.argv:
        results.merge_coverage("coverage", args=["--exclude", str(ROOT / "coverage-exclude.txt")])


vu.main(post_run=merge_coverage)
