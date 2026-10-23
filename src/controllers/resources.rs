use std::fs::{remove_file, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Execution-owned artifacts shared by cloned lazy plans.
#[derive(Clone)]
pub struct ExecutionResources {
    registry: Arc<Mutex<Vec<ManagedTempFile>>>,
    _dependencies: Arc<Vec<ExecutionResources>>,
    directory: PathBuf,
    allow_temp_files: bool,
    budget: Arc<TempBudget>,
    #[cfg(test)]
    evaluation_probe: Option<Arc<AtomicUsize>>,
}
struct TempBudget {
    limit: Result<Option<u64>, String>,
    used: std::sync::atomic::AtomicU64,
}
struct FileCharge {
    budget: Arc<TempBudget>,
    bytes: std::sync::atomic::AtomicU64,
}

impl FileCharge {
    fn write(&self, file: &mut File, bytes: &[u8]) -> io::Result<usize> {
        let requested = bytes.len() as u64;
        loop {
            let used = self.budget.used.load(std::sync::atomic::Ordering::SeqCst);
            let limit = self
                .budget
                .limit
                .as_ref()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.clone()))?;
            if let Some(max) = limit {
                if used.saturating_add(requested) > *max {
                    return Err(io::Error::other(format!(
                        "managed temporary data exceeds QLT_TEMP_LIMIT ({max} bytes)"
                    )));
                }
            }
            if self
                .budget
                .used
                .compare_exchange(
                    used,
                    used.saturating_add(requested),
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                )
                .is_ok()
            {
                break;
            }
        }
        match file.write(bytes) {
            Ok(written) => {
                self.bytes
                    .fetch_add(written as u64, std::sync::atomic::Ordering::SeqCst);
                if (written as u64) < requested {
                    self.budget.used.fetch_sub(
                        requested - written as u64,
                        std::sync::atomic::Ordering::SeqCst,
                    );
                }
                Ok(written)
            }
            Err(error) => {
                self.budget
                    .used
                    .fetch_sub(requested, std::sync::atomic::Ordering::SeqCst);
                Err(error)
            }
        }
    }
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
        let limit = match std::env::var("QLT_TEMP_LIMIT") {
            Ok(v) => match v.parse::<u64>() {
                Ok(n) if n > 0 => Ok(Some(n)),
                _ => Err("QLT_TEMP_LIMIT must be a positive integer number of bytes".into()),
            },
            Err(_) => Ok(None),
        };
        Self {
            registry: Arc::new(Mutex::new(Vec::new())),
            _dependencies: Arc::new(Vec::new()),
            directory,
            allow_temp_files: true,
            budget: Arc::new(TempBudget {
                limit,
                used: std::sync::atomic::AtomicU64::new(0),
            }),
            #[cfg(test)]
            evaluation_probe: None,
        }
    }

    /// Give a lazy stage its own artifacts while retaining the inputs it still scans.
    pub(crate) fn fork(&self, dependencies: Vec<Self>) -> Self {
        Self {
            registry: Arc::new(Mutex::new(Vec::new())),
            _dependencies: Arc::new(dependencies),
            directory: self.directory.clone(),
            allow_temp_files: self.allow_temp_files,
            budget: self.budget.clone(),
            #[cfg(test)]
            evaluation_probe: self.evaluation_probe.clone(),
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
    pub fn temp_usage(&self) -> (u64, Option<u64>) {
        (
            self.budget.used.load(std::sync::atomic::Ordering::SeqCst),
            self.budget.limit.as_ref().ok().and_then(|v| *v),
        )
    }
    #[cfg(test)]
    pub(crate) fn with_temp_limit_for_test(mut self, limit: u64) -> Self {
        self.budget = Arc::new(TempBudget {
            limit: Ok(Some(limit)),
            used: std::sync::atomic::AtomicU64::new(0),
        });
        self
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

    pub(crate) fn reserve_temp_file(
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
        if let Err(e) = &self.budget.limit {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, e.clone()));
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
                        charge: Arc::new(FileCharge {
                            budget: self.budget.clone(),
                            bytes: std::sync::atomic::AtomicU64::new(0),
                        }),
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

    pub(crate) fn retain_temp_file(&self, reservation: TempFileReservation) -> io::Result<()> {
        let managed = reservation.commit();
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

pub(crate) struct TempFileReservation {
    file: Option<File>,
    path: PathBuf,
    charge: Arc<FileCharge>,
}

impl TempFileReservation {
    pub fn file_mut(&mut self) -> Option<ReservationWriter<'_>> {
        Some(ReservationWriter {
            file: self.file.as_mut()?,
            charge: &self.charge,
        })
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
            charge: self.charge.clone(),
        }
    }
}
pub(crate) struct ReservationWriter<'a> {
    file: &'a mut File,
    charge: &'a FileCharge,
}
impl Write for ReservationWriter<'_> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.charge.write(self.file, b)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
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
    charge: Arc<FileCharge>,
}
impl Drop for FileCharge {
    fn drop(&mut self) {
        self.budget.used.fetch_sub(
            self.bytes.load(std::sync::atomic::Ordering::SeqCst),
            std::sync::atomic::Ordering::SeqCst,
        );
    }
}
pub(crate) struct AccountedFile {
    file: File,
    charge: Arc<FileCharge>,
}
impl AccountedFile {
    pub(crate) fn sync_all(&self) -> io::Result<()> {
        self.file.sync_all()
    }
}
impl Write for AccountedFile {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.charge.write(&mut self.file, b)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
impl polars::io::utils::file::WritableTrait for AccountedFile {
    fn close(&mut self) -> io::Result<()> {
        self.file.flush()
    }
    fn sync_all(&self) -> io::Result<()> {
        self.file.sync_all()
    }
    fn sync_data(&self) -> io::Result<()> {
        self.file.sync_data()
    }
}
impl ExecutionResources {
    pub(crate) fn accounted_file(&self, path: &Path, file: File) -> io::Result<AccountedFile> {
        let registry = self
            .registry
            .lock()
            .map_err(|_| io::Error::other("execution resource registry is poisoned"))?;
        let item = registry.iter().find(|r| r.path == path).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "temporary file is not managed")
        })?;
        Ok(AccountedFile {
            file,
            charge: item.charge.clone(),
        })
    }
    pub(crate) fn tracked_temp_file(&self, path: &Path, file: File) -> io::Result<AccountedFile> {
        let existing = {
            let registry = self
                .registry
                .lock()
                .map_err(|_| io::Error::other("execution resource registry is poisoned"))?;
            registry
                .iter()
                .find(|resource| resource.path == path)
                .map(|resource| resource.charge.clone())
        };
        if let Some(charge) = existing {
            return Ok(AccountedFile { file, charge });
        }
        self.adopt_temp_file(path, file)
    }
    pub(crate) fn adopt_temp_file(&self, path: &Path, file: File) -> io::Result<AccountedFile> {
        if let Err(e) = &self.budget.limit {
            drop(file);
            let _ = remove_file(path);
            return Err(io::Error::new(io::ErrorKind::InvalidInput, e.clone()));
        }
        let charge = Arc::new(FileCharge {
            budget: self.budget.clone(),
            bytes: std::sync::atomic::AtomicU64::new(0),
        });
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| io::Error::other("execution resource registry is poisoned"))?;
        if registry.iter().any(|resource| resource.path == path) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "temporary file is already managed",
            ));
        }
        registry.push(ManagedTempFile {
            path: path.to_path_buf(),
            charge: charge.clone(),
        });
        Ok(AccountedFile { file, charge })
    }
}

impl Drop for ManagedTempFile {
    fn drop(&mut self) {
        let _ = remove_file(&self.path);
    }
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
            assert!(path.exists());
            (path, resources.clone())
        };
        assert_eq!(clone.tracked_count(), 1);
        drop(resources);
        assert!(path.exists());
        drop(clone);
        assert!(!path.exists());
    }

    #[test]
    fn lazy_stage_keeps_its_input_files_until_its_last_owner() {
        let parent = ExecutionResources::new();
        let reservation = parent.reserve_temp_file("qlt-parent", "tmp").unwrap();
        let path = reservation.path().to_path_buf();
        parent.retain_temp_file(reservation).unwrap();
        let child = parent.fork(vec![parent.clone()]);
        let clone = child.clone();
        drop(parent);
        drop(child);
        assert!(path.exists());
        drop(clone);
        assert!(!path.exists());
    }

    #[test]
    fn abandoned_reservation_is_cleaned_immediately() {
        let reservation = ExecutionResources::new()
            .reserve_temp_file("qlt-test-abandoned", "tmp")
            .unwrap();
        let path = reservation.path().to_path_buf();
        assert!(path.exists());
        drop(reservation);
        assert!(!path.exists());
    }

    #[test]
    fn plan_resources_reject_reservations() {
        let resources = ExecutionResources::new_plan();
        assert!(resources.reserve_temp_file("plan", "tmp").is_err());
        assert_eq!(resources.tracked_count(), 0);
    }

    #[test]
    fn byte_budget_is_shared_by_forks_and_released_with_managed_files() {
        let mut resources = ExecutionResources::new();
        resources.budget = Arc::new(TempBudget {
            limit: Ok(Some(4)),
            used: std::sync::atomic::AtomicU64::new(0),
        });
        let mut reservation = resources.reserve_temp_file("budget", "tmp").unwrap();
        let first_path = reservation.path().to_path_buf();
        reservation.file_mut().unwrap().write_all(b"1234").unwrap();
        let child = resources.fork(vec![]);
        let mut other = child.reserve_temp_file("budget", "tmp").unwrap();
        assert!(other.file_mut().unwrap().write_all(b"x").is_err());
        assert_eq!(child.temp_usage(), (4, Some(4)));
        resources.retain_temp_file(reservation).unwrap();
        assert!(first_path.exists());
        drop(resources);
        drop(child);
        assert!(!first_path.exists());
        assert_eq!(other.file_mut().unwrap().write(b"1234").unwrap(), 4);
    }
}
