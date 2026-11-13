<h1 align="center">YAVS - Yet Another VHDL Simulator</h1>

[![Documentation](https://img.shields.io/readthedocs/yavs?label=documentation)](https://yavs.readthedocs.io)
[![Repository](https://img.shields.io/badge/github-ru551n%2Fyavs-blue?logo=github)](https://github.com/ru551n/yavs)
[![Release](https://img.shields.io/github/v/release/ru551n/yavs?include_prereleases)](https://github.com/ru551n/yavs/releases)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)
[![CI status](https://github.com/ru551n/yavs/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ru551n/yavs/actions/workflows/ci.yml)

yavs is a VHDL simulator written in Rust, for VHDL-1993, 2008 and 2019.

## What it does

- [VHDL support](https://yavs.readthedocs.io/en/latest/language-support.html): VHDL-1993, 2008 and 2019, mixed in one project, with the IEEE packages built in.
- [VS Code debugger](https://yavs.readthedocs.io/en/latest/debugging.html): available through the [DAP protocol](https://microsoft.github.io/debug-adapter-protocol/).
- [Interactive console](https://yavs.readthedocs.io/en/latest/interactive.html): breakpoints, stepping, `examine`, `force`, `drivers`, `trace x` and `restart` at a `yavs>` prompt or in a script.
- [Live waveforms](https://yavs.readthedocs.io/en/latest/waveforms.html): [The Surfer waveform viewer](https://surfer-project.org/) follows the run and the debugger's stops. FST files for any viewer.
- [Sane error messages, Rust style](https://yavs.readthedocs.io/en/latest/troubleshooting.html)
- [Coverage](https://yavs.readthedocs.io/en/latest/coverage.html): from statements to condition combinations, with HTML and Cobertura reports and a CI gate.
- [Static analysis](https://yavs.readthedocs.io/en/latest/lint.html): `yavs lint` reports what can be proved wrong before a simulation.
- [VHPI](https://yavs.readthedocs.io/en/latest/vhpi.html): plugins and C functions called from VHDL.
- [Platforms](https://yavs.readthedocs.io/en/latest/getting-started.html#installation): Linux x86_64, Linux aarch64 and Windows x86_64, one archive with nothing else to install.

## Quickstart

Install yavs, or update it by running the same line again (Linux x86_64 and aarch64):

```sh
curl -fsSL https://raw.githubusercontent.com/ru551n/yavs/main/install.sh | sh
```

On Windows x86_64, in PowerShell:

```powershell
powershell -c "irm https://raw.githubusercontent.com/ru551n/yavs/main/install.ps1 | iex"
```

Both install for your user only, without administrator rights, and check the download against the release's checksums. They are short; read [install.sh](install.sh) and [install.ps1](install.ps1) first if you like ([what they do](https://yavs.readthedocs.io/en/latest/getting-started.html#one-line-install)). Or [install by hand](https://yavs.readthedocs.io/en/latest/getting-started.html#installation). Then follow [Getting started](https://yavs.readthedocs.io/en/latest/getting-started.html#your-first-simulation).

### In GitHub Actions

```yaml
- uses: ru551n/yavs-action@v1
  with:
    version: 0.19.1
```

[yavs-action](https://github.com/ru551n/yavs-action) installs a release; [Continuous integration](https://yavs.readthedocs.io/en/latest/ci.html) has a whole job.

## Test frameworks

- [VUnit](https://yavs.readthedocs.io/en/latest/vunit.html): through the `vunit-yavs` package; it needs yavs 0.19.1 or later.
- [OSVVM](https://yavs.readthedocs.io/en/latest/libraries.html): analyzed from its sources, under VHDL-2008 and VHDL-2019.
- [UVVM](https://yavs.readthedocs.io/en/latest/libraries.html): analyzed from its sources; its testbenches that run pass, and a few are skipped (sources absent from the checkout, or over the time limit).

Results per suite are in the [conformance report](https://yavs.readthedocs.io/en/latest/conformance.html) of 2026-11-01.

## Documentation

- [User guide](https://yavs.readthedocs.io)
- [Command-line reference](https://yavs.readthedocs.io/en/latest/cli-reference.html)
- [Error codes](https://yavs.readthedocs.io/en/latest/error-codes.html)
- [Moving from another simulator](https://yavs.readthedocs.io/en/latest/migrating.html)
- [Performance](https://yavs.readthedocs.io/en/latest/performance.html)
- [Known limitations](https://yavs.readthedocs.io/en/latest/faq.html)
- [Release notes](https://yavs.readthedocs.io/en/latest/release_notes.html)
- [Context7](https://context7.com/ru551n/yavs), for coding assistants

yavs is a pre-release: expect gaps, and please [report](https://github.com/ru551n/yavs/issues) what breaks.

## Contributing

Pull requests and bug reports are welcome: see [`CONTRIBUTING.md`](CONTRIBUTING.md).

### Contributors

Thanks to everyone who has contributed so far:

- [malles030](https://github.com/malles030)
- [nipo](https://github.com/nipo)
- [Paebbels](https://github.com/Paebbels)

## License

yavs is licensed under either of [Apache-2.0](https://github.com/ru551n/yavs/blob/main/LICENSE-APACHE) or [MIT](https://github.com/ru551n/yavs/blob/main/LICENSE-MIT), at your option. The bundled IEEE, VITAL and Synopsys packages have their own terms: see [`lib/SOURCES.md`](https://github.com/ru551n/yavs/blob/main/lib/SOURCES.md), [`lib/LICENSE`](https://github.com/ru551n/yavs/blob/main/lib/LICENSE), [`lib/LICENSE-IEEE-VITAL`](https://github.com/ru551n/yavs/blob/main/lib/LICENSE-IEEE-VITAL) and [`lib/LICENSE-SYNOPSYS`](https://github.com/ru551n/yavs/blob/main/lib/LICENSE-SYNOPSYS). The Windows executable has the mimalloc memory allocator built in, MIT licensed: [`LICENSE-MIMALLOC`](https://github.com/ru551n/yavs/blob/main/LICENSE-MIMALLOC).
