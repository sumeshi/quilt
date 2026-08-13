import unittest
import tempfile
from pathlib import Path
from test_base import QuiltTestBase


class TestShow(QuiltTestBase):
    fixture = "sample-min.csv"

    def test_show_basic(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'show'])
        self.assertEqual(result.returncode, 0)
        lines = result.stdout.splitlines()
        self.assertEqual(len(lines), 30)
        self.assertTrue(lines[0].startswith("RecordNumber,EventRecordId,TimeCreated"))

    def test_show_no_extra_newline(self):
        result = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'show'])
        self.assertEqual(result.returncode, 0)
        self.assertFalse(result.stdout.endswith("\n\n"))

    def test_show_streaming_matches_show(self):
        plain = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'show'])
        streaming = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'show']
        )
        self.assertEqual(plain.returncode, 0)
        self.assertEqual(streaming.returncode, 0)
        self.assertEqual(streaming.stdout, plain.stdout)

    def test_implicit_finalizer_uses_show_when_stdout_is_not_tty(self):
        explicit = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture)), '-', 'show'])
        implicit = self.run_pipeline(['load', str(self.get_fixture_path(self.fixture))])
        self.assertEqual(explicit.returncode, 0)
        self.assertEqual(implicit.returncode, 0)
        self.assertEqual(implicit.stdout, explicit.stdout)

    def test_show_null_csv_contract(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path('delta.csv')), '-', 'select', 'signed', '-', 'show']
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn("signed\n", result.stdout)
        self.assertIn("\n\n", result.stdout)

    def test_show_run_matches_cli(self):
        cli = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'head', '2', '-', 'show']
        )
        run = self.run_pipeline(
            ['run', str(self.get_fixture_path('run-simple.yaml'))]
        )
        self.assertEqual(cli.returncode, 0)
        self.assertEqual(run.returncode, 0)

    def test_run_show_plan_builds_selected_stage_without_data_side_effects(self):
        result = self.run_pipeline(
            ['run', str(self.get_fixture_path('run-simple.yaml')), '--show-plan', 'select_columns']
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Logical query plan:", result.stdout)
        self.assertIn("Optimized query plan:", result.stdout)
        self.assertIn('col("EventId")', result.stdout)
        self.assertFalse((self.root_dir / "sample-min.csv").exists())

    def test_run_show_plan_supports_join_and_concat(self):
        result = self.run_pipeline(
            ['run', str(self.get_fixture_path('run-test.yaml')), '--show-plan', 'merge_stage']
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Logical query plan:", result.stdout)
        with tempfile.TemporaryDirectory() as tmp:
            config = Path(tmp) / "concat.yaml"
            source = self.get_fixture_path(self.fixture)
            config.write_text(
                "version: 1\n"
                "stages:\n"
                "- name: left\n"
                "  steps:\n"
                f"  - load:\n      paths: [{source}]\n"
                "- name: right\n"
                "  steps:\n"
                f"  - load:\n      paths: [{source}]\n"
                "- name: combined\n"
                "  concat:\n"
                "    inputs: [left, right]\n"
                "    how: vertical\n"
            )
            result = self.run_pipeline(['run', str(config), '--show-plan', 'combined'])
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("Logical query plan:", result.stdout)

    def test_run_show_plan_ignores_dump_and_partition_side_effects(self):
        with tempfile.TemporaryDirectory() as tmp:
            config = Path(tmp) / "finalizers.yaml"
            dump_target = Path(tmp) / "sentinel.csv"
            partition_target = Path(tmp) / "sentinel-partitions"
            source = self.get_fixture_path(self.fixture)
            config.write_text(
                "version: 1\n"
                "stages:\n"
                "- name: inspect\n"
                "  steps:\n"
                f"  - load:\n      paths: [{source}]\n"
                "  - select:\n      columns: [EventId, Level]\n"
                f"  - dump: {{output: {dump_target}}}\n"
                "  - partition:\n"
                "      column: Level\n"
                f"      output-dir: {partition_target}\n"
            )
            result = self.run_pipeline(['run', str(config), '--show-plan', 'inspect'])
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn('col("EventId")', result.stdout)
            self.assertFalse(dump_target.exists())
            self.assertFalse(partition_target.exists())

    def test_showtable_preview_contract(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'showtable']
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn("shape: (8+,", result.stdout)

    def test_showquery_is_plan_only(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'select', 'EventId', '-', 'showquery']
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn("Logical query plan:", result.stdout)

    def test_show_empty_input(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path('empty.csv')), '-', 'show']
        )
        self.assertEqual(result.returncode, 0)

    def test_show_invalid_lazy_conversion_is_error(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path('sample-min.csv')), '-', 'cast', 'Level', 'int', '-', 'show']
        )
        self.assertNotEqual(result.returncode, 0)

    def test_showtable_long_cell_is_bounded(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path('sample-min.csv')), '-', 'showtable']
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn("…", result.stdout)


if __name__ == "__main__":
    unittest.main()
