//! Per-invocation diagnostics context.
//!
//! This boundary intentionally owns no global state and accepts only typed
//! invocation metadata. The existing redaction policy remains the compatibility
//! implementation while callers migrate to this context.

use crate::error::QuiltError;
use serde_yml::Value;
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub(super) struct DiagnosticsContext {
    config_path: Option<String>,
}

impl DiagnosticsContext {
    pub(super) fn for_config(path: &Path) -> Self {
        Self {
            config_path: Some(path.display().to_string()),
        }
    }

    pub(super) fn config_path(&self) -> Option<&str> {
        self.config_path.as_deref()
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct DiagnosticPolicy {
    forms: Vec<String>,
    pub(super) sensitive_steps: HashSet<(String, usize)>,
    pub(super) sensitive_stages: HashSet<String>,
    pub(super) sensitive_stage_indices: HashSet<usize>,
    pub(super) sensitive_locations: HashSet<String>,
}

impl DiagnosticPolicy {
    pub(super) fn from_values(values: impl IntoIterator<Item = String>) -> Self {
        let mut forms = Vec::new();
        for value in values {
            if value.is_empty() {
                continue;
            }
            if value.chars().count() >= 4 {
                forms.push(value.clone());
                forms.push(format!("{value:?}"));
                if let Ok(yaml) = serde_yml::to_string(&Value::String(value.clone())) {
                    forms.push(yaml.trim().to_string());
                }
                let escaped = value
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "\\n")
                    .replace('\r', "\\r");
                forms.extend([format!("'{escaped}'"), format!("\"{escaped}\""), escaped]);
            }
        }
        forms.sort_by_key(|value| std::cmp::Reverse(value.len()));
        forms.dedup();
        Self {
            forms,
            sensitive_steps: HashSet::new(),
            sensitive_stages: HashSet::new(),
            sensitive_stage_indices: HashSet::new(),
            sensitive_locations: HashSet::new(),
        }
    }
    pub(super) fn sanitize(&self, mut text: String) -> String {
        for form in &self.forms {
            text = text.replace(form, "<redacted>");
        }
        text
    }
    pub(super) fn sanitize_sensitive_error(&self, error: QuiltError) -> QuiltError {
        match error {
            QuiltError::Validation { diagnostics } => QuiltError::Validation {
                diagnostics: diagnostics
                    .into_iter()
                    .map(|_| "<redacted diagnostic>".to_string())
                    .collect(),
            },
            QuiltError::Usage { .. } => QuiltError::Usage {
                message: "<redacted diagnostic>".into(),
            },
            QuiltError::Schema {
                operation, column, ..
            } => QuiltError::Schema {
                operation,
                column,
                message: "<redacted diagnostic>".into(),
            },
            QuiltError::Conversion {
                operation, column, ..
            } => QuiltError::Conversion {
                operation,
                column,
                message: "<redacted diagnostic>".into(),
            },
            QuiltError::Io { operation, .. } => QuiltError::Io {
                operation,
                path: None,
                message: "<redacted diagnostic>".into(),
            },
            QuiltError::Operation { operation, .. } => QuiltError::Operation {
                operation,
                message: "<redacted diagnostic>".into(),
            },
            QuiltError::Finalizer { operation, .. } => QuiltError::Finalizer {
                operation,
                message: "<redacted diagnostic>".into(),
            },
            QuiltError::Automation {
                stage,
                step,
                source,
                ..
            } => QuiltError::Automation {
                stage,
                step,
                message: "<redacted diagnostic>".into(),
                source: source.map(|source| Box::new(self.sanitize_sensitive_error(*source))),
            },
        }
    }
    pub(super) fn sanitize_step_error(&self, error: QuiltError, sensitive: bool) -> QuiltError {
        if sensitive {
            self.sanitize_sensitive_error(error)
        } else {
            self.sanitize_error(error)
        }
    }
    pub(super) fn sanitize_error(&self, error: QuiltError) -> QuiltError {
        match error {
            QuiltError::Validation { diagnostics } => QuiltError::Validation {
                diagnostics: diagnostics.into_iter().map(|v| self.sanitize(v)).collect(),
            },
            QuiltError::Usage { message } => QuiltError::Usage {
                message: self.sanitize(message),
            },
            QuiltError::Schema {
                operation,
                column,
                message,
            } => QuiltError::Schema {
                operation: self.sanitize(operation),
                column: column.map(|v| self.sanitize(v)),
                message: self.sanitize(message),
            },
            QuiltError::Conversion {
                operation,
                column,
                message,
            } => QuiltError::Conversion {
                operation: self.sanitize(operation),
                column: column.map(|v| self.sanitize(v)),
                message: self.sanitize(message),
            },
            QuiltError::Io {
                operation,
                path,
                message,
            } => QuiltError::Io {
                operation: self.sanitize(operation),
                path: path.map(|v| self.sanitize(v)),
                message: self.sanitize(message),
            },
            QuiltError::Operation { operation, message } => QuiltError::Operation {
                operation: self.sanitize(operation),
                message: self.sanitize(message),
            },
            QuiltError::Finalizer { operation, message } => QuiltError::Finalizer {
                operation: self.sanitize(operation),
                message: self.sanitize(message),
            },
            QuiltError::Automation {
                stage,
                step,
                message,
                source,
            } => QuiltError::Automation {
                stage: if self.stage_is_sensitive(&stage) {
                    "<redacted>".into()
                } else {
                    self.sanitize(stage)
                },
                step: step.map(|v| self.sanitize(v)),
                message: self.sanitize(message),
                source: source.map(|v| Box::new(self.sanitize_error(*v))),
            },
        }
    }
    pub(super) fn step_is_sensitive(&self, stage: &str, index: usize) -> bool {
        self.sensitive_steps.contains(&(stage.to_string(), index))
    }
    pub(super) fn stage_is_sensitive(&self, stage: &str) -> bool {
        self.sensitive_stages.contains(stage)
    }
    pub(super) fn sanitize_preflight_error(&self, error: QuiltError) -> QuiltError {
        match error {
            QuiltError::Validation { diagnostics } => QuiltError::Validation {
                diagnostics: diagnostics
                    .into_iter()
                    .map(|d| {
                        if self
                            .sensitive_locations
                            .iter()
                            .any(|l| location_matches(&d, l))
                        {
                            d.rsplit_once(": ")
                                .map(|(l, _)| format!("{l}: <redacted diagnostic>"))
                                .unwrap_or_else(|| "<redacted diagnostic>".into())
                        } else {
                            self.sanitize(d)
                        }
                    })
                    .collect(),
            },
            other => self.sanitize_error(other),
        }
    }
}

pub(super) fn location_matches(diagnostic: &str, location: &str) -> bool {
    let mut offset = 0;
    while let Some(found) = diagnostic[offset..].find(location) {
        let start = offset + found;
        let end = start + location.len();
        if diagnostic[end..]
            .chars()
            .next()
            .is_none_or(|c| c == '.' || c == ':' || c == ' ')
        {
            return true;
        }
        offset = end;
    }
    false
}

pub(super) fn parse_step_location(path: &str) -> Option<(usize, usize)> {
    let stages = path.strip_prefix("run.stages[")?;
    let (stage, rest) = stages.split_once("].steps[")?;
    let (step, _) = rest.split_once(']')?;
    Some((stage.parse().ok()?, step.parse().ok()?))
}

pub(super) fn sensitive_location(path: &str) -> Option<String> {
    let location = path.strip_prefix("run.")?;
    if let Some(step_start) = location.find(".steps[") {
        let step_end =
            location[step_start + ".steps[".len()..].find(']')? + step_start + ".steps[".len();
        return Some(location[..=step_end].to_string());
    }
    if let Some(branch_start) = location.find(".branch") {
        let branch_end = branch_start + ".branch".len();
        return Some(location[..branch_end].to_string());
    }
    let stage_end = location.find(']')?;
    Some(location[..=stage_end].to_string())
}

pub(super) fn parse_stage_location(path: &str) -> Option<usize> {
    let stages = path.strip_prefix("run.stages[")?;
    let (stage, _) = stages.split_once(']')?;
    stage.parse().ok()
}

pub(super) fn discover_secrets(raw: &str, overrides: &[String]) -> DiagnosticPolicy {
    let Ok(root) = serde_yml::from_str::<Value>(raw) else {
        return DiagnosticPolicy::default();
    };
    let Some(raw_parameters) = root.get("parameters").and_then(Value::as_mapping) else {
        return DiagnosticPolicy::default();
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
    DiagnosticPolicy::from_values(secrets)
}

pub(super) fn redact(error: QuiltError, policy: &DiagnosticPolicy) -> QuiltError {
    policy.sanitize_error(error)
}

#[cfg(test)]
mod tests {
    use super::DiagnosticsContext;
    use std::path::Path;

    #[test]
    fn context_keeps_only_operation_location_metadata() {
        let context = DiagnosticsContext::for_config(Path::new("run.yaml"));
        assert_eq!(context.config_path(), Some("run.yaml"));
    }
}
