import unittest
from pathlib import Path
from test_base import QuiltTestBase


class TestShowtable(QuiltTestBase):
    fixture = "sample-min.csv"

    def test_datetime_and_duration_are_human_readable(self):
        path = Path(self.temp_dir) / "times.csv"
        path.write_text("when,id\n2026-10-15T12:34:56Z,1\n2026-10-15T12:35:56Z,2\n,3\n")
        result = self.run_pipeline(["load", path], ["cast", "when", "datetime", "--timezone", "Asia/Tokyo"], ["delta", "when"], ["showtable"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("2026-10-15 21:34:56", result.stdout)
        self.assertIn("JST", result.stdout)
        self.assertIn("1m", result.stdout)
        self.assertIn("null", result.stdout)

    def test_showtable_basic(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'showtable'])
        self.assertEqual(result.returncode, 0)
        self.assertIn("shape: (8+, 27)", result.stdout)

    def test_showtable_with_select(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'select', 'EventId,Level', '-', 'showtable'])
        self.assertEqual(result.returncode, 0)
        self.assertIn("EventId", result.stdout)
        self.assertIn("Level", result.stdout)


if __name__ == "__main__":
    unittest.main()
