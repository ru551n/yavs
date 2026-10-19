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

46 LCS rows: 34 supported, 4 partial, 3 unsupported, 0 rejected by decision, 4 not tested, 1 not applicable.

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
| 012 | `'IMAGE` and `TO_STRING` for composite types | partial | ok/ok/ok/ok/ok (output differs) | reject | ok/ok/ok/reject/ok (output differs) | ok/ok/ok/ok/ok (output differs) | `TO_STRING` of an array of scalars works; type and object `'IMAGE` of arrays and records is an ICE; `TO_STRING` of a record fails at run time (#360). Cells list the five probes in order |
| 014 | Composites of protected types | partial | reject | reject | reject | ok (output differs) | analyzes; first method call is a null dereference at run time (#361) |
| 014a | Access types to composites of protected types | partial | reject | reject | reject | ok (output differs) | as 014 (#361) |
| 015 | Report the calling path | supported | crash | reject | ok | crash | `get_call_path` |
| 015a | Report the current file name | supported | ok | reject | ok | ok | `file_name`, `file_line` |
| 016 | Anonymous types | unsupported | reject | reject | reject | reject | `E0002` on `array (..) of` in an object declaration; no reference accepts the form I tried, so the intended syntax is unverified. No issue filed |
| 018 | Attributes for enumerated types | not tested | - | - | - | - | the exact attribute set could not be identified without the LCS text |
| 018a | `'INDEX` | supported | ok | reject | reject | ok |  |
| 018d | `'DESIGNATED_SUBTYPE` | supported | ok | reject | reject | ok |  |
| 019 | Constraints from initial values of signals and variables | supported | ok | reject | reject | ok |  |
| 023 | Relax library requirement on configurations | not tested | ok | ok | ok | ok | only a plain configuration analyzes in all; the relaxed form was not identified |
| 026c | Long integers | supported | ok | ok (output differs) | fatal | fatal | 64-bit integer range; `integer'high` is 2**63-1 in yavs, 2**31-1 in sim2; sim3 and sim4 stop at the 62-bit type |
| 030 | Garbage collection | not applicable | - | - | - | - | implementation property, no source syntax; `deallocate` and heap reuse are covered by the access type tests |
| 032 | `PATH_NAME` and `INSTANCE_NAME` with protected types | supported | ok/ok (output differs) | reject/ok | ok/ok (output differs) | ok | `pt'path_name` inside a method: yavs and two references give `:top:pt`, one gives `:top:sv:`, one `:top:sv:pt:` (not catalogued yet) |
| 033 | Composition with protected types | supported | ok | ok | ok | ok | a protected variable inside a protected body |
| 034 | Protected types with a generic clause | partial | reject | reject | reject | ok | the declaration and body analyze (opaque generics); `type t is new p.box generic map (..)` is `E0002` (#29) |
| 036a | Conditional expressions in declarations | supported | ok | reject | ok | ok |  |
| 041 | Record introspection and type reflection | supported | ok | reject | reject | ok (output differs) | `integer'reflect.simple_name` is `INTEGER` in yavs and sim1, `integer` in sim4 (sim2 and sim3 reject the probe; not catalogued) |
| 043 | Attributes for PSL | not tested | - | - | - | - | PSL is out of scope of this page |
| 045a | Mode view declarations | supported | ok | ok/fatal/ok | ok | ok | view ports, signal parameters of a view, an array of a viewed record; gaps in #254, #265 |
| 045c | `'CONVERSE` of a mode view | supported | ok | ok | ok | ok |  |
| 047 | Variable ports of protected type | supported | ok | reject | reject | ok |  |
| 049 | Generic map on a subprogram call | unsupported | ok | reject | reject | ok | `E9999` (#362); subprogram instantiation works |
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
| 099 | Extended ranges | not tested | ok | reject | reject | ok | scope unclear; a plain range loop works everywhere |
| I03 | Signatures in the formal part of a generic map | unsupported | ok | reject | reject | ok | `E9999` (#363) |
| I13 | Precedence of unary operators | supported | ok | ok | ok | ok | only `-k ** 2` and `not` against `=` and `and` probed; weak test |

## Gaps by use in practice

Signal: number of files in the local sources of two verification libraries and one external test runner
(in that order) that use the construct (a rough text count, not a parse). Mode views appear in 26 and 10
files of the libraries; call path and file name functions in 17, 69 and 38 files; `to_string` in 246, 149
and 73; conditional return in 5, 4 and 6.
Generic protected types, protected arrays, a generic map on a call and signatures in a generic map do not
occur in those sources.

| Rank | Gap | Issue |
|---|---|---|
| 1 | `'IMAGE` and `TO_STRING` of arrays and records (LCS-012) | #360 |
| 2 | Generic protected type instantiation (LCS-034) | #29 |
| 3 | Generic map on a subprogram call (LCS-049) | #362 |
| 4 | Arrays and records of protected types (LCS-014, 014a) | #361 |
| 5 | Signature in a generic map formal (LCS-I03) | #363 |
| 6 | Anonymous types (LCS-016): intended syntax unverified, no reference accepts the probe | none |

## Differences found

- `pt'path_name` inside a protected method: four different answers across yavs and the references
  (see row 032). The LRM text for the path of a protected object was not settled by this audit.
- `integer'reflect.simple_name` for `integer`: upper case in yavs, lower case in sim4.
- `to_string` of a record: element quoting differs between references (`(3,'1')` against `(3,1)`).
- Long integers (LCS-026c): sim2 keeps a 32-bit `integer'high`; sim3 and sim4 stop on a 62-bit type.
- Directory functions: sim3 reports `is not supported` warnings and returns false.
- Mode views, conditional compilation, `return .. when`, `open` slices and the std.env additions are
  accepted by sim1 and sim4 and by yavs; sim2 rejects most of them and sim3 rejects the generic and typing additions (see the matrix).
