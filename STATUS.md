# Status

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
concurrency test. The analyzer supplies `UnitRecord`s (kind, name, dependencies) per file.
