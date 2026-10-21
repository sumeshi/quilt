use crate::error::QuiltError;
use polars::prelude::*;
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{value::RawValue, Number};
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

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
                return Err(format!(
                    "{path}: type first appears after inference limit; use --infer-schema-length full"
                ));
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
                    if !incoming.iter().any(|(key, _)| key == name) {
                        existing.mark_nullable();
                    }
                }
                for (name, mut dtype) in incoming {
                    let child = if path.is_empty() {
                        name.clone()
                    } else {
                        format!("{path}.{name}")
                    };
                    if let Some((_, existing)) = fields.iter_mut().find(|(key, _)| key == &name) {
                        existing.merge(dtype, &child, bounded)?;
                    } else if bounded {
                        return Err(format!(
                            "{child}: field first appears after inference limit; use --infer-schema-length full"
                        ));
                    } else {
                        dtype.mark_nullable();
                        fields.push((name, dtype));
                    }
                }
            }
            (Self::List(inner), Self::List(other)) => {
                inner.merge(*other, &format!("{path}[]"), bounded)?
            }
            (
                Self::Integer { negative, wide },
                Self::Integer {
                    negative: rhs_negative,
                    wide: rhs_wide,
                },
            ) => {
                if (*negative || rhs_negative) && (*wide || rhs_wide) {
                    return Err(format!(
                        "{path}: incompatible signed/unsigned integer range; encode these IDs as strings"
                    ));
                }
                if bounded && rhs_wide && !*wide {
                    return Err(format!(
                        "{path}: integer range widens after inference limit; use --infer-schema-length full"
                    ));
                }
                *negative |= rhs_negative;
                *wide |= rhs_wide;
            }
            (left, right) if *left == right => (),
            (left, right) => {
                return Err(format!(
                    "{path}: incompatible NDJSON types: {left:?} and {right:?}; implicit coercion is disabled to preserve precision"
                ));
            }
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
                    return Err(format!(
                        "{path}: nullable empty objects are unsupported by the Polars NDJSON reader (it loses their null validity); add a typed field or normalize this field before loading"
                    ));
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

/// Validate records independently and write only accepted source lines to a
/// managed spool. Candidate schema changes are committed only after a whole
/// record merges successfully.
pub(super) fn quarantine(
    input_paths: &[PathBuf],
    sources: &[PathBuf],
    report: &super::rejected::SharedRejectedReport,
    infer_schema_length: Option<usize>,
    resources: &crate::controllers::resources::ExecutionResources,
) -> Result<(Vec<PathBuf>, Schema, Vec<bool>, usize), QuiltError> {
    if !resources.temp_files_enabled() {
        return Err(QuiltError::usage(
            "run --show-plan cannot inspect NDJSON rejects without execution",
        ));
    }
    if input_paths.len() != sources.len() {
        return Err(QuiltError::operation(
            "quarantine NDJSON",
            "input paths and source labels have different lengths",
        ));
    }
    let mut merged = JsonType::Unknown;
    let mut rejected = 0usize;
    let mut accepted_paths = Vec::with_capacity(input_paths.len());
    let mut empty_files = Vec::with_capacity(input_paths.len());
    for (input_path, source) in input_paths.iter().zip(sources) {
        let mut local = JsonType::Unknown;
        let input = File::open(input_path).map_err(|error| diagnostic(source, 0, error))?;
        let mut reader = BufReader::new(input);
        let mut accepted = resources
            .reserve_temp_file("qlt-jsonl-accepted", "jsonl")
            .map_err(|error| diagnostic(source, 0, error))?;
        let accepted_path = accepted.path().to_path_buf();
        let accepted_file = accepted
            .file_mut()
            .ok_or_else(|| diagnostic(source, 0, "accepted spool reservation unavailable"))?;
        let mut line_no = 0usize;
        let mut file_records = 0usize;
        let mut bytes = Vec::new();
        loop {
            bytes.clear();
            let count = reader
                .read_until(b'\n', &mut bytes)
                .map_err(|error| diagnostic(source, line_no + 1, error))?;
            if count == 0 {
                break;
            }
            line_no += 1;
            let raw = bytes.strip_suffix(b"\n").unwrap_or(&bytes);
            let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
            if raw.iter().all(u8::is_ascii_whitespace) {
                accepted_file
                    .write_all(&bytes)
                    .map_err(|error| diagnostic(source, line_no, error))?;
                continue;
            }
            let result = std::str::from_utf8(raw)
                .map_err(|error| format!("invalid UTF-8 at byte {}", error.valid_up_to()))
                .and_then(|text| {
                    let incoming = infer_record(text).map_err(|error| error.to_string())?;
                    if !matches!(incoming, JsonType::Object(..)) {
                        return Err("NDJSON records must be objects".into());
                    }
                    let mut local_candidate = local.clone();
                    local_candidate.merge(
                        incoming.clone(),
                        "",
                        infer_schema_length.is_some_and(|limit| file_records >= limit),
                    )?;
                    let mut merged_candidate = merged.clone();
                    merged_candidate.merge(incoming, "", false)?;
                    Ok((local_candidate, merged_candidate))
                });
            match result {
                Ok((local_candidate, merged_candidate)) => {
                    local = local_candidate;
                    merged = merged_candidate;
                    file_records += 1;
                    accepted_file
                        .write_all(&bytes)
                        .map_err(|error| diagnostic(source, line_no, error))?;
                }
                Err(reason) => {
                    report
                        .lock()
                        .map_err(|_| diagnostic(source, line_no, "reject report lock poisoned"))?
                        .write(&source.to_string_lossy(), line_no, raw, &reason)
                        .map_err(|error| diagnostic(source, line_no, error))?;
                    rejected += 1;
                }
            }
        }
        accepted.close_file();
        resources
            .retain_temp_file(accepted)
            .map_err(|error| diagnostic(source, 0, error))?;
        accepted_paths.push(accepted_path);
        empty_files.push(file_records == 0);
    }
    let source = sources
        .first()
        .map(PathBuf::as_path)
        .unwrap_or(Path::new("<input>"));
    merged
        .validate_empty("", false)
        .map_err(|error| diagnostic(source, 0, error))?;
    let schema = match merged.dtype() {
        DataType::Struct(fields) => Schema::from_iter(fields),
        _ => Schema::default(),
    };
    Ok((accepted_paths, schema, empty_files, rejected))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture_dir() -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("qlt-ndjson-reject-test-{stamp}"));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn quarantined_record_does_not_partially_change_schema() {
        let dir = fixture_dir();
        let source = dir.join("input.jsonl");
        let source_two = dir.join("input-two.jsonl");
        let report = dir.join("rejects.jsonl");
        fs::write(&source, b"{\"id\":1}\n").unwrap();
        fs::write(
            &source_two,
            b"{\"id\":\"bad\",\"later\":true}\n{\"later\":true}\n",
        )
        .unwrap();
        let resources = crate::controllers::resources::ExecutionResources::new_in(dir.clone());
        let report = std::sync::Arc::new(std::sync::Mutex::new(
            super::super::rejected::RejectedReport::new(&report).unwrap(),
        ));
        let (accepted, schema, _, rejected) = quarantine(
            &[source.clone(), source_two.clone()],
            &[source.clone(), source_two.clone()],
            &report,
            None,
            &resources,
        )
        .unwrap();
        assert_eq!(rejected, 1);
        assert_eq!(schema.len(), 2);
        let rows = fs::read_to_string(&accepted[0]).unwrap();
        assert!(rows.contains("\"id\":1"));
        let rows_two = fs::read_to_string(&accepted[1]).unwrap();
        assert!(rows_two.contains("\"later\":true"));
        assert!(!rows_two.contains("bad"));
        let report = std::sync::Arc::try_unwrap(report)
            .unwrap_or_else(|_| panic!("report still has shared owners"))
            .into_inner()
            .unwrap_or_else(|_| panic!("report lock poisoned"));
        report.publish().unwrap();
        let report_text = fs::read_to_string(dir.join("rejects.jsonl")).unwrap();
        assert!(report_text.contains("\"line\":1"));
        assert!(report_text.contains("input-two.jsonl"));
        assert!(report_text.contains("incompatible NDJSON types"));

        let bounded_one = dir.join("bounded-one.jsonl");
        let bounded_two = dir.join("bounded-two.jsonl");
        let bounded_report_path = dir.join("bounded-rejects.jsonl");
        fs::write(&bounded_one, b"{\"first\":1}\n").unwrap();
        fs::write(&bounded_two, b"{\"second\":2}\n{\"late\":3}\n").unwrap();
        let bounded_report = std::sync::Arc::new(std::sync::Mutex::new(
            super::super::rejected::RejectedReport::new(&bounded_report_path).unwrap(),
        ));
        let (bounded_accepted, bounded_schema, _, bounded_rejected) = quarantine(
            &[bounded_one.clone(), bounded_two.clone()],
            &[bounded_one, bounded_two.clone()],
            &bounded_report,
            Some(1),
            &resources,
        )
        .unwrap();
        assert_eq!(bounded_rejected, 1);
        assert_eq!(bounded_schema.len(), 2);
        let second_file = fs::read_to_string(&bounded_accepted[1]).unwrap();
        assert!(second_file.contains("\"second\":2"));
        assert!(!second_file.contains("late"));
        std::sync::Arc::try_unwrap(bounded_report)
            .unwrap_or_else(|_| panic!("bounded report still has shared owners"))
            .into_inner()
            .unwrap_or_else(|_| panic!("bounded report lock poisoned"))
            .publish()
            .unwrap();
        let bounded_report_text = fs::read_to_string(bounded_report_path).unwrap();
        assert!(bounded_report_text.contains("\"line\":2"));
        assert!(bounded_report_text.contains("bounded-two.jsonl"));

        let local_one = dir.join("local-one.jsonl");
        let local_two = dir.join("local-two.jsonl");
        let local_report_path = dir.join("local-rejects.jsonl");
        fs::write(&local_one, b"{\"a\":1}\n").unwrap();
        fs::write(&local_two, b"{\"b\":1}\n{\"a\":2}\n").unwrap();
        let local_report = std::sync::Arc::new(std::sync::Mutex::new(
            super::super::rejected::RejectedReport::new(&local_report_path).unwrap(),
        ));
        let (local_accepted, local_schema, _, local_rejected) = quarantine(
            &[local_one.clone(), local_two.clone()],
            &[local_one, local_two.clone()],
            &local_report,
            Some(1),
            &resources,
        )
        .unwrap();
        assert_eq!(local_rejected, 1);
        assert_eq!(local_schema.len(), 2);
        let second_file = fs::read_to_string(&local_accepted[1]).unwrap();
        assert!(second_file.contains("\"b\":1"));
        assert!(!second_file.contains("\"a\":2"));
        std::sync::Arc::try_unwrap(local_report)
            .unwrap_or_else(|_| panic!("local report still has shared owners"))
            .into_inner()
            .unwrap_or_else(|_| panic!("local report lock poisoned"))
            .publish()
            .unwrap();
        let local_report_text = fs::read_to_string(local_report_path).unwrap();
        assert!(local_report_text.contains("\"line\":2"));
        assert!(local_report_text.contains("local-two.jsonl"));
        assert!(local_report_text.contains("first appears after inference limit"));
        drop(resources);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn gzip_input_is_rejected_before_plan_inspection_reads_it() {
        let resources = crate::controllers::resources::ExecutionResources::new_plan();
        let error = prepare_path(Path::new("input.jsonl.gz"), &resources).unwrap_err();
        assert!(error.to_string().contains("cannot inspect gzipped NDJSON"));
    }
}
