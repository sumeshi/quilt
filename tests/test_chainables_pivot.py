import unittest
from test_base import QsvTestBase


class TestPivot(QsvTestBase):
    fixture = "sample-min.csv"

    def test_pivot_basic(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - pivot --rows EventId --cols Level --values EventRecordId --agg count - show"
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip().splitlines()[0], "EventId,Level,EventRecordId_count")
        self.assertEqual(
            set(result.stdout.strip().splitlines()[1:]),
            {"4688,LogAlways,14", "4689,LogAlways,14", "1102,Info,1"},
        )


if __name__ == "__main__":
    unittest.main()
