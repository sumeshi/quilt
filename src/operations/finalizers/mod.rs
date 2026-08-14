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

#[cfg(any(target_os = "linux", target_os = "android"))]
use rustix::io::Errno;

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
fn sync_parent_directory(parent: &Path, operation: &str) -> Result<(), QuiltError> {
    let directory = File::open(parent).map_err(|error| {
        QuiltError::io(
            operation,
            Some(parent.display().to_string()),
            error.to_string(),
        )
    })?;
    directory.sync_all().map_err(|error| {
        QuiltError::io(
            operation,
            Some(parent.display().to_string()),
            error.to_string(),
        )
    })
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path, _operation: &str) -> Result<(), QuiltError> {
    Ok(())
}

/// The normalized result shapes produced by finalizers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalizerResult {
    Stdout(String),
    Stderr(String),
    Scalar(String),
    File(PathBuf),
    Files(Vec<PathBuf>),
    PlanTable(String),
    Artifact(std::sync::Arc<OutputArtifact>),
}

#[derive(Clone)]
pub struct OutputArtifact {
    path: PathBuf,
    _resources: crate::controllers::resources::ExecutionResources,
}

impl std::fmt::Debug for OutputArtifact {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OutputArtifact")
            .field("path", &self.path)
            .finish()
    }
}

impl OutputArtifact {
    pub(crate) fn new(
        path: PathBuf,
        resources: crate::controllers::resources::ExecutionResources,
    ) -> Self {
        Self {
            path,
            _resources: resources,
        }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl PartialEq for OutputArtifact {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}
impl Eq for OutputArtifact {}

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
                let publish = publish_file_and_sync(
                    &NativePublicationBackend,
                    &temp,
                    target,
                    parent,
                    operation,
                );
                if publish.is_ok() {
                    let _ = fs::remove_file(&temp);
                }
                publish
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
        publish_file_and_sync(&NativePublicationBackend, &temp, target, parent, operation)?;
        let _ = fs::remove_file(&temp);
        Ok(())
    });
    result
}

/// Publish a staged file without replacing a target created concurrently.
/// Linux uses `renameat2(RENAME_NOREPLACE)` through rustix. On filesystems
/// lacking that primitive, the hard-link fallback is also no-clobber; it is
/// deliberately never a check-then-rename sequence.
#[cfg(windows)]
pub(crate) fn publish_file_noreplace(
    stage: &Path,
    target: &Path,
    operation: &str,
) -> Result<(), QuiltError> {
    windows_publish_noreplace(stage, target, operation)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
pub(crate) fn publish_file_noreplace(
    stage: &Path,
    target: &Path,
    operation: &str,
) -> Result<(), QuiltError> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        match rustix::fs::renameat_with(
            rustix::fs::CWD,
            stage,
            rustix::fs::CWD,
            target,
            rustix::fs::RenameFlags::NOREPLACE,
        ) {
            Ok(()) => return Ok(()),
            Err(Errno::OPNOTSUPP | Errno::NOSYS | Errno::INVAL) => {}
            Err(error) => {
                return Err(QuiltError::io(
                    operation,
                    Some(target.display().to_string()),
                    format!("atomic no-replace publish failed: {error}"),
                ));
            }
        }
    }
    fs::hard_link(stage, target).map_err(|error| {
        QuiltError::io(
            operation,
            Some(target.display().to_string()),
            format!("no-clobber fallback publish failed: {error}"),
        )
    })
}

#[cfg(not(any(windows, target_os = "linux", target_os = "android")))]
pub(crate) fn publish_file_noreplace(
    stage: &Path,
    target: &Path,
    operation: &str,
) -> Result<(), QuiltError> {
    fs::hard_link(stage, target).map_err(|error| {
        QuiltError::io(
            operation,
            Some(target.display().to_string()),
            format!("no-clobber fallback publish failed: {error}"),
        )
    })
}

#[cfg(windows)]
fn windows_publish_noreplace(
    stage: &Path,
    target: &Path,
    operation: &str,
) -> Result<(), QuiltError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
    let source = stage
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(QuiltError::io(
            operation,
            Some(target.display().to_string()),
            std::io::Error::last_os_error().to_string(),
        ));
    }
    Ok(())
}

/// Publish a staged directory without replacement. Directory publication has
/// no portable hard-link fallback; unsupported platforms return an actionable
/// error instead of using an overwriting rename.
#[cfg(windows)]
pub(crate) fn publish_directory_noreplace(
    stage: &Path,
    target: &Path,
    operation: &str,
) -> Result<(), QuiltError> {
    windows_publish_noreplace(stage, target, operation)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
pub(crate) fn publish_directory_noreplace(
    stage: &Path,
    target: &Path,
    operation: &str,
) -> Result<(), QuiltError> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        stage,
        rustix::fs::CWD,
        target,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(|error| {
        QuiltError::io(
            operation,
            Some(target.display().to_string()),
            format!("atomic no-replace directory publish failed: {error}"),
        )
    })
}

#[cfg(not(any(windows, target_os = "linux", target_os = "android")))]
pub(crate) fn publish_directory_noreplace(
    _stage: &Path,
    _target: &Path,
    operation: &str,
) -> Result<(), QuiltError> {
    Err(QuiltError::operation(
        operation,
        "atomic no-replace directory publication is unsupported on this platform/filesystem",
    ))
}

pub(crate) fn sync_published_parent(parent: &Path, operation: &str) -> Result<(), QuiltError> {
    sync_parent_directory(parent, operation).map_err(|error| match error {
        QuiltError::Io { path, message, .. } => QuiltError::io(
            operation,
            path,
            format!("publication completed but durability sync failed: {message}"),
        ),
        other => other,
    })
}

/// Injectable publication seam used by finalizers and deterministic tests.
pub(crate) trait PublicationBackend {
    fn publish_file(&self, stage: &Path, target: &Path, operation: &str) -> Result<(), QuiltError>;
    fn publish_directory(
        &self,
        stage: &Path,
        target: &Path,
        operation: &str,
    ) -> Result<(), QuiltError>;
    fn sync_parent(&self, parent: &Path, operation: &str) -> Result<(), QuiltError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct NativePublicationBackend;

impl PublicationBackend for NativePublicationBackend {
    fn publish_file(&self, stage: &Path, target: &Path, operation: &str) -> Result<(), QuiltError> {
        publish_file_noreplace(stage, target, operation)
    }

    fn publish_directory(
        &self,
        stage: &Path,
        target: &Path,
        operation: &str,
    ) -> Result<(), QuiltError> {
        publish_directory_noreplace(stage, target, operation)
    }

    fn sync_parent(&self, parent: &Path, operation: &str) -> Result<(), QuiltError> {
        sync_published_parent(parent, operation)
    }
}

pub(crate) fn publish_file_and_sync<B: PublicationBackend>(
    backend: &B,
    stage: &Path,
    target: &Path,
    parent: &Path,
    operation: &str,
) -> Result<(), QuiltError> {
    backend.publish_file(stage, target, operation)?;
    backend.sync_parent(parent, operation)
}

pub(crate) fn publish_directory_and_sync<B: PublicationBackend>(
    backend: &B,
    stage: &Path,
    target: &Path,
    parent: &Path,
    operation: &str,
) -> Result<(), QuiltError> {
    backend.publish_directory(stage, target, operation)?;
    backend.sync_parent(parent, operation)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteStatus {
    Complete,
    BrokenPipe,
}

pub fn write_stdout<W: Write>(
    result: &FinalizerResult,
    mut writer: W,
) -> Result<WriteStatus, QuiltError> {
    fn write_bytes<W: Write>(
        writer: &mut W,
        bytes: &[u8],
        operation: &str,
    ) -> Result<WriteStatus, QuiltError> {
        match writer.write_all(bytes) {
            Ok(()) => Ok(WriteStatus::Complete),
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {
                Ok(WriteStatus::BrokenPipe)
            }
            Err(error) => Err(QuiltError::io(operation, None::<String>, error.to_string())),
        }
    }
    match result {
        FinalizerResult::Stdout(text)
        | FinalizerResult::Stderr(text)
        | FinalizerResult::PlanTable(text) => {
            return write_bytes(&mut writer, text.as_bytes(), "write finalizer output");
        }
        FinalizerResult::Scalar(value) => {
            return write_bytes(
                &mut writer,
                format!("{value}\n").as_bytes(),
                "write finalizer scalar",
            );
        }
        FinalizerResult::File(_) | FinalizerResult::Files(_) => {}
        FinalizerResult::Artifact(artifact) => {
            let mut file = File::open(artifact.path()).map_err(|e| {
                QuiltError::io(
                    "read finalizer artifact",
                    Some(artifact.path().display().to_string()),
                    e.to_string(),
                )
            })?;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let read = std::io::Read::read(&mut file, &mut buffer).map_err(|e| {
                    QuiltError::io(
                        "read finalizer artifact",
                        Some(artifact.path().display().to_string()),
                        e.to_string(),
                    )
                })?;
                if read == 0 {
                    break;
                }
                if write_bytes(&mut writer, &buffer[..read], "write finalizer output")?
                    == WriteStatus::BrokenPipe
                {
                    return Ok(WriteStatus::BrokenPipe);
                }
            }
        }
    }
    Ok(WriteStatus::Complete)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controllers::resources::ExecutionResources;
    use polars::prelude::*;
    use std::io;

    struct BoundedWriter {
        max: usize,
        total: usize,
    }
    impl Write for BoundedWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.max = self.max.max(bytes.len());
            self.total += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct FailingWriter;
    impl Write for FailingWriter {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("injected writer failure"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct BrokenPipeWriter;
    impl Write for BrokenPipeWriter {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct CountingBrokenPipe {
        writes: usize,
    }
    impl Write for CountingBrokenPipe {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn show_artifact_copies_in_bounded_chunks_and_cleans_with_owner() {
        let resources = ExecutionResources::new();
        let frame = df!("value" => &["x".repeat(200_000)]).unwrap().lazy();
        let result = crate::operations::finalizers::show::show(&frame, &resources).unwrap();
        let artifact_path = match &result {
            FinalizerResult::Artifact(artifact) => artifact.path().to_path_buf(),
            other => panic!("unexpected result: {other:?}"),
        };
        assert!(artifact_path.exists());
        let mut writer = BoundedWriter { max: 0, total: 0 };
        write_stdout(&result, &mut writer).unwrap();
        assert!(writer.total > 200_000);
        assert!(writer.max <= 64 * 1024);
        drop(resources);
        assert!(artifact_path.exists());
        drop(result);
        assert!(!artifact_path.exists());
    }

    #[test]
    fn show_evaluation_failure_and_writer_failures_are_reported() {
        let resources = ExecutionResources::new();
        let bad = df!("value" => &["not-an-int"])
            .unwrap()
            .lazy()
            .select([col("missing")]);
        assert!(crate::operations::finalizers::show::show(&bad, &resources).is_err());
        let text = FinalizerResult::Stdout("value\n".into());
        assert!(write_stdout(&text, FailingWriter).is_err());
        assert_eq!(
            write_stdout(&text, BrokenPipeWriter).unwrap(),
            WriteStatus::BrokenPipe
        );
        let large = crate::operations::finalizers::show::show(
            &df!("value" => &["x".repeat(200_000)]).unwrap().lazy(),
            &resources,
        )
        .unwrap();
        let mut writer = CountingBrokenPipe { writes: 0 };
        assert_eq!(
            write_stdout(&large, &mut writer).unwrap(),
            WriteStatus::BrokenPipe
        );
        assert_eq!(writer.writes, 1);

        let empty = df!("value" => Vec::<String>::new()).unwrap().lazy();
        let empty_result = crate::operations::finalizers::show::show(&empty, &resources).unwrap();
        let mut empty_output = Vec::new();
        write_stdout(&empty_result, &mut empty_output).unwrap();
        assert_eq!(empty_output, b"value\n");
    }

    #[test]
    fn show_preserves_csv_quoting_and_null_bytes() {
        let frame = DataFrame::new(vec![Series::new(
            "value".into(),
            &[Some("a,b"), Some("quote\"x"), Some("line\nx"), None],
        )
        .into()])
        .unwrap()
        .lazy();
        let resources = ExecutionResources::new();
        let result = crate::operations::finalizers::show::show(&frame, &resources).unwrap();
        let mut output = Vec::new();
        write_stdout(&result, &mut output).unwrap();
        assert_eq!(output, b"value\n\"a,b\"\n\"quote\"\"x\"\n\"line\nx\"\n\n");
    }

    #[test]
    fn no_replace_file_publisher_preserves_existing_target() {
        let root = std::env::temp_dir().join(format!(
            "qlt-publish-noreplace-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let staged = root.join("staged");
        let target = root.join("target");
        std::fs::write(&staged, b"new").unwrap();
        std::fs::write(&target, b"old").unwrap();
        let error = publish_file_noreplace(&staged, &target, "test publication").unwrap_err();
        assert!(error.to_string().contains("target") || error.to_string().contains("exists"));
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn injected_publication_backend_covers_native_fallback_errors_and_sync() {
        #[derive(Clone, Copy)]
        enum FailureKind {
            UnsupportedPrimitiveFallback,
            FallbackFailure,
            PermissionDenied,
            CrossDevice,
            DirectorySuccess,
            DirectorySyncFailure,
        }

        struct Backend {
            kind: FailureKind,
        }

        impl Backend {
            fn io_failure(&self, operation: &str, target: &Path, message: &str) -> QuiltError {
                QuiltError::io(operation, Some(target.display().to_string()), message)
            }
        }

        impl PublicationBackend for Backend {
            fn publish_file(
                &self,
                stage: &Path,
                target: &Path,
                operation: &str,
            ) -> Result<(), QuiltError> {
                match self.kind {
                    FailureKind::UnsupportedPrimitiveFallback => std::fs::hard_link(stage, target)
                        .map_err(|error| self.io_failure(operation, target, &error.to_string())),
                    FailureKind::FallbackFailure => Err(self.io_failure(
                        operation,
                        target,
                        "unsupported native primitive; no-clobber fallback failed",
                    )),
                    FailureKind::PermissionDenied => Err(self.io_failure(
                        operation,
                        target,
                        "permission denied while publishing",
                    )),
                    FailureKind::CrossDevice => Err(self.io_failure(
                        operation,
                        target,
                        "cross-device publication is not supported",
                    )),
                    FailureKind::DirectorySuccess | FailureKind::DirectorySyncFailure => {
                        std::fs::rename(stage, target)
                            .map_err(|error| self.io_failure(operation, target, &error.to_string()))
                    }
                }
            }

            fn publish_directory(
                &self,
                stage: &Path,
                target: &Path,
                operation: &str,
            ) -> Result<(), QuiltError> {
                match self.kind {
                    FailureKind::DirectorySuccess | FailureKind::DirectorySyncFailure => {
                        std::fs::rename(stage, target)
                            .map_err(|error| self.io_failure(operation, target, &error.to_string()))
                    }
                    _ => Err(QuiltError::operation(
                        operation,
                        "injected directory publication failure",
                    )),
                }
            }

            fn sync_parent(&self, parent: &Path, operation: &str) -> Result<(), QuiltError> {
                if matches!(self.kind, FailureKind::DirectorySyncFailure) {
                    return Err(QuiltError::io(
                        operation,
                        Some(parent.display().to_string()),
                        "publication completed but durability sync failed",
                    ));
                }
                Ok(())
            }
        }

        let root = std::env::temp_dir().join(format!(
            "qlt-backend-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();

        let stage = root.join("stage-fallback");
        let target = root.join("target-fallback");
        std::fs::write(&stage, b"payload").unwrap();
        publish_file_and_sync(
            &Backend {
                kind: FailureKind::UnsupportedPrimitiveFallback,
            },
            &stage,
            &target,
            &root,
            "test backend",
        )
        .unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"payload");
        assert!(
            stage.exists(),
            "hard-link fallback must retain the staged inode"
        );

        for (suffix, kind, expected) in [
            (
                "fallback-failure",
                FailureKind::FallbackFailure,
                "fallback failed",
            ),
            (
                "permission-denied",
                FailureKind::PermissionDenied,
                "permission denied",
            ),
            ("cross-device", FailureKind::CrossDevice, "cross-device"),
        ] {
            let stage = root.join(format!("stage-{suffix}"));
            let target = root.join(format!("target-{suffix}"));
            std::fs::write(&stage, b"payload").unwrap();
            let error =
                publish_file_and_sync(&Backend { kind }, &stage, &target, &root, "test backend")
                    .unwrap_err();
            assert!(error.to_string().contains(expected));
            assert!(error.to_string().contains(&target.display().to_string()));
            assert!(!target.exists());
            assert!(stage.exists());
        }

        let stage = root.join("stage-sync");
        let sync_target = root.join("target-sync");
        std::fs::write(&stage, b"published").unwrap();
        let error = publish_file_and_sync(
            &Backend {
                kind: FailureKind::DirectorySyncFailure,
            },
            &stage,
            &sync_target,
            &root,
            "test backend",
        )
        .unwrap_err();
        assert!(error.to_string().contains("publication completed"));
        assert_eq!(std::fs::read(&sync_target).unwrap(), b"published");
        assert!(!stage.exists(), "native rename consumes the staged source");

        let stage = root.join("stage-directory");
        let target = root.join("target-directory");
        std::fs::create_dir(&stage).unwrap();
        std::fs::write(stage.join("part.csv"), b"part").unwrap();
        publish_directory_and_sync(
            &Backend {
                kind: FailureKind::DirectorySuccess,
            },
            &stage,
            &target,
            &root,
            "test directory backend",
        )
        .unwrap();
        assert!(target.join("part.csv").exists());
        assert!(!stage.exists());

        let stage = root.join("stage-directory-sync");
        let target = root.join("target-directory-sync");
        std::fs::create_dir(&stage).unwrap();
        std::fs::write(stage.join("part.csv"), b"part").unwrap();
        let error = publish_directory_and_sync(
            &Backend {
                kind: FailureKind::DirectorySyncFailure,
            },
            &stage,
            &target,
            &root,
            "test directory backend",
        )
        .unwrap_err();
        assert!(error.to_string().contains("publication completed"));
        assert!(target.join("part.csv").exists());
        assert!(!stage.exists());

        std::fs::remove_dir_all(root).unwrap();
    }
}
