import os
import unittest
from pathlib import Path

from test_base import QuiltTestBase


class TestRunOutputs(QuiltTestBase):
    def test_ordered_multiple_stdout_results(self):
        source = self.get_fixture_path("sample-min.csv")
        path = self.write_run_document(
            "stdout.yaml",
            f"""
            version: 1
            stages:
              - name: input
                steps: [{{load: {{paths: ["{source}"]}}}}, {{head: {{number: 1}}}}, {{headers: {{plain: true}}}}, {{show: null}}]
            """,
        )
        result = self.run_run_document(path)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertLess(result.stdout.index("RecordNumber"), result.stdout.index("1102"))

    def test_multiple_file_outputs_and_relative_paths(self):
        source = self.get_fixture_path("sample-min.csv")
        path = self.write_run_document(
            "files.yaml",
            f"""
            version: 1
            stages:
              - name: input
                steps: [{{load: {{paths: ["{source}"]}}}}, {{head: {{number: 1}}}}, {{dump: {{output: first.csv}}}}, {{dumpcache: {{output: second}}}}]
            """,
        )
        result = self.run_run_document(path)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(Path(self.temp_dir, "first.csv").exists())
        self.assertTrue(Path(self.temp_dir, "second.parquet").exists())

    def test_cli_output_is_relative_to_caller(self):
        source = self.get_fixture_path("sample-min.csv")
        path = self.write_run_document(
            "cli-output.yaml",
            f"""
            version: 1
            stages:
              - name: input
                steps: [{{load: {{paths: ["{source}"]}}}}, {{head: {{number: 1}}}}]
            """,
        )
        result = self.run_cli(["run", path, "--output", "caller.csv"], cwd=self.temp_dir)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(Path(self.temp_dir, "caller.csv").exists())

    def test_debug_output_is_stderr_and_file_only_has_no_implicit_show(self):
        source = self.get_fixture_path("sample-min.csv")
        path = self.write_run_document(
            "debug.yaml",
            f"""
            version: 1
            stages:
              - name: input
                steps: [{{load: {{paths: ["{source}"]}}}}, {{head: {{number: 1}}}}, {{dump: {{output: only.csv}}}}]
            """,
        )
        result = self.run_run_document(path)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertTrue(Path(self.temp_dir, "only.csv").exists())

    def test_existing_output_is_error_and_preserved(self):
        target = Path(self.temp_dir, "existing.csv")
        target.write_text("stale\n")
        result = self.run_pipeline(
            ["load", self.get_fixture_path("sample-min.csv")],
            ["dump", "--output", str(target)],
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("refusing to overwrite", result.stderr)
        self.assertEqual(target.read_text(), "stale\n")


if __name__ == "__main__":
    unittest.main()
