use crate::operations::quilters::sigma_json::{load_rules, referenced_fields, ZircRule};
use serde_yml::{Mapping, Value};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

pub fn sigma_to_quilt(
    input_path: &Path,
    output_path: Option<&Path>,
    annotate: bool,
    separate: bool,
) -> Result<Vec<PathBuf>, String> {
    if input_path.is_file() {
        let rules = load_rules(input_path)?;
        convert_ruleset(
            input_path,
            &rules,
            output_path,
            annotate,
            separate,
            input_path.parent().unwrap_or_else(|| Path::new(".")),
        )
    } else if input_path.is_dir() {
        let output_dir = output_path.ok_or_else(|| {
            "Directory conversion requires -o/--output <dir> for sigma2quilt.".to_string()
        })?;
        if output_dir.extension().is_some() {
            return Err(
                "Directory conversion requires -o/--output to be a directory path, not a file."
                    .to_string(),
            );
        }
        fs::create_dir_all(output_dir).map_err(|e| {
            format!(
                "Failed to create output directory {}: {e}",
                output_dir.display()
            )
        })?;

        let mut outputs = Vec::new();
        for entry in fs::read_dir(input_path)
            .map_err(|e| format!("Failed to read directory {}: {e}", input_path.display()))?
        {
            let entry = entry.map_err(|e| format!("Failed to read directory entry: {e}"))?;
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }

            let rules = load_rules(&path)?;
            outputs.extend(convert_ruleset(
                &path,
                &rules,
                Some(output_dir),
                annotate,
                separate,
                output_dir,
            )?);
        }

        if outputs.is_empty() {
            Err(format!(
                "No Zircolite JSON rule files (*.json) found under {}.",
                input_path.display()
            ))
        } else {
            outputs.sort();
            Ok(outputs)
        }
    } else {
        Err(format!(
            "Input path '{}' does not exist or is not accessible.",
            input_path.display()
        ))
    }
}

fn convert_ruleset(
    input_path: &Path,
    rules: &[ZircRule],
    output_path: Option<&Path>,
    annotate: bool,
    separate: bool,
    default_dir: &Path,
) -> Result<Vec<PathBuf>, String> {
    if rules.is_empty() {
        return Err(format!(
            "No rules with SQL content found in {}.",
            input_path.display()
        ));
    }

    if separate {
        let output_dir = output_path
            .filter(|path| path.extension().is_none())
            .unwrap_or(default_dir);
        fs::create_dir_all(output_dir).map_err(|e| {
            format!(
                "Failed to create output directory {}: {e}",
                output_dir.display()
            )
        })?;

        let mut outputs = Vec::new();
        let mut seen_names: HashMap<String, usize> = HashMap::new();
        for rule in rules {
            let filename = unique_rule_output_filename(rule, &mut seen_names);
            let output_file = output_dir.join(filename);
            let yaml = build_quilt_yaml(&output_file, std::slice::from_ref(rule), annotate);
            write_text_file(&output_file, &yaml)?;
            outputs.push(output_file);
        }
        let mapping_file = ruleset_mapping_file_path(input_path, output_dir);
        write_mapping_file(&mapping_file, rules)?;
        outputs.push(mapping_file);
        Ok(outputs)
    } else {
        let output_file = output_path
            .map(Path::to_path_buf)
            .unwrap_or_else(|| default_output_file(input_path, rules));
        let yaml = build_quilt_yaml(input_path, rules, annotate);
        write_text_file(&output_file, &yaml)?;
        let mapping_file = mapping_file_path(&output_file);
        write_mapping_file(&mapping_file, rules)?;
        Ok(vec![output_file, mapping_file])
    }
}

fn build_quilt_yaml(input_path: &Path, rules: &[ZircRule], annotate: bool) -> String {
    let mut stages = Mapping::new();
    stages.insert(
        Value::String("load_stage".to_string()),
        process_stage(None, load_steps("${input}")),
    );

    let mut detect_stage_names = Vec::new();
    for (index, rule) in rules.iter().enumerate() {
        let stage_name = unique_stage_name(&rule.title, index);
        stages.insert(
            Value::String(stage_name.clone()),
            process_stage(
                Some("load_stage"),
                where_steps(
                    rule.rule.first().map(String::as_str).unwrap_or_default(),
                    rule,
                    annotate,
                ),
            ),
        );
        detect_stage_names.push(stage_name);
    }

    let output_source = if detect_stage_names.len() == 1 {
        detect_stage_names[0].clone()
    } else {
        stages.insert(
            Value::String("merge_detections".to_string()),
            concat_stage(&detect_stage_names),
        );
        "merge_detections".to_string()
    };

    stages.insert(
        Value::String("output_stage".to_string()),
        output_stage(&output_source),
    );

    let title = format!(
        "Sigma JSON Conversion: {}",
        input_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("rules")
    );
    let description = format!("Generated by sigma2quilt from {} rules", rules.len());

    serialize_quilt(title, Some(description), stages)
}

fn load_steps(path: &str) -> Value {
    let mut load_args = Mapping::new();
    load_args.insert(
        Value::String("path".to_string()),
        Value::String(path.to_string()),
    );
    let mut steps = Mapping::new();
    steps.insert(Value::String("load".to_string()), Value::Mapping(load_args));
    Value::Mapping(steps)
}

fn where_steps(sql: &str, rule: &ZircRule, annotate: bool) -> Value {
    let mut where_args = Mapping::new();
    where_args.insert(
        Value::String("sql".to_string()),
        Value::String(sql.to_string()),
    );

    if annotate {
        where_args.insert(Value::String("annotate".to_string()), Value::Bool(true));
        where_args.insert(
            Value::String("sigma_title".to_string()),
            Value::String(rule.title.clone()),
        );
        where_args.insert(
            Value::String("sigma_id".to_string()),
            Value::String(rule.id.clone().unwrap_or_default()),
        );
        where_args.insert(
            Value::String("sigma_level".to_string()),
            Value::String(rule.level.clone().unwrap_or_default()),
        );
        where_args.insert(
            Value::String("sigma_tags".to_string()),
            Value::String(rule.tags.clone().unwrap_or_default().join(",")),
        );
    }

    let mut steps = Mapping::new();
    steps.insert(
        Value::String("where".to_string()),
        Value::Mapping(where_args),
    );
    Value::Mapping(steps)
}

fn process_stage(source: Option<&str>, steps: Value) -> Value {
    let mut stage = Mapping::new();
    stage.insert(
        Value::String("type".to_string()),
        Value::String("process".to_string()),
    );
    if let Some(source) = source {
        stage.insert(
            Value::String("source".to_string()),
            Value::String(source.to_string()),
        );
    }
    stage.insert(Value::String("steps".to_string()), steps);
    Value::Mapping(stage)
}

fn concat_stage(source_names: &[String]) -> Value {
    let mut params = Mapping::new();
    params.insert(
        Value::String("how".to_string()),
        Value::String("vertical".to_string()),
    );

    let mut stage = Mapping::new();
    stage.insert(
        Value::String("type".to_string()),
        Value::String("concat".to_string()),
    );
    stage.insert(
        Value::String("sources".to_string()),
        Value::Sequence(source_names.iter().cloned().map(Value::String).collect()),
    );
    stage.insert(Value::String("params".to_string()), Value::Mapping(params));
    Value::Mapping(stage)
}

fn output_stage(source: &str) -> Value {
    let mut dump_args = Mapping::new();
    dump_args.insert(
        Value::String("output".to_string()),
        Value::String("${output}".to_string()),
    );

    let mut steps = Mapping::new();
    steps.insert(Value::String("dump".to_string()), Value::Mapping(dump_args));

    let mut stage = Mapping::new();
    stage.insert(
        Value::String("type".to_string()),
        Value::String("output".to_string()),
    );
    stage.insert(
        Value::String("source".to_string()),
        Value::String(source.to_string()),
    );
    stage.insert(Value::String("steps".to_string()), Value::Mapping(steps));
    Value::Mapping(stage)
}

fn serialize_quilt(title: String, description: Option<String>, stages: Mapping) -> String {
    let mut root = Mapping::new();
    root.insert(Value::String("title".to_string()), Value::String(title));
    if let Some(description) = description {
        root.insert(
            Value::String("description".to_string()),
            Value::String(description),
        );
    }
    root.insert(Value::String("stages".to_string()), Value::Mapping(stages));
    serde_yml::to_string(&Value::Mapping(root)).unwrap_or_default()
}

fn default_output_file(input_path: &Path, rules: &[ZircRule]) -> PathBuf {
    input_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(if rules.len() == 1 {
            rule_output_filename(&rules[0].title)
        } else {
            default_ruleset_output_filename(input_path)
        })
}

fn default_ruleset_output_filename(input_path: &Path) -> String {
    let stem = input_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("rules");
    format!("quilt-{stem}.yaml")
}

fn rule_output_filename(title: &str) -> String {
    format!("quilt-{}.yaml", slugify_title(title))
}

fn mapping_file_path(quilt_output: &Path) -> PathBuf {
    let parent = quilt_output.parent().unwrap_or_else(|| Path::new("."));
    let stem = quilt_output
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("quilt");
    parent.join(format!("{stem}_mapping.json"))
}

fn ruleset_mapping_file_path(input_path: &Path, output_dir: &Path) -> PathBuf {
    let stem = input_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("rules");
    output_dir.join(format!("quilt-{stem}_mapping.json"))
}

fn write_mapping_file(path: &Path, rules: &[ZircRule]) -> Result<(), String> {
    let mut fields = BTreeSet::new();
    for rule in rules {
        for sql in &rule.rule {
            for field in referenced_fields(sql) {
                fields.insert(field);
            }
        }
    }

    let mapping: HashMap<String, String> = fields
        .into_iter()
        .map(|field| (field, String::new()))
        .collect();
    let content = serde_json::to_string_pretty(&mapping).map_err(|e| {
        format!(
            "Failed to serialize mapping template {}: {e}",
            path.display()
        )
    })?;
    write_text_file(path, &content)
}

fn unique_rule_output_filename(rule: &ZircRule, seen_names: &mut HashMap<String, usize>) -> String {
    let base = rule_output_filename(&rule.title);
    let count = seen_names
        .entry(base.clone())
        .and_modify(|count| *count += 1)
        .or_insert(1);
    if *count == 1 {
        base
    } else {
        let stem = base.strip_suffix(".yaml").unwrap_or(&base);
        format!("{stem}-{}.yaml", *count)
    }
}

fn unique_stage_name(title: &str, index: usize) -> String {
    format!("detect_{}_{}", index + 1, sanitize_stage_name(title))
}

fn sanitize_stage_name(input: &str) -> String {
    let mut normalized = input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    while normalized.contains("__") {
        normalized = normalized.replace("__", "_");
    }
    let normalized = normalized.trim_matches('_');
    if normalized.is_empty() {
        "sigma_rule".to_string()
    } else {
        normalized.to_string()
    }
}

fn slugify_title(input: &str) -> String {
    let mut slug = String::new();
    let mut previous_was_sep = false;

    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            previous_was_sep = false;
        } else if !previous_was_sep {
            slug.push('-');
            previous_was_sep = true;
        }
    }

    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "sigma-rule".to_string()
    } else {
        slug.to_string()
    }
}

fn write_text_file(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory {}: {e}", parent.display()))?;
    }
    fs::write(path, content).map_err(|e| format!("Failed to write {}: {e}", path.display()))
}
