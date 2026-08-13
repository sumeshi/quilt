import unittest
from test_base import QuiltTestBase


class TestShowtable(QuiltTestBase):
    fixture = "sample-min.csv"

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
