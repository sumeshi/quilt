import unittest
from test_base import QsvTestBase


class TestTimeslice(QsvTestBase):
    fixture = "sample-min.csv"

    def test_timeslice_start_before_data(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - timeslice TimeCreated --start '2016-01-01 00:00:00' - select Level - count - show"
        )
        self.assertEqual(result.stdout.strip(), "Level,count\nLogAlways,28\nInfo,1")

    def test_timeslice_start_after_data(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - timeslice TimeCreated --start '2017-01-01 00:00:00' - show"
        )
        self.assertEqual(len(result.stdout.strip().splitlines()), 1)

    def test_timeslice_end_after_data(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - timeslice TimeCreated --end '2099-12-31 00:00:00' - select Level - count - show"
        )
        self.assertEqual(result.stdout.strip(), "Level,count\nLogAlways,28\nInfo,1")

    def test_timeslice_end_before_data(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - timeslice TimeCreated --end '2016-01-01 00:00:00' - show"
        )
        self.assertEqual(len(result.stdout.strip().splitlines()), 1)

    def test_timeslice_both_bounds_include_all(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - timeslice TimeCreated --start '2016-10-06 00:00:00' --end '2016-10-06 23:59:59' - select Level - count - show"
        )
        self.assertEqual(result.stdout.strip(), "Level,count\nLogAlways,28\nInfo,1")

    def test_timeslice_requires_bounds(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - timeslice TimeCreated - show")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("requires at least one of --start or --end", result.stderr)

    def test_timeslice_nonexistent_column(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - timeslice NOSUCHCOL --start '2016-01-01' - show"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Error", result.stderr)

    def test_timeslice_with_changetz_output(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - changetz TimeCreated --from-tz UTC --to-tz Asia/Tokyo - timeslice TimeCreated --start '2016-10-06 10:00:00' --end '2016-10-06 11:00:00' - head 1 - show"
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn("+09:00", result.stdout)


if __name__ == "__main__":
    unittest.main()
