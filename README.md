# Quilt
[![MIT License](http://img.shields.io/badge/license-MIT-blue.svg?style=flat)](LICENSE)
[![CI](https://github.com/sumeshi/quilt/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/sumeshi/quilt/actions/workflows/ci.yml)

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
# yaml-language-server: $schema=https://raw.githubusercontent.com/sumeshi/quilt/develop/schemas/run-v2.schema.json
version: 2
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

To see available commands and basic usage, run `qlt` without arguments or with
`-h`. For command-specific usage, run `qlt <command> --help`.

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

### Interactive mode

```bash
$ qlt --lazy Security.jsonl
$ qlt --lazy --all-strings events.csv
```

Type pipeline commands such as `grep ERROR` or `filter 'id == 4624'`. Enter
confirms the draft; valid drafts preview live, and Tab completes commands,
options, and columns. Ctrl+A shows the available shortcuts; Esc cancels the prefix.
`:edit STEP` reopens the original command group containing
that 1-based step; the TUI shows the group's step range. An incomplete or invalid
edit shows marked context before that group until you enter a valid command.

| Key / command | Action |
| --- | --- |
| Tab; Up / Down | Complete commands, options, or columns; choose a completion |
| Ctrl+A, then `t` / `b` / `i` | Focus the table / pipeline / input |
| Table focus: arrows or `hjkl`; Enter; Esc | Move; open cell details; focus input |
| Pipeline focus: arrows or `j` / `k`; Enter; Esc | Move; edit a source group; focus input |
| Ctrl+A, then `f` | Expand or shrink the focused pane |
| Ctrl+A, then `?` / `h` / `e` / `d` | Keyboard help / command help / error details / row details |
| Ctrl+A, then `c` | Select columns |
| Ctrl+A, then `w` / `o` | Save YAML / export the confirmed result |
| Ctrl+A, then `C` / `x` | Create a checkpoint / clear the cache |
| Ctrl+A, then `p` | Switch between sample and full-data preview |
| Ctrl+A, then `u` / `r` | Undo / redo pipeline changes |
| Ctrl+A, then `z` | Cancel busy work (best effort) |
| Ctrl+A, then `q`; Ctrl-C | Quit / exit |
| Details: arrows or `j` / `k`; PgUp/PgDn; Home/End; `w`; Esc | Scroll; page; jump to ends; toggle wrapping; close |
| `:edit STEP` | Reopen the original command group containing the 1-based step |
| `:write PATH`; `:export PATH [--format csv\|jsonl]` | Save YAML; export the full confirmed result |
| `:cache [PATH]`; `:clear-cache` | Create a checkpoint; clear cache |
| `:undo`; `:redo`; `:quit` | Undo; redo; quit |

Export chooses JSONL for `.jsonl` and `.ndjson` paths and CSV for other paths;
`--format` overrides the extension. Exports do not overwrite existing files.
Busy work shows elapsed time, and cancellation may take time to finish.
Detail text is capped at 256 KiB and marked when truncated.

For frequency summaries, `select hoge,fuga - count` reports counts for each
unique combination, while `count hoge` lists `hoge` values by descending
frequency. By default, counts describe the preview sample of up to 10,000 input
rows; use Ctrl+A then `p` to switch to counts across all input data.

In the column picker, use Space to toggle, arrows or `hjkl` to move, `/` to
filter, `a` to select all filtered columns, `i` to invert, and `u` to clear.
While filtering, Enter ends the filter; press Enter again to apply the selection
to the draft. Esc cancels. Sample preview uses up to 10,000 source rows within a
32 MiB estimate; the table shows up to 100 rows and 128 columns. Initial loading
validates all input, and full preview evaluates all data. Temporary checkpoints
share a 512 MiB budget, are cleaned up on normal exit, and exports do not
overwrite existing files. YAML saves confirmed commands as a reusable `qlt run`
recipe, with input files supplied at execution time and no sample limits or
checkpoint paths. On exit, queued work is cancelled and any active operation
finishes before the terminal closes.

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

A standalone `-` separates commands. The chain is a single lazy query; it does not
serialize intermediate CSV between commands. Quote arguments containing spaces and
quoted globs you want Quilt to expand.

- Separate commands with `-`: `qlt load file.csv - select col1 - head 5`.
- Pass an option value beginning with `-` in attached form, such as `--separator=-`.
- Use `--` to stop option parsing for the current command before a positional value:
  `qlt load file.csv - grep -- -Info`.
- For a fixed-arity command, `-` after `--` can fill a required positional argument.
  Once those arguments are filled, the next `-` separates commands:
  `qlt load file.csv - grep -- - - show` searches for a literal hyphen.

`-` remains the command separator. Read finite standard input with an explicit
format, for example `producer | qlt load --stdin-format jsonl - show --format jsonl`.
Use `show` for stdout. Unknown or duplicate options are errors.

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
- JSON Lines files (.jsonl, .ndjson, .jsonl.gz, .ndjson.gz), including nested objects
- Parquet files (.parquet) - high performance, preserves data types

| Parameter     | Type        | Default | Description                                      |
|---------------|-------------|---------|--------------------------------------------------|
| path          | list[str] |         | One or more paths to CSV, JSONL/NDJSON, or Parquet files. Quoted glob patterns such as `"logs/*.tsv"` are supported. Input families cannot be mixed in one command. |
| -s, --separator | str       | `,`     | Field separator character (CSV files only).     |
| --low-memory  | flag    | `false` | Enable low-memory mode for very large files (CSV files only). |
| --no-headers  | flag    | `false` | Treat the first row as data, not headers (CSV files only). When enabled, columns will be named automatically (`column_1`, `column_2`, etc.). |
| --chunk-size  | int     | (auto)  | Number of rows to read per chunk (CSV files only). Controls memory usage during file processing. |
| --infer-schema-length | int or `full` | `full` | Limit CSV/NDJSON schema discovery per file; all later records are still validated. |
| --all-strings | flag | `false` | Read CSV fields as String before any conversion. |
| --schema | `column=type` | none | Override a CSV column's type; repeatable. Types: `string`, `int`, `uint`, `float`, `bool`. |
| --stdin-format | str | none | Read finite stdin as `csv`, `tsv`, `jsonl`, or `ndjson`; cannot be combined with paths. |
| --rejects | path | none | Write rejected CSV/NDJSON records and reasons to a new JSONL report; keep accepted records. |
| --source-column | str | none | Add each input's expanded path as a String column. Available for all input formats; an existing column with this name is an error. |

CSV separators must be one ASCII character; TSV requires an explicit tab separator.
Concatenated gzip members are all read, and corruption in a later member fails the load.
Gzip input is expanded to temporary disk before the lazy scan.
Existing input paths are treated literally, including brackets; otherwise glob patterns are expanded.

NDJSON discovers fields across all records and files by default. With
`--infer-schema-length N`, later records are still validated; late fields or types
are errors rather than silently omitted. Blank lines are ignored, and empty files
can accompany nonempty files. Inputs with no columns cannot be loaded.

Integer IDs are preserved exactly as Int64 or UInt64, including values above 2^53.
Incompatible types (including integer/float mixtures), out-of-range integers, and
negative values mixed with values above Int64::MAX are rejected. Normalize such
fields to strings before loading.
Duplicate object keys, including escaped-equivalent names at any nesting depth,
are rejected with the input path, physical line, and key path.

Missing fields and nulls have the same representation. Empty objects mixed with
null/missing values are currently unsupported; add a typed field or normalize them
before loading. Empty objects must also be normalized before Parquet export.
Gzipped NDJSON uses temporary disk proportional to the uncompressed input and
retains the original path in `--source-column`. Gzipped input is not supported by
`run --show-plan`.

CSV validates all records before the lazy scan and preserves leading-zero IDs as
strings. Quoted empty fields remain empty strings, distinct from null fields.
Fields without a lossless common numeric type also remain strings. Explicit
schema overrides are checked before conversion; casting after loading cannot recover
an identifier that was already parsed numerically. Input must be UTF-8; unsupported
UTF-16/UTF-32 BOMs and malformed UTF-8 fail with the source path and line.

```bash
$ qlt load events.csv --schema event_id=string --schema bytes=uint - show
$ qlt load events.csv --all-strings - show
$ qlt load events.jsonl --rejects rejected.jsonl - show --format jsonl
```

With `--rejects`, rejected records retain their source, physical line, raw text
(or hexadecimal bytes for invalid UTF-8), and rejection reason. The report is
published without overwriting an existing file. Accepted records use temporary
disk; malformed CSV quoting that prevents finding record boundaries still fails.
Input is strict by default. `run --check` does not read stdin, and `--show-plan`
cannot inspect inputs that require a spool.

CSV files loaded together must have compatible inferred column types. In particular,
a header-only CSV infers String columns and can conflict with numeric columns in a
nonempty sibling. This currently fails with a schema error; `--source-column` does
not resolve that mismatch.

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

For example, convert the JSONL output of `evtx2json` to CSV:

```bash
$ evtx2json Security.evtx --format jsonl -o Security.jsonl
$ qlt load Security.jsonl - flatten - dump -o Security.csv
$ qlt load Security.jsonl - flatten - isin event.code 4624,4625 - select '@timestamp,event.code,winlog.computer_name,winlog.event_data.SubjectUserName' - show
```

`flatten` turns nested object fields into dot-named columns. Arrays stay in one
cell and are encoded as JSON by `show` and `dump`; nulls become empty CSV cells.
Use `dumpcache` for Parquet output that preserves supported data types.

### Chainable Functions

#### `filter`
Keep rows matching a typed Boolean expression. Comparisons support `==`, `!=`,
`<`, `<=`, `>`, `>=`, column-to-column comparisons, `and`/`or`/`not` (also
`&&`/`||`/`!`), parentheses, and `IS NULL`/`IS NOT NULL`. Quote string literals;
use backticks for literal column names containing spaces or punctuation.
Incompatible types and implicit precision-losing conversions are errors.

```bash
$ qlt load events.jsonl - filter 'latency > 1000 and (status >= 500 or suspicious == true)'
$ qlt load events.csv - filter '`user name` == "alice" or message IS NULL'
```

#### `derive`
Create a new column from typed `+`, `-`, `*`, and `/` expressions. Existing output
columns are rejected. Integer results are checked Int64 values; cast a source to
Float64 explicitly before computing a floating-point rate.

```bash
$ qlt load events.csv - derive transferred 'sent + received'
$ qlt load events.csv - cast bytes float - derive bytes_per_second 'bytes / 60.0'
```

#### `aggregate`
Calculate several metrics in one lazy operation, optionally grouped by `--by`.
Pass each metric as a separate `mode:column:output` argument; `count:output`
counts rows. Modes are `count`, `sum`, `avg`, `min`, `max`, `median`, `std`,
`p0` through `p100`, and `nunique`. Column counts and `nunique` exclude nulls;
a sum with no non-null values is null. Integer sums fail on overflow. Groups
keep first-seen order, and the result can continue through the pipeline.
`min`, `max`, `p0`, and `p100` retain the input type and exact integer values.
Average, median, standard deviation, and interior percentiles use floating-point
statistics and can round large integers; use exact endpoints for identifier ranges.

```bash
$ qlt load events.jsonl - cast timestamp datetime - bucket timestamp 5m - aggregate count:events sum:bytes:total_bytes avg:latency:mean_latency p95:latency:p95_latency --by host,timestamp_bucket - show
```

#### `array-contains`, `array-get`, and `explode`
`array-contains <column> <value>` filters by exact typed scalar membership rather
than JSON substring matching. `null` searches for a null item; pass the quoted
string `"null"` within the argument to search for that text. Null lists do not match.
`array-get <column> <index> [--output name]` creates a column (default
`<column>_item`); negative indexes count from the end, and missing elements are null.
`explode <columns>` explicitly expands lists into rows. Null and empty lists
produce a null row; multiple selected lists must have matching lengths per row.

```bash
$ qlt load events.jsonl - array-contains tags suspicious - array-get tags 0 --output first_tag - show
$ qlt load events.jsonl - explode tags - count tags - show
```

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
- **Quoted colon notation**: `"col:1":"col:3"` - For column names containing colons (preserve the inner quotes when invoking from a shell)
- **Mixed formats**: `1,col2,4:6` - Combine different selection methods

**Disambiguation rule:** Exact column names take precedence over ranges, including
names such as `col1-col3` and `xmlns:auto-ns3`. For comma-containing names, use
inner quotes: `select '"a,b",xmlns:auto-ns3'`. In YAML, use a sequence such as
`columns: ["a,b", "xmlns:auto-ns3"]` to preserve literal names.
Column lists in `count`, `sort`, and `grep --column` use the same inner quoting,
which also preserves leading or trailing spaces in names.
Names such as `*` and `^name$` are also literal column names; quote them in the shell.

```bash
$ qlt load data.csv - select datetime                       # Select single column by name
$ qlt load data.csv - select col1,col3                      # Select specific columns by name
$ qlt load data.csv - select col1-col3                      # Select range using hyphen
$ qlt load data.csv - select col1:col3                      # Select range using colon
$ qlt load data.csv - select 1                              # Select 1st column (datetime)
$ qlt load data.csv - select 2:4                            # Select 2nd-4th columns (col1, col2, col3)
$ qlt load data.csv - select 2,4                            # Select 2nd and 4th columns (col1, col3)
$ qlt load data.csv - select '"col:1":"col:3"'              # For columns with colons in names
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
stores `Datetime[μs]` values. For datetime conversion, these options apply:

| Option | Default | Meaning |
|---|---|---|
| `--strict` | off | Disable fuzzy parsing; invalid non-null values still fail when this flag is omitted. |
| `--input-format FORMAT` | none | Parse with an explicit Chrono/strftime format. |
| `--epoch-unit s\|ms\|us\|ns` | none | Interpret numeric values using this unit; units are never guessed. |
| `--timezone ZONE` | none | Localize values without an offset to this timezone. |
| `--ambiguous error\|earliest\|latest` | `error` | Resolve repeated local times during DST fall-back. |
| `--nonexistent error\|shift-forward\|shift-backward` | `error` | Resolve nonexistent local times during DST spring-forward. |

Explicit input offsets determine the instant. Without an offset or `--timezone`, values
use a UTC-naive timeline; the investigation machine's local timezone is not assumed.
Datetime parsing options are rejected for non-datetime casts. They are also rejected
by `bucket` and `timeslice` when their input is already typed datetime.

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
$ qlt load data.csv - cast timestamp datetime --input-format '%Y-%m-%d %H:%M:%S' --timezone UTC - show
$ qlt load data.csv - cast epoch datetime --epoch-unit ms - show
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
existing columns and generated paths fail before the frame is changed. Empty
objects remain columns.

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

Integral deltas use lossless Int64 differences, including descending unsigned
values. UInt64 source values above i64::MAX and differences outside the Int64
range return a contextual conversion error at finalization. Float32 and Float64 retain their source precision.
Datetime deltas use Duration[μs] with checked unit conversion. The first row
and any pair involving a null produce null. Existing output names are rejected.

```bash
$ qlt load data.csv - delta count - show
$ qlt load data.csv - cast timestamp datetime - delta timestamp --output elapsed - show
$ qlt load data.csv - cast timestamp datetime - sort host,timestamp - delta timestamp --by host - show
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

Membership keys must be scalar; use `flatten` to select a nested field.

| Parameter | Type   | Default | Description                                                                          |
|-----------|--------|---------|--------------------------------------------------------------------------------------|
| colname   | str    |         | Column name to filter. Required.                                                     |
| values    | list   |         | Comma-separated values. Filters rows where the column matches any of these values (OR condition). Required. |

Values are parsed using the selected column's numeric or boolean type. Invalid
typed values are rejected instead of being silently compared as strings.

```bash
$ qlt load data.csv - isin col1 1
$ qlt load data.csv - isin col1 1,4
```

#### `contains`
Filter rows where a column contains a specific literal substring.

For nested columns, matching uses JSON text (including keys and escapes) and
preserves the original values. Use `flatten` to search a specific scalar field.

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

Global replacement leaves nested columns unchanged. To replace a nested field,
use `flatten` first; selecting an object or array directly is an error.

| Parameter   | Type   | Default | Description                                 |
|-------------|--------|---------|---------------------------------------------|
| pattern     | str    |         | Regex pattern to search for. Required.      |
| replacement | str    |         | Replacement string. Required.               |
| --column    | str    | (all)   | Apply replacement to specific column only. If not specified, applies to all columns. |
| -i, --ignore-case | flag | `false` | Perform case-insensitive matching.          |

When `--column` is omitted, `sed` replaces only String columns and preserves other
types. An explicit scalar target is converted to String; nested targets are rejected.

```bash
$ qlt load data.csv - sed foo foooooo                       # Replace 'foo' with 'foooooo' in all columns
$ qlt load data.csv - sed foo foooooo --column str          # Replace 'foo' with 'foooooo' in 'str' column only
$ qlt load data.csv - sed FOO foooooo -i                    # Case-insensitive replacement in all columns
$ qlt load data.csv - sed ".*o.*" foooooo --column str      # Regex replacement in specific column
```

#### `grep`
Filter rows where any column matches a regex pattern.

Nested columns are searched as JSON text, including keys and escapes; matched
rows retain their original values.

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
Count duplicate rows, grouping by all columns by default. Results are sorted by count descending; equal counts retain the first-seen group order.

> ⚠️ **Memory:** This is a global operation and may require memory proportional to the input.

| Parameter | Type | Default | Description |
|---|---|---|---|
| columns   | str | (all columns) | Optional positional column list. Use `col1` or `col1,col2` to group by specific columns only. |

The output column is named `count`; an input grouping column with that name is
rejected to avoid an ambiguous output schema.

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
Converts timestamps to a requested timezone and replaces the source column with
formatted String values. Use `cast ... datetime` when subsequent operations need
a typed datetime column.

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

During a DST fall-back, some local times occur twice. For example, 01:30 in
`America/New_York` can refer to either occurrence on the transition date:

- First occurrence: daylight time, before the transition.
- Second occurrence: standard time, after the transition.

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

At least one of `--start` or `--end` must be specified. Both boundaries are inclusive
(`[start, end]`); a date-only end means midnight at the start of that date. For string
input, the parser options listed under `cast` apply, including `--timezone` for naive
wall-clock values. For already-typed datetime input, omit parsing options. Embedded
offsets are authoritative instants; naive values without `--timezone` use the UTC-naive
timeline. Null timestamps are excluded. Invalid non-null values fail the command.

Example:
```bash
$ qlt load data.csv - timeslice timestamp --start "2023-01-01 00:00:00"
$ qlt load data.csv - timeslice timestamp --end "2023-12-31 23:59:59"
$ qlt load data.csv - timeslice timestamp --start "2023-06-01" --end "2023-06-30"
$ qlt load access.csv - timeslice timestamp --start "2023-01-01T10:00:00"
```

Time bucketing and aggregation use `bucket`, `count`, and `calc`.

#### `timeinfo`
Summarizes the earliest and latest timestamps over all selected rows, regardless of
input order. This is a lazy chainable operation, so its result can be displayed,
sorted, filtered, or saved with the usual finalizers. It does not sort the input.

| Parameter | Type | Default | Description |
|---|---|---|---|
| column | str | required | A Datetime column; cast strings first. |
| --by | str | none | Comma-separated grouping columns, such as `source_file` or `host,user`. |

The result contains grouping columns followed by `rows`, `valid_times`, `null_times`,
`first_seen`, and `last_seen`. `rows` includes null timestamps; `valid_times` excludes
them, and `null_times` counts them. An all-null group has null first/last timestamps.
Without `--by`, even an empty input produces one row with zero counts and null bounds.
Timestamp units and timezone metadata are preserved; group order is unspecified.
Grouping names must be unique and cannot use the five result-column names above.

With `load --source-column source_file` followed by `timeinfo ... --by source_file`,
the result includes every successfully loaded file, including schema-compatible
header-only CSVs and files with no
rows left after filtering. Such files have zero counts and null bounds. Other groupings
include only observed groups. Renaming the source column preserves this tracking;
removing or modifying its values ends it. Counts always reflect preceding filters.

```bash
# Per-file coverage, including files with no matching events
$ qlt load 'logs/*.csv' --source-column source_file - cast TimeCreated datetime --strict - timeinfo TimeCreated --by source_file - showtable
# Save the complete summary instead of the bounded table preview
$ qlt load 'logs/*.csv' --source-column source_file - cast TimeCreated datetime --strict - timeinfo TimeCreated --by source_file - dump -o coverage.csv
# Coverage after filtering, grouped by host
$ qlt load events.csv - cast TimeCreated datetime --strict - isin EventId 4624,4625 - timeinfo TimeCreated --by Computer - show
```

### Finalizers

Finalizers are used to output or summarize the processed data. Once a finalizer
is reached, only additional finalizers may follow; record-processing commands
cannot resume afterward. A pipeline may contain only one initializer, first.

#### `partition`
Splits data into separate CSV files based on unique values in a specified column. Each unique value creates its own file.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| colname | str |         | Column name to partition by. Required. |
| output_directory | str | `./partitions/` | Directory to save partitioned files. Optional - if not specified, creates a `./partitions/` directory. |

The output directory is created if it does not exist and must not already exist.
Its parent directory must already exist.
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
Displays the column headers of the current dataset. The formatted table numbers
columns from 1, matching numeric indices accepted by `select`.

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
25/50/75% quantiles, and max. Strings and Datetime columns report min/max; other supported Polars
dtypes show `-` for unsupported or unavailable measures. Empty and all-null
columns use `-` for unavailable statistics rather than panicking.

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
Writes the result to standard output. `--format csv` (default) includes the header;
`--format jsonl` emits one object per row. This command
takes no positional arguments. It is the implicit finalizer when a chain ends without one.
The full output is first written to a temporary file, then copied to stdout in bounded
chunks. This avoids emitting rows before lazy evaluation has succeeded, but uses disk
proportional to the rendered output. Use `showquery` to inspect the query plan.

Object and list columns are encoded as JSON strings inside CSV cells, with
normal CSV quoting. Null cells remain empty; empty lists are `[]`. Duration values at any nesting level
use exact integer counts in their declared unit (for example, `delta` uses microseconds).
JSONL preserves supported nested values, nulls and exact integers; datetime values
are strings and durations follow the same integer rule. Non-finite JSON numbers
are rejected, including inside CSV object/list cells. JSONL does not recover
distinctions already unified during loading, such as missing fields versus nulls.

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
- Struct fields are recursively displayed as dot-named columns; path collisions error
- Arrays remain one JSON cell per input row; rows are never automatically exploded
- The first 8 columns are displayed; the total column count and omitted count are shown
- Cells and headers are limited to 64 terminal display columns and truncated with `…`
- Newlines, tabs, ANSI/control and bidi characters are escaped before rendering
- Use `select` to choose columns and `show`/`dump` for complete machine-readable output

This command does not take any arguments or options.
Datetime, date, time, and duration cells use human-readable values; nulls remain
visible. The preview does not make an upstream global sort or aggregation bounded.
Use `show` or `dump` when you need every row.

```bash
$ qlt load data.csv - select col1,col2 - head 3 - showtable
# Output includes: shape: (3, 2) followed by formatted table
```

#### `dump`
Outputs the processing results to a CSV/TSV or JSONL file. The destination must not
already exist; output is first written to a temporary sibling and atomically
published with a no-replace primitive (with a safe hard-link fallback where
supported). The finalizer evaluates the lazy frame as its required write barrier.
As with `show`, the default is CSV; object and list values become JSON strings inside CSV cells;
nulls remain empty. Use `flatten` first to turn nested object fields into columns,
or `dumpcache` to preserve the original types in Parquet.

| Parameter | Type | Default | Description |
|---|---|---|---|
| --format | str | `csv` | `csv` or `jsonl`; JSONL uses the same type rules as `show`. |
| -o, --output | str | (automatic) | Destination path; defaults to `dump_<timestamp>.csv` or `.jsonl` for the selected format. |
| -s, --separator | char | `,` | Field separator character for the output CSV file. |

Example:
```bash
$ qlt load data.csv - dump                                  # Saves to dump_<timestamp>.csv
$ qlt load data.csv - head 100 - dump -o results.csv
$ qlt load data.csv - head 100 - dump --output results.csv
$ qlt load data.csv - head 100 - dump -o results.csv -s ';'
$ qlt load events.jsonl - filter 'status >= 500' - dump --format jsonl -o errors.jsonl
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
| `load` | CSV/TSV/gzip CSV, same-family JSONL/NDJSON, or Parquet; finite stdin with explicit format | Strict validation by default; `--rejects` explicitly isolates rejected records | Lazy scan after CSV/NDJSON validation; stdin/gzip/reject filtering use temporary disk |
| `select` | Existing names, 1-based indices, and ranges; output contains selected columns in requested order | Missing names, invalid/out-of-range/oversized ranges fail with usage/schema error | Lazy projection; schema-only validation |
| `isin` | Scalar column; output preserves schema and matching rows | Numeric/boolean values are parsed to the column type; nulls do not match; empty values produce an empty frame; invalid typed values and missing columns error | Lazy filter; schema-only validation |
| `contains` | String/coercible column plus literal substring; output preserves schema and matching rows | Scalars are string-coerced; nested values use JSON text; nulls do not match; missing columns and invalid options error | Lazy filter; schema-only validation |
| `grep` | Regex over all or named columns; output preserves schema and matching rows | Nulls stringify as non-matches; invalid regex/columns error | Lazy filter; schema-only validation |
| `sed` | Regex replacement over String columns, or one explicit scalar column | Nulls remain null; global mode preserves other types; explicit nested targets error | Lazy expression; schema-only validation |
| `head` | Any dtype; output keeps schema and at most N first rows | N must be non-negative; nulls unchanged | Lazy limit; bounded output evaluation |
| `tail` | Any dtype; output keeps schema and at most N last rows | N must be non-negative; nulls unchanged | Lazy tail barrier; upstream must be consumed to find final rows |
| `sort` | Existing sortable columns; output preserves schema and row values | Missing columns error; null order follows Polars | Lazy sort barrier; Polars may maintain input-proportional state |
| `count` | Optional grouping columns of any supported dtype; output is group columns plus `count` | Nulls form groups; missing columns and a grouping-column name collision with `count` error | Lazy grouped barrier; Polars observes the complete input |
| `timeinfo` | Datetime plus optional group keys; outputs counts and first/last timestamps | Null timestamps count separately; empty ungrouped input gives zero counts and null bounds | Lazy aggregate over all selected rows; no input sort |
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
| `show` | Any supported schema; CSV with header or JSONL | Conversion/write errors are returned; stdout contains data only | Lazy sink to a temporary artifact, then bounded copying |
| `showtable` | Any schema; bounded formatted preview with shape | Truncates rows/cells; rendering errors are returned | Intrinsic bounded-preview finalizer |
| `dump` | Any supported schema; writes CSV/TSV with header or JSONL | Existing target rejected; staged atomic write cleans failures | Streaming-capable sink/finalizer; target is never overwritten |
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
| --stage STAGE | str | Execute this stage and its required data/control dependencies. Ancestor finalizers are suppressed and unrelated inputs are not opened. |
| -o, --output | str | Destination for the selected stage or unique terminal non-branch stage. Overrides its single `dump` and retains its format, or adds CSV output when it has none. Relative to the caller working directory; existing files are rejected. |


```bash
$ qlt run rules/my_workflow.yaml
$ qlt run rules/my_workflow.yaml events.csv
$ qlt run rules/my_analysis.yaml -o result.csv
$ qlt run rules/my_workflow.yaml events.csv --stage output -o selected.csv
$ qlt run rules/my_workflow.yaml events.csv --show-plan output
```

Without `-o`, each YAML output path is used as written. With `-o`, the target stage
must be unambiguous: select it with `--stage`, or have one terminal non-branch stage.
Multiple `dump` steps within that target are rejected. Outputs explicitly declared in
other executed stages keep their own paths.

| Path origin | Relative-path base |
|---|---|
| YAML `load` paths, output paths, and `path` parameter defaults | Directory containing the YAML file |
| CLI input files, `--output`, and `--var` values declared as `path` | Caller working directory |

`--check` validates the complete document without opening input data. It cannot prove
that input columns or values are valid. `--show-plan` uses the same CLI inputs and
variables as execution and inspects the selected dependencies without running finalizers.
It can inspect input schemas, but does not evaluate rows or create intermediate files;
gzip sources that require a decompression spool cannot be inspected this way.
`--show-plan` cannot be combined with `--check` or `--stage`. Plans depending on secret
parameters are withheld, including plans produced by a YAML `showquery` step.

```yaml
version: 2
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

#### Step arguments in YAML

The operation name is the YAML key. Each `steps` item must be a one-key mapping. YAML
fields map to CLI positional arguments as follows; booleans default to `false`, omitted
numeric options keep the CLI defaults, and unknown fields are errors.

| Step | Fields |
|---|---|
| `load` | `paths` (optional with CLI input files or `stdin-format`), `source-column`, `separator`, `low-memory`, `no-headers`, `chunk-size`, `infer-schema-length`, `all-strings`, `schema` (column-to-type mapping), `stdin-format`, `rejects` |
| `select` | `columns` |
| `filter` | `expression` |
| `derive` | `output`, `expression` |
| `aggregate` | `metrics` (sequence of `{mode, column?, output}` mappings), optional `by` |
| `array-contains` | `column`, `value` (string; parsed against the element type) |
| `array-get` | `column`, `index`, optional `output` |
| `explode` | `columns` |
| `cast` | `column`, `type`; optional datetime fields `strict`, `input-format`, `epoch-unit`, `timezone`, `ambiguous`, `nonexistent` |
| `bucket` | `column`, `interval`; optional `output` and the same datetime fields |
| `delta` | `column`; optional `output`, `by` |
| `extract` | `column`, `pattern` |
| `flatten`, `uniq`, `stats`, `showtable`, `showquery` | `{}` |
| `parse-size` | `column` |
| `isin` | `column`, `values` (string/number or sequence) |
| `contains` | `column`, `pattern`, optional `ignore-case` |
| `sed` | `pattern`, `replacement`, optional `column`, `ignore-case` |
| `grep` | `pattern`, optional `columns`, `ignore-case`, `invert-match` |
| `head`, `tail` | optional `number` (default `5`) |
| `sort` | `columns`, optional `desc` |
| `count` | optional `columns` (all columns by default) |
| `timeinfo` | `column`, optional `by` |
| `changetz` | `column`, required `from-tz`, `to-tz`; optional `input-format`, `output-format`, `epoch-unit`, `strict`, `ambiguous`, `nonexistent`, `timezone` (YAML only) |
| `renamecol` | `old`, `new` |
| `timeslice` | `column`, at least one of `start` or `end`; optional datetime fields |
| `show` | optional `format` (`csv` or `jsonl`) |
| `headers` | optional `plain` |
| `dump` | optional `output`, `separator` (default comma), `format` (`csv` or `jsonl`) |
| `dumpcache` | optional `output` |
| `partition` | `column`; optional `output-dir` (default `./partitions`) |
| `calc` | `column` and exactly one true flag: `sum`, `avg`, `min`, `max`, `median`, `std` |

`grep` uses `--column` on the CLI but `columns` in YAML. A scalar column field is
one entry, including any commas; use an array for multiple columns, for example
`columns: [host, user]`. `select` retains its range/index syntax, with existing
column names taking precedence.

For example, these steps produce the same per-file summary as the CLI `timeinfo` example:

```yaml
- load: {paths: ["logs/*.csv"], source-column: source_file}
- cast: {column: TimeCreated, type: datetime, strict: true}
- timeinfo: {column: TimeCreated, by: [source_file]}
- show: {}
```

#### Pipeline Operations in YAML
Within a `run` document, stages can be different types to orchestrate the flow.

| Operation Type | Description                                                | Key Parameters                                                                                                                                    |
| -------------- | ---------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `process`      | Executes a series of qlt operations on a dataset.          | `name` and a sequence-form `steps` (for example, `- grep: {pattern: ERROR}`). `from` (optional) names a prior stage. |
| `concat`       | Concatenates multiple datasets (stages).                   | `name` and `concat: {inputs: [stage_a, stage_b], how: vertical}`. Only `vertical` (`v`) is supported; schemas must be compatible. |
| `join`         | Joins datasets from multiple stages based on keys.         | `name` and `join: {inputs: [...], on: [column], how: inner}`. Use `left-on` and `right-on` for asymmetric keys; `cross` needs no key. Modes: `inner` (default), `left`, `full` (`outer`), `cross`. Optional boolean `coalesce`. |
| `branch`       | Selects downstream target stages from a `row-count` or `parameter` predicate. | `name` and `branch: {input: stage, when: {row-count: {greater-than: 10}}, then: [target]}` with optional `else: [target]`. |

Every run document must use `version: 2` and a sequence of named stages. The schema is
strict: unknown document, stage, and step keys are rejected before any input is read.
Version 1 documents are rejected with migration guidance. Update `version` to `2`,
write multi-column fields as arrays, and rewrite aggregate metrics as mappings, for
example `{mode: sum, column: bytes, output: total}`.

Parameters are declared at the document root with one of `path`, `string`, `int`, or
`bool` types, with an optional `description` explaining each parameter. Values are
referenced as whole YAML values using `{"$param": name}`;
partial string interpolation is not supported. CLI `--var` values take precedence
over defaults. A parameter `path` default is relative to the run file; a `--var`
path is relative to the caller's working directory. A parameter cannot be both `required` and
have a `default`; secret values are redacted from diagnostics. Branch predicates support
typed `row-count` and `parameter` comparisons; scalar-result predicates are deferred.

Each predicate takes exactly one comparison: `equal`, `not-equal`, `greater-than`,
`greater-or-equal`, `less-than`, or `less-or-equal`. Boolean parameters support only
`equal` and `not-equal`. A parameter condition is written, for example,
`when: {parameter: {name: enabled, equal: true}}`, with `enabled` declared as `bool`.
The `else` target list is optional; an omitted false route runs no target stages.

```yaml
parameters:
  minimum_id:
    type: int
    default: 1000
```

YAML saved from the lazy interface takes input files as CLI arguments. File-output
paths become required `path` parameters named `output`, `output_2`, and so on;
partition directories use `output_dir`, and a rejects report uses `rejects`.
For a recipe with a file output:

```bash
$ qlt run workflow.yaml events.csv --var output=summary.csv
```

```yaml
version: 2
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
version: 2
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
version: 2
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
version: 2
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
    steps: [{dump: {output: selected-events.csv}}]
  - name: cache-output
    from: input
    steps: [{dumpcache: {output: events-cache.parquet}}]
```

Typed parameters and static validation:

```yaml
version: 2
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

One document may contain multiple independent file-output stages, executed in dependency
order. Only one stdout-producing finalizer may run; use `--stage` to choose one or save
multiple results with file finalizers. Without an explicit output, `run` exposes its final
frame and the CLI's implicit `show` applies. Selecting a stage controlled by a branch also includes that controlling branch;
an unchosen route is skipped. `--check` validates the complete schema, typed parameters, command arguments,
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

#### Reuse, progress, and failures

Each stage accepts `materialize: auto` (default), `always`, or `never`. Auto writes a
shared result to temporary Parquet when it has multiple effective consumers, including
row-count branch evaluation and row-reading finalizers. A stage used once stays lazy;
`sort` alone does not force a temporary Parquet. `never` can repeat upstream work when
consumers evaluate independently. `always` adds disk I/O even for a single consumer.
Auto keeps results lazy when their schema contains empty objects that Parquet cannot
represent; `always` reports an error for those schemas.

`--stage` limits this calculation to the selected execution graph. `--show-plan` reports
dependencies, inputs/outputs, and planned materialization alongside the query plan.
During execution, stderr reports selected stages and planned disk intermediates; completion
reports elapsed time and completed file outputs. These diagnostics do not add a full
row count. Intermediate files are released when no remaining lazy consumer needs them.

Outputs are committed individually. If a later stage fails, already-completed files
remain and the error lists them. There is no run-wide rollback or automatic resume.
Each file finalizer still rejects existing targets and cleans its incomplete staging.
Secrets are hidden in diagnostics and plans; a secret parameter is not a request to
redact values intentionally selected into exported data.

#### Reusable log investigation recipe

[`examples/timeline.yaml`](examples/timeline.yaml) applies an inclusive time range while
preserving every input column. Its default time column is `TimeCreated`; normalized Linux
logs use `timestamp`:

```sh
qlt run examples/timeline.yaml --var input=events.csv --var start=2024-01-01T00:00:00Z \
  --var end=2024-01-31T23:59:59Z --var output=period.csv
qlt run examples/timeline.yaml --var input=linux.csv --var time-column=timestamp \
  --var start=2024-01-01T00:00:00Z --var end=2024-01-31T23:59:59Z --var output=period.csv
```

Inputs are structured log exports;
raw Linux syslog/access-log text must first be converted to CSV or JSONL with a timestamp
column. No cross-source normalization is performed.

## Huge File Processing

Quilt supports streaming-capable execution for many large-file processing pipelines.

### Memory Behavior by Command

Not all commands stream. Before running a pipeline on a large file, check the memory behavior of each operation:

| Mode | Commands | Notes |
|------|----------|-------|
| **Streaming-capable** | `show`, `dump`, `dumpcache` | Prefer these sinks for large inputs; `show` uses temporary disk proportional to rendered output |
| **Bounded / lazy** | `head`, `headers`, `showtable`, `showquery` | `head` limits output; `headers` inspects schema only; `showtable` previews a bounded number of rows; plans do not collect |
| **Lazy / Polars-optimized** | `select`, `filter`, `derive`, `array-contains`, `array-get`, `explode`, `isin`, `contains`, `grep`, `sed`, `renamecol`, `cast`, `parse-size`, `bucket`, `delta`, `extract`, `flatten`, `changetz`, `timeslice` | `explode` can increase row count; grouped `delta` keeps group state |
| **Global barriers** ⚠️ | `sort`, `uniq`, `count`, `tail` | Polars may maintain input-proportional state to produce a global result |
| **Aggregate operations** | `aggregate`, `timeinfo`, `stats`, `calc` | Read all selected rows; group cardinality and the chosen statistic determine memory requirements |
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

### Temporary files and cleanup

Gzip expansion, stdin, rejected-record filtering, `show`, run materialization, and
partition staging use temporary disk.
`dump`/`dumpcache` stage beside their destination so completed files can be published
without overwriting existing data. Partition staging includes the complete output
directory as well as its intermediate Parquet. Available disk must cover these files
as well as the final result; lazy execution is not a guarantee of constant memory or disk.

Normal completion and handled errors remove execution-owned temporary files. SIGKILL
or a system crash can leave files behind. Exported CSV/Parquet and partition directories
are user data and are never removed as cache cleanup.

## Installation

### Pre-built Binaries
Download the latest release from [GitHub Releases](https://github.com/sumeshi/quilt/releases).

### Build from Source
```bash
$ git clone https://github.com/sumeshi/quilt.git
$ cd quilt
$ cargo build --locked
```

The executable is `target/debug/qlt` (`qlt.exe` on Windows). For an optimized
build, use `cargo build --release --locked`; the executable is in `target/release/`.

## Contributing

Install stable Rust with `rustfmt` and `clippy`, then run:

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all -- --check
```

## License
This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

Inspired by [xsv](https://github.com/BurntSushi/xsv).
