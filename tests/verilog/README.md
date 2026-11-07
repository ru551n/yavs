# tests/verilog: Verilog front-end test material

Inputs and expected outcomes for `yavs analyze` on Verilog (IEEE 1364-2005). Written fresh from the language reference and the
Verilog plan (#776); no third-party text. Read `AGENTS.md` here first.

| Path | What | Run by |
|---|---|---|
| `ci/*.v`, `ci/*.sv`, `ci/inc/` | Hand-written fixtures, one construct family per file, plus negative and near-miss files | `tools/verilog/fixtures.py` |
| `ci/fixtures.expected` | The expected outcome of every fixture (format below) | same, and the Rust tests of `yavs-verilog` |
| `lrm/` | Reserved for the per-clause LRM checks of the plan (not created yet) | |

```sh
python3 -I tools/verilog/fixtures.py --static          # manifest against files; needs no analyzer (not run by CI yet)
python3 -I tools/verilog/fixtures.py --yavs target/release/yavs
python3 -I tools/verilog/selftest.py                    # the tools against a stand-in analyzer, with deliberately wrong ones
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

## Coverage of the plan

Plan section 1.2 families, one file each: `preproc_*` (macros with arguments, nesting, `undef`, conditionals, `include` through
`-I`, command-line `-D`), `ansi_header`, `parameters` (string, 256-bit and real values, positional and named overrides,
`defparam`), `generate`, `udp_comb` and `udp_seq`, `specify` (path delays, conditions, `PATHPULSE$`) and
`specify_timing_checks` (checks only under `XIL_TIMING`, with and without `-D XIL_TIMING`), `attributes`, `gates`,
`strengths_nets` (tri0, tri1, supply, strengths, `glbl.` reference), `procedural_assign` (assign/deassign), `tasks_functions`,
`system_tasks`.

Negative files (`neg_*`): `neg_encrypted` and `neg_encrypted_legacy` (E4001, one error each; the synthetic envelope body has an
unterminated string and comment so a lexer that reads it fails), `neg_sv_extension.sv` (E4002), `neg_sv_keyword` (E4003),
`neg_unbalanced_ifdef` and `neg_stray_endif` (E4004), `neg_include_missing` (E4005). Near misses that must stay clean:
`sv_words_ok` (SystemVerilog words only in a string and comments), `pragma_plaintext_ok` (`` `pragma protect begin `` and `end``
mark plaintext: warnings, not E4001). `timescale_missing` expects W4006 (default 1ns/1ps). `unsupported_switch` and
`unsupported_nettypes` analyse cleanly; elaboration refuses them later.

Where the plan is silent the manifest proposes and the Verilog front-end agent decides: one `W*` per `pragma protect begin`/`end`
marker, W4006 without a position, the E4003 trigger `always_ff` (unambiguous: `logic` and `bit` are legal 1364 identifiers).
Change the line when the implementation decides otherwise, and say why in the commit.
