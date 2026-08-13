import unittest

from test_base import QuiltTestBase


class TestCommandSurface(QuiltTestBase):
    def test_global_help_matches_public_surface(self):
        result = self.run_pipeline(['--help'])
        self.assertEqual(result.returncode, 0, result.stderr)
        for command in (
            "load", "select", "cast", "parse-size", "bucket", "delta", "extract",
            "flatten", "isin", "contains", "sed", "grep", "head", "tail", "sort",
            "count", "uniq", "changetz", "renamecol", "timeslice", "calc",
            "partition", "headers", "stats", "showquery", "show", "showtable",
            "dump", "dumpcache", "run",
        ):
            with self.subTest(command=command):
                self.assertIn(command, result.stdout)

    def test_per_command_help_and_option_validation(self):
        for command, expected in (("calc", "--sum|--avg|--min|--max|--median|--std"), ("load", "JSONL/NDJSON")):
            result = self.run_pipeline([str(command), '--help'])
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(expected, result.stdout)

        unknown = self.run_pipeline(['calc', 'value', '--bogus'])
        self.assertNotEqual(unknown.returncode, 0)
        self.assertIn("Unknown option", unknown.stderr)

        arity = self.run_pipeline(
            ['load', str(self.get_fixture_path('sample-min.csv')), '-', 'flatten', 'extra']
        )
        self.assertNotEqual(arity.returncode, 0)
        self.assertIn("accepts no arguments", arity.stderr)

    def test_run_help_describes_automation_options(self):
        result = self.run_pipeline(["run", "--help"])
        self.assertEqual(result.returncode, 0, result.stderr)
        for option in ("--check", "--var", "--output", "--show-plan"):
            self.assertIn(option, result.stdout)

    def test_option_leading_positional_value_can_follow_double_dash(self):
        result = self.run_pipeline(
            ["load", str(self.get_fixture_path("sample-min.csv")), "-", "grep", "--", "-Info"]
        )
        self.assertEqual(result.returncode, 0, result.stderr)

        literal = self.run_pipeline(
            ["load", str(self.get_fixture_path("sample-min.csv")), "-", "grep", "--", "-"]
        )
        self.assertEqual(literal.returncode, 0, literal.stderr)

    def test_run_rejects_unknown_option(self):
        result = self.run_pipeline(
            ['run', str(self.get_fixture_path('run-simple.yaml')), '--bogus']
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Unknown option", result.stderr)


if __name__ == "__main__":
    unittest.main()
