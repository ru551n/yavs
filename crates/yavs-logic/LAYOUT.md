# Packed logic layout: nibbles vs bit-planes

Both layouts implement `Repr` and sit behind `Logic<R>`; `LogicVec` is the alias the rest of yavs
uses. Default is **bit-planes**; `--features nibbles` switches the alias. Both are always compiled,
tested against a `Vec<StdULogic>` reference (exhaustive pairs and triples at every bit position,
proptest) and benchmarked.

- **Nibbles:** one 4-bit code per element, two per byte. Ops are 16x16 table lookups per nibble.
- **Bit-planes:** `Vec<[u64; 4]>`, block `i` holds elements 64i..64i+63, plane k is bit k of the
  code. Ops are bit-parallel boolean formulas (about 10 word ops per 64 elements; resolution about 40).

The codes are the `std_ulogic` positions (`U X 0 1 Z W L H -` = 0..8), so a code is also the VHPI
`vhpiEnumT` value and the index into the FST character table. Element 0 is the rightmost element.

## Results

`cargo bench -p yavs-logic`, Intel Core Ultra 9 275HX, Rust 1.99, median ns per call, nibbles / planes.
Random operands over all nine values (add/compare: only 0/1).

| op | 1 | 8 | 32 | 64 | 1024 | 65536 |
|---|---|---|---|---|---|---|
| get+set one element | 3 / 4 | 7 / 8 | 7 / 8 | 7 / 8 | 7 / 9 | 8 / 8 |
| and | 12 / 12 | 12 / 12 | 15 / 12 | 23 / 12 | 250 / 29 | 15743 / 1462 |
| or | 12 / 12 | 12 / 12 | 15 / 12 | 23 / 12 | 249 / 29 | 15718 / 1448 |
| xor | 12 / 12 | 12 / 12 | 15 / 12 | 23 / 12 | 248 / 28 | 15714 / 1363 |
| resolve, 2 drivers | 12 / 16 | 12 / 16 | 20 / 17 | 24 / 16 | 251 / 46 | 15720 / 2570 |
| resolve, 4 drivers | 14 / 25 | 15 / 25 | 33 / 26 | 55 / 25 | 727 / 113 | 45929 / 6921 |
| add (unsigned) | 58 / 63 | 72 / 62 | 159 / 64 | 249 / 60 | 3032 / 84 | 348340 / 2542 |
| compare (unsigned) | 53 / 54 | 71 / 53 | 122 / 53 | 202 / 56 | 2398 / 69 | 150000 / 1748 |
| to_string | 18 / 17 | 22 / 24 | 41 / 57 | 61 / 98 | 636 / 1300 | 41442 / 82450 |

Every call here includes one heap allocation (about 10 ns), which is why widths up to 64 look flat.

## Recommendation: bit-planes

- **Speed:** equal up to 32 elements, then planes win 8x to 10x on logic ops and resolution,
  and 40x to 140x on add and compare at 1024+, because a vector that is all `0 1 L H` turns into
  plain `u64` words with three word ops per 64 elements (`to_words01`) and runs through `BitVec`.
  Nibbles must walk element by element for that check. Element get/set is the same (about 8 ns).
- **Memory:** both use 4 bits per element, so large vectors are identical. Planes round up to
  64-element blocks: a 1-bit signal costs 32 bytes against 1 byte. The kernel should keep scalars
  (the vast majority of signals) as a bare `StdULogic`, not a length-1 vector.
- **VHPI and waves:** nibbles map to a `vhpiEnumT` array or an FST character string with one
  table lookup per element. Planes need a transpose, but waves record only changes and have to
  decode the per element value either way; `to_string` is the one op where planes lose (2x,
  `get` costs four shifts). A block-wise decoder (one pass over the four words) would close that
  gap if the wave writer shows up in profiles. Planes give a cheap "did anything change" compare
  and a cheap "is this vector all 0/1" test (`to_words01`), which the wave writer can use to
  pick a 2-state encoding.
- **Simplicity:** nibbles are simpler to read and to patch; planes need the class-mask formulas
  (about 100 lines, all verified exhaustively against the scalar IEEE tables).

Keeping both costs about 100 lines each, and they share all tests, so both stay behind the feature.
