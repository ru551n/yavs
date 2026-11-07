# AGENTS.md: yavs-verilog

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-verilog`.

## 1. Purpose and boundaries

- Verilog (IEEE 1364-2005) front end: preprocessor, lexer, parser, module analysis and detection of
  encrypted envelopes and SystemVerilog. M0 of the plan in #776: no elaboration, no simulation (M1).
- Does not: render diagnostics (they are plain data, `Diag`, with codes in `CODES` for the caller to
  register), touch the library manifest or the cache (the caller does, from `Unit`), or decide the
  language of a file for the CLI (`is_verilog_path`, `is_systemverilog_path` are helpers).
- Depends on `yavs-diag` for `suggest` only (did-you-mean). Called by `crates/yavs` (`analyze`).

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | The API: `analyze`, `Options`, `Unit`, `Diag`, `Span`, `CODES`; per-file driver and E4001 |
| `src/pre.rs` | Preprocessor on bytes: macros, conditionals, `include`, `timescale`, `default_nettype`, `celldefine`, protected envelopes; output-to-source map (`Pre::span`); `State` persists across files; context digest |
| `src/lex.rs` | Lexer: tokens with offsets into the preprocessed text; keywords, `(*)` and `*)` rules; UDP table body is skipped |
| `src/ast.rs` | The syntax tree (public, in memory only; M1 lowers it) |
| `src/parse.rs`, `src/stmt.rs` | Recursive-descent parser: modules, UDPs, declarations, instances, generate, specify; statements and expressions |
| `src/analysis.rs` | Ports (ANSI and non-ANSI), duplicate declarations (E4203), undeclared names under `default_nettype none` (E4201) |
| `src/diag.rs` | `Diag` builders and the code table with `--explain` text |
| `examples/vcheck.rs` | Developer tool: analyze files, print diagnostics, `--each` for corpus runs |
| `tests/frontend.rs` | Tests through the public API with sources written to a scratch directory |

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
| `begin ... end` standing alone in a module (inside or outside `generate`) is accepted as an unnamed generate block | Vendor IP (MIG, blk_mem_gen) contains it and the other tools accept it; LRM policy rule 2 | |
| A decimal size of 0, or a number right after `#`, is not glued to a following `'b`/`'d` literal: `#1 'd0` is a delay and an unsized literal | A size is never zero (3.5.1); `` `define DLY #1 `` is used before literals in vendor IP | |
| `*)` closes an attribute only while one is open; `(*)` and `@( *)` are star lists | `@(/*AS*/*)` after comment removal | |
| UDP tables are read as text between `table` and `endtable` (symbols like `(1x)` are not tokens) | Lexing them as Verilog fails | |
| E4201 (undeclared) uses one flat set of every name declared anywhere in the module and is only on under `default_nettype none` | Loose by design: no false errors on vendor code; scoping is M1's job | `ponytail:` ceiling: a name declared in another scope is accepted |
| Specify blocks and timing checks are parsed into the tree and not evaluated; with `XIL_TIMING` defined one W4015 per file says so | D14 | plan 6 |
| `Unit::timescale` is the directive in force or `Options::default_timescale` (W4006 once per file) | D10 | |

## 4. Commands

```sh
cargo test -p yavs-verilog
cargo clippy -p yavs-verilog --all-targets -- -D warnings
cargo run -p yavs-verilog --example vcheck -- --each --quiet FILE...   # corpus-style run
```

## 5. Pitfalls

- Offsets in the AST are into the preprocessed text; map them with `Pre::span`, never use them on the original file.
- `Parser::unexpected` decides between E4003 and E4100 from the token and the one before it; add words to `SV_WORDS` with care (a word that is a legal identifier in 1364 must only count at statement or item start).
