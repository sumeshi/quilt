import unittest
from test_base import QuiltTestBase


class TestSort(QuiltTestBase):
    fixture = "sample-min.csv"

    def test_sort_numeric_asc(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'sort', 'EventId', '-', 'head', '1', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertIn("1102,Info", result.stdout)

    def test_sort_numeric_desc(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'sort', 'EventId', '--desc', '-', 'head', '1', '-', 'show']
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn(",4689,LogAlways,", result.stdout)

    def test_sort_string_asc(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'sort', 'Level', '-', 'head', '1', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertIn(",1102,Info,", result.stdout)

    def test_sort_multiple_columns(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'sort', 'EventId,RecordNumber', '-', 'head', '3', '-', 'show']
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 4)


if __name__ == "__main__":
    unittest.main()
