import csv
import gzip
import io
import json
from pathlib import Path

from test_base import QuiltTestBase


class TestJsonlQuality(QuiltTestBase):
    def setUp(self):
        super().setUp()
        self.source = self.get_fixture_path("jsonl-quality.jsonl")

    def write_jsonl(self, name, records):
        path = Path(self.temp_dir) / name
        path.write_text("".join(json.dumps(row, ensure_ascii=False) + "\n" for row in records), encoding="utf-8")
        return path

    def rows(self, result):
        self.assertEqual(result.returncode, 0, result.stderr)
        return list(csv.DictReader(io.StringIO(result.stdout)))

    def test_implicit_show_and_flatten_preserve_data(self):
        rows = self.rows(self.run_cli(["load", self.source]))
        self.assertEqual(len(rows), 4)
        self.assertEqual(rows[0]["id"], "9007199254740993")
        self.assertEqual(json.loads(rows[0]["empty"]), {})
        flattened = self.rows(self.run_pipeline(["load", self.source], ["flatten"], ["show"]))
        self.assertEqual(flattened[0]["event.deep.user.name"], "日本語😀")
        self.assertEqual(json.loads(flattened[0]["items"])[0]["values"], [1, None, 3])
        self.assertEqual(flattened[0]["empty"], "{}")
        self.assertEqual(flattened[2]["empty"], "{}")

    def test_showtable_flattens_without_exploding(self):
        for steps in ((["showtable"],), (["flatten"], ["select", "event.deep.user.name,tags,items,empty"], ["showtable"])):
            with self.subTest(steps=steps):
                result = self.run_pipeline(["load", self.source], *steps)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("event.deep.user.name", result.stdout)
                self.assertIn("日本語😀", result.stdout)
                self.assertIn('["host"', result.stdout)
                self.assertIn("{}", result.stdout)
                self.assertNotIn("\x1b", result.stdout)
                self.assertIn("shape: (4,", result.stdout)

    def test_display_bounds_and_control_characters(self):
        path = self.write_jsonl("display.jsonl", [{"line\nkey": "日本語😀\n\t\r\x1b[31m\u202e", "huge": "漢😀" * 100000, **{f"c{i}": i for i in range(12)}}] * 10)
        result = self.run_pipeline(["load", path], ["showtable"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("shape: (8+, 14)", result.stdout)
        self.assertIn("6 columns omitted", result.stdout)
        self.assertIn("⋮", result.stdout)
        self.assertIn("…", result.stdout)
        self.assertIn("line\\nkey", result.stdout)
        self.assertIn("\\n\\t\\r", result.stdout)
        self.assertNotIn("\x1b", result.stdout)
        self.assertNotIn("\t", result.stdout)
        self.assertNotIn("\u202e", result.stdout)
        self.assertLess(len(result.stdout), 15000)

    def test_late_fields_default_full_and_bounded_error(self):
        path = self.write_jsonl("late.jsonl", [{"id": 1, "nested": {"early": 1}}] * 1001 + [{"id": 2, "nested": {"late": 9007199254740993}}])
        result = self.run_pipeline(["load", path], ["flatten"], ["tail", "1"], ["show"])
        self.assertEqual(self.rows(result)[0]["nested.late"], "9007199254740993")
        bounded = self.run_pipeline(["load", path, "--infer-schema-length", "1000"], ["show"])
        self.assertNotEqual(bounded.returncode, 0)
        self.assertIn("line 1002", bounded.stderr)
        self.assertIn("nested.late", bounded.stderr)
        self.assertIn("--infer-schema-length full", bounded.stderr)
        self.assertEqual(bounded.stdout, "")
        document = self.write_run_document("bounded-schema", f"""
            version: 1
            stages:
            - name: events
              steps:
              - load: {{paths: [{json.dumps(str(path))}], infer-schema-length: 1000}}
              - show: {{}}
        """)
        run = self.run_run_document(document)
        self.assertNotEqual(run.returncode, 0)
        self.assertIn("nested.late", run.stderr)

    def test_cross_file_conflicts_and_bounded_null_or_empty_list(self):
        first = self.write_jsonl("first.jsonl", [{"nested": {"id": 9007199254740993}}])
        second = self.write_jsonl("second.jsonl", [{"nested": {"id": 1.5}}])
        result = self.run_cli(["load", first, second])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("nested.id", result.stderr)
        self.assertIn(str(second), result.stderr)
        first = self.write_jsonl("first.jsonl", [{"id": 1}])
        second = self.write_jsonl("second.jsonl", [{"id": 18446744073709551615}])
        rows = self.rows(self.run_cli(["load", first, second]))
        self.assertEqual([row["id"] for row in rows], ["1", "18446744073709551615"])
        for records in ([{"v": None}, {"v": 1}], [{"v": []}, {"v": [1]}]):
            path = self.write_jsonl("bounded-null.jsonl", records)
            result = self.run_cli(["load", path, "--infer-schema-length", "1"])
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("inference limit", result.stderr)

    def test_blank_lines_do_not_discard_records(self):
        path = Path(self.temp_dir) / "blanks.jsonl"
        path.write_text('\n{"id":1}\n\n  \n{"id":2}\n\n')
        rows = self.rows(self.run_cli(["load", path]))
        self.assertEqual([row["id"] for row in rows], ["1", "2"])

    def test_cross_file_nested_union_and_empty_inputs(self):
        empty = self.get_fixture_path("jsonl-quality-empty.jsonl")
        blanks = Path(self.temp_dir) / "blank.jsonl"
        blanks.write_text("\n  \n\t\n")
        rows = self.rows(self.run_pipeline(["load", empty, self.source, blanks, self.get_fixture_path("jsonl-quality-other.ndjson")], ["flatten"], ["select", "id,event.deep.user.role,other"], ["show"]))
        self.assertEqual(len(rows), 5)
        self.assertEqual(rows[-1]["id"], "9007199254740997")
        self.assertEqual(rows[-1]["event.deep.user.role"], "admin")
        self.assertEqual(rows[0]["event.deep.user.role"], "")
        failed = self.run_cli(["load", empty, blanks])
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn("empty inputs", failed.stderr)

    def test_integer_extremes_and_parquet_roundtrip(self):
        for values in ([0, 9007199254740993, 18446744073709551615], [-9223372036854775808, -1, 9007199254740993]):
            with self.subTest(values=values):
                path = self.write_jsonl("ints.jsonl", [{"id": n, "nested": {"ids": [n]}} for n in values])
                output = Path(self.temp_dir) / f"ints-{values[0]}.parquet"
                result = self.run_pipeline(["load", path], ["dumpcache", "-o", output])
                self.assertEqual(result.returncode, 0, result.stderr)
                rows = self.rows(self.run_cli(["load", output]))
                self.assertEqual([int(row["id"]) for row in rows], values)
                self.assertEqual([json.loads(row["nested"])["ids"][0] for row in rows], values)

    def test_nested_parquet_roundtrip_and_empty_struct_failure_is_atomic(self):
        source = self.get_fixture_path("flatten.jsonl")
        output = Path(self.temp_dir) / "nested.parquet"
        original = self.run_cli(["load", source])
        dumped = self.run_pipeline(["load", source], ["dumpcache", "-o", output])
        self.assertEqual(dumped.returncode, 0, dumped.stderr)
        restored = self.run_cli(["load", output])
        self.assertEqual(restored.returncode, 0, restored.stderr)
        self.assertEqual(restored.stdout, original.stdout)
        unsupported = Path(self.temp_dir) / "empty-struct.parquet"
        failed = self.run_pipeline(["load", self.source], ["dumpcache", "-o", unsupported])
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn("no child field", failed.stderr)
        self.assertFalse(unsupported.exists())
        self.assertFalse(list(Path(self.temp_dir).glob(".*.qlt-stage-*")))

    def test_type_conflicts_are_diagnosed_before_output(self):
        cases = [([{"id": 9007199254740993}, {"id": 1.5}], "incompatible NDJSON types"),
                 ([{"nested": {"id": 1}}, {"nested": {"id": "one"}}], "nested.id"),
                 ([{"id": -1}, {"id": 18446744073709551615}], "signed/unsigned"),
                 ([{"id": 18446744073709551616}], "range"),
                 ([{"items": [1, "a"]}], "incompatible NDJSON types")]
        for records, message in cases:
            with self.subTest(records=records):
                path = self.write_jsonl("bad-types.jsonl", records)
                result = self.run_cli(["load", path])
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(message, result.stderr)
                self.assertIn(str(path), result.stderr)
                self.assertEqual(result.stdout, "")

    def test_nullable_empty_structs_fail_instead_of_losing_nulls(self):
        for records in ([{"empty": {}}, {"empty": None}], [{"empty": None}, {"empty": {}}],
                        [{"empty": {}}, {}], [{"items": [{}, None]}]):
            with self.subTest(records=records):
                path = self.write_jsonl("nullable-empty.jsonl", records)
                result = self.run_cli(["load", path])
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("nullable empty objects", result.stderr)
                self.assertEqual(result.stdout, "")

    def test_special_column_names_cli_and_yaml(self):
        names = ['xmlns:auto-ns3', 'literal.dot', 'a,b', 'a-b', 'space key']
        result = self.run_pipeline(["load", self.source], ["select", 'xmlns:auto-ns3,literal.dot,"a,b",a-b,space key'], ["show"])
        rows = self.rows(result)
        self.assertEqual(list(rows[0]), names)
        doc = self.write_run_document("special-keys", f"""
            version: 1
            stages:
            - name: events
              steps:
              - load: {{paths: [{json.dumps(str(self.source))}]}}
              - select: {{columns: {json.dumps(names)}}}
              - show: {{}}
        """)
        run = self.run_run_document(doc)
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual(run.stdout, result.stdout)

    def test_flatten_collision_is_explicit(self):
        for record in ({"a.b": 1, "a": {"b": 2}}, {"a.b": {"c": 1}, "a": {"b.c": 2}}):
            path = self.write_jsonl("collision.jsonl", [record])
            result = self.run_pipeline(["load", path], ["flatten"], ["show"])
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("collid", result.stderr)
            table = self.run_pipeline(["load", path], ["showtable"])
            self.assertNotEqual(table.returncode, 0)

    def test_nested_search_preserves_original_types(self):
        for operation in (["grep", "日本語"], ["contains", "event", "日本語"], ["contains", "items", "alpha"]):
            with self.subTest(operation=operation):
                rows = self.rows(self.run_pipeline(["load", self.source], operation, ["show"]))
                self.assertEqual(len(rows), 1)
                self.assertEqual(json.loads(rows[0]["items"])[0]["name"], "alpha")
                self.assertEqual(rows[0]["id"], "9007199254740993")
        for operation in (["isin", "tags", "host"], ["sed", "alpha", "beta", "--column", "items"]):
            result = self.run_pipeline(["load", self.source], operation, ["show"])
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("nested", result.stderr)

    def test_nested_cli_yaml_parity(self):
        cases = [
            ([['showtable']], [{"showtable": {}}]),
            ([['flatten'], ['select', 'event.deep.user.name,tags'], ['showtable']],
             [{"flatten": {}}, {"select": {"columns": ["event.deep.user.name", "tags"]}}, {"showtable": {}}]),
            ([['grep', '日本語'], ['show']], [{"grep": {"pattern": "日本語"}}, {"show": {}}]),
            ([['contains', 'event', '日本語'], ['show']], [{"contains": {"column": "event", "pattern": "日本語"}}, {"show": {}}]),
        ]
        for cli_steps, yaml_steps in cases:
            with self.subTest(steps=cli_steps):
                cli = self.run_pipeline(["load", self.source], *cli_steps)
                document = self.write_run_document("parity", json.dumps({
                    "version": 1, "stages": [{"name": "events", "steps": [
                        {"load": {"paths": [str(self.source)]}}, *yaml_steps,
                    ]}],
                }, ensure_ascii=False))
                run = self.run_run_document(document)
                self.assertEqual(cli.returncode, 0, cli.stderr)
                self.assertEqual(run.returncode, 0, run.stderr)
                self.assertEqual(run.stdout, cli.stdout)

    def test_malformed_json_utf8_and_nonobjects(self):
        malformed = self.run_cli(["load", self.get_fixture_path("jsonl-quality-invalid.jsonl")])
        self.assertNotEqual(malformed.returncode, 0)
        self.assertIn("line 2", malformed.stderr)
        for data, message in ((b'{"id":1}\n{"id":', "line 2"), (b'{"id":"\xff"}\n', "UTF-8"), (b'[]\n', "must be objects")):
            with self.subTest(data=data):
                path = Path(self.temp_dir) / "invalid.jsonl"
                path.write_bytes(data)
                result = self.run_cli(["load", path])
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(str(path), result.stderr)
                self.assertIn(message, result.stderr)
                self.assertEqual(result.stdout, "")

    def test_gzip_multimember_and_source_path(self):
        path = Path(self.temp_dir) / "events.ndjson.gz"
        path.write_bytes(gzip.compress(b'{"id":1}\n') + gzip.compress(b'{"id":2,"late":true}\n'))
        rows = self.rows(self.run_cli(["load", path, "--source-column", "source"]))
        self.assertEqual([row["id"] for row in rows], ["1", "2"])
        self.assertEqual(rows[0]["source"], str(path))
        path.write_bytes(path.read_bytes()[:-5])
        result = self.run_cli(["load", path])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("decompress gzip", result.stderr)
        self.assertEqual(result.stdout, "")

    def test_gzip_yaml_and_temporary_cleanup(self):
        import os
        temp_root = Path(self.temp_dir) / "spools"
        temp_root.mkdir()
        path = Path(self.temp_dir) / "events.jsonl.gz"
        path.write_bytes(gzip.compress(b'{"id":9007199254740993,"tags":["host"]}\n'))
        document = self.write_run_document("gzip", f"""
            version: 1
            stages:
            - name: events
              steps:
              - load: {{paths: [{json.dumps(str(path))}]}}
              - show: {{}}
        """)
        result = self.run_cli(["run", document], env={**os.environ, "TMPDIR": str(temp_root)})
        self.assertEqual(self.rows(result)[0]["id"], "9007199254740993")
        self.assertEqual(list(temp_root.iterdir()), [])
        plan = self.run_cli(["run", document, "--show-plan", "events"], env={**os.environ, "TMPDIR": str(temp_root)})
        self.assertNotEqual(plan.returncode, 0)
        self.assertIn("cannot inspect gzipped NDJSON", plan.stderr)
        self.assertEqual(list(temp_root.iterdir()), [])

    def test_common_operations_on_nested_records(self):
        for operation in (["count"], ["uniq"], ["sort", "id"], ["stats"], ["headers"]):
            with self.subTest(operation=operation):
                result = self.run_pipeline(["load", self.source], operation)
                self.assertEqual(result.returncode, 0, result.stderr)
        counted = self.rows(self.run_pipeline(["load", self.source], ["count", "items"], ["show"]))
        self.assertEqual(sum(int(row["count"]) for row in counted), 4)
        self.assertEqual(sorted(int(row["count"]) for row in counted), [1, 1, 2])
        unique = self.rows(self.run_pipeline(["load", self.source], ["uniq"], ["show"]))
        self.assertEqual(len(unique), 3)
        for key in ("event", "tags", "items"):
            sorted_rows = self.rows(self.run_pipeline(["load", self.source], ["sort", key], ["show"]))
            self.assertEqual(len(sorted_rows), 4)
            self.assertEqual(sorted(row["id"] for row in sorted_rows), ["", "", "9007199254740993", "9007199254740995"])
