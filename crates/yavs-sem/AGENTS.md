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
  anything, or decide when to cache: `image.rs` only turns a settled `Db` into bytes and back (`docs/decisions.md` "Compilation cache"); the key, the files and the policy are `yavs-cache` and the CLI.
  Single-threaded by design.
- Depends on: `yavs-diag`, `yavs-syntax`, `yavs-lib` (`Libraries`, `UnitRecord`), `yavs-std`
  (`value::lex_abstract` for literals). Called by: `yavs` (analyze, elab), `yavs-lower`,
  `yavs-elab`, `fuzz/` (target `sem`).

## 2. Map

Full module table with owners: `CONTRIBUTING.md` "Module map". Entry points and core types:

| Item | File |
|---|---|
| `Db` (`release_analysis`: frees everything but the source map for a run that is done with the design, #409; arenas, `WellKnown`, `Poison`, `report`/`error`/`ice`/`unsupported`, `diagnostics`, `take_diagnostics`) | `src/db.rs` |
| `Options` (`rev`, `stdlib`, `libraries`, `relaxed`, `defines`), `LangRev = yavs_syntax::Std`, `default_stdlib()` (`$YAVS_STDLIB`, else the built-in copy under `stdlib::EMBEDDED`) | `src/lib.rs` |
| Built-in std/ieee: `build.rs` embeds `lib/std` and `lib/ieee` (`*.vhdl`); `stdlib::read`/`is_file` serve paths under `<yavs>/lib`, everything else from disk | `build.rs`, `src/stdlib.rs` |
| `Db::analyze_file` (one `yavs analyze` file, returns `UnitRecord`s), `analyze_closure`, `load_primary`, `load_body`; HIR validation in debug builds | `src/loader.rs` |
| Typed ids and `Arena` (`DeclId`, `ScopeId`, `ExprId`, `SubtypeId`, `TypeId`, `UnitId`, ...) | `src/ids.rs` |
| HIR shapes consumed by M2 (`Expr`/`ExprKind`, `Stmt`/`StmtKind`, `Call`, `Lsp`, `UnitHir`, `Binding`, `GenSelect`) | `src/hir.rs` |
| HIR invariants (sem.md 7.3) | `src/validate.rs` |
| `Db::to_image`, `from_image`, `settle`: the compilation cache image of a settled `Db` (serde + postcard, symbols through a per-image table) | `src/image.rs` |
| Static folding, `ConstValue`, `Db::fold_with` (used by elab) | `src/stat.rs` |
| Generics, instances by HIR cloning, `Db::instance_body`; for M2 `Db::package_regions` and `Db::decls_with_instances` (an instance's objects belong to the region declaring it, #101) | `src/generic.rs` |
| Group templates, groups, disconnection specifications (`DeclKind::GroupTemplate`/`Group`, `Db::disconnect_times`), `E1640`, `E1641`, `E1650`, `E1651` (#260) | `src/group.rs` |
| Packages and package bodies in declarative parts, `E1237` (#34) | `src/package.rs` |
| Units, configurations, `Db::default_binding` | `src/unit.rs`, `src/unit/binding.rs` |
| Codes and `--explain` texts (`codes!` macro, `EXPLAIN`) | `src/diag.rs` |
| Names, overloads, literals, aggregates | `src/expr/{names,cands,resolve,literal,aggregate}.rs` |
| Native symbols and mangling (`lib/NATIVE.md`) | `src/mangle.rs`, `src/implicit.rs` |
| Mixed 2008/2019 designs: `since` of std/ieee declarations (compared with the 2008 copy at load), `E1007` gate on names and use clauses, `W1008` for INTEGER bounds (sem.md 0.1, lrm-checks L52) | `src/rev.rs` |
| Tests: negative suite runner, xfail list, std/ieee corpus, rule 1 dumps, reflection, attributes, external-library corpus | `tests/diag_sem.rs`, `tests/diag_sem_xfail.txt`, `tests/stdlib.rs`, `tests/rule1.rs`, `tests/reflect.rs`, `tests/user_attributes.rs`, `tests/hdl_modules.rs` |
| `cargo run -p yavs-sem --example analyze -- [--std 2019] FILE...` | `examples/analyze.rs` |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| The parsed-file cache is keyed by (library, path): a file analyzed into several libraries is parsed once per library, because units are taken out of the cache when analyzed | #290 | `loader.rs` `parse_into_cache` |
| LRM policy: clear LRM wins and fails early; unclear: permissive, accept what the reference simulators accept, warn only where they differ (user, 2026-10-18) | Most designs elaborate | `docs/design/sem.md` "LRM interpretation policy", `docs/design/lrm-checks.md` |
| Mixed 2008/2019 files: each file parsed with its own standard, design analyzed against the newest; illegal under it `E1007`, changed meaning `W1008` once per construct (user, 2026-10-18) | External test runners and verification libraries from 2019 testbenches | `docs/plan.md` "Test-runner integration", sem.md 0.1 |
| One type/subtype representation, constraints as HIR expressions, folded values only a cache | Rule 1 (largest crash class) | `docs/plan.md` rule 1, sem.md 2.1, `docs/design/rule1-review.md` |
| Checked i64 static arithmetic; static overflow `E1401` where required, `W1401` elsewhere (run-time check kept) | Rule 7, Q2/Q5 | `docs/decisions.md` "Semantic analysis", sem.md 4 |
| Original spelling plus folded keys (`Designator::key()`) | Rule 8 | `docs/plan.md` rule 8, CONTRIBUTING "Names" |
| Cascade suppression with poison values; a failed file hides all its units in the session | No cascades; per-file results equal one `yavs analyze` per file | sem.md 8.2, `docs/decisions.md` "Semantic analysis" |
| Codes `E1xxx`/`W1xxx` with explain texts; one warning code per `--relaxed` relaxation (R1 `W1904`, R2 `W1830`) | Rule 13 | sem.md 8.1, 8.4 |
| Generic instance identity by mangling | Rule 4 | sem.md 5.3 |
| Default binding: directly visible entity, then the component declaration's library, then `work`; `W1703` when `work` has another; last architecture | Q4 | `docs/decisions.md` "Design units and binding" |
| Formal packages with a generic map are checked against the actual (`E1503`) | LRM clear, the third reference simulator rejects | `docs/decisions.md` "Generics" |
| Incomplete types in protected method profiles only in library `std` (user, 2026-10-18, amends U-2) | `std.reflection` needs it | `docs/decisions.md` "Incomplete types in protected method profiles" |
| 2019 method aliases in a protected type declaration forward to the private variable: the call's object is `ExprKind::PrivateVar` (`Db::forwarding`); an alias of a method of another object (shared variable, variable, by simple or expanded name) forwards to that object, the call's object an `Obj` of it (#32, #237) | 5.6.2, 6.6.3; the reference simulator agrees for private variables, the references differ on other objects (permissive) | sem.md 6.6, `docs/design/lrm-checks.md` L15 |
| Pure calls with globally static actuals are globally static; port array attributes globally static | Seen in external verification components; matches all simulators | `docs/decisions.md` "Interpreter: BIT/BOOLEAN codes and file writes" (last bullets) |
| Implicit vs explicit (any region): explicit wins when a call is otherwise ambiguous | Random packages of two verification libraries | same section |
| An architecture's name is hidden by a port or generic of the same name (12.3: the name belongs to the library, outside the entity's region) | `architecture a of and2` with port `a`, the reference simulator agrees | `src/unit.rs` (`hidden`) |
| Attribute specifications: a port or generic may be decorated in the architecture (permissive, L40); a label of the region's concurrent statements waits in `Db::pending_label_attrs` until `declare_label`, leftovers are `E1100` at the end of `conc_stmts` (L41) | External libraries (`surf`, `satcat5`, `vhdl-extras`; #167) | `src/attr.rs`, `docs/design/lrm-checks.md` L40, L41 |
| Constants, signals, generics, ports and constant/signal parameters of an access or file type are `E1223` in 2008 and 2019; constant parameters of such types are exempt only in library `std` (user, 2026-10-19: 2019 user code is rejected too, majority of the reference simulators); in 2008 protected methods also take no access or file result or parameter, 2019 accepts them (#212; user, 2026-10-19, rule 15, LCS-2016-004) | LRM clear, the reference simulators reject; the IEEE 2019 `std.env` source declares such parameters (#198) | `docs/design/lrm-checks.md` L46, `src/types.rs` `check_not_access_or_file` |
| Formal sub-element names (`d(w - 1 downto 0) =>`) are resolved in a one-name region holding the formal, inside the instantiating region (L42); in subprogram calls too, per overload candidate with fresh memos, the actual costed against the sub-element's type (`assoc_map` gives `Slot::Parts`) | open-logic (#168), an external test runner's verification components (#210) | `src/unit/binding.rs` (`partial`), `src/expr/names.rs` (`assoc_map`, `parts_cost`) |
| Array attributes of an object of a locally static subtype are locally static (L43); a part is resolved if any enclosing part's subtype is (L44) | External libraries (`poc`, `neorv32`, `vunit`; #169, #170) | `src/expr/names.rs`, `src/lsp.rs` |
| A conversion on a formal that is not fully constrained gives it its bounds when the conversion defines them, else `E1312` (F8 rejected every such conversion; user, 2026-10-19, accepts one whose type mark, subtype, or function parameter or result subtype is fully constrained): in the formal part the function's parameter subtype or the type mark, in the actual part the result subtype or the type mark (`binding.rs`, both checks; subprogram calls: `expr/names.rs`, lowering `converted_cells` takes the bounds from `formal_conv_subtype`); a type conversion actual's `Actual::Converted::actual_conv` is the type mark's own declaration (a subtype keeps its constraint) | LRM 5.3.2.2 (#262) | `docs/design/rule1-review.md` F8, `docs/design/lrm-checks.md` L87 |
| Conversions in subprogram calls (`conv(f) => conv(a)`): `assoc_map` gives `Slot::Conv`, costed per candidate with the formal part resolved in the formal's region; `Actual::Converted` carries `copy_in`/`copy_out` expressions; a conversion the class or mode does not allow is `E1313`, a formal conversion overloaded by result type only is `E1302` (#229) | LRM 6.5.7.1, 4.2.2.3; lowering reuses its expression code | `src/expr/names.rs` (`conv_actual`, `conv_cost`), `docs/design/lrm-checks.md` L66 |
| Port maps with both conversions (`conv(f) => conv(a)`, `inout`): the actual part is resolved against the formal's type, the formal part's result must be the actual's type (`E1301`) (#238) | LRM 6.5.7.1: each conversion maps one way | `src/unit/binding.rs` (`actual`), `docs/design/lrm-checks.md` L70 |
| `'stable`/`'quiet`/`'transaction`/`'delayed` in a subprogram: an error only of a formal signal parameter (4.2.2.2); of another signal the implicit signal is declared in the nearest region outside the subprogram; named in a loop or sequential block, in the enclosing process or concurrent region (#240) | The reference simulators accept | `src/expr/names.rs` `build_attr` |
| Reflection: `simple_name` etc. upper case, 1-based physical unit indices, 0-based element/literal indices (user, 2026-10-18) | Match the reference simulator; an external test runner's reflection package | `docs/decisions.md` "Reflection: names, indices and case" |
| 2019 files: signals and variables take missing bounds from the initial value (L20), ports of class variable of a protected type (L21), `open` sub-element actuals (L23); 2008 files keep the old errors | LCS-2016-019/047/001, the reference simulator agrees | `docs/design/lrm-checks.md` L20, L21, L23 |
| A subprogram declared in an entity gets its body in an architecture; `E1230` is checked at the end of the architecture | 12.1: one declarative region | `docs/decisions.md` "Missing subprogram bodies" |
| `A'INDEX(n)` and `R'RECORD` are type marks (`attr.rs` `index_type_mark`, `range_record_type_mark`); one anonymous range record type per scalar base type, cached in `Db::range_records`; `R'VALUE` is built as the aggregate `(left, right, direction)` from `'left`/`'right`/`'ascending` of the range (`range_value`, #282) | L12, L14 | `docs/design/lrm-checks.md` |
| Aggregate and case codes follow `tests/diag/sem/CODES.md`: mixed named/positional `E1326` (`E1324` retired), surplus positional record elements `E1322`, unknown record element `E1100`, one value for elements of different types `E1323`, `E1329` only for a choice that is not an element name; a named aggregate in a constrained context takes its bounds from its choices (L45); without one, array values under range choices give it their direction, a mix is `E1334` (L84, #230) | The suite is the contract; the reference simulators agree | sem.md 3.4, `docs/design/lrm-checks.md` L45 |
| Overload diagnostics: with one candidate the association error is reported (`E1100`, `E1310`, `E1311`; too many arguments `E1301` at the first extra one, functions and procedures alike; an object name actual of the wrong type `E1300` at that actual, an overloaded literal or call actual stays `E1301` at the callee, #48); an ambiguous procedure call whose candidates differ in one actual is `E1302` at that actual; help names types as the declaration wrote them (`Db::written_marks`) | Tests of #51; a precise span, user spelling (rule 8) | `src/expr/resolve.rs` (`assoc_mismatch`, `ambiguous_actual`) |
| Array attribute dimensions locally static (`E1410`) and existing (`E1623`) at the argument; a static index or non-null slice outside a static index range is `W1404` at the name (L51; user, 2026-10-19, rule 15: error only when executed) | sem.md 4.3 keeps the index check a warning since the code may be dead | `src/expr/names.rs` (`check_dimension`, `warn_static_index`), `docs/design/lrm-checks.md` L51 |
| Protected misuse is `E1905` (signal, constant, port, assignment target, operator operand: before `E1301`), a non-protected shared variable `E1904` (`W1904` under `--relaxed`); the objects are still declared so uses do not cascade (#52) | LRM 5.6.1, 6.4.2.4 | `src/protected.rs`, `docs/design/lrm-checks.md` L57 | A shared variable declared before the protected body in its own region (a package declaration) is `E1230` (#258, L83).
| 2019 conditional expressions: typed by context per alternative, static when all parts are; a conditional `return` and an assignment with `unaffected` become an HIR `If` (#33) | No new HIR shape for M2; precise errors per alternative | `src/expr/resolve.rs` (`conditional`), `src/stmt.rs` (`split_alts`), sem.md 6, `docs/design/lrm-checks.md` L60 |
| A subprogram header with a generic list and a generic map declares an instance (`generic.rs` `header_instance`, through the `is new` path); the uninstantiated subprogram stays undeclared, a body repeating the header completes it (`header_generic`); a non-object parameter is `E1232` (#227) | LRM 4.2.1 and 4.2.2.1 are clear; the reference simulators reject the calls | `docs/design/lrm-checks.md` L61 |
| 2019 named return values (`return r of t`): `r` is a `FromObject` subtype of a hidden constant (`Db::ret_names`); the call's `Expr::subtype` is its constrained context subtype, else `E1304`; scalar, array and record types and generic types checked per instance (`fill_scope`), a scalar `r`'s range is its hidden constant's (`scalar_range`), other types `E1233` (#33, #239, #327) | M2 passes the descriptor; no HIR shape change | `src/decl.rs` (`named_return`), `src/expr/resolve.rs` (`named_return_context`), `docs/design/lrm-checks.md` L62, L77 |
| An expanded name with a protected type prefix (`pt.n`, `pt.get`) is legal inside the type's declaration and body: the suffix is a declaration of that region (`protected_expanded`); outside it `E1103` (#250) | 8.3, 12.1: one declarative region; LRM clear, two of the reference simulators accept | `docs/design/lrm-checks.md` L74 |
| Packages in declarative parts (`DeclKind::Package`, `src/package.rs`): a `ScopeKind::Package` child of the declaring region, its body a `PackageBody` child of the package's region (`Db::body_package`), in the same region or, for a package in a package declaration, in its body (lookup continues in that body through `Scope::outer`); generic ones instantiate through the same path as library generic packages (#34) | LRM 4.7, 4.8, 12.1 (2008) | sem.md 5.3, `docs/design/lrm-checks.md` L81, L82 |
| Groups: the constituents match the template's entity class entries in order (`<>` last, zero constituents allowed for it); a label of the region's statements waits in `Db::pending_groups` until `declare_label` (leftovers `E1100` at the end of `conc_stmts`). Disconnection specifications name guarded signals of their own declarative part (not ports), the type mark has the signal's type, the time is globally static; `others` skips signals with a specification, `all` and a repeated name are `E1651` when one exists; the time is kept per signal in `Db::disconnect_times` (cloned with generic instances) and elab hands it to the kernel (#260) | LRM 6.9, 6.10, 7.4 clear; the reference simulators differ on `<>`, `others` and counts | `src/group.rs`, `docs/design/lrm-checks.md` L90, L91 |
| 2019 generic array types are `TypeKind::Array` declared by a `GenericType` declaration (no generic array class): array operations work inside the generic; anonymous index, element and designated types (`type is ...`) are unnamed generic types mapped with their owner (`anonymous_parts`); an index constraint needs a constrained actual (#30) | One array model for indexing, attributes and loops | sem.md 2.9, `docs/design/lrm-checks.md` L92 |
| 2019 objects with an anonymous type (`c : type is range <>`): one anonymous generic type per name (`anonymous_object_type`), bound to the type of the actual typed without context (`anonymous_actual`; universal values as INTEGER/REAL); entities, components and blocks append the types as hidden generic types (`Db::hidden_types`, `with_hidden_args`); a subprogram with anonymous parameter types (`Generic::implicit`) is instantiated per call (`implicit_instance`, once per region and types, in the region's `order` but not by name); a call inside its own body stays a call of it (#283); instances of generic subprograms keep unbound anonymous types and operators are instantiated per call too (`implicit_instance_of`); a formal subprogram's anonymous types are wildcards of their class (`profile_fits`, `GenericState::formal_anon`) (#308) | Reuses the generic type path of entities and instances | sem.md 2.9, `docs/design/lrm-checks.md` L94 |
| 2019 composites of protected types: record elements of protected types accepted in 2019 (`E1905` in 2008); `type_contains_protected` composites have no implicit operations (`implicit.rs` `shape`) and no assignment (`E1905`); a shared variable may be of a composite whose scalar subelements are all protected (`all_protected`); a variable port of one is `E9999` (#361) | LCS-2016-014, 014a; a record with a protected element and a non-protected one is `E1905` (5.3.1, #371) | `docs/design/lrm-checks.md` L102 |
| A 2019 call `f generic map (...) (args)` classifies the name as the instance of the one generic subprogram `f` denotes, made once per call span and region and kept in the region's `order` (`generic.rs` `call_instance`, #362) | LCS-2016-049; reuses `instantiate_subprogram` | `docs/design/lrm-checks.md` L103 |
| A 2019 generic map formal with a signature (`f [t return t] =>`) names the formals of that name whose profile matches (`generic.rs` `formals_with_signature`, used by `associate` and by `unit/binding.rs` `formal_of`); its type marks may name a generic type of the same list (#363) | LCS-2016-I03; the references accept that | `docs/design/lrm-checks.md` L104 |
| Unimplemented constructs report `Db::unsupported(span, what, "S<n>")` (`E9999`) | Fail early, no `todo!()` | CONTRIBUTING "Conventions" |

Module-local decisions go in this section and in the matching sem.md section, LRM readings in
`lrm-checks.md`, in the same commit (root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- **No orphan HIR nodes, no unresolved calls.** `validate.rs` runs in debug builds and every
  corpus test; a poison left behind without a diagnostic becomes an ICE there (fuzz found
  three, #25, #18).
- **`report_name`** never returns without a diagnostic: pass it the name's span and what the
  caller needed; `One`/`Overloads` are the caller's to handle (reaching it is an ICE).
- **Fail early.** Any error means `Err(Reported)`; never hand a partial HIR to elab. Never
  `unwrap_or(default)` on design data; `expect` only for yavs's own invariants.
- **`Db` is serialized (`image.rs`).** Every type reachable from a `Db` field derives `serde::Serialize`/`Deserialize` (no `&'static str`, no references, no trait objects, no interior pointers). A new `Db` field is either serialized (its type derives) or `#[serde(skip)]` because it is transient: then `Db::settle` clears it and `to_image` must refuse an image while it is non-empty. State that survives `analyze_closure` and is read later by elab or lowering must be serialized, or a warm run differs from a cold one (`crates/yavs/tests/cache.rs`, the difftest corpus). Library sources (manifests) are never serialized.
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
YAVS_CORPUS=~/.cache/rsim-corpus cargo test -p yavs-sem --features corpus   # external-library corpus
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
  false positives #23, #38, #157; bugs #25, #27, #36, #41, #154,
  #155; features #19, #22, #24, #28, #29, #30, #31, #32, #33, #34, #35; negative-suite xfail
  groups #45 to #54; driver-analysis perf #42; names/units leftovers #44;
  external verification library suites #115. Older unlabeled ones: #1, #2, #3 (sem false positives found by the
  difftest), #21 (empty mapped library).
