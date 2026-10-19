# AGENTS.md: yavs-llvm

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-llvm`.
Binding choices are in `docs/decisions.md` "LLVM bindings: llvm-sys 211, no inkwell"; Windows
in `crates/yavs-llvm/WINDOWS.md`; measurements in `STATUS.md` "llvm".

## 1. Purpose and boundaries

- The **LLVM tier** (M6): `compile(&Program, OptLevel) -> LlvmTier` lowers every
  non-elaboration IR function to LLVM 21 and JITs it through ORC LLJIT (portable CPU, O2, Rust
  runtime callbacks as absolute symbols). `LlvmTier` implements `yavs_interp::Tier`; `yavs run
  --tier=llvm` sets it on the runtime. Design: `docs/decisions.md` "LLVM tier: side exits to the
  interpreter".
- Compiled code runs on the interpreter frame's registers. Inline: `Const`, `ConstF`, `Move`,
  `Bin`/`BinRange` (not `**`), `Un`, `FBin` (not `**`), `FUn`, `IToF`, `Cmp`, `FCmp`, `Bool`,
  `Check`, `FCheck`, `Index`, `Br`, `CondBr`, `Case`. Exits to the interpreter: `Ret`, the
  calls, `CallNative`, `Report`, `Suspend`, `Unreachable`, `Elab`, and every guard failure
  (overflow, check, zero divisor, non-finite real, address over 4 GiB), which the interpreter
  re-executes for the identical error. Everything else goes through the `yavs_rt_step` callback.
  The open native lowerings are issues (section 6).
- Not yet: tier-up of hot functions on a background thread (`--tier=auto`, #125), the size
  threshold (#126, the user's), Windows (#127), aarch64 (#128). The spike (`spike.rs`) stays as
  the JIT smoke test.
- Everything is behind the `llvm` feature (off by default), so the workspace builds without
  LLVM 21.
- Depends on: `llvm-sys`, `yavs-ir`, `yavs-interp` (all optional, feature `llvm`). Called by: the
  `yavs` binary (feature `llvm`).

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | Feature gate, crate-root `#![allow(unsafe_code)]` with its comment, re-exports |
| `src/lower.rs` | `compile`, `LlvmTier` (`Tier` impl, `notes` of functions left interpreted, inline/interpreted counts), `Gen` (IR to LLVM per function, entry switch, guard exits), `yavs_rt_step` trampoline (panics carried over the JIT frames) |
| `src/jit.rs` | `Jit` (LLJIT, absolute symbols, lookup), `Module` (own `LLVMContext` per module), `OptLevel`, `portable_cpu`, `Error` |
| `src/spike.rs` | Generated test functions (checked sum, `#[repr(C)]` process state, 10k-instruction body) |
| `tests/tier.rs` | Tier against the interpreter: every integer op and range-checked op on edge values (results and error texts), a loop that runs entirely compiled |
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
| A function whose generated LLVM does not verify, or that names a register beyond its frame, stays interpreted with a note (`--verbose`); never a guess | Fail clearly, per function | `src/lower.rs` `compile` |
| Proposed size threshold: O2 up to 40k IR instructions per function, larger stays interpreted, big processes as several functions | Superlinear codegen time | `docs/decisions.md` "LLVM bindings"; decision issue #126 |
| Time resolution and integer semantics identical in every tier (checked i64) | Rule 7 | `docs/plan.md` rule 7 |
| `unsafe` denied in `Cargo.toml`, allowed at the crate root only; raw handles never leave `jit`/`lower`/`spike` | Root `AGENTS.md` rule 11 | `src/lib.rs`, `Cargo.toml` |

Module-local decisions go in this section in the same commit (root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- `cargo test --workspace` does **not** build this crate's code (feature off): run the feature
  build yourself after any change here.
- LLVM 21.1.8 with static libs is in `/usr/lib/llvm-21` on the dev host; `llvm-sys` finds it via
  `llvm-config` (Windows packages have none, see WINDOWS.md).
- Codegen cost grows faster than linearly with function size; measure with
  `examples/measure.rs` before changing thresholds, one process per row.
- Windows LLVM (static CRT, libxml2s stub) is researched, never run (#127).

## 5. How to work here

```sh
CARGO_BUILD_JOBS=6 cargo test -p yavs-llvm --features llvm
cargo build -p yavs --features llvm                    # yavs with --tier=llvm
cargo clippy -p yavs-llvm --features llvm --all-targets -- -D warnings
cargo run --release -p yavs-llvm --features llvm --example measure -- <stmts> [O0|O1|O2]
```

- No diagnostic codes of its own: the `yavs` binary reports `--tier=llvm` problems (`E0120`,
  `E0121`); a code of this crate would take a range in `crates/yavs-diag/src/lib.rs`.

## 6. Coordination

- Measure with the difftest: `YAVS_BIN=<yavs built with --features llvm> python3
  tools/difftest/difftest.py --candidate yavs-llvm` (must equal `--candidate yavs`), and
  `yavs run --tier=llvm --verbose` for what each design leaves to the interpreter.

- Open issues (snapshot 2026-10-19, `gh issue list -R ru551n/yavs --label crate:llvm`): #125
  tiering and background compilation (`--tier=auto`); #126 decision: O2 threshold of 40k
  instructions; #127 Windows LLVM unverified; #128 JIT tier and CI on aarch64. Native lowering
  still to do, by IR construct: #334 memory and addresses, #335 descriptors and layouts, #336
  calls between compiled functions, #337 natives, foreign calls and reports, #338 logic arrays,
  #339 signals and waits, #340 heap, #341 real conversions and powers. Epic: #144 (M6).
