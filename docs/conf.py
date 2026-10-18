"""
Sphinx configuration of the yavs user documentation (Read the Docs, .readthedocs.yaml).

Build with: sphinx-build -W -b html docs docs/_build/html

The other files in docs/ (plan, decisions, design notes, the VUnit CLI contract) are for
developers and agents and are not part of the site.
"""

import tomllib
from pathlib import Path

from packaging.version import Version

ROOT = Path(__file__).parent.parent

project = "yavs"
copyright = "2026, the yavs authors"  # pylint: disable=redefined-builtin
author = "the yavs authors"
with (ROOT / "Cargo.toml").open("rb") as fptr:
    release = tomllib.load(fptr)["workspace"]["package"]["version"]

extensions = ["myst_parser", "sphinxcontrib.towncrier.ext"]
# The release notes start at ## since they are included under a version heading
suppress_warnings = ["myst.header"]
exclude_patterns = [
    "_build",
    "plan.md",
    "decisions.md",
    "cli-contract.md",
    "test-runner-requirements.md",
    "design",
    "AGENTS.md",
    "CLAUDE.md",
]

html_theme = "sphinx_rtd_theme"
html_theme_options = {
    "prev_next_buttons_location": "both",
    "navigation_depth": 3,
}
# The "Edit on GitHub" link of every page
html_context = {
    "display_github": True,
    "github_user": "ru551n",
    "github_repo": "yavs",
    "github_version": "main",
    "conf_py_path": "/docs/",
}


# The release notes page, generated from release_notes: the draft of the news fragments in
# news.d, rendered by towncrier, and unreleased.md, then the releases, newest first. It is
# Markdown, since towncrier writes Markdown and the draft is parsed like the page.
towncrier_draft_autoversion_mode = "draft"
towncrier_draft_include_empty = False
towncrier_draft_working_directory = ROOT
_notes = ["# Release notes", ""]
_unreleased = sorted((ROOT / "release_notes").glob("unreleased.md"))
_fragments = [p for p in (ROOT / "release_notes" / "news.d").glob("*.md") if p.name != "README.md"]
if _unreleased or _fragments:
    _notes += ["## Unreleased", "", "```{towncrier-draft-entries}", "```", ""]
_releases = sorted(
    (ROOT / "release_notes").glob("[0-9]*.md"), key=lambda path: Version(path.stem), reverse=True
)
for _path in _unreleased + _releases:
    if _path.stem != "unreleased":
        _notes += [f"## {_path.stem}", ""]
    _notes += [f"```{{include}} ../release_notes/{_path.name}", ":heading-offset: 1", "```", ""]
(ROOT / "docs" / "release_notes.md").write_text("\n".join(_notes))
