# AGENTS.md: yavs-cache

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-cache`.

## 1. Purpose and boundaries

- The store of the compilation cache (`docs/decisions.md` "Compilation cache", #407): a [`Key`]
  (sha256 over everything an entry depends on, built with `KeyBuilder`) maps to a byte string in
  `<library dir>/yavs-cache/<kind>-<key>.img`. Atomic writes, validation of everything read,
  pruning to the newest `MAX_ENTRIES`, `Store::clean`.
- Does not: know VHDL, `Db`, or when to cache (`crates/yavs/src/cache.rs` decides; `yavs-sem`
  `image.rs` makes the bytes), lock anything, or ever fail a run: a write error is silent (`YAVS_LOG=debug`
  notes it), a bad read is a miss.
- Depends on: `sha2`, `twox-hash`, `memmap2`. Called by: `yavs`.

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | `build_id`, `Key`, `KeyBuilder`, `Store` (`get`, `put`, `note`/`noted`: an empty `.seen` file that says a caller wanted an entry once, newest `MAX_NOTES` kept, `clean`), entry header (magic, format, key, length, payload xxh3 checksum), `Entry` (the payload of a validated entry, mapped from the file: it implements `AsRef<[u8]>`, so it becomes a `yavs_diag::Buffer` and readers keep ranges of it), tests |
| `src/map.rs` | Maps an entry file (reads it where mapping is not possible); the only `unsafe` in the crate |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Every key mixes in `FORMAT` and `build_id()` (version, size and mtime of the executable) | An entry never outlives the code that made it, with no build-time hash | `docs/decisions.md` "Compilation cache" |
| Entry = header + payload, verified (magic, format, key echo, exact length, xxh3 checksum) before use | A truncated or damaged file is a miss, not a wrong result; xxh3 because sha256 of a 10 MB image cost as much as reading it (#426) | same |
| Writes: temporary file in the same directory, then `rename`; no lock | Concurrent runs never see half an entry; readers need no lock | same |
| `put` never returns an error to the caller | The cache can only make a run faster | same |
| Pruning by modification time, which `get` refreshes | Keep what is in use | same |
| An entry is mapped, validated before use, and never written in place | No read and no copy of a 7 MB image; a mapping stays valid when the name is replaced or removed (Unix) | `docs/decisions.md` "Zero-copy cache entries" |

## 4. Invariants and pitfalls

- Bump `FORMAT` when the header layout changes. A change of the payload type is covered by `build_id`.
- Key parts are length-prefixed (`KeyBuilder::part`): never concatenate before hashing.
- Windows: `rename` over an entry another process has open or mapped fails; the writer drops its temporary
  file and the existing entry (same key, same content) stays. A prune cannot remove a mapped entry either (ignored).
- Never write to an entry file in place or truncate one: a run may hold it mapped (a damaged file is found when
  `get` validates it, not later). `unsafe` stays in `map.rs`.

## 5. How to work here

```sh
cargo test -p yavs-cache
cargo test -p yavs --test cache      # the end-to-end gate
```

- No diagnostic codes (`E0123` of `yavs cache clean` is the CLI's).

## 6. Coordination

- #407 (all stages). Kinds in use: `sem` (analysis image, `crates/yavs/src/cache.rs`), `pre` (the analysis of the first files of a closure, shared by closures that start alike) and `low` (lowered run-time packages, `yavs-elab` `pkgcall::package_lowerer` through `yavs_sem::StageCache`) and `ir` (the lowered design with the optimised copies of the functions earlier runs called, `yavs-elab` `sim.rs` `LoweringKey`, #479); stage 3 (compiled code) would add one.
