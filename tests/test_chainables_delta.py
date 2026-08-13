import os
import tempfile
import unittest

from test_base import QuiltTestBase


class TestDelta(QuiltTestBase):
    fixture = "delta.csv"

    def test_signed_and_float_deltas_preserve_order_and_source(self):
        signed = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'delta', 'signed', '-', 'select', 'signed,signed_delta', '-', 'show']
        )
        self.assertEqual(signed.returncode, 0)
        self.assertEqual(
            signed.stdout.strip().splitlines(),
            ["signed,signed_delta", "1,", "3,2", "-2,-5", ",", "5,"],
        )

        floating = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'delta', 'float', '-', 'select', 'float,float_delta', '-', 'show']
        )
        self.assertEqual(floating.returncode, 0)
        self.assertEqual(
            floating.stdout.strip().splitlines(),
            ["float,float_delta", "1.5,", "0.5,-1.0", "2.25,1.75", "3.0,0.75", "4.0,1.0"],
        )

    def test_unsigned_deltas_are_signed_and_nulls_propagate(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'cast', 'unsigned', 'uint', '-', 'delta', 'unsigned', '-', 'select', 'unsigned,unsigned_delta', '-', 'show']
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(
            result.stdout.strip().splitlines(),
            ["unsigned,unsigned_delta", "1,", "4,3", "2,-2", ",", "10,"],
        )

    def test_datetime_deltas_are_microsecond_durations(self):
        result = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'cast', 'datetime', 'datetime', '-', 'delta', 'datetime', '--output', 'elapsed', '-', 'stats']
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn("elapsed", result.stdout)
        self.assertIn("duration[μs]", result.stdout)

        try:
            import pyarrow.parquet as pq
        except ImportError:
            self.skipTest("pyarrow is unavailable for duration value inspection")
        with tempfile.NamedTemporaryFile(suffix=".parquet", delete=False) as temporary:
            path = temporary.name
        os.unlink(path)
        try:
            dumped = self.run_pipeline(
                ['load', str(self.get_fixture_path(self.fixture)), '-', 'cast', 'datetime', 'datetime', '-', 'delta', 'datetime', '--output', 'elapsed', '-', 'dumpcache', '--output', str(path)]
            )
            self.assertEqual(dumped.returncode, 0)
            values = pq.read_table(path).column("elapsed").to_pylist()
            self.assertIsNone(values[0])
            self.assertEqual(values[1].total_seconds(), 1.5)
            self.assertEqual(values[2].total_seconds(), -2.5)
        finally:
            if os.path.exists(path):
                os.unlink(path)

    def test_chaining_names_collisions_and_failures(self):
        chained = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'delta', 'signed', '--output', 'first', '-', 'delta', 'first', '--output', 'second', '-', 'select', 'second', '-', 'head', '3', '-', 'show']
        )
        self.assertEqual(chained.returncode, 0)
        self.assertEqual(chained.stdout.strip().splitlines(), ["second", "", "", "-7"])

        collision = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'delta', 'signed', '--output', 'float', '-', 'show']
        )
        self.assertNotEqual(collision.returncode, 0)
        self.assertIn("already exists", collision.stderr)

        missing = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'delta', 'missing', '-', 'show']
        )
        self.assertNotEqual(missing.returncode, 0)
        self.assertIn("not found", missing.stderr)

        unsupported = self.run_pipeline(
            ['load', str(self.get_fixture_path(self.fixture)), '-', 'delta', 'text', '-', 'show']
        )
        self.assertNotEqual(unsupported.returncode, 0)
        self.assertIn("must be numeric or datetime", unsupported.stderr)

    def test_uint64_out_of_range_fails_without_panic(self):
        try:
            import pyarrow as pa
            import pyarrow.parquet as pq
        except ImportError:
            self.skipTest("pyarrow is unavailable for the unsigned extreme regression")

        with tempfile.NamedTemporaryFile(suffix=".parquet", delete=False) as temporary:
            path = temporary.name
        try:
            table = pa.table(
                {"unsigned": pa.array([0, 2**64 - 1, 0], type=pa.uint64())}
            )
            pq.write_table(table, path)
            result = self.run_pipeline(
                ['load', str(path), '-', 'delta', 'unsigned', '-', 'show']
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("conversion", result.stderr.lower())
            self.assertNotIn("panicked", result.stderr.lower())
        finally:
            if os.path.exists(path):
                os.unlink(path)

    def test_run_step(self):
        result = self.run_pipeline(
            ['run', str(self.get_fixture_path('run-delta.yaml'))]
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip().splitlines()[:3], ["signed,difference", "1,", "3,2"])

    def test_uint64_high_values_have_exact_small_deltas(self):
        try:
            import pyarrow as pa
            import pyarrow.parquet as pq
        except ImportError:
            self.skipTest("pyarrow is unavailable for the UInt64 precision regression")
        with tempfile.NamedTemporaryFile(suffix=".parquet", delete=False) as temporary:
            source_path = temporary.name
        with tempfile.NamedTemporaryFile(suffix=".parquet", delete=False) as temporary:
            output_path = temporary.name
        os.unlink(output_path)
        try:
            base = 2**64 - 10
            pq.write_table(
                pa.table({"value": pa.array([base, base + 2, base - 1], type=pa.uint64())}),
                source_path,
            )
            result = self.run_pipeline(
                ['load', str(source_path), '-', 'delta', 'value', '-', 'dumpcache', '--output', str(output_path)]
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            table = pq.read_table(output_path)
            self.assertEqual(str(table.schema.field("value_delta").type), "int64")
            self.assertEqual(table.column("value_delta").to_pylist(), [None, 2, -3])
        finally:
            for path in (source_path, output_path):
                if os.path.exists(path):
                    os.unlink(path)

    def test_integral_extreme_ranges_fail_cleanly(self):
        try:
            import pyarrow as pa
            import pyarrow.parquet as pq
        except ImportError:
            self.skipTest("pyarrow is unavailable for the range regression")
        for values, dtype in (
            ([0, 2**63 - 1, -2**63], pa.int64()),
            ([0, -2**63, 2**63 - 1], pa.int64()),
            ([0, 2**64 - 1], pa.uint64()),
        ):
            with tempfile.NamedTemporaryFile(suffix=".parquet", delete=False) as temporary:
                path = temporary.name
            try:
                pq.write_table(pa.table({"value": pa.array(values, type=dtype)}), path)
                result = self.run_pipeline(['load', str(path), '-', 'delta', 'value', '-', 'show'])
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("conversion", result.stderr.lower())
                self.assertNotIn("panicked", result.stderr.lower())
            finally:
                if os.path.exists(path):
                    os.unlink(path)


if __name__ == "__main__":
    unittest.main()
