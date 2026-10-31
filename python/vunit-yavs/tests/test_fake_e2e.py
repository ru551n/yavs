# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this file,
# You can obtain one at http://mozilla.org/MPL/2.0/.
#
# Copyright (c) 2014-2026, Lars Asplund lars.anders.asplund@gmail.com

"""
End to end: the project's run.py adds the package and runs against the fake yavs
"""

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from xml.etree import ElementTree

from vunit_yavs import YAVS_MIN_VERSION

HERE = Path(__file__).parent
PASS = "lib.tb_yavs.test_pass"
COMMA = "lib.tb_yavs.test with, comma: and colon"
FAIL = "lib.tb_yavs.test_fail"


def report(xml_file):
    """Return {test name: status} of a JUnit report"""
    result = {}
    for case in ElementTree.parse(xml_file).iter("testcase"):
        status = (
            "failed"
            if case.find("failure") is not None
            else "skipped" if case.find("skipped") is not None else "passed"
        )
        result[f"{case.get('classname')}.{case.get('name')}"] = status
    return result


@unittest.skipIf(sys.platform == "win32", "The fake yavs is a Python script without an .exe wrapper")
class TestFakeYavs(unittest.TestCase):
    """
    Run a real run.py with VUNIT_SIMULATOR=yavs against the fake yavs
    """

    def _run(self, *args, fail="", version=str(YAVS_MIN_VERSION), env=None):
        """Run run.py and return the exit code, the output, the JUnit results and the logged yavs calls"""
        with tempfile.TemporaryDirectory() as tempdir:
            tempdir = Path(tempdir)
            log = tempdir / "yavs.log"
            env_vars = dict(env or {})
            if "RUN_PY_MERGE_COVERAGE" in env_vars:  # The merged database goes into the scratch directory
                env_vars["RUN_PY_MERGE_COVERAGE"] = str(tempdir / env_vars["RUN_PY_MERGE_COVERAGE"])
            env = dict(
                os.environ,
                VUNIT_SIMULATOR="yavs",
                VUNIT_YAVS_PATH=str(HERE / "fake_bin"),
                FAKE_YAVS_LOG=str(log),
                FAKE_YAVS_FAIL=fail,
                FAKE_YAVS_VERSION=version,
                **env_vars,
            )
            xml = tempdir / "x.xml"
            proc = subprocess.run(
                [sys.executable, str(HERE / "project" / "run.py"), "--clean", "-o", str(tempdir / "out with space")]
                + ["-x", str(xml)]
                + list(args),
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                timeout=120,
                check=False,
            )
            calls = [json.loads(line) for line in log.read_text(encoding="utf-8").splitlines()] if log.exists() else []
            results = report(xml) if xml.exists() else {}
        return proc.returncode, proc.stdout, results, calls

    @staticmethod
    def _runs(calls):
        return [cmd for cmd in calls if cmd[0] == "run"]

    def test_pass(self):
        exit_code, _, results, calls = self._run(PASS, COMMA)
        self.assertEqual(exit_code, 0)
        self.assertEqual(results, {PASS: "passed", COMMA: "passed"})
        self.assertTrue(any(cmd[0] == "analyze" and cmd[-1].endswith("tb_yavs.vhd") for cmd in calls))
        runs = self._runs(calls)
        self.assertEqual(len(runs), 2)
        self.assertIn("tb_yavs.tb", runs[0])
        self.assertIn("--exit-severity=error", runs[0])
        self.assertFalse(any(arg.startswith(("--threads", "--dap")) for arg in runs[0]))
        for cmd in calls:
            if cmd[0] in ("analyze", "elab", "run"):
                flags = [arg for arg in cmd if arg.startswith("--std-cache=")]
                self.assertEqual(len(flags), 1, cmd)
                self.assertRegex(flags[0], r"out with space[/\\]yavs[/\\]std_cache$")

    def test_fail(self):
        exit_code, _, results, _ = self._run(fail="test_fail")
        self.assertEqual(exit_code, 1)
        self.assertEqual(results, {PASS: "passed", COMMA: "passed", FAIL: "failed"})

    def test_elaborate_only(self):
        exit_code, _, _, calls = self._run("--elaborate", PASS)
        self.assertEqual(exit_code, 0)
        self.assertTrue(any(cmd[0] == "elab" for cmd in calls))
        self.assertFalse(self._runs(calls))

    def test_too_old_yavs_fails_before_compiling(self):
        exit_code, output, _, calls = self._run(PASS, version="0.8.2")
        self.assertNotEqual(exit_code, 0)
        self.assertIn("yavs 0.8.2 at ", output)
        self.assertIn(f"which needs yavs {YAVS_MIN_VERSION} or later", output)
        self.assertFalse([call for call in calls if call[0] == "analyze"])

    def test_max_threads(self):
        exit_code, _, _, calls = self._run("--yavs-max-threads=3", PASS)
        self.assertEqual(exit_code, 0)
        self.assertIn("--threads=3", self._runs(calls)[0])

    def test_token_server(self):
        exit_code, output, results, calls = self._run("-p", "2", "--yavs-jobserver")
        self.assertEqual(exit_code, 0, output)
        self.assertEqual(set(results.values()), {"passed"})
        self.assertEqual(len([call for call in calls if call[0] == "tokens"]), 3)
        # Every simulation may use the -p slots; a test running alone gets the one free slot of -p 2
        self.assertTrue(all("--threads=2" in run for run in self._runs(calls)))
        self.assertIn(["tokens", 1], calls)

    def test_no_token_server_without_the_flag(self):
        _, _, _, calls = self._run("-p", "2", PASS, COMMA)
        self.assertFalse([call for call in calls if call[0] == "tokens"])
        self.assertFalse(any("--threads=2" in run for run in self._runs(calls)))

    def test_coverage_is_collected_merged_and_reported(self):
        merged = "merged"
        env = {"RUN_PY_MERGE_COVERAGE": merged, "RUN_PY_COVERAGE_ARGS": "--cobertura x.xml"}
        exit_code, output, _, calls = self._run("--yavs-cov", "--yavs-cov-fail-under=80", PASS, COMMA, env=env)
        self.assertEqual(exit_code, 0, output)
        for run in self._runs(calls):
            self.assertTrue(any(arg.startswith("--cov=") and arg.endswith("coverage.ycov") for arg in run), run)
        (merge,) = [call for call in calls if call[:2] == ["cov", "merge"]]
        self.assertEqual(len(merge), 6)  # cov, merge, -o, OUT, and the two databases
        (report_call,) = [call for call in calls if call[:2] == ["cov", "report"]]
        self.assertEqual(report_call[3:], ["--cobertura", "x.xml", "--fail-under=80"])
        self.assertIn("fake report of ", output)

    def test_coverage_below_the_threshold_fails_the_run(self):
        env = {"RUN_PY_MERGE_COVERAGE": "merged", "FAKE_YAVS_COVERAGE": "70"}
        exit_code, output, results, _ = self._run("--yavs-cov", "--yavs-cov-fail-under=80", PASS, env=env)
        self.assertEqual(exit_code, 1, output)
        self.assertEqual(results, {PASS: "passed"})
        self.assertIn("coverage 70.0% is below the required 80.0%", output)

    def test_no_coverage_without_a_request(self):
        _, _, _, calls = self._run(PASS)
        self.assertFalse(any(arg.startswith("--cov") for run in self._runs(calls) for arg in run))

    def test_merging_without_coverage_is_a_clear_error(self):
        exit_code, output, _, _ = self._run(PASS, env={"RUN_PY_MERGE_COVERAGE": "merged"})
        self.assertEqual(exit_code, 1)
        self.assertIn("no simulation ran with coverage", output)

    def test_coverage_refuses_the_debugger(self):
        exit_code, output, _, calls = self._run("--yavs-debug=5000", "--yavs-cov", PASS)
        self.assertEqual(exit_code, 1)
        self.assertIn("--yavs-debug and coverage cannot be combined", output)
        self.assertFalse(self._runs(calls))

    def test_debug_one_test(self):
        exit_code, output, results, calls = self._run("--yavs-debug=5000", "--yavs-max-threads=3", PASS)
        self.assertEqual(exit_code, 0, output)
        self.assertEqual(results, {PASS: "passed"})
        (run,) = self._runs(calls)
        self.assertEqual(run[1], "--dap=5000")
        self.assertFalse(any(arg.startswith("--threads") for arg in run))
        self.assertIn(f"Debugging {PASS}: attach a DAP client to 127.0.0.1:5000", output)

    def test_debug_refuses_the_second_test(self):
        exit_code, output, results, calls = self._run("--yavs-debug=5000", PASS, COMMA)
        self.assertEqual(exit_code, 1)
        self.assertEqual(len(self._runs(calls)), 1)
        self.assertEqual(sorted(results.values()), ["failed", "passed"])
        self.assertIn("--yavs-debug runs one simulation", output)

    def test_debug_refuses_parallel_simulations(self):
        exit_code, output, _, calls = self._run("--yavs-debug", "-p", "2", PASS)
        self.assertNotEqual(exit_code, 0)
        self.assertIn("-p 2 asks for parallel simulations", output)
        self.assertFalse(self._runs(calls))

    def test_debug_refuses_elaborate(self):
        exit_code, output, _, calls = self._run(PASS, "--elaborate", "--yavs-debug")
        self.assertNotEqual(exit_code, 0)
        self.assertIn("--yavs-debug and --elaborate cannot be combined", output)
        self.assertFalse(any(cmd[0] == "elab" for cmd in calls))
