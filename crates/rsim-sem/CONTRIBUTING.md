# Working on rsim-sem

`rsim-sem` is M1, semantic analysis. The binding design is `docs/design/sem.md`; read it first,
plus `docs/plan.md` (error motto, rules 1, 7, 8, 12, 13) and `docs/decisions.md`. Several agents
work on this crate at the same time, so each takes a module, marks it below, and keeps edits to
other modules small and additive.

## Module map

One file per area. "Slice" is the sem.md section 10 slice that fills it.

| Module | Area | Slice | Status / owner |
|---|---|---|---|
| `ids.rs` | typed ids, `Arena` | S1 | done |
| `db.rs` | `Db`, arenas, `WellKnown`, `Poison`, diagnostics helpers (`error`, `ice`, `unsupported`) | S1 | done; add fields additively |
| `loader.rs` | libraries, `UnitKey`/`UnitSlot`/`Unit`, file cache, on-demand loader, `analyze_file`, `UnitRecord`s | S1 | M1 lead |
| `unit.rs` | design units, context clauses, implicit context; configurations (6.5) | S1 (context), S5 (entity/arch/config) | M1 lead (S1) |
| `scope.rs` | regions, `declare`, `lookup`, use clauses, visibility (1.4), undeclared-name reporting | S1 | M1 lead |
| `decl.rs` | `Decl`/`DeclKind`/`Object`/`Subprogram`/`Body`/`Alias`/`Component`; declarative item analysis | S1 (objects, subprogram specs), S2 (bodies) | M1 lead |
| `types.rs` | types, subtypes, constraints, constrainedness, type/subtype declarations | S1 (scalars), S3 (composites, constraint model) | M1 lead (S1) |
| `implicit.rs` | implicitly declared operations per type (2.10) | S2 | free |
| `expr/cands.rs`, `expr/resolve.rs` | two-pass overload resolution (3.1, 3.2, 3.6, 3.7) | S2 | M1 lead |
| `expr/names.rs` | `Apply`/`Selected`/`Attribute` interpretation (1.5, 3.3) | S2 | M1 lead |
| `expr/literal.rs` | literals, bit-string expansion (3.5, Q11) | S2 | owner: literal |
| `expr/aggregate.rs` | aggregates and choice sets (3.4), shared with `case` | S3 | free |
| `stat.rs` | staticness, `ConstValue`, folding (section 4) | S1 (scalar folding), S3 | M1 lead (S1) |
| `attr.rs` | predefined attribute table, user attributes (6.3) | S3, S6 | free |
| `generic.rs` | generic classes, `Instance`, `GenericMap`, cloning (section 5) | S4 | free |
| `mangle.rs` | mangled names (5.3), NATIVE.md symbols (2.10) | S2, S4 | free |
| `stmt.rs` | sequential (S2) and concurrent (S5) statements | S2, S5 | free |
| `lsp.rs` | longest static prefixes, drivers, `all` (6.9) | S5 | free |
| `alias.rs` | aliases, signatures (6.2, 6.4) | S2 (std aliases), S6 | owner: s6 |
| `external.rs` | external names (6.1) | S6 | owner: s6 |
| `protected.rs` | protected types and bodies (6.6) | S6 | owner: s6 |
| `view.rs` | mode views, `'converse` (6.7) | S7 | owner: s6 |
| `hir.rs` | HIR node shapes M2 consumes (section 7) | all | shapes fixed; changes need a note in sem.md |
| `validate.rs` | HIR invariants (7.3) | S2 onward | owner: literal |
| `diag.rs` | codes (8.1) and `--explain` texts | all | add codes additively |
| `suggest.rs` | did-you-mean (8.3) | S1 | done |

To take a module, edit its row in your first commit (`owner: <short name>`), push, then work.
If you need a function in a module someone else owns, add it in a small separate commit, or ask
through the orchestrator.

## Conventions

- **Fail early, fail clearly.** Every problem is a `rsim_diag::Diagnostic` reported through
  `Db::report`/`Db::error`, in the rustc format: code, primary span, labels, notes, help. Never
  `unwrap_or(default)` on anything that comes from the design. A construct a later slice will
  handle reports `Db::unsupported(span, what, "S<n>")` (code `E9999`, says which slice): never a
  silent default, never `todo!()`.
- **Codes.** Every diagnostic uses a constant from `diag.rs` (ranges in sem.md 8.1). A new code
  gets its constant and its explain text in the same `codes!` entry. Negative tests live in
  `tests/diag/sem/<area>/<name>.vhd` with the rendered output in `<name>.stderr`.
- **Cascade suppression (8.2).** Failed analysis returns `Err(Reported)`; the error was already
  reported, callers stay silent. Poison entities (`db.poison.{decl,ty,subtype,expr}`,
  `DeclKind::Error`, `TypeKind::Error`, `ExprKind::Error`) are compatible with everything and
  never diagnosed again. An undeclared name is reported once per (unit, key). A failed unit is
  `UnitSlot::Failed` and silent to its dependents.
- **Internal errors.** A broken invariant is `Db::ice(span, what)` ("this is a bug in rsim"), not
  a panic. `expect` is only for invariants of rsim's own data, with a message.
- **Names (rule 8).** Keep `rsim_syntax::ast::Ident` (original `text`, folded `key`). Look up by
  `Designator::key()`: identifiers by folded key, operators as `"and"` with quotes, characters as
  `'a'`.
- **One subtype representation (rule 1).** Constraints are HIR expressions (`RangeC`), folded
  values are a cache on `Expr::value`. Never branch on "is this static" to pick a representation.
- **Checked i64 everywhere (rule 7).** Static integer and physical arithmetic uses `checked_*`
  and reports `E1401` on overflow.
- **Revisions (0.1).** `db.rev` is the design revision `D`; `db.file_rev()` the file's own `F`.
  Declarations carry `since`; a reference with `F < since` is `E1007`.
- **No copied code.** Never copy NVC or GHDL (GPL) or rust_hdl (MPL). Reading for behaviour is
  fine; write it fresh from the LRM and sem.md.
- **Style.** `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, no em dashes in
  comments, docs or commits. Mark deliberate shortcuts with a `ponytail:` comment naming the
  ceiling.

## Testing

- `cargo test -p rsim-sem`. The positive corpus test analyzes the vendored `lib/` tree
  (`tests/stdlib.rs`); extend its list as slices land (sem.md 10, "green when").
- Outputs and scratch files go to `~/.cache/rsim-sem`, never into the repo.
