import unittest

from test_base import QuiltTestBase


class TestRunConcat(QuiltTestBase):
    def document(self, how="vertical"):
        source = self.get_fixture_path("sample-min.csv")
        return self.write_run_document(
            "concat.yaml",
            f"""
            version: 1
            stages:
              - name: input
                steps: [{{load: {{paths: ["{source}"]}}}}]
              - name: first
                from: input
                steps: [{{select: {{columns: [EventId]}}}}, {{head: {{number: 1}}}}]
              - name: second
                from: input
                steps: [{{select: {{columns: [EventId]}}}}, {{head: {{number: 1}}}}]
              - name: merged
                concat: {{inputs: [first, second], how: {how}}}
              - name: output
                from: merged
                steps: [{{show: null}}]
            """,
        )

    def test_success_and_ordering(self):
        result = self.run_run_document(self.document())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip().splitlines(), ["EventId", "1102", "1102"])

    def test_shared_source_concat_is_deterministic(self):
        result = self.run_run_document(self.document())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.count("1102"), 2)

    def test_unsupported_how_is_rejected_before_io(self):
        result = self.run_cli(["run", "--check", self.document("horizontal")])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unsupported concat", result.stderr.lower())

    def test_schema_mismatch_returns_error_not_panic(self):
        path = self.write_run_document(
            "concat-mismatch.yaml",
            """
            version: 1
            stages:
              - name: left
                steps: [{load: {paths: [tests/fixtures/sample-min.csv]}}]
              - name: right
                steps: [{load: {paths: [tests/fixtures/sample-min.tsv]}}]
              - name: merged
                concat: {inputs: [left, right], how: vertical}
            """,
        )
        result = self.run_run_document(path)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("panicked", result.stderr.lower())


if __name__ == "__main__":
    unittest.main()
