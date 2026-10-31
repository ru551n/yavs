# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this file,
# You can obtain one at http://mozilla.org/MPL/2.0/.
#
# Copyright (c) 2014-2026, Lars Asplund lars.anders.asplund@gmail.com

"""
A tiny project run on yavs through the package route
"""

import os
from pathlib import Path
from vunit import VUnit

VU = VUnit.from_argv()
# The simulator package goes first: it must be added before anything selects the simulator
VU.add_package("vunit-yavs", allow_setup=True)
VU.add_vhdl_builtins()
LIB = VU.add_library("lib")
LIB.add_source_files(Path(__file__).parent / "tb_yavs.vhd")
if "EXTRA_VHDL" in os.environ:
    LIB.add_source_files(os.environ["EXTRA_VHDL"])

def post_run(results):
    """Merge the coverage databases when the test asks for it"""
    results.merge_coverage(os.environ["RUN_PY_MERGE_COVERAGE"], args=os.environ.get("RUN_PY_COVERAGE_ARGS", "").split())


VU.main(post_run=post_run if "RUN_PY_MERGE_COVERAGE" in os.environ else None)
