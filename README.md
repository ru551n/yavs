# yavs

[![Documentation](https://img.shields.io/readthedocs/yavs?label=documentation)](https://yavs.readthedocs.io)
[![Repository](https://img.shields.io/badge/github-ru551n%2Fyavs-blue?logo=github)](https://github.com/ru551n/yavs)
[![Release](https://img.shields.io/github/v/release/ru551n/yavs?include_prereleases)](https://github.com/ru551n/yavs/releases)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)
[![CI status](https://github.com/ru551n/yavs/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ru551n/yavs/actions/workflows/ci.yml)

Yet Another VHDL Simulator: VHDL-1993, 2008 and 2019 in Rust.

## Quickstart

Unpack a [release archive](https://github.com/ru551n/yavs/releases) and put its directory on
`PATH`, then analyze the files into a library (a directory) and run a top level:

```
yavs analyze --work=work:build/work hello_tb.vhd
yavs run --work=work:build/work hello_tb
```

The example files are in [`docs/examples/`](docs/examples); the output and a failing check are in the
[quickstart](https://yavs.readthedocs.io/en/latest/usage.html#your-first-simulation); the
[installation](https://yavs.readthedocs.io/en/latest/install.html) page says which archive to take
and what the compiled tier is. The guides: [command line](https://yavs.readthedocs.io/en/latest/usage.html),
[recipes](https://yavs.readthedocs.io/en/latest/recipes.html) for test runners and libraries,
[coverage](https://yavs.readthedocs.io/en/latest/coverage.html),
[VHPI](https://yavs.readthedocs.io/en/latest/vhpi.html),
[parallel](https://yavs.readthedocs.io/en/latest/parallel.html) and
[performance](https://yavs.readthedocs.io/en/latest/performance.html).

The documentation is also available to coding assistants through [Context7](https://context7.com/ru551n/yavs).

## Features

- **Fast:** on 41 real testbenches the geometric mean of the full run time (analysis included, `--tier=auto`) is 0.22 of an established open-source VHDL simulator's with a warm cache and 0.59 with a cold one; see [`docs/performance.md`](docs/performance.md).
- **Languages:** VHDL-1993 (`--std=93`), VHDL-2008 and VHDL-2019, mixed in one project: each file keeps its own standard; the IEEE libraries (including the VITAL and Synopsys packages) are built in. A subset of PSL is supported. What is supported and what is not is listed in [`docs/language-support.md`](docs/language-support.md). Plain Verilog (not SystemVerilog), for vendor IP and vendor libraries, is [planned](https://github.com/ru551n/yavs/issues/776).
- **Verification libraries:** the testbenches and test suites of the established open-source VHDL verification libraries and test runners run unmodified; the suites that are run, and their results, are in [`docs/conformance.md`](docs/conformance.md).
- **Code generation:** a run starts in the interpreter and, with `--tier=auto`, a background thread compiles the functions that become hot and swaps them in while the simulation continues; `--tier=llvm` or `--tier=clift` compiles everything before time 0, and compiled code is cached on disk. The output is identical on every tier. A [Cranelift](https://cranelift.dev/) code generator is bundled inside the simulator binary (`-nollvm` archives); LLVM, which is linked as an external library (the default archives), is supported as well for now and is scheduled for removal, so that everything ships in one binary. A source build without either feature has no compiled tier and runs the interpreter. See "Build variants and tiers" in the [installation guide](https://yavs.readthedocs.io/en/latest/install.html).
- **Coverage:** statement, branch, condition, expression and toggle coverage with per-test attribution and ranking, database merge, and text, HTML and Cobertura reports; see [`docs/coverage.md`](docs/coverage.md).
- **Waveforms:** FST (`--wave`) with records and arrays as scopes, include and exclude globs, a size cap and optional process variables (`--wave-variables`).
- **Debugging:** `yavs run --dap=PORT` lets you debug a simulation from VS Code (extension in `editors/vscode`) or any Debug Adapter Protocol editor: breakpoints on VHDL lines (with VHDL conditions and hit conditions such as `3` or `>= 3`) and on signal changes, stepping through the statements of each process, pausing a running simulation, and inspecting or changing signal and variable values. `yavs run --gui` opens the run in the early graphical debugger `yavs-gui` (`gui/`). A debugger session always runs the interpreter and on one thread. See [`docs/usage.rst`](docs/usage.rst).
- **VHPI:** `--vhpi` loads VHPI plugins; the implementation is partial. Work on full support for Python testbench frameworks that use VHPI is scheduled ([M5 VHPI epic #143](https://github.com/ru551n/yavs/issues/143), [#122](https://github.com/ru551n/yavs/issues/122)). What is supported is listed in [`docs/vhpi.md`](docs/vhpi.md).
- **Parallel kernel:** `--threads N` (default 1) evaluates processes in parallel with identical output at any thread count; it only helps large, flat designs; how it works and where performance stands is in [`docs/parallel.md`](docs/parallel.md).
- **Safe core:** written in Rust; the kernel, interpreter, parser and semantic analysis forbid `unsafe` code. It is confined to the compiled tiers, the foreign-call and VHPI layers, the cache's file mapping and one prefetch hint.
- **Platforms and licence:** Linux x86_64, Linux aarch64 and Windows x86_64; MIT OR Apache-2.0.

yavs is early: expect gaps and please [report](https://github.com/ru551n/yavs/issues) what breaks.

## Correctness and performance

- Conformance: [`docs/conformance.md`](docs/conformance.md)
- Performance: [`docs/performance.md`](docs/performance.md)

## Installation

Download the archive for your platform from the [releases](https://github.com/ru551n/yavs/releases) page, unpack it and put the directory on `PATH`. Every platform has two archives: `yavs-<version>-x86_64-unknown-linux-gnu.tar.gz`, `yavs-<version>-aarch64-unknown-linux-gnu.tar.gz` and `yavs-<version>-x86_64-pc-windows-msvc.zip` have the LLVM compiled tier, and the same names with `-nollvm` before the extension have the Cranelift tier and no LLVM. See [`docs/install.rst`](docs/install.rst) for the difference, checks and building from source.

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

The archives with the LLVM compiled tier contain LLVM (`lib/libLLVM.so.21.1` on Linux, linked into
`yavs.exe` on Windows), under the Apache License v2.0 with LLVM Exceptions
(`LICENSE-LLVM`).
