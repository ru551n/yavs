# Status

## Milestones

| Milestone | State |
|---|---|
| M0: workspace, lexer and parser | done: `rsim parse`, corpora below |
| M1: semantic analysis | design in `docs/design/sem.md` |
| M2 to M10 | not started (crate spikes below) |

## syntax (`crates/rsim-syntax`, M0)

Hand-written lexer, conditional analysis on the token stream and recursive-descent parser for
VHDL-2008 and VHDL-2019 into an owned AST (`rsim parse [--std 2008|2019] files...`). Diagnostics
go through `rsim-diag` (human, short and JSON formats; snapshots in `tests/diag/`).

Parse corpus: `tools/fetch-corpus.sh` fetches the corpora (default `~/.cache/rsim-corpus`), then
`RSIM_CORPUS=~/.cache/rsim-corpus cargo test -p rsim --features corpus` compares them with the
expected lists in `tests/parse-corpus/` and fails if `RSIM_CORPUS` is unset (`RSIM_BLESS=1`
updates the lists). Plain `cargo test --workspace` still parses `lib/` and runs the snapshots.

| Corpus | Files | Pass | Expected failures |
|---|---:|---:|---|
| `lib/` std and ieee, 2008 and 2019 (in repo) | 55 | 55 | none |
| VUnit `vunit/vhdl` | 194 | 194 | none |
| OSVVM | 36 | 36 | none |
| rust_hdl `vhdl_libraries` (std, ieee2008, synopsys, vital2000) | 37 | 37 | none |
| NVC `test/regress` + `test/parse` | 1401 | 1360 | 41: tests of invalid syntax (most of them), PSL, VHDL-87 syntax, `abs x ** 2`, one `` `if `` that needs `--define` |
| VESTs `vhdl-93` | 3782 | 3244 | 538: 533 in `non_compliant/analyzer_failure`, 2 other non-compliant, 2 broken book excerpts, 1 VHDL-93 file using `default` |
| GHDL `testsuite/gna` | 3216 | 2973 | 243: `analyze_failure` tests and fuzz crashers, PSL, 93 code using 2008 reserved words |

Throughput on the VUnit corpus (read, decode, lex, preprocess and parse; release build, one
thread, Intel Core Ultra 9 275HX, warm cache): about 5,600 files/s and 60 MB/s (best of three: 5,769 files/s, 62.4 MB/s).

Fuzzing: `cargo +nightly fuzz run parse` (lexer, directives, parser and rendering in both
standards); two 5-minute runs with 8 workers (210k executions in total) found no crashes or hangs.

## logic (`crates/rsim-logic`)

Done ahead of M3: `StdULogic` with the IEEE 1164 tables (checked against the vendored package
body), `Logic<R>`/`LogicVec` packed vectors in two layouts (nibbles, bit-planes), `BitVec` fast
path, numeric_std add/sub/compare/shift/rotate/integer conversion with X propagation, exhaustive
and property tests, criterion benchmarks. Default layout is bit-planes; see
`crates/rsim-logic/LAYOUT.md`. Open: the user confirms the layout decision (docs/decisions.md).

## llvm (`crates/rsim-llvm`)

LLVM risk spike ahead of M6, behind the `llvm` feature (`cargo test -p rsim-llvm --features llvm`).
Done: llvm-sys 211 bindings (docs/decisions.md), ORC LLJIT with a portable CPU, O2 through
`LLVMRunPasses`, Rust runtime callbacks as absolute symbols, a checked i64 sum that traps into
Rust on overflow, a process body reading and writing a `#[repr(C)]` struct, a 10k-instruction
function checked against its Rust reference semantics, an unresolved runtime symbol reported by
name, and background compilation on one thread with an atomic function-pointer swap while the
calling thread keeps running in the "interpreter". Windows: `crates/rsim-llvm/WINDOWS.md`
(researched, not yet run on Windows).

JIT cost of one generated function (`examples/measure.rs`: `state[w] := state[a] + (state[b] and
255)` with an overflow check, 13 IR instructions and 2 basic blocks per statement). One process
per row, release build, Intel Core Ultra 9 275HX, LLVM 21.1.8, x86-64-v2. Time is IR build +
passes + codegen and link (first lookup); memory is peak RSS growth over a process that already
created the JIT.

| IR instructions | opt | passes (ms) | codegen + link (ms) | total (ms) | peak RSS growth (MiB) |
|---:|:--:|---:|---:|---:|---:|
| 1,002 | O2 | 6 | 12 | 18 | 12 |
| 10,011 | O2 | 50 | 95 | 146 | 28 |
| 20,021 | O2 | 103 | 225 | 330 | 46 |
| 40,041 | O2 | 206 | 589 | 799 | 79 |
| 100,101 | O2 | 630 | 3,100 | 3,800 | 231 |
| 299,001 | O2 | 3,761 | 24,489 | 28,280 | 650 |
| 10,011 | O1 | 30 | 92 | 123 | 25 |
| 100,101 | O1 | 281 | 3,009 | 3,301 | 199 |
| 1,002 | O0 | 0 | 3 | 3 | 8 |
| 10,011 | O0 | 0 | 21 | 22 | 15 |
| 100,101 | O0 | 2 | 181 | 193 | 92 |
| 299,001 | O0 | 6 | 637 | 674 | 264 |

Codegen at O1/O2 grows faster than linearly in the size of one function (10x the instructions
costs 25x to 30x the time); O0 stays close to linear. Proposed threshold: O2 up to 40k
instructions per function, larger units stay interpreted (docs/decisions.md).
## lib (`crates/rsim-lib`)

Done: library directories (`rsim-lib.toml` manifest + `_RSIM_LIB` lock, rule 14), `NAME:PATH`
mapping, atomic manifest updates under an exclusive lock, shared-lock reads that never write,
on-demand closure query (unknown library/unit with suggestion, cycles, stale hashes), multi-process
concurrency test. The standard is stored per file (manifest format 2). The analyzer supplies `UnitRecord`s (kind, name, dependencies) per file.

## kernel (`crates/rsim-kernel`)

LRM 14.7 cycle against an abstract `Process` trait, tested with hand-written processes: time with
runtime resolution and checked arithmetic, drivers with transport and inertial (reject limit)
preemption per element, resolution per driver segment, `'event`/`'active`/`'last_event`/
`'last_value`/`'last_active`/`'transaction`, force/release/deposit, postponed processes, delta
limit, parallel evaluation with ordered commit (threads and shuffle invariance tested on 1000
processes), `SimControl` with run/step, interrupt, value-change bus and phase callbacks. Design in
`crates/rsim-kernel/DESIGN.md`. Not yet: null transactions/guarded signals, ports, `'stable`,
`'quiet`, `'delayed`.

Benchmark (`cargo run --release -p rsim-kernel --example bench -- 10000 <work>`): 10000 counters
on one clock, 10 us simulated (1000 rising edges), 24-core host, 8 worker threads.

| work per activation | threads | events/s | process runs/s |
|---|---|---|---|
| 0 | 1 | 2.95 M | 5.9 M |
| 0 | 8 | 3.12 M | 6.3 M |
| 1000 | 1 | 1.41 M | 2.8 M |
| 1000 | 8 | 2.51 M | 5.0 M |

With trivial processes the serial update and commit phases dominate, so threads gain little.

## ffi (`crates/rsim-ffi`)

Done: VHPIDIRECT call trampolines without libffi (SysV x86_64, Win64, AAPCS64), argument
classification in Rust, naked asm stubs, up to 64 arguments, and the VHDL to C mapping in NVC's
convention (`vhdl::lower`). Tests: unit tests, hand-written ABI tests and 431 generated callees
compared against direct calls. All pass on Linux x86_64 natively, on aarch64 under qemu-user and
on Windows under wine. The cross commands (configured through the environment, no cargo config):

```sh
# aarch64 Linux (qemu-user, aarch64-linux-gnu-gcc, rustup target aarch64-unknown-linux-gnu)
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER="qemu-aarch64 -L /usr/aarch64-linux-gnu" \
  cargo test -p rsim-ffi --target aarch64-unknown-linux-gnu
# Windows x86_64 (mingw-w64, wine, rustup target x86_64-pc-windows-gnu)
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=wine WINEDEBUG=-all \
  cargo test -p rsim-ffi --target x86_64-pc-windows-gnu
```

Open: marshalling VHDL values (bit-planes to bytes, records, copy-back) belongs to the interpreter.
