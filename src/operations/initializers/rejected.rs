use crate::controllers::log::LogController;
use crate::error::QuiltError;
use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Serialize)]
struct RejectRecord<'a> {
    source: &'a str,
    line: usize,
    raw_text: Option<&'a str>,
    raw_hex: Option<String>,
    reason: &'a str,
}

/// Streaming JSONL rejection report published atomically without overwriting an
/// existing destination. Invalid UTF-8 is preserved as lowercase hexadecimal.
pub(crate) struct RejectedReport {
    target: PathBuf,
    staging: PathBuf,
    file: Option<File>,
    record_count: usize,
}

pub(crate) type SharedRejectedReport = Arc<Mutex<RejectedReport>>;

impl RejectedReport {
    pub(crate) fn new(target: &Path) -> Result<Self, QuiltError> {
        if target.exists() {
            return Err(QuiltError::usage(format!(
                "rejects report already exists: {}",
                target.display()
            )));
        }
        let parent = target
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let name = target.file_name().unwrap_or_default().to_string_lossy();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        for attempt in 0..128u32 {
            let staging = parent.join(format!(
                ".{name}.{}.{}.{}.tmp",
                std::process::id(),
                stamp,
                attempt
            ));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            match options.open(&staging) {
                Ok(file) => {
                    return Ok(Self {
                        target: target.to_path_buf(),
                        staging,
                        file: Some(file),
                        record_count: 0,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(QuiltError::operation(
                        "create rejects report",
                        error.to_string(),
                    ));
                }
            }
        }
        Err(QuiltError::operation(
            "create rejects report",
            "could not reserve report staging file",
        ))
    }

    pub(crate) fn write(
        &mut self,
        source: &str,
        line: usize,
        raw: &[u8],
        reason: &str,
    ) -> Result<(), QuiltError> {
        let text = std::str::from_utf8(raw).ok();
        let hex = text.is_none().then(|| {
            raw.iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        });
        let record = RejectRecord {
            source,
            line,
            raw_text: text,
            raw_hex: hex,
            reason,
        };
        let file = self.file.as_mut().expect("report file is open");
        serde_json::to_writer(&mut *file, &record)
            .map_err(|error| QuiltError::operation("write rejects report", error.to_string()))?;
        file.write_all(b"\n")
            .map_err(|error| QuiltError::operation("write rejects report", error.to_string()))?;
        self.record_count += 1;
        Ok(())
    }

    pub(crate) fn publish(mut self) -> Result<(), QuiltError> {
        self.file
            .take()
            .expect("report file is open")
            .sync_all()
            .map_err(|error| QuiltError::operation("publish rejects report", error.to_string()))?;
        let parent = self
            .target
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        crate::operations::finalizers::publish_file_and_sync(
            &crate::operations::finalizers::NativePublicationBackend,
            &self.staging,
            &self.target,
            parent,
            "publish rejects report",
        )?;
        if self.record_count > 0 {
            LogController::warn(&format!(
                "Rejected {} record(s); details written to the rejects report",
                self.record_count
            ));
        }
        match std::fs::remove_file(&self.staging) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(QuiltError::operation(
                "clean up rejects report staging file",
                error.to_string(),
            )),
        }
    }
}

impl Drop for RejectedReport {
    fn drop(&mut self) {
        self.file.take();
        let _ = std::fs::remove_file(&self.staging);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_encodes_invalid_utf8_and_refuses_overwrite() {
        let root = std::env::temp_dir().join(format!(
            "qlt-reject-report-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("report.jsonl");
        let mut report = RejectedReport::new(&path).unwrap();
        report
            .write("source", 3, &[0xff, 0x61], "invalid UTF-8")
            .unwrap();
        report.publish().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"raw_hex\":\"ff61\""));
        assert!(RejectedReport::new(&path).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
