# yavs

[![Documentation](https://img.shields.io/readthedocs/yavs?label=documentation)](https://yavs.readthedocs.io)
[![Repository](https://img.shields.io/badge/github-ru551n%2Fyavs-blue?logo=github)](https://github.com/ru551n/yavs)
[![Release](https://img.shields.io/github/v/release/ru551n/yavs?include_prereleases)](https://github.com/ru551n/yavs/releases)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)
[![CI status](https://github.com/ru551n/yavs/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ru551n/yavs/actions/workflows/ci.yml)

Yet Another VHDL Simulator: VHDL-1993, 2008 and 2019 in Rust.

## Why yavs

- **Measured speed:** about 5 times faster than an established open-source VHDL simulator on repeated runs and 1.7 times on first runs, over 41 real testbenches with analysis included ([performance](docs/performance.md)).
- **Multi-threaded:** `--threads 4` runs large, flat RTL designs, where thousands of processes wake on the same clock edge, 2.4 to 2.6 times faster than one thread (whole run, measured 2026-10-28); processor models and testbench-style designs run at about their one-thread speed, with identical output at any thread count ([parallel simulation](docs/parallel.md)).
- **Evidence, not claims:** on every push to `main`, CI re-runs most of the [conformance report](docs/conformance.md), including a comparison of yavs's output with an established simulator's recorded output and two simulators' own regression suites run on yavs; every result says where and when it was produced, and the performance sheet gives its method.
- **Runs your verification libraries unmodified:** OSVVM's, UVVM's and VUnit's own test suites and Open Logic's testbenches run as published (results below).
- **Built for CI:** browsable HTML coverage reports with per-line hit counts and per-test attribution plus Cobertura for CI dashboards, identical output at any thread count, a compile cache for repeated runs, and one release archive per platform with nothing else to install.
- **Readable error messages:** Rust-style diagnostics that point at the exact code, with a stable error code; `yavs --explain E1100` explains any code:

  ```
  error[E1100]: `cnt` is not declared
    --> counter.vhd:15:16
     |
  10 |   signal count : unsigned(3 downto 0);
     |          ----- similar name declared here
  ...
  15 |       count <= cnt + 1;
     |                ^^^ not found in this scope
     |
     = help: a signal with a similar name exists: `count`
  ```
- **Open source and auditable:** MIT OR Apache-2.0, written in Rust; the kernel, interpreter, parser and semantic analysis forbid `unsafe` code.

| At a glance | |
| --- | --- |
| Speed, repeated / first run | 0.21 / 0.57 of an established open-source VHDL simulator's time: about 5 and 1.7 times faster (geometric mean over real testbenches) |
| OSVVM, VHDL-2008 and VHDL-2019 | 314/314 tests each |
| VUnit | 1357/1357 tests |
| UVVM | 131/137 tests |
| Open Logic | 6867/6867 tests |
| All suites | 14825/15018 tests; every failure is a tracked issue |
| Platforms | Linux x86_64, Linux aarch64, Windows x86_64 |

Figures from the [performance](docs/performance.md) and [conformance](docs/conformance.md) sheets of 2026-10-28.

## Quickstart

Unpack a [release archive](https://github.com/ru551n/yavs/releases) and put its directory on
`PATH`, then analyze the files into a library (a directory) and run a top level:

```
yavs analyze --work=work:build/work hello_tb.vhd
yavs run --work=work:build/work hello_tb
```

The example files are in [`docs/examples/`](docs/examples); the output and a failing check are in
[Getting started](https://yavs.readthedocs.io/en/latest/getting-started.html). The [user guide](https://yavs.readthedocs.io) covers
[installation](https://yavs.readthedocs.io/en/latest/install.html), [libraries](https://yavs.readthedocs.io/en/latest/libraries.html),
[waveforms](https://yavs.readthedocs.io/en/latest/waveforms.html), [coverage](https://yavs.readthedocs.io/en/latest/coverage.html),
[debugging](https://yavs.readthedocs.io/en/latest/debugging.html), [VHPI](https://yavs.readthedocs.io/en/latest/vhpi.html),
[running faster](https://yavs.readthedocs.io/en/latest/running-faster.html) and the [command line](https://yavs.readthedocs.io/en/latest/cli.html).

The documentation is also available to coding assistants through [Context7](https://context7.com/ru551n/yavs).

## Features

- **Languages:** VHDL-1993 (`--std=93`), VHDL-2008 and VHDL-2019, mixed in one project with each file keeping its own standard; the IEEE libraries (including the VITAL and Synopsys packages) are built in; a subset of PSL. Details in [`docs/language-support.md`](docs/language-support.md). Plain Verilog (not SystemVerilog), for vendor IP and vendor libraries, is [planned](https://github.com/ru551n/yavs/issues/776).
- **Code generation:** with `--tier=auto` a run starts in the interpreter while a background thread compiles the hot functions with the bundled [Cranelift](https://cranelift.dev/) code generator and swaps them in; `--tier=clift` compiles everything before time 0. Compiled code is cached on disk, and every tier gives identical output. See [Running faster](https://yavs.readthedocs.io/en/latest/running-faster.html).
- **Coverage:** statement, branch, condition, expression and toggle coverage; databases of several runs merge, `yavs cov rank` finds a small set of tests that reaches the same coverage, and reports come as text, HTML or Cobertura. See [`docs/coverage.md`](docs/coverage.md).
- **Waveforms:** standard FST (`--wave`) with records and arrays as scopes, include and exclude globs, a size cap and optional process variables (`--wave-variables`).
- **Debugging:** `yavs run --dap=PORT` debugs a simulation from VS Code (extension in `editors/vscode`) or any Debug Adapter Protocol editor: breakpoints on VHDL lines (with VHDL conditions and hit conditions such as `>= 3`) and on signal changes, stepping through each process, pausing, and inspecting or changing signal and variable values. A debugger session runs the interpreter on one thread. See [Debugging](https://yavs.readthedocs.io/en/latest/debugging.html).
- **VHPI:** `--vhpi` loads VHPI plugins; the implementation is partial, see [`docs/vhpi.md`](docs/vhpi.md).
- **Parallel kernel:** `--threads N` (default 1) evaluates processes in parallel; it helps only large, flat designs. See [`docs/parallel.md`](docs/parallel.md).

yavs is early: expect gaps and please [report](https://github.com/ru551n/yavs/issues) what breaks.

## Installation

Download the archive for your platform from the [releases](https://github.com/ru551n/yavs/releases) page, unpack it and put the directory on `PATH`. Every platform has one archive, with the Cranelift compiled tier built in and nothing else to install. See [`docs/install.rst`](docs/install.rst) for checks, the build variants and building from source.

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
