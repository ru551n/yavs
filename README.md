# yavs

[![Documentation](https://img.shields.io/readthedocs/yavs?label=documentation)](https://yavs.readthedocs.io)
[![Repository](https://img.shields.io/badge/github-ru551n%2Fyavs-blue?logo=github)](https://github.com/ru551n/yavs)
[![Release](https://img.shields.io/github/v/release/ru551n/yavs?include_prereleases)](https://github.com/ru551n/yavs/releases)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)
[![CI status](https://github.com/ru551n/yavs/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ru551n/yavs/actions/workflows/ci.yml)

Yet Another VHDL Simulator: VHDL-2008 and VHDL-2019 in Rust.

## Overview

yavs has its own parser and semantic analysis, an IR interpreter, a parallel simulation kernel
and diagnostics with source snippets and VHDL call stacks. VHDL-2008 and 2019 files mix in one
project, and the IEEE packages are built into the executable. VHPI, a JIT-compiled tier,
waveforms, a debug server and a GUI are planned. yavs is early: expect gaps and please
[report](https://github.com/ru551n/yavs/issues) what breaks.

## Installation

Download the archive for your platform (Linux x86_64, Linux aarch64, Windows x86_64) from the
[releases](https://github.com/ru551n/yavs/releases) page, unpack it and put the directory on
`PATH`. aarch64 is also known as arm64; run `uname -m` to see which one you need. The Linux
archives require glibc 2.17 or newer (RHEL/CentOS 7+, Debian 8+, Ubuntu 14.04+); on musl-based
systems such as Alpine, install the `gcompat` package. Or build from source with Rust stable and a C compiler:

```sh
cargo build --release -p yavs   # target/release/yavs
```

## Basic Example

```sh
yavs analyze --std=2008 --work=work:out/work tb.vhd
yavs run --work=work:out/work tb
```

See the [documentation](https://yavs.readthedocs.io) for the command line and the release notes. Contributors start with [`AGENTS.md`](AGENTS.md) and [`STATUS.md`](STATUS.md).

## License

yavs is licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your
option. The IEEE `std` and `ieee` sources in `lib/` are Apache-2.0 (`lib/LICENSE`,
`lib/SOURCES.md`).
