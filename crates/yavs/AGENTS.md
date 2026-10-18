# AGENTS.md: yavs (the CLI binary)

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs`.

## 1. Purpose and boundaries

- The `yavs` executable: `parse`, `analyze`, `elab`, `run`. Parses options (clap), expands `@file`
  response files, maps libraries, drives the stages in order and stops at the first failed one,
  renders diagnostics, maps outcomes to exit codes, prints reports in NVC's format.
- Does not: analyze, elaborate or simulate itself. All semantics live in the library crates; the
  CLI only wires them. VUnit depends on its behaviour through `docs/cli-contract.md`.
- Depends on: `yavs-diag`, `yavs-syntax`, `yavs-lib`, `yavs-sem`, `yavs-std`, `yavs-elab`,
  `yavs-kernel`, `clap`. Not yet wired: `yavs-vhpi` (M5), `yavs-wave` (M7), `yavs-llvm` (M6),
  `yavs-ffi` via `--load` (M3). Nothing depends on this crate.

## 2. Map

| File | Responsibility |
|---|---|
| `src/main.rs` | `Cli`/`Command` (clap), global `--color`, `--message-format`, `--encoding`; `parse` subcommand; `@file` expansion before clap |
| `src/analyze.rs` | `yavs analyze` (`Args`), `expand_response_files`, `stdlib()` (finds `lib/`), `Out` (counts errors/warnings, `-Werror`, "aborting due to" line, exit code), batch notes |
| `src/explain.rs` | `--explain` table (all codes but those of `yavs-sem`) and the every-code-has-a-text test |
| `src/elab.rs` | `yavs elab` (`Common`) and `yavs run` (`RunArgs`): closure from manifests, design revision, `Db::analyze_closure`, `yavs_elab::find_top`/`elaborate`/`lower`/`build`, kernel `Config`, `Printer` observer, `--stop-time` parsing, end-of-run exit code |
| `tests/cli.rs` | Exit code and stderr snapshots for `analyze`/`elab`/`run` (`tests/cli/`), library locking, concurrent batches |
| `tests/diag.rs` | Parse diagnostic snapshots (`tests/diag/*.vhd` vs `.stderr`, `-- yavs: ARGS` header) |
| `tests/lib_parse.rs` | Every file in `lib/` parses (2008 and 2019) |
| `tests/corpus.rs` | External parse corpora vs `tests/parse-corpus/` lists (`--features corpus`, `YAVS_CORPUS`) |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Stop at the first failed stage; never exit 0 after an error | Error motto | `docs/plan.md` "Error-handling motto", `docs/cli-contract.md` 0 |
| Exit codes: analyze 0/1 (file error)/2 (usage)/3 (ICE); run 0, 1 (failure, report at/above `--exit-severity`, error reported), 2 (usage, analysis or elab error), 3 (internal) | VUnit reads only zero/non-zero, humans need the class | `docs/cli-contract.md` 4, 5 |
| `--exit-severity` default `failure`; an `error` report still makes the run exit 1 at the end | Like NVC, GHDL, Questa (user, 2026-10-18) | `docs/decisions.md` "Runtime: default exit severity" |
| `std`/`ieee` are provided by yavs; mapping them is `E0102` (except `analyze --bootstrap`) | One std/ieee per design | `docs/cli-contract.md` 3 |
| Design revision = newest standard in the closure unless `--std` is given | Mixed 2008/2019 projects (user, 2026-10-18) | `docs/plan.md` "VUnit integration", `docs/design/sem.md` 0.1 |
| `--threads` defaults to 1 | Each yavs under VUnit `-p N` | `docs/plan.md` "VUnit integration" |
| `@file` response files from the start | VUnit on Windows | `docs/plan.md` "VUnit integration" |
| `--encoding=auto` default | User decision | `docs/decisions.md` "Parser", source encoding |
| Reports on stdout as `** Note: <fs>fs+<delta>: msg` then `    Process <path>` | tools/difftest parses NVC's format | `docs/decisions.md` "Runtime: default exit severity", `docs/cli-contract.md` 5 |
| Runtime errors are diagnostics on stderr (`E3001`-`E3010`, statement span, `called from here` per frame, time/delta/process note) via `Elaborated::runtime_diagnostic` | Same format as every other error | `docs/cli-contract.md` 5 |
| A runtime error while package or instance data is initialised (`Elaborated::build`) is exit 2 with the runtime diagnostic; an internal one is exit 3 | It is elaboration, and VUnit users need the VHDL location | `docs/cli-contract.md` 5 |
| Unimplemented flags (`--wave`, `--load`, `--vhpi`, `--dap`) are `E9999` "not implemented yet" naming the milestone, exit 2 | No silent ignore | root `AGENTS.md` rule 1 |
| `yavs analyze` holds one exclusive library session for the whole batch; elab/run only read | Rule 14 locking | `docs/plan.md` rule 14, `docs/cli-contract.md` 3 |

Module-local decisions go in this section in the same commit; CLI or VUnit-facing behaviour goes
into `docs/cli-contract.md` (root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- `docs/cli-contract.md` is a contract with VUnit (`~/git/vunit` `sim_if/yavs.py`). Change it and
  the code together, deliberately; `tests/cli.rs` pins the observable part.
- A panic in `analyze` is caught and reported as an `E9999` ICE with the file name (exit 3); `elab`
  and `run` are wrapped by `guard` in `main.rs` (no file name). The panic hook is silenced in `main`.
- `-W` forms (`Out::set_warn` in `analyze.rs`): `error`, `error=CODE`, `no-CODE`, `CODE`; the last
  option for a code wins, an unknown or non-warning code is `E0103`. Warnings are filtered in
  `Out::emit`, so every stage that goes through `Out` obeys them.
- `yavs --explain CODE` is `src/explain.rs`: its table covers every code except `yavs-sem`'s
  (`yavs_sem::diag::EXPLAIN`). A test scans all `crates/*/src` for code literals, so a new code
  fails `cargo test -p yavs` until it has a text (#136).
- `--ieee-warnings` is accepted and ignored on purpose (`ponytail:` comment in `elab.rs`).
- `stdlib()` order: `$YAVS_STDLIB`, then `lib/` next to the executable or up to three parents,
  then the checkout path compiled into `yavs-sem` (#43). A stale binary from a shared
  `CARGO_TARGET_DIR` uses another worktree's `lib/` and crates (#7): keep your own target dir.
- Tests run the built binary (`env!("CARGO_BIN_EXE_yavs")`); scratch dirs carry the pid
  (`yavs-cli-<pid>-<name>`). Keep that pattern, never a fixed name in `/tmp`.

## 5. How to work here

```sh
CARGO_BUILD_JOBS=6 cargo test -p yavs                    # cli, diag snapshots, lib/ parse
YAVS_BLESS=1 cargo test -p yavs                          # rewrite tests/diag and tests/cli expected files; review the diff
tools/fetch-corpus.sh && YAVS_CORPUS=~/.cache/rsim-corpus cargo test -p yavs --features corpus
cargo run -p yavs -- analyze --std=2008 --work=work:$HOME/.cache/<dir>/work f.vhd
cargo run -p yavs -- run --work=work:$HOME/.cache/<dir>/work top --threads 4
```

- New CLI behaviour: add a `Case` to `tests/cli.rs` with its source in `tests/cli/` and bless the
  `.stderr`. A parse diagnostic: add `tests/diag/NAME.vhd` and bless.
- Env vars: `YAVS_STDLIB` (std/ieee tree), `YAVS_BLESS=1`, `YAVS_CORPUS`, `NO_COLOR`.
- Codes owned: `E0100`-`E0199`/`W0100`-`W0199` (in use: E0100 unreadable file, E0101 response
  file, E0102 std/ieee mapped, E0103 `-W` form, E0104 no top level, E0105 bad `--stop-time`).

## 6. Coordination

- The difftest (`tools/difftest`) and the corpus report (`tools/corpus/report.sh`) drive this
  binary; output format changes break their parsers.
- Open issues (snapshot 2026-10-18, `gh issue list -R ru551n/yavs --label crate:cli`): #64 wire
  lowering, interpreter and kernel into `yavs run`; #95 `--load`; #110 VUnit SimulatorInterface;
  #116 `--wave`; #135 native Windows build; #22 bootstrap mode; #20
  `--encoding` in analyze.
