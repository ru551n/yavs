# yavs

[![Documentation](https://img.shields.io/readthedocs/yavs?label=documentation)](https://yavs.readthedocs.io)
[![Repository](https://img.shields.io/badge/github-ru551n%2Fyavs-blue?logo=github)](https://github.com/ru551n/yavs)
[![Release](https://img.shields.io/github/v/release/ru551n/yavs?include_prereleases)](https://github.com/ru551n/yavs/releases)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)
[![CI status](https://github.com/ru551n/yavs/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ru551n/yavs/actions/workflows/ci.yml)

Yet Another VHDL Simulator: VHDL-1993, 2008 and 2019 in Rust.

## Features

- **Languages:** VHDL-1993, 2008 and 2019 files mix in one project; the IEEE libraries (including the VITAL and Synopsys packages) are built in. A PSL subset (simple sequences, `assert`/`cover`, `always`, `never`, `next`, `until`, `before`, `abort`) runs as checker processes; the rest of PSL is skipped with a warning.
- **Three execution tiers:** an interpreter, LLVM and Cranelift. `--tier=auto` starts in the interpreter and compiles hot functions on a background thread, guided by a profile of the previous run; compiled code is cached on disk under a key made before code generation, so warm runs reuse it. The release archives come with LLVM or, as `-nollvm`, with Cranelift instead (see Installation); both run `--tier=auto` by default. The output is identical on every tier.
- **Fast:** on 41 real testbenches against an established open-source VHDL simulator, the geometric mean of the full run time (analysis included, `--tier=auto`) is 0.22 of the reference simulator's with a warm cache and 0.59 with a cold one; faster on 36 of 41 warm. A few designs are still slower; see the
  [head-to-head 7 section](docs/STATUS.md#real-world-performance-suite-yavs-against-sim1-head-to-head-7-2026-10-27-toolsrealbench) of `docs/STATUS.md` for method and every cell.
- **Parallel kernel:** `--threads N` evaluates processes in parallel; output is identical at any thread count.
- **Waveforms:** FST (`--wave`) with records and arrays as scopes, include and exclude globs, a size cap and optional process variables (`--wave-variables`).
- **Coverage:** statement, branch, condition, expression and toggle coverage (`--cov`), per-test attribution and ranking, database merge, text, HTML and Cobertura reports, exclusions by pragma, file or YAML.
- **Debugging:** a Debug Adapter Protocol server (`--dap`), the graphical debugger `yavs-gui`, diagnostics with source snippets and VHDL call stacks, `yavs --explain CODE` and JSON diagnostics (`--message-format=json`).
- **VHPI:** `--vhpi` loads VHPI plugins that walk the design, read and put values and register callbacks; the implementation is partial (trace comparison in [`docs/conformance.md`](docs/conformance.md)). Running cocotb over VHPI works for most of its own tests (66 of 68), but that integration is on hold.
- **Verification libraries:** OSVVM and UVVM testbenches run unmodified; VUnit drives yavs as a simulator through a yavs simulator interface (`VUNIT_SIMULATOR=yavs`; the contract is [`docs/cli-contract.md`](docs/cli-contract.md)).
- **Safe core:** written in Rust; the kernel, interpreter, parser and semantic analysis forbid `unsafe` code. It is confined to the compiled tiers, the foreign-call and VHPI layers, the cache's file mapping and one prefetch hint.
- **Platforms and licence:** Linux x86_64, Linux aarch64 and Windows x86_64; MIT OR Apache-2.0.

yavs is early: expect gaps and please [report](https://github.com/ru551n/yavs/issues) what breaks.

## Correctness evidence

yavs is checked against recorded output of two established open-source VHDL simulators and against the test suites of widely used VHDL libraries. The full report, with per-suite tables, the known differences from the language reference and the simulators, the platforms covered by CI and the known limits, is in [`docs/conformance.md`](docs/conformance.md) (machine-readable: [`docs/conformance.json`](docs/conformance.json)). Headline numbers:

- **539 of 539** differential testbenches give the recorded reference output with the interpreter, at 1 thread, 4 threads and 4 shuffled threads (output identical at every thread count); the compiled tiers give the same output (checked locally).
- **6675 of 6682** files of 33 third-party VHDL corpora (neorv32, GRLIB, OSVVM, UVVM, VUnit, hdl-modules, Open Logic, PoC and others) analyse; the 7 that do not are listed and tracked.
- **314 of 314** OSVVM regression tests (VHDL-2008 and VHDL-2019), **131 of 137** UVVM testbench runs (the other 6 need vendor libraries), **1357 of 1357** VUnit tests.
- **1458** self-checking tests from the reference simulators' own regression suites pass; each suite's pass list is gated in CI.
- **209** documented cases where simulators disagree with each other or with the language reference, each with a reproducer and a recorded decision.
- neorv32 (three configurations) and GRLIB LEON3 run to completion. The waveforms of 17 designs, among them neorv32 and GRLIB LEON3, were compared signal by signal and time step by time step with the reference simulators: 44824 of 44889 signals equal over 50 million value changes; the 65 that differ are classified as differences between the reference simulators themselves (details in the report).

Parts of the report are regenerated by CI on every push to `main` and on release tags; the parts that need installed reference simulators carry the date and commit of the local run. To reproduce:

```sh
tools/fetch-corpus.sh
cargo build --release -p yavs
tools/conformance/run.py --yavs target/release/yavs
```

## Installation

Download the archive for your platform (Linux x86_64, Linux aarch64, Windows x86_64) from the
[releases](https://github.com/ru551n/yavs/releases) page, unpack it and put the directory on
`PATH`. aarch64 is also known as arm64; run `uname -m` to see which one you need. The Linux
archives require glibc 2.17 or newer (RHEL/CentOS 7+, Debian 8+, Ubuntu 14.04+); on musl-based
systems such as Alpine, install the `gcompat` package.

Each platform has two archives. Take `yavs-<version>-<target>` (compiled tier LLVM 21, the
default) unless you need the smallest download: `yavs-<version>-<target>-nollvm` compiles with
Cranelift instead, has no LLVM inside and runs `--tier=auto` by default too; `--tier=llvm` is an
error there. `yavs --version` tells which one you have. Or build from source with Rust stable and a C compiler:

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
kept, and are also shipped unmodified (`lib/LICENSE-SYNOPSYS`). The VHDL-1993 `numeric_std` and
`numeric_bit` are derived from the Apache-2.0 sources (`lib/SOURCES.md`).

The graphical debugger `yavs-gui` (`gui/`) is a separate program under the EUPL-1.2
(`gui/LICENSE`), because the waveform viewer library it embeds (libsurfer) is. It talks to yavs
over the Debug Adapter Protocol and links no yavs code. In the release archives its licence is
`LICENSE-EUPL-1.2-yavs-gui`, which applies to the `yavs-gui` binary only.

The archives with the compiled tier contain LLVM (`lib/libLLVM.so.21.1` on Linux, linked into
`yavs.exe` on Windows), under the Apache License v2.0 with LLVM Exceptions
(`LICENSE-LLVM`).
