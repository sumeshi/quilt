import unittest
from test_base import QuiltTestBase


class TestCount(QuiltTestBase):
    fixture = "sample-min.csv"

    def test_count_by_event_id(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'select', 'EventId', '-', 'count', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip().splitlines()[0], "EventId,count")
        self.assertEqual(set(result.stdout.strip().splitlines()[1:]), {"4688,14", "4689,14", "1102,1"})

    def test_count_by_level(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'select', 'Level', '-', 'count', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "Level,count\nLogAlways,28\nInfo,1")

    def test_count_single_column_argument(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'count', 'EventId', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip().splitlines()[0], "EventId,count")
        self.assertEqual(set(result.stdout.strip().splitlines()[1:]), {"4688,14", "4689,14", "1102,1"})

    def test_count_multiple_columns_comma_separated(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'count', 'EventId,Level', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        lines = result.stdout.strip().splitlines()
        self.assertEqual(lines[0], "EventId,Level,count")
        self.assertEqual(set(lines[1:]), {"4688,LogAlways,14", "4689,LogAlways,14", "1102,Info,1"})

    def test_count_rejects_extra_positional_argument(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'count', 'EventId', 'Level', '-', 'show'])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("too many arguments", result.stderr)

    def test_count_rejects_output_name_collision(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'renamecol', 'EventId', 'count', '-', 'count', 'count', '-', 'show'])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("conflicts with output column", result.stderr)


if __name__ == "__main__":
    unittest.main()
