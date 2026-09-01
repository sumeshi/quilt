import unittest
from test_base import QuiltTestBase


class TestHead(QuiltTestBase):
    fixture = "sample-min.csv"

    def test_head_basic(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'head', '3', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 4)

    def test_head_short_option_style(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'select', 'EventId,Level', '-', 'head', '-n', '2', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 3)

    def test_head_long_option_style(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'select', 'EventId,Level', '-', 'head', '--number', '2', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 3)

    def test_head_zero(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'head', '0', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 1)

    def test_head_over_length(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'head', '100', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 30)

    def test_head_large_value_does_not_wrap(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'head', '4294967297', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 30)

    def test_head_rejects_extra_argument(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'head', '1', '2', '-', 'show'])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("too many arguments", result.stderr)


if __name__ == "__main__":
    unittest.main()
