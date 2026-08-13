import json
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

    def write_json_file(self, name, data):
        path = os.path.join(self.temp_dir, name)
        with open(path, "w", encoding="utf-8") as f:
            json.dump(data, f)
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

    def test_quilt_where_annotate_concat_schema_matches_on_sql_parse_failure(self):
        quilt = self.write_quilt(
            "where_annotate_concat.yaml",
            f"""
            title: 'Where Annotate Concat'
            stages:
              load_stage:
                type: process
                steps:
                  load:
                    path: "{self.get_fixture_path('sample-min.csv')}"
              invalid_rule:
                type: process
                source: load_stage
                steps:
                  where:
                    sql: "SELECT * FROM events WHERE MissingField = 'x'"
                    annotate: true
                    sigma_title: "invalid-rule"
                    sigma_id: "invalid-id"
                    sigma_level: "low"
                    sigma_tags: "test.invalid"
              valid_rule:
                type: process
                source: load_stage
                steps:
                  where:
                    sql: "SELECT * FROM events WHERE Level = 'Info'"
                    annotate: true
                    sigma_title: "valid-rule"
                    sigma_id: "valid-id"
                    sigma_level: "medium"
                    sigma_tags: "test.valid"
              merged:
                type: concat
                sources: [invalid_rule, valid_rule]
              final_stage:
                type: process
                source: merged
                steps:
                  show:
            """,
        )
        result = self.run_qsv_command(f"quilt {quilt}")
        self.assertEqual(result.returncode, 0)
        self.assertIn("sigma_title", result.stdout)
        self.assertIn("sigma_id", result.stdout)
        self.assertIn("sigma_level", result.stdout)
        self.assertIn("sigma_tags", result.stdout)
        self.assertIn("valid-rule,valid-id,medium,test.valid", result.stdout)
        self.assertNotIn("invalid-rule", result.stdout)
        self.assertNotIn("schema lengths differ", result.stderr)

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

    def test_quilt_cli_var_relative_input_falls_back_to_cwd(self):
        nested_dir = os.path.join(self.temp_dir, "rules")
        os.makedirs(nested_dir, exist_ok=True)
        quilt = os.path.join(nested_dir, "vars-input.yaml")
        with open(quilt, "w", encoding="utf-8") as f:
            f.write(
                textwrap.dedent(
                    """
                    title: "Vars Input"
                    description: "relative input path"
                    version: "1.0.0"
                    author: "qsv test suite"
                    stages:
                      process_data:
                        type: process
                        steps:
                          load:
                            path: "${input}"
                          head:
                            number: 1
                          show:
                    """
                ).strip()
                + "\n"
            )
        result = self.run_qsv_command(
            f"quilt {quilt} --var input=tests/fixtures/sample-min.csv"
        )
        self.assertEqual(result.returncode, 0)
        self.assertTrue(result.stdout.startswith("RecordNumber,EventRecordId"))

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

    def test_sigma2quilt_json_default_output(self):
        rules_path = self.write_json_file(
            "rules_windows_generic.json",
            [
                {
                    "title": "Suspicious High IntegrityLevel Conhost Legacy Option",
                    "id": "3037d961-21e9-4732-b27a-637bcc7bf539",
                    "level": "informational",
                    "tags": ["attack.defense-evasion", "attack.t1202"],
                    "rule": [
                        "SELECT * FROM logs WHERE Channel='Security' AND EventID=4688"
                    ],
                }
            ],
        )
        expected_output = os.path.join(
            self.temp_dir, "quilt-suspicious-high-integritylevel-conhost-legacy-option.yaml"
        )
        expected_mapping = os.path.join(
            self.temp_dir,
            "quilt-suspicious-high-integritylevel-conhost-legacy-option_mapping.json",
        )
        result = self.run_qsv_command(f"sigma2quilt {rules_path}")
        self.assertEqual(result.returncode, 0)
        self.assertTrue(os.path.exists(expected_output))
        self.assertTrue(os.path.exists(expected_mapping))
        self.assertIn(expected_output, result.stdout.strip())
        self.assertIn(f"mapping: {expected_mapping}", result.stdout.strip())

    def test_sigma2quilt_json_separate_outputs_rule_named_files(self):
        rules_path = self.write_json_file(
            "rules_windows_generic.json",
            [
                {
                    "title": "Rule One",
                    "id": "rule-one",
                    "level": "informational",
                    "tags": [],
                    "rule": ["SELECT * FROM logs WHERE EventID=4688"],
                },
                {
                    "title": "Rule Two",
                    "id": "rule-two",
                    "level": "informational",
                    "tags": [],
                    "rule": ["SELECT * FROM logs WHERE EventID=4103"],
                },
            ],
        )
        output_dir = os.path.join(self.temp_dir, "separate")
        result = self.run_qsv_command(
            f"sigma2quilt {rules_path} -o {output_dir} --separate"
        )
        self.assertEqual(result.returncode, 0)
        first_output = os.path.join(output_dir, "quilt-rule-one.yaml")
        second_output = os.path.join(output_dir, "quilt-rule-two.yaml")
        combined_mapping = os.path.join(
            output_dir, "quilt-rules_windows_generic_mapping.json"
        )
        self.assertTrue(os.path.exists(first_output))
        self.assertTrue(os.path.exists(second_output))
        self.assertTrue(os.path.exists(combined_mapping))
        self.assertIn(first_output, result.stdout)
        self.assertIn(second_output, result.stdout)
        self.assertIn(f"mapping: {combined_mapping}", result.stdout)

    def test_sigma2quilt_json_directory_requires_output_dir(self):
        rules_dir = os.path.join(self.temp_dir, "rules")
        os.makedirs(rules_dir, exist_ok=True)
        self.write_json_file(
            os.path.join("rules", "rules_one.json"),
            [
                {
                    "title": "Rule One",
                    "id": "rule-one",
                    "level": "informational",
                    "tags": [],
                    "rule": ["SELECT * FROM logs WHERE EventID=4688"],
                }
            ],
        )
        result = self.run_qsv_command(f"sigma2quilt {rules_dir}")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("requires -o/--output <dir>", result.stderr)

    def test_sigma2quilt_json_output_file(self):
        rules_path = self.write_json_file(
            "rules_windows_generic.json",
            [
                {
                    "title": "PowerShell Decompress Commands",
                    "id": "1ddc1472-8e52-4f7d-9f11-eab14fc171f5",
                    "level": "informational",
                    "tags": ["attack.defense-evasion", "attack.t1140"],
                    "rule": [
                        "SELECT * FROM logs WHERE EventID=4103 AND Payload LIKE '%Expand-Archive%' ESCAPE '\\'"
                    ],
                }
            ],
        )
        output_path = os.path.join(self.temp_dir, "custom.yaml")
        result = self.run_qsv_command(f"sigma2quilt {rules_path} -o {output_path}")
        self.assertEqual(result.returncode, 0)
        self.assertTrue(os.path.exists(output_path))
        self.assertTrue(os.path.exists(os.path.join(self.temp_dir, "custom_mapping.json")))
        with open(output_path, "r", encoding="utf-8") as f:
            content = f.read()
        self.assertIn("where:", content)
        self.assertNotIn("type: sigma", content)

    def test_sigma2quilt_json_generates_mapping_template(self):
        rules_path = self.write_json_file(
            "rules_windows_generic.json",
            [
                {
                    "title": "Suspicious Command Line",
                    "id": "rule-one",
                    "level": "informational",
                    "tags": [],
                    "rule": [
                        "SELECT * FROM logs WHERE EventID=4688 AND CommandLine LIKE '%ForceV1%' ESCAPE '\\'"
                    ],
                }
            ],
        )
        generated_quilt = os.path.join(self.temp_dir, "generated.yaml")
        generated_mapping = os.path.join(self.temp_dir, "generated_mapping.json")
        convert_result = self.run_qsv_command(
            f"sigma2quilt {rules_path} -o {generated_quilt}"
        )
        self.assertEqual(convert_result.returncode, 0)
        self.assertTrue(os.path.exists(generated_mapping))
        with open(generated_mapping, "r", encoding="utf-8") as f:
            mapping = json.load(f)
        self.assertEqual(mapping["CommandLine"], "")
        self.assertEqual(mapping["EventID"], "")

    def test_sigma2quilt_json_generated_quilt_runs(self):
        rules_path = self.write_json_file(
            "rules_windows_generic.json",
            [
                {
                    "title": "Suspicious High IntegrityLevel Conhost Legacy Option",
                    "id": "3037d961-21e9-4732-b27a-637bcc7bf539",
                    "level": "informational",
                    "tags": ["attack.defense-evasion", "attack.t1202"],
                    "rule": [
                        "SELECT * FROM logs WHERE Channel='Security' AND EventID=4688 AND CommandLine LIKE '%ForceV1%' ESCAPE '\\'"
                    ],
                }
            ],
        )
        csv_path = os.path.join(self.temp_dir, "logs.csv")
        with open(csv_path, "w", encoding="utf-8") as f:
            f.write(
                "\n".join(
                    [
                        "Channel,EventID,CommandLine",
                        r"Security,4688,C:\Windows\System32\conhost.exe 0xffffffff -ForceV1",
                        r"Security,4657,noop",
                    ]
                )
                + "\n"
            )

        generated_quilt = os.path.join(self.temp_dir, "generated.yaml")
        convert_result = self.run_qsv_command(
            f"sigma2quilt {rules_path} -o {generated_quilt}"
        )
        self.assertEqual(convert_result.returncode, 0)
        self.assertTrue(os.path.exists(generated_quilt))

        output_csv = os.path.join(self.temp_dir, "result.csv")
        run_result = self.run_qsv_command(
            f"quilt {generated_quilt} --var input={csv_path} --var output={output_csv}"
        )
        self.assertEqual(run_result.returncode, 0)
        self.assertTrue(os.path.exists(output_csv))
        with open(output_csv, "r", encoding="utf-8") as f:
            content = f.read()
        self.assertIn("Security,4688", content)

    def test_sigma2quilt_json_generated_quilt_runs_with_mapping_file(self):
        rules_path = self.write_json_file(
            "rules_windows_generic.json",
            [
                {
                    "title": "Mapped Command Line",
                    "id": "3037d961-21e9-4732-b27a-637bcc7bf539",
                    "level": "informational",
                    "tags": ["attack.defense-evasion", "attack.t1202"],
                    "rule": [
                        "SELECT * FROM logs WHERE Channel='Security' AND EventID=4688 AND CommandLine LIKE '%ForceV1%' ESCAPE '\\'"
                    ],
                }
            ],
        )
        csv_path = os.path.join(self.temp_dir, "logs.csv")
        with open(csv_path, "w", encoding="utf-8") as f:
            f.write(
                "\n".join(
                    [
                        "LogChannel,Eid,CmdLine",
                        r"Security,4688,C:\Windows\System32\conhost.exe 0xffffffff -ForceV1",
                        r"Security,4657,noop",
                    ]
                )
                + "\n"
            )

        generated_quilt = os.path.join(self.temp_dir, "generated.yaml")
        generated_mapping = os.path.join(self.temp_dir, "generated_mapping.json")
        convert_result = self.run_qsv_command(
            f"sigma2quilt {rules_path} -o {generated_quilt}"
        )
        self.assertEqual(convert_result.returncode, 0)
        with open(generated_mapping, "w", encoding="utf-8") as f:
            json.dump(
                {
                    "Channel": "LogChannel",
                    "EventID": "Eid",
                    "CommandLine": "CmdLine",
                },
                f,
            )

        output_csv = os.path.join(self.temp_dir, "result.csv")
        run_result = self.run_qsv_command(
            f"quilt {generated_quilt} --mapping {generated_mapping} --var input={csv_path} --var output={output_csv}"
        )
        self.assertEqual(run_result.returncode, 0)
        self.assertTrue(os.path.exists(output_csv))
        with open(output_csv, "r", encoding="utf-8") as f:
            content = f.read()
        self.assertIn("Security,4688", content)

    def test_sigma2quilt_json_annotate(self):
        rules_path = self.write_json_file(
            "rules_windows_generic.json",
            [
                {
                    "title": "Suspicious High IntegrityLevel Conhost Legacy Option",
                    "id": "3037d961-21e9-4732-b27a-637bcc7bf539",
                    "level": "informational",
                    "tags": ["attack.defense-evasion", "attack.t1202"],
                    "rule": [
                        "SELECT * FROM logs WHERE Channel='Security' AND EventID=4688"
                    ],
                }
            ],
        )
        generated_quilt = os.path.join(self.temp_dir, "annotated.yaml")
        convert_result = self.run_qsv_command(
            f"sigma2quilt {rules_path} -o {generated_quilt} --annotate"
        )
        self.assertEqual(convert_result.returncode, 0)
        with open(generated_quilt, "r", encoding="utf-8") as f:
            quilt_content = f.read()
        self.assertIn("sigma_title:", quilt_content)
        self.assertNotIn("type: sigma", quilt_content)

        csv_path = os.path.join(self.temp_dir, "logs.csv")
        with open(csv_path, "w", encoding="utf-8") as f:
            f.write(
                "\n".join(
                    [
                        "Channel,EventID",
                        "Security,4688",
                        "Security,4657",
                    ]
                )
                + "\n"
            )
        output_csv = os.path.join(self.temp_dir, "annotated.csv")
        run_result = self.run_qsv_command(
            f"quilt {generated_quilt} --var input={csv_path} --var output={output_csv}"
        )
        self.assertEqual(run_result.returncode, 0)
        with open(output_csv, "r", encoding="utf-8") as f:
            output_content = f.read()
        self.assertIn("sigma_title", output_content)


if __name__ == "__main__":
    unittest.main()
