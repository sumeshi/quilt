import unittest
from test_base import QuiltTestBase


class TestShowquery(QuiltTestBase):
    fixture = "sample-min.csv"

    def test_showquery_basic(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'select', 'EventId,Level', '-', 'showquery'])
        self.assertEqual(result.returncode, 0)
        self.assertIn("Logical query plan:", result.stdout)
        self.assertIn("sample-min.csv", result.stdout)


if __name__ == "__main__":
    unittest.main()
