"""Verdicts must distinguish unsupported features from language errors."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("vests", Path(__file__).with_name("vests.py"))
vests = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vests)


class RejectionTests(unittest.TestCase):
    def test_limitations_never_satisfy_must_fail_tests(self):
        for code in ("E0199", "E1999", "E2001", "E2199", "E9999"):
            for kind in ("analysis-fails", "fails"):
                with self.subTest(code=code, kind=kind):
                    verdict, reason = vests.judge(
                        kind, "analysis", f"error[{code}]: unsupported", "test.vhd"
                    )
                    self.assertEqual(verdict, "FAIL")
                    self.assertIn("no real rejection", reason)

    def test_a_language_error_still_satisfies_a_must_fail_test(self):
        verdict, _ = vests.judge(
            "fails", "run", "warning[W3102]: no wait\nerror[E2107]: out of range", "test.vhd"
        )
        self.assertEqual(verdict, "PASS")


if __name__ == "__main__":
    unittest.main()
