# Release note fragments

One file per user-visible change, named `<issue>.<type>.md` (`+<name>.<type>.md` when there is
no issue). Types: `breaking`, `feature`, `bugfix`, `doc`, `misc`. One or two sentences on what
a user can now do or what behaves differently: features, not numbers (no test counts or
benchmarks). The documentation shows them under "Unreleased"; a release collects them with
`uvx towncrier build --config release_notes/towncrier.toml --version X.Y.Z --yes` into `release_notes/unreleased.md`, renamed to
`release_notes/X.Y.Z.md` in the commit that bumps the version.
