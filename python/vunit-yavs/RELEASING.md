# Releasing vunit-yavs

vunit-yavs is released on its own, with its own version. Two tag workflows, which never start each
other:

| Tag | Workflow | Does |
|---|---|---|
| `v<version>` (`v[0-9]*`) | `.github/workflows/release.yml` | yavs binaries, GitHub pre-release |
| `vunit-yavs-v<version>` | `.github/workflows/vunit-yavs-release.yml` | checks the tag equals `pyproject.toml`, builds sdist and wheel, unit tests, `twine check`, the real-binary tests against the downloaded Linux archive of the yavs release `v<YAVS_MIN_VERSION>`, PyPI (trusted publishing, no tokens), GitHub release |

## One-time setup (owner)

1. PyPI: "Add a new pending publisher": project `vunit-yavs`, owner `ru551n`, repository `yavs`,
   workflow `vunit-yavs-release.yml`, environment `pypi`.
2. GitHub, Settings, Environments: create `pypi` (a required reviewer is a good idea).

## Before tagging

- Everything for the release is merged to `main`.
- `pyproject.toml`: `version` is the release version (the dev version `X.Y.Z.dev0` dropped), and
  the `vunit_hdl` bound names a released VUnit with simulator packages (it is a placeholder
  until then, and the wheel's tests cannot install without it).
- `YAVS_MIN_VERSION` in `vunit_yavs/__init__.py` is the first yavs with what the package uses.
- `CHANGELOG.md` has the version's entries.
- The one-time setup above is done.

## Release, in order

1. Tag `vunit-yavs-v<version>` on a `main` commit and push it. The real-binary tests run against
   the yavs binaries of `YAVS_MIN_VERSION`, not of the package version. When a package release
   needs a newer yavs, raise `YAVS_MIN_VERSION` first, and release that yavs (tag `v<version>`,
   wait for `release.yml` to publish it) before tagging the package.
2. Approve the `pypi` environment if it has a reviewer; check PyPI and the GitHub release.
3. Bump `version` to the next `X.Y.Z.dev0` and add an "unreleased" heading to `CHANGELOG.md`.
