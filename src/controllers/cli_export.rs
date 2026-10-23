//! Export typed commands into the public command-line token grammar.
//!
//! This is used by recipe editors that need to persist or re-open a command
//! without flattening its arguments into a lossy display string.

use crate::controllers::definitions::TypedCommand;
use crate::error::QuiltError;
use crate::operations::chainables::aggregate::AggregateMode;
use crate::operations::datetime::{AmbiguousPolicy, DateTimeConfig, EpochUnit, NonexistentPolicy};
use std::path::Path;

/// Quote one argument using the command editor's double-quoted token syntax.
pub(crate) fn quote_argument(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn option(out: &mut Vec<String>, name: &str, value: &str) {
    out.push(format!("--{name}"));
    out.push(value.to_owned());
}
fn flag(out: &mut Vec<String>, name: &str, yes: bool) {
    if yes {
        out.push(format!("--{name}"));
    }
}
fn cols(values: &[String]) -> String {
    values
        .iter()
        .map(|v| {
            if v.starts_with('-') || v.contains(',') || v.trim() != v || v.contains('"') {
                format!("\"{}\"", v.replace('"', "\"\""))
            } else {
                v.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}
fn unsupported(message: impl Into<String>) -> QuiltError {
    QuiltError::usage(format!(
        "cannot export typed command to CLI: {}",
        message.into()
    ))
}
fn path_arg(path: &Path) -> String {
    let s = path.to_string_lossy().into_owned();
    // `--` cannot be used before a variadic initializer and still permit the
    // pipeline delimiter, so make dash-leading relative paths unambiguous.
    if s.starts_with('-') {
        std::env::current_dir()
            .map(|cwd| cwd.join(path).to_string_lossy().into_owned())
            .unwrap_or(s)
    } else {
        s
    }
}
fn datetime(out: &mut Vec<String>, config: &DateTimeConfig) {
    if !config.options_present {
        return;
    }
    if config.strict {
        flag(out, "strict", true);
    } else {
        out.push("--strict=false".into());
    }
    if let Some(v) = &config.input_format {
        option(out, "input-format", v);
    }
    if let Some(v) = config.epoch_unit {
        option(out, "epoch-unit", epoch(v));
    }
    if let Some(v) = &config.timezone {
        option(out, "timezone", v);
    }
    option(out, "ambiguous", ambiguous(config.ambiguous));
    option(out, "nonexistent", nonexistent(config.nonexistent));
}
fn epoch(v: EpochUnit) -> &'static str {
    match v {
        EpochUnit::Seconds => "s",
        EpochUnit::Milliseconds => "ms",
        EpochUnit::Microseconds => "us",
        EpochUnit::Nanoseconds => "ns",
    }
}
fn ambiguous(v: AmbiguousPolicy) -> &'static str {
    match v {
        AmbiguousPolicy::Error => "error",
        AmbiguousPolicy::Earliest => "earliest",
        AmbiguousPolicy::Latest => "latest",
    }
}
fn nonexistent(v: NonexistentPolicy) -> &'static str {
    match v {
        NonexistentPolicy::Error => "error",
        NonexistentPolicy::ShiftForward => "shift-forward",
        NonexistentPolicy::ShiftBackward => "shift-backward",
    }
}
fn csv_list(values: &[String]) -> String {
    values
        .iter()
        .map(|v| {
            if v.is_empty() || v.contains(',') || v.contains('"') || v.trim() != v {
                format!("\"{}\"", v.replace('"', "\"\""))
            } else {
                v.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Return argv tokens for one typed command without pipeline separators.
fn command_argv(command: &TypedCommand) -> Result<Vec<String>, QuiltError> {
    let mut out = vec![command.name().to_owned()];
    let mut positional = Vec::<String>::new();
    match command {
        TypedCommand::Load(a) => {
            if let Some(v) = &a.source_column {
                option(&mut out, "source-column", v);
            }
            if a.separator != ',' {
                option(&mut out, "separator", &a.separator.to_string());
            }
            flag(&mut out, "low-memory", a.low_memory);
            flag(&mut out, "no-headers", a.no_headers);
            if let Some(v) = a.chunk_size {
                option(&mut out, "chunk-size", &v.to_string());
            }
            if let Some(v) = a.infer_schema_length {
                option(&mut out, "infer-schema-length", &v.to_string());
            }
            flag(&mut out, "all-strings", a.all_strings);
            for (name, ty) in &a.schema {
                option(&mut out, "schema", &format!("{name}={ty}"));
            }
            if let Some(v) = &a.stdin_format {
                option(&mut out, "stdin-format", v);
            }
            if let Some(v) = &a.rejects {
                option(&mut out, "rejects", &v.to_string_lossy());
            }
            for v in &a.json_strings {
                option(&mut out, "json-string", v);
            }
            flag(&mut out, "cache-input", a.cache_input);
            positional.extend(a.paths.iter().map(|p| path_arg(p)));
        }
        TypedCommand::Select(a) => positional.push(cols(&a.columns)),
        TypedCommand::Filter(a) => positional.push(a.expression.clone()),
        TypedCommand::Derive(a) => positional.extend([a.output.clone(), a.expression.clone()]),
        TypedCommand::Aggregate(a) => {
            for m in &a.metrics {
                let mode = match &m.mode {
                    AggregateMode::Count => "count".to_string(),
                    AggregateMode::Sum => "sum".into(),
                    AggregateMode::Avg => "avg".into(),
                    AggregateMode::Min => "min".into(),
                    AggregateMode::Max => "max".into(),
                    AggregateMode::Median => "median".into(),
                    AggregateMode::Std => "std".into(),
                    AggregateMode::Percentile(p) => format!("p{p}"),
                    AggregateMode::Nunique => "nunique".into(),
                };
                if m.output.contains(':') || m.column.as_ref().is_some_and(|s| s.contains(':')) {
                    positional.push(
                        serde_json::json!({"mode": mode, "column": &m.column, "output": &m.output})
                            .to_string(),
                    );
                } else {
                    positional.push(match &m.column {
                        Some(c) => format!("{mode}:{c}:{}", m.output),
                        None => format!("{mode}:{}", m.output),
                    });
                }
            }
            if !a.by.is_empty() {
                option(&mut out, "by", &cols(&a.by));
            }
        }
        TypedCommand::ArrayContains(a) => positional.extend([a.column.clone(), a.value.clone()]),
        TypedCommand::ArrayGet(a) => {
            positional.extend([a.column.clone(), a.index.to_string()]);
            option(&mut out, "output", &a.output);
        }
        TypedCommand::Explode(a) => positional.push(cols(&a.columns)),
        TypedCommand::Cast(a) => {
            positional.extend([a.column.clone(), a.target.clone()]);
            datetime(&mut out, &a.datetime);
        }
        TypedCommand::Bucket(a) => {
            positional.extend([a.column.clone(), a.interval.clone()]);
            if let Some(v) = &a.output {
                option(&mut out, "output", v);
            }
            datetime(&mut out, &a.datetime);
        }
        TypedCommand::Delta(a) => {
            positional.push(a.column.clone());
            if let Some(v) = &a.output {
                option(&mut out, "output", v);
            }
            if !a.by.is_empty() {
                option(&mut out, "by", &cols(&a.by));
            }
        }
        TypedCommand::Extract(a) => positional.extend([a.column.clone(), a.pattern.clone()]),
        TypedCommand::Flatten(_) | TypedCommand::Uniq(_) => {}
        TypedCommand::ParseSize(a) => positional.push(a.column.clone()),
        TypedCommand::Isin(a) => positional.extend([a.column.clone(), csv_list(&a.values)]),
        TypedCommand::Contains(a) => {
            positional.extend([a.column.clone(), a.pattern.clone()]);
            flag(&mut out, "ignore-case", a.ignore_case);
        }
        TypedCommand::Sed(a) => {
            positional.extend([a.pattern.clone(), a.replacement.clone()]);
            if let Some(v) = &a.column {
                option(&mut out, "column", v);
            }
            flag(&mut out, "ignore-case", a.ignore_case);
        }
        TypedCommand::Grep(a) => {
            positional.push(a.pattern.clone());
            flag(&mut out, "ignore-case", a.ignore_case);
            flag(&mut out, "invert-match", a.invert_match);
            if let Some(v) = &a.columns {
                option(&mut out, "column", &cols(v));
            }
        }
        TypedCommand::Head(a) => positional.push(a.number.to_string()),
        TypedCommand::Tail(a) => positional.push(a.number.to_string()),
        TypedCommand::Sort(a) => {
            positional.push(cols(&a.columns));
            flag(&mut out, "desc", a.descending);
        }
        TypedCommand::Count(a) => {
            if !a.columns.is_empty() {
                positional.push(cols(&a.columns));
            }
        }
        TypedCommand::TimeInfo(a) => {
            positional.push(a.column.clone());
            if !a.by.is_empty() {
                option(&mut out, "by", &cols(&a.by));
            }
        }
        TypedCommand::ChangeTz(a) => {
            positional.push(a.column.clone());
            option(&mut out, "from-tz", &a.from_tz);
            option(&mut out, "to-tz", &a.to_tz);
            if a.options_present {
                if a.strict {
                    flag(&mut out, "strict", true);
                } else {
                    out.push("--strict=false".into());
                }
                if let Some(v) = &a.input_format {
                    option(&mut out, "input-format", v);
                }
                option(&mut out, "ambiguous", ambiguous(a.ambiguous));
                option(&mut out, "nonexistent", nonexistent(a.nonexistent));
                if let Some(v) = a.epoch_unit {
                    option(&mut out, "epoch-unit", epoch(v));
                }
                if let Some(v) = &a.timezone {
                    option(&mut out, "timezone", v);
                }
            }
            if let Some(v) = &a.output_format {
                option(&mut out, "output-format", v);
            }
        }
        TypedCommand::RenameCol(a) => positional.extend([a.old.clone(), a.new.clone()]),
        TypedCommand::TimeSlice(a) => {
            positional.push(a.column.clone());
            if let Some(v) = &a.start {
                option(&mut out, "start", v);
            }
            if let Some(v) = &a.end {
                option(&mut out, "end", v);
            }
            datetime(&mut out, &a.datetime);
        }
        TypedCommand::Show(_)
        | TypedCommand::Headers(_)
        | TypedCommand::Dump(_)
        | TypedCommand::DumpCache(_)
        | TypedCommand::Partition(_)
        | TypedCommand::Calc(_)
        | TypedCommand::ShowTable(_)
        | TypedCommand::Stats(_)
        | TypedCommand::ShowQuery(_) => {
            return Err(unsupported("finalizers are not part of a lazy pipeline"))
        }
        TypedCommand::Run(_) => {
            return Err(unsupported(
                "run commands cannot be reopened as a lazy pipeline",
            ))
        }
    }
    // Keep options before positional arguments, then terminate option parsing
    // only when a positional could be mistaken for an option.
    if positional
        .iter()
        .any(|v| v.starts_with('-') && v.parse::<f64>().is_err())
    {
        out.push("--".into());
    }
    out.extend(positional);
    Ok(out)
}

/// Format one command as a human-readable command string.
pub(crate) fn format_command(command: &TypedCommand) -> Result<String, QuiltError> {
    let argv = command_argv(command)?;
    Ok(argv
        .iter()
        .map(|token| quote_argument(token))
        .collect::<Vec<_>>()
        .join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controllers::arguments::*;
    use crate::controllers::cli_adapter::parse_typed_commands;
    use crate::operations::datetime::{AmbiguousPolicy, DateTimeConfig, NonexistentPolicy};

    #[test]
    fn typed_command_argv_round_trips_options_and_quoted_unicode_values() {
        let commands = vec![
            TypedCommand::Load(LoadArgs {
                paths: vec!["input file.csv".into()],
                json_strings: vec!["/café~1field".into()],
                cache_input: true,
                separator: ',',
                low_memory: true,
                no_headers: false,
                chunk_size: Some(128),
                infer_schema_length: Some(24),
                source_column: None,
                all_strings: false,
                schema: vec![("名, 前".into(), "string".into())],
                stdin_format: None,
                rejects: None,
            }),
            TypedCommand::Select(SelectArgs {
                columns: vec!["first name".into(), "café".into(), "-name".into()],
            }),
            TypedCommand::Isin(IsinArgs {
                column: "café".into(),
                values: vec!["a,b".into(), "two \"quotes\"".into(), String::new()],
            }),
            TypedCommand::Aggregate(AggregateArgs {
                metrics: vec![
                    crate::operations::chainables::aggregate::AggregateSpec::new(
                        AggregateMode::Sum,
                        Some("amount:net".into()),
                        "sum:total",
                    ),
                ],
                by: vec!["group, name".into()],
            }),
            TypedCommand::Contains(ContainsArgs {
                column: "café".into(),
                pattern: "a \"quote\"".into(),
                ignore_case: true,
            }),
            TypedCommand::ChangeTz(ChangeTzArgs {
                column: "created at".into(),
                from_tz: "UTC".into(),
                to_tz: "Europe/Paris".into(),
                input_format: None,
                output_format: Some("%Y-%m-%d".into()),
                ambiguous: AmbiguousPolicy::Error,
                nonexistent: NonexistentPolicy::Error,
                strict: false,
                epoch_unit: None,
                timezone: None,
                options_present: false,
            }),
            TypedCommand::Cast(CastArgs {
                column: "created at".into(),
                target: "datetime".into(),
                datetime: DateTimeConfig {
                    strict: false,
                    input_format: Some("%Y-%m-%d %H:%M:%S".into()),
                    epoch_unit: None,
                    timezone: Some("Europe/Paris".into()),
                    ambiguous: AmbiguousPolicy::Earliest,
                    nonexistent: NonexistentPolicy::ShiftForward,
                    options_present: true,
                },
            }),
        ];
        for command in commands {
            let argv = command_argv(&command).unwrap();
            assert_eq!(parse_typed_commands(&argv).unwrap(), [command]);
        }
    }
}
