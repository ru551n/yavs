# AGENTS.md: fuzz/ (cargo-fuzz targets)

Read the root `AGENTS.md` first. This file is for an agent about to run or change the fuzzers.

## 1. Purpose and boundaries

- Front-end fuzzing from M0 (`docs/plan.md` rule 15: a week of fuzzing found 32 crashes in another simulator's front end).
  Own crate `yavs-fuzz` in its **own workspace** (cargo-fuzz needs nightly), depending on
  `yavs-diag`, `yavs-syntax`, `yavs-sem`. Not built by `cargo test --workspace`.
- `fuzz_targets/parse.rs`: lexer, conditional analysis, parser and rendering in both standards
  (both byte readings, UTF-8 and Latin-1) must never panic or hang.
- `fuzz_targets/sem.rs`: parse and analyze against yavs's std/ieee in 2008 and 2019; fails on a
  panic, a hang (`-timeout=20`), RSS over 2 GB, or any diagnostic containing "this is a bug in
  yavs". Writes each input to `/dev/shm/yavs-sem-fuzz-<pid>.vhd` (the loader reads files).
- `fuzz/corpus` and `fuzz/artifacts` are git-ignored; never commit them.

## 2. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Fuzz the parser and semantic analysis from M0 | Rule 15 | `docs/plan.md` rule 15, sem.md 9.3 |
| An ICE diagnostic counts as a crash, not only a panic | Error motto: broken invariants are bugs | `fuzz_targets/sem.rs`, `STATUS.md` "sem robustness" |
| Every find becomes a minimised regression test (`tests/diag/sem/fuzz/`, or a `yavs-syntax` unit test) before the fix lands | Keeps it fixed | `STATUS.md` "sem robustness" |

Decisions about fuzzing go here in the same commit (root `AGENTS.md` rule 24).

## 3. How to work here

```sh
cd fuzz
cargo +nightly fuzz run parse -- -max_total_time=300
cargo +nightly fuzz run sem -- -rss_limit_mb=2048 -timeout=20 -max_total_time=300 ../tests/diag/sem ../lib
cargo +nightly fuzz run sem --jobs 4 -- -rss_limit_mb=2048 -timeout=20     # parallel workers: watch memory (root rule 20)
cargo +nightly fuzz tmin sem artifacts/sem/crash-...                        # minimise a find
```

- Seeds can also include `../tests/corpus`; `-artifact_prefix=<dir>/` needs an existing directory (use a cache dir, not `/tmp`).
- Keep runs bounded (`-max_total_time`) and worker counts low on the shared host; run
  `cargo clean` in `fuzz/` afterwards (its own `target/`).
- Open issues (snapshot 2026-10-18): #87 periodic fuzz and parse-corpus gate; #44 fuzz regression
  guard.
