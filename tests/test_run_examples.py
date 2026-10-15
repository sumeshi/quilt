import unittest
from pathlib import Path

from test_base import QuiltTestBase


class TestRunExamples(QuiltTestBase):
    def test_timeline_recipe_with_windows_and_linux_columns(self):
        cases = (
            (
                "TimeCreated",
                "TimeCreated,EventId,Computer\n"
                "2026-01-01T12:00:00Z,1,host-a\n"
                "2026-01-02T12:00:00Z,2,host-b\n"
                "2026-01-03T12:00:00Z,3,host-c\n",
                "EventId",
                "2",
            ),
            (
                "timestamp",
                'timestamp,message,host\n'
                '2026-01-01T12:00:00Z,"before period",host-a\n'
                '2026-01-02T12:00:00Z,"inside period",host-b\n'
                '2026-01-03T12:00:00Z,"after period",host-c\n',
                "message",
                "inside period",
            ),
        )

        for index, (time_column, content, retained_column, expected) in enumerate(cases):
            with self.subTest(time_column=time_column):
                source = Path(self.temp_dir, f"input-{index}.csv")
                output = Path(self.temp_dir, f"output-{index}.csv")
                source.write_text(content, encoding="utf-8")
                result = self.run_cli(
                    [
                        "run",
                        str(self.root_dir / "examples" / "timeline.yaml"),
                        "--var",
                        f"input={source}",
                        "--var",
                        f"time-column={time_column}",
                        "--var",
                        "start=2026-01-02T00:00:00Z",
                        "--var",
                        "end=2026-01-02T23:59:59Z",
                        "--var",
                        f"output={output}",
                    ]
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                rows = output.read_text(encoding="utf-8").splitlines()
                self.assertIn(retained_column, rows[0].split(","))
                self.assertEqual(len(rows), 2)
                self.assertIn(expected, rows[1])
                self.assertIn("host-b", rows[1])


if __name__ == "__main__":
    unittest.main()
