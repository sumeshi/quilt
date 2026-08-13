use crate::error::QuiltError;
use crate::operations::finalizers::{atomic_write, FinalizerResult};
use polars::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const NULL_PARTITION: &str = "_null";
const MAX_PARTITION_NAME_BYTES: usize = 120;
static PARTITION_STAGE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn partition(
    df: &LazyFrame,
    colname: &str,
    output_dir: &str,
) -> Result<FinalizerResult, QuiltError> {
    partition_with_publisher(df, colname, output_dir, publish_directory_noreplace)
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
    let result = (|| {
        let collected = df.clone().collect().map_err(|error| {
            QuiltError::operation("partition", format!("failed to evaluate input: {error}"))
        })?;
        let groups = collected
            .partition_by([colname], true)
            .map_err(|error| QuiltError::operation("partition", error.to_string()))?;
        let mut named_groups = groups
            .into_iter()
            .map(|group| {
                let value = group
                    .column(colname)
                    .and_then(|column| column.get(0))
                    .map_err(|error| QuiltError::operation("partition", error.to_string()))?;
                Ok((partition_value(value), group))
            })
            .collect::<Result<Vec<_>, QuiltError>>()?;
        named_groups.sort_by(|left, right| left.0.cmp(&right.0));

        let mut used = std::collections::HashMap::<String, usize>::new();
        let mut names = Vec::with_capacity(named_groups.len());
        for (raw_value, mut group) in named_groups {
            let base = sanitize_filename(&raw_value);
            let count = used.entry(base.clone()).or_insert(0);
            *count += 1;
            let name = bounded_partition_name(&raw_value, &base, *count);
            let path = stage.join(format!("{name}.csv"));
            atomic_write(&path, "write partition file", move |file| {
                CsvWriter::new(file)
                    .include_header(true)
                    .finish(&mut group)
                    .map(|_| ())
                    .map_err(|error| {
                        QuiltError::finalizer("write partition file", error.to_string())
                    })
            })?;
            names.push(format!("{name}.csv"));
        }
        publish(&stage, &output_path)?;
        Ok(names
            .into_iter()
            .map(|name| output_path.join(name))
            .collect::<Vec<_>>())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result.map(FinalizerResult::Files)
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

#[cfg(any(target_os = "linux", target_os = "android"))]
fn publish_directory_noreplace(stage: &Path, target: &Path) -> Result<(), QuiltError> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        stage,
        rustix::fs::CWD,
        target,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(|error| {
        QuiltError::io(
            "publish partition directory",
            Some(target.display().to_string()),
            format!("atomic no-replace publish failed: {error}"),
        )
    })
}

#[cfg(windows)]
fn publish_directory_noreplace(stage: &Path, target: &Path) -> Result<(), QuiltError> {
    fs::rename(stage, target).map_err(|error| {
        QuiltError::io(
            "publish partition directory",
            Some(target.display().to_string()),
            format!("atomic no-replace publish failed: {error}"),
        )
    })
}

#[cfg(not(any(target_os = "linux", target_os = "android", windows)))]
fn publish_directory_noreplace(_stage: &Path, _target: &Path) -> Result<(), QuiltError> {
    Err(QuiltError::operation(
        "publish partition directory",
        "atomic no-replace directory publication is unsupported on this target",
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
                            publish_directory_noreplace(stage, target)
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
                let result = publish_directory_noreplace(stage, target);
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
}
