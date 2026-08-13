import os
import shutil
import tempfile
import unittest
from test_base import QuiltTestBase


class TestPartition(QuiltTestBase):
    fixture = "sample-min.csv"

    def test_partition_by_string(self):
        parent = tempfile.mkdtemp()
        output_dir = os.path.join(parent, "partitions")
        try:
            result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'partition', 'Level', str(output_dir)])
            self.assertEqual(result.returncode, 0)
            with open(os.path.join(output_dir, "Info.csv"), "r", encoding="utf-8") as f:
                self.assertEqual(len(f.read().splitlines()), 2)
            with open(os.path.join(output_dir, "LogAlways.csv"), "r", encoding="utf-8") as f:
                self.assertEqual(len(f.read().splitlines()), 29)
        finally:
            shutil.rmtree(parent)

    def test_partition_by_numeric(self):
        parent = tempfile.mkdtemp()
        output_dir = os.path.join(parent, "partitions")
        try:
            result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'partition', 'EventId', str(output_dir)])
            self.assertEqual(result.returncode, 0)
            self.assertTrue(os.path.exists(os.path.join(output_dir, "1102.csv")))
            self.assertTrue(os.path.exists(os.path.join(output_dir, "4688.csv")))
            self.assertTrue(os.path.exists(os.path.join(output_dir, "4689.csv")))
        finally:
            shutil.rmtree(parent)

    def test_partition_nonexistent_column(self):
        parent = tempfile.mkdtemp()
        output_dir = os.path.join(parent, "partitions")
        try:
            result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'partition', 'NOSUCHCOL', str(output_dir)])
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Error:", result.stderr)
            self.assertNotIn("panicked at", result.stderr)
        finally:
            shutil.rmtree(parent)


if __name__ == "__main__":
    unittest.main()
