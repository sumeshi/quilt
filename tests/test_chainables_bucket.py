import os
import tempfile
import unittest

from test_base import QsvTestBase


class TestBucket(QsvTestBase):
    fixture = "bucket.csv"

    def run_bucket(self, interval, output=None, selection="when_bucket"):
        output_arg = f" --output {output}" if output else ""
        return self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - cast when datetime - bucket when {interval}{output_arg} - select {selection} - show"
        )

    def test_floors_supported_intervals(self):
        expected = {
            "1s": "when_bucket\n2024-01-02T03:04:59.000000",
            "5m": "when_bucket\n2024-01-02T03:00:00.000000",
            "1h": "when_bucket\n2024-01-02T03:00:00.000000",
            "1d": "when_bucket\n2024-01-02T00:00:00.000000",
        }
        for interval, output in expected.items():
            with self.subTest(interval=interval):
                result = self.run_bucket(interval)
                self.assertEqual(result.returncode, 0)
                self.assertEqual(result.stdout.splitlines()[0:2], output.splitlines())

    def test_pre_epoch_floor_and_null_preservation(self):
        result = self.run_bucket("1s")
        self.assertEqual(result.returncode, 0)
        self.assertIn("1969-12-31T23:59:59", result.stdout)
        stats = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - cast when datetime - bucket when 1s - select when_bucket - stats"
        )
        self.assertEqual(stats.returncode, 0)
        self.assertIn("null_count", stats.stdout)
        self.assertIn("1", stats.stdout)

    def test_custom_output_preserves_source_and_chains(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - cast when datetime - bucket when 5m --output bucketed - select when,bucketed - head 1 - show"
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(
            result.stdout.strip(),
            "when,bucketed\n2024-01-02T03:04:59.999999,2024-01-02T03:00:00.000000",
        )

    def test_invalid_intervals_and_sources_fail(self):
        for interval in ("0s", "01s", "1w", "1", "1.5s", "999999999999999999999999s"):
            with self.subTest(interval=interval):
                result = self.run_bucket(interval)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("Invalid bucket interval", result.stderr)

        missing = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - bucket missing 1s - show"
        )
        self.assertNotEqual(missing.returncode, 0)
        self.assertIn("not found", missing.stderr)

        non_datetime = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - bucket when 1s - show"
        )
        self.assertNotEqual(non_datetime.returncode, 0)
        self.assertIn("must have datetime type", non_datetime.stderr)

    def test_output_collision_and_invalid_datetime_fail(self):
        collision = self.run_qsv_command(
            f"load {self.get_fixture_path('bucket-collision.csv')} - cast when datetime - bucket when 1s - show"
        )
        self.assertNotEqual(collision.returncode, 0)
        self.assertIn("already exists", collision.stderr)

        invalid = self.run_qsv_command(
            f"load {self.get_fixture_path('cast.csv')} - cast text datetime - bucket text 1s - show"
        )
        self.assertNotEqual(invalid.returncode, 0)
        self.assertIn("Cannot cast column", invalid.stderr)

    def test_quilt_step(self):
        result = self.run_qsv_command(
            f"quilt {self.get_fixture_path('quilt-bucket.yaml')}"
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(
            result.stdout.strip(),
            "when,bucketed\n2024-01-02T03:04:59.999999,2024-01-02T03:00:00.000000",
        )

    def test_extreme_datetime_floor_overflow_is_a_clean_error(self):
        try:
            import pyarrow as pa
            import pyarrow.parquet as pq
        except ImportError:
            self.skipTest("pyarrow is unavailable for the extreme Parquet regression")

        with tempfile.NamedTemporaryFile(suffix=".parquet", delete=False) as temporary:
            path = temporary.name
        try:
            extreme = -(2**63) + 1
            table = pa.table(
                {"when": pa.array([extreme], type=pa.int64()).cast(pa.timestamp("us"))}
            )
            pq.write_table(table, path)
            result = self.run_qsv_command(f"load {path} - bucket when 1s - show")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Bucket floor overflow", result.stderr)
            self.assertNotIn("panicked", result.stderr)
        finally:
            if os.path.exists(path):
                os.unlink(path)


if __name__ == "__main__":
    unittest.main()
