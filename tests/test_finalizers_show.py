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

    def test_show_streaming_matches_show(self):
        plain = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - show")
        streaming = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - show --batch-size 1MB"
        )
        self.assertEqual(plain.returncode, 0)
        self.assertEqual(streaming.returncode, 0)
        self.assertEqual(streaming.stdout, plain.stdout)

    def test_implicit_finalizer_uses_show_when_stdout_is_not_tty(self):
        explicit = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - show")
        implicit = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)}")
        self.assertEqual(explicit.returncode, 0)
        self.assertEqual(implicit.returncode, 0)
        self.assertEqual(implicit.stdout, explicit.stdout)


if __name__ == "__main__":
    unittest.main()
