import unittest

from test_base import QuiltTestBase


class TestExtract(QuiltTestBase):
    fixture = "extract.csv"

    def test_multiple_groups_unicode_unmatched_and_null(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'extract', 'value', '(?P<user>[^@]+)@(?P<domain>.+)', '-', 'select', 'value,user,domain', '-', 'show']
        )
        self.assertEqual(result.returncode, 0)
        lines = result.stdout.strip().splitlines()
        self.assertEqual(lines[0], "value,user,domain")
        self.assertEqual(lines[1], "alice@example.com,alice,example.com")
        self.assertEqual(lines[2], "bob@例え.テスト,bob,例え.テスト")
        self.assertEqual(lines[3], "not-an-email,,")
        self.assertEqual(lines[4], ",,")
        self.assertEqual(lines[5], "carol@example.net,carol,example.net")

    def test_optional_groups_and_chaining(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path('extract-optional.csv')), '-', 'extract', 'value', '^(?P<country>\\+\\d+-)?(?P<number>\\d+-\\d+)$', '-', 'select', 'country,number', '-', 'head', '2', '-', 'show']
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(
            result.stdout.strip().splitlines(),
            ["country,number", "+1-,555-1234", ",555-1234"],
        )

    def test_failures(self):
        cases = [
            (
                ['load', str(self.get_fixture_path(self.fixture)), '-', 'extract', 'value', '(', '-', 'show'],
                "Invalid extract regex",
            ),
            (
                ['load', str(self.get_fixture_path(self.fixture)), '-', 'extract', 'value', 'email', '-', 'show'],
                "named capture group",
            ),
            (
                ['load', str(self.get_fixture_path(self.fixture)), '-', 'extract', 'missing', '(?P<x>.+)', '-', 'show'],
                "not found",
            ),
            (
                ['load', str(self.get_fixture_path('cast.csv')), '-', 'extract', 'number', '(?P<x>.+)', '-', 'show'],
                "must be string",
            ),
            (
                ['load', str(self.get_fixture_path('extract-collision.csv')), '-', 'extract', 'value', '(?P<domain>[^@]+)', '-', 'show'],
                "already exists",
            ),
        ]
        for command, message in cases:
            with self.subTest(command=command):
                result = self.run_pipeline(command)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(message, result.stderr)

    def test_run_step(self):
        result = self.run_pipeline(
            ['run', str(self.get_fixture_path('run-extract.yaml'))]
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(
            result.stdout.strip(),
            "value,user,domain\nalice@example.com,alice,example.com",
        )


if __name__ == "__main__":
    unittest.main()
