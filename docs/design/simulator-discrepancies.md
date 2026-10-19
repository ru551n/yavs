# Simulator discrepancies

Every known case where the reference simulators disagree with each other, with the LRM or with
yavs, in one place. One entry per distinct behaviour. Each has a reproducer under
`tests/discrepancies/<id>/` and `tools/discrepancies/run.py` runs all of them on yavs and on any
configured simulator. Written 2026-10-19 on the user's request (all discrepancies, so that a
fourth simulator can be tested the same way when it is available).

## Labels, verdicts, kinds

- Simulators are named by label: `sim1`, `sim2`, `sim3`; `sim4` is to be filled when that
  simulator is available. The label to tool mapping is not in the repository (it lives in the
  untracked `tools/discrepancies/sims.local.conf` or in `DISCREP_SIM<N>` variables), as for the
  analysis corpora.
- Verdicts: `REJECT` (analysis or elaboration error), `OK` (accepted, and for a runnable case the
  same messages as yavs's expected output), `OK*` (accepted, different messages), `CRASH`
  (internal error or signal), `FATAL` (run-time failure), `TIMEOUT`. `unsupported` (the simulator rejects only because it lacks the feature, such as a missing
  `std.reflection`: no verdict, never counted in a majority; the case's `verdicts` file keeps the
  raw `REJECT`). For cases with two standard
  revisions the cell reads `08: V / 19: V` when they differ. Observed versions: sim1 1.23-devel,
  sim2 7.0.0-dev, sim3 2025.3 (starter edition), as recorded on 2026-10-19.
- Kind: **LRM ambiguity** (the text is unclear or silent, yavs follows root rule 2: permissive,
  match the other simulators, warn where they differ), **LRM clear** (the text decides; a
  simulator that deviates is wrong or not yet complete), **reference bug** (a crash or an
  unsupported construct in a reference), **library availability** (the case depends on a
  non-IEEE package that a simulator may or may not ship; no LRM reading applies). Where the kind is "LRM clear" and a simulator crashes
  or rejects, the deviation is that simulator's bug.
- "yavs" is yavs's recorded verdict from `expected-yavs` (the messages are in the file).
  "Decided" says who chose yavs's behaviour: `user, date` for a user ruling, `agent` for an
  agent decision under rule 2 (recorded in `docs/design/lrm-checks.md` or `docs/decisions.md`).
- Majority rule (user, 2026-10-19): yavs is never counted in a majority; a majority means 2 of
  the 3 reference simulators.
- The LRM column gives the clause; the reading is in `docs/design/lrm-checks.md` under the
  issue or `L` number in "Decided".

## Running

```sh
python3 tools/discrepancies/run.py --sims sim1,sim2,sim3,sim4      # table; yavs always runs
python3 tools/discrepancies/run.py --check                         # fail if yavs or a recorded verdict drifts
python3 tools/discrepancies/run.py --sims sim4 --record            # store a new simulator's verdicts
python3 tools/discrepancies/run.py --print-config                  # how to map labels to commands
```

`YAVS_BIN` names the yavs binary (default `yavs` on `PATH`). Each run is wrapped as
`timeout 300 systemd-run --user --scope -p MemoryMax=2G` (`--no-sandbox` to drop it). A new
simulator needs a command template in `TEMPLATES` of the script (analysis snippet, then an
elaborate-and-run snippet that exits 98 on an elaboration error); the Riviera-PRO style template
there is untested.


## Syntax

| Case | Construct | LRM | Kind | sim1 | sim2 | sim3 | sim4 | yavs | yavs's choice | Decided |
|---|---|---|---|---|---|---|---|---|---|---|
| `formal-package-per-element-box` | `<>` for one generic of a formal package map: `generic map (a => 1, b => <>)` | 2019 6.5.7.2 (text not at hand) | LRM ambiguity | REJECT | REJECT | REJECT | to be filled | REJECT | not parsed; no simulator parses it either | open |
| `psl-assert-always` | PSL `default clock` and `assert always` in an architecture | 2008 clause 17 (PSL by reference) | LRM ambiguity | OK | OK | REJECT | to be filled | OK | skipped with warning W0004, one per design unit | user, 2026-10-18 (#138) |
| `synopsys-arith-package` | `use ieee.std_logic_arith.all` and `std_logic_unsigned` (non-standard packages) | none (not IEEE packages) | library availability | OK | REJECT | OK | to be filled | REJECT | not shipped (clean-room, deferred): E1002 design unit not found | agent (plan: Synopsys packages deferred) |

## Sem

| Case | Construct | LRM | Kind | sim1 | sim2 | sim3 | sim4 | yavs | yavs's choice | Decided |
|---|---|---|---|---|---|---|---|---|---|---|
| `access-constant-parameter` | constant parameter of an access type (2008 and 2019) | 6.4.2.2, 6.5.2 | LRM clear | REJECT | REJECT | REJECT | to be filled | 08: REJECT / 19: OK | 2008: E1223; 2019: accepted (against the LRM: the 2019 6.5.2 still forbids an interface constant of an access type, and the std.env declarations that use one are a confirmed defect, VI#259; decision pending) | agent, 2026-10-18/19 (lrm-checks L46) |
| `access-to-protected` | access type designating a protected type | 5.4.1 | LRM ambiguity | 08: REJECT / 19: OK | 08: REJECT / 19: OK | REJECT | to be filled | 08: REJECT / 19: OK | 2008: E1221; 2019: accepted | agent (lrm-checks L19) |
| `actual-conversion-in-variable-parameter` | actual conversion of an in variable parameter | 6.5.7.1, 4.2.2.2 | LRM clear | REJECT | OK | OK | to be filled | OK | accepted | agent (#229, L66) |
| `alias-of-external-method` | method call through an alias of an external shared variable | 6.6.2, 8.7 | LRM clear | REJECT | OK | OK | to be filled | OK | accepted | agent (NVC.md) |
| `block-config-default-bound` | block configuration of a default-bound instance | 3.4.3 | LRM ambiguity | REJECT | REJECT | OK | to be filled | OK | accepted | agent (NVC.md) |
| `call-conversions` | conversions in subprogram call associations (formal and actual, scalar and composite) | 6.5.7.1 | LRM clear | REJECT | OK* | OK | to be filled | OK | supported, results by hand | agent (#229) |
| `config-label-wrong-component` | configuration names an instance of another component | 3.4.2, 7.3 | LRM clear | OK | REJECT | REJECT | to be filled | REJECT | E1701 at analysis (some simulators check at elaboration) | agent (NVC.md) |
| `config-rebinding` | configuration re-binds an instance already bound by a specification | 7.3.2 | LRM clear | OK | REJECT | OK | to be filled | REJECT | E1702 (against the LRM: an incremental binding may repeat the primary binding's entity aspect, 7.3.2.1; only a different entity aspect is an error; decision pending) | agent (NVC.md) |
| `div-by-zero-in-range` | division by zero in a constant type-range bound | 9.4.2 | LRM ambiguity | OK | REJECT | REJECT | to be filled | REJECT | folded at analysis, E1401 | agent (sem.md 4.2) |
| `duplicate-interface-param` | two parameters with one name in a subprogram declaration | 12.3 (homographs) | LRM clear | OK | REJECT | REJECT | to be filled | REJECT | E1101 | agent (NVC.md) |
| `formal-conversion-unconstrained-port` | formal conversion on an unconstrained out port | 6.5.7.1 | LRM ambiguity | REJECT | REJECT | REJECT | to be filled | REJECT | E1312: a conversion of a formal needs a fully constrained formal | agent (rule1-review F8) |
| `formal-package-map-mismatch` | actual package instance whose generic map differs from the formal package's | 6.5.7.2 | LRM clear | OK | REJECT | REJECT | to be filled | REJECT | E1503 | agent (NVC.md) |
| `formal-package-subprograms` | calling a formal subprogram through a formal package | 6.5.7.2 | LRM clear | FATAL | REJECT | OK | to be filled | OK | run (hand-checked) | agent (#101) |
| `generic-class-array-elem-mismatch` | 2019 generic array type: element type of the actual does not match | 6.5.3 (2019) | LRM clear | OK | REJECT | REJECT | to be filled | REJECT | E1503 | agent (NVC.md) |
| `generic-subprogram-of-block` | a generic subprogram of a block or component as the actual of an entity's generic subprogram | 6.5.7.2, 7.3.3, 11.2 | LRM ambiguity | REJECT | REJECT | REJECT | to be filled | REJECT | E1506 (all three reference simulators reject) | agent, 2026-10-19 (#222, L63) |
| `generic-subprogram-of-enclosing-entity` | an enclosing entity's own generic subprogram passed down as the actual of a nested instance's generic subprogram | 6.5.7.2 | LRM ambiguity | REJECT | OK | FATAL | to be filled | OK | accepted (sim2 runs it; sim3 accepts it and crashes at run time, recorded as FATAL; sim1 rejects) | user, 2026-10-19 (majority rule) |
| `i64-min-literal` | `-9223372036854775808 fs`: negated literal at the i64 minimum | 9.3, 5.2.3 (range of universal_integer implementation-defined) | LRM ambiguity | REJECT | REJECT | REJECT | to be filled | OK | accepted, folds to i64::MIN (all three reference simulators reject: no majority for it; decision pending) | agent, 2026-10-18 (#214, L58) |
| `implicit-condition-operator` | implicit `??` of a condition when only the type is visible (`use ieee.std_logic_1164.std_ulogic`) | 9.2.9 | LRM ambiguity | OK | REJECT | REJECT | to be filled | OK | the `??` of the type's region is found (sim2 and sim3 reject: the majority rejects; decision pending) | agent, 2026-10-18 (#48, L53) |
| `implicit-signal-in-subprogram` | `'stable`, `'quiet`, `'delayed`, `'transaction` named in a subprogram body | 4.2.2.2, 14.7.5 | LRM ambiguity | REJECT | OK | OK | to be filled | OK | accepted and run | agent, 2026-10-19 (L67) |
| `incomplete-type-never-completed` | incomplete type never completed | 5.4.2 | LRM clear | OK | OK | REJECT | to be filled | REJECT | E1220 (the simulators that accept are lenient) | agent (NVC.md) |
| `incremental-binding` | incremental binding: generic map with an open entity aspect in a configuration | 7.3.2.3 | LRM clear | REJECT | OK | OK | to be filled | OK | supported | agent (hierarchy/incremental_binding, ref=hand) |
| `individual-assoc-parts` | individual association of parameters by slices and record elements | 6.5.7.1 | LRM clear | REJECT | FATAL | OK | to be filled | OK | supported, results by hand | agent (#71) |
| `local-generic-package-same-unit` | a generic package with a body declared in an architecture, instantiated in the same architecture | 4.9, 4.7 (2008) | LRM ambiguity | REJECT | FATAL | REJECT | to be filled | OK | accepted: the instance clones the body once the region is analyzed | agent (lrm-checks L81, #34) |
| `method-alias-objects-full` | aliases of methods through shared variables of packages and object aliases | 5.6.2, 6.6.3 | LRM ambiguity | REJECT | FATAL | REJECT | to be filled | OK | accepted | user, 2026-10-19 (#237) |
| `method-alias-of-object` | 2019 alias of a method of a shared variable, called through the alias | 5.6.2, 6.6.3 | LRM ambiguity | REJECT | FATAL | REJECT | to be filled | OK | accepted | user, 2026-10-19 (#237) |
| `method-alias-of-process-variable` | 2019 alias of a method of a process variable | 5.6.2, 6.6.3 | LRM ambiguity | REJECT | FATAL | REJECT | to be filled | OK | accepted | user, 2026-10-19 (#237) |
| `method-alias-private-variable` | 2019 method alias with the implicit object inside the protected body | 5.6.2 (2019) | LRM clear | REJECT | REJECT | REJECT | to be filled | OK | accepted | agent (lrm-checks L15) |
| `multiple-sources-two-processes` | unresolved signal driven by two processes | 6.4.2.3, 14.7.3.1 | LRM clear | OK | OK | REJECT | to be filled | REJECT | E1812 at analysis where static; the other two reference simulators reject it at elaboration (6.4.2.3: the error is defined after elaboration), so all three reject the design | agent (NVC.md) |
| `named-return-in-return-and-formal` | named-return function call in a return statement and as the actual of a constrained formal | 4.2.1 (2019) | LRM ambiguity | REJECT | FATAL | REJECT | to be filled | OK | result subtype taken from the context | agent (L62, #33) |
| `named-return-record-generic-method` | named return values of a record type, of generic subprograms (also of a generic type) and of a protected method | 4.2.1 (2019) | LRM ambiguity | REJECT | REJECT | REJECT | to be filled | OK | result subtype taken from the target, as for arrays | agent (L62, #239) |
| `named-return-scalar` | named return value whose type mark denotes a scalar type, `return r of integer` | 4.2.1 (2019) | LRM ambiguity | REJECT | OK | REJECT | to be filled | REJECT | E1233 (sim2 accepts the analysis and fails at elaboration) | user, 2026-10-19 (L77, #239)
| `nested-generic-packages` | nested generic packages and a chain of formal packages | 4.7, 6.5.7.2 | LRM clear | REJECT | REJECT | OK | to be filled | OK | run (hand-checked); sim1 and sim2 crash (recorded as REJECT) | agent (#101) |
| `nested-package-runtime` | a package with a body declared in an architecture, used by a process | 4.7, 4.8, 3.3.2 (2008) | LRM clear | REJECT | OK | REJECT | to be filled | OK | the package's objects belong to the architecture | agent (#34) |
| `nested-package-second-body` | two bodies of one package declared in an architecture | 4.8, 12.1 | LRM clear | OK | OK | REJECT | to be filled | REJECT | E1101 at the second body | agent (#34) |
| `others-with-other-choice` | `others` combined with another choice in one association | 9.3.3.3 | LRM clear | OK | REJECT | REJECT | to be filled | REJECT | E1325 | agent (NVC.md) |
| `overflow-in-addition` | integer overflow of a constant addition in a non-required context | 9.2.2 | LRM ambiguity | REJECT | OK | OK | to be filled | OK | warning W1401, run-time check kept | agent (sem.md Q5) |
| `overload-by-conversion-type` | overloads of a conversion function distinguished only by the formal conversion's type | 4.5.3, 6.5.7.1 | LRM ambiguity | REJECT | REJECT | OK | to be filled | OK | resolved per candidate | agent (#229, L66) |
| `overloaded-attribute-spec` | attribute specification with an overloaded entity tag and no signature | 7.2 | LRM clear | REJECT | OK | OK | to be filled | OK | accepted, decorates every matching declaration | agent (NVC.md) |
| `package-generic-local-state` | generic package instance with a shared variable in a thrice-instantiated architecture | 4.8, 14.5 | LRM clear | REJECT | OK | OK | to be filled | OK | one set of objects per entity instance | agent (#101) |
| `package-in-generate` | a package declared in a generate statement's declarative part, one per iteration | 11.8, 3.3.2 (2008) | LRM clear | REJECT | REJECT | REJECT | to be filled | OK | one package per generate iteration | agent (#34) |
| `package-in-process` | a package declared in a process | 11.3 (2008) | LRM clear | REJECT | OK | REJECT | to be filled | OK | the package's objects are the process's | agent (#34) |
| `package-instance-local` | package instances in process, subprogram and generate declarative parts | 4.3, 11.3, 11.8 | LRM clear | REJECT | REJECT | OK* | to be filled | OK | run (hand-checked) | agent (#101) |
| `package-instance-signal-in-process` | an instance of a generic package declaring a signal, in a process | 4.9, 4.7, 11.3 | LRM clear | REJECT | REJECT | OK | to be filled | REJECT | E1237: the instance is a package declaration in the process, which declares no signals | agent (lrm-checks L82, #34) |
| `port-conv-formal-type-mark` | a type mark as the formal part of a port association, with a formal type it is not closely related to | 6.5.7.1, 9.3.6 | LRM clear | REJECT | REJECT | REJECT | to be filled | REJECT | E1350 (#259) | agent (#259) |
| `port-conv-inout` | both conversions of an `inout` port; the formal follows the actual when only a second driver of the resolved actual changes it | 6.5.7.1, 14.7.7.3 | LRM clear | CRASH | OK* (the formal does not follow) | OK | to be filled | OK | supported, results by hand | agent (#238, L70) |
| `port-conv-inout-one-way-in` | an `inout` port converted on the actual side only, `q => id(io)` | 6.5.7.1 | LRM ambiguity | REJECT | REJECT | OK | to be filled | REJECT | E1313 (#74, L72) | agent (#74, L72) |
| `port-conv-inout-one-way-out` | an `inout` port converted on the formal side only, `id(q) => io` | 6.5.7.1 | LRM ambiguity | OK | REJECT | OK | to be filled | OK | accepted by majority (user rule 2026-10-19): copy-out converts, copy-in passes the actual unchanged (#74, L72) | agent (#74, L72) |
| `port-map-gap-individual` | individual association of an unconstrained port leaving elements unassociated | 6.5.7.1 | LRM clear | REJECT | REJECT | REJECT | to be filled | REJECT | E1311 | agent, 2026-10-18 (#224, L65) |
| `positional-generic-after-type` | positional generic map after an interface type: do implicit `=` and `/=` take a position? | 6.5.3, 6.5.7.2 | LRM ambiguity | REJECT | OK | OK | to be filled | OK | they do not | agent, 2026-10-18 (#223, L69); confirmation pending |
| `protected-method-access-parameter` | protected method parameter of an access type | 5.6.2 | LRM ambiguity | 08: REJECT / 19: OK | REJECT | REJECT | to be filled | 08: REJECT / 19: OK | 2008: E1223; 2019: accepted (unclear, permissive) | agent, 2026-10-18 (#212) |
| `protected-method-access-result` | protected method result of an access type | 5.6.2 | LRM ambiguity | 08: REJECT / 19: OK | 08: REJECT / 19: OK | REJECT | to be filled | 08: REJECT / 19: OK | 2008: E1223; 2019: accepted (unclear, permissive) | agent, 2026-10-18 (#212) |
| `protected-type-expanded-name-method` | `pt.get` in the body of protected type `pt`: a method of the current object | 8.3, 12.1 | LRM clear | OK | REJECT | OK | to be filled | OK | accepted | agent (L74, #250) |
| `protected-type-expanded-name-mixed` | `pt.n` (state hidden by a parameter `n`) and `pt.get` in one protected body | 8.3, 12.1 | LRM clear | REJECT | REJECT | OK | to be filled | OK | accepted (sim1 accepts each alone) | agent (L74, #250) |
| `protected-type-expanded-name-state` | `pt.n` in the body of protected type `pt`, `n` hidden by a parameter | 8.3, 12.1 | LRM clear | OK | REJECT | OK | to be filled | OK | accepted | agent (L74, #250) |
| `shared-variable-before-body` | shared variable of a protected type in a package declaration, before the protected body (in the package body) | 14.4.2 | LRM clear | REJECT | OK (warning) | REJECT | to be filled | REJECT | E1230 at the variable (type as secondary) | LRM clear, 2 of 3 reference simulators (#258) |
| `signal-param-parts` | signal formals associated element by element | 6.5.7.1 | LRM clear | FATAL | OK | OK* | to be filled | OK | supported, results by hand | agent (#71) |
| `signal-param-records` | signal formals with records associated field by field | 6.5.7.1 | LRM clear | REJECT | OK | OK | to be filled | OK | supported, results by hand | agent (#71) |
| `signal-param-slices` | signal formals associated by slices | 6.5.7.1 | LRM clear | REJECT | FATAL | FATAL | to be filled | OK | supported, results by hand | agent (#71) |
| `stable-attr-in-subprogram` | `'stable` of a signal inside a subprogram body (not a formal) | 4.2.2.2 | LRM ambiguity | REJECT | OK | OK | to be filled | OK | accepted (the LRM forbids only formal signal parameters) | agent (NVC.md) |
| `static-index-out-of-range` | static index outside the range in a process | 8.4 | LRM ambiguity | REJECT | OK | REJECT | to be filled | OK | warning W1404 (code may be dead) | agent (sem.md 4.3) |
| `static-slice-out-of-range` | static slice outside the range | 8.5 | LRM ambiguity | REJECT | OK | REJECT | to be filled | OK | warning W1404 | agent (sem.md 4.3) |
| `subprogram-header-generic-map` | subprogram header with generic list and generic map (an instance) | 4.2.1, 4.4 | LRM clear | REJECT | REJECT | REJECT | to be filled | OK | the header declares an instance; calls run | agent, 2026-10-18 (#227, lrm-checks L61) |
| `subprogram-param-package-interface` | package interface in a subprogram parameter list | 4.2.2.1 | LRM clear | OK | REJECT | REJECT | to be filled | REJECT | E1232 | agent, 2026-10-18 (#227) |
| `type-through-formal-package-of-formal-package` | a type reached through a formal package of a formal package (`s.r.arr`) as a parameter type | 6.5.7.2, 4.8 | LRM ambiguity | REJECT | REJECT | REJECT | to be filled | REJECT | reject, `E1300` at the call (L76): sim1 and sim3 reject the call, sim2 rejects the expanded name `s.r.arr` itself, so it never reaches the call | user, 2026-10-19 (majority of the reference simulators, #101) |
| `unaffected-force` | `force unaffected` | 10.5.2.1 (2019) | LRM clear | CRASH | REJECT | 08: REJECT / 19: OK | to be filled | 08: REJECT / 19: OK | 2019: nothing forced; 2008: rejected | agent (L60) |
| `unaffected-variable` | `unaffected` in a variable assignment | 10.5.2.1 (2019) | LRM clear | CRASH | REJECT | 08: REJECT / 19: OK | to be filled | 08: REJECT / 19: OK | 2019: target unchanged; 2008: rejected | agent (L60) |
| `use-clause-constant-vs-literal` | use-clause clash between a constant and an enumeration literal | 12.4 | LRM clear | OK | REJECT | REJECT | to be filled | REJECT | E1102 (both invisible) | agent (NVC.md) |
| `variable-port` | 2019 port of class variable with a protected type | 6.5.2, 6.5.7.1 (2019), LCS-2016-047 | LRM clear | OK | REJECT | REJECT | to be filled | OK | accepted in 2019 files | agent (lrm-checks L21) |

## Elaboration

| Case | Construct | LRM | Kind | sim1 | sim2 | sim3 | sim4 | yavs | yavs's choice | Decided |
|---|---|---|---|---|---|---|---|---|---|---|
| `array-record-resolved-whole` | arrays of records resolved as a whole | 4.6, 14.7.2 | LRM clear | CRASH | FATAL | OK | to be filled | OK | whole composite driven | agent (#200, ref=hand) |
| `array-resolved-whole` | arrays resolved as a whole, partial drivers | 4.6, 14.7.2 | LRM clear | CRASH | FATAL | OK | to be filled | OK | whole composite driven | agent (resolution/array_whole, ref=hand) |
| `bus-guarded` | guarded bus resolution | 14.7.3 | LRM clear | OK* | OK | OK | to be filled | OK | LRM result, by hand | agent (resolution/bus_guarded, ref=hand) |
| `external-name-in-port-delta` | external name as the actual of an in port: delta at which the port follows the signal | 8.7, 6.5.6.3, 14.7 | LRM ambiguity | OK* | OK | OK | to be filled | OK | same delta, as a signal name actual | user, 2026-10-19 (#186, 2 of 3 reference simulators) |
| `inertial-part-actual` | `inertial` on a part of an individual association: delta at which the part follows the signal | 6.5.6.3, 6.5.7.1 | LRM clear | OK | REJECT | OK* | to be filled | OK | one delta later, as for a whole formal | agent, 2026-10-19 (#261, L80) |
| `external-name-in-port-init` | external name as the actual of an in port: the port's initial value | 8.7, 6.5.6.3 | LRM ambiguity | OK* | OK | OK* | to be filled | OK | the actual's initial value, not the port's default | user, 2026-10-19 (#186, 2 of 3 reference simulators) |
| `open-port-default` | open unconstrained in port with a default: bounds of the default | 6.5.6.3, 14.3.5 | LRM ambiguity | OK | OK | OK | to be filled | OK | bounds from the default | agent, 2026-10-18 (#218, L59) |
| `partial-driver-resolved-composite` | process driving one field of a record resolved as a whole: undriven fields in resolution | 4.6, 14.7.2 | LRM ambiguity | OK* | FATAL | OK | to be filled | OK | every leaf is driven by a driving process, resolver gets whole values | agent, 2026-10-18 (#193) |
| `relative-external-name-port-actual` | relative external name as a port actual: where the pathname starts | 8.7 | LRM clear | FATAL | OK | OK | to be filled | OK | starts at the region that contains the port map; sim1 starts at the instantiated entity and fails at elaboration ("name not found") | agent (corpus hierarchy/external_port_forms) |
| `unconstrained-records` | records with unconstrained fields | 5.3.3, 6.4.2.3 | LRM clear | OK* | OK | OK | to be filled | OK | LRM result, by hand | agent (ref=hand) |

## Runtime semantics

| Case | Construct | LRM | Kind | sim1 | sim2 | sim3 | sim4 | yavs | yavs's choice | Decided |
|---|---|---|---|---|---|---|---|---|---|---|
| `aggregate-target-range-choice` | range choices with array values in an aggregate target | 10.5.2.1, 9.3.3.3 | LRM ambiguity | OK | REJECT | OK | to be filled | REJECT | E1333 at analysis (one reference simulator rejects, the others accept with the aggregate's direction) | agent, 2026-10-19 (#244, L68) |
| `aggregate-piece-context-direction` | array values under range choices against a constrained context's direction, `v := (0 to 1 => a, 2 to 3 => b)` with `v` downto | 9.3.3.3 | LRM ambiguity | OK* | OK | REJECT | to be filled | OK | each range filled in the context's order (sim1 differs between an assignment and a qualified expression) | agent, 2026-10-19 (#230, L84) |
| `aggregate-piece-direction` | direction of a named aggregate with array values under downto range choices and no constrained context | 9.3.3.3 | LRM ambiguity | OK | OK | OK | to be filled | OK | the direction of those choices | agent, 2026-10-19 (#230, L84) |
| `aggregate-piece-element-range-direction` | a range choice of elements against the direction of one with an array value | 9.3.3.3 | LRM ambiguity | OK | OK | REJECT | to be filled | OK | accepted: choices of elements keep any direction | agent, 2026-10-19 (#230, L84, 2 of 3) |
| `aggregate-piece-mixed-direction` | array values under range choices of different directions in one aggregate | 9.3.3.3 | LRM ambiguity | OK | REJECT | REJECT | to be filled | REJECT | E1334 at analysis (sim1 takes the last choice's direction) | agent, 2026-10-19 (#230, L84, 2 of 3) |
| `assert-count-cycles` | assert counters read by a process in the cycle of another process's report | std.env (2019) | LRM ambiguity | OK* | OK* | OK | to be filled | OK | counts as of the cycle start plus the process's own reports | agent (#99) |
| `concat-null-operands` | bounds of the concatenation of two null arrays | 9.2.5 | LRM clear | REJECT | FATAL | OK | to be filled | OK | the right operand's bounds | agent (decisions.md) |
| `delayed-initial-value` | `S'delayed(T)` before T has passed | 14.7.5.2 | LRM clear | OK* | OK | OK | to be filled | OK | the prefix's initial value | agent, 2026-10-19 (L67) |
| `element-length-open-formal` | actual whose element length differs from the formal's constrained element, `ow(open)(7 downto 0)` given elements of length 4 | 5.3.2.2, 6.5.6.2 | LRM clear | OK* | FATAL | FATAL | to be filled | FATAL | length-mismatch runtime error at the call (E3009); sim1 accepts and reports the formal's bounds | agent (#257) |
| `ieee-body-warnings` | warnings reported by the numeric_std body itself | IEEE package source | LRM clear | OK* | OK* | OK | to be filled | OK | printed from the source | agent (ieee/ieee_warnings, ref=hand) |
| `integer-low-2019` | range of INTEGER in 2019 files | 5.2.3.1 (2019: at least -2**63+1 to 2**63-1) | LRM ambiguity | 08: OK / 19: OK* | 08: OK / 19: OK* | 08: OK / 19: OK* | to be filled | OK | 2008: 32 bits; 2019: -9223372036854775807 to 9223372036854775807 | agent (#214 area) |
| `last-value-slice` | `'last_value` of a slice of a signal | 16.2.4 | LRM clear | OK | FATAL | OK | to be filled | OK | the previous value of the slice | agent (#215) |
| `process-order-in-cycle` | order of the processes resumed in one cycle: sensitivity lists, one wait, several waits, a wait in a procedure, timeouts, an instance | 14.7.5.3 (order unspecified) | LRM ambiguity | OK* | OK* | OK | to be filled | OK | one-suspension-point processes in elaboration order, then the others in reverse elaboration order, signal-resumed before timeout-resumed; initialisation in elaboration order (the per-case majority) | user, 2026-10-19 (#252) |
| `reflect-access-get` | std.reflection `get` on an access to INTEGER | 2019 16.11 | reference bug | OK* | unsupported | unsupported | to be filled | OK | returns the designated value; sim3 has no std.reflection; sim1 returns the pointer | agent (reflect/access_file_prot, ref=hand) |
| `reflect-array-of-records` | std.reflection mirror of an array of records | 2019 16.11 | reference bug | FATAL | unsupported | unsupported | to be filled | OK | walks the elements; sim1 crashes | agent (reflect/walk, ref=hand) |
| `reflect-enum-subtype-range` | std.reflection mirror of an enumeration subtype's range | 2019 16.11 | reference bug | FATAL | unsupported | unsupported | to be filled | OK | the subtype's range; sim1 ignores it | agent (reflect/enums, ref=hand) |
| `reflect-index-bases` | bases of std.reflection indices (enumeration literals, elements 0; units 1) | 2019 16.11 (prose fixes no base) | LRM ambiguity | OK | unsupported | unsupported | to be filled | OK | same bases as sim1 | agent, 2026-10-18 (#156) |
| `reflect-multidim-get` | `array_value_mirror.get` on a two-dimensional array (index normalisation) | 2019 16.11 | reference bug | FATAL | unsupported | unsupported | to be filled | OK | LRM indices | agent (reflect/multidim, ref=hand) |
| `reflect-physical-time-value` | std.reflection value of a TIME mirror: unit of the integer | 2019 16.11 | LRM ambiguity | OK | unsupported | unsupported | to be filled | OK | resolution-limit units; sim1 uses femtoseconds | user ruling (decisions.md, reflection) |
| `reflect-signal-mirrors` | std.reflection mirrors of signals and ports | 2019 16.11 | reference bug | FATAL | unsupported | unsupported | to be filled | OK | mirrors the signal's value; sim1 returns garbage or crashes | agent (reflect/generics, ref=hand) |
| `shift-negative-count` | shift by a negative count | 9.2.4 | LRM clear | OK | OK | OK | to be filled | OK | `sla -k` is `sra k` | agent (decisions.md) |
| `transaction-at-init` | `S'transaction` at initialization | 14.7.5.2 | LRM clear | OK* | OK* | OK | to be filled | OK | no toggle at time 0 | user, 2026-10-19 (#241) |

## Adding a case

`tests/discrepancies/<id>/case.vhd` (single file, report the observable result with `report`),
`case.conf` (`top`, `std` list, `mode = run|analyze`), then `run.py --filter <id> --bless-yavs`
(review `expected-yavs` against the LRM) and `--record` for each simulator, and add the row here
(construct, LRM, kind, yavs's choice, who decided).

