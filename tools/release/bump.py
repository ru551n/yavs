#!/usr/bin/env python3
"""Set or check the one version of yavs, vunit-yavs and the VS Code extension (RELEASING.md).

  bump.py X.Y.Z            write X.Y.Z into every version spot
  bump.py --check [X.Y.Z]  fail unless every spot holds X.Y.Z (default: the Cargo workspace version)
"""

import datetime
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
V = r"(\d+\.\d+\.\d+)"

# (file, pattern with one version group); each pattern must match at least once.
SPOTS = [
    ("Cargo.toml", r'\[workspace\.package\]\nversion = "' + V + '"'),
    ("docs/install.rst", r"``yavs " + V + r" \(compiled tier"),
    ("README.md", r"it needs yavs " + V + " or later"),
    ("docs/libraries.rst", r"It needs yavs " + V + " or later"),
    ("docs/libraries.rst", r"with yavs " + V + " or later"),
    ("python/vunit-yavs/pyproject.toml", r'\[project\]\nname = "vunit-yavs"\nversion = "' + V + '"'),
    ("python/vunit-yavs/vunit_yavs/__init__.py", r'^YAVS_MIN_VERSION = YavsVersion\("' + V + r'"\)'),
    ("python/vunit-yavs/README.md", r"and yavs " + V + " or later"),
    ("editors/vscode/package.json", r'"name": "yavs-debug",[^{}]*?"version": "' + V + '"'),
    ("editors/vscode/README.md", r"yavs " + V + " or newer must be on"),
]
# The yavs workspace crates in these lock files (registry packages have a source line; yavs-fuzz has its own version).
LOCKS = ["Cargo.lock", "fuzz/Cargo.lock"]
LOCK_PKG = r'\[\[package\]\]\nname = "yavs(?!-fuzz")[\w-]*"\nversion = "' + V + '"\n(?!source)'
CHANGELOGS = ["python/vunit-yavs/CHANGELOG.md", "editors/vscode/CHANGELOG.md"]


def spans(text, pattern):
    return [m.span(1) for m in re.finditer(pattern, text, re.M)]


def all_spots():
    for name, pattern in SPOTS + [(lock, LOCK_PKG) for lock in LOCKS]:
        yield name, pattern, (ROOT / name).read_text(encoding="utf-8")


def check(version):
    bad = []
    for name, pattern, text in all_spots():
        found = [text[a:b] for a, b in spans(text, pattern)]
        if not found:
            bad.append(f"{name}: no version spot matches {pattern!r} (update tools/release/bump.py)")
        bad += [f"{name}: {v}, expected {version}" for v in found if v != version]
    for line in bad:
        print(f"error: {line}", file=sys.stderr)
    if bad:
        print(f"error: the versions disagree; run tools/release/bump.py {version} (RELEASING.md)", file=sys.stderr)
        return 1
    print(f"all versions are {version}")
    return 0


def changelog(text, version):
    """Rename an "## Unreleased" section to the version, or add one, and name the yavs it needs."""
    heading = f"## {version} ({datetime.date.today().isoformat()})"
    if re.search(rf"^## {re.escape(version)}\b", text, re.M):
        return text
    note = f"- Needs yavs {version} or later. All changes: the yavs {version} release notes.\n"
    if re.search(r"^## Unreleased[ \t]*$", text, re.M):
        text = re.sub(r"^## Unreleased[ \t]*$", heading, text, count=1, flags=re.M)
        # The note goes at the end of the renamed section.
        start = text.index(heading) + len(heading)
        nxt = re.search(r"^## ", text[start:], re.M)
        end = start + nxt.start() if nxt else len(text)
        body = text[start:end].rstrip("\n")
        return text[:start] + body + "\n" + note + ("\n" + text[end:] if nxt else "")
    first = re.search(r"^## ", text, re.M)
    at = first.start() if first else len(text)
    return text[:at] + f"{heading}\n\n{note}\n" + text[at:]


def bump(version):
    files = {}
    for name, pattern, text in all_spots():
        text = files.get(name, text)
        found = spans(text, pattern)
        if not found:
            sys.exit(f"error: {name}: no version spot matches {pattern!r} (update tools/release/bump.py)")
        for a, b in reversed(found):
            text = text[:a] + version + text[b:]
        files[name] = text
    for name in CHANGELOGS:
        files[name] = changelog((ROOT / name).read_text(encoding="utf-8"), version)
    for name, text in files.items():
        (ROOT / name).write_text(text, encoding="utf-8", newline="\n")
    return check(version)


def main(argv):
    if argv[:1] == ["--check"] and len(argv) <= 2:
        cargo = re.search(SPOTS[0][1], (ROOT / "Cargo.toml").read_text(encoding="utf-8"), re.M)
        return check(argv[1] if len(argv) == 2 else cargo.group(1))
    if len(argv) == 1 and re.fullmatch(V, argv[0]):
        return bump(argv[0])
    sys.exit(__doc__)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
