# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this file,
# You can obtain one at http://mozilla.org/MPL/2.0/.

"""
The option page of the user guide (docs/vunit-yavs.rst) names every option of the package and
nothing else
"""

import argparse
import json
import re
import unittest
from pathlib import Path

from vunit_yavs import YavsInterface

PAGE = Path(__file__).parents[3] / "docs" / "vunit-yavs.rst"


@unittest.skipUnless(PAGE.exists(), "Not run from the yavs repository")
class TestOptionPage(unittest.TestCase):
    """The page and the option definitions agree"""

    text = PAGE.read_text(encoding="utf-8") if PAGE.exists() else ""

    def check(self, what, defined, pattern):
        """The names `defined` are exactly those the page mentions by `pattern`"""
        documented = set(re.findall(pattern, self.text))
        self.assertEqual(
            set(),
            defined - documented,
            f"docs/vunit-yavs.rst does not document these {what}: write them in the page",
        )
        self.assertEqual(
            set(),
            documented - defined,
            f"docs/vunit-yavs.rst documents {what} the package does not have: remove them from the page",
        )

    def test_sim_and_compile_options(self):
        defined = {o.name for o in YavsInterface.sim_options + YavsInterface.compile_options}
        self.check("sim or compile options", defined, r"``(yavs\.[a-z_.]+)``")

    def test_command_line_flags(self):
        parser = argparse.ArgumentParser()
        YavsInterface.add_arguments(parser)
        actions = parser._actions  # pylint: disable=protected-access
        defined = {s for a in actions for s in a.option_strings if s not in ("-h", "--help")}
        self.check("command-line flags", defined, r"``(--yavs-[a-z-]+|-j|--jobs)\b")


TUTORIAL = Path(__file__).parents[3] / "docs" / "debugger-tutorial.rst"
DEMO = TUTORIAL.parent / "examples" / "fifo-debug"


@unittest.skipUnless(TUTORIAL.exists(), "Not run from the yavs repository")
class TestDebuggerTutorial(unittest.TestCase):
    """The files and lines the debugger tutorial names hold the text it says"""

    text = TUTORIAL.read_text(encoding="utf-8") if TUTORIAL.exists() else ""

    def test_every_named_line_holds_its_text(self):
        # Written in the page as: ``fifo.vhd`` line 40 (``next_count := next_count + 1;``)
        found = re.findall(r"``(\w+\.vhd)`` line (\d+) \(``([^`]+)``\)", self.text)
        self.assertEqual(3, len(found), "the page names three lines in the form checked here")
        for name, line, code in found:
            actual = (DEMO / name).read_text(encoding="utf-8").splitlines()[int(line) - 1]
            self.assertEqual(code, actual.strip(), f"{name} line {line} moved: fix the page")

    def test_lines_in_shown_output_hold_their_text(self):
        # Written in the page as: tb_fifo.vhd:20 (in output) or `break tb_fifo.vhd:62`
        expected = {
            "tb_fifo.vhd:20": "signal dout",
            "tb_fifo.vhd:17": "signal wr",
            "tb_fifo.vhd:35": "main : process",
            "tb_fifo.vhd:62": "check_equal(full",
        }
        self.assertEqual(
            set(expected), set(re.findall(r"\w+\.vhd:\d+", self.text)), "a line the page shows is not checked"
        )
        for ref, code in expected.items():
            name, line = ref.split(":")
            self.assertIn(code, (DEMO / name).read_text(encoding="utf-8").splitlines()[int(line) - 1], ref)

    def test_attach_needs_no_configuration(self):
        self.assertFalse((DEMO / ".vscode").exists())
        self.assertIn("yavs: Attach", self.text)
        self.assertIn("127.0.0.1:4711", self.text)

    def test_run_script_does_not_point_at_a_local_binary(self):
        script = (DEMO / "run.py").read_text(encoding="utf-8")
        self.assertIn('setdefault("VUNIT_SIMULATOR", "yavs")', script)
        self.assertNotIn('environ["VUNIT_YAVS_PATH"]', script)
        self.assertNotIn('setdefault("VUNIT_YAVS_PATH"', script)
        self.assertNotIn('ROOT / "bin"', script)
