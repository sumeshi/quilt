use crate::controllers::arguments::*;
use crate::controllers::definitions::{OperationId, TypedCommand};
use crate::error::QuiltError;
use crate::operations::datetime::{
    parse_ambiguous_policy, parse_epoch_unit, parse_nonexistent_policy, AmbiguousPolicy,
    DateTimeConfig, NonexistentPolicy,
};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum YamlStrings {
    One(String),
    Many(Vec<String>),
}
impl YamlStrings {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::One(v) => vec![v],
            Self::Many(v) => v,
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum YamlValues {
    One(serde_yml::Value),
    Many(Vec<serde_yml::Value>),
}
impl YamlValues {
    fn into_vec(self) -> Result<Vec<String>, QuiltError> {
        let values = match self {
            Self::One(serde_yml::Value::Sequence(v)) => v,
            Self::One(v) => vec![v],
            Self::Many(v) => v,
        };
        values
            .into_iter()
            .map(|v| match v {
                serde_yml::Value::String(s) => Ok(s),
                serde_yml::Value::Number(n) => Ok(n.to_string()),
                _ => Err(QuiltError::usage(
                    "Error: values must be strings or numbers",
                )),
            })
            .collect()
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YLoad {
    paths: Option<YamlStrings>,
    separator: Option<String>,
    #[serde(rename = "low-memory", default)]
    low_memory: bool,
    #[serde(rename = "no-headers", default)]
    no_headers: bool,
    #[serde(rename = "chunk-size")]
    chunk_size: Option<usize>,
    #[serde(rename = "infer-schema-length")]
    infer_schema_length: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YSelect {
    columns: YamlStrings,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YSort {
    columns: YamlStrings,
    #[serde(default)]
    desc: bool,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyYamlArgs {}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YCast {
    column: String,
    #[serde(rename = "type")]
    target: String,
    strict: Option<bool>,
    #[serde(rename = "input-format")]
    input_format: Option<String>,
    #[serde(rename = "epoch-unit")]
    epoch_unit: Option<String>,
    timezone: Option<String>,
    ambiguous: Option<String>,
    nonexistent: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YBucket {
    column: String,
    interval: String,
    output: Option<String>,
    strict: Option<bool>,
    #[serde(rename = "input-format")]
    input_format: Option<String>,
    #[serde(rename = "epoch-unit")]
    epoch_unit: Option<String>,
    timezone: Option<String>,
    ambiguous: Option<String>,
    nonexistent: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YDate {
    strict: Option<bool>,
    #[serde(rename = "input-format")]
    input_format: Option<String>,
    #[serde(rename = "epoch-unit")]
    epoch_unit: Option<String>,
    timezone: Option<String>,
    ambiguous: Option<String>,
    nonexistent: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YColumnOutput {
    column: String,
    output: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YExtract {
    column: String,
    pattern: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YIsin {
    column: String,
    values: YamlValues,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YContains {
    column: String,
    pattern: String,
    #[serde(rename = "ignore-case", default)]
    ignore_case: bool,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YSed {
    pattern: String,
    replacement: String,
    column: Option<String>,
    #[serde(rename = "ignore-case", default)]
    ignore_case: bool,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YGrep {
    pattern: String,
    #[serde(rename = "ignore-case", default)]
    ignore_case: bool,
    #[serde(rename = "invert-match", default)]
    invert_match: bool,
    columns: Option<YamlStrings>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YNumber {
    number: Option<usize>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YTz {
    column: String,
    #[serde(rename = "from-tz")]
    from_tz: String,
    #[serde(rename = "to-tz")]
    to_tz: String,
    #[serde(rename = "input-format")]
    input_format: Option<String>,
    #[serde(rename = "output-format")]
    output_format: Option<String>,
    strict: Option<bool>,
    #[serde(rename = "epoch-unit")]
    epoch_unit: Option<String>,
    timezone: Option<String>,
    ambiguous: Option<String>,
    nonexistent: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YRename {
    old: String,
    new: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YTimeSlice {
    column: String,
    start: Option<String>,
    end: Option<String>,
    strict: Option<bool>,
    #[serde(rename = "input-format")]
    input_format: Option<String>,
    #[serde(rename = "epoch-unit")]
    epoch_unit: Option<String>,
    timezone: Option<String>,
    ambiguous: Option<String>,
    nonexistent: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YShow {
    #[serde(default)]
    debug: bool,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YHeaders {
    #[serde(default)]
    plain: bool,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YDump {
    output: Option<String>,
    separator: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YDumpCache {
    output: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YPartition {
    column: String,
    #[serde(rename = "output-dir")]
    output_dir: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YCalc {
    column: String,
    #[serde(default)]
    sum: bool,
    #[serde(default)]
    avg: bool,
    #[serde(default)]
    min: bool,
    #[serde(default)]
    max: bool,
    #[serde(default)]
    median: bool,
    #[serde(default)]
    std: bool,
}

fn yaml_args<T: for<'de> Deserialize<'de>>(
    value: &serde_yml::Value,
    name: &str,
) -> Result<T, QuiltError> {
    let value = if value.is_null() {
        serde_yml::Value::Mapping(serde_yml::Mapping::new())
    } else {
        value.clone()
    };
    serde_yml::from_value(value).map_err(|e| {
        // Keep numeric chainable diagnostics identical across CLI and YAML
        // adapters.  The typed YAML DTO otherwise exposes serde's Rust type
        // name (`expected usize`), which is an implementation detail and
        // differs from the public CLI contract.
        if matches!(name, "head" | "tail") && e.to_string().contains("expected usize") {
            return QuiltError::usage(format!("Error: {name} requires a valid number"));
        }
        QuiltError::usage(format!("Error: invalid arguments for '{name}': {e}"))
    })
}

fn date_config(dt: YDate) -> Result<DateTimeConfig, QuiltError> {
    let options_present = dt.strict.is_some()
        || dt.input_format.is_some()
        || dt.epoch_unit.is_some()
        || dt.timezone.is_some()
        || dt.ambiguous.is_some()
        || dt.nonexistent.is_some();
    let ambiguous = dt
        .ambiguous
        .clone()
        .map(|v| parse_ambiguous_policy(&v).map_err(QuiltError::usage))
        .transpose()?;
    let nonexistent = dt
        .nonexistent
        .clone()
        .map(|v| parse_nonexistent_policy(&v).map_err(QuiltError::usage))
        .transpose()?;
    let epoch_unit = dt
        .epoch_unit
        .clone()
        .map(|v| parse_epoch_unit(&v).map_err(QuiltError::usage))
        .transpose()?;
    if let Some(tz) = &dt.timezone {
        if tz.parse::<chrono_tz::Tz>().is_err() {
            return Err(QuiltError::usage(format!("invalid timezone '{tz}'")));
        }
    }
    Ok(DateTimeConfig {
        strict: dt.strict.unwrap_or(false),
        input_format: dt.input_format,
        epoch_unit,
        timezone: dt.timezone,
        ambiguous: ambiguous.unwrap_or(AmbiguousPolicy::Error),
        nonexistent: nonexistent.unwrap_or(NonexistentPolicy::Error),
        options_present,
    })
}
fn separator(value: Option<String>) -> Result<char, QuiltError> {
    let value = value.unwrap_or_else(|| ",".into());
    if value.chars().count() != 1 || !value.is_ascii() {
        return Err(QuiltError::usage(
            "Error: Separator must be a single ASCII character",
        ));
    }
    Ok(value.chars().next().unwrap())
}

/// Parse one canonical run-document record-processing step directly into typed arguments.
pub fn parse_automation_step(
    name: &str,
    value: &serde_yml::Value,
) -> Result<TypedCommand, QuiltError> {
    let operation = OperationId::parse(name)
        .ok_or_else(|| QuiltError::usage(format!("Error: Unknown automation command '{name}'")))?;
    match operation {
        OperationId::Load => {
            let a: YLoad = yaml_args(value, name)?;
            let paths = a.paths.map(YamlStrings::into_vec).unwrap_or_default();
            let infer = match a.infer_schema_length.as_deref() {
                None => {
                    Some(crate::operations::initializers::load::DEFAULT_NDJSON_INFER_SCHEMA_LENGTH)
                }
                Some("full") => None,
                Some(v) => {
                    let n = v
                        .parse()
                        .map_err(|_| QuiltError::usage("Error: invalid infer-schema-length"))?;
                    if n == 0 {
                        return Err(QuiltError::usage(
                            "Error: infer-schema-length must be positive",
                        ));
                    }
                    Some(n)
                }
            };
            Ok(TypedCommand::Load(LoadArgs {
                paths: paths.into_iter().map(PathBuf::from).collect(),
                separator: separator(a.separator)?,
                low_memory: a.low_memory,
                no_headers: a.no_headers,
                chunk_size: a.chunk_size,
                infer_schema_length: infer,
            }))
        }
        OperationId::Select => {
            let a: YSelect = yaml_args(value, name)?;
            Ok(TypedCommand::Select(SelectArgs {
                columns: crate::controllers::cli_adapter::select_columns_from_input(
                    &a.columns.into_vec().join(","),
                )?,
            }))
        }
        OperationId::Sort => {
            let a: YSort = yaml_args(value, name)?;
            Ok(TypedCommand::Sort(SortArgs {
                columns: a.columns.into_vec(),
                descending: a.desc,
            }))
        }
        OperationId::Count => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct X {
                columns: Option<YamlStrings>,
            }
            let a: X = yaml_args(value, name)?;
            Ok(TypedCommand::Count(CountArgs {
                columns: a.columns.map(YamlStrings::into_vec).unwrap_or_default(),
            }))
        }
        OperationId::Cast => {
            let a: YCast = yaml_args(value, name)?;
            Ok(TypedCommand::Cast(CastArgs {
                column: a.column,
                target: a.target,
                datetime: date_config(YDate {
                    strict: a.strict,
                    input_format: a.input_format,
                    epoch_unit: a.epoch_unit,
                    timezone: a.timezone,
                    ambiguous: a.ambiguous,
                    nonexistent: a.nonexistent,
                })?,
            }))
        }
        OperationId::Bucket => {
            let a: YBucket = yaml_args(value, name)?;
            Ok(TypedCommand::Bucket(BucketArgs {
                column: a.column,
                interval: a.interval,
                output: a.output,
                datetime: date_config(YDate {
                    strict: a.strict,
                    input_format: a.input_format,
                    epoch_unit: a.epoch_unit,
                    timezone: a.timezone,
                    ambiguous: a.ambiguous,
                    nonexistent: a.nonexistent,
                })?,
            }))
        }
        OperationId::Delta => {
            let a: YColumnOutput = yaml_args(value, name)?;
            Ok(TypedCommand::Delta(ColumnOutputArgs {
                column: a.column,
                output: a.output,
            }))
        }
        OperationId::Extract => {
            let a: YExtract = yaml_args(value, name)?;
            Ok(TypedCommand::Extract(ExtractArgs {
                column: a.column,
                pattern: a.pattern,
            }))
        }
        OperationId::Flatten => {
            let _: EmptyYamlArgs = yaml_args(value, name)?;
            Ok(TypedCommand::Flatten(UnitArgs))
        }
        OperationId::Uniq => {
            let _: EmptyYamlArgs = yaml_args(value, name)?;
            Ok(TypedCommand::Uniq(UnitArgs))
        }
        OperationId::Stats => {
            let _: EmptyYamlArgs = yaml_args(value, name)?;
            Ok(TypedCommand::Stats(UnitArgs))
        }
        OperationId::ShowTable => {
            let _: EmptyYamlArgs = yaml_args(value, name)?;
            Ok(TypedCommand::ShowTable(UnitArgs))
        }
        OperationId::ShowQuery => {
            let _: EmptyYamlArgs = yaml_args(value, name)?;
            Ok(TypedCommand::ShowQuery(UnitArgs))
        }
        OperationId::ParseSize => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct X {
                column: String,
            }
            let a: X = yaml_args(value, name)?;
            Ok(TypedCommand::ParseSize(ParseSizeArgs { column: a.column }))
        }
        OperationId::Isin => {
            let a: YIsin = yaml_args(value, name)?;
            Ok(TypedCommand::Isin(IsinArgs {
                column: a.column,
                values: a.values.into_vec()?,
            }))
        }
        OperationId::Contains => {
            let a: YContains = yaml_args(value, name)?;
            Ok(TypedCommand::Contains(ContainsArgs {
                column: a.column,
                pattern: a.pattern,
                ignore_case: a.ignore_case,
            }))
        }
        OperationId::Sed => {
            let a: YSed = yaml_args(value, name)?;
            Ok(TypedCommand::Sed(SedArgs {
                pattern: a.pattern,
                replacement: a.replacement,
                column: a.column,
                ignore_case: a.ignore_case,
            }))
        }
        OperationId::Grep => {
            let a: YGrep = yaml_args(value, name)?;
            Ok(TypedCommand::Grep(GrepArgs {
                pattern: a.pattern,
                ignore_case: a.ignore_case,
                invert_match: a.invert_match,
                columns: a.columns.map(YamlStrings::into_vec),
            }))
        }
        OperationId::Head => {
            let a: YNumber = yaml_args(value, name)?;
            Ok(TypedCommand::Head(NumberArgs {
                number: a.number.unwrap_or(5),
            }))
        }
        OperationId::Tail => {
            let a: YNumber = yaml_args(value, name)?;
            Ok(TypedCommand::Tail(NumberArgs {
                number: a.number.unwrap_or(5),
            }))
        }
        OperationId::ChangeTz => {
            let a: YTz = yaml_args(value, name)?;
            let dt = date_config(YDate {
                strict: a.strict,
                input_format: a.input_format.clone(),
                epoch_unit: a.epoch_unit,
                timezone: a.timezone,
                ambiguous: a.ambiguous,
                nonexistent: a.nonexistent,
            })?;
            Ok(TypedCommand::ChangeTz(ChangeTzArgs {
                column: a.column,
                from_tz: a.from_tz,
                to_tz: a.to_tz,
                input_format: a.input_format,
                output_format: a.output_format,
                ambiguous: dt.ambiguous,
                nonexistent: dt.nonexistent,
                strict: dt.strict,
                epoch_unit: dt.epoch_unit,
                timezone: dt.timezone,
                options_present: dt.options_present,
            }))
        }
        OperationId::RenameCol => {
            let a: YRename = yaml_args(value, name)?;
            Ok(TypedCommand::RenameCol(RenameColArgs {
                old: a.old,
                new: a.new,
            }))
        }
        OperationId::TimeSlice => {
            let a: YTimeSlice = yaml_args(value, name)?;
            if a.start.is_none() && a.end.is_none() {
                return Err(QuiltError::usage(
                    "Error: timeslice in run requires at least one of 'start' or 'end'",
                ));
            }
            Ok(TypedCommand::TimeSlice(TimeSliceArgs {
                column: a.column,
                start: a.start,
                end: a.end,
                datetime: date_config(YDate {
                    strict: a.strict,
                    input_format: a.input_format,
                    epoch_unit: a.epoch_unit,
                    timezone: a.timezone,
                    ambiguous: a.ambiguous,
                    nonexistent: a.nonexistent,
                })?,
            }))
        }
        OperationId::Show => {
            let a: YShow = yaml_args(value, name)?;
            Ok(TypedCommand::Show(ShowArgs { debug: a.debug }))
        }
        OperationId::Headers => {
            let a: YHeaders = yaml_args(value, name)?;
            Ok(TypedCommand::Headers(HeadersArgs { plain: a.plain }))
        }
        OperationId::Dump => {
            let a: YDump = yaml_args(value, name)?;
            Ok(TypedCommand::Dump(DumpArgs {
                output: a.output,
                separator: separator(a.separator)?,
            }))
        }
        OperationId::DumpCache => {
            let a: YDumpCache = yaml_args(value, name)?;
            Ok(TypedCommand::DumpCache(DumpCacheArgs { output: a.output }))
        }
        OperationId::Partition => {
            let a: YPartition = yaml_args(value, name)?;
            Ok(TypedCommand::Partition(PartitionArgs {
                column: a.column,
                output_dir: a.output_dir.unwrap_or_else(|| "./partitions".into()),
            }))
        }
        OperationId::Calc => {
            let a: YCalc = yaml_args(value, name)?;
            let selected = [
                (a.sum, Aggregation::Sum),
                (a.avg, Aggregation::Avg),
                (a.min, Aggregation::Min),
                (a.max, Aggregation::Max),
                (a.median, Aggregation::Median),
                (a.std, Aggregation::Std),
            ]
            .into_iter()
            .filter(|x| x.0)
            .map(|x| x.1)
            .collect::<Vec<_>>();
            if selected.len() != 1 {
                return Err(QuiltError::usage(
                    "Error: calc requires exactly one aggregation flag",
                ));
            }
            Ok(TypedCommand::Calc(CalcArgs {
                column: a.column,
                aggregation: selected[0].clone(),
            }))
        }
        OperationId::Run => Err(QuiltError::usage(
            "Error: nested run steps are not record-processing commands",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(source: &str) -> serde_yml::Value {
        serde_yml::from_str(source).expect("test YAML should parse")
    }

    #[test]
    fn empty_commands_reject_unknown_fields() {
        for name in ["flatten", "uniq", "stats", "showtable", "showquery"] {
            assert!(
                parse_automation_step(name, &yaml("unexpected: true")).is_err(),
                "{name} accepted an unknown field"
            );
        }
    }

    #[test]
    fn select_rejects_sort_only_fields() {
        assert!(parse_automation_step("select", &yaml("columns: [id]\ndesc: true")).is_err());
        assert!(parse_automation_step("sort", &yaml("columns: [id]\ndesc: true")).is_ok());
    }

    #[test]
    fn load_accepts_only_canonical_paths_field() {
        assert!(parse_automation_step("load", &yaml("paths: [input.csv]")).is_ok());
        assert!(parse_automation_step("load", &yaml("path: input.csv")).is_err());
    }

    #[test]
    fn datetime_commands_reject_unknown_fields() {
        for (name, source) in [
            ("cast", "column: value\ntype: int\nunknown: true"),
            ("bucket", "column: when\ninterval: 1d\nunknown: true"),
            ("timeslice", "column: when\nstart: '00:00'\nunknown: true"),
            (
                "changetz",
                "column: when\nfrom-tz: UTC\nto-tz: UTC\nunknown: true",
            ),
        ] {
            assert!(
                parse_automation_step(name, &yaml(source)).is_err(),
                "{name} accepted an unknown datetime field"
            );
        }
    }

    #[test]
    fn numeric_yaml_diagnostics_match_cli_contract() {
        for name in ["head", "tail"] {
            let error = parse_automation_step(name, &yaml("number: nope")).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("Error: {name} requires a valid number")
            );
        }
    }
}
