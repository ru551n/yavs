# Releasing yavs

One release per version holds everything (maintainer, 2026-10-30, #903; `docs/decisions.md` "One
release with every artifact"):

- the simulator archives `yavs-<version>-<target>` for Linux x86_64, Linux aarch64 and Windows x86_64;
- the test runner package `vunit-yavs`: wheel and sdist, also published to PyPI;
- the VS Code debugger extension `yavs-debug-<version>.vsix` (Marketplace and Open VSX: not yet);
- the conformance report `yavs-<version>-conformance.md` and `.json`;
- one `SHA256SUMS` over all of them, and one set of release notes.

yavs (the Cargo workspace), `vunit-yavs` (its version and `YAVS_MIN_VERSION`) and the extension
(`editors/vscode/package.json`) carry the same version, and every release bumps all three.
`.github/workflows/release.yml` does the release on a `v<version>` tag. The older `vunit-yavs-v*`
and `vscode-v*` tags and releases stay as they are; no new ones are made.

## Versions between releases

There are no development versions: `main` carries the last released version everywhere until the
release commit bumps it. `tools/release/bump.py X.Y.Z` writes every version spot (the Cargo
manifest and lock files, `pyproject.toml`, `YAVS_MIN_VERSION`, `package.json`, the minimum yavs
named in the READMEs and `docs/`, the version in `docs/install.rst`) and adds a section for the
version to `python/vunit-yavs/CHANGELOG.md` and `editors/vscode/CHANGELOG.md` (an `## Unreleased`
section there becomes it). `tools/release/bump.py --check [X.Y.Z]` fails and names every spot
that differs; the `versions` job of `ci.yml` runs it on every push, and `release.yml` runs it with
the tag's version. A new version spot goes into the `SPOTS` table of `bump.py`.

## One-time setup (maintainer)

1. PyPI, project `vunit-yavs`, "Publishing": add a GitHub trusted publisher with owner `ru551n`,
   repository `yavs`, workflow `release.yml`, environment `pypi`. The publisher of the old
   `vunit-yavs-release.yml` can be removed. Without this the `pypi` job fails with
   `invalid-publisher` and says what to add.
2. GitHub, Settings, Environments: `pypi` exists (a required reviewer is a good idea).

## Release, in order

1. Everything for the release is on `main` and CI is green. The committed conformance report is
   current (`tools/conformance/AGENTS.md`; the `conformance` workflow fails a tag whose report is
   stale).
2. A dry run on `main`, if the release flow changed since the last release:
   `gh workflow run release.yml --ref main -f dry_run=true -f checks=true`. It builds and tests
   everything and keeps the files as workflow artifacts; it creates no release and publishes
   nothing.
3. The release commit:
   - `python3 -I tools/release/bump.py X.Y.Z`, then read `git diff`;
   - `uvx towncrier build --config release_notes/towncrier.toml --version X.Y.Z --yes`, and rename
     `release_notes/unreleased.md` to `release_notes/X.Y.Z.md`. Put a summary line and at most 10
     one-line bullets on top (see `release_notes/0.11.0.md`). The fragments of the package and
     the extension are part of these notes;
   - edit the new sections of the two CHANGELOGs if the generated line is not enough;
   - the `vunit_hdl` bound in `python/vunit-yavs/pyproject.toml` names a released version that
     installs.

   Commit as `release: yavs X.Y.Z`, push to `main`, wait for CI.
4. Tag that commit `vX.Y.Z` and push the tag. `release.yml` checks the versions, runs the CI
   checks, builds the archives, tests `vunit-yavs` on all three platforms and the extension against
   the archives of this run, creates the GitHub pre-release with every file, then uploads the wheel
   and sdist to PyPI.
5. Approve the `pypi` environment if it has a reviewer. Check the release page, `SHA256SUMS` and
   `pip install vunit-yavs==X.Y.Z`. If the `pypi` job failed (for example the trusted publisher was
   missing), fix the cause and re-run that job alone: the GitHub release is already complete.

Never move or delete a tag or a release that was published; a broken release is followed by the
next patch version.
