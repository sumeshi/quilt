import unittest
from test_base import QuiltTestBase


class TestIsin(QuiltTestBase):
    fixture = "sample-min.csv"

    def test_isin_single_value(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'isin', 'EventId', '1102', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 2)

    def test_isin_multiple_values(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'isin', 'EventId', '4688,4689', '-', 'select', 'EventId', '-', 'count', '-', 'show']
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(set(result.stdout.strip().splitlines()[1:]), {"4688,14", "4689,14"})

    def test_isin_string_column(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'isin', 'Level', 'Info', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 2)

    def test_isin_no_match(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'isin', 'EventId', '9999', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 1)

    def test_isin_uint_and_float_values(self):
        fixture = str(self.get_fixture_path('cast.csv'))
        uint_result = self.run_pipeline(['load', fixture, '-', 'cast', 'number', 'uint', '-', 'isin', 'number', '2', '-', 'show'])
        float_result = self.run_pipeline(['load', fixture, '-', 'cast', 'number', 'float', '-', 'isin', 'number', '2', '-', 'show'])
        self.assertEqual(uint_result.returncode, 0, uint_result.stderr)
        self.assertEqual(float_result.returncode, 0, float_result.stderr)
        self.assertEqual(len(uint_result.stdout.strip().splitlines()), 2)
        self.assertEqual(len(float_result.stdout.strip().splitlines()), 2)


if __name__ == "__main__":
    unittest.main()
