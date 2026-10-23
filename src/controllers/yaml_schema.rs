//! Test-time JSON Schema generation for the version 2 run-document format.
//!
//! Command argument schemas are generated from the same private DTOs used by
//! `yaml_adapter`; the document and stage schemas come from the automation
//! model. The checked-in artifact is refreshed by setting
//! `UPDATE_RUN_SCHEMA=1` while running this module's test.

use crate::controllers::definitions::{registry, CommandCategory};
use crate::controllers::yaml_adapter::command_argument_schemas;
use crate::operations::automation::run::RunDocument;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub(crate) fn generate_run_v2_schema() -> Value {
    let mut schema = serde_json::to_value(schemars::schema_for!(RunDocument))
        .expect("run document schema serializes");
    let defs = schema
        .get("$defs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let stages_pointer = if schema.pointer("/properties/stages/items").is_some() {
        "/properties/stages/items"
    } else {
        "/$defs/RunDocument/properties/stages/items"
    };
    if let Some(stages) = schema.pointer_mut(stages_pointer) {
        let mut expanded = inline_refs(stages.clone(), &defs, &mut Vec::new());
        replace_process_steps(&mut expanded, &command_argument_schemas());
        strip_nested_metadata(&mut expanded);
        add_aggregate_constraints(&mut expanded);
        add_parameter_refs(&mut expanded);
        *stages = expanded;
    }

    let document_properties = if schema.pointer("/properties").is_some() {
        "/properties"
    } else {
        "/$defs/RunDocument/properties"
    };
    if let Some(properties) = schema
        .pointer_mut(document_properties)
        .and_then(Value::as_object_mut)
    {
        if let Some(version) = properties.get_mut("version") {
            *version = json!({ "type": "integer", "const": 2 });
        }
        for key in ["title", "description", "author"] {
            if let Some(value) = properties.get_mut(key) {
                add_parameter_refs(value);
            }
        }
    }

    schema["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
    schema
}

fn inline_refs(schema: Value, defs: &Map<String, Value>, stack: &mut Vec<String>) -> Value {
    match schema {
        Value::Object(mut object) => {
            if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                if let Some(name) = reference.strip_prefix("#/$defs/") {
                    if !stack.iter().any(|entry| entry == name) {
                        if let Some(definition) = defs.get(name) {
                            stack.push(name.to_string());
                            let mut inlined = inline_refs(definition.clone(), defs, stack);
                            stack.pop();
                            object.remove("$ref");
                            if object.is_empty() {
                                return inlined;
                            }
                            if let Some(base) = inlined.as_object_mut() {
                                for (key, value) in object {
                                    base.insert(key, inline_refs(value, defs, stack));
                                }
                                return inlined;
                            }
                            return json!({"allOf": [inlined, object]});
                        }
                    }
                }
            }
            for value in object.values_mut() {
                *value = inline_refs(value.take(), defs, stack);
            }
            Value::Object(object)
        }
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| inline_refs(value, defs, stack))
                .collect(),
        ),
        scalar => scalar,
    }
}

fn replace_process_steps(schema: &mut Value, command_schemas: &BTreeMap<String, Value>) {
    if let Some(object) = schema.as_object_mut() {
        if object.contains_key("steps") {
            if let Some(steps) = object.get_mut("steps") {
                let variants = command_schemas
                    .iter()
                    .map(|(name, argument_schema)| {
                        let command_defs = argument_schema
                            .get("$defs")
                            .and_then(Value::as_object)
                            .cloned()
                            .unwrap_or_default();
                        let mut argument_schema =
                            inline_refs(argument_schema.clone(), &command_defs, &mut vec![]);
                        strip_nested_metadata(&mut argument_schema);
                        let argument_schema = if argument_schema
                            .get("required")
                            .and_then(Value::as_array)
                            .is_none_or(Vec::is_empty)
                        {
                            json!({
                                "anyOf": [argument_schema, { "type": "null" }]
                            })
                        } else {
                            argument_schema
                        };
                        json!({
                            "type": "object",
                            "properties": { (name): argument_schema },
                            "required": [name],
                            "additionalProperties": false
                        })
                    })
                    .collect::<Vec<_>>();
                *steps = json!({
                    "type": "array",
                    "items": { "oneOf": variants }
                });
            }
        }
        for value in object.values_mut() {
            replace_process_steps(value, command_schemas);
        }
    } else if let Some(array) = schema.as_array_mut() {
        for value in array {
            replace_process_steps(value, command_schemas);
        }
    }
}

fn strip_nested_metadata(schema: &mut Value) {
    if let Some(object) = schema.as_object_mut() {
        object.remove("$defs");
        object.remove("$schema");
        for value in object.values_mut() {
            strip_nested_metadata(value);
        }
    } else if let Some(array) = schema.as_array_mut() {
        for value in array {
            strip_nested_metadata(value);
        }
    }
}

fn add_parameter_refs(schema: &mut Value) {
    if let Some(object) = schema.as_object_mut() {
        for value in object.values_mut() {
            add_parameter_refs(value);
        }
        let is_typed_value = object.get("type").is_some_and(|value| {
            value == "string"
                || value == "integer"
                || value == "number"
                || value == "boolean"
                || value.as_array().is_some_and(|types| {
                    types.iter().any(|kind| {
                        matches!(
                            kind.as_str(),
                            Some("string" | "integer" | "number" | "boolean")
                        )
                    })
                })
        });
        if is_typed_value {
            let native = Value::Object(object.clone());
            *schema = json!({
                "anyOf": [
                    native,
                    {
                        "type": "object",
                        "properties": { "$param": { "type": "string" } },
                        "required": ["$param"],
                        "additionalProperties": false
                    }
                ]
            });
        }
    } else if let Some(array) = schema.as_array_mut() {
        for value in array {
            add_parameter_refs(value);
        }
    }
}

#[cfg(test)]
fn add_aggregate_constraints(schema: &mut Value) {
    let mut modes = vec![
        "count".to_string(),
        "sum".to_string(),
        "avg".to_string(),
        "min".to_string(),
        "max".to_string(),
        "median".to_string(),
        "std".to_string(),
        "nunique".to_string(),
    ];
    modes.extend((0..=100).map(|percentile| format!("p{percentile}")));
    visit_metric_items(schema, &modes);
}

#[cfg(test)]
fn visit_metric_items(schema: &mut Value, modes: &[String]) {
    if let Some(object) = schema.as_object_mut() {
        if object.contains_key("metrics") {
            object["metrics"]["minItems"] = json!(1);
            if let Some(items) = object
                .get_mut("metrics")
                .and_then(|metrics| metrics.get_mut("items"))
            {
                items["properties"]["mode"] = json!({ "type": "string", "enum": modes });
                items["allOf"] = json!([
                    {
                        "if": {
                            "properties": { "mode": { "enum": modes.iter().filter(|mode| mode.as_str() != "count").collect::<Vec<_>>() } },
                            "required": ["mode"]
                        },
                        "then": {
                            "required": ["column"],
                            "properties": { "column": { "not": { "type": "null" } } }
                        }
                    }
                ]);
                items["properties"]["output"]["minLength"] = json!(1);
            }
        }
        for value in object.values_mut() {
            visit_metric_items(value, modes);
        }
    } else if let Some(array) = schema.as_array_mut() {
        for value in array {
            visit_metric_items(value, modes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema_text() -> String {
        let schema = generate_run_v2_schema();
        format!("{}\n", serde_json::to_string_pretty(&schema).unwrap())
    }

    #[test]
    fn checked_in_schema_matches_typed_sources_and_core_contract() {
        let generated = schema_text();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/schemas/run-v2.schema.json");
        if std::env::var_os("UPDATE_RUN_SCHEMA").is_some() {
            std::fs::write(path, &generated).expect("write generated schema artifact");
        } else {
            assert!(
                generated == include_str!("../../schemas/run-v2.schema.json").replace("\r\n", "\n"),
                "schema artifact drift; run with UPDATE_RUN_SCHEMA=1 to refresh"
            );
        }
        let generated = command_argument_schemas();
        let registered = registry()
            .iter()
            .filter(|command| command.category != CommandCategory::Automation)
            .map(|command| command.name)
            .collect::<Vec<_>>();
        assert_eq!(generated.len(), registered.len());
        for name in registered {
            assert!(generated.contains_key(name), "missing schema for {name}");
        }
    }
}
