use crate::error::QuiltError;
use polars::prelude::*;
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

pub(super) fn is_gzip(path: &Path) -> bool {
    let name = path.to_string_lossy().to_ascii_lowercase();
    name.ends_with(".jsonl.gz") || name.ends_with(".ndjson.gz")
}

pub(super) fn prepare_path(
    path: &Path,
    resources: &crate::controllers::resources::ExecutionResources,
) -> Result<std::path::PathBuf, QuiltError> {
    if !is_gzip(path) {
        return Ok(path.to_path_buf());
    }
    if !resources.temp_files_enabled() {
        return Err(QuiltError::usage(
            "run --show-plan cannot inspect gzipped NDJSON without execution",
        ));
    }
    let input = File::open(path).map_err(|error| diagnostic(path, 0, error))?;
    let mut decoder = flate2::read::MultiGzDecoder::new(BufReader::new(input));
    let mut reservation = resources
        .reserve_temp_file("qlt-jsonl-spool", "jsonl")
        .map_err(|error| diagnostic(path, 0, error))?;
    let destination = reservation.path().to_path_buf();
    let file = reservation
        .file_mut()
        .ok_or_else(|| diagnostic(path, 0, "spool reservation unavailable"))?;
    std::io::copy(&mut decoder, file)
        .map_err(|error| diagnostic(path, 0, format!("decompress gzip: {error}")))?;
    resources
        .retain_temp_file(reservation)
        .map_err(|error| diagnostic(path, 0, error))?;
    Ok(destination)
}

// Retain the sign/range of integers until the final schema is constructed.
// Polars' generic JSON inference can otherwise promote them through Float64.
#[derive(Clone, Debug, PartialEq)]
enum JsonType {
    Unknown,
    Null,
    Bool,
    Integer { negative: bool, wide: bool },
    Float,
    String,
    List(Box<JsonType>),
    Object(Vec<(String, JsonType)>, bool),
}

impl JsonType {
    fn infer(value: &Value) -> Result<Self, String> {
        Ok(match value {
            Value::Null => Self::Null,
            Value::Bool(_) => Self::Bool,
            Value::String(_) => Self::String,
            Value::Number(number) => {
                if let Some(value) = number.as_i64() {
                    Self::Integer {
                        negative: value < 0,
                        wide: false,
                    }
                } else if number.as_u64().is_some() {
                    Self::Integer {
                        negative: false,
                        wide: true,
                    }
                } else if number.is_f64() && number.as_f64().is_some_and(f64::is_finite) {
                    Self::Float
                } else {
                    return Err("number outside supported Int64/UInt64/Float64 range".into());
                }
            }
            Value::Array(values) => {
                let mut inner = Self::Unknown;
                for value in values {
                    inner.merge(Self::infer(value)?, "[]", false)?;
                }
                Self::List(Box::new(inner))
            }
            Value::Object(fields) => Self::Object(
                fields
                    .iter()
                    .map(|(name, value)| {
                        Self::infer(value)
                            .map(|dtype| (name.clone(), dtype))
                            .map_err(|error| format!("{name}: {error}"))
                    })
                    .collect::<Result<_, _>>()?,
                false,
            ),
        })
    }

    fn merge(&mut self, mut other: Self, path: &str, bounded: bool) -> Result<(), String> {
        if matches!(other, Self::Unknown) {
            return Ok(());
        }
        if matches!(other, Self::Null) {
            if matches!(self, Self::Unknown) {
                *self = Self::Null;
            } else {
                self.mark_nullable();
            }
            return Ok(());
        }
        if matches!(self, Self::Null | Self::Unknown) {
            if bounded {
                return Err(format!("{path}: type first appears after inference limit; use --infer-schema-length full"));
            }
            if matches!(self, Self::Null) {
                other.mark_nullable();
            }
            *self = other;
            return Ok(());
        }
        match (&mut *self, other) {
            (Self::Object(fields, nullable), Self::Object(incoming, rhs_nullable)) => {
                *nullable |= rhs_nullable;
                for (name, existing) in fields.iter_mut() {
                    if !incoming.iter().any(|(key, _)| key == name) { existing.mark_nullable(); }
                }
                for (name, mut dtype) in incoming {
                    let child = if path.is_empty() { name.clone() } else { format!("{path}.{name}") };
                    if let Some((_, existing)) = fields.iter_mut().find(|(key, _)| key == &name) {
                        existing.merge(dtype, &child, bounded)?;
                    } else if bounded {
                        return Err(format!("{child}: field first appears after inference limit; use --infer-schema-length full"));
                    } else {
                        dtype.mark_nullable();
                        fields.push((name, dtype));
                    }
                }
            }
            (Self::List(inner), Self::List(other)) => inner.merge(*other, &format!("{path}[]"), bounded)?,
            (Self::Integer { negative, wide }, Self::Integer { negative: rhs_negative, wide: rhs_wide }) => {
                if (*negative || rhs_negative) && (*wide || rhs_wide) {
                    return Err(format!("{path}: incompatible signed/unsigned integer range; encode these IDs as strings"));
                }
                if bounded && rhs_wide && !*wide {
                    return Err(format!("{path}: integer range widens after inference limit; use --infer-schema-length full"));
                }
                *negative |= rhs_negative;
                *wide |= rhs_wide;
            }
            (left, right) if *left == right => (),
            (left, right) => return Err(format!("{path}: incompatible NDJSON types: {left:?} and {right:?}; implicit coercion is disabled to preserve precision")),
        }
        Ok(())
    }

    fn mark_nullable(&mut self) {
        if let Self::Object(_, nullable) = self {
            *nullable = true;
        }
    }

    fn validate_empty(&self, path: &str, ancestor_nullable: bool) -> Result<(), String> {
        match self {
            Self::Object(fields, nullable) => {
                let nullable = *nullable || ancestor_nullable;
                if fields.is_empty() && nullable {
                    return Err(format!("{path}: nullable empty objects are unsupported by the Polars NDJSON reader (it loses their null validity); add a typed field or normalize this field before loading"));
                }
                for (name, dtype) in fields {
                    dtype.validate_empty(&format!("{path}.{name}"), nullable)?;
                }
            }
            Self::List(inner) => inner.validate_empty(&format!("{path}[]"), false)?,
            _ => (),
        }
        Ok(())
    }

    fn dtype(&self) -> DataType {
        match self {
            Self::Unknown | Self::Null => DataType::Null,
            Self::Bool => DataType::Boolean,
            Self::Integer { wide: true, .. } => DataType::UInt64,
            Self::Integer { .. } => DataType::Int64,
            Self::Float => DataType::Float64,
            Self::String => DataType::String,
            Self::List(inner) => DataType::List(Box::new(inner.dtype())),
            Self::Object(fields, _) => DataType::Struct(
                fields
                    .iter()
                    .map(|(name, dtype)| Field::new(name.into(), dtype.dtype()))
                    .collect(),
            ),
        }
    }
}

fn diagnostic(path: &Path, line: usize, message: impl std::fmt::Display) -> QuiltError {
    QuiltError::schema(
        "validate NDJSON",
        None::<String>,
        format!("{}: line {line}: {message}", path.display()),
    )
}

// Traverse each record without building a second JSON tree. serde_json::Value
// intentionally keeps only the last occurrence of an object key, so this pass
// must run before Value parsing to make duplicate keys observable.
struct UniqueKeys {
    path: String,
}

impl<'de> DeserializeSeed<'de> for UniqueKeys {
    type Value = ();

    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for UniqueKeys {
    type Value = ();

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a JSON value")
    }
    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        let mut index = 0usize;
        while seq
            .next_element_seed(UniqueKeys {
                path: format!("{}[{index}]", self.path),
            })?
            .is_some()
        {
            index += 1;
        }
        Ok(())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let mut seen = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            let child = format!("{}[{key:?}]", self.path);
            if !seen.insert(key.clone()) {
                return Err(serde::de::Error::custom(format!(
                    "duplicate object key {key:?} at {child}"
                )));
            }
            map.next_value_seed(UniqueKeys { path: child })?;
        }
        Ok(())
    }
}

fn validate_unique_keys(input: &str) -> Result<(), serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_str(input);
    UniqueKeys { path: "$".into() }.deserialize(&mut deserializer)?;
    deserializer.end()
}

/// One record at a time: memory is proportional to the union schema plus the
/// largest record, rather than the number of rows. Validate even bounded input
/// beyond the sample so unknown fields and type changes cannot disappear.
pub(super) fn schema(
    paths: &[std::path::PathBuf],
    labels: &[std::path::PathBuf],
    limit: Option<usize>,
) -> Result<(Schema, Vec<bool>), QuiltError> {
    let mut merged = JsonType::Unknown;
    let mut empty = Vec::new();
    for (input, path) in paths.iter().zip(labels) {
        let file = File::open(input).map_err(|error| diagnostic(path, 0, error))?;
        let mut local = JsonType::Unknown;
        let mut records = 0;
        for (index, line) in BufReader::new(file).lines().enumerate() {
            let line = line.map_err(|error| diagnostic(path, index + 1, error))?;
            if line.trim().is_empty() {
                continue;
            }
            validate_unique_keys(&line).map_err(|error| diagnostic(path, index + 1, error))?;
            let value: Value =
                serde_json::from_str(&line).map_err(|error| diagnostic(path, index + 1, error))?;
            if !value.is_object() {
                return Err(diagnostic(
                    path,
                    index + 1,
                    "NDJSON records must be objects",
                ));
            }
            let incoming =
                JsonType::infer(&value).map_err(|error| diagnostic(path, index + 1, error))?;
            local
                .merge(incoming, "", limit.is_some_and(|n| records >= n))
                .map_err(|error| diagnostic(path, index + 1, error))?;
            records += 1;
        }
        merged
            .merge(local, "", false)
            .map_err(|error| diagnostic(path, 0, error))?;
        empty.push(records == 0);
    }
    merged
        .validate_empty("", false)
        .map_err(|error| diagnostic(&labels[0], 0, error))?;
    if matches!(merged, JsonType::Null | JsonType::Unknown) {
        return Ok((Schema::default(), empty));
    }
    match merged.dtype() {
        DataType::Struct(fields) => Ok((Schema::from_iter(fields), empty)),
        _ => unreachable!(),
    }
}
