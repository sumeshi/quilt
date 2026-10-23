use crate::controllers::resources::ExecutionResources;
use crate::error::QuiltError;
use crate::operations::finalizers::{
    publish_directory_and_sync, FinalizerResult, NativePublicationBackend,
};
use polars::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const NULL_PARTITION: &str = "_null";
const MAX_PARTITION_NAME_BYTES: usize = 120;
const PARTITION_BATCH_ROWS: usize = 4096;
const MAX_OPEN_PARTITION_WRITERS: usize = 16;
static PARTITION_STAGE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
struct PartitionMetrics {
    batch_count: usize,
    max_batch_rows: usize,
    max_open_writers: usize,
}

#[derive(Default)]
struct PartitionRunHooks<'a> {
    metrics: Option<&'a mut PartitionMetrics>,
    before_write: Option<&'a mut dyn FnMut(usize) -> Result<(), QuiltError>>,
}

pub fn partition(
    df: &LazyFrame,
    colname: &str,
    output_dir: &str,
) -> Result<FinalizerResult, QuiltError> {
    let resources = ExecutionResources::new();
    partition_with_resources(df, colname, output_dir, &resources)
}

pub(crate) fn partition_with_resources(
    df: &LazyFrame,
    colname: &str,
    output_dir: &str,
    resources: &ExecutionResources,
) -> Result<FinalizerResult, QuiltError> {
    partition_with_publisher(
        df,
        colname,
        output_dir,
        |stage, target| {
            let parent = target
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            publish_directory_and_sync(
                &NativePublicationBackend,
                stage,
                target,
                parent,
                "publish partition directory",
            )
        },
        resources,
    )
}

fn partition_with_publisher<P>(
    df: &LazyFrame,
    colname: &str,
    output_dir: &str,
    publish: P,
    resources: &ExecutionResources,
) -> Result<FinalizerResult, QuiltError>
where
    P: Fn(&Path, &Path) -> Result<(), QuiltError>,
{
    partition_with_publisher_inner(
        df,
        colname,
        output_dir,
        publish,
        PartitionRunHooks::default(),
        resources,
    )
}

fn partition_with_publisher_inner<P>(
    df: &LazyFrame,
    colname: &str,
    output_dir: &str,
    publish: P,
    mut hooks: PartitionRunHooks<'_>,
    resources: &ExecutionResources,
) -> Result<FinalizerResult, QuiltError>
where
    P: Fn(&Path, &Path) -> Result<(), QuiltError>,
{
    let schema = df.clone().collect_schema().map_err(|error| {
        QuiltError::schema(
            "partition",
            None::<String>,
            format!("failed to get DataFrame schema: {error}"),
        )
    })?;
    if schema.get(colname).is_none() {
        return Err(QuiltError::schema(
            "partition",
            Some(colname),
            format!("column '{colname}' not found"),
        ));
    }
    let output_path = Path::new(output_dir).to_path_buf();
    let stage = create_staging_directory(&output_path)?;
    let partition_resources = resources.fork(Vec::new());
    let mut before_write = hooks.before_write.take();
    let metrics = hooks.metrics.take();
    let mut writers = PartitionWriterPool::new(&stage, before_write.take(), &partition_resources);
    let result = (|| {
        let (spool_resources, spool) = spool_partition_input(df, &partition_resources)?;
        let partition_names = discover_partition_names(&spool, colname)?;
        write_partition_batches(&spool, colname, &partition_names, &mut writers, metrics)?;
        let names = writers.finish()?;
        drop(spool_resources);
        publish(&stage, &output_path)?;
        Ok(names
            .into_iter()
            .map(|name| output_path.join(name))
            .collect())
    })();
    drop(writers);
    drop(partition_resources);
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result.map(FinalizerResult::Files)
}

fn spool_partition_input(
    frame: &LazyFrame,
    resources: &ExecutionResources,
) -> Result<(ExecutionResources, Vec<PathBuf>), QuiltError> {
    let spool_resources = resources.fork(Vec::new());
    let paths = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let callback_paths = paths.clone();
    let callback_resources = spool_resources.clone();
    let sink = frame
        .clone()
        .sink_batches(
            PlanCallback::new(move |mut batch: DataFrame| {
                batch.align_chunks_par();
                let mut reservation = callback_resources
                    .reserve_temp_file("qlt-partition-input", "parquet")
                    .map_err(|error| {
                        polars::error::PolarsError::ComputeError(error.to_string().into())
                    })?;
                let path = reservation.path().to_path_buf();
                let writer = reservation.file_mut().ok_or_else(|| {
                    polars::error::PolarsError::ComputeError("partition spool unavailable".into())
                })?;
                ParquetWriter::new(writer)
                    .with_compression(ParquetCompression::Snappy)
                    .finish(&mut batch)?;
                callback_resources
                    .retain_temp_file(reservation)
                    .map_err(|error| {
                        polars::error::PolarsError::ComputeError(error.to_string().into())
                    })?;
                callback_paths
                    .lock()
                    .map_err(|_| {
                        polars::error::PolarsError::ComputeError(
                            "partition spool lock poisoned".into(),
                        )
                    })?
                    .push(path);
                Ok(false)
            }),
            true,
            std::num::NonZeroUsize::new(PARTITION_BATCH_ROWS),
        )
        .map_err(|error| {
            QuiltError::operation("partition", format!("failed to evaluate input: {error}"))
        })?;
    sink.collect().map_err(|error| {
        QuiltError::operation("partition", format!("failed to evaluate input: {error}"))
    })?;
    let paths = paths
        .lock()
        .map_err(|_| QuiltError::operation("partition", "partition spool lock poisoned"))?
        .clone();
    Ok((spool_resources, paths))
}

fn for_each_partition_batch<F>(spool: &[PathBuf], mut visit: F) -> Result<(), QuiltError>
where
    F: FnMut(DataFrame) -> Result<(), QuiltError>,
{
    for batch_path in spool {
        let reader = ParquetReader::new(File::open(batch_path).map_err(|error| {
            QuiltError::io(
                "open partition input batch",
                Some(batch_path.display().to_string()),
                error.to_string(),
            )
        })?)
        .set_low_memory(true)
        .read_parallel(ParallelStrategy::None);
        let batch = reader.finish().map_err(|error| {
            QuiltError::operation(
                "partition",
                format!("failed to read partition input batch: {error}"),
            )
        })?;
        visit(batch)?;
    }
    Ok(())
}

fn discover_partition_names(
    spool: &[PathBuf],
    colname: &str,
) -> Result<HashMap<Vec<u8>, String>, QuiltError> {
    // Typed identities are canonical binary encodings; display strings are
    // used only for filenames.
    let mut keys = HashMap::<Vec<u8>, (String, usize)>::new();
    for_each_partition_batch(spool, |batch| {
        let column = batch.column(colname).map_err(|error| {
            QuiltError::schema(
                "partition",
                Some(colname),
                format!("failed to read partition column: {error}"),
            )
        })?;
        for row in 0..batch.height() {
            let value = column.get(row).map_err(|error| {
                QuiltError::operation(
                    "partition",
                    format!("failed to read partition value: {error}"),
                )
            })?;
            let identity = partition_identity(value.clone())?;
            let first_seen = keys.len();
            keys.entry(identity)
                .or_insert_with(|| (partition_value(value), first_seen));
        }
        Ok(())
    })?;

    // Stable sorting preserves first-seen order for distinct typed values
    // whose display strings are identical, keeping suffix allocation
    // deterministic for a deterministic input stream.
    let mut keys = keys.into_iter().collect::<Vec<_>>();
    keys.sort_by(
        |(_, (left_name, left_index)), (_, (right_name, right_index))| {
            left_name.cmp(right_name).then(left_index.cmp(right_index))
        },
    );
    // Reserve final, bounded names globally. A generated suffix can itself be
    // a value's natural sanitized name (for example `a_b-2`), and Windows
    // treats case-only differences as the same path.
    let mut used = HashSet::<String>::new();
    let mut names = HashMap::with_capacity(keys.len());
    for (identity, (raw, _first_seen)) in keys {
        let base = sanitize_filename(&raw);
        let mut ordinal = 1usize;
        loop {
            let candidate = bounded_partition_name(&raw, &base, ordinal);
            if used.insert(candidate.to_lowercase()) {
                names.insert(identity, candidate);
                break;
            }
            ordinal = ordinal.checked_add(1).ok_or_else(|| {
                QuiltError::operation("partition", "could not allocate a unique partition name")
            })?;
        }
    }
    Ok(names)
}

fn write_partition_batches(
    spool: &[PathBuf],
    colname: &str,
    partition_names: &HashMap<Vec<u8>, String>,
    writers: &mut PartitionWriterPool<'_>,
    mut metrics: Option<&mut PartitionMetrics>,
) -> Result<(), QuiltError> {
    let result = for_each_partition_batch(spool, |batch| {
        if let Some(metrics) = metrics.as_deref_mut() {
            metrics.batch_count += 1;
            metrics.max_batch_rows = metrics.max_batch_rows.max(batch.height());
        }
        let column = batch.column(colname).map_err(|error| {
            QuiltError::schema(
                "partition",
                Some(colname),
                format!("failed to read partition column: {error}"),
            )
        })?;
        let mut rows_by_partition = BTreeMap::<String, Vec<IdxSize>>::new();
        for row in 0..batch.height() {
            let value = column.get(row).map_err(|error| {
                QuiltError::operation(
                    "partition",
                    format!("failed to read partition value: {error}"),
                )
            })?;
            let name = partition_names
                .get(&partition_identity(value)?)
                .ok_or_else(|| {
                    QuiltError::operation(
                        "partition",
                        "partition key disappeared between discovery and writing",
                    )
                })?;
            rows_by_partition
                .entry(name.clone())
                .or_default()
                .push(row as IdxSize);
        }

        for (name, rows) in rows_by_partition {
            let mut group = batch
                .take(&IdxCa::from_vec(PlSmallStr::EMPTY, rows))
                .map_err(|error| {
                    QuiltError::operation(
                        "partition",
                        format!("failed to gather partition rows: {error}"),
                    )
                })?;
            writers.write_batch(&name, &mut group)?;
        }
        Ok(())
    });
    if let Some(metrics) = metrics {
        metrics.max_open_writers = metrics.max_open_writers.max(writers.peak_open);
    }
    result
}

fn partition_identity(value: AnyValue<'_>) -> Result<Vec<u8>, QuiltError> {
    let mut identity = Vec::new();
    encode_partition_value(value, &mut identity)?;
    Ok(identity)
}

fn encode_partition_value(value: AnyValue<'_>, output: &mut Vec<u8>) -> Result<(), QuiltError> {
    let value = value.as_borrowed();
    let category_text = match &value {
        AnyValue::Categorical(_, _) | AnyValue::Enum(_, _) => Some(
            value
                .get_str()
                .ok_or_else(unsupported_partition_value)?
                .to_owned(),
        ),
        _ => None,
    };
    match value {
        AnyValue::Null => output.push(0),
        AnyValue::Boolean(value) => output.extend_from_slice(&[1, u8::from(value)]),
        AnyValue::Int8(value) => encode_scalar(output, 2, value.to_le_bytes()),
        AnyValue::Int16(value) => encode_scalar(output, 3, value.to_le_bytes()),
        AnyValue::Int32(value) => encode_scalar(output, 4, value.to_le_bytes()),
        AnyValue::Int64(value) => encode_scalar(output, 5, value.to_le_bytes()),
        AnyValue::Int128(value) => encode_scalar(output, 6, value.to_le_bytes()),
        AnyValue::UInt8(value) => encode_scalar(output, 7, value.to_le_bytes()),
        AnyValue::UInt16(value) => encode_scalar(output, 8, value.to_le_bytes()),
        AnyValue::UInt32(value) => encode_scalar(output, 9, value.to_le_bytes()),
        AnyValue::UInt64(value) => encode_scalar(output, 10, value.to_le_bytes()),
        AnyValue::UInt128(value) => encode_scalar(output, 11, value.to_le_bytes()),
        AnyValue::Float16(value) => {
            let bits = if value.is_nan() {
                0x7e00
            } else if value.to_bits() & 0x7fff == 0 {
                0
            } else {
                value.to_bits()
            };
            encode_scalar(output, 12, bits.to_le_bytes());
        }
        AnyValue::Float32(value) => {
            let bits = if value.is_nan() {
                0x7fc0_0000
            } else if value == 0.0 {
                0
            } else {
                value.to_bits()
            };
            encode_scalar(output, 13, bits.to_le_bytes());
        }
        AnyValue::Float64(value) => {
            let bits = if value.is_nan() {
                0x7ff8_0000_0000_0000
            } else if value == 0.0 {
                0
            } else {
                value.to_bits()
            };
            encode_scalar(output, 14, bits.to_le_bytes());
        }
        AnyValue::String(value) => encode_bytes(output, 15, value.as_bytes()),
        AnyValue::Binary(value) => encode_bytes(output, 16, value),
        AnyValue::Date(value) => encode_scalar(output, 17, value.to_le_bytes()),
        AnyValue::Datetime(value, unit, timezone) => {
            output.push(18);
            output.extend_from_slice(&value.to_le_bytes());
            encode_time_unit(unit, output);
            match timezone {
                Some(timezone) => encode_bytes(output, 1, timezone.as_str().as_bytes()),
                None => output.push(0),
            }
        }
        AnyValue::Duration(value, unit) => {
            output.push(19);
            output.extend_from_slice(&value.to_le_bytes());
            encode_time_unit(unit, output);
        }
        AnyValue::Time(value) => encode_scalar(output, 20, value.to_le_bytes()),
        AnyValue::Categorical(_, _) => {
            let value = category_text
                .as_deref()
                .ok_or_else(unsupported_partition_value)?;
            encode_bytes(output, 21, value.as_bytes());
        }
        AnyValue::Enum(_, _) => {
            let value = category_text
                .as_deref()
                .ok_or_else(unsupported_partition_value)?;
            encode_bytes(output, 22, value.as_bytes());
        }
        AnyValue::List(values) => {
            output.push(23);
            encode_len(output, values.len());
            for item in values.iter() {
                encode_partition_value(item, output)?;
            }
        }
        AnyValue::Struct(index, array, fields) => {
            output.push(24);
            encode_len(output, fields.len());
            for item in AnyValue::Struct(index, array, fields)._iter_struct_av() {
                encode_partition_value(item, output)?;
            }
        }
        AnyValue::StructOwned(values) => {
            output.push(24);
            encode_len(output, values.0.len());
            for item in values.0 {
                encode_partition_value(item, output)?;
            }
        }
        AnyValue::Decimal(value, scale, precision) => {
            output.push(25);
            output.extend_from_slice(&value.to_le_bytes());
            encode_len(output, scale);
            encode_len(output, precision);
        }
        _ => return Err(unsupported_partition_value()),
    }
    Ok(())
}

fn encode_scalar<const N: usize>(output: &mut Vec<u8>, tag: u8, bytes: [u8; N]) {
    output.push(tag);
    output.extend_from_slice(&bytes);
}

fn encode_bytes(output: &mut Vec<u8>, tag: u8, bytes: &[u8]) {
    output.push(tag);
    encode_len(output, bytes.len());
    output.extend_from_slice(bytes);
}

fn encode_len(output: &mut Vec<u8>, len: usize) {
    output.extend_from_slice(&(len as u64).to_le_bytes());
}

fn encode_time_unit(unit: TimeUnit, output: &mut Vec<u8>) {
    output.push(match unit {
        TimeUnit::Nanoseconds => 0,
        TimeUnit::Microseconds => 1,
        TimeUnit::Milliseconds => 2,
    });
}

fn unsupported_partition_value() -> QuiltError {
    QuiltError::operation(
        "partition",
        "partition column contains a value type without a lossless identity encoding",
    )
}

struct PartitionWriterPool<'a> {
    stage: PathBuf,
    writers: HashMap<String, crate::controllers::resources::AccountedFile>,
    order: VecDeque<String>,
    header_written: HashSet<String>,
    names: std::collections::BTreeSet<String>,
    peak_open: usize,
    before_write: Option<&'a mut dyn FnMut(usize) -> Result<(), QuiltError>>,
    resources: ExecutionResources,
}

impl<'a> PartitionWriterPool<'a> {
    fn new(
        stage: &Path,
        before_write: Option<&'a mut dyn FnMut(usize) -> Result<(), QuiltError>>,
        resources: &ExecutionResources,
    ) -> Self {
        Self {
            stage: stage.to_path_buf(),
            writers: HashMap::new(),
            order: VecDeque::new(),
            header_written: HashSet::new(),
            names: std::collections::BTreeSet::new(),
            peak_open: 0,
            before_write,
            resources: resources.clone(),
        }
    }

    fn write_batch(&mut self, name: &str, group: &mut DataFrame) -> Result<(), QuiltError> {
        if !self.writers.contains_key(name) {
            if self.writers.len() >= MAX_OPEN_PARTITION_WRITERS {
                if let Some(evicted) = self.order.pop_front() {
                    if let Some(file) = self.writers.remove(&evicted) {
                        file.sync_all().map_err(|error| {
                            QuiltError::io(
                                "sync partition file",
                                Some(
                                    self.stage
                                        .join(format!("{evicted}.csv"))
                                        .display()
                                        .to_string(),
                                ),
                                error.to_string(),
                            )
                        })?;
                    }
                }
            }
            let path = self.stage.join(format!("{name}.csv"));
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|error| {
                    QuiltError::io(
                        "open partition file",
                        Some(path.display().to_string()),
                        error.to_string(),
                    )
                })?;
            let file = self
                .resources
                .tracked_temp_file(&path, file)
                .map_err(|error| {
                    QuiltError::io(
                        "open partition file",
                        Some(path.display().to_string()),
                        error.to_string(),
                    )
                })?;
            self.writers.insert(name.to_string(), file);
            self.peak_open = self.peak_open.max(self.writers.len());
        }

        self.order.retain(|open_name| open_name != name);
        self.order.push_back(name.to_string());
        let include_header = self.header_written.insert(name.to_string());
        if let Some(before_write) = self.before_write.as_deref_mut() {
            before_write(self.names.len())?;
        }
        let file = self
            .writers
            .get_mut(name)
            .expect("partition writer inserted");
        write_partition_csv(&mut *file, include_header, group)?;
        self.names.insert(name.to_string());
        Ok(())
    }

    fn finish(&mut self) -> Result<Vec<String>, QuiltError> {
        for (name, file) in &mut self.writers {
            file.sync_all().map_err(|error| {
                QuiltError::io(
                    "sync partition file",
                    Some(self.stage.join(format!("{name}.csv")).display().to_string()),
                    error.to_string(),
                )
            })?;
        }
        self.writers.clear();
        Ok(self
            .names
            .iter()
            .map(|name| format!("{name}.csv"))
            .collect())
    }

    #[cfg(test)]
    fn peak_open(&self) -> usize {
        self.peak_open
    }
}

fn write_partition_csv<W: Write>(
    writer: W,
    include_header: bool,
    group: &mut DataFrame,
) -> Result<(), QuiltError> {
    // Keep the partition key and payload typed through grouping and gathering;
    // stringify nested values only at the same final CSV boundary used by
    // show/dump. This also keeps duration counts exact in their source unit.
    let mut output = super::csv::csv_frame(&group.clone().lazy(), "write partition file")?
        .collect()
        .map_err(|error| QuiltError::finalizer("write partition file", error.to_string()))?;
    CsvWriter::new(writer)
        .include_header(include_header)
        .finish(&mut output)
        .map_err(|error| QuiltError::finalizer("write partition file", error.to_string()))
}

fn create_staging_directory(output: &Path) -> Result<PathBuf, QuiltError> {
    if output.exists() {
        return Err(QuiltError::io(
            "create partition directory",
            Some(output.display().to_string()),
            "destination already exists; refusing to overwrite",
        ));
    }
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err(QuiltError::io(
            "create partition directory",
            Some(parent.display().to_string()),
            "destination directory does not exist",
        ));
    }
    let name = output
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("partitions");
    for _ in 0..64 {
        let nonce = PARTITION_STAGE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let stage = parent.join(format!(".{name}.qlt-stage-{}-{nonce}", std::process::id()));
        match fs::create_dir(&stage) {
            Ok(()) => return Ok(stage),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(QuiltError::io(
                    "create partition staging directory",
                    Some(stage.display().to_string()),
                    error.to_string(),
                ));
            }
        }
    }
    Err(QuiltError::io(
        "create partition staging directory",
        Some(output.display().to_string()),
        "could not reserve unique staging directory",
    ))
}

fn partition_value(value: AnyValue<'_>) -> String {
    match value {
        AnyValue::Null => NULL_PARTITION.to_string(),
        AnyValue::String(value) => value.to_string(),
        AnyValue::StringOwned(value) => value.to_string(),
        _ => value.to_string(),
    }
}

fn sanitize_filename(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|character| {
            if character.is_control() || "/\\:*?\"<>|".contains(character) {
                '_'
            } else {
                character
            }
        })
        .collect::<String>()
        .trim()
        .trim_end_matches(['.', ' '])
        .to_string();
    let sanitized = if sanitized.is_empty() {
        "_empty".to_string()
    } else if sanitized == "." || sanitized == ".." {
        "_dot".to_string()
    } else {
        sanitized
    };
    let sanitized = if is_reserved_filename(&sanitized) {
        format!("_{sanitized}")
    } else {
        sanitized
    };
    bound_filename(&sanitized, value)
}

fn bound_filename(value: &str, raw: &str) -> String {
    if value.len() <= MAX_PARTITION_NAME_BYTES {
        return value.to_string();
    }
    let hash = stable_hash(raw);
    let suffix = format!("-{hash:016x}");
    let limit = MAX_PARTITION_NAME_BYTES.saturating_sub(suffix.len());
    let mut end = limit.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &value[..end], suffix)
}

fn bounded_partition_name(raw: &str, sanitized: &str, ordinal: usize) -> String {
    let suffix = if ordinal == 1 {
        String::new()
    } else {
        format!("-{ordinal}")
    };
    let budget = MAX_PARTITION_NAME_BYTES.saturating_sub(suffix.len());
    let mut end = sanitized.len().min(budget);
    while end > 0 && !sanitized.is_char_boundary(end) {
        end -= 1;
    }
    let base = &sanitized[..end];
    if sanitized.len() <= budget {
        format!("{base}{suffix}")
    } else {
        let hash = format!("-{:016x}", stable_hash(raw));
        let hash_budget = budget.saturating_sub(hash.len());
        let mut hash_end = sanitized.len().min(hash_budget);
        while hash_end > 0 && !sanitized.is_char_boundary(hash_end) {
            hash_end -= 1;
        }
        format!("{}{hash}{suffix}", &sanitized[..hash_end])
    }
}

fn stable_hash(value: &str) -> u64 {
    value.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

fn is_reserved_filename(value: &str) -> bool {
    // Windows reserves device names even when followed by an extension (for
    // example, `CON.data`). Ignore the extension and trailing spaces/dots in
    // the stem when checking the reserved name.
    let stem = value
        .split('.')
        .next()
        .unwrap_or(value)
        .trim_end_matches([' ', '.']);
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::finalizers::{publish_directory_and_sync, NativePublicationBackend};
    use std::fs;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_parent(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "qlt-partition-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn concurrent_publishers_stage_complete_partitions_and_have_one_winner() {
        let parent = test_parent("concurrent");
        let target = parent.join("partitions");
        let barrier = Arc::new(Barrier::new(2));
        let handles = (0..2)
            .map(|_| {
                let target = target.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let frame = df!("part" => &["a", "b", "c"], "value" => &[1i64, 2, 3])
                        .unwrap()
                        .lazy();
                    partition_with_publisher(
                        &frame,
                        "part",
                        target.to_str().unwrap(),
                        move |stage, target| {
                            let unpublished = !target.exists();
                            let staged = fs::read_dir(stage).and_then(|entries| {
                                entries
                                    .map(|entry| fs::read_to_string(entry?.path()))
                                    .collect::<std::io::Result<Vec<_>>>()
                            });
                            barrier.wait();
                            assert!(unpublished);
                            let mut staged = staged.unwrap();
                            staged.sort();
                            assert_eq!(
                                staged,
                                [
                                    "part,value\na,1\n",
                                    "part,value\nb,2\n",
                                    "part,value\nc,3\n"
                                ]
                            );
                            let parent = target.parent().unwrap_or_else(|| Path::new("."));
                            publish_directory_and_sync(
                                &NativePublicationBackend,
                                stage,
                                target,
                                parent,
                                "publish partition directory",
                            )
                        },
                        &ExecutionResources::new(),
                    )
                    .is_ok()
                })
            })
            .collect::<Vec<_>>();
        let successes = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|success| *success)
            .count();
        assert_eq!(successes, 1);
        assert_eq!(fs::read_dir(&target).unwrap().count(), 3);
        for (name, value) in [("a", 1), ("b", 2), ("c", 3)] {
            assert_eq!(
                fs::read_to_string(target.join(format!("{name}.csv"))).unwrap(),
                format!("part,value\n{name},{value}\n")
            );
        }
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn failed_publish_leaves_no_target_or_staging_directory() {
        let parent = test_parent("failed-publish");
        let target = parent.join("partitions");
        let frame = df!("part" => &["a"], "value" => &[1i64]).unwrap().lazy();
        let result = partition_with_publisher(
            &frame,
            "part",
            target.to_str().unwrap(),
            |_stage, _target| Err(QuiltError::operation("test publish", "injected failure")),
            &ExecutionResources::new(),
        );
        assert!(result.is_err());
        assert!(!target.exists());
        assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn production_write_failure_cleans_after_first_partition_file() {
        let parent = test_parent("failed-production-write");
        let target = parent.join("partitions");
        let frame = df!("part" => &["a", "b"], "value" => &[1i64, 2])
            .unwrap()
            .lazy();
        let mut hook = |completed: usize| {
            if completed >= 1 {
                Err(QuiltError::operation(
                    "test partition write",
                    "injected failure after first partition",
                ))
            } else {
                Ok(())
            }
        };
        let hooks = PartitionRunHooks {
            metrics: None,
            before_write: Some(&mut hook),
        };
        let result = partition_with_publisher_inner(
            &frame,
            "part",
            target.to_str().unwrap(),
            |_stage, _target| panic!("publication must not run after write failure"),
            hooks,
            &ExecutionResources::new(),
        );
        assert!(result.is_err());
        assert!(!target.exists());
        assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn input_write_failure_leaves_no_target_or_staging_directory() {
        let parent = test_parent("failed-input");
        let target = parent.join("partitions");
        let frame = df!("part" => &["a"], "value" => &[1i64])
            .unwrap()
            .lazy()
            .select([col("missing")]);
        let result = partition(&frame, "part", target.to_str().unwrap());
        assert!(result.is_err());
        assert!(!target.exists());
        assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn partition_writer_open_failure_is_reported_without_partial_file() {
        let parent = test_parent("failed-writer");
        let stage = parent.join("missing-stage");
        let resources = ExecutionResources::new();
        let mut writers = PartitionWriterPool::new(&stage, None, &resources);
        let mut group = df!("part" => &["a"], "value" => &[1i64]).unwrap();
        assert!(writers.write_batch("a", &mut group).is_err());
        assert!(!stage.exists());
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn injected_mid_write_failure_cleans_staged_partition_directory() {
        struct FailingWriter;
        impl Write for FailingWriter {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("injected partition write failure"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let parent = test_parent("failed-mid-write");
        let stage = parent.join("stage");
        fs::create_dir(&stage).unwrap();
        let mut group = df!("part" => &["a"], "value" => &[1i64]).unwrap();
        let result = write_partition_csv(FailingWriter, true, &mut group);
        assert!(result.is_err());
        fs::remove_dir_all(&stage).unwrap();
        assert!(!stage.exists());
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn long_unicode_collision_keeps_ordinal_within_name_budget() {
        let parent = test_parent("long-name");
        let target = parent.join("partitions");
        let prefix = "界".repeat(39);
        let first = format!("{prefix}/");
        let second = format!("{prefix}\\");
        let frame = df!("part" => &[first.as_str(), second.as_str()], "value" => &[1i64, 2])
            .unwrap()
            .lazy();
        let result = partition(&frame, "part", target.to_str().unwrap()).unwrap();
        let FinalizerResult::Files(files) = result else {
            panic!("partition must return files");
        };
        assert!(files.iter().all(|file| {
            file.file_stem().unwrap().to_string_lossy().len() <= MAX_PARTITION_NAME_BYTES
        }));
        assert!(files
            .iter()
            .any(|file| { file.file_stem().unwrap().to_string_lossy().ends_with("-2") }));
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn bounded_writer_pool_preserves_headers_and_caps_open_files() {
        let parent = test_parent("bounded-writers");
        let stage = parent.join("stage");
        fs::create_dir(&stage).unwrap();
        let resources = ExecutionResources::new();
        let mut writers = PartitionWriterPool::new(&stage, None, &resources);
        let mut expected = HashMap::new();
        for row in 0..(MAX_OPEN_PARTITION_WRITERS * 4) {
            let name = format!("part-{row}");
            let value = row as i64;
            let mut group = df!("part" => &[name.as_str()], "value" => &[value]).unwrap();
            writers.write_batch(&name, &mut group).unwrap();
            expected.insert(name, format!("part,value\npart-{row},{value}\n"));
        }
        let mut repeated = df!("part" => &["part-0"], "value" => &[999i64]).unwrap();
        writers.write_batch("part-0", &mut repeated).unwrap();
        expected.get_mut("part-0").unwrap().push_str("part-0,999\n");
        assert!(writers.peak_open() <= MAX_OPEN_PARTITION_WRITERS);
        let names = writers.finish().unwrap();
        assert_eq!(names.len(), expected.len());
        for (name, contents) in expected {
            assert_eq!(
                fs::read_to_string(stage.join(format!("{name}.csv"))).unwrap(),
                contents
            );
        }
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn partition_budget_counts_evicted_staging_files_until_cleanup() {
        let parent = test_parent("budgeted-evicted-writers");
        let stage = parent.join("stage");
        fs::create_dir(&stage).unwrap();
        // Leave seven bytes after the first 16 complete files. The next
        // header needs eleven bytes, so it must fail even after one writer is
        // evicted. Releasing charges with handles would incorrectly pass.
        let first_sixteen_bytes = (0..16)
            .map(|row| {
                let name = format!("p{row}");
                format!("part,value\n{name},{row}\n").len() as u64
            })
            .sum::<u64>();
        let resources = ExecutionResources::new().with_temp_limit_for_test(first_sixteen_bytes + 7);
        let mut writers = PartitionWriterPool::new(&stage, None, &resources);
        for row in 0..17 {
            let name = format!("p{row}");
            let value = row as i64;
            let mut group = df!("part" => &[name.as_str()], "value" => &[value]).unwrap();
            let result = writers.write_batch(&name, &mut group);
            if row < 16 {
                result.unwrap();
            } else {
                assert!(result.is_err());
            }
        }
        assert_eq!(writers.peak_open(), MAX_OPEN_PARTITION_WRITERS);
        assert_eq!(resources.temp_usage().0, first_sixteen_bytes);
        drop(writers);
        drop(resources);
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn large_partition_is_batched_and_caps_open_writers_end_to_end() {
        let parent = test_parent("large-bounded");
        let target = parent.join("partitions");
        let partition_count = MAX_OPEN_PARTITION_WRITERS + 3;
        let row_count = PARTITION_BATCH_ROWS * 3 + 17;
        let parts = (0..row_count)
            .map(|row| format!("part-{}", row % partition_count))
            .collect::<Vec<_>>();
        let values = (0..row_count as i64).collect::<Vec<_>>();
        let frame = DataFrame::new_infer_height(vec![
            Series::new("part".into(), parts).into(),
            Series::new("value".into(), values).into(),
        ])
        .unwrap()
        .lazy();
        let mut metrics = PartitionMetrics::default();
        let hooks = PartitionRunHooks {
            metrics: Some(&mut metrics),
            before_write: None,
        };
        let FinalizerResult::Files(files) = partition_with_publisher_inner(
            &frame,
            "part",
            target.to_str().unwrap(),
            |stage, target| {
                let parent = target.parent().unwrap_or_else(|| Path::new("."));
                publish_directory_and_sync(
                    &NativePublicationBackend,
                    stage,
                    target,
                    parent,
                    "publish partition directory",
                )
            },
            hooks,
            &ExecutionResources::new(),
        )
        .unwrap() else {
            panic!("partition must return file paths");
        };
        assert_eq!(files.len(), partition_count);
        assert!(metrics.batch_count >= 4);
        assert!(metrics.max_batch_rows <= PARTITION_BATCH_ROWS);
        assert!(metrics.max_open_writers <= MAX_OPEN_PARTITION_WRITERS);
        let output_rows = files
            .iter()
            .map(|file| fs::read_to_string(file).unwrap().lines().count() - 1)
            .sum::<usize>();
        assert_eq!(output_rows, row_count);
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn disk_backed_partition_preserves_null_empty_and_headers() {
        let parent = test_parent("null-empty");
        let target = parent.join("partitions");
        let frame = df!(
            "part" => &[Some(""), None, Some("a/b"), Some("a\\b")],
            "value" => &[1i64, 2, 3, 4]
        )
        .unwrap()
        .lazy();
        let FinalizerResult::Files(files) =
            partition(&frame, "part", target.to_str().unwrap()).unwrap()
        else {
            panic!("partition must return file paths");
        };
        assert_eq!(files.len(), 4);
        assert!(target.join("_empty.csv").exists());
        assert!(target.join("_null.csv").exists());
        assert!(target.join("a_b.csv").exists());
        assert!(target.join("a_b-2.csv").exists());
        for file in files {
            let contents = fs::read_to_string(file).unwrap();
            assert!(contents.starts_with("part,value\n"));
            assert_eq!(contents.lines().count(), 2);
        }
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn typed_partition_identity_keeps_values_with_same_display_separate() {
        fn assert_payloads(files: &[PathBuf], expected: &[&str]) {
            let mut actual = files
                .iter()
                .map(|file| {
                    let contents = fs::read_to_string(file).unwrap();
                    let mut lines = contents.lines();
                    assert!(lines.next().unwrap().ends_with(",value"));
                    let row = lines.next().unwrap();
                    assert!(lines.next().is_none());
                    row.rsplit(',').next().unwrap().to_string()
                })
                .collect::<Vec<_>>();
            let mut expected = expected
                .iter()
                .map(|value| (*value).to_string())
                .collect::<Vec<_>>();
            actual.sort();
            expected.sort();
            assert_eq!(actual, expected);
        }

        fn assert_grouped_payloads(files: &[PathBuf], expected: &[&str]) {
            assert_eq!(files.len(), 1);
            let contents = fs::read_to_string(&files[0]).unwrap();
            let mut lines = contents.lines();
            assert!(lines.next().unwrap().ends_with(",value"));
            let mut actual = lines
                .map(|row| row.rsplit(',').next().unwrap().to_string())
                .collect::<Vec<_>>();
            let mut expected = expected
                .iter()
                .map(|value| (*value).to_string())
                .collect::<Vec<_>>();
            actual.sort();
            expected.sort();
            assert_eq!(actual, expected);
        }

        fn assert_dictionary_partition(parent: &Path, label: &str, dtype: DataType) {
            let row_count = PARTITION_BATCH_ROWS + 1;
            let parts = (0..row_count)
                .map(|row| if row % 2 == 0 { "a" } else { "b" })
                .collect::<Vec<_>>();
            let values = (0..row_count as i64).collect::<Vec<_>>();
            let frame = DataFrame::new_infer_height(vec![
                Series::new("part".into(), parts)
                    .cast(&dtype)
                    .unwrap()
                    .into(),
                Series::new("value".into(), values).into(),
            ])
            .unwrap()
            .lazy();
            let target = parent.join(label);
            let FinalizerResult::Files(files) =
                partition(&frame, "part", target.to_str().unwrap()).unwrap()
            else {
                panic!("partition must return file paths");
            };
            assert_eq!(files.len(), 2);
            let total_rows = files
                .iter()
                .map(|file| fs::read_to_string(file).unwrap().lines().count() - 1)
                .sum::<usize>();
            assert_eq!(total_rows, row_count);
            let mut seen_keys = HashSet::new();
            for file in files {
                let contents = fs::read_to_string(file).unwrap();
                let mut lines = contents.lines();
                assert_eq!(lines.next(), Some("part,value"));
                let rows = lines.collect::<Vec<_>>();
                let key = rows[0].split_once(',').unwrap().0;
                assert!(key == "a" || key == "b");
                assert!(rows.iter().all(|row| row.starts_with(&format!("{key},"))));
                seen_keys.insert(key.to_string());
                let mut expected = String::from("part,value\n");
                for row in 0..row_count {
                    if (row % 2 == 0 && key == "a") || (row % 2 == 1 && key == "b") {
                        expected.push_str(&format!("{key},{row}\n"));
                    }
                }
                assert_eq!(contents, expected);
            }
            assert_eq!(seen_keys, HashSet::from(["a".to_string(), "b".to_string()]));
        }

        let parent = test_parent("typed-identity");

        let floats = df!(
            "part" => &[1.2345678901f64, 1.2345678902],
            "value" => &[1i64, 2]
        )
        .unwrap()
        .lazy();
        let float_target = parent.join("floats");
        let FinalizerResult::Files(float_files) =
            partition(&floats, "part", float_target.to_str().unwrap()).unwrap()
        else {
            panic!("partition must return file paths");
        };
        assert_payloads(&float_files, &["1", "2"]);

        let zeroes = DataFrame::new_infer_height(vec![
            Series::new("part".into(), &[0.0f64, -0.0]).into(),
            Series::new("value".into(), &[4i64, 5]).into(),
        ])
        .unwrap()
        .lazy();
        let zero_target = parent.join("zeroes");
        let FinalizerResult::Files(zero_files) =
            partition(&zeroes, "part", zero_target.to_str().unwrap()).unwrap()
        else {
            panic!("partition must return file paths");
        };
        assert_grouped_payloads(&zero_files, &["4", "5"]);

        let nan_values = [
            f64::from_bits(0x7ff8_0000_0000_0001),
            f64::from_bits(0x7ff8_0000_0000_0002),
        ];
        let nans = DataFrame::new_infer_height(vec![
            Series::new("part".into(), &nan_values).into(),
            Series::new("value".into(), &[6i64, 7]).into(),
        ])
        .unwrap()
        .lazy();
        let nan_target = parent.join("nans");
        let FinalizerResult::Files(nan_files) =
            partition(&nans, "part", nan_target.to_str().unwrap()).unwrap()
        else {
            panic!("partition must return file paths");
        };
        assert_grouped_payloads(&nan_files, &["6", "7"]);

        let left = (0..24)
            .map(|index| if index == 12 { 700i64 } else { index })
            .collect::<Vec<_>>();
        let right = (0..24)
            .map(|index| if index == 12 { 701i64 } else { index })
            .collect::<Vec<_>>();
        let lists = DataFrame::new_infer_height(vec![
            Series::new(
                "part".into(),
                &[
                    Some(Series::new(PlSmallStr::EMPTY, left.as_slice())),
                    Some(Series::new(PlSmallStr::EMPTY, right.as_slice())),
                ],
            )
            .into(),
            Series::new("value".into(), &[1i64, 2]).into(),
        ])
        .unwrap()
        .lazy();
        let list_target = parent.join("lists");
        let FinalizerResult::Files(list_files) =
            partition(&lists, "part", list_target.to_str().unwrap()).unwrap()
        else {
            panic!("partition must return file paths");
        };
        assert_payloads(&list_files, &["1", "2"]);

        let fields = (0..24)
            .map(|index| {
                let values = if index == 12 {
                    [700i64, 701]
                } else {
                    [index, index]
                };
                Series::new(format!("field-{index}").into(), &values)
            })
            .collect::<Vec<_>>();
        let structs = StructChunked::from_series("part".into(), 2, fields.iter())
            .unwrap()
            .into_series();
        let structs = DataFrame::new_infer_height(vec![
            structs.into(),
            Series::new("value".into(), &[1i64, 2]).into(),
        ])
        .unwrap()
        .lazy();
        let struct_target = parent.join("structs");
        let FinalizerResult::Files(struct_files) =
            partition(&structs, "part", struct_target.to_str().unwrap()).unwrap()
        else {
            panic!("partition must return file paths");
        };
        assert_payloads(&struct_files, &["1", "2"]);

        let markers = df!(
            "part" => &[Some(""), Some("_null"), None],
            "value" => &[1i64, 2, 3]
        )
        .unwrap()
        .lazy();
        let marker_target = parent.join("markers");
        let FinalizerResult::Files(marker_files) =
            partition(&markers, "part", marker_target.to_str().unwrap()).unwrap()
        else {
            panic!("partition must return file paths");
        };
        assert_payloads(&marker_files, &["1", "2", "3"]);

        assert_dictionary_partition(
            &parent,
            "categorical",
            DataType::from_categories(Categories::global()),
        );
        assert_dictionary_partition(
            &parent,
            "enum",
            DataType::from_frozen_categories(FrozenCategories::new(["a", "b"]).unwrap()),
        );

        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn partition_names_remain_unique_after_sanitization_and_suffixing() {
        let parent = test_parent("sanitized-name-collision");
        let target = parent.join("partitions");
        let frame = df!(
            "part" => &[
                "a/b",
                "a\\b",
                "a_b-2",
                "Ä",
                "ä",
                "CON.data",
                "lpt1.logs",
            ],
            "value" => &[1i64, 2, 3, 4, 5, 6, 7]
        )
        .unwrap()
        .lazy();
        let FinalizerResult::Files(files) =
            partition(&frame, "part", target.to_str().unwrap()).unwrap()
        else {
            panic!("partition must return file paths");
        };
        assert_eq!(files.len(), 7);
        let names = files
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<HashSet<_>>();
        assert_eq!(names.len(), 7);
        assert!(names.contains("a_b.csv"));
        assert!(names.contains("a_b-2.csv"));
        assert!(names.contains("a_b-2-2.csv"));
        assert!(names.contains("Ä.csv"));
        assert!(names.contains("ä-2.csv"));
        assert!(names.contains("_CON.data.csv"));
        assert!(names.contains("_lpt1.logs.csv"));
        for (name, contents) in [
            ("a_b.csv", "part,value\na/b,1\n"),
            ("a_b-2.csv", "part,value\na\\b,2\n"),
            ("a_b-2-2.csv", "part,value\na_b-2,3\n"),
            ("Ä.csv", "part,value\nÄ,4\n"),
            ("ä-2.csv", "part,value\nä,5\n"),
            ("_CON.data.csv", "part,value\nCON.data,6\n"),
            ("_lpt1.logs.csv", "part,value\nlpt1.logs,7\n"),
        ] {
            assert_eq!(fs::read_to_string(target.join(name)).unwrap(), contents);
        }
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn partition_serializes_nested_payload_at_csv_boundary() {
        let parent = test_parent("nested-payload");
        let target = parent.join("partitions");
        let frame = DataFrame::new_infer_height(vec![
            Series::new("part".into(), &["a", "a"]).into(),
            Series::new(
                "payload".into(),
                &[Some(Series::new(PlSmallStr::EMPTY, &[1i64, 2])), None],
            )
            .into(),
        ])
        .unwrap()
        .lazy();
        partition(&frame, "part", target.to_str().unwrap()).unwrap();
        assert_eq!(
            fs::read_to_string(target.join("a.csv")).unwrap(),
            "part,payload\na,\"[1,2]\"\na,\n"
        );
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn partition_preserves_csv_bytes_for_strings_and_nullable_values() {
        let parent = test_parent("byte-semantics");
        let target = parent.join("partitions");
        let frame = df!(
            "part" => &[Some("001"), Some("null"), Some(""), None, Some("001")],
            "value" => &[Some("a,b"), Some("q\"uote"), Some("line\nbreak"), None, Some("plain")]
        )
        .unwrap()
        .lazy();
        partition(&frame, "part", target.to_str().unwrap()).unwrap();
        assert_eq!(
            fs::read(target.join("001.csv")).unwrap(),
            b"part,value\n001,\"a,b\"\n001,plain\n"
        );
        assert_eq!(
            fs::read(target.join("null.csv")).unwrap(),
            b"part,value\nnull,\"q\"\"uote\"\n"
        );
        assert_eq!(
            fs::read(target.join("_empty.csv")).unwrap(),
            b"part,value\n\"\",\"line\nbreak\"\n"
        );
        assert_eq!(
            fs::read(target.join("_null.csv")).unwrap(),
            b"part,value\n,\n"
        );
        fs::remove_dir_all(parent).unwrap();
    }
}
