use serde::{Deserialize, Serialize};
use serde_yml::Value;
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
    pub(super) fn yaml_value(&self) -> Value {
        match self {
            Self::Path(v) => Value::String(v.to_string_lossy().into_owned()),
            Self::String(v) => Value::String(v.clone()),
            Self::Int(v) => Value::Number((*v).into()),
            Self::Bool(v) => Value::Bool(*v),
        }
    }
    pub(super) fn redaction_value(&self) -> String {
        match self {
            Self::Path(v) => v.to_string_lossy().into_owned(),
            Self::String(v) => v.clone(),
            Self::Int(v) => v.to_string(),
            Self::Bool(v) => v.to_string(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParameterOrigin {
    Default,
    Cli,
}
#[derive(Debug, Clone)]
pub(super) struct ResolvedParameter {
    pub(super) parameter_type: ParameterType,
    pub(super) value: ParameterValue,
    pub(super) origin: ParameterOrigin,
    pub(super) secret: bool,
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
    pub(super) fn value_for(
        &self,
        ty: ParameterType,
        base: &Path,
    ) -> Result<ParameterValue, String> {
        match (ty, self) {
            (ParameterType::String, Self::String(v)) => Ok(ParameterValue::String(v.clone())),
            (ParameterType::Path, Self::String(v)) => {
                let p = Path::new(v);
                Ok(ParameterValue::Path(if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    std::env::current_dir()
                        .unwrap_or_else(|_| PathBuf::from("."))
                        .join(base)
                        .join(p)
                }))
            }
            (ParameterType::Int, Self::Int(v)) => Ok(ParameterValue::Int(*v)),
            (ParameterType::Bool, Self::Bool(v)) => Ok(ParameterValue::Bool(*v)),
            (expected, actual) => Err(format!("expected {expected:?} value, got {actual:?}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MaterializePolicy {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum StageConfig {
    Process(ProcessStage),
    Join(JoinStage),
    Concat(ConcatStage),
    Branch(BranchStage),
}

impl StageConfig {
    pub(super) fn name(&self) -> &str {
        match self {
            Self::Process(v) => &v.name,
            Self::Join(v) => &v.name,
            Self::Concat(v) => &v.name,
            Self::Branch(v) => &v.name,
        }
    }
    pub(super) fn dependencies(&self) -> Vec<String> {
        match self {
            Self::Process(v) => v.source.clone().into_iter().collect(),
            Self::Join(v) => v.join.inputs.clone(),
            Self::Concat(v) => v.concat.inputs.clone(),
            Self::Branch(v) => vec![v.branch.input.clone()],
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ProcessStage {
    pub name: String,
    #[serde(rename = "from")]
    pub source: Option<String>,
    pub steps: Vec<Value>,
    #[serde(default)]
    pub materialize: MaterializePolicy,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct JoinStage {
    pub name: String,
    pub join: JoinNode,
    #[serde(default)]
    pub materialize: MaterializePolicy,
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
    let value = Option::<Value>::deserialize(deserializer)?;
    value
        .map(|value| match value {
            Value::String(value) => Ok(vec![value]),
            Value::Sequence(values) => values
                .into_iter()
                .map(|value| match value {
                    Value::String(value) => Ok(value),
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
    #[serde(default)]
    pub materialize: MaterializePolicy,
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
    #[serde(default)]
    pub materialize: MaterializePolicy,
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
    pub(super) fn evaluate(&self, rows: usize) -> Result<bool, String> {
        let BranchPredicate::RowCount { row_count: p } = self else {
            return Err("parameter branch predicate requires resolved parameters".into());
        };
        let values = [
            p.equal.map(|v| rows as i64 == v),
            p.not_equal.map(|v| rows as i64 != v),
            p.greater_than.map(|v| rows as i64 > v),
            p.greater_or_equal.map(|v| rows as i64 >= v),
            p.less_than.map(|v| (rows as i64) < v),
            p.less_or_equal.map(|v| rows as i64 <= v),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        if values.len() != 1 {
            return Err("row-count predicate requires exactly one comparison".into());
        }
        Ok(values[0])
    }

    pub(super) fn validate_parameter(
        &self,
        parameters: &std::collections::HashMap<String, ResolvedParameter>,
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
        if operators.iter().flatten().count() != 1 {
            return Err("parameter predicate requires exactly one comparison".into());
        }
        let literal = operators.into_iter().flatten().next().unwrap();
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

    pub(super) fn evaluate_parameter(
        &self,
        parameters: &std::collections::HashMap<String, ResolvedParameter>,
        run_base: &Path,
    ) -> Result<bool, String> {
        let BranchPredicate::Parameter { parameter } = self else {
            return Err("not a parameter predicate".into());
        };
        self.validate_parameter(parameters, run_base)?;
        let resolved = parameters.get(&parameter.name).unwrap();
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
