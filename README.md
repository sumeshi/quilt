# Quilter-CSV
[![MIT License](http://img.shields.io/badge/license-MIT-blue.svg?style=flat)](LICENSE)
[![CI/CD Pipeline](https://github.com/sumeshi/qsv-rs/actions/workflows/release.yml/badge.svg?branch=main)](https://github.com/sumeshi/qsv-rs/actions/workflows/release.yml)

![qsv-rs](https://gist.githubusercontent.com/sumeshi/c2f430d352ae763273faadf9616a29e5/raw/8484142e88948ecc0c8887db8f3bbb5be0dbe51e/qsv-rs.svg)

A fast, flexible, and memory-efficient command-line tool written in Rust for processing large CSV and JSON Lines files. Inspired by [xsv](https://github.com/BurntSushi/xsv) and built on [Polars](https://www.pola.rs/), it's designed for handling tens or hundreds of gigabytes of data efficiently in workflows like log analysis and digital forensics.

> [!NOTE]
> The original version of this project was implemented in Python and can be found at [sumeshi/quilter-csv](https://github.com/sumeshi/quilter-csv). This Rust version is a complete rewrite.

## Features

- **Pipeline-style command chaining**: Chain multiple commands in a single line for fast and efficient data processing
- **Flexible filtering and transformation**: Perform operations like select, filter, sort, deduplicate, and timezone conversion
- **YAML-based batch processing (Quilt)**: Automate complex workflows using YAML configuration files

## Usage
![](https://gist.githubusercontent.com/sumeshi/644af27c8960a9b6be6c7470fe4dca59/raw/2a19fafd4f4075723c731e4a8c8d21c174cf0ffb/qsv.svg)

### Getting Help

To see available commands and options, run `qsv` without any arguments:

```bash
$ qsv -h
```

### Example

Here's an example of reading a CSV file, extracting rows that contain 4624 in the 'Event ID' column, and displaying the top 3 rows sorted by the 'Date and Time' column:

```bash
$ qsv load Security.csv - isin 'Event ID' 4624 - sort 'Date and Time' - head 3 - showtable
```

This command:
1. Loads `Security.csv`
2. Filters rows where `Event ID` is 4624
3. Sorts by `Date and Time`
4. Shows the first 3 rows as a table

### Command Structure

qsv commands are composed of three types of steps:

- **Initializer**: Loads data (e.g., `load`)
- **Chainable**: Transforms or filters data (e.g., `select`, `grep`, `sort`, etc.)
- **Finalizer**: Outputs or summarizes data (e.g., `show`, `showtable`, `headers`, etc.)

Each step is separated by a hyphen (`-`):

```bash
$ qsv <INITIALIZER> <args> - <CHAINABLE> <args> - <FINALIZER> <args>
```

### Command Separator `-`

The `-` token (a single hyphen surrounded by spaces) is the command separator. A standalone `-` is never treated as data.

- To separate commands: `qsv load file.csv - select col1 - head 5`
- If you need `-` as an option value, use an attached form such as `--separator=-` or `-s-`.

A standalone `-` positional value, including stdin-style usage, is not currently supported.

**Note:** If no finalizer is explicitly specified, default builds automatically use `showtable`, making it easy to quickly view results:

```bash
$ qsv load data.csv - select col1,col2 - head 5
# Equivalent to:
$ qsv load data.csv - select col1,col2 - head 5 - showtable
```

Builds compiled without the optional `table` feature fall back to `show` instead, and the `showtable` command prints a rebuild hint.

## Command Reference

### Migration from removed commands

The command surface now favors small composable operations:

- `timeround` → `bucket`
- Timeline bucketing → `bucket`
- Timeline aggregation → `bucket` + `count` or `calc`
- `pivot` → external analytical tools
- `convert` → `jq`, `yq`, or dedicated format tools

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

**Environment Variables:**
- `QSV_CHUNK_SIZE`: Default chunk size for CSV processing (overrides auto-detection, can be overridden by --chunk-size)
- `QSV_MEMORY_LIMIT_MB`: Memory limit for gzip decompression and streaming operations (default: 1024MB, range: 512-4096MB)

Example:
```bash
$ qsv load data.csv
$ qsv load data.csv.gz
$ qsv load data1.csv data2.csv data3.csv
$ qsv load events.jsonl - flatten - select user.name,process.command_line - show
$ qsv load "logs/*.tsv" -s $'\t'
$ qsv load "logs/*.tsv" --separator=$'\t'
$ qsv load data.csv --low-memory
$ qsv load data.csv --no-headers
$ qsv load data.csv --chunk-size 50000
$ qsv load cache.parquet                              # Load from parquet cache
$ qsv load cache1.parquet cache2.parquet              # Load multiple parquet files
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
$ qsv load data.csv - select datetime                       # Select single column by name
$ qsv load data.csv - select col1,col3                      # Select specific columns by name
$ qsv load data.csv - select col1-col3                      # Select range using hyphen
$ qsv load data.csv - select col1:col3                      # Select range using colon
$ qsv load data.csv - select 1                              # Select 1st column (datetime)
$ qsv load data.csv - select 2:4                            # Select 2nd-4th columns (col1, col2, col3)
$ qsv load data.csv - select 2,4                            # Select 2nd and 4th columns (col1, col3)
$ qsv load data.csv - select "col:1":"col:3"                # For columns with colons in names
$ qsv load data.csv - select 1,datetime,3:5                 # Mixed selection methods
```

#### `cast`
Strictly converts one column in place while preserving its position and name.

| Parameter | Type | Description |
|-----------|------|-------------|
| column | str | Source column name. Required. |
| type | str | `int` (`Int64`), `uint` (`UInt64`), `float` (`Float64`), `string`, `bool`, or `datetime`. Required. |

Nulls remain null and invalid non-null values fail the command. Datetimes use the
automatic parser shared by datetime operations and are stored as Polars
`Datetime[μs]` values.

```bash
$ qsv load data.csv - cast EventId int - show
$ qsv load data.csv - cast timestamp datetime - show
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
$ qsv load data.csv - parse-size size - show
```

#### `flatten`
Recursively expands nested JSON object (Struct) fields into dot-notated columns.
Scalar fields such as `user.name` and `process.command_line` can then be selected
normally. Lists and arrays, including lists of objects, remain list-valued and are
not exploded. Missing or null nested fields remain null. Name collisions between
existing columns and generated paths fail before the frame is changed.

```bash
$ qsv load events.jsonl - flatten - select user.name,process.command_line - show
```

#### bucket
Floors a datetime column to a fixed positive interval and adds a new column.

| Parameter | Type | Description |
|-----------|------|-------------|
| column | str | Datetime source column. Required. |
| interval | str | Positive interval matching ^[1-9][0-9]*(s|m|h|d)$. |
| --output | str | Output name; defaults to <column>_bucket. |

The source must be a typed datetime column. Use cast column datetime after CSV
loading. Output values use Polars Datetime[μs] precision, source and nulls are
preserved, and existing output names are rejected.

Examples:
$ qsv load data.csv - cast timestamp datetime - bucket timestamp 5m - show
$ qsv load data.csv - cast timestamp datetime - bucket timestamp 1h --output hour - show

#### delta
Calculates the current value minus the previous row without reordering or
replacing the source column.

| Parameter | Type | Description |
|-----------|------|-------------|
| column | str | Numeric or datetime source column. Required. |
| --output | str | Output name; defaults to <column>_delta. |

Numeric deltas use Float64 to represent signed and unsigned differences
without integer wraparound. Datetime deltas use Duration[μs]. The first row
and any pair involving a null produce null. Existing output names are rejected.

Examples:
$ qsv load data.csv - delta count - show
$ qsv load data.csv - cast timestamp datetime - delta timestamp --output elapsed - show

#### extract
Extracts named Rust regex capture groups into new nullable String columns while
preserving the source column.

| Parameter | Type | Description |
|-----------|------|-------------|
| column | str | String source column. Required. |
| regex | str | Rust regex with one or more named groups. Required. |

Unmatched rows and absent optional groups become null. Existing output names,
invalid regexes, and regexes without named groups are rejected. Quote the regex
for your shell; this command does not add an expression language.

Examples:
$ qsv load data.csv - extract message '(?P<user>[^@]+)@(?P<domain>.+)' - show
$ qsv load data.csv - extract path '^(?P<dir>.*)/(?P<file>[^/]+)$' - show
```

#### `isin`
Filter rows where a column matches any of the given values.

| Parameter | Type   | Default | Description                                                                          |
|-----------|--------|---------|--------------------------------------------------------------------------------------|
| colname   | str    |         | Column name to filter. Required.                                                     |
| values    | list   |         | Comma-separated values. Filters rows where the column matches any of these values (OR condition). Required. |

```bash
$ qsv load data.csv - isin col1 1
$ qsv load data.csv - isin col1 1,4
```

#### `contains`
Filter rows where a column contains a specific literal substring.

| Parameter   | Type   | Default | Description                                 |
|-------------|--------|---------|---------------------------------------------|
| colname     | str    |         | Column name to search. Required.            |
| substring   | str    |         | The literal substring to search for. Required. |
| -i, --ignorecase | flag | `false` | Perform case-insensitive matching.          |

```bash
$ qsv load data.csv - contains str ba
$ qsv load data.csv - contains str BA -i
$ qsv load data.csv - contains str BA --ignorecase
```

#### `sed`
Replace values in column(s) using a Regex pattern.

| Parameter   | Type   | Default | Description                                 |
|-------------|--------|---------|---------------------------------------------|
| pattern     | str    |         | Regex pattern to search for. Required.      |
| replacement | str    |         | Replacement string. Required.               |
| --column    | str    | (all)   | Apply replacement to specific column only. If not specified, applies to all columns. |
| -i, --ignorecase | flag | `false` | Perform case-insensitive matching.          |

> **Warning:** When `--column` is omitted, `sed` replaces across **all columns**. In log/DFIR data this can silently modify timestamps, EventIDs, file paths, and usernames. Always specify `--column` unless you intend a full-dataset replacement.

```bash
$ qsv load data.csv - sed foo foooooo                       # Replace 'foo' with 'foooooo' in all columns
$ qsv load data.csv - sed foo foooooo --column str          # Replace 'foo' with 'foooooo' in 'str' column only
$ qsv load data.csv - sed FOO foooooo -i                    # Case-insensitive replacement in all columns
$ qsv load data.csv - sed ".*o.*" foooooo --column str      # Regex replacement in specific column
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
$ qsv load data.csv - grep foo 
$ qsv load data.csv - grep "^FOO" -i                        # Case-insensitive search
$ qsv load data.csv - grep "^FOO" --ignore-case              # Long form case-insensitive
$ qsv load data.csv - grep "^FOO" -i -v                     # Case-insensitive inverted match
$ qsv load data.csv - grep "^FOO" --ignore-case --invert-match  # Long form inverted match
$ qsv load logs.csv - grep "FAILED" --column EventData
$ qsv load logs.csv - grep "192\\.168\\." --column src_ip,dst_ip
```

#### `head`
Displays the first N rows of the dataset.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| number | int  | 5       | Number of rows to display. Can be specified as positional argument or with -n/--number option. |
| -n, --number | int | | Alternative way to specify number of rows. |

```bash
$ qsv load data.csv - head 3
$ qsv load data.csv - head 10
$ qsv load data.csv - head -n 3
$ qsv load data.csv - head --number 10
```

#### `tail`
Displays the last N rows of the dataset.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| number | int  | 5       | Number of rows to display. Can be specified as positional argument or with -n/--number option. |
| -n, --number | int | | Alternative way to specify number of rows. |

```bash
$ qsv load data.csv - tail 3
$ qsv load data.csv - tail 10
$ qsv load data.csv - tail -n 3
$ qsv load data.csv - tail --number 10
```

#### `sort`
Sorts the dataset based on the specified column(s).

> ⚠️ **Memory:** This command materializes the full dataset into memory.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| colnames  | str/list |         | Column name(s) to sort by. Comma-separated for multiple columns (e.g., `col1,col3`) or a single column name. Required. |
| -d, --desc    | flag | `false` | Sort in descending order. Applies to all specified columns. |

```bash
$ qsv load data.csv - sort str
$ qsv load data.csv - sort str -d
$ qsv load data.csv - sort str --desc
$ qsv load data.csv - sort col1,col2,col3 --desc
```

#### `count`
Count duplicate rows, grouping by all columns by default. Results are automatically sorted by count in descending order.

> ⚠️ **Memory:** This command materializes the full dataset into memory.

| Parameter | Type | Default | Description |
|---|---|---|---|
| columns   | str | (all columns) | Optional positional column list. Use `col1` or `col1,col2` to group by specific columns only. |

```bash
$ qsv load Security.csv - count EventID          # Count by one column
$ qsv load proxy.csv - count src_ip,dst_ip       # Count by multiple columns
$ qsv load data.csv - count                       # Count all unique rows (original behavior)
$ qsv load data.csv - count - sort col1          # Count and then sort by col1 instead
```

#### `uniq`
Filters unique rows, removing duplicates based on all columns.

> ⚠️ **Memory:** This command materializes the full dataset into memory.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| (None)    |      |         | Takes no arguments. Removes duplicate rows based on all columns. |

```bash
$ qsv load data.csv - uniq
```

#### `changetz`
Changes the timezone of a datetime column.

| Parameter | Type | Default | Description |
|---|---|---|---|
| colname | str |         | Name of the datetime column. Required. |
| --from-tz | str |         | Source timezone (e.g., `UTC`, `America/New_York`, `local`). Required. |
| --to-tz | str |         | Target timezone (e.g., `Asia/Tokyo`). Required. |
| --input-format | str | `auto` | Input datetime format string (e.g., `%Y-%m-%d %H:%M:%S%.f`). `auto` uses intelligent parsing similar to Python's dateutil.parser, supporting fuzzy parsing and automatic format detection. |
| --output-format | str | `auto` | Output datetime format string (e.g., `%Y/%m/%d %H:%M:%S`). `auto` uses ISO8601 format `%Y-%m-%dT%H:%M:%S%.6f%:z` (microsecond precision). |
| --ambiguous | str | `earliest` | Strategy for ambiguous times during DST transitions: `earliest` (first occurrence) or `latest` (second occurrence). |

**Understanding `--ambiguous` option:**

During Daylight Saving Time (DST) transitions in autumn, clocks "fall back" creating duplicate hours. For example, 2:30 AM occurs twice:
- First time: 2:30 AM DST (before transition)  
- Second time: 2:30 AM Standard Time (after transition)

When encountering such ambiguous times:
- `earliest`: Uses the first occurrence (DST time)
- `latest`: Uses the second occurrence (Standard time)

Example:
```bash
$ qsv load data.csv - changetz datetime --from-tz UTC --to-tz Asia/Tokyo
# Output: 2023-01-01T09:00:00.123456+09:00 (ISO8601 with microsecond precision)

$ qsv load data.csv - changetz datetime --from-tz UTC --to-tz America/New_York --input-format "%Y/%m/%d %H:%M" --output-format "%Y-%m-%d %H:%M:%S"
# Custom output format

$ qsv load data.csv - changetz datetime --from-tz America/New_York --to-tz UTC --ambiguous latest
# Handle ambiguous DST times

# Automatic format detection (similar to Python dateutil.parser):
$ qsv load logs.csv - changetz timestamp --from-tz local --to-tz UTC
# Handles: "Jan 15, 2023 2:30 PM", "2023/01/15 14:30", "15-Jan-2023 14:30:00", etc.

# Fuzzy parsing with embedded text:
$ qsv load events.csv - changetz event_time --from-tz EST --to-tz UTC  
# Handles: "Meeting on January 15th, 2023 at 2:30 PM", "Call scheduled for Jan 15 2023"
```

**TODO:** Upgrade to 7-digit sub-second precision (100-nanosecond precision for Windows FILETIME compatibility) when chrono-tz library supports it. Current `auto` output uses microsecond precision.

#### `renamecol`
Renames a specific column.

| Parameter   | Type | Default | Description             |
|-------------|------|---------|-------------------------|
| old_name    | str  |         | The current column name. Required. |
| new_name    | str  |         | The new column name. Required.   |

```bash
$ qsv load data.csv - renamecol current_name new_name
```

#### `timeslice`
Filters data based on time ranges, extracting records within specified time boundaries.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| time_column | str |         | Name of the datetime column to filter on. Required. |
| --start | str | | Start time (inclusive). Optional. |
| --end | str | | End time (inclusive). Optional. |

At least one of `--start` or `--end` must be specified. Both boundaries are inclusive (`[start, end]`). Supports various datetime formats including ISO8601, timestamps, and common log formats.

Example:
```bash
$ qsv load data.csv - timeslice timestamp --start "2023-01-01 00:00:00"
$ qsv load data.csv - timeslice timestamp --end "2023-12-31 23:59:59"
$ qsv load data.csv - timeslice timestamp --start "2023-06-01" --end "2023-06-30"
$ qsv load access.log - timeslice timestamp --start "2023-01-01T10:00:00"
```

Time bucketing and aggregation use the `bucket`, `count`, and `calc` commands. See the command reference in `GOAL.md`.

### Finalizers

Finalizers are used to output or summarize the processed data. They are typically the last command in a chain.

#### `partition`
Splits data into separate CSV files based on unique values in a specified column. Each unique value creates its own file.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| colname | str |         | Column name to partition by. Required. |
| output_directory | str | `./partitions/` | Directory to save partitioned files. Optional - if not specified, creates a `./partitions/` directory. |

The output directory will be created if it doesn't exist. Each file is named after the unique value in the partition column (with invalid filename characters replaced by underscores).

Example:
```bash
$ qsv load data.csv - partition category                    # Uses default ./partitions/ directory
$ qsv load data.csv - partition category ./partitions/      # Explicit directory
$ qsv load sales.csv - partition region ./by_region/
$ qsv load logs.csv - partition date ./daily_logs/
$ qsv load data.csv - select col1,col2 - partition col1 ./numeric_partitions/
```

#### `calc`
Calculates one aggregation over a numeric column and prints exactly one raw scalar
value followed by a newline. Choose exactly one of `--sum`, `--avg`, `--min`,
`--max`, `--median`, or `--std`; standard deviation uses sample `ddof=1`.
Null-only and empty inputs print `null`. Non-numeric columns and missing columns
fail. Non-finite floating-point results use conventional raw spellings such as
`NaN` and `inf`.

```bash
$ qsv load data.csv - calc EventId --sum
$ qsv load data.csv - calc score --avg
$ qsv load data.csv - calc score --std
```

#### `headers`
Displays the column headers of the current dataset.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| -p, --plain   | flag | `false` | Display headers as plain text, one per line, instead of a formatted table. |

Example:
```bash
$ qsv load data.csv - headers
$ qsv load data.csv - headers -p
$ qsv load data.csv - headers --plain
```

#### `stats`
Displays summary statistics for each column in the dataset (e.g., count, null_count, mean, std, min, max).

> [!WARNING]
> This command loads the entire dataset into memory to compute statistics. It may fail or cause performance issues with very large files (e.g., 10GB+). For large datasets, consider using `head` or other filters to reduce the data size before running `stats`.

This command does not take any arguments or options.

Example:
```bash
$ qsv load data.csv - stats
```

#### `showquery`
Displays the Polars LazyFrame query plan. This is useful for debugging and understanding the operations being performed.

This command does not take any arguments or options.

Example:
```bash
$ qsv load data.csv - select col1 - showquery
```

#### `show`
Displays the resulting data as CSV to standard output. Header is included by default.

| Parameter | Type | Default | Description |
|---|---|---|---|
| --batch-size | str | `1GB` | Memory batch size for streaming large datasets (e.g., `512MB`, `2GB`). Range: 1MB-10GB. |

Example:
```bash
$ qsv load data.csv - head 5 - show
$ qsv load huge.csv - show --batch-size 2GB                 # Streaming mode for large files
$ qsv load data.csv - select col1,col2 - show --batch-size 512MB
```

#### `showtable`
Displays the resulting data in a formatted table to standard output. Shows table dimensions and intelligently truncates large datasets.

**Features:**
- Displays table size information (rows × columns) like Python Polars
- For datasets with 9+ rows: shows the first 8 rows and a truncation indicator (`⋮`)
- For datasets with 8 or fewer rows: shows all rows without truncation
- Automatically used as default finalizer when no explicit finalizer is specified

This command does not take any arguments or options.
This command is controlled by the optional cargo feature `table`, which is enabled in the default build.

> **Tip for large files:** Pipe through `head N` before `showtable`, or use `show` instead. `showtable` with implicit finalization collects all rows by default.

Example:
```bash
$ qsv load data.csv - select col1,col2 - head 3 - showtable
# Output includes: shape: (3, 2) followed by formatted table

$ qsv load large_data.csv - select col1,col2
# Automatically calls showtable if no finalizer specified
```

To build a smaller binary without table rendering support:

```bash
$ cargo build --release --no-default-features
```

In that build, `showtable` exits with a clear rebuild message and implicit finalization falls back to `show`.

#### `dump`
Outputs the processing results to a CSV file.

| Parameter | Type | Default | Description |
|---|---|---|---|
| -o, --output | str | `dump_<timestamp>.csv` | File path to save the CSV data. Optional - if not specified, a default timestamped filename is automatically generated. |
| -s, --separator | char | `,` | Field separator character for the output CSV file. |
| --batch-size | str | `1GB` | Memory batch size for streaming large datasets (e.g., `512MB`, `2GB`). Range: 1MB-10GB. |

Example:
```bash
$ qsv load data.csv - dump                                  # Saves to dump_<timestamp>.csv
$ qsv load data.csv - head 100 - dump -o results.csv
$ qsv load data.csv - head 100 - dump --output results.csv
$ qsv load data.csv - head 100 - dump -o results.csv -s ';'
$ qsv load huge.csv - dump -o output.csv --batch-size 2GB   # Streaming mode for large files
```

#### `dumpcache`
Saves the processing results as a Parquet cache file for fast reloading.

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
$ qsv load data.csv - head 100 - dumpcache                 # Auto-named cache file
$ qsv load data.csv - select col1,col2 - dumpcache -o cache.parquet
$ qsv load data.csv - sort col1 - dumpcache --output processed_data

# Load from cache for fast access
$ qsv load cache.parquet - show
```

### Quilt (YAML Workflows)

Quilt allows you to define complex data processing workflows in YAML configuration files. This is useful for automating repetitive tasks or creating reusable data processing pipelines.

#### Usage
The `quilt` command itself takes the path to a YAML configuration file. Input data sources and other parameters are typically defined within the YAML file.

```bash
$ qsv quilt <config_file_path.yaml> [options]
```
| Parameter | Type | Description |
|---|---|---|
| config_file_path.yaml | str | Path to the YAML configuration file defining the pipeline stages. Required. |
| -o, --output | str | Overrides the output path defined in the YAML config for the final dump operation (if any). |


#### Example: Running a Quilt File
```bash
$ qsv quilt rules/my_workflow.yaml
$ qsv quilt rules/my_analysis.yaml -o custom_output.csv
```

The YAML configuration file (e.g., `rules/my_workflow.yaml`) defines the stages and steps. For example, the `Sample YAML (rules/test.yaml)` below defines a pipeline that:
1. Loads data (implicitly or explicitly via a `load` step in a `process` stage).
2. Performs selections and a join operation across different stages.
3. Displays the final result as a table.

#### Pipeline Operations in YAML
Within a Quilt YAML file, stages can be of different types to orchestrate the flow.

| Operation Type | Description                                                | Key Parameters                                                                                                                                    |
| -------------- | ---------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `process`      | Executes a series of qsv operations on a dataset.          | `steps`: Dictionary of operations (e.g., `load`, `select`, `head`, `showtable`). Each key is a qsv command, and its value contains arguments/options. <br> `source` (optional): Specifies the output of a previous stage as input. |
| `concat`       | Concatenates multiple datasets (stages).                   | `sources`: List of stage names whose outputs to concatenate. <br>`params.how` (optional): Method for concatenation, `vertical` (default). Note: `horizontal` concatenation is not yet implemented. |
| `join`         | Joins datasets from multiple stages based on keys.         | `sources`: List of two stage names whose outputs to join. <br>`params.left_on`/`params.right_on` or `params.on`: Column(s) for joining. <br>`params.how` (optional): Join type (`inner`, `left`, `outer`, `cross`). |
| `where` step   | Filters rows using a SQL `WHERE` clause embedded in a process step. | `sql`: Full SQL statement such as `SELECT * FROM logs WHERE ...`. <br>`field_map` (optional): Inline mapping of SQL field names to CSV column names. <br>`annotate` (optional): Add `sigma_title`, `sigma_id`, `sigma_level`, `sigma_tags` columns. |

#### `sigma2quilt`

`sigma2quilt` converts Zircolite JSON rules into a regular quilt YAML file. The generated quilt uses normal `process`, `concat`, and `output` stages, and each rule becomes a `where` step over `${input}`.

- A single-rule JSON file defaults to `quilt-<rule-title>.yaml`
- Rule titles are converted to lowercase hyphen-joined filenames
- `--separate` writes one quilt file per rule
- Each conversion also writes one `_mapping.json` template for the whole input ruleset
- `--annotate` is mainly useful when multiple rules are kept in one generated quilt and you want the matched rows to retain per-rule metadata
- For `rules_dir/` input, `-o <dir>` is required

Supported SQL in the generated `where` step:
1. `=`
2. `LIKE ... ESCAPE '\'`
3. `NOT (...)`
4. `AND`
5. `OR`

Field resolution is:
1. Explicit mapping via `qsv quilt --mapping <file>` or `params.field_map`
2. Exact CSV column match
3. Case-insensitive CSV column match
4. Otherwise warn and skip that condition

```yaml
title: Sigma JSON Conversion: rules_windows_generic

stages:
  load_stage:
    type: process
    steps:
      load:
        path: ${input}

  detect_1_suspicious_high_integritylevel_conhost_legacy_option:
    type: process
    source: load_stage
    steps:
      where:
        sql: "SELECT * FROM logs WHERE Channel='Security' AND EventID=4688"
        annotate: true
        sigma_title: "Suspicious High IntegrityLevel Conhost Legacy Option"
        sigma_id: "3037d961-21e9-4732-b27a-637bcc7bf539"
        sigma_level: "informational"
        sigma_tags: "attack.defense-evasion,attack.t1202"

  output_stage:
    type: output
    source: detect_1_suspicious_high_integritylevel_conhost_legacy_option
    steps:
      dump:
        output: ${output}
```

Generated mapping template example:

```json
{
  "Channel": "",
  "CommandLine": "",
  "EventID": ""
}
```

Recommended flow:

1. Run `sigma2quilt` to generate both the quilt YAML and `_mapping.json`
2. Fill in the CSV column names inside `_mapping.json`
3. Run `quilt --mapping <generated_mapping.json>`

Examples:

```bash
$ qsv sigma2quilt rules_windows_generic.json
$ qsv sigma2quilt rules_windows_generic.json -o custom.yaml
$ qsv sigma2quilt rules_dir/ -o generated_quilts/
$ qsv sigma2quilt rules_windows_generic.json --annotate
$ qsv sigma2quilt rules_windows_generic.json --separate -o generated_quilts/
$ qsv quilt quilt-rules_windows_generic.yaml --mapping quilt-rules_windows_generic_mapping.json --var input=events.csv --var output=alerts.csv
```

For time bucketing in Quilt, use the `bucket` step followed by `count` or a `calc` output step.

For scalar aggregation in a Quilt output stage:

```yaml
output_stage:
  type: output
  source: process_stage
  steps:
    calc:
      column: EventId
      avg: true
```

## Huge File Processing

qsv-rs supports streaming processing for huge files without loading them entirely into memory.

### Memory Behavior by Command

Not all commands stream. Before running a pipeline on a large file, check the memory behavior of each operation:

| Mode | Commands | Notes |
|------|----------|-------|
| **Streaming** (safe for huge files) | `show`, `dump`, `head`, `tail` | Row-by-row; constant memory |
| **Lazy / Polars-optimized** | `select`, `isin`, `contains`, `grep`, `sed` | Pushdown; usually safe |
| **Materializing** ⚠️ | `sort`, `uniq`, `count`, `stats` | Loads all rows into memory |

> **Warning:** Running a materializing command on a multi-GB file may exhaust memory. Use `head`, `timeslice`, or `isin` to reduce the dataset first.

### Usage Examples

```bash
# Stream display huge files (1GB batches by default)
$ qsv load huge.csv - show

# Custom memory usage - 512MB batches
$ qsv load huge.csv - show --batch-size 512MB

# High-memory server - 2GB batches for maximum performance
$ qsv load huge.csv - show --batch-size 2GB

# Stream save large results to file with custom batch size
$ qsv load huge.csv - select important,columns - dump -o output.csv --batch-size 2GB
```

### Memory Configuration

```bash
# Configure batch size for your system
--batch-size 512MB    # Low memory systems
--batch-size 1GB      # Default (balanced)
--batch-size 2GB      # High memory systems (2GB+)

# Configure gzip decompression memory (environment variable)
export QSV_MEMORY_LIMIT_MB=512   # Low memory systems
export QSV_MEMORY_LIMIT_MB=1024  # Default (1GB)
export QSV_MEMORY_LIMIT_MB=2048  # High memory systems (2GB+)
```

### Gzip File Processing

```bash
# Process large gzip files with different memory settings
$ QSV_MEMORY_LIMIT_MB=2048 qsv load huge.csv.gz - show
$ QSV_MEMORY_LIMIT_MB=512 qsv load huge.csv.gz - head 1000 - show  # Low memory
```

### Parquet Cache for Performance

For repeated processing of large CSV files, convert to Parquet format for significantly faster loading.

**Performance Benefits:**
- Faster loading compared to CSV format
- Better compression (smaller file sizes)
- Preserves data types (no re-parsing needed)

```bash
# One-time conversion: CSV to Parquet cache
$ qsv load huge.csv - dumpcache -o huge.parquet

# Subsequent processing: Load from Parquet (much faster)
$ qsv load huge.parquet - select col1,col2 - show
$ qsv load huge.parquet - isin category "important" - dump -o result.csv
```

## Installation

### Pre-built Binaries
Download the latest release from [GitHub Releases](https://github.com/sumeshi/qsv-rs/releases).

### Build from Source
```bash
$ git clone https://github.com/sumeshi/qsv-rs.git
$ cd qsv-rs
$ cargo build --release
```

## Contributing
Contributions are welcome! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License
This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

Inspired by [xsv](https://github.com/BurntSushi/xsv).
