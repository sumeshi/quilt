import unittest
import subprocess
import os
from test_base import QsvTestBase


class TestLoad(QsvTestBase):
    def test_load_single_file(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path('sample-min.csv')} - show")
        self.assertEqual(result.returncode, 0)
        lines = result.stdout.strip().splitlines()
        self.assertEqual(lines[0].split(",")[:5], ["RecordNumber", "EventRecordId", "TimeCreated", "EventId", "Level"])
        self.assertEqual(len(lines), 30)

    def test_load_gzip_file(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path('sample-min.csv.gz')} - head 1 - show")
        self.assertEqual(result.returncode, 0)
        lines = result.stdout.strip().splitlines()
        self.assertEqual(len(lines), 2)
        self.assertIn("1102,Info", lines[1])

    def test_load_gzip_file_with_memory_limit_env(self):
        env = os.environ.copy()
        env["QSV_MEMORY_LIMIT_MB"] = "512"
        result = subprocess.run(
            f"{self.qsv_path} load {self.get_fixture_path('sample-min.csv.gz')} - head 1 - show",
            shell=True,
            capture_output=True,
            text=True,
            cwd=self.root_dir,
            env=env,
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn("1102,Info", result.stdout)

    def test_load_tsv_separator_short(self):
        separator = "\t"
        result = self.run_qsv_command(
            f"load {self.get_fixture_path('sample-min.tsv')} -s '{separator}' - select EventId,Level - head 1 - show"
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "EventId,Level\n1102,Info")

    def test_load_tsv_separator_long(self):
        separator = "\t"
        result = self.run_qsv_command(
            f"load {self.get_fixture_path('sample-min.tsv')} --separator '{separator}' - head 1 - show"
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 2)

    def test_load_multiple_files(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path('sample-min.csv')} {self.get_fixture_path('sample-min.csv')} - select Level - count - show"
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "Level,count\nLogAlways,56\nInfo,2")

    def test_load_glob_pattern(self):
        result = self.run_qsv_command(
            "load 'tests/fixtures/sample-min.c*sv' - head 1 - show"
        )
        self.assertEqual(result.returncode, 0)
        lines = result.stdout.strip().splitlines()
        self.assertEqual(len(lines), 2)
        self.assertEqual(lines[0].split(",")[:3], ["RecordNumber", "EventRecordId", "TimeCreated"])

    def test_load_low_memory(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path('sample-min.csv')} --low-memory - head 1 - show")
        self.assertEqual(result.returncode, 0)
        self.assertIn("1102,Info", result.stdout)

    def test_load_no_headers(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path('sample-min-noheader.csv')} --no-headers - select column_1,column_4,column_5 - head 1 - show"
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "column_1,column_4,column_5\n227126,1102,Info")

    def test_load_nonexistent_file(self):
        result = self.run_qsv_command("load non_existent_file.csv - show")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Error: File not found", result.stderr)

    def test_load_unmatched_glob_pattern(self):
        result = self.run_qsv_command("load 'tests/fixtures/does-not-exist-*.csv' - show")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("No files found matching pattern", result.stderr)


if __name__ == "__main__":
    unittest.main()
