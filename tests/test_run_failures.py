import unittest

from test_base import QuiltTestBase


class TestRunFailures(QuiltTestBase):
    def test_missing_config_and_missing_input_have_context(self):
        result = self.run_cli(["run", self.temp_dir + "/absent.yaml"])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("error", result.stderr.lower())
        path = self.write_run_document(
            "missing-input.yaml",
            """
            version: 1
            stages:
              - name: input
                steps: [{load: {paths: [absent.csv]}}]
            """,
        )
        result = self.run_run_document(path)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("input", result.stderr.lower())
        self.assertIn("steps[0]", result.stderr)

    def test_finalizer_must_be_last(self):
        path = self.write_run_document(
            "finalizer-order.yaml",
            """
            version: 1
            stages:
              - name: input
                steps:
                  - load: {paths: [absent.csv]}
                  - show: null
                  - head: {number: 1}
            """,
        )
        result = self.run_cli(["run", "--check", path])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("follow a finalizer", result.stderr.lower())

    def test_lazy_row_error_is_contextual(self):
        path = self.write_run_document(
            "lazy-error.yaml",
            f"""
            version: 1
            stages:
              - name: input
                steps:
                  - load: {{paths: ["{self.get_fixture_path('sample-min.csv')}"]}}
                  - cast: {{column: Level, type: int}}
                  - show: null
            """,
        )
        result = self.run_run_document(path)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("input", result.stderr.lower())
        self.assertIn("show", result.stderr.lower())
        self.assertIn("cast", result.stderr.lower())
        self.assertIn("conversion", result.stderr.lower())

    def test_secret_is_not_leaked_in_aggregated_errors(self):
        path = self.write_run_document(
            "secret-error.yaml",
            """
            version: 1
            parameters:
              input: {type: path, required: true, secret: true}
            stages:
              - name: input
                steps: [{load: {paths: [{"$param": input}]}}]
            """,
        )
        secret = "private-not-found.csv"
        result = self.run_run_document(path, "--var", f"input={secret}")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn(secret, result.stderr)
        self.assertIn("redacted", result.stderr.lower())


if __name__ == "__main__":
    unittest.main()
