import unittest
from test_base import QuiltTestBase


class TestHeaders(QuiltTestBase):
    fixture = "sample-min.csv"

    def test_headers_basic(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'headers', '--plain'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 27)

    def test_headers_contains_key_columns(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'headers', '--plain'])
        self.assertIn("TimeCreated", result.stdout)
        self.assertIn("EventId", result.stdout)
        self.assertIn("Level", result.stdout)
        self.assertIn("Payload", result.stdout)

    def test_headers_indices_are_one_based(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'headers'])
        self.assertEqual(result.returncode, 0)
        self.assertIn("│ 01 ┆ RecordNumber", result.stdout)
        self.assertIn("│ 27 ┆ Payload", result.stdout)
        self.assertNotIn("│ 00 ┆", result.stdout)


if __name__ == "__main__":
    unittest.main()
