import csv
import io
import json
from pathlib import Path

from test_base import QuiltTestBase


class TestJsonlCsv(QuiltTestBase):
    def setUp(self):
        super().setUp()
        self.source = Path(self.temp_dir) / "Security.jsonl"
        self.records = [
            {"id": 1, "event": {"code": 4624, "category": ["host"]},
             "tags": ["eventlog", '日本語,\"quoted\"\nline}'],
             "items": [{"name": "a", "values": [1, None, 3]}]},
            {"id": 2, "event": {"code": 4625, "category": []},
             "tags": [], "items": []},
            {"id": 3, "event": None, "tags": None, "items": None},
            {"id": 4},
        ]
        self.source.write_text(
            "".join(json.dumps(record, ensure_ascii=False) + "\n"
                    for record in self.records), encoding="utf-8"
        )

    def assert_nested_rows(self, rows):
        self.assertEqual(len(rows), len(self.records))
        for row, record in zip(rows, self.records):
            self.assertEqual(row["id"], str(record["id"]))
            for name in ("tags", "items"):
                if record.get(name) is None:
                    self.assertEqual(row[name], "")
                else:
                    self.assertEqual(json.loads(row[name]), record[name])

    def test_show_serializes_nested_values_as_json_cells(self):
        result = self.run_cli(["load", self.source, "-", "show"])
        self.assertEqual(result.returncode, 0, result.stderr)
        rows = list(csv.DictReader(io.StringIO(result.stdout)))
        self.assert_nested_rows(rows)
        self.assertEqual(json.loads(rows[0]["event"]), self.records[0]["event"])
        self.assertEqual(rows[2]["event"], "")

    def test_flatten_filter_and_dump_preserve_arrays(self):
        for separator in (",", "\t"):
            with self.subTest(separator=separator):
                output = Path(self.temp_dir) / ("out.csv" if separator == "," else "out.tsv")
                result = self.run_cli([
                    "load", self.source, "--infer-schema-length", "full", "-",
                    "flatten", "-", "isin", "event.code", "4624,4625", "-",
                    "dump", "-s", separator, "-o", output,
                ])
                self.assertEqual(result.returncode, 0, result.stderr)
                with output.open(encoding="utf-8", newline="") as handle:
                    rows = list(csv.DictReader(handle, delimiter=separator))
                self.assertEqual([row["event.code"] for row in rows], ["4624", "4625"])
                self.assertEqual(json.loads(rows[0]["tags"]), self.records[0]["tags"])
                self.assertEqual(json.loads(rows[1]["tags"]), [])
                self.assertEqual(json.loads(rows[0]["items"]), self.records[0]["items"])

    def test_dump_matches_show_including_null_and_missing_values(self):
        output = Path(self.temp_dir) / "out.csv"
        shown = self.run_cli(["load", self.source, "-", "show"])
        dumped = self.run_cli(["load", self.source, "-", "dump", "-o", output])
        self.assertEqual(shown.returncode, 0, shown.stderr)
        self.assertEqual(dumped.returncode, 0, dumped.stderr)
        self.assertEqual(output.read_text(encoding="utf-8"), shown.stdout)
        self.assert_nested_rows(list(csv.DictReader(io.StringIO(shown.stdout))))

    def test_run_yaml_uses_same_csv_serialization(self):
        output = Path(self.temp_dir) / "run.csv"
        document = self.write_run_document("jsonl-csv", f"""
            version: 1
            stages:
              - name: events
                steps:
                  - load: {{paths: [{json.dumps(str(self.source))}]}}
                  - flatten: {{}}
                  - dump: {{output: {json.dumps(str(output))}}}
        """)
        result = self.run_run_document(document)
        self.assertEqual(result.returncode, 0, result.stderr)
        with output.open(encoding="utf-8", newline="") as handle:
            rows = list(csv.DictReader(handle))
        self.assert_nested_rows(rows)
        self.assertEqual(rows[0]["event.code"], "4624")
