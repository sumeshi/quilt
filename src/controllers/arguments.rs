use crate::operations::datetime::{AmbiguousPolicy, DateTimeConfig, EpochUnit};
use std::path::PathBuf;

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
    pub infer_schema_length: Option<usize>,
    pub source_column: Option<String>,
    pub all_strings: bool,
    pub schema: Vec<(String, String)>,
    pub stdin_format: Option<String>,
    pub rejects: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Csv,
    Jsonl,
}
impl OutputFormat {
    pub fn parse(value: &str) -> Result<Self, crate::QuiltError> {
        match value {
            "csv" => Ok(Self::Csv),
            "jsonl" | "ndjson" => Ok(Self::Jsonl),
            _ => Err(crate::QuiltError::usage("format must be csv or jsonl")),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterArgs {
    pub expression: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeriveArgs {
    pub output: String,
    pub expression: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateArgs {
    pub metrics: Vec<crate::operations::chainables::aggregate::AggregateSpec>,
    pub by: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrayContainsArgs {
    pub column: String,
    pub value: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrayGetArgs {
    pub column: String,
    pub index: i64,
    pub output: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplodeArgs {
    pub columns: Vec<String>,
}

pub(crate) fn parse_metric(
    value: &str,
) -> Result<crate::operations::chainables::aggregate::AggregateSpec, crate::QuiltError> {
    use crate::operations::chainables::aggregate::{AggregateMode, AggregateSpec};
    let (mode, rest) = value.split_once(':').ok_or_else(|| {
        crate::QuiltError::usage("metric must be count:output or mode:column:output")
    })?;
    let mode = parse_aggregate_mode(mode)?;
    let (column, output) = match rest.rsplit_once(':') {
        Some((column, output)) if !column.is_empty() => (Some(column.to_string()), output),
        None if mode == AggregateMode::Count => (None, rest),
        _ => {
            return Err(crate::QuiltError::usage(
                "metric must be count:output or mode:column:output",
            ))
        }
    };
    if output.is_empty() {
        return Err(crate::QuiltError::usage(
            "metric output name must not be empty",
        ));
    }
    Ok(AggregateSpec::new(mode, column, output))
}

pub(crate) fn parse_aggregate_mode(
    value: &str,
) -> Result<crate::operations::chainables::aggregate::AggregateMode, crate::QuiltError> {
    use crate::operations::chainables::aggregate::AggregateMode;
    match value {
        "count" => Ok(AggregateMode::Count),
        "sum" => Ok(AggregateMode::Sum),
        "avg" => Ok(AggregateMode::Avg),
        "min" => Ok(AggregateMode::Min),
        "max" => Ok(AggregateMode::Max),
        "median" => Ok(AggregateMode::Median),
        "std" => Ok(AggregateMode::Std),
        "nunique" => Ok(AggregateMode::Nunique),
        s if s.starts_with('p') => Ok(AggregateMode::Percentile(
            s[1..]
                .parse::<u8>()
                .ok()
                .filter(|p| *p <= 100)
                .ok_or_else(|| crate::QuiltError::usage("percentile must be p0 through p100"))?,
        )),
        _ => Err(crate::QuiltError::usage("unknown aggregate mode")),
    }
}

pub(crate) fn parse_column_type(value: &str) -> Result<(String, String), crate::QuiltError> {
    let (name, dtype) = value.rsplit_once('=').ok_or_else(|| {
        crate::QuiltError::usage("schema must be column=string|int|uint|float|bool")
    })?;
    if name.is_empty() || !matches!(dtype, "string" | "int" | "uint" | "float" | "bool") {
        return Err(crate::QuiltError::usage(
            "schema must be column=string|int|uint|float|bool",
        ));
    }
    Ok((name.to_owned(), dtype.to_owned()))
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectArgs {
    pub columns: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UnitArgs;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseSizeArgs {
    pub column: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameColArgs {
    pub old: String,
    pub new: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShowArgs {
    pub format: OutputFormat,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadersArgs {
    pub plain: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpCacheArgs {
    pub output: Option<String>,
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
    pub by: Vec<String>,
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
pub struct TimeInfoArgs {
    pub column: String,
    pub by: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeTzArgs {
    pub column: String,
    pub from_tz: String,
    pub to_tz: String,
    pub input_format: Option<String>,
    pub output_format: Option<String>,
    pub ambiguous: AmbiguousPolicy,
    pub nonexistent: crate::operations::datetime::NonexistentPolicy,
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
    pub format: OutputFormat,
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
    pub stage: Option<String>,
}
