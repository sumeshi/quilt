import unittest
from test_base import QsvTestBase


class TestConvert(QsvTestBase):
    fixture = "sample-min.csv"

    def test_convert_json_to_yaml(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - select Payload - head 1 - convert Payload --from json --to yaml - show"
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn("SubjectDomainName: EXAMPLE", result.stdout)
        self.assertIn("SubjectUserName: Administrator", result.stdout)

    def test_convert_invalid_format(self):
        result = self.run_qsv_command(
            f"load {self.get_fixture_path(self.fixture)} - select Payload - head 1 - convert Payload --from invalid_fmt --to yaml - show"
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn("# Unsupported conversion: invalid_fmt to yaml", result.stdout)


if __name__ == "__main__":
    unittest.main()
