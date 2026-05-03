import unittest
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


if __name__ == "__main__":
    unittest.main()
