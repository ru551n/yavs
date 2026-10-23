# yavs

[![Documentation](https://img.shields.io/readthedocs/yavs?label=documentation)](https://yavs.readthedocs.io)
[![Repository](https://img.shields.io/badge/github-ru551n%2Fyavs-blue?logo=github)](https://github.com/ru551n/yavs)
[![Release](https://img.shields.io/github/v/release/ru551n/yavs?include_prereleases)](https://github.com/ru551n/yavs/releases)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)
[![CI status](https://github.com/ru551n/yavs/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ru551n/yavs/actions/workflows/ci.yml)

Yet Another VHDL Simulator: VHDL-2008 and VHDL-2019 in Rust.

## Overview

- **Language:** VHDL-2008 and VHDL-2019 files mix in one project; the IEEE libraries are built in.
- **Execution:** an interpreter plus an LLVM-compiled tier; `--tier=auto` (the default) compiles
  the hot code during the run. Analysed units, elaboration and compiled code are cached between runs.
- **Kernel:** event-driven, with a parallel mode (`--threads N`).
- **Debugging:** FST waveforms (`--wave`), a Debug Adapter Protocol server (`--dap`) and the
  graphical debugger `yavs-gui`, diagnostics with source snippets and VHDL call stacks,
  `yavs --explain CODE` and JSON diagnostics (`--message-format=json`).
- **Integration:** VHPI (`--vhpi`) and code and toggle coverage (`--cov`).
- **Libraries:** testbenches built on OSVVM, UVVM and VUnit run; cocotb runs over VHPI.

Performance work is in progress. yavs is early: expect gaps and please
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

See the [documentation](https://yavs.readthedocs.io) for the command line and the release notes. Contributors start with [`AGENTS.md`](AGENTS.md) and [`docs/STATUS.md`](docs/STATUS.md).

## License

yavs is licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your
option. The IEEE `std` and `ieee` sources in `lib/` are Apache-2.0 (`lib/LICENSE`,
`lib/SOURCES.md`). The built-in IEEE 1076.4 VITAL packages (`ieee.vital_timing`,
`ieee.vital_primitives`, `ieee.vital_memory`) are IEEE-copyrighted and carry no explicit licence
grant; they are redistributed unmodified, as other open-source VHDL simulators do (`lib/LICENSE-IEEE-VITAL`). The
built-in Synopsys packages (`ieee.std_logic_arith`, `std_logic_unsigned`, `std_logic_signed`,
`std_logic_misc`) are Synopsys copyright, redistributable without restriction when the notice is
kept, and are also shipped unmodified (`lib/LICENSE-SYNOPSYS`).

The graphical debugger `yavs-gui` (`gui/`) is a separate program under the EUPL-1.2
(`gui/LICENSE`), because the waveform viewer library it embeds (libsurfer) is. It talks to yavs
over the Debug Adapter Protocol and links no yavs code. In the release archives its licence is
`LICENSE-EUPL-1.2-yavs-gui`, which applies to the `yavs-gui` binary only.
