# Contributing to Quilt

Quilt is a Rust/Polars structured-record pipeline. The public binary is `qlt`; automation is
`qlt run`. Keep the initializer → chainable → finalizer model intact: operations append to one
`LazyFrame`, and evaluation belongs at a finalizer or documented global barrier.

## Project structure

```text
src/
├── controllers/       typed command model, parser, executor, and errors
├── operations/
│   ├── initializers/  LazyFrame sources such as load
│   ├── chainables/    lazy transformations and filters
│   ├── finalizers/    evaluation and output boundaries
│   └── automation/   v1 run documents, graph, join, concat, and branch
├── lib.rs             reusable library boundary and Rust tests
└── main.rs            binary boundary: stderr diagnostics and exit codes
tests/
├── fixtures/          deterministic CSV/JSONL/Parquet/YAML inputs
└── test_*.py          real-binary CLI and run-document contracts
```

## Development setup

Build the binary before Python tests:

```bash
cargo build --all-features --offline
```

`showtable` is part of the unconditional public surface. The no-default-features build is a
feature-matrix check, not a reduced command surface:

```bash
cargo build --no-default-features --offline
```

## Test and quality gates

The Python suite uses standard-library discovery; there is no manual test registration. The
wrapper exists for CI compatibility:

```bash
python3 -m unittest discover -s tests -p 'test_*.py'
cd tests && python3 run_tests.py
```

Run the complete Rust matrix and strict lint before submitting:

```bash
cargo test --all-features --offline
cargo test --no-default-features --offline
cargo build --all-features --offline
cargo build --no-default-features --offline
cargo clippy --all-targets --all-features --offline -- -D warnings
cargo fmt --check
git diff --check
```

Focused Python modules can be run with discovery:

```bash
python3 -m unittest discover -s tests -p 'test_run_join.py'
python3 -m unittest discover -s tests -p 'test_chainables_select.py'
```

Tests inherit `QuiltTestBase`, invoke `target/debug/qlt` with argv lists and `shell=False`, and
receive isolated temporary directories. Add a focused `test_*.py` module; discovery finds it
automatically. Run-document tests are split by schema, validation, graph, joins, concat,
parameters, operation reuse, outputs, and failures.

## Adding operations

1. Add the implementation under the appropriate `src/operations` category and return
   `Result<_, QuiltError>`.
2. Add typed arguments, registry specification, CLI parsing, and the automation adapter in the
   shared command model. CLI and run must use the same typed command and core implementation.
3. Document accepted dtypes, output schema, null/error behavior, lazy/barrier/finalizer status,
   and memory/streaming characteristics in `README.md`.
4. Add focused Rust tests for reusable behavior and real-binary Python tests for the public
   contract. Include CLI/run parity where applicable.
5. Run the full gates above and `python3 -m unittest tests.test_repository_audit`.

Unknown fields/options should fail before input I/O. Do not add compatibility aliases or a
second dispatch table. User-facing errors must carry operation/stage/path context without
leaking declared secret parameter values. Only `main.rs` may choose a process exit code.

## Performance baselines

Use the benchmark script for comparable measurements; it creates isolated temporary dump
targets and cleans them after each run:

```bash
./scripts/benchmark_baseline.sh
```

The README records a reproducible debug build/runtime baseline and the exact commands used to
repeat it. Do not commit generated binaries, benchmark output, cache files, or temporary data.

## Submitting changes

Use a focused branch and explain behavior changes, test coverage, and any memory or output
contract implications. Keep commits small enough to review and do not commit generated files.
