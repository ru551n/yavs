# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this file,
# You can obtain one at http://mozilla.org/MPL/2.0/.
#
# Copyright (c) 2014-2026, Lars Asplund lars.anders.asplund@gmail.com

"""
A tiny project run on yavs through the package route
"""

from pathlib import Path
from vunit import VUnit

VU = VUnit.from_argv()
# The simulator package goes first: it must be added before anything selects the simulator
VU.add_package("vunit-yavs", allow_setup=True)
VU.add_vhdl_builtins()
VU.add_library("lib").add_source_files(Path(__file__).parent / "tb_yavs.vhd")
VU.main()
