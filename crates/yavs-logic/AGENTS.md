# AGENTS.md: yavs-logic

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-logic`.

## 1. Purpose and boundaries

- IEEE 1164 logic: the scalar `StdULogic` with the 1164 truth tables, and packed vectors
  `Logic<R>` over two layouts (`Planes`, the default, and `Nibbles`), plus a `BitVec` fast path
  for 0/1 vectors with numeric_std-style add, subtract, compare, extend and integer conversion.
- Does not: know about signals, drivers, VHDL types or the IR. The kernel decides storage; the
  interpreter decides when to call these operations.
- Depends on: nothing (dev: `criterion`, `proptest`). Called by: `yavs-kernel` (bit-plane signal
  storage and resolution), `yavs-interp` (logic array ops), `yavs-elab`, `yavs-lower` (tests).

## 2. Map

| File | Responsibility |
|---|---|
| `src/scalar.rs` | `StdULogic` (`U X 0 1 Z W L H -` = codes 0..8), `ALL`, tables `RESOLVE`, `AND`, `OR`, `XOR`, `MATCH`, `NOT`, `TO_X01`, `TO_X01Z`, `TO_UX01`; scalar ops, `rising_edge`/`falling_edge` |
| `src/repr.rs` | `Repr` trait (`new`, `len`, `get`, `set`, `bin`, `un`, `to_words01`, `from_words01`), `BinOp`, `UnOp` |
| `src/planes.rs` | `Planes`: `Vec<Block>`, `Block = [u64; 4]` (64 elements, plane k = bit k of the code); `blocks`, `from_blocks`, `resolve_block`, `bin_block`, `not_block` (kernel accessors; the last two are the kernel's gates, #779) |
| `src/nibbles.rs` | `Nibbles`: 4-bit codes, table lookups (behind `--features nibbles` as the default alias) |
| `src/vec.rs` | `Logic<R>`: the vector API used by the rest of yavs; `LogicVec = Logic<DefaultRepr>` in `lib.rs` |
| `src/wide.rs` | Block-slice kernels over `[Block]` plus an element offset (copy, equality, `is_x`, `?=`, many-source resolution, `Lane`/`L4` four-blocks-at-a-time arithmetic); used by `Planes`, the kernel's `Buf` and the ieee natives |
| `src/bitvec.rs` | `BitVec`: plain `u64` words for 0/1 vectors, arithmetic and conversions |
| `tests/ieee_tables.rs` | Tables checked against the vendored `lib/ieee` std_logic_1164 body |
| `tests/layouts.rs` | Both layouts vs a `Vec<StdULogic>` reference: exhaustive pairs/triples at every bit position, proptest |
| `tests/wide.rs` | Every `wide` kernel and the word-level `Logic` slice/concat/shift/`?=`/`is_x` and `BitVec` compare/`to_i64` against per-element models: random planes, all nine values, odd lengths, every offset |
| `benches/layout.rs` | Criterion layout benchmark; results in `LAYOUT.md` |
| `benches/wide.rs` | The `wide` kernels against the pairwise per-block loops they replaced |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Bit-planes are the default layout everywhere, signals included (user, 2026-10-18) | 8x to 140x faster ops at width, same memory | `docs/plan.md` architecture `yavs-rt`; `docs/decisions.md` "Packed logic vectors: bit-planes"; `LAYOUT.md` |
| Nibbles stay compiled and tested behind `--features nibbles` | About 100 lines; shares all tests and benchmarks | `docs/decisions.md` "Packed logic vectors" |
| Codes are the `std_ulogic` positions; a code is also the VHPI `vhpiEnumT` value and the FST character index | One encoding at every boundary | `LAYOUT.md` |
| Element 0 is the rightmost element | Matches numeric bit order; elab maps row-major position p to `n - 1 - p` | `LAYOUT.md`; `docs/decisions.md` "Elaboration (yavs-elab, M2)" |
| BIT and BOOLEAN in bit-plane arrays use the codes of '0'/'1' (2/3) | Shared plane ops stay correct for `bit_vector` | `docs/decisions.md` "Interpreter: BIT/BOOLEAN codes" |
| Kernel keeps scalar signals as bare `StdULogic` | A 1-element vector costs 32 bytes | `docs/decisions.md` "Packed logic vectors" |
| `#![forbid(unsafe_code)]`; no `std::arch`, no per-CPU variants: wide loops are portable lane-wise code the compiler vectorises | Measured: an AVX2 build of the same loops is no faster; simple gates are faster as scalar words | `src/lib.rs`; `docs/decisions.md` "Wide plane kernels" |

Module-local decisions go in this section (and `LAYOUT.md` for layout data) in the same commit
(root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- Planes beyond `len` in the last block are cleared (`Planes::norm`) so the derived `==` works;
  an op that can set them (e.g. `not`) must call `norm`. Every new op needs the exhaustive tests at every bit position (block boundaries 63/64).
- Both layouts must give identical results; add a new op to `Repr` for both and to the reference
  comparison in `tests/layouts.rs`.
- A new loop over blocks goes in `wide.rs` with a case in `tests/wide.rs` (offsets 0, 3, 63, 64, lengths 0, 1, 63, 64, 65, over 256) and a `benches/wide.rs` entry; measure before keeping any lane or SIMD variant (simple gates lost to the transpose).
- `Planes::blocks`/`from_blocks`/`resolve_block`/`bin_block`/`not_block`, `yavs_logic::wide::{get_block, put_block, chunks, copy_range, eq_range, resolve_into}` and `Logic::repr`/`from_repr` are kernel API
  (`docs/decisions.md` "Kernel round 2"); changing them breaks `yavs-kernel`.
- `tests/ieee_tables.rs` reads `lib/ieee`; never "fix" a table without the 1164 body agreeing.

## 5. How to work here

```sh
CARGO_BUILD_JOBS=6 cargo test -p yavs-logic                    # planes default
cargo test -p yavs-logic --features nibbles                    # nibbles as default alias
cargo bench -p yavs-logic                                      # update LAYOUT.md if numbers matter
```

- No diagnostic codes: errors are not produced here (callers check lengths and ranges).

## 6. Coordination

- Changes that affect storage are kernel changes too: read `crates/yavs-kernel/DESIGN.md`.
- Open issues (snapshot 2026-10-18): none labelled `crate:logic`. #83 (records resolved as a
  whole) was closed with logic fields kept on planes.
