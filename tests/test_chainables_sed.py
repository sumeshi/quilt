import unittest
from test_base import QuiltTestBase


class TestSed(QuiltTestBase):
    fixture = "sample-min.csv"

    def test_sed_basic(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'sed', 'Info', 'Information', '--column', 'Level', '-', 'select', 'Level', '-', 'count', '-', 'show']
        )
        self.assertEqual(result.stdout.strip(), "Level,count\nLogAlways,28\nInformation,1")

    def test_sed_no_match(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'sed', 'NOMATCH', 'REPLACED', '--column', 'Level', '-', 'select', 'Level', '-', 'count', '-', 'show']
        )
        self.assertEqual(result.stdout.strip(), "Level,count\nLogAlways,28\nInfo,1")

    def test_sed_all_columns(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'sed', 'LogAlways', 'REDACTED', '-', 'select', 'Level', '-', 'count', '-', 'show']
        )
        self.assertEqual(result.stdout.strip(), "Level,count\nREDACTED,28\nInfo,1")


if __name__ == "__main__":
    unittest.main()
