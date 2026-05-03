import os
import shutil
import tempfile
import textwrap
import unittest
from test_base import QsvTestBase


class TestQuilt(QsvTestBase):
    QUILT_VERSION = "1.0.0"
    QUILT_AUTHOR = "qsv test suite"

    def setUp(self):
        super().setUp()
        self.temp_dir = tempfile.mkdtemp()

    def tearDown(self):
        shutil.rmtree(self.temp_dir, ignore_errors=True)

    def write_quilt(self, name, content):
        filename = name if name.startswith("quilt-") else f"quilt-{name}"
        if not filename.endswith(".yaml"):
            filename = f"{filename}.yaml"

        normalized = textwrap.dedent(content).strip()
        title = None
        body_lines = []

        for line in normalized.splitlines():
            stripped = line.strip()
            if stripped.startswith("title:"):
                title = stripped.split(":", 1)[1].strip().strip("'\"")
                continue
            if any(stripped.startswith(f"{field}:") for field in ("description", "version", "author")):
                continue
            body_lines.append(line)

        if title is None:
            title = filename.removesuffix(".yaml").replace("quilt-", "").replace("_", " ").replace("-", " ").title()

        description = f"Quilt test fixture for {title.lower()}"
        content = "\n".join(
            [
                f'title: "{title}"',
                f'description: "{description}"',
                f'version: "{self.QUILT_VERSION}"',
                f'author: "{self.QUILT_AUTHOR}"',
                *body_lines,
                "",
            ]
        )

        path = os.path.join(self.temp_dir, filename)
        with open(path, "w", encoding="utf-8") as f:
            f.write(content)
        return path

    def test_quilt_simple_pipeline(self):
        result = self.run_qsv_command(f"quilt {self.get_fixture_path('quilt-simple.yaml')}")
        self.assertEqual(result.returncode, 0)
        self.assertTrue(result.stdout.startswith("EventId,Level"))
        self.assertEqual(len(result.stdout.strip().splitlines()), 30)

    def test_quilt_join_operation(self):
        result = self.run_qsv_command(f"quilt {self.get_fixture_path('quilt-join.yaml')}")
        self.assertEqual(result.returncode, 0)
        self.assertTrue(result.stdout.startswith("TimeCreated,EventId,Level"))
        self.assertIn("1102,Info", result.stdout)

    def test_quilt_cross_join(self):
        quilt = self.write_quilt(
            "cross.yaml",
            f"""
            title: 'Cross Join Test'
            stages:
              load_stage:
                type: process
                steps:
                  load:
                    path: "{self.get_fixture_path('sample-min.csv')}"
              left_stage:
                type: process
                source: load_stage
                steps:
                  select:
                    colnames: [EventId]
                  head:
                    number: 2
              right_stage:
                type: process
                source: load_stage
                steps:
                  select:
                    colnames: [Level]
                  head:
                    number: 2
              join_stage:
                type: join
                sources: [left_stage, right_stage]
                params:
                  how: cross
              final_stage:
                type: process
                source: join_stage
                steps:
                  show:
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        self.assertEqual(result.returncode, 0)
        self.assertNotIn("__qsv_quilt_cross_join_key", result.stdout)
        self.assertEqual(len(result.stdout.strip().splitlines()), 5)

    def test_quilt_left_right_on(self):
        quilt = self.write_quilt(
            "left_right.yaml",
            f"""
            title: 'Asymmetric Join Test'
            stages:
              load_stage:
                type: process
                steps:
                  load:
                    path: "{self.get_fixture_path('sample-min.csv')}"
              left_stage:
                type: process
                source: load_stage
                steps:
                  select:
                    colnames: [TimeCreated, EventId]
                  renamecol:
                    old_name: TimeCreated
                    new_name: left_time
              right_stage:
                type: process
                source: load_stage
                steps:
                  select:
                    colnames: [TimeCreated, Level]
                  renamecol:
                    old_name: TimeCreated
                    new_name: right_time
              merge_stage:
                type: join
                sources: [left_stage, right_stage]
                params:
                  how: inner
                  left_on: left_time
                  right_on: right_time
              final_stage:
                type: process
                source: merge_stage
                steps:
                  select:
                    colnames: [left_time, EventId, Level]
                  head:
                    number: 1
                  show:
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        self.assertEqual(result.returncode, 0)
        self.assertIn("left_time,EventId,Level", result.stdout)

    def test_quilt_concat_success(self):
        quilt = self.write_quilt(
            "concat_success.yaml",
            f"""
            title: 'Concat Success'
            stages:
              load_stage:
                type: process
                steps:
                  load:
                    path: "{self.get_fixture_path('sample-min.csv')}"
              left_stage:
                type: process
                source: load_stage
                steps:
                  select:
                    colnames: [EventId, Level]
              right_stage:
                type: process
                source: load_stage
                steps:
                  select:
                    colnames: [EventId, Level]
              concat_stage:
                type: concat
                sources: [left_stage, right_stage]
                params:
                  how: vertical
              final_stage:
                type: process
                source: concat_stage
                steps:
                  show:
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.stdout.strip().splitlines()), 59)

    def test_quilt_concat_mismatch_no_panic(self):
        quilt = self.write_quilt(
            "concat_mismatch.yaml",
            f"""
            title: 'Concat Mismatch'
            stages:
              load_stage:
                type: process
                steps:
                  load:
                    path: "{self.get_fixture_path('sample-min.csv')}"
              left_stage:
                type: process
                source: load_stage
                steps:
                  select:
                    colnames: [EventId, Level]
              right_stage:
                type: process
                source: load_stage
                steps:
                  select:
                    colnames: [TimeCreated, Provider]
              concat_stage:
                type: concat
                sources: [left_stage, right_stage]
                params:
                  how: vertical
              final_stage:
                type: process
                source: concat_stage
                steps:
                  show:
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        self.assertIn("Error:", result.stderr)
        self.assertNotIn("panicked at", result.stderr)

    def test_quilt_timeround_step(self):
        quilt = self.write_quilt(
            "timeround.yaml",
            f"""
            title: 'Timeround'
            stages:
              process_data:
                type: process
                steps:
                  load:
                    path: "{self.get_fixture_path('sample-min.csv')}"
                  timeround:
                    colname: TimeCreated
                    unit: day
                    output: rounded_day
                  select:
                    colnames: [rounded_day, EventId]
                  head:
                    number: 1
                  show:
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "rounded_day,EventId\n2016-10-06,1102")

    def test_quilt_timeline_step(self):
        quilt = self.write_quilt(
            "timeline.yaml",
            f"""
            title: 'Timeline'
            stages:
              process_data:
                type: process
                steps:
                  load:
                    path: "{self.get_fixture_path('sample-min.csv')}"
                  timeline:
                    time_column: TimeCreated
                    interval: 1s
                  show:
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        self.assertEqual(result.returncode, 0)
        self.assertIn("timeline_1s,count", result.stdout)

    def test_quilt_timeslice_step(self):
        quilt = self.write_quilt(
            "timeslice.yaml",
            f"""
            title: 'Timeslice'
            stages:
              process_data:
                type: process
                steps:
                  load:
                    path: "{self.get_fixture_path('sample-min.csv')}"
                  timeslice:
                    time_column: TimeCreated
                    start: '2016-10-06 00:00:00'
                    end: '2016-10-06 23:59:59'
                  head:
                    number: 1
                  show:
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        self.assertEqual(result.returncode, 0)
        self.assertIn("1102,Info", result.stdout)

    def test_quilt_timeslice_requires_start_or_end(self):
        quilt = self.write_quilt(
            "timeslice-missing-bounds.yaml",
            f"""
            title: 'Timeslice Missing Bounds'
            stages:
              process_data:
                type: process
                steps:
                  load:
                    path: "{self.get_fixture_path('sample-min.csv')}"
                  timeslice:
                    time_column: TimeCreated
                  show:
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("timeslice in quilt requires at least one of 'start' or 'end'", result.stderr)

    def test_quilt_steps_sequence_allows_duplicate_commands(self):
        quilt = self.write_quilt(
            "steps-sequence.yaml",
            f"""
            title: 'Steps Sequence'
            stages:
              process_data:
                type: process
                steps:
                  - load:
                      path: "{self.get_fixture_path('sample-min.csv')}"
                  - grep:
                      pattern: "4688"
                  - grep:
                      pattern: "wevtutil.exe"
                  - select:
                      colnames: [EventId]
                  - count:
                  - show:
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "EventId,count\n4688,11")

    def test_quilt_mapping_steps_keep_underscore_duplicate_workaround(self):
        quilt = self.write_quilt(
            "steps-mapping-underscore.yaml",
            f"""
            title: 'Steps Mapping Underscore'
            stages:
              process_data:
                type: process
                steps:
                  load:
                    path: "{self.get_fixture_path('sample-min.csv')}"
                  grep_:
                    pattern: "4688"
                  grep__:
                    pattern: "wevtutil.exe"
                  select:
                    colnames: [EventId]
                  count:
                  show:
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "EventId,count\n4688,11")

    def test_quilt_cli_var_substitution(self):
        quilt = self.write_quilt(
            "vars.yaml",
            f"""
            title: 'Vars'
            stages:
              process_data:
                type: process
                steps:
                  load:
                    path: "{self.get_fixture_path('sample-min.csv')}"
                  isin:
                    colname: EventId
                    values: ["${{event_id}}"]
                  contains:
                    colname: ExecutableInfo
                    pattern: "${{exe}}"
                  select:
                    colnames: [EventId]
                  count:
                  show:
            """,
        )
        result = self.run_qsv_command(
            f"quilt {quilt} --var event_id=4688 --var exe=wevtutil.exe"
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "EventId,count\n4688,11")

    def test_quilt_forward_reference_is_resolved(self):
        quilt = self.write_quilt(
            "forward-reference.yaml",
            f"""
            title: 'Forward Reference'
            stages:
              final_stage:
                type: process
                source: projected_stage
                steps:
                  head:
                    number: 1
                  show:
              load_stage:
                type: process
                steps:
                  load:
                    path: "{self.get_fixture_path('sample-min.csv')}"
              projected_stage:
                type: process
                source: load_stage
                steps:
                  select:
                    colnames: [EventId, Level]
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "EventId,Level\n1102,Info")

    def test_quilt_cycle_fails(self):
        quilt = self.write_quilt(
            "cycle.yaml",
            """
            title: 'Cycle'
            stages:
              stage_a:
                type: process
                source: stage_b
                steps:
                  show:
              stage_b:
                type: process
                source: stage_a
                steps:
                  show:
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Circular stage dependency", result.stderr)

    def test_quilt_multi_source_join(self):
        quilt = self.write_quilt(
            "multi-join.yaml",
            f"""
            title: 'Multi Join'
            stages:
              load_stage:
                type: process
                steps:
                  load:
                    path: "{self.get_fixture_path('sample-min.csv')}"
              event_stage:
                type: process
                source: load_stage
                steps:
                  select:
                    colnames: [TimeCreated, EventId]
              level_stage:
                type: process
                source: load_stage
                steps:
                  select:
                    colnames: [TimeCreated, Level]
              provider_stage:
                type: process
                source: load_stage
                steps:
                  select:
                    colnames: [TimeCreated, Provider]
              merged_stage:
                type: join
                sources: [event_stage, level_stage, provider_stage]
                params:
                  how: inner
                  key: TimeCreated
                  coalesce: true
              final_stage:
                type: process
                source: merged_stage
                steps:
                  head:
                    number: 1
                  show:
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        self.assertEqual(result.returncode, 0)
        self.assertTrue(
            result.stdout.startswith("TimeCreated,EventId,Level,Provider")
        )

    def test_quilt_branch_stage(self):
        quilt = self.write_quilt(
            "branch.yaml",
            f"""
            title: 'Branch'
            stages:
              load_stage:
                type: process
                steps:
                  load:
                    path: "{self.get_fixture_path('sample-min.csv')}"
              branch_stage:
                type: branch
                source: load_stage
                params:
                  condition: "count > 10"
                then_steps:
                  select:
                    colnames: [EventId]
                  head:
                    number: 1
                  show:
                else_steps:
                  select:
                    colnames: [Level]
                  head:
                    number: 1
                  show:
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "EventId\n1102")

    def test_quilt_output_stage_debug_show_writes_to_stderr(self):
        quilt = self.write_quilt(
            "output-debug.yaml",
            f"""
            title: 'Output Debug'
            stages:
              load_stage:
                type: process
                steps:
                  load:
                    path: "{self.get_fixture_path('sample-min.csv')}"
              debug_stage:
                type: output
                source: load_stage
                steps:
                  show:
                    debug: true
              final_stage:
                type: process
                source: load_stage
                steps:
                  select:
                    colnames: [EventId]
                  head:
                    number: 1
                  show:
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "EventId\n1102")
        self.assertIn("RecordNumber,EventRecordId", result.stderr)

    def test_quilt_event_triage(self):
        quilt = self.write_quilt(
            "event_triage.yaml",
            f"""
            title: 'Event Triage'
            stages:
              load_data:
                type: process
                steps:
                  load:
                    path: "{self.get_fixture_path('sample-min.csv')}"
              filter_time:
                type: process
                source: load_data
                steps:
                  timeslice:
                    time_column: TimeCreated
                    start: '2016-10-06 00:00:00'
                    end: '2016-10-06 23:59:59'
              triage:
                type: process
                source: filter_time
                steps:
                  select:
                    colnames: [EventId]
                  count:
                  show:
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        lines = result.stdout.strip().splitlines()
        self.assertEqual(result.returncode, 0)
        self.assertEqual(lines[0], "EventId,count")
        self.assertEqual(set(lines[1:]), {"4688,14", "4689,14", "1102,1"})

    def test_quilt_process_burst_detection(self):
        quilt = self.write_quilt(
            "process_burst_detection.yaml",
            f"""
            title: 'Process Burst Detection'
            stages:
              load_data:
                type: process
                steps:
                  load:
                    path: "{self.get_fixture_path('sample-min.csv')}"
              filter_creates:
                type: process
                source: load_data
                steps:
                  isin:
                    colname: EventId
                    values: ["4688"]
              burst_check:
                type: process
                source: filter_creates
                steps:
                  timeline:
                    time_column: TimeCreated
                    interval: 1s
                  show:
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "timeline_1s,count\n2016-10-06 01:47:07,14")

    def test_quilt_lifecycle_balance(self):
        quilt = self.write_quilt(
            "lifecycle_balance.yaml",
            f"""
            title: 'Lifecycle Balance'
            stages:
              load_data:
                type: process
                steps:
                  load:
                    path: "{self.get_fixture_path('sample-min.csv')}"
              creates:
                type: process
                source: load_data
                steps:
                  isin:
                    colname: EventId
                    values: ["4688"]
                  select:
                    colnames: [EventId]
              exits:
                type: process
                source: load_data
                steps:
                  isin:
                    colname: EventId
                    values: ["4689"]
                  select:
                    colnames: [EventId]
              combined:
                type: concat
                sources: [creates, exits]
                params:
                  how: vertical
              summary:
                type: process
                source: combined
                steps:
                  count:
                  show:
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        lines = result.stdout.strip().splitlines()
        self.assertEqual(result.returncode, 0)
        self.assertEqual(lines[0], "EventId,count")
        self.assertEqual(set(lines[1:]), {"4688,14", "4689,14"})

    def test_quilt_log_tamper_hunt(self):
        quilt = self.write_quilt(
            "log_tamper_hunt.yaml",
            f"""
            title: 'Log Tamper Hunt'
            stages:
              load_data:
                type: process
                steps:
                  load:
                    path: "{self.get_fixture_path('sample-min.csv')}"
              hunt:
                type: process
                source: load_data
                steps:
                  grep:
                    pattern: "Event log cleared"
                  select:
                    colnames: [EventId, Level]
                  show:
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        lines = result.stdout.strip().splitlines()
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(lines), 2)
        self.assertIn("1102,Info", result.stdout)

    def test_quilt_provider_filter(self):
        quilt = self.write_quilt(
            "provider_filter.yaml",
            f"""
            title: 'Provider Filter'
            stages:
              load_data:
                type: process
                steps:
                  load:
                    path: "{self.get_fixture_path('sample-min.csv')}"
              filter_provider:
                type: process
                source: load_data
                steps:
                  contains:
                    colname: Provider
                    pattern: "Security-Auditing"
                  select:
                    colnames: [Level]
                  count:
                  show:
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "Level,count\nLogAlways,28")

    def test_quilt_jst_analysis(self):
        quilt = self.write_quilt(
            "jst_analysis.yaml",
            f"""
            title: 'JST Analysis'
            stages:
              load_data:
                type: process
                steps:
                  load:
                    path: "{self.get_fixture_path('sample-min.csv')}"
              convert_tz:
                type: process
                source: load_data
                steps:
                  changetz:
                    colname: TimeCreated
                    from-tz: UTC
                    to-tz: Asia/Tokyo
              result:
                type: process
                source: convert_tz
                steps:
                  select:
                    colnames: [TimeCreated, Level]
                  head:
                    number: 1
                  show:
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        self.assertEqual(result.returncode, 0)
        self.assertIn("+09:00", result.stdout)
        self.assertTrue(result.stdout.strip().startswith("TimeCreated,Level"))

    def test_quilt_undefined_source(self):
        quilt = self.write_quilt(
            "undefined_source.yaml",
            """
            title: 'Undefined Source'
            stages:
              final_stage:
                type: process
                source: missing_stage
                steps:
                  show:
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("depends on missing stage 'missing_stage'", result.stderr)

    def test_quilt_nonexistent_file(self):
        result = self.run_qsv_command("quilt nonexistent_file.yaml")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Error reading config file", result.stderr)

    def test_quilt_dump_output(self):
        cwd = os.getcwd()
        output_file = os.path.join(self.temp_dir, "test_output.csv")
        quilt = self.write_quilt(
            "dump.yaml",
            f"""
            title: 'Dump'
            stages:
              process_data:
                type: process
                steps:
                  load:
                    path: "{self.get_fixture_path('sample-min.csv')}"
                  select:
                    colnames: [TimeCreated, EventId]
                  head:
                    number: 2
                  dump:
                    output: "{output_file}"
            """,
        )
        try:
            os.chdir(self.root_dir)
            result = self.run_qsv_command(f"quilt {quilt}")
        finally:
            os.chdir(cwd)
        self.assertEqual(result.returncode, 0)
        self.assertTrue(os.path.exists(output_file))
        with open(output_file, "r", encoding="utf-8") as f:
            self.assertEqual(len(f.read().splitlines()), 3)


if __name__ == "__main__":
    unittest.main()
