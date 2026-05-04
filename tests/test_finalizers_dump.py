import os
import tempfile
import unittest
from pathlib import Path
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

    def test_dump_streaming_matches_dump(self):
        fd_plain, plain_output = tempfile.mkstemp(suffix=".csv")
        fd_stream, stream_output = tempfile.mkstemp(suffix=".csv")
        os.close(fd_plain)
        os.close(fd_stream)
        try:
            plain = self.run_qsv_command(
                f"load {self.get_fixture_path(self.fixture)} - dump -o {plain_output}"
            )
            streaming = self.run_qsv_command(
                f"load {self.get_fixture_path(self.fixture)} - dump --batch-size 1MB -o {stream_output}"
            )
            self.assertEqual(plain.returncode, 0)
            self.assertEqual(streaming.returncode, 0)
            with open(plain_output, "r", encoding="utf-8") as plain_file:
                with open(stream_output, "r", encoding="utf-8") as streaming_file:
                    self.assertEqual(streaming_file.read(), plain_file.read())
        finally:
            if os.path.exists(plain_output):
                os.remove(plain_output)
            if os.path.exists(stream_output):
                os.remove(stream_output)

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

    def test_dump_rejects_multi_character_separator(self):
        fd, output_file = tempfile.mkstemp(suffix=".csv")
        os.close(fd)
        try:
            result = self.run_qsv_command(
                f"load {self.get_fixture_path(self.fixture)} - dump --separator '||' -o {output_file}"
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Separator must be a single ASCII character", result.stderr)
        finally:
            if os.path.exists(output_file):
                os.remove(output_file)

    def test_dump_rejects_stdout_output_equals(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - dump --output=-"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("requires a file path", result.stderr)

    def test_dump_rejects_stdout_output_space(self):
        existing = {path.name for path in Path(self.root_dir).glob('dump_*.csv')}
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - dump --output -"
        )
        current = {path.name for path in Path(self.root_dir).glob('dump_*.csv')}

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("requires a file path", result.stderr)
        self.assertEqual(existing, current)

    def test_dump_missing_directory_exits_nonzero(self):
        temp_dir = tempfile.mkdtemp(prefix="qsv-missing-dir-")
        os.rmdir(temp_dir)
        output_file = os.path.join(temp_dir, "out.csv")
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - dump --output {output_file}"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Failed to create file", result.stderr)


if __name__ == "__main__":
    unittest.main()
