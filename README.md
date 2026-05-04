# Quilter-CSV
[![MIT License](http://img.shields.io/badge/license-MIT-blue.svg?style=flat)](LICENSE)
[![CI/CD Pipeline](https://github.com/sumeshi/qsv-rs/actions/workflows/release.yml/badge.svg?branch=main)](https://github.com/sumeshi/qsv-rs/actions/workflows/release.yml)

![qsv-rs](https://gist.githubusercontent.com/sumeshi/c2f430d352ae763273faadf9616a29e5/raw/8484142e88948ecc0c8887db8f3bbb5be0dbe51e/qsv-rs.svg)

A fast, flexible, and memory-efficient command-line tool written in Rust for processing large CSV files. Inspired by [xsv](https://github.com/BurntSushi/xsv) and built on [Polars](https://www.pola.rs/), it's designed for handling tens or hundreds of gigabytes of CSV data efficiently in workflows like log analysis and digital forensics.

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

**Note:** If no finalizer is explicitly specified, `showtable` is automatically used as the default finalizer, making it easy to quickly view results:

```bash
$ qsv load data.csv - select col1,col2 - head 5
# Equivalent to:
$ qsv load data.csv - select col1,col2 - head 5 - showtable
```

## Command Reference

### Initializers

#### `load`
Load one or more CSV or Parquet files.

**Supported formats:**
- CSV files (.csv, .tsv, .txt)
- Gzipped CSV files (.csv.gz)
- Parquet files (.parquet) - high performance, preserves data types

| Parameter     | Type        | Default | Description                                      |
|---------------|-------------|---------|--------------------------------------------------|
| path          | list[str] |         | One or more paths to CSV or Parquet files. Quoted glob patterns such as `"logs/*.tsv"` are supported. Cannot mix CSV and Parquet files in the same command. |
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
| -i, --ignore-case | flag | `false` | Perform case-insensitive matching. |
| -v, --invert-match | flag | `false` | Invert the sense of matching, to select non-matching lines. |

Example:
```bash
$ qsv load data.csv - grep foo 
$ qsv load data.csv - grep "^FOO" -i                        # Case-insensitive search
$ qsv load data.csv - grep "^FOO" --ignore-case              # Long form case-insensitive
$ qsv load data.csv - grep "^FOO" -i -v                     # Case-insensitive inverted match
$ qsv load data.csv - grep "^FOO" --ignore-case --invert-match  # Long form inverted match
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
Count duplicate rows, grouping by all columns. Results are automatically sorted by count in descending order.

| Parameter | Type | Default | Description |
|---|---|---|---|
| (None)    |      |         | Takes no arguments. Automatically sorts output by count column in descending order. |

```bash
$ qsv load data.csv - count
$ qsv load data.csv - count - sort col1  # Count and then sort by col1 instead
```

#### `uniq`
Filters unique rows, removing duplicates based on all columns.

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

#### `convert`
Converts data formats between JSON, YAML, and XML. Also supports formatting/prettifying data in the same format.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| colname | str |         | Column name containing the data to convert. Required. |
| --from | str |         | Source format: `json`, `yaml`, or `xml`. Required. |
| --to | str |         | Target format: `json`, `yaml`, or `xml`. Required. |

**Supported conversions:**
- Cross-format: `json ↔ yaml`, `json ↔ xml`, `yaml ↔ xml`
- Same-format (formatting): `json → json`, `yaml → yaml`, `xml → xml`

**Features:**
- Automatically handles malformed JSON with extra quotes
- Prettifies and formats data for better readability
- Preserves data structure during conversion

Example:
```bash
$ qsv load data.csv - convert json_col --from json --to yaml
$ qsv load data.csv - convert config --from yaml --to json
$ qsv load data.csv - convert data --from json --to xml
$ qsv load data.csv - convert messy_json --from json --to json  # Format/prettify JSON
$ qsv load data.csv - convert compact_yaml --from yaml --to yaml  # Format YAML
```

#### `timeline`
Aggregates data by time intervals, creating time-based summaries.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| time_column | str |         | Name of the datetime column to use for time bucketing. Required. |
| --interval | str |         | Time interval for aggregation (e.g., `1h`, `30m`, `5s`, `1d`). Required. |
| --sum | str | | Column name to sum within each time bucket. Optional. |
| --avg | str | | Column name to average within each time bucket. Optional. |
| --min | str | | Column name to find minimum within each time bucket. Optional. |
| --max | str | | Column name to find maximum within each time bucket. Optional. |
| --std | str | | Column name to calculate standard deviation within each time bucket. Optional. |

**Features:**
- Creates a time bucket column named `timeline_{interval}` (e.g., `timeline_1h`, `timeline_30m`)
- If no aggregation column is specified, only row counts are provided for each time bucket
- Supports various time interval formats: hours (`1h`), minutes (`30m`), seconds (`5s`), days (`1d`)

Example:
```bash
$ qsv load access.log - timeline timestamp --interval 1h
# Creates column: timeline_1h

$ qsv load metrics.csv - timeline time --interval 5m --avg cpu_usage
# Creates columns: timeline_5m, count, avg_cpu_usage

$ qsv load sales.csv - timeline date --interval 1d --sum amount
# Creates columns: timeline_1d, count, sum_amount

$ qsv load server.log - timeline timestamp --interval 30s --max response_time
# Creates columns: timeline_30s, count, max_response_time
```

#### `timeslice`
Filters data based on time ranges, extracting records within specified time boundaries.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| time_column | str |         | Name of the datetime column to filter on. Required. |
| --start | str | | Start time (inclusive). Optional. |
| --end | str | | End time (inclusive). Optional. |

At least one of `--start` or `--end` must be specified. Supports various datetime formats including ISO8601, timestamps, and common log formats.

Example:
```bash
$ qsv load data.csv - timeslice timestamp --start "2023-01-01 00:00:00"
$ qsv load data.csv - timeslice timestamp --end "2023-12-31 23:59:59"
$ qsv load data.csv - timeslice timestamp --start "2023-06-01" --end "2023-06-30"
$ qsv load access.log - timeslice timestamp --start "2023-01-01T10:00:00"
```

#### `pivot`
Creates grouped aggregations over row and column keys.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| --rows | str |         | Comma-separated list of columns for row grouping. Optional. |
| --cols | str |         | Comma-separated list of columns for column grouping. Optional. |
| --values | str |         | Column to aggregate values from. Required. |
| --agg | str |         | Aggregation function: `sum`, `mean`, `count`, `min`, `max`, `median`, `std`. Optional (default: `sum`). |

At least one of `--rows` or `--cols` must be specified. This currently returns a long-form grouped aggregation over the requested row and column keys, not a wide Excel-style cross-tabulation table.

Example:
```bash
$ qsv load sales.csv - pivot --rows region --cols product --values sales_amount --agg sum
$ qsv load data.csv - pivot --rows category --cols year --values revenue --agg mean
$ qsv load logs.csv - pivot --rows date --cols error_type --values count --agg count
$ qsv load metrics.csv - pivot --rows department --values performance --agg median
```

#### `timeround`
Rounds datetime values to specified time units, creating a new rounded column while preserving the original.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| colname | str |         | Name of the datetime column to round. Required. |
| --unit | str |         | Time unit for rounding: `y`/`year`, `M`/`month`, `d`/`day`, `h`/`hour`, `m`/`minute`, `s`/`second`. Required. |
| --output | str | (replaces original) | Name for the output column. If not specified, replaces the original column. |

**Features:**
- Rounds datetime values down to the nearest specified time unit boundary
- Useful for time-based grouping and analysis
- Supports both short (`h`, `d`) and long (`hour`, `day`) unit names
- Output format automatically adjusts to the specified unit (clean, minimal format)

**Output formats by unit:**
- **year (y)**: `2023`
- **month (M)**: `2023-01`
- **day (d)**: `2023-01-01`
- **hour (h)**: `2023-01-01 12`
- **minute (m)**: `2023-01-01 12:34`
- **second (s)**: `2023-01-01 12:34:56`

Example:
```bash
$ qsv load data.csv - timeround timestamp --unit d --output date_only
# Input:  2023-01-01 12:34:56
# Output: 2023-01-01

$ qsv load data.csv - timeround timestamp --unit h --output hour_rounded
# Input:  2023-01-01 12:34:56
# Output: 2023-01-01 12

$ qsv load logs.csv - timeround timestamp --unit m
# Rounds to minute boundary, replaces original column

$ qsv load metrics.csv - timeround created_at --unit year --output created_year
# Input:  2023-01-01 12:34:56
# Output: 2023
```

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

Example:
```bash
$ qsv load data.csv - select col1,col2 - head 3 - showtable
# Output includes: shape: (3, 2) followed by formatted table

$ qsv load large_data.csv - select col1,col2
# Automatically calls showtable if no finalizer specified
```

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

Timeline steps in Quilt use explicit aggregation keys:

```yaml
stages:
  hourly_metrics:
    type: process
    steps:
      load:
        path: metrics.csv
      timeline:
        time_column: timestamp
        interval: 1h
        agg_type: avg
        agg_column: cpu_usage
      show:
```

## Huge File Processing

qsv-rs supports streaming processing for huge files without loading them entirely into memory.

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
