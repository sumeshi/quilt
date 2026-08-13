use crate::controllers::command_model::{
    parse_automation_step, CommandCategory, DumpArgs, TypedCommand,
};
use crate::controllers::dataframe::DataFrameController;
use crate::controllers::executor::CommandExecutor;
use crate::controllers::log::LogController;
use crate::error::QuiltError;
use polars::prelude::{col, lit, JoinType, LazyFrame};
use serde::{Deserialize, Serialize};
use serde_yml::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunDocument {
    pub version: u32,
    pub title: Option<String>,
    pub description: Option<String>,
    pub author: Option<String>,
    pub stages: Vec<StageConfig>,
    #[serde(default)]
    pub parameters: std::collections::BTreeMap<String, ParameterDeclaration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParameterType {
    Path,
    String,
    Int,
    Bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParameterValue {
    Path(PathBuf),
    String(String),
    Int(i64),
    Bool(bool),
}

impl ParameterValue {
    fn yaml_value(&self) -> Value {
        match self {
            Self::Path(value) => Value::String(value.to_string_lossy().into_owned()),
            Self::String(value) => Value::String(value.clone()),
            Self::Int(value) => Value::Number((*value).into()),
            Self::Bool(value) => Value::Bool(*value),
        }
    }
    fn redaction_value(&self) -> String {
        match self {
            Self::Path(value) => value.to_string_lossy().into_owned(),
            Self::String(value) => value.clone(),
            Self::Int(value) => value.to_string(),
            Self::Bool(value) => value.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParameterOrigin {
    Default,
    Cli,
}

#[derive(Debug, Clone)]
struct ResolvedParameter {
    parameter_type: ParameterType,
    value: ParameterValue,
    origin: ParameterOrigin,
    secret: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ParameterDeclaration {
    #[serde(rename = "type")]
    pub parameter_type: ParameterType,
    #[serde(default)]
    pub required: bool,
    pub default: Option<ParameterLiteral>,
    #[serde(default)]
    pub secret: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParameterLiteral {
    Bool(bool),
    Int(i64),
    String(String),
}

impl ParameterLiteral {
    fn value_for(
        &self,
        parameter_type: ParameterType,
        base: &Path,
    ) -> Result<ParameterValue, String> {
        match (parameter_type, self) {
            (ParameterType::String, Self::String(value)) => {
                Ok(ParameterValue::String(value.clone()))
            }
            (ParameterType::Path, Self::String(value)) => {
                Ok(ParameterValue::Path(anchor_path(value, base)))
            }
            (ParameterType::Int, Self::Int(value)) => Ok(ParameterValue::Int(*value)),
            (ParameterType::Bool, Self::Bool(value)) => Ok(ParameterValue::Bool(*value)),
            (expected, actual) => Err(format!("expected {expected:?} value, got {actual:?}")),
        }
    }
}

struct PreparedRunDocument {
    document: RunDocument,
    parameters: HashMap<String, ResolvedParameter>,
    secret_values: Vec<String>,
}

fn anchor_path(value: &str, base: &Path) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(base)
            .join(path)
    }
}

fn resolve_parameterized_value(
    value: &mut Value,
    values: &HashMap<String, ResolvedParameter>,
    path: &str,
) -> Result<(), String> {
    match value {
        Value::Mapping(mapping) => {
            if mapping.len() == 1 && mapping.contains_key(Value::String("$param".into())) {
                let name = mapping
                    .get(Value::String("$param".into()))
                    .and_then(Value::as_str)
                    .ok_or_else(|| "$param must name a string parameter".to_string())?;
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
                resolve_parameterized_value(child, values, &child_path)?;
            }
        }
        Value::Sequence(sequence) => {
            for (index, child) in sequence.iter_mut().enumerate() {
                resolve_parameterized_value(child, values, &format!("{path}[{index}]"))?;
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

fn prepare_document(
    raw: &str,
    config_path: &Path,
    overrides: &[String],
) -> Result<PreparedRunDocument, QuiltError> {
    let mut root: Value = serde_yml::from_str(raw).map_err(|e| {
        QuiltError::automation("run", None, format!("Error parsing YAML config: {e}"))
    })?;
    let declarations: std::collections::BTreeMap<String, ParameterDeclaration> = root
        .get("parameters")
        .and_then(|value| serde_yml::from_value(value.clone()).ok())
        .unwrap_or_default();
    let config_base = config_path.parent().unwrap_or_else(|| Path::new("."));
    let mut values: HashMap<String, ResolvedParameter> = HashMap::new();
    let mut secret_values = Vec::new();
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
    secret_values.extend(
        values
            .values()
            .filter(|parameter| parameter.secret)
            .map(|parameter| parameter.value.redaction_value()),
    );
    resolve_parameterized_value(&mut root, &values, "run").map_err(|error| {
        QuiltError::validation(vec![diagnostic(config_path, "parameters", error)])
    })?;
    let document = serde_yml::from_value(root).map_err(|e| {
        QuiltError::automation(
            "run",
            None,
            format!("Error parsing run document {}: {e}", config_path.display()),
        )
    })?;
    Ok(PreparedRunDocument {
        document,
        parameters: values,
        secret_values,
    })
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum StageConfig {
    Process(ProcessStage),
    Join(JoinStage),
    Concat(ConcatStage),
    Branch(BranchStage),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ProcessStage {
    pub name: String,
    #[serde(rename = "from")]
    pub source: Option<String>,
    pub steps: Vec<Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct JoinStage {
    pub name: String,
    pub join: JoinNode,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct JoinNode {
    pub inputs: Vec<String>,
    pub how: Option<String>,
    #[serde(
        rename = "left-on",
        default,
        deserialize_with = "deserialize_optional_string_list"
    )]
    pub left_on: Option<Vec<String>>,
    #[serde(
        rename = "right-on",
        default,
        deserialize_with = "deserialize_optional_string_list"
    )]
    pub right_on: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_string_list")]
    pub on: Option<Vec<String>>,
    pub coalesce: Option<bool>,
}

fn deserialize_optional_string_list<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_yml::Value>::deserialize(deserializer)?;
    value
        .map(|value| match value {
            serde_yml::Value::String(value) => Ok(vec![value]),
            serde_yml::Value::Sequence(values) => values
                .into_iter()
                .map(|value| match value {
                    serde_yml::Value::String(value) => Ok(value),
                    _ => Err(serde::de::Error::custom("join key values must be strings")),
                })
                .collect(),
            _ => Err(serde::de::Error::custom(
                "join keys must be a string or sequence of strings",
            )),
        })
        .transpose()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ConcatStage {
    pub name: String,
    pub concat: ConcatNode,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ConcatNode {
    pub inputs: Vec<String>,
    pub how: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct BranchStage {
    pub name: String,
    pub branch: BranchNode,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct BranchNode {
    pub input: String,
    pub when: BranchPredicate,
    pub then: Vec<String>,
    pub r#else: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum BranchPredicate {
    RowCount {
        #[serde(rename = "row-count")]
        row_count: RowCountPredicate,
    },
    Parameter {
        parameter: ParameterPredicate,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct RowCountPredicate {
    #[serde(rename = "equal")]
    pub equal: Option<i64>,
    #[serde(rename = "not-equal")]
    pub not_equal: Option<i64>,
    #[serde(rename = "greater-than")]
    pub greater_than: Option<i64>,
    #[serde(rename = "greater-or-equal")]
    pub greater_or_equal: Option<i64>,
    #[serde(rename = "less-than")]
    pub less_than: Option<i64>,
    #[serde(rename = "less-or-equal")]
    pub less_or_equal: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ParameterPredicate {
    pub name: String,
    pub equal: Option<ParameterLiteral>,
    #[serde(rename = "not-equal")]
    pub not_equal: Option<ParameterLiteral>,
    #[serde(rename = "greater-than")]
    pub greater_than: Option<ParameterLiteral>,
    #[serde(rename = "greater-or-equal")]
    pub greater_or_equal: Option<ParameterLiteral>,
    #[serde(rename = "less-than")]
    pub less_than: Option<ParameterLiteral>,
    #[serde(rename = "less-or-equal")]
    pub less_or_equal: Option<ParameterLiteral>,
}

impl BranchPredicate {
    fn evaluate(&self, rows: usize) -> Result<bool, String> {
        let BranchPredicate::RowCount { row_count: p } = self else {
            return Err("parameter branch predicate requires resolved parameters".into());
        };
        let values = [
            p.equal.map(|v| (rows as i64) == v),
            p.not_equal.map(|v| (rows as i64) != v),
            p.greater_than.map(|v| (rows as i64) > v),
            p.greater_or_equal.map(|v| (rows as i64) >= v),
            p.less_than.map(|v| (rows as i64) < v),
            p.less_or_equal.map(|v| (rows as i64) <= v),
        ];
        let values = values.into_iter().flatten().collect::<Vec<_>>();
        if values.len() != 1 {
            return Err("row-count predicate requires exactly one comparison".into());
        }
        Ok(values[0])
    }

    fn validate_parameter(
        &self,
        parameters: &HashMap<String, ResolvedParameter>,
        run_base: &Path,
    ) -> Result<(), String> {
        let BranchPredicate::Parameter { parameter } = self else {
            return Ok(());
        };
        let resolved = parameters
            .get(&parameter.name)
            .ok_or_else(|| format!("unknown or unresolved parameter '{}'", parameter.name))?;
        let operators = [
            parameter.equal.as_ref(),
            parameter.not_equal.as_ref(),
            parameter.greater_than.as_ref(),
            parameter.greater_or_equal.as_ref(),
            parameter.less_than.as_ref(),
            parameter.less_or_equal.as_ref(),
        ];
        let literal_count = operators.iter().flatten().count();
        if literal_count != 1 {
            return Err("parameter predicate requires exactly one comparison".into());
        }
        let literal = operators
            .into_iter()
            .flatten()
            .next()
            .ok_or_else(|| "parameter predicate requires exactly one comparison".to_string())?;
        let expected = literal
            .value_for(
                resolved.parameter_type,
                match resolved.origin {
                    ParameterOrigin::Default => run_base,
                    ParameterOrigin::Cli => Path::new("."),
                },
            )
            .map_err(|_| {
                format!(
                    "parameter predicate literal type does not match parameter '{}'",
                    parameter.name
                )
            })?;
        if std::mem::discriminant(&expected) != std::mem::discriminant(&resolved.value) {
            return Err(format!(
                "parameter predicate literal type does not match parameter '{}'",
                parameter.name
            ));
        }
        if matches!(resolved.value, ParameterValue::Bool(_))
            && (parameter.greater_than.is_some()
                || parameter.greater_or_equal.is_some()
                || parameter.less_than.is_some()
                || parameter.less_or_equal.is_some())
        {
            return Err("boolean parameters support only equal/not-equal predicates".into());
        }
        Ok(())
    }

    fn evaluate_parameter(
        &self,
        parameters: &HashMap<String, ResolvedParameter>,
        run_base: &Path,
    ) -> Result<bool, String> {
        let BranchPredicate::Parameter { parameter } = self else {
            return Err("not a parameter predicate".into());
        };
        self.validate_parameter(parameters, run_base)?;
        let resolved = parameters
            .get(&parameter.name)
            .ok_or_else(|| format!("unknown or unresolved parameter '{}'", parameter.name))?;
        let compare = |literal: &ParameterLiteral| -> Result<std::cmp::Ordering, String> {
            let expected = literal.value_for(
                resolved.parameter_type,
                match resolved.origin {
                    ParameterOrigin::Default => run_base,
                    ParameterOrigin::Cli => Path::new("."),
                },
            )?;
            match (&resolved.value, &expected) {
                (ParameterValue::Int(a), ParameterValue::Int(b)) => Ok(a.cmp(b)),
                (ParameterValue::String(a), ParameterValue::String(b)) => Ok(a.cmp(b)),
                (ParameterValue::Path(a), ParameterValue::Path(b)) => Ok(a.cmp(b)),
                (ParameterValue::Bool(a), ParameterValue::Bool(b)) => Ok(a.cmp(b)),
                _ => Err("parameter predicate literal type mismatch".into()),
            }
        };
        if let Some(v) = &parameter.equal {
            return Ok(compare(v)? == std::cmp::Ordering::Equal);
        }
        if let Some(v) = &parameter.not_equal {
            return Ok(compare(v)? != std::cmp::Ordering::Equal);
        }
        if let Some(v) = &parameter.greater_than {
            return Ok(compare(v)? == std::cmp::Ordering::Greater);
        }
        if let Some(v) = &parameter.greater_or_equal {
            return Ok(compare(v)? != std::cmp::Ordering::Less);
        }
        if let Some(v) = &parameter.less_than {
            return Ok(compare(v)? == std::cmp::Ordering::Less);
        }
        if let Some(v) = &parameter.less_or_equal {
            return Ok(compare(v)? != std::cmp::Ordering::Greater);
        }
        Err("parameter predicate requires exactly one comparison".into())
    }
}

impl StageConfig {
    fn name(&self) -> &str {
        match self {
            Self::Process(stage) => &stage.name,
            Self::Join(stage) => &stage.name,
            Self::Concat(stage) => &stage.name,
            Self::Branch(stage) => &stage.name,
        }
    }
    fn dependencies(&self) -> Vec<String> {
        match self {
            Self::Process(stage) => stage.source.clone().into_iter().collect(),
            Self::Join(stage) => stage.join.inputs.clone(),
            Self::Concat(stage) => stage.concat.inputs.clone(),
            Self::Branch(stage) => vec![stage.branch.input.clone()],
        }
    }
}

fn parse_steps(steps: &Value) -> Result<Vec<(usize, String, Value)>, QuiltError> {
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

#[cfg(test)]
fn validate_step_sequence(stage_name: &str, steps: &[Value]) -> Result<(), QuiltError> {
    let parsed = parse_steps(&Value::Sequence(steps.to_vec()))?;
    let mut finalizer_seen = false;
    for (index, command_name, args) in parsed {
        let command = parse_automation_step(&command_name, &args)
            .map_err(|error| step_error(stage_name, index, &command_name, error))?;
        if finalizer_seen && !matches!(command.category(), CommandCategory::Finalizer) {
            return Err(step_error(
                stage_name,
                index,
                &command_name,
                QuiltError::usage(format!(
                    "Record step '{command_name}' cannot follow a finalizer in stage '{stage_name}'."
                )),
            ));
        }
        finalizer_seen |= matches!(command.category(), CommandCategory::Finalizer);
    }
    Ok(())
}

#[cfg(test)]
fn validate_run_document(
    document: &RunDocument,
    parameters: Option<&HashMap<String, ResolvedParameter>>,
) -> Result<(), QuiltError> {
    for stage in &document.stages {
        if stage.name().trim().is_empty() {
            return Err(QuiltError::usage("Stage names must be non-empty."));
        }
        let dependencies = stage.dependencies();
        if dependencies
            .iter()
            .any(|dependency| dependency.trim().is_empty())
        {
            return Err(QuiltError::usage(format!(
                "Stage '{}' contains an empty dependency.",
                stage.name()
            )));
        }
        let mut unique_dependencies = HashSet::new();
        if dependencies
            .iter()
            .any(|dependency| !unique_dependencies.insert(dependency))
        {
            return Err(QuiltError::usage(format!(
                "Stage '{}' contains duplicate dependencies.",
                stage.name()
            )));
        }
        match stage {
            StageConfig::Process(process) => validate_step_sequence(&process.name, &process.steps)?,
            StageConfig::Branch(branch) => {
                match &branch.branch.when {
                    BranchPredicate::RowCount { .. } => {
                        branch.branch.when.evaluate(0).map_err(QuiltError::usage)?;
                    }
                    BranchPredicate::Parameter { .. } => branch
                        .branch
                        .when
                        .validate_parameter(
                            parameters.ok_or_else(|| {
                                QuiltError::usage(
                                    "parameter predicate requires resolved parameters",
                                )
                            })?,
                            Path::new("."),
                        )
                        .map_err(QuiltError::usage)?,
                }
                let mut targets = HashSet::new();
                for target in branch
                    .branch
                    .then
                    .iter()
                    .chain(branch.branch.r#else.iter().flatten())
                {
                    if target.trim().is_empty() {
                        return Err(QuiltError::usage(format!(
                            "Branch stage '{}' has an empty target",
                            branch.name
                        )));
                    }
                    if !targets.insert(target) {
                        return Err(QuiltError::usage(format!(
                            "Branch stage '{}' contains duplicate target '{}'.",
                            branch.name, target
                        )));
                    }
                }
            }
            StageConfig::Join(join) => {
                let node = &join.join;
                if node.inputs.len() < 2 {
                    return Err(QuiltError::usage(format!(
                        "Join stage '{}' must have at least two inputs.",
                        join.name
                    )));
                }
                let how = node.how.as_deref().unwrap_or("inner").to_ascii_lowercase();
                if !matches!(how.as_str(), "inner" | "left" | "full" | "outer" | "cross") {
                    return Err(QuiltError::usage(format!(
                        "Unsupported join type '{how}' in stage '{}'.",
                        join.name
                    )));
                }
                let has_on = node.on.is_some();
                let has_left = node.left_on.is_some();
                let has_right = node.right_on.is_some();
                if has_on && (has_left || has_right) || has_left != has_right {
                    return Err(QuiltError::usage(format!(
                        "Join stage '{}' must specify exactly one key mode.",
                        join.name
                    )));
                }
                if let Some(keys) = node.on.as_ref() {
                    if keys.is_empty() || keys.iter().any(|key| key.trim().is_empty()) {
                        return Err(QuiltError::usage(format!(
                            "Join stage '{}' has empty join keys.",
                            join.name
                        )));
                    }
                }
                if let (Some(left), Some(right)) = (&node.left_on, &node.right_on) {
                    if left.is_empty()
                        || left.len() != right.len()
                        || left.iter().chain(right).any(|key| key.trim().is_empty())
                    {
                        return Err(QuiltError::usage(format!(
                            "Join stage '{}' has invalid asymmetric join keys.",
                            join.name
                        )));
                    }
                }
                if how == "cross" && (has_on || has_left) {
                    return Err(QuiltError::usage(format!(
                        "Cross join stage '{}' cannot specify join keys.",
                        join.name
                    )));
                }
                if how != "cross" && !has_on && !has_left {
                    return Err(QuiltError::usage(format!(
                        "Join stage '{}' requires join keys unless how is cross.",
                        join.name
                    )));
                }
            }
            StageConfig::Concat(concat) => {
                if concat.concat.inputs.len() < 2 {
                    return Err(QuiltError::usage(format!(
                        "Concat stage '{}' must have at least two inputs.",
                        concat.name
                    )));
                }
                let how = concat
                    .concat
                    .how
                    .as_deref()
                    .unwrap_or("vertical")
                    .to_ascii_lowercase();
                if !matches!(how.as_str(), "vertical" | "v") {
                    return Err(QuiltError::usage(format!(
                        "Unsupported concat type '{how}' in stage '{}'.",
                        concat.name
                    )));
                }
            }
        }
    }
    Ok(())
}

fn diagnostic(file: &Path, yaml_path: impl AsRef<str>, message: impl Into<String>) -> String {
    format!(
        "{}: {}: {}",
        file.display(),
        yaml_path.as_ref(),
        message.into()
    )
}

fn preflight_run_document(
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
    if let Ok((order, configs)) = collect_stage_configs(&document.stages) {
        if let Err(error) = resolve_stage_execution_order(&order, &configs) {
            diagnostics.push(diagnostic(config_path, "stages", error));
        }
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(QuiltError::validation(diagnostics))
    }
}

fn step_field_path(
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

fn collect_stage_configs(
    stages: &[StageConfig],
) -> Result<(Vec<String>, HashMap<String, StageConfig>), String> {
    let mut stage_order = Vec::with_capacity(stages.len());
    let mut stage_configs = HashMap::with_capacity(stages.len());

    for stage_config in stages {
        let stage_name = stage_config.name().to_string();
        if stage_configs.contains_key(&stage_name) {
            return Err(format!("Duplicate stage name '{stage_name}'."));
        }
        stage_order.push(stage_name.clone());
        stage_configs.insert(stage_name, stage_config.clone());
    }

    Ok((stage_order, stage_configs))
}

fn get_stage_dependencies(stage_config: &StageConfig) -> Vec<String> {
    stage_config.dependencies()
}

/// Build the dependency graph used for ordering and cycle checks.
///
/// A branch's `input` is a data dependency of the branch itself, while each
/// target is a control-flow successor.  The latter therefore becomes an
/// incoming dependency of the target (rather than a dependency of the branch)
/// so that a target cannot run before its branch has selected a route.
fn stage_order_dependencies(
    stage_order: &[String],
    stage_configs: &HashMap<String, StageConfig>,
) -> Result<HashMap<String, Vec<String>>, String> {
    let mut dependencies = stage_configs
        .iter()
        .map(|(name, config)| (name.clone(), get_stage_dependencies(config)))
        .collect::<HashMap<_, _>>();

    for (branch_name, config) in stage_configs {
        let StageConfig::Branch(branch) = config else {
            continue;
        };
        for target in branch
            .branch
            .then
            .iter()
            .chain(branch.branch.r#else.iter().flatten())
        {
            let target_dependencies = dependencies
                .get_mut(target)
                .ok_or_else(|| format!("Branch target '{target}' not found"))?;
            if !target_dependencies.contains(branch_name) {
                target_dependencies.push(branch_name.clone());
            }
        }
    }

    // Keep deterministic validation for all stages, including an empty list.
    for stage_name in stage_order {
        if !dependencies.contains_key(stage_name) {
            return Err(format!(
                "Stage '{stage_name}' not found during dependency resolution."
            ));
        }
    }
    Ok(dependencies)
}

fn visit_stage(
    stage_name: &str,
    stage_order: &[String],
    stage_dependencies: &HashMap<String, Vec<String>>,
    visiting: &mut HashSet<String>,
    visited: &mut HashSet<String>,
    resolved: &mut Vec<String>,
) -> Result<(), String> {
    if visited.contains(stage_name) {
        return Ok(());
    }
    if !visiting.insert(stage_name.to_string()) {
        return Err(format!(
            "Circular stage dependency detected while visiting '{stage_name}'."
        ));
    }

    let dependencies = stage_dependencies
        .get(stage_name)
        .ok_or_else(|| format!("Stage '{stage_name}' not found during dependency resolution."))?;

    for dep in dependencies {
        if !stage_dependencies.contains_key(dep) {
            return Err(format!(
                "Stage '{stage_name}' depends on missing stage '{dep}'."
            ));
        }
        visit_stage(
            dep,
            stage_order,
            stage_dependencies,
            visiting,
            visited,
            resolved,
        )?;
    }

    visiting.remove(stage_name);
    visited.insert(stage_name.to_string());
    resolved.push(stage_name.to_string());

    // keep signature stable for future ordering rules
    let _ = stage_order;
    Ok(())
}

fn resolve_stage_execution_order(
    stage_order: &[String],
    stage_configs: &HashMap<String, StageConfig>,
) -> Result<Vec<String>, String> {
    let stage_dependencies = stage_order_dependencies(stage_order, stage_configs)?;
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    let mut resolved = Vec::with_capacity(stage_order.len());

    for stage_name in stage_order {
        visit_stage(
            stage_name,
            stage_order,
            &stage_dependencies,
            &mut visiting,
            &mut visited,
            &mut resolved,
        )?;
    }

    Ok(resolved)
}

fn execute_steps(
    stage_name: &str,
    steps: &Value,
    mut stage_output_df: Option<LazyFrame>,
    step_context: &mut ExecuteStepContext<'_>,
) -> Result<Option<LazyFrame>, QuiltError> {
    let parsed_steps = parse_steps(steps)?;
    let mut finalizer_seen = false;

    for (step_index, raw_command_name, command_args_val) in parsed_steps {
        let command_name = raw_command_name.as_str();
        LogController::debug(&format!(
            "Applying step: {command_name} to stage '{stage_name}'"
        ));

        let has_yaml_paths = command_args_val
            .as_mapping()
            .map(|mapping| {
                mapping.contains_key(Value::String("path".to_string()))
                    || mapping.contains_key(Value::String("paths".to_string()))
            })
            .unwrap_or(false);
        if command_name == "load" && !has_yaml_paths && stage_output_df.is_some() {
            LogController::debug(&format!(
                "Stage '{stage_name}' already has data; skipping load without a path."
            ));
            continue;
        }

        let command_args = if command_name == "load" && !has_yaml_paths {
            let mut value = command_args_val.clone();
            if let Some(mapping) = value.as_mapping_mut() {
                let files = step_context.cli_input_files.ok_or_else(|| {
                    step_error(
                        stage_name,
                        step_index,
                        command_name,
                        QuiltError::usage(format!(
                            "No data source specified for load in stage '{stage_name}'."
                        )),
                    )
                })?;
                if files.is_empty() {
                    return Err(step_error(
                        stage_name,
                        step_index,
                        command_name,
                        QuiltError::usage(format!(
                            "No data source specified for load in stage '{stage_name}'."
                        )),
                    ));
                }
                mapping.insert(
                    Value::String("paths".to_string()),
                    Value::Sequence(
                        files
                            .iter()
                            .map(|path| Value::String(path.to_string_lossy().into_owned()))
                            .collect(),
                    ),
                );
            }
            value
        } else {
            command_args_val
        };

        let mut command = parse_automation_step(command_name, &command_args)
            .map_err(|error| step_error(stage_name, step_index, command_name, error))?;

        if finalizer_seen && !matches!(command.category(), CommandCategory::Finalizer) {
            return Err(step_error(
                stage_name,
                step_index,
                command_name,
                QuiltError::usage(format!(
                    "Record step '{command_name}' cannot follow a finalizer in stage '{stage_name}'."
                )),
            ));
        }

        if let TypedCommand::Load(load) = &mut command {
            for path in &mut load.paths {
                if path.is_relative() {
                    let config_relative = step_context
                        .config_path
                        .parent()
                        .unwrap_or_else(|| Path::new("."))
                        .join(&*path);
                    *path = config_relative;
                }
            }
        }
        if let TypedCommand::Dump(args) = &mut command {
            if let Some(path) = &mut args.output {
                let path_buf = PathBuf::from(&*path);
                if path_buf.is_relative() {
                    *path = step_context
                        .config_path
                        .parent()
                        .unwrap_or_else(|| Path::new("."))
                        .join(path_buf)
                        .to_string_lossy()
                        .into_owned();
                }
            }
        }
        if let TypedCommand::DumpCache { output: Some(path) } = &mut command {
            let path_buf = PathBuf::from(&*path);
            if path_buf.is_relative() {
                *path = step_context
                    .config_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(path_buf)
                    .to_string_lossy()
                    .into_owned();
            }
        }
        if let TypedCommand::Partition(args) = &mut command {
            let path_buf = PathBuf::from(&args.output_dir);
            if path_buf.is_relative() {
                args.output_dir = step_context
                    .config_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(path_buf)
                    .to_string_lossy()
                    .into_owned();
            }
        }

        if matches!(command.category(), CommandCategory::Finalizer) {
            finalizer_seen = true;
        }

        if !matches!(command, TypedCommand::Load(_)) && stage_output_df.is_none() {
            return Err(step_error(
                stage_name,
                step_index,
                command_name,
                QuiltError::usage(format!(
                    "No DataFrame available for step '{command_name}' in stage '{stage_name}'. Load data first or specify a valid source."
                )),
            ));
        }

        let mut executor = if let Some(frame) = stage_output_df.take() {
            CommandExecutor::from_frame(frame)
        } else {
            CommandExecutor::new()
        };
        if let Err(error) = executor.execute(&command) {
            return Err(QuiltError::automation_with_source(
                stage_name,
                Some(format!("steps[{step_index}]/{command_name}")),
                error,
            ));
        }
        step_context
            .results
            .extend(executor.finalizer_results().iter().cloned());
        stage_output_df = executor.into_frame();
    }

    Ok(stage_output_df)
}

pub(crate) fn step_error(
    stage: &str,
    index: usize,
    command: &str,
    error: QuiltError,
) -> QuiltError {
    match error {
        QuiltError::Automation { .. } => error,
        error => QuiltError::automation_with_source(
            stage,
            Some(format!("steps[{index}]/{command}")),
            error,
        ),
    }
}

struct ExecuteStepContext<'a> {
    config_path: &'a Path,
    cli_input_files: Option<&'a Vec<PathBuf>>,
    results: &'a mut Vec<crate::operations::finalizers::FinalizerResult>,
}
fn execute_concat_stage(
    stage_name: &str,
    stage_config: &ConcatStage,
    stage_results: &HashMap<String, LazyFrame>,
) -> Result<LazyFrame, String> {
    let sources_vec = &stage_config.concat.inputs;

    if sources_vec.len() < 2 {
        return Err(format!(
            "Concat stage '{stage_name}' must have at least two sources. Found {}.",
            sources_vec.len()
        ));
    }

    let mut dataframes_to_concat: Vec<LazyFrame> = Vec::new();
    let mut missing_sources = Vec::new();

    for source_name in sources_vec {
        if let Some(source_df) = stage_results.get(source_name) {
            dataframes_to_concat.push(source_df.clone());
        } else {
            missing_sources.push(source_name.as_str());
        }
    }

    if !missing_sources.is_empty() {
        return Err(format!(
            "Could not find source DataFrame(s): {missing_sources:?} for concat stage '{stage_name}'."
        ));
    }

    let concat_how = stage_config
        .concat
        .how
        .clone()
        .unwrap_or_else(|| "vertical".to_string());

    match concat_how.to_lowercase().as_str() {
        "vertical" | "v" => {
            let mut iter = dataframes_to_concat.into_iter();
            let mut result = iter
                .next()
                .ok_or_else(|| format!("Concat stage '{stage_name}' has no valid sources."))?;
            for df in iter {
                result = polars::prelude::concat([result, df], polars::prelude::UnionArgs::default())
                    .map_err(|e| {
                        format!(
                            "Failed to concatenate DataFrames vertically in stage '{stage_name}': {e}"
                        )
                    })?;
            }
            Ok(result)
        }
        "horizontal" | "h" => Err(format!(
            "Horizontal concatenation is not yet implemented for stage '{stage_name}'. Use 'vertical' instead."
        )),
        _ => Err(format!(
            "Invalid concat method '{concat_how}' for stage '{stage_name}'. Use 'vertical' or 'horizontal'."
        )),
    }
}

#[derive(Clone)]
enum JoinKeySpec {
    Cross,
    Symmetric(Vec<String>),
    Asymmetric {
        left: Vec<String>,
        right: Vec<String>,
    },
}

fn parse_join_type(stage_name: &str, how_str: &str) -> JoinType {
    match how_str.to_lowercase().as_str() {
        "inner" => JoinType::Inner,
        "left" => JoinType::Left,
        "outer" | "full" => JoinType::Full,
        "cross" => JoinType::Cross,
        _ => {
            LogController::warn(&format!(
                "Unsupported join type '{how_str}' for stage '{stage_name}'. Defaulting to inner join."
            ));
            JoinType::Inner
        }
    }
}

fn join_pair(
    left_df: LazyFrame,
    right_df: LazyFrame,
    stage_name: &str,
    join_type: JoinType,
    coalesce: bool,
    key_spec: &JoinKeySpec,
) -> LazyFrame {
    if matches!(key_spec, JoinKeySpec::Cross) {
        let cross_key = "__qlt_run_cross_join_key";
        let mut join_args = polars::prelude::JoinArgs::new(JoinType::Inner);
        if coalesce {
            join_args = join_args.with_coalesce(polars::prelude::JoinCoalesce::CoalesceColumns);
        }
        return left_df
            .with_column(lit(cross_key).alias(cross_key))
            .join(
                right_df.with_column(lit(cross_key).alias(cross_key)),
                &[col(cross_key)],
                &[col(cross_key)],
                join_args,
            )
            .select([col("*").exclude([cross_key])]);
    }

    let mut join_args = polars::prelude::JoinArgs::new(join_type);
    if coalesce {
        join_args = join_args.with_coalesce(polars::prelude::JoinCoalesce::CoalesceColumns);
    }

    let (left_on, right_on) = match key_spec {
        JoinKeySpec::Symmetric(cols) => (cols.clone(), cols.clone()),
        JoinKeySpec::Asymmetric { left, right } => (left.clone(), right.clone()),
        JoinKeySpec::Cross => unreachable!("cross join handled earlier"),
    };

    let left_on_exprs: Vec<_> = left_on.iter().map(col).collect();
    let right_on_exprs: Vec<_> = right_on.iter().map(col).collect();

    LogController::debug(&format!(
        "Joining stage '{stage_name}' with keys left={left_on:?} right={right_on:?}"
    ));

    left_df.join(right_df, &left_on_exprs, &right_on_exprs, join_args)
}

fn execute_join_stage(
    stage_name: &str,
    stage_config: &JoinStage,
    stage_results: &HashMap<String, LazyFrame>,
) -> Result<LazyFrame, String> {
    let sources = &stage_config.join.inputs;

    if sources.len() < 2 {
        return Err(format!(
            "Join stage '{stage_name}' must have at least two sources. Found {}.",
            sources.len()
        ));
    }

    let mut dataframes = Vec::with_capacity(sources.len());
    for source_name in sources {
        let df = stage_results.get(source_name).ok_or_else(|| {
            format!(
                "Could not find source DataFrame '{source_name}' for join stage '{stage_name}'."
            )
        })?;
        dataframes.push(df.clone());
    }

    let how_str = stage_config
        .join
        .how
        .clone()
        .unwrap_or_else(|| "inner".to_string());
    let join_type = parse_join_type(stage_name, &how_str);
    let coalesce = stage_config.join.coalesce.unwrap_or(false);
    let key_spec = if let Some(on) = &stage_config.join.on {
        JoinKeySpec::Symmetric(on.clone())
    } else if let (Some(left), Some(right)) =
        (&stage_config.join.left_on, &stage_config.join.right_on)
    {
        JoinKeySpec::Asymmetric {
            left: left.clone(),
            right: right.clone(),
        }
    } else if matches!(join_type, JoinType::Cross) {
        JoinKeySpec::Cross
    } else {
        return Err(format!(
            "Join stage '{stage_name}' requires 'on' or both 'left-on' and 'right-on'."
        ));
    };

    let mut iter = dataframes.into_iter();
    let mut result = iter
        .next()
        .ok_or_else(|| format!("Join stage '{stage_name}' has no valid sources."))?;
    for right_df in iter {
        result = join_pair(
            result,
            right_df,
            stage_name,
            join_type.clone(),
            coalesce,
            &key_spec,
        );
    }

    Ok(result)
}

fn redact_text(mut text: String, secret_values: &[String]) -> String {
    for secret in secret_values {
        if !secret.is_empty() {
            text = text.replace(secret, "<redacted>");
        }
    }
    text
}

fn redact_error(error: QuiltError, secret_values: &[String]) -> QuiltError {
    match error {
        QuiltError::Validation { diagnostics } => QuiltError::Validation {
            diagnostics: diagnostics
                .into_iter()
                .map(|value| redact_text(value, secret_values))
                .collect(),
        },
        QuiltError::Usage { message } => QuiltError::Usage {
            message: redact_text(message, secret_values),
        },
        QuiltError::Schema {
            operation,
            column,
            message,
        } => QuiltError::Schema {
            operation: redact_text(operation, secret_values),
            column: column.map(|value| redact_text(value, secret_values)),
            message: redact_text(message, secret_values),
        },
        QuiltError::Conversion {
            operation,
            column,
            message,
        } => QuiltError::Conversion {
            operation: redact_text(operation, secret_values),
            column: column.map(|value| redact_text(value, secret_values)),
            message: redact_text(message, secret_values),
        },
        QuiltError::Io {
            operation,
            path,
            message,
        } => QuiltError::Io {
            operation: redact_text(operation, secret_values),
            path: path.map(|value| redact_text(value, secret_values)),
            message: redact_text(message, secret_values),
        },
        QuiltError::Operation { operation, message } => QuiltError::Operation {
            operation: redact_text(operation, secret_values),
            message: redact_text(message, secret_values),
        },
        QuiltError::Finalizer { operation, message } => QuiltError::Finalizer {
            operation: redact_text(operation, secret_values),
            message: redact_text(message, secret_values),
        },
        QuiltError::Automation {
            stage,
            step,
            message,
            source,
        } => QuiltError::Automation {
            stage: redact_text(stage, secret_values),
            step: step.map(|value| redact_text(value, secret_values)),
            message: redact_text(message, secret_values),
            source: source.map(|value| Box::new(redact_error(*value, secret_values))),
        },
    }
}

fn discover_secret_values(raw: &str, overrides: &[String]) -> Vec<String> {
    let Ok(root) = serde_yml::from_str::<Value>(raw) else {
        return Vec::new();
    };
    let Some(raw_parameters) = root.get("parameters").and_then(Value::as_mapping) else {
        return Vec::new();
    };
    let mut secrets = Vec::new();
    for (raw_name, raw_declaration) in raw_parameters {
        let Some(name) = raw_name.as_str() else {
            continue;
        };
        let Some(mapping) = raw_declaration.as_mapping() else {
            continue;
        };
        if !mapping
            .get(Value::String("secret".into()))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        if let Some(default) = mapping
            .get(Value::String("default".into()))
            .and_then(Value::as_str)
        {
            secrets.push(default.to_string());
        }
        for override_value in overrides {
            if let Some((override_name, value)) = override_value.split_once('=') {
                if override_name == name {
                    secrets.push(value.to_string());
                }
            }
        }
    }
    secrets
}

pub fn run(
    controller: &mut DataFrameController,
    config_path_str: &str,
    cli_input_files: Option<Vec<PathBuf>>,
    output_path_str: Option<&str>,
    run_vars: &[String],
    check_only: bool,
) -> Result<Vec<crate::operations::finalizers::FinalizerResult>, QuiltError> {
    let mut secret_values = Vec::new();
    let mut finalizer_results = Vec::new();
    let result: Result<Vec<crate::operations::finalizers::FinalizerResult>, QuiltError> = (|| {
        let config_path = Path::new(config_path_str);
        let raw_config_content = fs::read_to_string(config_path).map_err(|error| {
            QuiltError::automation(
                "run",
                None,
                format!(
                    "Error reading config file {}: {error}",
                    config_path.display()
                ),
            )
        })?;

        secret_values = discover_secret_values(&raw_config_content, run_vars);
        let prepared = match prepare_document(&raw_config_content, config_path, run_vars) {
            Ok(prepared) => prepared,
            Err(parameter_error) => {
                // A missing/invalid parameter must not hide independent static
                // stage diagnostics when the raw document remains structurally
                // deserializable.
                if let Ok(raw_document) = serde_yml::from_str::<RunDocument>(&raw_config_content) {
                    if let Err(stage_error) =
                        preflight_run_document(&raw_document, &HashMap::new(), config_path)
                    {
                        return Err(QuiltError::validation(vec![
                            parameter_error.to_string(),
                            stage_error.to_string(),
                        ]));
                    }
                }
                return Err(parameter_error);
            }
        };
        secret_values = prepared.secret_values;
        let parameters = prepared.parameters;
        let run_document = prepared.document;
        if run_document.version != 1 {
            return Err(QuiltError::automation(
                "run",
                None,
                format!(
                    "Unsupported run document version {} (expected 1)",
                    run_document.version
                ),
            ));
        }
        preflight_run_document(&run_document, &parameters, config_path)?;
        let (stage_order, stage_configs) = collect_stage_configs(&run_document.stages)
            .map_err(|error| QuiltError::automation("run", None, error))?;
        let execution_order =
            resolve_stage_execution_order(&stage_order, &stage_configs).map_err(|error| {
                QuiltError::automation(
                    "run",
                    None,
                    format!("Error validating run document stage dependencies: {error}"),
                )
            })?;
        if check_only {
            return Ok(Vec::new());
        }

        LogController::info(&format!(
            "Executing run document '{}' with {} stage entries in YAML",
            run_document.title.as_deref().unwrap_or("run"),
            run_document.stages.len()
        ));
        let mut stage_results: HashMap<String, LazyFrame> = HashMap::new();
        let mut last_processed_df: Option<LazyFrame> = None;
        let mut skipped_stages = HashSet::new();

        for stage_name in execution_order {
            if skipped_stages.contains(&stage_name) {
                continue;
            }
            let stage_config = match stage_configs.get(&stage_name) {
                Some(sc) => sc,
                None => {
                    return Err(QuiltError::automation(
                        &stage_name,
                        None,
                        format!("Stage '{stage_name}' disappeared during execution."),
                    ));
                }
            };

            // A route can feed an intermediate stage rather than a finalizer. If
            // that route was not selected, propagate the skip through ordinary
            // data dependencies instead of attempting to execute with a missing
            // input frame.
            if stage_config
                .dependencies()
                .iter()
                .any(|dependency| skipped_stages.contains(dependency))
            {
                skipped_stages.insert(stage_name.clone());
                if let StageConfig::Branch(branch) = stage_config {
                    for target in branch
                        .branch
                        .then
                        .iter()
                        .chain(branch.branch.r#else.iter().flatten())
                    {
                        skipped_stages.insert(target.clone());
                    }
                }
                continue;
            }

            LogController::debug(&format!("Processing stage: {stage_name}"));

            let current_stage_input_df = stage_config
                .dependencies()
                .first()
                .and_then(|source_name| stage_results.get(source_name))
                .cloned();
            let mut process_step_context = ExecuteStepContext {
                config_path,
                cli_input_files: cli_input_files.as_ref(),
                results: &mut finalizer_results,
            };

            let stage_output_df = match stage_config {
                StageConfig::Process(process) => execute_steps(
                    &stage_name,
                    &Value::Sequence(process.steps.clone()),
                    current_stage_input_df.clone(),
                    &mut process_step_context,
                )?,
                StageConfig::Join(join) => execute_join_stage(&stage_name, join, &stage_results)
                    .map(Some)
                    .map_err(|error| QuiltError::automation(&stage_name, None, error))?,
                StageConfig::Concat(concat) => {
                    execute_concat_stage(&stage_name, concat, &stage_results)
                        .map(Some)
                        .map_err(|error| QuiltError::automation(&stage_name, None, error))?
                }
                StageConfig::Branch(branch) => {
                    // A branch node is routing metadata, not a user-visible
                    // output. Prevent an input frame from becoming an
                    // unrelated top-level --output result when both routes
                    // are empty.
                    last_processed_df = None;
                    let input_df = current_stage_input_df.ok_or_else(|| {
                        QuiltError::automation(
                            &stage_name,
                            None,
                            "Branch stage requires a valid input stage",
                        )
                    })?;
                    let rows = input_df
                        .clone()
                        .collect()
                        .map_err(|error| {
                            QuiltError::automation(&stage_name, None, error.to_string())
                        })?
                        .height();
                    let condition_result = match &branch.branch.when {
                        BranchPredicate::RowCount { .. } => branch.branch.when.evaluate(rows),
                        BranchPredicate::Parameter { .. } => branch.branch.when.evaluate_parameter(
                            &parameters,
                            config_path.parent().unwrap_or_else(|| Path::new(".")),
                        ),
                    }
                    .map_err(|error| QuiltError::automation(&stage_name, None, error))?;
                    let unselected = if condition_result {
                        branch.branch.r#else.as_ref()
                    } else {
                        Some(&branch.branch.then)
                    };
                    if let Some(targets) = unselected {
                        for target in targets {
                            skipped_stages.insert(target.clone());
                        }
                    }
                    Some(input_df)
                }
            };

            if let Some(df_to_store) = &stage_output_df {
                stage_results.insert(stage_name.clone(), df_to_store.clone());
                if !matches!(stage_config, StageConfig::Branch(_)) {
                    last_processed_df = Some(df_to_store.clone());
                }
                LogController::debug(&format!(
                    "Finished processing stage '{stage_name}'. Result stored."
                ));
            } else {
                LogController::warn(&format!(
                    "Stage '{stage_name}' did not produce a DataFrame."
                ));
            }
        }

        LogController::info(&format!(
            "Run document '{}' execution processing finished.",
            run_document.title.as_deref().unwrap_or("run")
        ));
        if let Some(path_str) = output_path_str {
            if let Some(final_df_to_dump) = last_processed_df {
                LogController::info(&format!("Saving final run output to: {path_str}"));
                let final_output_path = Path::new(path_str);
                let absolute_path = if final_output_path.is_absolute() {
                    final_output_path.to_path_buf()
                } else {
                    config_path
                        .parent()
                        .unwrap_or_else(|| Path::new("."))
                        .join(final_output_path)
                };
                if let Some(parent) = absolute_path.parent() {
                    if !parent.exists() {
                        std::fs::create_dir_all(parent).map_err(|error| {
                            QuiltError::automation(
                                "run",
                                None,
                                format!("Error creating directory {}: {error}", parent.display()),
                            )
                        })?;
                    }
                }
                let mut output_executor = CommandExecutor::from_frame(final_df_to_dump);
                output_executor
                    .execute(&TypedCommand::Dump(DumpArgs {
                        output: Some(absolute_path.to_string_lossy().into_owned()),
                        separator: ',',
                    }))
                    .map_err(|error| {
                        QuiltError::automation_with_source("run", Some("output".into()), error)
                    })?;
                finalizer_results.extend(output_executor.finalizer_results().iter().cloned());
            } else {
                LogController::warn(
                    "No final DataFrame from run execution to save for --output CLI option.",
                );
            }
        } else {
            if let Some(final_df_state) = last_processed_df {
                controller.set_df(final_df_state);
            }
            LogController::debug(
            "Run document finished. Output handled by YAML finalizer steps or by main CLI flow if no explicit output/show in YAML.",
        );
        }
        Ok(finalizer_results)
    })();
    result.map_err(|error| redact_error(error, &secret_values))
}

/// Build one canonical run stage and inspect its pending plan without running
/// any finalizer or collecting rows.
pub fn run_show_plan(
    config_path_str: &str,
    stage_name: &str,
) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
    let config_path = Path::new(config_path_str);
    let raw = fs::read_to_string(config_path)
        .map_err(|error| QuiltError::automation("run", None, error.to_string()))?;
    let prepared = prepare_document(&raw, config_path, &[])?;
    preflight_run_document(&prepared.document, &prepared.parameters, config_path)?;
    let (_, stages) = collect_stage_configs(&prepared.document.stages)
        .map_err(|error| QuiltError::automation("run", None, error))?;
    if !stages.contains_key(stage_name) {
        return Err(QuiltError::usage(format!(
            "run stage '{stage_name}' not found"
        )));
    }
    let mut cache = HashMap::new();
    let mut visiting = HashSet::new();
    let mut ignored_results = Vec::new();
    let frame = build_plan_stage(
        stage_name,
        &stages,
        &mut cache,
        &mut visiting,
        config_path,
        &mut ignored_results,
    )?;
    crate::operations::finalizers::showquery::showquery(&frame)
}

fn build_plan_stage(
    name: &str,
    stages: &HashMap<String, StageConfig>,
    cache: &mut HashMap<String, LazyFrame>,
    visiting: &mut HashSet<String>,
    config_path: &Path,
    results: &mut Vec<crate::operations::finalizers::FinalizerResult>,
) -> Result<LazyFrame, QuiltError> {
    if let Some(frame) = cache.get(name) {
        return Ok(frame.clone());
    }
    if !visiting.insert(name.to_string()) {
        return Err(QuiltError::automation(
            "run",
            None,
            format!("cycle while building plan at stage '{name}'"),
        ));
    }
    let stage = stages
        .get(name)
        .ok_or_else(|| QuiltError::usage(format!("run stage '{name}' not found")))?;
    let dependencies = stage.dependencies();
    let mut inputs = HashMap::new();
    for dependency in &dependencies {
        inputs.insert(
            dependency.clone(),
            build_plan_stage(dependency, stages, cache, visiting, config_path, results)?,
        );
    }
    let frame = match stage {
        StageConfig::Process(process) => {
            let filtered = process
                .steps
                .iter()
                .filter(|step| {
                    step.as_mapping()
                        .and_then(|mapping| mapping.keys().next())
                        .and_then(|key| key.as_str())
                        .and_then(|key| {
                            crate::controllers::command_model::command_specs()
                                .iter()
                                .find(|spec| spec.name == key)
                        })
                        .is_none_or(|spec| spec.category != CommandCategory::Finalizer)
                })
                .cloned()
                .collect::<Vec<_>>();
            let mut context = ExecuteStepContext {
                config_path,
                cli_input_files: None,
                results,
            };
            execute_steps(
                name,
                &Value::Sequence(filtered),
                dependencies
                    .first()
                    .and_then(|dependency| inputs.get(dependency).cloned()),
                &mut context,
            )?
            .ok_or_else(|| {
                QuiltError::usage(format!("run stage '{name}' did not produce a frame"))
            })?
        }
        StageConfig::Join(join) => execute_join_stage(name, join, &inputs)
            .map_err(|error| QuiltError::automation(name, None, error))?,
        StageConfig::Concat(concat) => execute_concat_stage(name, concat, &inputs)
            .map_err(|error| QuiltError::automation(name, None, error))?,
        StageConfig::Branch(_) => {
            return Err(QuiltError::usage(format!(
                "run --show-plan cannot inspect dynamic branch stage '{name}'"
            )));
        }
    };
    visiting.remove(name);
    cache.insert(name.to_string(), frame.clone());
    Ok(frame)
}

#[cfg(test)]
mod tests {
    #[test]
    fn typed_parameter_declarations_and_literals_are_strict() {
        let yaml = "version: 1\nparameters: {count: {type: int, default: 3}, enabled: {type: bool, default: true}}\nstages: [{name: input, steps: []}]";
        let prepared = prepare_document(yaml, Path::new("rules/run.yaml"), &[]).unwrap();
        assert_eq!(prepared.parameters["count"].value, ParameterValue::Int(3));
        assert_eq!(
            prepared.parameters["enabled"].value,
            ParameterValue::Bool(true)
        );
        assert!(prepare_document(
            "version: 1\nparameters: {count: {type: int, default: nope}}\nstages: []",
            Path::new("run.yaml"),
            &[]
        )
        .is_err());
    }

    #[test]
    fn preflight_collects_yaml_path_diagnostics() {
        let document = document(
            "version: 1\nstages: [{name: one, from: missing, steps: [{head: {number: nope}}]}]",
        );
        let parameters = HashMap::new();
        let error =
            preflight_run_document(&document, &parameters, Path::new("run.yaml")).unwrap_err();
        let rendered = error.to_string();
        assert!(rendered.contains("run.yaml: stages[0].dependencies[0]"));
        assert!(rendered.contains("run.yaml: stages[0].steps[0].head"));
    }
    use super::*;

    fn document(yaml: &str) -> RunDocument {
        serde_yml::from_str(yaml).expect("canonical run document should deserialize")
    }

    #[test]
    fn canonical_v1_supports_all_stage_kinds_and_repeated_steps() {
        let doc = document(
            r#"
version: 1
stages:
  - name: input
    steps:
      - load: {paths: [input.csv]}
      - grep: {pattern: ERROR}
      - grep: {pattern: WARN}
  - name: left
    from: input
    steps: [{select: {columns: [id]}}]
  - name: right
    from: input
    steps: [{select: {columns: [id, value]}}]
  - name: joined
    join: {inputs: [left, right], how: inner, on: [id], coalesce: true}
  - name: combined
    concat: {inputs: [left, right], how: vertical}
  - name: selected
    branch:
      input: joined
      when: {row-count: {greater-than: 0}}
      then: [left]
      else: [right]
"#,
        );
        assert_eq!(doc.version, 1);
        assert_eq!(doc.stages.len(), 6);
        assert_eq!(doc.stages[0].name(), "input");
        assert!(matches!(doc.stages[3], StageConfig::Join(_)));
        assert!(matches!(doc.stages[4], StageConfig::Concat(_)));
        assert!(matches!(doc.stages[5], StageConfig::Branch(_)));
        let ProcessStage { steps, .. } = match &doc.stages[0] {
            StageConfig::Process(stage) => stage,
            _ => panic!("expected process stage"),
        };
        assert_eq!(
            parse_steps(&Value::Sequence(steps.clone())).unwrap().len(),
            3
        );
    }

    #[test]
    fn dependency_resolution_is_deterministic_and_handles_shared_intermediates() {
        let doc = document(
            r#"
version: 1
stages:
  - name: output
    from: shared
    steps: [{show: {}}]
  - name: shared
    from: input
    steps: [{head: {number: 1}}]
  - name: other
    from: input
    steps: [{tail: {number: 1}}]
  - name: input
    steps: [{load: {paths: [input.csv]}}]
"#,
        );
        let (order, configs) = collect_stage_configs(&doc.stages).unwrap();
        assert_eq!(
            resolve_stage_execution_order(&order, &configs).unwrap(),
            ["input", "shared", "output", "other"]
        );
    }

    #[test]
    fn schema_rejects_unknown_keys_legacy_shapes_and_mapping_steps() {
        assert!(
            serde_yml::from_str::<RunDocument>("version: 1\nunknown: true\nstages: []").is_err()
        );
        assert!(serde_yml::from_str::<RunDocument>(
            "version: 1\nstages: [{name: input, steps: [], extra: true}]"
        )
        .is_err());
        assert!(serde_yml::from_str::<RunDocument>(
            "version: 1\nstages: [{name: input, steps: {load: {paths: [x]}}}]"
        )
        .is_err());
        assert!(serde_yml::from_str::<RunDocument>("version: '1.0.0'\nstages: {}").is_err());
        assert!(serde_yml::from_str::<RunDocument>(
            "version: 1\nstages: [{name: input, type: process, steps: []}]"
        )
        .is_err());
    }

    #[test]
    fn duplicate_names_cycles_and_invalid_steps_are_rejected_before_execution() {
        let duplicate =
            document("version: 1\nstages: [{name: same, steps: []}, {name: same, steps: []}]");
        assert!(collect_stage_configs(&duplicate.stages).is_err());

        let cycle = document(
            "version: 1\nstages: [{name: a, from: b, steps: []}, {name: b, from: a, steps: []}]",
        );
        let (order, configs) = collect_stage_configs(&cycle.stages).unwrap();
        assert!(resolve_stage_execution_order(&order, &configs).is_err());

        let invalid = document(
            "version: 1\nstages: [{name: input, steps: [{load: {paths: [x], typo: true}}]}]",
        );
        assert!(validate_run_document(&invalid, None).is_err());

        let misplaced = document(
            "version: 1\nstages: [{name: input, steps: [{load: {paths: [x]}}, {show: {}}, {head: {number: 1}}]}]",
        );
        assert!(validate_run_document(&misplaced, None).is_err());
    }

    #[test]
    fn public_run_rejects_static_shapes_before_nonexistent_input_io() {
        let root = std::env::temp_dir().join(format!("run-static-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&root);
        let input = root.join("does-not-exist.csv");
        let cases = [
            (
                "join",
                "join:\n      inputs: [a]\n      how: inner\n      'on': [id]",
                "at least two inputs",
            ),
            (
                "join-how",
                "join:\n      inputs: [a, b]\n      how: sideways\n      'on': [id]",
                "Unsupported join type",
            ),
            (
                "join-key",
                "join:\n      inputs: [a, b]\n      left-on: [id]",
                "exactly one key mode",
            ),
            (
                "concat",
                "concat:\n      inputs: [a]\n      how: vertical",
                "at least two inputs",
            ),
            (
                "concat-how",
                "concat:\n      inputs: [a, b]\n      how: horizontal",
                "Unsupported concat type",
            ),
            (
                "branch-lhs",
                "branch:\n      input: input\n      when: {row-count: {greater-than: 1, less-than: 2}}\n      then: []",
                "exactly one comparison",
            ),
            (
                "branch-op",
                "branch:\n      input: input\n      when: {row-count: {}}\n      then: []",
                "exactly one comparison",
            ),
            ("step-key", "steps: [{true: {}}]", "single-entry mapping"),
            (
                "step-category",
                "steps: [{run: {}}]",
                "single-entry mapping",
            ),
        ];
        for (name, shape, expected) in cases {
            let load = format!("- load:\n          paths: ['{}']", input.display());
            let yaml = if shape.starts_with("steps:") {
                format!(
                    "version: 1\nstages:\n  - name: input\n    steps:\n      {load}\n      - {}\n",
                    shape.strip_prefix("steps: ").unwrap()
                )
            } else {
                format!("version: 1\nstages:\n  - name: input\n    steps:\n      {load}\n  - name: a\n    from: input\n    steps: []\n  - name: b\n    from: input\n    steps: []\n  - name: bad\n    {shape}\n")
            };
            let path = root.join(format!("{name}.yaml"));
            std::fs::write(&path, yaml).unwrap();
            let mut controller = DataFrameController::new();
            let error = run(
                &mut controller,
                path.to_str().unwrap(),
                None,
                None,
                &[],
                false,
            )
            .unwrap_err();
            let rendered = error.to_string();
            assert!(!rendered.contains("File not found"), "{name}: {rendered}");
            assert!(
                rendered.contains(expected),
                "{name}: expected '{expected}', got {rendered}"
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn automation_steps_reject_wrong_yaml_scalar_types() {
        for (name, yaml) in [
            ("select", "columns: [1]"),
            ("load", "paths: [false]"),
            ("grep", "pattern: 42"),
            ("show", "debug: yes"),
        ] {
            assert!(
                parse_automation_step(name, &serde_yml::from_str(yaml).unwrap()).is_err(),
                "accepted wrong type for {name}"
            );
        }
    }
}
