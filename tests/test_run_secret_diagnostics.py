import os
import unittest
from pathlib import Path

from test_base import QuiltTestBase


class TestRunSecretDiagnostics(QuiltTestBase):
    def debug_env(self):
        env = os.environ.copy()
        env["RUST_LOG"] = "debug"
        return env

    def assert_clean(self, result, secret):
        combined = result.stdout + result.stderr
        self.assertNotIn(secret, combined)
        self.assertNotIn(repr(secret), combined)
        self.assertNotIn(secret.replace("\\", "\\\\"), combined)

    def test_secret_path_load_failure_has_structural_debug_only(self):
        secret = "secret-path-never-created.csv"
        path = self.write_run_document(
            "secret-load.yaml",
            f"""
            version: 1
            parameters:
              input: {{type: path, required: true, secret: true}}
            stages:
              - name: input
                steps: [{{load: {{paths: [{{"$param": input}}]}}}}]
            """,
        )
        for options in ([], ["--show-plan", "input"]):
            result = self.run_cli(
                ["run", path, "--var", f"input={secret}"] + options, env=self.debug_env()
            )
            self.assertNotEqual(result.returncode, 0)
            self.assert_clean(result, secret)
            self.assertIn("load", result.stderr.lower())

    def test_secret_string_is_absent_from_filter_and_sed_debug_logs(self):
        secret = "private-pattern-encoded-quoted"
        for step in (
            f'grep: {{pattern: {{"$param": pattern}}}}',
            f'contains: {{column: Level, pattern: {{"$param": pattern}}}}',
            f'sed: {{pattern: {{"$param": pattern}}, replacement: safe}}',
        ):
            with self.subTest(step=step):
                path = self.write_run_document(
                    "secret-operation.yaml",
                    f"""
                    version: 1
                    parameters:
                      pattern: {{type: string, required: true, secret: true}}
                    stages:
                      - name: input
                        steps:
                          - load: {{paths: ["{self.get_fixture_path('sample-min.csv')}"]}}
                          - {step}
                          - show: null
                    """,
                )
                result = self.run_cli(
                    ["run", path, "--var", f"pattern={secret}"], env=self.debug_env()
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assert_clean(result, secret)
                self.assertIn("Applying", result.stderr)

    def test_secret_datetime_failure_and_finalizer_output_are_redacted(self):
        secret = "not-a-datetime-secret-value"
        path = self.write_run_document(
            "secret-datetime.yaml",
            f"""
            version: 1
            parameters:
              value: {{type: string, required: true, secret: true}}
            stages:
              - name: input
                steps:
                  - load: {{paths: ["{self.get_fixture_path('sample-min.csv')}"]}}
                  - sed: {{pattern: "Info", replacement: {{"$param": value}}, column: Level}}
                  - cast: {{column: Level, type: datetime}}
                  - show: null
            """,
        )
        result = self.run_cli(
            ["run", path, "--var", f"value={secret}"], env=self.debug_env()
        )
        self.assertNotEqual(result.returncode, 0)
        self.assert_clean(result, secret)
        self.assertIn("cast", result.stderr.lower())

    def test_secret_finalizer_io_failure_is_redacted(self):
        target = Path(self.temp_dir) / "secret-finalizer-target"
        target.mkdir()
        secret = str(target)
        path = self.write_run_document(
            "secret-finalizer.yaml",
            f"""
            version: 1
            parameters:
              output: {{type: path, required: true, secret: true}}
            stages:
              - name: output
                steps:
                  - load: {{paths: ["{self.get_fixture_path('sample-min.csv')}"]}}
                  - dump: {{output: {{"$param": output}}}}
            """,
        )
        result = self.run_cli(
            ["run", path, "--var", f"output={secret}"], env=self.debug_env()
        )
        self.assertNotEqual(result.returncode, 0)
        self.assert_clean(result, secret)
        self.assertIn("dump", result.stderr.lower())

    def test_secret_scalar_values_do_not_appear_in_graph_preflight(self):
        for name, value, declaration in (
            ("secret-int", "24681357", "int"),
            ("secret-bool", "true", "bool"),
        ):
            with self.subTest(name=name):
                path = self.write_run_document(
                    name,
                    f"""
                    version: 1
                    parameters:
                      flag: {{type: {declaration}, required: true, secret: true}}
                    stages:
                      - name: route
                        branch:
                          input: absent
                          when: {{parameter: {{name: flag, equal: {{"$param": flag}}}}}}
                          then: [missing]
                          else: [also-missing]
                    """,
                )
                result = self.run_cli(
                    ["run", path, "--check", "--var", f"flag={value}"],
                    env=self.debug_env(),
                )
                self.assertNotEqual(result.returncode, 0)
                self.assert_clean(result, value)

    def test_secret_int_zero_redacts_load_step_by_provenance(self):
        path = self.write_run_document(
            "secret-zero-load.yaml",
            f"""
            version: 1
            parameters:
              length: {{type: int, required: true, secret: true}}
            stages:
              - name: input
                steps:
                  - load: {{paths: ["{self.get_fixture_path('sample-min.csv')}"], infer-schema-length: {{"$param": length}}}}
            """,
        )
        result = self.run_cli(
            ["run", path, "--var", "length=0"], env=self.debug_env()
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("stages[0].steps[0].load", result.stderr)
        self.assertIn("load", result.stderr.lower())
        self.assertIn("<redacted diagnostic>", result.stderr)
        self.assertNotIn("inference length", result.stderr.lower())

    def test_secret_bool_true_redacts_invalid_contains_step_but_nonsecret_does_not(self):
        def run(secret):
            path = self.write_run_document(
                "secret-contains.yaml",
                f"""
                version: 1
                parameters:
                  insensitive: {{type: bool, required: true, secret: {str(secret).lower()}}}
                stages:
                  - name: input
                    steps:
                      - load: {{paths: ["{self.get_fixture_path('sample-min.csv')}"]}}
                      - contains: {{column: MissingColumn, pattern: x, ignore-case: {{"$param": insensitive}}}}
                """,
            )
            return self.run_cli(
                ["run", path, "--var", "insensitive=true"], env=self.debug_env()
            )

        secret_result = run(True)
        self.assertNotEqual(secret_result.returncode, 0)
        self.assertIn("automation stage 'input', step 'steps[1]/contains'", secret_result.stderr)
        self.assertIn("contains", secret_result.stderr.lower())
        self.assertIn("<redacted diagnostic>", secret_result.stderr)
        self.assertNotIn("column not found", secret_result.stderr.lower())

        ordinary_result = run(False)
        self.assertNotEqual(ordinary_result.returncode, 0)
        self.assertIn("column not found", ordinary_result.stderr.lower())

    def test_check_redacts_only_sensitive_aggregated_diagnostic_locations(self):
        path = self.write_run_document(
            "secret-aggregate.yaml",
            f"""
            version: 1
            parameters:
              length: {{type: int, required: true, secret: true}}
            stages:
              - name: sensitive
                steps:
                  - load: {{paths: ["{self.get_fixture_path('sample-min.csv')}"], infer-schema-length: {{"$param": length}}}}
              - name: unrelated
                steps:
                  - head: {{number: nope}}
            """,
        )
        result = self.run_cli(
            ["run", path, "--check", "--var", "length=0"], env=self.debug_env()
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("stages[0].steps[0].load", result.stderr)
        self.assertIn("<redacted diagnostic>", result.stderr)
        self.assertIn("stages[1].steps[0].head", result.stderr)
        self.assertIn("valid number", result.stderr.lower())
        self.assertIn("number", result.stderr.lower())

    def test_secret_branch_predicate_preflight_location_is_redacted(self):
        path = self.write_run_document(
            "secret-branch.yaml",
            """
            version: 1
            parameters:
              flag: {type: bool, required: true, secret: true}
            stages:
              - name: route
                branch:
                  input: absent
                  when: {parameter: {name: missing, equal: {"$param": flag}}}
                  then: []
            """,
        )
        result = self.run_cli(
            ["run", path, "--check", "--var", "flag=true"], env=self.debug_env()
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("stages[0].branch", result.stderr)
        self.assertIn("<redacted diagnostic>", result.stderr)
        self.assertNotIn("unknown or unresolved parameter", result.stderr.lower())

    def test_success_debug_logs_contain_no_secret_arguments_or_metadata(self):
        source = Path(self.temp_dir) / "short-secret-input.csv"
        source.write_text("id,value\n1,x\n", encoding="utf-8")
        output = Path(self.temp_dir) / "short-secret-output.csv"
        path = self.write_run_document(
            "secret-success.yaml",
            f"""
            version: 1
            title: {{"$param": title}}
            parameters:
              title: {{type: string, required: true, secret: true}}
              column: {{type: string, required: true, secret: true}}
              replacement: {{type: string, required: true, secret: true}}
              count: {{type: int, required: true, secret: true}}
              insensitive: {{type: bool, required: true, secret: true}}
            stages:
              - name: stable-stage
                steps:
                  - load: {{paths: ["{source}"]}}
                  - sed: {{column: value, pattern: x, replacement: {{"$param": replacement}}}}
                  - contains: {{column: {{"$param": column}}, pattern: s, ignore-case: {{"$param": insensitive}}}}
                  - head: {{number: {{"$param": count}}}}
                  - dump: {{output: "{output}"}}
            """,
        )
        result = self.run_cli(
            [
                "run",
                path,
                "--var",
                "title=t",
                "--var",
                "column=id",
                "--var",
                "replacement=s",
                "--var",
                "count=1",
                "--var",
                "insensitive=true",
            ],
            env=self.debug_env(),
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(output.is_file())
        combined = result.stdout + result.stderr
        for representation in (
            "title=t",
            "column=id",
            "column 'id'",
            "replacement=s",
            "n=1",
            "ignorecase=true",
        ):
            self.assertNotIn(representation, combined)

    def test_plans_redact_short_secret_scalars_and_downstream_errors(self):
        source = Path(self.temp_dir, "plan.csv")
        source.write_text("value\nx\n")
        for secret in ("s", "private-query-literal"):
            with self.subTest(secret=secret):
                path = self.write_run_document(
                    "secret-plan.yaml",
                    f"""
                    version: 1
                    parameters:
                      replacement: {{type: string, required: true, secret: true}}
                    stages:
                      - name: input
                        steps:
                          - load: {{paths: ["{source}"]}}
                          - sed: {{column: value, pattern: x, replacement: {{"$param": replacement}}}}
                      - name: query
                        from: input
                        steps: [{{showquery: null}}]
                    """,
                )
                for options in (["--show-plan", "input"], []):
                    result = self.run_cli(["run", path, "--var", f"replacement={secret}"] + options)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIn("<redacted query plan>", result.stdout)
                    self.assertNotIn(f'"{secret}"', result.stdout)
                    if len(secret) > 1:
                        self.assert_clean(result, secret)

    def test_integer_and_boolean_secret_provenance_redacts_plans(self):
        source = self.get_fixture_path("sample-min.csv")
        path = self.write_run_document(
            "secret-scalar-plan.yaml",
            f"""
            version: 1
            parameters:
              limit: {{type: int, required: true, secret: true}}
              insensitive: {{type: bool, required: true, secret: true}}
            stages:
              - name: input
                steps:
                  - load: {{paths: ["{source}"]}}
                  - head: {{number: {{"$param": limit}}}}
                  - contains: {{column: Level, pattern: info, ignore-case: {{"$param": insensitive}}}}
                  - showquery: null
            """,
        )
        for options in ([], ["--show-plan", "input"]):
            result = self.run_cli(["run", path, "--var", "limit=1", "--var", "insensitive=true"] + options)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("<redacted query plan>", result.stdout)
            self.assertNotIn("true", result.stdout)
            self.assertNotIn("SLICE", result.stdout.upper())

    def test_short_secret_invalid_default_is_redacted_in_multiple_errors(self):
        path = self.write_run_document(
            "secret-invalid-default.yaml",
            """
            version: 1
            parameters:
              a: {type: int, default: s, secret: true}
              z: {type: int, required: true}
            stages:
              - name: input
                steps: [{head: {number: nope}}]
            """,
        )
        for options in (["--check"], ["--show-plan", "input"]):
            result = self.run_cli(["run", path] + options)
            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn('String("s")', result.stderr)
            self.assertIn("<redacted diagnostic>", result.stderr)
            self.assertIn("Missing required parameter 'z'", result.stderr)
            self.assertIn("valid number", result.stderr)

    def test_show_plan_preparation_errors_redact_secret_values(self):
        secret = "private-plan-secret"
        path = self.write_run_document(
            "secret-invalid-plan.yaml",
            """
            version: 1
            parameters:
              name: {type: string, required: true, secret: true}
            stages:
              - name: {"$param": name}
                from: missing
                steps: []
            """,
        )
        result = self.run_cli(["run", path, "--show-plan", secret, "--var", f"name={secret}"])
        self.assertNotEqual(result.returncode, 0)
        self.assert_clean(result, secret)

    def test_secret_stage_name_is_redacted_in_runtime_error_context(self):
        secret_stage = "stage-name-secret-unique"
        path = self.write_run_document(
            "secret-stage-name.yaml",
            f"""
            version: 1
            parameters:
              stage: {{type: string, required: true, secret: true}}
            stages:
              - name: {{"$param": stage}}
                steps:
                  - load: {{paths: ["{self.get_fixture_path('sample-min.csv')}"]}}
                  - contains: {{column: MissingColumn, pattern: x}}
            """,
        )
        result = self.run_cli(
            ["run", path, "--var", f"stage={secret_stage}"], env=self.debug_env()
        )
        self.assertNotEqual(result.returncode, 0)
        combined = result.stdout + result.stderr
        self.assertNotIn(secret_stage, combined)
        self.assertNotIn(repr(secret_stage), combined)
        self.assertNotIn(secret_stage.replace("-", "\\-"), combined)
        self.assertIn("automation stage '<redacted>'", combined)
        self.assertIn("contains", combined.lower())


if __name__ == "__main__":
    unittest.main()
