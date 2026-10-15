# Contributing to Quilt

Quilt is a Rust/Polars structured-record pipeline. The public binary is `qlt`; automation is
`qlt run`. Keep the initializer → chainable → finalizer model intact: operations append to one
`LazyFrame`, and evaluation belongs at a finalizer or documented global barrier.

## Project structure

```text
src/
├── controllers/       typed commands, CLI/YAML adapters, pipeline, executor, resources
├── operations/
│   ├── initializers/  LazyFrame sources such as load
│   ├── chainables/    lazy transformations and filters
│   ├── finalizers/    evaluation and output boundaries
│   └── automation/   v1 run documents, graph, join, concat, and branch
├── error.rs           classified errors and operation context
├── lib.rs             reusable library boundary and Rust tests
└── main.rs            binary boundary: stderr diagnostics and exit codes
examples/              runnable recipes also exercised by CLI tests
tests/
├── fixtures/          deterministic CSV/JSONL/Parquet/YAML inputs
└── test_*.py          real-binary CLI and run-document contracts
```

## Development setup

Install a stable Rust toolchain with `rustfmt` and `clippy`, and Python 3. The CLI tests
use only Python's standard library. From the repository root, build the binary first:

```bash
cargo build --locked
```

This produces `target/debug/qlt` (`target/debug/qlt.exe` on Windows), which the Python
harness invokes directly. Rebuild it after changing Rust code; the Python suite does
not build it or use an installed `qlt` from PATH. `showtable` and the other commands
are always available; there is no application feature matrix to build twice.

Use `--offline` on Cargo commands when dependencies are already available locally.
Use the same default target directory and development profile for builds, tests, and
lint; Cargo operations should run sequentially to avoid contention and duplicate work.

## Test and quality gates

The Python suite uses one standard-library discovery launcher. `tests/run_tests.py` is retained
as an equivalent entry point: it sets the test directory on `sys.path` and returns a
process status without maintaining a second registry. Choose either launcher:

```bash
python3 -m unittest discover -s tests -p 'test_*.py'
python3 tests/run_tests.py
```

Before submitting a code change, build the current CLI once, run the Python suite
above, and run these gates:

```bash
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
```

The CI workflow runs the real CLI suite on Linux and Windows. The Linux job also runs
the Rust tests, clippy, and formatting checks using the same build directory. Release
builds belong to the release workflow; ordinary validation does not require one.

Focused Python modules can be run with discovery:

```bash
python3 -m unittest discover -s tests -p 'test_run_join.py'
python3 -m unittest discover -s tests -p 'test_chainables_select.py'
```

Tests inherit `QuiltTestBase`, invoke the platform's debug executable with argv lists and `shell=False`, and
receive isolated temporary directories. Add a focused `test_*.py` module; discovery finds it
automatically. Run-document tests are split by schema, validation, graph, joins, concat,
parameters, operation reuse, outputs, and failures.

Test observable behavior and actual failure conditions. A schema-mismatch test must reach
the schema check with existing files, rather than pass because an input path is missing.
Keep coverage for datetime/DST boundaries, lazy ownership, secret diagnostics, output
protection, and cleanup. Do not restore tests that count source strings, pin file layout,
or search for names removed during an old migration. Reuse example recipes in smoke tests
instead of maintaining a second copy of their YAML. Documentation-only changes need
representative example checks, not a new build or the entire test matrix.

## Adding operations

1. Add the implementation under the appropriate `src/operations` category and return
   `Result<_, QuiltError>`.
2. Add one `define_operations!` declaration in `controllers/definitions.rs`; keep its generated
   exhaustive `TypedCommand`/`OperationId` coverage complete, then implement the CLI and YAML
   adapters against those generated types. CLI and run must use the same typed command and core
   implementation.
3. Document accepted dtypes, output schema, null/error behavior, lazy/barrier/finalizer status,
   and memory/streaming characteristics in `README.md`.
4. Wire execution through the existing `CommandExecutor` and `Pipeline` methods. If an
   operation changes or removes the source-path column, update its source inventory so
   `timeinfo --by` cannot substitute stale file names for real groups.
5. Add focused Rust tests for shared behavior and real-binary Python tests for CLI/YAML
   integration where needed. Avoid asserting the same implementation detail at both layers.
6. Run the relevant checks above; finish the full gates before submitting code changes.

Unknown fields/options should fail before input I/O. Do not add compatibility aliases or a
second dispatch table. User-facing errors must carry operation/stage/path context without
leaking declared secret parameter values. Only `main.rs` may choose a process exit code.

## Lazy execution and temporary resources

Keep the standalone `-` chain syntax and the shared CLI/YAML typed command path.
Chainables append lazy operations; schema checks may inspect metadata, while row
conversion remains deferred until evaluation. Global operations such as sort and
aggregation may require all upstream rows even though their plan is lazy. Do not add
an eager `collect()` or a full count just to produce progress messages.

A `Pipeline` owns both its `LazyFrame` and `ExecutionResources`. Cloned lazy plans must
keep the resources that back their scans alive. Run stages retain their dependencies;
a completed materialization gets independent ownership of its Parquet, and a completed
`show` artifact owns its CSV spool. Drop old owners only after every remaining consumer
is finished with them. Deleting a path because its producing stage has finished can
break a lazy downstream scan.

Output finalizers reject existing destinations and publish only completed files or
staged directories. Preserve those guarantees on errors. Run outputs are committed
individually, so later failures must report files that already succeeded. Plans and
static validation must retain the same parameter/path origins and secret policy as
execution, without creating temporary files.

## Build disk usage

`[profile.dev]` disables debug information and incremental compilation; the test profile
inherits these settings. Keep ordinary checks on `target/debug`. Avoid alternate target
directories, redundant feature/profile matrices, and comparison builds that create a
second dependency tree. On WSL2, creating and later deleting a large tree may still leave
the VHDX enlarged. If native debugging requires symbols, enable them deliberately for
that session and account for the extra artifacts.

These files have different owners and cleanup rules:

| Files | Purpose | Cleanup |
|---|---|---|
| `target/` | Rebuildable compiler output, binaries, and test executables | `cargo clean` discards it; the next build recreates dependencies. Avoid routine clean/rebuild cycles. |
| Cargo registry under `~/.cargo/registry` by default | Shared dependency downloads and sources | Shared across projects; Quilt does not delete it. |
| Quilt temporary spool/staging files | Inputs and results still needed by a running pipeline | Released with their last owner on normal completion or handled errors; forced termination can leave files. |
| Exported CSV, Parquet, and partition directories | User investigation results | Never treat these as disposable cache. |

On Linux/WSL, inspect build and registry size with `du -sh target ~/.cargo/registry`
(adjust the registry path if `CARGO_HOME` is customized). Disabling incremental growth
does not remove artifacts created with old settings or impose a hard disk limit.
Deleting files frees Linux space but does not necessarily shrink the WSL2 VHDX.

## Performance baselines

On Linux, the benchmark script measures an existing binary. Build the debug executable
once using the setup command above, then run:

```bash
./scripts/benchmark_baseline.sh
```

It never runs Cargo. Defaults are `target/debug/qlt`, `tests/fixtures/sample.csv`, and
three repetitions. Set `QLT_BENCH_BIN`, `QLT_BENCH_INPUT`, or `QLT_BENCH_RUNS` to change
them; custom input needs the `TimeCreated`, `EventId`, and `Level` columns used by the
cases. No release build or alternate target is needed. Each run uses isolated temporary
outputs and removes them after measurement.

The output records wall time, peak RSS, sampled peak temporary disk use, published
output size, and remaining temporary-file count. Short-lived temporary files can be
missed by sampling, so zero sampled bytes does not prove no disk use. Use the same
binary, input, and repetition count for comparisons, and avoid concurrent builds or
other heavy work. Check large global operations separately; a small fixture does not
establish constant-memory behavior. Do not commit generated binaries, benchmark output,
cache files, or temporary data.

## Submitting changes

Use a focused branch and explain behavior changes, test coverage, and any memory or output
contract implications. Keep commits small enough to review and do not commit generated files.
