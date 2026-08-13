import unittest
from test_base import QuiltTestBase


class TestStats(QuiltTestBase):
    fixture = "sample-min.csv"

    def test_stats_basic(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'stats'])
        self.assertEqual(result.returncode, 0)
        self.assertIn("RecordNumber", result.stdout)
        self.assertIn("min", result.stdout)
        self.assertIn("max", result.stdout)

    def test_stats_selected_columns(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'select', 'EventId,Level', '-', 'stats'])
        self.assertEqual(result.returncode, 0)
        self.assertIn("EventId", result.stdout)
        self.assertIn("Level", result.stdout)

    def test_stats_empty(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path('empty.csv')), '-', 'stats']
        )
        self.assertEqual(result.returncode, 0)

    def test_stats_null_contract(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path('delta.csv')), '-', 'stats']
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn("null_count", result.stdout)

    def test_stats_sample_standard_deviation_label(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path('sample-min.csv')), '-', 'stats']
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn("std", result.stdout)

    def test_stats_schema_order(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path('sample-min.csv')), '-', 'select', 'EventId,Level', '-', 'stats']
        )
        self.assertEqual(result.returncode, 0)
        self.assertLess(result.stdout.find("EventId"), result.stdout.find("Level"))

    def test_stats_nonnumeric_columns_do_not_panic(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path('sample-min.csv')), '-', 'select', 'Level', '-', 'stats']
        )
        self.assertEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
