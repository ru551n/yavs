# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this file,
# You can obtain one at http://mozilla.org/MPL/2.0/.
#
# Copyright (c) 2014-2026, Lars Asplund lars.anders.asplund@gmail.com

"""
A testbench on vendor primitives, the libraries built from the install in XILINX_VIVADO
"""

from pathlib import Path
from vunit import VUnit
from vunit_yavs import add_vivado_libraries

VU = VUnit.from_argv()
VU.add_package("vunit-yavs", allow_setup=True)
VU.add_vhdl_builtins()
add_vivado_libraries(VU)
VU.add_library("lib").add_source_files(Path(__file__).parent / "tb_vendor.vhd")
VU.main()
