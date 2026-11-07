# tests/verilog: Verilog front-end test material

Inputs and expected outcomes for `yavs analyze` on Verilog (IEEE 1364-2005). Written fresh from the language reference and the
Verilog plan (#776); no third-party text. Read `AGENTS.md` here first.

| Path | What | Run by |
|---|---|---|
| `ci/*.v`, `ci/*.sv`, `ci/inc/` | Hand-written fixtures, one construct family per file, plus negative and near-miss files | `tools/verilog/fixtures.py` |
| `ci/fixtures.expected` | The expected outcome of every fixture (format below) | same, and the Rust tests of `yavs-verilog` |
| `lrm/*.v`, `lrm/inc/` | The per-clause LRM checks of IEEE 1364-2005: one rule per file, `<clause>_<topic>.v`, the expected diagnostics in the file header (format below) | `crates/yavs-verilog/tests/lrm.rs` (`cargo test -p yavs-verilog`, part of CI) |

```sh
python3 -I tools/verilog/fixtures.py --static          # manifest against files; needs no analyzer
python3 -I tools/verilog/fixtures.py --yavs target/release/yavs
python3 -I tools/verilog/selftest.py                    # the tools' own failure and skip paths
```

## `ci/fixtures.expected`

One line per `yavs analyze` call; blank lines and `#` lines are ignored; words are split like a shell does. A file that must be
analysed under two argument sets has two lines.

```
STATUS  FILE  [args="..."] [diag=CODE[@LINE:COL][,...]] [units=a,b] [later=reason]
```

| Field | Meaning |
|---|---|
| `STATUS` | `OK`: exit status 0. `ERR`: exit status 1 and at least one error. Must agree with `diag` |
| `FILE` | Fixture in `ci/`; the call runs in `ci/`, so `-I inc` and the file name are relative |
| `args="..."` | Extra arguments of the call: `-I DIR`, `-D NAME[=VALUE]` (the call adds `--work=LIB:DIR` and the file) |
| `diag=` | Every error and warning the call prints, nothing else (notes are not compared). `CODE` is `E4001` and so on, or `W*` for any warning whose code is not registered yet. `@LINE:COL` is the start of the primary span; leave it out where the plan does not fix a position |
| `units=` | The exact list of units analysis records (active code only: a module under a false `` `ifdef `` is not one), case as written, in file order. `fixtures.py --static` checks only that each is declared in the text; the analyzer's own list is checked by the Rust tests of `yavs-verilog` |
| `later=` | Informational: the construct analyses fine and is refused later (for example at elaboration). Never compared |

A diagnostic list is compared as a multiset: order does not matter, and a position is checked only when the line gives one.

## `lrm/` headers

Every file starts with `//` lines, then the Verilog. The list of `expect` lines is exact (errors and warnings, order free, notes not compared); a clean file says `// expect: none`.

```
// LRM <clause>: <the rule, one sentence>
// yavs-test: args=-I inc -D NAME[=VALUE]            (optional; -I is relative to lrm/)
// units: a,b                                        (optional: the units recorded, in order)
// gap: G<n> <text>                                  (optional: the LRM says error, the front end accepts; behaviour pinned)
// expect: <CODE> [<FILE>:]<LINE>:<COL> <why>        (start of the primary span; FILE only for a span in an include)
// output: <text>                                    (optional, one per line: the exact output of a run of the last module)
// elab: <CODE>                                      (optional: the one error elaboration of the last module gives)
```

Files with `output` or `elab` lines (named `<clause>_eval_<topic>.v`) are also elaborated and run, the last module of the file
with no ports as the test's own top level, by `crates/yavs-verilog/tests/run.rs`; their rows are in the section "Elaboration and
run-time checks" of `docs/design/lrm-checks.md`.

A new check is one file plus one row in the Verilog section of `docs/design/lrm-checks.md` (the test fails for a file without a row and a row without a file).
Write the rule from the LRM, one illegal thing per negative file, and a legal near miss next to it. `docs/design/lrm-checks.md` also lists the readings
(V1 to V14) and the known gaps (G1 to G11).

## Coverage of the plan

Plan section 1.2 families, one file each: `preproc_*` (macros with arguments, nesting, `undef`, conditionals, `include` through
`-I`, command-line `-D`), `ansi_header`, `parameters` (string, 256-bit and real values, positional and named overrides,
`defparam`), `generate`, `udp_comb` and `udp_seq`, `specify` (path delays, conditions, `PATHPULSE$`) and
`specify_timing_checks` (checks only under `XIL_TIMING`, with and without `-D XIL_TIMING`), `attributes`, `gates`,
`strengths_nets` (tri0, tri1, supply, strengths, `glbl.` reference), `procedural_assign` (assign/deassign), `tasks_functions`,
`system_tasks`.

Encrypted files (decision D8 of #776): `encrypted_envelope` and `encrypted_legacy` analyse with exit 0 and exactly one notice W4007
at the envelope; the module of the plaintext header is recorded (`units=`), the body is never lexed (it holds an unterminated
string and comment, so a lexer that reads it fails). E4001 is the error on instantiating such a module and needs elaboration, so
no fixture here has it yet. Not covered: the comment form `//pragma protect begin_protected` that other tools write; it is a
comment to a directive-based detector (documented gap, `tools/verilog/AGENTS.md`).

Negative files (`neg_*`): `neg_sv_extension.sv` (E4002), `neg_sv_keyword` (E4003), `neg_unbalanced_ifdef` and `neg_stray_endif`
(E4004), `neg_include_missing` (E4005). Near misses that must stay clean: `sv_words_ok` (SystemVerilog words only in a string and
comments), `pragma_plaintext_ok` (`` `pragma protect begin `` and `end`` mark plaintext: warnings, no envelope). `timescale_missing`
expects W4006 (default 1ns/1ps). `unsupported_switch` and `unsupported_nettypes` analyse cleanly; elaboration refuses them later.

Where the plan is silent the manifest proposes and the Verilog front-end agent decides: one `W*` per `pragma protect begin`/`end`
marker, W4006 without a position, the E4003 trigger `always_ff` (unambiguous: `logic` and `bit` are legal 1364 identifiers).
Change the line when the implementation decides otherwise, and say why in the commit.
