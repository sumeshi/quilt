use crate::error::QuiltError;
use crate::operations::datetime::{
    parse_ambiguous_policy, parse_epoch_unit, parse_nonexistent_policy, AmbiguousPolicy,
    DateTimeConfig, EpochUnit, NonexistentPolicy,
};
use regex::Regex;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandCategory {
    Initializer,
    Chainable,
    Finalizer,
    Automation,
}

#[derive(Debug, Clone, Copy)]
pub struct OptionSpec {
    pub name: &'static str,
    pub short: Option<&'static str>,
    pub takes_value: bool,
    pub repeated: bool,
}
#[derive(Debug, Clone, Copy)]
pub struct CommandSpec {
    pub category: CommandCategory,
    pub name: &'static str,
    pub min_args: usize,
    pub max_args: Option<usize>,
    pub options: &'static [OptionSpec],
    pub help: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Aggregation {
    Sum,
    Avg,
    Min,
    Max,
    Median,
    Std,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadArgs {
    pub paths: Vec<PathBuf>,
    pub separator: char,
    pub low_memory: bool,
    pub no_headers: bool,
    pub chunk_size: Option<usize>,
    /// `Some(n)` bounds NDJSON inference to `n` records per file; `None` is
    /// the explicit full-inference mode.
    pub infer_schema_length: Option<usize>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectArgs {
    pub columns: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CastArgs {
    pub column: String,
    pub target: String,
    pub datetime: DateTimeConfig,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketArgs {
    pub column: String,
    pub interval: String,
    pub output: Option<String>,
    pub datetime: DateTimeConfig,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnOutputArgs {
    pub column: String,
    pub output: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractArgs {
    pub column: String,
    pub pattern: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsinArgs {
    pub column: String,
    pub values: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainsArgs {
    pub column: String,
    pub pattern: String,
    pub ignore_case: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SedArgs {
    pub pattern: String,
    pub replacement: String,
    pub column: Option<String>,
    pub ignore_case: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrepArgs {
    pub pattern: String,
    pub ignore_case: bool,
    pub invert_match: bool,
    pub columns: Option<Vec<String>>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberArgs {
    pub number: usize,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortArgs {
    pub columns: Vec<String>,
    pub descending: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountArgs {
    pub columns: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeTzArgs {
    pub column: String,
    pub from_tz: String,
    pub to_tz: String,
    pub input_format: Option<String>,
    pub output_format: Option<String>,
    pub ambiguous: AmbiguousPolicy,
    pub nonexistent: NonexistentPolicy,
    pub strict: bool,
    pub epoch_unit: Option<EpochUnit>,
    pub timezone: Option<String>,
    pub options_present: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeSliceArgs {
    pub column: String,
    pub start: Option<String>,
    pub end: Option<String>,
    pub datetime: DateTimeConfig,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpArgs {
    pub output: Option<String>,
    pub separator: char,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionArgs {
    pub column: String,
    pub output_dir: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalcArgs {
    pub column: String,
    pub aggregation: Aggregation,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunArgs {
    pub config: PathBuf,
    pub input_files: Vec<PathBuf>,
    pub output: Option<String>,
    pub vars: Vec<String>,
    pub check: bool,
    pub show_plan: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypedCommand {
    Load(LoadArgs),
    Select(SelectArgs),
    Cast(CastArgs),
    Bucket(BucketArgs),
    Delta(ColumnOutputArgs),
    Extract(ExtractArgs),
    Flatten,
    ParseSize { column: String },
    Isin(IsinArgs),
    Contains(ContainsArgs),
    Sed(SedArgs),
    Grep(GrepArgs),
    Head(NumberArgs),
    Tail(NumberArgs),
    Sort(SortArgs),
    Count(CountArgs),
    Uniq,
    ChangeTz(ChangeTzArgs),
    RenameCol { old: String, new: String },
    TimeSlice(TimeSliceArgs),
    Show { debug: bool },
    ShowTable,
    Headers { plain: bool },
    Stats,
    ShowQuery,
    Dump(DumpArgs),
    DumpCache { output: Option<String> },
    Partition(PartitionArgs),
    Calc(CalcArgs),
    Run(RunArgs),
}

impl TypedCommand {
    pub fn category(&self) -> CommandCategory {
        match self {
            Self::Load(_) => CommandCategory::Initializer,
            Self::Run(_) => CommandCategory::Automation,
            Self::Select(_)
            | Self::Cast(_)
            | Self::Bucket(_)
            | Self::Delta(_)
            | Self::Extract(_)
            | Self::Flatten
            | Self::ParseSize { .. }
            | Self::Isin(_)
            | Self::Contains(_)
            | Self::Sed(_)
            | Self::Grep(_)
            | Self::Head(_)
            | Self::Tail(_)
            | Self::Sort(_)
            | Self::Count(_)
            | Self::Uniq
            | Self::ChangeTz(_)
            | Self::RenameCol { .. }
            | Self::TimeSlice(_) => CommandCategory::Chainable,
            _ => CommandCategory::Finalizer,
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Load(_) => "load",
            Self::Select(_) => "select",
            Self::Cast(_) => "cast",
            Self::Bucket(_) => "bucket",
            Self::Delta(_) => "delta",
            Self::Extract(_) => "extract",
            Self::Flatten => "flatten",
            Self::ParseSize { .. } => "parse-size",
            Self::Isin(_) => "isin",
            Self::Contains(_) => "contains",
            Self::Sed(_) => "sed",
            Self::Grep(_) => "grep",
            Self::Head(_) => "head",
            Self::Tail(_) => "tail",
            Self::Sort(_) => "sort",
            Self::Count(_) => "count",
            Self::Uniq => "uniq",
            Self::ChangeTz(_) => "changetz",
            Self::RenameCol { .. } => "renamecol",
            Self::TimeSlice(_) => "timeslice",
            Self::Show { .. } => "show",
            Self::ShowTable => "showtable",
            Self::Headers { .. } => "headers",
            Self::Stats => "stats",
            Self::ShowQuery => "showquery",
            Self::Dump(_) => "dump",
            Self::DumpCache { .. } => "dumpcache",
            Self::Partition(_) => "partition",
            Self::Calc(_) => "calc",
            Self::Run(_) => "run",
        }
    }
}

const NONE: &[OptionSpec] = &[];
const DATETIME: &[OptionSpec] = &[
    OptionSpec {
        name: "strict",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "input-format",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "epoch-unit",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "timezone",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "ambiguous",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "nonexistent",
        short: None,
        takes_value: true,
        repeated: false,
    },
];
const BUCKET: &[OptionSpec] = &[
    OptionSpec {
        name: "output",
        short: Some("o"),
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "strict",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "input-format",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "epoch-unit",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "timezone",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "ambiguous",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "nonexistent",
        short: None,
        takes_value: true,
        repeated: false,
    },
];
const LOAD: &[OptionSpec] = &[
    OptionSpec {
        name: "separator",
        short: Some("s"),
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "low-memory",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "no-headers",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "chunk-size",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "infer-schema-length",
        short: None,
        takes_value: true,
        repeated: false,
    },
];
const OUTPUT: &[OptionSpec] = &[OptionSpec {
    name: "output",
    short: Some("o"),
    takes_value: true,
    repeated: false,
}];
const IGNORE: &[OptionSpec] = &[OptionSpec {
    name: "ignore-case",
    short: Some("i"),
    takes_value: false,
    repeated: false,
}];
const SED: &[OptionSpec] = &[
    OptionSpec {
        name: "ignore-case",
        short: Some("i"),
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "column",
        short: None,
        takes_value: true,
        repeated: false,
    },
];
const GREP: &[OptionSpec] = &[
    OptionSpec {
        name: "ignore-case",
        short: Some("i"),
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "invert-match",
        short: Some("v"),
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "column",
        short: None,
        takes_value: true,
        repeated: false,
    },
];
const NUMBER: &[OptionSpec] = &[OptionSpec {
    name: "number",
    short: Some("n"),
    takes_value: true,
    repeated: false,
}];
const DESC: &[OptionSpec] = &[OptionSpec {
    name: "desc",
    short: Some("d"),
    takes_value: false,
    repeated: false,
}];
const TZ: &[OptionSpec] = &[
    OptionSpec {
        name: "from-tz",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "to-tz",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "input-format",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "output-format",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "ambiguous",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "nonexistent",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "strict",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "epoch-unit",
        short: None,
        takes_value: true,
        repeated: false,
    },
];
const TIMESLICE: &[OptionSpec] = &[
    OptionSpec {
        name: "start",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "end",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "strict",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "input-format",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "epoch-unit",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "timezone",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "ambiguous",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "nonexistent",
        short: None,
        takes_value: true,
        repeated: false,
    },
];
const SHOW: &[OptionSpec] = &[OptionSpec {
    name: "debug",
    short: None,
    takes_value: false,
    repeated: false,
}];
const DUMP: &[OptionSpec] = &[
    OptionSpec {
        name: "output",
        short: Some("o"),
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "separator",
        short: Some("s"),
        takes_value: true,
        repeated: false,
    },
];
const HEADERS: &[OptionSpec] = &[OptionSpec {
    name: "plain",
    short: Some("p"),
    takes_value: false,
    repeated: false,
}];
const CALC: &[OptionSpec] = &[
    OptionSpec {
        name: "sum",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "avg",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "min",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "max",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "median",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "std",
        short: None,
        takes_value: false,
        repeated: false,
    },
];
const RUN: &[OptionSpec] = &[
    OptionSpec {
        name: "check",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "output",
        short: Some("o"),
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "var",
        short: None,
        takes_value: true,
        repeated: true,
    },
    OptionSpec {
        name: "show-plan",
        short: None,
        takes_value: true,
        repeated: false,
    },
];

static SPECS: &[CommandSpec] = &[
    CommandSpec {
        category: CommandCategory::Initializer,
        name: "load",
        min_args: 1,
        max_args: None,
        options: LOAD,
        help: "load [files...] [options] — CSV, JSONL/NDJSON, or Parquet",
    },
    CommandSpec {
        category: CommandCategory::Chainable,
        name: "select",
        min_args: 1,
        max_args: None,
        options: NONE,
        help: "select <columns>",
    },
    CommandSpec {
        category: CommandCategory::Chainable,
        name: "cast",
        min_args: 2,
        max_args: Some(2),
        options: DATETIME,
        help: "cast <column> <type>",
    },
    CommandSpec {
        category: CommandCategory::Chainable,
        name: "bucket",
        min_args: 2,
        max_args: Some(2),
        options: BUCKET,
        help: "bucket <column> <interval>",
    },
    CommandSpec {
        category: CommandCategory::Chainable,
        name: "delta",
        min_args: 1,
        max_args: Some(1),
        options: OUTPUT,
        help: "delta <column>",
    },
    CommandSpec {
        category: CommandCategory::Chainable,
        name: "extract",
        min_args: 2,
        max_args: Some(2),
        options: NONE,
        help: "extract <column> <regex>",
    },
    CommandSpec {
        category: CommandCategory::Chainable,
        name: "flatten",
        min_args: 0,
        max_args: Some(0),
        options: NONE,
        help: "flatten",
    },
    CommandSpec {
        category: CommandCategory::Chainable,
        name: "parse-size",
        min_args: 1,
        max_args: Some(1),
        options: NONE,
        help: "parse-size <column>",
    },
    CommandSpec {
        category: CommandCategory::Chainable,
        name: "isin",
        min_args: 2,
        max_args: Some(2),
        options: NONE,
        help: "isin <column> <values>",
    },
    CommandSpec {
        category: CommandCategory::Chainable,
        name: "contains",
        min_args: 2,
        max_args: Some(2),
        options: IGNORE,
        help: "contains <column> <pattern>",
    },
    CommandSpec {
        category: CommandCategory::Chainable,
        name: "sed",
        min_args: 2,
        max_args: Some(2),
        options: SED,
        help: "sed <pattern> <replacement>",
    },
    CommandSpec {
        category: CommandCategory::Chainable,
        name: "grep",
        min_args: 1,
        max_args: Some(1),
        options: GREP,
        help: "grep <pattern>",
    },
    CommandSpec {
        category: CommandCategory::Chainable,
        name: "head",
        min_args: 0,
        max_args: Some(1),
        options: NUMBER,
        help: "head [number]",
    },
    CommandSpec {
        category: CommandCategory::Chainable,
        name: "tail",
        min_args: 0,
        max_args: Some(1),
        options: NUMBER,
        help: "tail [number]",
    },
    CommandSpec {
        category: CommandCategory::Chainable,
        name: "sort",
        min_args: 1,
        max_args: Some(1),
        options: DESC,
        help: "sort <columns>",
    },
    CommandSpec {
        category: CommandCategory::Chainable,
        name: "count",
        min_args: 0,
        max_args: Some(1),
        options: NONE,
        help: "count [columns]",
    },
    CommandSpec {
        category: CommandCategory::Chainable,
        name: "uniq",
        min_args: 0,
        max_args: Some(0),
        options: NONE,
        help: "uniq",
    },
    CommandSpec {
        category: CommandCategory::Chainable,
        name: "changetz",
        min_args: 1,
        max_args: Some(1),
        options: TZ,
        help: "changetz <column> --from-tz <tz> --to-tz <tz>",
    },
    CommandSpec {
        category: CommandCategory::Chainable,
        name: "renamecol",
        min_args: 2,
        max_args: Some(2),
        options: NONE,
        help: "renamecol <old> <new>",
    },
    CommandSpec {
        category: CommandCategory::Chainable,
        name: "timeslice",
        min_args: 1,
        max_args: Some(1),
        options: TIMESLICE,
        help: "timeslice <column> [--start <time>] [--end <time>]",
    },
    CommandSpec {
        category: CommandCategory::Finalizer,
        name: "show",
        min_args: 0,
        max_args: Some(0),
        options: SHOW,
        help: "show",
    },
    CommandSpec {
        category: CommandCategory::Finalizer,
        name: "showtable",
        min_args: 0,
        max_args: Some(0),
        options: NONE,
        help: "showtable",
    },
    CommandSpec {
        category: CommandCategory::Finalizer,
        name: "headers",
        min_args: 0,
        max_args: Some(0),
        options: HEADERS,
        help: "headers [--plain]",
    },
    CommandSpec {
        category: CommandCategory::Finalizer,
        name: "stats",
        min_args: 0,
        max_args: Some(0),
        options: NONE,
        help: "stats",
    },
    CommandSpec {
        category: CommandCategory::Finalizer,
        name: "showquery",
        min_args: 0,
        max_args: Some(0),
        options: NONE,
        help: "showquery",
    },
    CommandSpec {
        category: CommandCategory::Finalizer,
        name: "dump",
        min_args: 0,
        max_args: Some(0),
        options: DUMP,
        help: "dump [options]",
    },
    CommandSpec {
        category: CommandCategory::Finalizer,
        name: "dumpcache",
        min_args: 0,
        max_args: Some(0),
        options: OUTPUT,
        help: "dumpcache [--output <file>]",
    },
    CommandSpec {
        category: CommandCategory::Finalizer,
        name: "partition",
        min_args: 1,
        max_args: Some(2),
        options: NONE,
        help: "partition <column> [directory]",
    },
    CommandSpec {
        category: CommandCategory::Finalizer,
        name: "calc",
        min_args: 1,
        max_args: Some(1),
        options: CALC,
        help: "calc <column> --sum|--avg|--min|--max|--median|--std",
    },
    CommandSpec {
        category: CommandCategory::Automation,
        name: "run",
        min_args: 1,
        max_args: None,
        options: RUN,
        help: "run <config> [files...] [options]",
    },
];
pub fn command_specs() -> &'static [CommandSpec] {
    SPECS
}

pub fn automation_record_command_names() -> impl Iterator<Item = &'static str> {
    SPECS
        .iter()
        .filter(|spec| {
            matches!(
                spec.category,
                CommandCategory::Initializer
                    | CommandCategory::Chainable
                    | CommandCategory::Finalizer
            )
        })
        .map(|spec| spec.name)
}

fn yaml_mapping(value: &serde_yml::Value) -> Result<&serde_yml::Mapping, QuiltError> {
    value.as_mapping().ok_or_else(|| {
        QuiltError::usage("Error: automation command arguments must be a YAML mapping")
    })
}

fn yaml_value<'a>(mapping: &'a serde_yml::Mapping, key: &str) -> Option<&'a serde_yml::Value> {
    mapping.get(serde_yml::Value::String(key.to_string()))
}

fn yaml_string(mapping: &serde_yml::Mapping, key: &str) -> Option<String> {
    yaml_value(mapping, key).and_then(|value| match value {
        serde_yml::Value::String(value) => Some(value.clone()),
        serde_yml::Value::Number(value) => Some(value.to_string()),
        _ => None,
    })
}

fn yaml_strings(mapping: &serde_yml::Mapping, key: &str) -> Option<Vec<String>> {
    yaml_value(mapping, key).and_then(|value| match value {
        serde_yml::Value::String(value) => Some(vec![value.clone()]),
        serde_yml::Value::Sequence(values) => Some(
            values
                .iter()
                .filter_map(|value| match value {
                    serde_yml::Value::String(value) => Some(value.clone()),
                    _ => None,
                })
                .collect(),
        ),
        _ => None,
    })
}

fn yaml_values(mapping: &serde_yml::Mapping, key: &str) -> Option<Vec<String>> {
    yaml_value(mapping, key).and_then(|value| match value {
        serde_yml::Value::String(value) => Some(vec![value.clone()]),
        serde_yml::Value::Sequence(values) => values
            .iter()
            .map(|value| match value {
                serde_yml::Value::String(value) => Some(value.clone()),
                serde_yml::Value::Number(value) => Some(value.to_string()),
                _ => None,
            })
            .collect(),
        _ => None,
    })
}

fn push_yaml_option(tokens: &mut Vec<String>, mapping: &serde_yml::Mapping, key: &str) {
    if let Some(value) = yaml_string(mapping, key) {
        tokens.push(format!("--{key}"));
        tokens.push(value);
    }
}

fn push_yaml_flag(tokens: &mut Vec<String>, mapping: &serde_yml::Mapping, key: &str) {
    if let Some(value) = yaml_value(mapping, key).and_then(serde_yml::Value::as_bool) {
        if value {
            tokens.push(format!("--{key}"));
        } else if key == "strict" {
            tokens.push(format!("--{key}=false"));
        }
    }
}

fn validate_automation_keys(name: &str, mapping: &serde_yml::Mapping) -> Result<(), QuiltError> {
    let allowed: &[&str] = match name {
        "load" => &[
            "paths",
            "separator",
            "low-memory",
            "no-headers",
            "chunk-size",
            "infer-schema-length",
        ],
        "select" | "sort" => &["columns", "desc"],
        "count" => &["columns"],
        "cast" => &[
            "column",
            "type",
            "strict",
            "input-format",
            "epoch-unit",
            "timezone",
            "ambiguous",
            "nonexistent",
        ],
        "bucket" => &[
            "column",
            "interval",
            "output",
            "strict",
            "input-format",
            "epoch-unit",
            "timezone",
            "ambiguous",
            "nonexistent",
        ],
        "delta" => &["column", "output"],
        "extract" => &["column", "pattern"],
        "flatten" | "uniq" | "stats" | "showtable" | "showquery" => &[],
        "parse-size" => &["column"],
        "isin" => &["column", "values"],
        "contains" => &["column", "pattern", "ignore-case"],
        "sed" => &["pattern", "replacement", "column", "ignore-case"],
        "grep" => &["pattern", "ignore-case", "invert-match", "columns"],
        "head" | "tail" => &["number"],
        "changetz" => &[
            "column",
            "from-tz",
            "to-tz",
            "input-format",
            "output-format",
            "ambiguous",
            "nonexistent",
            "strict",
            "epoch-unit",
        ],
        "renamecol" => &["old", "new"],
        "timeslice" => &[
            "column",
            "start",
            "end",
            "strict",
            "input-format",
            "epoch-unit",
            "timezone",
            "ambiguous",
            "nonexistent",
        ],
        "show" => &["debug"],
        "headers" => &["plain"],
        "dump" => &["output", "separator"],
        "dumpcache" => &["output"],
        "partition" => &["column", "output-dir"],
        "calc" => &["column", "sum", "avg", "min", "max", "median", "std"],
        "run" => &[],
        other => {
            return Err(QuiltError::usage(format!(
                "Error: Unknown automation command '{other}'"
            )))
        }
    };
    for key in mapping.keys() {
        let key = key.as_str().unwrap_or_default();
        if !allowed.contains(&key) {
            return Err(QuiltError::usage(format!(
                "Error: Unknown key '{key}' for automation command '{name}'"
            )));
        }
    }
    validate_automation_value_types(name, mapping)?;
    Ok(())
}

fn validate_automation_value_types(
    name: &str,
    mapping: &serde_yml::Mapping,
) -> Result<(), QuiltError> {
    let string_fields: &[&str] = match name {
        "load" => &["separator"],
        "cast" => &[
            "column",
            "type",
            "input-format",
            "epoch-unit",
            "timezone",
            "ambiguous",
            "nonexistent",
        ],
        "bucket" => &[
            "column",
            "interval",
            "output",
            "input-format",
            "epoch-unit",
            "timezone",
            "ambiguous",
            "nonexistent",
        ],
        "delta" | "extract" | "parse-size" | "isin" | "contains" | "partition" => &["column"],
        "changetz" => &[
            "column",
            "from-tz",
            "to-tz",
            "input-format",
            "output-format",
            "ambiguous",
            "nonexistent",
            "epoch-unit",
            "timezone",
        ],
        "timeslice" => &[
            "column",
            "start",
            "end",
            "input-format",
            "epoch-unit",
            "timezone",
            "ambiguous",
            "nonexistent",
        ],
        "sed" => &["pattern", "replacement", "column"],
        "grep" => &["pattern"],
        "renamecol" => &["old", "new"],
        "dump" => &["output", "separator"],
        "dumpcache" => &["output"],
        _ => &[],
    };
    for key in string_fields {
        if let Some(value) = yaml_value(mapping, key) {
            if !value.is_string() {
                return Err(QuiltError::usage(format!(
                    "Error: key '{key}' for automation command '{name}' must be a string"
                )));
            }
        }
    }
    let list_fields: &[&str] = match name {
        "load" => &["paths"],
        "select" | "sort" | "count" => &["columns"],
        "grep" => &["columns"],
        _ => &[],
    };
    for key in list_fields {
        if let Some(value) = yaml_value(mapping, key) {
            let valid = match value {
                serde_yml::Value::String(_) => true,
                serde_yml::Value::Sequence(values) => {
                    values.iter().all(serde_yml::Value::is_string)
                }
                _ => false,
            };
            if !valid {
                return Err(QuiltError::usage(format!(
                    "Error: key '{key}' for automation command '{name}' must be a string or sequence of strings"
                )));
            }
        }
    }
    for key in [
        "low-memory",
        "no-headers",
        "strict",
        "ignore-case",
        "invert-match",
        "desc",
        "debug",
        "plain",
    ] {
        if let Some(value) = yaml_value(mapping, key) {
            if !value.is_bool() {
                return Err(QuiltError::usage(format!(
                    "Error: flag '{key}' for automation command '{name}' must be boolean"
                )));
            }
        }
    }
    Ok(())
}

/// Parse one canonical run-document record-processing step into the shared typed command model.
pub fn parse_automation_step(
    name: &str,
    value: &serde_yml::Value,
) -> Result<TypedCommand, QuiltError> {
    let empty = serde_yml::Mapping::new();
    let mapping = if value.is_null() {
        &empty
    } else {
        yaml_mapping(value)?
    };
    validate_automation_keys(name, mapping)?;
    let mut tokens = vec![name.to_string()];
    match name {
        "load" => {
            if let Some(paths) =
                yaml_strings(mapping, "paths").or_else(|| yaml_strings(mapping, "path"))
            {
                tokens.extend(paths);
            } else {
                // A run stage may defer its input paths to the CLI positional
                // arguments. The execution layer replaces this empty input
                // before dispatch; use a sentinel here so shared CLI parsing
                // can still validate the remaining options.
                tokens.push("__run_input__".to_string());
            }
            push_yaml_option(&mut tokens, mapping, "separator");
            push_yaml_flag(&mut tokens, mapping, "low-memory");
            push_yaml_flag(&mut tokens, mapping, "no-headers");
            push_yaml_option(&mut tokens, mapping, "chunk-size");
            push_yaml_option(&mut tokens, mapping, "infer-schema-length");
        }
        "select" | "sort" | "count" => {
            if let Some(columns) = yaml_strings(mapping, "columns") {
                tokens.push(columns.join(","));
            }
            if name == "sort" {
                push_yaml_flag(&mut tokens, mapping, "desc");
            }
        }
        "cast" => {
            tokens.extend([
                yaml_string(mapping, "column")
                    .ok_or_else(|| QuiltError::usage("Error: cast requires a column"))?,
                yaml_string(mapping, "type")
                    .ok_or_else(|| QuiltError::usage("Error: cast requires a type"))?,
            ]);
            for key in [
                "strict",
                "input-format",
                "epoch-unit",
                "timezone",
                "ambiguous",
                "nonexistent",
            ] {
                if key == "strict" {
                    push_yaml_flag(&mut tokens, mapping, key);
                } else {
                    push_yaml_option(&mut tokens, mapping, key);
                }
            }
        }
        "bucket" => {
            tokens.extend([
                yaml_string(mapping, "column")
                    .ok_or_else(|| QuiltError::usage("Error: bucket requires a column"))?,
                yaml_string(mapping, "interval")
                    .ok_or_else(|| QuiltError::usage("Error: bucket requires an interval"))?,
            ]);
            for key in [
                "strict",
                "input-format",
                "epoch-unit",
                "timezone",
                "ambiguous",
                "nonexistent",
            ] {
                if key == "strict" {
                    push_yaml_flag(&mut tokens, mapping, key);
                } else {
                    push_yaml_option(&mut tokens, mapping, key);
                }
            }
            push_yaml_option(&mut tokens, mapping, "output");
        }
        "delta" => {
            tokens.push(
                yaml_string(mapping, "column")
                    .ok_or_else(|| QuiltError::usage("Error: delta requires a column"))?,
            );
            push_yaml_option(&mut tokens, mapping, "output");
        }
        "extract" => {
            tokens.extend([
                yaml_string(mapping, "column")
                    .ok_or_else(|| QuiltError::usage("Error: extract requires a column"))?,
                yaml_string(mapping, "pattern")
                    .ok_or_else(|| QuiltError::usage("Error: extract requires a pattern"))?,
            ]);
        }
        "flatten" | "uniq" | "stats" | "showtable" | "showquery" => {}
        "parse-size" => tokens.push(
            yaml_string(mapping, "column")
                .ok_or_else(|| QuiltError::usage("Error: parse-size requires a column"))?,
        ),
        "isin" => {
            tokens.extend([
                yaml_string(mapping, "column")
                    .ok_or_else(|| QuiltError::usage("Error: isin requires a column"))?,
                yaml_values(mapping, "values")
                    .ok_or_else(|| QuiltError::usage("Error: isin requires values"))?
                    .join(","),
            ]);
        }
        "contains" => {
            tokens.extend([
                yaml_string(mapping, "column")
                    .ok_or_else(|| QuiltError::usage("Error: contains requires a column"))?,
                yaml_string(mapping, "pattern")
                    .ok_or_else(|| QuiltError::usage("Error: contains requires a pattern"))?,
            ]);
            push_yaml_flag(&mut tokens, mapping, "ignore-case");
        }
        "sed" => {
            tokens.extend([
                yaml_string(mapping, "pattern")
                    .ok_or_else(|| QuiltError::usage("Error: sed requires a pattern"))?,
                yaml_string(mapping, "replacement")
                    .ok_or_else(|| QuiltError::usage("Error: sed requires a replacement"))?,
            ]);
            push_yaml_option(&mut tokens, mapping, "column");
            push_yaml_flag(&mut tokens, mapping, "ignore-case");
        }
        "grep" => {
            tokens.push(
                yaml_string(mapping, "pattern")
                    .ok_or_else(|| QuiltError::usage("Error: grep requires a pattern"))?,
            );
            push_yaml_flag(&mut tokens, mapping, "ignore-case");
            push_yaml_flag(&mut tokens, mapping, "invert-match");
            if let Some(columns) = yaml_strings(mapping, "columns") {
                tokens.extend(["--column".to_string(), columns.join(",")]);
            }
        }
        "head" | "tail" => {
            push_yaml_option(&mut tokens, mapping, "number");
        }
        "changetz" => {
            tokens.push(
                yaml_string(mapping, "column")
                    .ok_or_else(|| QuiltError::usage("Error: changetz requires a column"))?,
            );
            for key in [
                "from-tz",
                "to-tz",
                "input-format",
                "output-format",
                "ambiguous",
                "nonexistent",
                "strict",
                "epoch-unit",
            ] {
                if key == "strict" {
                    push_yaml_flag(&mut tokens, mapping, key);
                } else {
                    push_yaml_option(&mut tokens, mapping, key);
                }
            }
        }
        "renamecol" => {
            tokens.extend([
                yaml_string(mapping, "old")
                    .ok_or_else(|| QuiltError::usage("Error: renamecol requires an old column"))?,
                yaml_string(mapping, "new")
                    .ok_or_else(|| QuiltError::usage("Error: renamecol requires a new column"))?,
            ]);
        }
        "timeslice" => {
            tokens.push(
                yaml_string(mapping, "column")
                    .ok_or_else(|| QuiltError::usage("Error: timeslice requires a column"))?,
            );
            push_yaml_option(&mut tokens, mapping, "start");
            push_yaml_option(&mut tokens, mapping, "end");
            for key in [
                "strict",
                "input-format",
                "epoch-unit",
                "timezone",
                "ambiguous",
                "nonexistent",
            ] {
                if key == "strict" {
                    push_yaml_flag(&mut tokens, mapping, key);
                } else {
                    push_yaml_option(&mut tokens, mapping, key);
                }
            }
            if yaml_value(mapping, "start").is_none() && yaml_value(mapping, "end").is_none() {
                return Err(QuiltError::usage(
                    "Error: timeslice in run requires at least one of 'start' or 'end'",
                ));
            }
        }
        "show" => {
            push_yaml_flag(&mut tokens, mapping, "debug");
        }
        "headers" => push_yaml_flag(&mut tokens, mapping, "plain"),
        "dump" => {
            if let Some(output) = yaml_string(mapping, "output") {
                tokens.push(format!("--output={output}"));
            }
            push_yaml_option(&mut tokens, mapping, "separator");
        }
        "dumpcache" => push_yaml_option(&mut tokens, mapping, "output"),
        "partition" => {
            tokens.push(
                yaml_string(mapping, "column")
                    .ok_or_else(|| QuiltError::usage("Error: partition requires a column"))?,
            );
            if let Some(output_dir) = yaml_string(mapping, "output-dir") {
                tokens.push(output_dir);
            }
        }
        "calc" => {
            tokens.push(
                yaml_string(mapping, "column")
                    .ok_or_else(|| QuiltError::usage("Error: calc requires a column"))?,
            );
            for mode in ["sum", "avg", "min", "max", "median", "std"] {
                push_yaml_flag(&mut tokens, mapping, mode);
            }
        }
        "run" => {
            return Err(QuiltError::usage(
                "Error: nested run steps are not record-processing commands",
            ));
        }
        unknown => {
            return Err(QuiltError::usage(format!(
                "Error: Unknown automation command '{unknown}'"
            )));
        }
    }
    let commands = parse_typed_commands(&tokens)?;
    commands
        .into_iter()
        .next()
        .ok_or_else(|| QuiltError::operation("automation parser", "typed step produced no command"))
}

#[derive(Default)]
struct Raw {
    name: &'static str,
    args: Vec<String>,
    options: HashMap<String, Option<String>>,
    repeated: HashMap<String, Vec<String>>,
}
fn spec(name: &str) -> Option<&'static CommandSpec> {
    SPECS.iter().find(|s| s.name == name)
}
fn opt(s: &CommandSpec, key: &str) -> Option<&'static OptionSpec> {
    s.options
        .iter()
        .find(|o| o.name == key || o.short == Some(key))
}

pub fn parse_typed_commands(argv: &[String]) -> Result<Vec<TypedCommand>, QuiltError> {
    let mut raws = Vec::new();
    let mut current: Option<Raw> = None;
    let mut current_spec: Option<&CommandSpec> = None;
    let mut positional_only = false;
    let mut i = 0;
    while i < argv.len() {
        let token = &argv[i];
        // `-` separates pipeline commands, except while `--` is consuming
        // positional values. Once a command has reached its declared maximum
        // arity, a following `-` is unambiguously the pipeline separator;
        // this keeps `grep -- -` available as a literal pattern.
        let separator_after_positional = positional_only
            && token == "-"
            && current_spec.is_some_and(|spec| {
                current
                    .as_ref()
                    .is_some_and(|raw| spec.max_args.is_some_and(|max| raw.args.len() >= max))
            });
        if token == "-" && (!positional_only || separator_after_positional) {
            if let Some(raw) = current.take() {
                raws.push(raw);
            }
            current_spec = None;
            positional_only = false;
            i += 1;
            continue;
        }
        if current.is_none() {
            let s = spec(token)
                .ok_or_else(|| QuiltError::usage(format!("Error: Unknown command '{token}'")))?;
            current_spec = Some(s);
            current = Some(Raw {
                name: s.name,
                ..Default::default()
            });
            i += 1;
            continue;
        }
        if token == "--" && !positional_only {
            positional_only = true;
            i += 1;
            continue;
        }
        let s =
            current_spec.ok_or_else(|| QuiltError::operation("parser", "missing command spec"))?;
        let raw = current
            .as_mut()
            .ok_or_else(|| QuiltError::operation("parser", "missing command"))?;
        if !positional_only {
            if let Some(flag) = token
                .strip_prefix("--")
                .or_else(|| token.strip_prefix('-').filter(|x| !x.is_empty()))
            {
                let (key, inline) = flag
                    .split_once('=')
                    .map_or((flag, None), |(k, v)| (k, Some(v)));
                if let Some(o) = opt(s, key) {
                    let value = if o.takes_value {
                        if let Some(v) = inline {
                            v.to_string()
                        } else {
                            i += 1;
                            argv.get(i)
                                .ok_or_else(|| {
                                    QuiltError::usage(format!(
                                        "Error: Option '--{key}' for '{}' requires a value",
                                        s.name
                                    ))
                                })?
                                .clone()
                        }
                    } else {
                        if let Some(value) = inline {
                            if key != "strict" {
                                return Err(QuiltError::usage(format!(
                                    "Error: Flag '--{key}' do not accept values"
                                )));
                            }
                            if !matches!(value.to_ascii_lowercase().as_str(), "true" | "false") {
                                return Err(QuiltError::usage(format!(
                                    "Error: Flag '--{key}' accepts only true or false"
                                )));
                            }
                            value.to_string()
                        } else {
                            String::new()
                        }
                    };
                    let k = o.name.replace('-', "_");
                    if o.repeated {
                        raw.repeated.entry(k).or_default().push(value);
                    } else if raw
                        .options
                        .insert(
                            k,
                            if o.takes_value || inline.is_some() {
                                Some(value)
                            } else {
                                None
                            },
                        )
                        .is_some()
                    {
                        return Err(QuiltError::usage(format!(
                            "Error: Option '--{key}' may only be specified once"
                        )));
                    }
                    i += 1;
                    continue;
                }
                if token.parse::<f64>().is_err() {
                    return Err(QuiltError::usage(format!(
                        "Error: Unknown option '{token}' for command '{}'",
                        s.name
                    )));
                }
            }
        }
        raw.args.push(token.clone());
        i += 1;
    }
    if let Some(raw) = current {
        raws.push(raw);
    }
    if raws.is_empty() {
        return Err(QuiltError::usage("Error: No commands provided"));
    }
    raws.into_iter().map(build_typed).collect()
}

fn args(raw: &Raw, min: usize, max: Option<usize>, usage: &str) -> Result<(), QuiltError> {
    if raw.args.len() < min {
        return Err(QuiltError::usage(format!(
            "Error: requires more arguments. Usage: {usage}"
        )));
    }
    if max.is_some_and(|m| raw.args.len() > m) {
        if max == Some(0) {
            return Err(QuiltError::usage(format!(
                "Error: accepts no arguments. Usage: {usage}"
            )));
        }
        return Err(QuiltError::usage(format!(
            "Error: accepts too many arguments. Usage: {usage}"
        )));
    }
    Ok(())
}
fn val(raw: &Raw, key: &str) -> Option<String> {
    raw.options.get(key).and_then(|v| v.clone())
}
fn flag(raw: &Raw, key: &str) -> bool {
    raw.options.contains_key(key)
}
fn bool_flag(raw: &Raw, key: &str) -> bool {
    raw.options
        .get(key)
        .map(|value| value.as_deref() != Some("false"))
        .unwrap_or(false)
}

fn datetime_config(raw: &Raw) -> Result<DateTimeConfig, QuiltError> {
    let ambiguous = val(raw, "ambiguous")
        .map(|value| parse_ambiguous_policy(&value).map_err(QuiltError::usage))
        .transpose()?;
    let nonexistent = val(raw, "nonexistent")
        .map(|value| parse_nonexistent_policy(&value).map_err(QuiltError::usage))
        .transpose()?;
    let epoch_unit = val(raw, "epoch_unit")
        .map(|value| parse_epoch_unit(&value).map_err(QuiltError::usage))
        .transpose()?;
    let timezone = val(raw, "timezone");
    if let Some(value) = &timezone {
        if value.parse::<chrono_tz::Tz>().is_err() {
            return Err(QuiltError::usage(format!("invalid timezone '{value}'")));
        }
    }
    Ok(DateTimeConfig {
        strict: bool_flag(raw, "strict"),
        input_format: val(raw, "input_format"),
        epoch_unit,
        timezone,
        ambiguous: ambiguous.unwrap_or(AmbiguousPolicy::Error),
        nonexistent: nonexistent.unwrap_or(NonexistentPolicy::Error),
        options_present: [
            "strict",
            "input_format",
            "epoch_unit",
            "timezone",
            "ambiguous",
            "nonexistent",
        ]
        .iter()
        .any(|key| raw.options.contains_key(*key)),
    })
}
fn split_values(value: &str) -> Result<Vec<String>, QuiltError> {
    let values = value
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(String::from)
        .collect::<Vec<_>>();
    if values.is_empty() {
        Err(QuiltError::usage(
            "Error: requires at least one non-empty value",
        ))
    } else {
        Ok(values)
    }
}
fn select_columns(raw: &Raw) -> Result<Vec<String>, QuiltError> {
    let input = raw.args.join(",");
    let re = Regex::new(
        r"^(?P<p1>[A-Za-z_][A-Za-z_0-9]*)(?P<n1>\d+)(?P<sep>[:-])(?:(?P<p2>[A-Za-z_][A-Za-z_0-9]*)(?P<n2>\d+)|(?P<n3>\d+))$",
    )
    .map_err(|e| QuiltError::operation("select parser", e.to_string()))?;
    let mut columns = Vec::new();
    for part in input.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if let Some(c) = re.captures(part) {
            let p1 = c.name("p1").map(|x| x.as_str()).unwrap_or("");
            if let Some(p2) = c.name("p2") {
                if p1 != p2.as_str() {
                    return Err(QuiltError::usage(format!(
                        "Error: Mismatched prefixes in range '{part}'."
                    )));
                }
            }
            let n1: usize = c
                .name("n1")
                .and_then(|x| x.as_str().parse().ok())
                .ok_or_else(|| QuiltError::usage("Error: Invalid range"))?;
            let n2: usize = c
                .name("n2")
                .or_else(|| c.name("n3"))
                .and_then(|x| x.as_str().parse().ok())
                .ok_or_else(|| QuiltError::usage("Error: Invalid range"))?;
            if n1 > n2 {
                return Err(QuiltError::usage(format!(
                    "Error: Invalid range '{part}'. Start number must be <= end number."
                )));
            }
            if c.name("sep").is_some_and(|sep| sep.as_str() == ":") {
                for number in n1..=n2 {
                    columns.push(format!("{p1}{number}"));
                }
                continue;
            }
        }
        columns.push(part.to_string());
    }
    Ok(columns)
}

fn build_typed(mut raw: Raw) -> Result<TypedCommand, QuiltError> {
    let a = &raw.args;
    match raw.name {
        "load" => {
            args(&raw, 1, None, "load [files...]")?;
            let separator = val(&raw, "separator").unwrap_or_else(|| ",".into());
            if separator.chars().count() != 1 || !separator.is_ascii() {
                return Err(QuiltError::usage(
                    "Error: Separator must be a single ASCII character",
                ));
            }
            Ok(TypedCommand::Load(LoadArgs {
                paths: a.iter().map(PathBuf::from).collect(),
                separator: separator.chars().next().unwrap_or(','),
                low_memory: flag(&raw, "low_memory"),
                no_headers: flag(&raw, "no_headers"),
                chunk_size: val(&raw, "chunk_size")
                    .map(|v| v.parse())
                    .transpose()
                    .map_err(|_| QuiltError::usage("Error: Invalid chunk size"))?,
                infer_schema_length: match val(&raw, "infer_schema_length") {
                    None => Some(
                        crate::operations::initializers::load::DEFAULT_NDJSON_INFER_SCHEMA_LENGTH,
                    ),
                    Some(value) if value.eq_ignore_ascii_case("full") => None,
                    Some(value) => {
                        let length = value.parse::<usize>().map_err(|_| {
                            QuiltError::usage(
                                "Error: NDJSON inference length must be a positive integer or 'full'",
                            )
                        })?;
                        if length == 0 {
                            return Err(QuiltError::usage(
                                "Error: NDJSON inference length must be positive or 'full'",
                            ));
                        }
                        Some(length)
                    }
                },
            }))
        }
        "select" => {
            args(&raw, 1, None, "select <columns>")?;
            Ok(TypedCommand::Select(SelectArgs {
                columns: select_columns(&raw)?,
            }))
        }
        "cast" => {
            args(&raw, 2, Some(2), "cast <column> <type>")?;
            let datetime = datetime_config(&raw)?;
            if !a[1].eq_ignore_ascii_case("datetime") && datetime.options_present {
                return Err(QuiltError::usage(
                    "Error: datetime parsing options apply only to datetime casts",
                ));
            }
            Ok(TypedCommand::Cast(CastArgs {
                column: a[0].clone(),
                target: a[1].clone(),
                datetime,
            }))
        }
        "bucket" => {
            args(&raw, 2, Some(2), "bucket <column> <interval>")?;
            Ok(TypedCommand::Bucket(BucketArgs {
                column: a[0].clone(),
                interval: a[1].clone(),
                output: val(&raw, "output"),
                datetime: datetime_config(&raw)?,
            }))
        }
        "delta" => {
            args(&raw, 1, Some(1), "delta <column>")?;
            Ok(TypedCommand::Delta(ColumnOutputArgs {
                column: a[0].clone(),
                output: val(&raw, "output"),
            }))
        }
        "extract" => {
            args(&raw, 2, Some(2), "extract <column> <regex>")?;
            Ok(TypedCommand::Extract(ExtractArgs {
                column: a[0].clone(),
                pattern: a[1].clone(),
            }))
        }
        "flatten" => {
            args(&raw, 0, Some(0), "flatten")?;
            Ok(TypedCommand::Flatten)
        }
        "parse-size" => {
            args(&raw, 1, Some(1), "parse-size <column>")?;
            Ok(TypedCommand::ParseSize {
                column: a[0].clone(),
            })
        }
        "isin" => {
            args(&raw, 2, Some(2), "isin <column> <values>")?;
            Ok(TypedCommand::Isin(IsinArgs {
                column: a[0].clone(),
                values: split_values(&a[1])?,
            }))
        }
        "contains" => {
            args(&raw, 2, Some(2), "contains <column> <pattern>")?;
            Ok(TypedCommand::Contains(ContainsArgs {
                column: a[0].clone(),
                pattern: a[1].clone(),
                ignore_case: flag(&raw, "ignore_case"),
            }))
        }
        "sed" => {
            args(&raw, 2, Some(2), "sed <pattern> <replacement>")?;
            Ok(TypedCommand::Sed(SedArgs {
                pattern: a[0].clone(),
                replacement: a[1].clone(),
                column: val(&raw, "column"),
                ignore_case: flag(&raw, "ignore_case"),
            }))
        }
        "grep" => {
            args(&raw, 1, Some(1), "grep <pattern>")?;
            Ok(TypedCommand::Grep(GrepArgs {
                pattern: a[0].clone(),
                ignore_case: flag(&raw, "ignore_case"),
                invert_match: flag(&raw, "invert_match"),
                columns: val(&raw, "column").map(|v| split_values(&v)).transpose()?,
            }))
        }
        "head" => Ok(TypedCommand::Head(NumberArgs {
            number: val(&raw, "number")
                .or_else(|| a.first().cloned())
                .map(|v| v.parse())
                .transpose()
                .map_err(|_| QuiltError::usage("Error: head requires a valid number"))?
                .unwrap_or(5),
        })),
        "tail" => Ok(TypedCommand::Tail(NumberArgs {
            number: val(&raw, "number")
                .or_else(|| a.first().cloned())
                .map(|v| v.parse())
                .transpose()
                .map_err(|_| QuiltError::usage("Error: tail requires a valid number"))?
                .unwrap_or(5),
        })),
        "sort" => {
            args(&raw, 1, Some(1), "sort <columns>")?;
            Ok(TypedCommand::Sort(SortArgs {
                columns: split_values(&a[0])?,
                descending: flag(&raw, "desc"),
            }))
        }
        "count" => Ok(TypedCommand::Count(CountArgs {
            columns: a
                .iter()
                .flat_map(|v| v.split(','))
                .filter(|v| !v.trim().is_empty())
                .map(|v| v.trim().into())
                .collect(),
        })),
        "uniq" => {
            args(&raw, 0, Some(0), "uniq")?;
            Ok(TypedCommand::Uniq)
        }
        "changetz" => {
            args(
                &raw,
                1,
                Some(1),
                "changetz <column> --from-tz <tz> --to-tz <tz>",
            )?;
            let datetime = datetime_config(&raw)?;
            let from_tz = val(&raw, "from_tz")
                .ok_or_else(|| QuiltError::usage("Error: changetz requires --from-tz"))?;
            let to_tz = val(&raw, "to_tz")
                .ok_or_else(|| QuiltError::usage("Error: changetz requires --to-tz"))?;
            if from_tz.parse::<chrono_tz::Tz>().is_err() && !from_tz.eq_ignore_ascii_case("local") {
                return Err(QuiltError::usage(format!(
                    "invalid source timezone '{from_tz}'"
                )));
            }
            if to_tz.parse::<chrono_tz::Tz>().is_err() {
                return Err(QuiltError::usage(format!(
                    "Invalid target timezone '{to_tz}' (invalid target timezone)"
                )));
            }
            Ok(TypedCommand::ChangeTz(ChangeTzArgs {
                column: a[0].clone(),
                from_tz,
                to_tz,
                input_format: val(&raw, "input_format"),
                output_format: val(&raw, "output_format"),
                ambiguous: datetime.ambiguous,
                nonexistent: datetime.nonexistent,
                strict: bool_flag(&raw, "strict"),
                epoch_unit: datetime.epoch_unit,
                timezone: datetime.timezone,
                options_present: datetime.options_present,
            }))
        }
        "renamecol" => {
            args(&raw, 2, Some(2), "renamecol <old> <new>")?;
            Ok(TypedCommand::RenameCol {
                old: a[0].clone(),
                new: a[1].clone(),
            })
        }
        "timeslice" => {
            args(&raw, 1, Some(1), "timeslice <column> [--start] [--end]")?;
            let start = val(&raw, "start");
            let end = val(&raw, "end");
            if start.is_none() && end.is_none() {
                return Err(QuiltError::usage(
                    "Error: timeslice requires at least one of --start or --end",
                ));
            }
            Ok(TypedCommand::TimeSlice(TimeSliceArgs {
                column: a[0].clone(),
                start,
                end,
                datetime: datetime_config(&raw)?,
            }))
        }
        "show" => {
            args(&raw, 0, Some(0), "show")?;
            Ok(TypedCommand::Show {
                debug: flag(&raw, "debug"),
            })
        }
        "showtable" => {
            args(&raw, 0, Some(0), "showtable")?;
            Ok(TypedCommand::ShowTable)
        }
        "headers" => {
            args(&raw, 0, Some(0), "headers")?;
            Ok(TypedCommand::Headers {
                plain: flag(&raw, "plain"),
            })
        }
        "stats" => {
            args(&raw, 0, Some(0), "stats")?;
            Ok(TypedCommand::Stats)
        }
        "showquery" => {
            args(&raw, 0, Some(0), "showquery")?;
            Ok(TypedCommand::ShowQuery)
        }
        "dump" => {
            args(&raw, 0, Some(0), "dump")?;
            let separator = val(&raw, "separator").unwrap_or_else(|| ",".into());
            if separator.chars().count() != 1 || !separator.is_ascii() {
                return Err(QuiltError::usage(
                    "Error: Separator must be a single ASCII character",
                ));
            }
            Ok(TypedCommand::Dump(DumpArgs {
                output: val(&raw, "output"),
                separator: separator.chars().next().unwrap_or(','),
            }))
        }
        "dumpcache" => {
            args(&raw, 0, Some(0), "dumpcache")?;
            Ok(TypedCommand::DumpCache {
                output: val(&raw, "output"),
            })
        }
        "partition" => {
            args(&raw, 1, Some(2), "partition <column> [directory]")?;
            Ok(TypedCommand::Partition(PartitionArgs {
                column: a[0].clone(),
                output_dir: a.get(1).cloned().unwrap_or_else(|| "./partitions".into()),
            }))
        }
        "calc" => {
            if raw.args.len() > 1 {
                return Err(QuiltError::usage("Error: calc accepts exactly one column"));
            }
            args(
                &raw,
                1,
                Some(1),
                "calc <column> --sum|--avg|--min|--max|--median|--std",
            )?;
            let modes = [
                ("sum", Aggregation::Sum),
                ("avg", Aggregation::Avg),
                ("min", Aggregation::Min),
                ("max", Aggregation::Max),
                ("median", Aggregation::Median),
                ("std", Aggregation::Std),
            ];
            let selected = modes
                .iter()
                .filter(|(name, _)| flag(&raw, name))
                .map(|(_, mode)| mode.clone())
                .collect::<Vec<_>>();
            if selected.len() != 1 {
                return Err(QuiltError::usage(
                    "Error: calc requires exactly one aggregation flag",
                ));
            }
            Ok(TypedCommand::Calc(CalcArgs {
                column: a[0].clone(),
                aggregation: selected[0].clone(),
            }))
        }
        "run" => {
            args(&raw, 1, None, "run <config> [files...]")?;
            Ok(TypedCommand::Run(RunArgs {
                config: PathBuf::from(&a[0]),
                input_files: a[1..].iter().map(PathBuf::from).collect(),
                output: val(&raw, "output"),
                vars: raw.repeated.remove("var").unwrap_or_default(),
                check: flag(&raw, "check"),
                show_plan: val(&raw, "show_plan"),
            }))
        }
        _ => Err(QuiltError::operation("parser", "registry mismatch")),
    }
}

pub fn render_registry_help() -> String {
    let mut out = String::from("Commands:\n");
    for cat in [
        CommandCategory::Initializer,
        CommandCategory::Chainable,
        CommandCategory::Finalizer,
        CommandCategory::Automation,
    ] {
        out.push_str(&format!(
            "\n{}:\n",
            match cat {
                CommandCategory::Initializer => "Initializers",
                CommandCategory::Chainable => "Chainables",
                CommandCategory::Finalizer => "Finalizers",
                CommandCategory::Automation => "Automation",
            }
        ));
        for s in SPECS.iter().filter(|s| s.category == cat) {
            out.push_str(&format!("  {:12} {}\n", s.name, s.help));
        }
    }
    out
}
pub fn render_command_help(name: &str) -> Option<String> {
    let s = spec(name)?;
    if name == "run" {
        return Some(
            "run\n\nUsage: qlt run <config> [files...] [options]\n\n"
                .to_string()
                + "Execute a canonical version-1 YAML workflow. Paths in declared default values "
                + "are relative to the run file; --var path overrides are relative to the caller.\n"
                + "  --check                 Validate schema, parameters, graph, and commands without I/O\n"
                + "  --var name=value        Override a declared typed parameter (repeatable)\n"
                + "  --output, -o PATH       Override the final output destination\n"
                + "  --show-plan STAGE       Print a selected stage plan without evaluating rows\n"
                + "Parameter placeholders are whole YAML values: {\"$param\": name}; partial interpolation is rejected.\n",
        );
    }
    Some(format!("{}\n\nUsage: {}\n", s.name, s.help))
}
