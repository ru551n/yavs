# yavs

yavs ("Yet Another VHDL Simulator") is a VHDL-2008 and VHDL-2019 simulator written in Rust: its
own parser and semantic analysis, an IR interpreter, a parallel simulation kernel and
rustc-style diagnostics. VUnit support comes first; VHPI for cocotb, an LLVM tier, waveforms, a
DAP debug server and a GUI are planned.

yavs is early. What works today, with numbers, is in [`STATUS.md`](STATUS.md); the plan is
[`docs/plan.md`](docs/plan.md).

## Install

Download the archive for your platform from the
[releases](https://github.com/ru551n/yavs/releases) page (Linux x86_64, Linux aarch64, Windows
x86_64), check it against `SHA256SUMS`, unpack it and put the directory on `PATH`. The `std` and `ieee`
sources are built into the `yavs` executable; `YAVS_STDLIB` points yavs at another copy.

From source (Rust stable and a C compiler):

```sh
cargo build --release -p yavs   # target/release/yavs, std/ieee built in
```

## Use

```sh
yavs analyze --std=2008 --work=work:out/work tb.vhd
yavs run --work=work:out/work tb
```

The command line VUnit relies on is specified in [`docs/cli-contract.md`](docs/cli-contract.md).

## License

yavs is licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your
option. The IEEE `std` and `ieee` sources in `lib/` are Apache-2.0 (`lib/LICENSE`,
`lib/SOURCES.md`).
