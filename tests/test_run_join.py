import unittest

from test_base import QuiltTestBase


class TestRunJoin(QuiltTestBase):
    def document(self, node):
        source = self.get_fixture_path("sample-min.csv")
        return self.write_run_document(
            "join.yaml",
            f"""
            version: 1
            stages:
              - name: input
                steps: [{{load: {{paths: ["{source}"]}}}}]
              - name: left
                from: input
                steps: [{{select: {{columns: [EventId, TimeCreated]}}}}, {{head: {{number: 1}}}}]
              - name: right
                from: input
                steps: [{{select: {{columns: [EventId, Level]}}}}, {{head: {{number: 1}}}}]
              - name: merged
                {node}
              - name: output
                from: merged
                steps: [{{show: null}}]
            """,
        )

    def run_node(self, node):
        result = self.run_run_document(self.document(node))
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout

    def test_inner_left_full_and_cross(self):
        for how, expected in [
            ("inner, on: [EventId]", "EventId"),
            ("left, on: [EventId]", "EventId"),
            ("full, on: [EventId]", "EventId"),
            ("cross", "EventId"),
        ]:
            with self.subTest(how=how):
                output = self.run_node(f"join: {{inputs: [left, right], how: {how}}}")
                self.assertIn(expected, output)

    def test_asymmetric_left_on_right_on(self):
        source = self.get_fixture_path("sample-min.csv")
        path = self.write_run_document(
            "asymmetric.yaml",
            f"""
            version: 1
            stages:
              - name: input
                steps: [{{load: {{paths: ["{source}"]}}}}]
              - name: left
                from: input
                steps: [{{renamecol: {{old: EventId, new: left_id}}}}, {{head: {{number: 1}}}}]
              - name: right
                from: input
                steps: [{{renamecol: {{old: EventId, new: right_id}}}}, {{head: {{number: 1}}}}]
              - name: merged
                join: {{inputs: [left, right], how: inner, left-on: [left_id], right-on: [right_id]}}
              - name: output
                from: merged
                steps: [{{show: null}}]
            """,
        )
        result = self.run_run_document(path)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("left_id", result.stdout)

    def test_multi_source_join_and_coalesce(self):
        source = self.get_fixture_path("sample-min.csv")
        path = self.write_run_document(
            "multi-join.yaml",
            f"""
            version: 1
            stages:
              - name: input
                steps: [{{load: {{paths: ["{source}"]}}}}]
              - name: a
                from: input
                steps: [{{select: {{columns: [EventId]}}}}, {{head: {{number: 1}}}}]
              - name: b
                from: input
                steps: [{{select: {{columns: [EventId]}}}}, {{head: {{number: 1}}}}]
              - name: c
                from: input
                steps: [{{select: {{columns: [EventId]}}}}, {{head: {{number: 1}}}}]
              - name: merged
                join: {{inputs: [a, b, c], how: inner, on: [EventId], coalesce: true}}
              - name: output
                from: merged
                steps: [{{show: null}}]
            """,
        )
        result = self.run_run_document(path)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "EventId\n1102")

    def test_invalid_join_key_is_static_error(self):
        self.check_invalid("join: {inputs: [a, b], how: inner}", "requires join keys")
        self.check_invalid(
            "join: {inputs: [a, b], how: inner, on: [id], left-on: [id], right-on: [id]}",
            "exactly one",
        )

    def check_invalid(self, node, expected):
        path = self.write_run_document(
            "invalid-join.yaml",
            f"version: 1\nstages:\n  - name: a\n    steps: []\n  - name: b\n    steps: []\n  - name: bad\n    {node}\n",
        )
        result = self.run_cli(["run", "--check", path])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(expected, result.stderr.lower())


if __name__ == "__main__":
    unittest.main()
