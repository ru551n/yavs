# Language support

Which parts of the VHDL and PSL languages yavs accepts and runs. For the command line see [`usage.rst`](usage.rst); the choices yavs
makes where the language reference is open are in [`language-choices.rst`](language-choices.rst).

VHDL-1993 and VHDL-2008 are supported as the language reference defines them (`--std=93`, `--std=08`); files of different standards can be mixed in one project.
The rest of this page lists what is narrower than the full language.

## PSL

yavs runs a subset of PSL as checker processes, written as VHDL-2008 statements in an architecture, block or generate statement or,
with `--psl`, in comments that start with `-- psl`. The syntax and the semantics are described in the "PSL assertions" section of [`usage.rst`](usage.rst);
the tests are in `tests/corpus/psl` and `tests/cli`.

Supported:

- `default clock is <condition>;` and an `@<condition>` clock on a directive; `report` and `severity` clauses.
- Directives `assert`, `assume` and `cover`, with a property on VHDL conditions (`boolean`, `bit` or `std_ulogic`).
- Properties: `always`, `never`, `->`, `<->` and `iff`, `and` and `or` of properties, `next`, `next[n]`, `next_a`, `next_e`, `next_event`, `next_event_a`, `next_event_e`,
  `until`, `until_`, `before`, `before_`, `abort`, `sync_abort` and `async_abort` (checked at the clock tick, not between ticks), and a property on the right of `|->` and `|=>`.
  These are weak: an obligation still open at the end of the run is not a failure.
- Sequences in braces: concatenation `;`, fusion `:`, `|`, `&&`, `&`, `within`, and the repetitions `[*]`, `[+]`, `[*n]`, `[*m to n]`, `[*m to inf]`, `[=n]` and `[->n]`;
  in `cover`, in `assert never`, and on either side of `|->` and `|=>`.
- Built-in functions `prev`, `stable`, `rose`, `fell`, `isunknown`, `countones`, `onehot` and `onehot0` inside the conditions.
- Named `sequence` and `property` declarations with `boolean`, `const`, `sequence` and `property` formals, used where a sequence or a whole property is expected.

Skipped with warning `W0004` (once per design unit; the rest of the design runs as usual, and `-Werror=W0004` makes it an error):

- the strong forms (`eventually!`, `next!`, `until!`, `before!`, a sequence followed by `!`);
- other `property` and `sequence` declarations, the `next` and `ended` functions, directives without a clock, and verification units (`vunit`, `vmode`, `vprop`);
- a directive that tests more than 6 different conditions on the right side of `|->` or `|=>`.

Skipped without a warning, as they have nothing to check in a simulation: `restrict`, `restrict_guarantee`, `assume_guarantee`, `fairness`, `strong fairness`
and directives that call `nondet` or `nondet_vector`.

## VHDL-2019

Status per language change specification (LCS-2016) of the approved VHDL-2019 list, as checked with a small probe per feature (`--std=2019`) on 2026-10-19, with the changes made since
(the audit and its per-row notes are in [`design/vhdl2019-support.md`](design/vhdl2019-support.md)).
`supported` means the probe gives the result the language reference defines; `partial` means some forms work.
Issues for gaps carry the label `v2019`.

| LCS | Feature | Status | Limits |
| --- | --- | --- | --- |
| 001 | Partially connected vectors on a port map | supported | `open` for a slice of an out port |
| 002, 004 | Protected type parameters, protected methods with access and file parameters | supported | a constant access parameter of a protected method stays an error by decision |
| 006a | File I/O and TextIO updates | supported | `swrite`, `sread`, `flush` verified; other additions not probed |
| 006c, 006e, 006f | Directory API, environment API, standard conditional analysis identifiers | supported | directory: existence and working directory queries only |
| 007 | Sequential declaration regions | supported | only name hiding in nested regions probed |
| 011 | Date and time functions | supported | |
| 012 | `'IMAGE` and `TO_STRING` for composite types | supported | arrays and records |
| 014, 014a | Composites of protected types, access types to them | supported | a record mixing protected and other elements is `E1905`; a variable port of such a composite is `E1907` (not supported) |
| 015, 015a | Report the calling path, the current file name | supported | `get_call_path`, `file_name`, `file_line` |
| 016 | Anonymous types (`type is private`, `type is <>`, `type is range <>` in interface lists) | supported | |
| 018, 018a, 018d | Attributes for enumerated types, `'INDEX`, `'DESIGNATED_SUBTYPE` | supported | |
| 019 | Constraints from initial values of signals and variables | supported | |
| 023 | Relax library requirement on configurations | supported | |
| 026c | Long integers | supported | 64-bit `integer` range |
| 030 | Garbage collection | not applicable | implementation property; `deallocate` and heap reuse are tested with the access types |
| 032 | `PATH_NAME` and `INSTANCE_NAME` with protected types | supported | |
| 033 | Composition with protected types | supported | a protected variable inside a protected body |
| 034 | Protected types with a generic clause | partial | instantiation with a generic map works (`type t is new p.box generic map (..)`); a generic map in the protected type header is not implemented |
| 036a | Conditional expressions in declarations | supported | |
| 041 | Record introspection and type reflection (`std.reflection`, `'reflect`) | supported | tests in `tests/corpus/reflect` |
| 043 | Attributes for PSL | not tested | |
| 045a, 045c | Mode view declarations, `'CONVERSE` of a mode view | supported | view ports, signal parameters of a view, an array of a viewed record, individual association |
| 047 | Variable ports of protected type | supported | |
| 049 | Generic map on a subprogram call | supported | also expanded names and concurrent calls |
| 050 | Assert API | supported | `GetVhdlAssertCount`, `IsVhdlAssertFailed`, `ClearVhdlAssert` |
| 055a | Syntax regularisation: component declarations | supported | |
| 059, 059a | Array type generics and the generic type classes, ordering on any scalar array | supported | all generic type class forms analyse |
| 061 | Conditional compilation (`` `if ``, `` `elsif ``, `` `else ``, `` `end if ``) | supported | |
| 071a | Optional trailing semicolon | supported | |
| 072b | Function knows the vector size (`return rv_t of bit_vector`) | supported | |
| 075 | Closely related record types | supported | conversion between records of the same element names and types |
| 082 | Empty record | supported | |
| 086 | All interface lists can be ordered | supported | later items refer to earlier ones through `'subtype` |
| 094a | Conditional return | supported | `return x when c` in functions and procedures |
| 099 | Extended ranges | partial | `R'RECORD` and `R'VALUE` work; a constant of a range record type works as a range when its value is known at analysis (`E1410` otherwise) |
| I03 | Signatures in the formal part of a generic map | supported | |
| I13 | Precedence of unary operators | supported | only `-k ** 2` and `not` against `=` and `and` probed |
| none | Generic subprograms and generic types (`generic (type t; function f ...)`) | supported | see 016, 049, 059, I03 |
| none | `T'VALUE` of an array or record type | supported | implemented in commit `46c42d35` (#609); test `tests/corpus/composite/value_composite_2019` |

Not verified here: LCS-043 (not tested), the weak probes marked in the table (006a, 007, I13), and whether the list covers every change of the published 2019 standard
(the list is the approved LCS-2016 set; the audit could not reach the LCS pages and inferred the scope of each from its title).
