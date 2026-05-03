import re
import unittest
from test_base import QsvTestBase


class TestChangetz(QsvTestBase):
    fixture = "sample-min.csv"

    def test_changetz_basic(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - changetz TimeCreated --from-tz UTC --to-tz Asia/Tokyo - head 1 - show"
        )
        self.assertEqual(result.returncode, 0)
        self.assertRegex(result.stdout, r"2016-10-06T10:47:07\.\d{6}\+09:00")

    def test_changetz_microsecond_precision(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - changetz TimeCreated --from-tz UTC --to-tz Asia/Tokyo - head 1 - show"
        )
        self.assertEqual(result.returncode, 0)
        self.assertRegex(result.stdout, r"\.\d{6}\+09:00")
        self.assertNotRegex(result.stdout, r"\.\d{7}\+09:00")

    def test_changetz_invalid_tz(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - changetz TimeCreated --from-tz UTC --to-tz INVALID_ZONE - show"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Invalid target timezone", result.stderr)


if __name__ == "__main__":
    unittest.main()
