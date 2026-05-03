import unittest
from test_base import QsvTestBase


class TestStats(QsvTestBase):
    fixture = "sample-min.csv"

    def test_stats_basic(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - stats")
        self.assertEqual(result.returncode, 0)
        self.assertIn("RecordNumber", result.stdout)
        self.assertIn("min", result.stdout)
        self.assertIn("max", result.stdout)

    def test_stats_selected_columns(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - select EventId,Level - stats")
        self.assertEqual(result.returncode, 0)
        self.assertIn("EventId", result.stdout)
        self.assertIn("Level", result.stdout)


if __name__ == "__main__":
    unittest.main()
