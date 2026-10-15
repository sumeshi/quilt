"""Keep the crate and executable names stable for local tooling."""
import unittest
from pathlib import Path


class TestPackageMetadata(unittest.TestCase):
    def test_crate_and_binary_names(self):
        cargo = (Path(__file__).resolve().parents[1] / "Cargo.toml").read_text(
            encoding="utf-8"
        )
        self.assertIn('[package]\nname = "quilt"', cargo)
        self.assertIn('name = "qlt"', cargo)


if __name__ == "__main__":
    unittest.main()
