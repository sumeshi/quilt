import unittest
from test_base import QsvTestBase


class TestShow(QsvTestBase):
    fixture = "sample-min.csv"

    def test_show_basic(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - show")
        self.assertEqual(result.returncode, 0)
        lines = result.stdout.splitlines()
        self.assertEqual(len(lines), 30)
        self.assertTrue(lines[0].startswith("RecordNumber,EventRecordId,TimeCreated"))

    def test_show_no_extra_newline(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - show")
        self.assertEqual(result.returncode, 0)
        self.assertFalse(result.stdout.endswith("\n\n"))

    def test_show_streaming_no_extra_newline(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - show --batch-size 1MB")
        self.assertEqual(result.returncode, 0)
        self.assertFalse(result.stdout.endswith("\n\n"))


if __name__ == "__main__":
    unittest.main()
