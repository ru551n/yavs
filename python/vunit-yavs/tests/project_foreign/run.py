# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this file,
# You can obtain one at http://mozilla.org/MPL/2.0/.
#
# Copyright (c) 2014-2026, Lars Asplund lars.anders.asplund@gmail.com

"""
A project whose VHDL calls a native library, and Python embedded in it, through foreign
subprograms. NATIVE_DEMO_PYTHON=1 builds the library with Python.
"""

import os
from pathlib import Path

from vunit import VUnit

VU = VUnit.from_argv()
VU.add_package("vunit-yavs", allow_setup=True)
VU.add_vhdl_builtins()
VU.add_package("native_demo", allow_setup=True)
LIB = VU.add_library("lib")
LIB.add_source_files(Path(__file__).parent / "tb_native.vhd")
if os.environ.get("NATIVE_DEMO_PYTHON") == "1":
    LIB.add_source_files(Path(__file__).parent / "tb_python.vhd")
VU.main()
