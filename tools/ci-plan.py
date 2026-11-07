#!/usr/bin/env python3
"""What ci.yml runs for a push or pull request, from the changed files (docs/decisions.md "CI scope").

Reads EVENT, REF, BEFORE and BASE from the environment, writes the outputs of the `plan` job:
  scope     full: every Linux job. light: `check` and `vunit-yavs` run only the tests that read
            docs/, python/vunit-yavs/ or editors/vscode/, or the whole package tests when it
            changed (`pkg`). none: no job that builds yavs.
  pkg, os, vunit_os, stubs  the package changed, the platforms, the required check names to report
            as passed.
A file in neither list below means full, so a new kind of file runs everything. A test that starts
reading a file on the PROSE list needs that file off the list, or a light test.
  tools/ci-plan.py --selftest
"""
import json, os, re, subprocess, sys

# No job of ci.yml reads these (the versions job does, and it always runs).
PROSE = re.compile(
    r"^(?!lib/)(.*/)?(AGENTS|CLAUDE)\.md$"
    r"|^(crates|docs|tools|python|editors|fuzz)/.*\.md$"
    r"|^docs/conformance.*\.json$"
    r"|^release_notes/"
    r"|^install\.(sh|ps1)$|^tools/install/"
    r"|^(README|RELEASING|CONTRIBUTING)\.md$|^LICENSE|^context7\.json$|^\.readthedocs\.yaml$"
    r"|^\.github/dependabot\.yml$"
    r"|^\.github/workflows/(docs|fuzz|nightly|release|vscode|conformance|cocotb-windows|install)\.yml$"
    r"|^tools/(bench|stepcount|realbench|waveq|vhpi-conformance|release)/"
    r"|^tools/(claim|build-bench\.sh|check-sim1\.sh|update-ieee-libs\.sh|derive_libs\.py|gen_native_md\.py)$"
)
# Only the tests named in the light steps of ci.yml read these.
LIGHT = re.compile(r"^docs/|^python/vunit-yavs/|^editors/vscode/")
# Windows: source and build files (maintainer, 2026-11-01, #1014). Not tooling, docs or test data.
WINDOWS = re.compile(r"^crates/|^lib/|^Cargo\.(toml|lock)$|^\.cargo/|^\.github/workflows/ci\.yml$")
TEST_DATA = re.compile(r"^crates/[^/]+/tests/.*\.(vhdl?|expected|txt|script|stdout|stderr)$")
REQUIRED = {"check": "check (ubuntu-24.04)", "clift": "clift (ubuntu-24.04)", "corpus": "corpus",
            "vunit": "vunit-yavs (ubuntu-24.04)"}
LINUX, ARM, WIN = "ubuntu-24.04", "ubuntu-24.04-arm", "windows-latest"


def plan(files):
    """`files` is the changed paths, or None for every job on every platform."""
    if files is None:
        return dict(scope="full", pkg=True, os=[LINUX, ARM, WIN], vunit_os=[LINUX, WIN], stubs=[])
    if not files:  # An empty range says nothing: run the jobs.
        files = ["(empty range)"]
    code = [f for f in files if not PROSE.search(f)]
    scope = "none" if not code else "light" if all(LIGHT.search(f) for f in code) else "full"
    win = any(WINDOWS.search(f) and not TEST_DATA.search(f) for f in code)
    pkg = any(f.startswith("python/vunit-yavs/") for f in code)
    run = {"none": [], "light": ["check", "vunit"], "full": list(REQUIRED)}[scope]
    return dict(scope=scope, pkg=pkg, os=[LINUX] + [WIN] * win, vunit_os=[LINUX] + [WIN] * (win or pkg),
                stubs=[v for k, v in REQUIRED.items() if k not in run])


def changed():
    event, ref, before, base = (os.environ.get(k, "") for k in ("EVENT", "REF", "BEFORE", "BASE"))
    if event not in ("push", "pull_request") or ref.startswith("refs/tags/"):
        return None  # schedule, workflow_dispatch, a release tag: everything
    start = base if event == "pull_request" else before
    rng = f"{base}...HEAD" if event == "pull_request" else f"{before}..HEAD"
    r = subprocess.run(["git", "diff", "--name-only", rng], capture_output=True, text=True)
    if r.returncode or not start.strip("0"):
        print(f"::notice::no usable diff range ({rng}): everything runs, Windows included")
        return ["Cargo.lock"]
    return r.stdout.split("\n")[:-1]


def selftest():
    def t(files, scope, win=False, vwin=None, stubs=None):
        p = plan(files)
        assert p["scope"] == scope, (files, p)
        assert (WIN in p["os"]) == win, (files, p)
        assert (WIN in p["vunit_os"]) == (win if vwin is None else vwin), (files, p)
        if stubs is not None:
            assert len(p["stubs"]) == stubs, (files, p)

    for f in ["AGENTS.md", "install.sh", "install.ps1", "tools/install/test.sh", ".github/workflows/install.yml",
              "docs/STATUS.md", "crates/yavs-sem/AGENTS.md", "crates/yavs-ir/README.md",
              "release_notes/news.d/1.bugfix.md", "tools/bench/run.py", "tools/claim", "README.md",
              ".github/workflows/docs.yml", "docs/design/ir.md", "tests/diag/AGENTS.md"]:
        t([f], "none", stubs=4)
    for f in ["docs/cli.rst", "docs/examples/vunit/run.py", "editors/vscode/package.json"]:
        t([f], "light", stubs=2)
    t(["python/vunit-yavs/vunit_yavs/console.py"], "light", vwin=True)
    # Anything that can change results: the full Linux jobs. Unknown files too.
    for f in ["tests/corpus/x/a.vhd", "tests/cli/a.stderr", "tools/corpus/grlib-1993.expected",
              "tools/difftest/difftest.py", "tools/fetch-corpus.sh", "tests/verilog/ci/udp_comb.v",
              "tools/verilog/corpus.py", "tools/verilog/vendor.expected", "tools/ci-plan.py", "new-file", ".rustfmt.toml",
              "fuzz/Cargo.toml", ".github/workflows/new.yml", "tools/simconf.py"]:
        t([f], "full", stubs=0)
    for f in ["crates/yavs/src/main.rs", "crates/yavs-kernel/src/kernel.rs", "crates/yavs-vhpi/tests/plugin/plugin.c",
              "crates/yavs/tests/cli.rs", "lib/ieee2008/numeric_std.vhdl", "lib/NATIVE.md", "lib/AGENTS.md",
              "Cargo.lock", "Cargo.toml", "crates/yavs-kernel/Cargo.toml", "crates/yavs-sem/build.rs",
              ".cargo/config.toml", ".github/workflows/ci.yml"]:
        t([f], "full", win=True, stubs=0)
    t(["crates/yavs/tests/perf/a.vhd"], "full")  # test data: no Windows
    t(["docs/cli.rst", "AGENTS.md"], "light")
    t(["docs/cli.rst", "crates/yavs-ir/src/lib.rs"], "full", win=True)
    t([], "full")
    assert plan(["python/vunit-yavs/tests/test_docs.py"])["pkg"] and not plan(["docs/cli.rst"])["pkg"]
    assert plan(None) == dict(scope="full", pkg=True, os=[LINUX, ARM, WIN], vunit_os=[LINUX, WIN], stubs=[])
    print("ci-plan selftest: ok")


if __name__ == "__main__":
    if sys.argv[1:] == ["--selftest"]:
        selftest()
    else:
        p = plan(changed())
        print(p)
        with open(os.environ["GITHUB_OUTPUT"], "a") as out:
            out.write(f"scope={p['scope']}\npkg={str(p['pkg']).lower()}\n")
            for k in ("os", "vunit_os", "stubs"):
                out.write(f"{k}={json.dumps(p[k], separators=(',', ':'))}\n")
