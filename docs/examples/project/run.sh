#!/bin/sh
# Build the libraries and run the testbench. Pass more `yavs run` options as arguments.
set -e
cd "$(dirname "$0")"

yavs analyze --work=rtl:build/rtl rtl/counter_pkg.vhd rtl/counter.vhd
yavs analyze --work=work:build/work --map=rtl:build/rtl tb/counter_tb.vhd
yavs run --work=work:build/work --map=rtl:build/rtl counter_tb "$@"
