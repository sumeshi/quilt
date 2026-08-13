import shutil
import unittest
from pathlib import Path

from test_base import QuiltTestBase


class TestRunParameters(QuiltTestBase):
    def test_typed_string_parameter_reaches_operation(self):
        run_file = self.write_run_document(
            "parameter-string.yaml",
            f"""
            version: 1
            parameters:
              level: {{type: string, required: true}}
            stages:
              - name: input
                steps:
                  - load: {{paths: ["{self.get_fixture_path('sample-min.csv')}"]}}
                  - contains: {{column: Level, pattern: {{"$param": level}}}}
                  - select: {{columns: [Level]}}
                  - show: null
            """,
        )
        result = self.run_cli(["run", run_file, "--var", "level=Info"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip().splitlines(), ["Level", "Info"])

    def test_typed_int_parameter_and_cli_override(self):
        run_file = self.write_run_document(
            "parameter-int.yaml",
            f"""
            version: 1
            parameters:
              count: {{type: int, default: 1}}
            stages:
              - name: input
                steps:
                  - load: {{paths: ["{self.get_fixture_path('sample-min.csv')}"]}}
                  - head: {{number: {{"$param": count}}}}
                  - show: null
            """,
        )
        result = self.run_cli(["run", run_file, "--var", "count=2"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(result.stdout.strip().splitlines()), 3)

    def test_bool_parameter_branch_and_override(self):
        run_file = self.write_run_document(
            "parameter-bool.yaml",
            f"""
            version: 1
            parameters:
              enabled: {{type: bool, default: false}}
            stages:
              - name: input
                steps: [{{load: {{paths: ["{self.get_fixture_path('sample-min.csv')}"]}}}}]
              - name: route
                branch:
                  input: input
                  when: {{parameter: {{name: enabled, equal: true}}}}
                  then: [selected]
                  else: [fallback]
              - name: selected
                from: input
                steps: [{{select: {{columns: [EventId]}}}}, {{head: {{number: 1}}}}, {{show: null}}]
              - name: fallback
                from: input
                steps: [{{select: {{columns: [Level]}}}}, {{head: {{number: 1}}}}, {{show: null}}]
            """,
        )
        default = self.run_run_document(run_file)
        self.assertEqual(default.returncode, 0, default.stderr)
        self.assertEqual(default.stdout.strip(), "Level\nInfo")
        override = self.run_run_document(run_file, "--var", "enabled=true")
        self.assertEqual(override.returncode, 0, override.stderr)
        self.assertEqual(override.stdout.strip(), "EventId\n1102")

    def test_path_default_uses_document_directory_and_cli_override_uses_cwd(self):
        source = Path(self.temp_dir, "default.csv")
        shutil.copyfile(self.get_fixture_path("sample-min.csv"), source)
        override_source = Path(self.temp_dir, "override.csv")
        shutil.copyfile(self.get_fixture_path("sample-min.csv"), override_source)
        run_file = self.write_run_document(
            "parameter-path.yaml",
            """
            version: 1
            parameters:
              input: {type: path, default: default.csv}
            stages:
              - name: input
                steps:
                  - load: {paths: [{"$param": input}]}
                  - head: {number: 1}
                  - show: null
            """,
        )
        default = self.run_run_document(run_file)
        self.assertEqual(default.returncode, 0, default.stderr)
        self.assertEqual(len(default.stdout.strip().splitlines()), 2)
        override = self.run_cli(
            ["run", run_file, "--var", "input=override.csv"], cwd=self.temp_dir
        )
        self.assertEqual(override.returncode, 0, override.stderr)
        self.assertEqual(override.stdout, default.stdout)

    def test_whole_value_placeholder_succeeds_but_interpolation_is_rejected(self):
        valid = self.write_run_document(
            "whole-placeholder.yaml",
            f"""
            version: 1
            parameters:
              level: {{type: string, required: true}}
            stages:
              - name: input
                steps:
                  - load: {{paths: ["{self.get_fixture_path('sample-min.csv')}"]}}
                  - contains: {{column: Level, pattern: {{"$param": level}}}}
                  - show: null
            """,
        )
        result = self.run_cli(["run", valid, "--check", "--var", "level=Info"])
        self.assertEqual(result.returncode, 0, result.stderr)
        for pattern in ("prefix-${level}", "${level}"):
            invalid = self.write_run_document(
                "partial-placeholder.yaml",
                f"""
                version: 1
                parameters:
                  level: {{type: string, required: true}}
                stages:
                  - name: input
                    steps:
                      - load: {{paths: ["{self.get_fixture_path('sample-min.csv')}"]}}
                      - contains: {{column: Level, pattern: "{pattern}"}}
                """,
            )
            rejected = self.run_cli(
                ["run", invalid, "--check", "--var", "level=Info"]
            )
            self.assertNotEqual(rejected.returncode, 0)
            self.assertIn("interpolation", rejected.stderr.lower())

    def test_required_unknown_and_invalid_variables_fail(self):
        run_file = self.write_run_document(
            "parameter-errors.yaml",
            """
            version: 1
            parameters:
              count: {type: int, required: true}
            stages: []
            """,
        )
        for args, expected in [
            ([], "required"),
            (["--var", "unknown=1"], "unknown"),
            (["--var", "count=bad"], "int"),
        ]:
            with self.subTest(args=args):
                result = self.run_cli(["run", "--check", run_file, *args])
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(expected, result.stderr.lower())

    def test_secret_parameter_is_redacted(self):
        run_file = self.write_run_document(
            "parameter-secret.yaml",
            """
            version: 1
            parameters:
              input: {type: path, required: true, secret: true}
            stages:
              - name: input
                steps: [{load: {paths: [{"$param": input}]}}]
            """,
        )
        secret = "not-present-secret.csv"
        result = self.run_cli(["run", run_file, "--var", f"input={secret}"])
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn(secret, result.stderr)
        self.assertIn("redacted", result.stderr.lower())


if __name__ == "__main__":
    unittest.main()
