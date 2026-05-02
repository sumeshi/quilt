import unittest
import os
import tempfile
from test_base import QsvTestBase

class TestTimeround(QsvTestBase):
    
    def test_timeround_day_unit(self):
        """Test timeround with day unit"""
        result = self.run_qsv_command(f"load {self.get_fixture_path('comprehensive.csv')} - timeround timestamp --unit d --output date_only - select id,date_only,value - head 3 - show")
        self.assertEqual(result.returncode, 0)
        lines = result.stdout.strip().split('\n')
        self.assertTrue(len(lines) >= 2)
        # Check header contains expected columns
        self.assertIn('date_only', lines[0])
        # Check data format (should be YYYY-MM-DD)
        self.assertIn('2023-01-01', result.stdout)

    def test_timeround_hour_unit(self):
        """Test timeround with hour unit"""
        result = self.run_qsv_command(f"load {self.get_fixture_path('comprehensive.csv')} - timeround timestamp --unit h --output hour_rounded - select id,hour_rounded,value - head 3 - show")
        self.assertEqual(result.returncode, 0)
        lines = result.stdout.strip().split('\n')
        self.assertTrue(len(lines) >= 2)
        self.assertIn('hour_rounded', lines[0])
        # Check hour format (should be YYYY-MM-DD HH)
        self.assertIn('2023-01-01 12', result.stdout)
        self.assertIn('2023-01-01 15', result.stdout)

    def test_timeround_minute_unit(self):
        """Test timeround with minute unit"""
        result = self.run_qsv_command(f"load {self.get_fixture_path('comprehensive.csv')} - timeround timestamp --unit m --output minute_rounded - show")
        self.assertEqual(result.returncode, 0)
        lines = result.stdout.strip().split('\n')
        self.assertTrue(len(lines) >= 2)
        self.assertIn('minute_rounded', lines[0])
        # Check minute format (should be YYYY-MM-DD HH:MM)
        self.assertIn('2023-01-01 12:34', result.stdout)

    def test_timeround_year_unit(self):
        """Test timeround with year unit"""
        result = self.run_qsv_command(f"load {self.get_fixture_path('comprehensive.csv')} - timeround timestamp --unit y --output year_only - show")
        self.assertEqual(result.returncode, 0)
        lines = result.stdout.strip().split('\n')
        self.assertTrue(len(lines) >= 2)
        self.assertIn('year_only', lines[0])
        # Check year format (should be YYYY)
        self.assertIn(',2023', result.stdout)  # CSV format, so comma before year

    def test_timeround_replace_original(self):
        """Test timeround without --output (replaces original column)"""
        result = self.run_qsv_command(f"load {self.get_fixture_path('comprehensive.csv')} - timeround timestamp --unit d - show")
        self.assertEqual(result.returncode, 0)
        lines = result.stdout.strip().split('\n')
        self.assertTrue(len(lines) >= 2)
        # Check that timestamp column exists (should be replaced)
        self.assertIn('timestamp', lines[0])
        # Check that the values are day-formatted (should be YYYY-MM-DD)
        self.assertIn('2023-01-01', result.stdout)

    def test_timeround_missing_unit(self):
        """Test timeround without --unit option should fail"""
        result = self.run_qsv_command(f"load {self.get_fixture_path('comprehensive.csv')} - timeround timestamp - show")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Error:", result.stderr)
        self.assertIn("--unit", result.stderr)

    def test_timeround_missing_column(self):
        """Test timeround without column name should fail"""
        result = self.run_qsv_command(f"load {self.get_fixture_path('comprehensive.csv')} - timeround --unit d - show")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Error:", result.stderr)

    def test_timeround_invalid_unit(self):
        """Test timeround with invalid unit should fail"""
        result = self.run_qsv_command(f"load {self.get_fixture_path('comprehensive.csv')} - timeround timestamp --unit invalid - show")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Error:", result.stderr)
        self.assertIn("Invalid time unit", result.stderr)

    def test_timeround_chaining(self):
        """Test timeround can be chained with other operations"""
        result = self.run_qsv_command(f"load {self.get_fixture_path('comprehensive.csv')} - timeround timestamp --unit d --output date_only - select id,date_only,value - head 3 - show")
        self.assertEqual(result.returncode, 0)
        lines = result.stdout.strip().split('\n')
        self.assertEqual(len(lines), 4)  # Header + 3 data rows
        self.assertEqual(lines[0], "id,date_only,value")

    def test_timeround_with_changetz_output(self):
        """Test timeround can parse timezone-aware changetz output"""
        result = self.run_qsv_command(
            f"load {self.get_fixture_path('simple.csv')} - changetz datetime --from-tz UTC --to-tz Asia/Tokyo - timeround datetime --unit d --output day - select day,col1 - show"
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "\n".join([
            "day,col1",
            "2023-01-01,1",
            "2023-01-01,4",
            "2023-01-01,7",
        ]))

    def test_timeround_with_offset_string(self):
        """Test timeround can parse non-RFC3339 timezone offset strings"""
        with tempfile.NamedTemporaryFile("w", suffix=".csv", delete=False) as f:
            f.write("datetime,value\n")
            f.write("2023-01-01 09:15:30 +09:00,1\n")
            f.write("2023-01-01 10:45:30 +09:00,2\n")
            path = f.name

        try:
            result = self.run_qsv_command(
                f"load {path} - timeround datetime --unit h --output hour - select hour,value - show"
            )
            self.assertEqual(result.returncode, 0)
            self.assertEqual(result.stdout.strip(), "\n".join([
                "hour,value",
                "2023-01-01 09,1",
                "2023-01-01 10,2",
            ]))
        finally:
            os.remove(path)

if __name__ == "__main__":
    unittest.main() 
