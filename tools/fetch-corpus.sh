#!/usr/bin/env bash
# Fetch the external parse corpora at pinned commits into a directory.
#
#   tools/fetch-corpus.sh [DIR]     (default: ~/.cache/rsim-corpus)
#
# Then run the corpus tests with `YAVS_CORPUS=DIR cargo test -p yavs --features corpus`.
# Analysis corpora: `tools/corpus/run.py NAME` (finds the sources here). The corpora are read-only test inputs and are never copied into the repo.
set -euo pipefail

dir=${1:-$HOME/.cache/rsim-corpus}
mkdir -p "$dir"

# fetch NAME URL COMMIT [SPARSE-PATH...]
fetch() {
    local name=$1 url=$2 commit=$3
    shift 3
    local dest=$dir/$name
    if [ -d "$dest/.git" ] && [ "$(git -C "$dest" rev-parse HEAD)" = "$commit" ]; then
        if [ $# -gt 0 ]; then
            git -C "$dest" sparse-checkout set --no-cone "$@"
        fi
        echo "$name: up to date at $commit"
        return
    fi
    if [ -e "$dest" ]; then
        echo "$name: $dest exists at another commit; remove it and rerun" >&2
        exit 1
    fi
    git init -q "$dest"
    git -C "$dest" remote add origin "$url"
    if [ $# -gt 0 ]; then
        git -C "$dest" sparse-checkout set --no-cone "$@"
    fi
    git -C "$dest" fetch -q --depth 1 --filter=blob:none origin "$commit"
    git -C "$dest" -c advice.detachedHead=false checkout -q FETCH_HEAD
    echo "$name: $commit"
}

# fetch_with_submodules NAME URL COMMIT: like fetch, then the submodules at their recorded commits.
fetch_with_submodules() {
    fetch "$@"
    git -C "$dir/$1" submodule update -q --init --depth 1
}

fetch vunit https://github.com/VUnit/vunit d627259f9795f28f3d1f57a3a3de9f9978bd49b8 '/vunit/vhdl/' '/examples/vhdl/'
fetch osvvm https://github.com/OSVVM/OSVVM 2104430dbbd2c7c1fff2ee02e1996944c59b62c2
fetch rust_hdl https://github.com/VHDL-LS/rust_hdl a0fc2019d95e371d4ff3199a36aaaf0ba32c7145 '/vhdl_libraries/'
fetch nvc https://github.com/nickg/nvc ef5084a94be11c7edab41340518e4903f1f6eeb4 '/test/regress/' '/test/parse/' '/test/test_parse.c'
fetch vests https://github.com/nickg/vests f23a8cd103dd00ba58c1de434aea72a29efcd514
fetch ghdl https://github.com/ghdl/ghdl 71b3e1ac7088489b1214e0c626e00d2dfad7c6d9 '/testsuite/gna/'
# Analysis corpora (tools/corpus/*.toml): standalone OSVVM (tag 2026.09, with its library submodules) and UVVM.
fetch_with_submodules osvvm-libraries https://github.com/OSVVM/OsvvmLibraries ab47a2637008a0709bbf593aa1ab0060dcc39aac
fetch uvvm https://github.com/UVVM/UVVM 90d56e93c542bf0d5e2ab9f791cc7395bd1aa896
# Third-party designs the UVVM maintenance testbenches of the SPI and I2C VIPs need (tools/suites/uvvm.py maps them back).
fetch uvvm-supplementary https://github.com/UVVM/UVVM_SUPPLEMENTARY c91e3d7365a9aa0c0d67708e7bc9a5e3db9d8a75 '/bitvis_vip_spi/tb/maintenance_tb/spi_master_slave_opencores/'
fetch uvvm-i2c-master https://github.com/freecores/i2c 3b067f00ccced753b0502024766a51f58f3e04bc '/rtl/vhdl/'
fetch uvvm-i2c-slave https://github.com/oetr/FPGA-I2C-Minion dde7b0fc71241a8821c1dd21ef7b1716e12c31dc '/I2C_slave.vhd' '/debounce.vhd' '/txt_util.vhd'
# hdl-modules release v6.2.1 (src/ and rtl/ of each module are the analysis corpus).
fetch hdl-modules https://github.com/hdl-modules/hdl-modules c4f01c4bb69aab66c879f545e80abb477f1ebf35

# Verilog cell models for the Verilog analysis gate (tools/verilog/corpus.py cells): yosys techlibs/*/*sim*.v, ISC (COPYING); the quicklogic
# qlf_k6n10f cells_sim.v, brams_sim.v and dsp_sim.v carry an Apache-2.0 header (F4PGA). bram_types_sim.v is generated at build time and not in the checkout.
fetch yosys https://github.com/YosysHQ/yosys c30457480f8a905e252fb09ea688713a86e9da49 '/techlibs/' '/COPYING'

# Verilog test and design corpora (tools/verilog/corpus.py, one manifest each; inputs only, never committed). Licences: ivtest GPL-2.0 (an
# open-source Verilog simulator's regression suite), picorv32 ISC, verilog-ethernet/-axi/-uart/-axis/-pcie MIT, wb2axip Apache-2.0, zipcpu GPL-3.0 (the repository's
# rtl/ only), darkriscv BSD-3-Clause, serv ISC, ultraembedded riscv BSD-3-Clause and core_usb_host GPL (version 3 text, "License: GPL" in the headers), openMSP430 BSD-3-Clause, the OpenCores ethmac (LGPL) and
# wb_dma (free-use notice), dawsonjon/fpu MIT; cell models: XilinxUnisimLibrary Apache-2.0, sky130_fd_sc_hd Apache-2.0, f4pga-arch-defs ISC (the sim models only).
fetch ivtest https://github.com/steveicarus/ivtest a19e629a1879801ffcc6f2e6256ca435c20570f3
fetch picorv32 https://github.com/YosysHQ/picorv32 ef203c2b0a3fb793280f5114941416c425c5b461
fetch verilog-ethernet https://github.com/alexforencich/verilog-ethernet 77320a9471d19c7dd383914bc049e02d9f4f1ffb '/rtl/' '/COPYING'
fetch verilog-axi https://github.com/alexforencich/verilog-axi 516bd5dadc3365b7f9e225d2af8fe0b8d804fe53 '/rtl/' '/COPYING'
fetch verilog-uart https://github.com/alexforencich/verilog-uart 1b867e53af738e4a8bc7c839ca2f1c07f40382dc '/rtl/' '/COPYING'
fetch verilog-axis https://github.com/alexforencich/verilog-axis 48ff7a7e2ef782cf778d47910cf85835c64b1bce '/rtl/' '/COPYING'
fetch verilog-pcie https://github.com/alexforencich/verilog-pcie 25156a9a162c41c60f11f41590c7d006d015ae5a '/rtl/' '/COPYING'
fetch wb2axip https://github.com/ZipCPU/wb2axip 2e8d3bc2d26ddc33d1881022a2a2b9d3f0c16b9b '/rtl/' '/LICENSE*' '/COPYING*'
fetch zipcpu https://github.com/ZipCPU/zipcpu 42606d2d6ef55df313772232977621b2d72f0159 '/rtl/' '/COPYING*'
fetch darkriscv https://github.com/darklife/darkriscv 955511961f139c54d3385063cd9bf40f4dddadd8 '/rtl/' '/LICENSE*'
fetch serv https://github.com/olofk/serv f200eb2ed7b69ac1c6b8eddd47654522aeee5ce8 '/rtl/' '/LICENSE*'
fetch ultraembedded-riscv https://github.com/ultraembedded/riscv 7ae6f803e30f78c6ea3121e73c3adf50ff912730 '/core/' '/top_cache_axi/' '/top_tcm_axi/' '/top_tcm_wrapper/' '/LICENSE*'
fetch ultraembedded-usb-host https://github.com/ultraembedded/core_usb_host 81eb9f131dbb434a9047ae074fea5c31ef46ce5d '/src_v/' '/LICENSE*'
fetch openmsp430 https://github.com/olgirard/openmsp430 92c883abb4518dbc35b027e6cad5ffef5b2fbb81 '/core/rtl/' '/core/doc/' '/LICENSE*'
fetch opencores-ethmac https://github.com/freecores/ethmac dd26899086edf3b797d2775ef9502d204a9a8149 '/rtl/' '/README*'
fetch opencores-wb-dma https://github.com/freecores/wb_dma 56126339e2cef66c8769c3f8516ba7222063e851 '/rtl/'
fetch dawsonjon-fpu https://github.com/dawsonjon/fpu c45d5e0a9f1b7e945e2be6770dbc1203825a05c4 '/*/' '/COPYING*' '/README*'
fetch xilinx-unisim-lib https://github.com/Xilinx/XilinxUnisimLibrary 1c8e05fd1e9a79ceb8b996a0996674122eed086f '/verilog/' '/LICENSE'
fetch sky130-fd-sc-hd https://github.com/google/skywater-pdk-libs-sky130_fd_sc_hd ac7fb61f06e6470b94e8afdf7c25268f62fbd7b1 '/cells/**/*.v' '/models/**/*.v' '/LICENSE'
# The yosys regression designs (ISC): classic and corner-case Verilog, partly SystemVerilog (tests/errors is invalid on purpose and not fetched).
fetch yosys-tests https://github.com/YosysHQ/yosys c30457480f8a905e252fb09ea688713a86e9da49 '/tests/simple/' '/tests/asicworld/' '/tests/hana/' '/tests/various/' '/tests/verilog/' '/tests/memories/' '/tests/opt/' '/COPYING'
fetch f4pga-arch-defs https://github.com/f4pga/f4pga-arch-defs 3d4df2ea79dca19b8c5c155e0396e84f52e50451 '/xilinx/*.sim.v' '/xilinx/**/*.sim.v' '/quicklogic/**/*.sim.v' '/**/techmap/cells_sim.v' '/COPYING' '/LICENSE*'

# External library corpora (tools/corpus/gen_external.py): shallow, one pinned commit each.
fetch open-logic https://github.com/open-logic/open-logic d9f123aae1c6022288782668067311f9794da0a2
fetch en_cl_fix https://github.com/open-logic/en_cl_fix ecd7dd026837997f2cd8bc4861769abc6cbbc73c
# PoC Library v3.0.0 (Open Source VHDL Group; the OSVVM-based test benches need its OSVVM submodules).
fetch_with_submodules poc https://github.com/VHDL/PoC b90402732980e3fdbc4d2339c0b7d16dbc0b3d79
fetch json-for-vhdl https://github.com/Paebbels/JSON-for-VHDL 2ab1ebc2c788ececce152a49ffda706f84570e95
fetch vhdl-extras https://github.com/kevinpt/vhdl-extras b8916c4942e99d02fac04e25799cb2c996eb5870
fetch satcat5 https://github.com/the-aerospace-corporation/satcat5 58402dcbe64adc9c30d00c08edc20e3aef314ac6
fetch vhdl-compliance https://github.com/VHDL/Compliance-Tests 7c4449ee5027d933ee3d7c0465bd0a5d06f9c05e
fetch vhdl-interfaces https://github.com/VHDL/Interfaces d2fd277fdf3b4606b9bc335751077b6303ad683f
fetch vhdl-tool-checks https://github.com/Paebbels/VHDL-Tool-Checks 08cf0cc8238da587502fd96d525b4c992867ae76
fetch fletcher https://github.com/abs-tudelft/fletcher 45be6d6cb2c9546cee151c657dc675e03b11e302
fetch vhlib https://github.com/abs-tudelft/vhlib b69bbdaf7dacc06c861dd0982bb6653482912a76
fetch hvhdl-float https://github.com/hVHDL/hVHDL_floating_point 345b06cec458b3ad42ee2563deab817d52d8fdee
fetch neotrng https://github.com/stnolting/neoTRNG c688149f70fbd582eac1029cbfec4fcf97b719b2
fetch vhdl-ideas https://github.com/bpadalino/vhdl-ideas f5970bd9f66bf12179f651ab43bfa0db946a7035
# '/sw/' (runtime, linker script, image generator) builds the CoreMark image of tools/realbench/programs.py.
fetch neorv32 https://github.com/stnolting/neorv32 b2620454ebb2407e81a6a1965ef52e2a860b0432 '/rtl/' '/sim/' '/sw/' '/LICENSE'
# CPU benchmark programs (tools/realbench/programs.py, category cpubench): the NEORV32 CoreMark port (Apache-2.0), EEMBC CoreMark (Apache-2.0)
# and Microwatt (CC-BY-4.0 with IBM's additional-rights notice, see its LICENSE; fpga/LICENSE covers the files in fpga/). Built into images in the realbench cache, never committed.
fetch neorv32-coremark https://github.com/stnolting/neorv32-coremark e1553f10106faf3edfb34d86780b414782bc2aea
fetch eembc-coremark https://github.com/eembc/coremark 1f483d5b8316753a742cbf5590caf5bd0a4e4777
fetch microwatt https://github.com/antonblanchard/microwatt f341bed306a9a7281bf65200df8cc1f0a002617d '/*.vhdl' '/*.in' '/*.c' '/*.h' '/microwatt.core' '/fpga/' '/sim-unisim/' '/hello_world/' '/lib/' '/include/' '/micropython/' '/scripts/' '/LICENSE'
fetch surf https://github.com/slaclab/surf f0b5e3a7b5ab191be1cb5f4453e418299181c9d8 '*.vhd' '*.vhdl' '/LICENSE*'
# GRLIB IP library with the LEON3 and NOEL-V templates (GPL-2.0), release 2026.3 build 4301. The commit is the tagged release in a
# git mirror and is file-identical to the official tarball (https://download.gaisler.com/products/GRLIB/bin/grlib-gpl-2026.3-b4301.tar.gz,
# sha256 0978edc1c3c9fd20a9e06f6f283a1d95eb72778eacfa645ec1ab3f9d742253f2; only empty directories and the mirror's README differ).
fetch grlib https://github.com/TUT-ASI/leon3-grlib-gpl-mirror 020f83888d67e140bdca248f51fcc783fd8f99a0 '/Makefile' '/bin/' '/lib/' '/boards/gr-cpci-xc4v/' '/software/leon3/' '/software/noelv/' '/designs/leon3mp/' '/designs/leon3-minimal/' '/designs/noelv-generic/'
