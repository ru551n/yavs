# AGENTS.md: yavs-cov

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-cov`.
Decisions are in `docs/decisions.md` "Code coverage"; the user view in `docs/coverage.md`; the CLI side in `docs/cli-contract.md` section 5.

## 1. Purpose and boundaries

- Counts toggles of `bit`/`std_ulogic` scalars of signals and ports as a kernel observer
  (`toggle::Toggles`, subscribed only with toggle coverage on): one lock and one read of the plane
  words in place per cycle (`Observer::value_changes`), last levels as two bitsets.
- Turns the coverage counters of a run (`yavs_interp::Runtime::coverage`) into a `Database`
  keyed per plan rule 10, reads and writes its text form, merges databases, and renders the
  text summary and Cobertura XML.
- Does not: emit counters (yavs-lower, `LowerOptions::coverage`), count them (yavs-interp
  `Op::Cover*`, yavs-rt compiled `Cover`, #692), lay out the counter rows (yavs-elab `Elaborated::cov_rows`), or parse options
  (the `yavs` CLI: `run --cov`, `cov merge`, `cov report`).
- Counts the states and transitions of state machines as a second kernel observer
  (`fsm::Fsm`, subscribed only with `--cov-kind=fsm`); which signals are machines is `yavs_lint::machines`.
- Depends on: `yavs-diag`, `yavs-elab`, `yavs-ir`, `yavs-kernel` (observer), `yavs-lint` (state machine detection only), `yavs-sem`.
  Called by: `yavs`.

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | `Kind`, `Key`, `Entry` (with `tests`), `Database` (`tests`, `write`, `parse`, `merge`, `attribute`, `rank`, `summary`, `only`, `totals`, `cobertura`), `tenths`/`percent` (round down), `collect` (items to keys: owning unit, statement text hash, occurrence; rows to instance paths; `arm_name` turns `CovArm` into `Entry::text` of a branch) |
| `src/toggle.rs` | `toggles` (signals and ports to named scalars, `--cov-toggle-max` cap), `Toggles` (value-change observer over `SigView::planes`, `add_to` the database as one bulk insert) |
| `src/fsm.rs` | `fsm` (the machines of every instance to `Watch`es with their regions, process and waivers, and what is not measured), `Fsm` (value-change observer), `add_to` |
| `src/exclude.rs` | `Rules` (one exclusion file: `read`), `parse` (text rows), `apply` (W0110, W0111, W0114), `Database::uncovered` and `uncovered_yaml`, `Database::hits_by_test` (`--tests`); `collect` also scans `-- coverage off/on/off-line` pragmas (W0112, W0113) |
| `src/exclude/yaml.rs` | The schemas `EXCLUSION_FILE` (`ENTRY`: the fields of each kind) and `CONFIG_FILE`, read and checked by `yavs_diag::yaml`; `parse_yaml`: the checked tree to the same `Rule`s as the text rows; `quote` for `--uncovered=yaml`. A new rule kind: add its field list to `ENTRY` and its rule in `entry` (and its text row in `parse`) |
| `src/exclusions.rs` | `build` (the `exclude::Applied` of a run to a `Report`: every rule, inline and from an exclusion file, with status applied/redundant/matches nothing/metric not collected, its matched items, totals per kind before and after) and its `text`, `json` (version 1) and `html_body` forms: `cov report --exclusions` and the HTML page |
| `src/detail.rs` | `report`: the `--detailed` view of an instance: a table per kind, focused tables, entries through the diagnostics renderer, causes (`root`), tips, the text that excludes an item (`exclude_with`). A new `Kind` needs a table, an entry and a tip here |
| `src/export.rs` | `Database::json` (versioned, every kind), `lcov`, `sonarqube` |
| `src/html.rs` | `files` (sources a report needs), `pages` (`index.html`, `src-N.html` and `src-N-missed.html` (only lines with a missed or excluded item): static, inline CSS and one inline script (click-only popups, copy buttons), light/dark; a line's items are in a popup (groups of title + table), missed items carry their `exclude::miss_row` and `miss_yaml` with copy buttons) |
| `../yavs/tests/coverage.rs` | End-to-end counts on `tests/cli/cov_run.vhd` and toggles on `tests/cli/cov_toggle.vhd`, thread identity, merge, reports |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Key = (path, kind with arm, FNV-1a of the statement text with whitespace runs as one space, occurrence of that text in its unit in source order); never offsets | Rule 10: databases of older runs stay mergeable after edits elsewhere | `docs/plan.md` rule 10 |
| Architecture and entity items per instance path (`:top:u1`), summed over blocks and generate iterations; other items per `library.unit`, summed over callers | Readable reports; package code is shared | `docs/decisions.md` "Code coverage" |
| A unit owns the statements between its name and the next unit's name in its file (`Unit::span` is the name only); the same statement lowered twice (package instances) is one item | Units do not nest | this file |
| Every item is in the database, count 0 included; counts added with `saturating_add` | Uncovered code must show | this file |
| Text format `yavs-coverage 13` (checked against `MAGIC` 2026-11-14; 12 and 13 are in the rows below; 11 has the inputs on row lines where 10 and 9 hold a count of inputs, 8 to 10 operands per waiver; 9 has no state machines; 8 has no waivers and input counts on condition and expression lines; 7 has no operand text in its waivers; 6 lacks the unit and process fields that end each line, `Database::no_units`; was 4; see the combination row below): `test` lines, then items tab-separated (count, then the ids of the tests that hit it; the last field is the display text; toggle lines have none), sorted by key; test ids in name order; versions 2 and 3 still parsed; Cobertura with timestamp 0 and no toggles | Deterministic files at any `--threads` | `docs/cli-contract.md` 5 |
| Toggles: `Kind::Toggle(rise)` with `Key::sig` the scalar name, hash and occ 0; '0'/'L' and '1'/'H' levels, anything else breaks a toggle; the observer compares with the last level, so repeated notifications count nothing twice | Same counts at any `--threads` | `docs/decisions.md` "Toggle coverage" |

| Condition and expression rows: `Kind::Condition(row)`/`Expression(row)` (row = 2 * input + value), hash and occurrence of the *condition's* text (not the statement's), `Key::sig` the input's text, `Entry::text` = result char, space, whole condition; database lines `cN`/`eN` with the input before the text (version 5, 2 to 4 still read); `fec_text` formats a row for reports; Cobertura counts the rows as conditions of the line; inputs constant after elaboration have no rows (row numbers keep the input position, so a row number can skip) | An edit of an `if` body must keep its row exclusions | `docs/decisions.md` "Condition and expression coverage" |

| A `-- coverage off` region tests a branch arm against `CovInfo::pos` (condition, choices, or the first statement of the body of `others`/`else`), not its statement's line; no `pos` (`(no arm taken)`, loop body) keeps the statement's line | One `case` alternative or `elsif` must be excludable alone (#1042) | `docs/decisions.md` "Coverage exclusions" |

| Exclusions match by instance glob + kind/arm + hash prefix + occurrence, never by line; `files`/`files_exclude` patterns go through `yavs_diag::files`, relative to the exclusion file (`Rules::base`); pragma-excluded items are skipped when the key is added, after occurrences are numbered, so a pragma never renumbers its neighbours | Rows stay valid when code moves or pragmas change | `docs/decisions.md` "Coverage exclusions" |

| HTML report: static files, one small inline script, no external resources, no timestamp; a line shows the largest statement count on it, its items in a hover/focus popup (click pins), a line that ran in only some instances has its own class and a `ran/total` marker; sources read by the CLI (`E0113` if unreadable) and shown relative to the current directory or `--root` (the CLI rewrites `Entry::file` with `cov.rs::relative`, a lexical string compare that folds `\`, drive case, `\\?\` and `..`, unit-tested for Windows paths on every OS; the crate only formats) | Opens from disk, reproducible, the report rows are the exclusion format (user 2026-10-24, #689) | `docs/decisions.md` "Code coverage" |

| YAML exclusion and configuration files are checked against a schema by `yavs_diag::yaml` (raw scalar text, never a typed data model): unknown kinds, unknown keys (also of another kind), wrong types and missing fields are all listed in one `E0112` (`E0138` for the configuration file) with their lines, a duplicate key stops at the first; `--uncovered=yaml` double-quotes every value | `no`, `on` and digit-only or float-like hashes must stay strings; line numbers in errors | `docs/decisions.md` "Coverage exclusions" |
| A toggle exclusion has an optional direction (`rise`/`fall`); omitted is both; `W0114` per excluded direction that ran | Exclude exactly the missed direction | `docs/decisions.md` "Coverage exclusions" |

| A counter is `Cover { item }` plus the `cov_base` of the instance that runs it, never part of code or of a copy's key; per-instance copies, memoised blocks and inlined functions keep their counters, a copy that folds an arm away drops that arm's counters (count 0 for that instance) | Specialisation, memoisation and inlining stay on under `--cov`; counts are byte-identical to the shared code's | `docs/decisions.md` "Code coverage" (#699) |

| The toggle observer reads each cycle's changed ranges of a signal as their hull and compares plane words with the last levels (`prev0`/`prev1` bitsets, 'neither' is both clear); a rise is 'now 1 and last 0', a fall 'now 0 and last 1' | An element that did not change holds its last level, so the hull counts what the ranges would; a range costs word operations and one add per toggled bit (was a lock, an allocation and a loop over elements per range) | `docs/decisions.md` "Toggle observer cost" (#707) |
| Count-neutral skips stay on under `--cov` (user, 2026-10-24, #707): `skip_unchanged` (an unchanged assignment queues no transaction) for every kind; the idle skip only with toggle coverage alone, off when statement, branch, condition or expression counters are on (it leaves activations out). Lean wide-signal events stay off. A new skip must keep the databases byte-identical (`cmp` over `tests/cli/cov_*.vhd` and the realbench designs at 1 and 4 threads, interp/clift/auto) | Replaces the earlier rule "coverage observes activity"; neorv32 30 us toggle alone 10.0 G to 6.7 G | `docs/decisions.md` "Coverage keeps the count-neutral skips", `perf.rs` `skipping_follows_observers`, `idle_skip_changes_nothing` |
| Edge triggers stay on under `--cov`: the statement of a pure edge test counts one run at initialisation and one per edge, its 'no condition held' arm the initialisation run; the statements inside the `if` are unchanged. Plain signal copies and clock generators the kernel runs itself stay interpreted when the program has counters | The user accepted the changed counts of the edge test line (user, 2026-10-24) for most of the `--cov` overhead; a kernel-run copy would count 0 | `docs/decisions.md` "Edge triggers stay on under coverage", `docs/coverage.md` (#707) |

| A branch's `Entry::text` is its arm named from the source (`if C`, `elsif C`, `else`, `when CHOICES`, `(no arm taken)`, `loop body`), built from `CovInfo::arm`; the key is unchanged. Percentages and `--total` round down; the toggle words are `arrow()` (`rise (0->1)`) | Display only, so databases and exclusions stay compatible; a gate never rounds up | `docs/decisions.md` "Readable coverage report" (#877) |

| Combination items: `Kind::Combination(state)` (two bits per input: 0 not evaluated, 1 false, 2 true), hash and occurrence of the decision's text, `Key::sig` the input texts joined by `INPUT_SEP`, `Entry::text` the decision, `Entry::waivers` the in-source directives that waive the requirement (rule as written, reason, file, line; the count stays raw); database lines `mSTATE` with the waivers last (version 6, 2 to 5 still read and marked `Database::legacy`); `merge` returns the conflicts (a waiver on one side only, the item then merged without it) and the CLI makes them `E0137`; `exclude::apply` takes waived items out like exclusion rows, condition and expression rows with a `row` directive too (`Entry::waivers`, `Entry::operands`; `exclude::directive` writes the comment for a report), and lists every rule in `Applied::uses`; a `combination` exclusion row has a `0`/`1`/`-` pattern that can match several requirements; the statement hash ignores `--` comments | Comment and directive edits must not change identity; a waiver from one run must not hide a requirement of another; overlapping rules count a requirement once | `docs/decisions.md` "Combination coverage and inline exclusions" (#883) |

| A condition row of a `wait until` or `assert` hashes `wait until TEXT` or `assert TEXT`, not `TEXT` | Occurrences are numbered per kind and hash over what was lowered, so a shared hash would shift an `if` of the same text when `--cov-kind=assert` is on, and a merge would add its counts to the wrong row | `docs/decisions.md` "Conditions of `wait until` and `assert`" |
| The default report has tables of kinds and instances, no items; `--detailed` adds them (`detail::report`); `--uncovered`, `--tests` and the HTML report stay one row per item with every detail. A tip says only what the data supports; printed exclusion text must work when pasted (`printed_exclusions_work_when_pasted`). `W0114` is one warning per rule, paths through `--root` | A report of thousands of toggle rows hides the few statements a reader must act on | `docs/decisions.md` "The coverage text report" (#1020, user 2026-11-01) |

| Database version 13: state machine lines also hold the process and the waivers, 12 still read | `exclude state` and `exclude transition` directives and `process` rows reach these items | `docs/decisions.md` "State machine exclusions in the source" |
| Database version 12: `source`, path, text lines (`Database::sources`, `--cov-embed-sources`) and `at` lines after an item (`Entry::detail`), 2 to 11 still read; `only` and `exclude::apply` keep the sources | The HTML report needs no checkout, opt-in as the text travels with the file | `docs/decisions.md` "Embedded sources in the coverage database" (#1020) |
| State machine items: `Kind::Fsm(transition)`, `Key::sig` = `SIGNAL:STATE` or `SIGNAL:FROM->TO`, hash and occ 0, per scope path; database lines `f0`/`f1` (the toggle fields) and `unmeasured`; version 10, older ones still read; no Cobertura | Names survive edits; a value-change observer counts alike in every tier | `docs/decisions.md` "State machine coverage" |

Module-local decisions go in this section in the same commit (root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- A new optimiser pass must keep every `Cover`, `CoverIn` and `CoverCond` of code that can still run
  and must not read or write `cov_base` (it is the running instance's); `copies_count_alike` in
  `crates/yavs/tests/coverage.rs` compares a run with copies, memo and inlining against shared code.
- Counts of a clocked process's edge test depend on its edge trigger (`edge_triggers_count_their_edges` pins every form): a new trigger shape in `yavs-elab` changes them, in the open.
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
