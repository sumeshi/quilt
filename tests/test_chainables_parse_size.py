import unittest
import os
import tempfile

from test_base import QuiltTestBase


class TestParseSize(QuiltTestBase):
    fixture = "parse-size.csv"

    def test_all_units_and_decimal_values(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'parse-size', 'size', '-', 'show']
        )
        self.assertEqual(result.returncode, 0)
        values = [line.split(",")[0] for line in result.stdout.strip().splitlines()[1:]]
        self.assertEqual(
            values,
            ["1", "1000", "1000000", "1000000000", "1000000000000", "1024", "1048576", "1073741824", "1099511627776", "1500", "1536", "2000000"],
        )
        stats = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'parse-size', 'size', '-', 'select', 'size', '-', 'stats']
        )
        self.assertEqual(stats.returncode, 0)
        self.assertIn("null_count", stats.stdout)
        self.assertIn("1", stats.stdout)

    def test_parse_size_chains(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'parse-size', 'size', '-', 'cast', 'size', 'string', '-', 'select', 'size', '-', 'head', '2', '-', 'show']
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "size\n1\n1000")

    def test_parse_size_failures(self):
        cases = [
            ("parse-size-negative.csv", "negative"),
            ("parse-size-malformed.csv", "invalid"),
            ("parse-size-unknown-unit.csv", "unknown"),
            ("parse-size-fractional.csv", "fractional"),
            ("parse-size-overflow.csv", "overflow"),
        ]
        for fixture, message in cases:
            with self.subTest(fixture=fixture):
                result = self.run_pipeline(
                    ['load', str(self.get_fixture_path(fixture)), '-', 'parse-size', 'size', '-', 'show']
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(message, result.stderr.lower())

    def test_parse_size_missing_column(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'parse-size', 'missing', '-', 'show']
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not found", result.stderr)

    def test_parse_size_run_step(self):
        result = self.run_pipeline(
            ['run', str(self.get_fixture_path('run-parse-size.yaml'))]
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "size\n1\n1000")

    def test_large_lazy_chain_to_dump(self):
        """A 5,000-row lazy UDF chain reaches the sink without app-level materialization."""
        with tempfile.TemporaryDirectory() as directory:
            source = os.path.join(directory, "large.csv")
            output = os.path.join(directory, "large-out.csv")
            with open(source, "w", encoding="utf-8") as handle:
                handle.write("size\n")
                handle.write("1KB\n" * 5000)
            result = self.run_pipeline(
                ['load', str(source), '-', 'parse-size', 'size', '-', 'dump', '--output', str(output)]
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            with open(output, encoding="utf-8") as handle:
                rows = handle.read().splitlines()
            self.assertEqual(len(rows), 5001)
            self.assertEqual(rows[0], "size")
            self.assertEqual(rows[-1], "1000")


if __name__ == "__main__":
    unittest.main()
