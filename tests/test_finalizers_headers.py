import unittest
from test_base import QsvTestBase


class TestHeaders(QsvTestBase):
    fixture = "sample-min.csv"

    def test_headers_basic(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - headers --plain")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 27)

    def test_headers_contains_key_columns(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - headers --plain")
        self.assertIn("TimeCreated", result.stdout)
        self.assertIn("EventId", result.stdout)
        self.assertIn("Level", result.stdout)
        self.assertIn("Payload", result.stdout)


if __name__ == "__main__":
    unittest.main()
