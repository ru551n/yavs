# AGENTS.md: yavs-sem

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-sem`.
It summarises and links; the binding documents are `docs/design/sem.md` (design),
`crates/yavs-sem/CONTRIBUTING.md` (module owners, conventions, Requests) and
`docs/design/lrm-checks.md` (LRM rulings).

## 1. Purpose and boundaries

- M1 semantic analysis: libraries and design units (on-demand loading from manifests), scopes and
  visibility, the type system (one subtype representation, rule 1), two-pass overload
  resolution, static evaluation, generics and instances, aliases, attributes, external names,
  protected types, mode views, drivers and longest static prefixes, `std.reflection` typing.
  Output: a **typed HIR** (`hir.rs`) where every expression has its subtype and every call names
  its overload; diagnostics `E1xxx`/`W1xxx`.
- Does not: elaborate (generics values, hierarchy: `yavs-elab`), lower to IR (`yavs-lower`), run
  anything, or keep a compiled-unit cache (every run re-analyzes from source, `docs/plan.md`).
  Single-threaded by design.
- Depends on: `yavs-diag`, `yavs-syntax`, `yavs-lib` (`Libraries`, `UnitRecord`), `yavs-std`
  (`value::lex_abstract` for literals). Called by: `yavs` (analyze, elab), `yavs-lower`,
  `yavs-elab`, `fuzz/` (target `sem`).

## 2. Map

Full module table with owners: `CONTRIBUTING.md` "Module map". Entry points and core types:

| Item | File |
|---|---|
| `Db` (arenas, `WellKnown`, `Poison`, `report`/`error`/`ice`/`unsupported`, `diagnostics`, `take_diagnostics`) | `src/db.rs` |
| `Options` (`rev`, `stdlib`, `libraries`, `relaxed`, `defines`), `LangRev = yavs_syntax::Std`, `default_stdlib()` (`$YAVS_STDLIB`, else the built-in copy under `stdlib::EMBEDDED`) | `src/lib.rs` |
| Built-in std/ieee: `build.rs` embeds `lib/std` and `lib/ieee` (`*.vhdl`); `stdlib::read`/`is_file` serve paths under `<yavs>/lib`, everything else from disk | `build.rs`, `src/stdlib.rs` |
| `Db::analyze_file` (one `yavs analyze` file, returns `UnitRecord`s), `analyze_closure`, `load_primary`, `load_body`; HIR validation in debug builds | `src/loader.rs` |
| Typed ids and `Arena` (`DeclId`, `ScopeId`, `ExprId`, `SubtypeId`, `TypeId`, `UnitId`, ...) | `src/ids.rs` |
| HIR shapes consumed by M2 (`Expr`/`ExprKind`, `Stmt`/`StmtKind`, `Call`, `Lsp`, `UnitHir`, `Binding`, `GenSelect`) | `src/hir.rs` |
| HIR invariants (sem.md 7.3) | `src/validate.rs` |
| Static folding, `ConstValue`, `Db::fold_with` (used by elab) | `src/stat.rs` |
| Generics, instances by HIR cloning, `Db::instance_body` | `src/generic.rs` |
| Units, configurations, `Db::default_binding` | `src/unit.rs`, `src/unit/binding.rs` |
| Codes and `--explain` texts (`codes!` macro, `EXPLAIN`) | `src/diag.rs` |
| Names, overloads, literals, aggregates | `src/expr/{names,cands,resolve,literal,aggregate}.rs` |
| Native symbols and mangling (`lib/NATIVE.md`) | `src/mangle.rs`, `src/implicit.rs` |
| Tests: negative suite runner, xfail list, std/ieee corpus, rule 1 dumps, reflection, attributes, hdl-modules corpus | `tests/diag_sem.rs`, `tests/diag_sem_xfail.txt`, `tests/stdlib.rs`, `tests/rule1.rs`, `tests/reflect.rs`, `tests/user_attributes.rs`, `tests/hdl_modules.rs` |
| `cargo run -p yavs-sem --example analyze -- [--std 2019] FILE...` | `examples/analyze.rs` |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| LRM policy: clear LRM wins and fails early; unclear: permissive, accept what NVC/GHDL/Questa/Riviera accept, warn only where they differ (user, 2026-10-18) | Most designs elaborate | `docs/design/sem.md` "LRM interpretation policy", `docs/design/lrm-checks.md` |
| Mixed 2008/2019 files: each file parsed with its own standard, design analyzed against the newest; illegal under it `E1007`, changed meaning `W1008` once per construct (user, 2026-10-18) | VUnit/OSVVM from 2019 testbenches | `docs/plan.md` "VUnit integration", sem.md 0.1 |
| One type/subtype representation, constraints as HIR expressions, folded values only a cache | Rule 1 (largest crash class) | `docs/plan.md` rule 1, sem.md 2.1, `docs/design/rule1-review.md` |
| Checked i64 static arithmetic; static overflow `E1401` where required, `W1401` elsewhere (run-time check kept) | Rule 7, Q2/Q5 | `docs/decisions.md` "Semantic analysis", sem.md 4 |
| Original spelling plus folded keys (`Designator::key()`) | Rule 8 | `docs/plan.md` rule 8, CONTRIBUTING "Names" |
| Cascade suppression with poison values; a failed file hides all its units in the session | No cascades; per-file results equal one `yavs analyze` per file | sem.md 8.2, `docs/decisions.md` "Semantic analysis" |
| Codes `E1xxx`/`W1xxx` with explain texts; one warning code per `--relaxed` relaxation (R1 `W1904`, R2 `W1830`) | Rule 13 | sem.md 8.1, 8.4 |
| Generic instance identity by mangling | Rule 4 | sem.md 5.3 |
| Default binding: directly visible entity wins over `work`, `W1703` when both; last architecture | Q4, GHDL #66 | `docs/decisions.md` "Design units and binding" |
| Formal packages with a generic map are checked against the actual (`E1503`) | LRM clear, Questa rejects | `docs/decisions.md` "Generics" |
| Incomplete types in protected method profiles only in library `std` (user, 2026-10-18, amends U-2) | `std.reflection` needs it | `docs/decisions.md` "Incomplete types in protected method profiles" |
| Pure calls with globally static actuals are globally static; port array attributes globally static | OSVVM, matches all simulators | `docs/decisions.md` "Interpreter: BIT/BOOLEAN codes and file writes" (last bullets) |
| Implicit vs explicit (any region): explicit wins when a call is otherwise ambiguous | OSVVM RandomBasePkg, UVVM rand_pkg | same section |
| An architecture's name is hidden by a port or generic of the same name (12.3: the name belongs to the library, outside the entity's region) | `architecture a of and2` with port `a`, NVC agrees | `src/unit.rs` (`hidden`) |
| Attribute specifications: a port or generic may be decorated in the architecture (permissive, L40); a label of the region's concurrent statements waits in `Db::pending_label_attrs` until `declare_label`, leftovers are `E1100` at the end of `conc_stmts` (L41) | surf, satcat5, vhdl-extras (#167) | `src/attr.rs`, `docs/design/lrm-checks.md` L40, L41 |
| Formal sub-element names (`d(w - 1 downto 0) =>`) are resolved in a one-name region holding the formal, inside the instantiating region (L42) | open-logic (#168) | `src/unit/binding.rs` (`partial`) |
| Array attributes of an object of a locally static subtype are locally static (L43); a part is resolved if any enclosing part's subtype is (L44) | poc (#169), neorv32, VUnit (#170) | `src/expr/names.rs`, `src/lsp.rs` |
| A conversion on an unconstrained formal is `E1312` (user, F8) | GHDL and Questa reject | `docs/design/rule1-review.md` F8 |
| `'stable`/`'quiet`/`'transaction`/`'delayed` in a subprogram: an error only of a formal signal parameter (4.2.2.2); of another signal the implicit signal is declared in the nearest region outside the subprogram | GHDL and Questa accept | `src/expr/names.rs` `build_attr` |
| Reflection: `simple_name` etc. upper case, 1-based physical unit indices, 0-based element/literal indices (user, 2026-10-18) | Match NVC; VUnit record_dict | `docs/decisions.md` "Reflection: names, indices and case" |
| 2019 files: signals and variables take missing bounds from the initial value (L20), ports of class variable of a protected type (L21), `open` sub-element actuals (L23); 2008 files keep the old errors | LCS-2016-019/047/001, NVC agrees | `docs/design/lrm-checks.md` L20, L21, L23 |
| A subprogram declared in an entity gets its body in an architecture; `E1230` is checked at the end of the architecture | 12.1: one declarative region | `docs/decisions.md` "Missing subprogram bodies" |
| `A'INDEX(n)` and `R'RECORD` are type marks (`attr.rs` `index_type_mark`, `range_record_type_mark`); one anonymous range record type per scalar base type, cached in `Db::range_records` | L12, L14 | `docs/design/lrm-checks.md` |
| Aggregate and case codes follow `tests/diag/sem/CODES.md`: mixed named/positional `E1326` (`E1324` retired), surplus positional record elements `E1322`, unknown record element `E1100`, one value for elements of different types `E1323`, `E1329` only for a choice that is not an element name; a named aggregate in a constrained context takes its bounds from its choices (L45) | The suite is the contract; the reference simulators agree | sem.md 3.4, `docs/design/lrm-checks.md` L45 |
| Unimplemented constructs report `Db::unsupported(span, what, "S<n>")` (`E9999`) | Fail early, no `todo!()` | CONTRIBUTING "Conventions" |

Module-local decisions go in this section and in the matching sem.md section, LRM readings in
`lrm-checks.md`, in the same commit (root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- **No orphan HIR nodes, no unresolved calls.** `validate.rs` runs in debug builds and every
  corpus test; a poison left behind without a diagnostic becomes an ICE there (fuzz found
  three, #25, #18).
- **Fail early.** Any error means `Err(Reported)`; never hand a partial HIR to elab. Never
  `unwrap_or(default)` on design data; `expect` only for yavs's own invariants.
- **Determinism.** Diagnostics order must not depend on hash-map iteration; snapshots and the
  negative suite compare exact `(code, line, col)` sets.
- **xfail list.** A listed file that starts passing fails `diag_sem`: remove its line in the
  same commit (`YAVS_SEM_XFAIL_WRITE=1`, check the diff only removes lines). Any ICE fails the
  suite, xfail or not.
- **`hir.rs` shapes are an M2 contract:** a change needs a note in sem.md section 7 and a check of
  `yavs-lower`/`yavs-elab` (`cargo test --workspace`).
- **std/ieee path**: `default_stdlib()` is the only lookup, used by the CLI too: `$YAVS_STDLIB`,
  else the copy built into the binary (user, 2026-10-18), so a release is one executable. Read
  std/ieee files only through `stdlib::read`/`stdlib::is_file`, never `std::fs` directly. A
  change in `lib/` rebuilds yavs-sem (`rerun-if-changed`).
- **Revisions:** `db.rev` is the design revision, `db.file_rev()` the file's; declarations
  carry `since`.
- Scratch output under `~/.cache/rsim-sem`, never in the repo or a fixed `/tmp` name.

## 5. How to work here

```sh
CARGO_BUILD_JOBS=6 cargo test -p yavs-sem                         # unit, negative suite, stdlib, rule1
cargo test -p yavs-sem --test diag_sem                            # negative suite only
YAVS_SEM_XFAIL_WRITE=1 cargo test -p yavs-sem --test diag_sem     # regenerate the xfail list
YAVS_BLESS=1 cargo test -p yavs-sem --test diag_sem               # rewrite optional .stderr snapshots
YAVS_CORPUS=~/.cache/rsim-corpus cargo test -p yavs-sem --features corpus   # hdl-modules corpus
cargo run -p yavs-sem --example analyze -- --std 2019 f.vhd
tools/corpus/report.sh                                            # all analysis corpora, RESULTS.md
cd fuzz && cargo +nightly fuzz run sem                            # see fuzz/AGENTS.md
```

- New error: constant plus explain text in `src/diag.rs` (same `codes!` entry), a negative test
  `tests/diag/sem/<area>/<name>.vhd` with `-- expect:` headers (`tests/diag/sem/README.md`).
- Codes owned: `E1000`-`E1999`, `W1000`-`W1999` (ranges by area in sem.md 8.1), plus `E9999` for
  `unsupported`/`ice`.

## 6. Coordination

- **Claim your module** in `CONTRIBUTING.md` "Module map" in your first commit before editing;
  changes in another owner's module are small, additive, own commit, or a Request in
  `CONTRIBUTING.md` "Requests" (sections per requesting area, including "Requests from elab").
- Open issues (snapshot 2026-10-18, `gh issue list -R ru551n/yavs --label crate:sem`):
  false positives #23, #38, #157; bugs #18, #25, #27, #36, #41, #154,
  #155; features #19, #22, #24, #28, #29, #30, #31, #32, #33, #34, #35; negative-suite xfail
  groups #45 to #54; driver-analysis perf #42; names/units leftovers #44;
  OSVVM/UVVM suites #115. Older unlabeled ones: #1, #2, #3 (sem false positives found by the
  difftest), #20 (`--encoding` in analyze), #21 (empty mapped library).
