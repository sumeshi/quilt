import csv
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

from test_base import QuiltTestBase


class TestDatetimeContract(QuiltTestBase):
    """Real-binary checks that CLI and run use the same datetime payload."""

    def run_case(self, header, rows, cli_step, run_step):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            data = root / "input.csv"
            with data.open("w", newline="") as stream:
                writer = csv.writer(stream)
                writer.writerow(header)
                writer.writerows(rows)
            config = root / "pipeline.yaml"
            config.write_text(
                "version: 1\n"
                "title: datetime contract\n"
                "stages:\n"
                "- name: process\n"
                "  steps:\n"
                f"  - load:\n      paths: [{data}]\n"
                f"  - {run_step}\n"
                "  - show: {}\n"
            )
            cli = self.run_pipeline(
                ["load", str(data), "-", *cli_step.replace('"', "").replace("'", "").split(), "-", "show"]
            )
            run = self.run_pipeline(['run', str(config)])
            return cli, run

    def test_cast_bucket_and_timeslice_cli_run_identity(self):
        cli, run = self.run_case(
            ["when"],
            [["02/Jan/2024:03:04:05"]],
            'cast when datetime --input-format "%d/%b/%Y:%H:%M:%S"',
            'cast:\n      column: when\n      type: datetime\n      input-format: "%d/%b/%Y:%H:%M:%S"',
        )
        self.assertEqual(cli.returncode, 0)
        self.assertEqual(run.returncode, 0)
        self.assertEqual(cli.stdout, run.stdout)

        cli, run = self.run_case(
            ["when"], [["01/02/2024"]],
            "cast when datetime",
            "cast:\n      column: when\n      type: datetime",
        )
        self.assertNotEqual(cli.returncode, 0)
        self.assertNotEqual(run.returncode, 0)
        self.assertIn("ambiguous", cli.stderr)
        self.assertIn("ambiguous", run.stderr)

        cli, run = self.run_case(
            ["when"], [["2024-01-02T03:04:05Z"], [""]],
            "bucket when 1h",
            "bucket:\n      column: when\n      interval: 1h",
        )
        self.assertEqual(cli.returncode, 0)
        self.assertEqual(run.returncode, 0)
        self.assertEqual(cli.stdout, run.stdout)
        self.assertTrue(cli.stdout.rstrip().endswith(","))
        self.assertIn("2024-01-02T03:00:00.000000", cli.stdout)

        cli, run = self.run_case(
            ["when"], [["2024-01-02T03:04:05+09:00"]],
            "bucket when 1h --timezone America/New_York",
            "bucket:\n      column: when\n      interval: 1h\n      timezone: America/New_York",
        )
        self.assertEqual(cli.returncode, 0)
        self.assertEqual(run.returncode, 0)
        self.assertEqual(cli.stdout, run.stdout)
        self.assertIn("2024-01-01T13:00:00.000000-0500", cli.stdout)

        cli, run = self.run_case(
            ["when"],
            [["2024-01-02T03:04:05+09:00"], ["2024-01-02 03:00:00"]],
            "timeslice when --start '2024-01-01T00:00:00Z' --end '2024-01-02T00:00:00Z'",
            "timeslice:\n      column: when\n      start: '2024-01-01T00:00:00Z'\n      end: '2024-01-02T00:00:00Z'",
        )
        self.assertEqual(cli.returncode, 0)
        self.assertEqual(run.returncode, 0)
        self.assertEqual(cli.stdout, run.stdout)

        for step, run_step in (
            ("changetz when --from-tz UTC --to-tz Asia/Tokyo", "changetz:\n      column: when\n      from-tz: UTC\n      to-tz: Asia/Tokyo"),
            ("timeslice when --start '2024-01-01'", "timeslice:\n      column: when\n      start: '2024-01-01'"),
        ):
            cli, run = self.run_case(["when"], [["2024-01-02T03:04:05Z"], [""]], step, run_step)
            self.assertEqual(cli.returncode, 0)
            self.assertEqual(run.returncode, 0)
            self.assertEqual(cli.stdout, run.stdout)

        cli, run = self.run_case(
            ["when"],
            [["2024-01-02T03:04:05+09:00"]],
            'bucket when 1h --input-format "%Y-%m-%dT%H:%M:%S%:z"',
            'bucket:\n      column: when\n      interval: 1h\n      input-format: "%Y-%m-%dT%H:%M:%S%:z"',
        )
        self.assertEqual(cli.returncode, 0)
        self.assertEqual(run.returncode, 0)
        self.assertEqual(cli.stdout, run.stdout)

    def test_yaml_strict_false_matches_omitted_and_true_rejects_fuzzy_input(self):
        value = "January 2, 2024 3:04 AM"
        cli, run_false = self.run_case(
            ["when"],
            [[value]],
            "cast when datetime",
            "cast:\n      column: when\n      type: datetime\n      strict: false",
        )
        self.assertEqual(cli.returncode, 0)
        self.assertEqual(run_false.returncode, 0)
        self.assertEqual(cli.stdout, run_false.stdout)

        cli_strict, run_true = self.run_case(
            ["when"],
            [[value]],
            "cast when datetime --strict",
            "cast:\n      column: when\n      type: datetime\n      strict: true",
        )
        self.assertNotEqual(cli_strict.returncode, 0)
        self.assertNotEqual(run_true.returncode, 0)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            data = root / "typed.csv"
            data.write_text("when\n2024-01-02 03:04:05\n")
            for datetime_step, expected in (
                (
                    "  - bucket:\n"
                    "      column: when\n"
                    "      interval: 1h\n"
                    "      strict: false\n",
                    "parsing options apply only to string input",
                ),
                (
                    "  - timeslice:\n"
                    "      column: when\n"
                    "      start: '2024-01-01'\n"
                    "      strict: false\n",
                    "parsing options apply only to string timeslice input",
                ),
            ):
                config = root / ("bucket.yaml" if "bucket" in datetime_step else "timeslice.yaml")
                config.write_text(
                    "version: 1\n"
                    "stages:\n"
                    "- name: typed\n"
                    "  steps:\n"
                    f"  - load:\n      paths: [{data}]\n"
                    "  - cast:\n      column: when\n      type: datetime\n"
                    f"{datetime_step}"
                    "  - show: {}\n"
                )
                result = self.run_pipeline(['run', str(config)])
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(expected, result.stderr)

    def test_changetz_dst_policies_and_offset_authority(self):
        for policy in ("earliest", "latest"):
            cli, run = self.run_case(
                ["when"], [["2024-11-03 01:30:00"]],
                f"changetz when --from-tz America/New_York --to-tz UTC --ambiguous {policy}",
                f"changetz:\n      column: when\n      from-tz: America/New_York\n      to-tz: UTC\n      ambiguous: {policy}",
            )
            self.assertEqual(cli.returncode, 0)
            self.assertEqual(run.returncode, 0)
            self.assertEqual(cli.stdout, run.stdout)
            if policy == "earliest":
                self.assertIn("05:30:00.000000+00:00", cli.stdout)
            else:
                self.assertIn("06:30:00.000000+00:00", cli.stdout)
        cli, run = self.run_case(
            ["when"], [["2024-03-10 02:30:00"]],
            "changetz when --from-tz America/New_York --to-tz UTC --nonexistent shift-forward",
            "changetz:\n      column: when\n      from-tz: America/New_York\n      to-tz: UTC\n      nonexistent: shift-forward",
        )
        self.assertEqual(cli.returncode, 0)
        self.assertEqual(run.returncode, 0)
        self.assertEqual(cli.stdout, run.stdout)
        self.assertIn("07:00:00.000000+00:00", cli.stdout)

        with tempfile.TemporaryDirectory() as tmp:
            data = Path(tmp) / "gap.csv"
            data.write_text("when\n2024-03-10 02:30:00\n")
            result = self.run_pipeline(
                ['load', str(data), '-', 'changetz', 'when', '--from-tz', 'America/New_York', '--to-tz', 'UTC', '--nonexistent', 'error', '-', 'show']
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("nonexistent", result.stderr)
            result = self.run_pipeline(
                ['load', str(data), '-', 'changetz', 'when', '--from-tz', 'America/New_York', '--to-tz', 'UTC', '--nonexistent', 'shift-backward', '-', 'show']
            )
        self.assertEqual(result.returncode, 0)
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path('cast.csv')), '-', 'changetz', 'when', '--from-tz', 'America/New_York', '--to-tz', 'UTC', '--ambiguous', 'invalid', '-', 'show']
        )
        self.assertNotEqual(result.returncode, 0)
        result = self.run_pipeline(
            ['load', '/definitely/missing.csv', '-', 'timeslice', 'when', '--start', '2024-01-01', '--ambiguous', 'invalid', '-', 'show']
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ambiguous policy", result.stderr)

    def test_strict_epoch_and_redacted_lazy_errors(self):
        expected = {"s": "1969-12-31T23:59:59", "ms": "1969-12-31T23:59:59.999", "us": "1969-12-31T23:59:59.999999", "ns": "1969-12-31T23:59:59.999999"}
        for unit, values in (("s", ["-1", "1"]), ("ms", ["-1", "1"]), ("us", ["-1", "1"]), ("ns", ["-1000", "1000"])):
            cli, run = self.run_case(
                ["when"], [[value] for value in values], f"cast when datetime --epoch-unit {unit}",
                f"cast:\n      column: when\n      type: datetime\n      epoch-unit: {unit}",
            )
            self.assertEqual(cli.returncode, 0)
            self.assertEqual(run.returncode, 0)
            self.assertEqual(cli.stdout, run.stdout)
            self.assertIn(expected[unit], cli.stdout)
        with tempfile.TemporaryDirectory() as tmp:
            data = Path(tmp) / "fuzzy.csv"
            data.write_text("when\n01/02/2024\n")
            result = self.run_pipeline(
                ['load', str(data), '-', 'cast', 'when', 'datetime', '--strict', '-', 'show']
            )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("strict", result.stderr)
        self.assertNotIn("01/02/2024", result.stderr)

        with tempfile.TemporaryDirectory() as tmp:
            data = Path(tmp) / "invalid.csv"
            data.write_text("when\nnot-a-date\n")
            result = self.run_pipeline(
                ['load', str(data), '-', 'cast', 'when', 'datetime', '-', 'show']
            )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("row 0", result.stderr)
        self.assertIn("value redacted", result.stderr)
        self.assertNotIn("not-a-date", result.stderr)

        with tempfile.TemporaryDirectory() as tmp:
            data = Path(tmp) / "epoch.csv"
            data.write_text("when\n1\n9223372036854775807\n")
            sub_us = self.run_pipeline(
                ['load', str(data), '-', 'cast', 'when', 'datetime', '--epoch-unit', 'ns', '-', 'show']
            )
            overflow = self.run_pipeline(
                ['load', str(data), '-', 'cast', 'when', 'datetime', '--epoch-unit', 's', '-', 'show']
            )
        self.assertNotEqual(sub_us.returncode, 0)
        self.assertNotEqual(overflow.returncode, 0)
        self.assertTrue(
            "conversion" in overflow.stderr.lower()
            or "epoch value is outside" in overflow.stderr.lower()
        )

    def test_static_timezone_policy_errors_precede_input_io(self):
        result = self.run_pipeline(
            ['load', '/definitely/missing.csv', '-', 'cast', 'when', 'datetime', '--timezone', 'Not/AZone', '-', 'show']
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(
            "invalid target timezone" in result.stderr or "invalid timezone" in result.stderr
        )
        self.assertNotIn("No such file", result.stderr)
        result = self.run_pipeline(
            ['load', '/definitely/missing.csv', '-', 'changetz', 'when', '--from-tz', 'UTC', '--to-tz', 'Not/AZone', '-', 'show']
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(
            "invalid target timezone" in result.stderr or "invalid timezone" in result.stderr
        )
        result = self.run_pipeline(
            ['load', '/definitely/missing.csv', '-', 'cast', 'when', 'int', '--timezone', 'UTC', '-', 'show']
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("datetime parsing options", result.stderr)
        result = self.run_pipeline(
            ['load', '/definitely/missing.csv', '-', 'bucket', 'when', '1s', '--ambiguous', 'invalid', '-', 'show']
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ambiguous policy", result.stderr)
        with tempfile.TemporaryDirectory() as tmp:
            config = Path(tmp) / "typed.yaml"
            config.write_text(
                "version: 1\nstages:\n- name: p\n  steps:\n"
                f"  - load:\n      paths: [{self.get_fixture_path('cast.csv')}]\n"
                "  - cast:\n      column: when\n      type: datetime\n"
                "  - bucket:\n      column: when\n      interval: 1h\n      ambiguous: error\n"
                "  - show: {}\n"
            )
            result = self.run_pipeline(['run', str(config)])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("parsing options apply only to string input", result.stderr)

    def test_debug_reports_bounded_parser_family_without_values(self):
        with tempfile.TemporaryDirectory() as tmp:
            data = Path(tmp) / "many.csv"
            data.write_text("when\n" + "2024-01-02T03:04:05Z\n" * 40)
            command = [
                str(self.qlt_path), "load", str(data), "-", "cast", "when", "datetime", "-", "show"
            ]
            env = dict(os.environ, RUST_LOG="debug")
            result = subprocess.run(command, capture_output=True, text=True, cwd=self.root_dir, env=env)
        self.assertEqual(result.returncode, 0)
        self.assertIn("datetime parser family accepted", result.stderr)
        self.assertLessEqual(result.stderr.count("datetime parser family accepted"), 1)
        self.assertNotIn("2024-01-02T03:04:05Z", result.stderr)

        with tempfile.TemporaryDirectory() as tmp:
            data = Path(tmp) / "two.csv"
            data.write_text("when\n02/Jan/2024:03:04:05\n")
            config = Path(tmp) / "two.yaml"
            config.write_text(
                "version: 1\nstages:\n"
                f"- name: first\n  steps:\n  - load:\n      paths: [{data}]\n"
                "  - cast:\n      column: when\n      type: datetime\n"
                "  - show: {}\n"
                f"- name: second\n  steps:\n  - load:\n      paths: [{data}]\n"
                "  - cast:\n      column: when\n      type: datetime\n      input-format: '%d/%b/%Y:%H:%M:%S'\n"
                "  - show: {}\n"
            )
            env = dict(os.environ, RUST_LOG="debug")
            result = subprocess.run(
                [str(self.qlt_path), "run", str(config)],
                capture_output=True,
                text=True,
                cwd=self.root_dir,
                env=env,
            )
        self.assertEqual(result.returncode, 0)
        self.assertGreaterEqual(result.stderr.count("datetime parser family accepted"), 2)
        self.assertNotIn("02/Jan/2024:03:04:05", result.stderr)

    def test_row_errors_are_contextual_and_redacted(self):
        with tempfile.TemporaryDirectory() as tmp:
            data = Path(tmp) / "bad.csv"
            data.write_text("when\nnot-a-date\n")
            changetz = self.run_pipeline(
                ['load', str(data), '-', 'changetz', 'when', '--from-tz', 'UTC', '--to-tz', 'UTC', '-', 'show']
            )
            timeslice = self.run_pipeline(
                ['load', str(data), '-', 'timeslice', 'when', '--start', '2024-01-01', '-', 'show']
            )
        for result in (changetz, timeslice):
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("column 'when' row 0", result.stderr)
            self.assertIn("value redacted", result.stderr)
            self.assertNotIn("not-a-date", result.stderr)


if __name__ == "__main__":
    unittest.main()
