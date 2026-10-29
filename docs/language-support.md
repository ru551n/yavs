# Language support

**In short:** yavs runs VHDL-1993, VHDL-2008 and VHDL-2019 and lets files of different standards mix in one
project. Of the approved VHDL-2019 language additions most work, three work in part and one was not checked
(see the table below). PSL (the property language used for assertions) is supported as a subset. Plain Verilog is
planned ([#776](https://github.com/ru551n/yavs/issues/776)).

Which parts of VHDL and PSL yavs accepts and runs, and how well that is checked. The choices yavs makes where the
language reference is open are in [Where simulators differ](language-choices.rst).

## Choosing the standard

`--std` takes `93` (also written `1993`), `2008` or `2019`. The default for `parse` and `analyze` is `2008`. Any other
value (`87`, `02`, `08`, `2002`) is a usage error with exit status 2. VHDL-1987 and VHDL-2002 have no mode of their
own: write 1993 code with `--std=93` and 2002 code (protected types, for example) with `--std=2008`.

`--std` on `analyze` is the standard of the files of that call. `run` and `elab` read the standard recorded per
file, so a project can mix standards, and they run the whole design against one revision of `std` and `ieee`, the
newest among its files (`--std` overrides it). A 1993 file in a design that also has 2008 files is checked as a
1993 file but sees the 2008 packages, so 1993 code that overloads a subprogram on both `std_logic_vector` and
`std_ulogic_vector` fails there with the ordinary duplicate-declaration error (`E1101`).

## How to read this page

Each statement has one of these evidence levels.

| Level | Meaning |
| --- | --- |
| simulated | a regression testbench runs the feature and its output is compared with a recorded result |
| analysed | regression tests check that analysis accepts the valid form and rejects the invalid ones, but no testbench runs it |
| probed | checked by hand with a small design, no regression test yet |
| not tested | no check at all |

"Supported" means the checks give the result the language reference defines. "Partial" means some forms work and the
limits are listed. What is not implemented stops with a diagnostic that names the construct (see
"Known gaps" below); yavs does not silently run it differently.

How the evidence is gathered, and the pass counts of the test suites yavs is run against, are in the
[conformance](conformance.rst) report. Passing tests are evidence, not proof.

## VHDL-2008

VHDL-2008 is the default and the best checked revision. These features are supported and simulated unless marked
probed:

- Package and subprogram generics, generic types, formal packages, package instantiation in any declarative part.
- `fixed_pkg` and `float_pkg`, `numeric_std_unsigned`, `numeric_bit_unsigned`, the 2008 `std_logic_1164`,
  `numeric_std` and `math_real`, `to_string` and its relatives, `minimum` and `maximum`, the predefined vector types.
- External names (also as port actuals and with `force`), `force` and `release`, conditional and selected sequential
  assignments, `process (all)`, `elsif` and `case` generate, the condition operator `??`, matching `case?` and
  `select?` (probed), `rising_edge` and `falling_edge` on `boolean` (probed), context declarations (probed),
  extended bit string literals (probed), delimited comments (probed).
- Unconstrained element types, `'subtype` and `'element`, protected types, `std.env` (`stop`, `finish`,
  `resolution_limit`).
- PSL: a subset, see "PSL" below.

`--relaxed` turns a few errors that verification code often trips over into warnings: a shared variable of a type
that is not protected (`W1904`), a pure function that calls an impure one (`W1830`), an `others` aggregate assigned
to an object with an unconstrained subtype (`W1325`), and a signal parameter's actual that is not a static name
(`W1361`). Pass it to `analyze` and also to `run` and `elab`, which analyse
the files again.

## VHDL-1993

`--std=93` selects the 1993 language and the 1993 `std` and `ieee` packages. Valid 1993 code keeps its 1993 meaning:

- `std_logic_vector` is a type of its own, not a subtype of `std_ulogic_vector`, and `std_logic_1164`, `numeric_std`,
  `numeric_bit`, `std.textio` and `std_logic_textio` are the 1993 packages. A design of 1993 files alone runs against
  them (simulated).
- The words 2008 reserved (`assume`, `context`, `default`, `force`, `parameter`, `property`, `sequence`, `protected`
  and so on) are ordinary identifiers, and the names 2008 added to the predefined library (`to_string`,
  `maximum`, `integer_vector`) are free for your own declarations (simulated).
- A shared variable can have any type (simulated). `INTEGER` is 32 bits.
- Overloads on both `std_logic_vector` and `std_ulogic_vector` analyse, because they are distinct types in 1993
  (simulated).
- Reading an `out` port or parameter is an error (`E1360`, analysed).
- An explicit declaration hides an implicit one when both are visible through use clauses, as in 2008, so packages
  that redeclare predefined operators, such as `std_logic_unsigned`, analyse without a flag (simulated).
- 2008 syntax is rejected, mostly with `E0011` and otherwise as a syntax error: `process (all)`, conditional and selected assignments
  inside a process, `elsif` and `case` generate, package generics, `/* */` comments, bit string literals with a length
  or `U`, `S`, `D` bases, matching operators, external names, `force` and `release`, protected types (analysed, probed). Some 2008 constructs are accepted, see below.
- `numeric_std_unsigned`, `numeric_bit_unsigned`, `fixed_pkg`, `float_pkg`, the two contexts and `std.env` do not
  exist in 1993 mode. Naming one is an error that the unit is not found.
- PSL in a 1993 file: a `-- psl` comment is a plain comment without `--psl`. With `--psl` it is read in part (probed by hand, no regression test): `-- psl assert always true @ (clk'event);` analyzes, an assert that uses `rising_edge` fails only with `E1100` because the function is not in 1993, and `-- psl default clock is clk'event;` is a syntax error (`E0002`). Do not rely on PSL in 1993 files.

The 1993 language suite that yavs is run against at `--std=93` passes for the large majority of its tests (see the
conformance report); the failures are tracked as issues. One kind of failure matters when you rely on yavs to reject
bad code: invalid 1993 code that is still accepted. The 1993 restrictions that 2008 relaxed on aliases of
multidimensional arrays, unconstrained element types, `buffer` port associations, nested packages and array parts of
aggregates are errors in 93 mode; some other relaxed static and `case` rules, and some checks of 1993 and 2008 alike,
are still missing.

## VHDL-2019

Status per language change specification (LCS-2016) of the approved VHDL-2019 list, checked with a small design per
feature on 2026-10-19 and, where the table says so, again on 2026-10-28. Use `--std=2019`. The list is the approved
LCS set; it is not checked against every change of the published standard.

| LCS | Feature | Status | Evidence | Limits |
| --- | --- | --- | --- | --- |
| 001 | Partially connected vectors on a port map | supported | analysed | `open` for a slice of an out port |
| 002, 004 | Protected type parameters, protected methods with access and file parameters | supported | simulated | a constant access parameter of a protected method stays an error (`E1223`) by decision |
| 006a | File I/O and TextIO updates | supported | simulated | `swrite`, `sread` and `flush` run; other additions not checked |
| 006c | Directory API | supported | probed 2026-10-28 | `dir_open`, existence and kind queries, working directory, create and delete of directories work; other functions not checked |
| 006e, 006f | Environment API, standard conditional analysis identifiers | supported | probed, 006f also analysed | `getenv`, `vhdl_version`, `tool_*` |
| 007 | Sequential declaration regions | supported | probed | only name hiding in nested regions |
| 011 | Date and time functions | supported | simulated | |
| 012 | `'IMAGE` and `TO_STRING` for composite types | supported | simulated | arrays and records |
| 014, 014a | Composites of protected types, access types to them | supported | simulated | a record that mixes protected and other elements is `E1905`; a variable port of such a composite is `E1907` (not supported) |
| 015, 015a | Report the calling path, the current file name | supported | simulated | `get_call_path`, `file_name`, `file_line` |
| 016 | Anonymous types (`type is private`, `type is <>`, `type is range <>` in interface lists) | supported | simulated | |
| 018, 018a, 018d | Attributes for enumerated types, `'INDEX`, `'DESIGNATED_SUBTYPE` | supported | 018 simulated, others analysed | |
| 019 | Constraints from initial values of signals and variables | supported | simulated | |
| 023 | Relax library requirement on configurations | supported | analysed | |
| 026c | Long integers | supported | simulated | 64-bit `integer`; in 1993 and 2008 `integer` stays 32 bits |
| 030 | Garbage collection | not applicable | | implementation property, no source syntax |
| 032 | `PATH_NAME` and `INSTANCE_NAME` with protected types | supported | probed | the simulators differ on the path of a protected object |
| 033 | Composition with protected types | supported | probed | a protected variable inside a protected body |
| 034 | Protected types with a generic clause | partial | simulated | `type t is new p.box generic map (..)` works; a generic map in the protected type header is a syntax error (`E0002`) |
| 036a | Conditional expressions in declarations | supported | simulated | |
| 041 | Record introspection and type reflection (`std.reflection`, `'reflect`) | supported | simulated | |
| 043 | Attributes for PSL | not tested | not tested | |
| 045a, 045c | Mode view declarations, `'CONVERSE` of a mode view | supported | simulated | view ports, signal parameters of a view, an array of a viewed record, individual association |
| 047 | Variable ports of protected type | supported | simulated | |
| 049 | Generic map on a subprogram call | supported | simulated | also expanded names and concurrent calls |
| 050 | Assert API | partial | simulated | the counting functions (`GetVhdlAssertCount`, `IsVhdlAssertFailed`, `ClearVhdlAssert`) and `SetVhdlAssertEnable` run; `SetVhdlAssertFormat` with a non-empty format and the PSL status functions (`PslAssertFailed`, `PslIsCovered` and relatives) stop the run with a not-implemented error |
| 055a | Syntax regularisation: component declarations | supported | probed 2026-10-28 | |
| 059 | Array type generics and the generic type classes | supported | simulated | |
| 059a | Ordering on any scalar array | supported | probed 2026-10-28 | `<` on a `real_vector`; an error in 2008 |
| 061 | Conditional compilation (`` `if ``, `` `elsif ``, `` `else ``, `` `end if ``) | supported | analysed, probed 2026-10-28 | |
| 071a | Optional trailing semicolon | supported | probed 2026-10-28 | |
| 072b | Function knows the vector size (`return rv_t of bit_vector`) | supported | simulated | |
| 075 | Closely related record types | supported | simulated | conversion between records of the same element names and types |
| 082 | Empty record | supported | probed 2026-10-28 | |
| 086 | All interface lists can be ordered | supported | analysed | later items refer to earlier ones through `'subtype` |
| 094a | Conditional return | supported | simulated | `return x when c` in functions and procedures |
| 099 | Extended ranges | partial | simulated | `R'RECORD` and `R'VALUE` work; a range record object works as a range when its value is known at analysis, otherwise `E1410` |
| I03 | Signatures in the formal part of a generic map | supported | simulated | |
| I13 | Precedence of unary operators | supported | probed | only `-k ** 2` and `not` against `=` and `and` |
| none | Generic subprograms and generic types (`generic (type t; function f ...)`) | supported | simulated | |
| none | `T'VALUE` of an array or record type | supported | simulated | |

Not checked: LCS-043, the rows that are only probed, and whether the list covers every change of the published 2019
standard.

## PSL

yavs runs a subset of PSL as checker processes. A directive is written as a VHDL-2008 statement in an architecture,
block or generate statement or, with `--psl` (on `analyze` and `run`), in a comment that starts with `-- psl`:

```text
default clock is rising_edge(clk);
p1: assert always count < 16 report "count overflow";
p2: assert always req -> next[2] ack severity warning;
p3: assert always {start; req} |=> {busy[*1 to 3]; ack};
c1: cover {req; busy[*2 to 4]; done} report "transfer";
-- psl p4: assert always start -> next busy;
```

A failing `assert` or `assume` reports like a VHDL assertion, with severity `error` unless the directive gives
one; a `cover` prints its `report` text each time it is hit. The subset below is simulated.

Supported:

- `default clock is <condition>;`, an `@<condition>` clock on a directive, `report` and `severity` clauses.
- Directives `assert`, `assume` and `cover`, with a property on VHDL conditions (`boolean`, `bit` or `std_ulogic`).
- Properties: `always`, `never`, `->`, `<->` and `iff`, `and` and `or` of properties, `next`, `next[n]` (n up to 256),
  `next_a`, `next_e`, `next_event`, `next_event_a`, `next_event_e`, `until`, `until_`, `before`, `before_`, `abort`,
  `sync_abort` and `async_abort`, and a property on the right of `|->` and `|=>`. `abort` and `async_abort` are
  checked at the clock tick, not between ticks as the PSL standard defines the asynchronous abort. These properties
  are weak: an obligation still open at the end of the run is not a failure.
- Sequences in braces: concatenation `;`, fusion `:`, `|`, `&&`, `&`, `within`, and the repetitions `[*]`, `[+]`,
  `[*n]`, `[*m to n]`, `[*m to inf]`, `[=n]` and `[->n]`, in `cover`, in `assert never`, and on either side of `|->`
  and `|=>`. The operands of `&&` and `&` must each be written in braces, `{ {a; b} && {c; d} }`: with bare
  operands, `&&` is skipped (`W0004`) and `&` is read as VHDL concatenation (`E1340`).
- Built-in functions `prev`, `stable`, `rose`, `fell`, `isunknown`, `countones`, `onehot` and `onehot0`.
- Named `sequence` and `property` declarations with `boolean`, `const`, `sequence` and `property` formals.

Skipped with warning `W0004`, once per design unit, so that the rest of the design runs (`-Werror=W0004` makes it an
error). The skipped directive is not checked:

- the strong forms (`eventually!`, `next!`, `until!`, `before!`, a sequence followed by `!`);
- `not` of a temporal property, `forall`, the LTL spellings (`G`, `F`, `X`, `U`, `W`), declarations with typed formals
  such as `bit`, the `next` and `ended` functions, directives without a clock, and verification units (`vunit`,
  `vmode`, `vprop`);
- a directive that tests more than 6 different conditions on the right side of `|->` or `|=>`.

Skipped without a warning, as they have nothing to check in a simulation: `restrict`, `restrict_guarantee`,
`assume_guarantee`, `fairness`, `strong fairness` and directives that call `nondet` or `nondet_vector`.

A PSL directive written inside a process is a syntax error (`E0002`). The 2019 `std.env` functions that report PSL
state are not implemented (see LCS-050).

The warning text of `W0004` lists a smaller subset than the one above; this page is the more complete list.

## Libraries

`std` and `ieee` are built into the executable and analysed from source on demand; no other library is shipped.
For 2008 and 2019 they hold `std.standard`, `textio` and `env` (2019 adds `reflection`), and in `ieee`
`std_logic_1164`, `std_logic_textio`, `numeric_std`, `numeric_std_unsigned`, `numeric_bit`, `numeric_bit_unsigned`,
`math_real`, `math_complex`, `fixed_pkg` and `float_pkg` with their generic packages, the VITAL packages, the two
context declarations, and the non-standard Synopsys packages `std_logic_arith`, `std_logic_unsigned`,
`std_logic_signed` and `std_logic_misc`. The Synopsys packages are visible without a flag so that existing designs
that use them analyse and simulate; new code should prefer `numeric_std`. The 1993 libraries are the subset listed
under VHDL-1993 above. A design uses one revision of `std` and `ieee`, so `std` and `ieee` cannot be mapped to a
library of your own.

The `ieee` packages run from their VHDL bodies; a few hot subprograms also run as native fast paths with the same
results. Regression testbenches cover every package body at 2008 and 2019, and the Synopsys and VITAL packages
have testbenches of their own. 1993 mode has a smaller set of
testbenches for `numeric_std`, `std_logic_textio`, `std.textio` and the Synopsys packages.

## Simulation semantics

The simulation cycle follows the language reference: delta cycles, inertial and transport delay, resolved and guarded
signals, `postponed` processes, `force` and `release`, the signal attributes, shared variables, files, access types
with heap reclamation, and external names are simulated by the regression testbenches. Time is a 64-bit count of
femtoseconds by default; `--resolution` raises the limit and makes finer units an error. `integer` is 32 bits in
1993 and 2008 and 64 bits in 2019. A run stops with `E3002` after `--max-deltas` delta cycles (default 10000) at
one time.

Where the standard leaves the order of processes within a cycle open, yavs fixes it, the same at any `--threads`
(see [Where simulators differ](language-choices.rst)). `--shuffle` runs another order to find testbenches that depend
on it.

Mixed-language designs are not supported. Foreign subprograms (`VHPIDIRECT`) and VHPI plugins are described in
[VHPI](vhpi.md).

## Known gaps

Not implemented, so the run or the analysis stops with an error that names the construct (usually `E2001` or
`E3009` at run time, or a "not implemented" message from analysis):

- `'driving` and `'driving_value` of signal parameters and of record parts of a signal (`E2001`).
- Attributes of a range attribute other than `'left`, `'right`, `'low`, `'high` and `'ascending`,
  such as `t'range'length` or any attribute of `t'reverse_range`. These are reported as not
  implemented (`E1999`), with a note that says it is a limitation of yavs.
- `std.env` PSL state functions and non-empty `SetVhdlAssertFormat` (see LCS-050).
- A generic map in the header of a protected type (LCS-034) and range records known only at run time (LCS-099).
- PSL outside the subset above.

Invalid code that is accepted, or reported with a misleading message, in some cases: a variable that is not
shared in an entity, architecture or block declarative part, several subprogram, aggregate, deferred constant and
type checks, and the 1993 restrictions that 2008 relaxed. These are tracked as bugs in the issue tracker of the
[repository](https://github.com/ru551n/yavs/issues); the label `v2019` marks 2019 gaps.

A file declared in a package with a file open clause (`file f : text open read_mode is "x.txt";` in a package) is
not opened when the package is elaborated, and the first access fails with `E3009` "file is not open". Declare the
file in an architecture, or open it with `file_open`. This is a bug.
