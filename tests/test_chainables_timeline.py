import unittest
from test_base import QsvTestBase


class TestTimeline(QsvTestBase):
    fixture = "sample-min.csv"

    def test_timeline_count_by_second(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - timeline TimeCreated --interval 1s - show")
        self.assertEqual(result.stdout.strip(), "timeline_1s,count\n2016-10-06 01:47:07,29")

    def test_timeline_count_by_minute(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - timeline TimeCreated --interval 1m - show")
        self.assertEqual(result.stdout.strip(), "timeline_1m,count\n2016-10-06 01:47:00,29")

    def test_timeline_sum(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - timeline TimeCreated --interval 1m --sum EventId - show"
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn("sum_EventId", result.stdout)
        self.assertIn("132380.0", result.stdout)


if __name__ == "__main__":
    unittest.main()
