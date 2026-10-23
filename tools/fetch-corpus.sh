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

# External library corpora (tools/corpus/gen_external.py): shallow, one pinned commit each.
fetch open-logic https://github.com/open-logic/open-logic d9f123aae1c6022288782668067311f9794da0a2
fetch en_cl_fix https://github.com/open-logic/en_cl_fix ecd7dd026837997f2cd8bc4861769abc6cbbc73c
fetch poc https://github.com/VLSI-EDA/PoC 8c39b2407a97ec81b8601c4b64e4d2e2141ab9cf
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
fetch neorv32 https://github.com/stnolting/neorv32 b2620454ebb2407e81a6a1965ef52e2a860b0432 '/rtl/' '/sim/' '/LICENSE'
fetch surf https://github.com/slaclab/surf f0b5e3a7b5ab191be1cb5f4453e418299181c9d8 '*.vhd' '*.vhdl' '/LICENSE*'
# GRLIB IP library with the LEON3 and NOEL-V templates (GPL-2.0), release 2026.3 build 4301. The commit is the tagged release in a
# git mirror and is file-identical to the official tarball (https://download.gaisler.com/products/GRLIB/bin/grlib-gpl-2026.3-b4301.tar.gz,
# sha256 0978edc1c3c9fd20a9e06f6f283a1d95eb72778eacfa645ec1ab3f9d742253f2; only empty directories and the mirror's README differ).
fetch grlib https://github.com/TUT-ASI/leon3-grlib-gpl-mirror 020f83888d67e140bdca248f51fcc783fd8f99a0 '/Makefile' '/bin/' '/lib/' '/boards/gr-cpci-xc4v/' '/software/leon3/' '/software/noelv/' '/designs/leon3mp/' '/designs/leon3-minimal/' '/designs/noelv-generic/'
