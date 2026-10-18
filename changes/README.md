# Changelog fragments

One file per user-visible change, named `<issue>.<type>.md` (`+<name>.<type>.md` when there is
no issue). Types: `added`, `changed`, `fixed`, `removed`. One or two sentences on what a user
can now do or what behaves differently: features, not numbers (no test counts or benchmarks).
`uvx towncrier create 196.fixed.md --content "..."` writes one; `uvx towncrier build --draft`
previews the next release section. A release moves them into `CHANGELOG.md`.
