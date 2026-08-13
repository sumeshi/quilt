pub mod calc;
pub mod dump;
pub mod dumpcache;
pub mod headers;
pub mod partition;
pub mod show;
pub mod showquery;
pub mod showtable;
pub mod stats;

use crate::error::QuiltError;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static ATOMIC_STAGE_COUNTER: AtomicU64 = AtomicU64::new(0);

struct AtomicStage {
    path: PathBuf,
}

impl AtomicStage {
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for AtomicStage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn reserve_atomic_stage(
    parent: &Path,
    file_name: &str,
    operation: &str,
) -> Result<AtomicStage, QuiltError> {
    for _ in 0..64 {
        let nonce = ATOMIC_STAGE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(
            ".{file_name}.qlt-stage-{}-{nonce}",
            std::process::id()
        ));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(AtomicStage { path: candidate }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(QuiltError::io(
                    operation,
                    Some(candidate.display().to_string()),
                    error.to_string(),
                ));
            }
        }
    }
    Err(QuiltError::io(
        operation,
        Some(parent.display().to_string()),
        "could not reserve unique staging directory",
    ))
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) {
    if let Ok(directory) = File::open(parent) {
        let _ = directory.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) {}

/// The normalized result shapes produced by finalizers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalizerResult {
    Stdout(String),
    Stderr(String),
    Scalar(String),
    File(PathBuf),
    Files(Vec<PathBuf>),
    PlanTable(String),
}

/// Write a file through a same-filesystem temporary sibling and publish it
/// with a rename. Existing targets are rejected deliberately: callers that
/// want replacement must choose a new destination (or remove the old one).
/// The closure is also a small failure-injection seam for unit tests.
pub fn atomic_write<F>(target: &Path, operation: &str, write: F) -> Result<(), QuiltError>
where
    F: FnOnce(&mut File) -> Result<(), QuiltError>,
{
    validate_destination(target, operation)?;
    let parent = target
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err(QuiltError::io(
            operation,
            Some(parent.display().to_string()),
            "destination directory does not exist",
        ));
    }
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("output");
    let stage = reserve_atomic_stage(parent, file_name, operation)?;
    let temp = stage.path().join("payload");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|error| {
            QuiltError::io(
                operation,
                Some(temp.display().to_string()),
                error.to_string(),
            )
        })?;
    let result = match write(&mut file) {
        Err(error) => {
            drop(file);
            Err(error)
        }
        Ok(()) => match file.sync_all() {
            Err(error) => {
                drop(file);
                Err(QuiltError::io(
                    operation,
                    Some(temp.display().to_string()),
                    error.to_string(),
                ))
            }
            Ok(()) => {
                drop(file);
                // A hard link is a same-filesystem no-clobber publish: unlike
                // rename, it cannot replace a target created after the
                // preflight check.
                let publish = fs::hard_link(&temp, target).map_err(|error| {
                    QuiltError::io(
                        operation,
                        Some(target.display().to_string()),
                        error.to_string(),
                    )
                });
                if publish.is_ok() {
                    let _ = fs::remove_file(&temp);
                    sync_parent_directory(parent);
                }
                publish.map(|_| ())
            }
        },
    };
    result
}

pub fn atomic_path<F>(target: &Path, operation: &str, write: F) -> Result<(), QuiltError>
where
    F: FnOnce(&Path) -> Result<(), QuiltError>,
{
    validate_destination(target, operation)?;
    let parent = target
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("output");
    let stage = reserve_atomic_stage(parent, file_name, operation)?;
    let temp = stage.path().join("payload");
    let result = write(&temp).and_then(|_| {
        let payload = File::open(&temp).map_err(|error| {
            QuiltError::io(
                operation,
                Some(temp.display().to_string()),
                error.to_string(),
            )
        })?;
        payload.sync_all().map_err(|error| {
            QuiltError::io(
                operation,
                Some(temp.display().to_string()),
                error.to_string(),
            )
        })?;
        drop(payload);
        fs::hard_link(&temp, target).map_err(|error| {
            QuiltError::io(
                operation,
                Some(target.display().to_string()),
                error.to_string(),
            )
        })?;
        let _ = fs::remove_file(&temp);
        sync_parent_directory(parent);
        Ok(())
    });
    result
}

pub fn validate_destination(target: &Path, operation: &str) -> Result<(), QuiltError> {
    if target.exists() {
        return Err(QuiltError::io(
            operation,
            Some(target.display().to_string()),
            "destination already exists; refusing to overwrite",
        ));
    }
    let parent = target
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err(QuiltError::io(
            operation,
            Some(parent.display().to_string()),
            "destination directory does not exist",
        ));
    }
    Ok(())
}

pub fn io_error(operation: &str, path: Option<&Path>, message: impl Into<String>) -> QuiltError {
    QuiltError::io(
        operation,
        path.map(|value| value.display().to_string()),
        message,
    )
}

pub fn write_stdout<W: Write>(result: &FinalizerResult, mut writer: W) -> Result<(), QuiltError> {
    match result {
        FinalizerResult::Stdout(text)
        | FinalizerResult::Stderr(text)
        | FinalizerResult::PlanTable(text) => {
            writer
                .write_all(text.as_bytes())
                .map_err(|e| QuiltError::Io {
                    operation: "write finalizer output".into(),
                    path: None,
                    message: e.to_string(),
                })?;
        }
        FinalizerResult::Scalar(value) => {
            writeln!(writer, "{value}").map_err(|e| QuiltError::Io {
                operation: "write finalizer scalar".into(),
                path: None,
                message: e.to_string(),
            })?;
        }
        FinalizerResult::File(_) | FinalizerResult::Files(_) => {}
    }
    Ok(())
}

pub fn write_result(result: &FinalizerResult) -> Result<(), QuiltError> {
    match result {
        FinalizerResult::Stderr(_) => write_stdout(result, std::io::stderr()),
        _ => write_stdout(result, std::io::stdout()),
    }
}
