import unittest
from test_base import QsvTestBase


class TestRenamecol(QsvTestBase):
    fixture = "sample-min.csv"

    def test_renamecol_basic(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - select EventId,Level - renamecol Level severity - head 1 - show"
        )
        self.assertEqual(result.stdout.strip(), "EventId,severity\n1102,Info")

    def test_renamecol_nonexistent(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - renamecol NOSUCHCOL new_name - show")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Error:", result.stderr)


if __name__ == "__main__":
    unittest.main()
