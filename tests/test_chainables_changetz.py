import re
import unittest
from test_base import QuiltTestBase


class TestChangetz(QuiltTestBase):
    fixture = "sample-min.csv"

    def test_changetz_basic(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'changetz', 'TimeCreated', '--from-tz', 'UTC', '--to-tz', 'Asia/Tokyo', '-', 'head', '1', '-', 'show']
        )
        self.assertEqual(result.returncode, 0)
        self.assertRegex(result.stdout, r"2016-10-06T10:47:07\.\d{6}\+09:00")

    def test_changetz_microsecond_precision(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'changetz', 'TimeCreated', '--from-tz', 'UTC', '--to-tz', 'Asia/Tokyo', '-', 'head', '1', '-', 'show']
        )
        self.assertEqual(result.returncode, 0)
        self.assertRegex(result.stdout, r"\.\d{6}\+09:00")
        self.assertNotRegex(result.stdout, r"\.\d{7}\+09:00")

    def test_changetz_invalid_tz(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'changetz', 'TimeCreated', '--from-tz', 'UTC', '--to-tz', 'INVALID_ZONE', '-', 'show']
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Invalid target timezone", result.stderr)

    def test_changetz_accepts_datetime_column(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path('cast.csv')), '-', 'cast', 'when', 'datetime', '-', 'changetz', 'when', '--from-tz', 'UTC', '--to-tz', 'Asia/Tokyo', '-', 'head', '1', '-', 'show']
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertRegex(result.stdout, r"2023-01-02T12:04:05\.123456\+09:00")


if __name__ == "__main__":
    unittest.main()
