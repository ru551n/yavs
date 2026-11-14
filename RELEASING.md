# Releasing yavs

One release per version holds everything (maintainer, 2026-10-30, #903; `docs/decisions.md` "One
release with every artifact"):

- the simulator archives `yavs-<version>-<target>` for Linux x86_64, Linux aarch64 and Windows x86_64, and `yavs-<version>-<target>-compat` for the two x86_64 ones (the plain x86_64 archives are built for x86-64-v3, the compat ones for the baseline x86-64 CPU), each with a generated `README.txt`. Each one is a profile-guided build, trained on its own runner with its own flags (`tools/pgo/AGENTS.md`);
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
named in the READMEs and `docs/`, the version in `docs/getting-started.rst`) and adds a section for the
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
   stale, and `release.yml` publishes nothing until that run of the tag has passed). The `conformance` workflow runs nightly, not on a push (`docs/decisions.md` "Conformance runs
   nightly"), so for a report of the exact commit run `gh workflow run conformance.yml --ref main`
   and use its artifact, after checking that the run's `headSha` is that commit. Main need not be
   quiet for this: runs on `main` are not cancelled by a newer push.
2. A dry run on `main`, if the release flow changed since the last release:
   `gh workflow run release.yml --ref main -f dry_run=true -f checks=true`. It builds and tests
   everything and keeps the files as workflow artifacts; it creates no release and publishes
   nothing Its `conformance` job only lists the conformance runs of the commit: a dry run has no tag run to wait for.
   With `-f pgo_gain=true` each build job also builds the same archive without a profile and measures the difference on held-out designs (`tools/pgo/gain.py`, in the job summary and as artifacts `pgo-gain-*`). Do that when the training set, the toolchain or the runners changed, and update `docs/performance.md`.
3. The release commit:
   - `python3 -I tools/release/bump.py X.Y.Z`, then read `git diff`;
   - `uvx towncrier build --config release_notes/towncrier.toml --version X.Y.Z --yes`, and rename
     `release_notes/unreleased.md` to `release_notes/X.Y.Z.md`. Put a summary line and at most 10
     one-line bullets on top (see `release_notes/0.11.0.md`). The fragments of the package and
     the extension are part of these notes;
     The release notes page of the user guide (Read the Docs) is generated from these files at
     build time: the new `X.Y.Z.md` appears there, newest first, with no docs edit. Until then the
     pending fragments show under "Unreleased". The text is public, so it follows the user
     documentation rules (AGENTS.md rule 25): no internal paths, no other tools;
   - edit the new sections of the two CHANGELOGs if the generated line is not enough;
   - the `vunit_hdl` bound in `python/vunit-yavs/pyproject.toml` names a released version that
     installs.

   Commit as `release: yavs X.Y.Z`, push to `main`, wait for CI.
4. Tag that commit `vX.Y.Z` and push the tag. `release.yml` checks the versions, runs the CI
   checks, waits for the `conformance` run of the tag (a red one, a stale report included, stops the
   release before anything is published: the tag stays, release the next patch version), builds the archives, tests `vunit-yavs` on all three platforms and the extension against
   the archives of this run, creates the GitHub pre-release with every file, then uploads the wheel
   and sdist to PyPI.
5. Approve the `pypi` environment if it has a reviewer. Check the release page, `SHA256SUMS` and
   `pip install vunit-yavs==X.Y.Z`. If the `pypi` job failed (for example the trusted publisher was
   missing), fix the cause and re-run that job alone: the GitHub release is already complete.

Never move or delete a tag or a release that was published; a broken release is followed by the
next patch version.

## When a build job stops at the profile

Every archive is trained in its own job, and no job ships a binary built without a profile. In the last full dry run a build job took 10 to 17 minutes on Linux x86_64, 12 on aarch64 and 20 to 31 on Windows (the same job varies with the runner's CPU), of which the corpus on the binary is 1 to 2 minutes on Linux and 4 to 7 on Windows. All jobs of a dry run with checks came to about 170 runner minutes and 53 minutes of waiting. The last lines of the `Profile-guided build` step name the cause:

- `train: exit status N` with a command and its output: a training run failed, usually a compiler change that rejects a case in `tools/pgo/train/` or `tests/corpus`. The `corpus` job of `ci.yml` runs the training cases on every push, so this should show on `main` first. Fix the case or the compiler.
- `train: corpus run differs`: a `tests/corpus` testbench did not end as its header says.
- `pgo: the compiler complains about the profile`: the source changed between the two builds of the job.
- `pgo: the profile does not fit the final build`: the two builds differ in something cargo mixes into symbol names (`--target`, the linker it is configured with). See `tools/pgo/AGENTS.md` section 4; never lower the check.
- `pgo: the profile is empty or nearly so`: the training wrote (almost) nothing. Check that the instrumented build has the runtime and that `llvm-tools` is installed.
