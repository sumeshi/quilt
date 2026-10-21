//! Strict CSV schema discovery for lossless Polars scans.
use crate::controllers::resources::ExecutionResources;
use crate::error::QuiltError;
use crate::operations::initializers::rejected::SharedRejectedReport;
use polars::prelude::*;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug, Default)]
pub struct CsvInputOptions {
    pub all_strings: bool,
    pub schema: Vec<(String, String)>,
    pub infer_schema_length: Option<usize>,
    pub rejects: Option<PathBuf>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Null,
    Bool,
    Int,
    UInt,
    Float,
    String,
}
#[derive(Debug)]
struct Field {
    value: String,
    quoted: bool,
    valid_utf8: bool,
}
#[derive(Debug)]
struct CsvRecordError {
    message: String,
    line: u64,
}
#[derive(Debug)]
struct CsvRecord {
    fields: Vec<Field>,
    line: u64,
    raw: Vec<u8>,
}

#[derive(Clone, Copy)]
pub(crate) struct CsvScanSettings {
    pub separator: u8,
    pub has_header: bool,
    pub eol_char: u8,
    pub low_memory: bool,
    pub chunk_size: usize,
}
fn make_field(bytes: Vec<u8>, quoted: bool) -> Field {
    let valid_utf8 = std::str::from_utf8(&bytes).is_ok();
    Field {
        value: String::from_utf8_lossy(&bytes).into_owned(),
        quoted,
        valid_utf8,
    }
}
fn note_eol(eol_char: &mut Option<u8>, found: u8, line: u64) -> Result<(), CsvRecordError> {
    if eol_char.is_some_and(|previous| previous != found) {
        return Err(CsvRecordError {
            message: "mixed CSV record line endings are unsupported".into(),
            line,
        });
    }
    *eol_char = Some(found);
    Ok(())
}
fn advice() -> &'static str {
    "; use --all-strings or provide a column schema override to control CSV types"
}
fn classify(s: &str) -> Kind {
    if s.is_empty() {
        return Kind::Null;
    }
    if matches!(s, "true" | "false") {
        return Kind::Bool;
    }
    if s.starts_with('+') {
        return Kind::String;
    }
    let integer_part = s.split(['.', 'e', 'E']).next().unwrap_or(s);
    let digits = integer_part.strip_prefix('-').unwrap_or(integer_part);
    let integer_lexeme = s
        .strip_prefix('-')
        .unwrap_or(s)
        .bytes()
        .all(|b| b.is_ascii_digit());
    if digits.bytes().all(|b| b.is_ascii_digit()) && digits.len() > 1 && digits.starts_with('0') {
        return Kind::String;
    }
    if s.parse::<i64>().is_ok() {
        Kind::Int
    } else if !s.starts_with('-') && s.parse::<u64>().is_ok() {
        Kind::UInt
    } else if integer_lexeme {
        // An integer outside UInt64 is text; treating it as float would round it.
        Kind::String
    } else if s.parse::<f64>().is_ok_and(f64::is_finite) {
        Kind::Float
    } else {
        Kind::String
    }
}
fn exact_float(s: &str) -> bool {
    if let Ok(v) = s.parse::<i128>() {
        return (v as f64) as i128 == v;
    }
    if let Ok(v) = s.parse::<u128>() {
        return (v as f64) as u128 == v;
    }
    true
}
fn merge(a: Kind, b: Kind, had_inexact_integer: bool, had_negative: bool, value: &str) -> Kind {
    use Kind::*;
    match (a, b) {
        (Null, k) | (k, Null) => k,
        (String, _) | (_, String) => String,
        (x, y) if x == y => x,
        (Int, Float) | (Float, Int) | (UInt, Float) | (Float, UInt) => {
            if had_inexact_integer || !exact_float(value) {
                String
            } else {
                Float
            }
        }
        (Int, UInt) | (UInt, Int) => {
            if had_negative || value.starts_with('-') {
                String
            } else {
                UInt
            }
        }
        _ => String,
    }
}
fn next_record<R: BufRead>(
    r: &mut R,
    sep: u8,
    line: &mut u64,
    eol_char: &mut Option<u8>,
) -> Result<Option<CsvRecord>, CsvRecordError> {
    let mut fields = Vec::new();
    let mut value = Vec::new();
    let mut quoted = false;
    let mut inside = false;
    let mut start = true;
    let mut after_quote = false;
    let mut any = false;
    let mut start_line = *line;
    let mut raw = Vec::new();
    loop {
        let byte = r
            .fill_buf()
            .map_err(|e| CsvRecordError {
                message: e.to_string(),
                line: *line,
            })?
            .first()
            .copied();
        let Some(b) = byte else {
            if inside {
                return Err(CsvRecordError {
                    message: format!("unterminated quoted field near line {}", *line),
                    line: start_line,
                });
            }
            if !any && fields.is_empty() && value.is_empty() {
                return Ok(None);
            }
            fields.push(make_field(value, quoted));
            return Ok(Some(CsvRecord {
                fields,
                line: start_line,
                raw,
            }));
        };
        r.consume(1);
        any = true;
        raw.push(b);
        if after_quote {
            after_quote = false;
            if b == sep {
                fields.push(make_field(std::mem::take(&mut value), quoted));
                quoted = false;
                start = true;
                continue;
            }
            if b == b'\n' || b == b'\r' {
                raw.pop();
                let mut terminator = b;
                if b == b'\r'
                    && r.fill_buf()
                        .map_err(|e| CsvRecordError {
                            message: e.to_string(),
                            line: *line,
                        })?
                        .first()
                        == Some(&b'\n')
                {
                    r.consume(1);
                    terminator = b'\n';
                }
                note_eol(eol_char, terminator, start_line)?;
                *line += 1;
                fields.push(make_field(value, quoted));
                return Ok(Some(CsvRecord {
                    fields,
                    line: start_line,
                    raw,
                }));
            }
            return Err(CsvRecordError {
                message: format!("unexpected byte after closing quote near line {}", *line),
                line: start_line,
            });
        }
        if inside {
            if b == b'"' {
                if r.fill_buf()
                    .map_err(|e| CsvRecordError {
                        message: e.to_string(),
                        line: *line,
                    })?
                    .first()
                    == Some(&b'"')
                {
                    r.consume(1);
                    raw.push(b'"');
                    value.push(b'"');
                    continue;
                }
                inside = false;
                after_quote = true;
            } else {
                if b == b'\n' || b == b'\r' {
                    *line += 1;
                    value.push(b);
                    if b == b'\r'
                        && r.fill_buf()
                            .map_err(|e| CsvRecordError {
                                message: e.to_string(),
                                line: *line,
                            })?
                            .first()
                            == Some(&b'\n')
                    {
                        r.consume(1);
                        raw.push(b'\n');
                        value.push(b'\n');
                    }
                    continue;
                }
                value.push(b);
            }
            continue;
        }
        if start && b == b'"' {
            quoted = true;
            inside = true;
            start = false;
            continue;
        }
        if b == sep {
            fields.push(make_field(std::mem::take(&mut value), quoted));
            quoted = false;
            start = true;
            continue;
        }
        if b == b'\n' || b == b'\r' {
            let mut terminator = b;
            if b == b'\r'
                && r.fill_buf()
                    .map_err(|e| CsvRecordError {
                        message: e.to_string(),
                        line: *line,
                    })?
                    .first()
                    == Some(&b'\n')
            {
                r.consume(1);
                terminator = b'\n';
            }
            note_eol(eol_char, terminator, start_line)?;
            if fields.is_empty() && value.is_empty() && !quoted {
                *line += 1;
                raw.clear();
                any = false;
                start_line = *line;
                continue;
            }
            raw.pop();
            *line += 1;
            fields.push(make_field(value, quoted));
            return Ok(Some(CsvRecord {
                fields,
                line: start_line,
                raw,
            }));
        }
        if b == b'"' {
            return Err(CsvRecordError {
                message: format!("quote inside unquoted field near line {}", *line),
                line: start_line,
            });
        }
        value.push(b);
        start = false;
    }
}
fn kind_type(k: Kind) -> DataType {
    match k {
        Kind::Null | Kind::String => DataType::String,
        Kind::Bool => DataType::Boolean,
        Kind::Int => DataType::Int64,
        Kind::UInt => DataType::UInt64,
        Kind::Float => DataType::Float64,
    }
}
fn parse_kind(name: &str, s: &str) -> Result<Kind, QuiltError> {
    match s.to_ascii_lowercase().as_str() {
        "string" | "str" => Ok(Kind::String),
        "int" | "i64" | "integer" => Ok(Kind::Int),
        "uint" | "u64" => Ok(Kind::UInt),
        "float" | "f64" | "double" => Ok(Kind::Float),
        "bool" | "boolean" => Ok(Kind::Bool),
        _ => Err(QuiltError::usage(format!(
            "unsupported CSV type {s:?} for column {name:?}; use string, int, uint, float, or bool"
        ))),
    }
}
fn compatible(k: Kind, s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    match k {
        Kind::String => true,
        Kind::Bool => matches!(s, "true" | "false"),
        Kind::Int => s.parse::<i64>().is_ok(),
        Kind::UInt => s.parse::<u64>().is_ok(),
        Kind::Float => s.parse::<f64>().is_ok_and(f64::is_finite) && exact_float(s),
        Kind::Null => false,
    }
}
fn compatible_field(k: Kind, field: &Field) -> bool {
    if field.value.is_empty() && field.quoted {
        return k == Kind::String;
    }
    compatible(k, &field.value)
}
fn write_field(w: &mut Vec<u8>, s: &str, sep: u8) {
    w.push(b'"');
    for b in s.bytes() {
        if b == b'"' {
            w.push(b'"')
        }
        w.push(b)
    }
    w.push(b'"');
    w.push(sep)
}
fn write_record(w: &mut Vec<u8>, record: &[Field], sep: u8) {
    for field in record {
        if field.value.is_empty() && !field.quoted {
            w.push(sep);
        } else {
            write_field(w, &field.value, sep);
        }
    }
    if !w.is_empty() {
        w.pop();
    }
    w.push(b'\n');
}

/// Validate records in a streaming pass, then scan with the explicit schema.
/// A filtered managed spool is created only when rejects are enabled.
pub(crate) fn read_csv_lazy(
    path: &Path,
    source: &Path,
    settings: &CsvScanSettings,
    opts: &CsvInputOptions,
    resources: &ExecutionResources,
    report: Option<SharedRejectedReport>,
) -> Result<LazyFrame, QuiltError> {
    let sep = settings.separator;
    let has_header = settings.has_header;
    let mut file = File::open(path).map_err(|e| QuiltError::Io {
        operation: "open CSV".into(),
        path: Some(source.display().to_string()),
        message: e.to_string(),
    })?;
    let mut prefix = [0u8; 4];
    let mut prefix_len = 0usize;
    while prefix_len < prefix.len() {
        let n = file
            .read(&mut prefix[prefix_len..])
            .map_err(|e| QuiltError::Io {
                operation: "read CSV".into(),
                path: Some(source.display().to_string()),
                message: e.to_string(),
            })?;
        if n == 0 {
            break;
        }
        prefix_len += n;
    }
    if prefix[..prefix_len].starts_with(&[0xff, 0xfe])
        || prefix[..prefix_len].starts_with(&[0xfe, 0xff])
        || prefix[..prefix_len].starts_with(&[0, 0, 0xfe, 0xff])
        || prefix[..prefix_len].starts_with(&[0xff, 0xfe, 0, 0])
    {
        return Err(QuiltError::Io {
            operation: "read CSV".into(),
            path: Some(source.display().to_string()),
            message: "line 1: UTF-16/UTF-32 CSV is unsupported (expected UTF-8)".into(),
        });
    }
    let skip = if prefix[..prefix_len].starts_with(&[0xef, 0xbb, 0xbf]) {
        3
    } else {
        0
    };
    let chained = std::io::Cursor::new(prefix[skip..prefix_len].to_vec()).chain(file);
    let mut r = BufReader::new(chained);
    let mut line = 1u64;
    let mut eol_char = None;
    let mut names: Option<Vec<String>> = None;
    let mut kinds = Vec::new();
    let mut unsafe_integer = Vec::new();
    let mut had_negative = Vec::new();
    let mut count = 0usize;
    let overrides: HashMap<String, Kind> = opts
        .schema
        .iter()
        .map(|(n, k)| Ok((n.clone(), parse_kind(n, k)?)))
        .collect::<Result<_, QuiltError>>()?;
    if overrides.len() != opts.schema.len() {
        return Err(QuiltError::usage("duplicate CSV schema override column"));
    }
    let mut reservation = if report.is_some() {
        Some(
            resources
                .reserve_temp_file("qlt-csv-filtered", "csv")
                .map_err(|e| QuiltError::Io {
                    operation: "create CSV spool".into(),
                    path: Some(source.display().to_string()),
                    message: e.to_string(),
                })?,
        )
    } else {
        None
    };
    let spool_path = reservation.as_ref().map(|r| r.path().to_path_buf());
    loop {
        let CsvRecord {
            fields: record,
            line: here,
            raw,
        } = match next_record(&mut r, sep, &mut line, &mut eol_char) {
            Ok(Some(record)) => record,
            Ok(None) => break,
            Err(parse_error) => {
                return Err(QuiltError::Io {
                    operation: "parse CSV".into(),
                    path: Some(source.display().to_string()),
                    message: format!(
                        "line {}: {}{}",
                        parse_error.line,
                        parse_error.message,
                        if report.is_some() {
                            "; malformed quoting cannot be quarantined safely"
                        } else {
                            ""
                        }
                    ),
                });
            }
        };
        if names.is_none() && has_header {
            if record.iter().any(|field| !field.valid_utf8) {
                return Err(QuiltError::Io {
                    operation: "read CSV header".into(),
                    path: Some(source.display().to_string()),
                    message: format!("line {here}: invalid UTF-8 in column name"),
                });
            }
            let ns: Vec<String> = record.into_iter().map(|f| f.value).collect();
            if ns.iter().any(|s| s.is_empty()) {
                return Err(QuiltError::Io {
                    operation: "read CSV header".into(),
                    path: Some(source.display().to_string()),
                    message: format!("line {here}: empty column names are unsupported"),
                });
            }
            let mut seen = HashSet::new();
            if let Some(d) = ns.iter().find(|n| !seen.insert((*n).clone())) {
                return Err(QuiltError::Io {
                    operation: "read CSV header".into(),
                    path: Some(source.display().to_string()),
                    message: format!("line {here}: duplicate column name {d:?}"),
                });
            }
            names = Some(ns);
            kinds = vec![Kind::Null; names.as_ref().unwrap().len()];
            unsafe_integer = vec![false; kinds.len()];
            had_negative = vec![false; kinds.len()];
            let mut b = Vec::new();
            for n in names.as_ref().unwrap() {
                write_field(&mut b, n, sep)
            }
            if !b.is_empty() {
                b.pop();
            }
            b.push(b'\n');
            if let Some(reservation) = reservation.as_mut() {
                reservation
                    .file_mut()
                    .unwrap()
                    .write_all(&b)
                    .map_err(|e| QuiltError::Io {
                        operation: "write CSV spool".into(),
                        path: spool_path.as_ref().map(|p| p.display().to_string()),
                        message: e.to_string(),
                    })?;
            }
            continue;
        }
        if names.is_none() {
            names = Some(
                (0..record.len())
                    .map(|i| format!("column_{}", i + 1))
                    .collect(),
            );
            kinds = vec![Kind::Null; record.len()];
            unsafe_integer = vec![false; record.len()];
            had_negative = vec![false; record.len()];
        }
        let ns = names.as_ref().unwrap();
        if let Some((i, _)) = record.iter().enumerate().find(|(_, f)| !f.valid_utf8) {
            let reason = format!(
                "column {:?}: invalid UTF-8",
                ns.get(i).map(String::as_str).unwrap_or("<extra>")
            );
            if let Some(report) = &report {
                report
                    .lock()
                    .map_err(|_| QuiltError::Operation {
                        operation: "write rejects report".into(),
                        message: "report lock is poisoned".into(),
                    })?
                    .write(&source.display().to_string(), here as usize, &raw, &reason)?;
                continue;
            }
            return Err(QuiltError::Io {
                operation: "validate CSV".into(),
                path: Some(source.display().to_string()),
                message: format!("line {here}: {reason}{}", advice()),
            });
        }
        if record.len() != ns.len() {
            if let Some(report) = &report {
                let reason = format!("expected {} fields, found {}", ns.len(), record.len());
                report
                    .lock()
                    .map_err(|_| QuiltError::Operation {
                        operation: "write rejects report".into(),
                        message: "report lock is poisoned".into(),
                    })?
                    .write(&source.display().to_string(), here as usize, &raw, &reason)?;
                continue;
            }
            return Err(QuiltError::Io {
                operation: "validate CSV".into(),
                path: Some(source.display().to_string()),
                message: format!(
                    "line {here}: expected {} fields, found {}",
                    ns.len(),
                    record.len()
                ),
            });
        }
        let old_kinds = kinds.clone();
        let old_unsafe_integer = unsafe_integer.clone();
        let old_had_negative = had_negative.clone();
        let mut rejected: Option<(usize, String)> = None;
        for (i, f) in record.iter().enumerate() {
            let target = overrides.get(&ns[i]).copied();
            let observed = if opts.all_strings || (f.value.is_empty() && f.quoted) {
                Kind::String
            } else {
                classify(&f.value)
            };
            if let Some(k) = target {
                if !compatible_field(k, f) {
                    rejected = Some((
                        i,
                        format!(
                            "column {:?}: value {:?} is incompatible with {k:?}",
                            ns[i], f.value
                        ),
                    ));
                    break;
                }
                kinds[i] = k
            } else if opts.all_strings {
                kinds[i] = Kind::String
            } else {
                if opts.infer_schema_length.is_some_and(|limit| count >= limit) {
                    if kinds[i] == Kind::Null {
                        // A column that was entirely null in the sample has no
                        // useful inferred type. Freeze it as text so later rows
                        // cannot silently determine its dtype.
                        kinds[i] = Kind::String;
                    } else if !compatible_field(kinds[i], f) {
                        rejected = Some((
                            i,
                            format!(
                                "column {:?}: value {:?} is incompatible with inferred {:?}",
                                ns[i], f.value, kinds[i]
                            ),
                        ));
                        break;
                    }
                }
                if matches!(observed, Kind::Int | Kind::UInt) && !exact_float(&f.value) {
                    unsafe_integer[i] = true
                }
                if f.value.starts_with('-') && observed == Kind::Int {
                    had_negative[i] = true;
                }
                kinds[i] = merge(
                    kinds[i],
                    observed,
                    unsafe_integer[i],
                    had_negative[i],
                    &f.value,
                )
            }
        }
        if let Some((_, reason)) = rejected {
            kinds = old_kinds;
            unsafe_integer = old_unsafe_integer;
            had_negative = old_had_negative;
            if let Some(report) = &report {
                report
                    .lock()
                    .map_err(|_| QuiltError::Operation {
                        operation: "write rejects report".into(),
                        message: "report lock is poisoned".into(),
                    })?
                    .write(&source.display().to_string(), here as usize, &raw, &reason)?;
                continue;
            }
            return Err(QuiltError::Io {
                operation: "validate CSV".into(),
                path: Some(source.display().to_string()),
                message: format!("line {here}: {reason}{}", advice()),
            });
        }
        if let Some(reservation) = reservation.as_mut() {
            let mut row = Vec::new();
            write_record(&mut row, &record, sep);
            reservation
                .file_mut()
                .unwrap()
                .write_all(&row)
                .map_err(|e| QuiltError::Io {
                    operation: "write CSV spool".into(),
                    path: spool_path.as_ref().map(|p| p.display().to_string()),
                    message: e.to_string(),
                })?;
        }
        count += 1;
    }
    let ns = names.ok_or_else(|| QuiltError::Io {
        operation: "read CSV".into(),
        path: Some(source.display().to_string()),
        message: "input contains no records".into(),
    })?;
    for name in overrides.keys() {
        if !ns.contains(name) {
            return Err(QuiltError::usage(format!(
                "CSV schema override refers to unknown column {name:?}"
            )));
        }
    }
    if let Some(reservation) = reservation.as_mut() {
        reservation
            .file_mut()
            .unwrap()
            .flush()
            .map_err(|e| QuiltError::Io {
                operation: "flush CSV spool".into(),
                path: spool_path.as_ref().map(|p| p.display().to_string()),
                message: e.to_string(),
            })?;
    }
    let schema = Schema::from_iter(ns.iter().enumerate().map(|(i, n)| {
        (
            n.as_str().into(),
            kind_type(overrides.get(n).copied().unwrap_or(kinds[i])),
        )
    }));
    let scan_path = spool_path.as_deref().unwrap_or(path);
    let lf = LazyCsvReader::new(PlRefPath::try_from_path(scan_path)?)
        .with_glob(false)
        .with_has_header(has_header)
        .with_separator(sep)
        .with_eol_char(if reservation.is_some() {
            b'\n'
        } else {
            eol_char.unwrap_or(settings.eol_char)
        })
        .with_low_memory(settings.low_memory)
        .with_chunk_size(settings.chunk_size)
        .with_raise_if_empty(reservation.is_none())
        .with_schema(Some(Arc::new(schema)))
        .finish()
        .map_err(|e| QuiltError::Io {
            operation: "scan validated CSV".into(),
            path: Some(source.display().to_string()),
            message: e.to_string(),
        })?;
    if let Some(reservation) = reservation {
        resources
            .retain_temp_file(reservation)
            .map_err(|e| QuiltError::Io {
                operation: "retain CSV spool".into(),
                path: spool_path.map(|p| p.display().to_string()),
                message: e.to_string(),
            })?;
    }
    Ok(lf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn source(
        bytes: &[u8],
    ) -> (
        ExecutionResources,
        crate::controllers::resources::TempFileReservation,
    ) {
        let resources = ExecutionResources::new();
        let mut file = resources
            .reserve_temp_file("qlt-csv-source-test", "csv")
            .unwrap();
        file.file_mut().unwrap().write_all(bytes).unwrap();
        (resources, file)
    }

    fn scan_settings() -> CsvScanSettings {
        CsvScanSettings {
            separator: b',',
            has_header: true,
            eol_char: b'\n',
            low_memory: false,
            chunk_size: 8192,
        }
    }

    #[test]
    fn scan_preserves_ids_empty_text_and_mixed_large_integer_values() {
        let (resources, file) =
            source(b"id,decimal,empty,mixed\n00017,0.5,1,9007199254740993\n00018,-0.5,\"\",1.5\n");
        let frame = read_csv_lazy(
            file.path(),
            file.path(),
            &scan_settings(),
            &CsvInputOptions::default(),
            &resources,
            None,
        )
        .unwrap();
        let df = frame.collect().unwrap();
        assert_eq!(
            df.column("id").unwrap().str().unwrap().get(0),
            Some("00017")
        );
        assert_eq!(
            df.column("decimal").unwrap().f64().unwrap().get(0),
            Some(0.5)
        );
        assert_eq!(
            df.column("decimal").unwrap().f64().unwrap().get(1),
            Some(-0.5)
        );
        assert_eq!(df.column("empty").unwrap().str().unwrap().get(1), Some(""));
        assert_eq!(
            df.column("mixed").unwrap().str().unwrap().get(0),
            Some("9007199254740993")
        );
        assert_eq!(
            df.column("mixed").unwrap().str().unwrap().get(1),
            Some("1.5")
        );
    }

    #[test]
    fn scan_keeps_unquoted_null_distinct_from_quoted_empty_and_supports_u64() {
        let (resources, file) = source(b"value,n\n,18446744073709551615\n\"\",1\n");
        let frame = read_csv_lazy(
            file.path(),
            file.path(),
            &scan_settings(),
            &CsvInputOptions::default(),
            &resources,
            None,
        )
        .unwrap();
        let df = frame.collect().unwrap();
        assert_eq!(df.column("value").unwrap().is_null().get(0), Some(true));
        assert_eq!(df.column("value").unwrap().str().unwrap().get(1), Some(""));
        assert_eq!(
            df.column("n").unwrap().u64().unwrap().get(0),
            Some(u64::MAX)
        );

        let (resources, file) = source(b"n\n18446744073709551615\n");
        let all_strings = CsvInputOptions {
            all_strings: true,
            ..Default::default()
        };
        let frame = read_csv_lazy(
            file.path(),
            file.path(),
            &scan_settings(),
            &all_strings,
            &resources,
            None,
        )
        .unwrap();
        let df = frame.collect().unwrap();
        assert_eq!(
            df.column("n").unwrap().str().unwrap().get(0),
            Some("18446744073709551615")
        );

        let (resources, file) = source(b"n\n18446744073709551615\n");
        let explicit_uint = CsvInputOptions {
            schema: vec![("n".into(), "uint".into())],
            ..Default::default()
        };
        let frame = read_csv_lazy(
            file.path(),
            file.path(),
            &scan_settings(),
            &explicit_uint,
            &resources,
            None,
        )
        .unwrap();
        let df = frame.collect().unwrap();
        assert_eq!(
            df.column("n").unwrap().u64().unwrap().get(0),
            Some(u64::MAX)
        );
    }

    #[test]
    fn invalid_headers_and_unicode_boms_fail_before_scan_with_source_line() {
        let cases: &[(&[u8], &str)] = &[
            (b"name\xff\nvalue\n", "invalid UTF-8 in column name"),
            (&[0xff, 0xfe, b'n', 0, b'\n', 0], "UTF-16/UTF-32"),
            (&[0xfe, 0xff, 0, b'n', 0, b'\n'], "UTF-16/UTF-32"),
            (&[0xff, 0xfe, 0, 0, b'n', 0, 0, 0], "UTF-16/UTF-32"),
            (&[0, 0, 0xfe, 0xff, 0, 0, 0, b'n'], "UTF-16/UTF-32"),
        ];

        for (bytes, expected_message) in cases {
            let (resources, file) = source(bytes);
            let error = read_csv_lazy(
                file.path(),
                file.path(),
                &scan_settings(),
                &CsvInputOptions::default(),
                &resources,
                None,
            )
            .err()
            .unwrap();
            let rendered = error.to_string();
            assert!(rendered.contains("line 1"), "{rendered}");
            assert!(rendered.contains(expected_message), "{rendered}");
            assert!(
                rendered.contains(&file.path().display().to_string()),
                "{rendered}"
            );
        }
    }

    #[test]
    fn quoted_empty_is_rejected_by_explicit_numeric_schema() {
        let (resources, file) = source(b"value\n\"\"\n");
        let options = CsvInputOptions {
            schema: vec![("value".into(), "int".into())],
            ..Default::default()
        };
        let rendered = format!(
            "{:?}",
            read_csv_lazy(
                file.path(),
                file.path(),
                &scan_settings(),
                &options,
                &resources,
                None,
            )
            .err()
            .unwrap()
        );
        assert!(rendered.contains("incompatible with Int"), "{rendered}");
        assert!(rendered.contains("line 2"), "{rendered}");
    }

    #[test]
    fn scanner_tracks_physical_lines_inside_quoted_records() {
        let mut input = std::io::Cursor::new(b"a,b\n1,\"two\nlines\"\n".to_vec());
        let mut line = 1;
        let mut eol = None;
        assert_eq!(
            next_record(&mut input, b',', &mut line, &mut eol)
                .unwrap()
                .unwrap()
                .fields
                .len(),
            2
        );
        let row = next_record(&mut input, b',', &mut line, &mut eol)
            .unwrap()
            .unwrap()
            .fields;
        assert_eq!(row[1].value, "two\nlines");
        assert_eq!(line, 4);
    }

    #[test]
    fn scanner_handles_blank_lines_cr_only_and_quoted_cr_records() {
        let mut input = std::io::Cursor::new(b"\r1,\"two\rline\"\r2,end\r".to_vec());
        let mut line = 1;
        let mut eol = None;
        let first = next_record(&mut input, b',', &mut line, &mut eol)
            .unwrap()
            .unwrap();
        assert_eq!(first.line, 2);
        assert_eq!(first.fields[1].value, "two\rline");
        assert_eq!(line, 4);
        let second = next_record(&mut input, b',', &mut line, &mut eol)
            .unwrap()
            .unwrap();
        assert_eq!(second.line, 4);
        assert_eq!(second.fields[1].value, "end");
        assert_eq!(line, 5);

        let (resources, file) = source(b"value\r1\r2\r");
        let frame = read_csv_lazy(
            file.path(),
            file.path(),
            &scan_settings(),
            &CsvInputOptions::default(),
            &resources,
            None,
        )
        .unwrap();
        let df = frame.collect().unwrap();
        assert_eq!(df.height(), 2);
        assert_eq!(df.column("value").unwrap().i64().unwrap().get(1), Some(2));

        let mut mixed = std::io::Cursor::new(b"a\n1\r".to_vec());
        let mut line = 1;
        let mut eol = None;
        next_record(&mut mixed, b',', &mut line, &mut eol)
            .unwrap()
            .unwrap();
        assert!(next_record(&mut mixed, b',', &mut line, &mut eol).is_err());
    }

    #[test]
    fn bounded_late_type_error_reports_source_physical_line_and_advice() {
        let (resources, file) = source(b"value,other\n,x\n8,y\n");
        let options = CsvInputOptions {
            infer_schema_length: Some(1),
            ..Default::default()
        };
        let frame = read_csv_lazy(
            file.path(),
            file.path(),
            &scan_settings(),
            &options,
            &resources,
            None,
        )
        .unwrap();
        let df = frame.collect().unwrap();
        assert_eq!(df.column("value").unwrap().str().unwrap().get(1), Some("8"));

        let (resources, file) = source(b"value\n1\nword\n");
        let options = CsvInputOptions {
            infer_schema_length: Some(1),
            ..Default::default()
        };
        let err = read_csv_lazy(
            file.path(),
            file.path(),
            &scan_settings(),
            &options,
            &resources,
            None,
        )
        .err()
        .unwrap();
        let rendered = err.to_string();
        assert!(rendered.contains("line 3"), "{rendered}");
        assert!(rendered.contains("all-strings"), "{rendered}");
        assert!(
            rendered.contains(&file.path().display().to_string()),
            "{rendered}"
        );
    }

    #[test]
    fn rejects_report_keeps_exact_malformed_row_and_excludes_it_from_scan() {
        let (resources, file) =
            source(b"value,n\n\"keep\",1\n\"bad\",2,extra\n\xff,4\n\"later\",3\n");
        let report_path = std::env::temp_dir().join(format!(
            "qlt-csv-rejects-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let options = CsvInputOptions {
            rejects: Some(report_path.clone()),
            ..Default::default()
        };
        let frame = crate::operations::initializers::load::load_with_options(
            &[file.path().to_path_buf()],
            ",",
            false,
            false,
            None,
            None,
            None,
            crate::operations::initializers::load::LoadOptions {
                csv_options: Some(&options),
                ..Default::default()
            },
            &resources,
        )
        .unwrap();
        let df = frame.collect().unwrap();
        assert_eq!(df.height(), 2);
        let report = std::fs::read_to_string(&report_path).unwrap();
        let reports: Vec<serde_json::Value> = report
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0]["line"], 3);
        assert_eq!(reports[0]["raw_text"], "\"bad\",2,extra");
        assert_eq!(reports[1]["line"], 4);
        assert_eq!(reports[1]["raw_hex"], "ff2c34");
        std::fs::remove_file(report_path).unwrap();
    }

    #[test]
    fn malformed_quoted_record_fails_with_source_and_start_line_even_with_rejects() {
        let (resources, file) = source(b"value\nok\n\"unterminated\nlast row\n");
        let report_path = std::env::temp_dir().join(format!(
            "qlt-csv-quote-rejects-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let options = CsvInputOptions {
            rejects: Some(report_path.clone()),
            ..Default::default()
        };
        let result = crate::operations::initializers::load::load_with_options(
            &[file.path().to_path_buf()],
            ",",
            false,
            false,
            None,
            None,
            None,
            crate::operations::initializers::load::LoadOptions {
                csv_options: Some(&options),
                ..Default::default()
            },
            &resources,
        );
        let rendered = result.err().unwrap().to_string();
        assert!(rendered.contains("line 3"), "{rendered}");
        assert!(
            rendered.contains("malformed quoting cannot be quarantined safely"),
            "{rendered}"
        );
        assert!(
            rendered.contains(&file.path().display().to_string()),
            "{rendered}"
        );
        assert!(!report_path.exists());
    }
}
