# AGENTS.md: yavs-cov

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-cov`.
Decisions are in `docs/decisions.md` "Code coverage"; the user view in `docs/usage.rst` "Code
coverage"; the CLI side in `docs/cli-contract.md` section 5.

## 1. Purpose and boundaries

- Counts toggles of `bit`/`std_ulogic` scalars of signals and ports as a kernel observer
  (`toggle::Toggles`, subscribed only with toggle coverage on).
- Turns the coverage counters of a run (`yavs_interp::Runtime::coverage`) into a `Database`
  keyed per plan rule 10, reads and writes its text form, merges databases, and renders the
  text summary and Cobertura XML.
- Does not: emit counters (yavs-lower, `LowerOptions::coverage`), count them (yavs-interp
  `Inst::Cover`), lay out the counter rows (yavs-elab `Elaborated::cov_rows`), or parse options
  (the `yavs` CLI: `run --cov`, `cov merge`, `cov report`).
- Depends on: `yavs-diag`, `yavs-elab`, `yavs-ir`, `yavs-kernel` (observer), `yavs-sem`.
  Called by: `yavs`.

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | `Kind`, `Key`, `Entry` (with `tests`), `Database` (`tests`, `write`, `parse`, `merge`, `attribute`, `rank`, `summary`, `cobertura`), `collect` (items to keys: owning unit, statement text hash, occurrence; rows to instance paths) |
| `src/toggle.rs` | `toggles` (signals and ports to named scalars, `--cov-toggle-max` cap), `Toggles` (value-change observer, `add_to` the database) |
| `src/exclude.rs` | Exclusion file `parse`, `apply` (W0110, W0111, W0114), `Database::uncovered`, `Database::hits_by_test` (`--tests`); `collect` also scans `-- coverage off/on/off-line` pragmas (W0112, W0113) |
| `src/html.rs` | `files` (sources a report needs), `pages` (`index.html`, `src-N.html`: static, inline CSS, light/dark, no scripts, missed items carry their `exclude::miss_row`) |
| `../yavs/tests/coverage.rs` | End-to-end counts on `tests/cli/cov_run.vhd` and toggles on `tests/cli/cov_toggle.vhd`, thread identity, merge, reports |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Key = (path, kind with arm, FNV-1a of the statement text with whitespace runs as one space, occurrence of that text in its unit in source order); never offsets | Rule 10: databases of older runs stay mergeable after edits elsewhere | `docs/plan.md` rule 10 |
| Architecture and entity items per instance path (`:top:u1`), summed over blocks and generate iterations; other items per `library.unit`, summed over callers | Readable reports; package code is shared | `docs/decisions.md` "Code coverage" |
| A unit owns the statements between its name and the next unit's name in its file (`Unit::span` is the name only); the same statement lowered twice (package instances) is one item | Units do not nest | this file |
| Every item is in the database, count 0 included; counts added with `saturating_add` | Uncovered code must show | this file |
| Text format `yavs-coverage 4`: `test` lines, then items tab-separated (count, then the ids of the tests that hit it; the last field is the display text; toggle lines have none), sorted by key; test ids in name order; versions 2 and 3 still parsed; Cobertura with timestamp 0 and no toggles | Deterministic files at any `--threads` | `docs/cli-contract.md` 5 |
| Toggles: `Kind::Toggle(rise)` with `Key::sig` the scalar name, hash and occ 0; '0'/'L' and '1'/'H' levels, anything else breaks a toggle; the observer compares with the last level, so repeated notifications count nothing twice | Same counts at any `--threads` | `docs/decisions.md` "Toggle coverage" |

| Exclusions match by instance glob + kind/arm + hash prefix + occurrence, never by line; pragma-excluded items are skipped when the key is added, after occurrences are numbered, so a pragma never renumbers its neighbours | Rows stay valid when code moves or pragmas change | `docs/decisions.md` "Coverage exclusions" |

| HTML report: static files, no scripts and no external resources, no timestamp; a line shows the largest statement count on it, items listed under it; sources read by the CLI (`E0113` if unreadable), the crate only formats | Opens from disk, reproducible, the report rows are the exclusion format (user) | `docs/decisions.md` "Code coverage" |

Module-local decisions go in this section in the same commit (root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- `counts` must come from a design lowered with coverage on; its length is
  `cov_rows() * program.cov.len()` and `collect` walks it in rows of `program.cov.len()`.
- A new branch kind in lowering needs no change here as long as it is `CovKind::Branch(arm)`;
  a new `CovKind` variant needs a `Kind` and a letter in the text format.
- A new `Kind` needs its arm in `write`/`parse`, `summary`, `cobertura`, `exclude::head`/`matches`.
- `html.rs` writes no timestamp and iterates only ordered maps: keep it byte-deterministic.
  Every source text goes through `esc`; a new field shown in a page must too.

## 5. How to work here

```sh
CARGO_BUILD_JOBS=2 cargo test -p yavs-cov
CARGO_BUILD_JOBS=2 cargo test -p yavs --test coverage
```
