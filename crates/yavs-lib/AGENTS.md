# AGENTS.md: yavs-lib

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-lib`.

## 1. Purpose and boundaries

- Library directories on disk: a TOML manifest `yavs-lib.toml` per library, guarded by the
  advisory lock file `_YAVS_LIB` (rule 14), atomic manifest updates, and the on-demand
  **closure query** that turns a top unit into an ordered list of source files (with the standard
  of each file) for `elab`/`run`.
- Does not: parse or analyze VHDL (the analyzer hands in `UnitRecord`s: kind, name, dependencies,
  standard), cache compiled units (there is no incremental compilation cache, by plan), or know
  about `std`/`ieee` (those come from `lib/` through `yavs-sem`).
- Depends on: `yavs-diag`, `serde`, `toml`, `sha2`. Called by: `yavs-sem` (`Libraries` in
  `Options`), `yavs-elab`, `yavs` (analyze records files; elab computes the closure).

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | Re-exports; `Result<T>` = boxed span-less `Diagnostic`; `render` helper |
| `src/manifest.rs` | `MANIFEST_FILE`, `LOCK_FILE`, format version `FORMAT = 2` (standard per file), `Manifest` (parse, `to_text`, `units`, `find`, `file`), `FileEntry`, `UnitRecord` (builders `new`, `architecture`, `package_body`, `depends`), `UnitKind`, `UnitRef`, `fold`, `hash_content` (sha256) |
| `src/library.rs` | `Library` (`read` under a shared lock, `update_file`, `session`), `Session::record` (one exclusive hold for a whole analyze batch, one atomic rename per file), lock acquisition with a waiting note, process-wide `HELD` set |
| `src/closure.rs` | `Libraries` (`add`, `add_spec` for `NAME:PATH`, `get`, `names`), `closure(libs, top) -> Vec<ClosureFile>` (dependency order, cycles, stale hashes, unknown units with suggestions) |
| `src/error.rs` | `err`, `io` (code + path + OS error), `note` (`L0000`, stderr), `debug` (only with `YAVS_LOG=debug`) |
| `tests/lib.rs` | Re-analyze, cross-library order, errors, stale hash, cycles, lock waits, read-only libraries, multi-process concurrency (re-runs itself with `YAVS_LIB_TEST_CHILD`) |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| No incremental compilation cache; `elab`/`run` re-parse and re-analyze from source; analysis is on demand over the closure only | Keeps UNISIM-size libraries cheap without a cache | `docs/plan.md` "Decisions", architecture table `yavs-lib` |
| `_YAVS_LIB` locked with `std::fs::File::lock`/`lock_shared` (flock / LockFileEx) | Parallel `yavs analyze` into one library is safe | `docs/plan.md` rule 14 |
| `analyze` takes the exclusive lock while updating; each write is temp file plus atomic rename | Readers never see a half-written manifest | rule 14 |
| `elab`/`run` take the shared lock only while reading; `run` never writes into library dirs | A long simulation never blocks compilation (NVC #507) | rule 14 |
| Read-only libraries never fail: lock opened read-only, or no locking at all if it cannot be opened | Bazel sandboxes (NVC #651) | rule 14 |
| A lock wait longer than a few seconds prints a note naming the library | User feedback | rule 14 |
| Standard recorded per file (manifest format 2) | Mixed 2008/2019 projects (user, 2026-10-18) | `docs/plan.md` "VUnit integration", `docs/design/sem.md` 0.1 |
| Pre-rename manifest name fails with `L0004` asking to re-analyze; no reader for old names | Rename to yavs (user, 2026-10-18) | `docs/decisions.md` "Project name" |
| Names folded with `fold` (ASCII lower case; extended identifiers keep case) | Rule 8 lookup keys | `docs/plan.md` rule 8 |

Module-local decisions go in this section in the same commit; library behaviour visible to VUnit
goes into `docs/cli-contract.md` section 3 (root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- Never write into a library directory outside `update_file`/`Session::record`; never write while
  holding only the shared lock.
- `HELD` lets a process read a library it holds exclusively without deadlocking on its own lock.
  Keep it in sync if you add another lock path.
- Every error names what was missing and where yavs looked (paths in notes/help). Never return a
  default manifest for an unreadable one.
- The closure is ordered and deterministic (dependencies first); `tests/cli.rs` and the difftest
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
  `--map`ped library without a manifest must be empty, not L0001; #60 E1004 for OSVVM AXI4; #61 UNISIM/vendor libraries missing; #37
  fixed/float generic package bodies; #94 numeric_std, fixed and float from source; #117 to #119
  Synopsys, VITAL, vendor primitives (M4b).
