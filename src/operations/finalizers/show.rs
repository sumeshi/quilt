use crate::controllers::resources::ExecutionResources;
use crate::error::QuiltError;
use crate::operations::finalizers::{FinalizerResult, OutputArtifact};
use polars::prelude::*;
use std::sync::Arc;

/// Evaluate a lazy CSV sink into an execution-owned artifact. Rendering is
/// deliberately deferred to `write_stdout`, which copies in bounded chunks.
pub fn show(df: &LazyFrame, resources: &ExecutionResources) -> Result<FinalizerResult, QuiltError> {
    let reservation = resources
        .reserve_temp_file("qlt-show", "csv")
        .map_err(|error| {
            QuiltError::io("create show artifact", None::<String>, error.to_string())
        })?;
    let path = reservation.path().to_path_buf();
    resources.retain_temp_file(reservation).map_err(|error| {
        QuiltError::io(
            "retain show artifact",
            Some(path.display().to_string()),
            error.to_string(),
        )
    })?;
    let options = CsvWriterOptions {
        serialize_options: SerializeOptions {
            separator: b',',
            ..Default::default()
        },
        include_header: true,
        ..Default::default()
    };
    let sink = df
        .clone()
        .sink_csv(
            SinkTarget::Path(Arc::new(path.clone())),
            options,
            None,
            SinkOptions::default(),
        )
        .map_err(|error| QuiltError::finalizer("show CSV", error.to_string()))?;
    polars::prelude::collect_all([sink])
        .map_err(|error| QuiltError::finalizer("show CSV", error.to_string()))?;
    Ok(FinalizerResult::Artifact(Arc::new(OutputArtifact::new(
        path,
        resources.clone(),
    ))))
}
