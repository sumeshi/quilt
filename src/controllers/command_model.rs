pub use crate::controllers::arguments::*;

pub use crate::controllers::definitions::{
    automation_operations, by_name, command_specs, CommandCategory, CommandSpec,
    OperationDefinition, OperationId, OptionSpec, TypedCommand,
};
pub fn automation_record_command_names() -> impl Iterator<Item = &'static str> {
    crate::controllers::definitions::record_operations().map(|spec| spec.name)
}

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
            out.push_str(&format!("  {:12} {}\n", s.name, s.help));
        }
    }
    out
}
pub fn render_command_help(name: &str) -> Option<String> {
    let s = crate::controllers::definitions::by_name(name)?;
    if name == "run" {
        return Some(
            "run\n\nUsage: qlt run <config> [files...] [options]\n\n"
                .to_string()
                + "Execute a version-1 YAML workflow. If a load step omits paths, positional files "
                + "after the config path are supplied to that step.\n"
                + "  --check                 Parse and statically validate the run document without reading input data or writing output\n"
                + "  --var name=value        Supply a value for a declared typed parameter (repeatable)\n"
                + "  --output, -o PATH       CSV destination. If the YAML has a dump, this path replaces it; otherwise this is the dump path. Relative paths are from the run file directory. Existing files are rejected.\n"
                + "  --show-plan STAGE       Print a selected process, join, or concat stage plan without evaluating rows or running finalizers. Dynamic branch stages are rejected.\n"
                + "A parameter path default is relative to the run file; a --var path is relative to the caller.\n"
                + "Parameter placeholders are whole YAML values: {\"$param\": name}; partial interpolation is rejected.\n",
        );
    }
    let details = match name {
        "load" => "\nJSONL/NDJSON (including .jsonl.gz/.ndjson.gz): full, strict schema validation by default.\n  --infer-schema-length full|N  N limits schema discovery, but all records are validated; late fields error.\nDuplicate object keys are rejected with input path, line, and key path.\nInteger/float mixtures and incompatible signed/unsigned ranges are rejected without precision loss.\nGzip input is spooled to temporary disk; inference retains one record plus schema in memory.\n",
        "showtable" => "\nStruct fields are displayed as dot paths; lists stay JSON cells. Preview: 8 rows, 8 columns, 64 display columns per cell/header. Omitted content is marked. Controls are escaped. Use select to choose visible columns, show/dump for complete CSV output.\n",
        "flatten" => "\nLazily project recursive Struct fields as dot paths; preserve lists and non-null empty objects. Path collisions are errors. Lists are never exploded.\n",
        "show" | "dump" => "\nCSV output: nested objects/lists are JSON strings inside quoted CSV cells; nulls are empty. Use flatten to expose object fields as columns, dumpcache for typed Parquet.\n",
        "select" => "\nExact existing names take precedence over range syntax. Quote comma-containing names inside the argument, e.g. select '\"a,b\",xmlns:auto-ns3'. YAML columns sequences preserve literal names.\n",
        "grep" | "contains" => "\nNested columns are searched through their JSON representation (keys and values, JSON escaping included); matched rows retain original types. Use flatten for scalar-field search.\n",
        "sed" => "\nGlobal replacement leaves nested columns unchanged; selecting a nested column explicitly is an error. Use flatten to target scalar fields.\n",
        "isin" => "\nMembership keys must be scalar; nested columns are rejected. Use flatten for a scalar field or grep/contains for JSON text search.\n",
        _ => "",
    };
    Some(format!("{}\n\nUsage: {}\n{details}", s.name, s.help))
}
