import unittest
from test_base import QsvTestBase


class TestIsin(QsvTestBase):
    fixture = "sample-min.csv"

    def test_isin_single_value(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - isin EventId 1102 - show")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 2)

    def test_isin_multiple_values(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - isin EventId 4688,4689 - select EventId - count - show"
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(set(result.stdout.strip().splitlines()[1:]), {"4688,14", "4689,14"})

    def test_isin_string_column(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - isin Level Info - show")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 2)

    def test_isin_no_match(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - isin EventId 9999 - show")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 1)


if __name__ == "__main__":
    unittest.main()
