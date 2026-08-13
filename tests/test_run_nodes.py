import unittest

from test_base import QuiltTestBase


class TestRunNodes(QuiltTestBase):
    def make_node_document(self, join):
        source = self.get_fixture_path("sample-min.csv")
        return self.write_run_document(
            "nodes.yaml",
            f"""
            version: 1
            stages:
              - name: input
                steps: [{{load: {{paths: ["{source}"]}}}}]
              - name: left
                from: input
                steps: [{{select: {{columns: [EventId]}}}}, {{head: {{number: 1}}}}]
              - name: right
                from: input
                steps: [{{select: {{columns: [EventId, Level]}}}}, {{head: {{number: 1}}}}]
              - name: merged
                {join}
              - name: output
                from: merged
                steps: [{{show: null}}]
            """,
        )

    def test_inner_join(self):
        result = self.run_run_document(self.make_node_document("join: {inputs: [left, right], how: inner, on: [EventId]}"))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "EventId,Level\n1102,Info")

    def test_cross_join_does_not_leak_helper_column(self):
        result = self.run_run_document(self.make_node_document("join: {inputs: [left, right], how: cross}"))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("__qlt_run_cross_join_key", result.stdout)

    def test_concat_schema_mismatch_fails(self):
        run_file = self.write_run_document(
            "concat-error.yaml",
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
        result = self.run_run_document(run_file)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("panicked", result.stderr.lower())


if __name__ == "__main__":
    unittest.main()
