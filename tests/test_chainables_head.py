import unittest
from test_base import QsvTestBase


class TestHead(QsvTestBase):
    fixture = "sample-min.csv"

    def test_head_basic(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - head 3 - show")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 4)

    def test_head_short_option_style(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - select EventId,Level - head -n 2 - show")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 3)

    def test_head_long_option_style(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - select EventId,Level - head --number 2 - show")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 3)

    def test_head_zero(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - head 0 - show")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 1)

    def test_head_over_length(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - head 100 - show")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 30)


if __name__ == "__main__":
    unittest.main()
