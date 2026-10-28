"""
Sphinx configuration of the yavs user documentation (Read the Docs, .readthedocs.yaml).

Build with: sphinx-build -W -b html docs docs/_build/html

The site is a user guide. The other files in docs/ (plan, decisions, status, design notes, the
command line contract, the test runner requirements, the parallel kernel measurements, the full
conformance report) are for developers and agents and are not part of the site. To add a page to
the site, see docs/README.md.
"""

import os
import tomllib
from pathlib import Path

from packaging.version import Version

ROOT = Path(__file__).parent.parent

project = "yavs"
copyright = "2026, the yavs authors"  # pylint: disable=redefined-builtin
author = "the yavs authors"
with (ROOT / "Cargo.toml").open("rb") as fptr:
    release = tomllib.load(fptr)["workspace"]["package"]["version"]

extensions = ["myst_parser", "sphinxcontrib.towncrier.ext", "sphinx_llms_txt"]
# The release notes start at ## since they are included under a version heading
suppress_warnings = ["myst.header"]
exclude_patterns = [
    "_build",
    "plan.md",
    "STATUS.md",
    "decisions.md",
    "cli-contract.md",
    "test-runner-requirements.md",
    "parallel.md",
    # The generated report and its summary, which conformance.rst includes
    "conformance.md",
    "conformance-summary.md",
    "design",
    "AGENTS.md",
    "README.md",
    "CLAUDE.md",
]

# llms.txt and llms-full.txt (sphinx-llms-txt) are written to the root of the built site, for
# language-model tools such as Context7. html_baseurl makes the links in llms.txt absolute: Read
# the Docs sets the URL of the version being built, the fallback is the latest one. Point it at
# the new address if the site is published elsewhere (GitHub Pages job in the docs workflow).
html_baseurl = os.environ.get("READTHEDOCS_CANONICAL_URL", "https://yavs.readthedocs.io/en/latest/")
llms_txt_uri_template = "{base_url}{docname}.html"
llms_txt_summary = (
    "yavs (Yet Another VHDL Simulator) is a VHDL-1993/2008/2019 simulator written in Rust, "
    "with a command line interface (analyze, elab, run), compiled-code tiers, coverage, FST "
    "waveforms, DAP debugging and a VHPI plugin interface."
)

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
towncrier_draft_config_path = ROOT / "release_notes" / "towncrier.toml"
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

# The site shows the summary of the generated conformance report, the part before "## Details",
# cut here so that llms-full.txt (which ignores include options) holds only the summary too.
_report = (ROOT / "docs" / "conformance.md").read_text()
if "\n## Details\n" not in _report:
    raise RuntimeError("docs/conformance.md has no '## Details' heading to cut the site summary at")
(ROOT / "docs" / "conformance-summary.md").write_text(_report.split("\n## Details\n", 1)[0] + "\n")
