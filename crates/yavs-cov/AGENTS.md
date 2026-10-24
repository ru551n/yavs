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
| `src/exclude.rs` | Exclusion file `parse` (text rows), `apply` (W0110, W0111, W0114), `Database::uncovered` and `uncovered_yaml`, `Database::hits_by_test` (`--tests`); `collect` also scans `-- coverage off/on/off-line` pragmas (W0112, W0113) |
| `src/exclude/yaml.rs` | `parse_yaml`: `saphyr-parser` events to a small node tree to the same `Rule`s as the text rows; `quote` for `--uncovered=yaml`. A new rule kind: add its key lists and rule in `entry` (and its text row in `parse`) |
| `src/html.rs` | `files` (sources a report needs), `pages` (`index.html`, `src-N.html` and `src-N-missed.html` (only lines with a missed or excluded item): static, inline CSS and one inline script (pin, copy buttons), light/dark; a line's items are in a popup (groups of title + table), missed items carry their `exclude::miss_row` and `miss_yaml` with copy buttons) |
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

| HTML report: static files, one small inline script, no external resources, no timestamp; a line shows the largest statement count on it, its items in a hover/focus popup (click pins), a line that ran in only some instances has its own class and a `ran/total` marker; sources read by the CLI (`E0113` if unreadable) and shown relative to the current directory or `--root` (the CLI rewrites `Entry::file`, the crate only formats) | Opens from disk, reproducible, the report rows are the exclusion format (user 2026-10-24, #689) | `docs/decisions.md` "Code coverage" |

| YAML exclusion files read scalars as raw text through `saphyr-parser` events, never through a typed data model; unknown kinds, unknown keys (also of another kind), missing fields and duplicate keys are `E0112` with the line; `--uncovered=yaml` double-quotes every value | `no`, `on` and digit-only or float-like hashes must stay strings; line numbers in errors | `docs/decisions.md` "Coverage exclusions" |
| A toggle exclusion has an optional direction (`rise`/`fall`); omitted is both; `W0114` per excluded direction that ran | Exclude exactly the missed direction | `docs/decisions.md` "Coverage exclusions" |

Module-local decisions go in this section in the same commit (root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- `counts` must come from a design lowered with coverage on; its length is
  `cov_rows() * program.cov.len()` and `collect` walks it in rows of `program.cov.len()`.
- A new branch kind in lowering needs no change here as long as it is `CovKind::Branch(arm)`;
  a new `CovKind` variant needs a `Kind` and a letter in the text format.
- A new `Kind` needs its arm in `write`/`parse`, `summary`, `cobertura`, `exclude::head`/`matches`.
- `html.rs` writes no timestamp and iterates only ordered maps: keep it byte-deterministic.
  Every source text goes through `esc`; a new field shown in a page must too.
- A new item kind adds a group to `popup` (title + table), never inline rows; the script must
  stay optional (hover and focus are CSS), with the copy buttons hidden until it runs.

## 5. How to work here

```sh
CARGO_BUILD_JOBS=2 cargo test -p yavs-cov
CARGO_BUILD_JOBS=2 cargo test -p yavs --test coverage
```
