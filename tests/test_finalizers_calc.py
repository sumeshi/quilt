import unittest

from test_base import QsvTestBase


class TestCalc(QsvTestBase):
    fixture = "calc.csv"

    def run_calc(self, mode, column="value", extra=""):
        return self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - calc {column} --{mode} {extra}".strip()
        )

    def test_all_modes_and_exact_scalar_output(self):
        expected = {
            "sum": "10",
            "avg": "2.5",
            "min": "1",
            "max": "4",
            "median": "2.5",
            "std": "1.2909944487358056",
        }
        for mode, value in expected.items():
            with self.subTest(mode=mode):
                result = self.run_calc(mode)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, value + "\n")

    def test_nulls_empty_and_singleton_std(self):
        null_only = self.run_qsv_command(
            f"load {self.get_fixture_path('calc-null.csv')} - cast value int - calc value --sum"
        )
        self.assertEqual(null_only.returncode, 0, null_only.stderr)
        self.assertEqual(null_only.stdout, "null\n")

        filtered_empty = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - isin value 99 - calc value --sum"
        )
        self.assertEqual(filtered_empty.returncode, 0, filtered_empty.stderr)
        self.assertEqual(filtered_empty.stdout, "null\n")

        empty = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - head 0 - calc value --avg"
        )
        self.assertEqual(empty.returncode, 0, empty.stderr)
        self.assertEqual(empty.stdout, "null\n")

        singleton = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - head 1 - calc value --std"
        )
        self.assertEqual(singleton.returncode, 0, singleton.stderr)
        self.assertEqual(singleton.stdout, "null\n")

    def test_validation_and_parser_boundaries(self):
        cases = [
            (f"load {self.get_fixture_path(self.fixture)} - calc value", "exactly one"),
            (
                f"load {self.get_fixture_path(self.fixture)} - calc value --sum --avg",
                "exactly one",
            ),
            (
                f"load {self.get_fixture_path(self.fixture)} - calc value --sum 123",
                "exactly one column",
            ),
            (
                f"load {self.get_fixture_path(self.fixture)} - calc value --sum=true",
                "do not accept values",
            ),
            (
                f"load {self.get_fixture_path(self.fixture)} - calc missing --sum",
                "not found",
            ),
            (
                f"load {self.get_fixture_path('extract.csv')} - calc value --sum",
                "must be numeric",
            ),
        ]
        for command, message in cases:
            with self.subTest(command=command):
                result = self.run_qsv_command(command)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(message, result.stderr)

    def test_finalizer_boundary_and_quilt(self):
        chained = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - calc value --sum - headers --plain"
        )
        self.assertEqual(chained.returncode, 0, chained.stderr)
        self.assertEqual(chained.stdout, "10\nvalue\n")

        quilt = self.run_qsv_command(
            f"quilt {self.get_fixture_path('quilt-calc.yaml')}"
        )
        self.assertEqual(quilt.returncode, 0, quilt.stderr)
        self.assertEqual(quilt.stdout, "10\n")


if __name__ == "__main__":
    unittest.main()
