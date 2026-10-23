# AGENTS.md: tests/diag/ (diagnostic snapshots and the negative sem suite)

Read the root `AGENTS.md` and `tests/AGENTS.md` first. Contracts: `crates/yavs/tests/diag.rs`
(top level) and `tests/diag/sem/README.md` (the `sem/` suite; also `CODES.md`, `SIM1.md`).

## 1. Purpose and boundaries

- `tests/diag/NAME.vhd` plus `NAME.stderr`: rendered output of `yavs parse` (colour off) must
  equal the snapshot, in the style of compiler UI tests. An optional first line `-- yavs: ARGS` adds
  command line arguments (e.g. `--std 2019`, `--message-format json`). Owned by `yavs-syntax` and
  `yavs-diag`.
- `tests/diag/sem/<area>/NAME.vhd`: inputs `yavs-sem` must reject, warn about, or accept silently
  (`*_ok.vhd` near misses). The header `-- yavs-test: std=... [flags=...]`, `-- expect: CODE
  line:col reason` (every diagnostic, nothing else, cascades must not appear), `-- help:`
  fragments. Optional `NAME.stderr` rendered snapshots. Areas by sem.md section: `vis hom use unit
  typ acc stat ovl agg lit expr gen drv prot alias attr sig ext v2019 mix relax fuzz`.
- Not here: CLI snapshots (`tests/cli`), runtime behaviour (`tests/corpus`).

## 2. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Snapshots of rendered text are part of review | Diagnostics are a user interface | `docs/plan.md` "Verification", root rule 14 |
| Exact `(code, line, col)` sets, warnings included, positions at the start of the primary span | Checks cascade suppression | `tests/diag/sem/README.md`, sem.md 8.2 |
| Every negative file is checked against the reference simulators (`check_ref.py`); disagreements are justified in `SIM1.md` | LRM policy (match existing simulators) | `tests/diag/sem/README.md` |
| One illegal thing per file, `-- LRM <clause>` named | Focused failures | same, "Adding a test" |
| Files that cannot pass yet are listed in `crates/yavs-sem/tests/diag_sem_xfail.txt`; any ICE fails regardless | Visible progress, no hidden crashes | `crates/yavs-sem/CONTRIBUTING.md` "Testing" |
| `fuzz/` holds minimised fuzzer finds as regression tests | Rule 15 | `docs/STATUS.md` "sem robustness" |

## 3. How to work here

```sh
cargo test -p yavs --test diag                              # top-level snapshots
YAVS_BLESS=1 cargo test -p yavs --test diag                 # rewrite them, then read git diff
cargo test -p yavs-sem --test diag_sem                      # negative suite
YAVS_SEM_XFAIL_WRITE=1 cargo test -p yavs-sem --test diag_sem   # regenerate the xfail list (diff must only remove lines)
YAVS_BLESS=1 cargo test -p yavs-sem --test diag_sem         # fill existing (possibly empty) .stderr snapshots
python3 tests/diag/sem/check_ref.py -j 8 FILE...            # reference simulator agreement (scratch ~/.cache/rsim-negtests)
```

- Codes: from the owning crate's registry (`crates/yavs-diag/src/lib.rs`, sem.md 8.1);
  `sem/CODES.md` records where each code came from.

## 4. Coordination

- Open issues (snapshot 2026-10-18): xfail groups #45 to #54 (`crate:sem`, `test`); #136
  registry and explain texts.
