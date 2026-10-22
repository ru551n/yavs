# Negative semantic-analysis suite (`tests/diag/sem`)

Small, focused VHDL inputs that `yavs-sem` must reject (or warn about), plus some legal "near-miss" files that must stay silent. Written for milestone M1 from `docs/design/sem.md` (section 8 codes, cascade suppression, help patterns, `--relaxed`) and checked against the reference simulators (`SIM1.md`). Nothing in here is copied from the reference simulators.

Layout: `<area>/<name>.vhd`. Areas follow sem.md:

| Dir | Area |
|---|---|
| `vis` | visibility, lookup, expanded names, hiding |
| `hom` | homographs and redeclaration, legal overloading |
| `use` | use clauses, library/context clauses, potential visibility |
| `unit` | design units, binding, components, configurations |
| `typ` | types, subtypes, constraints, incomplete/file/resolution |
| `acc` | access types, allocators, dereference |
| `stat` | static evaluation, ranges, overflow, static length and index checks |
| `ovl` | overload resolution, ambiguity, associations, operators |
| `agg` | aggregates, choices, case coverage |
| `lit` | literals |
| `expr` | assignments, conditions, conversions, class/mode, statement placement |
| `gen` | generics (packages, types, subprograms) |
| `drv` | drivers, signals in subprograms, sensitivity, signal attributes |
| `prot` | protected types |
| `alias`, `attr`, `sig`, `ext` | aliases, attributes, signatures, external names |
| `v2019` | VHDL-2019: views, private variables, generic type classes, conditional expressions |
| `mix` | mixed 2008/2019 (`E1007`, `W1008`) |
| `relax` | `--relaxed` pairs |
| `fuzz` | minimised fuzzer finds (regression tests) |

## Header format

Every file starts with a machine-readable header of `--` comments, then the VHDL:

```
-- yavs-test: std=2008|2019 [lib=NAME] [flags=A,B]
-- expect: <CODE> <line>:<col> <short reason>      (one per expected diagnostic, or `-- expect: none`)
-- help: <fragment of expected help text>          (optional, zero or more)
-- LRM <clause>                                    (informational)
```

- `std`: revision the file is parsed as (the file's own revision `F` in sem.md 0.1).
- `lib`: logical library the file is analyzed into (default `work`). Unused so far.
- `flags`: comma-separated yavs flags. Used: `--relaxed`, and the test-only pseudo-flag `--design-std=2019`, meaning "the design revision `D` is 2019 because another unit in the closure is 2019" (the file itself is still `std`). The harness maps it to whatever the manifest/closure does; the sem.md 0.1 mixed-revision tests need it.
- `expect` lists **every** diagnostic, warnings included, and nothing else: cascades must not appear (sem.md 8.2). Order is source order. `line:col` is 1-based, in characters, counted in the file including the header lines. The position is the **start of the primary span**. The spans are the test author's reading of sem.md, not measured from yavs-sem: for an operator application the start of the whole expression, for a call the start of the callee name, for a missing actual the start of the call or instance, for a method-without-body the declaration of the method. If yavs-sem deliberately uses another span, fix the test by moving the position; the code and the rest stay.
- `help` fragments must occur in the rendered `help:` line of some expected diagnostic of the file.
- Codes come from sem.md 8.1. Where 8.1 has none, the tests use a **proposed** code listed in `CODES.md`.

A file is clean (`-- expect: none`) when it is legal and yavs-sem must report nothing. These are the near-miss guards (`*_ok.vhd` and a few others): legal overloading, legal hiding, deferred constants, and so on.

## How the yavs-sem harness consumes it

1. Collect `tests/diag/sem/**/*.vhd`. For each file read the header, analyze it with `std`, the `flags` and the `lib`, in a fresh `Db` with std/ieee loaded.
2. Compare the produced diagnostics with the `expect` lines **exactly**: same set of `(code, line, col)`, no extra and no missing diagnostics (this is what checks cascade suppression), warnings included. Reason text is not compared.
3. Check each `help:` fragment against the rendered help of the file's diagnostics (substring).
4. A file with `expect: none` must produce no diagnostics at all.
5. Rendered-text snapshots: a file with a `<name>.stderr` next to it must also render exactly that (human format, path relative to the repository). Create an empty `.stderr` and run `YAVS_BLESS=1 cargo test -p yavs-sem --test diag_sem` to fill it; bless rewrites only existing snapshots. The headers stay the contract for every file.

Cases that depend on a second unit (E1007 and W1008, with `--design-std=2019`) put the 2019 side in the harness (the std/ieee revision), not in a second file.

## Reference check (`check_ref.py`)

`python3 check_ref.py [-j N] [files]` runs the reference simulators on each file (a fresh work dir under `~/.cache/rsim-negtests/work`, `timeout 60 systemd-run --user --scope -p MemoryMax=1G`, at most 8 in parallel) and prints one TSV row: path, expected codes with lines, the reference simulator verdict and error lines, the second reference simulator verdict and error lines. `SIM1.md` is the recorded result, with a justification for every file the reference simulator does not agree with.

## Adding a test

One illegal thing per file, no unrelated errors, a short name that says what is wrong. Put `-- LRM <clause>`. Run `check_ref.py`; the reference simulator must reject at the same line (or the file goes into the disagreement table with a reason). Do not rely on behaviour of only one reference simulator.
