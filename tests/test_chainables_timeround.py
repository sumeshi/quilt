import os
import tempfile
import unittest
from test_base import QsvTestBase


class TestTimeround(QsvTestBase):
    fixture = "sample-min.csv"

    def test_timeround_day(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - timeround TimeCreated --unit d --output day - select day - head 1 - show")
        self.assertEqual(result.stdout.strip(), "day\n2016-10-06")

    def test_timeround_hour(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - timeround TimeCreated --unit h --output hour - select hour - head 1 - show")
        self.assertEqual(result.stdout.strip(), "hour\n2016-10-06 01")

    def test_timeround_minute(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - timeround TimeCreated --unit m --output minute - select minute - head 1 - show")
        self.assertEqual(result.stdout.strip(), "minute\n2016-10-06 01:47")

    def test_timeround_second(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - timeround TimeCreated --unit s --output second - select second - head 1 - show")
        self.assertEqual(result.stdout.strip(), "second\n2016-10-06 01:47:07")

    def test_timeround_year(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - timeround TimeCreated --unit y --output yr - select yr - head 1 - show")
        self.assertEqual(result.stdout.strip(), "yr\n2016")

    def test_timeround_month(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - timeround TimeCreated --unit M --output mo - select mo - head 1 - show")
        self.assertEqual(result.stdout.strip(), "mo\n2016-10")

    def test_timeround_replace_original(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - timeround TimeCreated --unit d - select TimeCreated - head 1 - show")
        self.assertEqual(result.stdout.strip(), "TimeCreated\n2016-10-06")

    def test_timeround_invalid_unit(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - timeround TimeCreated --unit invalid - show")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Invalid time unit", result.stderr)

    def test_timeround_with_changetz_output(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - changetz TimeCreated --from-tz UTC --to-tz Asia/Tokyo - timeround TimeCreated --unit d --output day - select day - head 1 - show"
        )
        self.assertEqual(result.stdout.strip(), "day\n2016-10-06")

    def test_timeround_with_offset_string(self):
        fd, path = tempfile.mkstemp(suffix=".csv", text=True)
        os.close(fd)
        try:
            with open(path, "w", encoding="utf-8") as f:
                f.write("datetime,value\n2016-10-06 10:47:07 +09:00,1\n")
            result = self.run_qsv_command(
                f"load {path} - timeround datetime --unit h --output rounded - select rounded - head 1 - show"
            )
            self.assertEqual(result.stdout.strip(), "rounded\n2016-10-06 10")
        finally:
            if os.path.exists(path):
                os.remove(path)


if __name__ == "__main__":
    unittest.main()
