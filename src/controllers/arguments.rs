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
    pub debug: bool,
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
