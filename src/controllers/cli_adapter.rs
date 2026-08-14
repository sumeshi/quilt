use crate::controllers::arguments::*;
use crate::controllers::definitions::{CommandSpec, OperationId, OptionSpec, TypedCommand};
use crate::error::QuiltError;
use crate::operations::datetime::{
    parse_ambiguous_policy, parse_epoch_unit, parse_nonexistent_policy, AmbiguousPolicy,
    DateTimeConfig, NonexistentPolicy,
};
use regex::Regex;
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
pub(crate) fn select_columns_from_input(input: &str) -> Result<Vec<String>, QuiltError> {
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
    let operation = OperationId::parse(raw.name)
        .ok_or_else(|| QuiltError::usage(format!("Error: Unknown command '{}'", raw.name)))?;
    match operation {
        OperationId::Load => {
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
        OperationId::Select => {
            args(&raw, 1, None, "select <columns>")?;
            Ok(TypedCommand::Select(SelectArgs {
                columns: select_columns_from_input(&raw.args.join(","))?,
            }))
        }
        OperationId::Cast => {
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
                columns: val(&raw, "column").map(|v| split_values(&v)).transpose()?,
            }))
        }
        OperationId::Head => Ok(TypedCommand::Head(NumberArgs {
            number: val(&raw, "number")
                .or_else(|| a.first().cloned())
                .map(|v| v.parse())
                .transpose()
                .map_err(|_| QuiltError::usage("Error: head requires a valid number"))?
                .unwrap_or(5),
        })),
        OperationId::Tail => Ok(TypedCommand::Tail(NumberArgs {
            number: val(&raw, "number")
                .or_else(|| a.first().cloned())
                .map(|v| v.parse())
                .transpose()
                .map_err(|_| QuiltError::usage("Error: tail requires a valid number"))?
                .unwrap_or(5),
        })),
        OperationId::Sort => {
            args(&raw, 1, Some(1), "sort <columns>")?;
            Ok(TypedCommand::Sort(SortArgs {
                columns: split_values(&a[0])?,
                descending: flag(&raw, "desc"),
            }))
        }
        OperationId::Count => Ok(TypedCommand::Count(CountArgs {
            columns: a
                .iter()
                .flat_map(|v| v.split(','))
                .filter(|v| !v.trim().is_empty())
                .map(|v| v.trim().into())
                .collect(),
        })),
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
                debug: flag(&raw, "debug"),
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
            Ok(TypedCommand::Run(RunArgs {
                config: PathBuf::from(&a[0]),
                input_files: a[1..].iter().map(PathBuf::from).collect(),
                output: val(&raw, "output"),
                vars: raw.repeated.remove("var").unwrap_or_default(),
                check: flag(&raw, "check"),
                show_plan: val(&raw, "show_plan"),
            }))
        }
    }
}
