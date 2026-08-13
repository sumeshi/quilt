import unittest

from test_base import QsvTestBase


class TestCast(QsvTestBase):
    fixture = "cast.csv"

    def test_cast_int_replaces_column_and_preserves_null(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - cast number int - show"
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip().splitlines(), ["number,text,truth,when", "2,alpha,true,2023-01-02T03:04:05.123456", "7,beta,false,2024-02-03 04:05:06", ",gamma,,"])

    def test_cast_all_targets(self):
        commands = {
            "uint": "number",
            "float": "number",
            "string": "number",
            "bool": "truth",
            "datetime": "when",
        }
        for target, column in commands.items():
            with self.subTest(target=target):
                result = self.run_qsv_command(
                    f"load {self.get_fixture_path(self.fixture)} - cast {column} {target} - select {column} - head 2 - show"
                )
                self.assertEqual(result.returncode, 0)
                self.assertEqual(len(result.stdout.strip().splitlines()), 3)

    def test_cast_chains(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - cast number string - cast number int - select number - head 1 - show"
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "number\n2")

    def test_cast_rejects_invalid_values_missing_columns_and_types(self):
        cases = [
            (f"load {self.get_fixture_path(self.fixture)} - cast text int - show", "Cannot cast column"),
            (f"load {self.get_fixture_path(self.fixture)} - cast missing int - show", "not found"),
            (f"load {self.get_fixture_path(self.fixture)} - cast number decimal - show", "Unsupported cast type"),
        ]
        for command, message in cases:
            with self.subTest(command=command):
                result = self.run_qsv_command(command)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(message, result.stderr)

    def test_cast_quilt_step(self):
        result = self.run_qsv_command(
            f"quilt {self.get_fixture_path('quilt-cast.yaml')}"
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "number\n2")


if __name__ == "__main__":
    unittest.main()
