"""Standard-library discovery entry point for the real-binary test layers."""

import pathlib
import sys
import unittest


def main():
    tests_dir = pathlib.Path(__file__).resolve().parent
    sys.path.insert(0, str(tests_dir))
    suite = unittest.defaultTestLoader.discover(
        str(tests_dir), pattern="test_*.py", top_level_dir=str(tests_dir)
    )
    return unittest.TextTestRunner(verbosity=1).run(suite).wasSuccessful()


if __name__ == "__main__":
    raise SystemExit(0 if main() else 1)
