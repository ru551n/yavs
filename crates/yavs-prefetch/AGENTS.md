# AGENTS.md: yavs-prefetch

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-prefetch`.

## 1. Purpose and boundaries

- A cache prefetch hint, `lines(&T, n)`: ask the CPU to load the first `n` cache lines (`LINE`, 64
  bytes) of a value into the cache without waiting. The one place the kernel's batch loop needs a
  CPU instruction, so `yavs-kernel` stays `forbid(unsafe_code)` (`docs/decisions.md` "Prefetch of
  the next process in a batch", #766).
- Does not: know anything of the kernel, processes or VHDL, or change program state (a prefetch
  never faults, whatever the address). On a target other than x86-64 `lines` does nothing.
- Depends on: nothing. Called by: `yavs-kernel` (`kernel.rs`, the batch loop: `PREFETCH_BATCH`,
  `PREFETCH_AHEAD`, `PREFETCH_LINES`, `SLOT_LINES`).

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | `LINE`, `lines`, and the private `hint` (`_mm_prefetch` with `_MM_HINT_T0`): the only `unsafe` |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| The crate has its own `[lints.rust]` (`unsafe_code = "deny"`), not `workspace = true`; `unsafe` is allowed in `hint` alone | The workspace forbids unsafe code and `forbid` cannot be relaxed by an attribute | root `AGENTS.md` rule 11, `Cargo.toml` of this crate |
| `lines` is safe to call on any reference; the pointer arithmetic is `wrapping_add` | A prefetch is only a hint, so a line past the end of the value is harmless | `src/lib.rs` |

## 4. Invariants and pitfalls

- Keep `hint` the only `unsafe` function and keep the `SAFETY` comment true: no other intrinsic
  that can fault or write belongs here.
- Another architecture's prefetch (AArch64 `prfm`) is not implemented: `hint` is a no-op there.

## 5. How to work here

```sh
cargo build -p yavs-prefetch
```

- No tests of its own: the effect is measured through the kernel (`docs/decisions.md` "Prefetch of
  the next process in a batch").
- No diagnostic codes.

## 6. Coordination

- #766 (the compiled clocked RTL rounds that introduced it).
