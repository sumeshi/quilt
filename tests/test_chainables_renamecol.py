import unittest
from test_base import QuiltTestBase


class TestRenamecol(QuiltTestBase):
    fixture = "sample-min.csv"

    def test_renamecol_basic(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'select', 'EventId,Level', '-', 'renamecol', 'Level', 'severity', '-', 'head', '1', '-', 'show']
        )
        self.assertEqual(result.stdout.strip(), "EventId,severity\n1102,Info")

    def test_renamecol_nonexistent(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'renamecol', 'NOSUCHCOL', 'new_name', '-', 'show'])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Error:", result.stderr)

    def test_renamecol_rejects_destination_collision(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'renamecol', 'EventId', 'Level', '-', 'show'])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("destination column already exists", result.stderr)


if __name__ == "__main__":
    unittest.main()
