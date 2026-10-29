# yavs conformance report

Generated: yavs 0.9.0 at commit `a89e38656d` (2026-10-28) by `tools/conformance/run.py`; do not edit by hand.

yavs is checked by running the test suites of VHDL libraries, a VHDL language test suite and real designs on it, and by comparing its output
with two established open-source VHDL simulators, called sim1 and sim2 here. Each suite judges its own tests. Over the suites below, 14894 of 15021
tests pass; every failure is tracked as an issue. This is evidence, not a proof of correctness.

| Project or suite | What it is | Passed / total |
| --- | --- | ---: |
| Differential testbenches | small testbenches of this repository; output must equal sim1's, at 1 and 4 threads | 550/550 |
| Known simulator differences | one reproducer per case where simulators disagree | 214/214 |
| sim1's regression tests | a reference simulator's own self-checking tests (runnable ones) | 1002/1070 |
| sim2's regression tests | a reference simulator's own self-checking tests (runnable ones) | 458/504 |
| VESTs | VHDL-1993 language test suite | 3687/3694 |
| UVVM | verification library: its own testbenches | 131/137 |
| OSVVM, VHDL-2008 | verification library: its own regression | 314/314 |
| OSVVM, VHDL-2019 | verification library: its own regression | 314/314 |
| VUnit | test framework: its own VHDL test suites | 1357/1357 |
| Open Logic | FPGA standard library: its own testbenches run with yavs | 6867/6867 |
| Real designs | neorv32 (three configurations) and GRLIB LEON3 run to completion | 4/4 designs |
| Waveforms of real designs | signals of 17 designs equal to sim1's at every time step | 44824/44889 signals |
| Third-party VHDL code | files of 33 libraries and designs that analyse | 6675/6682 files |
| VHPI | plugin call traces identical to sim1's | 2/23 traces |

Terms used here: *passed / total* counts the tests a suite judges itself, out of the tests that can be judged (a test whose own
expectation is wrong or depends on the implementation is not counted, with its reason). To *analyse* a file is to
check it and store it in a library; to *simulate* is to elaborate the design and run it. A *tier* is the interpreter or one of the
compilers (LLVM, Cranelift) that run a design; every tier gives the same output. The *language reference* is the IEEE 1076 standard.

## Details

The sections below give each result in full. The last table says where and when each was produced: on CI for every push to `main` and every release tag, or by a local run with installed reference simulators, with its date and commit.

### Differential testbenches

Every testbench of `tests/corpus` runs on yavs and its normalised output is compared with the recorded output of sim1 (`tools/difftest/README.md`). Strict mode: no ignored fields unless the test says so with a comment. The 4-thread columns also require output byte-identical to the 1-thread run.

| group | 1 thread | 4 threads | 4 threads shuffled |
| --- | --- | --- | --- |
| access | 8/8 | 8/8 | 8/8 |
| arith | 8/8 | 8/8 | 8/8 |
| composite | 118/118 | 118/118 | 118/118 |
| delta | 13/13 | 13/13 | 13/13 |
| env | 14/14 | 14/14 | 14/14 |
| force | 12/12 | 12/12 | 12/12 |
| generics | 52/52 | 52/52 | 52/52 |
| hierarchy | 84/84 | 84/84 | 84/84 |
| ieee | 21/21 | 21/21 | 21/21 |
| implicit | 6/6 | 6/6 | 6/6 |
| postponed | 2/2 | 2/2 | 2/2 |
| process | 39/39 | 39/39 | 39/39 |
| protected | 22/22 | 22/22 | 22/22 |
| psl | 13/13 | 13/13 | 13/13 |
| reflect | 14/14 | 14/14 | 14/14 |
| report | 10/10 | 10/10 | 10/10 |
| resolution | 22/22 | 22/22 | 22/22 |
| rtl | 4/4 | 4/4 | 4/4 |
| scalar | 17/17 | 17/17 | 17/17 |
| shared | 4/4 | 4/4 | 4/4 |
| sigassign | 27/27 | 27/27 | 27/27 |
| std93 | 6/6 | 6/6 | 6/6 |
| synopsys | 20/20 | 20/20 | 20/20 |
| textio | 14/14 | 14/14 | 14/14 |
| total | 550/550 | 550/550 | 550/550 |

Failing: none.

The references against the recorded sim1 outputs (local, 2026-10-26 at `cd5268f8b2`): sim1 533/535, sim2 314/535. This shows how far the oracle is itself stable across simulators.

The compiled tiers must give the same output as the interpreter (local, 2026-10-26 at `cd5268f8b2`, a build with the `llvm` feature): tier `auto`: 1 thread 535/535, 4 threads 535/535, 4 threads shuffled 535/535; tier `llvm`: 1 thread 535/535, 4 threads 535/535, 4 threads shuffled 535/535.

### The reference simulators' own regression tests

Each reference simulator ships a suite of self-checking tests. They run in place from the fetched checkout on yavs and are judged by the suite's own verdict. The pass lists (`tools/difftest/external-*.pass`) are gated: a listed test that stops passing fails CI. The others fail the suite's own verdict, fail to analyse or elaborate, or are skipped (other languages, VHPI, scripts); the suites contain tests of features yavs does not implement and tests of reference-specific behaviour.

| suite | tests | pass | gated pass list | regressions |
| --- | ---: | ---: | ---: | ---: |
| sim1 | 1473 | 1002 | 990 | 0 |
| sim2 | 1151 | 458 | 455 | 0 |

Not passing: sim1: 68; sim2: 46 (the classes of each are in `docs/conformance.json`; a timeout under load moves a test between them, so the report does not).

### Known simulator differences

`docs/design/simulator-discrepancies.md` lists every case where the references disagree with each other, with the language reference or with yavs:
214 catalogued behaviours, each with a reproducer under `tests/discrepancies/<case>/`. CI runs every reproducer on yavs and fails when yavs's output
differs from the recorded one (matches). Kinds: LRM clear 131, LRM ambiguity 77, reference bug 5, library availability 1.

Policy (`docs/plan.md`): where the language reference is clear, yavs follows it even against all references; where it is unclear yavs is permissive
and follows the majority of the references that run the construct. The verdicts of each reference are in the catalogue; the table gives the clause, the kind,
and yavs's verdict.

<details><summary>All catalogued cases</summary>

| case | LRM | kind | yavs |
| --- | --- | --- | --- |
| [`anonymous-generic-constant`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/anonymous-generic-constant) | 6.5.2, 6.5.3 (2019) | LRM clear | OK |
| [`anonymous-subprogram-parameter`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/anonymous-subprogram-parameter) | 6.5.2, 6.5.3 (2019) | LRM clear | OK |
| [`composite-image-quoting`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/composite-image-quoting) | 16.2.2, 5.7 (2019) | LRM ambiguity | as sim4 |
| [`anonymous-formal-subprogram`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/anonymous-formal-subprogram) | 6.5.2, 6.5.4 (2019) | LRM clear | OK |
| [`anonymous-generic-subprogram`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/anonymous-generic-subprogram) | 4.2.1, 6.5.2 (2019) | LRM clear | OK |
| [`anonymous-operator`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/anonymous-operator) | 6.5.2, 9.2 (2019) | LRM ambiguity | OK |
| [`psl-sere-alternative`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/psl-sere-alternative) | PSL (IEEE 1850) 6.1.1.2.1 | LRM clear | OK |
| [`psl-sere-equal-repeat`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/psl-sere-equal-repeat) | PSL (IEEE 1850) 6.1.2.2 | LRM clear | OK |
| [`psl-sere-within-and`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/psl-sere-within-and) | PSL (IEEE 1850) 6.1.1.2.2, 6.1.1.2.4 | LRM clear | OK |
| [`psl-named-sequence`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/psl-named-sequence) | PSL (IEEE 1850) 6.5 | LRM clear | OK |
| [`psl-temporal-operators`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/psl-temporal-operators) | PSL (IEEE 1850) 6.2.1.3 to 6.2.1.7 | LRM clear | OK |
| [`psl-abort`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/psl-abort) | PSL (IEEE 1850) 6.2.1.3 to 6.2.1.7 | LRM clear | OK |
| [`formal-package-per-element-box`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/formal-package-per-element-box) | 2019 6.5.7.2 (text not at hand) | LRM ambiguity | REJECT |
| [`psl-assert-always`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/psl-assert-always) | 2008 clause 17 (PSL by reference) | LRM ambiguity | OK |
| [`psl-assert-severity`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/psl-assert-severity) | 2008 clause 17 (PSL by reference) | LRM ambiguity | OK |
| [`psl-assume`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/psl-assume) | 2008 clause 17 (PSL by reference) | LRM ambiguity | OK |
| [`psl-explicit-clock`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/psl-explicit-clock) | 2008 clause 17 (PSL by reference) | LRM ambiguity | OK |
| [`synopsys-arith-package`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/synopsys-arith-package) | none (not IEEE packages) | library availability | OK |
| [`v93-delimited-comment`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/v93-delimited-comment) | 1993 13.8 | LRM clear | REJECT |
| [`v93-elsif-generate`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/v93-elsif-generate) | 1993 9.7 | LRM clear | REJECT (`E0011`) |
| [`v93-reserved-2008-word`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/v93-reserved-2008-word) | 1993 13.9 | LRM clear | OK |
| [`v93-explicit-implicit-homograph`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/v93-explicit-implicit-homograph) | 1993 10.4 | LRM ambiguity | OK |
| [`v93-out-port-read`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/v93-out-port-read) | 1993 4.3.2 | LRM clear | REJECT (`E1360`) |
| [`v93-out-port-read-contexts`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/v93-out-port-read-contexts) | 1993 4.3.2 | LRM clear | REJECT (`E1360`) |
| [`v93-out-port-read-alias`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/v93-out-port-read-alias) | 1993 4.3.2, 6.6 | LRM ambiguity | OK |
| [`access-constant-parameter`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/access-constant-parameter) | 6.4.2.2, 6.5.2 | LRM clear | REJECT |
| [`access-to-protected`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/access-to-protected) | 5.4.1 | LRM ambiguity | 08: REJECT / 19: OK |
| [`actual-conversion-in-variable-parameter`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/actual-conversion-in-variable-parameter) | 6.5.7.1, 4.2.2.2 | LRM clear | OK |
| [`alias-of-external-method`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/alias-of-external-method) | 6.6.2, 8.7 | LRM clear | OK |
| [`block-config-default-bound`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/block-config-default-bound) | 3.4.3 | LRM ambiguity | OK |
| [`call-conversions`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/call-conversions) | 6.5.7.1 | LRM clear | OK |
| [`config-label-wrong-component`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/config-label-wrong-component) | 3.4.2, 7.3 | LRM clear | REJECT |
| [`config-rebinding`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/config-rebinding) | 7.3.2 | LRM clear | OK |
| [`disconnect-others`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/disconnect-others) | 7.4 | LRM clear | OK |
| [`disconnect-twice`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/disconnect-twice) | 7.4 | LRM clear | REJECT |
| [`div-by-zero-in-range`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/div-by-zero-in-range) | 9.4.2 | LRM ambiguity | REJECT |
| [`duplicate-interface-param`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/duplicate-interface-param) | 12.3 (homographs) | LRM clear | REJECT |
| [`formal-conversion-unconstrained-port`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/formal-conversion-unconstrained-port) | 6.5.7.1 | LRM ambiguity | REJECT |
| [`formal-package-map-mismatch`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/formal-package-map-mismatch) | 6.5.7.2 | LRM clear | REJECT |
| [`formal-package-subprograms`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/formal-package-subprograms) | 6.5.7.2 | LRM clear | OK |
| [`generic-array-anonymous-types`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/generic-array-anonymous-types) | 6.5.3 (2019) | LRM clear | OK |
| [`generic-array-index-constraint`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/generic-array-index-constraint) | 6.5.3 (2019) | LRM clear | OK |
| [`generic-class-array-elem-mismatch`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/generic-class-array-elem-mismatch) | 6.5.3 (2019) | LRM clear | REJECT |
| [`generic-map-in-call`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/generic-map-in-call) | 4.4, 9.3.4, LCS-2016-049 (2019) | LRM clear | OK |
| [`generic-map-formal-signature`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/generic-map-formal-signature) | 6.5.7.2, LCS-2016-I03 (2019) | LRM clear | OK |
| [`generic-subprogram-of-block`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/generic-subprogram-of-block) | 6.5.7.2, 7.3.3, 11.2 | LRM ambiguity | REJECT |
| [`generic-subprogram-of-enclosing-entity`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/generic-subprogram-of-enclosing-entity) | 6.5.7.2 | LRM ambiguity | OK |
| [`group-attribute`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/group-attribute) | 6.10, 7.2 | LRM clear | OK |
| [`group-constituent-count`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/group-constituent-count) | 6.10 | LRM clear | REJECT |
| [`group-mixed-class-template`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/group-mixed-class-template) | 6.9, 6.10 | LRM clear | OK |
| [`group-template-box-empty`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/group-template-box-empty) | 6.10 | LRM ambiguity | OK |
| [`homograph-subprogram-instances`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/homograph-subprogram-instances) | 12.3, 4.10 | LRM clear | REJECT |
| [`i64-min-literal`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/i64-min-literal) | 9.3, 5.2.3 (range of universal_integer implementation-define | LRM ambiguity | REJECT |
| [`implicit-condition-operator`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/implicit-condition-operator) | 9.2.9 | LRM ambiguity | OK |
| [`implicit-signal-in-subprogram`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/implicit-signal-in-subprogram) | 4.2.2.2, 14.7.5 | LRM ambiguity | OK |
| [`incomplete-type-never-completed`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/incomplete-type-never-completed) | 5.4.2 | LRM clear | REJECT |
| [`incremental-binding`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/incremental-binding) | 7.3.2.3 | LRM clear | OK |
| [`individual-assoc-parts`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/individual-assoc-parts) | 6.5.7.1 | LRM clear | OK |
| [`local-generic-package-same-unit`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/local-generic-package-same-unit) | 4.9, 4.7 (2008) | LRM ambiguity | OK |
| [`method-alias-objects-full`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/method-alias-objects-full) | 5.6.2, 6.6.3 | LRM ambiguity | OK |
| [`method-alias-of-object`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/method-alias-of-object) | 5.6.2, 6.6.3 | LRM ambiguity | OK |
| [`method-alias-of-process-variable`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/method-alias-of-process-variable) | 5.6.2, 6.6.3 | LRM ambiguity | OK |
| [`method-alias-private-variable`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/method-alias-private-variable) | 5.6.2 (2019) | LRM clear | OK |
| [`multiple-sources-two-processes`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/multiple-sources-two-processes) | 6.4.2.3, 14.7.3.1 | LRM clear | REJECT |
| [`named-return-in-return-and-formal`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/named-return-in-return-and-formal) | 4.2.1 (2019) | LRM ambiguity | OK |
| [`named-return-record-generic-method`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/named-return-record-generic-method) | 4.2.1 (2019) | LRM ambiguity | OK |
| [`named-return-scalar`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/named-return-scalar) | 4.2.1 (2019) | LRM ambiguity | OK |
| [`nested-generic-packages`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/nested-generic-packages) | 4.7, 6.5.7.2 | LRM clear | OK |
| [`nested-package-runtime`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/nested-package-runtime) | 4.7, 4.8, 3.3.2 (2008) | LRM clear | OK |
| [`nested-package-second-body`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/nested-package-second-body) | 4.8, 12.1 | LRM clear | REJECT |
| [`others-with-other-choice`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/others-with-other-choice) | 9.3.3.3 | LRM clear | REJECT |
| [`overflow-in-addition`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/overflow-in-addition) | 9.2.2 | LRM ambiguity | OK |
| [`overload-by-conversion-type`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/overload-by-conversion-type) | 4.5.3, 6.5.7.1 | LRM ambiguity | REJECT |
| [`overloaded-attribute-spec`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/overloaded-attribute-spec) | 7.2 | LRM clear | OK |
| [`package-generic-local-state`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/package-generic-local-state) | 4.8, 14.5 | LRM clear | OK |
| [`package-in-generate`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/package-in-generate) | 11.8, 3.3.2 (2008) | LRM clear | OK |
| [`package-generic-map`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/package-generic-map) | 4.7, 4.9 | LRM clear | OK |
| [`package-in-process`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/package-in-process) | 11.3 (2008) | LRM clear | OK |
| [`package-instance-local`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/package-instance-local) | 4.3, 11.3, 11.8 | LRM clear | OK |
| [`package-instance-signal-in-process`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/package-instance-signal-in-process) | 4.9, 4.7, 11.3 | LRM clear | REJECT |
| [`port-conv-formal-type-mark`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/port-conv-formal-type-mark) | 6.5.7.1, 9.3.6 | LRM clear | REJECT |
| [`port-conv-inout`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/port-conv-inout) | 6.5.7.1, 14.7.7.3 | LRM clear | OK |
| [`port-conv-inout-one-way-in`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/port-conv-inout-one-way-in) | 6.5.7.1 | LRM ambiguity | REJECT |
| [`port-conv-inout-one-way-out`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/port-conv-inout-one-way-out) | 6.5.7.1 | LRM ambiguity | OK |
| [`port-map-gap-individual`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/port-map-gap-individual) | 6.5.7.1 | LRM clear | REJECT |
| [`positional-generic-after-type`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/positional-generic-after-type) | 6.5.3, 6.5.7.2 | LRM ambiguity | OK |
| [`protected-composite-shared`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/protected-composite-shared) | 6.4.2.4, LCS-2016-014 (2019) | LRM ambiguity | OK |
| [`protected-composites`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/protected-composites) | 5.3, 5.4, LCS-2016-014, 014a (2019) | LRM clear | OK |
| [`protected-record-mixed`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/protected-record-mixed) | 5.3.1 (2019) | LRM clear | REJECT |
| [`protected-method-access-parameter`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/protected-method-access-parameter) | 5.6.2 | LRM ambiguity | 08: REJECT / 19: OK |
| [`protected-method-access-result`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/protected-method-access-result) | 5.6.2 | LRM ambiguity | 08: REJECT / 19: OK |
| [`protected-type-expanded-name-method`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/protected-type-expanded-name-method) | 8.3, 12.1 | LRM clear | OK |
| [`protected-type-expanded-name-mixed`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/protected-type-expanded-name-mixed) | 8.3, 12.1 | LRM clear | OK |
| [`protected-type-expanded-name-state`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/protected-type-expanded-name-state) | 8.3, 12.1 | LRM clear | OK |
| [`shared-variable-before-body`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/shared-variable-before-body) | 14.4.2 | LRM clear | REJECT |
| [`signal-param-parts`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/signal-param-parts) | 6.5.7.1 | LRM clear | OK |
| [`signal-param-records`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/signal-param-records) | 6.5.7.1 | LRM clear | OK |
| [`signal-param-slices`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/signal-param-slices) | 6.5.7.1 | LRM clear | OK |
| [`stable-attr-in-subprogram`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/stable-attr-in-subprogram) | 4.2.2.2 | LRM ambiguity | OK |
| [`static-index-out-of-range`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/static-index-out-of-range) | 8.4 | LRM ambiguity | OK |
| [`static-slice-out-of-range`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/static-slice-out-of-range) | 8.5 | LRM ambiguity | OK |
| [`subprogram-header-generic-map`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/subprogram-header-generic-map) | 4.2.1, 4.4 | LRM clear | OK |
| [`subprogram-param-package-interface`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/subprogram-param-package-interface) | 4.2.2.1 | LRM clear | REJECT |
| [`type-through-formal-package-of-formal-package`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/type-through-formal-package-of-formal-package) | 6.5.7.2, 4.8 | LRM ambiguity | OK |
| [`unaffected-force`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/unaffected-force) | 10.5.2.1 (2019) | LRM clear | 08: REJECT / 19: OK |
| [`unaffected-variable`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/unaffected-variable) | 10.5.2.1 (2019) | LRM clear | 08: REJECT / 19: OK |
| [`use-clause-constant-vs-literal`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/use-clause-constant-vs-literal) | 12.4 | LRM clear | REJECT |
| [`variable-port`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/variable-port) | 6.5.2, 6.5.7.1 (2019), LCS-2016-047 | LRM clear | OK |
| [`view-individual-driven-elements`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/view-individual-driven-elements) | 6.5.2, 14.7.2 | LRM clear | OK |
| [`force-resolved-composite-member`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/force-resolved-composite-member) | 10.5.2.1 | LRM clear | REJECT |
| [`force-twice`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/force-twice) | 10.5.2.1, 14.7.3 (when a force is an event is not stated) | LRM ambiguity | as sim1 |
| [`allocator-overload`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/allocator-overload) | 9.3.7, 12.5 | LRM clear | OK |
| [`array-nested-resolved-whole`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/array-nested-resolved-whole) | 4.6, 14.7.2, 14.7.3.1 | LRM clear | OK |
| [`array-record-resolved-whole`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/array-record-resolved-whole) | 4.6, 14.7.2 | LRM clear | OK |
| [`array-resolved-whole`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/array-resolved-whole) | 4.6, 14.7.2 | LRM clear | OK |
| [`null-waveform-in-waveform`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/null-waveform-in-waveform) | 10.5.2.2, 14.7.3.1 | LRM clear | OK |
| [`bus-guarded`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/bus-guarded) | 14.7.3 | LRM clear | OK |
| [`conv-actual-unconstrained-result`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/conv-actual-unconstrained-result) | 5.3.2.2 | LRM clear | REJECT |
| [`conv-fn-param-unconstrained-out-port`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/conv-fn-param-unconstrained-out-port) | 5.3.2.2 | LRM clear | OK |
| [`config-generate-subtype-spec`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/config-generate-subtype-spec) | 3.4.2 | LRM clear | OK |
| [`config-two-generate-ranges`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/config-two-generate-ranges) | 1.3.1, 3.4.2 | LRM clear | OK |
| [`external-name-in-port-delta`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/external-name-in-port-delta) | 8.7, 6.5.6.3, 14.7 | LRM ambiguity | OK |
| [`external-name-in-port-init`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/external-name-in-port-init) | 8.7, 6.5.6.3 | LRM ambiguity | OK |
| [`inertial-part-actual`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/inertial-part-actual) | 6.5.6.3, 6.5.7.1 | LRM clear | OK |
| [`open-port-default`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/open-port-default) | 6.5.6.3, 14.3.5 | LRM ambiguity | OK |
| [`part-range-check-record-field`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/part-range-check-record-field) | 6.5.6.3, 14.7.3.1 | LRM clear | FATAL |
| [`partial-driver-resolved-composite`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/partial-driver-resolved-composite) | 4.6, 14.7.2 | LRM ambiguity | OK |
| [`record-nested-resolved-whole`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/record-nested-resolved-whole) | 4.6, 14.7.2, 14.7.3.1 | LRM clear | OK |
| [`relative-external-name-port-actual`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/relative-external-name-port-actual) | 8.7 | LRM clear | OK |
| [`time-below-resolution`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/time-below-resolution) | 5.2.4.2 | LRM clear | REJECT |
| [`type-conv-unconstrained-composite`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/type-conv-unconstrained-composite) | 5.3.2.2 | LRM clear | OK |
| [`type-conv-unconstrained-in-port`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/type-conv-unconstrained-in-port) | 5.3.2.2 | LRM clear | OK |
| [`type-conv-unconstrained-out-port`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/type-conv-unconstrained-out-port) | 5.3.2.2 | LRM clear | OK |
| [`unconstrained-records`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/unconstrained-records) | 5.3.3, 6.4.2.3 | LRM clear | OK |
| [`external-name-architecture-subprogram-unbound`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/external-name-architecture-subprogram-unbound) | 8.7, 14.4.1 | LRM ambiguity | REJECT |
| [`external-name-package-subprogram-unbound`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/external-name-package-subprogram-unbound) | 8.7 | LRM ambiguity | OK |
| [`external-name-in-package-declaration`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/external-name-in-package-declaration) | 8.7, 14.4.1 | LRM clear | REJECT |
| [`procedure-in-process-drives-external`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/procedure-in-process-drives-external) | 14.7.2, 8.7 | LRM clear | OK |
| [`elab-time-report-error`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/elab-time-report-error) | 10.4, 14.7.5 (silent on elaboration-time reports) | LRM ambiguity | FATAL: prints the error, simulates on, e |
| [`signal-init-reads-port`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/signal-init-reads-port) | 14.4.2.4, 14.7.3.3 | LRM ambiguity | OK |
| [`constant-init-reads-signal`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/constant-init-reads-signal) | 14.4.2.2, 14.5.2 | LRM ambiguity | OK |
| [`path-name-of-block-label-in-inner-region`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/path-name-of-block-label-in-inner-region) | 16.2.5 | LRM clear | OK |
| [`aggregate-piece-context-direction`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/aggregate-piece-context-direction) | 9.3.3.3 | LRM ambiguity | OK |
| [`aggregate-piece-direction`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/aggregate-piece-direction) | 9.3.3.3 | LRM ambiguity | OK |
| [`aggregate-piece-element-range-direction`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/aggregate-piece-element-range-direction) | 9.3.3.3 | LRM ambiguity | OK |
| [`aggregate-target-several-choices`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/aggregate-target-several-choices) | 10.5.2.1, 10.6.2.1 | LRM clear | REJECT |
| [`aggregate-target-overlap`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/aggregate-target-overlap) | 10.5.2.1, 10.6.2.1 | LRM ambiguity | OK |
| [`aggregate-piece-mixed-direction`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/aggregate-piece-mixed-direction) | 9.3.3.3 | LRM ambiguity | REJECT |
| [`aggregate-target-element-choice`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/aggregate-target-element-choice) | 10.6.2.1, 9.3.3.3 | LRM ambiguity | OK |
| [`aggregate-target-element-order`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/aggregate-target-element-order) | 10.6.2.1, 9.3.3.3 | LRM ambiguity | OK |
| [`aggregate-target-range-choice`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/aggregate-target-range-choice) | 10.5.2.1, 10.6.2.1, 9.3.3.3 | LRM ambiguity | OK |
| [`aggregate-target-range-directions`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/aggregate-target-range-directions) | 10.6.2.1, 9.3.3.3 | LRM ambiguity | REJECT |
| [`aggregate-target-range-order`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/aggregate-target-range-order) | 10.6.2.1, 9.3.3.3 | LRM ambiguity | OK |
| [`aggregate-target-record-names`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/aggregate-target-record-names) | 10.5.2.1, 10.6.2.1, 9.3.3.3 | LRM clear | OK |
| [`aggregate-target-nested-record`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/aggregate-target-nested-record) | 10.5.2.1, 10.6.2.1 | LRM ambiguity | OK |
| [`aggregate-target-nested-array`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/aggregate-target-nested-array) | 10.5.2.1, 10.6.2.1 | LRM ambiguity | OK |
| [`aggregate-target-matrix`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/aggregate-target-matrix) | 10.5.2.1, 10.6.2.1, 9.3.3.3 | LRM ambiguity | OK |
| [`assert-count-cycles`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/assert-count-cycles) | std.env (2019) | LRM ambiguity | OK |
| [`concat-null-operands`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/concat-null-operands) | 9.2.5 | LRM clear | OK |
| [`concat-null-operands-three`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/concat-null-operands-three) | 9.2.5 | LRM clear | OK |
| [`concat-null-operands-four`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/concat-null-operands-four) | 9.2.5 | LRM clear | OK |
| [`delayed-initial-value`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/delayed-initial-value) | 14.7.5.2 | LRM clear | OK |
| [`element-length-open-formal`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/element-length-open-formal) | 5.3.2.2, 6.5.6.2 | LRM clear | FATAL |
| [`ieee-body-warnings`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/ieee-body-warnings) | IEEE package source | LRM clear | OK |
| [`integer-low-2019`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/integer-low-2019) | 5.2.3.1 (2019: at least -2**63+1 to 2**63-1) | LRM ambiguity | OK |
| [`last-value-slice`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/last-value-slice) | 16.2.4 | LRM clear | OK |
| [`process-order-in-cycle`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/process-order-in-cycle) | 14.7.5.3 (order unspecified) | LRM ambiguity | OK |
| [`reflect-access-get`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/reflect-access-get) | 2019 16.11 | reference bug | OK |
| [`reflect-array-of-records`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/reflect-array-of-records) | 2019 16.11 | reference bug | OK |
| [`reflect-enum-subtype-range`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/reflect-enum-subtype-range) | 2019 16.11 | reference bug | OK |
| [`reflect-index-bases`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/reflect-index-bases) | 2019 16.11 (prose fixes no base) | LRM ambiguity | OK |
| [`reflect-multidim-get`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/reflect-multidim-get) | 2019 16.11 | reference bug | OK |
| [`reflect-physical-time-value`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/reflect-physical-time-value) | 2019 16.11 | LRM ambiguity | OK |
| [`reflect-signal-mirrors`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/reflect-signal-mirrors) | 2019 16.11 | reference bug | OK |
| [`shift-negative-count`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/shift-negative-count) | 9.2.4 | LRM clear | OK |
| [`transaction-at-init`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/transaction-at-init) | 14.7.5.2 | LRM clear | OK |
| [`external-name-subprogram-generate`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/external-name-subprogram-generate) | 8.7 | LRM clear | OK |
| [`external-constant-run-time-value`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/external-constant-run-time-value) | 8.7, 16.2.5 | LRM clear | OK |
| [`pure-function-returns-access`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/pure-function-returns-access) | 4.2.1 | LRM clear | REJECT |
| [`force-in-port-actual`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/force-in-port-actual) | 10.5.2.1, 14.7.3.3 | LRM clear | OK |
| [`delayed-in-port-init`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/delayed-in-port-init) | 16.2.3, 14.7.5.2 | LRM ambiguity | OK |
| [`force-resolved-composite-whole`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/force-resolved-composite-whole) | 14.7.3.2, 14.7.3.3 | LRM clear | OK |
| [`release-after-part-release`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/release-after-part-release) | 10.5.2.1 | LRM clear | OK |
| [`release-array-of-records`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/release-array-of-records) | 10.5.2.1 | LRM clear | OK |
| [`protected-path-name`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/protected-path-name) | 16.2.5 (2019) | LRM ambiguity | OK |
| [`reflect-integer-simple-name`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/reflect-integer-simple-name) | 16.11 (2019) | LRM ambiguity | OK |
| [`last-value-composite`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/last-value-composite) | 16.2.4 | LRM clear | OK |
| [`active-in-delta-of-assignment`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/active-in-delta-of-assignment) | 14.7.3.2, 16.2.4 | LRM clear | OK |
| [`use-type-operations`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/use-type-operations) | 12.4 | LRM ambiguity | OK |
| [`resolution-array-bounds`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/resolution-array-bounds) | 4.6, 14.7.3.2 | LRM ambiguity | OK |
| [`wait-past-time-high`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/wait-past-time-high) | 10.2, 14.7.5 | LRM ambiguity | OK |
| [`text-file-read-write`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/text-file-read-write) | 5.5.2, 16.4 | LRM ambiguity | OK |
| [`textio-read-failure`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/textio-read-failure) | 16.4 | LRM ambiguity | FATAL |
| [`textio-read-bitvector-white-space`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/textio-read-bitvector-white-space) | 16.4 | LRM ambiguity | OK |
| [`formal-package-without-actual`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/formal-package-without-actual) | 6.5.6.2, 6.5.7.2 | LRM clear | REJECT |
| [`range-constraint-outside-type-mark-static`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/range-constraint-outside-type-mark-static) | 5.2.1, 6.3 | LRM clear | REJECT |
| [`range-constraint-outside-type-mark-dynamic`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/range-constraint-outside-type-mark-dynamic) | 5.2.1, 6.3 | LRM clear | FATAL |
| [`index-constraint-outside-index-subtype-static`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/index-constraint-outside-index-subtype-static) | 5.3.2.2, 6.3 | LRM clear | REJECT |
| [`index-constraint-outside-index-subtype`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/index-constraint-outside-index-subtype) | 5.3.2.2, 6.3 | LRM clear | FATAL |
| [`array-conversion-index-outside-target-static`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/array-conversion-index-outside-target-static) | 9.3.6 | LRM clear | FATAL |
| [`array-conversion-index-outside-target-dynamic`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/array-conversion-index-outside-target-dynamic) | 9.3.6 | LRM clear | FATAL |
| [`wait-in-procedure-of-sensitivity-list-process`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/wait-in-procedure-of-sensitivity-list-process) | 11.3 | LRM clear | REJECT |
| [`wait-in-procedure-of-sensitivity-list-process-package`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/wait-in-procedure-of-sensitivity-list-process-package) | 11.3 | LRM clear | FATAL |
| [`integer-literal-outside`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/integer-literal-outside) | 9.3.6, 5.2.1 | LRM clear | FATAL |
| [`record-aggregate-field-range`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/record-aggregate-field-range) | 9.3.3.2, 5.2.1 | LRM clear | FATAL |
| [`aggregate-target-self-value`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/aggregate-target-self-value) | 10.6.2.1 | LRM clear | OK |
| [`package-instance-path-name`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/package-instance-path-name) | 16.2.5 | LRM clear | OK |
| [`subprogram-signature-instance-name`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/subprogram-signature-instance-name) | 16.2.5 | LRM ambiguity | OK: `show[string, std_logic, natural]` ( |
| [`psl-edge-unknown-value`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/psl-edge-unknown-value) | PSL (IEEE 1850) 5.2.3.4, 5.2.3.5 | LRM ambiguity | OK |
| [`short-circuit-constant-false-effect`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/short-circuit-constant-false-effect) | 9.2.2 | LRM clear | OK |
| [`index-constraint-element-bound`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/index-constraint-element-bound) | 5.3.2.2 | LRM clear | FATAL |
| [`subtype-range-bound-variable`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/subtype-range-bound-variable) | 14.4.2.1 | LRM clear | OK |
| [`subtype-of-subtype-bound-once`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/subtype-of-subtype-bound-once) | 14.4.2.1 | LRM clear | OK |
| [`unused-constant-index-constraint`](https://github.com/ru551n/yavs/tree/main/tests/discrepancies/unused-constant-index-constraint) | 5.3.2.2, 14.4.2.1 | LRM clear | REJECT |

</details>

### Third-party VHDL code

Third-party VHDL libraries and designs, fetched at pinned commits, analysed file by file in build order. A file counts as passing when yavs analyses it; the gated list (`tools/corpus/<name>.expected`) must match exactly, in both directions. The last column is sim1 on the same files (recorded list `<name>.sim1.expected`). Failures are mostly constructs the corpus uses that yavs rejects on purpose or does not support yet, each tracked as an issue.

| corpus | yavs passes | equals gated list | sim1 passes |
| --- | ---: | --- | ---: |
| en_cl_fix-2008 | 149/149 | yes | 149/149 |
| fletcher-2008 | 101/101 | yes | 101/101 |
| grlib-1993 | 454/454 | yes | 454/454 |
| grlib-2008 | 410/410 | yes | 410/410 |
| hdl-modules-2008 | **83/86** | yes | 86/86 |
| hdl-modules-2019 | **83/86** | yes | 86/86 |
| json-for-vhdl-2008 | 149/149 | yes | 149/149 |
| neorv32-2008 | 67/67 | yes | 67/67 |
| neotrng-2008 | 2/2 | yes | 2/2 |
| open-logic-2008 | 386/386 | yes | 386/386 |
| osvvm-2008 | 174/174 | yes | 174/174 |
| osvvm-2019 | 174/174 | yes | 174/174 |
| poc-2008 | 181/181 | yes | 181/181 |
| satcat5-2008 | 246/246 | yes | 246/246 |
| surf-2008 | 318/318 | yes | 310/318 |
| unisim-2008 | a vendor library directory is not available |  | - |
| uvvm-2008 | 235/235 | yes | 235/235 |
| uvvm-2019 | 235/235 | yes | 235/235 |
| vests-1993 | 1939/1939 | yes | 1939/1939 |
| vhdl-compliance-2008 | 171/171 | yes | 171/171 |
| vhdl-compliance-2019 | 208/208 | yes | 208/208 |
| vhdl-extras-2008 | 25/25 | yes | 25/25 |
| vhdl-ideas-2019 | **18/19** | yes | 18/19 |
| vhdl-interfaces-2019 | 34/34 | yes | 34/34 |
| vhdl-tool-checks-2008 | 1/1 | yes | 1/1 |
| vhdl-tool-checks-2019 | 1/1 | yes | 1/1 |
| vhlib-2008 | 26/26 | yes | 26/26 |
| vunit-2008 | 143/143 | yes | 143/143 |
| vunit-2019 | 143/143 | yes | 143/143 |
| vunit-tests-2008 | 218/218 | yes | 218/218 |
| vunit-tests-2019 | 218/218 | yes | 218/218 |
| yavs-lib-1993 | 28/28 | yes | - |
| yavs-lib-2008 | 27/27 | yes | 27/27 |
| yavs-lib-2019 | 28/28 | yes | 26/28 |

### Verification libraries and test frameworks

The libraries' own testbenches and regression suites run on yavs, each with the library's verdict.

| suite | runs | pass | fail | skip | note |
| --- | ---: | ---: | ---: | ---: | --- |
| UVVM maintenance testbenches (CI) | 137 | 131 | 0 | 6 | one run per entity, architecture and generic set the project's own script lists |
| OSVVM `.pro` regression, VHDL-2008 (CI) | 314 | 314 | 0 | 0 | pass = exit 0 and the library's `DONE PASSED` line |
| OSVVM `.pro` regression, VHDL-2019 (CI) | 314 | 314 | 0 | 0 | pass = exit 0 and the library's `DONE PASSED` line |
| VUnit's own VHDL suites (local, 2026-10-20 at `f2ab69be`) | 1357 | 1357 | 0 | 0 | 11 suites; recorded in docs/STATUS.md (#112) |
| Open Logic's own testbenches run with yavs (local, 2026-10-28 at `43000dd6`) | 6867 | 6867 | 0 | 0 | Open Logic 4.7.0 with its en_cl_fix submodule; every test of the project's own run.py (base, axi, intf, fix, ft; 6867 in all) |

### VESTs, the VHDL-1993 language suite

VESTs, the VHDL test suite for IEEE 1076-1993 (University of Cincinnati; the billowitch, ashenden and clifton-labs parts), run on yavs at `--std=93` with the suite's own test lists and expectations (`tools/suites/vests.py`, whose docstring has the exact rules). Compliant tests are analysed and run (billowitch, clifton-labs: pass = the test's PASSED line, no FAILED line, no error) or analysed only where the suite only builds them (ashenden); non-compliant tests must fail: the billowitch `analyzer_failure` ones at analysis, the others at any step. Where a test's expectation is wrong by the language reference, or the file is not legal VHDL-1993, or every tool rejects it under the suite's own build list, or the result is implementation-defined, the test carries a recorded reason in the driver: the first kind counts as a pass, the others are not counted. The billowitch `disputed` directory has no expectation and is not run.

| group | pass | not counted |
| --- | ---: | ---: |
| billowitch/compliant | **1608/1612** | 0 |
| billowitch/analyzer_failure | 1490/1490 | 0 |
| billowitch/simulator_failure | 38/38 | 0 |
| ashenden/compliant | **448/451** | 30 |
| ashenden/non_compliant | 7/7 | 1 |
| clifton-labs/compliant | 96/96 | 13 |
| total | **3687/3694** | 44 |

Of the 1490 `analyzer_failure` passes, 1466 fail at analysis and 24 are stopped by a yavs error at elaboration or run time before the test's own verdict, each checked to be the error the test is about (values out of range, lengths, delays, non-static actuals: the language reference leaves open when such errors are detected). As in the suite's own runner, an analysis failure counts without a check that it is the error the test is about. In every group that must fail, a first error that is E1999 (not analysed yet), E2001 (not lowered yet), E9999 or a crash is a failure.

Failures by the issue that tracks their cause:

- [#816](https://github.com/ru551n/yavs/issues/816): `tc1781.vhd`
- [#817](https://github.com/ru551n/yavs/issues/817): `tc3153.vhd`, `tc3156.vhd`, `tc3159.vhd`
- [#819](https://github.com/ru551n/yavs/issues/819): `ch_11_fg_11_09.vhd`, `ch_11_fg_11_10.vhd`
- [#820](https://github.com/ru551n/yavs/issues/820): `ch_13_fg_13_24.vhd`


### VHPI

Small C plugins call the VHPI functions in a fixed order on small designs (23 plugin runs); the traces from yavs and sim1 are diffed (`tools/vhpi-conformance`). 2 traces are identical. Of sim1's 2042 trace lines yavs reproduces 1382; 660 differ and yavs prints 876 lines sim1 does not. Differences are judged one by one: many are properties or callbacks that sim1 reports as unsupported and yavs provides, some are deliberate (`tools/vhpi-conformance/AGENTS.md`), real defects become issues.

### Real designs

Complete designs, run with the interpreter. neorv32: a RISC-V system running its built-in program until the stop time, three configurations (pass = the run finishes with status 0). GRLIB LEON3: the template design boots from its PROM image and runs GRLIB's own system test (pass = status 0 and the line `Test passed`).

| design | interp |
| --- | --- |
| grlib_leon3mp | pass |
| neorv32_dual_full | pass |
| neorv32_single_cache | pass |
| neorv32_single_min | pass |

Under the `auto` tier (local, 2026-10-26 at `cd5268f8b2`): grlib_leon3mp pass, neorv32_dual_full pass, neorv32_single_cache pass, neorv32_single_min pass.

### Waveforms of real designs

17 real designs, among them neorv32 and GRLIB LEON3, were run on yavs and the references and compared signal by signal and time step by time step: with the interpreter 44824 of 44889 signals are equal over 50301686 value changes; 65 of the 65 that differ are differences between the reference simulators themselves, 0 are unexplained.

<details><summary>Every design and signal class</summary>

Waveform equivalence: run locally on 2026-10-27 at 05aa8453fb (yavs build 5dc3cb5d28), references sim1, sim2.
Reproduce: `python3 -I tools/waveq/run.py BIN`

| design | A | B | signals | value changes | equal | different | equal as slice | other shape | only A | only B | differences | A compiled code |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| rtl_ff | yavs-interp | sim1 | 3 | 604 | 3 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_ff | yavs-auto | sim1 | 3 | 604 | 3 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_ff | yavs-llvm | sim1 | 3 | 604 | 3 | 0 | 0 | 0 | 0 | 0 | none | 285 IR instructions |
| rtl_ff | sim2 | sim1 | 3 | 604 | 3 | 0 | 0 | 0 | 0 | 0 | none |  |
| rtl_lfsr | yavs-interp | sim1 | 66 | 33067 | 66 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_lfsr | yavs-auto | sim1 | 66 | 33067 | 66 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_lfsr | yavs-llvm | sim1 | 66 | 33067 | 66 | 0 | 0 | 0 | 0 | 0 | none | 357 IR instructions |
| rtl_lfsr | sim2 | sim1 | 66 | 33067 | 66 | 0 | 0 | 0 | 0 | 0 | none |  |
| rtl_chain | yavs-interp | sim1 | 4 | 605 | 4 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_chain | yavs-auto | sim1 | 4 | 605 | 4 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_chain | yavs-llvm | sim1 | 4 | 605 | 4 | 0 | 0 | 0 | 0 | 0 | none | 261 IR instructions |
| rtl_chain | sim2 | sim1 | 4 | 605 | 4 | 0 | 0 | 0 | 0 | 0 | none |  |
| rtl_fan | yavs-interp | sim1 | 4 | 405 | 4 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_fan | yavs-auto | sim1 | 4 | 405 | 4 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_fan | yavs-llvm | sim1 | 4 | 405 | 4 | 0 | 0 | 0 | 0 | 0 | none | 137 IR instructions |
| rtl_fan | sim2 | sim1 | 4 | 405 | 4 | 0 | 0 | 0 | 0 | 0 | none |  |
| rtl_bus | yavs-interp | sim1 | 4 | 2005 | 4 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_bus | yavs-auto | sim1 | 4 | 2005 | 4 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_bus | yavs-llvm | sim1 | 4 | 2005 | 4 | 0 | 0 | 0 | 0 | 0 | none | 261 IR instructions |
| rtl_bus | sim2 | sim1 | 4 | 2005 | 4 | 0 | 0 | 0 | 0 | 0 | none |  |
| rtl_ram | yavs-interp | sim1 | 16390 | 30385 | 16390 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_ram | yavs-auto | sim1 | 16390 | 30385 | 16390 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| rtl_ram | yavs-llvm | sim1 | 16390 | 30385 | 16390 | 0 | 0 | 0 | 0 | 0 | none | 675 IR instructions |
| rtl_ram | sim2 | sim1 | 16390 | 30385 | 16390 | 0 | 0 | 0 | 0 | 0 | none |  |
| lfsr64_xor | yavs-interp | sim1 | 67 | 33567 | 67 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| lfsr64_xor | yavs-auto | sim1 | 67 | 33567 | 67 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| lfsr64_xor | yavs-llvm | sim1 | 67 | 33567 | 67 | 0 | 0 | 0 | 0 | 0 | none | 382 IR instructions |
| lfsr64_xor | sim2 | sim1 | 67 | 33567 | 67 | 0 | 0 | 0 | 0 | 0 | none |  |
| lfsr64 | yavs-interp | sim1 | 67 | 33567 | 67 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| lfsr64 | yavs-auto | sim1 | 67 | 33567 | 67 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| lfsr64 | yavs-llvm | sim1 | 67 | 33567 | 67 | 0 | 0 | 0 | 0 | 0 | none | 309 IR instructions |
| lfsr64 | sim2 | sim1 | 67 | 33567 | 67 | 0 | 0 | 0 | 0 | 0 | none |  |
| neorv32_single_min | yavs-interp | sim1 | 13466 | 15877072 | 13466 | 0 | 133 | 0 | 1045 | 0 | none | 0 |
| neorv32_single_min | yavs-auto | sim1 | 13466 | 15877072 | 13466 | 0 | 133 | 0 | 1045 | 0 | none | 0 |
| neorv32_single_min | yavs-llvm | sim1 | 13466 | 15877072 | 13466 | 0 | 133 | 0 | 1045 | 0 | none | 128233 IR instructions |
| grlib_leon3mp | yavs-interp | sim1 | 13897 | 33197020 | 13897 | 0 | 230 | 0 | 3843 | 0 | none | 0 |
| grlib_leon3mp | yavs-auto | sim1 | 13897 | 33197020 | 13897 | 0 | 230 | 0 | 3843 | 0 | none | 4 functions |
| grlib_leon3mp | yavs-llvm | sim1 | 13897 | 33197020 | 13897 | 0 | 230 | 0 | 3843 | 0 | none | 226142 IR instructions |
| uvvm_sb_slv | yavs-interp | sim1 | 3 | 3 | 3 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| uvvm_sb_slv | yavs-auto | sim1 | 3 | 3 | 3 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| uvvm_sb_slv | yavs-llvm | sim1 | 3 | 3 | 3 | 0 | 0 | 0 | 0 | 0 | none | 42834 IR instructions |
| uvvm_util_rand | yavs-interp | sim1 | 3 | 3 | 3 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| uvvm_util_rand | yavs-auto | sim1 | 3 | 3 | 3 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| uvvm_util_rand | yavs-llvm | sim1 | 3 | 3 | 3 | 0 | 0 | 0 | 0 | 0 | none | 84159 IR instructions |
| uvvm_axistream_vvc | yavs-interp | sim1 | 154 | 785584 | 150 | 4 | 0 | 0 | 20 | 1 | reference process-order-in-cycle: 4 | 0 |
| uvvm_axistream_vvc | yavs-auto | sim1 | 154 | 785584 | 150 | 4 | 0 | 0 | 20 | 1 | reference process-order-in-cycle: 4 | 0 |
| uvvm_axistream_vvc | yavs-llvm | sim1 | 154 | 785584 | 150 | 4 | 0 | 0 | 20 | 1 | reference process-order-in-cycle: 4 | 69265 IR instructions |
| uvvm_methods_log | yavs-interp | sim1 | 3 | 3 | 3 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| uvvm_methods_log | yavs-auto | sim1 | 3 | 3 | 3 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| uvvm_methods_log | yavs-llvm | sim1 | 3 | 3 | 3 | 0 | 0 | 0 | 0 | 0 | none | 14064 IR instructions |
| uvvm_func_cov | yavs-interp | sim1 | 4 | 4 | 4 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| uvvm_func_cov | yavs-auto | sim1 | 4 | 4 | 4 | 0 | 0 | 0 | 0 | 0 | none | 0 |
| uvvm_func_cov | yavs-llvm | sim1 | 4 | 4 | 4 | 0 | 0 | 0 | 0 | 0 | none | 45937 IR instructions |
| osvvm_axis_random | yavs-interp | sim1 | 223 | 109554 | 202 | 21 | 0 | 0 | 2 | 0 | reference process-order-in-cycle: 21 | 0 |
| osvvm_axis_random | yavs-auto | sim1 | 223 | 109554 | 202 | 21 | 0 | 0 | 2 | 0 | reference process-order-in-cycle: 21 | 0 |
| osvvm_axis_random | yavs-llvm | sim1 | 223 | 109554 | 202 | 21 | 0 | 0 | 2 | 0 | reference process-order-in-cycle: 21 | 55734 IR instructions |
| osvvm_axi4_random | yavs-interp | sim1 | 531 | 198238 | 491 | 40 | 0 | 0 | 0 | 0 | reference process-order-in-cycle: 40 | 0 |
| osvvm_axi4_random | yavs-auto | sim1 | 531 | 198238 | 491 | 40 | 0 | 0 | 0 | 0 | reference process-order-in-cycle: 40 | 0 |
| osvvm_axi4_random | yavs-llvm | sim1 | 531 | 198238 | 491 | 40 | 0 | 0 | 0 | 0 | reference process-order-in-cycle: 40 | 69313 IR instructions |

Not configured (no waveform keys for the kind of design in tools/sims.local.conf): sim2 for 9 designs

Differences explained:

- reference process-order-in-cycle: the verification components take IDs from a shared counter in the order their processes run in one cycle; the LRM leaves that order open (14.7.5.3) and the references differ among themselves (docs/design/simulator-discrepancies.md); every bus signal is equal

</details>

### Platforms covered by CI

| job | runs on |
| --- | --- |
| `check` | ubuntu-24.04, ubuntu-24.04-arm, windows-latest |
| `llvm` | ubuntu-24.04, ubuntu-24.04-arm, windows-latest |
| `clift` | ubuntu-24.04, ubuntu-24.04-arm, windows-latest |

The `check` job is `cargo fmt`, `clippy -D warnings` and the whole test suite of the workspace; the `llvm` job the same for the LLVM tier plus a simulation smoke run. The `corpus` and `conformance` jobs run on Linux x86_64 only.

### Where and when each result was produced

| suite | where | date, commit | tool |
| --- | --- | --- | --- |
| Differential testbenches | CI | this report (see Generated) | `tools/difftest/difftest.py` |
| Differential testbenches, `auto` tier | local | 2026-10-26, `cd5268f8b2` | `tools/conformance/run.py --local --llvm` |
| Differential testbenches, `llvm` tier | local | 2026-10-26, `cd5268f8b2` | `tools/conformance/run.py --local --llvm` |
| Known simulator differences | CI | this report (see Generated) | `tools/discrepancies/run.py` |
| sim1's regression tests | CI | this report (see Generated) | `tools/difftest/external.py` |
| sim2's regression tests | CI | this report (see Generated) | `tools/difftest/external.py` |
| VESTs | CI | this report (see Generated) | `tools/suites/vests.py` |
| UVVM | CI | this report (see Generated) | `tools/suites/uvvm.py` |
| OSVVM, VHDL-2008 | CI | this report (see Generated) | `tools/suites/pro.py` |
| OSVVM, VHDL-2019 | CI | this report (see Generated) | `tools/suites/pro.py` |
| VUnit | local | 2026-10-20, `f2ab69be` | VUnit's own `run.py` scripts |
| Open Logic | local | 2026-10-28, `43000dd6` | Open Logic's `sim/run.py` |
| Real designs | CI | this report (see Generated) | `tools/realbench/run.py` |
| Waveforms of real designs | local | 2026-10-27, `05aa8453fb` | `tools/waveq` |
| Third-party VHDL code | CI | this report (see Generated) | `tools/corpus/run.py` |
| VHPI | local | 2026-10-26, `cd5268f8b2` | `tools/vhpi-conformance/run.py` |

## How to reproduce

```sh
tools/fetch-corpus.sh                        # the external suites and libraries at pinned commits, ~/.cache/rsim-corpus
cargo build --release -p yavs                # add --features llvm for the LLVM tier (needs LLVM 21)
tools/conformance/run.py --yavs target/release/yavs --out ~/.cache/yavs-conformance                 # what CI runs
tools/conformance/run.py --yavs target/release/yavs --out ~/.cache/yavs-conformance --local --llvm  # plus the local results
tools/conformance/run.py --check --yavs target/release/yavs --out ~/.cache/yavs-conformance         # is the committed report current?
```

The local results need the reference simulators, configured in the untracked `tools/sims.local.conf`; a `--local` run rewrites `docs/conformance-local.json`, which is committed with the regenerated report. Each tool in the table above documents its own options.

## Known limits

| area | state |
| --- | --- |
| Verilog and mixed-language designs | not supported yet; the plan is [#776](https://github.com/ru551n/yavs/issues/776) |
| Encrypted IP | an encrypted VHDL file stops with E0012 ([#775](https://github.com/ru551n/yavs/issues/775)); the contents are never guessed |
| PSL | a subset; the rest is skipped with warning W0004 or listed in the tracking issue [#445](https://github.com/ru551n/yavs/issues/445) |
| SDF and VITAL timing, SystemC, macOS | out of scope (docs/plan.md, "Out of scope") |
| Platforms | Linux x86_64, Linux aarch64 and Windows x86_64 only |
| LLVM tier | the release archives use the Cranelift compiled tier (`--tier=auto` default); the LLVM tier exists only in a build from source with the `llvm` feature. CI runs a slice of the corpus under the LLVM tier on the three platforms |
| Reference simulators | the comparison is against two established open-source VHDL simulators (sim3 and sim4, two commercial simulators, vote in part of the discrepancy catalogue); where the language reference is unclear yavs is permissive and the choice is recorded below |
| What a pass means | a pass is agreement with a recorded reference output or with a suite's own verdict. It is evidence, not a proof of correctness; each known disagreement is listed in the catalogue |
