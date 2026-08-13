import os
import tempfile
import unittest
from pathlib import Path

from test_base import QuiltTestBase


class TestDumpcache(QuiltTestBase):
    fixture = "sample-min.csv"

    def test_dumpcache_extension_and_load_back(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / "cache.out"
            result = self.run_pipeline(
                ['load', str(self.get_fixture_path(self.fixture)), '-', 'dumpcache', '--output', str(target)]
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            parquet = target.with_suffix(".parquet")
            self.assertTrue(parquet.exists())
            loaded = self.run_pipeline(['load', str(parquet), '-', 'head', '1', '-', 'show'])
            self.assertEqual(loaded.returncode, 0, loaded.stderr)
            self.assertIn("RecordNumber", loaded.stdout)

    def test_dumpcache_existing_target_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / "cache.parquet"
            target.write_text("sentinel")
            result = self.run_pipeline(
                ['load', str(self.get_fixture_path(self.fixture)), '-', 'dumpcache', '--output', str(target)]
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("refusing to overwrite", result.stderr)
            self.assertEqual(target.read_text(), "sentinel")

    def test_dumpcache_missing_directory_is_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / "missing" / "cache.parquet"
            result = self.run_pipeline(
                ['load', str(self.get_fixture_path(self.fixture)), '-', 'dumpcache', '--output', str(target)]
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("directory does not exist", result.stderr)

    def test_dumpcache_lazy_failure_leaves_no_target(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / "failed.parquet"
            result = self.run_pipeline(
                ['load', str(self.get_fixture_path(self.fixture)), '-', 'cast', 'Level', 'int', '-', 'dumpcache', '--output', str(target)]
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(target.exists())
            self.assertEqual(list(target.parent.glob(".failed.parquet.qlt-*")), [])

    def test_dumpcache_cli_and_run_have_same_result(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            cli_target = root / "cli.parquet"
            run_target = root / "run.parquet"
            config = root / "run.yaml"
            config.write_text(
                "version: 1\n"
                "stages:\n"
                "- name: output\n"
                "  steps:\n"
                f"  - load:\n      paths: [{self.get_fixture_path(self.fixture)}]\n"
                f"  - dumpcache:\n      output: {run_target}\n"
            )
            cli = self.run_pipeline(
                ['load', str(self.get_fixture_path(self.fixture)), '-', 'dumpcache', '--output', str(cli_target)]
            )
            run = self.run_pipeline(['run', str(config)])
            self.assertEqual(cli.returncode, 0, cli.stderr)
            self.assertEqual(run.returncode, 0, run.stderr)
            self.assertTrue(cli_target.exists())
            self.assertTrue(run_target.exists())
            self.assertGreater(cli_target.stat().st_size, 0)
            self.assertGreater(run_target.stat().st_size, 0)


if __name__ == "__main__":
    unittest.main()
