import unittest
from test_base import QsvTestBase


class TestShowquery(QsvTestBase):
    fixture = "sample-min.csv"

    def test_showquery_basic(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - select EventId,Level - showquery")
        self.assertEqual(result.returncode, 0)
        self.assertIn("Logical query plan:", result.stdout)
        self.assertIn("sample-min.csv", result.stdout)


if __name__ == "__main__":
    unittest.main()
