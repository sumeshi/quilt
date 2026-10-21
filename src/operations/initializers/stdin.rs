use crate::controllers::resources::ExecutionResources;
use crate::error::QuiltError;
use std::io;
use std::path::PathBuf;

/// Copy stdin into an execution-owned spool without retaining its contents in memory.
/// Plan inspection resources are disabled, so the check happens before stdin is read.
pub(crate) fn validate_format(format: &str) -> Result<(), QuiltError> {
    if matches!(
        format.to_ascii_lowercase().as_str(),
        "csv" | "tsv" | "jsonl" | "ndjson"
    ) {
        Ok(())
    } else {
        Err(QuiltError::usage(
            "stdin format must be csv, tsv, jsonl, or ndjson",
        ))
    }
}

pub(super) fn spool(format: &str, resources: &ExecutionResources) -> Result<PathBuf, QuiltError> {
    validate_format(format)?;
    if !resources.temp_files_enabled() {
        return Err(QuiltError::usage(
            "run --show-plan cannot inspect stdin without execution",
        ));
    }
    let extension = match format.to_ascii_lowercase().as_str() {
        "csv" => "csv",
        "tsv" => "tsv",
        "jsonl" | "ndjson" => "jsonl",
        _ => unreachable!("format was validated before spooling"),
    };
    let mut reservation = resources
        .reserve_temp_file("qlt-stdin-spool", extension)
        .map_err(|error| QuiltError::operation("spool stdin", error.to_string()))?;
    let destination = reservation.path().to_path_buf();
    let file = reservation
        .file_mut()
        .ok_or_else(|| QuiltError::operation("spool stdin", "spool reservation unavailable"))?;
    io::copy(&mut io::stdin().lock(), file)
        .map_err(|error| QuiltError::operation("read stdin", error.to_string()))?;
    resources
        .retain_temp_file(reservation)
        .map_err(|error| QuiltError::operation("retain stdin spool", error.to_string()))?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_inspection_refuses_stdin_before_reading() {
        let resources = ExecutionResources::new_plan();
        let error = spool("jsonl", &resources).unwrap_err();
        assert!(error.to_string().contains("cannot inspect stdin"));
        assert_eq!(resources.tracked_count(), 0);
    }
}
