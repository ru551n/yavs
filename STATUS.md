# Status

## Milestones

| Milestone | State |
|---|---|
| M0: workspace, lexer and parser | done: `yavs parse`, corpora below |
| M1: semantic analysis | in progress: slices S1 to S3 mostly in, `docs/design/sem.md` |
| M2: IR, interpreter, elaboration, kernel | `yavs-ir` and `yavs-interp` done against hand-written IR (below); elaboration without processes done (below); lowering in progress |
| M4: external test runner end to end | first external test runner testbench runs (#110): `VUNIT_SIMULATOR=yavs python run.py` on a pass/fail `test_runner` and the runner's `examples/vhdl/user_guide` give the right pass/fail per test with the runner's `rsim-integration` branch; the runner's own suites: 10 of 11 pass completely, `check` has one failure (#281, process order in a delta), table below (#112) |
| M7: waves | `yavs run --wave` writes an FST of the whole design (#129, #116): hierarchy, records/arrays as scopes, enum tables, globs and size cap, final value per time step, same file at any `--threads`, finished on Ctrl-C; read back with wellen in `crates/yavs/tests/wave.rs` |
| M3, M5, M6, M8 to M10 | not started (crate spikes below) |

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

## CI and releases

`.github/workflows/ci.yml` runs fmt, clippy and `cargo test --workspace` (default features) on
ubuntu-24.04, ubuntu-24.04-arm and windows-latest (MSVC) for every push to main and every pull
request (#134); a `corpus` job (Linux x86_64) fetches the pinned external corpora and fails on any difference from `tools/corpus/*.expected` (#63); not yet in CI: JIT tier. `.github/workflows/release.yml`
builds and packages `yavs` with `lib/` for the three targets on a `v*` tag and publishes a
pre-release with SHA256SUMS (#165); `workflow_dispatch` with `dry_run` only builds the archives. The Linux archives are built with cargo-zigbuild against glibc 2.17 and smoke-tested in centos:7.
An installed binary finds `lib/` next to itself (#43).

## sem (`crates/yavs-sem`, M1)

- A negated abstract literal at the i64 minimum (`-9223372036854775808`, `-9223372036854775808 fs`) folds to `i64::MIN` (L58, #214); `std.standard` writes TIME's lower bound as that literal.
- Instances of one generic subprogram declared under one name (`function f is new f generic map (...)`)
  overload by their instantiated profiles, next to ordinary overloads and across use clauses (#28):
  `tests/diag/sem/gen/subprogram_instances_*`, difftest `generics/subprog_instance_overload` (2008, 2019).
- A subprogram header with a generic list and a generic map (`function f generic (type t) generic map
  (t => integer) parameter (v : t) return t`) declares an instance through the `is new` path, also
  with a separate body that repeats the header (#227, L61): `tests/diag/sem/gen/subprogram_header_map_*`,
  difftest `generics/subprog_header_map_2019`. A type, subprogram or package interface in a
  parameter list is `E1232` (4.2.2.1), in a port clause `E1234` (6.5.6.3); a generic list or map on a formal generic
  subprogram is `E1235` (6.5.4, #236).
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
  or file type (or containing one) are `E1223` (#198; constant parameters are legal in 2019; 2008 protected method profiles too, #212); protected ones `E1905`.
- Protected types (#52): signals and constants of a protected type, assignments to a protected
  variable and operators on protected operands are `E1905`; a shared variable of another type is
  `E1904` (`W1904` with `--relaxed`); a method without a body is `E1230` at its name. A shared variable of a protected type declared before its body in the same region (a package declaration) is `E1230` at the variable (#258).
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
  package or a package instance (#34; groups, group templates and disconnection specifications
  are still E9999, #260).
- Types (S3): scalars, arrays, records, element/record/open constraints merged once, resolution
  indications, file element checks; the rule 1 representation is pinned on numeric_std
  (`numeric_std_subtypes_rule_1`, `numeric_std_body_rule_1`) and reviewed for the composite cases
  in `docs/design/rule1-review.md` (`tests/rule1.rs`; recommendation: approve with fixes, findings
  F1, F2, F4, F5, F7, F8 open as #98 to #111, F3 and F6 fixed); the user's approval is pending.
- Generics (S4, `generic.rs`): generic packages, subprograms and (2019) protected types; 2008
  `type T` and the 2019 classes; formal subprograms (`is <>`, `is name`) and formal packages
  (`(<>)`, `(default)`, generic map, checked against the actual instance; a parameter type through a
  formal package of a formal package, `s.r.arr`, matches no actual outside the generic, `E1300`, L76); package and subprogram
  instances, also inside generic units, by HIR cloning (visible part eager, bodies lazy via
  `Db::instance_body`); `W1501`, `W1506`; design entities with generic types, subprograms or
  packages cloned with their architecture per distinct map for elab (`generic/entity.rs`,
  `Db::instantiate_entity`, concurrent statements cloned too); components (`Db::instantiate_component`,
  per map, for elab) and blocks (in place, `Db::instantiate_blocks`) with them too, configuration
  specifications cloned with their region, default binding by name of generic types, subprograms
  and packages (#222). The fixed/float packages' declarations, generic bodies
  and instance bodies analyze clean in 2008 and 2019 (#37); mode views of generic packages and subprograms are
  cloned with them (#32). Open: anonymous generic types, instantiating a generic protected type
  (parser request).
- 2019 mode views and private variables (S7, #32): view declarations (`E1901`, `E1902`),
  `'converse` (`E1621` on a non-view), view ports and signal parameters (`E1903`), views in
  generic units; private variables visible only in the protected body (`E1105`), method aliases
  that forward to a private variable (`E1906` for an alias of anything else), and aliases of
  methods of shared variables and variables (#237; prefixes other than an object name are
  `E9999`; a call of such a failed alias reports nothing more, #245). View ports, nested and array views, and calls through method aliases simulate
  (`hierarchy/mode_view_*`, `protected/private_variables`, `protected/method_alias_objects`), and so do signal
  parameters with a view (record, nested, `'converse` and array views, slices, several calling processes; #254), also associated element by element (#265).
  `buffer` and `linkage` subprogram parameters are `E1236`.
- 2019 front end (corpus fixes #173 to #180): `A'INDEX(n)` and `R'RECORD` type marks, signals
  and variables with bounds from the initial value, variable ports of protected types, `open`
  sub-element actuals, formal packages of design entities; entity subprograms with bodies in an
  architecture. Initializer-bound signals elaborate and run (#187; records and arrays of records with such fields, #215);
  variable ports elaborate and run (#188): the actual is a shared variable or a variable port
  of the same type, never `open` or unassociated (`E1311`).
- Differential corpus (#163, #181): `implicit/` covers the implicit operations end to end (radix strings, arithmetic and logic, access and file operations, edges, record equality, `hand_edges` for the LRM cases where the reference simulator deviates); `hierarchy/external_*` covers external names as port actuals (absolute, relative, generate-indexed, package). Implicit native symbols separate the operand-shape form with `#`.
- 2019 conditional expressions (`a when c else b` in initial values, assignments, actuals,
  `return` and parentheses), the conditional `return` and `unaffected` in variable and force
  assignments analyze, lower and run (#33; difftest `process/conditional_expr_2019`,
  `process/unaffected_2019`). Named return values (`return r of t`) of array type take the
  result subtype from the call's context and run (`process/named_return_2019`), so do record
  types and named return values in generic subprograms, generic packages and protected
  methods (#239, `process/named_return_record_2019`, `process/named_return_generic_2019`);
  scalar types are `E1233`. `return when c;` in procedures runs (#239,
  `process/return_when_2019`).
- Static folding: checked i64, values checked against the target subtype (`E1403`) and the base
  range, division/mod by zero and negative exponents, static length mismatch (`E1405`).

## syntax (`crates/yavs-syntax`, M0)

Hand-written lexer, conditional analysis on the token stream and recursive-descent parser for
VHDL-2008 and VHDL-2019 into an owned AST (`yavs parse [--std 2008|2019] files...`). Diagnostics
go through `yavs-diag` (human, short and JSON formats; snapshots in `tests/diag/`). PSL is skipped
with one `W0004` warning per design unit (#138); the PSL does not run.

Parse corpus: `tools/fetch-corpus.sh` fetches the corpora (default `~/.cache/rsim-corpus`), then
`YAVS_CORPUS=~/.cache/rsim-corpus cargo test -p yavs --features corpus` compares them with the
expected lists in `tests/parse-corpus/` and fails if `YAVS_CORPUS` is unset (`YAVS_BLESS=1`
updates the lists). Plain `cargo test --workspace` still parses `lib/` and runs the snapshots.

| Corpus | Files | Pass | Expected failures |
|---|---:|---:|---|
| `lib/` std and ieee, 2008 and 2019 (in repo) | 55 | 55 | none |
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
positional aggregates as values and targets (range choices in targets are `E1333`, #244), and of named aggregates and arrays with open
element subtypes as values (#89, #230; without a constrained context they take the direction of their range choices, a mix is `E1334`; an array value under `others` is `E1323`, #242); `&` on arrays of arrays and of records (#234); package
objects at run time (`Lowerer::package`: constants evaluated once, composites with run-time
bounds too, shared protected variables, package signals and files); composite architecture
constants evaluated once per region (#73). `tests/corpus.rs` runs every corpus group plus
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
`force`/`release` of scalar signals, signals of arrays of records and of records with
array-of-record fields (struct of arrays; whole, element, field, bit and slice targets,
element sensitivity, resolution as a whole, #200; field and element bounds from the initial
value, a partially constrained subtype or a port's actual, signal parameters of such types,
package signals of record types, attributes and implicit signals of parts over several
fields, #215), shared protected variables (created through `call_shared`),
user resolution functions (yavs-elab resolver over `yavs_interp::resolve`). Difftest: access 3/3,
composite 6/6, force 3/3, protected 6/6, resolution 4/5, textio 2/3 at 1 and 4 threads.

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

| Run | Fuzz time | Executions | Crashes found | Fixed | Open |
|---|---:|---:|---:|---:|---|
| 4 workers, ~1.1 h CPU in total | ~27 min wall | ~280,000 | 3 distinct ICEs (procedure call left `ExprKind::Error`; physical-literal unit undeclared; alias of a selected object name) | 3 | none |

The procedure-call ICE was fixed on main in parallel; the other two are fixed here, all with
regression tests in `tests/diag/sem/fuzz/`. The last 3 chunks (~190,000 executions) found nothing.

## logic (`crates/yavs-logic`)

Done ahead of M3: `StdULogic` with the IEEE 1164 tables (checked against the vendored package
body), `Logic<R>`/`LogicVec` packed vectors in two layouts (nibbles, bit-planes), `BitVec` fast
path, numeric_std add/sub/compare/shift/rotate/integer conversion with X propagation, exhaustive
and property tests, criterion benchmarks. Default layout is bit-planes; see
`crates/yavs-logic/LAYOUT.md`; the user confirmed bit-planes everywhere, signals included (docs/plan.md, 2026-10-18).

## llvm (`crates/yavs-llvm`)

JIT risk spike ahead of M6, behind the `llvm` feature (`cargo test -p yavs-llvm --features llvm`).
Done: llvm-sys 211 bindings (docs/decisions.md), ORC LLJIT with a portable CPU, O2 through
`LLVMRunPasses`, Rust runtime callbacks as absolute symbols, a checked i64 sum that traps into
Rust on overflow, a process body reading and writing a `#[repr(C)]` struct, a 10k-instruction
function checked against its Rust reference semantics, an unresolved runtime symbol reported by
name, and background compilation on one thread with an atomic function-pointer swap while the
calling thread keeps running in the "interpreter". Windows: `crates/yavs-llvm/WINDOWS.md`
(researched, not yet run on Windows).

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
(`Process::several_waits`) in reverse, signal-resumed before timeout-resumed.

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
`to_string`/`'image`), VHPIDIRECT calls of scalars, generational heaps with precise-root GC (descriptor-aware, deallocated slots quarantined),
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
(`'PATH_NAME`) runs at simulation start with W2118 (#220). A process's impure constants run once, when the process starts (#251; difftest `protected/impure_const_once`). Also done: package bodies and generic instance bodies
loaded before elaboration, a package needing a body without one (E2116), conversion functions
and type conversions in port maps (kernel converters; conversion functions of record ports and
arrays of records convert all leaves in one call in the same delta, #74; both conversions of
`inout` ports, #238; a formal-side-only conversion of an `inout` port, scalar and composite, #74), individual
association of unconstrained array ports, implicit signals (`'stable`,
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
resolved as a whole (every driving process drives every leaf, #193; nested in another
composite: E9999), an external name as an `in` port actual as a signal name actual, same
delta (#186), linkage ports and mode-view ports, entities with generic types,
subprograms and packages (yavs-sem `Db::instantiate_entity` clones the entity and architecture
once per distinct map of those actuals, with its own mangled identity; generic constants stay
instance data; #75; components and blocks with them, configuration specifications in such an
architecture, #222), `others` in a configuration after component configurations naming
specification-bound instances, 2019 variable ports denoting the shared variable at the end of
their chain of actuals (architecture, block, generate or package shared variables, through
entities, components and blocks; a top-level one is E2119; #188), non-static expression actuals of `in` ports and of parts of
them driven by their equivalent IR process one delta later (records leaf by leaf, ranges from
the instance's generics checked, expression actuals in a binding indication's port map),
static expression parts in the formal's initial value, unconstrained formals bounded by a
static value or a conversion function's result subtype, type conversions of record ports and
arrays of composites (#74), `inertial` signal-name actuals as expression actuals, whole
formals (#233) and parts of an individual association (#261).

Processes: `Elaborated::lower` lowers every architecture owning a process (yavs-lower) and
`Elaborated::build` creates one yavs-interp process per process instance with per-instance data
(restart test with the interpreter).

CLI: `yavs elab` and `yavs run` per docs/cli-contract.md (closure from the manifests, design
revision = newest standard in the closure, `-g` after TOP, `@file`, exit codes, reports in the reference simulator's
format on stdout). Difftest: see `tests/corpus/RESULTS.md` (122 of 123 at 1, 4 and 4 shuffled
threads on 2026-10-18; `generics/generic_types_2019` and the other generic-type cases pass since #75).

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
Windows `.exe` export table (MinGW checked; MSVC `/DEF:` still never run, #124). No `yavs.lib`.
`crates/yavs/tests/vhpi_exports.rs` checks the ELF `.dynsym` against the header (Linux only).

## discrepancies (simulator catalogue)

`docs/design/simulator-discrepancies.md` lists 80 constructs where the reference simulators
disagree with each other, the LRM or yavs, each with a reproducer in `tests/discrepancies/` and a
recorded verdict per simulator. `python3 tools/discrepancies/run.py --sims sim1,sim2,sim3 --check`
runs them all (yavs plus the configured simulators; the label mapping is local) and fails when yavs
or a recorded verdict drifts. A fourth simulator is added with one template and `--record`. The
user page `docs/language-choices.rst` is the user-facing subset.

## difftest (yavs candidate)

`python3 tools/difftest/difftest.py --candidate yavs` drives `yavs analyze`, `yavs elab` and
`yavs run`; `tools/difftest/report.sh` writes `tests/corpus/RESULTS.md`: 69 of 78 at 1 thread,
4 threads (identical to 1) and 4 threads shuffled. `std.env.stop`/`finish` end the calling
process at once; check messages print reals as `%.15g`; runtime errors are coded diagnostics
(`E3001`-`E3010`, kernel `ErrorKind::code`) with the VHDL call stack, which difftest reads back
as `fatal` events. Normalisations for the reference simulator's free choices (default assert message without a
path, per-test `ignore=order`) are in `docs/decisions.md`. The 9 failures wait for sem (#1, #3)
and lowering (out signal parameters, `'stable`, `'path_name`, guarded blocks).

IEEE packages from source (`tests/corpus/ieee`, #94): std_logic_1164, numeric_std, numeric_bit,
math_real, math_complex, fixed_pkg and float_pkg bodies, 2008 and 2019, equal the reference simulator at 1, 4 and 4
shuffled threads (17 of 17, with textio of std_logic_vector, fixed and float values);
`ieee_warnings` (`ref=hand`) checks the numeric_std body's own warnings, which the reference simulator omits.
