# Status

## Milestones

| Milestone | State |
|---|---|
| M0: workspace, lexer and parser | done: `rsim parse`, corpora below |
| M1: semantic analysis | in progress: slices S1 to S3 mostly in, `docs/design/sem.md` |
| M2: IR, interpreter, elaboration, kernel | `rsim-ir` and `rsim-interp` done against hand-written IR (below); elaboration without processes done (below); lowering in progress |
| M3 to M10 | not started (crate spikes below) |

## sem (`crates/rsim-sem`, M1)

- Negative suite `tests/diag/sem` run by `tests/diag_sem.rs` (exact code, line and column; optional
  rendered `.stderr` snapshots); what still fails is in `tests/diag_sem_xfail.txt`.
- `tests/stdlib.rs`: std, ieee packages, and the bodies of math_real, math_complex, numeric_std,
  numeric_bit and numeric_*_unsigned analyze clean (2008 and 2019). std_logic_1164's body stops on
  `.all`. `numeric_std_body_rule_1` is the rule 1 review: every object of the numeric_std body
  has its bounds from the one subtype representation.
- Declarations: objects, subprograms (operator arity `E1303`, function parameter modes, missing
  bodies `E1230`), components, generic and port clauses (components, blocks), attribute
  declarations and specifications, aliases, protected types.
- Types (S3): scalars, arrays, records, element/record/open constraints merged once, resolution
  indications, file element checks; the rule 1 representation is pinned on numeric_std
  (`numeric_std_subtypes_rule_1`, `numeric_std_body_rule_1`); the user's review is pending.
- Generics (S4, `generic.rs`): generic packages, subprograms and (2019) protected types; 2008
  `type T` and the 2019 classes; formal subprograms (`is <>`, `is name`) and formal packages
  (`(<>)`, `(default)`, generic map, checked against the actual instance); package and subprogram
  instances, also inside generic units, by HIR cloning (visible part eager, bodies lazy via
  `Db::instance_body`); `W1501`, `W1506`. The fixed/float packages' declarations and instances
  analyze clean; their bodies wait for `E1325` and `.all`/allocators. Open: anonymous generic
  types, mode views in generic units, instantiating a generic protected type (parser request).
- Static folding: checked i64, values checked against the target subtype (`E1403`) and the base
  range, division/mod by zero and negative exponents, static length mismatch (`E1405`).

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

## sem design units (`crates/rsim-sem`, M1 slice S5)

Entities, architectures (`of e` and `of lib.e`), components, instances (component, entity,
configuration; `l : c;`), generic and port maps (positional, named, sub-element, conversions,
`open`), configuration specifications and declarations (block, generate index, range and
alternative, component configurations with incremental binding), default binding
(`Db::default_binding`, `W1703`), manifest dependencies on named architectures and default-bound
entities. Negative tests in `tests/diag/sem/unit/`; unit tests in `src/unit/tests.rs`. Generic
types, subprograms and packages on components and blocks go through `generic.rs`. Open: mode
views (S7),
subtype indications as generate specifications.

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
preemption per element, resolution per source segment, `'event`/`'active`/`'last_event`/
`'last_value`/`'last_active`, force/release/deposit, postponed processes, delta limit, parallel
evaluation with ordered commit (threads and shuffle invariance tested on 1000+ processes),
`SimControl` with run/step, interrupt, value-change bus and phase callbacks. Design in
`crates/rsim-kernel/DESIGN.md`.

Kernel asks of docs/design/ir.md, all done: K1 uncollapsed ports with conversions in the update
phase (no extra delta), K2 kernel-maintained `'stable`/`'quiet`/`'delayed`/`'transaction`, K3
`'driving_value`, K4 `'driving`, null transactions and guarded `bus`/`register` signals with
disconnection time, K5 null-range signals, K6 buffered user-file streams, K7 bit-plane storage
for logic signals (bytes for everything else).

Benchmark (`cargo run --release -p rsim-kernel --example bench -- 10000 <work>`), 24-core host
under load from other agents (runs vary by about 15%), 8 worker threads. "Before" is the byte-only
kernel of the first round, run back to back with "after".

| case | threads | before | after |
|---|---|---|---|
| 10000 counters, work 0 | 1 | 3.3 s (3.0 M events/s) | 3.6 to 3.8 s (2.7 M events/s) |
| 10000 counters, work 0 | 8 | 3.1 to 3.3 s | 3.2 to 3.8 s |
| 10000 counters, work 1000 | 1 | 7.1 s (1.41 M events/s) | 7.8 s (1.29 M events/s) |
| 10000 counters, work 1000 | 8 | 4.3 s (2.35 M events/s) | 4.7 s (2.14 M events/s) |
| 1024-bit std_logic bus, 4 drivers, 10000 edges | 1 | 0.36 s (28 M resolved elements/s) | 0.115 s (89 M/s) |
| 1024-bit std_logic bus, 4 drivers, 10000 edges | 8 | 0.82 to 0.87 s | 0.33 to 0.39 s |

Before, the bus was 1 byte per element with a per-element resolution callback; after, it is
bit-planes with plane-wise resolution (3x faster). Scalar byte signals pay 10 to 15% for the
storage enum and the port/implicit machinery. With trivial processes the serial update and
commit phases dominate, so threads gain little.

## ir and interp (`crates/rsim-ir`, `crates/rsim-interp`, M2)

`rsim-ir`: Program/Func/Block/Inst with 32-bit ids and a `StmtId` per instruction, runtime type
descriptions with descriptor-based layouts (bit-plane logic arrays, records, arrays with dope),
compact constants, natives by NATIVE.md symbol, foreign references, flag propagation
(`MAY_SUSPEND`, `TOUCHES_SHARED`), resume numbering, a linear verifier and a text dump
(`crates/rsim-ir/README.md` says what is stable). `rsim-interp`: `InterpProcess` implements the
kernel `Process` (heap frame stack, waits inside procedures), checked arithmetic through
rsim-std, checks, signal ops mapped 1:1 to `Ctx`, natives (`now`, `stop`, `finish`, scalar
`to_string`/`'image`), VHPIDIRECT calls of scalars, generational heaps with precise-root GC,
serial marking, kernel-free `call` for elaboration, runtime errors with the VHDL stack as
`SimError::trace`, and Riviera/Questa-style debug hooks (docs/design/ir.md 3.4).

Tests (`cargo test -p rsim-ir -p rsim-interp`): 11 end-to-end designs run through the kernel at
1 and 4 threads with identical traces (clock and counter, delta ordering, wait forms with a
wait inside a procedure, overflow with stack trace, bit-plane ops across blocks, access types
and stale handle, GC roots, natives, foreign call, elaboration call, serial marking), 5 debug
tests (conditional line breakpoint with hit count, step over, step in with frame and locals,
global step across processes, signal breakpoint and process table), verifier and layout tests.
Not done: `Elab*` instructions (rsim-elab), records with runtime layouts beyond `LayoutOf`,
protected types beyond `CallProt` dispatch, files and textio natives, foreign arrays.

## elab (`crates/rsim-elab`, M2)

Done (without processes, which need rsim-lower and rsim-interp): `find_top` (`ENTITY`,
`ENTITY.ARCH`, `CONFIGURATION`), `elaborate` as a pure function (restart test: two elaborations
and two kernel builds give identical results), hierarchy with LRM `'path_name`/`'instance_name`
in both LRM and original spelling, unlabeled processes `_pN`, generics (actuals, `-g` overrides
through the rsim-std `'value` parser, defaults, range check), constants on demand, for/if/case
generates, blocks, entity/component/configuration instances, binding (configuration declaration,
then specification, then default binding), signals split into kernel leaves (records as struct
of arrays, `std_ulogic`/`bit`/`boolean` as bit-planes), the sub-element map
(`NetGraph::select`), uncollapsed ports with unconstrained formals taking the actual's bounds,
static expression actuals, per-process merged drivers, multiple-source check across the
hierarchy, LRM 14.4.2 body-before-call check, `Elaborated::build_with` into the kernel.
Elaboration-time evaluation folds globally static expressions with `Db::fold_with`; function
calls and aggregates wait for rsim-interp (E2006). Not yet: conversion functions and
non-static expression actuals in port maps, implicit signals, guarded blocks and signals,
package signals, external names, user resolution functions, generic types and subprograms.

CLI: `rsim elab` and `rsim run` per docs/cli-contract.md (closure from the manifests, design
revision = newest standard in the closure, `-g` after TOP, `@file`, exit codes). `run` simulates
only designs without processes until processes are lowered; otherwise it reports E9999.
Corpus `hierarchy/` and `generics/`: 5 of 9 elaborate (`--print-hierarchy`); the other 4 stop in
analysis.

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
