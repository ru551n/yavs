# AGENTS.md: yavs-verilog

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-verilog`.

## 1. Purpose and boundaries

- Verilog (IEEE 1364-2005) front end: preprocessor, lexer, parser, module analysis and detection of
  encrypted envelopes and SystemVerilog (M0 of the plan in #776). No elaboration or lowering yet.
- The scheduling domain (#776 step A, `docs/design/verilog-scheduling.md`): the Verilog regions inside one
  serial kernel process, with processes as Rust closures until lowering produces them.
- Does not: render diagnostics (they are plain data, `Diag`, with codes in `CODES` for the caller to
  register), touch the library manifest or the cache (the caller does, from `Unit`), or decide the
  language of a file for the CLI (`is_verilog_path`, `is_systemverilog_path` are helpers).
- Depends on `yavs-diag` for `suggest` only (did-you-mean), and on `yavs-kernel` and `yavs-logic` for the
  domain (`Process`, `Ctx`, `LogicVec`). Called by `crates/yavs` (`analyze`).

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | The API: `analyze`, `Options`, `Unit`, `Diag`, `Span`, `CODES`; per-file driver and E4001 |
| `src/pre.rs` | Preprocessor on bytes: macros, conditionals, `include`, `timescale`, `default_nettype`, `celldefine`, `begin_keywords` (`KwSet`, the lexer reads the set per token), `line`, protected envelopes; output-to-source map (`Pre::span`); `State` persists across files; context digest |
| `src/lex.rs` | Lexer: tokens with offsets into the preprocessed text; keywords, `(*)` and `*)` rules; UDP table body is skipped |
| `src/ast.rs` | The syntax tree (public, in memory only; M1 lowers it) |
| `src/parse.rs`, `src/stmt.rs` | Recursive-descent parser: modules, UDPs, declarations, instances, generate, specify; statements and expressions |
| `src/analysis.rs` | Ports (ANSI and non-ANSI), duplicate declarations per scope (E4203: module, task, function, named block, generate block, specify block), undeclared names under `default_nettype none` (E4201) |
| `src/diag.rs` | `Diag` builders and the code table with `--explain` text |
| `src/runtime.rs` | `Domain` (nets and regs, processes, `always`, `assign`), `Exec` (what a process body reads and writes: `get`, blocking `set`, `nba`, `display`, `finish`), `Wait`, `Edge` and `edge` (Table 9-9), the region loop (`settle`), future work, the event limit |
| `src/bridge.rs` | `Domain::input`/`output` and `impl Process for Domain`: inputs as `TO_X01Z`, the publication queue (`Core::record`), one batch per delta, the waits |
| `examples/vcheck.rs` | Developer tool: analyze files, print diagnostics, `--each` for corpus runs |
| `tests/frontend.rs` | Tests through the public API with sources written to a scratch directory |
| `tests/lrm.rs` | Runs every `tests/verilog/lrm/*.v` check (one LRM rule per file, header with the exact expected diagnostics) and checks that each is a row of `docs/design/lrm-checks.md` |
| `tests/sched.rs` | Scheduling: the kernel behaviour the bridge rests on, the regions, D4 at the boundary, edges, initialisation, the event limit, stop requests, 1 against 4 threads |
| `tests/props.rs` | `proptest` properties: number literals of every base and width, macro chains and argument counts, nested conditionals, damaged real sources (no panic, spans inside the file) |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Diagnostics are data (`code`, severity, message, span, label, notes, help); spans are byte ranges of the original files (`AnalysisResult::files`), also through macros (the call site) and includes | The crate has no renderer or source map; the caller owns both | #776 M0 split |
| The preprocessor works on bytes and keeps an output-to-source map; the lexer and parser see preprocessed text only | Non-UTF-8 comments exist in vendor sources; offsets stay exact | |
| Macros, `timescale` and `default_nettype` persist over the files of one call; each unit records a digest of the state entering its file | 1364 compilation-unit behaviour; reproducible single-file re-analysis | plan 3.7 |
| An envelope (`pragma protect begin_protected`, legacy `protect ... `endprotect or `protected ... `endprotected) is replaced by a marker byte and is never an error at analysis: a module whose header precedes it is recorded as an encrypted unit (ports from the readable header), a file with an envelope outside any module records nothing, and either way one W4007 per file. E4001 is reserved for the error at elaboration (M1) | D8 and the coordinator's ruling: record at analysis, error when instantiated | D8 |
| `pragma protect begin/end` plaintext markers warn per marker (W4009) and the plaintext is compiled | The plaintext is readable | plan 5 |
| SystemVerilog words are ordinary identifiers to the lexer; the parser reports E4003 naming the word where it appears as a construct (item or statement start, port or parameter type, `++`, `+=`, `.*`, `end : label`, time literals) | `logic`, `ref`, `bit` are legal 1364 identifiers elsewhere | plan 6 |
| A syntax error abandons the module it is in (skip to `endmodule`); a file with an error yields no units; the first failing file stops the batch (`stopped_at`) | No guessed AST, no cascades | AGENTS.md rule 1 |
| `$error`, `$warning`, `$info` and `$fatal` are accepted as module and generate-block items (`Item::ElabTask`, kept and not executed before M1); any other system task at item level is `E4100` | Deliberate extension beyond 1364-2005: vendor and cell models use them; `docs/design/lrm-checks.md` V4 | |
| `begin ... end` standing alone in a module (inside or outside `generate`) is accepted as an unnamed generate block | Vendor IP (MIG, blk_mem_gen) contains it and the other tools accept it; LRM policy rule 2 | |
| A decimal size of 0, or a number right after `#`, is not glued to a following `'b`/`'d` literal: `#1 'd0` is a delay and an unsized literal | A size is never zero (3.5.1); `` `define DLY #1 `` is used before literals in vendor IP | |
| `*)` closes an attribute only while one is open; `(*)` and `@( *)` are star lists | `@(/*AS*/*)` after comment removal | |
| UDP tables are read as text between `table` and `endtable` (symbols like `(1x)` are not tokens) | Lexing them as Verilog fails | |
| E4201 (undeclared) uses one flat set of every name declared anywhere in the module and is only on under `default_nettype none` | Loose by design: no false errors on vendor code; scoping is M1's job | `ponytail:` ceiling: a name declared in another scope is accepted |
| Specify blocks and timing checks are parsed into the tree and not evaluated; with `XIL_TIMING` defined one W4015 per file says so | D14 | plan 6 |
| A front-end rule of the LRM gets a file in `tests/verilog/lrm` and a row in `docs/design/lrm-checks.md` in the same commit; where the LRM is clear and the front end accepts, the file is pinned with a `// gap:` header and listed as a known gap | Rule 14 and the LRM policy; the VHDL side does the same with `tests/diag/sem` | `lrm-checks.md` "Per-clause checks" |
| `` `include `` takes a quoted name, `<name>` or an argument-less macro that expands to one; a gate has its terminals by position, a fixed number per gate; the variable of a generate loop must be a genvar (`E4204`) | LRM 7.1, 12.4.1; the include forms are the other tools' | `lrm-checks.md` V5, V8 |
| The checks of `tests/verilog/lrm` were audited against the text of IEEE 1364-2005 on 2026-11-07: clause numbers are the standard's, the table in `lrm-checks.md` is generated from the file headers, and the choices where yavs differs from the text are the readings V5 to V14 and the gaps G1 to G11 there. Parser state that limits what is legal (`in_func`, `local`, `in_gen`, `genvars`) is reset in `module()` | 164 of the first checks had a wrong clause number; one error and one position per file | `lrm-checks.md` "Per-clause checks" |
| `macromodule` is `module`; the ports of an ANSI header are not declared again in the body (E4203); the same implicit port twice in a non-ANSI header is legal; a block with declarations is named (W4206 when not, V14); `begin_keywords` sets are read per token | LRM 12.1, 12.3.3, 12.3.4, 9.8.1, 19.11 | `lrm-checks.md` V6, V10, V12, V14 |
| Spans of diagnostics are clamped to the file: an unexpected end points at the empty range at the end | A span past the end of the file breaks renderers | `tests/props.rs` |
| `Unit::timescale` is the directive in force or `Options::default_timescale` (W4006 once per file) | D10 | |
| All Verilog is one `Domain`, one serial kernel process; its regions run inside one activation and `SimControl` is untouched | Blocking writes and continuous assignments cost no delta (a clock buffer must not skew); the VHDL cycle stays as it is | `docs/design/verilog-scheduling.md` 1 |
| Output changes go out as ordered batches, one per kernel delta: Active results at `d+1`, each NBA region one batch later (an empty batch 0 still costs its delta); a port written again goes to the batch after its last entry and the cursor follows | D4; no change of a port is lost and the order across ports holds | same, 2; open question: skip empty batches |
| Future NBAs and `#d` resumptions are in the domain's own time map, never kernel transactions | `Waveform::schedule` drops later transactions even for transport | same, 3 |
| `posedge`/`negedge` by Table 9-9 on element 0 (`edge`), inputs read as `TO_X01Z`; a VHDL initial value is no event | 9.7.2; `Ctx::edge` has the VHDL rules | same, 1 and 2 |
| A process that runs more than `EVENT_LIMIT` (10000) times at one time is `E3009`; the loop polls `Ctx::stop_requested` every 1024 activations | The kernel's delta limit cannot see a loop inside one activation; a per-process count does not grow with the design | same, 4; `ponytail:` a dedicated code later |

Ceilings of the domain (each a `ponytail:` note in the code): `docs/design/verilog-scheduling.md` section 5.

## 4. Commands

```sh
cargo test -p yavs-verilog
cargo clippy -p yavs-verilog --all-targets -- -D warnings
cargo run -p yavs-verilog --example vcheck -- --each --quiet FILE...   # corpus-style run
```

## 5. Pitfalls

- The domain starts every process at initialisation in declaration order: a process that must see an
  initial write has to be declared (and so waiting) before the writer.
- Boundary signals are logic signals (`SignalType::logic`) of the net's width; `Ctx::logic` refuses byte signals.
- Offsets in the AST are into the preprocessed text; map them with `Pre::span`, never use them on the original file.
- `Parser::unexpected` decides between E4003 and E4100 from the token and the one before it; add words to `SV_WORDS` with care (a word that is a legal identifier in 1364 must only count at statement or item start).
