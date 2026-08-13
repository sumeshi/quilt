import unittest

from test_base import QuiltTestBase


class TestRunOperationReuse(QuiltTestBase):
    def run_step(self, step):
        source = self.get_fixture_path("sample-min.csv")
        path = self.write_run_document(
            "operation.yaml",
            f"""
            version: 1
            stages:
              - name: input
                steps:
                  - load: {{paths: ["{source}"]}}
                  - {step}
                  - show: null
            """,
        )
        return self.run_run_document(path)

    def assert_run_matches_cli(self, run_step, cli_step, source="sample-min.csv"):
        run_result = self.run_step(run_step)
        cli_result = self.run_pipeline(
            ["load", self.get_fixture_path(source)], cli_step, ["show"]
        )
        self.assertEqual(
            (run_result.returncode, run_result.stdout, run_result.stderr),
            (cli_result.returncode, cli_result.stdout, cli_result.stderr),
        )

    def test_cast_and_filter_reuse_cli_contract(self):
        self.assert_run_matches_cli(
            "cast: {column: EventId, type: int}", ["cast", "EventId", "int"]
        )
        self.assert_run_matches_cli("grep: {pattern: '1102'}", ["grep", "1102"])

    def test_bucket_and_delta_use_canonical_keys(self):
        self.assert_run_matches_cli(
            "bucket: {column: TimeCreated, interval: 1h, output: hour}",
            ["bucket", "TimeCreated", "1h", "--output", "hour"],
        )
        self.assert_run_matches_cli(
            "delta: {column: EventId, output: difference}",
            ["delta", "EventId", "--output", "difference"],
        )

    def test_extract_flatten_and_parse_size_run(self):
        self.assert_run_matches_cli(
            "extract: {column: ExecutableInfo, pattern: '(?P<exe>.*)'}",
            ["extract", "ExecutableInfo", "(?P<exe>.*)"],
        )
        self.assert_run_matches_cli("flatten: {}", ["flatten"])

        path = self.write_run_document(
            "parse-size.yaml",
            f"""
            version: 1
            stages:
              - name: input
                steps:
                  - load: {{paths: ["{self.get_fixture_path('parse-size.csv')}"]}}
                  - parse-size: {{column: size}}
                  - show: null
            """,
        )
        result = self.run_run_document(path)
        direct = self.run_pipeline(
            ["load", self.get_fixture_path("parse-size.csv")],
            ["parse-size", "size"],
            ["show"],
        )
        self.assertEqual(
            (result.returncode, result.stdout, result.stderr),
            (direct.returncode, direct.stdout, direct.stderr),
        )

    def test_run_operation_validation_matches_cli_class(self):
        result = self.run_step("cast: {column: DoesNotExist, type: int}")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not found", result.stderr.lower())


if __name__ == "__main__":
    unittest.main()
