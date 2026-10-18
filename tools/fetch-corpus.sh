#!/usr/bin/env bash
# Fetch the external parse corpora at pinned commits into a directory.
#
#   tools/fetch-corpus.sh [DIR]     (default: ~/.cache/rsim-corpus)
#
# Then run the corpus tests with `RSIM_CORPUS=DIR cargo test --test corpus`.
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

fetch vunit https://github.com/VUnit/vunit d627259f9795f28f3d1f57a3a3de9f9978bd49b8 '/vunit/vhdl/'
fetch osvvm https://github.com/OSVVM/OSVVM 2104430dbbd2c7c1fff2ee02e1996944c59b62c2
fetch rust_hdl https://github.com/VHDL-LS/rust_hdl a0fc2019d95e371d4ff3199a36aaaf0ba32c7145 '/vhdl_libraries/'
fetch nvc https://github.com/nickg/nvc ef5084a94be11c7edab41340518e4903f1f6eeb4 '/test/regress/' '/test/parse/' '/test/test_parse.c'
fetch vests https://github.com/nickg/vests f23a8cd103dd00ba58c1de434aea72a29efcd514
fetch ghdl https://github.com/ghdl/ghdl 71b3e1ac7088489b1214e0c626e00d2dfad7c6d9 '/testsuite/gna/'
# Analysis corpora (tools/corpus/*.toml): standalone OSVVM (tag 2026.09, with its library submodules) and UVVM.
fetch_with_submodules osvvm-libraries https://github.com/OSVVM/OsvvmLibraries ab47a2637008a0709bbf593aa1ab0060dcc39aac
fetch uvvm https://github.com/UVVM/UVVM 90d56e93c542bf0d5e2ab9f791cc7395bd1aa896
# hdl-modules release v6.2.1 (src/ and rtl/ of each module are the analysis corpus).
fetch hdl-modules https://github.com/hdl-modules/hdl-modules c4f01c4bb69aab66c879f545e80abb477f1ebf35
