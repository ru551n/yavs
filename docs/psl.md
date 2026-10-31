# PSL

PSL is the property language used for assertions. This page lists the subset yavs runs, as checker processes,
and what it skips. The exact behaviour where simulators differ is in [Where simulators differ](language-choices.rst).

## Writing a directive

A directive is written as a VHDL-2008 statement in an architecture,
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

## What is supported

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

## What is skipped with a warning

Skipped with warning `W0004`, once per design unit, so that the rest of the design runs (`-Werror=W0004` makes it an
error). The skipped directive is not checked:

- the strong forms (`eventually!`, `next!`, `until!`, `before!`, a sequence followed by `!`);
- `not` of a temporal property, `forall`, the LTL spellings (`G`, `F`, `X`, `U`, `W`), declarations with typed formals
  such as `bit`, the `next` and `ended` functions, directives without a clock, and verification units (`vunit`,
  `vmode`, `vprop`);
- a directive that tests more than 6 different conditions on the right side of `|->` or `|=>`.

## What is skipped without a warning

Skipped without a warning, as they have nothing to check in a simulation: `restrict`, `restrict_guarantee`,
`assume_guarantee`, `fairness`, `strong fairness` and directives that call `nondet` or `nondet_vector`.

## Errors and limits

A PSL directive written inside a process is a syntax error (`E0002`). The 2019 `std.env` functions that report PSL
state are not implemented (see LCS-050 in [Language support](language-support.md)).

The warning text of `W0004` lists a smaller subset than the one above; this page is the more complete list.

## See also

- [Language support](language-support.md): VHDL-2008 and VHDL-2019.
- [Where simulators differ](language-choices.rst): PSL in comments and `--psl`.
- [Error codes](error-codes.rst): `W0004` and `E0002`.
