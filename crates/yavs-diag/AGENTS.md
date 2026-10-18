# AGENTS.md: yavs-diag

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-diag`.

## 1. Purpose and boundaries

- One diagnostics model for every stage (`Diagnostic`, `Label`, `Severity`, `Span`, `SourceMap`)
  and one renderer (`render`) in three formats: rustc-style human (via `annotate-snippets`),
  `short` (one line), `json` (one object per line).
- Owns the **code registry** (the table in the crate docs of `src/lib.rs`).
- Does not: decide codes for other crates (each stage defines its `pub const`s next to the code
  that reports them), print anything, count errors, or carry runtime stack traces (the kernel's
  `SimError::trace` does that; the CLI prints runtime errors itself).
- Depends on: `annotate-snippets` only. Called by: `yavs-syntax`, `yavs-sem`, `yavs-lib`,
  `yavs-ir`, `yavs-interp`, `yavs-lower`, `yavs-elab`, `yavs-vhpi`, `yavs` (CLI). Keep it a leaf.

## 2. Map

Everything is in `src/lib.rs` (about 530 lines).

| Item | Role |
|---|---|
| `FileId`, `Span` (`file`, `start`, `end`: byte offsets into the file on disk), `Span::to` | Source positions |
| `SourceFile`, `SourceMap::add` / `add_latin1` / `get` | Loaded files; Latin-1 files keep their raw bytes and map offsets to the decoded text (`text_offset`, `line_col`) |
| `Severity` (`Error`, `Warning`, `Note`) | Level |
| `Diagnostic::error/warning/new`, `with_primary`, `with_secondary`, `with_note`, `with_help` | Builder |
| `MessageFormat` (`Human`, `Short`, `Json`), `ColorChoice::use_color` (TTY and `NO_COLOR`) | Output options |
| `render(sources, diag, format, color) -> String` | The one renderer; ends with a newline |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| rustc-format diagnostics: code, primary span, labels, notes, help | Error motto: fail early, fail clearly | `docs/plan.md` "Error-handling motto", architecture table |
| Every code is stable, unique, never reused; one range per stage | `-W`/`-Werror` per code, `--explain` | `docs/plan.md` design rule 13; root `AGENTS.md` rule 5 |
| An empty code is only for summary lines ("aborting due to N previous errors") | Not a real diagnostic | crate docs |
| Spans are byte offsets into the file on disk, also for Latin-1 files | `--encoding=auto|latin1|utf8` (user decision) | `docs/decisions.md` "Parser", source encoding |
| Colour off when stderr is not a TTY or `NO_COLOR` is set; `--color` overrides | plan architecture table | `ColorChoice::use_color` |
| JSON shape is rustc-compatible (`level`, `children`, `rendered`); the contract won over the old `severity`/`notes`/`help` (#158) | Editors, DAP and VUnit can reuse rustc JSON handling | `docs/cli-contract.md` section 0 |

Module-local decisions go here, in this section, in the same commit (root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- `render` must never panic on a span outside the file (fuzzing renders every parser error);
  `line_col` clamps.
- Snapshot output is part of review: any change in human or JSON text changes
  `tests/diag/*.stderr`, `tests/cli/*.stderr` and `crates/yavs-sem` snapshots. Bless and read the
  diff (`YAVS_BLESS=1`).
- The JSON keys are rustc-compatible (#158, `docs/cli-contract.md` section 0): `level`, `code` (plain
  string), `message`, `spans[]` (`file`, `byte_start`, `byte_end`, `line_start`, `column_start`,
  `line_end`, `column_end`, `primary`, `label`), `children[]` (notes then help, each `{level,
  message, spans:[], children:[], rendered:null}`), `rendered` (human text, no colour). Change
  the contract and the renderer together.
- Runtime errors carry `yavs_kernel::ErrorKind::code` (`E3001`-`E3010`) and are rendered by
  `yavs_elab::Elaborated::runtime_diagnostic`. A caught panic is `E9999`.
- `yavs --explain CODE` lives in the CLI (`crates/yavs/src/explain.rs`): this crate stays a leaf and
  only owns the registry table. Its test fails for any code literal in `crates/*/src` without a text.

## 5. How to work here

```sh
CARGO_BUILD_JOBS=6 cargo test -p yavs-diag          # unit tests (line/col, Latin-1, human, short, json)
cargo test -p yavs --test diag --test cli           # rendered snapshots through the binary
YAVS_BLESS=1 cargo test -p yavs --test diag         # accept new output, then inspect git diff
```

- Add a unit test in the `tests` module of `src/lib.rs` for renderer changes; add a
  `tests/diag/NAME.vhd` snapshot when the change is visible end to end.
- Codes owned here: none. A new range for a new crate goes into the registry table in
  `src/lib.rs` in the same commit as its first code.

## 6. Coordination

- Every crate depends on this one: a signature change touches all of them. Prefer additive
  builder methods.
- Open issues (snapshot 2026-10-18, `gh issue list -R ru551n/yavs --label crate:diag`):
  none at the moment.
