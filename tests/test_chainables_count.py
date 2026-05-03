import unittest
from test_base import QsvTestBase


class TestCount(QsvTestBase):
    fixture = "sample-min.csv"

    def test_count_by_event_id(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - select EventId - count - show")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip().splitlines()[0], "EventId,count")
        self.assertEqual(set(result.stdout.strip().splitlines()[1:]), {"4688,14", "4689,14", "1102,1"})

    def test_count_by_level(self):
        result = self.run_qsv_command(f"load {self.get_fixture_path(self.fixture)} - select Level - count - show")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "Level,count\nLogAlways,28\nInfo,1")


if __name__ == "__main__":
    unittest.main()
