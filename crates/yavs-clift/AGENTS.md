# AGENTS.md: yavs-clift

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-clift`.
Spike for #481 item 1; measurements in `STATUS.md` "clift", decision in `docs/decisions.md`
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
- Everything is behind the `clift` feature (off by default); `yavs --features clift`.
- Depends on `yavs-llvm` (without its `llvm` feature) for `rt.rs`, which holds everything the two
  code generators share, `yavs-interp`, `yavs-ir`.

## 2. Map

| File | Responsibility |
|---|---|
| `src/lower.rs` | `Jit` (a Cranelift JIT module with the runtime callbacks as symbols), `gen_func` (IR to Cranelift IR per function: register cache per block, side exits, slow paths emitted after the body), `compile` |
| `src/auto.rs` | `AutoTier`, `Heat` (sampling tier-up) |
| `tests/tier.rs` | The LLVM tier's tier-against-interpreter tests, run against this tier |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Registers stay in the frame's register array; a per-block cache of SSA values avoids reloads, and is dropped at every block start, join and callback | Callbacks and the interpreter read and write the array; no SSA promotion across blocks | `src/lower.rs` |
| Slow paths (step the instruction in the interpreter) are emitted after the body and rejoin the fast path | Keeps the hot path straight; a stepped instruction may write any register | `src/lower.rs` |
| Not lowered inline (stepped): calls, natives other than the `std_ulogic` tables and edges, `LayoutOf`, `Deref`, `New`, reports, `**`, signal operations other than the four that use `yavs_rt_signal` | Same list as the LLVM tier minus `LayoutOf` | `src/lower.rs` |
| Measurement knobs `YAVS_CLIFT_OPT` (`none`, `speed`), `YAVS_CLIFT_VERIFY` (`1` default), `YAVS_CLIFT_RA` | Spike measurement only | `src/lower.rs` |

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
