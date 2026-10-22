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
    One(yaml_serde::Value),
    Many(Vec<yaml_serde::Value>),
}
impl YamlValues {
    fn into_vec(self) -> Result<Vec<String>, QuiltError> {
        let values = match self {
            Self::One(yaml_serde::Value::Sequence(v)) => v,
            Self::One(v) => vec![v],
            Self::Many(v) => v,
        };
        values
            .into_iter()
            .map(|v| match v {
                yaml_serde::Value::String(s) => Ok(s),
                yaml_serde::Value::Number(n) => Ok(n.to_string()),
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
    #[serde(rename = "all-strings", default)]
    all_strings: bool,
    #[serde(default)]
    schema: std::collections::BTreeMap<String, String>,
    #[serde(rename = "stdin-format")]
    stdin_format: Option<String>,
    rejects: Option<PathBuf>,
    paths: Option<YamlStrings>,
    #[serde(rename = "source-column")]
    source_column: Option<String>,
    separator: Option<String>,
    #[serde(rename = "low-memory", default)]
    low_memory: bool,
    #[serde(rename = "no-headers", default)]
    no_headers: bool,
    #[serde(rename = "chunk-size")]
    chunk_size: Option<usize>,
    #[serde(rename = "infer-schema-length")]
    infer_schema_length: Option<YInferSchemaLength>,
}
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum YInferSchemaLength {
    Number(usize),
    Text(String),
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
    by: Option<YamlStrings>,
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
    format: Option<String>,
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
    format: Option<String>,
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
    value: &yaml_serde::Value,
    name: &str,
) -> Result<T, QuiltError> {
    let value = if value.is_null() {
        yaml_serde::Value::Mapping(yaml_serde::Mapping::new())
    } else {
        value.clone()
    };
    yaml_serde::from_value(value).map_err(|e| {
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
    value: &yaml_serde::Value,
) -> Result<TypedCommand, QuiltError> {
    let operation = OperationId::parse(name)
        .ok_or_else(|| QuiltError::usage(format!("Error: Unknown automation command '{name}'")))?;
    let command = match operation {
        OperationId::Load => {
            let a: YLoad = yaml_args(value, name)?;
            let paths = a.paths.map(YamlStrings::into_vec).unwrap_or_default();
            if let Some(format) = &a.stdin_format {
                crate::operations::initializers::stdin::validate_format(format)?;
                if !paths.is_empty() {
                    return Err(QuiltError::usage(
                        "stdin-format cannot be combined with input paths",
                    ));
                }
            }
            let infer = match a.infer_schema_length {
                None => crate::operations::initializers::load::DEFAULT_NDJSON_INFER_SCHEMA_LENGTH,
                Some(YInferSchemaLength::Text(v)) if v.eq_ignore_ascii_case("full") => None,
                Some(YInferSchemaLength::Number(n)) => {
                    if n == 0 {
                        return Err(QuiltError::usage(
                            "Error: infer-schema-length must be positive",
                        ));
                    }
                    Some(n)
                }
                Some(YInferSchemaLength::Text(v)) => {
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
                source_column: a.source_column,
                all_strings: a.all_strings,
                schema: a
                    .schema
                    .into_iter()
                    .map(|(name, dtype)| parse_column_type(&format!("{name}={dtype}")))
                    .collect::<Result<_, _>>()?,
                stdin_format: a.stdin_format,
                rejects: a.rejects,
                separator: separator(a.separator)?,
                low_memory: a.low_memory,
                no_headers: a.no_headers,
                chunk_size: a.chunk_size,
                infer_schema_length: infer,
            }))
        }
        OperationId::Filter => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct X {
                expression: String,
            }
            let a: X = yaml_args(value, name)?;
            Ok(TypedCommand::Filter(FilterArgs {
                expression: a.expression,
            }))
        }
        OperationId::Derive => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct X {
                output: String,
                expression: String,
            }
            let a: X = yaml_args(value, name)?;
            Ok(TypedCommand::Derive(DeriveArgs {
                output: a.output,
                expression: a.expression,
            }))
        }
        OperationId::Aggregate => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct X {
                metrics: YamlStrings,
                by: Option<YamlStrings>,
            }
            let a: X = yaml_args(value, name)?;
            Ok(TypedCommand::Aggregate(AggregateArgs {
                metrics: a
                    .metrics
                    .into_vec()
                    .iter()
                    .map(|s| parse_metric(s))
                    .collect::<Result<_, _>>()?,
                by: a.by.map(YamlStrings::into_vec).unwrap_or_default(),
            }))
        }
        OperationId::ArrayContains => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct X {
                column: String,
                value: String,
            }
            let a: X = yaml_args(value, name)?;
            Ok(TypedCommand::ArrayContains(ArrayContainsArgs {
                column: a.column,
                value: a.value,
            }))
        }
        OperationId::ArrayGet => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct X {
                column: String,
                index: i64,
                output: Option<String>,
            }
            let a: X = yaml_args(value, name)?;
            Ok(TypedCommand::ArrayGet(ArrayGetArgs {
                output: a.output.unwrap_or_else(|| format!("{}_item", a.column)),
                column: a.column,
                index: a.index,
            }))
        }
        OperationId::Explode => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct X {
                columns: YamlStrings,
            }
            let a: X = yaml_args(value, name)?;
            Ok(TypedCommand::Explode(ExplodeArgs {
                columns: a.columns.into_vec(),
            }))
        }
        OperationId::Select => {
            let a: YSelect = yaml_args(value, name)?;
            Ok(TypedCommand::Select(SelectArgs {
                columns: match a.columns {
                    YamlStrings::One(value) => {
                        crate::controllers::cli_adapter::select_columns_from_input(&value)?
                    }
                    YamlStrings::Many(values) => values,
                },
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
        OperationId::TimeInfo => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct X {
                column: String,
                by: Option<YamlStrings>,
            }
            let a: X = yaml_args(value, name)?;
            Ok(TypedCommand::TimeInfo(TimeInfoArgs {
                column: a.column,
                by: a
                    .by
                    .map(|value| match value {
                        YamlStrings::One(value) => {
                            crate::controllers::cli_adapter::select_columns_from_input(&value)
                        }
                        YamlStrings::Many(values) => Ok(values),
                    })
                    .transpose()?
                    .unwrap_or_default(),
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
                by: a.by.map(YamlStrings::into_vec).unwrap_or_default(),
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
            Ok(TypedCommand::Show(ShowArgs {
                format: a
                    .format
                    .as_deref()
                    .map(OutputFormat::parse)
                    .transpose()?
                    .unwrap_or_default(),
            }))
        }
        OperationId::Headers => {
            let a: YHeaders = yaml_args(value, name)?;
            Ok(TypedCommand::Headers(HeadersArgs { plain: a.plain }))
        }
        OperationId::Dump => {
            let a: YDump = yaml_args(value, name)?;
            let format = a
                .format
                .as_deref()
                .map(OutputFormat::parse)
                .transpose()?
                .unwrap_or_default();
            if format == OutputFormat::Jsonl && a.separator.is_some() {
                return Err(QuiltError::usage("separator applies only to CSV output"));
            }
            Ok(TypedCommand::Dump(DumpArgs {
                format,
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
    }?;
    crate::controllers::command_model::validate_command(&command)?;
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(source: &str) -> yaml_serde::Value {
        yaml_serde::from_str(source).expect("test YAML should parse")
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
    fn timeinfo_by_preserves_literal_yaml_names_and_matches_cli_selection() {
        let from_yaml = parse_automation_step(
            "timeinfo",
            &yaml("column: when\nby: [\"a,b\", xmlns:auto-ns3]"),
        )
        .unwrap();
        let from_cli = crate::controllers::cli_adapter::parse_typed_commands(&[
            "timeinfo".into(),
            "when".into(),
            "--by".into(),
            "\"a,b\",xmlns:auto-ns3".into(),
        ])
        .unwrap()
        .remove(0);
        assert_eq!(from_yaml, from_cli);
        assert_eq!(
            from_yaml,
            TypedCommand::TimeInfo(TimeInfoArgs {
                column: "when".into(),
                by: vec!["a,b".into(), "xmlns:auto-ns3".into()],
            })
        );
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
    fn schema_independent_datetime_validation_matches_cli_and_yaml() {
        for (command, yaml_args, cli_args, expected_valid) in [
            (
                "cast",
                "column: value\ntype: int\ninput-format: '%F'",
                vec!["cast", "value", "int", "--input-format", "%F"],
                false,
            ),
            (
                "cast",
                "column: value\ntype: made-up",
                vec!["cast", "value", "made-up"],
                false,
            ),
            (
                "cast",
                "column: when\ntype: datetime\ninput-format: '%F'",
                vec!["cast", "when", "datetime", "--input-format", "%F"],
                true,
            ),
            (
                "bucket",
                "column: when\ninterval: nonsense",
                vec!["bucket", "when", "nonsense"],
                false,
            ),
        ] {
            let yaml_result = parse_automation_step(command, &yaml(yaml_args));
            let cli_result = crate::controllers::cli_adapter::parse_typed_commands(
                &cli_args
                    .iter()
                    .map(|arg| (*arg).to_owned())
                    .collect::<Vec<_>>(),
            );
            assert_eq!(
                yaml_result.is_ok(),
                expected_valid,
                "YAML {command}: {yaml_args}"
            );
            assert_eq!(
                cli_result.is_ok(),
                expected_valid,
                "CLI {command}: {cli_args:?}"
            );
        }
    }
}
