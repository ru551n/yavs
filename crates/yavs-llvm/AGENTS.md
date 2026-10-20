# AGENTS.md: yavs-llvm

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-llvm`.
Binding choices are in `docs/decisions.md` "LLVM bindings: llvm-sys 211, no inkwell"; Windows
in `crates/yavs-llvm/WINDOWS.md`; measurements in `STATUS.md` "llvm".

## 1. Purpose and boundaries

- The **LLVM tier** (M6): `compile(&Program, OptLevel, max_size) -> LlvmTier` lowers every
  non-elaboration IR function to LLVM 21 and JITs it through ORC LLJIT (portable CPU, O2, Rust
  runtime callbacks as absolute symbols). `LlvmTier` implements `yavs_interp::Tier`; `yavs run
  --tier=llvm` sets it on the runtime. Design: `docs/decisions.md` "LLVM tier: side exits to the
  interpreter".
- Compiled code runs on the interpreter frame's registers, slots and temporary area
  (`yavs_interp::TierFrame`). Inline: `Const`, `ConstF`, `Move`, `Bin`/`BinRange` (not `**`),
  `Un`, `FBin` (not `**`), `FUn`, `IToF`, `Cmp`, `FCmp`, `Bool`, `Check`, `FCheck`, `Index`,
  `Br`, `CondBr`, `Case`, and with a slow path that steps the instruction: `AddrOf` (not `Up`),
  `Load`, `Store`, `Copy`, `Alloc`, `Stmt`, `MakeDope`, `DopeLen`, `Pos`, `SubtypeCheck`,
  `Field`, `LayoutOf` (types whose element layout is static), `LGet`, `LSet`. Memory is
  reached through `Mem` (host pointers of the frame's slots, the temporary area, the instance
  data and a small cache of objects `yavs_rt_obj` asked `TierHost::object` for). Exits to the
  interpreter: `Ret`, the calls, `CallNative`, `Report`, `Suspend`, `Unreachable`, `Elab`, and
  every guard failure (overflow, check, zero divisor, non-finite real, address over 4 GiB),
  which the interpreter re-executes for the identical error. Everything else goes through the
  `yavs_rt_step` callback. The open native lowerings are issues (section 6).
- `AutoTier::start` (`yavs run --tier=auto`) compiles the functions the interpreter reports hot
  (`Heat`), one at a time on one background thread; `AutoTier::stop` ends that at the end of
  the run. The
  spike (`spike.rs`) stays as the JIT smoke test.
- Everything is behind the `llvm` feature (off by default), so the workspace builds without
  LLVM 21.
- Depends on: `llvm-sys`, `yavs-ir`, `yavs-interp` (all optional, feature `llvm`). Called by: the
  `yavs` binary (feature `llvm`).

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | Feature gate, crate-root `#![allow(unsafe_code)]` with its comment, re-exports |
| `src/lower.rs` | `compile`, `compile_one` (one function in a module of its own), `too_big` (size cap), `enter`, `new_jit`, `LlvmTier` (`Tier` impl, `notes` of functions left interpreted, inline/interpreted counts), `Mem` (what compiled code reads and writes, field offsets `MEM_*`), `Gen` (IR to LLVM per function, entry switch, guard exits, slow paths, `host`: an address to a host pointer, inline for frame addresses known in the block, else the internal `yavs_host_r`/`yavs_host_w`), `static_layout`, `yavs_rt_step` and `yavs_rt_obj` trampolines (panics carried over the JIT frames) |
| `src/auto.rs` | `AutoTier` (`--tier=auto`): compile thread fed by `Tier::hot` (the JIT made with the first hot function), per-function `OnceLock` entries (`Tier::ready`), `Heat` threshold, `Note` callback, `stop`, `exit_now` |
| `src/jit.rs` | `Jit` (LLJIT, absolute symbols, lookup), `Module` (own `LLVMContext` per module), `OptLevel`, `portable_cpu`, `Error` |
| `src/spike.rs` | Generated test functions (checked sum, `#[repr(C)]` process state, 10k-instruction body) |
| `tests/tier.rs` | Tier against the interpreter: every integer op and range-checked op, loads and stores of every width in every reachable space, temporaries, copies, descriptors, layouts and logic elements, all on edge values (results and error texts), a loop that runs entirely compiled, `AutoTier` compiling a hot loop and skipping one over the size cap |
| `tests/jit.rs` | Spike tests: overflow trap into Rust, struct state, reference semantics, unresolved symbol by name, background compile and swap |
| `examples/measure.rs` | JIT cost per function size (table in STATUS.md) |
| `WINDOWS.md` | Where LLVM 21 comes from on Windows, llvm-sys static linking, ORC on COFF, CI recipe |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| The interpreter is the semantic reference: every suite must give identical output LLVM-only and interpreter-only | Rule 5 | `docs/plan.md` rule 5, "Verification" |
| LLVM 21 pinned (`llvm-sys =211.1.1`, `strict-versioning`, `prefer-dynamic`), optional feature; LLVM is an approved manual dependency | Rule 5; root `AGENTS.md` rule 9 | `docs/decisions.md` "LLVM bindings" |
| Raw llvm-sys, no inkwell | inkwell lacks ORC LLJIT | same |
| ORC LLJIT, no hand-written object loader; host functions as absolute symbols, nothing looked up in the process | Rule 5 | same |
| Portable CPU: `x86-64-v2` or `generic` aarch64, never the host CPU; `default<O2>` | Rule 5 | same |
| One `LLVMContext` per module (`LLVMOrcCreateNewThreadSafeContextFromLLVMContext`) | LLVM 21 removed `ThreadSafeContextGetContext` | same |
| Runtime errors call a Rust `extern "C"` callback and return a status; no unwinding through JIT frames | Win64 has no unwind info for them | same; WINDOWS.md |
| Shared `libLLVM-21.so` on Linux; static libs from the release tarball on Windows | Link memory; llvm-sys refuses dynamic on Windows | same |
| Side exits: compiled code works on the interpreter frame, returns to the interpreter at calls, waits, reports and every slow or error path, steps other instructions through `yavs_rt_step` | Rule 5 by construction; one implementation of frames, heap and traces | `docs/decisions.md` "LLVM tier: side exits" |
| Memory instructions run natively only where the bytes lie in memory `Mem` reaches and no error is possible; anything else steps the whole instruction, before any side effect | Rule 5: one implementation of address spaces and errors | `docs/decisions.md` "LLVM tier: side exits" (Memory) |
| GC roots: `New`, `Free` and `TempRoot` are stepped and natives and calls are exits, so a collection inside an activation sees the registers, slots and temporaries in the machine; `Stmt` steps while `Mem::temp_roots` (`TierFrame::temp_roots`) is non-zero so the interpreter drops the ending statement's recorded temporaries | Precise roots in both tiers (#422) | `docs/decisions.md` "Shared heap collection" |
| A function whose generated LLVM does not verify, or that names a register beyond its frame, stays interpreted with a note (`--verbose`); never a guess | Fail clearly, per function | `src/lower.rs` `compile` |
| Size cap: a function of more than 50k yavs IR instructions stays interpreted unless `--tier-max-size` raises it (both tiers); `--tier=auto` compiles only hot functions | Superlinear codegen time (user, 2026-10-19) | `docs/decisions.md` "LLVM bindings"; #126 |
| `--tier=auto` (opt-in, default `interp`): the interpreter's estimate of what compiled code saves (ops run in its predecoded loop, minus 12 per op compiled code leaves) calls `Tier::hot` at `Heat::DEFAULT` (1M plus 20k per IR instruction, about the compile time at O0); ONE compile thread, one module per function at O0 (the CLI picks it: O1 codegen costs 10x, IR passes before O0 codegen do not shrink it); entries published through `OnceLock`, swapped in on entry and at the next jump; `stop(false)` drops the queue and joins an idle thread, and when it is busy the CLI closes the design's open files (`Runtime::close_files`) and ends the process with `exit_now` (`_exit` after flushing) | The simulation and the exit never wait for compilation; output independent of compile timing (rule 5); LLVM static destructors never run under the compile thread; not the default while compiled code is slower than the predecoded interpreter (STATUS.md "llvm") | `docs/decisions.md` "LLVM tier: side exits", #125 |
| Time resolution and integer semantics identical in every tier (checked i64) | Rule 7 | `docs/plan.md` rule 7 |
| `unsafe` denied in `Cargo.toml`, allowed at the crate root only; raw handles never leave `jit`/`lower`/`spike` (`auto` only declares `fflush` and `_exit` for `exit_now`) | Root `AGENTS.md` rule 11 | `src/lib.rs`, `Cargo.toml` |

Module-local decisions go in this section in the same commit (root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- `cargo test --workspace` does **not** build this crate's code (feature off): run the feature
  build yourself after any change here.
- LLVM 21.1.8 with static libs is in `/usr/lib/llvm-21` on the dev host; `llvm-sys` finds it via
  `llvm-config` (Windows packages have none, see WINDOWS.md).
- Codegen cost grows faster than linearly with function size; measure with
  `examples/measure.rs` before changing thresholds, one process per row.
- CI (`llvm` job in `.github/workflows/ci.yml`, #127, #128) clippys, tests and runs 47 difftest cases
  with `--tier=llvm` and with `--tier=auto --tier-heat=1,0,0` (at 1 and 4 threads) on Linux x86_64, Linux aarch64 (native runner) and Windows x86_64 (tests only,
  the difftest harness needs a POSIX shell). Windows needs the steps in WINDOWS.md section 4: static
  CRT, empty `libxml2s.lib`, SDK import libs copied into the LLVM `lib` dir.

## 5. How to work here

```sh
CARGO_BUILD_JOBS=6 cargo test -p yavs-llvm --features llvm
cargo build -p yavs --features llvm                    # yavs with --tier=llvm
cargo clippy -p yavs-llvm --features llvm --all-targets -- -D warnings
cargo run --release -p yavs-llvm --features llvm --example measure -- <stmts> [O0|O1|O2]
```

- No diagnostic codes of its own: the `yavs` binary reports `--tier=llvm`/`auto` problems (`E0120`,
  `E0121`); a code of this crate would take a range in `crates/yavs-diag/src/lib.rs`.

## 6. Coordination

- Measure with the difftest: `YAVS_BIN=<yavs built with --features llvm> python3
  tools/difftest/difftest.py --candidate yavs-llvm` (must equal `--candidate yavs`), and
  `yavs run --tier=llvm --verbose` for what each design leaves to the interpreter.

- `--candidate yavs-auto` runs the corpus with `--tier=auto` (also must equal `yavs`);
  `YAVS_RUN_FLAGS=--tier-heat=1,0,0` compiles every function from its first call or jump, so the
  swap-ins happen in the middle of each test.

- Open issues (snapshot 2026-10-19, `gh issue list -R ru551n/yavs --label crate:llvm`): #125
  LLVM tier milestone (tier-up done; open until the M2 to M5 suites pass LLVM-only); native lowering
  still to do, by IR construct: #336 calls between compiled functions, #337 natives, foreign calls and reports, #338 logic
  arrays (`LGet`/`LSet` done),
  #339 signals and waits, #340 heap, #341 real conversions and powers. Epic: #144 (M6).
