use std::fs::{remove_file, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Execution-owned artifacts shared by cloned lazy plans.
#[derive(Clone)]
pub struct ExecutionResources {
    registry: Arc<Mutex<Vec<Arc<ManagedTempFile>>>>,
    directory: PathBuf,
    allow_temp_files: bool,
    #[cfg(test)]
    evaluation_probe: Option<Arc<AtomicUsize>>,
}

impl Default for ExecutionResources {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutionResources {
    pub fn new() -> Self {
        Self::new_in(std::env::temp_dir())
    }

    pub fn new_in(directory: PathBuf) -> Self {
        Self {
            registry: Arc::new(Mutex::new(Vec::new())),
            directory,
            allow_temp_files: true,
            #[cfg(test)]
            evaluation_probe: None,
        }
    }

    pub fn new_plan() -> Self {
        let mut resources = Self::new();
        resources.allow_temp_files = false;
        resources
    }

    pub(crate) fn temp_files_enabled(&self) -> bool {
        self.allow_temp_files
    }

    /// Attach a test-scoped probe to source plans created with these resources.
    ///
    /// The probe is deliberately owned by the execution resources, so parallel
    /// tests do not share a process-global counter. It counts evaluation-probe
    /// callbacks, not operating-system reads or physical scan batches.
    #[cfg(test)]
    pub(crate) fn with_evaluation_probe(mut self, probe: Arc<AtomicUsize>) -> Self {
        self.evaluation_probe = Some(probe);
        self
    }

    #[cfg(test)]
    pub(crate) fn evaluation_probe(&self) -> Option<Arc<AtomicUsize>> {
        self.evaluation_probe.clone()
    }

    pub fn reserve_temp_file(
        &self,
        prefix: &str,
        extension: &str,
    ) -> io::Result<TempFileReservation> {
        if !self.allow_temp_files {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "temporary resources are disabled for plan inspection",
            ));
        }
        let directory = &self.directory;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        for attempt in 0..128u32 {
            let path = directory.join(format!(
                ".{prefix}-{}-{stamp}-{attempt}.{extension}",
                std::process::id()
            ));
            let mut options = OpenOptions::new();
            options.write(true).read(true).create_new(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            match options.open(&path) {
                Ok(file) => {
                    return Ok(TempFileReservation {
                        file: Some(file),
                        path,
                    })
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not reserve a unique temporary resource",
        ))
    }

    pub fn retain_temp_file(&self, reservation: TempFileReservation) -> io::Result<()> {
        let managed = Arc::new(reservation.commit());
        self.registry
            .lock()
            .map_err(|_| io::Error::other("execution resource registry is poisoned"))?
            .push(managed);
        Ok(())
    }

    #[cfg(test)]
    pub fn tracked_count(&self) -> usize {
        self.registry
            .lock()
            .map(|resources| resources.len())
            .unwrap_or(0)
    }

    #[cfg(test)]
    pub fn tracked_paths(&self) -> Vec<PathBuf> {
        self.registry
            .lock()
            .map(|resources| {
                resources
                    .iter()
                    .map(|resource| resource.path.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Add a test-scoped evaluation probe to a lazy source plan. The callback
/// count describes probe invocations, rather than OS reads or scan batches.
#[cfg(test)]
pub(crate) fn instrument_evaluation(
    frame: polars::prelude::LazyFrame,
    resources: &ExecutionResources,
) -> polars::prelude::LazyFrame {
    let Some(probe) = resources.evaluation_probe() else {
        return frame;
    };
    frame.map(
        move |frame| {
            probe.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(frame)
        },
        polars::prelude::AllowedOptimizations::default(),
        None,
        Some("qlt_test_evaluation_probe"),
    )
}

#[cfg(not(test))]
pub(crate) fn instrument_evaluation(
    frame: polars::prelude::LazyFrame,
    _resources: &ExecutionResources,
) -> polars::prelude::LazyFrame {
    frame
}

pub struct TempFileReservation {
    file: Option<File>,
    path: PathBuf,
}

impl TempFileReservation {
    pub fn file_mut(&mut self) -> Option<&mut File> {
        self.file.as_mut()
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn close_file(&mut self) {
        drop(self.file.take());
    }
    fn commit(mut self) -> ManagedTempFile {
        drop(self.file.take());
        ManagedTempFile {
            path: std::mem::take(&mut self.path),
        }
    }
}

impl Drop for TempFileReservation {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = remove_file(&self.path);
    }
}

struct ManagedTempFile {
    path: PathBuf,
}

impl Drop for ManagedTempFile {
    fn drop(&mut self) {
        let _ = remove_file(&self.path);
    }
}

pub fn path_exists(path: &Path) -> bool {
    path.exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reservation_is_private_and_cleaned_after_registry_drop() {
        let resources = ExecutionResources::new();
        let (path, clone) = {
            let mut reservation = resources.reserve_temp_file("qlt-test", "tmp").unwrap();
            let path = reservation.path().to_path_buf();
            reservation.file_mut().unwrap().write_all(b"test").unwrap();
            resources.retain_temp_file(reservation).unwrap();
            assert!(path_exists(&path));
            (path, resources.clone())
        };
        assert_eq!(clone.tracked_count(), 1);
        drop(resources);
        assert!(path_exists(&path));
        drop(clone);
        assert!(!path_exists(&path));
    }

    #[test]
    fn abandoned_reservation_is_cleaned_immediately() {
        let reservation = ExecutionResources::new()
            .reserve_temp_file("qlt-test-abandoned", "tmp")
            .unwrap();
        let path = reservation.path().to_path_buf();
        assert!(path_exists(&path));
        drop(reservation);
        assert!(!path_exists(&path));
    }

    #[test]
    fn plan_resources_reject_reservations() {
        let resources = ExecutionResources::new_plan();
        assert!(resources.reserve_temp_file("plan", "tmp").is_err());
        assert_eq!(resources.tracked_count(), 0);
    }
}
