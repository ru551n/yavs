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
- Depends on: `sha2`. Called by: `yavs`.

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | `build_id`, `Key`, `KeyBuilder`, `Store` (`get`, `put`, `clean`), entry header (magic, format, key, length, payload sha256), tests |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Every key mixes in `FORMAT` and `build_id()` (version, size and mtime of the executable) | An entry never outlives the code that made it, with no build-time hash | `docs/decisions.md` "Compilation cache" |
| Entry = header + payload, verified (magic, format, key echo, exact length, sha256) before use | A truncated or damaged file is a miss, not a wrong result | same |
| Writes: temporary file in the same directory, then `rename`; no lock | Concurrent runs never see half an entry; readers need no lock | same |
| `put` never returns an error to the caller | The cache can only make a run faster | same |
| Pruning by modification time, which `get` refreshes | Keep what is in use | same |

## 4. Invariants and pitfalls

- Bump `FORMAT` when the header layout changes. A change of the payload type is covered by `build_id`.
- Key parts are length-prefixed (`KeyBuilder::part`): never concatenate before hashing.
- Windows: `rename` over an entry another process has open fails; the writer drops its temporary
  file and the existing entry (same key, same content) stays.

## 5. How to work here

```sh
cargo test -p yavs-cache
cargo test -p yavs --test cache      # the end-to-end gate
```

- No diagnostic codes (`E0123` of `yavs cache clean` is the CLI's).

## 6. Coordination

- #407 (all stages). Stage 2 (lowered IR) and 3 (compiled code) add kinds to this store.
