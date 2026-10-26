# AGENTS.md: yavs-lib

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-lib`.

## 1. Purpose and boundaries

- Library directories on disk: an index `yavs-lib.toml` per library (files in analysis order, where each
  unit is) and a record per source file in `yavs-lib.d/` (unit names and dependencies), guarded by the
  advisory lock file `_YAVS_LIB` (rule 14), atomic updates (records first, the index last), and the on-demand
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
| `src/manifest.rs` | `MANIFEST_FILE`, `LOCK_FILE`, `RECORD_DIR`, format version `FORMAT = 3`, `Manifest` (the index: one string buffer with spans, so a parse is a few allocations and a clone a copy; it keeps the text it was read from or last wrote and where each file's entry is in it: `parse_owned`, `render` (copies the entries that did not change, debug builds also write the whole text and compare), `adopt`, `src`; lookups `units`, `find`, `file`, `files`, `records`; `push_file`, `remove_file`, `take_units` for the session), `FileRef` (path, hash, standard, `partial`, `unchanged`, `units`), `UnitLoc`, `Stamp`, `UnitRecord` (builders `new`, `architecture`, `package_body`, `depends`), `UnitKind`, `UnitRef`, `fold`, `hash_content` (sha256) |
| `src/record.rs` | The record of a file: `render`, `parse`, `digest` (the name of the record), `load` (checks the digest and the path; `L0003` for a missing or damaged record) |
| `src/text.rs` | The text format shared by the index and the records: `put_str` (escapes `"`, `\` and control characters as TOML), `Cur::string` and `Cur::plain` (the end of a string is found eight bytes at a time, `plain_len`) |
| `src/library.rs` | `Library` (`read` under a shared lock returns an `Arc<Manifest>` (the index), kept while its file is old and unchanged or, under this process's own lock, until any session of the process writes (`Fresh`, `WRITES`); `read_locked` keeps the shared lock for the closure; `update_file`, `session`), `Session::record` (one exclusive hold for a whole analyze batch; per file one record rename, one index rename), `record_deferred` + `flush` (the batch form: records per file, the index written once by `flush`, #747), lock acquisition with a waiting note, process-wide `HELD` set |
| `src/closure.rs` | `Libraries` (`add`, `add_spec` for `NAME:PATH`, `get`, `names`), `closure(libs, top) -> Vec<ClosureFile>` (dependency order, cycles, stale hashes, unknown units with suggestions; reads the records of the reachable units only, under the libraries' shared locks) |
| `src/error.rs` | `err`, `io` (code + path + OS error), `note` (`L0000`, stderr), `debug` (only with `YAVS_LOG=debug`) |
| `tests/lib.rs` | Re-analyze, cross-library order, errors, stale hash, cycles, lock waits, read-only libraries, multi-process concurrency (re-runs itself with `YAVS_LIB_TEST_CHILD`) |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| The manifest is never a cache of analysis; analysis is on demand over the closure only (the closure's analysis may be read from the compilation cache directory `yavs-cache/` of the work library, written by the CLI, not by this crate) | Keeps UNISIM-size libraries cheap; the cache lifts the earlier no-cache rule (user, 2026-10-19) | `docs/decisions.md` "Compilation cache", `docs/plan.md` architecture table |
| `_YAVS_LIB` locked with `std::fs::File::lock`/`lock_shared` (flock / LockFileEx) | Parallel `yavs analyze` into one library is safe | `docs/plan.md` rule 14 |
| `analyze` takes the exclusive lock while updating; the index is written as a temp file plus atomic rename, a record under its own name (`write_record`: nothing reads it before the index names it; one of the same length is kept, a shorter one, from a killed call, is rewritten whole) | Readers never see a half-written index; a record rename was a fiftieth of a cold call (#747) | rule 14 |
| A `Session` keeps the parsed index and its text in memory; each `record` writes the file's record and then the index (`record_deferred` leaves the index and the deletion of replaced records to `flush`, which `yavs analyze` calls once per call and after a failure: a batch of 250 files rewrote and renamed an index of 40 KB 250 times, 0.6 G kernel instructions, #747; nobody reads the library meanwhile, readers wait for the lock; a record replaced and made again within the batch is not deleted), each as a temp file in the library directory renamed into place (the platform's replace-existing rename on Windows), with NO fsync, and writes nothing that is unchanged; then it deletes the records the index no longer names | A reader or a crash sees the old or the new index, never a torn one; a power loss may lose the last analyze, which the next analyze redoes. The fsync at session end cost 3 to 5 ms of wall per `analyze`, more than the analysis (#561; the analysed-unit cache is unsynced too). Earlier: a batch of 60 files re-read, re-parsed and synced the manifest 60 times | user, 2026-10-23 (#561) |
| The library is an index plus a record per source file; a record is named by the digest of its text and never rewritten; the index names the digest. The closure holds the shared locks of its libraries while it reads records | A call reads and writes the index and its own records, not the whole library (#599): a warm `analyze` of one file into a 246-file library is 9 million instructions instead of 17; an interrupted write leaves the old index and an unnamed record; a damaged or missing record is `L0003` | user, 2026-10-23 (#599), `docs/decisions.md` "Library layout" |
| `elab`/`run` take the shared lock only while reading; `run` never writes into library dirs | A long simulation never blocks compilation | rule 14 |
| Read-only libraries never fail: lock opened read-only, or no locking at all if it cannot be opened | Sandboxed build systems | rule 14 |
| A lock wait longer than a few seconds prints a note naming the library | User feedback | rule 14 |
| Standard recorded per file (manifest format 2) | Mixed 2008/2019 projects (user, 2026-10-18) | `docs/plan.md` "Test-runner integration", `docs/design/sem.md` 0.1 |
| Pre-rename manifest name fails with `L0004` asking to re-analyze; no reader for old names | Rename to yavs (user, 2026-10-18) | `docs/decisions.md` "Project name" |
| A library of another `FORMAT` is `L0004` naming the directory, to be deleted and analyzed again; no conversion | `elab`/`run` never write a library, and the files are rebuilt by analysing them (#599) | `docs/decisions.md` "Library layout" |
| A manifest parsed under the process's own exclusive lock is current until a `Session` of the process writes (`WRITES` counts the writes of all of them, bumped before and after the index rename, #635); the session starts from that parse instead of reading the file again | `analyze` parsed the work library's manifest twice per call, and a batch rewrites it after every file so the two-second age rule never trusted a parse (#581) | #581 |
| The index and the records are written by `text.rs` and read by its reader (a subset of TOML; `toml` is a dev-dependency used by a test that checks the index is valid TOML). The index is written by splicing: the entry of a file that did not change is copied from the text last read or written (`Manifest::render`, `adopt`, #424); a change to the index that is not made through `push_file`, `remove_file` or `take_units` must clear the file's `block` | The `toml` serializer and parser were a tenth of an `analyze` of one file into a library of a few hundred units (#581); one reader and one writer replace the earlier pair of fast path and fallback | #599 |
| The index records `size`/`mtime_ns` (a `Stamp`) per file when `analyze` found the file unmodified for two seconds; `closure` skips the read and hash of a file with the same stamp | A run no longer reads and hashes every source of the closure (#426); the age rule keeps an edit within the clock granularity from being trusted | `docs/decisions.md` "Per-run overhead" |
| `Library::read` returns the index it parsed before while the file has the same stamp (still taking the lock) | The closure and the image restore both read each library (#426) | same |
| `Manifest::take_units` marks a file `partial` (optional key) when a later file took some of its units; the index then lists only the rest (the record is not rewritten), and the analysed-unit cache registers the file by parsing it, not from the record | A file registered from its remaining units alone left out the replaced ones, so re-analysing it met the other file's unit (#577) | `docs/decisions.md` "Analysed-unit cache" |
| Names folded with `fold` (ASCII lower case; extended identifiers keep case) | Rule 8 lookup keys | `docs/plan.md` rule 8 |

Module-local decisions go in this section in the same commit; library behaviour visible to external test runners
goes into `docs/cli-contract.md` section 3 (root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- Never write into a library directory outside `update_file`/`Session::record`; never write while
  holding only the shared lock. Never rewrite a record in place (its name is the digest of its text),
  and write the records before the index.
- Whoever reads records (`Manifest::records`, only `closure` and the analysed-unit cache's warm check) holds
  the library's shared lock (or the process's exclusive one) meanwhile; `Library::read` alone does not.
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

- The library layout is read by every `yavs` binary in parallel worktrees; bump `FORMAT` and
  fail clearly on the old one rather than reading both.
- Open issues (snapshot 2026-10-18, `gh issue list -R ru551n/yavs --label crate:lib`): #21 a
  `--map`ped library without a manifest must be empty, not L0001; #60 E1004 for an external AXI4 library; #61 UNISIM/vendor libraries missing;
  #94 numeric_std, fixed and float from source; #117 Synopsys (done), #118 VITAL (done), #119
  vendor primitives (M4b).
