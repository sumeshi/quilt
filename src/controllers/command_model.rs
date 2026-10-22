pub use crate::controllers::arguments::*;

pub use crate::controllers::definitions::{
    by_name, registry, CommandCategory, CommandSpec, OperationId, OptionSpec, TypedCommand,
};

pub use crate::controllers::cli_adapter::parse_typed_commands;
pub use crate::controllers::yaml_adapter::parse_automation_step;

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
    if name == "run" {
        return Some(
            format!(
                "run\n\nUsage: qlt {}\n{}\n\nExecute a version-1 YAML workflow. If a load step omits paths, positional files ",
                s.help, s.summary
            )
                + "after the config path are supplied to that step.\n"
                + "  --check                 Parse and statically validate the run document without reading input data or writing output\n"
                + "  --var name=value        Supply a value for a declared typed parameter (repeatable)\n"
                + "  --output, -o PATH       Output destination, relative to the caller. Replaces the selected stage's single dump path and retains its format; defaults to CSV when adding an output. Existing files are rejected.\n"
                + "  --show-plan STAGE       Print a selected process, join, or concat stage plan without evaluating rows or running finalizers. Dynamic branch stages are rejected.\n"
                + "A parameter path default is relative to the run file; a --var path is relative to the caller.\n"
                + "Parameter placeholders are whole YAML values: {\"$param\": name}; partial interpolation is rejected.\n",
        );
    }
    let details = match name {
        "count" => "\nCount repeated rows with `count`, or count values in selected columns with `count <columns>` (for example, `count hoge`). Select multiple columns to count each combination (for example, `select hoge,fuga - count`). Results are sorted by count, highest first; null values form a group and are counted.\n",
        "load" => "\nCSV and JSONL: full schema discovery and strict record validation by default.\n  --infer-schema-length full|N  Bound discovery but validate all later records.\n  --all-strings                 Read CSV fields as String, preserving identifiers.\n  --schema column=type          Override a CSV column (repeatable): string,int,uint,float,bool.\n  --stdin-format csv|tsv|jsonl   Read finite stdin instead of files.\n  --rejects PATH                Quarantine rejected records in a new JSONL report.\nCSV inference preserves leading-zero identifiers and integers outside a lossless numeric common type as strings. Unsupported encodings fail with the source path. JSONL duplicate keys and incompatible types fail unless quarantined. Gzip/stdin/reject filtering use managed temporary disk.\n",
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
        "isin" => "\nMembership keys must be scalar; nested columns are rejected. Use flatten for a scalar field or grep/contains for JSON text search.\n",
        _ => "",
    };
    Some(format!(
        "{}\n\nUsage: {}\n{}\n{details}",
        s.name, s.help, s.summary
    ))
}
