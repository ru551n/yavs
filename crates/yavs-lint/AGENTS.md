# AGENTS.md: yavs-lint

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-lint`.

## 1. Purpose and boundaries

- The rules of `yavs lint` over the analysed HIR, the classes they belong to, the configuration and
  waiver files, and the report files. Users' view: `docs/lint.rst`; decisions: `docs/decisions.md`
  "Static analysis"; command line: `crates/yavs/src/lint.rs`, `docs/cli-contract.md` 5a.
- Does not: parse or analyze (`yavs-sem`), choose the files, render console output (`yavs-diag`
  through the CLI), run during `analyze`, `elab` or `run`.
- Depends on: `yavs-diag`, `yavs-sem`, `yavs-syntax` (lint_005), `saphyr-parser`, `serde_json`. Called by: `yavs`.
- The rules and their test cases come from the owner's project of the same licence, MIT OR
  Apache-2.0 (licence note; copied once, not kept in sync, maintainer 2026-10-30).

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | `Certainty`, `Rule`, `rule`, `Finding`, `lint` (configuration applied, order, dedup; facts) |
| `src/rules.rs` | `RULES`, the one list; `check` and `Walk`: the regions and statements of a unit, the definite rules and lint_712, 713, 760; lint_005 |
| `src/design.rs` | One architecture as a design, flattened to `Conc`: lint_001, 002, 004, 600, 601, 602, 603, 700, 701, 702, 703, 710, 711, 720, 730; `shape` says what a process is; the design facts; `component` (lint_750) |
| `src/config.rs` | YAML to `Config` (`rule:`, levels global, group, rule) and `Waivers`, `find_up` |
| `src/report.rs` | `Reports`: JSON, SARIF, JUnit, Code Climate, SonarQube |

## 3. Design decisions that bind this module

| Decision | Why |
|---|---|
| Rule ids `lint_NNN`, their own namespace; a rule is added to `RULES` and to `docs/lint-rules.rst` (generated, `YAVS_BLESS=1 cargo test -p yavs --bin yavs cli_reference`) | One list, so `--explain`, the page and the configuration agree |
| A rule ships only if it reports nothing false on `tools/corpus` | A report that needs no second reading is the value of the tool |
| A rule judges only what analysis folded; statically turned off code is not walked | No guessing about generics |
| Predefined operators only for lint_741 and lint_781 | Overloads define their own behaviour |
| Generic instances are not walked: their spans are the generic's | One finding per place |

## 4. Invariants and pitfalls

- A finding outside the files of the given units is dropped by `check`: a walk of a package body
  also walks its package's region, where a subprogram with a spec is declared.
- A spec's declaration holds the body; the body's own declaration is not in any region. Name a
  place with `body_place`.
- The HIR never has poison here: a file that failed analysis has no units in `lint`'s list.

## 5. How to work here

```sh
cargo test -p yavs-lint                  # configuration and waiver files
cargo test -p yavs --test lint           # every rule on tests/lint/*.vhd, the command line
```

A new rule: add it to `RULES`, its test in `Walk`, an `-- expect:` regression file or line in
`crates/yavs/tests/lint/`, regenerate the page, run it over the corpora.
