# AGENTS.md: yavs-llvm

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-llvm`.
Binding choices are in `docs/decisions.md` "LLVM bindings: llvm-sys 211, no inkwell"; Windows
in `crates/yavs-llvm/WINDOWS.md`; measurements in `STATUS.md` "llvm".

## 1. Purpose and boundaries

- Today an **M6 risk spike**: llvm-sys 211 bindings, one ORC LLJIT per simulation with a portable
  CPU, O2 via `LLVMRunPasses`, Rust runtime callbacks as absolute symbols, checked arithmetic
  that traps into Rust, background compilation on one thread with an atomic function-pointer
  swap. It does **not** lower `yavs-ir` yet (#125).
- M6 target: lower IR to LLVM 21 with tiering (start interpreted, move hot processes to LLVM),
  compile on a background thread so simulation never waits, no object cache.
- Everything is behind the `llvm` feature (off by default), so the workspace builds without
  LLVM 21.
- Depends on: `llvm-sys` (optional). Called by: nobody yet. Will consume `yavs-ir` and implement
  the kernel `Process` like `yavs-interp`.

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | Feature gate, crate-root `#![allow(unsafe_code)]` with its comment, re-exports |
| `src/jit.rs` | `Jit` (LLJIT, absolute symbols, lookup), `Module` (own `LLVMContext` per module), `OptLevel`, `portable_cpu`, `Error` |
| `src/spike.rs` | Generated test functions (checked sum, `#[repr(C)]` process state, 10k-instruction body) |
| `tests/jit.rs` | Spike tests: overflow trap into Rust, struct state, reference semantics, unresolved symbol by name, background compile and swap |
| `examples/measure.rs` | JIT cost per function size (table in STATUS.md) |
| `WINDOWS.md` | Where LLVM 21 comes from on Windows, llvm-sys static linking, ORC on COFF, CI recipe |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| The interpreter is the semantic reference: every suite must give identical output LLVM-only and interpreter-only | Rule 5 (GHDL #1829, NVC #1688) | `docs/plan.md` rule 5, "Verification" |
| LLVM 21 pinned (`llvm-sys =211.1.1`, `strict-versioning`, `prefer-dynamic`), optional feature; LLVM is an approved manual dependency | Rule 5; root `AGENTS.md` rule 9 | `docs/decisions.md` "LLVM bindings" |
| Raw llvm-sys, no inkwell | inkwell lacks ORC LLJIT | same |
| ORC LLJIT, no hand-written object loader; host functions as absolute symbols, nothing looked up in the process | Rule 5 | same |
| Portable CPU: `x86-64-v2` or `generic` aarch64, never the host CPU; `default<O2>` | Rule 5 (NVC #887/#1311) | same |
| One `LLVMContext` per module (`LLVMOrcCreateNewThreadSafeContextFromLLVMContext`) | LLVM 21 removed `ThreadSafeContextGetContext` | same |
| Runtime errors call a Rust `extern "C"` callback and return a status; no unwinding through JIT frames | Win64 has no unwind info for them | same; WINDOWS.md |
| Shared `libLLVM-21.so` on Linux; static libs from the release tarball on Windows | Link memory; llvm-sys refuses dynamic on Windows | same |
| Proposed size threshold: O2 up to 40k IR instructions per function, larger stays interpreted, big processes as several functions | Superlinear codegen time | `docs/decisions.md` "LLVM bindings"; decision issue #126 |
| Time resolution and integer semantics identical in every tier (checked i64) | Rule 7 | `docs/plan.md` rule 7 |
| `unsafe` denied in `Cargo.toml`, allowed at the crate root only; raw handles never leave `jit`/`spike` | Root `AGENTS.md` rule 11 | `src/lib.rs`, `Cargo.toml` |

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
cargo clippy -p yavs-llvm --features llvm --all-targets -- -D warnings
cargo run --release -p yavs-llvm --features llvm --example measure -- <stmts> [O0|O1|O2]
```

- No diagnostic codes yet; when it reports to users, pick a range and add it to the registry in
  `crates/yavs-diag/src/lib.rs`.

## 6. Coordination

- Open issues (snapshot 2026-10-18, `gh issue list -R ru551n/yavs --label crate:llvm`): #125
  lower IR to LLVM 21 with tiering and background compilation; #126 decision: O2 threshold of 40k
  instructions; #127 Windows LLVM unverified; #128 LLVM tier and CI on aarch64. Epic: #144 (M6).
