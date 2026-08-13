import unittest

from test_base import QuiltTestBase


class TestRunValidation(QuiltTestBase):
    def test_unknown_keys_and_mapping_steps_are_rejected_before_io(self):
        cases = [
            ("unknown.yaml", "unknown: true", "unknown"),
            ("mapping.yaml", "steps: {load: {paths: [missing.csv]}}", "sequence"),
        ]
        for name, body, expected in cases:
            with self.subTest(name=name):
                run_file = self.write_run_document(
                    name,
                    f"""
                    version: 1
                    {body}
                    stages:
                      - name: input
                        steps: [{{load: {{paths: [missing.csv]}}}}]
                    """,
                )
                result = self.run_cli(["run", "--check", run_file])
                self.assertNotEqual(result.returncode, 0)
                self.assertTrue(
                    expected in result.stderr.lower()
                    or (name == "mapping.yaml" and "unknown field" in result.stderr.lower())
                )
                self.assertNotIn("no such file", result.stderr.lower())

    def test_cycles_and_missing_dependencies_are_rejected(self):
        run_file = self.write_run_document(
            "cycle.yaml",
            """
            version: 1
            stages:
              - name: a
                from: b
                steps: []
              - name: b
                from: a
                steps: []
            """,
        )
        result = self.run_cli(["run", "--check", run_file])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("circular", result.stderr.lower())

    def test_unknown_run_options_are_rejected(self):
        result = self.run_cli(
            ["run", self.get_fixture_path("run-simple.yaml"), "--bogus"]
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unknown option", result.stderr.lower())


if __name__ == "__main__":
    unittest.main()
