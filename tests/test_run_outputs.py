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

    def test_ordered_multiple_show_artifacts(self):
        source = self.get_fixture_path("sample-min.csv")
        path = self.write_run_document(
            "two-show.yaml",
            f"""
            version: 1
            stages:
              - name: input
                steps: [{{load: {{paths: ["{source}"]}}}}, {{head: {{number: 1}}}}]
              - name: first
                from: input
                steps: [{{select: {{columns: [RecordNumber, Level]}}}}, {{show: null}}]
              - name: second
                from: input
                steps: [{{select: {{columns: [Level]}}}}, {{show: null}}]
            """,
        )
        result = self.run_run_document(path)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "RecordNumber,Level\n227126,Info\nLevel\nInfo\n")

    def test_materialize_policy_fanout_and_show_plan_are_explicit(self):
        source = self.get_fixture_path("sample-min.csv")
        path = self.write_run_document(
            "materialize.yaml",
            f"""
            version: 1
            stages:
              - name: input
                materialize: auto
                steps: [{{load: {{paths: ["{source}"]}}}}]
              - name: left
                from: input
                steps: [{{head: {{number: 1}}}}, {{select: {{columns: [EventId, Level]}}}}, {{show: null}}]
              - name: right
                from: input
                steps: [{{head: {{number: 1}}}}, {{select: {{columns: [Level]}}}}, {{show: null}}]
            """,
        )
        plan = self.run_cli(["run", path, "--show-plan", "left"])
        self.assertEqual(plan.returncode, 0, plan.stderr)
        self.assertIn("materialize=disk", plan.stdout)
        result = self.run_run_document(path)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "EventId,Level\n1102,Info\nLevel\nInfo\n")

    def test_materialize_fanout_writes_distinct_csv_and_parquet_outputs(self):
        source = self.get_fixture_path("sample-min.csv")
        path = self.write_run_document(
            "materialize-files.yaml",
            f"""
            version: 1
            stages:
              - name: input
                materialize: auto
                steps: [{{load: {{paths: ["{source}"]}}}}]
              - name: csv_branch
                from: input
                steps: [{{head: {{number: 1}}}}, {{select: {{columns: [EventId, Level]}}}}, {{dump: {{output: csv_branch.csv}}}}]
              - name: parquet_branch
                from: input
                steps: [{{head: {{number: 1}}}}, {{select: {{columns: [Level]}}}}, {{dumpcache: {{output: parquet_branch}}}}]
            """,
        )
        result = self.run_run_document(path)
        self.assertEqual(result.returncode, 0, result.stderr)
        csv_output = Path(self.temp_dir, "csv_branch.csv")
        parquet_output = Path(self.temp_dir, "parquet_branch.parquet")
        self.assertEqual(csv_output.read_text(), "EventId,Level\n1102,Info\n")
        self.assertGreater(parquet_output.stat().st_size, 4)
        self.assertEqual(parquet_output.read_bytes()[:4], b"PAR1")

    def test_downstream_output_failure_preserves_existing_target(self):
        source = self.get_fixture_path("sample-min.csv")
        target = Path(self.temp_dir, "existing-output.csv")
        target.write_text("keep-me\n")
        path = self.write_run_document(
            "materialize-output-failure.yaml",
            f"""
            version: 1
            stages:
              - name: input
                materialize: always
                steps: [{{load: {{paths: ["{source}"]}}}}]
            """,
        )
        result = self.run_cli(["run", path, "--output", str(target)])
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(target.read_text(), "keep-me\n")
        self.assertFalse(list(Path(self.temp_dir).glob(".qlt-stage-materialized-*")))
        self.assertFalse(list(Path(self.temp_dir).glob(".qlt-gzip-spool-*")))

    def test_materialize_never_preserves_lazy_recomputation_and_always_is_typed(self):
        source = self.get_fixture_path("sample-min.csv")
        path = self.write_run_document(
            "materialize-never.yaml",
            f"""
            version: 1
            stages:
              - name: input
                materialize: never
                steps: [{{load: {{paths: ["{source}"]}}}}]
            """,
        )
        result = self.run_run_document(path)
        self.assertEqual(result.returncode, 0, result.stderr)
        invalid = self.write_run_document(
            "materialize-invalid.yaml",
            """
            version: 1
            stages:
              - name: input
                materialize: sometimes
                steps: []
            """,
        )
        result = self.run_run_document(invalid)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("materialize", result.stderr)

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
