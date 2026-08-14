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
                + "Execute a canonical version-1 YAML workflow. Paths in declared default values "
                + "are relative to the run file; --var path overrides are relative to the caller.\n"
                + "  --check                 Validate schema, parameters, graph, and commands without I/O\n"
                + "  --var name=value        Override a declared typed parameter (repeatable)\n"
                + "  --output, -o PATH       Override the YAML dump path, or write CSV if there is no dump\n"
                + "  --show-plan STAGE       Print a selected stage plan without evaluating rows\n"
                + "Parameter placeholders are whole YAML values: {\"$param\": name}; partial interpolation is rejected.\n",
        );
    }
    Some(format!("{}\n\nUsage: {}\n", s.name, s.help))
}
