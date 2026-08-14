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
    partition_with_publisher(df, colname, output_dir, |stage, target| {
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
    })
}

fn partition_with_publisher<P>(
    df: &LazyFrame,
    colname: &str,
    output_dir: &str,
    publish: P,
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
    )
}

fn partition_with_publisher_inner<P>(
    df: &LazyFrame,
    colname: &str,
    output_dir: &str,
    publish: P,
    mut hooks: PartitionRunHooks<'_>,
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
    let mut before_write = hooks.before_write.take();
    let metrics = hooks.metrics.take();
    let mut writers = PartitionWriterPool::new(&stage, before_write.take());
    let result = (|| {
        let spool = stage.join(".qlt-partition-input");
        fs::create_dir(&spool).map_err(|error| {
            QuiltError::io(
                "create partition input spool",
                Some(spool.display().to_string()),
                error.to_string(),
            )
        })?;
        // The max-size partition sink is the streaming row-group iterator:
        // it emits bounded Parquet files in one source pass. Reading those
        // files independently avoids offset-slicing one growing Parquet file,
        // which would otherwise rescan preceding row-group metadata per batch.
        df.clone()
            .sink_parquet_partitioned(
                std::sync::Arc::new(spool.clone()),
                None,
                PartitionVariant::MaxSize(PARTITION_BATCH_ROWS as IdxSize),
                ParquetWriteOptions::default(),
                None,
                SinkOptions::default(),
            )
            .and_then(|frame| frame.collect_with_engine(Engine::Streaming))
            .map_err(|error| {
                QuiltError::operation("partition", format!("failed to evaluate input: {error}"))
            })?;

        let partition_names = discover_partition_names(&spool, colname)?;
        write_partition_batches(&spool, colname, &partition_names, &mut writers, metrics)?;
        let names = writers.finish()?;
        fs::remove_dir_all(&spool).map_err(|error| {
            QuiltError::io(
                "remove partition input spool",
                Some(spool.display().to_string()),
                error.to_string(),
            )
        })?;
        publish(&stage, &output_path)?;
        Ok(names
            .into_iter()
            .map(|name| output_path.join(name))
            .collect())
    })();
    drop(writers);
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result.map(FinalizerResult::Files)
}

fn for_each_partition_batch<F>(spool: &Path, mut visit: F) -> Result<(), QuiltError>
where
    F: FnMut(DataFrame) -> Result<(), QuiltError>,
{
    let mut batches = fs::read_dir(spool)
        .map_err(|error| {
            QuiltError::io(
                "read partition input spool",
                Some(spool.display().to_string()),
                error.to_string(),
            )
        })?
        .map(|entry| {
            entry.map(|entry| entry.path()).map_err(|error| {
                QuiltError::io(
                    "read partition input spool entry",
                    Some(spool.display().to_string()),
                    error.to_string(),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    batches.sort();
    for batch_path in batches {
        let reader = ParquetReader::new(File::open(&batch_path).map_err(|error| {
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
    spool: &Path,
    colname: &str,
) -> Result<HashMap<String, String>, QuiltError> {
    let mut keys = BTreeMap::<String, String>::new();
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
            let raw = partition_value(value.clone());
            keys.entry(partition_identity(value)).or_insert(raw);
        }
        Ok(())
    })?;

    let mut sorted_keys = keys.into_iter().collect::<Vec<_>>();
    sorted_keys.sort_by(|left, right| left.1.cmp(&right.1).then(left.0.cmp(&right.0)));
    let mut used = HashMap::<String, usize>::new();
    let mut names = HashMap::with_capacity(sorted_keys.len());
    for (identity, raw) in sorted_keys {
        let base = sanitize_filename(&raw);
        let count = used.entry(base.clone()).or_insert(0);
        *count += 1;
        names.insert(identity, bounded_partition_name(&raw, &base, *count));
    }
    Ok(names)
}

fn write_partition_batches(
    spool: &Path,
    colname: &str,
    partition_names: &HashMap<String, String>,
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
            rows_by_partition
                .entry(partition_identity(value))
                .or_default()
                .push(row as IdxSize);
        }

        for (identity, rows) in rows_by_partition {
            let name = partition_names.get(&identity).ok_or_else(|| {
                QuiltError::operation(
                    "partition",
                    "partition key disappeared between discovery and writing",
                )
            })?;
            let mut group = batch
                .take(&IdxCa::from_vec(PlSmallStr::EMPTY, rows))
                .map_err(|error| {
                    QuiltError::operation(
                        "partition",
                        format!("failed to gather partition rows: {error}"),
                    )
                })?;
            writers.write_batch(name, &mut group)?;
        }
        Ok(())
    });
    if let Some(metrics) = metrics {
        metrics.max_open_writers = metrics.max_open_writers.max(writers.peak_open);
    }
    result
}

fn partition_identity(value: AnyValue<'_>) -> String {
    format!("{}\0{}", value.dtype(), partition_value(value))
}

struct PartitionWriterPool<'a> {
    stage: PathBuf,
    writers: HashMap<String, File>,
    order: VecDeque<String>,
    header_written: HashSet<String>,
    names: std::collections::BTreeSet<String>,
    peak_open: usize,
    before_write: Option<&'a mut dyn FnMut(usize) -> Result<(), QuiltError>>,
}

impl<'a> PartitionWriterPool<'a> {
    fn new(
        stage: &Path,
        before_write: Option<&'a mut dyn FnMut(usize) -> Result<(), QuiltError>>,
    ) -> Self {
        Self {
            stage: stage.to_path_buf(),
            writers: HashMap::new(),
            order: VecDeque::new(),
            header_written: HashSet::new(),
            names: std::collections::BTreeSet::new(),
            peak_open: 0,
            before_write,
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
    CsvWriter::new(writer)
        .include_header(include_header)
        .finish(group)
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
    matches!(
        value.to_ascii_uppercase().as_str(),
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
    fn concurrent_publishers_have_one_winner() {
        let parent = test_parent("concurrent");
        let target = parent.join("partitions");
        let barrier = Arc::new(Barrier::new(2));
        let handles = (0..2)
            .map(|_| {
                let target = target.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let frame = df!("part" => &["a", "b"], "value" => &[1i64, 2])
                        .unwrap()
                        .lazy();
                    partition_with_publisher(
                        &frame,
                        "part",
                        target.to_str().unwrap(),
                        move |stage, target| {
                            barrier.wait();
                            let parent = target.parent().unwrap_or_else(|| Path::new("."));
                            publish_directory_and_sync(
                                &NativePublicationBackend,
                                stage,
                                target,
                                parent,
                                "publish partition directory",
                            )
                        },
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
        assert_eq!(fs::read_dir(&target).unwrap().count(), 2);
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn observer_sees_no_partial_partition_directory() {
        let parent = test_parent("observer");
        let target = parent.join("partitions");
        let started = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let invalid = Arc::new(AtomicBool::new(false));
        let observer_target = target.clone();
        let observer_finished = Arc::clone(&finished);
        let observer_invalid = Arc::clone(&invalid);
        let observer = thread::spawn(move || {
            while !observer_finished.load(Ordering::Acquire) {
                if observer_target.exists() {
                    match fs::read_dir(&observer_target) {
                        Ok(entries) => {
                            if entries.count() != 3 {
                                observer_invalid.store(true, Ordering::Release);
                                break;
                            }
                        }
                        Err(_) => {
                            observer_invalid.store(true, Ordering::Release);
                            break;
                        }
                    }
                }
                thread::yield_now();
            }
        });
        let publish_started = Arc::clone(&started);
        let publish_finished = Arc::clone(&finished);
        let frame = df!(
            "part" => &["a", "b", "c"],
            "value" => &[1i64, 2, 3]
        )
        .unwrap()
        .lazy();
        let result = partition_with_publisher(
            &frame,
            "part",
            target.to_str().unwrap(),
            move |stage, target| {
                publish_started.store(true, Ordering::Release);
                thread::sleep(Duration::from_millis(25));
                let parent = target.parent().unwrap_or_else(|| Path::new("."));
                let result = publish_directory_and_sync(
                    &NativePublicationBackend,
                    stage,
                    target,
                    parent,
                    "publish partition directory",
                );
                publish_finished.store(true, Ordering::Release);
                result
            },
        );
        assert!(result.is_ok());
        assert!(started.load(Ordering::Acquire));
        observer.join().unwrap();
        assert!(!invalid.load(Ordering::Acquire));
        assert_eq!(fs::read_dir(&target).unwrap().count(), 3);
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
        let mut writers = PartitionWriterPool::new(&stage, None);
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
        let mut writers = PartitionWriterPool::new(&stage, None);
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
    fn large_partition_is_batched_and_caps_open_writers_end_to_end() {
        let parent = test_parent("large-bounded");
        let target = parent.join("partitions");
        let partition_count = MAX_OPEN_PARTITION_WRITERS + 3;
        let row_count = PARTITION_BATCH_ROWS * 3 + 17;
        let parts = (0..row_count)
            .map(|row| format!("part-{}", row % partition_count))
            .collect::<Vec<_>>();
        let values = (0..row_count as i64).collect::<Vec<_>>();
        let frame = DataFrame::new(vec![
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
