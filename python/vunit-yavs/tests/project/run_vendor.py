# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this file,
# You can obtain one at http://mozilla.org/MPL/2.0/.
#
# Copyright (c) 2014-2026, Lars Asplund lars.anders.asplund@gmail.com

"""
A testbench on vendor primitives, the libraries analyzed from the install in XILINX_VIVADO
"""

import os
from pathlib import Path
from vunit import VUnit

SRC = Path(os.environ["XILINX_VIVADO"]) / "data" / "vhdl" / "src"

VU = VUnit.from_argv()
VU.add_package("vunit-yavs", allow_setup=True)
VU.add_vhdl_builtins()
UNISIM = VU.add_library("unisim")
UNISIM.add_source_files([SRC / "unisims" / "unisim_VPKG.vhd", SRC / "unisims" / "unisim_retarget_VCOMP.vhd"])
UNISIM.add_source_files(SRC / "unisims" / "primitive" / "*.vhd")
UNISIM.add_source_files(SRC / "unisims" / "retarget" / "*.vhd")
VU.add_library("unimacro").add_source_files(SRC / "unimacro" / "*.vhd")
VU.add_library("lib").add_source_files(Path(__file__).parent / "tb_vendor.vhd")
VU.main()
