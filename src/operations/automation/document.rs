//! Document/schema and parameter-preflight boundary.

use super::super::model::{
    BranchPredicate, ParameterDeclaration, ParameterLiteral, ParameterOrigin, ParameterType,
    PlannedStep, ProcessStage, ResolvedParameter, RunDocument, StageConfig,
};
use super::diagnostics::{
    parse_stage_location, parse_step_location, sensitive_location, DiagnosticPolicy,
};
use super::planner;
use crate::controllers::command_model::{parse_automation_step, CommandCategory, TypedCommand};
use crate::error::QuiltError;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use yaml_serde::Value;

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

pub(super) fn with_source_locations(error: QuiltError, raw: &str, file: &Path) -> QuiltError {
    let QuiltError::Validation { diagnostics } = error else {
        return error;
    };
    let prefix = format!("{}: ", file.display());
    QuiltError::validation(
        diagnostics
            .into_iter()
            .map(|diagnostic| {
                let Some((path, message)) = diagnostic
                    .strip_prefix(&prefix)
                    .and_then(|s| s.split_once(": "))
                else {
                    return diagnostic;
                };
                let mut parent = path;
                let position = loop {
                    if let Some(position) = super::location::source_location(raw, parent) {
                        break Some(position);
                    }
                    match parent.rfind(['.', '[']) {
                        Some(end) => parent = &parent[..end],
                        None => break None,
                    }
                };
                match position {
                    Some((line, column)) => {
                        format!("{}:{line}:{column}: {path}: {message}", file.display())
                    }
                    None => diagnostic,
                }
            })
            .collect(),
    )
}

fn serde_field_path(path: &str, message: &str) -> String {
    let path = path.trim_start_matches('.');
    for prefix in ["unknown field `", "missing field `"] {
        if let Some(rest) = message.strip_prefix(prefix) {
            if let Some((field, _)) = rest.split_once('`') {
                if path == field || path.ends_with(&format!(".{field}")) {
                    return path.to_owned();
                }
                return if path.is_empty() {
                    field.to_owned()
                } else {
                    format!("{path}.{field}")
                };
            }
        }
    }
    path.to_owned()
}

fn parse_step(step: &Value, index: usize) -> Result<(&str, &Value), QuiltError> {
    let mapping = step.as_mapping().ok_or_else(|| {
        QuiltError::usage(format!(
            "Step {} must be a single-entry mapping like '- grep: {{...}}'.",
            index + 1
        ))
    })?;
    if mapping.len() != 1 {
        return Err(QuiltError::usage(format!(
            "Step {} must contain exactly one command entry, found {}.",
            index + 1,
            mapping.len()
        )));
    }
    let (name, args) = mapping.iter().next().unwrap();
    let name = name.as_str().ok_or_else(|| {
        QuiltError::usage(format!("Step {} command name must be a string.", index + 1))
    })?;
    Ok((name, args))
}

fn resolve_command_paths(command: &mut TypedCommand, config_path: &Path) {
    let base = config_path.parent().unwrap_or_else(|| Path::new("."));
    match command {
        TypedCommand::Load(args) => {
            if let Some(path) = &mut args.rejects {
                if path.is_relative() {
                    *path = base.join(&*path);
                }
            }
            for path in &mut args.paths {
                if path.is_relative() {
                    *path = base.join(&*path);
                }
            }
        }
        TypedCommand::Dump(crate::controllers::command_model::DumpArgs {
            output: Some(path),
            ..
        })
        | TypedCommand::DumpCache(crate::controllers::command_model::DumpCacheArgs {
            output: Some(path),
        }) => {
            if Path::new(path).is_relative() && path != "-" {
                *path = base.join(&*path).to_string_lossy().into_owned();
            }
        }
        TypedCommand::Partition(args) if Path::new(&args.output_dir).is_relative() => {
            args.output_dir = base.join(&args.output_dir).to_string_lossy().into_owned();
        }
        _ => {}
    }
}

pub(super) fn step_field_path(
    stage_path: &str,
    index: usize,
    command: &str,
    error: &QuiltError,
) -> String {
    let base = format!("{stage_path}.steps[{index}].{command}");
    match error {
        QuiltError::Config { path, .. } if !path.is_empty() && path != "." && path != command => {
            if path.starts_with('[') {
                format!("{base}{path}")
            } else {
                format!("{base}.{path}")
            }
        }
        _ => base,
    }
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
            // A load schema is itself a mapping of literal column names. Do
            // not interpret its top-level `$param` column as a placeholder;
            // values within the mapping still use the normal resolver.
            let is_load_schema = path.ends_with(".load.schema");
            if !is_load_schema
                && mapping.len() == 1
                && mapping.contains_key(Value::String("$param".into()))
            {
                let name = mapping
                    .get(Value::String("$param".into()))
                    .and_then(Value::as_str)
                    .ok_or_else(|| "$param must name a string parameter".to_string())?;
                if values.get(name).is_some_and(|parameter| parameter.secret) {
                    diagnostics.sensitive_locations.insert(
                        sensitive_location(path).unwrap_or_else(|| {
                            path.strip_prefix("run.").unwrap_or(path).to_string()
                        }),
                    );
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
            // Schema mappings describe literal column names, including keys
            // containing `$param`; elsewhere an extra `$param` key is a
            // malformed attempt at a whole-value placeholder.
            if !is_load_schema && mapping.keys().any(|key| key.as_str() == Some("$param")) {
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
) -> Result<planner::ExecutionPlan, QuiltError> {
    let mut diagnostics = Vec::new();
    let mut stages = Vec::with_capacity(document.stages.len());
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
                let mut steps = Vec::with_capacity(process.steps.len());
                let mut finalizer_seen = false;
                for (step_index, raw_step) in process.steps.iter().enumerate() {
                    let (command, args) = match parse_step(raw_step, step_index) {
                        Ok(parsed) => parsed,
                        Err(error) => {
                            diagnostics.push(diagnostic(
                                config_path,
                                format!("{stage_path}.steps[{step_index}]"),
                                error.to_string(),
                            ));
                            continue;
                        }
                    };
                    match parse_automation_step(command, args) {
                        Ok(mut typed) => {
                            if finalizer_seen && typed.category() != CommandCategory::Finalizer {
                                diagnostics.push(diagnostic(
                                    config_path,
                                    format!("{stage_path}.steps[{step_index}].{command}"),
                                    "record step cannot follow a finalizer",
                                ));
                            }
                            finalizer_seen |= typed.category() == CommandCategory::Finalizer;
                            let load_paths_omitted = matches!(&typed, TypedCommand::Load(load) if load.stdin_format.is_none())
                                && args.get("paths").is_none();
                            resolve_command_paths(&mut typed, config_path);
                            steps.push(PlannedStep {
                                index: step_index,
                                command: typed,
                                load_paths_omitted,
                            });
                        }
                        Err(error) => {
                            let message = match &error {
                                QuiltError::Config { message, .. } => message.clone(),
                                _ => error.to_string(),
                            };
                            diagnostics.push(diagnostic(
                                config_path,
                                step_field_path(&stage_path, step_index, command, &error),
                                message,
                            ));
                        }
                    }
                }
                stages.push(StageConfig::Process(ProcessStage {
                    name: process.name.clone(),
                    source: process.source.clone(),
                    steps,
                    materialize: process.materialize,
                }));
            }
            StageConfig::Join(join) => {
                stages.push(StageConfig::Join(join.clone()));
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
                stages.push(StageConfig::Concat(concat.clone()));
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
                stages.push(StageConfig::Branch(branch.clone()));
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
    match planner::build(&stages) {
        Ok(plan) if diagnostics.is_empty() => Ok(plan),
        result => {
            if let Err(error) = result {
                diagnostics.push(diagnostic(config_path, "stages", error.to_string()));
            }
            Err(QuiltError::validation(diagnostics))
        }
    }
}

pub(super) fn prepare(input: DocumentInput<'_>) -> Result<PreparedRunDocument, QuiltError> {
    let raw = input.raw;
    let config_path = input.path;
    let overrides = input.overrides;
    let mut root: Value = yaml_serde::from_str(raw).map_err(|e| {
        QuiltError::automation("run", None, format!("Error parsing YAML config: {e}"))
    })?;
    let version = root.get("version").and_then(Value::as_u64);
    if version != Some(2) {
        return Err(QuiltError::validation(vec![diagnostic(
            config_path,
            "version",
            if version == Some(1) {
                "run document version 1 is no longer supported; migrate to version: 2, use arrays for multiple columns, and replace aggregate metric strings with mappings containing mode, column, and output"
            } else {
                "run documents require version: 2"
            },
        )]));
    }
    let declarations: std::collections::BTreeMap<String, ParameterDeclaration> = root
        .get("parameters")
        .and_then(|value| yaml_serde::from_value(value.clone()).ok())
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
                if !matches!(
                    key,
                    "type" | "required" | "default" | "secret" | "description"
                ) {
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
    let document: RunDocument = serde_path_to_error::deserialize(root).map_err(|e| {
        let message = e.inner().to_string();
        let path = serde_field_path(&e.path().to_string(), &message);
        let error = QuiltError::validation(vec![diagnostic(config_path, path, message)]);
        diagnostics.sanitize_step_error(error, !diagnostics.sensitive_locations.is_empty())
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

#[cfg(test)]
mod tests {
    use super::{prepare, DocumentInput};
    use std::path::Path;

    #[test]
    fn document_boundary_validates_version_without_execution() {
        let prepared = prepare(DocumentInput {
            raw: "version: 2\nstages: []",
            path: Path::new("run.yaml"),
            overrides: &[],
        })
        .unwrap();
        assert_eq!(prepared.document.version, 2);
    }
}
