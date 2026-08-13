import unittest
import subprocess
import os
import json
import tempfile
from test_base import QuiltTestBase


class TestLoad(QuiltTestBase):
    def test_load_single_file(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path('sample-min.csv')), '-', 'show'])
        self.assertEqual(result.returncode, 0)
        lines = result.stdout.strip().splitlines()
        self.assertEqual(lines[0].split(",")[:5], ["RecordNumber", "EventRecordId", "TimeCreated", "EventId", "Level"])
        self.assertEqual(len(lines), 30)

    def test_load_gzip_file(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path('sample-min.csv.gz')), '-', 'head', '1', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        lines = result.stdout.strip().splitlines()
        self.assertEqual(len(lines), 2)
        self.assertIn("1102,Info", lines[1])

    def test_load_gzip_file_with_memory_limit_env(self):
        env = os.environ.copy()
        env["QLT_MEMORY_LIMIT_MB"] = "512"
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path('sample-min.csv.gz')), '-', 'head', '1', '-', 'show'],
            env=env,
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn("1102,Info", result.stdout)

    def test_load_tsv_separator_short(self):
        separator = "\t"
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path('sample-min.tsv')), '-s', str(separator), '-', 'select', 'EventId,Level', '-', 'head', '1', '-', 'show']
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "EventId,Level\n1102,Info")

    def test_load_tsv_separator_long(self):
        separator = "\t"
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path('sample-min.tsv')), '--separator', str(separator), '-', 'head', '1', '-', 'show']
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 2)

    def test_load_multiple_files(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path('sample-min.csv')), str(self.get_fixture_path('sample-min.csv')), '-', 'select', 'Level', '-', 'count', '-', 'show']
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "Level,count\nLogAlways,56\nInfo,2")

    def test_load_glob_pattern(self):
        result = self.run_pipeline(
            ['load', 'tests/fixtures/sample-min.c*sv', '-', 'head', '1', '-', 'show']
        )
        self.assertEqual(result.returncode, 0)
        lines = result.stdout.strip().splitlines()
        self.assertEqual(len(lines), 2)
        self.assertEqual(lines[0].split(",")[:3], ["RecordNumber", "EventRecordId", "TimeCreated"])

    def test_load_low_memory(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path('sample-min.csv')), '--low-memory', '-', 'head', '1', '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertIn("1102,Info", result.stdout)

    def test_load_no_headers(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path('sample-min-noheader.csv')), '--no-headers', '-', 'select', 'column_1,column_4,column_5', '-', 'head', '1', '-', 'show']
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "column_1,column_4,column_5\n227126,1102,Info")

    def test_load_nonexistent_file(self):
        result = self.run_pipeline(['load', 'non_existent_file.csv', '-', 'show'])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Error: File not found", result.stderr)

    def test_load_unmatched_glob_pattern(self):
        result = self.run_pipeline(['load', 'tests/fixtures/does-not-exist-*.csv', '-', 'show'])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("No files found matching pattern", result.stderr)

    def test_ndjson_bounded_and_full_inference_cli_and_run(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = os.path.abspath(directory)
            ndjson_path = os.path.join(directory, "sparse.jsonl")
            with open(ndjson_path, "w", encoding="utf-8") as handle:
                for _ in range(1001):
                    handle.write(json.dumps({"id": 1}) + "\n")
                handle.write(json.dumps({"id": 1, "late": True}) + "\n")

            default = self.run_pipeline(['load', str(ndjson_path), '-', 'headers'])
            self.assertEqual(default.returncode, 0, default.stderr)
            self.assertNotIn("late", default.stdout)

            full = self.run_pipeline(
                ['load', str(ndjson_path), '--infer-schema-length', 'full', '-', 'headers']
            )
            self.assertEqual(full.returncode, 0, full.stderr)
            self.assertIn("late", full.stdout)

            default_run_config = os.path.join(directory, "run-default.yaml")
            with open(default_run_config, "w", encoding="utf-8") as handle:
                handle.write(
                    "version: 1\n"
                    "stages:\n"
                    "- name: process\n"
                    "  steps:\n"
                    "  - load:\n"
                    "      paths: [sparse.jsonl]\n"
                    "  - headers: {}\n"
                )
            run_default = self.run_pipeline(['run', str(default_run_config)])
            self.assertEqual(run_default.returncode, 0, run_default.stderr)
            self.assertNotIn("late", run_default.stdout)

            full_run_config = os.path.join(directory, "run-full.yaml")
            with open(full_run_config, "w", encoding="utf-8") as handle:
                handle.write(
                    "version: 1\n"
                    "stages:\n"
                    "- name: process\n"
                    "  steps:\n"
                    "  - load:\n"
                    "      paths: [sparse.jsonl]\n"
                    "      infer-schema-length: full\n"
                    "  - headers: {}\n"
                )
            run_full = self.run_pipeline(['run', str(full_run_config)])
            self.assertEqual(run_full.returncode, 0, run_full.stderr)
            self.assertIn("late", run_full.stdout)


if __name__ == "__main__":
    unittest.main()
