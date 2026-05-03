import unittest
from test_base import QsvTestBase


class TestContains(QsvTestBase):
    fixture = "sample-min.csv"

    def test_contains_basic(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - contains MapDescription 'A new process' - select Level - count - show"
        )
        self.assertEqual(result.stdout.strip(), "Level,count\nLogAlways,14")

    def test_contains_no_match(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - contains Level 'NOMATCH' - show")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 1)

    def test_contains_ignorecase(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - contains Level -i 'info' - select Level - count - show"
        )
        self.assertEqual(result.stdout.strip(), "Level,count\nInfo,1")

    def test_contains_no_invert_option(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - contains Level 'Info' --invert - show"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Unknown option '--invert'", result.stderr)


if __name__ == "__main__":
    unittest.main()
