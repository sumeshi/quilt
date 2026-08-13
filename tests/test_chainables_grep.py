import unittest
from test_base import QuiltTestBase


class TestGrep(QuiltTestBase):
    fixture = "sample-min.csv"

    def test_grep_basic(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'grep', 'Event log cleared', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 2)
        self.assertIn("1102,Info", result.stdout)

    def test_grep_partial(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'grep', 'new process has been created', '-', 'select', 'Level', '-', 'count', '-', 'show']
        )
        self.assertEqual(result.stdout.strip(), "Level,count\nLogAlways,14")

    def test_grep_no_match(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'grep', 'NOMATCH_XYZ', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 1)

    def test_grep_invert(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'grep', '-v', 'LogAlways', '-', 'select', 'Level', '-', 'count', '-', 'show']
        )
        self.assertEqual(result.stdout.strip(), "Level,count\nInfo,1")

    def test_grep_case_insensitive_short(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'grep', '-i', 'EVENT LOG CLEARED', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 2)

    def test_grep_case_insensitive_long(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'grep', '--ignore-case', 'EVENT LOG CLEARED', '-', 'show']
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 2)

    def test_grep_invert_case_insensitive(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'grep', '-v', '-i', 'logalways', '-', 'select', 'Level', '-', 'count', '-', 'show']
        )
        self.assertEqual(result.stdout.strip(), "Level,count\nInfo,1")

    def test_grep_column_single(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'grep', 'Eventlog', '--column', 'Provider', '-', 'show']
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 2)
        self.assertIn("Microsoft-Windows-Eventlog", result.stdout)

    def test_grep_column_multiple(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'grep', 'Eventlog|Info', '--column', 'Provider,Level', '-', 'show']
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 2)
        self.assertIn("1102,Info", result.stdout)


if __name__ == "__main__":
    unittest.main()
