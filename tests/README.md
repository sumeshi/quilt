# Tests

The test suite has two executable layers:

- Rust unit tests cover typed command parsing, executor/finalizer boundaries, lazy error
  handling, and graph planning.
- Python tests invoke the compiled `target/debug/qlt` binary with argv lists. They cover the
  CLI surface and the v1 `run` document contract.

The Python harness uses `shell=False`, passes every value as a separate argument, and gives
each test an isolated temporary directory. Build the binary before running it:

```bash
cargo build --all-features --offline
python3 -m unittest discover -s tests -p 'test_*.py'
cd tests && python3 run_tests.py
```

Useful Rust checks are:

```bash
cargo test --all-features --offline
cargo test --no-default-features --offline
cargo clippy --all-targets --all-features --offline -- -D warnings
```

## Fixtures

`sample-min.csv` is the default fixture: it is small, deterministic, and contains 27 columns
and 29 rows. Use the larger `sample.csv` only when input size matters. The neighboring
`sample-min.tsv`, gzip, and no-header variants cover loader options. JSONL/NDJSON fixtures
cover nested flattening; `calc-*.csv`, `cast.csv`, `delta.csv`, `parse-size-*.csv`, and the
other small CSV files cover operation-specific edge cases. `run-*.yaml` files are canonical
v1 run documents used by integration tests.

## Python modules

`test_base.py` contains the shared real-binary harness. The CLI operation tests are grouped
by subsystem:

- `test_initializers_load.py`
- `test_chainables_*.py` (bucket, cast, changetz, contains, count, delta, extract, flatten,
  grep, head, isin, parse-size, renamecol, sed, select, sort, tail, timeslice, and uniq)
- `test_finalizers_*.py` (calc, dump, dumpcache, headers, partition, show, showquery,
  showtable, and stats)
- `test_command_surface.py` and `test_datetime_contract.py`

The run-document tests are intentionally split by contract:

- `test_run_contract.py` and `test_run_schema.py`: v1 shape, command forms, and schema errors
- `test_run_validation.py` and `test_run_failures.py`: static diagnostics and contextual
  runtime failures
- `test_run_parameters.py`: typed parameters, overrides, and secret redaction
- `test_run_graph.py` and `test_run_nodes.py`: dependencies, branches, and stage execution
- `test_run_join.py` and `test_run_concat.py`: multi-input dataframe operations
- `test_run_operation_reuse.py`: chainable dispatch and repeated steps
- `test_run_outputs.py`: ordered stdout results, file outputs, caller-relative paths, and
  existing-target rejection (the existing file is preserved)
- `test_repository_audit.py`: removed public names, stale fixtures/modules, and package/binary
  metadata

Run one module from the repository root with discovery, for example:

```bash
python3 -m unittest discover -s tests -p 'test_run_join.py'
```

## Coverage principles

Logical operation behavior belongs in focused CLI tests; reusable typed behavior belongs in
Rust tests. Run tests also verify that validation happens before input I/O, that graph errors
identify their stage/path, that finalizers remain terminal, and that lazy row failures retain
operation context without exposing declared secret values. A fixed-row parse-size-to-dump
case provides stable sink/lazy evaluation evidence without relying on process RSS timing.
