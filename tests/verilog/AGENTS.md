# AGENTS.md: tests/verilog/ (Verilog front-end fixtures)

Read the root `AGENTS.md` and `tests/AGENTS.md` first. `README.md` here holds the manifest format. Plan and milestones: issue #776.

## 1. Purpose and boundaries

- `ci/`: small hand-written Verilog files with the outcome `yavs analyze` must give, one construct family per file, plus
  encrypted-envelope files (W4007, D8) and negative files for the diagnostics E4002 to E4005 and near misses. Checked by `tools/verilog/fixtures.py` (`--static` needs
  no analyzer; CI runs it against the release binary in the `corpus` job).
- `lrm/`: the per-clause LRM checks (`<clause>_<topic>.v`, one rule per file, expected diagnostics in the file header, rules recorded in
  `docs/design/lrm-checks.md`). Run by `crates/yavs-verilog/tests/lrm.rs`, not by `fixtures.py`. The `*_eval_*.v` files also carry
  `// output:` or `// elab:` lines and are elaborated and run by `crates/yavs-verilog/tests/run.rs` (#776 step C).
- Not here: the vendor and open-source cell-model corpus gate (`tools/verilog/corpus.py`, manifests in `tools/verilog/`); it
  reads installs in place and commits only file lists (root rule 7).

## 2. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| One manifest `ci/fixtures.expected` instead of a header in each file | The same file runs under several argument sets (`XIL_TIMING`), and a harness reads one file | Verilog test material, `docs/decisions.md` |
| A fixture's diagnostics are the exact set of errors and warnings, positions optional | Cascades must not appear (as `tests/diag/sem`) | `tests/diag/AGENTS.md` |
| Fixtures never copy vendor text; the encrypted envelopes are synthetic | Licences (root rules 6, 7) | root `AGENTS.md` |
| E4003 fixtures use `always_ff` | `logic`, `bit` and `int` are legal 1364 identifiers; the policy for them is open | `README.md` |
| `lrm/` files carry their expectation in a header, not in a manifest | One rule per file as in `tests/diag/sem`; the Rust runner needs no binary | `README.md` |
| Notes are not compared | The "timing checks not evaluated" note is not an error or warning | plan section 6 |

## 3. Pitfalls

- Positions are the start of the primary span (the backtick of a directive). Editing a fixture moves them: run
  `fixtures.py --static`, which checks that each position lies inside the file, and fix the manifest line.
- A fixture that analyses cleanly must carry `` `timescale ``, or it expects W4006.
- Every `.v`/`.sv` file in `ci/` needs a manifest line (`--static` fails otherwise).

## 4. Commands

```sh
python3 -I tools/verilog/fixtures.py --static
python3 -I tools/verilog/fixtures.py --yavs target/release/yavs
python3 -I tools/verilog/selftest.py
```
