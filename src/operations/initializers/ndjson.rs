use crate::error::QuiltError;
use polars::prelude::*;
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{value::RawValue, Number};
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
    fn number(number: &Number) -> Result<Self, String> {
        if let Some(value) = number.as_i64() {
            Ok(Self::Integer {
                negative: value < 0,
                wide: false,
            })
        } else if number.as_u64().is_some() {
            Ok(Self::Integer {
                negative: false,
                wide: true,
            })
        } else if number.is_f64() && number.as_f64().is_some_and(f64::is_finite) {
            Ok(Self::Float)
        } else {
            Err("number outside supported Int64/UInt64/Float64 range".into())
        }
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

// Dispatch on borrowed JSON tokens instead of Value's private number/raw-value
// map protocol. Actual object keys always remain keys, regardless of spelling.
struct SchemaVisitor {
    path: String,
    depth: usize,
}

impl SchemaVisitor {
    fn infer(self, value: &RawValue) -> Result<JsonType, serde_json::Error> {
        let input = value.get();
        let token = input.as_bytes()[0];
        // RawValue skips containers iteratively; preserve serde_json's usual
        // recursion bound for our recursive schema traversal.
        if matches!(token, b'{' | b'[') && self.depth >= 128 {
            return Err(serde::de::Error::custom(format!(
                "{}: recursion limit exceeded",
                self.path
            )));
        }
        let mut parser = serde_json::Deserializer::from_str(input);
        let dtype = match token {
            b'{' => parser.deserialize_map(self)?,
            b'[' => parser.deserialize_seq(self)?,
            b'"' => parser.deserialize_str(self)?,
            b't' | b'f' => parser.deserialize_bool(self)?,
            b'n' => parser.deserialize_unit(self)?,
            _ => {
                let number: Number = serde_json::from_str(input)?;
                return JsonType::number(&number)
                    .map_err(|error| serde::de::Error::custom(format!("{}: {error}", self.path)));
            }
        };
        parser.end()?;
        Ok(dtype)
    }
}

impl<'de> DeserializeSeed<'de> for SchemaVisitor {
    type Value = JsonType;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<JsonType, D::Error> {
        let value = <&RawValue>::deserialize(deserializer)?;
        self.infer(value).map_err(serde::de::Error::custom)
    }
}

impl<'de> Visitor<'de> for SchemaVisitor {
    type Value = JsonType;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a JSON value")
    }
    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<JsonType, E> {
        Ok(JsonType::Bool)
    }
    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<JsonType, E> {
        Ok(JsonType::String)
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<JsonType, E> {
        Ok(JsonType::Null)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<JsonType, A::Error> {
        let mut inner = JsonType::Unknown;
        let mut index = 0usize;
        loop {
            let path = format!("{}[{index}]", self.path);
            let Some(dtype) = seq.next_element_seed(SchemaVisitor {
                path: path.clone(),
                depth: self.depth + 1,
            })?
            else {
                break;
            };
            inner
                .merge(dtype, &path, false)
                .map_err(serde::de::Error::custom)?;
            index += 1;
        }
        Ok(JsonType::List(Box::new(inner)))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<JsonType, A::Error> {
        let mut seen = HashSet::new();
        let mut fields = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            let child = format!("{}[{key:?}]", self.path);
            if !seen.insert(key.clone()) {
                return Err(serde::de::Error::custom(format!(
                    "duplicate object key {key:?} at {child}"
                )));
            }
            let dtype = map.next_value_seed(SchemaVisitor {
                path: child,
                depth: self.depth + 1,
            })?;
            fields.push((key, dtype));
        }
        Ok(JsonType::Object(fields, false))
    }
}

fn infer_record(input: &str) -> Result<JsonType, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_str(input);
    let dtype = SchemaVisitor {
        path: "$".into(),
        depth: 1,
    }
    .deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(dtype)
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
            let incoming =
                infer_record(&line).map_err(|error| diagnostic(path, index + 1, error))?;
            if !matches!(incoming, JsonType::Object(..)) {
                return Err(diagnostic(
                    path,
                    index + 1,
                    "NDJSON records must be objects",
                ));
            }
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
