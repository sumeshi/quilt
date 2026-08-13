import unittest
from test_base import QuiltTestBase


class TestUniq(QuiltTestBase):
    fixture = "sample-min.csv"

    def test_uniq_string(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'select', 'Level', '-', 'sort', 'Level', '-', 'uniq', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "Level\nInfo\nLogAlways")

    def test_uniq_numeric(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'select', 'EventId', '-', 'sort', 'EventId', '-', 'uniq', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "EventId\n1102\n4688\n4689")


if __name__ == "__main__":
    unittest.main()
