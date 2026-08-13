import unittest
from test_base import QuiltTestBase


class TestTail(QuiltTestBase):
    fixture = "sample-min.csv"

    def test_tail_basic(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'tail', '3', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 4)
        self.assertIn("227154", result.stdout)

    def test_tail_short_option_style(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'select', 'EventId,Level', '-', 'tail', '-n', '2', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 3)

    def test_tail_long_option_style(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'select', 'EventId,Level', '-', 'tail', '--number', '2', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 3)

    def test_tail_zero(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'tail', '0', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 1)

    def test_tail_over_length(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'tail', '100', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 30)


if __name__ == "__main__":
    unittest.main()
