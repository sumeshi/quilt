# Tests

## Policy

### Test Style

The `qsv-rs` test suite is composed of **Python subprocess-based end-to-end tests** only.
Each test invokes the real compiled binary at `target/debug/qsv` and verifies stdout, stderr,
and the process exit code.

Do not add Rust `#[test]` unit tests here. Most of `qsv` is thin orchestration over Polars
LazyFrame operations, so end-to-end coverage is the preferred approach.

### Fixture Policy

**Use `sample-min.csv` as the default fixture for all tests whenever possible.**

Prefer `sample-min.csv` over `sample.csv` unless the behavior being tested genuinely depends on
large input size. Logical correctness should be proven with the small fixture whenever possible.

| File | Purpose |
|------|---------|
| `fixtures/sample-min.csv` | Primary fixture for nearly all tests (29 rows, 27 columns) |
| `fixtures/sample-min.csv.gz` | Gzip loading tests |
| `fixtures/sample-min.tsv` | TSV separator tests |
| `fixtures/sample-min-noheader.csv` | `--no-headers` tests |
| `fixtures/sample.csv` | Larger real-world dataset with 62,031 rows, derived from the sample data in [`jpcertcc/logontracer`](https://github.com/jpcertcc/logontracer); avoid unless size is required |
| `fixtures/quilt-*.yaml` | Pipeline definitions used by `quilt` integration tests |

### `sample-min.csv` Structure

This dataset is a CSV export of Windows Security Event Log data.

```text
RecordNumber, EventRecordId, TimeCreated, EventId, Level, Provider, Channel,
ProcessId, ThreadId, Computer, ChunkNumber, UserId, MapDescription, UserName,
RemoteHost, PayloadData1..6, ExecutableInfo, HiddenRecord, SourceFile,
Keywords, ExtraDataOffset, Payload
```

It contains **27 columns and 29 data rows**. Frequently referenced values:

| Column | Value |
|--------|-------|
| `TimeCreated` | All rows are `2016-10-06 01:47:07.xxxx` within the same second |
| `EventId` | `1102` x 1, `4688` x 14, `4689` x 14 |
| `Level` | `Info` x 1, `LogAlways` x 28 |
| `MapDescription` | `Event log cleared`, `A new process has been created`, `A process has exited` |
| `UserId`, `RemoteHost` | Empty in every row |
| `Payload` | JSON string values, already quoted for CSV |

The first column, `RecordNumber`, includes a UTF-8 BOM in the source file. Polars handles this
transparently, so tests should refer to the column as `RecordNumber`.

## Running Tests

```bash
# Build the binary first
cargo build

# Run from the tests directory
cd tests
python3 run_tests.py

# Run a single test class
python3 -m unittest test_chainables_grep.TestGrep
```

## Layout

```text
tests/
├── README.md
├── run_tests.py
├── test_base.py
├── fixtures/
│   ├── sample-min.csv
│   ├── sample-min.csv.gz
│   ├── sample-min.tsv
│   ├── sample-min-noheader.csv
│   ├── sample.csv
│   ├── quilt-simple.yaml
│   ├── quilt-join.yaml
│   ├── quilt-complex.yaml
│   ├── quilt-dump.yaml
│   └── quilt-test.yaml
├── test_initializers_load.py
├── test_chainables_*.py
├── test_finalizers_*.py
└── test_quilters_quilt.py
```

## File Overview

### `test_base.py`

Defines `QsvTestBase(unittest.TestCase)`.
`run_qsv_command(command_str)` executes `target/debug/qsv <command>` through `subprocess` and
returns `CompletedProcess`. All test classes inherit from it.

### `test_initializers_load.py`

Coverage for `load`:

- Loading a single CSV file
- Loading gzip-compressed CSV data
- Loading TSV via `-s '\t'` or `--separator '\t'`
- Loading multiple files and verifying concatenated row counts
- `--low-memory`
- `--no-headers`, with automatically generated column names such as `column_1`
- Graceful failure for missing input files

### `test_chainables_select.py`

Coverage for `select`:

- Single-column selection
- Multiple comma-separated columns
- Named ranges such as `EventId:Level` and `EventId-Level`
- Numeric indexes such as `4` and `4:5`
- Mixed numeric and named references such as `4,Level`
- Failure on unknown column names

### `test_chainables_grep.py`

Coverage for `grep`, which searches all columns as strings:

- Exact and partial pattern matches
- No-match behavior, returning only the header row
- `-v` / `--invert-match`
- `-i` / `--ignorecase`
- Combined `-v -i`

### `test_chainables_contains.py`

Coverage for `contains`, which filters within one specific column:

- Basic substring filtering
- No-match behavior
- `-i` / `--ignorecase`
- Rejection of unsupported invert options

### `test_chainables_isin.py`

Coverage for `isin`:

- Single-value filtering such as `isin EventId 1102`
- Multi-value filtering such as `isin EventId 4688,4689`
- String-column filtering such as `isin Level Info`
- No-match behavior

### `test_chainables_sort.py`

Coverage for `sort`:

- Ascending and descending numeric sorts
- Ascending string sorts
- Multi-column sorting such as `sort EventId,RecordNumber`

### `test_chainables_sed.py`

Coverage for `sed`:

- Basic replacement in a specific column
- No-op behavior when nothing matches
- All-column replacement via `sed '' old new`

When `sed` is applied across all columns, numeric columns are converted to strings by design.

### `test_chainables_count.py`

Coverage for `count`, which performs `GROUP BY + COUNT` over the current DataFrame.
It does not take a column-name argument. Narrow the input first, for example:
`select Level - count`.

- Counting grouped string values
- Counting grouped numeric values

### `test_chainables_uniq.py`

Coverage for `uniq`, which removes duplicate rows.
It is usually paired with `sort`.

- Deduplicating string values
- Deduplicating numeric values

### `test_chainables_renamecol.py`

Coverage for `renamecol`:

- Basic renaming
- Failure on missing source columns

### `test_chainables_head.py` and `test_chainables_tail.py`

Coverage for `head N` and `tail N`:

- Basic row-count behavior
- Cases where `N` exceeds the dataset size

### `test_chainables_changetz.py`

Coverage for `changetz`:

- UTC to `Asia/Tokyo` conversion
- Verification that output precision is microsecond-level (`%.6f`)
- Failure on invalid timezone names

The tests also verify that `changetz` output can be piped into `timeslice` and `timeround`.

### `test_chainables_timeslice.py`

Coverage for `timeslice`:

- `--start` only
- `--end` only
- Both `--start` and `--end`
- Failure when both bounds are omitted
- Boundary values outside the data range
- Failure on missing columns
- Passing timezone-aware strings from `changetz`

Because all rows in `sample-min.csv` fall within the same second, range testing is based on
boundary values such as "before the dataset" and "after the dataset".

### `test_chainables_timeround.py`

Coverage for `timeround`:

- Units `y`, `M`, `d`, `h`, `m`, and `s`
- `--output` adding a new column
- In-place overwrite when `--output` is omitted
- Failure on invalid units
- Passing timezone-aware strings from `changetz`
- Directly handling offset strings such as `+09:00`

### `test_chainables_timeline.py`

Coverage for `timeline`:

- `--interval 1s`, which produces one bucket for `sample-min.csv`
- `--interval 1m`, which also produces one bucket
- Aggregate modes such as `--sum EventId`

The output time column is named `timeline_{interval}`, for example `timeline_1s`.

### `test_chainables_pivot.py`

Coverage for `pivot`, which converts grouped results into a pivoted layout.
This follows the current implementation, which behaves as grouped aggregation rather than a
spreadsheet-style crosstab.

### `test_chainables_convert.py`

Coverage for `convert`:

- Converting the JSON `Payload` column to YAML
- Unsupported conversions, which emit a comment rather than hard-failing

### `test_finalizers_show.py`

Coverage for `show`:

- Basic CSV output
- No extra trailing blank lines
- Streaming output via `--batch-size`

### `test_finalizers_dump.py`

Coverage for `dump`:

- Basic file output
- Streaming output matching the normal row count
- Overwriting an existing destination instead of appending

### `test_finalizers_partition.py`

Coverage for `partition`:

- Partitioning by `Level`
- Partitioning by `EventId`
- Failure on missing columns without a Rust panic

### `test_finalizers_stats.py`

Coverage for `stats`:

- Basic statistics output
- Statistics over a reduced column set

### `test_finalizers_headers.py`

Coverage for `headers`:

- `--plain` output with one column name per line
- Presence of key columns such as `TimeCreated`, `EventId`, `Level`, and `Payload`

### `test_finalizers_showtable.py`

Coverage for `showtable`:

- Truncated display such as `shape: (8+, 27)` for large-enough input
- Column names still appearing after projection

### `test_finalizers_showquery.py`

Coverage for `showquery`:

- Presence of `Logical query plan:`
- Presence of the loaded input path

### `test_quilters_quilt.py`

Integration coverage for `quilt`.
Because `quilt` dispatches to the rest of the command set internally, these tests focus on:

- `join` behavior, including `inner`, `left`, `full`, and `cross`
- `key`, `on`, and `left_on` / `right_on`
- Ensuring the internal cross-join helper column does not leak
- `concat` success and schema-mismatch failure without panic
- Chainable dispatch inside quilt
- List-based repeated steps and mapping-based backward compatibility
- `--var` parameter injection
- Dependency resolution, forward references, and cycle detection
- Multi-source joins
- `branch` stages
- `output` stages and debug-to-stderr output
- Hard failures for invalid quilt inputs such as missing YAML files or missing stage dependencies

Current `quilt` behavior that should be preserved:

- `steps` supports both sequence form (`- grep: ...`) and legacy mapping form
- Repeated step names are allowed through sequence-form steps
- `--var key=value` performs `${key}` substitution before YAML parsing
- `join` supports more than two sources when `key` or `on` is provided
- Stage dependencies are validated before execution, including forward references and cycle checks
- `type: branch` supports `params.condition` with `then_steps` and `else_steps`
- `type: output` is available for output-only stages
- `show`, `showtable`, and `headers` can emit debug output to `stderr`

## Known Design Limits

The following are current design limits and are not direct test targets:

- Streaming `show --batch-size` and `dump --batch-size` repeatedly re-run slices of the query plan
- `sed '' old new` converts numeric columns to strings
- The `quilt` dispatch table is separate from the top-level CLI command registry
