# Diagnostic codes used by the negative suite

Every `-- expect:` code in `tests/diag/sem/**` is listed here. "sem.md 8.1" means the code table in `docs/design/sem.md` section 8.1. Codes marked **proposed** are not in the 8.1 table (or sem.md uses two different codes for one thing); the yavs-sem owner decides. Nothing here is binding until `yavs-sem::diag` defines it.

## Proposed codes (not in sem.md 8.1, or ambiguous there)

Status (2026-10-18): every proposed code in this table is now defined in `yavs-sem::diag`
(`crates/yavs-sem/src/diag.rs`), so the table records where each code came from; W1990 stays unused.

| Code | Meaning | sem.md section | Tests |
|---|---|---|---|
| E1104 | prefix of a use clause or context reference is not a package, library, package instance or context declaration | 1.4 (suffix "must denote a package or an instance") | `use/prefix_not_a_package`, `use/prefix_is_type`, `use/context_not_a_context` |
| E1303 | illegal operator declaration: not an operator symbol, or wrong number of parameters for the operator | 4.5.2 LRM, not in sem.md | `ovl/operator_wrong_arity`, `ovl/unary_only_operator_binary` |
| E1507 | a conversion on the formal side of a generic or port with an anonymous type (`type is ...`): the formal's type can only come from the actual | 6.5.2, 6.5.3 LRM (2019), not in sem.md | `v2019/anonymous_object_formal_conversion` |
| E1623 | attribute dimension argument out of range for the prefix (`m'length(2)` of a 1-D array) | 6.3 / 4.2 "attribute dimension arguments" | `stat/attribute_dimension_out_of_range` |
| E1700 | instantiated name is not a component, entity or configuration | 3.3 (`l: p;` re-interpretation) | `unit/component_instance_not_a_component` |
| W1325 | R5 relaxation: an `others` aggregate takes the bounds of an unconstrained assignment target | 8.4 R5, LRM 9.3.3.3 | `relax/others_unconstrained_target_relaxed` |
| W1361 | R6 relaxation: a non-static actual of a signal parameter | 8.4 R6, LRM 4.2.2.3 | `relax/signal_actual_not_static_relaxed` |
| W1830 | R2 relaxation: pure function calls something impure (own warning code per relaxation, 8.4) | 8.4 R2, 6.6 | `relax/pure_function_calls_impure_relaxed` |
| W1904 | R1 relaxation: shared variable of non-protected type | 6.6 names `W1904` explicitly; 8.1 only lists `W1990` | `relax/shared_variable_unprotected_relaxed` |
| W1401 | static overflow where the value is not required; run-time check kept | Q5 (11), not in 8.1 | `stat/integer_overflow_addition`, `stat/integer_overflow_multiplication` |
| E1901 vs E1660 | view element missing or duplicate. 8.1 says `E1901`; section 6.7 says `E1660`. The tests use `E1901` (8.1 is the table) | 6.7 vs 8.1 | `v2019/view_element_*`, `v2019/view_nested_wrong_record` |
| E1820 vs E1327 | case statement duplicates/coverage. 8.1: `E1820`; section 3.4 says "the same code checks case statements" as `E1327`. The tests use `E1820` for `case` and `E1327` for aggregates | 3.4 vs 8.1 | `agg/case_*`, `stat/enum_choice_overlap_static` |
| W1990 | 8.1 lists it as "relaxed rule applied". Not used: the suite uses one code per relaxation (W1830, W1904) | 8.1, 8.4 | none |

## Owned by another stage

| Code | Meaning | Test |
|---|---|---|
| E0004 | construct requires VHDL-2019 (yavs-syntax). 2019-only syntax in a 2008 file never reaches sem, so `E1910` (8.1) is unreachable for syntactic constructs and has no test | `mix/generic_class_2008_file` |
| E0003 | end label mismatch (yavs-syntax). `E1106` from 8.1 is therefore also covered by the syntax stage and has no sem test | `tests/diag/end_label.vhd` |

## Codes from 8.1 with no test, and why

| Code | Why |
|---|---|
| E1003 dependency cycle | needs a manifest with two files; the reference simulators cannot be run on it as a single file. Add to the harness-level tests with `yavs-lib` |
| E1631 several signature matches | a full signature (parameters and result) is unique among homographs; no legal input produces it. Candidate for removal |
| E1910 | see E0004 above |
| E1106 | see E0003 above |

## All other codes used

| Code | Meaning (short) | sem.md |
|---|---|---|
| E1001 | unknown library | 8.1 |
| E1002 | unit not found in library | 8.1 |
| E1004 | secondary unit without a primary unit | 8.1 |
| E1005 | `work` in a context declaration | 8.1 |
| E1007 | not available under the file's or the design's revision | 0.1, 8.1 |
| E1100 | not declared | 8.1 |
| E1101 | redeclaration (homograph) | 8.1 |
| E1102 | use clause conflict | 8.1 |
| E1103 | exists but not visible here | 8.1 |
| E1105 | private / not in the visible part | 8.1 |
| E1201 | not a type | 8.1 |
| E1210 | dimension constrained twice | 8.1 |
| E1211 | constraint on a type that cannot take it | 8.1 |
| E1212 | object needs a fully constrained subtype | 8.1 |
| E1213 | bad record constraint | 8.1 |
| E1214 | index constraint arity | 8.1 |
| E1220 | incomplete type not completed or misused | 8.1 |
| E1221 | 2008 access to a file or protected type | 8.1 |
| E1222 | illegal file element type | 8.1 |
| E1223 | constant (2008), signal, generic, port or parameter of an access or file type, also in 2008 protected method profiles | 8.1 |
| E1224 | constant without a value outside a package declaration | 6.4.2.2 |
| E1225 | bad resolution function | 8.1 |
| E1226 | deferred constant used before its full declaration: in an initial value of its package, or in its package body (subprogram bodies too) | 6.4.2.2 |
| E1227 | full declaration of a deferred constant with a subtype indication that does not conform | 6.4.2.2 |
| E1230 | missing body (subprogram, protected type, deferred constant in a package body) | 8.1, 4.8 |
| E1238 | plain (not shared) variable outside a process, subprogram or protected type | 6.4.2.4 |
| E1239 | component declaration, configuration or disconnection specification in a process or subprogram | 4.3, 11.3 |
| E1240 | subprogram body whose specification does not conform to its declaration | 4.10 |
| E1232 | type, subprogram or package interface in a parameter list | 4.2.2.1 |
| E1233 | named return value (2019) of a type that is not scalar, array or record, e.g. an access type (#239; scalar types accepted since the user ruling of 2026-10-19) | 4.2.1 (2019) |
| E1234 | type, subprogram or package interface in a port list | 6.5.6.3 |
| E1235 | generic list or generic map on a formal generic subprogram | 6.5.4 |
| E1236 | subprogram parameter of mode `buffer` or `linkage` | 4.2.2.1 |
| E1237 | signal or shared variable in a process or subprogram, also in a package or package instance there (#34) | 11.3, 4.3, 4.7, 4.9 |
| E1300 | type mismatch | 8.1 |
| E1301 | no matching overload | 8.1 |
| E1302 | ambiguous | 8.1 |
| E1304 | named return value (2019) without a constrained context subtype (#33) | 4.2.1 (2019) |
| E1310 | formal associated twice | 8.1 |
| E1311 | missing actual | 8.1 |
| E1312 | conversion on an unconstrained formal that defines no bounds (rule 1 review F8; a conversion to a fully constrained type mark or subtype, or function subtype, is accepted, #262) | 5.3.2.2, 6.5.7.1 |
| E1313 | conversion the formal's class or mode does not allow (formal conversion of an `in` parameter, actual conversion of an `out` one, `inout` with one conversion, any conversion of a signal parameter) | 6.5.7.1, 4.2.2.3 |
| E1320 | aggregate type not determined | 8.1 |
| E1321 | record aggregate: element missing | 3.4 |
| E1322 | record aggregate: element duplicate or too many positional | 3.4 |
| E1323 | aggregate element type / `others` types / dimension depth | 3.4 |
| E1325 | `others` misplaced or needs a constrained subtype | 3.4 |
| E1326 | mixed named/positional, non-static or null range choice misuse | 3.4 |
| E1327 | duplicate or overlapping aggregate choice | 3.4 |
| E1328 | index not covered | 3.4 |
| E1334 | array values under range choices of different directions in one aggregate (L84) | 3.4 |
| E1330 | character not in type | 8.1 |
| E1331 | bit string loses digits | 8.1 |
| E1340 | condition not BOOLEAN | 3.6 |
| E1341 | `??` ambiguous | 3.6 |
| E1350 | illegal conversion | 3.7 |
| E1360 | class/mode misuse | 8.1 |
| E1362 | force or release of a member of a composite signal resolved as a whole (L97) | 8.1 |
| E1401 | static overflow where required | 8.1 |
| E1402 | literal out of range | 8.1 |
| E1403 | static value outside subtype | 8.1 |
| E1405 | static length mismatch | 8.1 |
| E1410 | not locally static where required | 8.1 |
| E1501 | not a generic unit | 8.1 |
| E1502 | missing generic actual | 8.1 |
| E1503 | actual outside generic class | 8.1 |
| E1504 | no or ambiguous `is <>` default | 8.1 |
| E1505 | use of an uninstantiated generic | 8.1 |
| E1601 | external name class misuse | 8.1 |
| E1610 | alias of a non-static name | 8.1 |
| E1611 | alias/attribute spec needs a signature | 8.1 |
| E1620 | implicit signal attribute in a subprogram | 8.1 |
| E1621 | unknown attribute / attribute not defined for the prefix | 8.1 |
| E1622 | entity class mismatch in an attribute specification | 8.1 |
| E1630 | signature matches nothing | 8.1 |
| E1640 | group template or group declaration illegal (`<>` not last, not a template) | 6.9, 6.10 |
| E1641 | group constituent illegal (class, count, not a named entity) | 6.10 |
| E1650 | disconnection specification illegal (not a guarded signal of the part, type, not static) | 7.4 |
| E1651 | second disconnection specification for a guarded signal | 7.4 |
| E1701 | label is not an instance of the component | 8.1 |
| E1702 | rebinding | 8.1 |
| E1703 | component/entity port or generic mismatch | 8.1 |
| E1704 | binding without an entity where one is needed | 8.1 |
| E1801 | `wait` misplaced, or a call of a procedure that waits from a process with a sensitivity list | 8.1, 10.2, 11.3 |
| E1802 | `return` misplaced | 8.1 |
| E1810 | signal driven from outside a process | 8.1 |
| E1811 | `next`/`exit` misplaced or unknown label | 8.1 |
| E1812 | unresolved signal with multiple sources (defined in `yavs-sem::diag`, not in 8.1) | 6.9 |
| E1813 | entity statement not passive (defined in `yavs-sem::diag`, not in 8.1) | 6.9 |
| E1820 | case coverage / duplicates | 8.1 |
| E1830 | pure function purity | 8.1 |
| E1901 | view element missing or duplicate (see proposed table) | 8.1 |
| E1902 | view on a non-record / not a view | 8.1 |
| E1903 | view port mode misuse | 8.1 |
| E1904 | shared / private variable not protected | 8.1 |
| E1905 | illegal use of a protected type | 8.1 |
| E1907 | variable port of a composite of protected types, not supported | 6.5.2 |
| E1999 | construct not implemented yet in the semantic analysis (a limitation of yavs, #842) | 8.1 |
| W1008 | construct means something different under the design revision | 0.1, 8.1 |
| W1403 | static operand of a qualified expression or type conversion outside its subtype, in possibly dead code | 9.3.5, 9.3.6, not in 8.1 |
| W1404 | static index out of range in possibly dead code | 8.1 |
| W1501 | unused generic | 8.1 |
