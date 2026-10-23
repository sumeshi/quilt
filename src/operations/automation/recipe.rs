use super::{diagnostics, document, orchestrator};
use crate::controllers::command_model::TypedCommand;
use crate::controllers::command_model::{by_name, CommandCategory};
use crate::error::QuiltError;
use crate::operations::automation::model::{PlannedStep, StageConfig};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Load a validated v2 run document as one editable linear command sequence.
/// DAG stages and routing are deliberately rejected because the lazy editor
/// has one pipeline and cannot represent their execution semantics.
pub fn load_lazy_recipe(
    recipe: &Path,
    input_files: &[PathBuf],
    overrides: &[String],
) -> Result<Vec<TypedCommand>, QuiltError> {
    let recipe = if recipe.is_absolute() {
        recipe.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| QuiltError::io("lazy recipe", None::<String>, e.to_string()))?
            .join(recipe)
    };
    let raw = std::fs::read_to_string(&recipe).map_err(|error| {
        QuiltError::io(
            "read lazy recipe",
            Some(recipe.display().to_string()),
            error.to_string(),
        )
    })?;
    let initial_diagnostics = diagnostics::discover_secrets(&raw, overrides);
    let mut effective_overrides = overrides.to_vec();
    add_unused_finalizer_parameter_values(&raw, overrides, &mut effective_overrides)
        .map_err(|error| diagnostics::redact(error, &initial_diagnostics))?;
    let discovered = diagnostics::discover_secrets(&raw, &effective_overrides);
    let prepared = orchestrator::prepare_document(&raw, &recipe, &effective_overrides)
        .map_err(|error| diagnostics::redact(error, &discovered))?;
    let record_parameters = record_parameter_uses(&raw)
        .map_err(|error| diagnostics::redact(error, &prepared.diagnostics))?;
    if record_parameters.iter().any(|name| {
        prepared
            .parameters
            .get(name)
            .is_some_and(|value| value.secret)
    }) {
        return Err(QuiltError::usage(
            "lazy editor cannot open a recipe whose pipeline steps use secret parameters because editing or exporting would reveal their values; use `qlt run`",
        ));
    }
    let plan = document::preflight(&prepared.document, &prepared.parameters, &recipe).map_err(
        |error| {
            prepared
                .diagnostics
                .sanitize_preflight_error(document::with_source_locations(error, &raw, &recipe))
        },
    )?;
    if plan.order.len() != prepared.document.stages.len() {
        return Err(QuiltError::usage(
            "lazy recipes must be a single linear workflow",
        ));
    }
    let mut commands = Vec::new();
    let mut saw_load = false;
    let mut previous: Option<String> = None;
    for name in &plan.order {
        let stage = plan.stages.get(name).expect("planned stage exists");
        let StageConfig::Process(process) = stage else {
            return Err(QuiltError::usage(
                "lazy recipes support process stages only; join, concat, and branch stages are unsupported",
            ));
        };
        if process.source.as_ref() != previous.as_ref() {
            return Err(QuiltError::usage(
                "lazy recipes must connect process stages in one linear chain",
            ));
        }
        for step in &process.steps {
            let PlannedStep { command, .. } = step;
            if step.load_paths_omitted && process.source.is_some() {
                continue;
            }
            if let TypedCommand::Load(load) = command {
                if saw_load {
                    return Err(QuiltError::usage(
                        "lazy recipes support one explicit load step; a later load would replace the current pipeline",
                    ));
                }
                saw_load = true;
                if load.rejects.is_some() {
                    return Err(QuiltError::usage(
                        "lazy recipes cannot write load rejects files",
                    ));
                }
            }
            if command.category() == crate::controllers::command_model::CommandCategory::Finalizer {
                continue;
            }
            commands.push(command.clone());
        }
        previous = Some(name.clone());
    }
    if !input_files.is_empty() {
        let load = commands
            .iter_mut()
            .find_map(|command| match command {
                TypedCommand::Load(args) => Some(args),
                _ => None,
            })
            .ok_or_else(|| {
                QuiltError::usage("lazy recipe has no load step for input file overrides")
            })?;
        load.paths = input_files
            .iter()
            .map(|path| {
                if path.is_absolute() {
                    path.clone()
                } else {
                    std::env::current_dir().unwrap_or_default().join(path)
                }
            })
            .collect();
    }
    Ok(commands)
}

fn record_parameter_uses(raw: &str) -> Result<HashSet<String>, QuiltError> {
    let value: yaml_serde::Value = yaml_serde::from_str(raw)
        .map_err(|error| QuiltError::usage(format!("Error parsing lazy recipe: {error}")))?;
    let mut found = HashSet::new();
    if let Some(stages) = value.get("stages").and_then(yaml_serde::Value::as_sequence) {
        for stage in stages {
            let Some(steps) = stage.get("steps").and_then(yaml_serde::Value::as_sequence) else {
                continue;
            };
            for step in steps {
                let Some((name, args)) = step.as_mapping().and_then(|map| map.iter().next()) else {
                    continue;
                };
                if name
                    .as_str()
                    .and_then(by_name)
                    .is_some_and(|spec| spec.category == CommandCategory::Finalizer)
                {
                    continue;
                }
                collect_parameters(args, &mut found);
            }
        }
    }
    Ok(found)
}

fn add_unused_finalizer_parameter_values(
    raw: &str,
    user_overrides: &[String],
    effective: &mut Vec<String>,
) -> Result<(), QuiltError> {
    let value: yaml_serde::Value = yaml_serde::from_str(raw)
        .map_err(|error| QuiltError::usage(format!("Error parsing lazy recipe: {error}")))?;
    let declarations = value
        .get("parameters")
        .and_then(yaml_serde::Value::as_mapping);
    let Some(declarations) = declarations else {
        return Ok(());
    };
    let mut overridden = user_overrides
        .iter()
        .filter_map(|item| item.split_once('=').map(|(name, _)| name.to_owned()))
        .collect::<HashSet<_>>();
    let mut finalizer_uses = HashSet::new();
    let mut record_uses = HashSet::new();
    if let Some(stages) = value.get("stages").and_then(yaml_serde::Value::as_sequence) {
        for stage in stages {
            let Some(steps) = stage.get("steps").and_then(yaml_serde::Value::as_sequence) else {
                continue;
            };
            for step in steps {
                let Some((name, args)) = step.as_mapping().and_then(|map| map.iter().next()) else {
                    continue;
                };
                let is_finalizer = name
                    .as_str()
                    .and_then(by_name)
                    .is_some_and(|spec| spec.category == CommandCategory::Finalizer);
                let uses = if is_finalizer {
                    &mut finalizer_uses
                } else {
                    &mut record_uses
                };
                collect_parameters(args, uses);
            }
        }
    }
    for (name, declaration) in declarations {
        let Some(name) = name.as_str() else {
            continue;
        };
        if overridden.contains(name) || record_uses.contains(name) || !finalizer_uses.contains(name)
        {
            continue;
        }
        let Some(fields) = declaration.as_mapping() else {
            continue;
        };
        let required = fields
            .get(yaml_serde::Value::String("required".into()))
            .and_then(yaml_serde::Value::as_bool)
            .unwrap_or(false);
        let has_default = fields.contains_key(yaml_serde::Value::String("default".into()));
        if !required || has_default {
            continue;
        }
        let ty = fields
            .get(yaml_serde::Value::String("type".into()))
            .and_then(yaml_serde::Value::as_str)
            .unwrap_or("");
        let fallback = match ty {
            "path" => std::env::temp_dir()
                .join(format!("qlt-lazy-unused-output-{}", std::process::id()))
                .to_string_lossy()
                .into_owned(),
            "string" => "qlt-lazy-unused-output".into(),
            "int" => "0".into(),
            "bool" => "false".into(),
            _ => continue,
        };
        effective.push(format!("{name}={fallback}"));
        overridden.insert(name.to_owned());
    }
    Ok(())
}

fn collect_parameters(value: &yaml_serde::Value, found: &mut HashSet<String>) {
    if let Some(mapping) = value.as_mapping() {
        if mapping.len() == 1 {
            if let Some(name) = mapping
                .get(yaml_serde::Value::String("$param".into()))
                .and_then(yaml_serde::Value::as_str)
            {
                found.insert(name.to_owned());
                return;
            }
        }
        for (_, value) in mapping.iter() {
            collect_parameters(value, found);
        }
    } else if let Some(sequence) = value.as_sequence() {
        for value in sequence {
            collect_parameters(value, found);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::load_lazy_recipe;
    use crate::controllers::command_model::TypedCommand;

    #[test]
    fn reopen_recipe_applies_file_override_and_ignores_unused_required_finalizer_output() {
        let directory = tempfile::tempdir().unwrap();
        let recipe = directory.path().join("workflow.yaml");
        let input = directory.path().join("override.csv");
        std::fs::write(&input, "id,name\n001,Ada\n").unwrap();
        std::fs::write(
            &recipe,
            r#"
version: 2
parameters:
  output:
    type: path
    required: true
stages:
  - name: input
    steps:
      - load:
          paths: [placeholder.csv]
          all-strings: true
      - head:
          number: 1
      - dump:
          output:
            $param: output
"#,
        )
        .unwrap();

        let commands = load_lazy_recipe(&recipe, std::slice::from_ref(&input), &[]).unwrap();
        assert_eq!(commands.len(), 2);
        let TypedCommand::Load(load) = &commands[0] else {
            panic!("first command is not load")
        };
        assert_eq!(load.paths, vec![input.clone()]);
        assert!(load.all_strings);
        assert!(matches!(commands[1], TypedCommand::Head(_)));
    }

    #[test]
    fn recipe_rejects_branching_stage_shapes() {
        let directory = tempfile::tempdir().unwrap();
        let recipe = directory.path().join("workflow.yaml");
        std::fs::write(&recipe, "version: 2\nstages: [{name: input, steps: [{load: {paths: [x.csv]}}]}, {name: join, join: {inputs: [input, input], on: [id]}}]\n").unwrap();
        let error = load_lazy_recipe(&recipe, &[], &[]).unwrap_err();
        assert!(error.to_string().contains("process stages only"));
    }

    #[test]
    fn reopen_recipe_skips_inherited_empty_load_steps() {
        let directory = tempfile::tempdir().unwrap();
        let recipe = directory.path().join("inherited.yaml");
        std::fs::write(
            &recipe,
            r#"
version: 2
stages:
  - name: input
    steps:
      - load: {paths: [placeholder.csv]}
      - head: {number: 2}
  - name: inherited
    from: input
    steps:
      - load: {}
      - tail: {number: 1}
"#,
        )
        .unwrap();
        let commands = load_lazy_recipe(&recipe, &[], &[]).unwrap();
        assert_eq!(commands.len(), 3);
        assert!(matches!(commands[0], TypedCommand::Load(_)));
        assert!(matches!(commands[1], TypedCommand::Head(_)));
        assert!(matches!(commands[2], TypedCommand::Tail(_)));
    }

    #[test]
    fn recipe_rejects_secret_record_parameters_before_editor_export() {
        let directory = tempfile::tempdir().unwrap();
        let recipe = directory.path().join("secret.yaml");
        std::fs::write(
            &recipe,
            r#"
version: 2
parameters:
  needle:
    type: string
    required: true
    secret: true
stages:
  - name: input
    steps:
      - load: {paths: [input.csv]}
      - grep: {pattern: {$param: needle}}
"#,
        )
        .unwrap();
        let error = load_lazy_recipe(&recipe, &[], &["needle=private-value".into()]).unwrap_err();
        let rendered = error.to_string();
        assert!(rendered.contains("use `qlt run`"));
        assert!(!rendered.contains("private-value"));
    }

    #[test]
    fn secret_parameter_preflight_errors_are_redacted_like_run() {
        let directory = tempfile::tempdir().unwrap();
        let recipe = directory.path().join("secret-finalizer.yaml");
        let sentinel = "PRIVATE_SENTINEL_49382";
        std::fs::write(
            &recipe,
            format!(
                r#"{{"version":2,"parameters":{{"token":{{"type":"string","secret":true,"default":"{sentinel}"}}}},"stages":[{{"name":"pipeline","steps":[{{"load":{{"paths":["events.csv"]}}}},{{"calc":{{"column":"x","sum":{{"$param":"token"}}}}}}]}}]}}"#
            ),
        )
        .unwrap();

        let error = load_lazy_recipe(&recipe, &[], &[]).unwrap_err().to_string();
        assert!(!error.contains(sentinel), "{error}");
        assert!(error.contains("<redacted diagnostic>"), "{error}");
    }
}
