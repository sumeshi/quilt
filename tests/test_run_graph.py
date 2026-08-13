import unittest

from test_base import QuiltTestBase


class TestRunGraph(QuiltTestBase):
    def test_branch_then_route(self):
        run_file = self.write_run_document(
            "branch.yaml",
            f"""
            version: 1
            stages:
              - name: input
                steps: [{{load: {{paths: ["{self.get_fixture_path('sample-min.csv')}"]}}}}]
              - name: route
                branch:
                  input: input
                  when: {{row-count: {{greater-than: 0}}}}
                  then: [selected]
                  else: [fallback]
              - name: selected
                from: input
                steps: [{{select: {{columns: [EventId]}}}}, {{head: {{number: 1}}}}, {{show: null}}]
              - name: fallback
                from: input
                steps: [{{select: {{columns: [Level]}}}}, {{show: null}}]
            """,
        )
        result = self.run_cli(["run", run_file])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "EventId\n1102")

    def test_branch_else_route(self):
        run_file = self.write_run_document(
            "branch-else.yaml",
            f"""
            version: 1
            stages:
              - name: input
                steps: [{{load: {{paths: ["{self.get_fixture_path('sample-min.csv')}"]}}}}]
              - name: route
                branch:
                  input: input
                  when: {{row-count: {{greater-than: 99999}}}}
                  then: [selected]
                  else: [fallback]
              - name: selected
                from: input
                steps: [{{select: {{columns: [EventId]}}}}, {{show: null}}]
              - name: fallback
                from: input
                steps: [{{select: {{columns: [Level]}}}}, {{head: {{number: 1}}}}, {{show: null}}]
            """,
        )
        result = self.run_cli(["run", run_file])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "Level\nInfo")

    def test_missing_stage_dependency_fails(self):
        run_file = self.write_run_document(
            "missing-stage.yaml",
            """
            version: 1
            stages:
              - name: output
                from: absent
                steps: []
            """,
        )
        result = self.run_cli(["run", "--check", run_file])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing", result.stderr.lower())

    def test_shared_intermediate_can_feed_multiple_outputs(self):
        run_file = self.write_run_document(
            "shared-output.yaml",
            f"""
            version: 1
            stages:
              - name: input
                steps: [{{load: {{paths: ["{self.get_fixture_path('sample-min.csv')}"]}}}}]
              - name: shared
                from: input
                steps: [{{head: {{number: 1}}}}]
              - name: first
                from: shared
                steps: [{{select: {{columns: [EventId]}}}}, {{show: null}}]
              - name: second
                from: shared
                steps: [{{select: {{columns: [Level]}}}}, {{show: null}}]
            """,
        )
        result = self.run_cli(["run", run_file])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("EventId", result.stdout)
        self.assertIn("Level", result.stdout)


if __name__ == "__main__":
    unittest.main()
