use crate::controllers::resources::ExecutionResources;
use crate::error::QuiltError;
use crate::operations::finalizers::{FinalizerResult, OutputArtifact};
use polars::prelude::*;
use std::sync::Arc;

/// Evaluate a lazy CSV sink into an execution-owned artifact. Rendering is
/// deliberately deferred to `write_stdout`, which copies in bounded chunks.
pub fn show(df: &LazyFrame, resources: &ExecutionResources) -> Result<FinalizerResult, QuiltError> {
    let output_resources = resources.fork(Vec::new());
    let reservation = output_resources
        .reserve_temp_file("qlt-show", "csv")
        .map_err(|error| {
            QuiltError::io("create show artifact", None::<String>, error.to_string())
        })?;
    let path = reservation.path().to_path_buf();
    output_resources
        .retain_temp_file(reservation)
        .map_err(|error| {
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
        }
        .into(),
        include_header: true,
        ..Default::default()
    };
    let sink = super::csv::csv_frame(df, "show CSV")?
        .sink(
            SinkDestination::File {
                target: SinkTarget::Path(PlRefPath::try_from_path(&path)?),
            },
            FileWriteFormat::Csv(options),
            UnifiedSinkArgs::default(),
        )
        .map_err(|error| QuiltError::finalizer("show CSV", error.to_string()))?;
    sink.collect()
        .map_err(|error| QuiltError::finalizer("show CSV", error.to_string()))?;
    Ok(FinalizerResult::Artifact(Arc::new(OutputArtifact::new(
        path,
        output_resources,
    ))))
}
