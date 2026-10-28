# Language support

**In short:** yavs runs VHDL-1993, VHDL-2008 and VHDL-2019 as the language reference defines them,
and files of different standards can be mixed in one project. Of VHDL-2019 every approved language
addition works except two that work in part and one that was not checked. PSL (the property language used for assertions) is
supported as a subset. Plain Verilog is planned ([#776](https://github.com/ru551n/yavs/issues/776)).

How to select the standard and use PSL: [`usage.rst`](usage.rst). Where the language reference leaves
the behaviour open and what yavs chooses: [`language-choices.rst`](language-choices.rst).

## VHDL-1993 and VHDL-2008

Supported as defined (`--std=93`, `--std=08`).

## VHDL-2019

Each approved language addition ("LCS" is the number of its change specification), checked with a small test
program per feature (`--std=2019`, 2026-10-19, with the fixes made since). The detailed audit is in
[`design/vhdl2019-support.md`](design/vhdl2019-support.md).

**Supported:**

- Partially connected vectors on a port map (001)
- Protected type parameters and protected methods with access and file parameters (002, 004)
- File I/O and TextIO updates (006a), directory API (006c), environment API (006e), standard conditional analysis identifiers (006f)
- Sequential declaration regions (007)
- Date and time functions (011)
- `'IMAGE` and `TO_STRING` of arrays and records (012)
- Composites of protected types and access types to them (014, 014a)
- Calling path and current file name (015, 015a)
- Anonymous types in interface lists (016)
- Attributes for enumerated types, `'INDEX`, `'DESIGNATED_SUBTYPE` (018, 018a, 018d)
- Constraints from initial values (019)
- Configurations of entities in other libraries (023)
- 64-bit `integer` (026c)
- `PATH_NAME` and `INSTANCE_NAME` with protected types (032)
- Composition with protected types (033)
- Conditional expressions in declarations (036a)
- Reflection, `std.reflection` and `'reflect` (041)
- Variable ports of protected type (047)
- Generic map on a subprogram call (049)
- Assert API (050)
- `end;` ending a component declaration (055a)
- Generic types, array type generics and ordering on any scalar array (059, 059a)
- Conditional compilation, `` `if `` ... `` `end if `` (061)
- Optional trailing semicolon (071a)
- `return rv_t of bit_vector` (072b)
- Closely related record types (075)
- Empty record (082)
- All interface lists can be ordered (086)
- Conditional return, `return x when c` (094a)
- Mode views and `'CONVERSE` (045a, 045c)
- Signatures in the formal part of a generic map (I03)
- Precedence of unary operators (I13)
- `T'VALUE` of an array or record type

**Partly supported:**

- Protected types with a generic clause (034): instantiation with a generic map works, a generic map in the protected type header does not.
- Extended ranges (099): `R'RECORD` and `R'VALUE` work, and a constant of a range record type works as a range when its value is known at analysis.

**Not applicable:** garbage collection (030) is not a language feature; `deallocate` works.

**Not checked:** attributes for PSL (043).

Limits worth knowing:

- A variable port of a composite of protected types is rejected (error `E1907`).
- A constant access parameter of a protected method is rejected on purpose.
- File I/O, directory and conditional-analysis rows were checked on the main functions only, and the
  precedence of unary operators on `-k ** 2` and `not` only.

## PSL

yavs runs a subset of PSL as checkers written as VHDL-2008 statements in an architecture, block or
generate statement or, with `--psl`, in comments that start with `-- psl`. Syntax and details:
"PSL assertions" in [`usage.rst`](usage.rst).

| Area | Supported |
| --- | --- |
| Directives | `assert`, `assume`, `cover`, with `report` and `severity` |
| Clock | `default clock is <condition>;` or an `@<condition>` on a directive |
| Properties | `always`, `never`, `->`, `<->`, `iff`, `and`, `or`, `next` (also `next[n]`), `next_a`, `next_e`, `next_event` (and `_a`, `_e`), `until`, `until_`, `before`, `before_`, `abort`, `sync_abort`, `async_abort`, and a property after `\|->` and `\|=>` |
| Sequences | in braces: `;`, `:`, `\|`, `&&`, `&`, `within`, repetitions `[*]`, `[+]`, `[*n]`, `[*m to n]`, `[*m to inf]`, `[=n]`, `[->n]` |
| Functions | `prev`, `stable`, `rose`, `fell`, `isunknown`, `countones`, `onehot`, `onehot0` |
| Named items | `sequence` and `property` declarations with `boolean`, `const`, `sequence` and `property` formals |

Properties are checked in their weak form: an obligation still open when the simulation ends is not a failure.
`abort` and `async_abort` are checked at the clock tick, not between ticks.

**What is skipped.** A construct outside the subset is skipped with warning `W0004`, once per design
unit, and the rest of the design runs as usual (`-Werror=W0004` turns it into an error):

- the strong forms (`eventually!`, `next!`, `until!`, `before!`, a sequence followed by `!`);
- other `property` and `sequence` declarations, the `next` and `ended` functions, directives without a clock, and verification units (`vunit`, `vmode`, `vprop`);
- a directive that tests more than 6 different conditions on the right side of `|->` or `|=>`.

Skipped without a warning, because a simulation has nothing to check: `restrict`, `restrict_guarantee`,
`assume_guarantee`, `fairness`, `strong fairness` and directives that call `nondet` or `nondet_vector`.

## Details

The VHDL-2019 list is the approved LCS-2016 set. The audit could not reach the change specification
pages and inferred the scope of each addition from its title, so the list may not match every change in
the published standard. `T'VALUE` of composites was added after the audit (test
`tests/corpus/composite/value_composite_2019`).
