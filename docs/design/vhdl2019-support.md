# VHDL-2019 support matrix

Feature-by-feature status of the approved VHDL-2019 language change specifications (LCS) in yavs,
with the result of the four reference simulators (`sim1` to `sim4`, labels as in
`docs/design/simulator-discrepancies.md`) on the same small test per feature. Written 2026-10-19.

## How it was made

- The feature list is the approved LCS-2016 list; the LCS pages were unreachable during this audit, so
  titles and numbers come from the public feature table of one reference simulator and the scope of
  each LCS was inferred from the title, the vendored 2019 library sources and the existing yavs notes. Where the scope could not
  be pinned down the row says `not tested`.
- Each row has one or more fresh probes (a few lines of VHDL, `--std=2019`), analyzed and, where the
  feature has run-time meaning, run on yavs and on every reference. The probes were not added to
  `tests/discrepancies/`; the rows that expose a difference between references are listed under
  "Differences found".
- yavs status: `supported` (probe gives the LRM result and agrees with the references that run it),
  `partial` (some forms work), `unsupported`, `rejected by decision` (none of the LCS probes; the
  access-typed constant parameter rule L46 is the one decision that touches an LCS), `not tested`,
  `not applicable`.
- Reference cells: `ok` runs with the same messages as yavs, `ok (output differs)` runs with other
  messages, `reject` fails analysis or elaboration, `crash`, `fatal` stops at run time. When a row
  has several probes the cells list their results in the order of the probe list in the note.
- Only the revision 2019 was run; yavs 2008 behaviour for these features is in
  `docs/design/lrm-checks.md`.

## Summary

46 LCS rows: 41 supported, 0 partial, 0 unsupported, 0 rejected by decision, 4 not tested, 1 not applicable.

Correction 2026-10-28: the matrix below has 43 rows marked supported, one partly supported (099), one not tested (043) and
one not applicable (030). The line above was never updated. With the corrections at the end of this page, rows 034 and 050
are partial too: 41 supported, 3 partial, 1 not tested, 1 not applicable.

For comparison, over the rows that have a probe, the references run the probe (`ok` or `ok (output differs)`) for the following number of rows: sim1 37, sim2 11, sim3 22, sim4 39 (rows with several probes count only when all probes agree).

## Matrix

| LCS-2016 | Feature | yavs | sim1 | sim2 | sim3 | sim4 | Note |
|---|---|---|---|---|---|---|---|
| 001 | Partially connected vectors on a port map | supported | ok | reject | reject | ok | `open` for a slice of an out port |
| 002 | Protected type parameters on function interfaces | supported | ok | ok | reject | ok |  |
| 004 | Protected methods with access and file parameters | supported | ok | reject | reject | ok | variable parameters; constant access parameters stay `E1223` by decision (L46) |
| 006a | File I/O and TextIO updates | supported | ok | ok | ok | fatal | `swrite`, `sread`, `flush` only; other additions not probed |
| 006c | Directory API | supported | ok | reject | ok (output differs) | ok | existence and working directory queries only |
| 006e | Environment API | supported | ok | reject | ok | ok | `getenv`, `vhdl_version`, tool identification |
| 006f | Standard conditional analysis identifiers | supported | ok | reject | ok | ok |  |
| 007 | Sequential declaration regions | supported | ok | ok | ok | ok | editorial rule; only name hiding in nested regions probed |
| 011 | Date and time functions | supported | ok | reject | ok | ok |  |
| 012 | `'IMAGE` and `TO_STRING` for composite types | supported | ok/ok/ok/ok/ok (output differs) | reject | ok/ok/ok/reject/ok (output differs) | ok/ok/ok/ok/ok (output differs) | `'IMAGE` (type mark and object forms) and `TO_STRING` of arrays and records run (#360, L105); the references differ in quoting (catalogue `composite-image-quoting`). Cells list the five probes in order |
| 014 | Composites of protected types | supported | reject | reject | reject | ok | each element is its own object; a record mixing protected and other elements is `E1905` (5.3.1, L102, #371); a variable port of such a composite is `E1907` (not supported, #370) |
| 014a | Access types to composites of protected types | supported | reject | reject | reject | ok | allocation and deallocation (#361) |
| 015 | Report the calling path | supported | crash | reject | ok | crash | `get_call_path` |
| 015a | Report the current file name | supported | ok | reject | ok | ok | `file_name`, `file_line` |
| 016 | Anonymous types | supported | reject | reject | reject | reject | the LCS is about interface declarations: `a : in type is private`, `b : in type is <>`, `c : type is range <>` in ports, generics and parameters take their type from the actual (#283, #308, L94; the intended syntax is the LCS example in the conformance suite). No reference simulator binds the anonymous type from its actual, so the tests are `ref=hand`: `tests/corpus/generics/anonymous_ports_2019_hand.vhd`, `anonymous_objects_2019_hand.vhd` |
| 018 | Attributes for enumerated types | supported | ok | reject | reject | ok | `T'LENGTH` of a discrete type or subtype, the number of its values (0 for a null range; a physical type stays `E1621`); the other attributes of the LCS example (`'base` as a type mark, `'ascending`, `'reverse_range` of enumeration-indexed arrays) already worked (#445) |
| 018a | `'INDEX` | supported | ok | reject | reject | ok |  |
| 018d | `'DESIGNATED_SUBTYPE` | supported | ok | reject | reject | ok |  |
| 019 | Constraints from initial values of signals and variables | supported | ok | reject | reject | ok |  |
| 023 | Relax library requirement on configurations | supported | ok | reject | reject | ok | a configuration declaration may name an entity of another library (`configuration c of other.e`); before 2019 it is `E1007` as in all four references (#445) |
| 026c | Long integers | supported | ok | ok (output differs) | fatal | fatal | 64-bit integer range; `integer'high` is 2**63-1 in yavs, 2**31-1 in sim2; sim3 and sim4 stop at the 62-bit type |
| 030 | Garbage collection | not applicable | - | - | - | - | implementation property, no source syntax; `deallocate` and heap reuse are covered by the access type tests |
| 032 | `PATH_NAME` and `INSTANCE_NAME` with protected types | supported | ok/ok (output differs) | reject/ok | ok/ok (output differs) | ok | `pt'path_name` inside a method: yavs and two references give `:top:pt`, one gives `:top:sv:`, one `:top:sv:pt:` (catalogued as `protected-path-name`) |
| 033 | Composition with protected types | supported | ok | ok | ok | ok | a protected variable inside a protected body |
| 034 | Protected types with a generic clause | supported | reject | reject | reject | ok | `type t is new p.box generic map (..)` instantiates the type: methods, generics in the body, several instances (#29). A generic map in the protected type header is not implemented |
| 036a | Conditional expressions in declarations | supported | ok | reject | ok | ok |  |
| 041 | Record introspection and type reflection | supported | ok | reject | reject | ok (output differs) | `integer'reflect.simple_name` is `INTEGER` in yavs and sim1, `integer` in sim4 (sim2 and sim3 reject the probe; catalogued as `reflect-integer-simple-name`) |
| 043 | Attributes for PSL | not tested | - | - | - | - | PSL is out of scope of this page |
| 045a | Mode view declarations | supported | ok | ok/fatal/ok | ok | ok | view ports, signal parameters of a view, an array of a viewed record; gaps in #254, #265 |
| 045c | `'CONVERSE` of a mode view | supported | ok | ok | ok | ok |  |
| 047 | Variable ports of protected type | supported | ok | reject | reject | ok |  |
| 049 | Generic map on a subprogram call | supported | ok | reject | reject | ok | each call instantiates the generic subprogram (#362, L103); expanded names and concurrent calls too (sim1 rejects those two forms) |
| 050 | Assert API | supported | ok | reject | ok | ok | `GetVhdlAssertCount`, `IsVhdlAssertFailed`, `ClearVhdlAssert` |
| 055a | Syntax regularisation: component declarations | supported | ok | reject | ok | ok | `end;` ending a component declaration |
| 059 | Array type generics and the generic type classes | supported | ok | reject | reject | ok | all class forms analyze; an instance with an array type generic runs |
| 059a | Ordering on any scalar array | supported | ok | ok | ok | ok |  |
| 061 | Conditional compilation | supported | ok | reject | ok | ok |  |
| 071a | Optional trailing semicolon | supported | ok | ok | ok | ok |  |
| 072b | Function knows the vector size (`return rv_t of bit_vector`) | supported | ok | reject | reject | ok |  |
| 075 | Closely related record types | supported | reject | reject | reject | ok | conversion between records of the same element names and types |
| 082 | Empty record | supported | ok | reject | ok | ok |  |
| 086 | All interface lists can be ordered | supported | ok | reject | reject | ok | later items refer to earlier ones through `'subtype` |
| 094a | Conditional return | supported | ok | reject | reject | ok | `return x when c` in functions and procedures |
| 099 | Extended ranges | partly supported | ok | reject | reject | ok | `R'RECORD` and `R'VALUE` were there; a constant of a range record type now works as a range (index constraint, range constraint, array type, loop) when its value is known at analysis (`E1410` otherwise); the sole accepting reference answers `'ascending` wrongly for one case (#445) |
| I03 | Signatures in the formal part of a generic map | supported | ok | reject | reject | ok | subprogram, package and entity instances; overloaded formals; the signature may name the list's generic types (#363, L104) |
| I13 | Precedence of unary operators | supported | ok | ok | ok | ok | only `-k ** 2` and `not` against `=` and `and` probed; weak test |

## Gaps by use in practice

Issues for VHDL-2019 gaps carry the GitHub label `v2019` (user, 2026-10-19); give a new gap issue that
label.

Signal: number of files in the local sources of two verification libraries and one external test runner
(in that order) that use the construct (a rough text count, not a parse). Mode views appear in 26 and 10
files of the libraries; call path and file name functions in 17, 69 and 38 files; `to_string` in 246, 149
and 73; conditional return in 5, 4 and 6.
Generic protected types, protected arrays, a generic map on a call and signatures in a generic map do not
occur in those sources.

| Rank | Gap | Issue |
|---|---|---|
| 1 | `'IMAGE` and `TO_STRING` of arrays and records (LCS-012): done | #360 |
| 2 | Generic protected type instantiation (LCS-034): done | #29 |
| 3 | Generic map on a subprogram call (LCS-049): done | #362 |
| 4 | Arrays and records of protected types (LCS-014, 014a): done | #361 |
| 5 | Signature in a generic map formal (LCS-I03): done | #363 |
| 6 | Anonymous types (LCS-016): done | #283, #308 |

## Differences found

- `pt'path_name` inside a protected method: four different answers across yavs and the references
  (see row 032). The LRM text for the path of a protected object was not settled by this audit.
- `integer'reflect.simple_name` for `integer`: upper case in yavs, lower case in sim4.
- `to_string` of a record: element quoting differs between references (`(3,'1')` against `(3,1)`);
  yavs follows the majority, `(3,1)` (catalogue `composite-image-quoting`, L105).
- Long integers (LCS-026c): sim2 keeps a 32-bit `integer'high`; sim3 and sim4 stop on a 62-bit type.
- Directory functions: sim3 reports `is not supported` warnings and returns false.
- Mode views, conditional compilation, `return .. when`, `open` slices and the std.env additions are
  accepted by sim1 and sim4 and by yavs; sim2 rejects most of them and sim3 rejects the generic and typing additions (see the matrix).

## Corrections and re-probes, 2026-10-28

The matrix above is the 2026-10-19 audit and is kept as written. These rows were probed again on 2026-10-28 at commit
`ac2c94ff` (`--std=2019`, interpreter, one small design per statement); everything else was not re-probed.

- **006c Directory API**: besides existence and working-directory queries, `dir_open`, `dir_itemisdir`, `dir_itemisfile`,
  `dir_createdir` and `dir_deletedir` run and return `status_ok`. `dir_deletefile` and the other functions were not probed.
- **034 Protected types with a generic clause**: a generic map in the protected type header (`type t is protected generic map (..)`)
  is a syntax error (`E0002`, "expected `;`"), not a not-implemented diagnostic. The row stays partial: `type t is new p.box generic
  map (..)` works (test `tests/corpus/protected/generic_instance_2019.vhd`).
- **050 Assert API**: `SetVhdlAssertEnable` and `GetVhdlAssertEnable` run. `SetVhdlAssertFormat` with a non-empty format stops the run
  (`E3009`, "not implemented yet (#99)"), and `PslAssertFailed` and `PslIsCovered` stop it with `E3009` "native subprogram ... is not implemented in
  yavs yet" (`lib/NATIVE.md` lists the unimplemented PSL state natives). The matrix row covers the counting functions only.
- **055a, 071a, 082, 059a, 061**: the probes (`end;` of a component, a trailing semicolon in port, generic and parameter lists, an empty
  record, `<` on a `real_vector` (an error at `--std=2008`), `` `if `` on `TOOL_NAME`) analyse and run as the matrix says.
- **026c**: `integer'high` is 9223372036854775807 at `--std=2019` and 2147483647 at `--std=2008` and `--std=93`.
- **099 Extended ranges**: a range record constant whose value depends on a generic is `E1410` ("needs a constant with a static
  value"), as the row says.
- **Rows that now have regression testbenches** (the matrix says "probes were not added"): 011, 012, 014, 016, 034, 036a, 045, 047, 049, 072b, 075,
  094a and I03 run in `tests/corpus` (`*_2019*`, `hierarchy/mode_view_*`, `protected/`, `generics/`, `process/`). All 544 corpus testbenches pass
  at 1 and 4 threads with the interpreter on this commit.
