//! Document/schema and parameter-preflight boundary.

use super::super::model::{
    BranchPredicate, ParameterDeclaration, ParameterLiteral, ParameterOrigin, ParameterType,
    ResolvedParameter, RunDocument, StageConfig,
};
use super::diagnostics::{
    parse_stage_location, parse_step_location, sensitive_location, DiagnosticPolicy,
};
use super::planner;
use crate::controllers::command_model::{parse_automation_step, CommandCategory, TypedCommand};
use crate::error::QuiltError;
use serde_yml::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Debug)]
pub(super) struct PreparedRunDocument {
    pub(super) document: RunDocument,
    pub(super) parameters: HashMap<String, ResolvedParameter>,
    pub(super) diagnostics: DiagnosticPolicy,
}

pub(super) fn diagnostic(
    file: &Path,
    yaml_path: impl AsRef<str>,
    message: impl Into<String>,
) -> String {
    format!(
        "{}: {}: {}",
        file.display(),
        yaml_path.as_ref(),
        message.into()
    )
}

pub(super) fn parse_steps(steps: &Value) -> Result<Vec<(usize, String, Value)>, QuiltError> {
    match steps {
        Value::Sequence(sequence) => {
            let mut parsed_steps = Vec::with_capacity(sequence.len());

            for (index, step) in sequence.iter().enumerate() {
                match step {
                    Value::Mapping(mapping) if mapping.len() == 1 => {
                        if let Some((command_name_val, command_args_val)) = mapping.iter().next() {
                            let command_name = command_name_val.as_str().ok_or_else(|| {
                                QuiltError::usage(format!(
                                    "Step {} command name must be a string.",
                                    index + 1
                                ))
                            })?;
                            parsed_steps.push((
                                index,
                                command_name.to_string(),
                                command_args_val.clone(),
                            ));
                        }
                    }
                    Value::Mapping(mapping) => {
                        return Err(QuiltError::usage(format!(
                            "Step {} must contain exactly one command entry, found {}.",
                            index + 1,
                            mapping.len()
                        )));
                    }
                    _ => {
                        return Err(QuiltError::usage(format!(
                            "Step {} must be a single-entry mapping like '- grep: {{...}}'.",
                            index + 1
                        )));
                    }
                }
            }

            Ok(parsed_steps)
        }
        _ => Err(QuiltError::usage(
            "Process stage 'steps' must be a sequence.",
        )),
    }
}

pub(super) fn step_field_path(
    stage_path: &str,
    index: usize,
    command: &str,
    args: &Value,
    message: &str,
) -> String {
    let base = format!("{stage_path}.steps[{index}].{command}");
    if let Some((marker, start)) = ["unknown field '", "Unknown key '"]
        .iter()
        .find_map(|marker| message.find(marker).map(|start| (*marker, start)))
    {
        let rest = &message[start + marker.len()..];
        if let Some(end) = rest.find('\'') {
            return format!("{base}.{}", &rest[..end]);
        }
    }
    if message.contains("invalid type")
        || message.contains("expected")
        || message.contains("requires a valid")
    {
        if let Some(mapping) = args.as_mapping() {
            for key in ["number", "interval", "separator", "type"] {
                if mapping.contains_key(Value::String(key.to_string())) {
                    return format!("{base}.{key}");
                }
            }
        }
    }
    base
}

pub(super) fn resolve_parameterized_value(
    value: &mut Value,
    values: &HashMap<String, ResolvedParameter>,
    path: &str,
    diagnostics: &mut DiagnosticPolicy,
    stage_names: &HashMap<usize, String>,
) -> Result<(), String> {
    match value {
        Value::Mapping(mapping) => {
            if mapping.len() == 1 && mapping.contains_key(Value::String("$param".into())) {
                let name = mapping
                    .get(Value::String("$param".into()))
                    .and_then(Value::as_str)
                    .ok_or_else(|| "$param must name a string parameter".to_string())?;
                if values.get(name).is_some_and(|parameter| parameter.secret) {
                    if let Some(location) = sensitive_location(path) {
                        diagnostics.sensitive_locations.insert(location);
                    }
                    if let Some((stage_index, step_index)) = parse_step_location(path) {
                        if let Some(stage) = stage_names.get(&stage_index) {
                            diagnostics
                                .sensitive_steps
                                .insert((stage.clone(), step_index));
                        }
                    } else if let Some(stage_index) = parse_stage_location(path) {
                        diagnostics.sensitive_stage_indices.insert(stage_index);
                        if let Some(stage) = stage_names.get(&stage_index) {
                            diagnostics.sensitive_stages.insert(stage.clone());
                        }
                    }
                }
                *value = values
                    .get(name)
                    .map(|value| value.value.yaml_value())
                    .ok_or_else(|| format!("{path}: unresolved parameter '{name}'"))?;
                return Ok(());
            }
            if mapping
                .keys()
                .any(|key| key.as_str().is_some_and(|key| key.contains("$param")))
            {
                return Err(format!("{path}: $param must be a whole mapping value"));
            }
            for (key, child) in mapping.iter_mut() {
                let child_path = format!("{path}.{}", key.as_str().unwrap_or("<key>"));
                resolve_parameterized_value(child, values, &child_path, diagnostics, stage_names)?;
            }
        }
        Value::Sequence(sequence) => {
            for (index, child) in sequence.iter_mut().enumerate() {
                resolve_parameterized_value(
                    child,
                    values,
                    &format!("{path}[{index}]"),
                    diagnostics,
                    stage_names,
                )?;
            }
        }
        Value::String(string) if string.contains("${") => {
            return Err(format!(
                "{path}: legacy or partial parameter interpolation is not supported"
            ))
        }
        _ => {}
    }
    Ok(())
}

#[derive(Debug)]
pub(super) struct DocumentInput<'a> {
    pub raw: &'a str,
    pub path: &'a Path,
    pub overrides: &'a [String],
}

pub(super) fn preflight(
    document: &RunDocument,
    parameters: &HashMap<String, ResolvedParameter>,
    config_path: &Path,
) -> Result<(), QuiltError> {
    let mut diagnostics = Vec::new();
    let mut names = HashMap::new();
    for (index, stage) in document.stages.iter().enumerate() {
        let stage_path = format!("stages[{index}]");
        let name = stage.name();
        if name.trim().is_empty() {
            diagnostics.push(diagnostic(
                config_path,
                format!("{stage_path}.name"),
                "must be non-empty",
            ));
        }
        if let Some(previous) = names.insert(name.to_string(), index) {
            diagnostics.push(diagnostic(
                config_path,
                format!("{stage_path}.name"),
                format!("duplicates stages[{previous}]"),
            ));
        }
        let dependencies = stage.dependencies();
        for (dep_index, dependency) in dependencies.iter().enumerate() {
            if dependency.trim().is_empty() {
                diagnostics.push(diagnostic(
                    config_path,
                    format!("{stage_path}.dependencies[{dep_index}]"),
                    "must be non-empty",
                ));
            }
            if !names.contains_key(dependency)
                && !document
                    .stages
                    .iter()
                    .any(|candidate| candidate.name() == dependency)
            {
                diagnostics.push(diagnostic(
                    config_path,
                    format!("{stage_path}.dependencies[{dep_index}]"),
                    format!("missing stage '{dependency}'"),
                ));
            }
        }
        match stage {
            StageConfig::Process(process) => {
                let mut finalizer_seen = false;
                for (step_index, raw_step) in process.steps.iter().enumerate() {
                    match parse_steps(&Value::Sequence(vec![raw_step.clone()])) {
                        Ok(parsed) => {
                            if let Some((_, command, args)) = parsed.into_iter().next() {
                                match parse_automation_step(&command, &args) {
                                    Ok(typed) => {
                                        if let TypedCommand::Bucket(bucket) = &typed {
                                            if let Err(error) = crate::operations::chainables::bucket::validate_interval(&bucket.interval) {
                                                diagnostics.push(diagnostic(config_path, format!("{stage_path}.steps[{step_index}].bucket.interval"), error.to_string()));
                                            }
                                        }
                                        if finalizer_seen
                                            && !matches!(
                                                typed.category(),
                                                CommandCategory::Finalizer
                                            )
                                        {
                                            diagnostics.push(diagnostic(
                                                config_path,
                                                format!(
                                                    "{stage_path}.steps[{step_index}].{command}"
                                                ),
                                                "record step cannot follow a finalizer",
                                            ));
                                        }
                                        finalizer_seen |=
                                            matches!(typed.category(), CommandCategory::Finalizer);
                                    }
                                    Err(error) => diagnostics.push(diagnostic(
                                        config_path,
                                        step_field_path(
                                            &stage_path,
                                            step_index,
                                            &command,
                                            &args,
                                            &error.to_string(),
                                        ),
                                        error.to_string(),
                                    )),
                                }
                            }
                        }
                        Err(error) => diagnostics.push(diagnostic(
                            config_path,
                            format!("{stage_path}.steps[{step_index}]"),
                            error.to_string(),
                        )),
                    }
                }
            }
            StageConfig::Join(join) => {
                if join.join.inputs.len() < 2 {
                    diagnostics.push(diagnostic(
                        config_path,
                        format!("{stage_path}.join.inputs"),
                        "must have at least two inputs",
                    ));
                }
                let how = join
                    .join
                    .how
                    .as_deref()
                    .unwrap_or("inner")
                    .to_ascii_lowercase();
                if !matches!(how.as_str(), "inner" | "left" | "full" | "outer" | "cross") {
                    diagnostics.push(diagnostic(
                        config_path,
                        format!("{stage_path}.join.how"),
                        format!("Unsupported join type '{how}'"),
                    ));
                }
                let has_on = join.join.on.is_some();
                let has_left = join.join.left_on.is_some();
                let has_right = join.join.right_on.is_some();
                if (has_on && (has_left || has_right)) || has_left != has_right {
                    diagnostics.push(diagnostic(
                        config_path,
                        format!("{stage_path}.join"),
                        "must specify exactly one key mode",
                    ));
                }
                if let Some(keys) = join.join.on.as_ref() {
                    if keys.is_empty() || keys.iter().any(|key| key.trim().is_empty()) {
                        diagnostics.push(diagnostic(
                            config_path,
                            format!("{stage_path}.join.on"),
                            "must contain non-empty join keys",
                        ));
                    }
                }
                if let (Some(left), Some(right)) = (&join.join.left_on, &join.join.right_on) {
                    if left.is_empty()
                        || left.len() != right.len()
                        || left.iter().chain(right).any(|key| key.trim().is_empty())
                    {
                        diagnostics.push(diagnostic(
                            config_path,
                            format!("{stage_path}.join"),
                            "has invalid asymmetric join keys",
                        ));
                    }
                }
                if how == "cross" && (has_on || has_left) {
                    diagnostics.push(diagnostic(
                        config_path,
                        format!("{stage_path}.join"),
                        "cross join cannot specify join keys",
                    ));
                }
                if how != "cross" && !has_on && !has_left {
                    diagnostics.push(diagnostic(
                        config_path,
                        format!("{stage_path}.join"),
                        "requires join keys unless how is cross",
                    ));
                }
            }
            StageConfig::Concat(concat) => {
                if concat.concat.inputs.len() < 2 {
                    diagnostics.push(diagnostic(
                        config_path,
                        format!("{stage_path}.concat.inputs"),
                        "must have at least two inputs",
                    ));
                }
                let how = concat
                    .concat
                    .how
                    .as_deref()
                    .unwrap_or("vertical")
                    .to_ascii_lowercase();
                if !matches!(how.as_str(), "vertical" | "v") {
                    diagnostics.push(diagnostic(
                        config_path,
                        format!("{stage_path}.concat.how"),
                        format!("Unsupported concat type '{how}'"),
                    ));
                }
            }
            StageConfig::Branch(branch) => {
                let predicate_error = match &branch.branch.when {
                    BranchPredicate::RowCount { .. } => branch.branch.when.evaluate(0),
                    BranchPredicate::Parameter { .. } => branch
                        .branch
                        .when
                        .validate_parameter(
                            parameters,
                            config_path.parent().unwrap_or_else(|| Path::new(".")),
                        )
                        .map(|_| true),
                };
                if let Err(error) = predicate_error {
                    diagnostics.push(diagnostic(
                        config_path,
                        format!("{stage_path}.branch.when"),
                        error,
                    ));
                }
                let mut seen_targets = HashSet::new();
                let else_targets = branch.branch.r#else.as_deref().unwrap_or(&[]);
                for (route, targets) in [
                    ("then", branch.branch.then.as_slice()),
                    ("else", else_targets),
                ] {
                    for (target_index, target) in targets.iter().enumerate() {
                        if target.trim().is_empty() {
                            diagnostics.push(diagnostic(
                                config_path,
                                format!("{stage_path}.branch.{route}[{target_index}]"),
                                "must be non-empty",
                            ));
                        }
                        if !seen_targets.insert(target) {
                            diagnostics.push(diagnostic(
                                config_path,
                                format!("{stage_path}.branch.{route}[{target_index}]"),
                                "duplicate branch target",
                            ));
                        }
                        if !document
                            .stages
                            .iter()
                            .any(|candidate| candidate.name() == target)
                        {
                            diagnostics.push(diagnostic(
                                config_path,
                                format!("{stage_path}.branch.{route}[{target_index}]"),
                                format!("missing stage '{target}'"),
                            ));
                        }
                    }
                }
            }
        }
    }
    if let Err(error) = planner::build(document) {
        diagnostics.push(diagnostic(config_path, "stages", error.to_string()));
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(QuiltError::validation(diagnostics))
    }
}

pub(super) fn prepare(input: DocumentInput<'_>) -> Result<PreparedRunDocument, QuiltError> {
    let raw = input.raw;
    let config_path = input.path;
    let overrides = input.overrides;
    let mut root: Value = serde_yml::from_str(raw).map_err(|e| {
        QuiltError::automation("run", None, format!("Error parsing YAML config: {e}"))
    })?;
    let declarations: std::collections::BTreeMap<String, ParameterDeclaration> = root
        .get("parameters")
        .and_then(|value| serde_yml::from_value(value.clone()).ok())
        .unwrap_or_default();
    let config_base = config_path.parent().unwrap_or_else(|| Path::new("."));
    let mut values: HashMap<String, ResolvedParameter> = HashMap::new();
    let mut diagnostics = Vec::new();
    if let Some(raw_parameters) = root.get("parameters").and_then(Value::as_mapping) {
        for (raw_name, raw_declaration) in raw_parameters {
            let Some(name) = raw_name.as_str() else {
                diagnostics.push(diagnostic(
                    config_path,
                    "parameters.<non-string>",
                    "parameter name must be a string",
                ));
                continue;
            };
            let Some(mapping) = raw_declaration.as_mapping() else {
                diagnostics.push(diagnostic(
                    config_path,
                    format!("parameters.{name}"),
                    "declaration must be a mapping",
                ));
                continue;
            };
            for key in mapping.keys().filter_map(Value::as_str) {
                if !matches!(key, "type" | "required" | "default" | "secret") {
                    diagnostics.push(diagnostic(
                        config_path,
                        format!("parameters.{name}.{key}"),
                        "unknown parameter declaration key",
                    ));
                }
            }
            let raw_type_value = mapping.get(Value::String("type".into()));
            if raw_type_value.is_none() {
                diagnostics.push(diagnostic(
                    config_path,
                    format!("parameters.{name}.type"),
                    "missing parameter type",
                ));
            }
            if let Some(raw_type) = raw_type_value {
                if !matches!(raw_type.as_str(), Some("path" | "string" | "int" | "bool")) {
                    diagnostics.push(diagnostic(
                        config_path,
                        format!("parameters.{name}.type"),
                        "type must be path, string, int, or bool",
                    ));
                }
            }
            for key in ["required", "secret"] {
                if let Some(value) = mapping.get(Value::String(key.to_string())) {
                    if !value.is_bool() {
                        diagnostics.push(diagnostic(
                            config_path,
                            format!("parameters.{name}.{key}"),
                            "must be boolean",
                        ));
                    }
                }
            }
        }
    }
    if !diagnostics.is_empty() {
        return Err(QuiltError::validation(diagnostics));
    }
    for (name, declaration) in &declarations {
        if name.trim().is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            diagnostics.push(diagnostic(
                config_path,
                format!("parameters.{name}"),
                format!("invalid parameter name '{name}'"),
            ));
            continue;
        }
        if let Some(default) = &declaration.default {
            let parsed_default = default
                .value_for(declaration.parameter_type, config_base)
                .map_err(|e| {
                    diagnostics.push(diagnostic(
                        config_path,
                        format!("parameters.{name}.default"),
                        e,
                    ));
                    "invalid default"
                });
            let Ok(parsed_default) = parsed_default else {
                continue;
            };
            values.insert(
                name.clone(),
                ResolvedParameter {
                    parameter_type: declaration.parameter_type,
                    value: parsed_default,
                    origin: ParameterOrigin::Default,
                    secret: declaration.secret,
                },
            );
        }
        if declaration.required && declaration.default.is_some() {
            diagnostics.push(diagnostic(
                config_path,
                format!("parameters.{name}.required"),
                "required parameter cannot have a default",
            ));
        }
    }
    let mut seen = HashSet::new();
    for (index, override_value) in overrides.iter().enumerate() {
        let Some((name, raw_value)) = override_value.split_once('=') else {
            diagnostics.push(diagnostic(
                config_path,
                format!("--var[{index}]"),
                "--var expects name=value",
            ));
            continue;
        };
        if !seen.insert(name) {
            diagnostics.push(diagnostic(
                config_path,
                format!("--var[{index}]"),
                format!("Duplicate parameter override '{name}'"),
            ));
            continue;
        }
        let Some(declaration) = declarations.get(name) else {
            diagnostics.push(diagnostic(
                config_path,
                format!("--var[{index}]"),
                format!("Unknown parameter '{name}'"),
            ));
            continue;
        };
        let literal = match declaration.parameter_type {
            ParameterType::Int => match raw_value.parse::<i64>() {
                Ok(value) => ParameterLiteral::Int(value),
                Err(_) => {
                    diagnostics.push(diagnostic(
                        config_path,
                        format!("--var[{index}].{name}"),
                        "Invalid integer parameter",
                    ));
                    continue;
                }
            },
            ParameterType::Bool => match raw_value.parse::<bool>() {
                Ok(value) => ParameterLiteral::Bool(value),
                Err(_) => {
                    diagnostics.push(diagnostic(
                        config_path,
                        format!("--var[{index}].{name}"),
                        "Invalid boolean parameter",
                    ));
                    continue;
                }
            },
            ParameterType::Path | ParameterType::String => {
                ParameterLiteral::String(raw_value.to_string())
            }
        };
        let parsed = literal
            .value_for(
                declaration.parameter_type,
                if declaration.parameter_type == ParameterType::Path {
                    Path::new(".")
                } else {
                    config_base
                },
            )
            .map_err(|e| {
                diagnostics.push(diagnostic(config_path, format!("--var[{index}].{name}"), e));
            })
            .ok();
        let Some(parsed) = parsed else {
            continue;
        };
        values.insert(
            name.to_string(),
            ResolvedParameter {
                parameter_type: declaration.parameter_type,
                value: parsed,
                origin: ParameterOrigin::Cli,
                secret: declaration.secret,
            },
        );
    }
    for (name, declaration) in &declarations {
        if declaration.required && !values.contains_key(name) {
            diagnostics.push(diagnostic(
                config_path,
                format!("parameters.{name}.required"),
                format!("Missing required parameter '{name}'"),
            ));
        }
    }
    if !diagnostics.is_empty() {
        return Err(QuiltError::validation(diagnostics));
    }
    let mut diagnostics = DiagnosticPolicy::from_values(
        values
            .values()
            .filter(|parameter| parameter.secret)
            .map(|parameter| parameter.value.redaction_value()),
    );
    let stage_names = root
        .get("stages")
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, stage)| {
            stage
                .get("name")
                .and_then(Value::as_str)
                .map(|name| (index, name.to_string()))
        })
        .collect::<HashMap<_, _>>();
    resolve_parameterized_value(&mut root, &values, "run", &mut diagnostics, &stage_names)
        .map_err(|error| {
            QuiltError::validation(vec![diagnostic(config_path, "parameters", error)])
        })?;
    if let Some(stages) = root.get("stages").and_then(Value::as_sequence) {
        for (index, stage) in stages.iter().enumerate() {
            if let Some(materialize) = stage.get("materialize") {
                let valid = materialize
                    .as_str()
                    .is_some_and(|value| matches!(value, "auto" | "always" | "never"));
                if !valid {
                    diagnostics
                        .sensitive_locations
                        .insert(format!("stages[{index}].materialize"));
                    return Err(QuiltError::validation(vec![diagnostic(
                        config_path,
                        format!("stages[{index}].materialize"),
                        "materialize must be auto, always, or never",
                    )]));
                }
            }
        }
    }
    let document: RunDocument = serde_yml::from_value(root).map_err(|e| {
        QuiltError::automation(
            "run",
            None,
            format!("Error parsing run document {}: {e}", config_path.display()),
        )
    })?;
    for stage_index in diagnostics.sensitive_stage_indices.iter().copied() {
        if let Some(stage) = document.stages.get(stage_index) {
            diagnostics
                .sensitive_stages
                .insert(stage.name().to_string());
        }
    }
    Ok(PreparedRunDocument {
        document,
        parameters: values,
        diagnostics,
    })
}

pub(super) fn schema_version(document: &RunDocument) -> Result<(), QuiltError> {
    if document.version == 1 {
        Ok(())
    } else {
        Err(QuiltError::automation(
            "run",
            None,
            format!(
                "Unsupported run document version {} (expected 1)",
                document.version
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{prepare, schema_version, DocumentInput};
    use std::path::Path;

    #[test]
    fn document_boundary_validates_version_without_execution() {
        let prepared = prepare(DocumentInput {
            raw: "version: 1\nstages: []",
            path: Path::new("run.yaml"),
            overrides: &[],
        })
        .unwrap();
        schema_version(&prepared.document).unwrap();
    }
}
