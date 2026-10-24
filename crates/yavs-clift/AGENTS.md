# AGENTS.md: yavs-clift

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-clift`.
Spike for #481 item 1; measurements in `docs/STATUS.md` "clift", decision in `docs/decisions.md`
"Cranelift tier (spike)".

## 1. Purpose and boundaries

- A second code generator for the compiled-code contract of `yavs-llvm`: the same function
  signature (`fn(env, regs, mem, pc) -> u32` on the interpreter frame's registers), the same
  frame view (`yavs_llvm::rt::Mem`, `TierFrame`), the same callbacks (`yavs_rt_step`,
  `yavs_rt_signal`, `yavs_rt_host`), the same side exits. Cranelift is a Cargo dependency, so a
  release binary gets a compiled tier without an LLVM install.
- `compile` (`yavs run --tier=clift`) translates every non-elaboration function before time 0;
  `AutoTier` (`--tier=clift-auto`) translates the functions where the sampled interpreter time
  reaches their `Heat`, on one background thread (the scheme of `yavs-llvm`'s `AutoTier`, one
  level).
- `First` (`FirstLevel`) is the Cranelift first level of `yavs-llvm`'s `AutoTier` (`yavs run --tier=auto3`, a yavs built with both features): interpreter, Cranelift for warm functions, LLVM O2 for those that stay hot.
- Everything is behind the `clift` feature (off by default); `yavs --features clift`.
- Depends on `yavs-llvm` (without its `llvm` feature) for `rt.rs`, which holds everything the two
  code generators share, `yavs-interp`, `yavs-ir`.

## 2. Map

| File | Responsibility |
|---|---|
| `src/lower.rs` | `Jit` (a Cranelift JIT module with the runtime callbacks as symbols), `gen_func` (IR to Cranelift IR per function: register cache per block, side exits, slow paths emitted after the body), `compile`; native-frame variants (`Jit::native_of`, `try_native`, `build`, `Gen::native_call`, `record_frame`) and the `Inl` inliner |
| `src/auto.rs` | `AutoTier`, `Heat` (sampling tier-up) |
| `src/lib.rs` | `ClifTier`, `First` (the `FirstLevel` generator for `--tier=auto3`) |
| `tests/tier.rs` | The LLVM tier's tier-against-interpreter tests, run against this tier |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Registers stay in the frame's register array; a per-block cache of SSA values avoids reloads, and is dropped at every block start, join and callback | Callbacks and the interpreter read and write the array; no SSA promotion across blocks | `src/lower.rs` |
| Slow paths (step the instruction in the interpreter) are emitted after the body and rejoin the fast path | Keeps the hot path straight; a stepped instruction may write any register | `src/lower.rs` |
| The kernel-facing instructions are lowered as the LLVM generator does (2026-10-24): `Inst::Edge` and the `rising_edge`/`falling_edge` natives (the kernel's resume key in `Mem::known`, else the event stamp, the codes and the last-value planes of the `TierSig` entry), `SigReadScalar` (codes, planes, or the bytes of an integer, enumeration or real signal), `SigRead` (up to 64 elements through `yavs_rt_codes`, more through `yavs_rt_planes`), `SigAttr` `'event`, the scalar logic assignment the interpreter fuses (`scalar_drives`: the fused `Alloc`/`Store`/`Index`/`LSet` are skipped, `Drive` calls `yavs_rt_drive_bit`), other single-element `Drive`s (`yavs_rt_drive`), `WaitOn` plus `Suspend` (`yavs_rt_wait`), a `WaitOn` that is not the last (`yavs_rt_wait_on`), the wait that names the same signals (`static_wait`, `yavs_rt_wait_same` at the `Stmt`), frame-mode `CallNative` of the natives `direct_native` allows (`yavs_rt_native` with `NATIVE_FRAME_MODE`); `TempRoot`, `NewFrame`, `Stmt`, `Deref`, `MemEq` and `Cat` go to `TierHost::signal` instead of the generic step. A heap or array access outside the frame, temporary area and instance data first tries the entry of `Mem::objs` (one load and compare) before `yavs_rt_host`. None of this is in a native-frame variant (a signal operation or a callback that needs a step makes the function frame-mode only) | Without them a clocked process made 4800 signal callbacks and 960 steps per 1000 activations against 4 and 8 under LLVM, and ran twice as many instructions as the interpreter | `src/lower.rs` `impl Gen` "kernel-facing instructions" |
| Not lowered inline (stepped): calls (except to a function with a native variant), natives other than the `std_ulogic` tables, edges and the frame-mode `direct_native` ones, `Deref`, `New`, reports, `**`, `SlicePos`, `LEq`, `LFill`, `NewFill`, `PkgAlloc`, `SigLastValue`, and signal operations other than those above | Same list as the LLVM tier minus `LayoutOf` | `src/lower.rs` |
| `AutoTier` calls `NativeFlags::set` (`Runtime::native_flags`) after publishing an entry | The interpreter's fast loop asks only those flags; without it translated code is never entered | `src/auto.rs`, `yavs-interp` |
| A function that is small (400 IR instructions, 128 registers, 1 KiB of slots), has no static link and needs no step on any path gets a second body `n<id>` (native-frame variant: registers and slots on the native stack, `(env, mem, depth, args) -> (status, value)`); callers call it directly, so a call costs no interpreter frame. Anything that cannot finish (a failing check, a slow path, `Unreachable`) records the frame (`yavs_rt_deopt`: registers, slots, pc, temporary base) and every caller on the way records itself; the frame-mode root pushes them all (`yavs_rt_deopt_end` = `TierHost::materialise`) and returns `TIER_UNWIND`, so errors, call chains and the rest of the run are the interpreter's | The call protocol of the interpreter (frame push, two view refreshes, lookup, return) was 80% of the time of call-heavy code; the machinery is the LLVM tier's "compiled frames" | `docs/decisions.md` "Compiled frames", `src/lower.rs` |
| A native variant is not made for a function that passes an address of its own frame to a call, nor for one that calls a function without a variant (a step needs the interpreter's frame) | Addresses are resolved against the interpreter's top frame; a native frame's slots are not there, so the callee would materialise the whole chain on every call | `Gen::native_call` |
| Cranelift's `Inline` (`Context::inline`) inlines the small bodies (up to 150 Cranelift instructions, 2000 in all per caller) into their callers before compilation; the alias regions of guest memory are shared between caller and callee (ids 2, 3), the registers of a native variant have their own (4) | The inliner merges regions by id; sharing the guest-memory region keeps stores and loads of caller and callee ordered | `src/lower.rs` `Inl` |
| Registers the interpreter starts a frame with (`rt::preloads`: defined once by a `Const`, `ConstF` or a non-`Up` `AddrOf`) are treated as known addresses at every block start (`base_known`); a frame-mode entry after them may find the register of a parameter's slot address unset (`Pre::lean`), so a load or store through it goes by the known frame offset, never by the register | Without it a recursive function read the caller's parameter after a merge with main (the interpreter leaves those registers zero) | `src/lower.rs` `gen_func` |
| Measurement knobs `YAVS_CLIFT_NATIVE` (`0` turns the variants off), `YAVS_CLIFT_NATIVE_MAX` (most variants made), `YAVS_CLIFT_INLINE` (largest inlined body, `0` for none), `YAVS_CLIFT_DUMP` (IR of each function and why a variant was or was not made), `YAVS_CLIFT_TRACE` (a line per materialised frame); plus `YAVS_CLIFT_OPT` (`speed` default, `none`), `YAVS_CLIFT_VERIFY` (on in debug builds, off in release), `YAVS_CLIFT_RA` (`backtracking` default, `single_pass`) | Spike measurement only | `src/lower.rs` |

## 4. Invariants and pitfalls

- A value cached for a register is valid only while the generator is in a block dominated by the
  block that defined it: clear the cache at every point that joins paths or calls out.
- Compiled code never reports an error itself: an inline instruction that fails returns its own
  pc and the interpreter re-executes it (rule 5).

## 5. How to work here

```sh
cargo test -p yavs-clift --features clift
cargo build --release -p yavs --features clift
YAVS_BIN=<yavs built with --features clift> python3 tools/difftest/difftest.py --candidate yavs-clift
```
