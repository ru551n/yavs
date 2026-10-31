# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this file,
# You can obtain one at http://mozilla.org/MPL/2.0/.

"""
The option page of the user guide (docs/vunit-yavs.rst) names every option of the package and
nothing else
"""

import argparse
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
        self.check("sim or compile options", defined, r"``(yavs\.[a-z_]+)``")

    def test_command_line_flags(self):
        parser = argparse.ArgumentParser()
        YavsInterface.add_arguments(parser)
        actions = parser._actions  # pylint: disable=protected-access
        defined = {s for a in actions for s in a.option_strings if s.startswith("--yavs-")}
        self.check("command-line flags", defined, r"``(--yavs-[a-z-]+)")

    def test_gui_flag(self):
        self.assertTrue(YavsInterface.supports_gui_flag)
        self.assertIn("``--gui``", self.text)
