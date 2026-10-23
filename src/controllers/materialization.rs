//! Shared Parquet checkpointing for lazy frames.

use crate::controllers::resources::ExecutionResources;
use crate::QuiltError;
use polars::prelude::*;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

struct CappedWriter {
    file: crate::controllers::resources::AccountedFile,
    written: u64,
    limit: Option<u64>,
}

impl Write for CappedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(limit) = self.limit {
            let remaining = limit.saturating_sub(self.written);
            if bytes.len() as u64 > remaining {
                return Err(io::Error::other("Parquet checkpoint exceeds byte limit"));
            }
        }
        let n = self.file.write(bytes)?;
        self.written += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Stream a lazy frame into a managed Parquet checkpoint and return a scan of it.
/// The optional limit is enforced on every underlying write.
pub(crate) fn materialize_parquet(
    mut frame: LazyFrame,
    resources: &ExecutionResources,
    prefix: &str,
    byte_limit: Option<u64>,
) -> Result<(LazyFrame, ExecutionResources, u64), QuiltError> {
    let schema = frame
        .collect_schema()
        .map_err(|e| QuiltError::operation("materialize frame", e.to_string()))?;
    let owned = resources.fork(Vec::new());
    let mut reservation = owned
        .reserve_temp_file(prefix, "parquet")
        .map_err(|e| QuiltError::io("create materialization", None::<String>, e.to_string()))?;
    let path = reservation.path().to_path_buf();
    reservation.close_file();
    owned.retain_temp_file(reservation).map_err(|e| {
        QuiltError::io(
            "retain materialization",
            Some(path.display().to_string()),
            e.to_string(),
        )
    })?;
    let file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&path)
        .map_err(|e| {
            QuiltError::io(
                "open materialization",
                Some(path.display().to_string()),
                e.to_string(),
            )
        })?;
    let writer = CappedWriter {
        file: owned.accounted_file(&path, file).map_err(|e| {
            QuiltError::io(
                "open materialization",
                Some(path.display().to_string()),
                e.to_string(),
            )
        })?,
        written: 0,
        limit: byte_limit,
    };
    let batched = ParquetWriter::new(writer)
        .with_compression(ParquetCompression::Snappy)
        .batched(&schema)
        .map_err(|e| QuiltError::operation("materialize frame", e.to_string()))?;
    let shared = Arc::new(Mutex::new(Some(batched)));
    let callback_writer = shared.clone();
    let sink = frame
        .sink_batches(
            PlanCallback::new(move |mut batch: DataFrame| {
                // A callback batch can retain different chunk boundaries per
                // column after streaming vstack/splitting. Batched Parquet writes
                // require each row group batch to have aligned column chunks.
                batch.align_chunks_par();
                let mut guard = callback_writer.lock().map_err(|_| {
                    polars::error::PolarsError::ComputeError("Parquet writer lock poisoned".into())
                })?;
                let writer = guard.as_mut().ok_or_else(|| {
                    polars::error::PolarsError::ComputeError("Parquet writer unavailable".into())
                })?;
                writer
                    .write_batch(&batch)
                    .map_err(|e| polars::error::PolarsError::ComputeError(e.to_string().into()))?;
                Ok(false)
            }),
            true,
            NonZeroUsize::new(65_536),
        )
        .map_err(|e| QuiltError::operation("materialize frame", e.to_string()))?;
    sink.collect()
        .map_err(|e| QuiltError::operation("materialize frame", e.to_string()))?;
    let batched = shared
        .lock()
        .map_err(|_| QuiltError::operation("materialize frame", "Parquet writer poisoned"))?
        .take()
        .ok_or_else(|| QuiltError::operation("materialize frame", "Parquet writer unavailable"))?;
    let size = batched
        .finish()
        .map_err(|e| QuiltError::operation("materialize frame", e.to_string()))?;
    let scan =
        LazyFrame::scan_parquet(PlRefPath::try_from_path(&path)?, ScanArgsParquet::default())
            .map_err(|e| {
                QuiltError::io(
                    "scan materialization",
                    Some(path.display().to_string()),
                    e.to_string(),
                )
            })?;
    Ok((scan, owned, size))
}

#[cfg(test)]
mod tests {
    use super::materialize_parquet;
    use crate::controllers::resources::ExecutionResources;
    use polars::prelude::*;

    #[test]
    fn parquet_replay_aligns_misaligned_chunks_across_multiple_batches() {
        let left = (0i64..40_000).collect::<Vec<_>>();
        let left_tail = (40_000i64..70_000).collect::<Vec<_>>();
        let right = (0i64..20_000).collect::<Vec<_>>();
        let right_tail = (20_000i64..70_000).collect::<Vec<_>>();
        let mut a = Series::new("a".into(), left);
        a.append(&Series::new("a".into(), left_tail)).unwrap();
        let mut b = Series::new("b".into(), right);
        b.append(&Series::new("b".into(), right_tail)).unwrap();
        let frame = DataFrame::new_infer_height(vec![a.into(), b.into()]).unwrap();
        assert_eq!(
            frame
                .column("a")
                .unwrap()
                .as_materialized_series()
                .chunks()
                .iter()
                .map(|chunk| chunk.len())
                .collect::<Vec<_>>(),
            vec![40_000, 30_000]
        );
        assert_eq!(
            frame
                .column("b")
                .unwrap()
                .as_materialized_series()
                .chunks()
                .iter()
                .map(|chunk| chunk.len())
                .collect::<Vec<_>>(),
            vec![20_000, 50_000]
        );

        let dir = tempfile::tempdir().unwrap();
        let resources = ExecutionResources::new_in(dir.path().to_path_buf());
        let (replayed, _resources, _) =
            materialize_parquet(frame.lazy(), &resources, "qlt-test-aligned-batches", None)
                .unwrap();
        let output = replayed.collect().unwrap();
        assert_eq!(
            output
                .column("a")
                .unwrap()
                .as_materialized_series()
                .i64()
                .unwrap()
                .into_no_null_iter()
                .collect::<Vec<_>>(),
            (0i64..70_000).collect::<Vec<_>>()
        );
        assert_eq!(
            output
                .column("b")
                .unwrap()
                .as_materialized_series()
                .i64()
                .unwrap()
                .into_no_null_iter()
                .collect::<Vec<_>>(),
            (0i64..70_000).collect::<Vec<_>>()
        );
    }
}
