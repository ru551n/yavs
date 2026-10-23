# AGENTS.md: yavs-lib

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-lib`.

## 1. Purpose and boundaries

- Library directories on disk: a TOML manifest `yavs-lib.toml` per library, guarded by the
  advisory lock file `_YAVS_LIB` (rule 14), atomic manifest updates, and the on-demand
  **closure query** that turns a top unit into an ordered list of source files (with the standard
  of each file) for `elab`/`run`.
- Does not: parse or analyze VHDL (the analyzer hands in `UnitRecord`s: kind, name, dependencies,
  standard), store compiled units (the compilation cache is `yavs-cache` plus `yavs-sem` `image.rs`), or know
  about `std`/`ieee` (those come from `lib/` through `yavs-sem`).
- Depends on: `yavs-diag`, `serde`, `toml`, `sha2`. Called by: `yavs-sem` (`Libraries` in
  `Options`), `yavs-elab`, `yavs` (analyze records files; elab computes the closure).

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | Re-exports; `Result<T>` = boxed span-less `Diagnostic`; `render` helper |
| `src/manifest.rs` | `MANIFEST_FILE`, `LOCK_FILE`, format version `FORMAT = 2` (standard per file), `Manifest` (parse with a fast reader for the written text and `toml` for the rest, `to_text` written directly (`write_text`, byte-identical to the `toml` text; strings that need an escape go to `toml`), `units`, `find`, `file`), `FileEntry` (with the optional `size`/`mtime_ns` stamp and `unchanged`), `Stamp`, `UnitRecord` (builders `new`, `architecture`, `package_body`, `depends`), `UnitKind`, `UnitRef`, `fold`, `hash_content` (sha256) |
| `src/library.rs` | `Library` (`read` under a shared lock returns an `Arc<Manifest>`, kept while its file is old and unchanged or, under this process's own lock, until any session of the process writes (`Fresh`, `WRITES`); `update_file`, `session`), `Session::record` (one exclusive hold for a whole analyze batch, one atomic rename per file), lock acquisition with a waiting note, process-wide `HELD` set |
| `src/closure.rs` | `Libraries` (`add`, `add_spec` for `NAME:PATH`, `get`, `names`), `closure(libs, top) -> Vec<ClosureFile>` (dependency order, cycles, stale hashes, unknown units with suggestions) |
| `src/error.rs` | `err`, `io` (code + path + OS error), `note` (`L0000`, stderr), `debug` (only with `YAVS_LOG=debug`) |
| `tests/lib.rs` | Re-analyze, cross-library order, errors, stale hash, cycles, lock waits, read-only libraries, multi-process concurrency (re-runs itself with `YAVS_LIB_TEST_CHILD`) |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| The manifest is never a cache of analysis; analysis is on demand over the closure only (the closure's analysis may be read from the compilation cache directory `yavs-cache/` of the work library, written by the CLI, not by this crate) | Keeps UNISIM-size libraries cheap; the cache lifts the earlier no-cache rule (user, 2026-10-19) | `docs/decisions.md` "Compilation cache", `docs/plan.md` architecture table |
| `_YAVS_LIB` locked with `std::fs::File::lock`/`lock_shared` (flock / LockFileEx) | Parallel `yavs analyze` into one library is safe | `docs/plan.md` rule 14 |
| `analyze` takes the exclusive lock while updating; each write is temp file plus atomic rename | Readers never see a half-written manifest | rule 14 |
| A `Session` keeps the parsed manifest and the text of the file in memory; each `record` writes a temp file in the library directory and renames it over the manifest (the platform's replace-existing rename on Windows), with NO fsync, and skips the write when the text is unchanged | A reader or a crash sees the old or the new manifest, never a torn one; a power loss may lose the last analyze, which the next analyze redoes. The fsync at session end cost 3 to 5 ms of wall per `analyze`, more than the analysis (#561; the analysed-unit cache is unsynced too). Earlier: a batch of 60 files re-read, re-parsed and synced the manifest 60 times | user, 2026-10-23 (#561) |
| `elab`/`run` take the shared lock only while reading; `run` never writes into library dirs | A long simulation never blocks compilation | rule 14 |
| Read-only libraries never fail: lock opened read-only, or no locking at all if it cannot be opened | Sandboxed build systems | rule 14 |
| A lock wait longer than a few seconds prints a note naming the library | User feedback | rule 14 |
| Standard recorded per file (manifest format 2) | Mixed 2008/2019 projects (user, 2026-10-18) | `docs/plan.md` "Test-runner integration", `docs/design/sem.md` 0.1 |
| Pre-rename manifest name fails with `L0004` asking to re-analyze; no reader for old names | Rename to yavs (user, 2026-10-18) | `docs/decisions.md` "Project name" |
| A manifest parsed under the process's own exclusive lock is current until a `Session` of the process writes (`WRITES` counts the writes of all of them); the session starts from that parse instead of reading the file again | `analyze` parsed the work library's manifest twice per call, and a batch rewrites it after every file so the two-second age rule never trusted a parse (#581) | #581 |
| `Manifest::to_text` writes the text itself and falls back to `toml` for any string with a quote, backslash, control character or non-ASCII byte; the test compares both texts | The `toml` serializer was a tenth of an `analyze` of one file into a library of a few hundred units | #581 |
| `FileEntry` records `size`/`mtime_ns` (a `Stamp`) when `analyze` found the file unmodified for two seconds; `closure` skips the read and hash of a file with the same stamp | A run no longer reads and hashes every source of the closure (#426); the age rule keeps an edit within the clock granularity from being trusted | `docs/decisions.md` "Per-run overhead" |
| `Library::read` returns the manifest it parsed before while the file has the same stamp (still taking the lock) | The closure and the image restore both read each library (#426) | same |
| `Manifest::parse` first tries `parse_written`, a reader for exactly the text `to_text` writes, and falls back to `toml` | The `toml` crate took 5 ms per run for a testbench library (#426); the fallback keeps every other input, error and diagnostic as before | same |
| `replace_file` marks a file `partial` (optional key, format 2) when a later file took some of its units; the record then lists only the rest, and the analysed-unit cache registers the file by parsing it, not from the record | A file registered from its remaining units alone left out the replaced ones, so re-analysing it met the other file's unit (#577) | `docs/decisions.md` "Analysed-unit cache" |
| Names folded with `fold` (ASCII lower case; extended identifiers keep case) | Rule 8 lookup keys | `docs/plan.md` rule 8 |

Module-local decisions go in this section in the same commit; library behaviour visible to external test runners
goes into `docs/cli-contract.md` section 3 (root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- Never write into a library directory outside `update_file`/`Session::record`; never write while
  holding only the shared lock.
- `HELD` lets a process read a library it holds exclusively without deadlocking on its own lock.
  Keep it in sync if you add another lock path.
- Every error names what was missing and where yavs looked (paths in notes/help). Never return a
  default manifest for an unreadable one.
- The closure is ordered and deterministic (dependencies first; a package body in a file of its own follows its package's file as soon as everything it needs is placed, so that closures that share the package's files share the body's place, #407); `tests/cli.rs` and the difftest
  depend on it.
- Tests use `tempfile` directories; the concurrency test spawns the test binary itself. Do not add
  fixed scratch paths.

## 5. How to work here

```sh
CARGO_BUILD_JOBS=6 cargo test -p yavs-lib
cargo test -p yavs --test cli                    # library locking and batches through the CLI
YAVS_LOG=debug cargo run -p yavs -- elab ...     # show lock-free reads
```

- Codes owned: `L0000`-`L0099` (L0000 note, L0001 library not found, L0002 I/O, L0003 bad
  manifest, L0004 old manifest name, L0010 create dir, L0011 lock, L0020 bad mapping, L0030
  unknown library, L0031 unknown unit, L0032 cycle, L0033 source unreadable, L0034 source changed
  since analysis). Add new ones here and to the registry table in `crates/yavs-diag/src/lib.rs`.

## 6. Coordination

- The manifest format is read by every `yavs` binary in parallel worktrees; bump `FORMAT` and
  fail clearly on the old one rather than reading both.
- Open issues (snapshot 2026-10-18, `gh issue list -R ru551n/yavs --label crate:lib`): #21 a
  `--map`ped library without a manifest must be empty, not L0001; #60 E1004 for an external AXI4 library; #61 UNISIM/vendor libraries missing;
  #94 numeric_std, fixed and float from source; #117 Synopsys (done), #118 VITAL (done), #119
  vendor primitives (M4b).
