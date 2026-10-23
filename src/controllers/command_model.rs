pub use crate::controllers::arguments::*;

pub use crate::controllers::definitions::{
    by_name, registry, CommandCategory, CommandSpec, OperationId, OptionSpec, TypedCommand,
};

pub use crate::controllers::cli_adapter::parse_typed_commands;
pub use crate::controllers::yaml_adapter::parse_automation_step;

/// Validate command arguments that do not depend on an input schema.
/// Adapters call this after constructing the canonical typed command so CLI
/// execution and automation preflight enforce the same static contract.
pub fn validate_command(command: &TypedCommand) -> Result<(), crate::error::QuiltError> {
    match command {
        TypedCommand::Load(args) => {
            crate::operations::initializers::ndjson::validate_json_pointers(&args.json_strings)
        }
        TypedCommand::Grep(args) => {
            crate::operations::chainables::grep::validate(&args.pattern, args.ignore_case)
        }
        TypedCommand::Sed(args) => {
            crate::operations::chainables::sed::validate(&args.pattern, args.ignore_case)
        }
        TypedCommand::Extract(args) => {
            crate::operations::chainables::extract::validate(&args.pattern)
        }
        TypedCommand::Filter(args) => {
            crate::operations::chainables::filter::validate_syntax(&args.expression)
        }
        TypedCommand::Derive(args) => {
            crate::operations::chainables::filter::validate_syntax(&args.expression)
        }
        TypedCommand::Aggregate(args) => {
            crate::operations::chainables::aggregate::validate_specs(&args.metrics, &args.by)
        }
        TypedCommand::Cast(args) => {
            crate::operations::chainables::cast::validate_config(&args.target, &args.datetime)
        }
        TypedCommand::Bucket(args) => {
            crate::operations::chainables::bucket::validate_interval(&args.interval)?;
            validate_datetime_format(&args.datetime)
        }
        TypedCommand::TimeSlice(args) => validate_datetime_format(&args.datetime),
        TypedCommand::ChangeTz(args) => {
            if let Some(format) = &args.input_format {
                crate::operations::datetime::validate_config_format(format)
                    .map_err(crate::QuiltError::usage)?;
            }
            if let Some(format) = &args.output_format {
                crate::operations::datetime::validate_config_format(format)
                    .map_err(crate::QuiltError::usage)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn validate_datetime_format(
    config: &crate::operations::datetime::DateTimeConfig,
) -> Result<(), crate::QuiltError> {
    config
        .input_format
        .as_deref()
        .map(crate::operations::datetime::validate_config_format)
        .transpose()
        .map(|_| ())
        .map_err(crate::QuiltError::usage)
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
        for s in crate::controllers::definitions::registry()
            .iter()
            .filter(|s| s.category == cat)
        {
            out.push_str(&format!("  {:12} {} — {}\n", s.name, s.help, s.summary));
        }
    }
    out
}
pub fn render_command_help(name: &str) -> Option<String> {
    let s = crate::controllers::definitions::by_name(name)?;
    let details = match name {
        "run" => "\nExecute a version-2 YAML workflow. Version 1 requires migration to version 2. Omitted load paths take positional input files. --stage selects one stage and its dependencies; --output replaces the selected stage's single dump path. CLI paths are relative to the caller; YAML paths and path defaults are relative to the recipe. --check validates without input reads or output writes. --show-plan inspects a selected non-branch stage without evaluating rows. Parameter references occupy a whole YAML value: {$param: name}.\n",
        "count" => "\nCount repeated rows with `count`, or count values in selected columns with `count <columns>` (for example, `count hoge`). Select multiple columns to count each combination (for example, `select hoge,fuga - count`). Results are sorted by count, highest first; null values form a group and are counted.\n",
        "load" => "\nCSV and JSONL use complete schema discovery and strict record validation by default. Bounded discovery still validates later records. CSV schema options preserve identifiers; --json-string /path normalizes selected JSONL fields. --cache-input materializes validated input once for reuse in the current session. Managed temporary disk can be limited with QLT_TEMP_LIMIT (bytes).\n",
        "filter" => "\nTyped Boolean expressions: == != < <= > >=, and/or/not (also && || !), parentheses, IS NULL/IS NOT NULL. Strings are quoted; backticks quote literal column names. Column comparisons preserve exact integers and reject incompatible types.\n",
        "derive" => "\nCreate a new column with + - * / and typed column/literal expressions. Integer results are checked Int64 values. Use cast <column> float before floating-point rates; integer columns are never silently promoted to Float64. Existing output columns are rejected.\n",
        "aggregate" => "\nOne or more metric arguments: count:output or mode:column:output. Modes: count,sum,avg,min,max,median,std,p0..p100,nunique. --by selects comma-separated grouping columns; groups keep first-seen order. Integer sums fail on overflow; sum over no non-null values is null; nunique excludes null. Min/max/p0/p100 retain exact source values. Other statistics use floating-point results and may round large integers. The result remains chainable.\n",
        "array-contains" => "\nFilter by exact typed scalar list membership. null searches for a null item; quote 'null' within the value to search that string. Null lists do not match.\n",
        "array-get" => "\nCreate a new column with one list element; negative indexes count from the end and out-of-range indexes return null. Default output: <column>_item.\n",
        "explode" => "\nExplicitly expand selected list columns into rows. Multiple list columns must have matching element counts per row. Empty and null lists produce a null element row.\n",
        "delta" => "\n--by columns limits the preceding-row difference to each group, in current row order. --output sets the output column. Numeric/temporal integer arithmetic is checked; CSV duration output uses integer counts in its source unit.\n",
        "showtable" => "\nStruct fields are displayed as dot paths; lists stay JSON cells. Preview: 8 rows, 8 columns, 64 display columns per cell/header. Omitted content is marked. Controls are escaped. Use select to choose visible columns, show/dump for complete CSV output.\n",
        "flatten" => "\nLazily project recursive Struct fields as dot paths; preserve lists and non-null empty objects. Path collisions are errors. Lists are never exploded.\n",
        "show" | "dump" => "\n--format csv|jsonl selects the output (default CSV). JSONL retains supported nested values, nulls and exact integers; datetime values are strings and durations are integer counts in their declared unit. Non-finite JSON numbers fail.\nCSV output: nested objects/lists are JSON strings inside quoted CSV cells; nulls are empty. Use flatten to expose object fields as columns, dumpcache for typed Parquet.\n",
        "select" => "\nExact existing names take precedence over range syntax; * and ^name$ are literal names. Quote comma-containing names inside the argument, e.g. select '\"a,b\",xmlns:auto-ns3'. YAML columns sequences preserve literal names.\n",
        "grep" | "contains" => "\nNested columns are searched through their JSON representation (keys and values, JSON escaping included); matched rows retain original types. Use flatten for scalar-field search.\n",
        "sed" => "\nGlobal replacement modifies only String columns, preserving numeric, Boolean, temporal and nested columns. An explicit scalar target is converted to String; a nested target is an error. Use flatten to target scalar fields.\n",
        "isin" => "\nAn empty values argument selects no rows; CSV-quoted empty values match the empty string. Membership keys must be scalar; nested columns are rejected. Use flatten for a scalar field or grep/contains for JSON text search.\n",
        _ => "",
    };
    let mut options = String::new();
    for option in s.options {
        let meta = crate::controllers::definitions::option_metadata(s, option);
        let short = option
            .short
            .map(|short| format!("-{short}, "))
            .unwrap_or_default();
        let value = if option.takes_value {
            format!(" <{}>", meta.value_hint.unwrap_or("value"))
        } else {
            String::new()
        };
        options.push_str(&format!(
            "  {short}--{}{value}  {}",
            option.name, meta.description
        ));
        if !meta.choices.is_empty() {
            options.push_str(&format!(" [{}]", meta.choices.join("|")));
        }
        if option.repeated {
            options.push_str(" (repeatable)");
        }
        options.push('\n');
    }
    Some(format!(
        "{}\n\nUsage: qlt {}\n{}\n{}{}",
        s.name, s.help, s.summary, options, details
    ))
}
