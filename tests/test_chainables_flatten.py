import unittest

from test_base import QuiltTestBase


class TestFlatten(QuiltTestBase):
    fixture = "flatten.jsonl"

    def test_recursive_fields_nulls_lists_and_select(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'flatten', '-', 'select', 'id,user.name,user.profile.city,user.profile.active', '-', 'show']
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            result.stdout.strip().splitlines(),
            [
                "id,user.name,user.profile.city,user.profile.active",
                "1,Alice,Tokyo,true",
                "2,Bob,,false",
                "3,,,",
                "4,Carol,Paris,true",
            ],
        )

        headers = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'flatten', '-', 'headers']
        )
        self.assertEqual(headers.returncode, 0, headers.stderr)
        self.assertIn("tags", headers.stdout)
        self.assertIn("items", headers.stdout)

    def test_sparse_fields_across_files_and_ndjson_extension(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), str(self.get_fixture_path('flatten-2.ndjson')), '-', 'flatten', '-', 'select', 'id,user.profile.city', '-', 'show']
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip().splitlines()[-1], "5,Osaka")

    def test_collisions_and_mixed_families_fail(self):
        collision = self.run_pipeline(
            ['load', str(self.get_fixture_path('flatten-collision.jsonl')), '-', 'flatten', '-', 'show']
        )
        self.assertNotEqual(collision.returncode, 0)
        self.assertIn("collides", collision.stderr)

        mixed = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), str(self.get_fixture_path('sample-min.csv')), '-', 'show']
        )
        self.assertNotEqual(mixed.returncode, 0)
        self.assertIn("Cannot mix", mixed.stderr)

    def test_invalid_records_fail_cleanly_and_mixed_numbers_are_supported(self):
        for fixture in ("flatten-malformed.jsonl", "flatten-non-object.jsonl"):
            result = self.run_pipeline(
                ['load', str(self.get_fixture_path(fixture)), '-', 'show']
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn("panicked", result.stderr.lower())

        result = self.run_pipeline(
            ['load', str(self.get_fixture_path('flatten-mixed-types.jsonl')), '-', 'show']
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip().splitlines(), ["value", "1.0", "2.5"])

    def test_run_step(self):
        result = self.run_pipeline(
            ['run', str(self.get_fixture_path('run-flatten.yaml'))]
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("user.name", result.stdout.splitlines()[0])


if __name__ == "__main__":
    unittest.main()
