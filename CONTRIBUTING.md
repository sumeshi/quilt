# Contributing to Quilter-CSV

Thank you for your interest in contributing to Quilter-CSV!

## Development Setup

### Using Dev Container

1. Clone the repository and open in VS Code
2. Click "Reopen in Container" when prompted
3. The development environment will be set up automatically

## Project Structure

```
src/
├── controllers/     # Command parsing and control
├── operations/      # Data processing operations
│   ├── chainables/  # Transformations (select, filter, etc.)
│   ├── finalizers/  # Output operations (show, dump, etc.)
│   └── initializers/ # Data loading
└── main.rs         # Entry point
tests/              # Python test suite
```

## Testing

### Running Tests

```bash
# Run all tests (132 tests covering all features)
$ python3 tests/run_tests.py

# Run individual test modules
$ python3 tests/test_chainables_select.py
$ python3 tests/test_finalizers_show.py
$ python3 tests/test_quilters_quilt.py

# Run using unittest module
$ python3 -m unittest tests.test_chainables_select
```

### Test Requirements

- **100% Feature Coverage**: All new features must include comprehensive tests
- **Test Categories**: Initializers, Chainables, Finalizers, Quilters
- **Naming Convention**: `test_{category}_{feature}.py`
- **Base Class**: Inherit from `QsvTestBase` for consistent infrastructure
- **Manual Registration**: Add new test classes to `tests/run_tests.py`

### Adding Tests for New Features

1. **Create Test File**: Follow naming pattern `test_{category}_{feature}.py`
2. **Implement Tests**: Use `QsvTestBase` and cover all functionality
3. **Register in run_tests.py**:
   ```python
   # Add import
   from test_chainables_newfeature import TestNewFeature
   
   # Add to appropriate list
   chainables = [
       TestSelect,
       # ... existing tests ...
       TestNewFeature,  # Add here
   ]
   ```
4. **Use Fixtures**: Leverage existing test data in `tests/fixtures/`
5. **Verify Coverage**: Ensure all commands, options, and edge cases are tested

## Code Standards

```bash
# Format and lint
$ cargo fmt --all
$ cargo fmt-check
$ cargo lint

# Check before submitting
$ cargo build --release
$ python3 tests/run_tests.py
```

## Performance and Size Baselines

Use the benchmark script before and after optimization work so runtime and release binary size are captured with the same workflow:

```bash
$ ./scripts/benchmark_baseline.sh
```

The script builds `target/release/qsv`, prints the binary size in bytes, and measures representative `load`, `select`, `grep`, `timeline`, `dump`, and `quilt` commands against the test fixtures.

To compare a smaller build without the table renderer:

```bash
$ ./scripts/benchmark_baseline.sh --no-default-features
```

The optional `table` cargo feature controls `showtable` and the `comfy-table` dependency. Default builds keep `showtable`; non-`table` builds fall back to `show` when no finalizer is specified, and `showtable` prints a clear rebuild hint.

The current Polars `0.48.1` IO feature set still pulls `object_store`, `reqwest`, `rustls`, and `ring` through `polars-io` when CSV/Parquet support is enabled. The audit for smaller local-only builds did not find a narrower feature combination that preserves the current CLI surface while removing those remote IO dependencies.

## Submitting Changes

1. Create a feature branch: `git checkout -b feature/your-feature`
2. Make your changes with clear commit messages
3. Add tests for new functionality
4. Ensure all tests pass
5. Submit a pull request with a clear description

## Adding New Operations

1. **Create Operation**: Implement in `src/operations/chainables/yourfeature.rs`
2. **Module Export**: Add to `src/operations/chainables/mod.rs`
3. **Controller Integration**: Add to dataframe controller
4. **Command Parsing**: Add command parsing in `main.rs`
5. **Write Tests**: Create `tests/test_chainables_yourfeature.py`
6. **Register Tests**: Manually add test class to `tests/run_tests.py`
7. **Verify**: Run `python3 tests/run_tests.py` to ensure all tests pass

## Getting Help

- Report issues: [GitHub Issues](https://github.com/sumeshi/qsv-rs/issues)
- Ask questions: [GitHub Discussions](https://github.com/sumeshi/qsv-rs/discussions)
