import unittest

from test_base import QsvTestBase


class TestCommandSurface(QsvTestBase):
    def test_global_help_matches_public_surface(self):
        result = self.run_qsv_command("--help")
        self.assertEqual(result.returncode, 0, result.stderr)
        for command in (
            "load", "select", "cast", "parse-size", "bucket", "delta", "extract",
            "flatten", "isin", "contains", "sed", "grep", "head", "tail", "sort",
            "count", "uniq", "changetz", "renamecol", "timeslice", "calc",
            "partition", "headers", "stats", "showquery", "show", "showtable",
            "dump", "dumpcache", "quilt", "sigma2quilt",
        ):
            with self.subTest(command=command):
                self.assertIn(command, result.stdout)
        for command in ("pivot", "timeline", "timeround", "convert"):
            with self.subTest(removed=command):
                self.assertNotIn(f"  {command}", result.stdout)

    def test_per_command_help_and_option_validation(self):
        for command, expected in (("calc", "--sum|--avg|--min|--max|--median|--std"), ("load", "JSONL/NDJSON")):
            result = self.run_qsv_command(f"{command} --help")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(expected, result.stdout)

        unknown = self.run_qsv_command("calc value --bogus")
        self.assertNotEqual(unknown.returncode, 0)
        self.assertIn("Unknown option", unknown.stderr)

        arity = self.run_qsv_command(
            f"load {self.get_fixture_path('sample-min.csv')} - flatten extra"
        )
        self.assertNotEqual(arity.returncode, 0)
        self.assertIn("accepts no arguments", arity.stderr)

    def test_removed_commands_are_unknown(self):
        for command in ("pivot", "timeline", "timeround", "convert"):
            with self.subTest(command=command):
                result = self.run_qsv_command(command)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("Unknown command", result.stderr)


if __name__ == "__main__":
    unittest.main()
