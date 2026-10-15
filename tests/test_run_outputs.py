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
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("multiple stdout", result.stderr)
        self.assertEqual(result.stdout, "")

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
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("multiple stdout", result.stderr)
        selected = self.run_cli(["run", path, "--stage", "second"])
        self.assertEqual(selected.returncode, 0, selected.stderr)
        self.assertEqual(selected.stdout, "Level\nInfo\n")

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
        self.assertIn("input: materialize=lazy", plan.stdout)
        self.assertNotIn("right:", plan.stdout)
        result = self.run_cli(["run", path, "--stage", "left"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "EventId,Level\n1102,Info\n")

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

    def test_cli_output_overrides_yaml_dump_path(self):
        source = self.get_fixture_path("sample-min.csv")
        yaml_target = Path(self.temp_dir, "yaml.csv")
        cli_target = Path(self.temp_dir, "cli.csv")
        path = self.write_run_document(
            "override.yaml",
            f"""
            version: 1
            stages:
              - name: input
                steps:
                  - load: {{paths: ["{source}"]}}
                  - head: {{number: 1}}
                  - dump: {{output: {yaml_target}}}
            """,
        )
        without = self.run_run_document(path)
        self.assertEqual(without.returncode, 0, without.stderr)
        self.assertTrue(yaml_target.exists())
        yaml_target.unlink()

        result = self.run_cli(["run", path, "--output", str(cli_target)])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(cli_target.exists())
        self.assertFalse(yaml_target.exists())
        self.assertIn("RecordNumber", cli_target.read_text())

    def test_cli_output_is_relative_to_caller(self):
        source = self.get_fixture_path("sample-min.csv")
        rules = Path(self.temp_dir, "rules")
        rules.mkdir()
        for index, finalizer in enumerate(("", "    - dump: {output: yaml.csv}\n")):
            path = rules / f"output-{index}.yaml"
            path.write_text(
                "version: 1\nstages:\n  - name: input\n    steps:\n"
                f'    - load: {{paths: ["{source}"]}}\n'
                "    - head: {number: 1}\n" + finalizer
            )
            target = f"caller-{index}.csv"
            result = self.run_cli(["run", str(path.relative_to(self.temp_dir)), "--output", target], cwd=self.temp_dir)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(Path(self.temp_dir, target).exists())
            self.assertFalse((rules / target).exists())
            self.assertFalse((rules / "yaml.csv").exists())

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

    def test_stage_avoids_unrelated_inputs_and_ancestor_outputs(self):
        source = self.get_fixture_path("sample-min.csv")
        path = self.write_run_document(
            "selected.yaml",
            f"""
            version: 1
            stages:
              - name: input
                steps: [{{load: {{paths: ["{source}"]}}}}, {{dump: {{output: ancestor.csv}}}}]
              - name: wanted
                from: input
                steps: [{{head: {{number: 1}}}}, {{select: {{columns: [EventId]}}}}, {{show: null}}]
              - name: unrelated
                steps: [{{load: {{paths: [missing.csv]}}}}, {{dump: {{output: unrelated.csv}}}}]
            """,
        )
        result = self.run_cli(["run", path, "--stage", "wanted"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "EventId\n1102\n")
        self.assertFalse(Path(self.temp_dir, "ancestor.csv").exists())
        self.assertFalse(Path(self.temp_dir, "unrelated.csv").exists())
        self.assertIn("completed 2 stages", result.stderr)

    def test_output_requires_one_sink_and_selection_overrides_only_its_dump(self):
        source = self.get_fixture_path("sample-min.csv")
        path = self.write_run_document(
            "ambiguous.yaml",
            f"""
            version: 1
            stages:
              - name: input
                steps: [{{load: {{paths: ["{source}"]}}}}]
              - name: left
                from: input
                steps: [{{dump: {{output: left.csv}}}}]
              - name: right
                from: input
                steps: [{{dump: {{output: right.csv}}}}]
            """,
        )
        target = Path(self.temp_dir, "chosen.csv")
        result = self.run_cli(["run", path, "--output", str(target)])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--stage", result.stderr)
        self.assertFalse(target.exists())
        result = self.run_cli(["run", path, "--stage", "left", "--output", str(target)])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(target.is_file())
        self.assertFalse(Path(self.temp_dir, "left.csv").exists())
        self.assertFalse(Path(self.temp_dir, "right.csv").exists())

    def test_output_rejects_multiple_dumps_before_writing(self):
        source = self.get_fixture_path("sample-min.csv")
        path = self.write_run_document(
            "multiple-dumps.yaml",
            f"""
            version: 1
            stages:
              - name: input
                steps:
                  - load: {{paths: ["{source}"]}}
                  - dump: {{output: first.csv}}
                  - dump: {{output: second.csv}}
            """,
        )
        target = Path(self.temp_dir, "chosen.csv")
        result = self.run_cli(["run", path, "--output", str(target)])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("one dump", result.stderr)
        for name in ("chosen.csv", "first.csv", "second.csv"):
            self.assertFalse(Path(self.temp_dir, name).exists())

    def test_relative_config_cli_inputs_output_and_plan_have_one_origin(self):
        rules = Path(self.temp_dir, "rules")
        rules.mkdir()
        Path(self.temp_dir, "events.csv").write_text("id\n1\n2\n")
        (rules / "recipe.yaml").write_text("""version: 1
parameters:
  limit: {type: int, required: true}
stages:
  - name: input
    steps:
      - load: {}
      - head: {number: {"$param": limit}}
""")
        arguments = ["run", "rules/recipe.yaml", "events.csv", "--var", "limit=1"]
        plan = self.run_cli(arguments + ["--show-plan", "input"], cwd=self.temp_dir)
        self.assertEqual(plan.returncode, 0, plan.stderr)
        self.assertIn("materialize=lazy", plan.stdout)
        result = self.run_cli(arguments + ["--output", "result.csv"], cwd=self.temp_dir)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(Path(self.temp_dir, "result.csv").read_text(), "id\n1\n")
        self.assertFalse((rules / "result.csv").exists())

    def test_later_failure_reports_completed_files_and_preserves_targets(self):
        source = self.get_fixture_path("sample-min.csv")
        completed = Path(self.temp_dir, "completed.csv")
        existing = Path(self.temp_dir, "existing.csv")
        existing.write_text("keep\n")
        path = self.write_run_document(
            "partial.yaml",
            f"""
            version: 1
            stages:
              - name: input
                steps:
                  - load: {{paths: ["{source}"]}}
                  - dump: {{output: completed.csv}}
                  - dump: {{output: existing.csv}}
            """,
        )
        result = self.run_cli(["run", path])
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(completed.is_file())
        self.assertEqual(existing.read_text(), "keep\n")
        self.assertIn(f"completed file {completed}", result.stderr)

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
