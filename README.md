# yavs

[![Documentation](https://img.shields.io/readthedocs/yavs?label=documentation)](https://yavs.readthedocs.io)
[![Repository](https://img.shields.io/badge/github-ru551n%2Fyavs-blue?logo=github)](https://github.com/ru551n/yavs)
[![Release](https://img.shields.io/github/v/release/ru551n/yavs?include_prereleases)](https://github.com/ru551n/yavs/releases)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)
[![CI status](https://github.com/ru551n/yavs/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ru551n/yavs/actions/workflows/ci.yml)

Yet Another VHDL Simulator: VHDL-1993, 2008 and 2019 in Rust.

## Features

- **Languages:** VHDL-1993, 2008 and 2019 files mix in one project; the IEEE libraries (including the VITAL and Synopsys packages) are built in. A subset of PSL is supported. What is supported and what is not is listed in [`docs/language-support.md`](docs/language-support.md).
- **Code generation:** a Cranelift code generator is bundled inside the simulator binary; LLVM, which is linked as an external library, is supported as well for now and is scheduled for removal, so that everything ships in one binary. `--tier=auto` chooses per function, and compiled code is cached on disk. The output is identical on every tier.
- **Fast:** on 41 real testbenches the geometric mean of the full run time (analysis included, `--tier=auto`) is 0.22 of an established open-source VHDL simulator's with a warm cache and 0.59 with a cold one; see [`docs/performance.md`](docs/performance.md).
- **Parallel kernel:** `--threads N` evaluates processes in parallel; output is identical at any thread count.
- **Waveforms:** FST (`--wave`) with records and arrays as scopes, include and exclude globs, a size cap and optional process variables (`--wave-variables`).
- **Coverage:** statement, branch, condition, expression and toggle coverage (`--cov`), per-test attribution and ranking, database merge, text, HTML and Cobertura reports, exclusions by pragma, file or YAML.
- **Debugging:** `yavs run --dap=PORT` lets you debug a simulation from VS Code (extension in `editors/vscode`) or any Debug Adapter Protocol editor: breakpoints on VHDL lines (with VHDL conditions and hit counts) and on signal changes, stepping through the statements of each process, pausing a running simulation, and inspecting or changing signal and variable values. See [`docs/usage.rst`](docs/usage.rst).
- **VHPI:** `--vhpi` loads VHPI plugins that walk the design, read and put values and register callbacks; the implementation is partial (trace comparison in [`docs/conformance.md`](docs/conformance.md)). Running cocotb over VHPI works for most of its own tests (66 of 68), but that integration is on hold.
- **Verification libraries:** compliant with OSVVM, UVVM and VUnit: their testbenches and test suites run unmodified.
- **Safe core:** written in Rust; the kernel, interpreter, parser and semantic analysis forbid `unsafe` code. It is confined to the compiled tiers, the foreign-call and VHPI layers, the cache's file mapping and one prefetch hint.
- **Platforms and licence:** Linux x86_64, Linux aarch64 and Windows x86_64; MIT OR Apache-2.0.

yavs is early: expect gaps and please [report](https://github.com/ru551n/yavs/issues) what breaks.

## Correctness and performance

yavs is checked against recorded output of established open-source VHDL simulators and against the test suites of widely used VHDL libraries and real designs.

**Results per suite, known differences and limits: [`docs/conformance.md`](docs/conformance.md)** (machine-readable: [`docs/conformance.json`](docs/conformance.json); regenerate with `tools/conformance/run.py`).
**Speed against an established open-source VHDL simulator, with method and every testbench: [`docs/performance.md`](docs/performance.md).**

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

The archives with the compiled tier contain LLVM (`lib/libLLVM.so.21.1` on Linux, linked into
`yavs.exe` on Windows), under the Apache License v2.0 with LLVM Exceptions
(`LICENSE-LLVM`).
