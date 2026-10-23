# AGENTS.md: yavs-ffi

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-ffi`.

## 1. Purpose and boundaries

- Calls C functions whose signature is known only at run time (VHPIDIRECT `foreign`
  subprograms) **without libffi**: argument classification in plain Rust per target ABI, plus a
  small asm trampoline per target. Also the VHDL to C type mapping in the reference simulator's convention, so C
  code written for the reference simulator works unchanged.
- Targets: x86_64 Linux (SysV), x86_64 Windows (Win64), aarch64 Linux (AAPCS64). Any other target
  is a compile error.
- Does not: marshal VHDL values (bit-planes to bytes, records, copy-back: `yavs-interp`
  `foreign.rs`), load shared libraries or look up symbols (`yavs-vhpi` `bind_foreign`, called by
  `yavs run --load`).
- Depends on: nothing. Called by: `yavs-ir` (`CType`, `Signature` in `ForeignRef`), `yavs-interp`,
  `yavs-lower` (`vhdl::lower_result` for C types), `yavs-elab` (`ForeignFn` in `Ir`), `yavs-vhpi`.

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | `CType`, `Signature`, `Arg`, `Ret`, `MAX_ARGS = 64`, errors |
| `src/abi.rs` | Classification: integer register, float register or stack slot per ABI |
| `src/sys.rs` | The only `unsafe`: naked asm trampolines per target and the call into them; `ForeignFn`, `call`, `testing` |
| `src/typed.rs` | `CArgType`/`CRetType`/`CFnPtr`: signatures from Rust `extern "C" fn` types, `ForeignFn::from_fn` without `unsafe` |
| `src/vhdl.rs` | VHDL to C mapping (`vhdl::lower`): smallest signed int for integer ranges, smallest unsigned for enums, ... |
| `build.rs` | Generates the property-test callees for `tests/generated.rs` (fixed seed, all signatures up to two parameters) |
| `tests/abi.rs`, `tests/generated.rs` | Hand-written ABI cases; 431 generated callees compared with direct calls |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Own per-ABI trampolines (SysV, AArch64, Win64), no libffi | Minimal dependencies, no manual install | `docs/plan.md` architecture `yavs-rt`, ir.md 1.7 |
| `CType` derives serde (the one dependency, `serde`, with its derive) | `ForeignRef` is part of the serialised IR (the compilation cache of lowered packages, #407) | `docs/decisions.md` "Compilation cache" |
| `unsafe` denied crate-wide, allowed only in `sys` (crate opts out of the workspace `forbid`) | Root `AGENTS.md` rule 11 | `Cargo.toml` comment |
| The reference simulator's VHDL to C type convention | Existing VHPIDIRECT C code works | `src/vhdl.rs`; `docs/plan.md` "Foreign subprograms" |
| More than 64 arguments is an error, never a crash | Fail early | `MAX_ARGS` |
| Typed Rust callees bind without `unsafe` (`ForeignFn::from_fn`); real symbols are bound by the elaborator, the one unsafe step | Safe tests and natives | `docs/decisions.md` "IR and interpreter" (Foreign calls) |
| All three platforms tier 1: test aarch64 under qemu-user, Windows under wine | Plan platforms | `docs/plan.md` "Platforms"; docs/STATUS.md "ffi" |

Module-local decisions go in this section in the same commit (root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- Stack pointer 16-byte aligned at the call; stack slots copied below it. Win64 needs the 32-byte
  shadow space and passes floats in the matching positional register; AAPCS64 and SysV count
  integer and float registers separately. Every change gets a generated or hand-written case on
  **all three** targets (cross commands below), not just the host.
- Keep classification in Rust (`abi.rs`) and the asm minimal; no per-signature asm.
- The generated tests use a fixed seed: changing `build.rs` changes the case set, review it.

## 5. How to work here

```sh
CARGO_BUILD_JOBS=6 cargo test -p yavs-ffi
# aarch64 Linux (qemu-user, aarch64-linux-gnu-gcc, rustup target aarch64-unknown-linux-gnu)
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER="qemu-aarch64 -L /usr/aarch64-linux-gnu" \
  cargo test -p yavs-ffi --target aarch64-unknown-linux-gnu
# Windows x86_64 (mingw-w64, wine, rustup target x86_64-pc-windows-gnu)
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=wine WINEDEBUG=-all \
  cargo test -p yavs-ffi --target x86_64-pc-windows-gnu
```

- No diagnostic codes: errors are returned to `yavs-interp`, which reports them as runtime errors.

## 6. Coordination

- Marshalling and copy-back rules live in `yavs-interp` (`src/foreign.rs`) and
  `docs/decisions.md` "IR and interpreter" (Foreign arrays).
- Open issues (snapshot 2026-10-18, `gh issue list -R ru551n/yavs --label crate:ffi`): none
  (#95 `--load` end to end is done).
