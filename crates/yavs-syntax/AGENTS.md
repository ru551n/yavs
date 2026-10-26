# AGENTS.md: yavs-syntax

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-syntax`.

## 1. Purpose and boundaries

- Lexer, conditional analysis (`` `if `` and other tool directives, evaluated on the token stream)
  and a hand-written recursive-descent parser for VHDL-1993, VHDL-2008 and VHDL-2019 into an owned,
  purely syntactic AST with spans.
- Does not: resolve names, decide what an ambiguous form means (`f(x)` call vs index vs slice vs
  conversion, `label: p;` call vs instance, choices that may be subtypes), type anything, or read
  library manifests. All of that is `yavs-sem`.
- Single-threaded by design (`docs/plan.md`: parsing and analysis run on one thread).
- Depends on: `yavs-diag`. Called by: `yavs-sem` (main consumer), `yavs-lower` and `yavs-elab`
  (AST types, `Symbol`), `yavs` (the `parse` command), `fuzz/`.

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | `Std`, `Options` (`std`, `defines`), `Encoding`, `load_source`, `parse_file`, `parse_str`, `Parsed` (`design` only when no error), `codes` |
| `src/symbol.rs` | `Symbol`: global interner behind a lock plus a per-thread copy (hits and `as_str` take no lock; both are open-addressing `Index`es probed with one hash that they keep), leaked strings, `as_str() -> &'static str`, `index()`; `encode_table`/`DecodeTable`: the symbol text table of a cache image, each symbol interned on first read (an atomic per symbol) |
| `src/source.rs` | `Src` (bytes plus Latin-1 flag) |
| `src/token.rs` | `Tok`, `Kw` (keywords per standard), `Token` |
| `src/lexer.rs` | `lex`: bytes to tokens, `-- psl` pragmas, `E0001`, `E0009`, `W0003` |
| `src/protect.rs` | Language-neutral detector of `` `protect begin_protected `` envelopes (`begins_envelope`, `envelope_info`); the lexer reports `E0012` and stops, `parse_file` skips the later stages (#775) |
| `src/directive.rs` | `preprocess`: `` `if/elsif/else/end ``, `` `warning/error ``, unknown directives `W0002`; standard identifiers (`VHDL_VERSION`, `TOOL_*`) |
| `src/parser/mod.rs` | `Parser`, `PResult`/`Recover`, resync, `MAX_DEPTH = 64`, fuel counter |
| `src/parser/unit.rs`, `decl.rs`, `stmt.rs`, `expr.rs` | Design units, declarations, statements, expressions and names |
| `src/parser/psl.rs` | The PSL subset: `default clock`, directives to checker processes (#226), built-in function calls to process variables (#445) |
| `src/parser/psl_named.rs` | Named PSL sequences and properties: declarations as token ranges, expansion with the formals bound, renaming of boolean formals (#445) |
| `src/parser/psl_prop.rs` | Temporal PSL properties: parser, terms (formula progression), enumeration of the reachable terms, checker process generation (#445) |
| `src/parser/psl_seq.rs` | PSL sequences: SERE parser, position automaton (Glushkov), deterministic obligations, checker process generation (#445) |
| `src/ast.rs` | The tree: `DesignFile`, `DesignUnit`, `LibraryUnit`, `Decl`, `SeqStmt`, `ConcStmt`, `Expr`, `Name`/`NameKind::Apply`, `Range::Name`, `Ident` (`text`, `key`, `span`) |
| `src/tests.rs` | Unit tests (about 1,000 lines) |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Own parser, 2008 and 2019 in one grammar, gated by `Std` (`E0004` for 2019 syntax in 2008, `E0011` for 2008 syntax in 1993, #629) | Plan decision | `docs/plan.md` "Decisions made with the user" |
| `Ident` keeps original spelling plus folded key; extended identifiers fold to themselves | VHPI `CaseName`, waves, diagnostics | `docs/plan.md` rule 8 |
| Global interner, leaked strings, per-thread mirror of the table; a text is hashed once and the hash is kept in the slot | Symbols valid on runtime threads; a hit or `as_str` takes no lock (#424); a miss hashed five times and growing re-read every text (#742) | `docs/decisions.md` "Parser", "Cold analysis, round 4" |
| List parsers (statements, declarations, interface and association lists) push onto a scratch stack in `Parser` and move the elements into an exact-size `Vec`; rare large AST variants (`Actual::Range/Subtype`, `Constraint::Range`, interface type/subprogram/package) are boxed | No doubling of 200 to 400 byte elements, smaller nodes, less copying per parse step | #424 |
| `parse_file` returns the tree only when there was no error; recovery inside a file lists independent errors; cascades at the same or a lexically bad token are dropped | Fail early, no guessed AST | `docs/decisions.md` "Parser"; error motto |
| An encrypted envelope (`` `protect begin_protected ``) is `E0012`, the lexer stops there and no later stage runs; hard error, no recorded unit | Whole VHDL units are encrypted, there is no header to record; one clear error instead of thousands of base64 warnings; `docs/decisions.md` "Parser" | #775 |
| Conditional analysis in 2008 files too; unknown directives warn `W0002`; `TOOL_VENDOR`/`TOOL_NAME` = `yavs` | Harmless, 2008 allows tool directives | `docs/decisions.md` "Parser" |
| 2019 makes only `end component` optional; other bare `end;` is `E0002` with a help | LRM, matches the reference simulators | `docs/design/lrm-checks.md` P1 |
| Where 2019 conditional expressions are allowed | Permissive per LRM policy | `docs/design/lrm-checks.md` P2, `docs/decisions.md` "Parser" |
| Strict on `abs x ** 2`, chained relations, mixed logical operators; `10ns` accepted with `W0003` | LRM grammar; common real code | `docs/decisions.md` "Parser" |
| Encoding: `auto` (UTF-8 if valid else Latin-1), `latin1`, `utf8` (`E0010`); non-Latin-1 char in literal/identifier `E0009`; spans are file byte offsets | User decision | `docs/decisions.md` "Parser", source encoding |
| A type mark takes no parentheses except the dimension of 2019 `A'INDEX(n)` (`NameKind::Apply` on the attribute), and an indexed or sliced object name whose parentheses are followed by `'` (`arr(0)'subtype`, #616): those parentheses belong to the name, never to an index constraint | The index subtype is scalar, so no index constraint can follow; #174; a constraint is never followed by `'` | `src/parser/expr.rs` `type_mark` |
| PSL outside the subset is skipped with one `W0004` per design unit (`E0005` retired, never reuse); the skip ends at the `;` outside `()`, `[]`, `{}` (`sync`), a `vunit`/`vmode`/`vprop` at its closing `}`; a concurrent `assert`/`cover` counts as PSL by its first property token (`at_psl_directive`) | User decision 2026-10-18: do not fail designs that merely carry assertions yavs cannot run | `docs/decisions.md` "Parser", #138 |
| The PSL subset (`default clock`, `assert`/`assume` `always`/`never`/`->`/`next[n]`, `cover`, `@` clocks) becomes an implicit checker process in the tree (`parser/psl.rs`); an attempt that does not fit restores the position and drops its diagnostics, then the skip runs; `-- psl` comment pragmas are lexed as tokens only with `Options::psl` (`--psl`; user, 2026-10-19) (`lexer.rs` `comment`, `is_pragma`); `->` is one token (`Tok::Imply`) | Nothing new in sem, lowering or the kernel; deterministic at any thread count | `docs/decisions.md` "Parser", #226 |
| `restrict`, `restrict_guarantee`, `assume_guarantee`, `fairness`, `strong fairness` and a directive that calls `nondet`/`nondet_vector` are skipped without `W0004` (`psl_formal_only`, called by `psl_skip`; `at_psl_directive` takes an `assert` with a `nondet` call as PSL) | Formal-only: nothing to run in simulation | `docs/decisions.md` "Parser", user 2026-10-23, #445 |
| PSL sequences (`{a; b[*2]}`, fusion, alternation, `&&`, `&`, `within`, `[=n]`, `[->n]`, suffix implication) become a position automaton in process variables (`parser/psl_seq.rs`): the antecedent and `cover`/`never` use the union of the matches in progress, the right side of an implication a determinised obligation (at most 6 booleans, 64 states); a boolean needs `Parser::psl_no_sig` so that `[` ends it instead of opening a signature; limits skip the directive with `W0004` | Exact failure per start without kernel changes | `docs/decisions.md` "Parser", L123, #445 |
| Named PSL sequences and properties are token ranges expanded by parsing the body again with the formals bound (`parser/psl_named.rs`: `boolean` formals are process variables and a renamed name, `const` a number, `sequence` and `property` the actual's tokens); a built-in on a formal or a typed formal skips the directive with `W0004` | No tree clone; same process shape as the plain directives | `docs/decisions.md` "Parser", #445 |
| Temporal PSL properties (`next_a`/`next_e`/`next_event*`, `until*`, `before*`, `sync_abort`/`abort`/`async_abort`, `<->`, nested `next`/`always`, a property after `|->`) become terms stepped per tick; the reachable terms are enumerated at parse time, one process variable each (`parser/psl_prop.rs`); strong operators skip the directive with `W0004`; `abort` and `async_abort` run as `sync_abort` (user, 2026-10-23) | Weak finite-trace semantics with no change in sem, lowering or the kernel; the end-of-run check of strong operators has no place in a process | `docs/decisions.md` "Parser", L133, #445 |
| No VHDL-93 mode: 93 code using 2008 reserved words is an expected corpus failure | Plan scope | `docs/decisions.md` "Parser", corpus lists |

Module-local decisions go in this section in the same commit; LRM readings go to
`docs/design/lrm-checks.md` (root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- The parser never panics or loops: every list loop must make progress, nesting is capped at 64
  (`E0006`; one level of parentheses, an aggregate or a call counts once, in `primary`, not in `expr`: a chain of 30 to 60 parenthesised groups is real RTL, #604; the cap is what a 2 MiB debug-build stack parses, and the rest of the pipeline was checked to 60 levels on 2 MiB), and the fuel counter stops a stalled parse. A new loop without progress is a fuzz
  crash waiting to happen (`fuzz/` target `parse`, rule 15).
- Keep ambiguous forms ambiguous. Resolving them here breaks `yavs-sem`'s name interpretation
  (`docs/design/sem.md` 3.3).
- Spans are byte offsets into the bytes on disk, never into decoded text.
- AST shape changes ripple into `yavs-sem` (all of `src/`), `yavs-lower` and `yavs-elab`. Make
  them additive where possible and coordinate via `crates/yavs-sem/CONTRIBUTING.md` Requests.
- A new list parser uses the scratch stacks of `Parser` (`sc`): note the length, push, and cut back
  to it on an error path, so no half-built list stays above a caller's mark.
- Keep AST nodes small: a variant that is large and rare is boxed (check `size_of` of `Decl`,
  `SeqStmt`, `AssocElem` when adding fields); nodes are moved by value through every parse step.
- Any change to diagnostic text changes `tests/diag/*.stderr`: bless and review.
- 2019 protected type instantiation, `type t is new g generic map (...)`, is
  `TypeDef::ProtectedInstance` (#29).

## 5. How to work here

```sh
CARGO_BUILD_JOBS=6 cargo test -p yavs-syntax            # unit tests
cargo test -p yavs --test diag --test lib_parse          # snapshots in tests/diag, all of lib/ parses
YAVS_BLESS=1 cargo test -p yavs --test diag              # accept rendered changes, review the diff
tools/fetch-corpus.sh && YAVS_CORPUS=~/.cache/rsim-corpus cargo test -p yavs --features corpus
cd fuzz && cargo +nightly fuzz run parse                 # see fuzz/AGENTS.md
cargo run -p yavs -- parse --std 2019 --dump-ast f.vhd   # inspect the tree
```

- New syntax: a unit test in `src/tests.rs` plus, for an error, a `tests/diag/NAME.vhd` snapshot
  (first line `-- yavs: --std 2019` for 2019). Corpus list changes in `tests/parse-corpus/` must
  be explained in the commit.
- Codes owned: `E0001`-`E0099`, `W0001`-`W0099` (`yavs_syntax::codes`: E0001 to E0012, W0001 to
  W0004), plus `E9999` for internal errors.

## 6. Coordination

- `yavs-sem` is the only real consumer of AST details; check its `CONTRIBUTING.md` for requests
  addressed to the parser.
- Open issues (snapshot 2026-10-18, `gh issue list -R ru551n/yavs --label crate:syntax`): #87 periodic fuzz and parse-corpus gate; #29 generic map on variable declarations (2019 generic
  protected types).
