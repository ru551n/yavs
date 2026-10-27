#!/bin/bash
# Builds the libLLVM that the Linux release archives with the compiled tier ship (docs/decisions.md
# "Release archives with and without the compiled tier"). Run it inside a manylinux2014 container
# (glibc 2.17, devtoolset GCC): the result needs nothing beyond libc, libm, libdl, libpthread and
# librt, so the archive keeps the glibc 2.17 floor of the Linux archives.
#
#   build-libllvm.sh PREFIX [WORKDIR]
#
# PREFIX gets bin/llvm-config, include/ and lib/libLLVM.so.21.1 (one shared library, the C API
# only, the host architecture as the only target, no zlib/zstd/xml2/edit/ffi/terminfo). Point
# LLVM_SYS_211_PREFIX at it to build yavs with `--features llvm`.
set -euo pipefail
ver=21.1.8
sha=4633a23617fa31a3ea51242586ea7fb1da7140e426bd62fc164261fe036aa142 # llvm-project-$ver.src.tar.xz
prefix=$(realpath -m "${1:?usage: build-libllvm.sh PREFIX [WORKDIR]}")
work=$(realpath -m "${2:-${TMPDIR:-/var/tmp}/yavs-libllvm}")
jobs=${LLVM_BUILD_JOBS:-$(nproc)}
case "$(uname -m)" in
  x86_64) target=X86 ;;
  aarch64) target=AArch64 ;;
  *) echo "build-libllvm.sh: unsupported architecture $(uname -m)"; exit 1 ;;
esac
mkdir -p "$work"
cd "$work"
# manylinux2014 has cmake and a devtoolset GCC; ninja and Python come from the Python tools.
export PATH=/opt/python/cp312-cp312/bin:$PATH
command -v ninja >/dev/null || pip install -q ninja
src=llvm-project-$ver.src.tar.xz
if ! echo "$sha  $src" | sha256sum -c - >/dev/null 2>&1; then
  curl -fsSLo "$src" "https://github.com/llvm/llvm-project/releases/download/llvmorg-$ver/$src"
  echo "$sha  $src" | sha256sum -c -
fi
rm -rf "llvm-project-$ver.src" build
tar xf "$src" "llvm-project-$ver.src/llvm" "llvm-project-$ver.src/cmake" "llvm-project-$ver.src/third-party"
cmake -S "llvm-project-$ver.src/llvm" -B build -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$prefix" \
  -DLLVM_TARGETS_TO_BUILD="$target" \
  -DLLVM_BUILD_LLVM_DYLIB=ON -DLLVM_LINK_LLVM_DYLIB=ON -DLLVM_STATIC_LINK_CXX_STDLIB=ON \
  -DLLVM_ENABLE_ASSERTIONS=OFF -DLLVM_ENABLE_ZLIB=OFF -DLLVM_ENABLE_ZSTD=OFF \
  -DLLVM_ENABLE_LIBXML2=OFF -DLLVM_ENABLE_LIBEDIT=OFF -DLLVM_ENABLE_FFI=OFF \
  -DLLVM_ENABLE_TERMINFO=OFF -DLLVM_ENABLE_BINDINGS=OFF -DLLVM_ENABLE_Z3_SOLVER=OFF \
  -DLLVM_INCLUDE_TESTS=OFF -DLLVM_INCLUDE_EXAMPLES=OFF -DLLVM_INCLUDE_BENCHMARKS=OFF \
  -DLLVM_INCLUDE_DOCS=OFF -DCMAKE_SHARED_LINKER_FLAGS=-static-libgcc
ninja -C build -j "$jobs" install-LLVM install-llvm-config install-llvm-headers
strip --strip-unneeded "$prefix"/lib/libLLVM.so.*.*
# The same gates as the yavs binary: no GLIBC symbol above 2.17, and no library but libc's own.
lib=$(ls "$prefix"/lib/libLLVM.so.*.*)
v=$(objdump -T "$lib" | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -n1)
echo "$lib: highest glibc symbol version: $v"
[[ "$(printf '%s\n' GLIBC_2.17 "$v" | sort -V | tail -n1)" == GLIBC_2.17 ]] || { echo "needs more than glibc 2.17"; exit 1; }
needed=$(objdump -p "$lib" | awk '$1 == "NEEDED" {print $2}' | grep -vE '^(libc|libm|libdl|libpthread|librt|ld-linux.*)\.so' || true)
[[ -z "$needed" ]] || { echo "$lib needs more libraries: $needed"; exit 1; }
