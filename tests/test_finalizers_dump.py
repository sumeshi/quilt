import os
import tempfile
import unittest
from test_base import QsvTestBase


class TestDump(QsvTestBase):
    fixture = "sample-min.csv"

    def test_dump_basic(self):
        fd, output_file = tempfile.mkstemp(suffix=".csv")
        os.close(fd)
        try:
            result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - dump -o {output_file}")
            self.assertEqual(result.returncode, 0)
            self.assertTrue(os.path.exists(output_file))
            with open(output_file, "r", encoding="utf-8") as f:
                self.assertEqual(len(f.read().splitlines()), 30)
        finally:
            if os.path.exists(output_file):
                os.remove(output_file)

    def test_dump_streaming(self):
        fd, output_file = tempfile.mkstemp(suffix=".csv")
        os.close(fd)
        try:
            result = self.run_qsv_command(
                f"load {self.get_fixture_path(self.fixture)} - dump --batch-size 1MB -o {output_file}"
            )
            self.assertEqual(result.returncode, 0)
            with open(output_file, "r", encoding="utf-8") as f:
                self.assertEqual(len(f.read().splitlines()), 30)
        finally:
            if os.path.exists(output_file):
                os.remove(output_file)

    def test_dump_overwrites(self):
        fd, output_file = tempfile.mkstemp(suffix=".csv")
        os.close(fd)
        try:
            with open(output_file, "w", encoding="utf-8") as f:
                f.write("stale\n")
            first = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - head 1 - dump -o {output_file}")
            second = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - dump -o {output_file}")
            self.assertEqual(first.returncode, 0)
            self.assertEqual(second.returncode, 0)
            with open(output_file, "r", encoding="utf-8") as f:
                self.assertEqual(len(f.read().splitlines()), 30)
        finally:
            if os.path.exists(output_file):
                os.remove(output_file)


if __name__ == "__main__":
    unittest.main()
