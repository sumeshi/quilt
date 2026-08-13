"""Repository-level checks for removed public names and compatibility scope."""

import re
import unittest
from pathlib import Path


class TestRepositoryAudit(unittest.TestCase):
    root = Path(__file__).resolve().parents[1]
    ignored_files = {
        Path("GOAL.md"),
        Path("TASKS.md"),
        Path(".tokensave/config.json"),
    }
    suffixes = {".rs", ".py", ".md", ".toml", ".yaml", ".yml", ".sh"}

    def repository_text(self):
        for path in self.root.rglob("*"):
            if (
                not path.is_file()
                or path.suffix not in self.suffixes
                or any(part in {".git", "target", "__pycache__"} for part in path.parts)
                or path.relative_to(self.root) in self.ignored_files
            ):
                continue
            yield path.relative_to(self.root), path.read_text(encoding="utf-8")

    def test_removed_public_names_are_absent(self):
        forbidden = [
            "qsv" + "-rs",
            "sigma" + "2" + "quilt",
            "batch" + "-size",
            "read" + "write",
        ]
        old_run_test = "tests/" + "test_" + "run.py"
        old_fixture_prefix = "tests/fixtures/" + "quilt-"
        old_quilter_test = "test_" + "quilters"
        violations = []
        for relative, content in self.repository_text():
            for name in forbidden:
                if name in content:
                    violations.append(f"{relative}: {name}")
            if old_run_test in content:
                violations.append(f"{relative}: obsolete run test module reference")
            if old_fixture_prefix in content:
                violations.append(f"{relative}: stale quilt fixture reference")
            if old_quilter_test in content:
                violations.append(f"{relative}: stale quilter test reference")
            if re.search(r"\bqsv\s+quilt\b|\bqlt\s+quilt\b", content):
                violations.append(f"{relative}: removed automation command")
        self.assertEqual(violations, [])

    def test_removed_modules_and_fixtures_are_not_present(self):
        removed = [
            self.root / "src/controllers/sigma_convert.rs",
            self.root / "src/operations/quilters",
        ]
        removed.extend((self.root / "tests/fixtures").glob("q" + "uilt-*"))
        removed.extend((self.root / "tests").glob("test_" + "quilters*"))
        self.assertEqual([str(path) for path in removed if path.exists()], [])

    def test_package_and_binary_contract_are_canonical(self):
        cargo = (self.root / "Cargo.toml").read_text(encoding="utf-8")
        self.assertIn('name = "quilt"', cargo)
        self.assertNotIn('name = "' + "qsv" + "-rs" + '"', cargo)
        self.assertIn('name = "qlt"', cargo)
        self.assertNotIn('name = "qsv"', cargo)

    def test_developer_docs_match_discovery_and_feature_surface(self):
        contributing = (self.root / "CONTRIBUTING.md").read_text(encoding="utf-8")
        stale_phrases = [
            "132 tests",
            "Quilters",
            "Manual Registration",
            "cargo fmt-check",
            "cargo lint",
            "optional `table`",
            "rebuild hint",
        ]
        self.assertEqual(
            [phrase for phrase in stale_phrases if phrase in contributing], []
        )
        self.assertIn("unittest discover", contributing)
        self.assertIn("--no-default-features", contributing)
        self.assertIn("cargo clippy --all-targets --all-features", contributing)


if __name__ == "__main__":
    unittest.main()
