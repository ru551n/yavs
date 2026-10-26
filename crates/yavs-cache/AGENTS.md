# AGENTS.md: yavs-cache

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-cache`.

## 1. Purpose and boundaries

- The store of the compilation cache (`docs/decisions.md` "Compilation cache", #407): a [`Key`]
  (sha256 over everything an entry depends on, built with `KeyBuilder`) maps to a byte string in
  `<library dir>/yavs-cache/<kind>-<key>.img`. Atomic writes, validation of everything read,
  pruning to the newest `MAX_ENTRIES`, `Store::clean`. The compiled code of the LLVM tier has a
  store of its own in the subdirectory `obj/` (`Store::objects_of_library`, #505).
- Does not: know VHDL, `Db`, or when to cache (`crates/yavs/src/cache.rs` decides; `yavs-sem`
  `image.rs` makes the bytes), lock anything, or ever fail a run: a write error is silent (`YAVS_LOG=debug`
  notes it), a bad read is a miss.
- Depends on: `sha2`, `twox-hash`, `memmap2`. Called by: `yavs`.

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | `build_id`, `Key`, `KeyBuilder`, `Store` (`of_library`, `objects_of_library` with the `OBJ_*` limits, `get`, `put`, `note`/`noted`: an empty `.seen` file that says a caller wanted an entry once, newest `MAX_NOTES` kept (trimmed once 64 past it), pruning on demand (`needs_sweep`: names are counted first, once per `Store` and process, then one per put: a batch of 250 puts listed a growing directory 250 times, #747; entries other processes add meanwhile are seen by the next sweep, so the entry limit is soft by their puts; a put creates the directory only when the file create fails, writes header and payload in two writes, sizes and ages only over `MAX_ENTRIES`, after a big entry, or every 5 s via `.swept`), `clean`), entry header (magic, format, key, length, payload xxh3 checksum), `Entry` (the payload of a validated entry, mapped from the file: it implements `AsRef<[u8]>`, so it becomes a `yavs_diag::Buffer` and readers keep ranges of it), tests |
| `src/map.rs` | Maps an entry file (reads it where mapping is not possible; one `fstat`, for the length; on Unix the entry's time is refreshed through the open file, not through a second open for writing, #426), `trim_heap` (`malloc_trim`), `huge_heap` (transparent huge pages on the heap at start, #556, `docs/decisions.md` "Huge pages for the heap"; reads the mode of the system with one `read`); the only `unsafe` in the crate |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Every key mixes in `FORMAT` and `build_id()` (version, hash of all sources made by `crates/yavs/build.rs`, executable size, `YAVS_CACHE_SALT`; mtime without the hash); stage keys also hold the optimiser switches (`crates/yavs/src/cache.rs` `stage_key`) | An entry never outlives the code or the switches that made it | `docs/decisions.md` "Build and switch fingerprint" (#491) |
| Entry = header + payload, verified (magic, format, key echo, exact length, xxh3 checksum) before use | A truncated or damaged file is a miss, not a wrong result; xxh3 because sha256 of a 10 MB image cost as much as reading it (#426) | same |
| Writes: temporary file in the same directory, then `rename`; no lock | Concurrent runs never see half an entry; readers need no lock | same |
| The analysed-unit store (`direct`) creates an entry under its own name (`create_new`), no temporary file and rename; a name that exists (cut short, or another run's) falls back to the rename. `payload_len` checks the header's length against the file size | A cold call makes a hundred or more small entries and a rename was a twentieth of its cycles; a reader that meets a half-written entry finds length or checksum wrong, which is a miss (#747, `docs/decisions.md` "Batched analysis") | #747 |
| `put` never returns an error to the caller | The cache can only make a run faster | same |
| Pruning by modification time, which `get` refreshes | Keep what is in use | same |
| Analysed units in `units/` (`Store::units_of_library`, `UNIT_MAX_ENTRIES` 16384, `UNIT_MAX_BYTES` 512 MiB); a read does not refresh the entry's time; `clean` removes them too | One entry per design unit of a library, a hundred read per call (#537) | `docs/decisions.md` "Analysed-unit cache" |
| Compiled code in `obj/` under limits of its own (`OBJ_MAX_ENTRIES` 4096, `OBJ_MAX_BYTES` 256 MiB); `clean` of the library's store removes it too | One small object per compiled module would otherwise push the stage entries out | `docs/decisions.md` "Compiled code in the compilation cache" (#505) |
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

- #407 (all stages). Kinds in use: `sem` (analysis image, `crates/yavs/src/cache.rs`), `pre` (the analysis of the first files of a closure, shared by closures that start alike) and `low` (lowered run-time packages, `yavs-elab` `pkgcall::package_lowerer` through `yavs_sem::StageCache`) and `ecall` (results of pure elaboration-time calls, one entry per closure, `yavs-elab` `callcache.rs`, #539) and `ir` (the lowered design with the optimised copies of the functions earlier runs called, `yavs-elab` `sim.rs` `LoweringKey`, #479) and, in the cache of the library of a prefix's last file, `lib`, `libopt` and `libinit` (the lowered library, its optimised functions and the initial state of its packages, `yavs-elab` `libstage.rs`); `Store::claim` keeps parallel runs from all making one big entry; a directory keeps at most `MAX_BYTES` (512 MiB) of entries beside `MAX_ENTRIES`. Stage 3, compiled code (#505): kind `obj` in `obj/` (`yavs-llvm` `objcache.rs`), one object file per compiled module.
