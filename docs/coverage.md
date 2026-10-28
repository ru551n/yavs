# Code coverage

yavs counts how often your design's code ran and writes the counts to a coverage database.
Each run writes one database; you merge them and report on the result. Without `--cov` there
are no counters and no cost.

## Kinds

| Kind | What is counted |
|------|-----------------|
| `statement` | every sequential and concurrent statement, each time it runs (declarations are not counted) |
| `branch` | each arm of `if`/`elsif`/`else` (an `if` without `else` has a "no condition held" arm), each `case` alternative, each choice of a conditional or selected assignment, and the body of a `for`/`while` loop per iteration |
| `toggle` | for every `bit` and `std_ulogic`/`std_logic` scalar of every signal and port, the 0 to 1 and 1 to 0 changes; `'L'` is 0, `'H'` is 1, other values are neither, so a change through `'Z'` or `'X'` counts nothing |
| `condition` | focused expression coverage: for every `if`/`elsif`, conditional assignment, `while`, `exit when` or `next when` with two or more inputs, one row per input and value |
| `expression` | the same for the logic expression assigned to a signal or variable |

An input of a condition or expression is covered at value `v` when some evaluation flipped it
alone and the result changed. Short-circuited inputs count as masked. Not measured: `assert` and
`wait until` conditions, vector operands, and trees of more than 31 inputs. The exact rules are
in `docs/usage.rst`, section "Code coverage".

Code is counted per instance and shown under its path (`:tb:u_dut`); package code once under
`library.package`. The `std` and `ieee` libraries are not counted. Items are identified by their
path and the text of their statement, not their position, so databases stay mergeable after
edits elsewhere in the file.

A process that does nothing unless a clock edge happens runs only at that edge, with or without
`--cov`. Its edge test therefore counts one run at initialisation plus one per edge.

## Flags (`yavs run`)

| Flag | Meaning |
|------|---------|
| `--cov[=FILE]` | turn coverage on and write the database to `FILE` (default `yavs.ycov`, in the working directory) at the end of the run, whatever its outcome; an elaboration failure writes none. The file is overwritten, never appended to. Write the `=` form: `--cov FILE` does not take the file |
| `--cov-kind=LIST` | comma separated kinds from the table above; default `statement,branch,toggle`. `condition` and `expression` add instructions to every evaluation of a multi-input condition, so ask for them |
| `--cov-toggle-max=N` | leave signals with more than `N` scalars (default 65536, for example large memories) out of toggle coverage; warning `W0115` |
| `--cov-test=NAME` | the name of this run's test in the database; default is the enabled test case of a `runner_cfg` generic, else the top level with its `-g` generics, such as `tb(n=0)` |
| `--cov-exclude-translate-off` | also leave `translate_off` ... `translate_on` regions out |

`--cov` cannot be combined with `--dap` yet.

## The database

The `.ycov` file is yavs's own text format with a version line; it may change between yavs
versions. Tests that ran with the same name count as one test, and the database records which
tests hit each item.

## Merge

```
yavs cov merge a.ycov b.ycov -o all.ycov
```

adds up the counts of the runs and keeps, for every item, the tests that hit it.

## Report

```
yavs cov report all.ycov [--cobertura FILE.xml] [--html DIR [--root DIR]]
                         [--exclude FILE] [--uncovered[=yaml] | --tests] [-W...]
```

Several databases are merged first. With no other option it prints totals for each kind that
the database holds, then one line per instance or package, then the excluded items with their
reasons.

* **Text** (default): totals such as `statements 18/24 75.0%`, then the per-path table.
* **`--cobertura FILE.xml`**: Cobertura XML for CI services, one entry per source line. Cobertura
  has no toggles, so those are only in the text and HTML reports.
* **`--html DIR`**: static pages with inline style and one inline script, no external
  resources, so `index.html` opens from disk. Missed lines are red, partly covered lines
  (ran in some instances only) purple, lines with a missed branch arm or toggle yellow. Clicking
  a line opens its items per instance with counts and the tests that hit it. Each file also has
  a `src-N-missed.html` page with only the lines that have gaps. Sources are read from the paths
  stored in the database, so run the command where they resolve (otherwise error `E0113`,
  nothing written). `--root DIR` changes which path prefix is stripped from the names shown.
  The same databases always give the same bytes.
* **`--uncovered`**: instead of the summary, one exclusion row per item that never ran, with
  file, line and the statement as a comment. `--uncovered=yaml` prints the same as a YAML
  exclusion file.
* **`--tests`**: instead of the summary, one row per item that ran, followed by `# tests:` and
  the tests that hit it. Cannot be combined with `--uncovered`.

`yavs cov` exits 0, or 1 with `E0110` (cannot write), `E0111` (cannot read a database) or
`E0112` (bad exclusion file), or when `-Werror` turns a warning into an error.

## Rank

```
yavs cov rank all.ycov [--exclude FILE]
```

Merges its inputs and prints a small set of tests that reaches the same coverage (picked
greedily: each pick is the test adding the most items not yet hit), the tests that set does
not need, and per test how many items it hit and how many no other test hit.

## Exclusions

In the source:

```vhdl
-- coverage off: debug only
debug_counter <= debug_counter + 1;
-- coverage on
x <= y; -- coverage off-line
```

`coverage off` (reason optional) starts a region whose statements are left out, `coverage on`
ends it, and `coverage off-line` leaves out the statements starting on its line.
`-- yavs coverage ...` is accepted as a synonym. A region never closed excludes the rest of the
file (`W0112`); a `coverage on` without a `coverage off` is `W0113`.

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
and `file`; `*` and `?` match in instance paths, toggle paths and signal names, and file
patterns. A statement is matched by its path and text hash, so it stays excluded when it moves; the
line is only for reading (`W0110` when it is stale). A row that matches nothing is `W0111`, an
excluded item that did run `W0114`. A file whose name ends in `.yaml` or `.yml` is read as YAML:

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

Excluded items are left out of the totals and Cobertura and listed under "excluded". The
database keeps the raw counts. The full row and key reference is in `docs/usage.rst`.

## Tiers and threads

Counts are the same in every execution tier (`--tier=interp`, `llvm`, `auto`) and at any
`--threads`. This is tested by comparing the database files byte for byte: the `tiers_agree`
test in `crates/yavs/tests/coverage.rs` runs the same design with `--tier=interp`, `llvm`,
`llvm --threads=4` and `auto` and requires identical files for all kinds; `coverage_fec.rs`
does the same for condition and expression rows, and `run_merge_report` compares one and four
threads. These tier tests only exist in builds that include the LLVM tier. A further test
(`copies_count_alike`) checks that per-instance specialised code counts like shared code.

## Worked example

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
unless you pass `--cov-test`). The report prints a line each for statements, branches, toggles
and conditions with hit/total and a percentage, then a table per instance. Open
`cov-html/index.html` to click through the missed lines. Edit `exclusions.yaml`, deleting the
entries that must stay as gaps, and pass it back with `--exclude exclusions.yaml`. `rank` tells
you whether one of the two tests adds nothing the other lacks.
