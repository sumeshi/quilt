import unittest

from test_base import QuiltTestBase


class TestRunContract(QuiltTestBase):
    def test_process_fixture_executes_through_run(self):
        result = self.run_run_document(self.get_fixture_path("run-simple.yaml"))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(result.stdout.strip().splitlines()), 30)
        self.assertTrue(result.stdout.startswith("EventId,Level"))

    def test_join_fixture_executes_and_keeps_schema(self):
        result = self.run_run_document(self.get_fixture_path("run-join.yaml"))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(result.stdout.startswith("TimeCreated,EventId,Level"))
        self.assertIn("1102,Info", result.stdout)

    def test_forward_reference_and_shared_intermediate(self):
        run_file = self.write_run_document(
            "forward.yaml",
            f"""
            version: 1
            stages:
              - name: output
                from: shared
                steps:
                  - head: {{number: 1}}
                  - show: null
              - name: shared
                from: input
                steps:
                  - select: {{columns: [EventId, Level]}}
              - name: input
                steps:
                  - load: {{paths: ["{self.get_fixture_path('sample-min.csv')}"]}}
            """,
        )
        result = self.run_run_document(run_file)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "EventId,Level\n1102,Info")

    def test_join_and_concat_nodes(self):
        run_file = self.write_run_document(
            "nodes.yaml",
            f"""
            version: 1
            stages:
              - name: input
                steps:
                  - load: {{paths: ["{self.get_fixture_path('sample-min.csv')}"]}}
              - name: left
                from: input
                steps:
                  - select: {{columns: [EventId]}}
                  - head: {{number: 1}}
              - name: right
                from: input
                steps:
                  - select: {{columns: [EventId]}}
                  - head: {{number: 1}}
              - name: joined
                join: {{inputs: [left, right], how: inner, on: [EventId]}}
              - name: output
                from: joined
                steps:
                  - show: null
            """,
        )
        result = self.run_run_document(run_file)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "EventId\n1102")

    def test_check_does_not_open_missing_input_or_write_output(self):
        output = self.temp_dir + "/output.csv"
        run_file = self.write_run_document(
            "check.yaml",
            """
            version: 1
            stages:
              - name: input
                steps:
                  - load: {paths: [missing.csv]}
            """,
        )
        result = self.run_cli(["run", "--check", run_file, "--output", output])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(__import__("os").path.exists(output))


if __name__ == "__main__":
    unittest.main()
