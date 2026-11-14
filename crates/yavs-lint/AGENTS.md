# AGENTS.md: yavs-lint

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-lint`.

## 1. Purpose and boundaries

- The rules of `yavs lint` over the analysed HIR, the classes they belong to, the configuration and
  waiver files, and the report files. Users' view: `docs/lint.rst`; decisions: `docs/decisions.md`
  "Static analysis"; command line: `crates/yavs/src/lint.rs`, `docs/cli-contract.md` 5a.
- Does not: parse or analyze (`yavs-sem`), choose the files (it expands the entries of `file_list`, the CLI decides what is read), render console output (`yavs-diag`
  through the CLI), run a rule during `analyze`, `elab` or `run`. `machines` (detection only) runs
  for `yavs-cov` under `--cov-kind=fsm`.
- Depends on: `yavs-diag`, `yavs-sem`, `yavs-syntax` (lint_005), `regex` (`re:` in lint_602, lint_603), `serde_json`; the YAML reader and schema check are `yavs_diag::yaml`. Called by: `yavs`, `yavs-cov`, `yavs-lower` (`machines`, for `--cov-kind=fsm`).
- The rules and their test cases come from the owner's project of the same licence, MIT OR
  Apache-2.0 (licence note; copied once, not kept in sync, maintainer 2026-10-30).

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | `Certainty`, `Rule`, `rule`, `Finding`, `lint` (configuration applied, order, dedup; facts) |
| `src/rules.rs` | `RULES`, the one list; `check` and `Walk`: the regions and statements of a unit, the definite rules and lint_712, 713, 760; lint_005 |
| `src/design.rs` | One architecture as a design, flattened to `Conc`: lint_001, 002, 004, 600, 601, 602, 603, 700, 701, 702, 703, 720, 730 (710, 711 in `design/fsm.rs`); `shape` says what a process is; `Flow`: the dependencies of a process for lint_720; the design facts; `component` (lint_750) |
| `src/design/fsm.rs` | `machines`: the state machines of an architecture, the one reading of `lint_710`, `lint_711`, the design facts and `--cov-kind=fsm`; `NotMeasured` for `W0120` |
| `src/config.rs` | The schemas `CONFIG` and `WAIVERS` (every key, type and nesting of the two files; docs/lint.rst has the same tables), and the checked tree to `Config` (`rule:`, levels global, group, rule; `rtl:`, `testbench:`, `Kind`; `file_list`, `expand`, `near_entry`) and `Waivers`, `find_up` |
| `src/testbench.rs` | Which kind a file is, only when a block for one exists |
| `src/report.rs` | `Reports`: JSON, SARIF, JUnit, Code Climate, SonarQube |

## 3. Design decisions that bind this module

| Decision | Why |
|---|---|
| Rule ids `lint_NNN`, their own namespace; a rule is added to `RULES` and to `docs/lint-rules.rst` (generated, `YAVS_BLESS=1 cargo test -p yavs --bin yavs cli_reference`) | One list, so `--explain`, the page and the configuration agree |
| A rule ships only if it reports nothing false on `tools/corpus` | A report that needs no second reading is the value of the tool |
| A rule judges only what analysis folded; statically turned off code is not walked | No guessing about generics |
| Predefined operators only for lint_741 and lint_781 | Overloads define their own behaviour |
| Generic instances are not walked: their spans are the generic's | One finding per place |
| One state machine detection (`design/fsm.rs`) for the rules, the facts and coverage; the rules also need `Machine::readable` | Maintainer, 2026-11-01; `docs/decisions.md` "State machine coverage" |
| A new key is added to the schema (`CONFIG`, `WAIVERS`) and to the tables of `docs/lint.rst` (a test checks that); every unknown key, rule and rule group, and every wrong type, fails the file as E0134, all listed in one error, with no list of what they might be. decisions.md "Static analysis" | A static analysis tool, not a style guide or formatter; a misspelled waiver key would widen it (maintainer, 2026-11-01) |
| `file_list` is relative to the working directory, `exclude` and waivers to their file; decisions.md "Static analysis" | A real configuration relies on it |
| `exclude` and a waiver's `files` share `yavs_diag::files::Place::covered_by`, coverage exclusion files too; waivers also keep matching the reported path and a bare name | Directories mean the same everywhere; existing waiver files keep working |

## 4. Invariants and pitfalls

- A finding outside the files of the given units is dropped by `check`: a walk of a package body
  also walks its package's region, where a subprogram with a spec is declared.
- A spec's declaration holds the body; the body's own declaration is not in any region. Name a
  place with `body_name` (the span of the body's own name).
- The HIR never has poison here: a file that failed analysis has no units in `lint`'s list.

## 5. How to work here

```sh
cargo test -p yavs-lint                  # configuration and waiver files
cargo test -p yavs --test lint           # every rule on tests/lint/*.vhd, the command line
```

A new rule: add it to `RULES`, its test in `Walk`, an `-- expect:` regression file or line in
`crates/yavs/tests/lint/`, regenerate the page, run it over the corpora.
