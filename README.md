# Quilt
[![MIT License](http://img.shields.io/badge/license-MIT-blue.svg?style=flat)](LICENSE)
[![CI/CD Pipeline](https://github.com/sumeshi/quilt/actions/workflows/release.yml/badge.svg?branch=main)](https://github.com/sumeshi/quilt/actions/workflows/release.yml)

![quilt](https://github.com/user-attachments/assets/06953756-430c-49d3-98bd-11c4b16c8bea)

A Rust CLI for processing CSV/TSV, JSONL/NDJSON, and Parquet data with composable pipelines and streaming-capable execution.
Built for ad-hoc analysis of logs, event exports, and forensic datasets.

> [!NOTE]
> The original version of this project was implemented in Python and can be found at [sumeshi/quilter-csv](https://github.com/sumeshi/quilter-csv). This Rust version is a complete rewrite.

## Features

- **Pipeline-style command chaining**: Chain multiple operations in a single command
- **Flexible filtering and transformation**: Perform operations like select, filter, sort, deduplicate, and timezone conversion
- **YAML workflow automation**: Compose validated `run` documents with joins, branches, and multiple outputs

Stage schema example:

```yaml
version: 1
stages:
  - name: errors
    steps:
      - load: {paths: [events.csv]}
      - grep: {pattern: ERROR}
      - dump: {output: errors.csv}
```

Workflows can branch, join datasets, reuse intermediate stages, and write multiple outputs.

## Usage
![](https://gist.githubusercontent.com/sumeshi/644af27c8960a9b6be6c7470fe4dca59/raw/2a19fafd4f4075723c731e4a8c8d21c174cf0ffb/qlt.svg)

### Getting Help

To see available commands and options, run `qlt` without any arguments:

```bash
$ qlt -h
```

### Example

Here's an example of reading a CSV file, extracting rows that contain 4624 in the 'Event ID' column, and displaying the top 3 rows sorted by the 'Date and Time' column:

```bash
$ qlt load Security.csv - isin 'Event ID' 4624 - sort 'Date and Time' - head 3 - showtable
```

This command:
1. Loads `Security.csv`
2. Filters rows where `Event ID` is 4624
3. Sorts by `Date and Time`
4. Shows the first 3 rows as a table

### Command Structure

qlt commands are composed of three types of steps:

- **Initializer**: Loads data (e.g., `load`)
- **Chainable**: Transforms or filters data (e.g., `select`, `grep`, `sort`, etc.)
- **Finalizer**: Outputs or summarizes data (e.g., `show`, `showtable`, `headers`, etc.)

Each step is separated by a hyphen (`-`):

```bash
$ qlt <INITIALIZER> <args> - <CHAINABLE> <args> - <FINALIZER> <args>
```

### Command Separator `-`

The `-` token (a single hyphen surrounded by spaces) is the command separator. A standalone `-` is never treated as data.

- To separate commands: `qlt load file.csv - select col1 - head 5`
- If you need `-` as an option value, use an attached form such as `--separator=-` or `-s-`.
- If a positional value begins with `-`, pass `--` first: `qlt load file.csv - grep -- -Info`.

A standalone `-` positional value, including stdin-style usage, is not currently supported.

If no finalizer is specified, Quilt uses machine-readable `show`. This does not
depend on TTY or build features. `showtable` is always a bounded preview and is
never the implicit finalizer.

```bash
$ qlt load data.csv - select col1,col2 - head 5
# Equivalent to:
$ qlt load data.csv - select col1,col2 - head 5 - show
```

## Command Reference

### Initializers

#### `load`
Load one or more CSV, JSONL/NDJSON, or Parquet files.

**Supported formats:**
- CSV files (.csv, .tsv, .txt)
- Gzipped CSV files (.csv.gz)
- JSON Lines files (.jsonl, .ndjson), including nested objects
- Parquet files (.parquet) - high performance, preserves data types

| Parameter     | Type        | Default | Description                                      |
|---------------|-------------|---------|--------------------------------------------------|
| path          | list[str] |         | One or more paths to CSV, JSONL/NDJSON, or Parquet files. Quoted glob patterns such as `"logs/*.tsv"` are supported. Input families cannot be mixed in one command. |
| -s, --separator | str       | `,`     | Field separator character (CSV files only).     |
| --low-memory  | flag    | `false` | Enable low-memory mode for very large files (CSV files only). |
| --no-headers  | flag    | `false` | Treat the first row as data, not headers (CSV files only). When enabled, columns will be named automatically (`column_1`, `column_2`, etc.). |
| --chunk-size  | int     | (auto)  | Number of rows to read per chunk (CSV files only). Controls memory usage during file processing. |
| --infer-schema-length | int or `full` | `1000` | Number of NDJSON records inspected per file for schema inference. Use `full` when sparse fields require a complete scan. |

**Environment Variables:**
- `QLT_CHUNK_SIZE`: Positive default chunk size for CSV processing. An explicit `--chunk-size` takes precedence and bypasses this variable; malformed, zero, or overflowing environment values are errors.

Example:
```bash
$ qlt load data.csv
$ qlt load data.csv.gz
$ qlt load data1.csv data2.csv data3.csv
$ qlt load events.jsonl - flatten - select user.name,process.command_line - show
$ qlt load "logs/*.tsv" -s $'\t'
$ qlt load "logs/*.tsv" --separator=$'\t'
$ qlt load data.csv --low-memory
$ qlt load data.csv --no-headers
$ qlt load data.csv --chunk-size 50000
$ qlt load events.ndjson --infer-schema-length full
$ qlt load cache.parquet                              # Load from parquet cache
$ qlt load cache1.parquet cache2.parquet              # Load multiple parquet files
```

### Chainable Functions

#### `select`
Select columns by name, numeric index, or range notation.

| Parameter | Type                | Default | Description                                                                                                |
|-----------|---------------------|---------|------------------------------------------------------------------------------------------------------------|
| colnames  | str/list/range      |         | Column name(s) or indices. Supports multiple formats (see examples below). This is a required argument. |

**Column Selection Formats:**
- **Individual columns**: `col1,col3` - Select specific columns by name
- **Numeric indices**: `1,3` - Select columns by position (1-based indexing)  
- **Range notation (hyphen)**: `col1-col3` - Select range using hyphen
- **Range notation (colon)**: `col1:col3` - Select range using colon
- **Numeric range**: `2:4` - Select 2nd through 4th columns (e.g., col1, col2, col3)
- **Quoted colon notation**: `"col:1":"col:3"` - For column names containing colons
- **Mixed formats**: `1,col2,4:6` - Combine different selection methods

**Disambiguation rule:** If an exact column name matching `col1-col3` exists, it is selected as-is. Range expansion only occurs when no exact match is found.

```bash
$ qlt load data.csv - select datetime                       # Select single column by name
$ qlt load data.csv - select col1,col3                      # Select specific columns by name
$ qlt load data.csv - select col1-col3                      # Select range using hyphen
$ qlt load data.csv - select col1:col3                      # Select range using colon
$ qlt load data.csv - select 1                              # Select 1st column (datetime)
$ qlt load data.csv - select 2:4                            # Select 2nd-4th columns (col1, col2, col3)
$ qlt load data.csv - select 2,4                            # Select 2nd and 4th columns (col1, col3)
$ qlt load data.csv - select "col:1":"col:3"                # For columns with colons in names
$ qlt load data.csv - select 1,datetime,3:5                 # Mixed selection methods
```

#### `cast`
Strictly converts one column in place while preserving its position and name.

| Parameter | Type | Description |
|-----------|------|-------------|
| column | str | Source column name. Required. |
| type | str | `int` (`Int64`), `uint` (`UInt64`), `float` (`Float64`), `string`, `bool`, or `datetime`. Required. |

Nulls remain null and invalid non-null values fail at finalization. Datetime
parsing is shared with `changetz`, `timeslice`, and string-input `bucket` and
stores `Datetime[μs]` values. Canonical options are `--strict`,
`--input-format FORMAT`, `--epoch-unit s|ms|us|ns`, `--timezone ZONE`,
`--ambiguous error|earliest|latest`, and
`--nonexistent error|shift-forward|shift-backward`. Defaults are fuzzy mode,
no explicit format/epoch/timezone, and `error` for both DST policies.

Precedence is explicit format, strict RFC3339/ISO and known formats, explicit
epoch unit, then bounded fuzzy forms. Strict disables fuzzy parsing; ambiguous
numeric dates require `--input-format`. Supported forms include RFC3339,
ISO-like dates, Apache access-log timestamps, month-name dates, weekday/month
log forms, and numeric dates with a time. Embedded free-text extraction is not
supported. Epoch units are never inferred, negatives are accepted, and values
normalize to microseconds; nanoseconds below one microsecond are rejected.

```bash
$ qlt load data.csv - cast EventId int - show
$ qlt load data.csv - cast timestamp datetime - show
```

#### `parse-size`
Converts human-readable size values to integer byte counts in place.

| Parameter | Type | Description |
|-----------|------|-------------|
| column | str | Source column name. Required. |

Supported case-sensitive units are `B`, `KB`, `MB`, `GB`, `TB`, `KiB`, `MiB`,
`GiB`, and `TiB`. SI units use powers of 1000; IEC units use powers of 1024.
Surrounding whitespace is ignored. Decimal magnitudes are accepted only when
the exact byte result is integral. The output type is `UInt64`; nulls remain
null, while negative, malformed, unknown-unit, fractional-byte, and overflow
values fail the command.

```bash
$ qlt load data.csv - parse-size size - show
```

#### `flatten`
Recursively expands nested JSON object (Struct) fields into dot-notated columns.
Scalar fields such as `user.name` and `process.command_line` can then be selected
normally. Lists and arrays, including lists of objects, remain list-valued and are
not exploded. Missing or null nested fields remain null. Name collisions between
existing columns and generated paths fail before the frame is changed.

```bash
$ qlt load events.jsonl - flatten - select user.name,process.command_line - show
```

#### `bucket`
Floors a datetime column to a fixed positive interval and adds a new column.

| Parameter | Type | Description |
|-----------|------|-------------|
| column | str | Datetime source column. Required. |
| interval | str | Positive interval matching `^[1-9][0-9]*(s\|m\|h\|d)$`. |
| `-o`, `--output` | str | Output name; defaults to `<column>_bucket`. |

The source may be typed datetime or string. Typed input preserves source
unit/timezone metadata and rejects datetime parsing options. String input uses
the shared datetime options (default bounded parsing is valid without options)
and produces `Datetime[μs]` with requested timezone
metadata. Flooring uses checked Euclidean division, so negative timestamps floor
toward negative infinity; existing output names are rejected. Millisecond
inputs require an interval divisible by 1ms.

```bash
$ qlt load data.csv - cast timestamp datetime - bucket timestamp 5m - show
$ qlt load data.csv - cast timestamp datetime - bucket timestamp 1h --output hour - show
```

#### `delta`
Calculates the current value minus the previous row without reordering or
replacing the source column.

| Parameter | Type | Description |
|-----------|------|-------------|
| column | str | Numeric or datetime source column. Required. |
| `-o`, `--output` | str | Output name; defaults to `<column>_delta`. |

Integral deltas use lossless Int64 differences (including descending unsigned
values), using an internal Decimal128 subtraction before the final Int64 cast;
UInt64 values or differences outside i64::MAX return a contextual conversion
error at finalization. Float32 and Float64 retain their source precision.
Datetime deltas use Duration[μs] with checked unit conversion. The first row
and any pair involving a null produce null. Existing output names are rejected.

```bash
$ qlt load data.csv - delta count - show
$ qlt load data.csv - cast timestamp datetime - delta timestamp --output elapsed - show
```

#### `extract`
Extracts named Rust regex capture groups into new nullable String columns while
preserving the source column.

| Parameter | Type | Description |
|-----------|------|-------------|
| column | str | String source column. Required. |
| regex | str | Rust regex with one or more named groups. Required. |

Unmatched rows and absent optional groups become null. Existing output names,
invalid regexes, and regexes without named groups are rejected. Quote the regex
for your shell; this command does not add an expression language.

```bash
$ qlt load data.csv - extract message '(?P<user>[^@]+)@(?P<domain>.+)' - show
$ qlt load data.csv - extract path '^(?P<dir>.*)/(?P<file>[^/]+)$' - show
```

#### `isin`
Filter rows where a column matches any of the given values.

| Parameter | Type   | Default | Description                                                                          |
|-----------|--------|---------|--------------------------------------------------------------------------------------|
| colname   | str    |         | Column name to filter. Required.                                                     |
| values    | list   |         | Comma-separated values. Filters rows where the column matches any of these values (OR condition). Required. |

```bash
$ qlt load data.csv - isin col1 1
$ qlt load data.csv - isin col1 1,4
```

#### `contains`
Filter rows where a column contains a specific literal substring.

| Parameter   | Type   | Default | Description                                 |
|-------------|--------|---------|---------------------------------------------|
| colname     | str    |         | Column name to search. Required.            |
| substring   | str    |         | The literal substring to search for. Required. |
| -i, --ignore-case | flag | `false` | Perform case-insensitive matching.          |

```bash
$ qlt load data.csv - contains str ba
$ qlt load data.csv - contains str BA -i
$ qlt load data.csv - contains str BA --ignore-case
```

#### `sed`
Replace values in column(s) using a Regex pattern.

| Parameter   | Type   | Default | Description                                 |
|-------------|--------|---------|---------------------------------------------|
| pattern     | str    |         | Regex pattern to search for. Required.      |
| replacement | str    |         | Replacement string. Required.               |
| --column    | str    | (all)   | Apply replacement to specific column only. If not specified, applies to all columns. |
| -i, --ignore-case | flag | `false` | Perform case-insensitive matching.          |

> **Warning:** When `--column` is omitted, `sed` replaces across **all columns**. In log/DFIR data this can silently modify timestamps, EventIDs, file paths, and usernames. Always specify `--column` unless you intend a full-dataset replacement.

```bash
$ qlt load data.csv - sed foo foooooo                       # Replace 'foo' with 'foooooo' in all columns
$ qlt load data.csv - sed foo foooooo --column str          # Replace 'foo' with 'foooooo' in 'str' column only
$ qlt load data.csv - sed FOO foooooo -i                    # Case-insensitive replacement in all columns
$ qlt load data.csv - sed ".*o.*" foooooo --column str      # Regex replacement in specific column
```

#### `grep`
Filter rows where any column matches a regex pattern.

| Parameter | Type | Default | Description |
|---|---|---|---|
| pattern | str |         | Regex pattern to search for in any column. Required. |
| --column | str | (all columns) | Restrict search to specific column(s). Comma-separated for multiple. |
| -i, --ignore-case | flag | `false` | Perform case-insensitive matching. |
| -v, --invert-match | flag | `false` | Invert the sense of matching, to select non-matching lines. |

Example:
```bash
$ qlt load data.csv - grep foo
$ qlt load data.csv - grep "^FOO" -i                        # Case-insensitive search
$ qlt load data.csv - grep "^FOO" --ignore-case              # Long form case-insensitive
$ qlt load data.csv - grep "^FOO" -i -v                     # Case-insensitive inverted match
$ qlt load data.csv - grep "^FOO" --ignore-case --invert-match  # Long form inverted match
$ qlt load logs.csv - grep "FAILED" --column EventData
$ qlt load logs.csv - grep "192\\.168\\." --column src_ip,dst_ip
```

#### `head`
Limits the dataset to its first N rows.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| number | int  | 5       | Number of rows to keep. Can be specified as positional argument or with -n/--number option. |
| -n, --number | int | | Alternative way to specify number of rows. |

```bash
$ qlt load data.csv - head 3
$ qlt load data.csv - head 10
$ qlt load data.csv - head -n 3
$ qlt load data.csv - head --number 10
```

#### `tail`
Keeps the last N rows of the dataset.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| number | int  | 5       | Number of rows to keep. Can be specified as positional argument or with -n/--number option. |
| -n, --number | int | | Alternative way to specify number of rows. |

```bash
$ qlt load data.csv - tail 3
$ qlt load data.csv - tail 10
$ qlt load data.csv - tail -n 3
$ qlt load data.csv - tail --number 10
```

#### `sort`
Sorts the dataset based on the specified column(s).

> ⚠️ **Memory:** This is a global operation and may require memory proportional to the input.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| colnames  | str/list |         | Column name(s) to sort by. Comma-separated for multiple columns (e.g., `col1,col3`) or a single column name. Required. |
| -d, --desc    | flag | `false` | Sort in descending order. Applies to all specified columns. |

```bash
$ qlt load data.csv - sort str
$ qlt load data.csv - sort str -d
$ qlt load data.csv - sort str --desc
$ qlt load data.csv - sort col1,col2,col3 --desc
```

#### `count`
Count duplicate rows, grouping by all columns by default. Results are automatically sorted by count in descending order.

> ⚠️ **Memory:** This is a global operation and may require memory proportional to the input.

| Parameter | Type | Default | Description |
|---|---|---|---|
| columns   | str | (all columns) | Optional positional column list. Use `col1` or `col1,col2` to group by specific columns only. |

```bash
$ qlt load Security.csv - count EventID          # Count by one column
$ qlt load proxy.csv - count src_ip,dst_ip       # Count by multiple columns
$ qlt load data.csv - count                       # Count all unique rows (original behavior)
$ qlt load data.csv - count - sort col1          # Count and then sort by col1 instead
```

#### `uniq`
Filters unique rows, removing duplicates based on all columns.

> ⚠️ **Memory:** This is a global operation and may require memory proportional to the input.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| (None)    |      |         | Takes no arguments. Removes duplicate rows based on all columns. |

```bash
$ qlt load data.csv - uniq
```

#### `changetz`
Changes the timezone of a datetime column.

| Parameter | Type | Default | Description |
|---|---|---|---|
| colname | str |         | Name of the datetime column. Required. |
| --from-tz | str |         | Source timezone (e.g., `UTC`, `America/New_York`, `local`). Required. |
| --to-tz | str |         | Target timezone (e.g., `Asia/Tokyo`). Required. |
| --input-format | str | none | Chrono/strftime format; takes precedence over automatic parsing. |
| --output-format | str | ISO-like | Output format; default `%Y-%m-%dT%H:%M:%S%.6f%:z`. |
| --strict | flag | false | Disable bounded fuzzy parsing. |
| --epoch-unit | s/ms/us/ns | none | Interpret numeric input only with this explicit unit. |
| --ambiguous | error/earliest/latest | error | DST fall-back policy. |
| --nonexistent | error/shift-forward/shift-backward | error | DST spring-forward policy. |

Offset-bearing input is an instant and its offset is authoritative over
`--from-tz`; values normalize to UTC at microsecond precision before rendering
in `--to-tz`. Without an offset, `--from-tz` localizes the wall-clock value.
Shift policies move to the nearest valid wall-clock minute in the requested
direction. Invalid options fail before execution; row errors remain lazy and
include operation/column/row context without echoing raw values.

**Understanding `--ambiguous` option:**

During Daylight Saving Time (DST) transitions in autumn, clocks "fall back" creating duplicate hours. For example, 2:30 AM occurs twice:
- First time: 2:30 AM DST (before transition)  
- Second time: 2:30 AM Standard Time (after transition)

When encountering such ambiguous times:
- `earliest`: Uses the first occurrence (DST time)
- `latest`: Uses the second occurrence (Standard time)

Example:
```bash
$ qlt load data.csv - changetz datetime --from-tz UTC --to-tz Asia/Tokyo
# Output: 2023-01-01T09:00:00.123456+09:00 (ISO8601 with microsecond precision)

$ qlt load data.csv - changetz datetime --from-tz UTC --to-tz America/New_York --input-format "%Y/%m/%d %H:%M" --output-format "%Y-%m-%d %H:%M:%S"
# Custom output format

$ qlt load data.csv - changetz datetime --from-tz America/New_York --to-tz UTC --ambiguous latest
# Handle ambiguous DST times

# Bounded automatic parsing of supported ISO/log forms:
$ qlt load logs.csv - changetz timestamp --from-tz local --to-tz UTC
# Handles: "Jan 15, 2023 2:30 PM", "2023/01/15 14:30", "15-Jan-2023 14:30:00", etc.

# Explicit format takes precedence over automatic parsing:
$ qlt load events.csv - changetz event_time --from-tz America/New_York --to-tz UTC --input-format "%d/%b/%Y:%H:%M:%S"
```

#### `renamecol`
Renames a specific column.

| Parameter   | Type | Default | Description             |
|-------------|------|---------|-------------------------|
| old_name    | str  |         | The current column name. Required. |
| new_name    | str  |         | The new column name. Required.   |

```bash
$ qlt load data.csv - renamecol current_name new_name
```

#### `timeslice`
Filters data based on time ranges, extracting records within specified time boundaries.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| time_column | str |         | Name of the datetime column to filter on. Required. |
| --start | str | | Start time (inclusive). Optional. |
| --end | str | | End time (inclusive). Optional. |

At least one of `--start` or `--end` must be specified. Both boundaries are inclusive (`[start, end]`). It accepts the same parser options as `cast`, including `--timezone` for naive wall-clock values, with microsecond precision. Embedded offsets are authoritative instants and normalize to UTC; naive values without `--timezone` use the UTC-naive timeline.

Example:
```bash
$ qlt load data.csv - timeslice timestamp --start "2023-01-01 00:00:00"
$ qlt load data.csv - timeslice timestamp --end "2023-12-31 23:59:59"
$ qlt load data.csv - timeslice timestamp --start "2023-06-01" --end "2023-06-30"
$ qlt load access.log - timeslice timestamp --start "2023-01-01T10:00:00"
```

Time bucketing and aggregation use `bucket`, `count`, and `calc`.

### Finalizers

Finalizers are used to output or summarize the processed data. They are typically the last command in a chain.

#### `partition`
Splits data into separate CSV files based on unique values in a specified column. Each unique value creates its own file.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| colname | str |         | Column name to partition by. Required. |
| output_directory | str | `./partitions/` | Directory to save partitioned files. Optional - if not specified, creates a `./partitions/` directory. |

The output directory is created if it does not exist and must not already exist.
Unsafe filename characters and path separators become `_`; null
uses `_null`, empty values use `_empty`, reserved names are prefixed with `_`,
and sanitization collisions receive deterministic `-2`, `-3`, ... suffixes.
Files are written through same-filesystem temporary siblings and are never
silently overwritten. Partition input is staged in a schema-preserving Parquet
spool and replayed in bounded batches through a capped open-writer pool; each
partition receives one header.
Partition files are published only after the complete directory has been built,
so a failed partition leaves no partial output.

Example:
```bash
$ qlt load data.csv - partition category                    # Uses default ./partitions/ directory
$ qlt load data.csv - partition category ./partitions/      # Explicit directory
$ qlt load sales.csv - partition region ./by_region/
$ qlt load logs.csv - partition date ./daily_logs/
$ qlt load data.csv - select col1,col2 - partition col1 ./numeric_partitions/
```

#### `calc`
Calculates one aggregation over a numeric column and prints exactly one raw scalar
value followed by a newline. Choose exactly one of `--sum`, `--avg`, `--min`,
`--max`, `--median`, or `--std`; standard deviation uses sample `ddof=1`.
Null-only and empty inputs print `null`. Non-numeric columns and missing columns
fail. Non-finite floating-point results use conventional raw spellings such as
`NaN` and `inf`.

```bash
$ qlt load data.csv - calc EventId --sum
$ qlt load data.csv - calc score --avg
$ qlt load data.csv - calc score --std
```

#### `headers`
Displays the column headers of the current dataset.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| -p, --plain   | flag | `false` | Display headers as plain text, one per line, instead of a formatted table. |

Example:
```bash
$ qlt load data.csv - headers
$ qlt load data.csv - headers -p
$ qlt load data.csv - headers --plain
```

#### `stats`
Displays summary statistics for each column in the dataset (e.g., count, null_count, mean, std, min, max).

`stats` is an explicit aggregate barrier: it evaluates one aggregate result and
may use memory proportional to the input. Numeric columns report count,
null_count, mean, sample standard deviation (`ddof=1`), min, linear-interpolated
25/50/75% quantiles, and max. Strings report min/max; other supported Polars
dtypes show `-` for unsupported measures. Empty and all-null columns report
`null`/`-` rather than panicking.

This command does not take any arguments or options.

Example:
```bash
$ qlt load data.csv - stats
```

#### `showquery`
Displays the logical and optimized Polars LazyFrame query plans without
collecting data. The exact text is Polars-version dependent and is intended
for inspection rather than a stable machine-readable interchange format. Run
stages use the same typed finalizer operation when a stage includes
`showquery`.

This command does not take any arguments or options.

Example:
```bash
$ qlt load data.csv - select col1 - showquery
```

#### `show`
Writes the result as CSV to standard output, including the header. This command
takes no arguments. It is the implicit finalizer when a chain ends without one.

```bash
$ qlt load data.csv - head 5 - show
```

#### `showtable`
Displays the resulting data in a formatted table to standard output. Shows table dimensions and intelligently truncates large datasets.

**Features:**
- Displays table size information (rows × columns) like Python Polars
- For datasets with 9+ rows: shows the first 8 rows and a truncation indicator (`⋮`)
- For datasets with 8 or fewer rows: shows all rows without truncation
- At most 8 rows are evaluated for the preview, plus one row to detect truncation
- Cell contents are limited to 40 characters and truncated with `…`

This command does not take any arguments or options.
> **Tip for large files:** Use `head` before `showtable`, or use `show` instead.

```bash
$ qlt load data.csv - select col1,col2 - head 3 - showtable
# Output includes: shape: (3, 2) followed by formatted table
```

#### `dump`
Outputs the processing results to a CSV/TSV file. The destination must not
already exist; output is first written to a temporary sibling and atomically
published with a no-replace primitive (with a safe hard-link fallback where
supported). The finalizer evaluates the lazy frame as its required write barrier.

| Parameter | Type | Default | Description |
|---|---|---|---|
| -o, --output | str | `dump_<timestamp>.csv` | File path to save the CSV data. Optional - if not specified, a default timestamped filename is automatically generated. |
| -s, --separator | char | `,` | Field separator character for the output CSV file. |

Example:
```bash
$ qlt load data.csv - dump                                  # Saves to dump_<timestamp>.csv
$ qlt load data.csv - head 100 - dump -o results.csv
$ qlt load data.csv - head 100 - dump --output results.csv
$ qlt load data.csv - head 100 - dump -o results.csv -s ';'
```

#### `dumpcache`
Saves the processing results as a Snappy-compressed Parquet cache file for fast
reloading. The destination extension is normalized to `.parquet`, existing
targets are rejected, and the file is atomically published after evaluation;
schema, nested values, and timezone metadata are preserved by the Parquet
writer.

**Features:**
- Saves DataFrame as compressed Parquet format
- Preserves data types (unlike CSV)
- High-performance for large datasets
- Can be loaded back using the `load` command

| Parameter | Type | Default | Description |
|---|---|---|---|
| -o, --output | str | `cache_<timestamp>.parquet` | Output file path (optional). Extension will be changed to .parquet if not specified. |

Example:
```bash
$ qlt load data.csv - head 100 - dumpcache                 # Auto-named cache file
$ qlt load data.csv - select col1,col2 - dumpcache -o cache.parquet
$ qlt load data.csv - sort col1 - dumpcache --output processed_data

# Load from cache for fast access
$ qlt load cache.parquet - show
```

### Operation contract matrix

After `load`, operations receive the current Polars `LazyFrame`. “Plan” operations append a lazy
expression and do not collect the complete frame; schema inspection may call `collect_schema()`
to reject missing columns before evaluation. “Barrier” operations remain lazy in the plan but
need all upstream rows when the finalizer evaluates them. Finalizers are the evaluation/output
boundary. Unless noted, null input values remain null and invalid non-null values return a
contextual error at evaluation time.

| Operation | Accepted input and output schema | Null/error behavior | Evaluation and memory |
|---|---|---|---|
| `load` | CSV/TSV/gzip CSV, same-family JSONL/NDJSON, or Parquet; outputs inferred/preserved columns | Missing files, mixed families, malformed records, and incompatible schemas fail before/at scan | Lazy scan; bounded NDJSON inference (1000 rows by default), `full` scans all records |
| `select` | Existing names, 1-based indices, and ranges; output contains selected columns in requested order | Missing names, invalid/out-of-range/oversized ranges fail with usage/schema error | Lazy projection; schema-only validation |
| `isin` | Any column; output preserves schema and matching rows | Nulls do not match string values; empty values produce an empty frame; missing column errors | Lazy filter; schema-only validation |
| `contains` | String/coercible column plus literal substring; output preserves schema and matching rows | Nulls do not match; missing/non-string columns and invalid options error | Lazy filter; schema-only validation |
| `grep` | Regex over all or named columns; output preserves schema and matching rows | Nulls stringify as non-matches; invalid regex/columns error | Lazy filter; schema-only validation |
| `sed` | Regex replacement over all or one column; output keeps names (all-column mode may stringify values) | Nulls remain null; invalid regex/column errors | Lazy expression; schema-only validation |
| `head` | Any dtype; output keeps schema and at most N first rows | N must be non-negative; nulls unchanged | Lazy limit; bounded output evaluation |
| `tail` | Any dtype; output keeps schema and at most N last rows | N must be non-negative; nulls unchanged | Lazy tail barrier; upstream must be consumed to find final rows |
| `sort` | Existing sortable columns; output preserves schema and row values | Missing columns error; null order follows Polars | Lazy sort barrier; Polars may maintain input-proportional state |
| `count` | Optional grouping columns of any supported dtype; output is group columns plus `count` | Nulls form groups; missing columns error | Lazy grouped barrier; Polars observes the complete input |
| `uniq` | Any schema; output removes duplicate rows | Nulls compare using Polars equality semantics | Lazy distinct barrier; Polars may maintain global state |
| `changetz` | Datetime or parseable string source; output preserves source column after timezone rendering | Nulls remain null; invalid timezone/DST/row values error lazily | Lazy chunk UDF; parsing is per execution chunk |
| `renamecol` | Existing source name to a new unique name; schema changes only the name | Missing source or destination collision errors | Lazy projection/schema operation |
| `timeslice` | Datetime or parseable string source; output keeps schema and rows within inclusive bounds | At least one bound required; nulls are excluded; invalid bounds/rows error lazily | Lazy filter with per-chunk parsing |
| `cast` | `int`→Int64, `uint`→UInt64, `float`→Float64, `string`, `bool`, or datetime; replaces source in place | Nulls remain null; strict invalid values and overflow error at finalization | Lazy expression/UDF; conversion occurs at sink |
| `parse-size` | String size values with SI/IEC units; replaces source as UInt64 | Nulls remain null; negative, malformed, fractional-byte, and overflow values error at finalization | Lazy conversion UDF; sink-time evaluation |
| `bucket` | Datetime/string source plus positive interval; adds `<column>_bucket` or `--output` as Datetime[μs] | Nulls remain null; invalid interval, parse, collision, and overflow errors | Lazy expression/UDF; checked floor at sink |
| `delta` | Numeric or datetime source; adds `<column>_delta` (Int64/Float or Duration[μs]) | First row, null pairs, and invalid overflow cases are null/error per source contract | Lazy window expression; prior-row state at evaluation |
| `extract` | String source plus named Rust regex groups; adds nullable String columns | Unmatched/absent optional groups are null; invalid regex, no names, non-string, and collisions error | Lazy extraction UDF; sink-time evaluation |
| `flatten` | Nested JSON Struct fields; recursively adds dot-named scalar fields, preserving lists | Missing/null nested fields remain null; collisions and mixed/non-object records error | Lazy struct projection; lists are not exploded |
| `calc` | Numeric source and exactly one aggregation; outputs one raw scalar line | Null-only/empty is `null`; non-numeric/missing source errors | Finalizer aggregate barrier |
| `partition` | Any source dtype; writes one CSV per distinct value with sanitized names | Null is `_null`, empty is `_empty`; unsafe names/collisions are sanitized; existing destination rejected | Global finalizer; schema-preserving Parquet spool, bounded batches, capped open writers, atomic staged directory |
| `headers` | Any schema; outputs names (plain one-per-line or formatted) | Empty schema is valid; no row evaluation required | Finalizer/schema inspection only |
| `stats` | Supported Polars dtypes; outputs one summary table per column | Null counts are reported; unsupported measures use `-`; empty/all-null inputs do not panic | Finalizer aggregate barrier; may use input-sized memory |
| `showquery` | Any LazyFrame; outputs logical/optimized plan text | No row conversion; plan errors are returned | Inspection finalizer; does not collect rows |
| `show` | Any schema; outputs machine-readable CSV including header | Conversion/write errors are returned; stdout contains data only | Lazy CSV sink to an execution-owned temporary artifact, then bounded 64KiB copying |
| `showtable` | Any schema; bounded formatted preview with shape | Truncates rows/cells; rendering errors are returned | Intrinsic bounded-preview finalizer |
| `dump` | Any schema; writes CSV/TSV with header | Existing target rejected; staged atomic write cleans failures | Streaming-capable sink/finalizer; target is never overwritten |
| `dumpcache` | Any schema including nested/timezone values; writes Snappy Parquet | Existing target rejected; extension normalized to `.parquet`; failures clean staging | Typed Parquet finalizer; evaluates and writes cache |

### Automation

#### `run`

Quilt allows you to define complex data processing workflows in YAML configuration files. This is useful for automating repetitive tasks or creating reusable data processing pipelines.

The `run` command takes the path to a YAML configuration file. Input data
sources and other parameters are typically defined within the YAML file. If a
`load` step omits `paths`, positional files after the config path are supplied
to that step.

```bash
$ qlt run <config_file_path.yaml> [files...] [options]
```
| Parameter | Type | Description |
|---|---|---|
| config_file_path.yaml | str | Path to the YAML configuration file defining the pipeline stages. Required. |
| files... | list | Optional input files used by `load` steps that omit `paths`. |
| --check | flag | Parse and statically validate the run document without reading input data or writing output. |
| --show-plan STAGE | str | Build the selected process, join, or concat stage and print its logical/optimized plan without evaluating rows or running finalizers. Dynamic branch stages are rejected. |
| --var name=value | repeated | Supply a value for a declared typed parameter. |
| -o, --output | str | CSV destination. If the YAML has a `dump`, this path replaces it. If not, this is the dump path. Relative paths are from the run file directory. Existing files are rejected. |


```bash
$ qlt run rules/my_workflow.yaml
$ qlt run rules/my_workflow.yaml events.csv
$ qlt run rules/my_analysis.yaml -o result.csv
```

No `-o` uses the YAML `dump` path. With `-o`, the CLI path wins.

```yaml
version: 1
stages:
  - name: output
    steps:
      - load: {paths: [events.csv]}
      - dump: {output: default.csv}
```

```bash
$ qlt run workflow.yaml              # writes default.csv
$ qlt run workflow.yaml -o result.csv  # writes result.csv only
```

#### Pipeline Operations in YAML
Within a `run` document, stages can be different types to orchestrate the flow.

| Operation Type | Description                                                | Key Parameters                                                                                                                                    |
| -------------- | ---------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `process`      | Executes a series of qlt operations on a dataset.          | `name` and a sequence-form `steps` (for example, `- grep: {pattern: ERROR}`). `from` (optional) names a prior stage. |
| `concat`       | Concatenates multiple datasets (stages).                   | `name` and `concat: {inputs: [stage_a, stage_b], how: vertical}`. Horizontal concatenation is not yet implemented. |
| `join`         | Joins datasets from multiple stages based on keys.         | `name` and `join: {inputs: [...], on: [column], how: inner}`. Use `left-on` and `right-on` for asymmetric keys; `cross` needs no key. Optional `coalesce`. |
| `branch`       | Selects downstream target stages from a `row-count` or `parameter` predicate. | `name` and `branch: {input: stage, when: {row-count: {greater-than: 10}}, then: [target]}` with optional `else: [target]`. |

Every run document must use `version: 1` and a sequence of named stages. The schema is
strict: unknown document, stage, and step keys are rejected before any input is read.

Parameters are declared at the document root with one of `path`, `string`, `int`, or
`bool` types. Values are referenced as whole YAML values using `{"$param": name}`;
partial string interpolation is not supported. CLI `--var` values take precedence
over defaults. A parameter `path` default is relative to the run file; a `--var`
path is relative to the caller's working directory. A parameter cannot be both `required` and
have a `default`; secret values are redacted from diagnostics. Branch predicates support
typed `row-count` and `parameter` comparisons; scalar-result predicates are deferred.

```yaml
parameters:
  minimum_id:
    type: int
    default: 1000
```

```yaml
version: 1
title: Event projection
stages:
  - name: input
    steps:
      - load:
          paths: [events.csv]
      - select:
          columns: [TimeCreated, EventId, Level]
  - name: output
    from: input
    steps:
      - head:
          number: 10
      - show: {}
```

Join two stages and write the result:

```yaml
version: 1
stages:
  - name: events
    steps: [{load: {paths: [events.csv]}}]
  - name: labels
    steps: [{load: {paths: [labels.csv]}}]
  - name: merged
    join: {inputs: [events, labels], how: left, on: [EventId]}
  - name: output
    from: merged
    steps: [{dump: {output: merged.csv}}]
```

Concatenate compatible stages and route a branch:

```yaml
version: 1
stages:
  - name: first
    steps: [{load: {paths: [part-a.csv]}}]
  - name: second
    steps: [{load: {paths: [part-b.csv]}}]
  - name: all
    concat: {inputs: [first, second], how: vertical}
  - name: route
    branch:
      input: all
      when: {row-count: {greater-than: 0}}
      then: [present]
      else: [empty]
  - name: present
    from: all
    steps: [{head: {number: 10}}, {show: {}}]
  - name: empty
    from: all
    steps: [{headers: {plain: true}}]
```

Multiple outputs can be reached from one branch (or declared as independent stages):

```yaml
version: 1
stages:
  - name: input
    steps: [{load: {paths: [events.csv]}}]
  - name: route
    branch:
      input: input
      when: {row-count: {greater-than: 0}}
      then: [csv-output, cache-output]
  - name: csv-output
    from: input
    steps: [{dump: {output: events.csv}}]
  - name: cache-output
    from: input
    steps: [{dumpcache: {output: events-cache.parquet}}]
```

Typed parameters and static validation:

```yaml
version: 1
parameters:
  source:
    type: path
    required: true
  limit:
    type: int
    default: 10
stages:
  - name: output
    steps:
      - load: {paths: [{"$param": source}]}
      - head: {number: {"$param": limit}}
      - dump: {output: result.csv}
```

```bash
qlt run workflow.yaml --check --var source=events.csv
qlt run workflow.yaml --var source=events.csv --var limit=100
```

One document may contain multiple independent output stages; each finalizer result is emitted
in stage order. `--check` validates the complete schema, typed parameters, command arguments,
stage references, and cycles without opening declared input files or creating output files.

For time bucketing in a run document, use the `bucket` step followed by `count` or a `calc` output step.

For scalar aggregation in a `run` output stage:

```yaml
- name: output
  from: process_stage
  steps:
    - calc:
        column: EventId
        avg: true
```

## Huge File Processing

Quilt supports streaming-capable execution for many large-file processing pipelines.

### Memory Behavior by Command

Not all commands stream. Before running a pipeline on a large file, check the memory behavior of each operation:

| Mode | Commands | Notes |
|------|----------|-------|
| **Streaming-capable** | `show`, `dump`, `dumpcache` | Prefer these sinks for large inputs; `show` uses bounded memory while its temporary CSV artifact consumes disk proportional to rendered output |
| **Bounded / lazy** | `head`, `headers`, `showtable`, `showquery` | `head` limits output; `headers` inspects schema only; `showtable` previews a bounded number of rows; plans do not collect |
| **Lazy / Polars-optimized** | `select`, `isin`, `contains`, `grep`, `sed`, `renamecol`, `cast`, `parse-size`, `bucket`, `delta`, `extract`, `flatten`, `changetz`, `timeslice` | Pushdown or per-chunk UDFs; usually safe |
| **Global barriers** ⚠️ | `sort`, `uniq`, `count`, `tail` | Polars may maintain input-proportional state to produce a global result |
| **Aggregate finalizers** ⚠️ | `stats`, `calc` | Materialize input-proportional state |
| **Global / disk-backed finalizers** ⚠️ | `partition` | Scans the complete input through a schema-preserving Parquet spool and bounded batches; output directory is fully staged before publication |

> **Warning:** Running a global barrier or aggregate finalizer on a multi-GB file may require substantial input-proportional state. `partition` bounds in-memory writer state but consumes disk proportional to the input and output. Use `head`, `timeslice`, or `isin` to reduce the dataset first.

The lazy chainables remain logical-plan nodes until a finalizer or sink runs.
Their map/UDF closures collect values only from the current Polars execution
chunk to build a replacement column; they do not call `collect()` on the
LazyFrame or materialize the complete input. Chunk-local UDFs can still limit
some streaming optimizations, so use `dump`/`dumpcache` sinks and avoid global
barriers when bounded memory is required.

### Gzip File Processing

```bash
# Gzip input is decompressed to an execution-owned temporary CSV spool, then
# scanned lazily. The spool uses bounded decompression buffers and disk space
# proportional to the uncompressed input until the pipeline is dropped.
$ qlt load huge.csv.gz - head 1000 - show
```

### Parquet Cache for Performance

For repeated processing of large CSV files, converting once to Parquet avoids repeated CSV parsing and preserves inferred data types.

**Performance Benefits:**
- Avoids repeated CSV parsing
- Typically offers compact columnar storage
- Preserves data types

```bash
# One-time conversion: CSV to Parquet cache
$ qlt load huge.csv - dumpcache -o huge.parquet

# Subsequent processing: Load from Parquet without reparsing CSV
$ qlt load huge.parquet - select col1,col2 - show
$ qlt load huge.parquet - isin category "important" - dump -o result.csv
```

## Installation

### Pre-built Binaries
Download the latest release from [GitHub Releases](https://github.com/sumeshi/quilt/releases).

### Build from Source
```bash
$ git clone https://github.com/sumeshi/quilt.git
$ cd quilt
$ cargo build --release
```

## Contributing
Contributions are welcome! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License
This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

Inspired by [xsv](https://github.com/BurntSushi/xsv).
