//! Export typed CLI commands as reusable version-2 automation steps.
use super::arguments::*;
use super::definitions::{CommandCategory, TypedCommand};
use crate::{operations::chainables::aggregate::AggregateMode, QuiltError};
use serde_json::{json, Map, Value as J};

fn path_parameter(parameters: &mut Map<String, J>, name: &str, description: &str) -> J {
    parameters.insert(
        name.into(),
        json!({"type":"path", "required":true, "description":description}),
    );
    json!({"$param":name})
}
fn jstrings(xs: &[String]) -> J {
    json!(xs)
}
fn put_opt(m: &mut Map<String, J>, k: &str, v: &Option<String>) {
    if let Some(v) = v {
        m.insert(k.into(), json!(v));
    }
}
fn datetime(m: &mut Map<String, J>, d: &crate::operations::datetime::DateTimeConfig) {
    if !d.options_present {
        return;
    }
    m.insert("strict".into(), json!(d.strict));
    put_opt(m, "input-format", &d.input_format);
    if let Some(v) = d.epoch_unit {
        m.insert(
            "epoch-unit".into(),
            json!(match v {
                crate::operations::datetime::EpochUnit::Seconds => "s",
                crate::operations::datetime::EpochUnit::Milliseconds => "ms",
                crate::operations::datetime::EpochUnit::Microseconds => "us",
                crate::operations::datetime::EpochUnit::Nanoseconds => "ns",
            }),
        );
    }
    put_opt(m, "timezone", &d.timezone);
    if d.ambiguous != crate::operations::datetime::AmbiguousPolicy::Error {
        m.insert(
            "ambiguous".into(),
            json!(match d.ambiguous {
                crate::operations::datetime::AmbiguousPolicy::Earliest => "earliest",
                _ => "latest",
            }),
        );
    }
    if d.nonexistent != crate::operations::datetime::NonexistentPolicy::Error {
        m.insert(
            "nonexistent".into(),
            json!(match d.nonexistent {
                crate::operations::datetime::NonexistentPolicy::ShiftForward => "shift-forward",
                _ => "shift-backward",
            }),
        );
    }
}
fn step(
    c: &TypedCommand,
    parameters: &mut Map<String, J>,
    output_index: &mut usize,
    partition_index: &mut usize,
) -> Result<(&'static str, J), QuiltError> {
    use TypedCommand::*;
    let mut m = Map::new();
    macro_rules! s {
        ($k:expr,$v:expr) => {{
            m.insert(($k).into(), json!($v));
        }};
    }
    match c {
        Load(a) => {
            if !a.json_strings.is_empty() {
                s!("json-string", a.json_strings);
            }
            if a.cache_input {
                s!("cache-input", true);
            }
            // Input paths are supplied by `qlt run recipe.yaml file...` so
            // exported recipes can be replayed against different data.
            s!("separator", a.separator.to_string());
            if a.low_memory {
                s!("low-memory", true)
            }
            if a.no_headers {
                s!("no-headers", true)
            }
            if let Some(v) = a.chunk_size {
                s!("chunk-size", v)
            }
            if let Some(v) = a.infer_schema_length {
                s!("infer-schema-length", v)
            } else {
                s!("infer-schema-length", "full")
            }
            put_opt(&mut m, "source-column", &a.source_column);
            if a.all_strings {
                s!("all-strings", true)
            }
            if !a.schema.is_empty() {
                m.insert(
                    "schema".into(),
                    json!(a
                        .schema
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect::<std::collections::BTreeMap<_, _>>()),
                );
            }
            put_opt(&mut m, "stdin-format", &a.stdin_format);
            if a.rejects.is_some() {
                m.insert(
                    "rejects".into(),
                    path_parameter(parameters, "rejects", "Path for rejected input rows"),
                );
            }
        }
        Select(a) => {
            m.insert("columns".into(), jstrings(&a.columns));
        }
        Filter(a) => s!("expression", a.expression),
        Derive(a) => {
            s!("output", a.output);
            s!("expression", a.expression)
        }
        Aggregate(a) => {
            let metrics: Vec<J> = a
                .metrics
                .iter()
                .map(|x| {
                    let mode = match x.mode {
                        AggregateMode::Count => "count".into(),
                        AggregateMode::Sum => "sum".into(),
                        AggregateMode::Avg => "avg".into(),
                        AggregateMode::Min => "min".into(),
                        AggregateMode::Max => "max".into(),
                        AggregateMode::Median => "median".into(),
                        AggregateMode::Std => "std".into(),
                        AggregateMode::Nunique => "nunique".into(),
                        AggregateMode::Percentile(p) => format!("p{p}"),
                    };
                    let mut item = Map::new();
                    item.insert("mode".into(), json!(mode));
                    if let Some(column) = &x.column {
                        item.insert("column".into(), json!(column));
                    }
                    item.insert("output".into(), json!(x.output));
                    J::Object(item)
                })
                .collect();
            m.insert("metrics".into(), json!(metrics));
            if !a.by.is_empty() {
                m.insert("by".into(), jstrings(&a.by));
            }
        }
        ArrayContains(a) => {
            s!("column", a.column);
            s!("value", a.value)
        }
        ArrayGet(a) => {
            s!("column", a.column);
            s!("index", a.index);
            s!("output", a.output)
        }
        Explode(a) => {
            m.insert("columns".into(), jstrings(&a.columns));
        }
        Cast(a) => {
            s!("column", a.column);
            s!("type", a.target);
            datetime(&mut m, &a.datetime)
        }
        Bucket(a) => {
            s!("column", a.column);
            s!("interval", a.interval);
            put_opt(&mut m, "output", &a.output);
            datetime(&mut m, &a.datetime)
        }
        Delta(a) => {
            s!("column", a.column);
            put_opt(&mut m, "output", &a.output);
            if !a.by.is_empty() {
                m.insert("by".into(), jstrings(&a.by));
            }
        }
        Extract(a) => {
            s!("column", a.column);
            s!("pattern", a.pattern)
        }
        Flatten(_) | Uniq(_) | Stats(_) | ShowTable(_) | ShowQuery(_) => {}
        ParseSize(a) => s!("column", a.column),
        Isin(a) => {
            s!("column", a.column);
            m.insert("values".into(), json!(a.values));
        }
        Contains(a) => {
            s!("column", a.column);
            s!("pattern", a.pattern);
            if a.ignore_case {
                s!("ignore-case", true)
            }
        }
        Sed(a) => {
            s!("pattern", a.pattern);
            s!("replacement", a.replacement);
            put_opt(&mut m, "column", &a.column);
            if a.ignore_case {
                s!("ignore-case", true)
            }
        }
        Grep(a) => {
            s!("pattern", a.pattern);
            if a.ignore_case {
                s!("ignore-case", true)
            }
            if a.invert_match {
                s!("invert-match", true)
            }
            if let Some(v) = &a.columns {
                m.insert("columns".into(), jstrings(v));
            }
        }
        Head(a) | Tail(a) => s!("number", a.number),
        Sort(a) => {
            m.insert("columns".into(), jstrings(&a.columns));
            if a.descending {
                s!("desc", true)
            }
        }
        Count(a) => {
            if !a.columns.is_empty() {
                m.insert("columns".into(), jstrings(&a.columns));
            }
        }
        TimeInfo(a) => {
            s!("column", a.column);
            if !a.by.is_empty() {
                m.insert("by".into(), jstrings(&a.by));
            }
        }
        ChangeTz(a) => {
            s!("column", a.column);
            s!("from-tz", a.from_tz);
            s!("to-tz", a.to_tz);
            put_opt(&mut m, "output-format", &a.output_format);
            if a.options_present {
                put_opt(&mut m, "input-format", &a.input_format);
                s!("strict", a.strict);
                if let Some(v) = a.epoch_unit {
                    s!(
                        "epoch-unit",
                        match v {
                            crate::operations::datetime::EpochUnit::Seconds => "s",
                            crate::operations::datetime::EpochUnit::Milliseconds => "ms",
                            crate::operations::datetime::EpochUnit::Microseconds => "us",
                            crate::operations::datetime::EpochUnit::Nanoseconds => "ns",
                        }
                    )
                }
                put_opt(&mut m, "timezone", &a.timezone);
                if a.ambiguous != crate::operations::datetime::AmbiguousPolicy::Error {
                    s!(
                        "ambiguous",
                        match a.ambiguous {
                            crate::operations::datetime::AmbiguousPolicy::Earliest => "earliest",
                            _ => "latest",
                        }
                    )
                }
                if a.nonexistent != crate::operations::datetime::NonexistentPolicy::Error {
                    s!(
                        "nonexistent",
                        match a.nonexistent {
                            crate::operations::datetime::NonexistentPolicy::ShiftForward =>
                                "shift-forward",
                            _ => "shift-backward",
                        }
                    )
                }
            }
        }
        RenameCol(a) => {
            s!("old", a.old);
            s!("new", a.new)
        }
        TimeSlice(a) => {
            s!("column", a.column);
            put_opt(&mut m, "start", &a.start);
            put_opt(&mut m, "end", &a.end);
            datetime(&mut m, &a.datetime)
        }
        Show(a) => {
            if a.format == OutputFormat::Jsonl {
                s!("format", "jsonl")
            }
        }
        Headers(a) => {
            if a.plain {
                s!("plain", true)
            }
        }
        Dump(a) => {
            if a.output.as_deref() == Some("-") {
                s!("output", "-")
            } else {
                *output_index += 1;
                let name = if *output_index == 1 {
                    "output".into()
                } else {
                    format!("output_{}", *output_index)
                };
                m.insert(
                    "output".into(),
                    path_parameter(parameters, &name, "Destination file for exported data"),
                );
            }
            if a.format == OutputFormat::Jsonl {
                s!("format", "jsonl")
            }
            if a.separator != ',' {
                s!("separator", a.separator.to_string())
            }
        }
        DumpCache(_) => {
            *output_index += 1;
            let name = if *output_index == 1 {
                "output".into()
            } else {
                format!("output_{}", *output_index)
            };
            m.insert(
                "output".into(),
                path_parameter(parameters, &name, "Destination file for cached data"),
            );
        }
        Partition(a) => {
            s!("column", a.column);
            *partition_index += 1;
            let name = if *partition_index == 1 {
                "output_dir".into()
            } else {
                format!("output_dir_{}", *partition_index)
            };
            m.insert(
                "output-dir".into(),
                path_parameter(parameters, &name, "Directory for partition files"),
            );
        }
        Calc(a) => {
            s!("column", a.column);
            s!(
                match a.aggregation {
                    Aggregation::Sum => "sum",
                    Aggregation::Avg => "avg",
                    Aggregation::Min => "min",
                    Aggregation::Max => "max",
                    Aggregation::Median => "median",
                    Aggregation::Std => "std",
                },
                true
            )
        }
        Run(_) => {
            return Err(QuiltError::usage(
                "run cannot be exported as a record-processing workflow",
            ))
        }
    }
    Ok((c.name(), J::Object(m)))
}

pub(crate) fn workflow_yaml(commands: &[TypedCommand]) -> Result<String, QuiltError> {
    if commands.is_empty() {
        return Err(QuiltError::usage("cannot export an empty workflow"));
    }
    if !matches!(commands.first(), Some(TypedCommand::Load(_))) {
        return Err(QuiltError::usage(
            "an exported workflow must begin with load",
        ));
    }
    let mut finalizer_seen = false;
    for (index, command) in commands.iter().enumerate() {
        if command.category() == CommandCategory::Initializer && index != 0 {
            return Err(QuiltError::usage("load must be the first workflow command"));
        }
        if finalizer_seen && command.category() != CommandCategory::Finalizer {
            return Err(QuiltError::usage(
                "a workflow command cannot follow a finalizer",
            ));
        }
        finalizer_seen |= command.category() == CommandCategory::Finalizer;
    }
    if commands.iter().any(|c| {
        c.category() == CommandCategory::Initializer && !matches!(c, TypedCommand::Load(_))
    }) {
        return Err(QuiltError::usage("unsupported workflow initializer"));
    }
    if commands
        .iter()
        .any(|c| c.category() == CommandCategory::Automation)
    {
        return Err(QuiltError::usage(
            "run cannot be exported as a record-processing workflow",
        ));
    }
    let mut steps = Vec::new();
    let mut parameters = Map::new();
    let mut output_index = 0;
    let mut partition_index = 0;
    for c in commands {
        let (n, v) = step(c, &mut parameters, &mut output_index, &mut partition_index)?;
        steps.push(json!({(n):v}));
    }
    if commands.last().is_some_and(|c| {
        matches!(
            c.category(),
            CommandCategory::Initializer | CommandCategory::Chainable
        )
    }) {
        steps.push(json!({"show":{}}));
    }
    let doc =
        json!({"version":2,"parameters":parameters,"stages":[{"name":"analysis","steps":steps}]});
    let json_doc = serde_json::to_string(&doc)
        .map_err(|e| QuiltError::usage(format!("could not serialize workflow: {e}")))?;
    let yaml_doc: yaml_serde::Value = yaml_serde::from_str(&json_doc)
        .map_err(|e| QuiltError::usage(format!("could not convert workflow values: {e}")))?;
    yaml_serde::to_string(&yaml_doc)
        .map(|yaml| format!("# yaml-language-server: $schema=https://raw.githubusercontent.com/sumeshi/quilt/develop/schemas/run-v2.schema.json\n{yaml}"))
        .map_err(|e| QuiltError::usage(format!("could not serialize workflow: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controllers::command_model::parse_automation_step;

    #[test]
    fn typed_steps_roundtrip_through_version_two_yaml() {
        let commands = vec![
            TypedCommand::Load(LoadArgs {
                json_strings: Vec::new(),
                cache_input: false,
                paths: vec!["input.csv".into()],
                separator: ',',
                low_memory: true,
                no_headers: false,
                chunk_size: Some(31),
                infer_schema_length: None,
                source_column: Some("xmlns:auto-ns3".into()),
                all_strings: true,
                schema: vec![("a,b".into(), "uint".into())],
                stdin_format: None,
                rejects: None,
            }),
            TypedCommand::Select(SelectArgs {
                columns: vec!["a,b".into(), "xmlns:auto-ns3".into()],
            }),
            TypedCommand::Aggregate(AggregateArgs {
                metrics: vec![
                    crate::operations::chainables::aggregate::AggregateSpec::new(
                        AggregateMode::Sum,
                        Some("a,b".into()),
                        "total:raw",
                    ),
                ],
                by: vec!["xmlns:auto-ns3".into()],
            }),
            TypedCommand::Sort(SortArgs {
                columns: vec!["total:raw".into()],
                descending: true,
            }),
            TypedCommand::Cast(CastArgs {
                column: "when".into(),
                target: "datetime".into(),
                datetime: crate::operations::datetime::DateTimeConfig {
                    strict: false,
                    input_format: Some("%Y-%m-%d %H:%M".into()),
                    epoch_unit: None,
                    timezone: Some("America/New_York".into()),
                    ambiguous: crate::operations::datetime::AmbiguousPolicy::Earliest,
                    nonexistent: crate::operations::datetime::NonexistentPolicy::ShiftForward,
                    options_present: true,
                },
            }),
            TypedCommand::ChangeTz(ChangeTzArgs {
                column: "when".into(),
                from_tz: "America/New_York".into(),
                to_tz: "UTC".into(),
                input_format: None,
                output_format: None,
                ambiguous: crate::operations::datetime::AmbiguousPolicy::Error,
                nonexistent: crate::operations::datetime::NonexistentPolicy::Error,
                strict: false,
                epoch_unit: None,
                timezone: None,
                options_present: true,
            }),
        ];
        let raw = workflow_yaml(&commands).unwrap();
        let value: yaml_serde::Value = yaml_serde::from_str(&raw).unwrap();
        let root = value.as_mapping().unwrap();
        let stages = root
            .get(yaml_serde::Value::String("stages".into()))
            .unwrap()
            .as_sequence()
            .unwrap();
        let steps = stages[0]
            .as_mapping()
            .unwrap()
            .get(yaml_serde::Value::String("steps".into()))
            .unwrap()
            .as_sequence()
            .unwrap();
        for (index, command) in commands.iter().enumerate() {
            let step = steps[index].as_mapping().unwrap();
            let (name, args) = step.iter().next().unwrap();
            let parsed = parse_automation_step(name.as_str().unwrap(), args).unwrap();
            if matches!(command, TypedCommand::Load(_)) {
                if let (TypedCommand::Load(expected), TypedCommand::Load(actual)) =
                    (command, parsed)
                {
                    assert_eq!(expected.source_column, actual.source_column);
                    assert_eq!(expected.chunk_size, actual.chunk_size);
                    assert_eq!(expected.schema, actual.schema);
                    assert_eq!(expected.infer_schema_length, actual.infer_schema_length);
                }
            } else {
                assert_eq!(parsed, *command);
            }
        }
    }

    #[test]
    fn exported_workflow_replays_from_another_directory_with_exact_column_names() {
        use crate::controllers::pipeline::PipelineState;
        use crate::controllers::{executor::CommandExecutor, resources::ExecutionResources};
        use crate::operations::automation::run::run;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let input = root.join("input.csv");
        let replay_input = root.join("replay.csv");
        let runs = root.join("nested");
        std::fs::create_dir_all(&runs).unwrap();
        let expected = root.join("expected.csv");
        let output = runs.join("actual.csv");
        std::fs::write(
            &input,
            "id,\"odd,name\",xmlns:auto-ns3\n2,beta,x\n1,alpha,y\n",
        )
        .unwrap();
        std::fs::write(
            &replay_input,
            "id,\"odd,name\",xmlns:auto-ns3\n4,delta,z\n3,gamma,w\n",
        )
        .unwrap();
        let commands = vec![
            TypedCommand::Load(LoadArgs {
                json_strings: Vec::new(),
                cache_input: false,
                paths: vec![input.clone()],
                separator: ',',
                low_memory: false,
                no_headers: false,
                chunk_size: None,
                infer_schema_length: None,
                source_column: None,
                all_strings: true,
                schema: vec![],
                stdin_format: None,
                rejects: None,
            }),
            TypedCommand::Select(SelectArgs {
                columns: vec!["id".into(), "odd,name".into(), "xmlns:auto-ns3".into()],
            }),
            TypedCommand::Sort(SortArgs {
                columns: vec!["id".into()],
                descending: false,
            }),
        ];
        let baseline_cmds = vec![
            TypedCommand::Load(LoadArgs {
                json_strings: Vec::new(),
                cache_input: false,
                paths: vec![replay_input.clone()],
                separator: ',',
                low_memory: false,
                no_headers: false,
                chunk_size: None,
                infer_schema_length: None,
                source_column: None,
                all_strings: true,
                schema: vec![],
                stdin_format: None,
                rejects: None,
            }),
            TypedCommand::Select(SelectArgs {
                columns: vec!["id".into(), "odd,name".into(), "xmlns:auto-ns3".into()],
            }),
            TypedCommand::Sort(SortArgs {
                columns: vec!["id".into()],
                descending: false,
            }),
            TypedCommand::Dump(DumpArgs {
                output: Some(expected.to_string_lossy().into_owned()),
                separator: ',',
                format: OutputFormat::Csv,
            }),
        ];
        let mut baseline =
            CommandExecutor::new_with_resources(ExecutionResources::new_in(root.to_path_buf()));
        baseline.execute_plan(&baseline_cmds).unwrap();
        let yaml_commands = [
            commands,
            vec![TypedCommand::Dump(DumpArgs {
                output: Some(output.to_string_lossy().into_owned()),
                separator: ',',
                format: OutputFormat::Csv,
            })],
        ]
        .concat();
        let raw = workflow_yaml(&yaml_commands).unwrap();
        assert!(!raw.contains(input.to_str().unwrap()));
        assert!(!raw.contains(output.to_str().unwrap()));
        assert!(raw.contains("$schema="));
        let config = runs.join("workflow.yaml");
        std::fs::write(&config, raw).unwrap();
        let mut controller = PipelineState::empty(ExecutionResources::new_in(runs.clone()));
        run(
            &mut controller,
            config.to_str().unwrap(),
            Some(vec![replay_input]),
            None,
            &[format!("output={}", output.display())],
            false,
            None,
        )
        .unwrap();
        assert_eq!(
            std::fs::read(&expected).unwrap(),
            std::fs::read(&output).unwrap()
        );
    }

    #[test]
    fn empty_and_automation_commands_fail() {
        assert!(workflow_yaml(&[]).is_err());
        assert!(workflow_yaml(&[
            TypedCommand::Load(LoadArgs {
                json_strings: Vec::new(),
                cache_input: false,
                paths: vec![],
                separator: ',',
                low_memory: false,
                no_headers: false,
                chunk_size: None,
                infer_schema_length: Some(1000),
                source_column: None,
                all_strings: false,
                schema: vec![],
                stdin_format: None,
                rejects: None
            }),
            TypedCommand::Show(ShowArgs {
                format: OutputFormat::Csv
            }),
            TypedCommand::Head(NumberArgs { number: 2 }),
        ])
        .is_err());
        assert!(workflow_yaml(&[TypedCommand::Run(RunArgs {
            config: "x".into(),
            input_files: vec![],
            output: None,
            vars: vec![],
            check: false,
            show_plan: None,
            stage: None
        })])
        .is_err());
    }
}
