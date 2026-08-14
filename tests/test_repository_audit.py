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

    def test_architecture_guards_preserve_t13_to_t19_boundaries(self):
        """Keep the audit claims executable as the implementation evolves."""
        source_root = self.root / "src"
        rust = "\n".join(
            path.read_text(encoding="utf-8")
            for path in source_root.rglob("*.rs")
            if "target" not in path.parts
        )
        csv = (source_root / "controllers" / "csv.rs").read_text(encoding="utf-8")
        finalizers = (source_root / "operations" / "finalizers" / "mod.rs").read_text(
            encoding="utf-8"
        )
        finalizer_production = finalizers.split("#[cfg(test)]", 1)[0]
        show = (source_root / "operations" / "finalizers" / "show.rs").read_text(
            encoding="utf-8"
        )
        run = (source_root / "operations" / "automation" / "run.rs").read_text(
            encoding="utf-8"
        )
        pipeline = (source_root / "controllers" / "pipeline.rs").read_text(
            encoding="utf-8"
        )

        # Confidentiality: run diagnostics pass through the redaction policy,
        # and debug logging contains command/stage metadata rather than values.
        self.assertIn("fn redact_error", run)
        self.assertIn("DiagnosticPolicy", run)
        for relative in [
            Path("operations/automation/run.rs"),
            Path("controllers/csv.rs"),
            Path("controllers/batch.rs"),
        ]:
            log_source = (source_root / relative).read_text(encoding="utf-8")
            for line in log_source.splitlines():
                if "LogController::" in line and "format" in line:
                    self.assertNotRegex(
                        line,
                        r"\b(?:path|value|token|secret|parameter)\b",
                        f"raw-sensitive log formatting in {relative}: {line}",
                    )

        # Gzip and stdout are bounded/spooled paths, never whole-payload reads.
        self.assertIn("GZIP_BUFFER_SIZE", csv)
        gzip_start = csv.index("fn read_gzipped_csv_file")
        gzip_end = csv.index("fn concat_csv_files", gzip_start)
        gzip_loader = csv[gzip_start:gzip_end]
        self.assertNotIn("read_to_end", gzip_loader)
        self.assertNotIn("read_to_string", gzip_loader)
        self.assertNotIn("collect", gzip_loader)
        self.assertIn("reserve_temp_file", gzip_loader)
        self.assertIn("retain_temp_file", gzip_loader)
        self.assertIn("64 * 1024", finalizers)
        self.assertIn("FinalizerResult::Artifact", show)
        self.assertNotIn("FinalizerResult::Stdout", show)
        self.assertNotRegex(finalizers, r"write_all\([^\n]*(?:output|text)\.as_bytes")

        # Unloaded operations fail through one typed state guard.
        self.assertIn('"Error: No data loaded', pipeline)
        self.assertGreaterEqual(pipeline.count("pub fn loaded"), 2)

        # The generated operation declaration is the sole registry source;
        # both adapters consume OperationId rather than synthesizing tokens.
        definitions = (source_root / "controllers" / "definitions.rs").read_text(
            encoding="utf-8"
        )
        command_model = (source_root / "controllers" / "command_model.rs").read_text(
            encoding="utf-8"
        )
        definitions_production = definitions.split("#[cfg(test)]", 1)[0]
        self.assertEqual(definitions_production.count("define_operations! {"), 1)
        self.assertEqual(definitions_production.count("static SPECS:"), 1)
        self.assertNotIn("static SPECS", command_model)
        self.assertIn(
            "OperationId::parse",
            (source_root / "controllers" / "cli_adapter.rs").read_text(encoding="utf-8"),
        )
        self.assertIn(
            "OperationId::parse",
            (source_root / "controllers" / "yaml_adapter.rs").read_text(encoding="utf-8"),
        )

        # No-clobber publication owns the only production hard-link fallback;
        # directory publication has no link shortcut and no overwriting rename.
        self.assertNotIn("fs::hard_link", rust.replace(finalizers, ""))
        self.assertEqual(finalizer_production.count("fs::hard_link"), 2)
        directory_publish = finalizer_production[
            finalizer_production.index("pub(crate) fn publish_directory_noreplace") :
        ]
        self.assertNotIn("hard_link", directory_publish)
        self.assertNotIn("fs::rename(", finalizer_production)
        self.assertIn("publish_file_noreplace", finalizers)

    def test_run_orchestration_has_typed_phase_boundaries(self):
        automation = self.root / "src" / "operations" / "automation"
        expected = {
            "document.rs",
            "diagnostics.rs",
            "planner.rs",
            "materialization.rs",
            "executor.rs",
            "orchestrator.rs",
        }
        self.assertTrue(expected.issubset({path.name for path in automation.glob("*.rs")}))
        run_source = (automation / "run.rs").read_text(encoding="utf-8")
        self.assertIn("pub use orchestrator::{run, run_show_plan};", run_source)
        self.assertNotIn("include!", run_source)
        self.assertIn("DocumentInput", run_source)
        self.assertIn("planner::build", run_source)
        self.assertIn("MaterializationPlan", (automation / "materialization.rs").read_text(encoding="utf-8"))
        self.assertIn("ProcessRequest", (automation / "executor.rs").read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
