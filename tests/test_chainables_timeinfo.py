import csv
import io
from pathlib import Path

from test_base import QuiltTestBase


class TestTimeinfo(QuiltTestBase):
    def read_rows(self, result):
        self.assertEqual(result.returncode, 0, result.stderr)
        return list(csv.DictReader(io.StringIO(result.stdout)))

    def test_unsorted_times_nulls_and_timezone_boundary(self):
        path = Path(self.temp_dir) / "events.csv"
        path.write_text("when,host\n2026-10-15T01:00:00+09:00,a\n,a\n2026-10-14T18:00:00Z,a\n2026-10-14T17:00:00Z,b\n")
        rows = self.read_rows(self.run_pipeline(["load", path], ["cast", "when", "datetime", "--timezone", "UTC"], ["timeinfo", "when", "--by", "host"], ["show"]))
        by_host = {row["host"]: row for row in rows}
        self.assertEqual((by_host["a"]["rows"], by_host["a"]["valid_times"], by_host["a"]["null_times"]), ("3", "2", "1"))
        self.assertIn("2026-10-14T16:00:00", by_host["a"]["first_seen"])
        self.assertIn("2026-10-14T18:00:00", by_host["a"]["last_seen"])

    def test_empty_and_all_null_ungrouped_have_one_summary(self):
        path = Path(self.temp_dir) / "events.csv"
        for body, count in (("", "0"), (",1\n,2\n", "2")):
            with self.subTest(count=count):
                path.write_text("when,id\n" + body)
                rows = self.read_rows(self.run_pipeline(["load", path], ["cast", "when", "datetime"], ["timeinfo", "when"], ["show"]))
                self.assertEqual(len(rows), 1)
                self.assertEqual(rows[0], {"rows": count, "valid_times": "0", "null_times": count, "first_seen": "", "last_seen": ""})

    def test_file_inventory_empty_and_filtered_files_and_yaml_parity(self):
        root = Path(self.temp_dir)
        (root / "a.csv").write_text("when,host\n2026-10-15T12:00:00Z,a\n")
        (root / "b.csv").write_text("when,host\n")
        (root / "c.csv").write_text("when,host\n2026-10-16T12:00:00Z,c\n")
        steps = (["load", root / "*.csv", "--source-column", "source"], ["cast", "when", "datetime"], ["grep", "a", "--column", "host"], ["timeinfo", "when", "--by", "source"], ["show"])
        cli_rows = self.read_rows(self.run_pipeline(*steps))
        by_source = {Path(row["source"]).name: row for row in cli_rows}
        self.assertEqual({name: row["rows"] for name, row in by_source.items()}, {"a.csv": "1", "b.csv": "0", "c.csv": "0"})
        ordinary = self.read_rows(self.run_pipeline(steps[0], ["show"]))
        self.assertEqual(len(ordinary), 2)
        config = self.write_run_document("timeinfo", f"""
            version: 1
            title: file periods
            stages:
            - name: input
              steps:
              - load: {{paths: '{root}/*.csv', source-column: source}}
              - cast: {{column: when, type: datetime}}
            - name: periods
              from: input
              steps:
              - grep: {{pattern: a, columns: [host]}}
              - timeinfo: {{column: when, by: [source]}}
              - show: {{}}
        """)
        yaml_rows = self.read_rows(self.run_run_document(config))
        self.assertEqual(sorted(yaml_rows, key=lambda row: row["source"]), sorted(cli_rows, key=lambda row: row["source"]))

    def test_invalid_datetime_is_not_silently_dropped(self):
        path = Path(self.temp_dir) / "events.csv"
        path.write_text("when\nnot-a-date\n")
        result = self.run_pipeline(["load", path], ["cast", "when", "datetime"], ["timeinfo", "when"], ["show"])
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "")

    def test_empty_ndjson_file_keeps_zero_row_source_summary(self):
        root = Path(self.temp_dir)
        (root / "events.jsonl").write_text('{"when":"2026-10-15T12:00:00Z"}\n')
        (root / "empty.jsonl").touch()
        load = ["load", root / "*.jsonl", "--source-column", "source"]
        self.assertEqual(len(self.read_rows(self.run_pipeline(load, ["show"]))), 1)
        rows = self.read_rows(self.run_pipeline(load, ["cast", "when", "datetime"], ["timeinfo", "when", "--by", "source"], ["show"]))
        self.assertEqual({Path(row["source"]).name: row["rows"] for row in rows}, {"events.jsonl": "1", "empty.jsonl": "0"})

    def test_editing_source_values_does_not_lose_summary_groups(self):
        path = Path(self.temp_dir) / "events.csv"
        path.write_text("when\n2026-10-15T12:00:00Z\n")
        rows = self.read_rows(self.run_pipeline(["load", path, "--source-column", "source"], ["cast", "when", "datetime"], ["sed", "events", "renamed", "--column", "source"], ["timeinfo", "when", "--by", "source"], ["show"]))
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["rows"], "1")
        self.assertIn("renamed.csv", rows[0]["source"])

    def test_reusing_dropped_source_name_keeps_current_groups_in_cli_and_yaml(self):
        path = Path(self.temp_dir) / "events.csv"
        path.write_text("when,host\n2026-10-15T12:00:00Z,host-a\n")
        cli = self.run_pipeline(["load", path, "--source-column", "source"], ["cast", "when", "datetime"], ["select", "when,host"], ["renamecol", "host", "source"], ["timeinfo", "when", "--by", "source"], ["show"])
        config = self.write_run_document("reused-source", f"""
            version: 1
            title: reused source name
            stages:
            - name: input
              steps:
              - load: {{paths: '{path}', source-column: source}}
              - cast: {{column: when, type: datetime}}
              - select: {{columns: [when, host]}}
            - name: periods
              from: input
              steps:
              - renamecol: {{old: host, new: source}}
              - timeinfo: {{column: when, by: [source]}}
              - show: {{}}
        """)
        cli_rows = self.read_rows(cli)
        self.assertEqual(len(cli_rows), 1)
        self.assertEqual((cli_rows[0]["source"], cli_rows[0]["rows"]), ("host-a", "1"))
        self.assertEqual(self.read_rows(self.run_run_document(config)), cli_rows)
