use crate::controllers::arguments::*;
use crate::controllers::definitions::{CommandSpec, OperationId, OptionSpec, TypedCommand};
use crate::error::QuiltError;
use crate::operations::datetime::{
    parse_ambiguous_policy, parse_epoch_unit, parse_nonexistent_policy, AmbiguousPolicy,
    DateTimeConfig, NonexistentPolicy,
};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Default)]
struct Raw {
    name: &'static str,
    args: Vec<String>,
    options: HashMap<String, Option<String>>,
    repeated: HashMap<String, Vec<String>>,
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
            let s = crate::controllers::definitions::by_name(token)
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
                let (key, inline) = if let Some((key, value)) = flag.split_once('=') {
                    (key, Some(value))
                } else if !token.starts_with("--") && flag.len() > 1 && flag.is_char_boundary(1) {
                    let (short, attached) = flag.split_at(1);
                    if opt(s, short).is_some_and(|option| option.takes_value) {
                        (short, Some(attached))
                    } else {
                        (flag, None)
                    }
                } else {
                    (flag, None)
                };
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
    raws.into_iter()
        .map(build_typed)
        .map(|command| {
            let command = command?;
            crate::controllers::command_model::validate_command(&command)?;
            Ok(command)
        })
        .collect()
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
pub(crate) fn parse_column_list(input: &str) -> Result<Vec<String>, QuiltError> {
    let mut columns = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    for (index, character) in input.char_indices() {
        if character == '"' {
            quoted = !quoted;
        }
        if character == ',' && !quoted {
            columns.push(input[start..index].trim().to_string());
            start = index + 1;
        }
    }
    if quoted {
        return Err(QuiltError::usage("Error: unclosed quote in column list"));
    }
    columns.push(input[start..].trim().to_string());
    for name in &mut columns {
        if name.starts_with('"')
            && name.ends_with('"')
            && name.len() >= 2
            && !has_unquoted_colon(name)
        {
            *name = name[1..name.len() - 1].replace("\"\"", "\"");
        }
    }
    if columns.iter().any(String::is_empty) {
        return Err(QuiltError::usage("Error: column list must not be empty"));
    }
    Ok(columns)
}

fn has_unquoted_colon(input: &str) -> bool {
    let bytes = input.as_bytes();
    let mut quoted = false;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'"' {
            if quoted && bytes.get(index + 1) == Some(&b'"') {
                index += 2;
                continue;
            }
            quoted = !quoted;
        } else if bytes[index] == b':' && !quoted {
            return true;
        }
        index += 1;
    }
    false
}

fn build_typed(mut raw: Raw) -> Result<TypedCommand, QuiltError> {
    let a = &raw.args;
    let operation = OperationId::parse(raw.name)
        .ok_or_else(|| QuiltError::usage(format!("Error: Unknown command '{}'", raw.name)))?;
    match operation {
        OperationId::Load => {
            args(&raw, 0, None, "load [files...] --stdin-format csv|jsonl")?;
            if a.is_empty() && val(&raw, "stdin_format").is_none() {
                return Err(QuiltError::usage(
                    "load requires input paths or --stdin-format",
                ));
            }
            if !a.is_empty() && val(&raw, "stdin_format").is_some() {
                return Err(QuiltError::usage(
                    "--stdin-format cannot be combined with input paths",
                ));
            }
            if let Some(format) = val(&raw, "stdin_format") {
                crate::operations::initializers::stdin::validate_format(&format)?;
            }
            let separator = val(&raw, "separator").unwrap_or_else(|| ",".into());
            if separator.chars().count() != 1 || !separator.is_ascii() {
                return Err(QuiltError::usage(
                    "Error: Separator must be a single ASCII character",
                ));
            }
            Ok(TypedCommand::Load(LoadArgs {
                paths: a.iter().map(PathBuf::from).collect(),
                source_column: val(&raw, "source_column"),
                all_strings: flag(&raw, "all_strings"),
                schema: raw
                    .repeated
                    .get("schema")
                    .into_iter()
                    .flatten()
                    .map(|v| parse_column_type(v))
                    .collect::<Result<_, _>>()?,
                stdin_format: val(&raw, "stdin_format"),
                rejects: val(&raw, "rejects").map(PathBuf::from),
                separator: separator.chars().next().unwrap_or(','),
                low_memory: flag(&raw, "low_memory"),
                no_headers: flag(&raw, "no_headers"),
                chunk_size: val(&raw, "chunk_size")
                    .map(|v| v.parse())
                    .transpose()
                    .map_err(|_| QuiltError::usage("Error: Invalid chunk size"))?,
                infer_schema_length: match val(&raw, "infer_schema_length") {
                    None => {
                        crate::operations::initializers::load::DEFAULT_NDJSON_INFER_SCHEMA_LENGTH
                    }
                    Some(value) if value.eq_ignore_ascii_case("full") => None,
                    Some(value) => {
                        let length = value.parse::<usize>().map_err(|_| {
                            QuiltError::usage(
                                "Error: inference length must be a positive integer or 'full'",
                            )
                        })?;
                        if length == 0 {
                            return Err(QuiltError::usage(
                                "Error: inference length must be positive or 'full'",
                            ));
                        }
                        Some(length)
                    }
                },
            }))
        }
        OperationId::Filter => {
            args(&raw, 1, Some(1), "filter <expression>")?;
            Ok(TypedCommand::Filter(FilterArgs {
                expression: a[0].clone(),
            }))
        }
        OperationId::Derive => {
            args(&raw, 2, Some(2), "derive <output> <expression>")?;
            Ok(TypedCommand::Derive(DeriveArgs {
                output: a[0].clone(),
                expression: a[1].clone(),
            }))
        }
        OperationId::Aggregate => {
            args(
                &raw,
                1,
                None,
                "aggregate <mode:column:output...> [--by columns]",
            )?;
            Ok(TypedCommand::Aggregate(AggregateArgs {
                metrics: a
                    .iter()
                    .map(|s| parse_metric(s))
                    .collect::<Result<_, _>>()?,
                by: val(&raw, "by")
                    .map(|s| parse_column_list(&s))
                    .transpose()?
                    .unwrap_or_default(),
            }))
        }
        OperationId::ArrayContains => {
            args(&raw, 2, Some(2), "array-contains <column> <value>")?;
            Ok(TypedCommand::ArrayContains(ArrayContainsArgs {
                column: a[0].clone(),
                value: a[1].clone(),
            }))
        }
        OperationId::ArrayGet => {
            args(&raw, 2, Some(2), "array-get <column> <index>")?;
            Ok(TypedCommand::ArrayGet(ArrayGetArgs {
                column: a[0].clone(),
                index: a[1]
                    .parse()
                    .map_err(|_| QuiltError::usage("array-get index must be an integer"))?,
                output: val(&raw, "output").unwrap_or_else(|| format!("{}_item", a[0])),
            }))
        }
        OperationId::Explode => {
            args(&raw, 1, Some(1), "explode <columns>")?;
            Ok(TypedCommand::Explode(ExplodeArgs {
                columns: parse_column_list(&a[0])?,
            }))
        }
        OperationId::Select => {
            args(&raw, 1, None, "select <columns>")?;
            Ok(TypedCommand::Select(SelectArgs {
                columns: parse_column_list(&raw.args.join(","))?,
            }))
        }
        OperationId::Cast => {
            args(&raw, 2, Some(2), "cast <column> <type>")?;
            let datetime = datetime_config(&raw)?;
            Ok(TypedCommand::Cast(CastArgs {
                column: a[0].clone(),
                target: a[1].clone(),
                datetime,
            }))
        }
        OperationId::Bucket => {
            args(&raw, 2, Some(2), "bucket <column> <interval>")?;
            Ok(TypedCommand::Bucket(BucketArgs {
                column: a[0].clone(),
                interval: a[1].clone(),
                output: val(&raw, "output"),
                datetime: datetime_config(&raw)?,
            }))
        }
        OperationId::Delta => {
            args(&raw, 1, Some(1), "delta <column>")?;
            Ok(TypedCommand::Delta(ColumnOutputArgs {
                by: val(&raw, "by")
                    .map(|s| parse_column_list(&s))
                    .transpose()?
                    .unwrap_or_default(),
                column: a[0].clone(),
                output: val(&raw, "output"),
            }))
        }
        OperationId::Extract => {
            args(&raw, 2, Some(2), "extract <column> <regex>")?;
            Ok(TypedCommand::Extract(ExtractArgs {
                column: a[0].clone(),
                pattern: a[1].clone(),
            }))
        }
        OperationId::Flatten => {
            args(&raw, 0, Some(0), "flatten")?;
            Ok(TypedCommand::Flatten(UnitArgs))
        }
        OperationId::ParseSize => {
            args(&raw, 1, Some(1), "parse-size <column>")?;
            Ok(TypedCommand::ParseSize(ParseSizeArgs {
                column: a[0].clone(),
            }))
        }
        OperationId::Isin => {
            args(&raw, 2, Some(2), "isin <column> <values>")?;
            Ok(TypedCommand::Isin(IsinArgs {
                column: a[0].clone(),
                values: split_values(&a[1])?,
            }))
        }
        OperationId::Contains => {
            args(&raw, 2, Some(2), "contains <column> <pattern>")?;
            Ok(TypedCommand::Contains(ContainsArgs {
                column: a[0].clone(),
                pattern: a[1].clone(),
                ignore_case: flag(&raw, "ignore_case"),
            }))
        }
        OperationId::Sed => {
            args(&raw, 2, Some(2), "sed <pattern> <replacement>")?;
            Ok(TypedCommand::Sed(SedArgs {
                pattern: a[0].clone(),
                replacement: a[1].clone(),
                column: val(&raw, "column"),
                ignore_case: flag(&raw, "ignore_case"),
            }))
        }
        OperationId::Grep => {
            args(&raw, 1, Some(1), "grep <pattern>")?;
            Ok(TypedCommand::Grep(GrepArgs {
                pattern: a[0].clone(),
                ignore_case: flag(&raw, "ignore_case"),
                invert_match: flag(&raw, "invert_match"),
                columns: val(&raw, "column")
                    .map(|v| parse_column_list(&v))
                    .transpose()?,
            }))
        }
        OperationId::Head => {
            args(&raw, 0, Some(1), "head [number]")?;
            Ok(TypedCommand::Head(NumberArgs {
                number: val(&raw, "number")
                    .or_else(|| a.first().cloned())
                    .map(|v| v.parse())
                    .transpose()
                    .map_err(|_| QuiltError::usage("Error: head requires a valid number"))?
                    .unwrap_or(5),
            }))
        }
        OperationId::Tail => {
            args(&raw, 0, Some(1), "tail [number]")?;
            Ok(TypedCommand::Tail(NumberArgs {
                number: val(&raw, "number")
                    .or_else(|| a.first().cloned())
                    .map(|v| v.parse())
                    .transpose()
                    .map_err(|_| QuiltError::usage("Error: tail requires a valid number"))?
                    .unwrap_or(5),
            }))
        }
        OperationId::Sort => {
            args(&raw, 1, Some(1), "sort <columns>")?;
            Ok(TypedCommand::Sort(SortArgs {
                columns: parse_column_list(&a[0])?,
                descending: flag(&raw, "desc"),
            }))
        }
        OperationId::Count => {
            args(&raw, 0, Some(1), "count [columns]")?;
            Ok(TypedCommand::Count(CountArgs {
                columns: a
                    .first()
                    .map(|value| parse_column_list(value))
                    .transpose()?
                    .unwrap_or_default(),
            }))
        }
        OperationId::TimeInfo => {
            args(
                &raw,
                1,
                Some(1),
                "timeinfo <datetime-column> [--by columns]",
            )?;
            Ok(TypedCommand::TimeInfo(TimeInfoArgs {
                column: a[0].clone(),
                by: val(&raw, "by")
                    .map(|value| parse_column_list(&value))
                    .transpose()?
                    .unwrap_or_default(),
            }))
        }
        OperationId::Uniq => {
            args(&raw, 0, Some(0), "uniq")?;
            Ok(TypedCommand::Uniq(UnitArgs))
        }
        OperationId::ChangeTz => {
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
        OperationId::RenameCol => {
            args(&raw, 2, Some(2), "renamecol <old> <new>")?;
            Ok(TypedCommand::RenameCol(RenameColArgs {
                old: a[0].clone(),
                new: a[1].clone(),
            }))
        }
        OperationId::TimeSlice => {
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
        OperationId::Show => {
            args(&raw, 0, Some(0), "show")?;
            Ok(TypedCommand::Show(ShowArgs {
                format: val(&raw, "format")
                    .as_deref()
                    .map(OutputFormat::parse)
                    .transpose()?
                    .unwrap_or_default(),
            }))
        }
        OperationId::ShowTable => {
            args(&raw, 0, Some(0), "showtable")?;
            Ok(TypedCommand::ShowTable(UnitArgs))
        }
        OperationId::Headers => {
            args(&raw, 0, Some(0), "headers")?;
            Ok(TypedCommand::Headers(HeadersArgs {
                plain: flag(&raw, "plain"),
            }))
        }
        OperationId::Stats => {
            args(&raw, 0, Some(0), "stats")?;
            Ok(TypedCommand::Stats(UnitArgs))
        }
        OperationId::ShowQuery => {
            args(&raw, 0, Some(0), "showquery")?;
            Ok(TypedCommand::ShowQuery(UnitArgs))
        }
        OperationId::Dump => {
            args(&raw, 0, Some(0), "dump")?;
            let format = val(&raw, "format")
                .as_deref()
                .map(OutputFormat::parse)
                .transpose()?
                .unwrap_or_default();
            if format == OutputFormat::Jsonl && val(&raw, "separator").is_some() {
                return Err(QuiltError::usage("separator applies only to CSV output"));
            }
            let separator = val(&raw, "separator").unwrap_or_else(|| ",".into());
            if separator.chars().count() != 1 || !separator.is_ascii() {
                return Err(QuiltError::usage(
                    "Error: Separator must be a single ASCII character",
                ));
            }
            Ok(TypedCommand::Dump(DumpArgs {
                format,
                output: val(&raw, "output"),
                separator: separator.chars().next().unwrap_or(','),
            }))
        }
        OperationId::DumpCache => {
            args(&raw, 0, Some(0), "dumpcache")?;
            Ok(TypedCommand::DumpCache(DumpCacheArgs {
                output: val(&raw, "output"),
            }))
        }
        OperationId::Partition => {
            args(&raw, 1, Some(2), "partition <column> [directory]")?;
            Ok(TypedCommand::Partition(PartitionArgs {
                column: a[0].clone(),
                output_dir: a.get(1).cloned().unwrap_or_else(|| "./partitions".into()),
            }))
        }
        OperationId::Calc => {
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
        OperationId::Run => {
            args(&raw, 1, None, "run <config> [files...]")?;
            if val(&raw, "stage").is_some() && val(&raw, "show_plan").is_some() {
                return Err(QuiltError::usage(
                    "run --stage and --show-plan cannot be combined",
                ));
            }
            if flag(&raw, "check") && val(&raw, "show_plan").is_some() {
                return Err(QuiltError::usage(
                    "run --check and --show-plan cannot be combined",
                ));
            }
            Ok(TypedCommand::Run(RunArgs {
                config: PathBuf::from(&a[0]),
                input_files: a[1..].iter().map(PathBuf::from).collect(),
                output: val(&raw, "output"),
                vars: raw.repeated.remove("var").unwrap_or_default(),
                check: flag(&raw, "check"),
                show_plan: val(&raw, "show_plan"),
                stage: val(&raw, "stage"),
            }))
        }
    }
}

#[cfg(test)]
mod select_columns_tests {
    use super::{parse_column_list, parse_typed_commands};
    use crate::controllers::definitions::TypedCommand;

    #[test]
    fn select_column_names_unescape_csv_quotes_without_unquoting_ranges() {
        let escaped_header = format!("\"{}\"", r#"a":"b"#.replace('"', "\"\""));
        assert_eq!(parse_column_list(&escaped_header).unwrap(), [r#"a":"b"#]);
        assert_eq!(
            parse_column_list(r#""ns:a":"ns:b""#).unwrap(),
            [r#""ns:a":"ns:b""#]
        );
    }

    #[test]
    fn column_list_commands_share_quoted_comma_and_space_grammar() {
        let quoted = r#""a,b",plain"#;
        let expected = ["a,b", "plain"];
        let commands = [
            vec!["sort".into(), quoted.into()],
            vec!["count".into(), quoted.into()],
            vec![
                "grep".into(),
                "pattern".into(),
                "--column".into(),
                quoted.into(),
            ],
        ];
        for argv in commands {
            let parsed = parse_typed_commands(&argv).unwrap();
            let columns = match &parsed[0] {
                TypedCommand::Sort(args) => &args.columns,
                TypedCommand::Count(args) => &args.columns,
                TypedCommand::Grep(args) => args.columns.as_ref().unwrap(),
                _ => unreachable!(),
            };
            assert_eq!(columns, &expected);
        }
        assert_eq!(parse_column_list(r#"" a b ""#).unwrap(), [" a b "]);
    }
}
