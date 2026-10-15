# Tests

The Rust unit tests cover shared operation behavior. The Python `unittest` suite runs
the compiled CLI and checks public command and `run` YAML behavior using temporary
directories.

From the repository root:

```sh
cargo build
python3 -m unittest discover -s tests -p 'test_*.py'
cargo test
```

To run one Python module, pass its filename as the discovery pattern, for example:

```sh
python3 -m unittest discover -s tests -p 'test_run_join.py'
```

Fixtures live in `fixtures/`. Tests use `QuiltTestBase`, which passes arguments as an
argv list with shell parsing disabled and gives each test a temporary working directory.
The harness selects `qlt.exe` on Windows and `qlt` elsewhere.
