# Code coverage

Coverage tells you which parts of your design's code ran during a simulation. It is a
two-step workflow:

1. **Run** with `yavs run --cov`. Each run writes one database file (`.ycov`) with the counts.
2. **Report** with `yavs cov`: merge the databases of several runs, then print, export or rank.

You get a `.ycov` database per run, and from it a text summary, an HTML site, Cobertura XML
for CI, or a list of items that never ran. Without `--cov` there are no counters and no cost.

## Example

```
yavs analyze --work=lib:out/lib rtl/*.vhd tb/tb.vhd
yavs run --work=lib:out/lib --cov=a.ycov --cov-kind=statement,branch,toggle,condition tb -gtest=a
yavs run --work=lib:out/lib --cov=b.ycov --cov-kind=statement,branch,toggle,condition tb -gtest=b
yavs cov merge a.ycov b.ycov -o all.ycov
yavs cov report all.ycov --html cov-html --cobertura coverage.xml
yavs cov report all.ycov --uncovered=yaml > exclusions.yaml
yavs cov rank all.ycov
```

The two runs write `a.ycov` and `b.ycov` (named `tb(test=a)` and `tb(test=b)` in the database,
unless you pass `--cov-test`). The report prints one line each for statements, branches,
toggles and conditions with hit/total and a percentage, then a table per instance. Open
`cov-html/index.html` to click through the missed lines. Edit `exclusions.yaml`, deleting the
entries that must stay as gaps, and pass it back with `--exclude exclusions.yaml`. `rank`
tells you whether one of the two tests adds nothing the other lacks.

## What is counted

| Kind | What is counted |
|------|-----------------|
| `statement` | every sequential and concurrent statement, each time it runs (declarations are not counted) |
| `branch` | each arm of `if`/`elsif`/`else` (an `if` without `else` has a "no condition held" arm), each `case` alternative, each choice of a conditional or selected assignment, and the body of a `for`/`while` loop per iteration |
| `toggle` | for every `bit` and `std_ulogic`/`std_logic` scalar of every signal and port, the 0 to 1 and 1 to 0 changes |
| `condition` | for every `if`/`elsif`, conditional assignment, `while`, `exit when` or `next when` with two or more inputs, one row per input and value |
| `expression` | the same for the logic expression assigned to a signal or variable |

## Flags for `yavs run`

| Flag | Meaning |
|------|---------|
| `--cov[=FILE]` | turn coverage on; write the database to `FILE` (default `yavs.ycov` in the working directory). Write the `=` form: `--cov FILE` does not take the file |
| `--cov-kind=LIST` | comma separated kinds from the table above; default `statement,branch,toggle` |
| `--cov-toggle-max=N` | leave signals with more than `N` scalars (default 65536, for example large memories) out of toggle coverage |
| `--cov-test=NAME` | the name of this run's test in the database |
| `--cov-exclude-translate-off` | also leave `translate_off` ... `translate_on` regions out |

`--cov` cannot be combined with `--dap` yet: the run stops before time 0 with a "not implemented" error (exit 2).

## Commands

| Command | What it does |
|---------|--------------|
| `yavs cov merge a.ycov b.ycov -o all.ycov` | adds up the counts of the runs and keeps, for every item, the tests that hit it |
| `yavs cov report all.ycov [options]` | merges its inputs, then reports (options below) |
| `yavs cov rank all.ycov [--exclude FILE]` | prints a small set of tests that reaches the same coverage, the tests that set does not need, and per test how many items it hit and how many no other test hit |

Report options:

| Option | Output |
|--------|--------|
| (none) | totals such as `statements 18/24 75.0%` for each kind the database holds, one line per instance or package, then the excluded items with their reasons |
| `--cobertura FILE.xml` | Cobertura XML for CI services, one entry per source line (it has no toggles, so those are only in the text and HTML reports) |
| `--html DIR [--root DIR]` | static pages that open from disk (`index.html`); `--root` changes which path prefix is stripped from the names shown |
| `--uncovered[=yaml]` | one exclusion row per item that never ran, with file, line and the statement as a comment; `=yaml` prints a YAML exclusion file |
| `--tests` | one row per item that ran, followed by `# tests:` and the tests that hit it; cannot be combined with `--uncovered` |
| `--exclude FILE` | apply an exclusion file (see Exclusions) |

## Excluding code

In the source:

```vhdl
-- coverage off: debug only
debug_counter <= debug_counter + 1;
-- coverage on
x <= y; -- coverage off-line
```

`coverage off` (reason optional) starts a region whose statements are left out, `coverage on`
ends it, and `coverage off-line` leaves out the statements starting on its line.
`-- yavs coverage ...` is accepted as a synonym.

Outside the source, give `yavs cov report` or `yavs cov rank` an `--exclude FILE`. Copy rows
from `--uncovered`, optionally with a reason after `#`:

```
branch :tb:dut:u_fsm  c41e09a7b3d2.0.3  src/fsm.vhd:88   # unreachable: state is one-hot
toggle :tb:dut data(*) fall
instance :tb:u_debug*                                    # debug logic
file */generated/*.vhd
unit work.pkg_tools
```

Row types are `statement`, `branch`, `condition`, `expression`, `toggle`, `instance`, `unit`
and `file`; `*` and `?` match in instance paths, toggle paths, signal names and file
patterns. A file whose name ends in `.yaml` or `.yml` is read as YAML:

```yaml
exclude:
  - kind: branch
    instance: ":tb:dut:u_fsm"
    hash: "c41e09a7b3d2.0.3"
    reason: unreachable, state is one-hot
  - kind: toggle
    instance: ":tb:dut"
    signal: "data(*)"
    direction: fall        # optional: rise or fall, default both
  - kind: instance
    instance: ":tb:u_debug*"
```

Excluded items are left out of the totals and Cobertura and listed under "excluded" with their
reasons; the database keeps the raw counts. Rows match by instance, kind and the hash of the
statement's text, so an exclusion stays valid when the statement moves; the file and line are only
there to read (a stale line is warning `W0110`). `yavs cov report --uncovered DB` and
`--uncovered=yaml` print rows ready to copy:

| Row | Fields |
|-----|--------|
| `statement`, `branch` | `INSTANCE HASH [FILE:LINE]` (YAML: `instance`, `hash`, `line`) |
| `condition`, `expression` | `INSTANCE HEX12.OCC ROW` (YAML: plus `row: N`) |
| `toggle` | `PATH SCALAR [rise\|fall]` (YAML: `instance`, `signal`, `direction`) |
| `instance` | an instance path and everything below it |
| `unit` | a package, such as `work.pkg_tools` |
| `file` | a file glob, such as `*/generated/*.vhd` |

In YAML, quote values that start with `*` or contain `: `.

## Which tests cover what

Each database names its test: `--cov-test=NAME`, by default the enabled test case of a
`runner_cfg` generic, else the top level with its `-g` generics (`tb(n=0)`). `yavs cov merge`
keeps, for every item, the tests that hit it. `yavs cov rank` prints a small set of tests that
reaches the same coverage and the tests that set does not need; `yavs cov report --tests` lists
the tests that hit each item.

## Details

**Where counts go.** Code is counted per instance and shown under its path (`:tb:u_dut`);
package code once under `library.package`. The `std` and `ieee` libraries are not counted.
Items are identified by their path and the text of their statement, not their position, so
databases stay mergeable after edits elsewhere in the file. A statement exclusion works the same
way: it is matched by path and text hash, so it stays excluded when the statement moves, and the
line number in the row is only for reading.

**Toggle values.** `'L'` counts as 0 and `'H'` as 1; other values are neither, so a change
through `'Z'` or `'X'` counts nothing.

**Condition and expression coverage.** An input is covered at value `v` when some evaluation
flipped it alone and the result changed. In `if a and (b or c)` with `a` true, `b` false and `c`
true, flipping `a` or `c` alone changes the result and flipping `b` does not: that evaluation covers
`a = 1` and `c = 1`. Short-circuited inputs count as masked. Not measured: `assert` and `wait until`
conditions, vector operands, and trees of more than 31 inputs. These two kinds cost time in every
evaluation of a multi-input condition, so ask for them only when you need them.

**Clocked processes.** A process that does nothing unless a clock edge happens runs only at
that edge, with or without `--cov`. Its edge test therefore counts one run at initialisation
plus one per edge.

**The database.** The `.ycov` file is written at the end of the run whatever its outcome, and
overwritten, never appended to. Its format may change between yavs versions.

**HTML report.** Static pages that open from disk. Missed lines are red, lines that ran in only
some instances purple, lines with a missed branch arm or toggle yellow; click a line to see its
items per instance, with counts and the tests that hit them. Each file also has a page with only
its gaps. Sources are read from the paths stored in the database, so run the command where they
resolve.

**Exit status and diagnostics.** `yavs cov` exits 0, or 1 on an error or when `-Werror` turns a
warning into an error.

| Code | Meaning |
|------|---------|
| `E0110` | cannot write the output |
| `E0111` | cannot read a database |
| `E0112` | bad exclusion file |
| `E0113` | HTML report cannot find a source file at its stored path; nothing is written |
| `W0110` | an exclusion row's line number is stale (the statement is still matched by its hash) |
| `W0111` | an exclusion row matches nothing |
| `W0112` | a `coverage off` region is never closed, so it excludes the rest of the file |
| `W0113` | a `coverage on` without a preceding `coverage off` |
| `W0114` | an excluded item did run |
| `W0115` | a signal was left out of toggle coverage because of `--cov-toggle-max` |

**Tiers and threads.** Counts are the same in every tier and at any `--threads`.
