import unittest
from test_base import QuiltTestBase


class TestSelect(QuiltTestBase):
    fixture = "sample-min.csv"

    def test_select_single_column(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'select', 'Level', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        lines = result.stdout.strip().splitlines()
        self.assertEqual(lines[0], "Level")
        self.assertEqual(len(lines), 30)

    def test_select_multiple_columns(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'select', 'EventId,Level', '-', 'head', '1', '-', 'show'])
        self.assertEqual(result.stdout.strip(), "EventId,Level\n1102,Info")

    def test_select_range_colon(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'select', 'EventId:Level', '-', 'head', '1', '-', 'show'])
        self.assertEqual(result.stdout.strip(), "EventId,Level\n1102,Info")

    def test_select_range_hyphen(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'select', 'EventId-Level', '-', 'head', '1', '-', 'show'])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Column 'EventId-Level' not found", result.stderr)

    def test_select_hyphen_prefers_exact_column_name(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path('select-hyphen-literal.csv')), '-', 'select', 'col1-col3', '-', 'show']
        )
        self.assertEqual(result.stdout.strip(), "col1-col3\nexact_value")

    def test_select_hyphen_expands_range_when_no_exact_match(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path('select-hyphen-range.csv')), '-', 'select', 'col1-col3', '-', 'show']
        )
        self.assertEqual(result.stdout.strip(), "col1,col2,col3\na,b,c")

    def test_select_numeric_index(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'select', '4', '-', 'head', '1', '-', 'show'])
        self.assertEqual(result.stdout.strip(), "EventId\n1102")

    def test_select_numeric_range(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'select', '4:5', '-', 'head', '1', '-', 'show'])
        self.assertEqual(result.stdout.strip(), "EventId,Level\n1102,Info")

    def test_select_quoted_colon_range(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path('select-colon-range.csv')), '-', 'select', '"col:1":"col:3"', '-', 'show']
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), 'col:1,middle,col:3\na,b,c')

    def test_select_mixed(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'select', '4,Level', '-', 'head', '1', '-', 'show'])
        self.assertEqual(result.stdout.strip(), "EventId,Level\n1102,Info")

    def test_select_nonexistent_column(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'select', 'NOSUCHCOL', '-', 'show'])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Error:", result.stderr)

    def test_select_oversized_hyphen_range_is_clean_error(self):
        result = self.run_pipeline(
            [
                "load",
                str(self.get_fixture_path(self.fixture)),
                "-",
                "select",
                "col999999999999999999999999999999-100000000000000000000000000000",
                "-",
                "show",
            ]
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("panicked", result.stderr.lower())
        self.assertIn("invalid range", result.stderr.lower())

    def test_select_large_colon_range_does_not_expand_before_schema_check(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'select', 'col1:col1000000000', '-', 'show']
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not found", result.stderr.lower())


if __name__ == "__main__":
    unittest.main()
