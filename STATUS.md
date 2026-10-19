# Status

## Milestones

| Milestone | State |
|---|---|
| M0: workspace, lexer and parser | done: `yavs parse`, corpora below |
| M1: semantic analysis | in progress: slices S1 to S3 mostly in, `docs/design/sem.md` |
| M2: IR, interpreter, elaboration, kernel | done: the differential corpus passes at 1 thread, 4 threads and 4 shuffled threads (`tests/corpus/RESULTS.md`); see the lower, ir and interp, elab and kernel sections. Not done: binary file read/write of records and arrays of composites (ir and interp section) |
| M4: external test runner end to end | first external test runner testbench runs (#110): `VUNIT_SIMULATOR=yavs python run.py` on a pass/fail `test_runner` and the runner's `examples/vhdl/user_guide` give the right pass/fail per test with the runner's `rsim-integration` branch; the runner's own suites: 10 of 11 pass completely, `check` has one failure (#281, process order in a delta), table below (#112) |
| M7: waves | `yavs run --wave` writes an FST of the whole design (#129, #116): hierarchy, records/arrays as scopes, enum tables, globs and size cap, final value per time step, same file at any `--threads`, finished on Ctrl-C; signals left out by globs or the cap cost nothing at run time; instances in generates, 2-D arrays; read back with wellen in `crates/yavs/tests/wave.rs`. integers carry the subtype name and the width of their range, generate alternatives keep their label (#306). `--wave-variables` records process and non-protected shared variables, records and arrays as scopes, integers with the width of their elaborated range, sampled at the end of each time step through the observer accessors (#305, #348, #321). Open: protected-type shared variables |
| M3: full types and libraries | done per `docs/plan.md`: arrays, records, access and protected types, resolution (composite, access, protected and resolution groups in `tests/corpus/RESULTS.md`); `numeric_std`, fixed and float from source (#94); textio, `env`, package generics, external names; `--load` foreign subprograms (#95); bit-plane layout (user, 2026-10-18). Not done: binary file read/write of records and arrays of composites (ir and interp section) |
| M8: DAP server | first slice (#130): `yavs run --dap=PORT\|stdio` serves the Debug Adapter Protocol over the interpreter's debug session (`crates/yavs-dap`): threads per process, line breakpoints with hit counts, continue, pause (in a process or between cycles), run for or until a time, next delta and next time step (custom requests and VS Code commands, #324), signal breakpoints on a change or a value (DAP data breakpoints and `yavs/setSignalBreakpoints`, #322), force, deposit and release of whole signals and `setVariable` on signals and locals with values parsed as VHDL literals of the target type (`yavs_std::value::literal`, #323), conditions on line and data breakpoints (VHDL expressions evaluated by the server), composite variables expanded by field and index (paged), a Hierarchy scope and `yavs/hierarchy` (#326), step over/in/out, stack frames with source positions, locals, region signals and generics, evaluate for locals and signals by name or path, output events, exit status; one thread under `--dap`, so sessions are deterministic. Scripted sessions in `crates/yavs/tests/dap.rs`. VS Code extension `editors/vscode` (debugger type `yavs`, analyzes and launches over stdio): verified with the DAP test client (`test/client.js`) and in a real headless VS Code (`test/vscode`, breakpoint hit, frame, `setVariable`, force and release commands, a conditional breakpoint, scopes and an expanded array, continue to the end); not in CI. Live waves (#325): `yavs/waveSubscribe` records the client's selection (globs, array cap, variables) with the `--wave` recorder into a second sink and streams `yavs/waves` batches of completed time steps (values as text, bounded queue, never blocks the simulation); `--wave` and `--vhpi` work under `--dap` (the FST equals a plain run's). Open: VHPI plugin stdout under `--dap=stdio` #389 |
| M9: coverage | statement and branch coverage (#131): `yavs run --cov[=FILE]` writes a per-run database keyed by instance path and statement text hash, `yavs cov merge`, `yavs cov report` (text summary, Cobertura XML); no counters without `--cov`; counts identical at any `--threads`; pinned in `crates/yavs/tests/coverage.rs`. Exclusions (#351): `-- coverage off/on` pragmas, `cov report --uncovered` rows and `--exclude FILE`. Toggle coverage (#343): 0->1 and 1->0 per `bit`/`std_logic` scalar of signals and ports from the value-change bus, `--cov-kind`, `--cov-toggle-max` (W0115), `toggle` exclusion rows. Per-test attribution (#353): `--cov-test`, tests per item kept through `cov merge`, `yavs cov rank` (greedy minimal set, unique items per test), `cov report --tests`; database version 4. HTML report (#344): `cov report --html DIR`, static pages with line counts, a missed-only page per file, missed items as exclusion rows, tests per item (E0113 for an unreadable source). |
| M6: JIT tier | in progress: `yavs run --tier=llvm` (feature `llvm`) compiles every function with LLVM 21 and passes the whole difftest corpus (300/300 at 1 thread, identical to the interpreter, rule 5; 289/289 at 4 shuffled threads before the corpus grew); 61% of the compiled programs' instructions run as native code, the rest through the interpreter (side exits, `docs/decisions.md`); open: native lowering of the rest (#334 to #341), tier-up on a background thread (#125), threshold (#126); CI builds and tests the feature on Linux x86_64, aarch64 and Windows; llvm section |
| M5, M10 | not started (M5: the `vhpi_*` export surface (#266), `yavs run --vhpi` (#267) and the design hierarchy object model (#269) are done; values and most callbacks are not; `yavs run` accepts `+plusargs` after the top level, readable through `vhpiArgcP` and the `vhpiArgvs` iterator (#298), `crates/yavs-vhpi`; M10: design `docs/design/gui.md` (#384, decisions answered 2026-10-19); slice G1 (#388): `gui/` workspace (EUPL-1.2) with `yavs-gui`, which starts `yavs run --dap=stdio` or attaches to a port, shows a dock layout of placeholder panels, a console of the run's output and Run/Pause/Stop, tested headless against a scripted DAP server (`gui/AGENTS.md`); `yavs run --gui` starts it (E0122 when missing); release archives ship it; G2 to G6 and the server slices S1 to S4 not started) |

### External test runner: its own VHDL suites (#112)

`VUNIT_SIMULATOR=yavs`, `-p 4`, release build of main at 5acb38be; pass / fail / skipped tests per
suite (`vunit/vhdl/<suite>/test`), including `tb_axi_stream_protocol_checker` with its generic configurations. Skipped as reported by the runner (13, all after the one failure in `check`).

| Suite | Pass | Fail | Skip | Root causes (issues) |
|---|---|---|---|---|
| check | 243 | 1 | 13 | in-delta process order puts the testbench before concurrent-call checker loops #281 (1 failure, the other 13 tests of `tb_check_stable` are skipped after it) |
| com | 145 | 0 | 0 | none (#246 fixed; re-measured after it) |
| data_types | 210 | 0 | 0 | none |
| dictionary | 8 | 0 | 0 | none |
| logging | 113 | 0 | 0 | none |
| path | 2 | 0 | 0 | none |
| random | 4 | 0 | 0 | none |
| record_dict (2019) | 6 | 0 | 0 | none |
| run | 5 | 0 | 0 | none |
| string_ops | 15 | 0 | 0 | none |
| verification_components | 579 | 0 | 0 | none (#251, #248, #252 fixed; all 52 `tb_axi_stream_protocol_checker` configurations pass) |

### Verification library: its own testbenches (#277)

`tools/suites/uvvm.py` (without `--private-copies`, #290 is fixed), release build of main at ba53e23d (#285 to #302 and #304 in), `-j 2`; one run per testbench
entity, architecture and generic set the component's own regression script lists (137 in all).
Pass = exit 0 and the library's final summary `>> Simulation SUCCESS`. 45 runs pass. Only the first root cause of each run is counted.
The runs now reach simulation; 57 of the 63 failures end at elaboration on one cause (#328).

| Component | Pass | Fail | Skip | First root cause (issues) |
|---|---|---|---|---|
| uvvm_util | 31 | 2 | 3 | none that are yavs results (both `rand_basic` testbenches pass since #329, the descriptor of a folded constant of arrays of unconstrained arrays kept only two levels); not yavs results: a test that reads a file an earlier run wrote (1), a test that needs one reference simulator's extension (1); 3 skipped: over 5 minutes (two `rand_gaussian`, `rand_cyclic_performance`) |
| uvvm_assertions | 2 | 0 | 0 | none |
| uvvm_vvc_framework | 1 | 2 | 0 | record resolved as a whole with a `std_logic` element #328 |
| bitvis_irqc | 2 | 0 | 0 | none |
| bitvis_uart | 1 | 2 | 0 | #328 |
| bitvis_vip_scoreboard | 3 | 0 | 0 | none |
| bitvis_vip_avalon_mm | 0 | 5 | 0 | #328 (3); 2 not yavs results: the testbench's FIFO is a third-party design file absent from the pinned checkout, so the component stays unbound |
| bitvis_vip_avalon_st | 0 | 6 | 0 | #328 |
| bitvis_vip_axi | 1 | 1 | 0 | #328 |
| bitvis_vip_axilite | 0 | 2 | 0 | #328 |
| bitvis_vip_axistream | 0 | 22 | 0 | #328 |
| bitvis_vip_clock_generator | 0 | 1 | 0 | #328 |
| bitvis_vip_error_injection | 2 | 0 | 0 | none |
| bitvis_vip_gmii | 0 | 2 | 0 | #328 |
| bitvis_vip_sbi | 0 | 2 | 0 | #328 |
| bitvis_vip_ethernet | 0 | 3 | 1 | #328; 1 skipped: third-party design files absent from the pinned checkout |
| bitvis_vip_gpio | 0 | 1 | 0 | #328 |
| bitvis_vip_i2c | 0 | 0 | 1 | skipped: third-party design files absent from the pinned checkout |
| bitvis_vip_rgmii | 0 | 2 | 0 | #328 |
| bitvis_vip_spec_cov | 0 | 1 | 0 | shared variable whose type holds access values #303 |
| bitvis_vip_spi | 0 | 0 | 24 | skipped: third-party design files absent from the pinned checkout |
| bitvis_vip_uart | 0 | 3 | 0 | #328 |
| bitvis_vip_wishbone | 0 | 2 | 0 | #328 |
| external_vip_apb | 3 | 3 | 0 | #328 |
| Total | 45 | 63 | 29 | |

The generated-VIP tests of the framework component (generator output compiled and simulated) are not run by the driver.

### Verification library: its own `.pro` regression (#276)

`tools/suites/pro.py` reads the library's build and test scripts (`OsvvmLibraries.pro`, `RunAllTests.pro`) with a Tcl subset, analyzes
521 files in script order and runs each test (`yavs run`, release build of main at 8bbefaed, 2 in parallel). Pass = exit 0 and the
library's own `DONE PASSED` summary line. The script settings package body is generated with the scripts' release as settings
revision, as the scripts do (the transcript layout the tests compare against the validated results follows it). 2008 and 2019:
all 314 tests pass, the same test for test; the native co-simulation directories (`CoSim`, `CoSimPCIe`) are skipped, not run.
Root causes fixed on the way: #291, #292, #294, #295, #296, #304, #311 (= #313), #314, #315 (= #312).

### Reference simulators' own regression tests, run time (#354)

`tools/difftest/external.py` runs the self-checking regression tests of the two reference simulator
suites in place from the corpus cache, on yavs (release build) and on the reference simulator
that owns the suite, and classifies each test (`tools/difftest/README.md` "External regression
tests"). Tests for an earlier standard run as 2008 on yavs. Counts on main at fc46f5ed (before)
and after #365 to #368 (2026-10-19):

| Suite | Runnable | Reference bad here | pass | wording only (`fatalmsg`) | order | other output | outcome | elab | analysis |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| first (`external.py nvc`), before | 1070 | 20 | 791 | 52 | 3 | 37 | 64 | 52 | 51 |
| first, after | 1070 | 20 | 798 | 52 | 3 | 39 | 55 | 52 | 51 |
| second (`external.py ghdl`), before | 504 | 30 | 377 | 0 | 0 | 21 | 16 | 19 | 41 |
| second, after | 504 | 30 | 381 | 0 | 0 | 21 | 12 | 19 | 41 |

Skipped by the harness: 403 and 647 (other languages, VHPI, coverage, Tcl and shell scripts,
second-suite scripts with control flow or tool options, negative analysis tests). The passing
tests are listed in `tools/difftest/external-{nvc,ghdl}.pass`; the CI `corpus` job checks them
against the suites' own verdicts (exit status) without a reference simulator. Every listed test gives
identical output at `--threads 1` and `--threads 4`.

Remaining run-time causes, ranked by tests (yavs bugs have issues): reports in resolution and
conversion functions during the run print `(init)` and bypass the report path, #372 (12);
unsupported PSL beyond the subset, W0004 (17, plus 4 that differ only in the directive path);
missing run-time checks, #379 (about 11); PSL-free reference quirks and LRM-unclear cases (about
15: mirror identity in reflection, null array bounds, `'transaction` start value 2 to 2, textio
read of bits with spaces, real image format of the second suite, environment and time stamps);
ICE on arrays with element constraints, #380 (3); `'path_name` of block and generate labels,
#376 (3); binary files of records and arrays of composites (3); guarded assignments with a false
guard, #373 (2); access to protected types, #377 (2); predefined operations on arrays of
composites (2); aggregate longer than a non-static constant subtype is an ICE, #378 (2); failed
textio READ stops the run, #381 (1); a repeated force in one cycle, #382 (1); sensitivity of
aggregate choices, #374 (1); files of subprograms not closed on return, #375 (1); and single
tests (alias and partially constrained record bounds, `hread` digit count, an impure function in
a type evaluated twice, `log2` precision, 2019 mode view parameters). Analysis and elaboration
failures (about 160) are mostly "not lowered yet" (`E2001`) features and VHDL-93 code read as
2008; the analysis corpora cover them.

## CI and releases

`.github/workflows/ci.yml` runs fmt, clippy and `cargo test --workspace` (default features) on
ubuntu-24.04, ubuntu-24.04-arm and windows-latest (MSVC) for every push to main and every pull
request (#134); a `corpus` job (Linux x86_64) fetches the pinned external corpora and fails on any difference from `tools/corpus/*.expected` (#63) and on any test of `tools/difftest/external-*.pass` that no longer meets its suite's verdict (#354); not yet in CI: JIT tier. `.github/workflows/gui.yml` checks the `gui/` workspace
(#388): fmt, clippy and tests on Linux x86_64, a build on Linux aarch64 and Windows, only when
`gui/` changes. `.github/workflows/release.yml`
builds and packages `yavs` and `yavs-gui` (with `LICENSE-EUPL-1.2-yavs-gui`) with `lib/` for the three targets on a `v*` tag and publishes a
pre-release with SHA256SUMS (#165); `workflow_dispatch` with `dry_run` only builds the archives. The Linux archives are built with cargo-zigbuild against glibc 2.17 and smoke-tested in centos:7.
An installed binary finds `lib/` next to itself (#43).

## sem (`crates/yavs-sem`, M1)

- A negated abstract literal at the i64 minimum (`-9223372036854775808`, `-9223372036854775808 fs`) is `E1402` in user code (L58, #214, majority rule); only library `std` uses it (TIME's lower bound) and folds it to `i64::MIN`.
- Instances of one generic subprogram declared under one name (`function f is new f generic map (...)`)
  overload by their instantiated profiles, next to ordinary overloads and across use clauses (#28):
  `tests/diag/sem/gen/subprogram_instances_*`, difftest `generics/subprog_instance_overload` (2008, 2019).
- A subprogram header with a generic list and a generic map (`function f generic (type t) generic map
  (t => integer) parameter (v : t) return t`) declares an instance through the `is new` path, also
  with a separate body that repeats the header (#227, L61): `tests/diag/sem/gen/subprogram_header_map_*`,
  difftest `generics/subprog_header_map_2019`. A type, subprogram or package interface in a
  parameter list is `E1232` (4.2.2.1), in a port clause `E1234` (6.5.6.3); a generic list or map on a formal generic
  subprogram is `E1235` (6.5.4, #236).
- 2019 calls with a generic map (`f generic map (t => integer) (x)`, LCS-2016-049, #362, L103):
  functions, procedures and concurrent procedure calls instantiate the one generic subprogram the
  name denotes, once per call, and run: `tests/diag/sem/gen/call_generic_map_*`, difftest
  `generics/call_generic_map_2019`, `generics/call_generic_map_2019_hand`.
- 2019 signatures on generic map formals (`f [integer return integer] => inc`, LCS-2016-I03,
  #363, L104) pick one of overloaded formal subprograms in subprogram, package and entity
  instances; the signature may name a generic type of the same list; one that fits no formal is
  `E1630`: `tests/diag/sem/gen/generic_map_signature_*`, difftest `generics/generic_map_signature_2019`.
- Negative suite `tests/diag/sem` run by `tests/diag_sem.rs` (exact code, line and column; optional
  rendered `.stderr` snapshots); what still fails is in `tests/diag_sem_xfail.txt` (the
  aggregates and case choices group passes, #45; external names and literals, #49; aliases, #46).
- Overload diagnostics (#51): association errors of the only candidate (`E1100` unknown formal,
  `E1310` formal twice, `E1311` missing actual, `E1301` at the first extra argument), procedures
  used as functions and functions used as statements (`E1300`), no function with the expected
  result type (`E1301`), an ambiguous procedure call reported at the one actual that decides it
  (`E1302` with a qualification help in the declaration's spelling), candidate lists in help, and
  hints for calls from several packages and for vector/integer operators.
- Subprogram calls with sub-element (individual) associations (`pr(x(0) => s)`, `f(r.i => 1)`,
  slices), also of unconstrained formals, resolve and give `Actual::Individual`; each actual is
  typed by its sub-element, a non-static formal index is `E1410`, whole and part `E1310` (#210); the elements of an unconstrained formal must form a
  contiguous range, each once: a gap is `E1311`, a repeat `E1310` (#224).
  Lowering runs them (#71), signal formals with several parts too.
- Conversions in subprogram call associations (`seti(to_r(x) => r)`, `inc(real(x) =>
  integer(r))`, #229) resolve to `Actual::Converted`, per overload candidate; conversions the
  formal's class or mode does not allow are `E1313` (6.5.7.1, 4.2.2.3). Lowering copies in
  through the actual part and back through the formal part (scalars, arrays, records).
- Mixed 2008/2019 designs (#50, sem.md 0.1, `src/rev.rs`): under a 2019 design revision a 2008
  file that names a std/ieee declaration the 2008 copy lacks (`std.env.tool_name`,
  `std.reflection`) gets `E1007` and does not see such declarations through use clauses;
  `'left`/`'right`/`'high`/`'low` of an INTEGER subtype whose bound differs (64-bit INTEGER) get
  `W1008` once per construct.
- Expression typing and conversions (#48): type conversions between types that are not closely
  related (`E1350`, 9.3.6), actuals of the wrong class for variable, signal and file parameters
  (`E1360`), an object actual of the wrong type for the only candidate (`E1300` at the actual),
  a universal bound against a non-integer one (`E1300` at that bound), ambiguous `??` (`E1341`),
  `??` found next to the condition's type when none is visible (L53), and conversion helps
  (`unsigned(x)`, `to_unsigned(x, n)`) on type mismatches.
- Object types: constants, signals, generics, ports and constant or signal parameters of an access
  or file type (or containing one) are `E1223` (#198; also in 2019, except in library `std`; 2008 protected method profiles too, 2019 accepts them, #212); protected ones `E1905`.
- Protected types (#52): signals and constants of a protected type, assignments to a protected
  variable and operators on protected operands are `E1905`; a shared variable of another type is
  `E1904` (`W1904` with `--relaxed`); a method without a body is `E1230` at its name. A shared variable of a protected type declared before its body in the same region (a package declaration) is `E1230` at the variable (#258).
- 2019 composites of protected types (#361, LCS-2016-014, 014a, L102): arrays, records and allocated
  objects whose elements are protected get one new object per element (variables, protected state,
  subprogram frames, allocators, shared variables whose scalar subelements are all protected);
  method calls through indexing, selection and `.all` run, and such types have no `=` or
  assignment (`E1905`). A variable port of such a composite is `E9999` (#370). Sem suite
  `prot/composite_of_protected_*`, difftest `protected/composites_2019_hand`.
- Use clauses and visibility (#54): expanded names into labelled statements and subprograms
  (8.3, `E1103` outside them), and with a protected type prefix inside its body (`pt.n`, `pt.get`,
  #250), body declarations named from outside the package (`E1105`), use
  and context prefixes of the wrong kind (`E1104` at the suffix), and a missing-use-clause help
  for undeclared names that a loaded package (with `ieee` visible: `ieee_std_context`) declares.
- Static checks (#53): array attribute dimensions must be locally static (`E1410`) and exist
  (`E1623`), static indices and slices outside a static index range warn (`W1404`), and an
  constant without a value outside a package declaration is `E1224` (#213), and a deferred constant
  without a full declaration in the package body is `E1230`.
- A package body is one declarative region with its package (4.8): redeclaring a package name in the body is `E1101` (#232, lrm-checks L64).
- `tests/stdlib.rs`: std, ieee packages, and the bodies of std_logic_1164, math_real,
  math_complex, numeric_std, numeric_bit and numeric_*_unsigned analyze clean (2008 and 2019).
  `numeric_std_body_rule_1` is the rule 1 review: every object and object alias of the numeric_std
  body has its bounds from the one subtype representation.
- Declarations: objects, subprograms (operator arity `E1303`, function parameter modes, missing
  bodies `E1230`), components, generic and port clauses (components, blocks), attribute
  declarations and specifications, aliases, protected types; packages and package bodies declared
  in declarative parts (2008, generic ones too; a nested body in the outer package's body),
  signals and shared variables in processes and subprograms `E1237`, also through a nested
  package or a package instance, also one whose body declares them (#34, #284: also before the body); group templates and
  group declarations (`E1640`, `E1641`, label constituents, attributes on groups) and disconnection specifications
  (guarded signals, `others`, `all`, a static time; `E1650`, `E1651`; the time reaches the
  kernel signal) (#260).
- Types (S3): scalars, arrays, records, element/record/open constraints merged once, resolution
  indications, file element checks; the rule 1 representation is pinned on numeric_std
  (`numeric_std_subtypes_rule_1`, `numeric_std_body_rule_1`) and reviewed for the composite cases
  in `docs/design/rule1-review.md` (`tests/rule1.rs`; recommendation: approve with fixes, findings
  F1, F2, F4, F5, F7, F8 open as #98 to #111, F3 and F6 fixed); the user's approval is pending.
- Generics (S4, `generic.rs`): generic packages, subprograms and (2019) protected types; 2008
  `type T` and the 2019 classes; formal subprograms (`is <>`, `is name`) and formal packages
  (`(<>)`, `(default)`, generic map, checked against the actual instance; a parameter type through a
  formal package of a formal package, `s.r.arr`, is the actual instance's type, L76); package and subprogram
  instances, also inside generic units, by HIR cloning (visible part eager, bodies lazy via
  `Db::instance_body`); `W1501`, `W1506`; design entities with generic types, subprograms or
  packages cloned with their architecture per distinct map for elab (`generic/entity.rs`,
  `Db::instantiate_entity`, concurrent statements cloned too); components (`Db::instantiate_component`,
  per map, for elab) and blocks (in place, `Db::instantiate_blocks`) with them too, configuration
  specifications cloned with their region, default binding by name of generic types, subprograms
  and packages (#222). The fixed/float packages' declarations, generic bodies
  and instance bodies analyze clean in 2008 and 2019 (#37); mode views of generic packages and subprograms are
  cloned with them (#32). 2019 generic array types with anonymous index and element types
  (`array (type is range <>) of type is private`), an index constraint, and anonymous designated or
  file element types, in packages, subprograms and entities (#30; sem suite
  `v2019/generic_array_*`, difftest `generics/generic_arrays_2019`,
  `generics/generic_array_constrained_2019_hand`). Generic constants, ports and
  subprogram parameters of an anonymous type (`c : type is range <>`) in packages, subprograms, entities, components, blocks and
  calls (#283; sem suite `v2019/anonymous_object_*`, difftest `generics/anonymous_objects_2019_hand`). Anonymous parameter
  types also work in formal subprograms, in subprograms with a generic list and in user operators called with operator syntax
  (#308; sem suite `v2019/anonymous_{formal_subprogram,generic_subprogram,operator}*`, difftest
  `generics/anonymous_subprograms_2019_hand`). Open: instantiating a generic protected type (parser request).
- 2019 mode views and private variables (S7, #32): view declarations (`E1901`, `E1902`),
  `'converse` (`E1621` on a non-view), view ports and signal parameters (`E1903`), views in
  generic units; private variables visible only in the protected body (`E1105`), method aliases
  that forward to a private variable (`E1906` for an alias of anything else), and aliases of
  methods of shared variables and variables (#237; prefixes other than an object name are
  `E9999`; a call of such a failed alias reports nothing more, #245). View ports, nested and array views, and calls through method aliases simulate
  (`hierarchy/mode_view_*`, `protected/private_variables`, `protected/method_alias_objects`), and so do signal
  parameters with a view (record, nested, `'converse` and array views, slices, several calling processes; #254), also associated element by element (#265).
  `buffer` and `linkage` subprogram parameters are `E1236`.
- 2019 front end (corpus fixes #173 to #180): `A'INDEX(n)` and `R'RECORD` type marks, `R'VALUE` of a range attribute (#282; `composite/range_value`), signals
  and variables with bounds from the initial value, variable ports of protected types, `open`
  sub-element actuals, formal packages of design entities; entity subprograms with bodies in an
  architecture. Initializer-bound signals elaborate and run (#187; records and arrays of records with such fields, #215);
  variable ports elaborate and run (#188): the actual is a shared variable or a variable port
  of the same type, never `open` or unassociated (`E1311`).
- Deferred constants at elaboration (#295): generic defaults, ranges and generate bounds read the full declaration in the package body (`generics/deferred_constant`).
- Differential corpus (#163, #181): `implicit/` covers the implicit operations end to end (radix strings, arithmetic and logic, access and file operations, edges, record equality, `hand_edges` for the LRM cases where the reference simulator deviates); `hierarchy/external_*` covers external names as port actuals (absolute, relative, generate-indexed, package). Implicit native symbols separate the operand-shape form with `#`.
- 2019 conditional expressions (`a when c else b` in initial values, assignments, actuals,
  `return` and parentheses), the conditional `return` and `unaffected` in variable and force
  assignments analyze, lower and run (#33; difftest `process/conditional_expr_2019`,
  `process/unaffected_2019`). Named return values (`return r of t`) of array type take the
  result subtype from the call's context and run (`process/named_return_2019`), so do record
  types and named return values in generic subprograms, generic packages and protected
  methods (#239, `process/named_return_record_2019`, `process/named_return_generic_2019`);
  so do scalar types, whose `r` takes the target's range (#327,
  `process/named_return_scalar_2019`). `return when c;` in procedures runs (#239,
  `process/return_when_2019`).
- Static folding: checked i64, values checked against the target subtype (`E1403`) and the base
  range, division/mod by zero and negative exponents, static length mismatch (`E1405`).

## syntax (`crates/yavs-syntax`, M0)

Hand-written lexer, conditional analysis on the token stream and recursive-descent parser for
VHDL-2008 and VHDL-2019 into an owned AST (`yavs parse [--std 2008|2019] files...`). Diagnostics
go through `yavs-diag` (human, short and JSON formats; snapshots in `tests/diag/`). A simple PSL
subset runs as implicit checker processes, also in `-- psl` comments with `--psl` (#226); other PSL is skipped
with one `W0004` warning per design unit (#138).

Vendor libraries (#119, #61): `tools/corpus/unisim-2008.toml` reads `unisim`/`unimacro` in place from `$YAVS_VENDOR_UNISIM` (skipped when unset): all 1073 files analyze (952 of the 1073 before the Synopsys packages; the 121 failures were exactly the missing `std_logic_arith`, `std_logic_signed` and `std_logic_unsigned`), with the VITAL (#118) and Synopsys (#117) packages built in. The Synopsys packages `std_logic_arith`, `std_logic_unsigned`, `std_logic_signed` and `std_logic_misc` are the original files, unmodified, in `lib/ieee/{2008,2019}` (`lib/SOURCES.md`, `lib/LICENSE-SYNOPSYS`), visible without a flag; `tests/corpus/synopsys/` (20 testbenches, reference simulator or the original package text for 6, `--threads 1` and `4`) passes 20 of 20. The 6 hdl-modules files that need `unisim` analyze against it.

Parse corpus: `tools/fetch-corpus.sh` fetches the corpora (default `~/.cache/rsim-corpus`), then
`YAVS_CORPUS=~/.cache/rsim-corpus cargo test -p yavs --features corpus` compares them with the
expected lists in `tests/parse-corpus/` and fails if `YAVS_CORPUS` is unset (`YAVS_BLESS=1`
updates the lists). Plain `cargo test --workspace` still parses `lib/` and runs the snapshots.

| Corpus | Files | Pass | Expected failures |
|---|---:|---:|---|
| `lib/` std and ieee, 2008 and 2019 (in repo) | 83 | 83 | none |
| External test runner library (`vunit`) | 194 | 194 | none |
| External verification library (`osvvm`) | 36 | 36 | none |
| Third-party parser library sources (std, ieee2008, synopsys, vital2000) | 37 | 37 | none |
| Reference simulator regression and parse tests | 1401 | 1366 | 35: tests of invalid syntax (most of them), VHDL-87 syntax, `abs x ** 2`, one `` `if `` that needs `--define` |
| Conformance suite `vhdl-93` | 3782 | 3244 | 538: 533 in `non_compliant/analyzer_failure`, 2 other non-compliant, 2 broken book excerpts, 1 VHDL-93 file using `default` |
| Second reference simulator testsuite | 3216 | 3009 | 207: `analyze_failure` tests and fuzz crashers, 93 code using 2008 reserved words |

Throughput on the `vunit` corpus (read, decode, lex, preprocess and parse; release build, one
thread, Intel Core Ultra 9 275HX, warm cache): about 5,600 files/s and 60 MB/s (best of three: 5,769 files/s, 62.4 MB/s).

Fuzzing: `cargo +nightly fuzz run parse` (lexer, directives, parser and rendering in both
standards); two 5-minute runs with 8 workers (210k executions in total) found no crashes or hangs.

## sem design units (`crates/yavs-sem`, M1 slice S5)

Entities, architectures (`of e` and `of lib.e`), components, instances (component, entity,
configuration; `l : c;`), generic and port maps (positional, named, sub-element, conversions,
`open`), configuration specifications and declarations (block, generate index, range and
alternative, component configurations with incremental binding), default binding
(`Db::default_binding`, `W1703`), manifest dependencies on named architectures and default-bound
entities. Negative tests in `tests/diag/sem/unit/`; unit tests in `src/unit/tests.rs`. Generic
types, subprograms and packages on components and blocks go through `generic.rs`. Open:
subtype indications as generate specifications.

## lower (`crates/yavs-lower`, M2)

HIR to IR for every architecture the elaborator instantiates: generics, ports (unconstrained
array ports take the actual's bounds), blocks (guarded ones with their guard process), generates
(one instance data per region), processes and concurrent statements; scalar and array
expressions with runtime descriptors (bit-planes for logic arrays), aggregates, conversions,
attributes including 'path_name/'instance_name, 'driving/'driving_value and signal attributes
of elements and slices; all sequential statements including case over arrays and matching
case, aggregate targets, every wait form; subprograms with signal parameters of every mode
(arrays too, `DriveSlot` for out/inout), signals assigned in subprograms declared in a process (driven through that process's slots, #206), `signal out` actuals passed the driver slot of their own longest static prefix (#231), copy-out of logic elements and slices, individually associated parameters (`pr(x(0) => s)`, `swap(v(4) => a, v(5) => b)`, record elements, slices; unconstrained formals take their bounds from the parts; a signal formal of several parts, also in different signals and field by field for records, is a reference of several parts, #71); parameters associated through conversions (copy-in through the actual part, copy-out through the formal part, #229); nested names of signals of arrays of arrays
(`m(i)(j)`, `m(i)(7 downto 4)`, `g(i, j)`, below a slice `m(0 to 1)(0)`, through subtype
aliases) as targets, actuals and sensitivity (#89, #230); 2008 array-valued pieces of
positional aggregates as values and targets, of named aggregate targets with range or element choices (#244, #264; a lone non-static element choice too, every target length-checked against its value), and of named aggregates and arrays with open
element subtypes as values (#89, #230; without a constrained context they take the direction of their range choices, a mix is `E1334`; an array value under `others` is `E1323`, #242); `&` on arrays of arrays and of records (#234); package
objects at run time (`Lowerer::package`: constants evaluated once, composites with run-time
bounds too, shared protected variables, package signals and files); composite architecture
constants evaluated once per region (#73); aliases of record elements of ports (#292) and of static logic elements and slices of variables (#296); `'range` of a package signal in subtypes (#294); bounds of record elements of ports in constants, signal subtypes and generate ranges (#304, difftest `composite/record_port_bounds`, `composite/record_in_port_bounds`); `'subtype` of objects bounded by a port, and aliases of record parts of variables keeping their element bounds (#315, difftest `composite/alias_object_subtype`); aggregates with a port's range as choice in initial values (#311, difftest `composite/agg_port_range_choice`); a lowering failure without a diagnostic is an ICE; non-protected shared variables (`--relaxed`) in
packages, architectures, blocks and generates, one per declaring region, whose processes run
serially (#285), including types holding access values, whose objects live in the shared heap (#303). `tests/corpus.rs` runs every corpus group plus
`crates/yavs-lower/tests/vhdl/run` (blessed from the reference simulator) through yavs-elab and the kernel; the external test runner's
minimal testbench runs (#110). Design and conventions: `crates/yavs-lower/DESIGN.md`.
`'element` attributes of objects with open element bounds (unconstrained ports, aliases of external names, parameters) read the object's descriptor (#235, #243; difftest `composite/element_attr_open`).
A `'element` chain of any depth (`x'element'element'left`) walks the object's descriptor level by level; sem counts the depth in `FromObject` (#256; difftest `composite/element_depth`).
A formal such as `ow(open)(7 downto 0)` has the formal's element bounds (`'element'left` is 7 whatever the actual's) and rejects an actual whose element length differs (#257, L79; difftest `composite/element_open_formal`, `element_open_formal_mismatch`).
A folded constant read as a whole carries its subtype's bounds, element bounds included, not its initialiser literals' (#247).
Composites (lower2): records (2008 unconstrained elements with runtime descriptors), arrays of
composites, multi-dimensional aggregates, composite constants, access types (`new`, `.all`,
`deallocate`), process-local protected objects (`CallProt`), protected state initial values
that call the body's methods (#204), subprograms nested in protected methods calling the
body's methods and naming its variables (#221), file objects and textio,
`force`/`release` of scalar and composite signals and their parts (every scalar subelement,
simple, conditional and selected forms, default mode from the target's port, a member of a
composite resolved as a whole is `E1362`, #68), signals of arrays of records and of records with
array-of-record fields (struct of arrays; whole, element, field, bit and slice targets,
element sensitivity, resolution as a whole, #200; field and element bounds from the initial
value, a partially constrained subtype or a port's actual, signal parameters of such types,
package signals of record types, attributes and implicit signals of parts over several
fields, #215), shared protected variables (created through `call_shared`),
user resolution functions (yavs-elab resolver over `yavs_interp::resolve`, once per element, on byte and logic leaves, #293). Difftest: access 3/3,
composite 6/6, force 8/8, protected 6/6, resolution 4/5, textio 2/3 at 1 and 4 threads.

- An alias of a constant with a static value in a subprogram has no storage; its uses lower the aliased name (#300; difftest `composite/alias_constant`).

- Entity statements (passive assertions and processes in an entity, LRM 3.2.4) lower with the architecture's, entity ones first (#301; difftest `hierarchy/entity_statements`).

- Aggregate variable targets of a record value, `(x, y) := f;`, positional elements (#302; difftest `composite/record_aggregate_target`).

- Aggregate targets of records with named fields, nested aggregate targets (records of records, arrays of arrays, records holding arrays) and matrix targets, for variables and signals (#350, L98; difftests `composite/aggregate_target_{logic_fields,named_record,nested_record,nested_array,matrix,matrix_cover}`, discrepancies `aggregate-target-{record-names,nested-record,nested-array,matrix}`). Still `E2001`: record signals, arrays of composites and arrays of logic arrays as elements of signal targets; arrays of logic arrays and matrices of composites as targets; non-static choices and pieces in matrix targets.

- Aggregate targets naming one object more than once (several choices in an association, or the same static name twice) are `E1335` (#307, L96; `tests/diag/sem/expr/agg_target_*`, discrepancies `aggregate-target-*`).

## sem robustness

`cargo +nightly fuzz run sem` (in `fuzz/`, seeds `tests/diag/sem`, `lib/`) parses and analyzes each input
in 2008 and 2019 against yavs's std/ieee and fails on a panic, a hang (`-timeout=20`), an RSS over 2 GB or
any diagnostic carrying "this is a bug in yavs". The negative suite fails on any ICE too, xfail or not.
`yavs analyze` over all ten `tools/corpus` manifests (debug build, so the HIR validator runs; `timeout 60`
per file) shows no panic and no timeout.

External library corpora (`tools/corpus/gen_external.py`, validated per file with the reference simulators; rejects and reasons in `tools/corpus/rejects/`): `en_cl_fix`, `open-logic`, `poc`, `json-for-vhdl`, `vhdl-extras`,
`vhlib`, `fletcher`, `satcat5`, `surf`, `neorv32`, `neotrng`, `vhdl-compliance`, `vhdl-interfaces`, `vhdl-tool-checks`,
`vhdl-ideas` and `vunit-tests`. Several carry a `[testbenches]` section (top units, runner, pass
condition) for the M2/M4 simulation tests. Results: `tools/corpus/RESULTS.md`.

Analysis corpus failures, classified 2026-10-19 (release build of main, all 29 manifests, cascades
followed to the first failing file): 3552 PASS, 299 FAIL. 292 stop on the missing Synopsys
packages (#117, #171; approach is the user's decision): `std_logic_misc` 126 (fletcher 89, vhlib
19, open-logic 18), `std_logic_unsigned` 85 and `std_logic_arith` 81 (surf); the reference
simulators accept these files. 6 (hdl-modules 2008 and 2019) need the vendor `unisim` library
(#61; they analyze with `--extra-map unisim:...`). 1 is intended (`vhdl-ideas/sim/frequency.vhdl`,
E1101, L22/L89). No other root cause is left.

| Run | Fuzz time | Executions | Crashes found | Fixed | Open |
|---|---:|---:|---:|---:|---|
| 4 workers, ~1.1 h CPU in total | ~27 min wall | ~280,000 | 3 distinct ICEs (procedure call left `ExprKind::Error`; physical-literal unit undeclared; alias of a selected object name) | 3 | none |
| 1 worker, 2 x 30 min (`parse` ~99,000 runs, `sem` ~98,000 after the fix; seeds `tests/diag/sem`, `tests/corpus`, `lib/`) | ~60 min wall | ~197,000 | `parse`: none. `sem`: `'record` on a range under 2008 was an ICE (#359) | 1 | none |

The procedure-call ICE was fixed on main in parallel; the other two are fixed here, all with
regression tests in `tests/diag/sem/fuzz/`. The last 3 chunks (~190,000 executions) found nothing.

## logic (`crates/yavs-logic`)

Done ahead of M3: `StdULogic` with the IEEE 1164 tables (checked against the vendored package
body), `Logic<R>`/`LogicVec` packed vectors in two layouts (nibbles, bit-planes), `BitVec` fast
path, numeric_std add/sub/compare/shift/rotate/integer conversion with X propagation, exhaustive
and property tests, criterion benchmarks. Default layout is bit-planes; see
`crates/yavs-logic/LAYOUT.md`; the user confirmed bit-planes everywhere, signals included (docs/plan.md, 2026-10-18).

## llvm (`crates/yavs-llvm`)

LLVM tier (M6), behind the `llvm` feature (`cargo test -p yavs-llvm --features llvm`; `cargo build
-p yavs --features llvm` for `yavs run --tier=llvm`). `compile` lowers every non-elaboration IR
function to one LLVM function on the interpreter frame's registers: register arithmetic,
compares, checks, address arithmetic and branches are native; other instructions go through a
callback into the interpreter or return to it (calls, waits, reports, every slow or error path).
Difftest corpus under `--tier=llvm` (`difftest.py --candidate yavs-llvm`): 300/300 at 1 thread (289/289 at 4 shuffled threads, measured before the corpus grew),
the same as the interpreter. Over the corpus programs 463545 instructions are compiled inline and
292347 executed by the interpreter (Load 86317, AddrOf 50301, Stmt 22193, Alloc 19994, Store
18879, DopeLen 17142, LayoutOf 15683, Copy 11453, ...; issues #334 to #341). Whole corpus, one
run after the other, debug build, 2026-10-19: 30 s interpreted, 123 s with `--tier=llvm`, almost
all of it compiling every function at O2 before time 0 (tier-up of hot functions only is #125).

Spike (before M6), done: llvm-sys 211 bindings (docs/decisions.md), ORC LLJIT with a portable CPU, O2 through
`LLVMRunPasses`, Rust runtime callbacks as absolute symbols, a checked i64 sum that traps into
Rust on overflow, a process body reading and writing a `#[repr(C)]` struct, a 10k-instruction
function checked against its Rust reference semantics, an unresolved runtime symbol reported by
name, and background compilation on one thread with an atomic function-pointer swap while the
calling thread keeps running in the "interpreter". Windows: `crates/yavs-llvm/WINDOWS.md`
(CI-verified: `cargo test -p yavs-llvm -p yavs --features llvm` passes on `windows-latest`, `ubuntu-24.04-arm` and `ubuntu-24.04`).

JIT cost of one generated function (`examples/measure.rs`: `state[w] := state[a] + (state[b] and
255)` with an overflow check, 13 IR instructions and 2 basic blocks per statement). One process
per row, release build, Intel Core Ultra 9 275HX, LLVM 21.1.8, x86-64-v2. Time is IR build +
passes + codegen and link (first lookup); memory is peak RSS growth over a process that already
created the JIT.

| IR instructions | opt | passes (ms) | codegen + link (ms) | total (ms) | peak RSS growth (MiB) |
|---:|:--:|---:|---:|---:|---:|
| 1,002 | O2 | 6 | 12 | 18 | 12 |
| 10,011 | O2 | 50 | 95 | 146 | 28 |
| 20,021 | O2 | 103 | 225 | 330 | 46 |
| 40,041 | O2 | 206 | 589 | 799 | 79 |
| 100,101 | O2 | 630 | 3,100 | 3,800 | 231 |
| 299,001 | O2 | 3,761 | 24,489 | 28,280 | 650 |
| 10,011 | O1 | 30 | 92 | 123 | 25 |
| 100,101 | O1 | 281 | 3,009 | 3,301 | 199 |
| 1,002 | O0 | 0 | 3 | 3 | 8 |
| 10,011 | O0 | 0 | 21 | 22 | 15 |
| 100,101 | O0 | 2 | 181 | 193 | 92 |
| 299,001 | O0 | 6 | 637 | 674 | 264 |

Codegen at O1/O2 grows faster than linearly in the size of one function (10x the instructions
costs 25x to 30x the time); O0 stays close to linear. Proposed threshold: O2 up to 40k
instructions per function, larger units stay interpreted (docs/decisions.md).
## lib (`crates/yavs-lib`)

Done: library directories (`yavs-lib.toml` manifest + `_YAVS_LIB` lock, rule 14), `NAME:PATH`
mapping, atomic manifest updates under an exclusive lock, shared-lock reads that never write,
on-demand closure query (unknown library/unit with suggestion, cycles, stale hashes), multi-process
concurrency test. The standard is stored per file (manifest format 2). The analyzer supplies `UnitRecord`s (kind, name, dependencies) per file. A directory with a manifest from before the rename to yavs fails with L0004 (re-analyze).

## kernel (`crates/yavs-kernel`)

LRM 14.7 cycle against an abstract `Process` trait, tested with hand-written processes: time with
runtime resolution and checked arithmetic, drivers with transport and inertial (reject limit)
preemption per element, resolution per source segment, `'event`/`'active`/`'last_event`/
`'last_value`/`'last_active`, force/release/deposit, postponed processes, delta limit, parallel
evaluation with ordered commit (threads and shuffle invariance tested on 1000+ processes),
`SimControl` with run/step, interrupt, value-change bus and phase callbacks. Design in
`crates/yavs-kernel/DESIGN.md`.

Kernel asks of docs/design/ir.md, all done: K1 uncollapsed ports with conversions in the update
phase (no extra delta; a port of several parts in different signals converts all of them in one
call, `Builder::port_parts`, #74), K2 kernel-maintained `'stable`/`'quiet`/`'delayed`/`'transaction`, K3
`'driving_value`, K4 `'driving`, null transactions and guarded `bus`/`register` signals with
disconnection time, K5 null-range signals, K7 bit-plane storage for logic signals (bytes for
everything else). K6 (buffered user-file streams) was dropped: disk files are the interpreter's
own and their callers serial (#82). Serial marking reaches the kernel through `Process::serial`,
so shared-variable, protected-type and file users run in order at any `--threads` (#107). The order
of the processes of one cycle is fixed (#252): elaboration order at initialisation; then the
processes with one suspension point in elaboration order, then the others
(`Process::several_waits`) in reverse, signal-resumed before timeout-resumed; concurrent
procedure calls (`Process::concurrent_call`) before the other several-wait ones (#281). Observers
schedule and cancel wake-ups at an absolute time (`Hook::wake_at`, `Hook::cancel`,
`Observer::wake`, #268): a wake-up gives a cycle at its time with the usual phase callbacks,
forces and deposits from it apply in that cycle, a wake-up in the past is E3005, and the
simulation does not end as idle while one is pending (scenario tests, thread invariance at 1, 4
and 8 threads).

Benchmark (`cargo run --release -p yavs-kernel --example bench -- 10000 <work>`), 24-core host
under load from other agents (runs vary by about 15%), 8 worker threads. "Before" is the byte-only
kernel of the first round, run back to back with "after".

| case | threads | before | after |
|---|---|---|---|
| 10000 counters, work 0 | 1 | 3.3 s (3.0 M events/s) | 3.6 to 3.8 s (2.7 M events/s) |
| 10000 counters, work 0 | 8 | 3.1 to 3.3 s | 3.2 to 3.8 s |
| 10000 counters, work 1000 | 1 | 7.1 s (1.41 M events/s) | 7.8 s (1.29 M events/s) |
| 10000 counters, work 1000 | 8 | 4.3 s (2.35 M events/s) | 4.7 s (2.14 M events/s) |
| 1024-bit std_logic bus, 4 drivers, 10000 edges | 1 | 0.36 s (28 M resolved elements/s) | 0.115 s (89 M/s) |
| 1024-bit std_logic bus, 4 drivers, 10000 edges | 8 | 0.82 to 0.87 s | 0.33 to 0.39 s |

Before, the bus was 1 byte per element with a per-element resolution callback; after, it is
bit-planes with plane-wise resolution (3x faster). Scalar byte signals pay 10 to 15% for the
storage enum and the port/implicit machinery. With trivial processes the serial update and
commit phases dominate, so threads gain little.

Parallel update of independent signals (#103: no port, no composite group, one task per
signal). Same host and load (load average 12 to 29), median of 3 to 6 interleaved runs, before
against after:

| case | threads | before | after |
|---|---|---|---|
| 10000 counters, work 0 | 1 / 4 / 8 | 5.9 / 5.8 / 5.1 s | 5.4 / 3.6 / 3.9 s |
| 10000 counters, work 1000 | 1 / 4 / 8 | 13.0 / 8.4 / 7.8 s | 12.0 / 7.6 / 5.5 s |
| 5000 std_logic signals x 2 drivers | 1 / 4 / 8 | 5.6 / 5.4 / 5.0 s | 5.5 / 3.6 / 4.6 s |

The queue drain, port and group signals, wake and commit stay serial (DESIGN.md "Open
questions").

## ir and interp (`crates/yavs-ir`, `crates/yavs-interp`, M2)

`yavs-ir`: Program/Func/Block/Inst with 32-bit ids and a `StmtId` per instruction, runtime type
descriptions with descriptor-based layouts (bit-plane logic arrays, records, arrays with dope),
compact constants, natives by NATIVE.md symbol, foreign references, flag propagation
(`MAY_SUSPEND`, `TOUCHES_SHARED`), resume numbering, a linear verifier and a text dump
(`crates/yavs-ir/README.md` says what is stable). `yavs-interp`: `InterpProcess` implements the
kernel `Process` (heap frame stack, waits inside procedures), checked arithmetic through
yavs-std, checks, signal ops mapped 1:1 to `Ctx`, natives (`now`, `stop`, `finish`, scalar
`to_string`/`'image`), VHPIDIRECT calls of scalars, generational heaps with precise-root GC (descriptor-aware, deallocated slots quarantined until a suspension or, in a long activation, a recycle between instructions, #355; the shared heap, where serial processes allocate, collected between cycles from the shared variables, the objects elaboration made and every serial process's frames, #303),
serial marking, kernel-free `call` / `call_shared` for elaboration (allocations with the shared state, at any call depth, in the shared heap, #246), protected objects,
std.textio (read/write of every textio type, linear in the line length over many calls,
readline/writeline/tee, hread/oread/sread, justify), TEXT file ops including the 2019 ones, std.env (getenv, dir_*, tool and version
strings, time conversions, resolution_limit, the 2019 time_record functions, call paths and
the VHDL assert state), VHPIDIRECT calls with scalars, arrays,
records and access values (copy-back included), signal references of several parts in
different signals (`SigParts`, `SigSlice`, #71), runtime errors with the VHDL stack as
`SimError::trace`, and debug hooks in the style of commercial simulators (docs/design/ir.md 3.4).

Tests (`cargo test -p yavs-ir -p yavs-interp`): 11 end-to-end designs run through the kernel at
1 and 4 threads with identical traces (clock and counter, delta ordering, wait forms with a
wait inside a procedure, overflow with stack trace, bit-plane ops across blocks, access types
and stale handle, GC roots, natives, foreign call, elaboration call, serial marking), 5 debug
tests (conditional line breakpoint with hit count, step over, step in with frame and locals,
global step across processes, signal breakpoint and process table), verifier and layout tests.
Also 5 foreign marshalling tests, 5 textio/file/env tests and a protected-type test.
Not done: `Elab*` instructions (yavs-elab), records with runtime layouts beyond `LayoutOf`,
binary file read/write of records and arrays of composites, the PSL natives, `SetVhdlAssertFormat` with a non-empty format.

std.reflection (`yavs-interp/src/reflect.rs`, `yavs-lower/src/reflect.rs`): all 123 mirror
methods and `'reflect` (value and subtype mirrors, built from the value and its descriptor;
private-heap mirror objects traced by the GC). End to end through `yavs run` for every class:
enumeration, integer, floating, physical (TIME in resolution-limit units, `yavs run --resolution`, #253), record, array
(multi-dimensional, unconstrained parameters, null), access, file and protected mirrors, of
variables, constants, signals, ports and type marks, with mirror variables and generic
subprogram instances (`tests/corpus/reflect`, 12/12 at 1, 4 and 4 shuffled threads; 6 against
the reference simulator, 6 `ref=hand` where it is wrong or crashes). Scalar subtypes whose range
is not static (generic or parameter bounds) get their range at the `'reflect`, also as element,
index or designated subtypes. The CLI has no resolution option yet, so TIME mirrors are
exercised end to end in femtoseconds only (other resolutions: a unit test).

- A `report` in a function run without a kernel (elaboration time) prints `** Note: (init): msg` on stderr and goes on; severity failure stops (#299; difftest `report/elab_report`); it is counted in the VHDL assert counts, so an error exits 1 at the end (#349; `report/elab_report_error`, catalogue `elab-time-report-error`).

### Shared heap collector: overhead measurement (#303, 2026-10-19)

Release builds, `origin/main` (84bd5988) against the commit before the collector (0a89b92f),
busy 24-core host (load about 30), 5 alternating repetitions per case, median CPU seconds
(user+sys) and [min-max]. Collector counters from a temporary instrumented build (not
committed); no profiler was available (`perf_event_paranoid` 4).

| Case | Before | After | Delta | Collections | GC time | Peak RSS before / after |
|---|---|---|---|---|---|---|
| a. no access types (counter, shift register, 5000 clocks) | 5.34 [4.7-6.0] | 5.59 [5.0-5.7] | +4.7% (noise) | 0 shared, 5 private | 0 | 18 / 18 MB |
| b1. 2M textio lines, one activation, file output | 3.72 [3.6-4.0] | 3.73 [3.4-4.1] | +0.3% | 0 | 0 | 551 / 543 MB |
| b2. same, serial process (touches a protected object) | 4.74 [4.5-6.0] | 4.52 [4.4-4.8] | -4.6% | 0 | 0 | 551 / 543 MB |
| c1. 100k scoreboard push/check | 21.8 [18.0-23.8] | 21.5 [20.1-22.8] | -1.5% | 15 | 0.65 ms | 36 / 37 MB |
| c1b. scoreboard with a 50k-item backlog plus churn | 19.9 [17.0-23.5] | 20.1 [17.3-23.0] | +0.6% | 1 | 11.8 ms | 48 / 50 MB |
| c2. sparse memory model, 10k words x 3 passes (33 MB live) | 17.7 [12.3-19.8] | 17.3 [13.8-19.2] | -2.6% | 2 | 91 ms (0.6%) | 294 / 313 MB |
| c3. coverage model, 256 bins, 30k samples | 17.5 [16.1-19.6] | 18.6 [15.9-20.7] | +6.7% (noise) | 0 | 0 | 38 / 38 MB |
| d. library testbench with queues (scoreboard) | 31.6 [30.0-36.6] | 31.8 [30.8-34.1] | +0.6% | 2 | 16.7 ms (0.05%) | 314 / 266 MB |

Findings:

- The overhead is not measurable: every delta is inside the run-to-run spread (10-30%) on the
  shared host. The counters bound it: collection time is at most 0.6% of a run (c2), typically
  below 0.05%; `publish_shared` (run at every process suspension) costs 70 to 130 ns per call,
  below 0.1% even for the non-serial design (a, 30k calls, 2 ms). It builds the frame roots
  before it checks for the shared state, so non-serial processes pay it too; harmless, and
  since #356 it checks first.
- Per collection: mark is proportional to the live set (about 1.1 ms per MB of live data
  when one large object holds handles: c2 marks 33 MB in 38 ms; c1b marks 150k objects, 8 MB,
  in 11 ms; small scoreboards 35 us). Sweep and recycle are proportional to the slot count,
  not the live set (d: 14.5 ms for 527k slots). Large long-lived structures are re-marked on
  every collection, but the trigger (more allocated than both the threshold and what survived)
  makes the cost amortised: a collection happens only after allocating as much as it marks.
- Leak check, serial process dropping a 1 KB `new` string per iteration with a wait (400k
  iterations): before 10.2 MB, after 10.2 MB, after with collection disabled 434 MB (1.1 KB per
  iteration). Same process not serial: 10 MB in all three. Textio lines and scoreboard items
  are deallocated by their libraries, so they do not grow even without collection (200k
  lines with waits, 200k scoreboard items: flat at 9 and 37 MB in all three). The collector
  restores the boundedness serial processes had when they allocated in their private heap.
- Gaps: objects of elaboration are sealed and never collected (sealed set 9 KB for the
  scoreboard designs, 147 KB for the library testbench). No collection runs inside one
  activation of a serial process (b1: 2M lines in one loop without a wait never reach a
  safepoint, 550 MB of quarantined slots, about 270 bytes per deallocated line); this is the
  same before the collector. Fixed by #355: a long activation recycles freed slots between
  instructions; peak RSS of b1 and b2 went from 544 MB to 10 MB (3 runs each, CPU time
  unchanged within the spread, same output).
- Ranking by measured benefit: (1) a safepoint or slot reclaim inside long activations
  (memory only; 550 MB in b1, equal before and after); (2) dirty flags on protected-object
  writes, only worthwhile for live sets of hundreds of MB (worst pause measured 38 ms, 0.6%
  of a run); (3) allocation-volume triggers already exist; (4) per-process heaps collected in
  parallel: no measured benefit, the shared heap work is below 1% of every run.
  Recommendation: the current overhead is acceptable; no redesign needed.
### Startup against simulation per test (2026-10-19)

Question: how much of a small test's time is startup, to judge an analyzed-unit cache. Release
build of `origin/main` (92dfc349) with temporary phase timers (not committed), interpreter tier,
the external test runner's `verification_components` suite (579 tests, 576 reached the timer
line; the other 3 end through a different path), host load average about 15 from other agents
(absolute times are inflated, shares are what matters). The runner analyzes each file once up
front (7.6 s for the suite, `-p 4`); every `yavs run` then re-parses and re-analyzes the closure
of its top level from the sources.

| Phase (ms per test) | median | p90 | Share of summed per-test time (`-p 4`, 576 tests, 2672 s) |
|---|---|---|---|
| parse + analyze of the closure | 461 | 596 | 10.3% |
| of which std/ieee (23 units) | 178 | 185 | 3.7% |
| of which test runner library + testbench (about 112 units) | 212 | 233 | 4.6% |
| of which third-party verification library (about 18-30 units) | 70 | 189 | 1.9% |
| library manifest / closure | 8 | 9 | 0.2% |
| elaboration (expand hierarchy) | 3175 | 4138 | 56.8% |
| lowering | 18 | 30 | 0.4% |
| package and instance initialisation (`build_full`) | 294 | 596 | 7.4% |
| simulation | 418 | 3003 | 25.3% |
| LLVM compile (`--tier=llvm`, 3 tests) | 14000 to 43000 | | not in the default run |

- Suite wall time: 671 s at `-p 4` (plus 7.6 s runner analysis). The 50-test subset (every 11th
  test) at `-p 1` takes 231 s (in-process median 4.1 s, startup 3.5 s of it); a reference
  simulator takes 9.7 s for the same 50 tests (4.5 s of runner compile for the whole suite).
- Units re-analyzed: median 153 units from 110 files per test (at most 181), 89341 unit analyses
  over the suite for fewer than 200 distinct units; no user-edited file is among them, all are
  library or testbench sources that did not change.
- Cache estimate: a cache load cost of 10% of analyze time (not measurable: no serializer
  exists) saves 247 s of the 2672 s, 9.2%; the suite would take about 610 s instead of 671 s at
  `-p 4`. A free cache would save 10.3%. Elaboration is the dominant startup cost (56%), and it
  varies by testbench from 46 ms (`tb_memory`) to 4.4 s (`tb_axi_stream`); it is the thing to
  look at before a cache.
- Not measured: the two testbenches of the neural-network project (their runner needs the Python
  FFI build of the runner, not set up here).



## elab (`crates/yavs-elab`, M2)

Done (without processes, which need yavs-lower and yavs-interp): `find_top` (`ENTITY`,
`ENTITY.ARCH`, `CONFIGURATION`), `elaborate` as a pure function (restart test: two elaborations
and two kernel builds give identical results), hierarchy with LRM `'path_name`/`'instance_name`
in both LRM and original spelling, unlabeled processes `_pN`, generics (actuals, `-g` overrides
through the yavs-std `'value` parser, defaults, range check), constants on demand, for/if/case
generates, blocks, entity/component/configuration instances, binding (configuration declaration,
then specification, then default binding), signals split into kernel leaves (records as struct
of arrays, `std_ulogic`/`bit`/`boolean` as bit-planes), the sub-element map
(`NetGraph::select`), uncollapsed ports with unconstrained formals taking the actual's bounds (an open `in` port, the default's, #218),
static expression actuals, per-process merged drivers, multiple-source check across the
hierarchy, LRM 14.4.2 body-before-call check, `Elaborated::build_with` into the kernel.
Elaboration-time evaluation folds globally static expressions with `Db::fold_with`, evaluates
aggregates of any number of dimensions, indexing, slices, fields, concatenation and composite
equality itself, runs the other predefined operations (shifts, `TO_STRING` of reals and
physicals, ...) through their yavs-interp natives (#220), and runs user function calls in
yavs-interp (the function lowered alone; any composite, arrays of records and records with
array elements included, as arguments and results through `call_values`). A function that uses
package objects (impure registries in shared variables) runs against the elaborated packages in
elaboration order, constants with impure initial values are evaluated in declaration order,
package constants with impure initial values are read from the package data, and building the
simulation replays these calls (#77); an impure constant elaboration cannot evaluate at all
runs at simulation start with W2118 (#220); `'PATH_NAME`/`'INSTANCE_NAME` of a design unit are evaluated during elaboration (#314). A process's impure constants run once, when the process starts (#251; difftest `protected/impure_const_once`). Also done: package bodies and generic instance bodies
loaded before elaboration, a package needing a body without one (E2116), conversion functions
and type conversions in port maps (kernel converters; conversion functions of record ports and
arrays of records convert all leaves in one call in the same delta, #74; both conversions of
`inout` ports, #238; a formal-side-only conversion of an `inout` port, scalar and composite, #74), individual
association of unconstrained array ports and of record ports with unconstrained elements (#288), implicit signals (`'stable`,
`'quiet`, `'delayed`, `'transaction`, kernel-maintained, also named in process bodies and
subprograms, #240), package signals (`:lib:pkg:s`, every
signal of a loaded package, visible from every region), package data initialised once in
analysis order with `yavs_interp::init_package` (shared variables, constants computed once,
#183; a declaration constant calling its own package's function is E2114 before lowering, #197),
objects of generic package instances (design units with their own package data; instances in
packages, entities, architectures, blocks, generates, processes and subprograms add theirs to
that region, #101), objects of packages declared in those regions (generic or not, instances of
local generic packages in the same and nested regions and through an instance of an enclosing
generic package; `generics/nested_packages`, `local_generic_packages`,
`local_generic_packages_hand`, #34), shared variables of blocks and generates one object per block, generate
alternative or iteration, shared by the nested regions (#249),
external signal names bound in elaboration order (E2117), aliases of external signals in regions and processes (read, waited on, driven; #19), external names used directly in processes, concurrent statements and expression or `inertial` port actuals (bound once the hierarchy exists), external constants, external shared variables of protected types (method calls), external signals with records and aliases of parts of external names (#93), external names in subprograms of regions and packages, package paths of every class and external constants computed at run time (#255), guarded blocks and `bus`/`register`
signals, blocks with generics and ports, incremental binding, nested and record element
resolution, user resolution functions (one declared in an entity or architecture reads its
instance's data; in a block, generate or subprogram it is E9999, #164), records and arrays
resolved as a whole (every driving process drives every leaf, #193), also nested in another
composite (each outermost resolved subelement, every array element of one, resolved with its own
sources; resolution below it unused, #333), an external name as an `in` port actual as a signal name actual, same
delta (#186), linkage ports and mode-view ports, entities with generic types,
subprograms and packages (yavs-sem `Db::instantiate_entity` clones the entity and architecture
once per distinct map of those actuals, with its own mangled identity; generic constants stay
instance data; #75; components and blocks with them, configuration specifications in such an
architecture, #222), `others` in a configuration after component configurations naming
specification-bound instances, 2019 variable ports denoting the shared variable at the end of
their chain of actuals (architecture, block, generate or package shared variables, through
entities, components and blocks; a top-level one is E2119; #188), a TIME unit below `yavs run --resolution` anywhere in the elaborated design is E2120 (LRM 5.2.4.2, std and ieee exempt), an incremental binding repeating the entity aspect of its configuration specification (E1702 only for a different one), non-static expression actuals of `in` ports and of parts of
them driven by their equivalent IR process one delta later (records leaf by leaf, ranges from
the instance's generics checked, scalar parts checked against their subtypes (#263), expression actuals in a binding indication's port map),
static expression parts in the formal's initial value, unconstrained formals bounded by a
static value or a conversion function's result subtype, type conversions of record ports and
arrays of composites (#74), conversions on unconstrained ports bounded by the conversion's type
mark or subtype, composites included (#262), `inertial` signal-name actuals as expression actuals, whole
formals (#233) and parts of an individual association (#261).

Processes: `Elaborated::lower` lowers every architecture owning a process (yavs-lower) and
`Elaborated::build` creates one yavs-interp process per process instance with per-instance data
(restart test with the interpreter).

CLI: `yavs elab` and `yavs run` per docs/cli-contract.md (closure from the manifests, design
revision = newest standard in the closure, `-g` after TOP, `@file`, exit codes, reports in the reference simulator's
format on stdout). Difftest: see `tests/corpus/RESULTS.md`; `generics/generic_types_2019` and the other generic-type cases pass since #75.

## ffi (`crates/yavs-ffi`)

Done: VHPIDIRECT call trampolines without libffi (SysV x86_64, Win64, AAPCS64), argument
classification in Rust, naked asm stubs, up to 64 arguments, and the VHDL to C mapping in the reference simulator's
convention (`vhdl::lower`). Tests: unit tests, hand-written ABI tests and 431 generated callees
compared against direct calls. All pass on Linux x86_64 natively, on aarch64 under qemu-user and
on Windows under wine. The cross commands (configured through the environment, no cargo config):

```sh
# aarch64 Linux (qemu-user, aarch64-linux-gnu-gcc, rustup target aarch64-unknown-linux-gnu)
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER="qemu-aarch64 -L /usr/aarch64-linux-gnu" \
  cargo test -p yavs-ffi --target aarch64-unknown-linux-gnu
# Windows x86_64 (mingw-w64, wine, rustup target x86_64-pc-windows-gnu)
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=wine WINEDEBUG=-all \
  cargo test -p yavs-ffi --target x86_64-pc-windows-gnu
```

`yavs run --load=LIB` end to end (#95): `'FOREIGN "VHPIDIRECT symbol"` subprograms (placeholder
bodies allowed) are lowered to `CallForeign`, bound to symbols in the `--load` libraries, then
the executable (E0106 cannot load before the simulation; symbol not found is warning W0108 before time 0 and E0107 only at the call), and
called with scalars, scalar `out`/`inout`, arrays (with copy-back), records and access values.
Test `crates/yavs/tests/foreign.rs` compiles a C library at test time: passes on Linux x86_64
and on aarch64 under qemu-user (`QEMU_LD_PREFIX=/usr/aarch64-linux-gnu` so the spawned `yavs`
finds its loader); on Windows x86_64 GNU the same testbench and error cases pass under wine by
hand with a MinGW DLL (the test cannot spawn the compiler inside wine). Native Windows (MSVC CI)
runs the test as written.

## vhpi export (`crates/yavs-vhpi`, `crates/yavs`)

Done (#266): the `yavs` executable exports the full `vhpi_*` C ABI of `vhpi_user.h` (42 symbols,
functions and the `vhpiFS`..`vhpiS` time constants) on Linux x86_64 and aarch64 (also in the
glibc 2.17 cargo-zigbuild release builds, where the linker exports more than the list) and in the
Windows `.exe` export table (MinGW checked; MSVC `/DEF:` and the `yavs run --vhpi` plugin load run in the `windows-latest` CI job, #124). No import library is built or shipped (user decision, 2026-10-18): a VHPI plugin brings its own.
`crates/yavs/tests/vhpi_exports.rs` checks the ELF `.dynsym` (Linux) and the PE export directory (Windows, in the `windows-latest` CI job, #124) against the header.

Done (#267): `yavs run --vhpi=LIB[:ENTRY]` (repeatable) loads plugins after elaboration,
fires start/end of simulation (and start of initialization) callbacks through a kernel
`Observer`, and ends the run on `vhpi_control(vhpiStop | vhpiFinish)` (status 0, new
`Hook::end`); load failures are `E0901`/`E0902` with exit 2. `crates/yavs/tests/vhpi_run.rs`
(2 tests, Unix only) loads the `yavs-vhpi` C test plugin through the CLI, both entry forms.

Done (#272, object-independent part; #123): simulation-phase and after-delay callbacks through
`yavs_vhpi::VhpiObserver` and two new kernel phases (`EndOfInit`, `NextCycle`): start/end of
initialization, next time step, start of next cycle, start and end of processes and start of
postponed (kernel phases `StartOfProcesses`, `EndOfProcesses` (also once after initialization), `StartOfPostponed`, #317), last known delta cycle (read-write), end
of time step (read-only; both also at time 0 after initialization when no delta follows, #347), after delay, each with its `Rep` form; `vhpi_remove_cb`,
`vhpi_enable_cb`, `vhpi_disable_cb`, `vhpi_get_cb_info` (removing a matured callback is legal);
disabled callbacks sit outside the per-phase sets and cost nothing per cycle. `vhpi_get_time`
and `vhpiResolutionLimitP` follow `--resolution`. `crates/yavs/tests/vhpi_run.rs` (7 tests) runs
`tests/plugin/sched.c` scenarios: timed finish at T in resolution units, after-delay chains and
a removed repeating one, read-write before postponed before read-only, enable/disable with
100000 disabled callbacks, misuse; output identical at `--threads 1` and `4`.

Done (#271, #272 value changes): `vhpi_put_value` on signals, ports and their record fields and
array elements, in every format `vhpi_get_value` produces (`Read::decode`) and integers into
logic vectors (#319) and `vhpiStrVal` text shorter or longer than a string (padded with NUL or cut, #345), queued in the state and handed to the kernel as
`Hook::deposit`/`force`/`release` when the callback returns, always in a new delta (#319):
`vhpiDepositPropagate` deposits, `vhpiForcePropagate` forces (the non-propagating modes are refused, user, 2026-10-19; force mode `out` for ports of
mode out/inout/buffer, `in` otherwise), `vhpiRelease` releases (back to the drivers' value, also after a deposit, #310); refused in the read-only phases;
constants, generics, records as a whole, bad values and `vhpiSizeConstraint` are `W0903`.
`vhpiCbValueChange` on signals, ports and sub-elements, keyed by kernel signal and dispatched
from `Observer::value_change`, once per cycle (one registered during a cycle's dispatch from the next cycle, #346), with the value filled in the requested format;
with none enabled the observer does not take the state lock. `crates/yavs/tests/vhpi_run.rs`
`vhpi_drives_a_clocked_design` (a counter clocked and reset only from VHPI, force, release and
deposit on a driven signal, value changes on a signal, a port, a bit and a record; identical at
`--threads 1` and `4`, `tests/put/expected.txt`) and `vhpi_put_value_misuse_is_reported`
(`tests/put/errors.txt`). Missing: `vhpi_schedule_transaction`.

Done (#309, variables): `vhpi_get_value` and `vhpi_put_value` (deposits) on process variables,
non-protected shared variables of architectures, blocks and packages, and their record fields
and array elements, from simulation callbacks (processes are suspended there). Process
variables through the kernel's `Hook::with_process` and `Process::as_any` into the
interpreter's frame (`InterpProcess::var`/`set_var`), shared variables at the slot address
elaboration computes (`Elaborated::build_full`, `yavs_interp::shared_var`); values cross as
elaborated values (`yavs_elab::ir_decode`/`ir_encode`). A deposit is written at once, so
later reads in the callback and the process when it resumes see it. Before the process first ran (#342) its declarative part runs on first access (`InterpProcess::prestart`), and every region shared variable has a slot even when no process uses it. Forces, releases, the
read-only phases and a process variable before its process first ran (before
`vhpiCbEndOfInitialization`) are `W0903`; a protected object has no value. Array bounds of
objects with a constrained subtype now come from the subtype, not the initial value.
`crates/yavs/tests/vhpi_run.rs` `vhpi_reads_and_deposits_variables` (`tests/plugin/vars.c`,
`tests/vars/expected.txt`, identical at `--threads 1` and `4`). Not yet: reading a process
variable before its process ran, a region's shared variable no process uses (#342), variables
of subprograms.

Done (#274): `vhpi_register_foreignf` and `vhpi_get_foreignf_info` (`crates/yavs-vhpi/src/foreign.rs`):
`'FOREIGN "VHPI library model"` functions and procedures (lowering: `ForeignRef::model`; interpreter:
`ForeignModel`, `Bindings::models`; CLI binds them after the plugins load) run the plugin's `execf`
with the call as `obj`; parameters through `vhpiParamDecls`, `vhpi_get_value`, and `vhpi_put_value`
on `out`/`inout` parameters and on the call for the function result; `elabf` runs once per bound
subprogram; an unregistered model is `W0108` then `E0107` at the call. Scalars and arrays of scalars
only. `crates/yavs/tests/vhpi_run.rs` `vhpi_foreign_subprograms` (functions of integer, real, time,
logic vector, boolean; procedures with `out`, `inout` and array `inout`; identical at `--threads 4`,
`tests/foreign/expected.txt`) and `vhpi_foreign_missing_model_and_result`. Missing:
`vhpi_get_data`/`vhpi_put_data`.
Done (#357): `vhpiLibF` and `vhpiAppF` models run: a library model's `elabf` when a foreign model of its
library is bound, its `execf` and an application model's `elabf` and `execf` at the start of the
simulation (`vhpi_library_and_application_models`, `tests/libapp/expected.txt`, same at `--threads 4`;
conformance case `libapp`, no reference runs them).
Done (#330, array results): a foreign function may return a one-dimensional array of scalars, sized
with `vhpiSizeConstraint` (unconstrained, indexed `1 to n`) or by its constrained result subtype;
tests in `vhpi_foreign_subprograms` (`rev`, `iota`, `word`, `nibble`) and
`vhpi_foreign_missing_model_and_result` (`nosize`); conformance plugin `tools/vhpi-conformance/plugins/fresult.c`.
Done (#330, parameters): record parameters (fields through `vhpiSelectedNames`, written for
`out`/`inout`), `in` access parameters (`vhpiPtrVal`), signal parameters (value at the call; for
`out`/`inout` a deposit and transactions on the caller's driver from `vhpiDrivers` with
`vhpi_schedule_transaction`); tests in `vhpi_foreign_subprograms` (`norm`, `flip`, `peek`,
`drive`), conformance plugin `tools/vhpi-conformance/plugins/fparam.c`.
Done (#330, foreign architectures): `"VHPI library model"` architectures elaborate as their ports
(`Elaborated::foreign_archs`); the models' `elabf` and `execf` run before the build, create
processes and drivers with `vhpi_create` (`Elaborated::ext_procs`) and drive them with
`vhpi_schedule_transaction` (kernel `Hook::assign`); an unregistered model is `E0905`. Test
`vhpi_foreign_architecture` (`tests/plugin/farch.c`, `tests/farch/expected.txt`, identical at
`--threads 4`); conformance plugin `tools/vhpi-conformance/plugins/farch.c`.

Done (#269): the design hierarchy object model (`crates/yavs-vhpi/src/design.rs`), a snapshot of
the elaborated design made before plugins load: root and component instances, blocks, `for`
iterations (`G(0)`), `if`/`case` generates, process statements, package instances (`@lib:pkg`),
generics, ports (with mode), signals, constants (generate parameters included) and variables;
record fields and array elements of any dimension and index type as `vhpiSelectedNameK` and
`vhpiIndexedNameK` objects made on demand. `vhpi_handle` (root, tool, upper and immediate region,
design and primary unit, prefix), `vhpi_handle_by_name` (absolute and relative, case-insensitive,
`.field` and `(index)` selections), `vhpi_handle_by_index`, `vhpi_iterator`/`vhpi_scan`
(`vhpiInternalRegions`, `vhpiBlockStmts`, `vhpiCompInstStmts`, `vhpiStmts` (#316), `vhpiDecls`, `vhpiSigDecls`, `vhpiPortDecls`, `vhpiGenericDecls`,
`vhpiConstDecls`, `vhpiVarDecls`, `vhpiPackInsts`, `vhpiIndexedNames`, `vhpiSelectedNames`),
`vhpiNameP`, `vhpiCaseNameP` (original spelling), full names, kind, file, line, mode,
`vhpiDefNameP`, `vhpiIsCompositeP`, `vhpiSizeP`; `vhpi_compare_handles` compares objects.
Checked by `crates/yavs/tests/vhpi_run.rs` `plugin_walks_the_design_hierarchy` (C plugin
`crates/yavs-vhpi/tests/plugin/hier.c` against `tests/hier/expected.txt`) and unit tests;
names and kinds agree with the reference simulator that has VHPI wherever it reports them.
Missing: equivalent processes of concurrent statements, aliases, files, `vhpiGenerateIndexP`;
sub-elements of variables whose bounds are only known at run time (`W0904`).

Done (#270, values): `vhpi_get_value` (`crates/yavs-vhpi/src/value.rs`) on signals, ports,
generics and constants and their record fields and array elements (any dimension), from callbacks
through the observer's `Hook` (signals) or the elaborated value (generics, constants), in
`vhpiObjTypeVal`, `vhpiBinStrVal`, `vhpiOctStrVal`, `vhpiDecStrVal`, `vhpiHexStrVal`,
`vhpiStrVal`, `vhpiCharVal`, `vhpiEnumVal` (also on integers, #318), `vhpiSmallEnumVal`, `vhpiLogicVal`, `vhpiIntVal`,
`vhpiLongIntVal`, `vhpiRealVal`, `vhpiTimeVal`, `vhpiPhysVal` and the vector forms
(`vhpiEnumVecVal`, `vhpiSmallEnumVecVal`, `vhpiLogicVecVal`, `vhpiIntVecVal`,
`vhpiLongIntVecVal`, `vhpiRealVecVal`, `vhpiTimeVecVal`, `vhpiPhysVecVal`), with the
buffer-size protocol. `crates/yavs/tests/vhpi_run.rs` `plugin_reads_values_in_every_format`
(C plugin `tests/plugin/values.c`, 27 objects by 22 formats, against `tests/values/expected.txt`)
plus unit tests; every value the reference simulator with VHPI reports correctly agrees.
Types (`crates/yavs-vhpi/src/types.rs`, a table built with the snapshot): `vhpiType` (and the
2008 `vhpiSubtype`) of declarations and sub-elements, `vhpiBaseType`, `vhpiElemType`,
`vhpiConstraints` (integer, enumeration, floating and physical ranges with `vhpiLeftBoundP`,
`vhpiRightBoundP`, `vhpiIsUpP`, `vhpiIsNullP`, the float and physical bound properties),
`vhpiEnumLiterals`, `vhpiRecordElems`, `vhpiUnitDecls`, `vhpiNumLiteralsP`,
`vhpiNumDimensionsP`, `vhpiNumFieldsP`, `vhpiIsUnconstrainedP`, `vhpiIsScalarP`,
`vhpiIsAnonymousP`, `vhpiPositionP`, `vhpiPhysPositionP`; the same plugin dumps every object's
type, and it agrees with the reference simulator on kinds, names, bases, bounds, dimensions,
literals and fields (yavs also gives enumeration and floating ranges and unit names).
Missing: variables' values, `vhpiPtrVal`, `vhpiRawDataVal`, `vhpiSmallPhys*`, records as a
whole (`W0904`); the element subtype of a resolved array subtype (`std_logic_vector`'s
elements report `STD_ULOGIC`).

Done (#273): `tools/vhpi-conformance/run.py` runs 5 C plugins (walk, props, values, cbs, put) on 2 small designs under yavs and the primary reference simulator and diffs the canonical traces, no install needed. Baseline at `ba53e23d` (reference lines matched): walk 28/172, props 158/328, values 159/230, cbs 78/114, put 108/149; put_value (#271) was still open.
Three-way run (#273, second reference simulator with VHPI, `SIM4_BIN`), on main with #269 to #272 (yavs lines matching: both references / only the first / only the second / neither): walk 9/64/19/61, props 152/24/92/60, values 102/57/56/15, cbs 64/15/5/3, put 60/41/22/26 (case-folded, LCS-aligned; many `neither` lines are yavs reporting a property both references give as -1). Real gaps where yavs differs from both: `vhpi_iterator` for block and instance statements (#316), end-of-processes callbacks (#317), `vhpiEnumVal` on integers (#318), integer put into a vector and the delta cycle after a propagating put (#319); release after deposit is #310. Reference quirks (the references differ from each other): upper-case names in one, a dummy root above the design in the other, `vhpi_put_value` on undriven signals, time resolution, string formats of enumerations. After #310, #316 to #319 (and the non-propagating put modes refused): walk 20/93/32/92 (yavs now lists block, instance and statement iterators the references answer differently, more lines), props 152/24/92/60, values 104/57/56/13, cbs 81/18/16/14 (the remaining `neither` lines are start-of-processes callbacks the second reference orders around after-delay callbacks and omits in a time step's last delta), put 66/59/9/15, nonprop (new) 56/50/6/5.

## discrepancies (simulator catalogue)

`docs/design/simulator-discrepancies.md` lists 132 cases where the reference simulators
disagree with each other, the LRM or yavs, each with a reproducer in `tests/discrepancies/` and a
recorded verdict per simulator. `python3 tools/discrepancies/run.py --sims sim1,sim2,sim3,sim4 --check`
runs them all (yavs plus the configured simulators; the label mapping is local) and fails when yavs
or a recorded verdict drifts. A fourth simulator is added with one template and `--record`. The
user page `docs/language-choices.rst` is the user-facing subset.

`docs/design/vhdl2019-support.md` is the VHDL-2019 support matrix: one row per approved LCS with yavs's status and the result of the four references on a small probe (2026-10-19: 34 supported, 4 partial, 3 unsupported, 5 not tested or not applicable; gaps #360 to #363, #29).

## test runner recipes

`docs/recipes.rst` (user page): CLI use from an external Python test runner, libraries from source, per-file `--std`, response files, exit codes, and the vendor-library pattern (#275).

## difftest (yavs candidate)

`python3 tools/difftest/difftest.py --candidate yavs` drives `yavs analyze`, `yavs elab` and
`yavs run`; `tools/difftest/report.sh` writes `tests/corpus/RESULTS.md`, which has the current pass counts per
group (249 of 249 at 1 thread, 4 threads (identical to 1) and 4 threads shuffled). `std.env.stop`/`finish` end the calling
process at once; check messages print reals as `%.15g`; runtime errors are coded diagnostics
(`E3001`-`E3010`, kernel `ErrorKind::code`) with the VHDL call stack, which difftest reads back
as `fatal` events. Normalisations for the reference simulator's free choices (default assert message without a
path, per-test `ignore=order`) are in `docs/decisions.md`.

IEEE packages from source (`tests/corpus/ieee`, #94): std_logic_1164, numeric_std, numeric_bit,
math_real, math_complex, fixed_pkg and float_pkg bodies, 2008 and 2019, equal the reference simulator at 1, 4 and 4
shuffled threads (17 of 17, with textio of std_logic_vector, fixed and float values);
`ieee_warnings` (`ref=hand`) checks the numeric_std body's own warnings, which the reference simulator omits.
