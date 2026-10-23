# AGENTS.md: tests/parse-corpus/ (expected parse results)

Read the root `AGENTS.md` and `tests/AGENTS.md` first.

- One list per external corpus (`vunit.txt`, `osvvm.txt`, `rust_hdl.txt`, `sim1.txt`,
  `vests.txt`, `sim2.txt`), one line `PASS|FAIL <std> <source>/<path>` per file, compared by
  `crates/yavs/tests/corpus.rs` (`--features corpus`), which fails when `YAVS_CORPUS` is unset.
  Which files are parsed as 2019 and why some are expected failures: `docs/decisions.md`
  "Parser" (corpus lists) and the table in `docs/STATUS.md` "syntax".
- The corpora come from `tools/fetch-corpus.sh` at pinned commits; bump a commit and the lists
  together. Never commit corpus files (root rule 7).
- An expected `FAIL` must be a test of invalid syntax, PSL, VHDL-87/93-only code or a known
  issue; a new `FAIL` for valid code is a parser bug (file an issue, `crate:syntax`).

```sh
tools/fetch-corpus.sh
YAVS_CORPUS=~/.cache/rsim-corpus cargo test -p yavs --features corpus
YAVS_BLESS=1 YAVS_CORPUS=~/.cache/rsim-corpus cargo test -p yavs --features corpus   # rewrite lists, review the diff
```

Open issues (snapshot 2026-10-18): #87 periodic parse-corpus gate. Decisions about these lists go here in the same commit (root rule 24).
