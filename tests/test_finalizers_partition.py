import os
import shutil
import tempfile
import unittest
from test_base import QsvTestBase


class TestPartition(QsvTestBase):
    fixture = "sample-min.csv"

    def test_partition_by_string(self):
        output_dir = tempfile.mkdtemp()
        try:
            result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - partition Level {output_dir}")
            self.assertEqual(result.returncode, 0)
            with open(os.path.join(output_dir, "Info.csv"), "r", encoding="utf-8") as f:
                self.assertEqual(len(f.read().splitlines()), 2)
            with open(os.path.join(output_dir, "LogAlways.csv"), "r", encoding="utf-8") as f:
                self.assertEqual(len(f.read().splitlines()), 29)
        finally:
            shutil.rmtree(output_dir)

    def test_partition_by_numeric(self):
        output_dir = tempfile.mkdtemp()
        try:
            result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - partition EventId {output_dir}")
            self.assertEqual(result.returncode, 0)
            self.assertTrue(os.path.exists(os.path.join(output_dir, "1102.csv")))
            self.assertTrue(os.path.exists(os.path.join(output_dir, "4688.csv")))
            self.assertTrue(os.path.exists(os.path.join(output_dir, "4689.csv")))
        finally:
            shutil.rmtree(output_dir)

    def test_partition_nonexistent_column(self):
        output_dir = tempfile.mkdtemp()
        try:
            result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - partition NOSUCHCOL {output_dir}")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Error:", result.stderr)
            self.assertNotIn("panicked at", result.stderr)
        finally:
            shutil.rmtree(output_dir)


if __name__ == "__main__":
    unittest.main()
