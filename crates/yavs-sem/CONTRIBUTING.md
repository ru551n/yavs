# Working on yavs-sem

`yavs-sem` is M1, semantic analysis. The binding design is `docs/design/sem.md`; read it first,
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
| `unit.rs` | design units, context clauses, implicit context; configurations (6.5) | S1 (context), S5 (entity/arch/config) | owner: units (S1 parts by M1 lead are done); sem08: corpus false positives #167-#170; ctxlib: context-reference libraries (#36); semuse: use clauses and visibility #54 |
| `scope.rs` | regions, `declare`, `lookup`, use clauses, visibility (1.4), undeclared-name reporting | S1 | M1 lead; semuse: use clauses and visibility #54 |
| `decl.rs` | `Decl`/`DeclKind`/`Object`/`Subprogram`/`Body`/`Alias`/`Component`; declarative item analysis | S1 (objects, subprogram specs), S2 (bodies) | M1 lead; semstatic: deferred constants outside packages #53; s7left: named return values #33; subhdr: subprogram header generic maps and non-object parameters in `subprogram_spec` #227; constnoval: constants without a value outside packages and missing full declarations #213 |
| `types.rs` | types, subtypes, constraints, constrainedness, type/subtype declarations | S1 (scalars), S3 (composites, constraint model) | M1 lead (S1); semdrv: range attribute prefix diagnostics #47 |
| `implicit.rs` | implicitly declared operations per type (2.10) | S2 | owner: implicit |
| `expr/cands.rs`, `expr/resolve.rs` | two-pass overload resolution (3.1, 3.2, 3.6, 3.7) | S2 | M1 lead; semtyping: expression typing, conversions, conditions, parameter class checks #48; indivassoc: sub-element associations in calls #210; s7left: 2019 conditional expressions #33 |
| `expr/names.rs` | `Apply`/`Selected`/`Attribute` interpretation (1.5, 3.3) | S2 | owner: names (core by M1 lead: classify, name_cands, build_name, calls, index/slice/field, attributes); sem08: corpus false positives #167-#170; semuse: expanded names into constructs #54; semtyping: type conversions (9.3.6) #48; indivassoc: sub-element associations in calls #210; semstatic: attribute dimension, static index and slice checks #53 |
| `expr/literal.rs` | literals, bit-string expansion (3.5, Q11) | S2 | owner: literal; semext: negative suite #49 |
| `expr/aggregate.rs` | aggregates and choice sets (3.4), shared with `case` | S3 | owner: aggregate; semaggr: negative suite #45 |
| `stat.rs` | staticness, `ConstValue`, folding (section 4) | S1 (scalar folding), S3 | M1 lead (S1) |
| `attr.rs` | predefined attribute table, user attributes (6.3) | S3, S6 | owner: aggregate; sem08: corpus false positives #167-#170 (user attribute specifications) |
| `generic.rs` | generic classes, `Instance`, `GenericMap`, cloning (section 5); 2008 `type T` has implicit `=`, `/=` (Q9, L18) | S4 | owner: generic; gentypes: concurrent statements in the clone, `seed_type` (#75); subhdr: header generic maps through `subprogram_instance` (#227) |
| `generic/entity.rs` | design entities with generic types, subprograms or packages cloned per map (`Db::instantiate_entity`, 5.3) | S4 | owner: gentypes (#75) |
| `mangle.rs` | mangled names (5.3), NATIVE.md symbols (2.10) | S2, S4 | owner: implicit |
| `stmt.rs` | sequential (S2) and concurrent (S5) statements | S2, S5 | owner: stmt; sem08: corpus false positives #167-#170; s7left: `unaffected` and conditional `return` #33 |
| `lsp.rs` | longest static prefixes, drivers, `all` (6.9) | S5 | owner: stmt; sem08: corpus false positives #167-#170 |
| `alias.rs` | aliases, signatures (6.2, 6.4) | S2 (std aliases), S6 | owner: s6; semalias: alias negative suite #46 |
| `external.rs` | external names (6.1) | S6 | owner: s6 |
| `protected.rs` | protected types and bodies (6.6) | S6 | owner: s6; protacc: #212 method profile check (small, in `decl.rs` `subprogram_spec`); semprot: protected and `--relaxed` negative suite #52 (wiring in `decl.rs` `object_decl`, `stmt.rs` variable assignment, `resolve.rs` operator errors) |
| `alloc.rs` | allocators, dereference (`.all`, implicit), incomplete type uses, `'designated_subtype` (2.6, 3.7) | S6 | owner: alloc |
| `view.rs` | mode views, `'converse` (6.7) | S7 | owner: s6 |
| `rev.rs` | mixed 2008/2019 designs: `since` of std/ieee declarations, `E1007`, `W1008` (0.1) | S7 | owner: semrev (#50) |
| `reflect.rs` | 2019 `'reflect`, `std.reflection` mirrors (2.11) | S7 | owner: reflect |
| `hir.rs` | HIR node shapes M2 consumes (section 7) | all | shapes fixed; changes need a note in sem.md |
| `validate.rs` | HIR invariants (7.3) | S2 onward | owner: literal |
| `diag.rs` | codes (8.1) and `--explain` texts | all | add codes additively |
| `suggest.rs` | did-you-mean (8.3) | S1 | done |

To take a module, edit its row in your first commit (`owner: <short name>`), push, then work.
If you need a function in a module someone else owns, add it in a small separate commit, or ask
through the orchestrator.

## Conventions

- **Fail early, fail clearly.** Every problem is a `yavs_diag::Diagnostic` reported through
  `Db::report`/`Db::error`, with a code, primary span, labels, notes and help. Never
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
- **Internal errors.** A broken invariant is `Db::ice(span, what)` ("this is a bug in yavs"), not
  a panic. `expect` is only for invariants of yavs's own data, with a message.
- **Names (rule 8).** Keep `yavs_syntax::ast::Ident` (original `text`, folded `key`). Look up by
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

- `tests/diag_sem.rs` runs the negative suite `tests/diag/sem` (contract in its README.md):
  exact `(code, line, col)` sets. Files that cannot pass yet are listed in
  `tests/diag_sem_xfail.txt`; a listed file that starts passing fails the test, so delete its
  line in the same commit (`YAVS_SEM_XFAIL_WRITE=1 cargo test -p yavs-sem --test diag_sem`
  regenerates the list; check the diff only removes lines).
- `cargo run -p yavs-sem --example analyze -- [--std 2019] FILE...` prints the diagnostics.
- `cargo test -p yavs-sem`. The positive corpus test analyzes the vendored `lib/` tree
  (`tests/stdlib.rs`); extend its list as slices land (sem.md 10, "green when").
- Outputs and scratch files go to `~/.cache/rsim-sem`, never into the repo.

## Requests

From sem robustness (fuzzing, `fuzz/fuzz_targets/sem.rs`):

- **names** (expr/names.rs): done (#18). `report_name(scope, r, span, what)` reports an object part
  (`NameRes::Value`) as `E1300` "expected `what`" and ends in `Db::ice` for `One`/`Overloads`,
  which every caller must handle itself. Regression tests: `tests/diag/sem/fuzz/report_name_*.vhd`.
- **lead** (decl.rs): aliases of indexed and sliced object names (`alias a : t is v(3)`) are
  `E9999` (S3), selected ones (`alias a : t is rec.f`) too; the corpus (an external verification library's utility package) needs both.

From s6 (alias/external/protected/view). The S6/S7 checks exist as `Db` methods with unit tests;
they need calls from the analyzers that own the constructs:

- **decl.rs**: alias declarations: `object_alias` when the name denotes an object (or a subtype
  indication is written), else `nonobject_alias` with the lookup result and the resolved
  `alias::Signature`, then `declare` every returned decl; in a protected type declaration also
  `check_protected_alias`. Shared variables: `check_shared_variable`. Signals, ports and
  constants: `check_not_protected`; parameters: `check_protected_param`. Protected types:
  `check_protected_decl_items` / `check_protected_body_items` on the AST, `pair_protected_body`
  (the body region's parent becomes the declaration region, so a method body must complete the
  spec found there), `check_protected_complete` at `end protected body`, `check_protected_bodies`
  at the end of each region (for a package: at the end of its body). Views: `declare_view`, and
  `view_port` for `p : view v [of t]` interface objects.
- **expr/names.rs**: `external_name` for `NameKind::External`; `protected_member` for selection
  from a protected object (E1100 missing, E1105 private); `match_signature` for signatures in
  attribute specifications, generic subprogram actuals and `use`; `view_ref` for view names
  (`'converse` sets `converse: true`).
- **types.rs** (ruling U-2): incomplete types are allowed in method profiles of a protected type
  declaration in both 2008 and 2019, only in library `std` (`Db::protected_of_region` / `ScopeKind::Protected` tells the
  region).

From implicit (implicit.rs, mangle.rs):

- implicit: type declaration analysis should call `Db::declare_implicit_ops(ty, scope, span)`
  right after `declare` of each full type declaration (and after setting the matching
  `well_known` field in `std.standard`); for `universal_integer`/`universal_real`, call it in
  `begin_standard` with the standard scope once their `TypeDef`s exist. Operations naming a type
  not declared yet wait in `pending_implicit` and are created when it appears.
- implicit: `TypeDef::base_subtype` of a constrained array type (`type a is array (0 to 7) of
  bit`) must be an unconstrained subtype of the anonymous base: implicit operators use it for
  their operands and results (`a & a` is not `0 to 7`).
- implicit: explicit bodiless std subprograms get `Body::Native(mangle::native_symbol(pkg,
  protected_type, spec))`; `tests/native_symbols.rs` pins the rule against NATIVE.md (235 rows).
- implicit: range records (2019 `R'RECORD`, U-3) are not created yet; they need the U-3 decision
  on the `DIRECTION` name and belong with attribute analysis.

## Wiring status (M1 lead)

Done in decl.rs/types.rs: alias declarations (`object_alias`, `nonobject_alias`, signatures via
`Db::signature_of`), `check_shared_variable`, `check_not_protected` (signals, constants),
`check_protected_param`, protected type declarations and bodies (`check_protected_decl_items`,
`check_protected_alias`, `pair_protected_body`, `check_protected_body_items`,
`check_protected_complete`), `check_protected_bodies` at the end of every region except package
and protected declarations (packages: at the end of the package body). U-2 needs no check:
type marks accept incomplete types everywhere for now.

Hooks the M1 lead calls; the module owner fills the body (keep the signature):

- **stmt.rs** `Db::seq_stmts(&mut self, scope: ScopeId, stmts: &[ast::SeqStmt]) -> Box<[StmtId]>`:
  called for subprogram bodies with the subprogram's parameter region (its `owner` is the
  subprogram decl). Currently a stub reporting `E9999`.
- **implicit.rs** `declare_implicit_ops(ty, scope)` after every type declaration (also the
  universal types at the start of `std.standard`), `flush_pending_implicit()` after every
  `std.standard` type and subtype.

Protected method calls are wired (reflect): `obj.m` / `obj.m(args)` in names.rs
(`Via::Method`, `method_call_cands`, `method_object`) and in `proc_call`, through
`Db::method_target` / `Db::method_decls` (protected.rs, quiet) and `protected_member` (reports).
An access to a protected type is dereferenced implicitly (`ExprKind::Deref`, built inline in
`method_object`; alloc: switch it to `Db::deref` once that lands).

Wiring still open (M1 lead will do these in expr/names.rs unless someone asks for them):
`external_name` (`NameKind::External`), `view_ref`, `match_signature` in `use`. Done:
calls inside a protected body use the implicit object (`method_of: None`), `match_signature`
in attribute specifications, the aggregate resolver. Linkage ports: reads are E1360 through
`check_linkage_reads` from stmt.rs's `st_expr`/`st_target` (names).

From aggregate (expr/aggregate.rs, attr.rs). The analyses exist with unit tests; they need calls:

- **expr/resolve.rs**: implement `aggregate::AggResolver` (cands, resolve, discrete_range,
  type_mark) and call `resolve_aggregate(db, r, assocs, ctx_subtype, span)` for an aggregate with
  `Expected::Type`; pass the context *subtype* (an `others` needs the constrained one), and
  `aggregate::no_context` (E1320) for `Expected::Any`. Nested aggregates in elements are handled
  inside, with the element subtype as context.
- Done (names): `attr::Prefix`/`attr::applies`/`attr::select`/`result_subtype` in names.rs,
  user attributes `x'a` through `attr::user_value`; decl.rs through `declare_attribute` and
  `specify` (lead). Open: `attr::implicit_signal` is not called yet (`'delayed` and friends are
  plain `ExprKind::Attr` nodes).

### stmt -> lead (expr, decl, unit)

`stmt.rs` reaches other modules through the `st_*` hooks at its bottom. Wired: `st_expr`
(`expr`), `st_name` (`name_cands` + `build_name`), `st_discrete_range` (`discrete_range`),
`st_decls` (`decls_in`), `st_case_choices` (aggregate.rs `resolve_choices` + `check_case`),
`st_lookup_guard` (`lookup`), labels and parameters through `declare`. Still `E9999`, wanted:

| Hook | Wanted | Owner |
|---|---|---|
| `st_proc_call(scope, &ast::Name)` | procedure call: overload with procedure result, `hir::Call` | expr |

- Done (units): `st_interfaces`, `st_assoc`, `st_inst_target` call `unit/binding.rs`
  (`interface_list`, `assoc_list`, `inst_target`); `l : c;` parsed as a procedure call
  instantiates when `c` is not a subprogram; entities and architectures run
  `analyze_conc_stmts`.
- Loop parameters must have `stat: None` and generate parameters `Globally` on their `Obj`
  expressions: the longest static prefix depends on it (lsp.rs).
- `seq_stmts(scope, stmts)` (decl.rs's hook) is implemented; nested bodies inherit `wait` and
  driver rules through `Db::stmt_owners`, so analyze bodies declared in a process while it is open
  (they are: `st_decls` runs inside the process).

## Requests from the M1 lead

- **stmt**: resolve values that have a target subtype with
  `Db::expr_in_subtype(scope, e, target_subtype)` (variable and signal assignment values,
  `return` values with the function's return subtype), so `x := (others => '0')` takes its
  bounds from `x` (E1325 otherwise; std_logic_1164 and numeric_std bodies hit this).
- **names**: `.all` and implicit dereference of access values (`L.all'length`, `p.field`,
  `p(i)` with `p` an access): classify returns `Unsupported` for `.all` today; std_logic_1164's
  body needs it.
- **stmt** uses `Db::proc_call(scope, name)` (resolve.rs) through `st_proc_call` (done).

From reflect (reflect.rs):

- **generic**: `to_dict generic (type record_t) parameter (value : record_t)` (an external test runner's
  reflection package) needs generic subprogram declarations, bodies and `function f is new f generic
  map (record_t => t)` instances overloaded under one name. `value'reflect` is an ordinary
  `ExprKind::Attr` with `AttrPrefix::Expr`; the instance clone must remap that prefix (and with it
  the reflected subtype) like any other attribute, nothing reflection-specific.
- **M3 (runtime)**: open question for the user: the unit of `PHYSICAL_VALUE_MIRROR.value`.
  record_dict_pkg multiplies it by `std.env.resolution_limit` (a commercial simulator); the reference simulator returns fs.

From alloc (alloc.rs):

- **expr/resolve.rs** `proc_call` replaces the call expression with `ExprKind::Error` after taking
  its arguments; the debug HIR validation reports that as an internal error for every procedure
  call statement (so `deallocate(p)` cannot have a negative-suite test yet).
- **U-2**: the reference simulators 2026.1 all reject incomplete types in protected method profiles in
  user code (`tests/diag/sem/NVC.md`); done: the ruling is limited to `std`.
- ponytail: allocators do not filter overloads by designated type (`f(new integer)` with `f`
  overloaded on two access types is `E1302`), as `null` is.

From cli (`yavs analyze`):

- **loader.rs** a `--map`ped library without `yavs-lib.toml` is an empty library (cli-contract
  section 3), but loading it reports `L0001 library not found`. Request: treat a missing manifest
  as an empty library, so the use is `E1002`/`E1004` (unit not found), not `L0001`.
- **std/ieee as work**: `tools/corpus/yavs-lib-*` analyzes lib/std and lib/ieee themselves with
  `--work=std:...`, which the CLI rejects (E0102, contract: yavs resolves them). The stdlib corpus
  needs a bootstrap mode from the sem owners (a `Db` mode that analyzes std.standard as work)
  before the CLI can offer one.

## Units (unit.rs, unit/binding.rs)

- Entities, architectures, configuration
  specifications (`config_spec`; stored in `Db::config_specs` by region once the statement
  part is analyzed) and configuration declarations (`UnitHir::Configuration`) are analyzed.
- `Db::default_binding(scope, component)` is the 7.3.3 rule for elab (Q4), with `scope` from
  `StmtKind::Instance::scope`: directly visible
  entity first (`W1703` at the instance when `work` has a different one), last architecture in
  manifest or analysis order.
- `Unit::refs` are manifest dependencies on units not loaded during analysis (an instance's
  named architecture, the default-bound entity of a component); `unit_record` adds them.
- Generic types, subprograms and packages on entities, components and blocks report `E9999`
  (S4, generic); mode view ports report `E9999` (S7); a subtype indication as a generate
  specification (`for g(natural range 1 to 2)`) reports `E9999`.
- An entity and its architecture are one region (12.1): `declare` in an architecture checks the
  entity's names (`E1101`).

### generic -> others

generic.rs (S4) is in. What other modules can use, and what is still open:

- `Db::generic_list(scope, &[InterfaceDecl])` analyzes a generic clause (constants, types with
  their class operations, subprograms with `is <>`/`is name` defaults, formal packages with
  `(<>)`, `(default)` or a generic map). `interface_list` routes non-object generics of components
  and blocks through `generic_item`.
- `Db::package_region(decl)` is the one place that maps a package, package instance or formal
  package decl to its region (`E1505` for an uninstantiated generic package); unit.rs `region_of`
  and names.rs `classify` call it. `NameRes::Uninstantiated` reports through `report_name`.
- `Db::instance_body(inst)` clones a package instance's body (or an instantiated subprogram's)
  on demand and gives the instance's subprograms their `Body::Hir`; M2 calls it. Package and
  subprogram instances inside generic units are cloned with them.
- `W1501`: `Db::lookup` marks generics used; reported after the package body (or the package
  declaration when there is none) and after a generic subprogram body.
- Generic protected types (2019): generics analyzed opaquely, visible in the body; an object of
  one is `E1505`. Request (parser): `generic_map_aspect` on variable declarations (2019 6.4.2.4),
  then generic.rs instantiates the type by cloning; a generic map in the protected type header is
  `E9999` until then.
- Open: anonymous generic types (`type is private` in an interface object, S7), mode views inside
  generic units (views are not analyzed yet), formal packages with a map stay opaque inside the
  generic (their generics do not take the map's values).
- lib: the bodies of `fixed_generic_pkg` and `float_generic_pkg` and the instance bodies of
  `fixed_pkg`/`float_pkg` analyze clean (`tests/stdlib.rs`, #37).

## Requests from elab (yavs-elab)

Corpus groups `tests/corpus/hierarchy` and `generics` (difftest) stop in analysis:

- **units/names**: `architecture a of and2 is begin y <= a and b; end;` with a port `a`: E1361
  "this name does not denote a value" at `a` (the architecture's own name hides the port).
  `hierarchy/component_binding.vhd`, `hierarchy/nested_instances.vhd`.
- **stmt/aggregate**: `o <= (others => '1')` for a port `o` of an unconstrained subtype (bounds
  from the actual at elaboration, 9.3.3.3; the reference simulator accepts): E1325. `generics/generic_map.vhd`. The
  same E1325 for a concurrent assignment to a constrained `std_logic_vector` signal,
  `s <= (others => 'Z');`.
- **generic**: `generics/generic_types_2019.vhd`: generic subprogram `img` of a generic type `t`
  called with `a : t` is E1301; `port map (1, 2)` for ports of type `t` with `t => integer` is
  E1300.
- **units**: for elaboration of `for` generates under a configuration, `GenSelect::Index`
  expressions are evaluated without the generate's scope (fine while they are locally static).
- **generic** (#75, done): entities with generic types, subprograms or packages.
  `Db::instantiate_entity(entity, arch, actuals, at)` (`generic/entity.rs`) clones the entity
  and architecture once per distinct map of those actuals (generic constants stay generics of
  the clone); elab calls it per instance. Defaults of entity generic subprograms are
  resolved at the instance (`binding.rs` `subprogram_default`, `E1504`). Open: configuration
  specifications inside such an architecture (`E9999`).
