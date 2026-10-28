# yavs

[![Documentation](https://img.shields.io/readthedocs/yavs?label=documentation)](https://yavs.readthedocs.io)
[![Repository](https://img.shields.io/badge/github-ru551n%2Fyavs-blue?logo=github)](https://github.com/ru551n/yavs)
[![Release](https://img.shields.io/github/v/release/ru551n/yavs?include_prereleases)](https://github.com/ru551n/yavs/releases)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)
[![CI status](https://github.com/ru551n/yavs/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ru551n/yavs/actions/workflows/ci.yml)

Yet Another VHDL Simulator: VHDL-1993, 2008 and 2019 in Rust.

## Features

- **Fast:** on 41 real testbenches the geometric mean of the full run time (analysis included, `--tier=auto`) is 0.22 of an established open-source VHDL simulator's with a warm cache and 0.59 with a cold one; see [`docs/performance.md`](docs/performance.md).
- **Languages:** VHDL-1993, 2008 and 2019 files mix in one project; the IEEE libraries (including the VITAL and Synopsys packages) are built in. A subset of PSL is supported. What is supported and what is not is listed in [`docs/language-support.md`](docs/language-support.md). Plain Verilog (not SystemVerilog), for vendor IP and vendor libraries, is [planned](https://github.com/ru551n/yavs/issues/776).
- **Verification libraries:** compliant with OSVVM, UVVM and VUnit: their testbenches and test suites run unmodified.
- **Code generation:** yavs generates native code with [Cranelift](https://cranelift.dev/) instead of LLVM, to avoid external dependencies. `--tier=auto` chooses per function, and compiled code is cached on disk. The output is identical on every tier.
- **Coverage:** statement, branch, condition, expression and toggle coverage with per-test attribution and ranking, database merge, and text, HTML and Cobertura reports; see [`docs/coverage.md`](docs/coverage.md).
- **Waveforms:** FST (`--wave`) with records and arrays as scopes, include and exclude globs, a size cap and optional process variables (`--wave-variables`).
- **Debugging:** `yavs run --dap=PORT` lets you debug a simulation from VS Code (extension in `editors/vscode`) or any Debug Adapter Protocol editor: breakpoints on VHDL lines (with VHDL conditions and hit counts) and on signal changes, stepping through the statements of each process, pausing a running simulation, and inspecting or changing signal and variable values. See [`docs/usage.rst`](docs/usage.rst).
- **VHPI:** `--vhpi` loads VHPI plugins; the implementation is partial. Work on full cocotb support is scheduled ([M5 VHPI epic #143](https://github.com/ru551n/yavs/issues/143); cocotb's own regression is #122). What is supported is listed in [`docs/vhpi.md`](docs/vhpi.md).
- **Parallel kernel:** `--threads N` evaluates processes in parallel with identical output at any thread count; how it works and where performance stands is in [`docs/parallel.md`](docs/parallel.md).
- **Safe core:** written in Rust; the kernel, interpreter, parser and semantic analysis forbid `unsafe` code. It is confined to the compiled tiers, the foreign-call and VHPI layers, the cache's file mapping and one prefetch hint.
- **Platforms and licence:** Linux x86_64, Linux aarch64 and Windows x86_64; MIT OR Apache-2.0.

yavs is early: expect gaps and please [report](https://github.com/ru551n/yavs/issues) what breaks.

## Correctness and performance

- Conformance: [`docs/conformance.md`](docs/conformance.md)
- Performance: [`docs/performance.md`](docs/performance.md)

## Installation

Download the archive for your platform from the [releases](https://github.com/ru551n/yavs/releases) page, unpack it and put the directory on `PATH`. There is one archive per platform: `yavs-<version>-x86_64-unknown-linux-gnu.tar.gz`, `yavs-<version>-aarch64-unknown-linux-gnu.tar.gz` and `yavs-<version>-x86_64-pc-windows-msvc.zip`. See [`docs/install.rst`](docs/install.rst) for checks and building from source.

## License

yavs is licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your
option. The IEEE `std` and `ieee` sources in `lib/` are Apache-2.0 (`lib/LICENSE`,
`lib/SOURCES.md`). The built-in IEEE 1076.4 VITAL packages (`ieee.vital_timing`,
`ieee.vital_primitives`, `ieee.vital_memory`) are IEEE-copyrighted and carry no explicit licence
grant; they are redistributed unmodified, as other open-source VHDL simulators do (`lib/LICENSE-IEEE-VITAL`). The
built-in Synopsys packages (`ieee.std_logic_arith`, `std_logic_unsigned`, `std_logic_signed`,
`std_logic_misc`) are Synopsys copyright, redistributable without restriction when the notice is
kept, and are also shipped unmodified (`lib/LICENSE-SYNOPSYS`). The VHDL-1993 `numeric_std` and
`numeric_bit` are derived from the Apache-2.0 sources (`lib/SOURCES.md`).

The graphical debugger `yavs-gui` (`gui/`) is a separate program under the EUPL-1.2
(`gui/LICENSE`), because the waveform viewer library it embeds (libsurfer) is. It talks to yavs
over the Debug Adapter Protocol and links no yavs code. In the release archives its licence is
`LICENSE-EUPL-1.2-yavs-gui`, which applies to the `yavs-gui` binary only.
