# Working on yavs-sem

`yavs-sem` is M1, semantic analysis. The binding design is `docs/design/sem.md`; read it first,
plus `docs/plan.md` (error motto, rules 1, 7, 8, 12, 13) and `docs/decisions.md`. Several agents
work on this crate at the same time, so each takes a module, marks it below, and keeps edits to
other modules small and additive.

## Module map

One file per area. "Slice" is the sem.md section 10 slice that fills it.

| Module | Area | Slice | Status / owner |
|---|---|---|---|
| `ids.rs` | typed ids, `Arena` | S1 | done; sem3perf: cold-analysis performance (#442) |
| `db.rs` | `Db`, arenas, `WellKnown`, `Poison`, diagnostics helpers (`error`, `ice`, `unsupported`) | S1 | done; add fields additively |
| `image.rs` | `Db::to_image`/`from_image`/`settle`: the settled `Db` as bytes for the compilation cache (serde on every arena type; a new `Db` field needs `#[serde(skip)]` and an entry in `to_image`'s pending check, or a derive) | #407 | cache |
| `units.rs`, `units/canon.rs` | the analysed-unit cache: recording a unit analysed from source, writing its entry, planning and replaying it, undo; `Db::differs_from` | #537 | owner: unitcache (#537, #539); cold4: `--relaxed` as a recorded question (`Db::relaxed`, #740, hooks in `stmt.rs`, `decl.rs`, `protected.rs`); also small hooks in `loader.rs` (`load_unit`, `parse_into_cache`, `library`, `analyze_file_hashed`, `unit_known`), `ids.rs` (`Reloc`, owner runs, `Watch`), `db.rs` (fields) and the library queries of `unit.rs`, `scope.rs`, `generic.rs`, `unit/binding.rs` |
| `loader.rs` | libraries, `UnitKey`/`UnitSlot`/`Unit`, file cache, on-demand loader, `analyze_file`, `UnitRecord`s | S1 | M1 lead; fix7: unit order within a file and units declared twice (`later_in_file`, `dup_in_file`, #810) |
| `unit.rs` | design units, context clauses, implicit context; configurations (6.5) | S1 (context), S5 (entity/arch/config) | owner: units (S1 parts by M1 lead are done); sem08: corpus false positives #167-#170; ctxlib: context-reference libraries (#36); semuse: use clauses and visibility #54; gencomp: `default_map`/`binds_by_name` for generic types, subprograms and packages, `instantiate_blocks` call (#222); decisions1: `subprogram_actual` rejects component and block generic subprograms (`E1506`); varports: variable port actuals (`open`, unassociated, `inertial`, conversions) in `unit/binding.rs` (#188); portleft: `inertial` parts of individual associations (#261), conversions of unconstrained ports (#262) in `unit/binding.rs`; maki: `in` actual of a written formal #654; fix4: the same for parts, conversions in parts (#654, #611); fix6: 1993 buffer port associations in `unit/binding.rs` (#814) |
| `scope.rs` | regions, `declare`, `lookup`, use clauses, visibility (1.4), undeclared-name reporting | S1 | M1 lead; semuse: use clauses and visibility #54; bodyredecl: package body homographs #232; sem3perf: cold-analysis performance, lookup forwarding and cache (#442); maki: constant vs subprogram use-clause conflict #628 |
| `decl.rs` | `Decl`/`DeclKind`/`Object`/`Subprogram`/`Body`/`Alias`/`Component`; declarative item analysis | S1 (objects, subprogram specs), S2 (bodies) | M1 lead; semstatic: deferred constants outside packages #53; s7left: named return values #33; subhdr: subprogram header generic maps and non-object parameters in `subprogram_spec` #227 (done); constnoval: constants without a value outside packages and missing full declarations #213; extuse: aliases of external names #19; namedret: named return values of records, generic subprograms and protected methods #239; portiface: non-signal interfaces in `interface_list` #236; fix6: deferred constant early uses and conformance (`check_deferred_uses`, `check_full_conforms`, #809), waiting procedures called by functions (#823); fix7: deferred constant uses in the package body (`check_deferred_read`, #846); fix7: body conformance (`check_conforms`), variable and `bus` parameters (#808) |
| `types.rs` | types, subtypes, constraints, constrainedness, type/subtype declarations | S1 (scalars), S3 (composites, constraint model) | M1 lead (S1); semdrv: range attribute prefix diagnostics #47; maki: index constraint on an access subtype #643; fix6: type mark as a type definition range (#822), 1993 constrained elements (#814); fix7: file elements, files of multidimensional arrays (#813) |
| `implicit.rs` | implicitly declared operations per type (2.10) | S2 | owner: implicit |
| `expr/cands.rs`, `expr/resolve.rs` | two-pass overload resolution (3.1, 3.2, 3.6, 3.7) | S2 | M1 lead; out93: reads of `out` ports and parameters in 1993 files (#629, L138): `Db::out_reads`, `not_read`, `not_read_actual`, `report_out_reads` here, the hooks in `expr/names.rs`, `types.rs`, `alias.rs`, `stmt.rs`, `unit.rs`, `unit/binding.rs`; semtyping: expression typing, conversions, conditions, parameter class checks #48; indivassoc: sub-element associations in calls #210; s7left: 2019 conditional expressions #33; callconv: conversions in call associations #229; sem3perf: candidate sets without heap, operator candidate filtering (#442); fix6: composite element values in `check_in_subtype` (`elem_outside`, #821), attribute type mismatch (#822) |
| `expr/names.rs` | `Apply`/`Selected`/`Attribute` interpretation (1.5, 3.3) | S2 | owner: names (core by M1 lead: classify, name_cands, build_name, calls, index/slice/field, attributes); sem08: corpus false positives #167-#170; semuse: expanded names into constructs #54; semtyping: type conversions (9.3.6) #48; indivassoc: sub-element associations in calls #210; semstatic: attribute dimension, static index and slice checks #53; callconv: conversions in call associations #229; blockshv: protected type prefixes of expanded names #250; maki: indexing/slicing attribute values #653, #657, #658; fix6: types as expanded-name prefixes (#822); fix7: attributes of `linkage` ports, static signal actuals, 1993 closely related arrays (#813); fix7: attributes of a range attribute (#818); fix8: own package as expanded-name prefix (#854) |
| `expr/literal.rs` | literals, bit-string expansion (3.5, Q11) | S2 | owner: literal; semext: negative suite #49 |
| `expr/aggregate.rs` | aggregates and choice sets (3.4), shared with `case` | S3 | owner: aggregate; semaggr: negative suite #45; fix6: 1993 array pieces (#814); fix7: null range choices (#812) |
| `edge.rs` | clock-edge shapes of the HIR (`Db::edge_shape`, `level_of`, `same_name`), shared by the elaborator's edge triggers and the lowering (#632) | M2 | owner: edge632 (also small hooks in `expr/resolve.rs` `not_stable_as_event` and `attr.rs` `discard_implicit_signal`) |
| `stat.rs` | staticness, `ConstValue`, folding (section 4) | S1 (scalar folding), S3 | M1 lead (S1) |
| `attr.rs` | predefined attribute table, user attributes (6.3) | S3, S6 | owner: aggregate; sem08: corpus false positives #167-#170 (user attribute specifications) |
| `generic.rs` | generic classes, `Instance`, `GenericMap`, cloning (section 5); 2008 `type T` has implicit `=`, `/=` (Q9, L18) | S4 | owner: generic; gentypes: concurrent statements in the clone, `seed_type` (#75); subhdr: header generic maps through `subprogram_instance` (#227, done); namedret: named return values in instances #239; pkggen: `instance_regions`, `package_regions`, `decls_with_instances` for M2 (#101); genarr: generic array types as `TypeKind::Array`, anonymous index/element/designated types, index constraints (`generic_type`, `check_class`, `anonymous_parts`; #30); anonobj: anonymous types of generic constants, ports and parameters (`bind_const`, `implicit_instance`; #283); v2019c: generic maps in calls (`call_instance`, #362), signatures in formals (`formals_with_signature`, #363) |
| `group.rs` | group templates, groups (6.9, 6.10), disconnection specifications (7.4) | S5, S6 | owner: groups (#260) |
| `package.rs` | packages and package bodies in declarative parts (`DeclKind::Package`, 4.7), `E1234` | S4 | owner: nestgen (#34); fix6: 1993 nested packages (#814) |
| `generic/entity.rs` | design entities with generic types, subprograms or packages cloned per map (`Db::instantiate_entity`, 5.3) | S4 | owner: gentypes (#75); gencomp: components, blocks, cloned configuration specifications (#222) |
| `mangle.rs` | mangled names (5.3), NATIVE.md symbols (2.10) | S2, S4 | owner: implicit |
| `stmt.rs` | sequential (S2) and concurrent (S5) statements | S2, S5 | owner: stmt; sem08: corpus false positives #167-#170; s7left: `unaffected` and conditional `return` #33; gencomp: generic blocks queued for `instantiate_blocks` (#222); namedret: `return when c;` #239; extsub: `E1810` for external signal targets (#255); fix6: `check_calls_wait` for functions, `procedure_waits` (#823); fix7: locally static aggregate target elements, guarded targets (#812) |
| `lsp.rs` | longest static prefixes, drivers, `all` (6.9) | S5 | owner: stmt; sem08: corpus false positives #167-#170 |
| `alias.rs` | aliases, signatures (6.2, 6.4) | S2 (std aliases), S6 | owner: s6; semalias: alias negative suite #46; fix6: alias length and 1993 multidimensional aliases (#813, #814) |
| `external.rs` | external names (6.1) | S6 | owner: s6; extseq: implicit aliases of external names used directly (`alias_external_uses`, #93); extsub: subprogram owners (`ExtOwner::Sub`), package paths (`bind_package_paths`, #255) |
| `protected.rs` | protected types and bodies (6.6) | S6 | owner: s6; protacc: #212 method profile check (small, in `decl.rs` `subprogram_spec`); semprot: protected and `--relaxed` negative suite #52; v2019c: composites of protected types #361 (also `types.rs` record elements, `implicit.rs` `shape`, `decl.rs` variable ports) (wiring in `decl.rs` `object_decl`, `stmt.rs` variable assignment, `resolve.rs` operator errors) |
| `alloc.rs` | allocators, dereference (`.all`, implicit), incomplete type uses, `'designated_subtype` (2.6, 3.7) | S6 | owner: alloc |
| `view.rs` | mode views, `'converse` (6.7) | S7 | owner: s6; modeview: views in generic units, `'converse` checks, method aliases of private variables #32 |
| `rev.rs` | mixed 2008/2019 designs: `since` of std/ieee declarations, `E1007`, `W1008` (0.1) | S7 | owner: semrev (#50) |
| `reflect.rs` | 2019 `'reflect`, `std.reflection` mirrors (2.11) | S7 | owner: reflect |
| `hir.rs` | HIR node shapes M2 consumes (section 7) | all | shapes fixed; changes need a note in sem.md |
| `validate.rs` | HIR invariants (7.3) | S2 onward | owner: literal |
| `diag.rs` | codes (8.1) and `--explain` texts | all | add codes additively |
| `suggest.rs` | did-you-mean (8.3) | S1 | done; didyoumean: kind noun, abbreviations, declaration label, `similar`/`suggest`/`help` shared by every unknown-name site (#853) |

To take a module, edit its row in your first commit (`owner: <short name>`), push, then work.
If you need a function in a module someone else owns, add it in a small separate commit, or ask
through the orchestrator.

## Conventions

- **Fail early, fail clearly.** Every problem is a `yavs_diag::Diagnostic` reported through
  `Db::report`/`Db::error`, with a code, primary span, labels, notes and help. Never
  `unwrap_or(default)` on anything that comes from the design. A construct a later slice will
  handle reports `Db::unsupported(span, what, "S<n>")` (code `E1999`, says which slice): never a
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
- **No copied code.** Never copy sim1 or sim2 (GPL) or rust_hdl (MPL). Reading for behaviour is
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
  `E1999` (S3), selected ones (`alias a : t is rec.f`) too; the corpus (an external verification library's utility package) needs both.

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
  subprogram decl). Currently a stub reporting `E1999`.
- **implicit.rs** `declare_implicit_ops(ty, scope)` after every type declaration (also the
  universal types at the start of `std.standard`), `flush_pending_implicit()` after every
  `std.standard` type and subtype.

Protected method calls are wired (reflect): `obj.m` / `obj.m(args)` in names.rs
(`Via::Method`, `method_call_cands`, `method_object`) and in `proc_call`, through
`Db::method_target` / `Db::method_decls` (protected.rs, quiet) and `protected_member` (reports).
An access to a protected type is dereferenced implicitly (`ExprKind::Deref`, built inline in
`method_object`; alloc: switch it to `Db::deref` once that lands).

Done: `external_name` (`NameKind::External`, #49, aliases of external names #19), `view_ref`
(`Db::view_name`: view ports and `'converse`), method aliases of private variables (modeview,
#32: `Db::forwarding`, `forwarded_object` in names.rs builds `ExprKind::PrivateVar` as the
call's object), `match_signature` in alias declarations, attribute specifications and generic
subprogram actuals (use clauses take no signature, LRM 12.4), calls inside a protected body use
the implicit object (`method_of: None`), the aggregate resolver. Linkage ports: reads are E1360 through
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
`st_lookup_guard` (`lookup`), labels and parameters through `declare`. Still `E1999`, wanted:

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
  user code (`tests/diag/sem/SIM1.md`); done: the ruling is limited to `std`.
- Allocators filter overloads by designated type (`CandSet::Alloc`, #27); `null` is still the
  class `ANY_ACCESS`.

From cli (`yavs analyze`):

- **loader.rs** a `--map`ped library without `yavs-lib.toml` is an empty library (cli-contract
  section 3), but loading it reports `L0001 library not found`. Request: treat a missing manifest
  as an empty library, so the use is `E1002`/`E1004` (unit not found), not `L0001`.
- **std/ieee as work**: done (#22): `yavs analyze --bootstrap` allows `std`/`ieee` as `--work`/`--map`
  (cli-contract section 3), which `tools/corpus/yavs-lib-*` uses.

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
- Generic types, subprograms and packages on entities, components and blocks report `E1999`
  (S4, generic). A subtype indication as a generate specification (`for g(natural range 1 to
  2)`) is a `GenSelect::Range` over the subtype (#35).
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
  one is `E1505`. `type t is new g generic map (...)` is `protected_instance` (generic.rs): the
  region is cloned at once, the body by `instance_body` (#29). A generic map in the protected
  type header is still a not-implemented diagnostic.
- Open: anonymous generic types (`type is private` in an interface object, S7), formal packages
  with a map stay opaque inside the generic (their generics do not take the map's values). Mode
  views are cloned with their declaration (`r_view`; the converse of a clone is materialized
  again on use).
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
  resolved at the instance (`binding.rs` `subprogram_default`, `E1504`). Components
  (`Db::instantiate_component`), blocks (`Db::instantiate_blocks`) and configuration
  specifications inside such an architecture followed in #222.
