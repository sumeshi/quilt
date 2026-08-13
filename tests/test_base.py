import shutil
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path
from typing import Sequence

class QuiltTestBase(unittest.TestCase):
    """Shared real-binary harness for CLI and run-document contract tests."""

    root_dir = Path(__file__).resolve().parents[1]
    fixtures_dir = root_dir / "tests" / "fixtures"
    qlt_path = root_dir / "target" / "debug" / "qlt"

    @classmethod
    def setUpClass(cls):
        super().setUpClass()
        if not cls.qlt_path.is_file():
            raise AssertionError(
                f"compiled qlt binary not found at {cls.qlt_path}; "
                "run 'cargo build' before Python tests"
            )

    def setUp(self):
        super().setUp()
        self.temp_dir = tempfile.mkdtemp(prefix="qlt-test-")
        self.addCleanup(shutil.rmtree, self.temp_dir, True)
    
    def get_fixture_path(self, filename):
        """
        Get the absolute path to a fixture file
        
        Args:
            filename: Name of the fixture file
            
        Returns:
            Absolute path to the fixture file as string
        """
        return str(self.fixtures_dir / filename)
    
    def run_cli(self, args: Sequence[object], *, cwd=None, env=None):
        """Run qlt with an argv list; shell parsing is deliberately disabled."""
        argv = [str(self.qlt_path), *(str(arg) for arg in args)]
        return subprocess.run(
            argv,
            capture_output=True,
            text=True,
            cwd=cwd or self.root_dir,
            env=env,
            check=False,
        )

    def run_pipeline(self, *steps: Sequence[object], cwd=None, env=None):
        """Run one or more argv-form pipeline steps separated by '-'."""
        args = []
        for index, step in enumerate(steps):
            if index:
                args.append("-")
            args.extend(str(arg) for arg in step)
        return self.run_cli(args, cwd=cwd, env=env)

    def run_run_document(self, config, *args):
        """Run a canonical v1 document using the real qlt run command."""
        return self.run_cli(["run", config, *args])

    def write_run_document(self, name, content):
        """Write a temporary canonical run document and return its path."""
        filename = name if name.startswith("run-") else f"run-{name}"
        if not filename.endswith(".yaml"):
            filename = f"{filename}.yaml"
        path = Path(self.temp_dir) / filename
        path.write_text(textwrap.dedent(content).strip() + "\n", encoding="utf-8")
        return str(path)
