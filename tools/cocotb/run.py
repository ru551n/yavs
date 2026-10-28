#!/usr/bin/env python3
"""cocotb's own simulator regression (a pinned release) on yavs over VHPI, VHDL only.

  tools/cocotb/run.py --yavs BIN [--dir DIR] [--out DIR] [--jobs N] [--json FILE] [job ...]
  tools/cocotb/run.py --setup-only [--dir DIR]

Setup (once per DIR, default ~/.cache/yavs-cocotb): a venv with the pinned cocotb and its test
dependencies (needs `uv` and network for pip), a shallow clone of the same cocotb tag (its tests and
examples), cocotb's VHPI library built from that clone without simulator defines as
libcocotbvhpi_yavs (needs a C++ compiler), and the yavs makefile and Python runner installed into
the venv. Each run copies the tests to --out (default DIR/out) and runs every job (a test case,
an example, a pytest group; names like `test_cases/test_array` or just `test_array`) with
SIM=yavs TOPLEVEL_LANG=vhdl under a 300 s timeout and (without --no-sandbox) a 2 GB memory cap. A job passes when it
exits 0 (cocotb's own verdict, expected failures included). Exit status 1 when a job fails or one
in EXPECTED_FAIL passes. Python stdlib only.
"""
import argparse, concurrent.futures as cf, hashlib, json, os, shutil, subprocess, sys, xml.etree.ElementTree as ET
from pathlib import Path

HERE = Path(__file__).resolve().parent
COCOTB = "2.1.0"
SHA = "e85fae957d5c93894740b164ea9cc6567015b954"  # the v2.1.0 tag; its tests and examples are not in the sdist
PYTHON = "3.13"
REPO = "https://github.com/cocotb/cocotb"
# cocotb's simulator regression (its noxfile): make in tests/test_cases and EXAMPLES, pytest -k
# simulator_required, the pytest plugin's tests and the PYTEST_EXAMPLES; Verilog-only parts skip themselves
EXAMPLES = ["adder/tests", "simple_dff", "matrix_multiplier/tests", "mixed_language/tests"]
PYTEST_EXAMPLES = ["adder", "simple_dff", "matrix_multiplier", "mixed_language"]
TEST_DEPS = ["pytest==9.1.1", "ipython==9.16.1", "pexpect==4.9.0", "psutil==7.2.2"]
# Gates on the simulator name that leave yavs out only because cocotb does not know it yet: in the
# scratch copy the gating line becomes one that names yavs, as the upstream change would. Each entry
# is (file, sha256 prefix of the pinned file, {line number: new line}); a changed file stops the run.
# The gates: VHDL libraries, the top-level library, waves, timescale, IPython, the recursive object
# count of test_iteration_vhdl (the one three reference simulators agree on), an example's generics.
LIBS = "$(filter yavs,$(shell echo $(SIM) | tr A-Z a-z))"
GATES = [
    ("tests/test_cases/test_vhdl_libraries/Makefile", "af2b8730b4e99ffb", {15: f"ifneq ({LIBS},)", 23: f"else ifeq ({LIBS},)"}),
    ("tests/test_cases/test_vhdl_libraries_multiple/Makefile", "3876e7bc4c263dea", {20: f"ifneq ({LIBS},)", 28: f"else ifeq ({LIBS},)"}),
    ("tests/test_cases/test_toplevel_library/Makefile", "9a7c2e64b78fcfe0", {5: "ifeq ($(SIM)_$(TOPLEVEL_LANG),yavs_vhdl)"}),
    ("tests/pytest/test_vhdl_libraries_multiple.py", "5a0e063a48340a20", {29: '    os.getenv("SIM", "") not in ["yavs"],'}),
    ("tests/pytest/test_waves.py", "92ec7c30e64bc254", {74: '    sim not in ["yavs"],', 85: '    elif sim == "yavs":'}),
    ("tests/pytest/test_timescale.py", "74a6d90f58fd8685", {38: '    os.getenv("SIM", "") not in ["yavs"],'}),
    ("tests/pytest/test_ipython_support.py", "e21734081055400e", {58: '    sim not in ["yavs"],'}),
    ("tests/test_cases/test_iteration_vhdl/test_iteration.py", "d6ee967ccf4af126", {36: '            "yavs",'}),
    ("examples/matrix_multiplier/tests/Makefile", "a039ac826a358156", {38: "    ifneq ($(filter $(SIM),yavs),)"}),
]
# jobs that fail for reasons outside yavs, with the reason (none today)
EXPECTED_FAIL: dict[str, str] = {}


def run(cmd, **kw):
    p = subprocess.run([str(c) for c in cmd], text=True, capture_output=True, **kw)
    if p.returncode:
        sys.exit(f"error: {' '.join(map(str, cmd[:4]))} failed ({p.returncode}):\n{(p.stdout + p.stderr)[-3000:]}")
    return p.stdout


def site(venv):
    return Path(run([venv / "bin/python", "-c", "import sysconfig; print(sysconfig.get_paths()['purelib'])"]).strip())


def append(f, text):
    mark = "# added by yavs tools/cocotb\n"
    s = f.read_text()
    if mark not in s:
        f.write_text(s + "\n" + mark + text)


def setup(d):
    venv, src = d / "venv", d / "src"
    if not shutil.which("uv"):
        sys.exit("error: `uv` not found on PATH; it creates the venv (https://docs.astral.sh/uv/)")
    if not (venv / "bin/python").exists():
        run(["uv", "venv", "-q", "--python", PYTHON, venv])
    run(["uv", "pip", "install", "-q", "--python", venv / "bin/python", f"cocotb=={COCOTB}", *TEST_DEPS])
    if not src.exists():
        run(["git", "clone", "-q", "--depth", "1", "--branch", f"v{COCOTB}", REPO, src])
    head = run(["git", "-C", src, "rev-parse", "HEAD"]).strip()
    if head != SHA:
        sys.exit(f"error: {src} is at {head}, not v{COCOTB} ({SHA}); delete it and rerun")
    sp = site(venv)
    libs, share = sp / "cocotb/libs", src / "src/cocotb/share"
    lib = libs / "libcocotbvhpi_yavs.so"
    gpi = sorted((share / "lib/gpi/vhpi").glob("*.cpp"))
    if not lib.exists() or any(f.stat().st_mtime > lib.stat().st_mtime for f in gpi):
        cxx = os.environ.get("CXX", "c++")
        # the manylinux wheel's libgpi uses the old libstdc++ string ABI
        run([cxx, "-std=c++11", "-O2", "-shared", "-fPIC", "-DCOCOTBVHPI_EXPORTS", "-D__STDC_FORMAT_MACROS", "-D_GLIBCXX_USE_CXX11_ABI=0",
             f"-I{share / 'include'}", f"-I{src / 'src/cocotb'}", *gpi, "-o", lib,
             f"-L{libs}", "-lgpi", "-Wl,-rpath,$ORIGIN"])
    shutil.copy(HERE / "Makefile.yavs", sp / "cocotb_tools/makefiles/simulators/Makefile.yavs")
    shutil.copy(HERE / "yavs_runner.py", sp / "yavs_runner.py")
    # the upstream change in miniature: yavs in the runner table and the pytest plugin's simulator table
    append(sp / "cocotb_tools/runner.py", 'from yavs_runner import Yavs  # noqa: E402\nSUPPORTED_RUNNERS["yavs"] = Yavs\n')
    append(sp / "cocotb_tools/_pytest/hdl.py", '_SIMULATORS["yavs"] = "yavs"\n')
    return venv, src


def counts(files):
    """Test counts of JUnit files: pass, fail, skip; and the names of the failing tests."""
    n, failed = [0, 0, 0], []
    for f in files:
        try:
            root = ET.parse(f).getroot()
        except ET.ParseError:
            continue
        for tc in root.iter("testcase"):
            k = 1 if tc.find("failure") is not None or tc.find("error") is not None else 2 if tc.find("skipped") is not None else 0
            n[k] += 1
            failed += [tc.get("name")] if k == 1 else []
    return n, failed


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--yavs", type=Path)
    ap.add_argument("--dir", type=Path, default=Path("~/.cache/yavs-cocotb").expanduser())
    ap.add_argument("--out", type=Path, default=None, help="scratch (default DIR/out)")
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--json", type=Path)
    ap.add_argument("--setup-only", action="store_true")
    ap.add_argument("--no-sandbox", action="store_true", help="no systemd-run memory cap (CI)")
    ap.add_argument("--run-args", default="", help="extra `yavs run` options, e.g. '--threads=4 --tier=interp'")
    ap.add_argument("cases", nargs="*")
    a = ap.parse_args()
    d = a.dir.expanduser().resolve()
    d.mkdir(parents=True, exist_ok=True)
    venv, src = setup(d)
    if a.setup_only:
        return
    if not a.yavs or not a.yavs.is_file():
        sys.exit(f"error: --yavs {a.yavs} is not a file; build it with `cargo build --release -p yavs`")
    yavs = a.yavs.resolve()
    out = (a.out or d / "out").expanduser().resolve()
    mark = out / ".yavs-cocotb-out"
    if out.exists():
        if not mark.exists():
            sys.exit(f"error: --out {out} exists and was not made by this tool; give a new directory")
        shutil.rmtree(out)
    out.mkdir(parents=True)
    mark.touch()
    for x in ["tests", "examples", "pyproject.toml"]:
        (shutil.copytree if (src / x).is_dir() else shutil.copy)(src / x, out / x)
    for f, sha, lines in GATES:
        text = (out / f).read_bytes()
        if hashlib.sha256(text).hexdigest()[:16] != sha:
            sys.exit(f"error: {f} is not the pinned cocotb {COCOTB} file; update GATES")
        ls = text.decode().split("\n")
        for n, line in lines.items():
            ls[n - 1] = line
        (out / f).write_text("\n".join(ls))
    # --run-args reach both the makefile and the Python runner through a wrapper of yavs
    if a.run_args:
        w = out / "yavs"
        w.write_text(f'#!/bin/sh\nif [ "$1" = run ]; then shift; exec "{yavs}" run {a.run_args} "$@"; fi\nexec "{yavs}" "$@"\n')
        w.chmod(0o755)
        yavs = w
    t = out / "tests"
    # (name, directory, command, JUnit files; None: the make results and the runner's results under sim_build)
    jobs = [(f"test_cases/{p.name}", p, ["make"], None) for p in sorted((t / "test_cases").iterdir()) if (p / "Makefile").exists()]
    jobs += [(f"examples/{e}", out / "examples" / e, ["make"], None) for e in EXAMPLES]
    py = ["pytest", "-p", "no:cacheprovider", "-v"]
    jobs += [("pytest", out, [*py, "-k", "simulator_required", "--junitxml=pytest.xml"], [out / "pytest.xml"]),
             ("pytest_plugin", out, [*py, "tests/pytest_plugin", "--cocotb-simulator", "yavs", "--cocotb-gpi-interfaces", "vhpi",
                                     "--cocotb-toplevel-lang", "vhdl", "--junitxml=plugin.xml"], [out / "plugin.xml"])]
    jobs += [(f"examples-pytest/{e}", out / "examples" / e, [*py, "--junitxml=pytest.xml"], [out / "examples" / e / "pytest.xml"])
             for e in PYTEST_EXAMPLES]
    if a.cases:
        jobs = [j for j in jobs if j[0] in a.cases or j[0].split("/")[-1] in a.cases]
    # cocotb's noxfile settings for every simulator test (configure_test_env)
    env = {**os.environ, "PYTHONWARNINGS": "error,ignore::DeprecationWarning:site,ignore:coroutine :RuntimeWarning",
           "COCOTB_SCHEDULER_DEBUG": "1", "PYTHON_VERSION": PYTHON, "GPI_DEBUG": "1", "PYGPI_DEBUG": "1", "PATH": f"{venv / 'bin'}{os.pathsep}{os.environ['PATH']}", "VIRTUAL_ENV": str(venv),
           "SIM": "yavs", "TOPLEVEL_LANG": "vhdl", "VHDL_GPI_INTERFACE": "vhpi", "YAVS_BIN": str(yavs),
           "YAVS_STD_CACHE": str(out / "std")}
    env.pop("PYTHONHOME", None)

    def one(j):
        name, cd, cmd, xml = j
        cap = [] if a.no_sandbox else ["systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=2G"]
        p = subprocess.run(["timeout", "300", *cap, "env", "-C", cd, f"PWD={cd}", *cmd],
                           env=env, text=True, capture_output=True, errors="replace")
        (out / f"{name.replace('/', '-')}.log").write_text(p.stdout + p.stderr)
        if xml is None:
            # cases that check their own expected failures delete results.xml and keep the combined file
            xml = [*cd.glob("results*.xml"), *cd.glob("sim_build/**/*.result.xml")] or [*cd.glob("combined_results.xml")]
        n, failed = counts(xml)
        return name, p.returncode, n, failed

    with cf.ThreadPoolExecutor(a.jobs) as ex:
        res = sorted(ex.map(one, jobs))
    bad = []
    for c, rc, (p, f, s), failed in res:
        ok = (rc == 0) != (c in EXPECTED_FAIL)
        bad += [] if ok else [c]
        print(f"{'PASS' if ok else 'FAIL'} {c} rc={rc} tests: {p} pass, {f} fail, {s} skip")
        if not ok:
            print(f"  failing tests: {' '.join(failed) or 'none recorded'}; log tail:")
            log = (out / f"{c.replace('/', '-')}.log").read_text().splitlines()
            print("\n".join("  | " + l for l in log[-40:]))
    t = [sum(r[2][i] for r in res) for i in range(3)]
    ran = [c for c, _, n, _ in res if n[0] + n[1]]
    print(f"cocotb {COCOTB}: {len(res) - len(bad)}/{len(res)} jobs pass ({len(ran)} ran tests, {len([c for c in ran if c not in bad])} of them pass; "
          f"the rest skip themselves: Verilog only or nothing to simulate); tests {t[0]} pass, {t[1]} fail (expected by their jobs), {t[2]} skip; logs in {out}")
    if a.json:
        a.json.write_text(json.dumps({"cocotb": COCOTB, "jobs": len(res), "pass": len(res) - len(bad), "failed": bad, "ran": len(ran),
                                      "tests": dict(zip(["pass", "fail", "skip"], t)), "run_args": a.run_args,
                                      "per_job": {c: {"rc": rc, "tests": n, "failed": f} for c, rc, n, f in res}}, indent=1, sort_keys=True))
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
