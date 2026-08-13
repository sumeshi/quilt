import unittest

from test_base import QuiltTestBase


class TestRunSchema(QuiltTestBase):
    def check(self, text, expected):
        path = self.write_run_document("schema.yaml", text)
        result = self.run_cli(["run", "--check", path])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(expected.lower(), result.stderr.lower())
        self.assertNotIn("file not found", result.stderr.lower())

    def test_version_is_required_and_exact(self):
        self.check("stages: []", "version")
        self.check("version: 2\nstages: []", "version")

    def test_stages_and_steps_are_sequences(self):
        self.check("version: 1\nstages: {}", "sequence")
        self.check("version: 1\nstages:\n  - name: x\n    steps: {}", "variant")

    def test_each_step_is_single_entry_mapping(self):
        self.check(
            "version: 1\nstages:\n"
            "  - name: x\n    steps:\n"
            "      - {load: {paths: [x]}, head: {number: 1}}\n",
            "exactly one",
        )

    def test_unknown_stage_and_step_keys_are_rejected(self):
        self.check(
            "version: 1\nstages:\n"
            "  - name: x\n    extra: true\n    steps: []\n",
            "variant",
        )
        self.check(
            "version: 1\nstages:\n"
            "  - name: x\n    steps: [{load: {paths: [x], typo: true}}]\n",
            "unknown",
        )

    def test_type_errors_are_reported_with_yaml_path(self):
        path = self.write_run_document(
            "paths.yaml",
            """
            version: 1
            stages:
              - name: input
                steps:
                  - load: {paths: [false]}
            """,
        )
        result = self.run_cli(["run", "--check", path])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("stages[0]", result.stderr)
        self.assertIn("paths", result.stderr)

    def test_multiple_static_errors_are_aggregated(self):
        path = self.write_run_document(
            "multiple.yaml",
            """
            version: 1
            stages:
              - name: x
                from: absent
                steps: [{load: {paths: [x], typo: true}}]
              - name: x
                steps: []
            """,
        )
        result = self.run_cli(["run", "--check", path])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("stages[0]", result.stderr)
        self.assertIn("stages[1]", result.stderr)

    def test_check_has_no_input_or_output_side_effects(self):
        output = self.temp_dir + "/out.csv"
        path = self.write_run_document(
            "no-io.yaml",
            """
            version: 1
            stages:
              - name: x
                steps: [{load: {paths: [missing.csv]}}]
            """,
        )
        result = self.run_cli(["run", "--check", path, "--output", output])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(__import__("os").path.exists(output))


if __name__ == "__main__":
    unittest.main()
